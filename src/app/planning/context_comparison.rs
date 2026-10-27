//! Inspect real worker-prepared payloads without a work claim or dispatch path.
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use super::{State, context, store};
use crate::{
    app::{App, Overlay, sources},
    providers::{Event, Request},
};

pub(in crate::app) struct Prepared {
    pub turn: context::ComparisonTurn,
    pub selection: [u8; 32],
    pub basis: [u8; 32],
    pub local_only: bool,
    pub not_before_arrival: u64,
}

impl State {
    /// Revision/save time, UI navigation and caret positions are not turn facts.
    /// In particular a basket-only store acknowledgment cannot change this key.
    pub(in crate::app) fn context_basis(
        &self,
        task: &store::Snapshot,
        resume: Option<&str>,
    ) -> [u8; 32] {
        let bytes = serde_json::to_vec(&(
            &task.project,
            task.id,
            task.lead_partner,
            &task.goal,
            &task.criteria,
            &task.next_action,
            &task.command,
            &task.workflows,
            &task.approved_memories,
            task.request_text(),
            task.complete,
            task.archived,
            &self.selection,
            &self.edit_paths,
            &self.handoff,
            resume,
        ))
        .expect("serializable local preparation facts");
        let mut hash = Sha256::new();
        hash.update(bytes);
        hash.update(
            serde_json::to_vec(&self.screenshot.as_ref().map(|image| &image.capture))
                .expect("serializable frozen screenshot identity"),
        );
        hash.update(
            serde_json::to_vec(&self.context_plan.as_ref().map(|plan| {
                (
                    &plan.packet,
                    &plan.basis,
                    &plan.policy,
                    &plan.tokenizer,
                    plan.budget,
                    &plan.guards,
                )
            }))
            .expect("serializable frozen context identity"),
        );
        hash.update(
            serde_json::to_vec(&self.tool_catalog.as_ref().map(|catalog| {
                (
                    &catalog.document_digest,
                    &catalog.source_path,
                    &catalog.source_sha256,
                    &catalog.connection_scope,
                    catalog.agent,
                    catalog.task_id,
                    catalog.partner,
                    &catalog.entries,
                )
            }))
            .expect("serializable frozen catalog identity"),
        );
        hash.update([
            u8::from(self.authentication_invalidated),
            u8::from(self.connection_invalidated),
        ]);
        if let Some(history) = self.native_histories.get(&(self.agent, self.operation)) {
            hash.update(history.owner.connection_scope.len().to_le_bytes());
            hash.update(history.owner.connection_scope.as_bytes());
            for message in history.messages.iter() {
                hash.update([match message.role {
                    crate::providers::ConversationRole::User => 0,
                    crate::providers::ConversationRole::Assistant => 1,
                }]);
                hash.update(message.text.len().to_le_bytes());
                hash.update(message.text.as_bytes());
            }
        }
        hash.finalize().into()
    }

    pub(in crate::app) fn context_inspection(&self) -> bool {
        self.context_inspection_only
    }

