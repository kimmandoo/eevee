//! Ephemeral installation reviews; neither navigation nor restoration carries consent.
use super::*;
use crate::messages::Message;
use crate::platform::distribution::{LifecycleReview, Operation};

#[derive(Default)]
pub struct State {
    pub operation: Option<Operation>,
    pub review: Option<LifecycleReview>,
    pub check_requested: bool,
    pub apply_requested: bool,
    pub choice: bool,
    pub visible: bool,
    pub scroll: u16,
    pub scroll_max: u16,
    pub page_rows: u16,
    pub seen_rows: usize,
    pub scope_rows: usize,
    pub scope_width: u16,
    pub scope: String,
    pub message: Option<Message>,
    pub error: String,
    owner: (i64, i64),
    revision: u64,
}

impl State {
    pub fn invalidate(&mut self) {
        self.visible = false;
        self.choice = false;
        self.apply_requested = false;
        self.seen_rows = 0;
        self.scroll = 0;
    }

    pub fn leave(&mut self) {
        self.invalidate();
        self.check_requested = false;
    }
}

impl App {
    /// Installation changes require settled local ownership, not cancellation by implication.
    pub fn lifecycle_ready(&self) -> bool {
        self.authentication_ready()
            && !self.draft_dirty()
            && !self.planning_any_unknown_effects()
            && !self.execution_any_unknown()
            && !self.planning_any_journal_failed()
            && !self.execution_any_journal_failed()
            && !self.planning_storage_failed()
            && !self.agents.busy()
            && !self.moves.persistence_pending()
            && !self.records.persistence_pending()
            && !self.memory.saving
            && self.memory.pending.is_none()
            && self.memory_request.is_none()
            && !self.memory_loading
            && self.party_request.is_none()
            && !self.party_inflight
            && !self.images_busy
            && self.images_pending.is_none()
            && self.pending_request.is_none()
            && self.pending_command.is_none()
            && self.confirmation.is_none()
            && !self.quit
            && !self.quit_after_save
            && !self.reset_after_save
    }

    pub(super) fn lifecycle_command(&mut self, operation: Operation) -> bool {
        if self.editing {
            self.retain_action_editor();
        }
        self.command_popup = None;
        self.open_overlay(Overlay::Updates);
        self.updates = State {
            operation: Some(operation),
            check_requested: true,
            owner: (self.task_id, self.lead.id),
            message: Some(Message::LifecycleWaiting),
            ..State::default()
        };
        true
    }

    pub fn take_lifecycle_check(&mut self) -> Option<Operation> {
        if !self.updates.check_requested
            || self.overlay != Some(Overlay::Updates)
            || self.command_popup.is_some()
            || self.width < 60
            || self.height < 18
            || !self.lifecycle_ready()
        {
            return None;
        }
        let owner = (self.task_id, self.lead.id);
        // An initial draft save can allocate its first task while a read-only check waits.
        if self.updates.owner != owner && self.updates.owner != (0, self.lead.id) {
            self.updates.leave();
            self.updates.message = Some(Message::LifecycleExpired);
            return None;
        }
        self.updates.owner = owner;
        self.updates.check_requested = false;
        self.updates.operation
    }

    pub fn lifecycle_prepared(&mut self, result: Result<LifecycleReview, String>) {
        self.updates.invalidate();
        self.updates.review = None;
        self.updates.scope.clear();
        self.updates.scope_rows = 0;
        self.updates.scope_width = 0;
        self.updates.error.clear();
        self.updates.message = None;
        self.updates.owner = (self.task_id, self.lead.id);
        self.updates.revision = self.revision;
        match result {
            Ok(review) => self.updates.review = Some(review),
            Err(error) => {
                self.updates.error = crate::workspace::privacy::redact(&error).into_owned();
                self.updates.message = Some(Message::LifecycleUnavailable);
            }
        }
    }

    pub fn take_lifecycle_apply(&mut self) -> Option<LifecycleReview> {
        if !std::mem::take(&mut self.updates.apply_requested) {
            return None;
        }
        if self.overlay != Some(Overlay::Updates)
            || self.command_popup.is_some()
            || !self.updates.visible
            || !self.updates.choice
            || self.width < 60
            || self.height < 18
            || self.updates.owner != (self.task_id, self.lead.id)
            || self.updates.revision != self.revision
            || !self.lifecycle_ready()
        {
            self.updates.invalidate();
            self.updates.review = None;
            self.updates.scope.clear();
            self.updates.scope_rows = 0;
            self.updates.message = Some(Message::LifecycleExpired);
            return None;
        }
        self.updates.invalidate();
        self.updates.review.take().filter(|review| review.can_apply)
    }

    pub(super) fn lifecycle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press || !key.modifiers.is_empty() {
            return false;
        }
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('n')) {
            self.updates.leave();
            self.close_overlay();
            return true;
        }
        if key.code == KeyCode::Char('r') {
            if let Some(operation) = self.updates.operation {
                return self.lifecycle_command(operation);
            }
            return false;
        }
        if self.width < 60 || self.height < 18 {
            self.updates.invalidate();
            return false;
        }
        match key.code {
            KeyCode::Up => self.updates.scroll = self.updates.scroll.saturating_sub(1),
            KeyCode::Down => self.updates.scroll = self.updates.scroll.saturating_add(1),
            KeyCode::PageUp => {
                self.updates.scroll = self
                    .updates
                    .scroll
                    .saturating_sub(self.updates.page_rows.max(1))
            }
            KeyCode::PageDown => {
                self.updates.scroll = self
                    .updates
                    .scroll
                    .saturating_add(self.updates.page_rows.max(1))
            }
            KeyCode::Home => self.updates.scroll = 0,
            KeyCode::End => self.updates.scroll = self.updates.scroll_max,
            KeyCode::Left => self.updates.choice = false,
            KeyCode::Right | KeyCode::Tab => {
                if self.updates.visible && self.updates.review.as_ref().is_some_and(|r| r.can_apply)
                {
                    self.updates.choice = !self.updates.choice;
                }
            }
            KeyCode::Char('y') if self.updates.visible => {
                self.updates.choice = true;
                self.updates.apply_requested =
                    self.updates.review.as_ref().is_some_and(|r| r.can_apply);
            }
            KeyCode::Enter if self.updates.choice && self.updates.visible => {
                self.updates.apply_requested =
                    self.updates.review.as_ref().is_some_and(|r| r.can_apply);
            }
            KeyCode::Enter => {
                self.updates.leave();
                self.close_overlay();
            }
            _ => return false,
        }
        self.updates.scroll = self.updates.scroll.min(self.updates.scroll_max);
        true
    }

    pub fn lifecycle_failed(&mut self, error: String) {
        self.lifecycle_prepared(Err(error));
        self.updates.message = Some(Message::LifecycleFailed);
    }
}
