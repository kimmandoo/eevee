//! Grapheme-safe text editing and editor open/apply/finish.
use super::*;

impl App {
    pub(super) fn prepare_init(&mut self) {
        let request = if self.composer_is_request() {
            Some(self.input.as_str())
        } else {
            self.drafts
                .iter()
                .find(|draft| draft.field == store::EditorField::Request)
                .map(|draft| draft.text.as_str())
        };
        if request.is_some_and(|text| !text.is_empty()) {
            self.confirmation_reviewed = false;
            self.confirmation = Some(Confirmation {
                kind: ConfirmationKind::ReplaceWithInit,
                destructive: false,
            });
        } else {
            self.apply_init();
        }
    }

    pub(super) fn apply_init(&mut self) {
        if self.planning_any_busy() || self.execution_any_pending() {
            self.notice = "Finish or cancel active work before preparing /init.".into();
            return;
        }
        if self.memory.saving
            || self.memory.pending.is_some()
            || self
                .memory_request
                .as_ref()
                .is_some_and(|request| request.mutation.is_some())
            || self.planning_any_journal_failed()
            || self.execution_any_journal_failed()
            || self.planning_any_unknown_effects()
            || self.execution_any_unknown()
        {
            self.notice = "Resolve retained operations or provider outcome before preparing /init; drafts kept.".into();
            return;
        }
        let project = self
            .repository
            .as_ref()
            .map(|report| report.repository.root.to_string_lossy())
            .unwrap_or_else(|| std::borrow::Cow::Borrowed(self.cwd.as_str()));
        let request = format!(
            "Prepare a project-initialization proposal for the current project at {project:?}.\n\n\
             Inspect the repository before making claims: identify its structure, languages, existing project guidance (including any AGENTS.md), coding conventions, and documented build, test, lint and formatting commands. Cite the files supporting your findings; label unknowns and do not invent commands or results. Do not run build/test commands just to prepare this guide.\n\n\
             Propose a concise AGENTS.md project guide with repository-specific navigation, conventions, and verified-from-documentation commands. If AGENTS.md already exists, preserve it and propose a reviewed change rather than replacing it. Show the proposed content or diff and any open questions.\n\n\
             This is a Plan/read-only proposal request, not permission to initialize or modify files. Do not create, overwrite or change AGENTS.md or other files. File creation or changes require a later, separate explicit approval. Do not claim initialization is complete."
        );
        if request.len() > MAX_GOAL_BYTES {
            self.notice = "Initialization request exceeds the draft limit for this project path. Write a shorter initialization request manually; existing draft and role kept.".into();
            return;
        }
        self.pending_request = None;
        if self.editing {
            self.retain_editor();
        }
        // Use the persisted form/role transition so opening the later review
        // cannot restore an old Implement role or selected write scope.
        self.request_navigation(execution::Action::SelectForm(4));
        if self.execution.confirmation.is_some() {
            return;
        }
        self.planning
            .set_operation(crate::providers::Operation::Plan);
        self.planning.edit_paths.clear();
        self.request_home(false);
        self.input.clear();
        self.cursor = 0;
        self.insert_input(&request);
        self.changed_task();
        self.notice = "/init prepared an editable request; selected Espeon / Plan and cleared write paths. Enter sends when ready. No files changed or request sent.".into();
    }

