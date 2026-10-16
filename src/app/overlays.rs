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
        // The roster is local navigation: fronting another partner leaves the
        // request with its owner (PT06B).
        if self.planning.busy() && !matches!(overlay, Overlay::Planning | Overlay::Party) {
            self.notice = "Resolve or cancel the active provider request first.".into();
            return;
        }
        if overlay == Overlay::Planning && self.execution.busy() {
            self.notice = "Finish or cancel the local command before starting planning.".into();
            return;
        }
        if overlay == Overlay::Planning && matches!(self.form(), 3 | 4 | 7) {
            self.planning.set_operation(match self.form() {
                3 => crate::providers::Operation::Implement,
                7 => crate::providers::Operation::Review,
                _ => crate::providers::Operation::Plan,
            });
        }
        if overlay == Overlay::Workflows {
            self.workflow_open();
        }
        if overlay == Overlay::Memory
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
            Overlay::Tasks | Overlay::Criteria | Overlay::Repository | Overlay::Sources
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
        if overlay == Overlay::Sources {
            self.sources
                .catalogue(self.task_id, self.repository.as_ref());
        } else if overlay == Overlay::Execution {
            self.execution.open(self.task_id);
        }
    }

    pub(super) fn close_overlay(&mut self) {
        if self.overlay == Some(Overlay::Agents) {
            self.agents.cancel_preview();
        }
        self.save_selection();
        let frame = self.overlay_history.pop();
        self.restore_frame(frame);
        if self.overlay == Some(Overlay::Execution) {
            self.execution.open(self.task_id);
        }
        if self.overlay.is_none()
            && let Some(field) = self.composer_return.take()
        {
            if field == store::EditorField::Request {
                self.open_editor(field);
            } else {
                self.begin_edit_field(field);
            }
        }
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
                    self.request_return_to_eevee();
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
                self.request_navigation(execution::Action::SelectForm(self.selected));
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
                | Overlay::Tasks
                | Overlay::Criteria
                | Overlay::Repository
                | Overlay::Sources
                | Overlay::Images
                | Overlay::Party,
            ) => ScrollSurface::Preview,
            _ => match self.effective_focus() {
                Pane::Companion => ScrollSurface::Companion,
                Pane::Workspace => ScrollSurface::Workspace,
                Pane::Inspector => ScrollSurface::Inspector,
            },
        }
    }

    pub(super) fn scroll_key(&mut self, key: KeyCode) -> bool {
        let page = self.height.saturating_sub(8).max(1);
        let surface = self.scroll_surface().index();
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
