//! Ephemeral, owner-bound recipe authoring; no serialized draft or admission format changes.
use super::super::{App, Overlay};
use crate::{
    messages::Message,
    store,
    workspace::workflows::authoring::{Draft, Export, command},
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::path::Path;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Menu,
    Commands,
    Review,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Scope,
    Criteria,
    Steps,
    Path,
}
impl Field {
    pub fn title(self) -> Message {
        match self {
            Self::Name => Message::RecipeAuthoringName,
            Self::Scope => Message::RecipeAuthoringScope,
            Self::Criteria => Message::RecipeAuthoringCriteria,
            Self::Steps => Message::RecipeAuthoringSteps,
            Self::Path => Message::RecipeAuthoringPath,
        }
    }
}
#[derive(Default)]
pub struct State {
    pub active: bool,
    pub draft: Draft,
    pub mode: Mode,
    pub field: Option<Field>,
    pub cursor: usize,
    pub done: bool,
    pub selected: usize,
    pub scroll: u16,
    pub scroll_max: u16,
    pub page: u16,
    pub reviewed: bool,
    pub notice: Option<Message>,
    pub error: String,
    pub export: Option<Export>,
    pub inflight: bool,
    pub generation: u64,
    owner: (i64, i64),
}
impl State {
    pub fn editing(&self) -> bool {
        self.active && self.field.is_some()
    }
    pub fn value(&self, field: Field) -> &str {
        match field {
            Field::Name => &self.draft.name,
            Field::Scope => &self.draft.scope,
            Field::Criteria => &self.draft.criteria,
            Field::Steps => &self.draft.steps,
            Field::Path => &self.draft.path,
        }
    }
    fn value_mut(&mut self, field: Field) -> &mut String {
        match field {
            Field::Name => &mut self.draft.name,
            Field::Scope => &mut self.draft.scope,
            Field::Criteria => &mut self.draft.criteria,
            Field::Steps => &mut self.draft.steps,
            Field::Path => &mut self.draft.path,
        }
    }
    pub fn invalidate(&mut self) {
        self.reviewed = false;
    }
    fn edit(&mut self, field: Field) {
        self.export = None;
        self.field = Some(field);
        self.cursor = self.value(field).len();
        self.done = false;
        self.invalidate();
    }
    pub fn paste(&mut self, value: &str) -> bool {
        let Some(field) = self.field.filter(|_| self.active) else {
            return false;
        };
        let text: String = value
            .chars()
            .filter(|ch| super::super::safe_character(*ch))
            .collect();
        if self.value(field).len().saturating_add(text.len())
            > crate::workspace::workflows::MAX_TEXT_BYTES
        {
            self.notice = Some(Message::RecipeAuthoringLimit);
            return true;
        }
        let cursor = self.cursor;
        self.value_mut(field).insert_str(cursor, &text);
        self.cursor = self
            .value(field)
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i >= cursor + text.len())
            .unwrap_or(self.value(field).len());
        self.export = None;
        self.invalidate();
        true
    }
    fn editor_key(&mut self, key: KeyEvent) {
        let Some(field) = self.field else {
            return;
        };
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            self.done = !self.done;
            return;
        }
        if key.code == KeyCode::Esc
            || key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL)
            || key.code == KeyCode::Enter
                && (self.done || key.modifiers.contains(KeyModifiers::CONTROL))
        {
            self.field = None;
            self.notice = Some(Message::RecipeAuthoringDraftKept);
            return;
        }
        if key.code == KeyCode::Enter {
            self.paste("\n");
            return;
        }
        if let KeyCode::Char(ch) = key.code
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.paste(ch.encode_utf8(&mut [0; 4]));
            return;
        }
        let value = self.value(field);
        let cursor = self.cursor;
        let before = value[..cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i);
        let after = value
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i > cursor)
            .unwrap_or(value.len());
        let start = value[..cursor].rfind('\n').map_or(0, |i| i + 1);
        let end = value[cursor..]
            .find('\n')
            .map_or(value.len(), |i| cursor + i);
        match key.code {
            KeyCode::Left => self.cursor = before,
            KeyCode::Right => self.cursor = after,
            KeyCode::Home => self.cursor = start,
            KeyCode::End => self.cursor = end,
            KeyCode::Backspace => {
                self.value_mut(field).replace_range(before..cursor, "");
                self.cursor = before;
            }
            KeyCode::Delete => {
                self.value_mut(field).replace_range(cursor..after, "");
            }
            KeyCode::Up | KeyCode::Down => {
                let column = ratatui::text::Span::raw(&value[start..cursor]).width();
                let target = if key.code == KeyCode::Up && start > 0 {
                    let end = start - 1;
                    Some((value[..end].rfind('\n').map_or(0, |i| i + 1), end))
                } else if key.code == KeyCode::Down && end < value.len() {
                    Some((
                        end + 1,
                        value[end + 1..]
                            .find('\n')
                            .map_or(value.len(), |i| end + 1 + i),
                    ))
                } else {
                    None
                };
                if let Some((start, end)) = target {
                    let mut width = 0;
                    let mut next = start;
                    for (offset, grapheme) in value[start..end].grapheme_indices(true) {
                        let count = ratatui::text::Span::raw(grapheme).width();
                        if width + count > column {
                            break;
                        }
                        width += count;
                        next = start + offset + grapheme.len();
                    }
                    self.cursor = next;
                }
            }
            _ => {}
        }
    }
}

