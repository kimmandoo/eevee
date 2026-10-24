//! ACP v1 fs callbacks and structured diff observations.
//! https://agentclientprotocol.com/protocol/v1/file-system
//! https://agentclientprotocol.com/protocol/v1/tool-calls
use super::*;
use crate::providers::writes::{FileTool, WriteFailure, WriteProposal, WriteSet};
use std::path::{Component, Path};

pub(super) struct PendingWrite {
    pub id: Value,
    proposal: WriteProposal,
    item_id: String,
    text: String,
}

pub(super) fn capture_files(
    workspace: &Workspace,
    input: &Input,
) -> Result<Option<WriteSet>, String> {
    if input.inspect_models || input.inspect_sessions {
        return Ok(None);
    }
    let scope = if input.operation == Operation::Implement {
        if input.work.as_ref().is_none_or(|claim| !claim.writing) {
            return Err(
                "ACP implementation requires the operation-bound durable writer lease".into(),
            );
        }
        if input.edit_paths.is_empty() {
            WriteSet::capture_workspace(
                &workspace.project,
                &workspace.data_dir,
                &workspace.exclusions,
            )?
        } else {
            WriteSet::capture(
                &workspace.project,
                &workspace.data_dir,
                &input.edit_paths,
                &workspace.exclusions,
            )?
        }
    } else {
        WriteSet::capture_readonly(
            &workspace.project,
            &workspace.data_dir,
            &workspace.exclusions,
        )?
    };
    Ok(Some(scope))
}

fn relative_path(root: &Path, params: &Value) -> Result<PathBuf, String> {
    let raw = params
        .get("path")
        .and_then(Value::as_str)
        .ok_or("ACP file path must be a string")?;
    let path = Path::new(raw);
    if raw.len() > 4096
        || !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(
            "ACP file path must be an absolute literal path inside the current project".into(),
        );
    }
    path.strip_prefix(root)
        .map(Path::to_path_buf)
        .map_err(|_| "ACP file path is outside the current project".into())
}

// ACP returns content only, so never silently return a byte-truncated slice.
// Start at the requested line and require a narrower limit if output is bounded.
fn read_content(scope: &mut WriteSet, path: PathBuf, params: &Value) -> Result<String, String> {
    let number = |key: &str, default: usize| -> Result<usize, String> {
        match params.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(value) => value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0)
                .ok_or_else(|| format!("ACP {key} must be a positive integer")),
        }
    };
    let line = number("line", 1)?;
    let limit = number("limit", usize::MAX)?;
    let result = scope.observe_tool(&FileTool::Read {
        path,
        start_line: line,
        end_line: 0,
    })?;
    let content = result["content"]
        .as_str()
        .ok_or("Source executor returned no content")?;
    let selected: String = content.split_inclusive('\n').take(limit).collect();
    if result["next_start_line"].is_number() && content.split_inclusive('\n').count() < limit {
        return Err("ACP requested read exceeds the 64 KiB output bound; retry with a narrower line/limit range".into());
    }
    Ok(selected)
}

fn file_permission(proposal: &WriteProposal) -> PromptKind {
    PromptKind::Permission {
        description: format!(
            "Eevee-mediated ACP client write only. Agent-owned tools remain external provider policy; this consent does not authorize them.\n{}",
            proposal.description()
        ),
        file: Some(proposal.file_review()),
        options: vec![
            PermissionOption {
                label: "Deny".into(),
                description: "No client filesystem changes".into(),
                answer: json!({"decision":"decline"}),
            },
            PermissionOption {
                label: "Allow once".into(),
                description: "Apply only this exact content and listed parents once".into(),
                answer: json!({"decision":"accept"}),
            },
        ],
    }
}

