//! Background worker events: agent preflight, repository and store results.
use super::*;

impl App {
    pub fn agent_event(&mut self, event: crate::providers::agent::Event) -> bool {
        self.agents.event(event);
        // Discovery/probe results belong to the connections drawer. In
        // particular, delayed startup discovery must not repaint an idle home.
        self.overlay == Some(Overlay::Agents)
    }

    pub fn repo_event(&mut self, event: repo::Event) {
        use std::fmt::Write;
        self.repo_loading = false;
        match event {
            repo::Event::Updated(report) => {
                self.repo_status = if report.initial {
                    "Repository: baseline captured"
                } else {
                    "Repository: refreshed / original baseline retained"
                }
                .into();
                self.repo_error.clear();
                let mut text = format!(
                    "ROOT\n{:?}\nGit metadata: {:?}\n\nBASELINE / CURRENT\nHEAD: {} / {}\nFiles: {} / {}\nCurrent coverage complete: {} / stable observation: {}\nIgnored untracked files and submodule contents are excluded.\n\nCHANGES\n",
                    report.repository.root,
                    report.repository.git_dir,
                    report
                        .baseline
                        .head
                        .as_deref()
                        .unwrap_or("unborn/unavailable"),
                    report
                        .current
                        .head
                        .as_deref()
                        .unwrap_or("unborn/unavailable"),
                    report.baseline.files.len(),
                    report.current.files.len(),
                    report.current.complete,
                    report.current.stable
                );
                if report.changes.is_empty() {
                    text.push_str("No observed changes in captured scope; incomplete coverage is not a clean verdict.\n");
                }
                for change in &report.changes {
                    if text.len() >= 64 * 1024 {
                        text.push_str("Change list display truncated; no full-coverage claim.\n");
                        break;
                    }
                    let origin = match change.origin {
                        repo::ChangeOrigin::PreExisting => "pre-existing",
                        repo::ChangeOrigin::Unattributed => "unattributed",
                        repo::ChangeOrigin::Unknown => "unknown",
                    };
                    let _ = writeln!(
                        text,
                        "{origin} / {:?} / initially dirty: {} / {:?}",
                        change.kind, change.was_dirty, change.path
                    );
                }
                text.push_str("\nREAD-ONLY RAW HEAD → WORKTREE DIFF\nNo filters/textconv; staged identities are tracked separately. No authorship is inferred.\n");
                if report.current.working_diff.truncated {
                    text.push_str("TRUNCATED: only the bounded captured diff follows.\n");
                }
                if let Some(error) = &report.current.working_diff.unavailable {
                    let _ = writeln!(text, "Diff unavailable/partial: {error}");
                }
                text.push_str(&report.current.working_diff.text);
                text.push_str("\nOBSERVATION WARNINGS\n");
                for warning in report.current.warnings.iter().take(16) {
                    let _ = writeln!(text, "{warning}");
                }
                text.retain(safe_character);
                self.repo_text = text;
                self.repository = Some(report);
            }
            repo::Event::Unsupported { path, reason } => {
                self.repo_status = "Repository: unsupported".into();
                self.repo_error =
                    format!("{path:?}: {reason}. Task data is retained; /refresh retries.");
                self.repo_error.retain(safe_character);
            }
            repo::Event::Failed { operation, message } => {
                self.repo_status = "Repository: inspection failed".into();
                self.repo_error = format!(
                    "{operation}: {message}. Previous evidence is retained; /refresh retries the read-only inspection."
                );
                self.repo_error.retain(safe_character);
            }
        }
        if self.overlay == Some(Overlay::Workflows) {
            self.workflow_refresh();
        }
    }