    fn prepare_context_receipt(
        &mut self,
        resume: bool,
        task: &store::Snapshot,
        excerpts: &[context::Excerpt],
        report: Option<&std::sync::Arc<crate::workspace::repo::Report>>,
    ) -> Result<(), String> {
        if self.task != task.id {
            if self.task != 0
                || !self.items.is_empty()
                || self.thread_id.is_some()
                || !self.native_histories.is_empty()
                || !self.sessions.is_empty()
            {
                return Err(
                    "Context inspection cannot replace another task's retained conversation."
                        .into(),
                );
            }
            self.task = task.id;
        }
        // Move, do not copy, the retained presentation aside. `prepare` still
        // constructs the exact ordinary immutable Input, but inspection must
        // not rewrite the prior response's attribution or paused viewport.
        let submitted = std::mem::take(&mut self.submitted_request);
        let notice = std::mem::take(&mut self.notice);
        let instruction = self.submitted_instruction.take();
        let observed = self.observed_model.take();
        let receipt = std::mem::take(&mut self.receipt);
        let receipt_claim = self.receipt_claim.take();
        let inspection = self.inspection.take();
        let candidate = self.native_candidate.take();
        let request_partner = self.native_request_partner;
        let request_selection = std::mem::take(&mut self.request_selection);
        let generation = self.generation;
        let slots = std::mem::take(&mut self.slots);
        let prompts = std::mem::take(&mut self.queued_prompts);
        let cache = std::mem::take(&mut self.response_cache);
        let scroll = (self.scroll, self.scroll_max, self.saved_scroll);
        let flags = (
            self.inspect_models,
            self.inspect_sessions,
            self.connection_invalidated,
            self.catalogue_dismissed,
        );
        self.prepare(resume, false, false, task, excerpts, report);
        let result = if let Some(Request::Preview(input)) = &mut self.pending {
            input.local_only = true;
            Ok(())
        } else {
            Err(std::mem::take(&mut self.notice))
        };
        self.notice = notice;
        self.submitted_request = submitted;
        self.submitted_instruction = instruction;
        self.observed_model = observed;
        self.receipt = receipt;
        self.receipt_claim = receipt_claim;
        self.inspection = inspection;
        self.native_candidate = candidate;
        self.native_request_partner = request_partner;
        self.request_selection = request_selection;
        self.generation = generation;
        self.slots = slots;
        self.queued_prompts = prompts;
        self.response_cache = cache;
        (self.scroll, self.scroll_max, self.saved_scroll) = scroll;
        (
            self.inspect_models,
            self.inspect_sessions,
            self.connection_invalidated,
            self.catalogue_dismissed,
        ) = flags;
        self.send_after_prepare = false;
        self.invalidate_review();
        result
    }

    pub(in crate::app) fn consume_context_preview(&mut self, event: &Event) -> bool {
        if !self.context_inspection() {
            return false;
        }
        let completed = match event {
            Event::Preview(preview) => self.accepts_preview(preview),
            Event::Error {
                generation: None, ..
            } => self.awaiting_preview,
            _ => false,
        };
        if !completed {
            return false;
        }
        self.context_inspection_only = false;
        self.awaiting_preview = false;
        self.context_preview = None;
        self.preview = None;
        self.pending = None;
        self.send_after_prepare = false;
        self.auto_run = false;
        self.cancel_when_ready = false;
        self.invalidate_review();
        true
    }
}

impl App {
    /// Existing Sources sheet and draft ownership; inspection grants nothing.
    pub(in crate::app) fn open_context_comparison(&mut self) -> bool {
        if self.sources.editor.is_some() || self.sources.confirmation.is_some() {
            return true;
        }
        if self.editing {
            self.retain_editor();
            self.composer_return = Some(self.editor_field);
        }
        self.open_overlay(Overlay::Sources);
        self.sources.mode = sources::Mode::Comparison;
        self.sources.comparison_refresh = true;
        self.settle_context_comparison();
        true
    }

    /// Called by the ordinary app settlement loop, also after committed chip
    /// removal. Existing workers revalidate sources and committed references.
    pub(in crate::app) fn settle_context_comparison(&mut self) -> bool {
        if !self.sources.comparison_refresh || self.planning.context_inspection() {
            return false;
        }
        if self.sources.busy() || !self.committed_input_ready() {
            return false;
        }
        if self.planning.preview.is_some() {
            // A prepared send's old one-shot review cannot survive a comparison.
            self.planning.cancel();
            self.refresh_approvals();
            return true;
        }
        if self.planning.cancelling {
            return false;
        }
        if self.planning.busy() || self.planning.unknown_effects() {
            self.sources.comparison_refresh = false;
            self.sources.error =
                "Wait for owned provider work before comparing context locally.".into();
            return true;
        }
        let Some(task) = self
            .tasks
            .iter()
            .find(|task| task.id == self.task_id)
            .cloned()
        else {
            self.sources.comparison_refresh = false;
            self.sources.error =
                "Save an active task before preparing its context comparison.".into();
            return true;
        };
        if let Err(error) = self.memory.selected_snapshot() {
            self.sources.comparison_refresh = false;
            self.sources.error = error;
            return true;
        }
        let resume = self.sources.comparison_resume.is_some();
        if resume && self.sources.comparison_resume != self.planning.thread_id {
            self.sources.clear_wire_request();
            self.sources.error =
                "The prepared session changed; no context comparison retained.".into();
            return true;
        }
        if let Err(error) = self.prepare_context_tools(&task) {
            self.sources.comparison_refresh = false;
            self.sources.error = error;
            return true;
        }
        if let Err(error) = self.planning.prepare_context_receipt(
            resume,
            &task,
            &self.sources.basket,
            self.repository.as_ref(),
        ) {
            self.sources.comparison_refresh = false;
            self.sources.error = error;
            return true;
        }
        self.planning.context_inspection_only = true;
        // Reuse instruction substitution, but never acquire a work lease, record
        // a dispatch intention, arm automatic Run, or expose a send approval.
        self.stage_provider_work();
        true
    }