// Only ACP's typed content.diff schema is a file observation. rawInput, title,
// locations and opaque tool names are never guessed into a write or approval.
fn diffs(content: &Value) -> Vec<(&str, &str, Option<&str>)> {
    let Some(entries) = content.as_array() else {
        return Vec::new();
    };
    if entries.len() > crate::providers::writes::MAX_FILES {
        return Vec::new();
    }
    entries
        .iter()
        .filter_map(|entry| {
            if entry.get("type").and_then(Value::as_str) != Some("diff") {
                return None;
            }
            let path = entry.get("path")?.as_str()?;
            let new = entry.get("newText")?.as_str()?;
            let old = match entry.get("oldText") {
                None | Some(Value::Null) => None,
                Some(Value::String(text)) => Some(text.as_str()),
                _ => return None,
            };
            if path.is_empty()
                || path.len() > 4096
                || new.len() > crate::providers::writes::MAX_BYTES
                || old.is_some_and(|text| text.len() > crate::providers::writes::MAX_BYTES)
            {
                return None;
            }
            Some((path, new, old))
        })
        .collect()
}

pub(super) fn external_file_review(tool: &Value) -> Option<crate::providers::FileReview> {
    if tool.get("kind").and_then(Value::as_str) != Some("edit") {
        return None;
    }
    let content = tool.get("content")?;
    let entries = content.as_array()?;
    let diffs = diffs(content);
    if entries.len() != 1 || diffs.len() != 1 {
        return None;
    }
    entries[0].get("oldText")?;
    let (path, content, old) = diffs[0];
    Some(crate::providers::FileReview {
        path: path.into(),
        destination: None,
        syntax: Some(crate::providers::RustSyntaxReview::Unavailable),
        kind: if old.is_some() {
            crate::providers::FileReviewKind::Modify
        } else {
            crate::providers::FileReviewKind::Create
        },
        directories: Vec::new(),
        bytes: content.len(),
        excerpt: crate::providers::FileReview::excerpt(content),
        edit: false,
        external: true,
    })
}

