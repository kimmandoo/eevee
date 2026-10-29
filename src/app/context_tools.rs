//! Integration for reviewed context, receipts, catalogs and artifact inspection.
use super::*;
use crate::workspace::context;
use crossterm::event::{KeyCode, KeyEvent};

impl App {
    pub(super) fn context_tool_command(&mut self, input: &str) -> bool {
        let (name, argument) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
        if self.tool_plan_command(input) {
            return true;
        }
        if name == "remember-source" {
            let selected_basket = self.overlay == Some(Overlay::Sources)
                && self.sources.mode == sources::Mode::Basket;
            if !selected_basket {
                if self.editing {
                    self.retain_action_editor();
                }
                self.open_context_sources();
                self.sources.show_basket();
                self.sources.memory_notice = Some(crate::messages::Message::SourceMemorySelect);
                return true;
            }
            let action = self
                .sources
                .command(name, self.task_id, self.repository.as_ref());
            return self.apply_source_action(action);
        }
        if name == "storage-retry" {
            self.planning.receipts.retry();
            for state in self.cooperation.parked.values_mut() {
                state.receipts.retry();
            }
            return false;
        }
        let context_action = matches!(
            name,
            "optimize"
                | "compact"
                | "context-apply"
                | "context-cancel"
                | "context-clear"
                | "context-cache-retry"
                | "context-cache-discard"
                | "context-cache-purge"
        );
        if !context_action
            && !matches!(
                name,
                "receipts"
                    | "tools"
                    | "tool-setup"
                    | "tool-clear"
                    | "artifacts"
                    | "artifact-search"
                    | "artifact-locations"
            )
        {
            return false;
        }
        if self.editing {
            self.retain_action_editor();
        }
        if context_action {
            let task = self
                .tasks
                .iter()
                .find(|task| task.id == self.task_id)
                .cloned()
                .unwrap_or_else(|| store::Snapshot::empty(self.cwd.clone()));
            self.sources.configure_optimizer(&task);
            self.sources.set_optimizer_runs(&self.execution.history);
            self.open_context_sources();
            if name == "compact" && self.execution.history_pending(self.task_id) {
                self.sources.error = "Wait for run history to load before compacting; resolve journal storage failures explicitly.".into();
                return true;
            }
            let action = self
                .sources
                .command(input, self.task_id, self.repository.as_ref());
            return self.apply_source_action(action);
        }
        if name == "receipts" {
            self.planning.receipts.open(self.task_id, self.lead.id);
            self.open_overlay(Overlay::Receipts);
            return true;
        }
        if self.tool_catalog.command(
            name,
            argument,
            &self.cwd,
            self.planning.agent,
            self.task_id,
            self.lead.id,
        ) {
            self.open_overlay(Overlay::ToolCatalog);
            return true;
        }
        if self.artifacts.command(name, self.task_id) {
            self.open_overlay(Overlay::Artifacts);
            return true;
        }
        false
    }

    fn source_memory_owner_current(&self) -> bool {
        self.task_id > 0
            && !self.quit
            && !self.quit_after_save
            && !self.reset_after_save
            && !self.transitioning
            && matches!(self.storage, StorageState::Saved)
            && self.saved_revision == self.revision
            && !self.save_requested
            && self.save_inflight.is_none()
            && self.tasks.iter().any(|task| {
                task.id == self.task_id
                    && task.revision == self.revision
                    && task.project == self.cwd
                    && task.lead_partner == self.lead.id
            })
            && self.memory.task_id == self.task_id
            && self.memory.project == self.cwd
    }

    pub(super) fn remember_source(&mut self, id: i64) {
        use crate::messages::Message;
        self.sources.cancel_memory_adoption();
        if self.overlay != Some(Overlay::Sources)
            || self.width < 60
            || self.height < 18
            || self.confirmation.is_some()
            || !self.source_memory_owner_current()
        {
            self.sources.memory_notice = Some(Message::SourceMemoryOwner);
            return;
        }
        if self.memory_loading
            || self.memory_request.is_some()
            || self.memory.busy()
            || self.memory.candidate.is_some()
            || !self.memory.draft.is_empty()
        {
            self.sources.memory_notice = Some(Message::SourceMemoryBusy);
            return;
        }
        self.sources.begin_memory_adoption(
            id,
            self.task_id,
            self.attribution(),
            self.revision,
            self.repository.as_ref(),
        );
    }

