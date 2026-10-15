//! Key dispatch: event update loop, per-surface keys, quit and confirmation.
use super::*;

impl App {
    /// Returns whether visible state changed. The event loop otherwise stays asleep.
    pub fn update(&mut self, event: Event) -> bool {
        self.update_at(event, std::time::Instant::now())
    }

    /// Inject the event clock for deterministic autosave scheduling.
    pub fn update_at(&mut self, event: Event, now: std::time::Instant) -> bool {
        let resized = matches!(event, Event::Resize(_, _));
        let command_action = self.command_popup.is_some()
            && matches!(&event, Event::Key(key) if key.code == KeyCode::Enter);
        let revision = self.revision;
        let navigation = self.navigation();
        let input_state = (self.input.len(), self.cursor);
        let changed = match event {
            Event::Resize(width, height) => {
                if (width, height) == (self.width, self.height) {
                    return false;
                }
                self.width = width;
                self.confirmation_reviewed = false;
                self.height = height;
                self.agents.affirmative = false;
                if let Some(popup) = &mut self.command_popup {
                    popup.selected = None;
                }
                if let Some(confirmation) = &mut self.confirmation {
                    confirmation.destructive = false;
                }
                self.execution.affirmative = false;
                self.execution.confirmation_visible = false;
                self.agents.reviewed = false;
                self.images_reviewed = false;
                self.images_selected = 0;
                self.planning.invalidate_review();
                self.handoff.invalidate_review();
                self.sources.invalidate_review();
                true
            }
            Event::Paste(_) if self.reset_after_save || self.confirmation.is_some() => false,
            Event::Paste(text)
                if self.command_popup.is_some()
                    && self.editing
                    && self.confirmation.is_none()
                    && !self.transitioning
                    && !self.quit_after_save =>
            {
                self.command_popup = None;
                self.insert_input(&text)
            }
            Event::Paste(text) if self.overlay == Some(Overlay::Workflows) => {
                self.workflow_paste(&text)
            }
            Event::Paste(text) if self.overlay == Some(Overlay::Memory) => {
                self.memory_paste(&text, now)
            }
            Event::Paste(text) if self.overlay == Some(Overlay::Handoff) => {
                self.handoff.paste(&text)
            }
            Event::Paste(text) if self.overlay == Some(Overlay::Planning) => {
                self.planning.paste(&text)
            }
            Event::Paste(text)
                if self.overlay == Some(Overlay::Sources)
                    && self.sources.editor.is_some()
                    && self.confirmation.is_none()
                    && !self.transitioning =>
            {
                self.sources.paste(&text)
            }
            Event::Paste(text)
                if self.editing
                    && self.confirmation.is_none()
                    && !self.quit_after_save
                    && !self.transitioning =>
            {
                self.insert_input(&text)
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                // A notice answers the previous action only; the next key
                // starts from a clean footer.
                self.notice.clear();
                if self.confirmation.is_none() {
                    self.confirmation_reviewed = false;
                }
                self.key(key, now)
            }
            _ => false,
        };
        if changed
            && !resized
            && (!command_action || !self.planning.busy())
            && !self.transitioning
            && !self.quit
            && !self.quit_after_save
            && self.revision == revision
            && (navigation != self.navigation() || input_state != (self.input.len(), self.cursor))
            && (self.task_id > 0
                || !self.goal.is_empty()
                || self.draft_dirty()
                || self.save_requested
                || matches!(self.storage, StorageState::Saving))
        {
            self.changed_implicitly(now);
        }
        changed
    }

    pub(super) fn handoff_key(&mut self, key: KeyEvent) -> bool {
        if self.handoff.needs_saved_input(&key) && !self.authentication_ready() {
            self.handoff.notice = "Finish or retry task/context/journal saves and active work before exporting committed facts.".into();
            return true;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.request_quit();
            return true;
        }
        if key.code == KeyCode::Esc && !self.handoff.busy() && self.handoff.candidate.is_none() {
            self.close_overlay();
            return true;
        }
        let empty;
        let task = if let Some(task) = self.tasks.iter().find(|task| task.id == self.task_id) {
            task
        } else {
            empty = store::Snapshot::empty(self.cwd.clone());
            &empty
        };
        self.handoff.key(
            key,
            self.width >= 60 && self.height >= 18,
            task,
            &self.sources.basket,
            &[],
        )
    }