impl App {
    pub fn recipe_export_busy(&self) -> bool {
        self.workflow.authoring.inflight
    }

    pub fn recipe_save(&mut self) {
        if self.recipe_export_busy()
            || self.party_inflight
            || self.party_request.is_some()
            || self.confirmation.is_some()
            || self.transitioning
            || self.quit_after_save
            || self.reset_after_save
        {
            self.notice = (Message::RecipeAuthoringBusy).text().into();
            return;
        }
        self.open_overlay(Overlay::Workflows);
        if self.overlay != Some(Overlay::Workflows) {
            return;
        }
        self.workflow.tab = super::Tab::Recipe;
        self.workflow.preview = false;
        self.workflow.affirmative = false;
        let owner = (self.task_id, self.lead.id);
        if self.workflow.authoring.owner != owner {
            let generation = self.workflow.authoring.generation.saturating_add(1);
            self.workflow.authoring = State {
                owner,
                generation,
                ..State::default()
            };
        }
        self.workflow.authoring.active = true;
        self.workflow.authoring.mode = Mode::Menu;
        self.workflow.authoring.export = None;
        self.workflow.authoring.invalidate();
        self.workflow.authoring.notice = Some(Message::RecipeAuthoringPolicy);
    }

    pub(super) fn recipe_authoring_key(&mut self, key: KeyEvent) -> bool {
        if !self.workflow.authoring.active {
            return false;
        }
        if key.kind == KeyEventKind::Release {
            return true;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.request_quit();
            return true;
        }
        if self.recipe_export_busy() {
            return true;
        }
        if self.width < 60 || self.height < 18 {
            self.workflow.authoring.invalidate();
            if key.code == KeyCode::Esc {
                self.workflow.authoring.mode = Mode::Menu;
                self.workflow.authoring.field = None;
                self.workflow.authoring.export = None;
                self.workflow.authoring.active = false;
                self.close_overlay();
            }
            return true;
        }
        if self.workflow.authoring.editing() {
            self.workflow.authoring.editor_key(key);
            return true;
        }
        if key.code == KeyCode::Esc {
            let state = &mut self.workflow.authoring;
            if state.mode == Mode::Menu {
                state.active = false;
            }
            if state.mode == Mode::Commands {
                state.selected = 5;
            }
            state.mode = Mode::Menu;
            state.export = None;
            state.scroll = 0;
            state.invalidate();
            state.notice = Some(Message::RecipeAuthoringCancelled);
            return true;
        }
        if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            let state = &mut self.workflow.authoring;
            state.scroll = if key.code == KeyCode::PageDown {
                state
                    .scroll
                    .saturating_add(state.page.max(1))
                    .min(state.scroll_max)
            } else {
                state.scroll.saturating_sub(state.page.max(1))
            };
            return true;
        }
        match self.workflow.authoring.mode {
            Mode::Menu => match key.code {
                KeyCode::Up => {
                    self.workflow.authoring.selected =
                        self.workflow.authoring.selected.saturating_sub(1)
                }
                KeyCode::Down => {
                    self.workflow.authoring.selected = (self.workflow.authoring.selected + 1).min(6)
                }
                KeyCode::Enter if key.kind == KeyEventKind::Press => {
                    let field = match self.workflow.authoring.selected {
                        0 => Some(Field::Name),
                        1 => Some(Field::Scope),
                        2 => Some(Field::Criteria),
                        3 => Some(Field::Steps),
                        4 => Some(Field::Path),
                        _ => None,
                    };
                    if let Some(field) = field {
                        self.workflow.authoring.edit(field);
                    } else if self.workflow.authoring.selected == 5 {
                        self.workflow.authoring.mode = Mode::Commands;
                        self.workflow.authoring.selected = 0;
                        self.workflow.authoring.scroll = 0;
                    } else {
                        self.recipe_export_preview();
                    }
                }
                _ => {}
            },
            Mode::Commands => {
                let count = self
                    .execution
                    .history
                    .iter()
                    .filter(|run| run.prepared.task_id == self.task_id)
                    .count();
                match key.code {
                    KeyCode::Up => {
                        self.workflow.authoring.selected =
                            self.workflow.authoring.selected.saturating_sub(1);
                        self.workflow.authoring.scroll = 0;
                    }
                    KeyCode::Down => {
                        self.workflow.authoring.selected =
                            (self.workflow.authoring.selected + 1).min(count.saturating_sub(1));
                        self.workflow.authoring.scroll = 0;
                    }
                    KeyCode::Char('o') => {
                        if let Some(run) = self
                            .execution
                            .history
                            .iter()
                            .filter(|run| run.prepared.task_id == self.task_id)
                            .nth(self.workflow.authoring.selected)
                        {
                            let reference = crate::workspace::workflows::EvidenceRef::run(run);
                            if let Ok(reference) = reference {
                                self.workflow_open_reference(reference);
                            }
                        }
                    }
                    KeyCode::Enter | KeyCode::Char(' ') if key.kind == KeyEventKind::Press => {
                        if let Some(run) = self
                            .execution
                            .history
                            .iter()
                            .filter(|run| run.prepared.task_id == self.task_id)
                            .nth(self.workflow.authoring.selected)
                        {
                            match command(Path::new(&self.cwd), self.task_id, run) {
                                Err(error) => self.workflow.authoring.error = error,
                                Ok(_) => {
                                    let runs = &mut self.workflow.authoring.draft.runs;
                                    let id = &run.prepared.run_id;
                                    if let Some(index) = runs.iter().position(|item| item == id) {
                                        runs.remove(index);
                                    } else if runs.len() < crate::workspace::workflows::MAX_ENTRIES
                                    {
                                        runs.push(id.clone());
                                    }
                                    self.workflow.authoring.error.clear();
                                    self.workflow.authoring.invalidate();
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Mode::Review => {
                if key.code == KeyCode::Enter && key.kind == KeyEventKind::Press {
                    self.workflow.authoring.mode = Mode::Menu;
                    self.workflow.authoring.export = None;
                    self.workflow.authoring.invalidate();
                    self.workflow.authoring.notice = Some(Message::RecipeAuthoringCancelled);
                    return true;
                }
                if key.code == KeyCode::Char('y')
                    && key.kind == KeyEventKind::Press
                    && key.modifiers.is_empty()
                {
                    self.recipe_export_confirm();
                }
            }
        }
        true
    }

    fn recipe_export_ready(&self) -> bool {
        self.task_id > 0
            && self.committed_input_ready()
            && self.records_can_switch()
            && !self.records.persistence_pending()
            && self.records.local.is_empty()
            && !self.planning_any_busy()
            && !self.execution_any_pending()
            && !self.planning_any_unknown()
            && self.pending_request.is_none()
            && self.pending_command.is_none()
            && !self.moves.mutation_pending()
            && !self.party_inflight
            && self.party_request.is_none()
            && !self.transitioning
            && !self.quit_after_save
            && !self.reset_after_save
            && !self.quit
            && self.confirmation.is_none()
            && self.command_popup.is_none()
    }

    fn recipe_export_preview(&mut self) {
        self.workflow.authoring.error.clear();
        self.workflow.authoring.export = None;
        self.workflow.authoring.invalidate();
        if !self.recipe_export_ready() {
            self.workflow.authoring.notice = Some(Message::RecipeAuthoringBusy);
            return;
        }
        let state = &mut self.workflow.authoring;
        state.generation = state.generation.saturating_add(1);
        match state.draft.preview(
            state.generation,
            self.task_id,
            self.lead.id,
            self.revision,
            Path::new(&self.cwd),
            &self.execution.history,
        ) {
            Ok(export) => {
                state.export = Some(export);
                state.mode = Mode::Review;
                state.scroll = 0;
                state.notice = Some(Message::RecipeAuthoringNeutral);
            }
            Err(error) => {
                state.error = error;
                state.notice = None;
            }
        }
    }

    fn recipe_export_confirm(&mut self) {
        if !self.workflow.authoring.reviewed
            || !self.recipe_export_ready()
            || self.overlay != Some(Overlay::Workflows)
            || self.width < 60
            || self.height < 18
        {
            self.workflow.authoring.invalidate();
            self.workflow.authoring.notice = Some(Message::RecipeAuthoringChanged);
            return;
        }
        let state = &self.workflow.authoring;
        let Some(export) = state.export.as_ref() else {
            return;
        };
        if !export.matches(
            state.generation,
            self.task_id,
            self.lead.id,
            self.revision,
            Path::new(&self.cwd),
        ) || export
            .runs()
            .iter()
            .any(|expected| !self.execution.history.iter().any(|run| run == expected))
        {
            self.workflow.authoring.invalidate();
            self.workflow.authoring.notice = Some(Message::RecipeAuthoringChanged);
            return;
        }
        let export = self.workflow.authoring.export.take().unwrap();
        self.workflow.authoring.inflight = true;
        self.workflow.authoring.invalidate();
        self.workflow.authoring.notice = Some(Message::RecipeAuthoringWorking);
        self.party_request = Some(store::PartyRequest::Formation(Box::new(
            store::formations::Request::ExportWorkflow(Box::new(export)),
        )));
    }

    pub(in crate::app) fn recipe_export_event(&mut self, result: Result<u64, String>) {
        let state = &mut self.workflow.authoring;
        state.inflight = false;
        state.invalidate();
        state.mode = Mode::Menu;
        match result {
            Ok(generation) if generation == state.generation => {
                state.notice = Some(Message::RecipeAuthoringSaved);
                state.error.clear();
            }
            Ok(_) => state.notice = Some(Message::RecipeAuthoringChanged),
            Err(error) => {
                state.error = error;
                state.notice = None;
            }
        }
    }
}

#[cfg(test)]
mod tests;