impl Engine<'_> {
    pub(super) fn validate_file_owner(&self) -> Result<(), String> {
        self.run
            .input
            .validate_committed(&self.workspace.data_dir)?;
        let writing = self.run.input.operation == Operation::Implement;
        if (writing
            && self
                .run
                .input
                .work
                .as_ref()
                .is_none_or(|claim| !claim.writing))
            || self
                .files
                .as_ref()
                .is_none_or(|files| files.writing() != writing)
        {
            return Err("ACP file callbacks lost their operation-bound durable lease".into());
        }
        let (index, _) = profile(self.run.input.agent)?;
        crate::providers::agent::revalidate_external_trust(
            index,
            &self.workspace.project,
            &self.run.trust,
        )
    }

    fn file_error(&mut self, id: Value, message: &str) -> Result<(), String> {
        self.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":safe(message)}}))
    }

    fn publish_file(&self, id: &str, text: &str, phase: FilePhase) -> Result<(), String> {
        emit(
            self.events,
            Event::Item {
                generation: self.run.generation,
                task_id: self.run.input.task.id,
                thread_id: self
                    .session
                    .clone()
                    .ok_or("ACP file observation before session")?,
                turn_id: self.turn(),
                item_id: id.into(),
                kind: ItemKind::File(phase),
                text: text.into(),
                complete: true,
            },
        )
    }

    pub(super) fn file_request(
        &mut self,
        id: Value,
        method: &str,
        params: &Value,
    ) -> Result<(), String> {
        if self.permissions.seen.len() >= 4096 || !self.permissions.seen.insert(id.to_string()) {
            return Err("Repeated/excessive ACP callback identity; no replay".into());
        }
        let prepared = (|| -> Result<PathBuf, String> {
            if !self.dispatched
                || self.terminal.is_some()
                || self.cancelling.is_some()
                || self.stop.load(Ordering::Acquire)
                || self.cancelled.load(Ordering::Acquire) == self.run.generation
            {
                return Err("ACP client file callback is not in an active owned turn".into());
            }
            if params.get("sessionId").and_then(Value::as_str) != self.session.as_deref() {
                return Err("ACP client file callback has wrong session identity".into());
            }
            self.validate_file_owner()?;
            relative_path(&self.workspace.project, params)
        })();
        let path = match prepared {
            Ok(path) => path,
            Err(error) => return self.file_error(id, &error),
        };
        if method == "fs/read_text_file" {
            let result = read_content(
                self.files.as_mut().ok_or("ACP file scope missing")?,
                path,
                params,
            );
            self.validate_file_owner()?;
            return match result {
                Ok(content) => {
                    self.send(json!({"jsonrpc":"2.0","id":id,"result":{"content":content}}))
                }
                Err(error) => self.file_error(id, &error),
            };
        }
        self.callback_sequence += 1;
        let item_id = format!(
            "eevee-file-acp-{}-{}",
            self.run.generation, self.callback_sequence
        );
        let content = params.get("content").and_then(Value::as_str);
        let text = content
            .filter(|text| text.len() <= crate::providers::writes::MAX_BYTES)
            .map(|content| {
                crate::providers::file_preview::display(&path.to_string_lossy(), content, true)
            });
        if let Some(text) = &text {
            self.publish_file(&item_id, text, FilePhase::Received)?;
        }
        let proposal = (|| -> Result<WriteProposal, String> {
            if self.run.input.operation != Operation::Implement {
                return Err("ACP Plan/Review does not permit client writes".into());
            }
            if self.pending_write.is_some() || !self.permissions.pending.is_empty() {
                return Err("Concurrent ACP file permission refused".into());
            }
            let content = content.ok_or("ACP write requires complete UTF-8 content")?;
            if content.len() > crate::providers::writes::MAX_BYTES {
                return Err("ACP write exceeds 64 KiB".into());
            }
            let mut candidate = self.files.clone().ok_or("ACP file scope missing")?;
            candidate.admit_paths(std::slice::from_ref(&path))?;
            let proposal = candidate.propose(path, content.into())?;
            self.files = Some(candidate);
            Ok(proposal)
        })();
        let proposal = match proposal {
            Ok(proposal) => proposal,
            Err(error) => {
                if let Some(text) = &text {
                    self.publish_file(&item_id, text, FilePhase::Refused)?;
                }
                return self.file_error(id, &error);
            }
        };
        let text = text.ok_or("ACP complete write preview missing")?;
        self.pending_write = Some(PendingWrite {
            id: id.clone(),
            proposal,
            item_id: item_id.clone(),
            text: text.clone(),
        });
        self.publish_file(&item_id, &text, FilePhase::AwaitingReview)?;
        emit(
            self.events,
            Event::Prompt {
                generation: self.run.generation,
                task_id: self.run.input.task.id,
                request: Prompt {
                    arrival: super::super::approval_arrival(),
                    request_id: id,
                    thread_id: self.session.clone().ok_or("ACP file session missing")?,
                    turn_id: self.turn(),
                    item_id,
                    kind: file_permission(&self.pending_write.as_ref().unwrap().proposal),
                },
            },
        )
    }

    pub(super) fn answer_file(
        &mut self,
        generation: u64,
        id: Value,
        answer: Value,
    ) -> Result<(), String> {
        if generation != self.run.generation
            || !matches!(
                answer.get("decision").and_then(Value::as_str),
                Some("accept" | "decline")
            )
            || answer.as_object().is_none_or(|object| object.len() != 1)
        {
            return emit(
                self.events,
                Event::Error {
                    generation: Some(generation),
                    message: "Stale/invalid ACP file answer rejected; no write".into(),
                },
            );
        }
        if self.cancelling.is_some()
            || self.stop.load(Ordering::Acquire)
            || self.cancelled.load(Ordering::Acquire) == generation
        {
            return self.close_file_permission();
        }
        let pending = self
            .pending_write
            .take()
            .ok_or("ACP file approval already consumed")?;
        let mut phase = FilePhase::Denied;
        let mut receipt = None;
        let mut unknown = false;
        let message = if answer["decision"] == "accept" {
            phase = FilePhase::Refused;
            let ready = (|| -> Result<(), String> {
                self.validate_file_owner()?;
                if self.cancelling.is_some()
                    || self.terminal.is_some()
                    || self.stop.load(Ordering::Acquire)
                    || self.cancelled.load(Ordering::Acquire) == generation
                {
                    return Err("Cancelled ACP file write refused before effects".into());
                }
                if self.events.lock().len() >= EVENT_LIMIT - 16 {
                    return Err(
                        "ACP file write refused before effects: result queue is full".into(),
                    );
                }
                Ok(())
            })();
            let result = match ready {
                Ok(()) => self.files.as_mut().unwrap().apply(&pending.proposal),
                Err(message) => Err(WriteFailure {
                    message,
                    uncertain: false,
                }),
            };
            match result {
                Ok(result) => {
                    phase = FilePhase::Applied;
                    receipt = Some(result.receipt());
                    result.message
                }
                Err(error) => {
                    unknown = error.uncertain;
                    if unknown {
                        phase = FilePhase::Unknown;
                    }
                    error.message
                }
            }
        } else {
            "ACP client file write denied; no Eevee filesystem changes".into()
        };
        if unknown {
            self.terminal = Some((Outcome::Unknown, message.clone()));
            emit(
                self.events,
                Event::LocalEffectsUnknown {
                    generation,
                    task_id: self.run.input.task.id,
                    message: message.clone(),
                },
            )?;
        }
        // Persist the actual local outcome before attempting to reply to the peer.
        emit(self.events, Event::Journal {
            generation, task_id: self.run.input.task.id,
            record: crate::store::ProviderRecord {
                id: format!("acp-local-{}-{}-{}", std::process::id(), self.journal_nonce, pending.item_id),
                agent: self.run.input.agent, generation,
                thread_id: self.session.clone().ok_or("ACP file receipt session missing")?, turn_id: self.turn(), item_id: pending.item_id.clone(),
                origin: "ACP-requested Eevee-mediated file write".into(),
                coverage: "Exact one-shot content/parents; Applied requires local resulting digest. Not provider-owned tool evidence, a check, or a commit.".into(),
                text: receipt.unwrap_or_else(|| safe(&message)), complete: !unknown,
                partner: self.run.input.work.as_ref().map(|claim| claim.owner.clone()),
            },
        })?;
        self.publish_file(&pending.item_id, &pending.text, phase)?;
        emit(
            self.events,
            Event::PromptClosed {
                generation,
                request_id: id.clone(),
            },
        )?;
        if phase == FilePhase::Applied {
            self.send(json!({"jsonrpc":"2.0","id":id,"result":{}}))?;
        } else {
            self.file_error(id, &message)?;
        }
        self.status(&message)
    }

    pub(super) fn close_file_permission(&mut self) -> Result<(), String> {
        if let Some(pending) = self.pending_write.take() {
            self.publish_file(&pending.item_id, &pending.text, FilePhase::Interrupted)?;
            emit(
                self.events,
                Event::PromptClosed {
                    generation: self.run.generation,
                    request_id: pending.id.clone(),
                },
            )?;
            self.file_error(
                pending.id,
                "ACP file approval cancelled; no client write performed",
            )?;
        }
        Ok(())
    }

    pub(super) fn observe_file_reports(
        &mut self,
        tool_id: &str,
        update: &Value,
    ) -> Result<(), String> {
        if let Some(content) = update.get("content") {
            let observed = diffs(content)
                .into_iter()
                .enumerate()
                .map(|(index, (path, new, _))| {
                    (
                        format!("eevee-file-{tool_id}-{index}"),
                        crate::providers::file_preview::display(path, new, true),
                    )
                })
                .collect::<Vec<_>>();
            // Empty/malformed replacement snapshots do not erase previous observations.
            if !observed.is_empty() {
                self.reported_files.insert(tool_id.into(), observed);
            }
            let (count, bytes) = self
                .reported_files
                .values()
                .flatten()
                .fold((0usize, 0usize), |(count, bytes), (_, text)| {
                    (count + 1, bytes + text.len())
                });
            if count > ITEM_LIMIT || bytes > OUTPUT_LIMIT {
                return Err("ACP structured file observation bound exceeded".into());
            }
        }
        let status = self
            .items
            .get(tool_id)
            .and_then(|item| item.fields.get("status"))
            .and_then(Value::as_str);
        let phase = match status {
            Some("completed") => FilePhase::Reported,
            Some("failed") => FilePhase::ReportedFailed,
            _ => FilePhase::Received,
        };
        if let Some(files) = self.reported_files.get(tool_id) {
            for (id, text) in files {
                self.publish_file(id, text, phase)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