    pub(in crate::app) fn remember_context_preparation(&mut self, local_only: bool) {
        let Some(Request::Preview(input)) = &self.planning.pending else {
            return;
        };
        let basis = self.context_preparation_basis(&input.task, input.resume.as_deref());
        let selection = context::selection_identity(&input.excerpts);
        if !local_only {
            self.sources.clear_wire_request();
        }
        let same = self.sources.comparison_basis == Some(basis)
            && self.sources.comparison.turn.as_ref().is_some_and(|turn| {
                turn.task_id == input.task.id
                    && turn.partner == input.task.lead_partner
                    && turn.agent == input.agent
                    && turn.operation == input.operation
            });
        let request = if same {
            self.sources
                .comparison
                .turn
                .as_ref()
                .expect("same turn")
                .request
        } else {
            self.sources.clear_wire_request();
            static NEXT: AtomicU64 = AtomicU64::new(1);
            NEXT.fetch_add(1, Ordering::Relaxed)
        };
        self.sources.comparison_basis = Some(basis);
        self.sources.comparison_refresh = local_only;
        self.sources.comparison_resume.clone_from(&input.resume);
        self.planning.context_preview = Some(Prepared {
            turn: context::ComparisonTurn {
                task_id: input.task.id,
                partner: input.task.lead_partner,
                agent: input.agent,
                operation: input.operation,
                request,
            },
            selection,
            basis,
            local_only,
            not_before_arrival: crate::providers::approval_arrival(),
        });
    }

    pub(in crate::app) fn record_context_preview(&mut self, preview: &crate::providers::Preview) {
        let Some(prepared) = self.planning.context_preview.take() else {
            return;
        };
        let current = self.tasks.iter().find(|task| task.id == self.task_id);
        let matches = prepared.turn.task_id == self.task_id
            && prepared.turn.partner == self.lead.id
            && prepared.turn.agent == self.planning.agent
            && prepared.turn.operation == self.planning.operation
            && prepared.selection == context::selection_identity(&self.sources.basket)
            && current.is_some_and(|task| {
                self.context_preparation_basis(task, self.sources.comparison_resume.as_deref())
                    == prepared.basis
            })
            && self.sources.comparison_basis == Some(prepared.basis);
        self.sources.comparison_refresh = false;
        if !matches {
            self.sources.clear_wire_request();
            self.sources.error = "Context turn or selection changed during local preparation; no comparison retained.".into();
            return;
        }
        if preview.comparison_scope.is_none()
            || self.sources.comparison_scope != preview.comparison_scope
        {
            self.sources.comparison.clear();
        }
        self.sources.comparison_scope = preview.comparison_scope;
        match self
            .sources
            .set_wire_request_for(prepared.turn, &preview.payload, prepared.selection)
        {
            Ok(()) => {
                if prepared.local_only {
                    self.sources.error = preview.blockers.join("\n");
                }
            }
            Err(error) => {
                self.sources.clear_wire_request();
                self.sources.error = if preview.blockers.is_empty() {
                    error
                } else {
                    preview.blockers.join("\n")
                };
            }
        }
    }

