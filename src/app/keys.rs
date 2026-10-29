//! Key dispatch: event update loop, per-surface keys, quit and confirmation.
use super::*;
use crossterm::event::{MouseEvent, MouseEventKind};

impl App {
    /// Returns whether visible state changed. The event loop otherwise stays asleep.
    pub fn update(&mut self, event: Event) -> bool {
        self.update_at(event, std::time::Instant::now())
    }

    /// Inject the event clock for deterministic autosave scheduling.
    pub fn update_at(&mut self, event: Event, now: std::time::Instant) -> bool {
        self.sources
            .configure_screenshot_route(self.planning.agent, &self.planning.selection);
        let hide_carry_over = matches!(&event, Event::Paste(_))
            || matches!(&event, Event::Key(key) if key.kind != KeyEventKind::Release);
        let motion_hidden = (hide_carry_over || matches!(&event, Event::Resize(_, _)))
            && self.evolution.motion.take().is_some();
        let carry_over_hidden = hide_carry_over && self.carry_over.hide();
        let evolution_hidden = hide_carry_over
            && (self.evolution.popup_shown
                || self
                    .evolution
                    .transition
                    .is_some_and(|change| change.from == change.to))
            && self.evolution.transition.take().is_some();
        // A shown item notice fades on the next real key/paste, which still acts.
        let item_hidden = hide_carry_over
            && self.item_notice_visible().is_some()
            && self.item_notice.take().is_some();
        let resized = matches!(event, Event::Resize(_, _));
        let command_action = self.command_popup.is_some()
            && matches!(&event, Event::Key(key) if key.code == KeyCode::Enter);
        let revision = self.revision;
        let navigation = self.navigation();
        let input_state = (self.input.len(), self.cursor);
        let changed = match event {
            Event::Resize(width, height) => {
                if (width, height) == (self.width, self.height) {
                    return motion_hidden;
                }
                self.width = width;
                self.confirmation_reviewed = false;
                self.formations.invalidate();
                self.workflow.authoring.invalidate();
                self.updates.invalidate();
                self.external_editor.invalidate();
                self.tool_catalog.invalidate();
                self.tool_plans.invalidate();
                self.parked_drafts.invalidate();
                self.artifacts.review_visible = false;
                self.formations.apply_pending = false;
                self.height = height;
                self.agents.affirmative = false;
                if let Some(popup) = &mut self.command_popup {
                    popup.selected = None;
                }
                if let Some(confirmation) = &mut self.confirmation {
                    confirmation.destructive = false;
                }
                self.execution.invalidate_review_exposure();
                self.agents.reviewed = false;
                self.images_reviewed = false;
                self.images_selected = 0;
                self.planning.invalidate_review();
                for state in self.cooperation.parked.values_mut() {
                    state.invalidate_review();
                }
                for state in self.cooperation.executions.values_mut() {
                    state.invalidate_review_exposure();
                }
                if let Some(review) = self.cooperation_ui.release.as_mut() {
                    review.reviewed = false;
                    review.affirmative = false;
                }
                if let Some(review) = self.cooperation.cancel_review.as_mut() {
                    review.reviewed = false;
                    review.affirmative = false;
                }
                if let Some(review) = self.cooperation.journal_discard.as_mut() {
                    review.reviewed = false;
                    review.affirmative = false;
                }
                self.handoff.invalidate_review();
                self.sources.invalidate_review();
                self.moves.invalidate_review();
                self.records.invalidate_review();
                true
            }
            Event::Paste(_)
                if self.formation_inspector_active() || self.held_inspection_active() =>
            {
                false
            }
            Event::Paste(_)
                if self.cooperation.cancel_review.is_some()
                    || self.cooperation.journal_discard.is_some()
                    || self.cooperation_ui.release.is_some() =>
            {
                false
            }
            Event::Paste(_) if self.reset_after_save || self.confirmation.is_some() => false,
            Event::Paste(_)
                if matches!(
                    self.overlay,
                    Some(Overlay::Updates | Overlay::ExternalEditor)
                ) =>
            {
                false
            }
            Event::Paste(text) if self.overlay == Some(Overlay::Retained) => {
                self.retained.paste(&text);
                true
            }
            Event::Paste(text) if self.overlay == Some(Overlay::ToolCatalog) => {
                self.tool_catalog.paste(&text);
                true
            }
            Event::Paste(text) if self.overlay == Some(Overlay::Artifacts) => {
                self.artifacts.paste(&text)
            }
            Event::Paste(_) if self.overlay == Some(Overlay::Receipts) => false,
            Event::Paste(_) if self.overlay == Some(Overlay::ParkedDrafts) => false,
            Event::Paste(text) if self.overlay == Some(Overlay::ToolPlans) => {
                if let Some(editor) = &mut self.tool_plans.editor
                    && editor.len() + text.len() <= 64 * 1024
                {
                    editor.push_str(&text);
                }
                true
            }
            Event::Paste(_)
                if matches!(
                    self.overlay,
                    Some(
                        Overlay::EvolutionPreview
                            | Overlay::Journey
                            | Overlay::Bag
                            | Overlay::Usage
                            | Overlay::Rules
                            | Overlay::Mcp
                    )
                ) =>
            {
                false
            }
            Event::Paste(text) if self.nickname_editor.is_some() => self.nickname_paste(&text),
            Event::Paste(text)
                if self.overlay == Some(Overlay::Agents) && self.agents.connection_editing() =>
            {
                self.agents.paste_connection(&text)
            }
            Event::Paste(_) if self.overlay == Some(Overlay::Agents) => true,
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
            Event::Paste(text) if self.overlay == Some(Overlay::Moves) => self.move_paste(&text),
            Event::Paste(text) if self.overlay == Some(Overlay::Formations) => {
                self.formation_paste(&text)
            }
            Event::Paste(_) if self.overlay == Some(Overlay::Held) => true,
            Event::Paste(text) if self.overlay == Some(Overlay::Records) => {
                self.record_paste(&text)
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
            Event::Mouse(mouse) => self.response_mouse(mouse),
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                // A notice answers the previous action only; the next key
                // starts from a clean footer.
                self.notice.clear();
                if self.confirmation.is_none() {
                    self.confirmation_reviewed = false;
                }
                let was_specialist_editor = self.overlay == Some(Overlay::Workflows)
                    && (self.workflow.editor.is_some() || self.workflow.authoring.editing());
                let changed = self.key(key, now);
                let settled = self.settle_direct_action(key, was_specialist_editor);
                changed || settled
            }
            _ => false,
        };
        // Receipt invalidation belongs to text mutations, not this buffer's
        // length: retaining/restoring an unchanged draft also changes it.
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
        let presented = self.reconcile_permission_review();
        changed
            || presented
            || carry_over_hidden
            || evolution_hidden
            || motion_hidden
            || item_hidden
    }