    pub(super) fn insert_input(&mut self, text: &str) -> bool {
        self.normalize_cursor();
        let capacity = MAX_GOAL_BYTES.saturating_sub(self.input.len());
        let mut inserted = String::with_capacity(text.len().min(capacity));
        let was_truncated = self.input_truncated;
        let mut chars = text.chars().peekable();
        while let Some(mut character) = chars.next() {
            if character == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                character = '\n';
            }
            if !safe_character(character) {
                continue;
            }
            if inserted.len() + character.len_utf8() > capacity {
                self.input_truncated = true;
                break;
            }
            inserted.push(character);
        }
        if inserted.is_empty() {
            return self.input_truncated != was_truncated;
        }
        self.input.insert_str(self.cursor, &inserted);
        self.cursor += inserted.len();
        // A combining insertion can merge with the following grapheme.
        self.cursor = self
            .input
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|&index| index >= self.cursor)
            .unwrap_or(self.input.len());
        self.preferred_column = None;
        true
    }

    pub(super) fn normalize_cursor(&mut self) {
        self.cursor = self.cursor.min(self.input.len());
        if self.cursor != self.input.len() {
            self.cursor = self
                .input
                .grapheme_indices(true)
                .map(|(index, _)| index)
                .take_while(|&index| index <= self.cursor)
                .last()
                .unwrap_or(0);
        }
    }

    pub(super) fn previous_boundary(&self) -> usize {
        self.input
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .take_while(|&index| index < self.cursor)
            .last()
            .unwrap_or(0)
    }

    pub(super) fn next_boundary(&self) -> usize {
        self.input
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|&index| index > self.cursor)
            .unwrap_or(self.input.len())
    }

    pub(super) fn line_start(&self) -> usize {
        self.input[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1)
    }

    pub(super) fn line_end(&self) -> usize {
        self.input[self.cursor..]
            .find('\n')
            .map_or(self.input.len(), |index| self.cursor + index)
    }

    pub(super) fn move_cursor(&mut self, cursor: usize) -> bool {
        self.preferred_column = None;
        let changed = self.cursor != cursor;
        self.cursor = cursor;
        changed
    }

    pub(super) fn move_vertical(&mut self, down: bool) -> bool {
        let start = self.line_start();
        let end = self.line_end();
        let column = self
            .preferred_column
            .unwrap_or_else(|| Span::raw(&self.input[start..self.cursor]).width());
        let (target_start, target_end) = if down {
            if end == self.input.len() {
                return false;
            }
            let next = end + 1;
            let next_end = self.input[next..]
                .find('\n')
                .map_or(self.input.len(), |index| next + index);
            (next, next_end)
        } else {
            if start == 0 {
                return false;
            }
            let previous_end = start - 1;
            let previous = self.input[..previous_end]
                .rfind('\n')
                .map_or(0, |index| index + 1);
            (previous, previous_end)
        };
        let mut cursor = target_start;
        let mut cells = 0;
        for (index, grapheme) in self.input[target_start..target_end].grapheme_indices(true) {
            let width = Span::raw(grapheme).width();
            if cells + width > column {
                break;
            }
            cells += width;
            cursor = target_start + index + grapheme.len();
        }
        self.preferred_column = Some(column);
        let changed = cursor != self.cursor;
        self.cursor = cursor;
        changed
    }

    pub(super) fn begin_edit(&mut self) {
        self.begin_edit_field(store::EditorField::Goal);
    }

    pub(super) fn begin_edit_field(&mut self, field: store::EditorField) {
        if (self.planning_any_busy() || self.execution_any_busy())
            && !matches!(
                field,
                store::EditorField::Request | store::EditorField::Command
            )
        {
            self.notice = "Cancel or finish the provider request before editing task input.".into();
            self.open_overlay(Overlay::Planning);
            return;
        }
        if field == store::EditorField::Command
            && (self.execution.busy() || self.cooperation.command_saves.contains_key(&self.lead.id))
        {
            self.notice = "Finish or cancel the owned command before replacing its command input; your draft is kept.".into();
            return;
        }
        if self.complete
            && matches!(
                field,
                store::EditorField::Goal | store::EditorField::Criteria
            )
        {
            self.confirmation = Some(Confirmation {
                kind: ConfirmationKind::Reopen(field),
                destructive: false,
            });
            return;
        }
        self.open_editor(field);
    }

    pub(super) fn retain_action_editor(&mut self) {
        let field = self
            .temporary_editor_return
            .take()
            .filter(|owner| owner.task == self.task_id && owner.partner == self.lead.id)
            .map_or(self.editor_field, |owner| owner.field);
        self.retain_editor();
        self.composer_return = Some(field);
        self.composer_return_owner = Some((self.task_id, self.lead.id));
    }

    pub(super) fn open_editor(&mut self, field: store::EditorField) {
        if field == store::EditorField::NextAction {
            self.workflow.pending_decision = None;
        }
        if field == store::EditorField::Command {
            self.execution.invalidate();
            self.pending_command = None;
        }
        self.save_selection();
        self.temporary_editor_return = self
            .composer_return
            .take()
            .filter(|return_field| *return_field != field)
            .filter(|_| {
                self.composer_return_owner
                    .is_none_or(|owner| owner == (self.task_id, self.lead.id))
            })
            .map(|field| EditorReturn {
                task: self.task_id,
                partner: self.lead.id,
                field,
            });
        self.composer_return_owner = None;
        self.edit_return = self.overlay_frame();
        self.overlay = None;
        self.editor_field = field;
        self.edit_original = self.applied_text(field);
        if let Some(draft) = self.drafts.iter().find(|draft| draft.field == field) {
            self.input.clone_from(&draft.text);
            self.cursor = draft.cursor;
        } else {
            self.input.clone_from(&self.edit_original);
            self.cursor = self.input.len();
        }
        self.normalize_cursor();
        self.input_truncated = false;
        self.preferred_column = None;
        self.editing = true;
        self.notice.clear();
    }

    pub(super) fn apply_editor(&mut self) -> bool {
        if self.width < 60 || self.height < 18 {
            self.notice = "Enlarge the terminal before applying this draft.".into();
            return false;
        }
        let applied_field = self.editor_field;
        match self.editor_field {
            store::EditorField::Request => return self.submit_request(),
            store::EditorField::Goal => self.goal = std::mem::take(&mut self.input),
            store::EditorField::NextAction => {
                self.next_action = std::mem::take(&mut self.input);
                self.workflow_commit_decision();
            }
            store::EditorField::Command => {
                if let Some(reason) = self.work_capacity_reason(self.lead.id) {
                    self.notice = reason;
                    return true;
                }
                self.command = std::mem::take(&mut self.input);
                self.pending_command = Some((self.command.clone(), self.attribution()));
            }
            store::EditorField::Criteria => {
                let lines: Vec<&str> = self
                    .input
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .collect();
                if lines.len() > 64 || lines.iter().any(|line| line.len() > 256) {
                    self.notice =
                        "At most 64 criteria, each at most 256 UTF-8 bytes. Draft kept.".into();
                    return true;
                }
                let next: Vec<_> = lines
                    .into_iter()
                    .enumerate()
                    .map(|(index, text)| store::Criterion {
                        met: self
                            .criteria
                            .get(index)
                            .is_some_and(|old| old.text == text && old.met),
                        text: text.to_owned(),
                    })
                    .collect();
                if let Err(error) = self.workflow_reconcile_criteria_origins(&next) {
                    self.notice = error;
                    return true;
                }
                self.criteria = next;
                self.input.clear();
                self.criterion_selection = self
                    .criterion_selection
                    .min(self.criteria.len().saturating_sub(1));
            }
        }
        self.drafts.retain(|draft| draft.field != applied_field);
        self.finish_edit();
        self.changed_task();
        if applied_field == store::EditorField::Command {
            self.track_pending_command();
        }
        if applied_field == store::EditorField::Goal {
            self.notice = "Goal applied; / or Ctrl+K opens actions.".into();
        }
        true
    }

    pub(super) fn finish_edit(&mut self) {
        self.composer_menu = None;
        self.composer_catalogue = false;
        self.editing = false;
        self.cursor = 0;
        self.input_truncated = false;
        self.preferred_column = None;
        let frame = self.edit_return.take();
        let owner = self.temporary_editor_return.take();
        // An approval focused after the editor opened owns the visible surface.
        // Saving or retaining an older editor must not dismiss it.
        if self.overlay.is_none() {
            self.restore_frame(frame);
            if self.overlay.is_none()
                && let Some(owner) = owner
                && owner.task == self.task_id
                && owner.partner == self.lead.id
                && self.confirmation.is_none()
            {
                self.open_editor(owner.field);
            }
        }
    }
}