    pub(super) fn planning_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if key.kind == KeyEventKind::Repeat {
                return false;
            }
            if self.planning.busy() || self.planning.outcome_unknown() {
                self.planning.request_quit();
            } else {
                self.request_quit();
            }
            return true;
        }
        if key.code == KeyCode::Esc
            && !self.planning.busy()
            && self.planning.request.is_none()
            && !self.planning.editing
            && !self.planning.journal_mode
            && self.planning.inspection.is_none()
            && self.planning.picker.is_none()
            && self.planning.selection_change.is_none()
        {
            self.close_overlay();
            return true;
        }
        if self.planning.needs_saved_input(&key) {
            if let Err(error) = self.memory.selected_snapshot() {
                self.planning.notice = format!("Review selected memories first: {error}");
                return true;
            }
            if !matches!(self.storage, StorageState::Saved)
                || self.saved_revision != self.revision
                || self.save_requested
                || self.sources.busy()
                || self.sources.store_failed
                || self.execution.busy()
                || self.planning.journal_failed()
            {
                self.planning.notice = "Finish or retry task/context saves and local work before previewing or running.".into();
                return true;
            }
            if self.planning.preview.is_some()
                && !self.planning.matches_task(self.task_id, self.revision)
            {
                self.planning.cancel();
                self.planning.notice =
                    "Task changed since preview. Create a new preview; nothing was sent.".into();
                return true;
            }
        }
        let empty;
        let task = if let Some(task) = self.tasks.iter().find(|task| task.id == self.task_id) {
            task
        } else {
            empty = store::Snapshot::empty(self.cwd.clone());
            &empty
        };
        let operation = self.planning.operation;
        let choosing_locally = self.planning.picker.is_some()
            || self.planning.selection_change.is_some()
            || self.planning.switch_to.is_some();
        let changed = self.planning.key(
            key,
            self.width >= 60 && self.height >= 18,
            task,
            &self.sources.basket,
            self.repository.as_ref(),
        );
        if matches!(
            self.planning.pending,
            Some(crate::providers::Request::Run { .. })
        ) && !self.planning.inspect_models
            && !self.planning.inspect_sessions
        {
            self.planning.submitted_request = task.request_text().into();
            self.planning.failed_request = false;
            self.request_home(true);
        } else if choosing_locally
            && !self.planning.busy()
            && self.planning.selection_change.is_none()
            && self.planning.request.is_none()
        {
            // Model/effort and provider choices return to their invoking
            // surface, rather than leaving an empty planning drawer behind.
            self.close_overlay();
            self.notice.clone_from(&self.planning.notice);
        } else if self.planning.preview.is_none()
            && self.planning.request.is_none()
            && self.planning.picker.is_none()
            && self.planning.paths_editor.is_none()
            && self.planning.switch_to.is_none()
            && !self.planning.quit_review
            && !self.planning.journal_mode
            && self.planning.inspection.is_none()
            && key.code == KeyCode::Esc
        {
            self.request_home(false);
        }
        if operation != self.planning.operation {
            self.set_form(match self.planning.operation {
                crate::providers::Operation::Implement => 3,
                crate::providers::Operation::Review => 7,
                crate::providers::Operation::Plan => 4,
            });
            self.changed_task();
        }
        changed
    }

    pub(super) fn planning_command(&mut self, name: &str) -> bool {
        if self.planning.needs_saved_command(name) {
            if let Err(error) = self.memory.selected_snapshot() {
                self.planning.notice = format!("Review selected memories first: {error}");
                return true;
            }
            if !self.planning_input_ready() {
                self.planning.notice = "Wait for task/context/journal storage acknowledgement and local work before previewing. Nothing was sent.".into();
                return true;
            }
        }
        let empty;
        let task = if let Some(task) = self.tasks.iter().find(|task| task.id == self.task_id) {
            task
        } else {
            empty = store::Snapshot::empty(self.cwd.clone());
            &empty
        };
        let operation = self.planning.operation;
        let changed = self.planning.command(
            name,
            self.width >= 60 && self.height >= 18,
            task,
            &self.sources.basket,
            self.repository.as_ref(),
        );
        if operation != self.planning.operation {
            self.set_form(match self.planning.operation {
                crate::providers::Operation::Implement => 3,
                crate::providers::Operation::Review => 7,
                crate::providers::Operation::Plan => 4,
            });
            self.changed_task();
        }
        changed
    }

    pub(super) fn handoff_command(&mut self, name: &str) -> bool {
        if self.handoff.command_needs_saved_input(name) && !self.authentication_ready() {
            self.handoff.notice = "Finish or retry task/context/journal saves and active work before exporting committed facts.".into();
            return true;
        }
        let empty;
        let task = if let Some(task) = self.tasks.iter().find(|task| task.id == self.task_id) {
            task
        } else {
            empty = store::Snapshot::empty(self.cwd.clone());
            &empty
        };
        self.handoff.command(
            name,
            self.width >= 60 && self.height >= 18,
            task,
            &self.sources.basket,
            &[],
        )
    }

    pub fn authentication_ready(&self) -> bool {
        self.committed_input_ready() && !self.planning.busy()
    }

    pub(super) fn planning_input_ready(&self) -> bool {
        self.committed_input_ready() && !self.planning.busy_except_picker()
    }

    pub(super) fn committed_input_ready(&self) -> bool {
        matches!(self.storage, StorageState::Saved)
            && self.saved_revision == self.revision
            && !self.save_requested
            && !self.sources.busy()
            && !self.sources.store_failed
            && !self.execution.busy()
            && !self.planning.journal_failed()
            && !self.transitioning
            && self.management.is_none()
            && self.switch_task.is_none()
            && (!self.editing || self.composer_is_request())
    }

    /// A provider login/status check finished: a connected task provider
    /// with no cached catalogue fetches its models automatically.
    pub fn connection_returned(
        &mut self,
        profile: usize,
        outcome: crate::providers::auth::Outcome,
    ) {
        self.agents.auth_returned(profile, outcome);
        if outcome == crate::providers::auth::Outcome::Authenticated
            && profile == self.agents.active
            && !self.planning.catalogues.contains_key(&self.planning.agent)
        {
            self.planning.want_catalogue();
        }
    }

    /// Fetch provider metadata without requiring a saved task or draft.
    pub fn drive_catalogue(&mut self) -> bool {
        if !self.planning.catalogue_due()
            || self.execution.busy()
            || self.transitioning
            || self.management.is_some()
            || self.switch_task.is_some()
        {
            return false;
        }
        let empty;
        let task = if let Some(task) = self.tasks.iter().find(|task| task.id == self.task_id) {
            task
        } else {
            empty = store::Snapshot::empty(self.cwd.clone());
            &empty
        };
        self.planning.start_automatic_catalogue(task, &[], None);
        true
    }

    pub fn authentication_returned(&mut self) {
        self.planning.invalidate_authentication();
        self.repo_refresh = true;
    }

    pub fn settle_planning(&mut self) -> bool {
        if std::mem::take(&mut self.planning.failed_request) {
            if self.composer_is_request() && self.input.is_empty() {
                self.input = self.planning.submitted_request.to_string();
                self.cursor = self.input.len();
                self.changed_task();
                self.notice = "Request stopped before output; the original request is restored for editing and review.".into();
            } else {
                self.notice = "Request stopped before output; your follow-up is retained and the submitted request remains above.".into();
            }
            return true;
        }
        if self.drive_request() {
            return true;
        }
        // Only an actual consent/input surface opens a modal. Stream events
        // and provider completion never move the response away from home.
        if self.overlay.is_none()
            && (self.planning.preview.is_some() || self.planning.request.is_some())
        {
            if self.composer_is_request() {
                self.retain_editor();
                self.composer_return = Some(store::EditorField::Request);
            }
            self.open_overlay(Overlay::Planning);
            return true;
        }
        if self.overlay == Some(Overlay::Planning)
            && self.planning.preview.is_none()
            && self.planning.request.is_none()
            && self.planning.picker.is_none()
            && self.planning.paths_editor.is_none()
            && self.planning.switch_to.is_none()
            && self.planning.inspection.is_none()
            && !self.planning.quit_review
            && !self.planning.inspect_models
            && !self.planning.inspect_sessions
            && !self.planning.journal_mode
            && self.composer_return == Some(store::EditorField::Request)
        {
            self.request_home(false);
            self.notice.clone_from(&self.planning.notice);
            return true;
        }
        if let Some(document) = self.handoff.output.take() {
            self.planning.handoff = Some(document);
            self.planning.notice = "Portable handoff accepted as unverified context. It grants no tool permission; review a fresh provider request.".into();
            return true;
        }
        if let Some(decision) = self.handoff.decision.take() {
            self.planning.decision = Some(decision);
        }
        if std::mem::take(&mut self.planning.quit_ready) {
            if let Some(action) = self.execution.request_navigation(execution::Action::Quit) {
                self.apply_execution_action(action);
            } else if self.execution.visible() {
                self.open_overlay(Overlay::Execution);
            }
            return true;
        }
        if std::mem::take(&mut self.planning.edit_goal) {
            self.begin_edit();
            return true;
        }
        if let Some(decision) = self.planning.decision.take() {
            if decision.len() > 4096 {
                self.planning.notice =
                    "Decision exceeds the Next action limit; nothing was copied or truncated."
                        .into();
                return true;
            }
            if self
                .drafts
                .iter()
                .any(|draft| draft.field == store::EditorField::NextAction)
            {
                self.planning.notice = "Existing Next action draft retained. Save or discard it before selecting this decision again.".into();
                return true;
            }
            self.begin_edit_field(store::EditorField::NextAction);
            if self.editing {
                self.input.clear();
                self.cursor = 0;
                self.insert_input(&decision);
                self.notice = "User-selected provider planning, not verified execution. Review/edit, then Enter explicitly saves as Next action.".into();
            }
            return true;
        }
        false
    }

    pub(super) fn command_surface_editing(&self) -> bool {
        self.sources.editor.is_some()
            || self.execution.searching
            || self.planning.paths_editor.is_some()
            || self.planning.picker.is_some()
            || self.planning.custom_model.is_some()
            || self.planning.editing
            || self.workflow.editor.is_some()
            || self.memory.editing()
            || self.handoff.editing
    }

    pub(super) fn key(&mut self, key: KeyEvent, now: std::time::Instant) -> bool {
        if self.transitioning {
            if self.deletion_preview_pending && key.code == KeyCode::Esc {
                self.deletion_preview_pending = false;
                self.transitioning = false;
                self.management = None;
                return true;
            }
            return false;
        }
        if self.reset_after_save {
            if key.code == KeyCode::Esc && key.kind == KeyEventKind::Press {
                self.reset_after_save = false;
                if self.overlay.is_none() && self.composer_return.is_some() {
                    self.close_overlay();
                }
                self.notice = "Reset cancelled; current task and drafts retained.".into();
                return true;
            }
            return false;
        }
        if self.quit_after_save {
            if key.code == KeyCode::Esc {
                self.quit_after_save = false;
                return true;
            }
            return false;
        }
        if self.confirmation.is_some() {
            return self.confirmation_key(key);
        }
        if key.code == KeyCode::Char('c')
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && key.kind == KeyEventKind::Press
        {
            self.request_quit();
            return true;
        }
        if self.command_popup.is_some() {
            return self.command_popup_key(key);
        }
        // Backspace is the previous-form toggle on every non-text surface;
        // inside any text field it stays a plain delete. Held repeats are
        // consumed once so the toggle never flips twice from one keydown.
        if key.code == KeyCode::Backspace
            && key.modifiers.is_empty()
            && !self.editing
            && !self.command_surface_editing()
        {
            if key.kind == KeyEventKind::Press {
                self.previous_form_toggle();
            }
            return true;
        }
        if key.kind == KeyEventKind::Press
            && !self.command_surface_editing()
            && ((key.code == KeyCode::Char('/')
                && key.modifiers.is_empty()
                && (!self.editing
                    || matches!(
                        self.editor_field,
                        store::EditorField::Request
                            | store::EditorField::Goal
                            | store::EditorField::NextAction
                    )))
                || (key.code == KeyCode::Char('k') && key.modifiers == KeyModifiers::CONTROL))
        {
            self.open_commands();
            return true;
        }
        if key.kind == KeyEventKind::Repeat
            && (key.code == KeyCode::Enter
                || (key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('c')))
        {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.request_quit();
            return true;
        }
        match self.overlay {
            Some(Overlay::Workflows) => return self.workflow_key(key),
            Some(Overlay::Memory) => return self.memory_key(key, now),
            Some(Overlay::Handoff) => return self.handoff_key(key),
            Some(Overlay::Planning) => return self.planning_key(key),
            Some(Overlay::Execution) => return self.execution_key(key),
            Some(Overlay::Sources) => return self.source_key(key),
            Some(Overlay::Agents) => {
                if key.code == KeyCode::Esc
                    && !self.agents.preview
                    && self.agents.auth_preview.is_none()
                    && !self.agents.auth_busy()
                    && self.agents.at_root()
                {
                    self.close_overlay();
                    return true;
                }
                if self.agents.auth_preview.is_some()
                    && ((self.agents.affirmative && key.code == KeyCode::Enter)
                        || (key.kind == KeyEventKind::Press
                            && key.modifiers.is_empty()
                            && key.code == KeyCode::Char('y')))
                    && !self.authentication_ready()
                {
                    self.agents.notice = "Finish or retry saves and active work before authentication. No login started.".into();
                    return true;
                }
                self.agents.active = self.planning.agent.index();
                let changed = self.agents.key(key, self.width >= 60 && self.height >= 18);
                self.use_selected_provider();
                return changed;
            }
            Some(Overlay::Images) => return self.images_key(key),
            _ => {}
        }
        if self.overlay.is_none()
            && self.response_on_home()
            && (matches!(key.code, KeyCode::PageUp | KeyCode::PageDown)
                || key.modifiers == KeyModifiers::ALT
                    && matches!(
                        key.code,
                        KeyCode::Up | KeyCode::Down | KeyCode::End | KeyCode::Home
                    ))
        {
            return self.planning.home_response_key(key.code);
        }
        if self.editing {
            return self.edit_key(key);
        }
        if key.code == KeyCode::Char('s') && key.modifiers == KeyModifiers::CONTROL {
            self.flush_save();
            return true;
        }
        if key.code == KeyCode::Esc && self.overlay.is_some() {
            self.close_overlay();
            return true;
        }
        if self.width < 60 || self.height < 18 {
            return false;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        if let Some(overlay) = self.overlay {
            return self.overlay_key(overlay, key.code);
        }
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                return self.cycle_focus(key.code == KeyCode::BackTab);
            }
            KeyCode::Enter => match self.effective_focus() {
                Pane::Companion => self.open_overlay(Overlay::Forms),
                Pane::Workspace => self.open_editor(store::EditorField::Request),
                Pane::Inspector => self.open_overlay(Overlay::Inspector),
            },
            KeyCode::Char(character) if safe_character(character) => {
                self.open_editor(store::EditorField::Request);
                if self.editing {
                    let mut bytes = [0; 4];
                    return self.insert_input(character.encode_utf8(&mut bytes));
                }
            }
            code => return self.scroll_key(code),
        }
        true
    }

    /// Hands hub requests (use for task, model, effort) to the planning drawer.
    pub(super) fn use_selected_provider(&mut self) {
        let name = if let Some(profile) = self.agents.use_for_task.take() {
            crate::providers::Agent::from_index(profile).map(crate::providers::Agent::command)
        } else {
            self.agents.pick.take()
        };
        if let Some(name) = name {
            self.open_overlay(Overlay::Planning);
            if self.overlay == Some(Overlay::Planning) {
                self.planning_command(name);
                if !self.planning.busy() {
                    self.close_overlay();
                    self.notice.clone_from(&self.planning.notice);
                }
            }
        }
    }

    pub(super) fn retain_editor(&mut self) {
        let saved_request = self.composer_is_request()
            && matches!(self.storage, StorageState::Saved)
            && self.revision == self.saved_revision
            && !self.save_requested;
        self.drafts.retain(|draft| draft.field != self.editor_field);
        if self.draft_dirty() || self.cursor != self.input.len() {
            self.drafts.push(store::Draft {
                field: self.editor_field,
                text: self.input.clone(),
                cursor: self.cursor,
            });
        }
        self.input.clear();
        self.finish_edit();
        if !saved_request {
            self.changed_task();
        }
    }

    pub(super) fn edit_key(&mut self, mut key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('s') && key.modifiers == KeyModifiers::CONTROL {
            if self.composer_is_request() {
                self.pending_request = None;
                self.changed_task();
                self.flush_save();
                self.notice = if self.planning.holds_committed_input() {
                    "Draft pending until the current request ends; not yet saved."
                } else {
                    "Local save requested; Enter reviews before sending."
                }
                .into();
                return true;
            }
            self.retain_editor();
            return true;
        }
        // Native VT/ConPTY terminals may encode Backspace as BS (Ctrl+H),
        // while others use DEL. Both delete one complete grapheme.
        if key.code == KeyCode::Char('h') && key.modifiers == KeyModifiers::CONTROL {
            key.code = KeyCode::Backspace;
            key.modifiers = KeyModifiers::NONE;
        }
        // A portable newline key for terminals that cannot distinguish
        // Shift+Enter from Enter. Never bind a pasted newline to local apply.
        if key.code == KeyCode::Char('n') && key.modifiers == KeyModifiers::CONTROL {
            return self.insert_input("\n");
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        self.normalize_cursor();
        if let Some(changed) = self.composer_key(key) {
            return changed;
        }
        match key.code {
            KeyCode::Esc => {
                if self.draft_dirty() {
                    self.confirmation = Some(Confirmation {
                        kind: ConfirmationKind::DiscardDraft,
                        destructive: false,
                    });
                } else {
                    self.drafts.retain(|draft| draft.field != self.editor_field);
                    self.input.clear();
                    self.finish_edit();
                }
                true
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.insert_input("\n")
            }
            KeyCode::Enter => self.apply_editor(),
            KeyCode::Left => self.move_cursor(self.previous_boundary()),
            KeyCode::Right => self.move_cursor(self.next_boundary()),
            KeyCode::Home => self.move_cursor(self.line_start()),
            KeyCode::End => self.move_cursor(self.line_end()),
            KeyCode::Up => self.move_vertical(false),
            KeyCode::Down => self.move_vertical(true),
            KeyCode::Backspace => {
                let start = self.previous_boundary();
                if start == self.cursor {
                    return false;
                }
                self.input.replace_range(start..self.cursor, "");
                self.cursor = start;
                self.normalize_cursor();
                self.preferred_column = None;
                true
            }
            KeyCode::Delete => {
                let end = self.next_boundary();
                if end == self.cursor {
                    return false;
                }
                self.input.replace_range(self.cursor..end, "");
                self.normalize_cursor();
                self.preferred_column = None;
                true
            }
            KeyCode::Char(character) if safe_character(character) && character != '\n' => {
                let mut bytes = [0; 4];
                self.insert_input(character.encode_utf8(&mut bytes))
            }
            _ => false,
        }
    }

    pub(super) fn request_quit(&mut self) {
        self.request_navigation(execution::Action::Quit);
    }

    pub(super) fn finish_request_quit(&mut self) {
        // A committed, clean task exits directly; only unsaved or failed
        // state needs the save/stay decision.
        if self.draft_dirty()
            || self.revision != self.saved_revision
            || self.sources.persistence_pending()
            || self.sources.store_failed
            || self.planning.journal_failed()
        {
            self.confirmation = Some(Confirmation {
                kind: ConfirmationKind::Quit,
                destructive: false,
            });
        } else {
            self.quit = true;
        }
    }

    pub fn deletion_pages(&self) -> usize {
        self.delete_preview
            .iter()
            .map(|target| {
                1 + target.excerpts.len()
                    + target.runs.len()
                    + target.provider_records.len()
                    + target.memories.len()
            })
            .sum::<usize>()
            .div_ceil(DELETE_PREVIEW_PAGE)
            .max(1)
    }

    pub(super) fn confirmation_key(&mut self, mut key: KeyEvent) -> bool {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            || key.kind != KeyEventKind::Press
        {
            return false;
        }
        if key.modifiers.is_empty() {
            match key.code {
                KeyCode::Char('n') => {
                    self.confirmation = None;
                    self.confirmation_reviewed = false;
                    return true;
                }
                KeyCode::Char('y')
                    if self.confirmation_reviewed && self.width >= 60 && self.height >= 18 =>
                {
                    self.confirmation
                        .as_mut()
                        .expect("active confirmation")
                        .destructive = true;
                    key.code = KeyCode::Enter;
                }
                _ => {}
            }
        }
        if key.code == KeyCode::Enter
            && self
                .confirmation
                .is_some_and(|confirmation| confirmation.destructive)
            && (!self.confirmation_reviewed || self.width < 60 || self.height < 18)
        {
            self.notice = "Review the visible confirmation before approving.".into();
            return true;
        }
        if self
            .confirmation
            .is_some_and(|confirmation| confirmation.kind == ConfirmationKind::DeleteTasks)
            && matches!(key.code, KeyCode::PageUp | KeyCode::PageDown)
        {
            return self.scroll_key(key.code);
        }
        if self
            .confirmation
            .is_some_and(|c| c.kind == ConfirmationKind::DeleteTasks)
            && matches!(key.code, KeyCode::Home | KeyCode::End)
        {
            self.delete_page = if key.code == KeyCode::End {
                (self.delete_page + 1).min(self.deletion_pages() - 1)
            } else {
                self.delete_page.saturating_sub(1)
            };
            self.surface_scroll[ScrollSurface::Preview.index()] = 0;
            self.confirmation
                .as_mut()
                .expect("active deletion review")
                .destructive = false;
            return true;
        }
        let confirmation = self.confirmation.as_mut().expect("active confirmation");
        match key.code {
            KeyCode::Esc => {
                self.confirmation = None;
                true
            }
            KeyCode::Tab | KeyCode::BackTab => {
                confirmation.destructive = !confirmation.destructive;
                true
            }
            KeyCode::Left | KeyCode::Up | KeyCode::Home => {
                let changed = confirmation.destructive;
                confirmation.destructive = false;
                changed
            }
            KeyCode::Right | KeyCode::Down | KeyCode::End => {
                let changed = !confirmation.destructive;
                confirmation.destructive = true;
                changed
            }
            KeyCode::Enter => {
                let confirmation = self.confirmation.take().expect("active confirmation");
                self.confirmation_reviewed = false;
                if confirmation.destructive {
                    match confirmation.kind {
                        ConfirmationKind::ResetTask => {
                            if self.editing {
                                let field = self.editor_field;
                                self.retain_editor();
                                self.composer_return = Some(field);
                            }
                            self.request_navigation(execution::Action::ResetTask);
                        }
                        ConfirmationKind::ReplaceWithInit => self.apply_init(),
                        ConfirmationKind::DiscardDraft => {
                            self.drafts.retain(|draft| draft.field != self.editor_field);
                            self.input.clear();
                            self.finish_edit();
                            self.changed_task();
                        }
                        ConfirmationKind::Quit => {
                            self.flush_save();
                            if matches!(
                                self.storage,
                                StorageState::Pending
                                    | StorageState::Saving
                                    | StorageState::Loading
                            ) || self.sources.persistence_pending()
                                || self.execution.busy()
                                || self.planning.busy()
                            {
                                self.quit_after_save = true;
                            } else {
                                self.quit = true;
                            }
                        }
                        ConfirmationKind::Complete => {
                            if !self.criteria.is_empty()
                                && self.criteria.iter().all(|criterion| criterion.met)
                            {
                                self.complete = true;
                                self.changed_task();
                                self.notice = "Task completed by your criterion review; no command evidence is implied.".into();
                            }
                        }
                        ConfirmationKind::Reopen(field) => {
                            self.complete = false;
                            self.changed_task();
                            self.open_editor(field);
                        }
                        ConfirmationKind::DeleteTasks => {
                            self.management = Some(store::Management::Delete {
                                ids: std::mem::take(&mut self.delete_targets),
                                expired_before: self.delete_expired_before.take(),
                            });
                            self.transitioning = true;
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }
}