    fn response_mouse(&mut self, mouse: MouseEvent) -> bool {
        if self.width < 60
            || self.height < 18
            || self.transitioning
            || self.reset_after_save
            || self.quit_after_save
            || self.confirmation.is_some()
            || self.command_popup.is_some()
            || self.nickname_editor.is_some()
            || self.cooperation_ui.release.is_some()
            || self.cooperation.cancel_review.is_some()
            || self.cooperation.journal_discard.is_some()
            || !matches!(self.overlay, None | Some(Overlay::Planning))
            || self.overlay.is_none()
                && (self.composer_menu.is_some()
                    || self.editing
                        && matches!(
                            self.editor_field,
                            store::EditorField::Criteria | store::EditorField::Command
                        ))
        {
            return false;
        }
        let lines = match mouse.kind {
            MouseEventKind::ScrollUp => -3,
            MouseEventKind::ScrollDown => 3,
            _ => return false,
        };
        self.planning.response_wheel(mouse.column, mouse.row, lines)
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
        let decisions = self.selected_profile_handoff();
        let empty;
        let task = if let Some(task) = self.tasks.iter().find(|task| task.id == self.task_id) {
            task
        } else {
            empty = store::Snapshot::empty(self.cwd.clone());
            &empty
        };
        let generation = self.handoff.ready_generation;
        let changed = self.handoff.key(
            key,
            self.width >= 60 && self.height >= 18,
            task,
            &self.sources.basket,
            &decisions,
        );
        if self.handoff.ready_generation != generation {
            self.record_handoff_ready();
        }
        changed
    }