    pub fn store_event(&mut self, event: store::Event) {
        match event {
            store::Event::Memory { task_id, result } => {
                if task_id == self.task_id {
                    let before = self.memory.draft_snapshot();
                    let saving = self.memory.saving;
                    self.memory_loading = false;
                    self.memory.finish(result);
                    if saving || before != self.memory.draft_snapshot() {
                        self.changed_task();
                    }
                }
            }
            store::Event::Journal { task_id, result } => {
                self.planning.journal_event(task_id, result);
            }
            store::Event::Party(result) => {
                self.party_inflight = false;
                match result {
                    Ok(partners) => {
                        if let Some(partner) = partners.last() {
                            self.notice = format!(
                                "{} joined the party in Eevee form ({} of {}).",
                                Self::partner_label(partner),
                                partners.len(),
                                store::MAX_PARTNERS
                            );
                        }
                        self.party = partners;
                    }
                    Err(message) => {
                        self.notice = message;
                    }
                }
            }
            store::Event::Loaded(project) => {
                let local_changes = self.revision != self.saved_revision;
                self.usage_bytes = project.usage_bytes;
                self.tasks = project.tasks;
                self.party = project.partners.clone();
                let target = if self.task_id > 0 {
                    Some(self.task_id)
                } else {
                    project.active_id
                };
                if let Some(snapshot) = self
                    .tasks
                    .iter()
                    .find(|task| Some(task.id) == target)
                    .cloned()
                {
                    // The task's lead partner fronts this session; its stored
                    // form is what the task resumes into. With local changes,
                    // keep the live lead (unsaved lead/form switches) and only
                    // refresh its identity fields from the committed roster.
                    if !local_changes {
                        self.load_task(snapshot);
                    } else if self.task_id == snapshot.id {
                        if let Some(partner) = project
                            .partners
                            .iter()
                            .find(|partner| partner.id == self.lead.id)
                        {
                            let form = self.lead.form;
                            self.lead = partner.clone();
                            self.lead.form = form;
                        }
                        self.saved_revision = snapshot.revision;
                        self.revision = self.revision.max(snapshot.revision.saturating_add(1));
                    }
                }
                self.save_requested = self.revision != self.saved_revision;
                self.storage = if self.save_requested {
                    StorageState::Saving
                } else {
                    StorageState::Saved
                };
            }
            store::Event::Selected(snapshot) => {
                self.load_task(snapshot);
                self.transitioning = false;
                self.notice.clear();
            }
            store::Event::Saved(snapshot) => {
                if self.task_id == 0 {
                    self.task_id = snapshot.id;
                    self.memory.reset(snapshot.id, snapshot.project.clone());
                    self.memory_loading = true;
                    self.memory_request = Some(store::MemoryRequest {
                        task_id: snapshot.id,
                        mutation: None,
                    });
                }
                if self.task_id == snapshot.id {
                    self.updated_at = snapshot.updated_at;
                    self.saved_revision = self.saved_revision.max(snapshot.revision);
                    if snapshot.revision == self.revision {
                        self.goal.clone_from(&snapshot.goal);
                        self.criteria.clone_from(&snapshot.criteria);
                        self.next_action.clone_from(&snapshot.next_action);
                        self.command.clone_from(&snapshot.command);
                        self.redacted = snapshot.redacted;
                        if snapshot.redacted {
                            self.notice = "Identifiable secrets were masked in local storage. Detection is incomplete; review exports before sharing.".into();
                        }
                        self.drafts.clone_from(&snapshot.drafts);
                        self.storage = StorageState::Saved;
                        if let Some(command) = self.pending_command.take() {
                            self.execution.prepare_command(self.task_id, &command);
                        }
                    }
                }
                if let Some(previous) = self.tasks.iter_mut().find(|task| task.id == snapshot.id) {
                    *previous = snapshot;
                } else {
                    self.tasks.push(snapshot);
                }
            }
            store::Event::Managed {
                project,
                affected,
                warning,
            } => {
                self.transitioning = false;
                self.usage_bytes = project.usage_bytes;
                let current_removed = self.task_id > 0
                    && affected.contains(&self.task_id)
                    && !project
                        .tasks
                        .iter()
                        .any(|task| task.id == self.task_id && !task.archived);
                self.party = project.partners;
                self.tasks = project.tasks;
                if current_removed {
                    let next = self
                        .tasks
                        .iter()
                        .find(|task| Some(task.id) == project.active_id && !task.archived)
                        .cloned()
                        .unwrap_or_else(|| store::Snapshot::empty(self.cwd.clone()));
                    self.load_task(next);
                    self.open_overlay(Overlay::Tasks);
                }
                self.task_selection = self
                    .task_selection
                    .min(self.visible_tasks().count().saturating_sub(1));
                self.delete_targets.clear();
                self.delete_preview.clear();
                self.notice = format!(
                    "Updated {} selected local task records. Repository files and exports were not changed.",
                    affected.len()
                );
                if let Some(warning) = warning {
                    self.notice
                        .push_str(&format!(" Cleanup warning: {warning}"));
                }
            }
            store::Event::ManagementFailed(message) => {
                self.transitioning = false;
                self.deletion_preview_pending = false;
                self.notice = format!("Task operation failed: {message}. No automatic retry.");
            }
            store::Event::DeletePreview {
                targets,
                expired_before,
            } => {
                if self.deletion_preview_pending
                    && targets
                        .iter()
                        .map(|target| target.task.id)
                        .eq(self.delete_targets.iter().copied())
                {
                    self.deletion_preview_pending = false;
                    self.transitioning = false;
                    self.delete_preview = targets;
                    self.delete_page = 0;
                    self.delete_expired_before = expired_before;
                    self.surface_scroll[ScrollSurface::Preview.index()] = 0;
                    self.confirmation = Some(Confirmation {
                        kind: ConfirmationKind::DeleteTasks,
                        destructive: false,
                    });
                }
            }
            store::Event::Context { task_id, result } => {
                self.sources.stored(self.task_id, task_id, result);
                if self.overlay == Some(Overlay::Workflows) {
                    self.workflow_refresh();
                }
            }
            store::Event::Run {
                task_id,
                request_id,
                run_id,
                result,
            } => {
                self.execution.store_event(
                    task_id,
                    request_id.as_deref(),
                    run_id.as_deref(),
                    result.map(|response| *response),
                );
                if self.overlay == Some(Overlay::Workflows) {
                    self.workflow_refresh();
                }
            }
            store::Event::Failed { revision, message } => {
                self.transitioning = false;
                if revision.is_none_or(|revision| revision == self.revision) {
                    self.storage = StorageState::Failed(message);
                    self.quit_after_save = false;
                }
            }
        }
        self.settle_exit();
    }
}
