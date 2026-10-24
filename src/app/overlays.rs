//! Overlay stack, selection restore, image controls and scrolling/focus.
use super::*;

impl App {
    pub(super) fn save_selection(&mut self) {
        if let Some(Overlay::Forms) = self.overlay {
            self.form_selection = self.selected;
        }
    }

    pub(super) fn restore_selection(&mut self) {
        if let Some(Overlay::Forms) = self.overlay {
            self.selected = self.form_selection;
        }
    }

    pub(super) fn overlay_frame(&self) -> Option<OverlayFrame> {
        self.overlay.map(|overlay| OverlayFrame {
            overlay,
            selected: self.selected,
            focus: self.focus,
            preview_form: self.preview_form,
            form_return_selected: self.form_return_selected,
        })
    }

    pub(super) fn restore_frame(&mut self, frame: Option<OverlayFrame>) {
        self.overlay = frame.map(|frame| frame.overlay);
        if let Some(frame) = frame {
            self.selected = frame.selected;
            self.focus = frame.focus;
            self.preview_form = frame.preview_form;
            self.form_return_selected = frame.form_return_selected;
        }
    }

    pub(super) fn open_overlay(&mut self, overlay: Overlay) {
        self.open_overlay_with_source_catalogue(overlay, true);
    }

    /// Direct context actions already own a specific worker operation. Opening
    /// their review must not enqueue an unrelated catalogue ahead of that work.
    pub(super) fn open_context_sources(&mut self) {
        self.open_overlay_with_source_catalogue(Overlay::Sources, false);
    }

    fn open_overlay_with_source_catalogue(&mut self, overlay: Overlay, request_catalogue: bool) {
        self.updates.leave();
        self.workflow.authoring.invalidate();
        if overlay == Overlay::Sources
            && let Some(task) = self
                .tasks
                .iter()
                .find(|task| task.id == self.task_id)
                .cloned()
        {
            self.sources.configure_optimizer(&task);
            self.sources.set_optimizer_runs(&self.execution.history);
        }
        if self.overlay == Some(Overlay::Moves) && overlay != Overlay::Moves {
            self.cancel_move_validation();
        }
        if overlay != Overlay::Sources {
            self.sources.cancel_memory_adoption();
            self.pending_source_action = None;
        }
        if overlay != Overlay::Planning {
            self.pending_provider_picker = None;
        }
        if self
            .direct_sheet_return
            .is_some_and(|(_, _, sheet)| sheet != overlay)
        {
            self.direct_sheet_return = None;
        }
        if overlay != Overlay::Memory {
            self.pending_memory_action = None;
        }
        if self.overlay != Some(overlay) {
            self.clear_approval_focus();
        }
        if overlay == Overlay::Workflows {
            self.workflow_open();
        }
        if overlay == Overlay::Memory
            && self.overlay != Some(Overlay::Memory)
            && !self.memory.saving
            && !self.memory_loading
            && self.task_id > 0
        {
            self.memory_loading = true;
            self.memory_request = Some(store::MemoryRequest {
                task_id: self.task_id,
                mutation: None,
            });
        }
        self.save_selection();
        if let Some(previous) = self.overlay_frame() {
            if previous.overlay == overlay {
                return;
            }
            // Returning to an already open surface unwinds to its frame.
            // Repeated cross-navigation must not grow an Esc maze.
            if let Some(index) = self
                .overlay_history
                .iter()
                .position(|frame| frame.overlay == overlay)
            {
                let frame = self.overlay_history.remove(index);
                self.overlay_history.truncate(index);
                self.restore_frame(Some(frame));
                return;
            }
            self.overlay_history.push(previous);
        }
        if overlay == Overlay::Preview {
            self.preview_form = if self.overlay == Some(Overlay::Forms) {
                self.selected
            } else {
                self.form()
            };
        }
        if matches!(
            overlay,
            Overlay::Tasks
                | Overlay::Criteria
                | Overlay::Repository
                | Overlay::Sources
                | Overlay::EvolutionPreview
        ) {
            self.surface_scroll[ScrollSurface::Preview.index()] = 0;
        }
        if overlay == Overlay::Images {
            self.images_selected = 0;
            self.images_reviewed = false;
            self.surface_scroll[ScrollSurface::Preview.index()] = 0;
        }
        self.overlay = Some(overlay);
        self.restore_selection();
        if overlay == Overlay::Forms {
            self.form_return_selected = false;
        }
        if overlay == Overlay::Party {
            self.selected = self
                .party
                .iter()
                .position(|partner| partner.id == self.lead.id)
                .unwrap_or(0);
        }
        if overlay == Overlay::Sources && request_catalogue {
            self.sources.catalogue(
                self.task_id,
                self.repository.as_ref(),
                self.repo_loading || self.repo_refresh,
            );
        } else if overlay == Overlay::Execution {
            self.execution.open(self.task_id);
        }
    }