    /// Shift+A, deliberately distinct from the one-shot `a`/`y` keys.
    pub(super) fn allow_all_key(key: KeyEvent) -> bool {
        key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('A')
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    }

    pub(super) fn planning_key(&mut self, key: KeyEvent) -> bool {
        if Self::allow_all_key(key)
            && !self.planning.auto_permission_review
            && self.planning.reviewed
            && self.planning.required_permission_review()
            && self.width >= 60
            && self.height >= 18
            && self
                .planning
                .owner()
                .is_some_and(|owner| owner.partner == self.lead.id)
        {
            return self.auto_approve_command("auto-approve");
        }
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if key.kind == KeyEventKind::Repeat {
                return false;
            }
            self.request_quit();
            return true;
        }
        if key.kind == KeyEventKind::Press
            && key.modifiers.is_empty()
            && key.code == KeyCode::Char('c')
            && self.width >= 60
            && self.height >= 18
            && !self.planning.quit_review
            && !self.planning.journal_mode
            && self.planning.picker.is_none()
            && self.planning.selection_change.is_none()
            && self.planning.paths_editor.is_none()
            && self.planning.custom_model.is_none()
            && self.planning.switch_to.is_none()
            && self.planning.preview.as_ref().is_some_and(|preview| {
                !preview.blockers.is_empty() && preview.task_id == self.task_id
            })
            && self
                .planning
                .owner()
                .is_none_or(|owner| owner.partner == self.lead.id)
        {
            // Recovery is navigation, not renewed send intent. Discard the
            // blocked preparation through its owned cancellation path; setup
            // mutations/login still wait for the ordinary saved-and-idle gate.
            let agent = self.planning.agent;
            let generation = self.planning.preview.as_ref().unwrap().generation;
            self.pending_request = None;
            if self.planning.cancel() {
                self.planning.connection_recovery = Some(generation);
                self.planning.failed_request = false;
                self.planning.inspection = Some(self.planning.submitted_request.to_string());
                self.planning.scroll = 0;
                self.planning.scroll_max = 0;
                self.open_overlay(Overlay::Agents);
                self.agents.active = agent.index();
                self.agents.begin_connection(agent);
                self.refresh_approvals();
            }
            return true;
        }
        if key.code == KeyCode::Esc
            && (self.planning.request.is_some() || self.planning.preview.is_some())
        {
            self.defer_approval();
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
        if self.planning.work.is_none() && self.planning.needs_saved_input(&key) {
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
        let task = self
            .planning
            .immutable_task()
            .cloned()
            .or_else(|| {
                self.tasks
                    .iter()
                    .find(|task| task.id == self.task_id)
                    .cloned()
            })
            .unwrap_or_else(|| store::Snapshot::empty(self.cwd.clone()));
        let choosing_locally = self.planning.picker.is_some()
            || self.planning.selection_change.is_some()
            || self.planning.switch_to.is_some();
        let changed = self.planning.key(
            key,
            self.width >= 60 && self.height >= 18,
            &task,
            &self.sources.basket,
            self.repository.as_ref(),
        );
        self.settle_provider_picker();
        self.refresh_approvals();
        if matches!(
            self.planning.pending,
            Some(crate::providers::Request::Answer { .. })
        ) {
            self.queue_approval_response_notice();
            self.close_overlay();
            return changed;
        }
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
        changed
    }

    pub(super) fn planning_command(&mut self, name: &str) -> bool {
        if name == "quit" {
            self.request_quit();
            return true;
        }
        if self.planning.preference_restore_error().is_some()
            && matches!(name, "new" | "resume" | "sessions" | "models-refresh")
        {
            self.planning.notice = self.planning.preference_restore_error().unwrap().to_owned();
            return true;
        }
        if self.planning.preferences_loading()
            && matches!(name, "new" | "resume" | "sessions" | "models-refresh")
        {
            if name == "models-refresh" {
                self.planning.want_catalogue();
            }
            self.planning.notice =
                "Restoring the last provider/model preference before connecting…".into();
            return true;
        }
        if matches!(name, "new" | "resume" | "sessions" | "models-refresh")
            && !self.planning.busy_except_picker()
            && let Some(reason) = self.work_capacity_reason(self.lead.id)
        {
            self.planning.notice = reason;
            return true;
        }
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
        let task = self
            .tasks
            .iter()
            .find(|task| task.id == self.task_id)
            .cloned()
            .unwrap_or_else(|| store::Snapshot::empty(self.cwd.clone()));
        if matches!(name, "new" | "resume")
            && let Err(error) = self.prepare_context_tools(&task)
        {
            self.planning.notice = error;
            return true;
        }
        let changed = self.planning.command(
            name,
            self.width >= 60 && self.height >= 18,
            &task,
            &self.sources.basket,
            self.repository.as_ref(),
        );
        self.stage_provider_work();
        self.refresh_approvals();
        if name == "decision" {
            self.workflow.pending_decision = self.planning.decision.clone();
        }
        changed
    }

    pub(super) fn handoff_command(&mut self, name: &str) -> bool {
        if self.handoff.command_needs_saved_input(name) && !self.authentication_ready() {
            self.handoff.notice = "Finish or retry task/context/journal saves and active work before exporting committed facts.".into();
            return true;
        }
        let decisions = self.selected_profile_handoff();
        let empty;
        let task = if let Some(task) = self.tasks.iter().find(|task| task.id == self.task_id) {
            task
        } else {
            empty = store::Snapshot::empty(self.cwd.clone());
            &empty
        };
        let generation = self.handoff.ready_generation;
        let changed = self.handoff.command(
            name,
            self.width >= 60 && self.height >= 18,
            task,
            &self.sources.basket,
            &decisions,
        );
        if self.handoff.ready_generation != generation {
            self.record_handoff_ready();
        }
        changed
    }

    pub fn authentication_ready(&self) -> bool {
        self.committed_input_ready() && !self.planning_any_busy() && !self.execution_any_pending()
    }

    pub(super) fn planning_input_ready(&self) -> bool {
        self.committed_input_ready()
            && !self.planning.busy_except_picker()
            && !self.tool_plans.busy()
    }

    pub(super) fn committed_input_ready(&self) -> bool {
        matches!(self.storage, StorageState::Saved)
            && self.saved_revision == self.revision
            && !self.save_requested
            && !self.sources.busy()
            && !self.sources.store_failed
            && !self.planning.journal_failed()
            && !self.recipe_export_busy()
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
        self.connection_settings_changed();
        if outcome == crate::providers::auth::Outcome::Authenticated
            && profile == self.agents.active
            && !self.planning.catalogues.contains_key(&self.planning.agent)
        {
            self.planning.want_catalogue();
        }
    }

    pub fn openai_connection_returned(
        &mut self,
        plan: &crate::providers::auth::Plan,
        generation: Option<u64>,
        credential: Option<crate::providers::auth::OpenAiCredential>,
        outcome: crate::providers::auth::Outcome,
    ) {
        let owner = (self.task_id, self.lead.id, self.planning.agent);
        let admitted = self.overlay == Some(Overlay::Agents) && self.authentication_ready();
        self.agents
            .finish_openai_login(plan, generation, credential, outcome, &owner, admitted);
        self.connection_settings_changed();
        if self.agents.use_for_task == Some(crate::providers::Agent::ChatGPT.index())
            && self.agents.pick == Some("model")
        {
            self.pending_openai_selection = Some(owner);
            self.settle_openai_selection();
        }
    }

    /// Successful local connection edits invalidate only their future route state.
    pub fn connection_settings_changed(&mut self) -> bool {
        let mut changed = false;
        while let Some(agent) = self.agents.take_connection_change() {
            self.planning.invalidate_connection(agent);
            self.tool_catalog.disconnected(agent);
            for state in self.cooperation.parked.values_mut() {
                state.invalidate_connection(agent);
            }
            changed = true;
        }
        changed
    }

    /// Fetch provider metadata without requiring a saved task or draft.
    pub fn drive_catalogue(&mut self) -> bool {
        if self.planning.preference_restore_error().is_some()
            || self.planning.preferences_loading()
            || !self.planning.catalogue_due()
            || self.execution.busy()
            || self.transitioning
            || self.management.is_some()
            || self.switch_task.is_some()
            || self.work_capacity_reason(self.lead.id).is_some()
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
        for state in self.cooperation.parked.values_mut() {
            state.invalidate_authentication();
        }
        self.repo_refresh = true;
    }

    fn settle_source_action(&mut self) -> bool {
        let Some((task, partner, action)) = self.pending_source_action else {
            return false;
        };
        if task != self.task_id
            || partner != self.lead.id
            || self.overlay != Some(Overlay::Sources)
            || self.confirmation.is_some()
            || self.has_focused_approval()
            || self.sources.confirmation.is_some()
            || self.sources.editor.is_some()
        {
            self.pending_source_action = None;
            return false;
        }
        if self.sources.store_failed {
            self.pending_source_action = None;
            return false;
        }
        if self.sources.busy() || self.width < 60 || self.height < 18 {
            return false;
        }
        if !self.sources.error.is_empty() {
            self.pending_source_action = None;
            return false;
        }
        self.pending_source_action = None;
        if let Some(reason) = self
            .sources
            .command_choices()
            .into_iter()
            .find(|choice| choice.command == action)
            .and_then(|choice| choice.reason)
        {
            self.sources.error = reason;
            return true;
        }
        let action = self
            .sources
            .command(action, self.task_id, self.repository.as_ref());
        self.apply_source_action(action)
    }

    pub fn settle_planning(&mut self) -> bool {
        if self.mcp.poll() {
            return true;
        }
        if self.settle_steering() {
            return true;
        }
        self.settle_berries();
        if self.settle_context_comparison() {
            return true;
        }
        let openai_selection = self.settle_openai_selection();
        let source_action = self.settle_source_action();
        let provider_picker = self.settle_provider_picker();
        if source_action || provider_picker || openai_selection {
            return true;
        }
        if let Some((task, partner)) = self.return_after_cancel {
            if task != self.task_id || partner != self.lead.id {
                self.return_after_cancel = None;
                self.notice = "Return to Eevee cancelled: the task or lead changed.".into();
                return true;
            }
            if !self.planning.busy() {
                self.return_after_cancel = None;
                if self.planning.unknown_effects() || self.planning.journal_failed() {
                    self.notice = "Return to Eevee stopped: inspect the provider outcome or retained journal first.".into();
                } else {
                    self.request_navigation(execution::Action::SelectForm(0));
                }
                return true;
            }
        }
        if std::mem::take(&mut self.planning.failed_request) {
            if let Some(owner) = self
                .planning
                .owner()
                .filter(|owner| owner.partner != self.lead.id)
                .cloned()
            {
                // The request belongs to the partner that sent it, not the one in front.
                self.restore_partner_request(&owner);
                return true;
            }
            if self.composer_is_request() && self.input.is_empty() {
                self.input = self.planning.submitted_request.to_string();
                self.cursor = self.input.len();
                self.changed_task();
                self.notice = format!(
                    "{} Original request restored; edit and press Enter to send again.",
                    self.planning.notice
                );
            } else {
                self.notice = format!("{} Your draft is retained.", self.planning.notice);
            }
            if self.planning.picker == Some(planning::Picker::Model)
                && self.overlay.is_none()
                && self.composer_is_request()
            {
                self.retain_action_editor();
                self.open_overlay(Overlay::Planning);
            }
            return true;
        }
        if self.drive_request() {
            return true;
        }
        if self.planning.send_after_prepare && self.planning.preview.is_some() {
            if self.width < 60
                || self.height < 18
                || self.planning.work.is_none()
                    && !self.planning.matches_task(self.task_id, self.revision)
            {
                self.planning.cancel();
                self.notice = "Draft changed before dispatch or the terminal is too small; the request is retained. Press Enter when ready.".into();
                return true;
            }
            if self.planning.work.is_none() && !self.committed_input_ready() {
                return false;
            }
            self.planning.send_after_prepare = false;
            self.refresh_approvals();
            let preview = self.planning.preview.as_ref().expect("prepared request");
            if !preview.blockers.is_empty() {
                let reason = format!(
                    "Request blocked: {} /approvals Details",
                    preview.blockers.join(" ")
                );
                self.planning.failed_request = true;
                self.planning.notice.clone_from(&reason);
                self.notice = reason;
                return true;
            }
            if !preview.requires_connection_review && self.planning.dispatch_prepared() {
                if let Some(task) = self.planning.immutable_task() {
                    self.planning.submitted_request = task.request_text().into();
                }
                self.planning.failed_request = false;
                let consumed = self.input == self.planning.submitted_request.as_ref();
                self.request_home(consumed);
                return true;
            }
        }
        if self.overlay == Some(Overlay::Planning)
            && self.planning.preview.is_none()
            && self.planning.request.is_none()
            && self.planning.picker.is_none()
            && self.planning.selection_change.is_none()
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
            self.workflow.pending_decision = None;
            self.planning.decision = Some(decision);
        }
        if std::mem::take(&mut self.planning.quit_ready) {
            self.request_quit();
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
            let chosen = self
                .workflow
                .pending_decision
                .take()
                .filter(|chosen| chosen == &decision);
            self.begin_edit_field(store::EditorField::NextAction);
            if self.editing {
                self.workflow.pending_decision = chosen;
                self.input.clear();
                self.cursor = 0;
                self.insert_input(&decision);
                self.notice = "User-selected provider planning, not verified execution. Review/edit, then Enter explicitly saves as Next action.".into();
            }
            return true;
        }
        self.reconcile_permission_review()
    }