    /// Resolve the worker's policy/freshness observation before entering the
    /// existing local-memory review. This never approves, selects or dispatches.
    pub fn context_source_event(&mut self, event: context::Event) {
        use crate::{
            messages::Message,
            workspace::memory::{Provenance, SourceKind},
        };
        let Some(event) = self.receive_impact(event) else {
            return;
        };
        if event.task_id != self.task_id {
            self.sources.cancel_memory_adoption();
        }
        self.sources.event(self.task_id, event);
        let Some(intent) = self.sources.take_ready_memory_adoption() else {
            return;
        };
        if self.overlay != Some(Overlay::Sources)
            || self.width < 60
            || self.height < 18
            || self.confirmation.is_some()
            || !self.source_memory_owner_current()
            || intent.capture.task_id != self.task_id
            || intent.owner != self.attribution()
            || intent.revision != self.revision
            || !self.sources.memory_adoption_matches(&intent)
            || self.repo_loading
            || self.repo_refresh
            || self
                .repository
                .as_ref()
                .is_none_or(|report| !std::sync::Arc::ptr_eq(report, &intent.report))
        {
            self.sources.memory_notice = Some(Message::SourceMemoryOwner);
            return;
        }
        if self.memory_loading
            || self.memory_request.is_some()
            || self.memory.busy()
            || self.memory.candidate.is_some()
            || !self.memory.draft.is_empty()
        {
            self.sources.memory_notice = Some(Message::SourceMemoryBusy);
            return;
        }
        let provenance = Provenance {
            kind: SourceKind::UserAuthored,
            reference: format!("task:{}/source:{}", self.task_id, intent.capture.id),
            task_id: self.task_id,
        };
        match self.memory.propose(&intent.capture.text, provenance, false) {
            Ok(()) => {
                self.open_overlay(Overlay::Memory);
                self.sources.memory_notice = Some(Message::SourceMemoryProposed);
                self.changed_task();
            }
            Err(error) => {
                self.sources.memory_notice = Some(Message::SourceMemoryUnavailable);
                self.sources.error = error;
            }
        }
    }

    pub(super) fn context_tool_key(&mut self, key: KeyEvent) -> bool {
        if self.width < 60 || self.height < 18 {
            if key.code == KeyCode::Esc {
                self.close_overlay();
            }
            return true;
        }
        let close = match self.overlay {
            Some(Overlay::Receipts) => !self.planning.receipts.key(key),
            Some(Overlay::Artifacts) => {
                self.artifacts.key(key);
                !self.artifacts.visible()
            }
            Some(Overlay::ToolCatalog) => {
                matches!(self.tool_catalog.key(key), tool_catalog::Action::Close)
            }
            _ => return false,
        };
        if close {
            self.close_overlay();
        }
        true
    }

    /// Freeze only the committed task's selected local state, never UI drafts.
    pub(super) fn prepare_context_tools(&mut self, task: &store::Snapshot) -> Result<(), String> {
        self.sources.configure_optimizer(task);
        self.planning.context_plan = self.sources.applied_plan().cloned();
        if self.sources.screenshot_store_busy || self.sources.screenshot_storage_failed() {
            return Err("Screenshot storage has not acknowledged the exact selection; settle or explicitly retry it before preparing a request".into());
        }
        self.planning.screenshot = self
            .sources
            .screenshot
            .as_ref()
            .filter(|image| {
                image.capture.task_id == task.id
                    && image.capture.partner == task.lead_partner
                    && image.capture.root == task.project
            })
            .cloned();
        self.planning.tool_catalog =
            self.tool_catalog
                .selection(self.planning.agent, task.id, task.lead_partner)?;
        self.planning.behavior_profile = if self.profile_skip.contains(&task.lead_partner) {
            None
        } else {
            self.form_profiles.selected(&task.project, task.form)?
        };
        Ok(())
    }

    pub fn submit_context_tool_stores(&mut self, worker: &store::Worker) -> usize {
        let mut accepted = usize::from(self.artifacts.pump(worker));
        if !self.retained.inflight && worker.try_retained(&mut self.retained.pending) {
            self.retained.inflight = true;
            accepted += 1;
        }
        if !self.tool_catalog.inflight && worker.try_tool_catalog(&mut self.tool_catalog.pending) {
            self.tool_catalog.inflight = true;
            accepted += 1;
        }
        if self.sources.context_cache_retryable()
            && worker.try_context_cache(&mut self.sources.context_cache_request)
        {
            accepted += 1;
        }
        accepted
    }

    pub(super) fn receipt_store_event(
        &mut self,
        request: store::receipts::Request,
        result: Result<store::receipts::Response, String>,
    ) {
        if let store::receipts::Operation::Put(outgoing) = &request.operation
            && let Some(child) = self.delegation.children.values_mut().find(|child| {
                child
                    .state
                    .receipt_claim
                    .as_ref()
                    .is_some_and(|claim| claim.request_id == outgoing.record.request_id)
            })
        {
            let refusal = if !outgoing.record.submitted {
                result.as_ref().err().filter(|error| {
                    error.starts_with("Outgoing receipt capacity reached;")
                        || error.starts_with("Local storage quota exceeded (live SQLite + retained output + committed reservations);")
                }).cloned()
            } else {
                None
            };
            child.state.receipts.event(request, result);
            if let Some(error) = refusal {
                child.refuse_reservation(error);
            }
            child.state.settle_work();
            return;
        }
        if let Some(state) = self.planning_state_mut(request.partner) {
            state.receipts.event(request, result);
            state.settle_work();
        }
    }

    pub(super) fn receipt_sources_purged(&mut self, task_id: i64) {
        self.retained.invalidate_task(task_id);
        self.planning.receipts.source_purged(task_id);
        for state in self.cooperation.parked.values_mut() {
            state.receipts.source_purged(task_id);
        }
    }
}