    pub(super) fn close_overlay(&mut self) {
        self.updates.leave();
        self.workflow.authoring.invalidate();
        if self.overlay == Some(Overlay::Moves) {
            self.cancel_move_validation();
        }
        if self.overlay == Some(Overlay::Sources) {
            self.sources.cancel_memory_adoption();
            self.cancel_move_pins();
        }
        self.pending_source_action = None;
        self.pending_provider_picker = None;
        self.direct_sheet_return = None;
        self.pending_memory_action = None;
        self.clear_approval_focus();
        if self.overlay == Some(Overlay::Agents) {
            self.agents.cancel_preview();
        }
        self.save_selection();
        let frame = self.overlay_history.pop();
        self.restore_frame(frame);
        if self.overlay == Some(Overlay::Execution) {
            self.execution.open(self.task_id);
        }
        if self.overlay.is_none() {
            let owner = self.composer_return_owner.take();
            if let Some(field) = self.composer_return.take()
                && owner.is_none_or(|owner| owner == (self.task_id, self.lead.id))
                && self.confirmation.is_none()
            {
                if field == store::EditorField::Request {
                    self.open_editor(field);
                } else {
                    self.begin_edit_field(field);
                }
            }
        }
    }

    /// Called after the owning domain handles a key, never before its reviews.
    pub(super) fn settle_direct_action(
        &mut self,
        key: KeyEvent,
        was_specialist_editor: bool,
    ) -> bool {
        let Some((task, partner, sheet)) = self.direct_sheet_return else {
            return false;
        };
        if task != self.task_id || partner != self.lead.id || self.overlay != Some(sheet) {
            self.direct_sheet_return = None;
            return false;
        }
        if key.kind != KeyEventKind::Press
            || !key.modifiers.is_empty()
            || self.confirmation.is_some()
            || self.has_focused_approval()
        {
            return false;
        }
        let back = key.code == KeyCode::Esc;
        let finished = match sheet {
            Overlay::Sources => {
                back && self.sources.editor.is_none() && self.sources.confirmation.is_none()
            }
            Overlay::Execution => {
                back && self.execution.confirmation.is_none() && !self.execution.searching
            }
            Overlay::Handoff => back && !self.handoff.busy(),
            Overlay::Memory => {
                back && self.memory.mode == memory::Mode::Browse
                    && !self.memory.saving
                    && self.memory.pending.is_none()
            }
            Overlay::Workflows => {
                (back || key.code == KeyCode::Enter && was_specialist_editor)
                    && self.workflow.editor.is_none()
                    && !self.workflow.authoring.active
                    && self.workflow.chooser.is_none()
                    && !self.workflow.preview
                    && self.workflow.relay.is_none()
                    && !self.workflow.criteria_preview
            }
            _ => false,
        };
        if finished {
            self.close_overlay();
        }
        finished
    }

