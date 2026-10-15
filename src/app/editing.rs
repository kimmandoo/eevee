//! Grapheme-safe text editing and editor open/apply/finish.
use super::*;

impl App {
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
        if self.planning.busy() {
            self.notice = "Cancel or finish the provider request before editing task input.".into();
            self.open_overlay(Overlay::Planning);
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

    pub(super) fn open_editor(&mut self, field: store::EditorField) {
        if field == store::EditorField::Command {
            self.execution.invalidate();
            self.pending_command = None;
        }
        self.save_selection();
        self.composer_return = None;
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
        match self.editor_field {
            store::EditorField::Request => return self.submit_request(),
            store::EditorField::Goal => self.goal = std::mem::take(&mut self.input),
            store::EditorField::NextAction => self.next_action = std::mem::take(&mut self.input),
            store::EditorField::Command => {
                self.command = std::mem::take(&mut self.input);
                self.pending_command = Some(self.command.clone());
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
                self.criteria = lines
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
                self.input.clear();
                self.criterion_selection = self
                    .criterion_selection
                    .min(self.criteria.len().saturating_sub(1));
            }
        }
        self.drafts.retain(|draft| draft.field != self.editor_field);
        self.finish_edit();
        self.changed_task();
        if self.editor_field == store::EditorField::Goal {
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
        self.restore_frame(frame);
    }
}