    fn context_preparation_basis(&self, task: &store::Snapshot, resume: Option<&str>) -> [u8; 32] {
        let replacement = crate::app::habits::replacement(
            &self.records.snapshot.habits,
            &task.approved_memories,
            &task.project,
            task.request_text(),
        );
        let mut hash = Sha256::new();
        hash.update(self.planning.context_basis(task, resume));
        hash.update(serde_json::to_vec(&replacement).expect("serializable instruction selection"));
        if let Some(report) = &self.repository {
            hash.update([1]);
            hash.update((std::sync::Arc::as_ptr(report) as usize).to_le_bytes());
            hash.update(report.repository.root.as_os_str().as_encoded_bytes());
        } else {
            hash.update([0]);
        }
        hash.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{Agent, Operation, Preview};

    fn app() -> App {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.task_id = 7;
        app.goal = "Compare selected context".into();
        app.revision = 1;
        app.saved_revision = 1;
        app.save_requested = false;
        app.editing = false;
        app.storage = crate::app::StorageState::Saved;
        app.planning.receive_preferences(None);
        app.planning.agent = Agent::Codex;
        app.planning.operation = Operation::Plan;
        app.planning.front = Some(app.attribution());
        app.tasks.push(app.snapshot("/workspace"));
        app
    }

    fn capture(id: i64) -> context::Excerpt {
        context::Excerpt {
            id,
            task_id: 7,
            root: "/workspace".into(),
            path: format!("資料{id}.rs"),
            start_line: 1,
            end_line: 1,
            source_sha256: [id as u8; 32],
            text: "한글 \"quoted\" \\\n".into(),
            available: true,
            origin_run: None,
        }
    }

    fn complete(app: &mut App, generation: u64) -> String {
        let Some(Request::Preview(input)) = app.planning.pending.take() else {
            panic!(
                "missing local Preview: source={}, planning={}, ready={}, refresh={}",
                app.sources.error,
                app.planning.notice,
                app.committed_input_ready(),
                app.sources.comparison_refresh
            );
        };
        assert!(input.work.is_none());
        assert!(input.local_only);
        assert!(!input.inspect_models && !input.inspect_sessions);
        let payload = serde_json::to_string_pretty(&serde_json::json!({
            "purpose": "unchanged", "task": {"request":input.task.request_text()},
            "selected_sources": {"captures":input.excerpts.iter().map(|capture| {
                serde_json::json!({"id":capture.id,"text":capture.text})
            }).collect::<Vec<_>>()},
            "approved_memories": [],
        }))
        .unwrap();
        app.provider_event(
            app.lead.id,
            Event::Preview(Preview {
                comparison_scope: crate::providers::ComparisonScope::from_identity(
                    &"fixed local connection",
                ),
                arrival: crate::providers::approval_arrival(),
                generation,
                task_id: 7,
                thread_id: None,
                destination: None,
                requires_connection_review: true,
                trust: Vec::new(),
                payload: payload.clone(),
                blockers: Vec::new(),
            }),
        );
        payload
    }

    #[test]
    fn command_from_saved_editor_survives_async_source_catalogue() {
        use crossterm::event::{Event as InputEvent, KeyCode, KeyEvent, KeyModifiers};
        use std::sync::Arc;

        use crate::workspace::repo;

        let mut app = app();
        let snapshot = Arc::new(repo::Snapshot {
            head: None,
            files: Default::default(),
            working_diff: Default::default(),
            complete: true,
            stable: true,
            warnings: Vec::new(),
        });
        app.repository = Some(Arc::new(repo::Report {
            repository: repo::RepositoryIdentity {
                root: "/workspace".into(),
                git_root: Some("/workspace".into()),
                git_dir: Some("/workspace/.git".into()),
                storage_root: None,
            },
            baseline: Arc::clone(&snapshot),
            current: snapshot,
            changes: Vec::new(),
            initial: true,
        }));
        app.sources.basket = vec![capture(91), capture(92)];
        app.open_editor(store::EditorField::Request);
        app.input = "Keep this unsent 한글 draft".into();
        app.cursor = "Keep this ".len();
        let draft = (app.input.clone(), app.cursor);
        app.tasks[0] = app.snapshot("/workspace");
        app.saved_revision = app.revision;
        app.save_requested = false;
        app.storage = crate::app::StorageState::Saved;

        app.update(InputEvent::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        )));
        for character in "context-compare".chars() {
            app.update(InputEvent::Key(KeyEvent::new(
                KeyCode::Char(character),
                KeyModifiers::NONE,
            )));
        }
        app.update(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(app.sources.mode == sources::Mode::Comparison);
        assert!(app.sources.comparison_refresh);
        assert!(app.sources.source_busy);
        assert!(matches!(
            app.sources.pending.take(),
            Some(context::Request {
                operation: context::Operation::Catalogue { .. },
                ..
            })
        ));
        assert!(app.planning.pending.is_none());

        app.sources.event(
            7,
            context::Event {
                task_id: 7,
                result: Ok(context::Response::Catalogue(Vec::new())),
            },
        );
        // Navigation autosaves may settle independently of the source worker.
        app.tasks[0] = app.snapshot("/workspace");
        app.saved_revision = app.revision;
        app.save_requested = false;
        app.storage = crate::app::StorageState::Saved;
        assert!(app.settle_context_comparison());
        complete(&mut app, 1);
        assert!(app.sources.comparison.current.is_some());
        assert!(app.planning.pending.is_none() && app.planning.preview.is_none());
        assert!(app.planning.work.is_none() && app.planning.work_pending.is_none());
        assert!(!app.planning.send_after_prepare);
        let retained = app
            .drafts
            .iter()
            .find(|draft| draft.field == store::EditorField::Request)
            .expect("retained request");
        assert_eq!((&retained.text, retained.cursor), (&draft.0, draft.1));

        app.update(InputEvent::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(app.editing);
        assert_eq!((&app.input, app.cursor), (&draft.0, draft.1));
        assert!(app.sources.comparison.current.is_some());
        app.update(InputEvent::Key(KeyEvent::new(
            KeyCode::Delete,
            KeyModifiers::NONE,
        )));
        assert!(app.sources.comparison.current.is_none());
        app.update(InputEvent::Paste("x".into()));
        assert_eq!(app.input.len(), draft.0.len());
        assert_ne!(app.input, draft.0);
        assert!(app.sources.comparison.turn.is_none());
        assert!(!app.sources.comparison_refresh);
    }

    #[test]
    fn actual_editor_mutations_invalidate_but_caret_navigation_does_not() {
        use crossterm::event::{Event as InputEvent, KeyCode, KeyEvent, KeyModifiers};

        let key = |code| InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE));
        for mutation in [
            key(KeyCode::Backspace),
            key(KeyCode::Delete),
            key(KeyCode::Char('x')),
            InputEvent::Paste("한글".into()),
        ] {
            let mut app = app();
            app.open_editor(store::EditorField::Request);
            app.input = "same request".into();
            app.cursor = 4;
            app.tasks[0] = app.snapshot("/workspace");
            app.saved_revision = app.revision;
            app.save_requested = false;
            app.storage = crate::app::StorageState::Saved;
            app.sources.mode = sources::Mode::Comparison;
            app.sources.comparison_refresh = true;
            assert!(app.settle_context_comparison());
            complete(&mut app, 1);
            let turn = app.sources.comparison.turn.clone();
            app.update(key(KeyCode::Left));
            app.update(key(KeyCode::Right));
            assert_eq!(app.sources.comparison.turn, turn);
            assert!(app.sources.comparison.current.is_some());
            app.update(mutation);
            assert!(app.sources.comparison.current.is_none());
            assert!(app.sources.comparison.turn.is_none());
            assert!(!app.sources.comparison_refresh);
        }
    }

    #[test]
    fn local_full_turn_receipts_never_arm_dispatch_and_preserve_draft_caret() {
        let mut app = app();
        app.sources.basket = vec![capture(91), capture(92)];
        app.input = "same draft 한글".into();
        app.cursor = "same ".len();
        let draft = (app.input.clone(), app.cursor);
        app.sources.mode = sources::Mode::Comparison;
        app.sources.comparison_refresh = true;
        app.settle_context_comparison();
        assert!(app.sources.mode == sources::Mode::Comparison);
        assert!(app.planning.work.is_none() && app.planning.work_pending.is_none());
        let previous = complete(&mut app, 1);
        assert!(app.planning.preview.is_none());
        assert!(app.planning.pending.is_none());
        assert!(!app.planning.reviewed && !app.planning.affirmative);
        assert!(!app.planning.send_after_prepare);
        assert!(!app.planning.busy());
        assert_eq!((app.input.clone(), app.cursor), draft);
        let turn = app.sources.comparison.turn.clone().unwrap();
        // Use the ordinary removal review and durable acknowledgment.
        app.sources.inspect_chip(91, true, 7);
        let review = app.sources.confirmation.as_mut().unwrap();
        review.reviewed = true;
        review.destructive = true;
        app.sources.key(
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
            7,
            None,
        );
        assert_eq!(app.sources.basket.len(), 2);
        assert!(app.sources.comparison.previous.is_none());
        app.sources.pending_store.take();
        app.sources.stored(7, 7, Ok(vec![capture(92)]));
        assert!(app.sources.comparison_refresh);
        assert!(app.sources.comparison.current.is_none());
        assert!(app.settle_context_comparison());
        let current = complete(&mut app, 2);
        assert_eq!(app.sources.comparison.turn, Some(turn));
        let delta = app.sources.comparison.delta().unwrap();
        assert_eq!(delta.bytes.previous, previous.len());
        assert_eq!(delta.bytes.current, current.len());
        assert!(delta.bytes.delta() < 0);
        assert_eq!((app.input.clone(), app.cursor), draft);
        assert!(app.planning.pending.is_none() && app.planning.preview.is_none());
        assert!(app.planning.work.is_none() && app.planning.work_pending.is_none());
    }

    #[test]
    fn stale_selection_or_changed_model_refuses_a_late_local_receipt() {
        for changed_model in [false, true] {
            let mut app = app();
            app.sources.basket = vec![capture(91)];
            app.sources.mode = sources::Mode::Comparison;
            app.sources.comparison_refresh = true;
            app.settle_context_comparison();
            if changed_model {
                app.planning.selection.model = Some("different-model".into());
            } else {
                app.sources.basket[0].source_sha256 = [4; 32];
                app.sources.basket[0].text = "replacement bytes".into();
            }
            complete(&mut app, 1);
            assert!(app.sources.comparison.current.is_none());
            assert!(app.sources.comparison.previous.is_none());
            assert!(app.sources.wire.is_none());
            assert!(app.planning.pending.is_none() && app.planning.preview.is_none());
            assert!(!app.sources.comparison_refresh);
        }
    }

    #[test]
    fn revision_only_changes_keep_logical_turn_but_request_facts_do_not() {
        let state = State::new();
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.goal = "same facts".into();
        let basis = state.context_basis(&task, None);
        task.revision += 1;
        task.updated_at += 1;
        assert_eq!(state.context_basis(&task, None), basis);
        task.goal.push_str(" changed");
        assert_ne!(state.context_basis(&task, None), basis);
        task.goal = "same facts".into();
        assert_ne!(state.context_basis(&task, Some("resumed-session")), basis);
    }

    #[test]
    fn ordinary_unpin_and_purge_never_prepare_a_provider_or_require_a_saved_draft() {
        for purge in [false, true] {
            let mut app = app();
            app.planning = State::new();
            app.sources.basket = vec![capture(91)];
            app.sources.mode = sources::Mode::Basket;
            app.save_requested = true;
            app.sources
                .command(if purge { "purge" } else { "unpin" }, 7, None);
            let review = app.sources.confirmation.as_mut().unwrap();
            review.reviewed = true;
            review.destructive = true;
            app.sources.key(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Enter,
                    crossterm::event::KeyModifiers::NONE,
                ),
                7,
                None,
            );
            assert!(app.sources.pending_store.is_some());
            assert!(!app.sources.comparison_refresh);
            assert!(!app.settle_context_comparison());
            assert!(app.planning.pending.is_none());
            assert!(app.planning.work.is_none() && app.planning.work_pending.is_none());
            app.sources.pending_store.take();
            let rows = if purge {
                let mut retained = capture(91);
                retained.available = false;
                retained.text.clear();
                vec![retained]
            } else {
                Vec::new()
            };
            app.sources.stored(7, 7, Ok(rows));
            assert!(!app.sources.comparison_refresh && !app.settle_context_comparison());
            assert!(app.planning.pending.is_none());
        }
    }
}

