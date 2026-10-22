//! Task lifecycle: snapshots, loading, creation, selection, archive, deletion and completion.
use super::*;

impl App {
    pub fn editor_label(&self) -> &str {
        match self.editor_field {
            store::EditorField::Request => "Request",
            store::EditorField::Goal => "Goal",
            store::EditorField::Criteria => "Acceptance criteria",
            store::EditorField::NextAction => "Next action",
            store::EditorField::Command => "Command / explicit JSON argv or shell",
        }
    }

    pub(super) fn applied_text(&self, field: store::EditorField) -> String {
        match field {
            store::EditorField::Request => String::new(),
            store::EditorField::Goal => self.goal.clone(),
            store::EditorField::Criteria => self
                .criteria
                .iter()
                .map(|criterion| criterion.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            store::EditorField::NextAction => self.next_action.clone(),
            store::EditorField::Command => self.command.clone(),
        }
    }

    pub(super) fn navigation(&self) -> store::Navigation {
        store::Navigation {
            focus: match self.focus {
                Pane::Companion => 0,
                Pane::Workspace => 1,
                Pane::Inspector => 2,
            },
            linear: self.linear,
            form_selection: self.form_selection,
            criterion_selection: self.criterion_selection,
            scroll: self.surface_scroll,
            previous_form: self.previous_form,
            overlay: match self.overlay {
                Some(Overlay::Forms) => Some(0),
                Some(Overlay::Help) => Some(2),
                Some(Overlay::Inspector) => Some(3),
                Some(Overlay::Companion) => Some(4),
                Some(Overlay::Preview) => Some(5),
                Some(Overlay::Workflows) => Some(6),
                Some(Overlay::Memory) => Some(7),
                _ => None,
            },
            editing: self.editing,
            editor_field: self.editor_field,
            dismissed_suggestion: self.dismissed_suggestion.clone(),
        }
    }

    pub(super) fn partner_draft(&self) -> store::PartnerDrafts {
        let mut drafts = self.drafts.clone();
        if self.editing {
            drafts.retain(|draft| draft.field != self.editor_field);
            if self.draft_dirty() || self.cursor != self.input.len() {
                drafts.push(store::Draft {
                    field: self.editor_field,
                    text: self.input.clone(),
                    cursor: self.cursor,
                });
            }
        }
        store::PartnerDrafts {
            drafts,
            editor_field: self.editor_field,
            editing: self.editing || self.composer_return.is_some(),
            previous_form: self.previous_form,
        }
    }

    pub fn snapshot(&self, project: &str) -> store::Snapshot {
        let mut partner_drafts = self.partner_drafts.clone();
        partner_drafts.insert(self.lead.id, self.partner_draft());
        store::Snapshot {
            project: project.to_owned(),
            goal: self.goal.clone(),
            form: self.form(),
            lead_partner: self.lead().id,
            revision: self.revision,
            id: self.task_id,
            criteria: self.criteria.clone(),
            next_action: self.next_action.clone(),
            command: self.command.clone(),
            workflows: self.workflows.clone(),
            workflow_draft: self.workflow.draft_snapshot(),
            memory_draft: self.memory.draft_snapshot(),
            approved_memories: self.memory.selected_snapshot().unwrap_or_else(|_| {
                self.tasks
                    .iter()
                    .find(|task| task.id == self.task_id)
                    .map(|task| task.approved_memories.clone())
                    .unwrap_or_default()
            }),
            complete: self.complete,
            partner_drafts,
            navigation: self.navigation(),
            updated_at: self.updated_at,
            archived: self.archived,
            archived_at: self.archived_at,
            redacted: self.redacted,
            local_records: self.records.local.clone(),
        }
    }

    pub(super) fn load_task(&mut self, snapshot: store::Snapshot) {
        self.records.restoring = true;
        let task_changed = self.task_id != snapshot.id;
        self.task_id = snapshot.id;
        self.pending_command = None;
        self.pending_request = None;
        self.goal = snapshot.goal;
        // The task's lead fronts this task with its full stored identity; a
        // missing row (guarded by store inspection) falls back to a bare id.
        let lead = self
            .party
            .iter()
            .find(|partner| partner.id == snapshot.lead_partner)
            .cloned()
            .unwrap_or_else(|| store::Partner {
                id: snapshot.lead_partner,
                ..store::Partner::default()
            });
        if self.lead.id != lead.id {
            self.front_planning(&lead);
        }
        self.lead = lead;
        if task_changed {
            self.cooperation.parked.clear();
            self.cooperation.executions.clear();
            self.cooperation.execution_actions.clear();
            self.cooperation.command_saves.clear();
            self.pending_request_owner = None;
            self.planning.reset(snapshot.id);
        }
        self.execution.set_task(snapshot.id);
        self.sync_front();
        self.set_form(snapshot.form);
        // Loading/restoring a stored form is not an accepted form switch.
        self.clear_carry_over();
        self.revision = snapshot.revision;
        self.saved_revision = snapshot.revision;
        self.criteria = snapshot.criteria;
        self.next_action = snapshot.next_action;
        self.command = snapshot.command;
        self.workflows = snapshot.workflows;
        self.workflow = workflows::State::default();
        self.workflow.restore_draft(snapshot.workflow_draft);
        self.memory.reset(snapshot.id, snapshot.project.clone());
        let _ = self.memory.restore_draft(&snapshot.memory_draft);
        self.memory_request = (snapshot.id > 0).then_some(store::MemoryRequest {
            task_id: snapshot.id,
            mutation: None,
        });
        self.memory_loading = snapshot.id > 0;
        self.complete = snapshot.complete;
        self.updated_at = snapshot.updated_at;
        self.archived = snapshot.archived;
        self.archived_at = snapshot.archived_at;
        self.redacted = snapshot.redacted;
        self.partner_drafts = snapshot.partner_drafts;
        let draft = self
            .partner_drafts
            .get(&self.lead.id)
            .cloned()
            .unwrap_or_default();
        self.drafts = draft.drafts;
        self.input.clear();
        self.edit_original.clear();
        self.confirmation = None;
        self.overlay_history.clear();
        self.edit_return = None;
        self.composer_menu = None;
        self.command_popup = None;
        self.composer_catalogue = false;
        self.composer_return = None;
        self.composer_return_owner = None;
        self.temporary_editor_return = None;
        self.direct_sheet_return = None;
        self.pending_memory_action = None;
        self.pending_source_action = None;
        self.pending_provider_picker = None;
        self.editing = false;
        self.cursor = 0;
        let navigation = snapshot.navigation;
        self.dismissed_suggestion = navigation.dismissed_suggestion;
        self.focus = match navigation.focus {
            0 => Pane::Companion,
            2 => Pane::Inspector,
            _ => Pane::Workspace,
        };
        self.linear = navigation.linear;
        self.form_selection = navigation.form_selection;
        // Restored after set_form above so the stored value wins over the
        // in-memory tracking the load path just recorded.
        self.previous_form = draft.previous_form;
        self.surface_scroll = navigation.scroll;
        self.surface_scroll_max = [u16::MAX; 5];
        self.surface_scroll_page = [1; 5];
        self.overlay = match navigation.overlay {
            Some(0) => Some(Overlay::Forms),
            // Legacy palette navigation is safely returned to the workspace.
            Some(1) => None,
            Some(2) => Some(Overlay::Help),
            Some(3) => Some(Overlay::Inspector),
            Some(4) => Some(Overlay::Companion),
            Some(5) => Some(Overlay::Preview),
            Some(6) => Some(Overlay::Workflows),
            Some(7) => Some(Overlay::Memory),
            _ => None,
        };
        self.restore_selection();
        self.editor_field = draft.editor_field;
        if draft.editing && self.overlay.is_some() {
            self.composer_return = Some(draft.editor_field);
            self.composer_return_owner = Some((self.task_id, self.lead.id));
        } else if draft.editing {
            self.open_editor(draft.editor_field);
        } else if self.overlay.is_none() {
            self.open_editor(store::EditorField::Request);
        }
        self.storage = StorageState::Saved;
        self.show_archived = false;
        self.save_requested = false;
        self.autosave = None;
        self.save_inflight = None;
        let selection = self
            .visible_tasks()
            .position(|task| task.id == self.task_id)
            .unwrap_or(0);
        self.task_selection = selection;
        self.criterion_selection = navigation
            .criterion_selection
            .min(self.criteria.len().saturating_sub(1));
        self.apply_relay_draft_role();
        self.records.restoring = false;
        self.records_invalidate();
    }

    pub(super) fn new_task(&mut self) {
        self.request_navigation(execution::Action::NewTask);
    }

    pub(super) fn finish_new_task(&mut self) {
        self.flush_save();
        if !matches!(self.storage, StorageState::Saved)
            || self.save_requested
            || self.sources.persistence_pending()
            || self.sources.store_failed
        {
            self.notice = "Finish or retry the current save before creating another task.".into();
            return;
        }
        let mut task = store::Snapshot::empty(self.cwd.clone());
        task.navigation.linear = self.linear;
        self.load_task(task);
        self.overlay = None;
        self.notice.clear();
        self.open_editor(store::EditorField::Request);
    }

    pub(super) fn select_task(&mut self) {
        if self.show_archived {
            self.notice = "Restore this archived task before resuming it.".into();
            return;
        }
        let selected = self
            .visible_tasks()
            .nth(self.task_selection)
            .map(|task| task.id);
        if let Some(id) = selected {
            self.request_navigation(execution::Action::SwitchTask(id));
        }
    }

    pub(super) fn request_return_to_eevee(&mut self) {
        if self.planning.busy() {
            self.confirmation_reviewed = false;
            self.confirmation = Some(Confirmation {
                kind: ConfirmationKind::ReturnToEevee,
                destructive: false,
            });
        } else {
            self.request_navigation(execution::Action::SelectForm(0));
        }
    }

    pub(super) fn request_navigation(&mut self, action: execution::Action) {
        if !matches!(action, execution::Action::Quit) && !self.records_can_switch() {
            self.notice =
                "Wait for the local record batch to commit before switching forms or tasks.".into();
            return;
        }
        if !matches!(action, execution::Action::Quit) && self.moves.mutation_pending() {
            self.notice =
                "Wait for the local move operation to commit before changing tasks or forms."
                    .into();
            return;
        }
        if self.memory.saving
            || self
                .memory_request
                .as_ref()
                .is_some_and(|request| request.mutation.is_some())
            || self.memory.pending.is_some()
        {
            self.notice = "Wait for the approved-memory operation to finish before changing tasks or leaving.".into();
            return;
        }
        let form_only = matches!(action, execution::Action::SelectForm(_));
        let blocked = if matches!(action, execution::Action::Quit) {
            self.quit_requires_work_review()
        } else if form_only {
            self.planning.busy()
        } else {
            self.planning_any_busy()
        };
        if matches!(action, execution::Action::Quit)
            && (blocked
                || self.planning_any_unknown()
                || self.execution_any_unknown()
                || self.planning_any_journal_failed()
                || self.execution_any_journal_failed())
        {
            self.open_cancel_review(true);
            return;
        }
        if blocked
            || !form_only
                && (self.planning_any_journal_failed()
                    || self.planning_any_unknown_effects()
                    || self.execution_any_unknown())
        {
            self.notice = "Finish or explicitly cancel owned work, and inspect retained journal/outcome before changing tasks; /party-board lists every owner.".into();
            return;
        }
        if !form_only && !matches!(action, execution::Action::Quit) && self.execution_any_pending()
        {
            let only_front_command = self.execution.busy()
                && self.cooperation.command_saves.is_empty()
                && self
                    .cooperation
                    .executions
                    .values()
                    .all(|state| !state.busy() && !state.storage_pending());
            if !only_front_command {
                self.notice = "Other owned commands or command saves must settle before changing tasks; /cancel-all reviews every owner.".into();
                return;
            }
        }
        if form_only
            && self
                .command_owner()
                .is_some_and(|owner| owner != self.lead.id)
        {
            self.apply_execution_action(action);
            return;
        }
        if let Some(action) = self.execution.request_navigation(action) {
            self.apply_execution_action(action);
        } else if self.execution.visible() {
            self.open_overlay(Overlay::Execution);
        }
    }

    pub fn apply_execution_action(&mut self, action: execution::Action) {
        match action {
            execution::Action::PinLocation(location, run) => {
                if let Err(error) = self.pin_execution_location(location, run) {
                    self.execution.notice.clone_from(&error);
                    self.notice = error;
                }
            }
            execution::Action::Rerun(run) => {
                if let Err(error) = self.stage_execution_rerun(run) {
                    self.execution.notice.clone_from(&error);
                    self.notice = error;
                }
            }
            execution::Action::NewTask => self.finish_new_task(),
            execution::Action::ResetTask => {
                self.pending_request = None;
                self.pending_command = None;
                self.flush_save();
                if matches!(self.storage, StorageState::Pending | StorageState::Saving)
                    || self.save_requested
                    || self.sources.persistence_pending()
                {
                    if matches!(
                        self.storage,
                        StorageState::Failed(_) | StorageState::Loading
                    ) || self.sources.store_failed
                    {
                        self.notice = "Retry storage, then /reset again; the current task and drafts are retained.".into();
                    } else {
                        self.reset_after_save = true;
                        self.notice = "Saving old task and drafts before reset / Esc Stay".into();
                    }
                } else {
                    self.finish_new_task();
                }
            }
            execution::Action::Quit => self.finish_request_quit(),
            execution::Action::SelectForm(index) => {
                if self.form() != index {
                    self.set_form(index);
                    self.planning.select_form_operation(index);
                    self.changed_task();
                }
                if index == 0 {
                    self.overlay = None;
                    self.overlay_history.clear();
                }
                if self.overlay == Some(Overlay::Execution) {
                    self.close_overlay();
                }
                if self.overlay == Some(Overlay::Forms) {
                    self.close_overlay();
                }
                if self.overlay.is_none() && self.composer_return.is_some() {
                    self.close_overlay();
                }
            }
            execution::Action::EditCommand => {
                self.execution.invalidate();
                self.pending_command = None;
                self.begin_edit_field(store::EditorField::Command);
            }
            execution::Action::SwitchTask(id) => {
                self.flush_save();
                if !matches!(self.storage, StorageState::Saved)
                    || self.save_requested
                    || self.sources.persistence_pending()
                    || self.sources.store_failed
                {
                    self.notice = "Finish or retry task/context saves before switching; the current task was retained.".into();
                } else {
                    self.switch_task = Some(id);
                    self.clear_carry_over();
                    self.transitioning = true;
                    self.storage = StorageState::Loading;
                }
            }
        }
    }

    pub(super) fn settle_reset(&mut self) {
        if !self.reset_after_save {
            return;
        }
        if matches!(self.storage, StorageState::Failed(_)) || self.sources.store_failed {
            self.reset_after_save = false;
            self.notice =
                "Reset stopped: retry storage, then /reset again. Old task and drafts retained."
                    .into();
        } else if matches!(self.storage, StorageState::Saved)
            && self.saved_revision == self.revision
            && !self.save_requested
            && self.save_inflight.is_none()
            && !self.sources.persistence_pending()
        {
            self.reset_after_save = false;
            self.request_navigation(execution::Action::ResetTask);
        }
    }
    pub fn settle_exit(&mut self) {
        if self.quit_after_save && self.moves.failed.is_some() {
            self.quit_after_save = false;
            self.open_moves();
            self.notice = "Move storage failed during exit. Retry or review discard; nothing was silently lost.".into();
            return;
        }
        if self.quit_after_save
            && matches!(self.storage, StorageState::Saved)
            && self.saved_revision == self.revision
            && !self.save_requested
            && self.save_inflight.is_none()
            && !self.sources.persistence_pending()
            && !self.execution_any_pending()
            && !self.planning_any_busy()
            && !self.moves.persistence_pending()
        {
            if self.sources.store_failed || self.planning_any_journal_failed() {
                self.quit_after_save = false;
                self.confirmation = Some(Confirmation {
                    kind: ConfirmationKind::Quit,
                    destructive: false,
                });
            } else {
                self.quit = true;
            }
        }
    }

    pub(super) fn source_key(&mut self, key: KeyEvent) -> bool {
        if self.width < 60 || self.height < 18 {
            self.sources.invalidate_review();
        }
        let action = self
            .sources
            .key(key, self.task_id, self.repository.as_ref());
        self.apply_source_action(action)
    }

    pub(super) fn apply_source_action(&mut self, action: sources::Action) -> bool {
        match action {
            sources::Action::Close => self.close_overlay(),
            sources::Action::Quit => self.request_quit(),
            sources::Action::RefreshRepository => {
                self.repo_refresh = true;
                self.repo_loading = true;
            }
            sources::Action::Scroll(code) => return self.scroll_key(code),
            sources::Action::Changed => self.surface_scroll[ScrollSurface::Preview.index()] = 0,
        }
        true
    }

    pub(super) fn execution_key(&mut self, key: KeyEvent) -> bool {
        if (self.width < 60 || self.height < 18)
            && (key.code == KeyCode::Enter
                || (key.kind == KeyEventKind::Press
                    && key.modifiers.is_empty()
                    && key.code == KeyCode::Char('y')))
            && matches!(
                self.execution.confirmation,
                Some(execution::Confirmation::Approval)
            )
        {
            return false;
        }
        if let Some(action) = self.execution.key(key) {
            self.apply_execution_action(action);
        }
        self.refresh_approvals();
        if !self.execution.visible() && self.overlay == Some(Overlay::Execution) {
            self.close_overlay();
        }
        true
    }

    pub fn visible_tasks(&self) -> impl Iterator<Item = &store::Snapshot> {
        self.tasks
            .iter()
            .filter(|task| task.archived == self.show_archived)
    }

    pub(super) fn archive_selected(&mut self, archived: bool) {
        let selected = self
            .visible_tasks()
            .nth(self.task_selection)
            .map(|task| task.id);
        let Some(id) = selected else { return };
        if archived
            && id == self.task_id
            && (!matches!(self.storage, StorageState::Saved) || self.save_requested)
        {
            self.notice = "Finish or retry this task's save before archiving it.".into();
            return;
        }
        self.management = Some(store::Management::Archive { id, archived });
        self.transitioning = true;
    }

    pub(super) fn preview_delete(&mut self, expired: bool) {
        self.delete_targets.clear();
        self.delete_expired_before = None;
        let selected = self
            .visible_tasks()
            .nth(self.task_selection)
            .map(|task| task.id);
        if expired {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let age = u128::from(self.policy.retention_days) * 86_400_000;
            let cutoff = now.saturating_sub(age).min(i64::MAX as u128) as i64;
            self.delete_targets.extend(
                self.tasks
                    .iter()
                    .filter(|task| {
                        task.archived && task.archived_at > 0 && task.archived_at <= cutoff
                    })
                    .map(|task| task.id),
            );
            self.delete_expired_before = Some(cutoff);
        } else if let Some(id) = selected {
            self.delete_targets.push(id);
        }
        if self.delete_targets.is_empty() {
            self.notice = "No eligible tasks selected; nothing was deleted.".into();
            return;
        }
        self.delete_preview.clear();
        self.deletion_preview_pending = true;
        self.transitioning = true;
        self.management = Some(store::Management::PreviewDelete {
            ids: self.delete_targets.clone(),
            expired_before: self.delete_expired_before,
        });
    }

    pub(super) fn request_complete(&mut self) {
        if self.complete {
            self.notice =
                "Already complete. Editing goal or criteria requires explicit reopening.".into();
        } else if self.drafts.iter().any(|draft| {
            matches!(
                draft.field,
                store::EditorField::Goal | store::EditorField::Criteria
            )
        }) {
            self.notice =
                "Apply or discard retained goal/criteria drafts before completing the task.".into();
        } else if self.criteria.is_empty() || self.criteria.iter().any(|criterion| !criterion.met) {
            self.notice =
                "Complete requires at least one criterion and every criterion checked.".into();
        } else {
            self.confirmation = Some(Confirmation {
                kind: ConfirmationKind::Complete,
                destructive: false,
            });
        }
    }

    pub(super) fn changed_task(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.save_requested = true;
        self.flush_save();
    }
}