    pub(super) fn images_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Esc {
            self.close_overlay();
            return true;
        }
        if self.width < 60 || self.height < 18 || !key.modifiers.is_empty() {
            return false;
        }
        match key.code {
            KeyCode::Up | KeyCode::Left | KeyCode::BackTab => {
                self.images_selected = (self.images_selected + 2) % 3;
            }
            KeyCode::Down | KeyCode::Right | KeyCode::Tab => {
                self.images_selected = (self.images_selected + 1) % 3;
            }
            KeyCode::Enter => match self.images_selected {
                0 => self.close_overlay(),
                1 if self.images_busy => {
                    self.images_cancel = true;
                    self.images_notice =
                        "Cancelling; current request may take up to 30 seconds. Downloaded files stay available.".into();
                }
                1 if !self.images_enabled => {
                    self.images_notice =
                        "Images are disabled. Restart with --images auto or --images cells.".into();
                }
                1 if self.images_reviewed => {
                    self.images_pending = Some(crate::ui::artwork::Request::Download);
                    self.images_busy = true;
                    self.images_selected = 0;
                    self.images_notice = "Downloading pinned artwork; you can keep working.".into();
                }
                2 if !self.images_busy => {
                    self.images_pending = Some(crate::ui::artwork::Request::Reload);
                    self.images_busy = true;
                    self.images_selected = 0;
                    self.images_notice = "Reloading local images; no network request.".into();
                }
                _ => return false,
            },
            code => return self.scroll_key(code),
        }
        true
    }

    pub(super) fn toggle_criterion(&mut self) {
        if self.complete {
            self.confirmation = Some(Confirmation {
                kind: ConfirmationKind::Reopen(store::EditorField::Criteria),
                destructive: false,
            });
        } else if let Some(criterion) = self.criteria.get_mut(self.criterion_selection) {
            criterion.met = !criterion.met;
            self.changed_task();
        }
    }

    pub(super) fn task_overlay_key(&mut self, overlay: Overlay, key: KeyCode) -> bool {
        if matches!(key, KeyCode::PageUp | KeyCode::PageDown) {
            return self.scroll_key(key);
        }
        match (overlay, key) {
            (Overlay::Tasks, KeyCode::Enter) => {
                self.select_task();
                return true;
            }
            (Overlay::Criteria, KeyCode::Enter) => {
                self.toggle_criterion();
                return true;
            }
            _ => {}
        }
        let task_count = self.visible_tasks().count();
        let (selection, count) = if overlay == Overlay::Tasks {
            (&mut self.task_selection, task_count)
        } else {
            (&mut self.criterion_selection, self.criteria.len())
        };
        if count == 0 {
            return false;
        }
        let previous = *selection;
        *selection = match key {
            KeyCode::Up => previous.saturating_sub(1),
            KeyCode::Down => (previous + 1).min(count - 1),
            KeyCode::Home => 0,
            KeyCode::End => count - 1,
            _ => return false,
        };
        let changed = previous != *selection;
        if changed {
            self.surface_scroll[ScrollSurface::Preview.index()] = 0;
        }
        changed
    }

    pub(super) fn overlay_key(&mut self, overlay: Overlay, key: KeyCode) -> bool {
        if matches!(overlay, Overlay::Tasks | Overlay::Criteria) {
            return self.task_overlay_key(overlay, key);
        }
        if overlay == Overlay::Party {
            return self.party_overlay_key(key);
        }
        if overlay == Overlay::Companion && key == KeyCode::Char('c') {
            self.reopen_carry_over();
            return true;
        }
        if overlay == Overlay::Companion && key == KeyCode::Char('n') {
            self.open_nickname(self.lead().id);
            return true;
        }
        if overlay == Overlay::Companion && key == KeyCode::Enter {
            self.open_overlay(Overlay::Forms);
            return true;
        }
        if overlay == Overlay::Party && key == KeyCode::Char('a') {
            self.request_add_partner();
            return true;
        }
        let count = match overlay {
            Overlay::Forms => FORMS.len(),
            _ => return self.scroll_key(key),
        };
        if key == KeyCode::Char('d') {
            return self.command("suggestion-dismiss");
        }
        let previous = self.selected;
        let grid =
            overlay == Overlay::Forms && self.width >= 90 && self.height >= 30 && !self.linear;
        let vertical_step = if grid { 3 } else { 1 };
        if self.form_return_selected {
            match key {
                KeyCode::Enter => {
                    self.picker_return_to_eevee();
                    return true;
                }
                KeyCode::Down | KeyCode::Left | KeyCode::Right => {
                    self.form_return_selected = false;
                    self.selected = 0;
                    self.save_selection();
                    return true;
                }
                KeyCode::End => self.form_return_selected = false,
                _ => return false,
            }
        }
        match key {
            KeyCode::Up if self.selected < vertical_step => {
                self.form_return_selected = true;
                return true;
            }
            KeyCode::Up => self.selected -= vertical_step,
            KeyCode::Down => self.selected = (self.selected + vertical_step) % count,
            KeyCode::Left if overlay == Overlay::Forms => {
                self.selected = (self.selected + count - 1) % count
            }
            KeyCode::Right if overlay == Overlay::Forms => {
                self.selected = (self.selected + 1) % count
            }
            KeyCode::Home => {
                self.form_return_selected = true;
                return true;
            }
            KeyCode::End => self.selected = count - 1,
            KeyCode::Enter => {
                self.picker_select_form(self.selected);
                return true;
            }
            _ => return false,
        }
        self.save_selection();
        previous != self.selected
    }

    pub(super) fn scroll_surface(&self) -> ScrollSurface {
        match self.overlay {
            Some(Overlay::Companion) => ScrollSurface::Companion,
            Some(Overlay::Inspector) => ScrollSurface::Inspector,
            Some(Overlay::Help) => ScrollSurface::Help,
            Some(
                Overlay::Preview
                | Overlay::EvolutionPreview
                | Overlay::Tasks
                | Overlay::Criteria
                | Overlay::Repository
                | Overlay::Sources
                | Overlay::Images
                | Overlay::Party
                | Overlay::PartyBoard,
            ) => ScrollSurface::Preview,
            _ => match self.effective_focus() {
                Pane::Companion => ScrollSurface::Companion,
                Pane::Workspace => ScrollSurface::Workspace,
                Pane::Inspector => ScrollSurface::Inspector,
            },
        }
    }

    pub(super) fn scroll_key(&mut self, key: KeyCode) -> bool {
        let surface = self.scroll_surface().index();
        let page = self.surface_scroll_page[surface];
        let previous = self.surface_scroll[surface];
        self.surface_scroll[surface] = match key {
            KeyCode::Up => previous.saturating_sub(1),
            KeyCode::Down => previous.saturating_add(1),
            KeyCode::PageUp => previous.saturating_sub(page),
            KeyCode::PageDown => previous.saturating_add(page),
            KeyCode::Home => 0,
            KeyCode::End => self.surface_scroll_max[surface],
            _ => return false,
        }
        .min(self.surface_scroll_max[surface]);
        previous != self.surface_scroll[surface]
    }

    pub(super) fn cycle_focus(&mut self, backwards: bool) -> bool {
        if self.linear || self.width < 90 {
            return false;
        }
        self.focus = if self.width < 120 {
            match self.effective_focus() {
                Pane::Companion => Pane::Workspace,
                _ => Pane::Companion,
            }
        } else {
            match (self.effective_focus(), backwards) {
                (Pane::Companion, false) | (Pane::Inspector, true) => Pane::Workspace,
                (Pane::Workspace, false) | (Pane::Companion, true) => Pane::Inspector,
                (Pane::Inspector, false) | (Pane::Workspace, true) => Pane::Companion,
            }
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returning_to_open_surface_unwinds_cycles_and_restores_selection() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.focus = Pane::Inspector;
        app.open_overlay(Overlay::Inspector);
        app.open_overlay(Overlay::Forms);
        app.selected = 5;
        app.open_overlay(Overlay::Help);
        app.open_overlay(Overlay::Forms);
        assert_eq!(app.overlay, Some(Overlay::Forms));
        assert_eq!(app.selected, 5);
        app.update(Event::Resize(40, 10));
        app.close_overlay();
        assert_eq!(app.overlay, Some(Overlay::Inspector));
        app.close_overlay();
        assert_eq!(app.overlay, None);
        app.update(Event::Resize(120, 36));
        assert_eq!(app.effective_focus(), Pane::Inspector);
    }

    #[test]
    fn repeated_cross_navigation_does_not_accumulate_back_steps() {
        let mut app = App::new("/workspace".into(), 60, 18);
        for _ in 0..20 {
            app.open_overlay(Overlay::Inspector);
            app.open_overlay(Overlay::Help);
        }
        app.close_overlay();
        assert_eq!(app.overlay, Some(Overlay::Inspector));
        app.close_overlay();
        assert_eq!(app.overlay, None);
    }
}