#[cfg(test)]
mod correlation_tests {
    use super::*;

    #[test]
    fn a_previous_requests_preview_cannot_settle_a_new_local_preparation() {
        let mut state = State::new();
        state.task = 7;
        state.awaiting_preview = true;
        state.context_inspection_only = true;
        state.context_preview = Some(Prepared {
            turn: context::ComparisonTurn {
                task_id: 7,
                partner: 1,
                agent: crate::providers::Agent::Codex,
                operation: crate::providers::Operation::Plan,
                request: 2,
            },
            selection: [0; 32],
            basis: [0; 32],
            local_only: true,
            not_before_arrival: 20,
        });
        let preview = |arrival| {
            Event::Preview(crate::providers::Preview {
                comparison_scope: None,
                arrival,
                generation: 1,
                task_id: 7,
                thread_id: None,
                destination: None,
                requires_connection_review: true,
                trust: Vec::new(),
                payload: "{}".into(),
                blockers: Vec::new(),
            })
        };
        state.event(preview(19));
        assert!(state.awaiting_preview);
        assert!(state.context_inspection());
        assert!(state.context_preview.is_some());
        assert!(state.preview.is_none() && state.pending.is_none());
        state.event(preview(21));
        assert!(!state.awaiting_preview);
        assert!(!state.context_inspection());
        assert!(state.preview.is_none() && state.pending.is_none());
        assert!(!state.reviewed && !state.affirmative && !state.send_after_prepare);
    }
}