    pub(super) fn command_surface_editing(&self) -> bool {
        self.composer_menu.is_some()
            || self.sources.editor.is_some()
            || self.execution.searching
            || self.planning.paths_editor.is_some()
            || self.planning.picker.is_some()
            || self.planning.custom_model.is_some()
            || self.planning.editing
            || self.workflow.editor.is_some()
            || self.workflow.todo_editor.is_some()
            || self.workflow.authoring.editing()
            || self.memory.editing()
            || self.moves.editing()
            || self.overlay == Some(Overlay::Formations) && self.formations.editing()
            || self.overlay == Some(Overlay::Artifacts) && self.artifacts.editor.is_some()
            || self.overlay == Some(Overlay::Retained)
                && (self.retained.editor.is_some() || self.retained.mode == retained::Mode::Search)
            || self.overlay == Some(Overlay::ToolCatalog)
                && matches!(
                    self.tool_catalog.mode,
                    tool_catalog::Mode::Search | tool_catalog::Mode::Path
                )
            || self.handoff.editing
            || self.agents.connection_editing()
    }

    pub(super) fn key(&mut self, key: KeyEvent, now: std::time::Instant) -> bool {
        if let Some(handled) = self.steer_choice_key(key) {
            return handled;
        }
        let scoped_inspector = self.formation_inspector_active() || self.held_inspection_active();
        if self.overlay == Some(Overlay::Agents) && !scoped_inspector {
            self.agents.connection_owner = Some((self.task_id, self.lead.id, self.planning.agent));
        }
        if self.journal_discard_key(key) || self.cancellation_key(key) || self.cooperation_key(key)
        {
            return true;
        }
        if self.overlay == Some(Overlay::Agents)
            && !scoped_inspector
            && self.agents.connection_editing()
        {
            if key.code == KeyCode::Char('c')
                && key.modifiers == KeyModifiers::CONTROL
                && key.kind == KeyEventKind::Press
            {
                self.request_quit();
                return true;
            }
            if self.agents.connection_needs_idle(&key) && !self.authentication_ready() {
                self.agents.notice =
                    "Finish or retry saves and active work before saving or forgetting a connection.".into();
                return true;
            }
            let changed = self.agents.key(key, self.width >= 60 && self.height >= 18);
            let connection_changed = self.connection_settings_changed();
            if !self.agents.connection_editing()
                && (key.code == KeyCode::Char('s') && key.modifiers == KeyModifiers::CONTROL
                    || connection_changed
                        && self.agents.use_for_task.is_none()
                        && self.agents.pick.is_none())
            {
                self.close_overlay();
                self.notice.clone_from(&self.agents.notice);
            } else {
                self.use_selected_provider();
            }
            return changed || connection_changed;
        }
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
        if self.formation_inspector_key(key) || self.held_inspection_key(key) {
            return true;
        }
        if self.command_popup.is_some() {
            return self.command_popup_key(key);
        }
        if self.nickname_editor.is_some() {
            return self.nickname_key(key);
        }
        if self.overlay == Some(Overlay::Planning) && self.planning.auto_permission_review {
            return self.planning_key(key);
        }
        if key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('a')
            && key.modifiers == KeyModifiers::ALT
            && self.approval_shortcut_available()
        {
            self.focus_approvals();
            return true;
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
            && key.code == KeyCode::Char('/')
            && key.modifiers.is_empty()
            && matches!(
                self.overlay,
                Some(Overlay::Artifacts | Overlay::ToolCatalog)
            )
        {
            return self.context_tool_key(key);
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
            Some(Overlay::Retained) => return self.retained_key(key),
            Some(Overlay::Bag) => return self.bag_key(key),
            Some(Overlay::Usage) => return self.usage_key(key),
            Some(Overlay::Rules) => return self.rules_key(key),
            Some(Overlay::Mcp) => return self.mcp_key(key),
            Some(Overlay::ParkedDrafts) => return self.parked_draft_key(key),
            Some(Overlay::ToolPlans) => return self.tool_plan_key(key),
            Some(Overlay::EvolutionPreview | Overlay::Journey) => {
                if key.code == KeyCode::Esc {
                    self.close_overlay();
                    return true;
                }
                return self.scroll_key(key.code);
            }
            Some(Overlay::Updates) => return self.lifecycle_key(key),
            Some(Overlay::ExternalEditor) => return self.external_editor_key(key),
            Some(Overlay::Artifacts | Overlay::ToolCatalog | Overlay::Receipts) => {
                return self.context_tool_key(key);
            }
            Some(Overlay::Formations) => return self.formation_key(key),
            Some(Overlay::Held) => return self.held_key(key),
            Some(Overlay::Moves) => return self.move_key(key),
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
            Some(Overlay::Records) => return self.record_key(key),
            _ => {}
        }
        if self.overlay.is_none()
            && self.response_on_home()
            && self.width >= 60
            && self.height >= 18
            && self.composer_menu.is_none()
            && !(self.editing
                && matches!(
                    self.editor_field,
                    store::EditorField::Criteria | store::EditorField::Command
                ))
        {
            if key.modifiers.is_empty() && matches!(key.code, KeyCode::PageUp | KeyCode::PageDown)
                || key.modifiers == KeyModifiers::ALT
                    && matches!(
                        key.code,
                        KeyCode::Up | KeyCode::Down | KeyCode::End | KeyCode::Home
                    )
            {
                return self.planning.home_response_key(key.code);
            }
            if !self.editing
                && self.effective_focus() == Pane::Workspace
                && key.modifiers.is_empty()
            {
                match key.code {
                    KeyCode::Up => return self.planning.scroll_response_lines(-1),
                    KeyCode::Down => return self.planning.scroll_response_lines(1),
                    KeyCode::Home | KeyCode::End => {
                        return self.planning.home_response_key(key.code);
                    }
                    _ => {}
                }
            }
        }
        if self.overlay.is_none()
            && !self.editing
            && self.effective_focus() == Pane::Workspace
            && key.kind == KeyEventKind::Press
            && key.modifiers.is_empty()
            && key.code == KeyCode::Enter
            && self.focus_own_approval()
        {
            return true;
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
            KeyCode::Char('c') if self.effective_focus() == Pane::Companion => {
                self.reopen_carry_over();
            }
            KeyCode::Char('n') if self.effective_focus() == Pane::Companion => {
                self.open_nickname(self.lead().id);
            }
            KeyCode::Char('m') if self.effective_focus() == Pane::Workspace => {
                self.open_moves();
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

    fn settle_openai_selection(&mut self) -> bool {
        let Some(owner) = self.pending_openai_selection else {
            return false;
        };
        if owner != (self.task_id, self.lead.id, self.planning.agent)
            || self.overlay != Some(Overlay::Agents)
            || self.agents.page != super::agents::Page::Account
            || self.agents.use_for_task != Some(crate::providers::Agent::ChatGPT.index())
            || self.agents.pick != Some("model")
        {
            self.pending_openai_selection = None;
            self.agents.use_for_task = None;
            self.agents.pick = None;
            return true;
        }
        if self.width < 60 || self.height < 18 || !self.authentication_ready() {
            return false;
        }
        self.pending_openai_selection = None;
        self.use_selected_provider();
        true
    }

    /// Resume only the explicit task/partner/route selection that requested it.
    fn settle_provider_picker(&mut self) -> bool {
        let Some((task, partner, source, target, picker)) = self.pending_provider_picker else {
            return false;
        };
        if task != self.task_id
            || partner != self.lead.id
            || self.overlay != Some(Overlay::Planning)
            || self.planning.switch_to.is_some_and(|route| route != target)
            || self.planning.agent != source && self.planning.agent != target
        {
            self.pending_provider_picker = None;
            return false;
        }
        if self.planning.switch_to == Some(target) {
            return false;
        }
        if self.planning.agent != target {
            self.pending_provider_picker = None;
            return false;
        }
        if self.planning.busy_except_picker()
            || self.confirmation.is_some()
            || self.has_focused_approval()
        {
            return false;
        }
        self.pending_provider_picker = None;
        self.planning_command(picker)
    }

    /// Drain paired provider/picker intent without dropping a mandatory switch review.
    pub(super) fn use_selected_provider(&mut self) {
        if self.pending_openai_selection.is_some() {
            self.settle_openai_selection();
            return;
        }
        let profile = self.agents.use_for_task.take();
        let target = profile.and_then(crate::providers::Agent::from_index);
        let picker = self.agents.pick.take();
        if target.is_none() && picker.is_none() {
            return;
        }
        self.pending_provider_picker = None;
        self.open_overlay(Overlay::Planning);
        if self.overlay != Some(Overlay::Planning) {
            return;
        }
        let source = self.planning.agent;
        if let Some(target) = target {
            self.planning_command(target.command());
        }
        if let Some(picker) = picker {
            self.pending_provider_picker = Some((
                self.task_id,
                self.lead.id,
                source,
                target.unwrap_or(source),
                picker,
            ));
            self.settle_provider_picker();
        }
        self.overlay_history
            .retain(|frame| frame.overlay != Overlay::Agents);
        if !self.planning.busy() {
            self.close_overlay();
            self.notice.clone_from(&self.planning.notice);
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
                    "Local save requested; Enter sends to the selected provider."
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
            KeyCode::Enter if key.kind == KeyEventKind::Press && key.modifiers.is_empty() => {
                self.apply_editor()
            }
            KeyCode::Enter => false,
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
                self.sources.clear_wire_request();
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
                self.sources.clear_wire_request();
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
        self.sources.cancel_memory_adoption();
        self.workflow.authoring.invalidate();
        if self.quit_requires_work_review()
            || self.planning_any_unknown()
            || self.execution_any_unknown()
            || self.planning_any_journal_failed()
            || self.execution_any_journal_failed()
        {
            self.open_cancel_review(true);
        } else {
            self.request_navigation(execution::Action::Quit);
        }
    }

    pub(super) fn finish_request_quit(&mut self) {
        self.cancel_pending_moves();
        if self.moves.failed.is_some() {
            self.open_moves();
            self.notice = "Move storage failed. Retry or explicitly review discarding that local operation before exit.".into();
            return;
        }
        // A committed, clean task exits directly; only unsaved or failed
        // state needs the save/stay decision.
        if self.draft_dirty()
            || self.revision != self.saved_revision
            || self.sources.persistence_pending()
            || self.sources.store_failed
            || self.planning_any_journal_failed()
            || self.moves.learning.is_some()
        {
            self.confirmation = Some(Confirmation {
                kind: ConfirmationKind::Quit,
                destructive: false,
            });
        } else if self.planning_any_busy()
            || self.execution_any_pending()
            || self.moves.persistence_pending()
            || self.recipe_export_busy()
        {
            self.quit_after_save = true;
            self.notice = "Finishing local persistence before exit…".into();
        } else {
            self.quit = true;
        }
    }

    pub fn deletion_pages(&self) -> usize {
        self.delete_preview
            .iter()
            .map(|target| {
                1 + target.excerpts.len()
                    + target.screenshots.len()
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
                        ConfirmationKind::ReturnToEevee => {
                            self.return_after_cancel = Some((self.task_id, self.lead.id));
                            self.planning.cancel();
                        }
                        ConfirmationKind::DiscardDraft => {
                            self.drafts.retain(|draft| draft.field != self.editor_field);
                            self.input.clear();
                            self.sources.clear_wire_request();
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
                                || self.execution_any_pending()
                                || self.planning_any_busy()
                                || self.moves.persistence_pending()
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