#[cfg(test)]
mod inspection_behavior_tests {
    use super::*;
    use crate::providers::{Agent, ConversationOwner, NativeConversation, Operation};
    use std::sync::Arc;

    #[test]
    fn inspection_preserves_retained_response_history_session_and_viewport() {
        let mut state = State::new();
        state.task = 7;
        state.agent = Agent::OpenAI;
        state.operation = Operation::Plan;
        state.thread_id = Some("retained-thread".into());
        state.submitted_request = Arc::from("previous authored request");
        state.notice = "retained response notice".into();
        state.observed_model = Some("previous-response-model".into());
        state.selection.model = Some("current-choice".into());
        state.generation = Some(55);
        state.scroll = 12;
        state.scroll_max = 30;
        state.saved_scroll = 8;
        state.response_scroll = 17;
        state.following = false;
        state.items.push(super::super::Item {
            thread_id: "retained-thread".into(),
            turn_id: "old-turn".into(),
            item_id: "old-item".into(),
            kind: crate::providers::ItemKind::Message,
            text: "retained output 한국어".into(),
            complete: true,
            request: Arc::from("previous authored request"),
            owner: None,
        });
        let history = Arc::new(NativeConversation {
            owner: ConversationOwner {
                task_id: 7,
                partner: 1,
                agent: Agent::OpenAI,
                operation: Operation::Plan,
                connection_scope: "private local scope".into(),
            },
            messages: Arc::from([]),
        });
        state
            .native_histories
            .insert((Agent::OpenAI, Operation::Plan), Arc::clone(&history));
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.goal = "current request facts".into();
        state
            .prepare_context_receipt(false, &task, &[], None)
            .unwrap();
        let Some(Request::Preview(input)) = state.pending.take() else {
            panic!("missing local preparation");
        };
        assert!(Arc::ptr_eq(input.conversation.as_ref().unwrap(), &history));
        assert!(input.work.is_none());
        state.context_inspection_only = true;
        state.event(Event::Preview(crate::providers::Preview {
            comparison_scope: None,
            arrival: crate::providers::approval_arrival(),
            generation: 56,
            task_id: 7,
            thread_id: None,
            destination: None,
            requires_connection_review: true,
            trust: Vec::new(),
            payload: "{}".into(),
            blockers: Vec::new(),
        }));
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].text, "retained output 한국어");
        assert_eq!(&*state.submitted_request, "previous authored request");
        assert_eq!(
            state.observed_model.as_deref(),
            Some("previous-response-model")
        );
        assert_eq!(state.notice, "retained response notice");
        assert_eq!(state.selection.model.as_deref(), Some("current-choice"));
        assert_eq!(state.thread_id.as_deref(), Some("retained-thread"));
        assert!(Arc::ptr_eq(
            state
                .native_histories
                .get(&(Agent::OpenAI, Operation::Plan))
                .unwrap(),
            &history
        ));
        assert_eq!(state.generation, Some(55));
        assert_eq!(
            (
                state.scroll,
                state.scroll_max,
                state.saved_scroll,
                state.response_scroll
            ),
            (12, 30, 8, 17)
        );
        assert!(!state.following);
        assert!(state.pending.is_none() && state.preview.is_none());
        assert!(!state.reviewed && !state.affirmative && !state.send_after_prepare);
    }
}
