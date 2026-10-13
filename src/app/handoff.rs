//! Manual handoff review. No provider, journal, or task-completion operations.
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    context,
    handoff::{self, Document, Format},
    store,
};

const PATH_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Idle,
    Export(Format),
    Import,
}

pub enum Action {
    Export {
        path: PathBuf,
        document: Document,
        format: Format,
    },
    Import {
        path: PathBuf,
    },
}

pub struct State {
    pub pending: Option<Action>,
    /// Only separately reviewed and accepted context is emitted here. The parent
    /// may attach it to planning, but must revalidate at preview AND dispatch.
    pub output: Option<Document>,
    /// Explicit copy into the existing editor; it is not an automatic save.
    pub decision: Option<String>,
    pub candidate: Option<Document>,
    pub notice: String,
    pub preview: String,
    pub scroll: u16,
    pub scroll_max: u16,
    pub reviewed: bool,
    pub affirmative: bool,
    pub editing: bool,
    pub path: String,
    pub cursor: usize,
    pub mode: Mode,
    pub selected_decision: usize,
    task_id: i64,
    accepted_import: bool,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            pending: None,
            output: None,
            decision: None,
            candidate: None,
            notice:
                "Local-only handoff. j JSON / m Markdown export, i import. No CLI or model starts."
                    .into(),
            preview: String::new(),
            scroll: 0,
            scroll_max: 0,
            reviewed: false,
            affirmative: false,
            editing: false,
            path: String::new(),
            cursor: 0,
            mode: Mode::Idle,
            selected_decision: 0,
            task_id: 0,
            accepted_import: false,
        }
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.editing || self.mode != Mode::Idle
    }

    pub fn needs_saved_input(&self, key: &KeyEvent) -> bool {
        key.kind == KeyEventKind::Press
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && ((self.mode == Mode::Idle && matches!(key.code, KeyCode::Char('j' | 'm')))
                || (matches!(self.mode, Mode::Export(_)) && key.code == KeyCode::Enter))
    }

    pub fn selected_import_decision(&self) -> Option<&str> {
        if !self.accepted_import || self.mode != Mode::Idle {
            return None;
        }
        self.candidate
            .as_ref()?
            .decisions
            .get(self.selected_decision)
            .map(String::as_str)
    }

    pub fn reset(&mut self, task_id: i64) {
        *self = Self::new();
        self.task_id = task_id;
    }

    pub fn invalidate_review(&mut self) {
        self.reviewed = false;
        self.affirmative = false;
    }

    fn review(&mut self, document: Document, format: Format) -> Result<(), String> {
        self.preview = handoff::encode(&document, format)?;
        self.candidate = Some(document);
        self.scroll = 0;
        self.scroll_max = 0;
        self.selected_decision = 0;
        self.accepted_import = false;
        self.invalidate_review();
        Ok(())
    }

    fn start(&mut self, mode: Mode) {
        self.mode = mode;
        self.editing = true;
        self.path.clear();
        self.cursor = 0;
        self.candidate = None;
        self.accepted_import = false;
        self.preview.clear();
        self.invalidate_review();
        self.notice = match mode {
            Mode::Import => "Enter a literal existing JSON/Markdown handoff path. Reading does not apply it or run a provider.",
            _ => "Choose a deliberate NEW file path. Existing files and symlink/reparse routes are refused. Enter opens review; no write yet.",
        }.into();
    }

    pub fn paste(&mut self, text: &str) -> bool {
        if !self.editing {
            return false;
        }
        if text.chars().any(|ch| ch.is_control()) || handoff::sanitize(text) != text {
            self.notice =
                "Path input refused: controls or recognizable credentials are not permitted."
                    .into();
            return true;
        }
        if text.len() > PATH_BYTES.saturating_sub(self.path.len()) {
            self.notice = "Path limit is 4096 UTF-8 bytes; paste was not inserted.".into();
            return true;
        }
        self.path.insert_str(self.cursor, text);
        if handoff::sanitize(&self.path) != self.path {
            self.path
                .replace_range(self.cursor..self.cursor + text.len(), "");
            self.notice =
                "Path input would contain a recognizable credential; insertion refused.".into();
            return true;
        }
        self.cursor += text.len();
        self.cursor = self
            .path
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|index| *index >= self.cursor)
            .unwrap_or(self.path.len());
        self.invalidate_review();
        !text.is_empty()
    }

    fn path_key(&mut self, key: KeyEvent) -> bool {
        if let KeyCode::Char(ch) = key.code
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return self.paste(ch.encode_utf8(&mut [0; 4]));
        }
        let before = self.path[..self.cursor]
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .next_back()
            .unwrap_or(0);
        let after = self
            .path
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|index| *index > self.cursor)
            .unwrap_or(self.path.len());
        match key.code {
            KeyCode::Left => self.cursor = before,
            KeyCode::Right => self.cursor = after,
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.path.len(),
            KeyCode::Backspace => {
                self.path.replace_range(before..self.cursor, "");
                self.cursor = before;
            }
            KeyCode::Delete => {
                self.path.replace_range(self.cursor..after, "");
            }
            _ => return false,
        }
        true
    }

    /// Executes only bounded, manual file I/O. Parent calls once per UI iteration;
    /// the import result is merely a candidate and cannot directly affect planning.
    pub fn process_pending(&mut self) {
        let Some(action) = self.pending.take() else {
            return;
        };
        match action {
            Action::Export {
                path,
                document,
                format,
            } => {
                self.mode = Mode::Idle;
                self.notice = match handoff::write_new(&path, &document, format) {
                    Ok(()) => "Reviewed handoff written to the chosen NEW file. This is not verified evidence. Eevee deletion cannot erase this export, copies, or provider-owned storage.".into(),
                    Err(error) => handoff::sanitize(&error),
                };
                self.invalidate_review();
            }
            Action::Import { path } => match handoff::read_file(&path)
                .and_then(|document| self.review(document, Format::Json))
            {
                Ok(()) => {
                    self.notice = "Import read only. All claims are user-supplied/unverified. Review every page, then choose Accept separately. No task fields, criteria, or evidence changed.".into();
                }
                Err(error) => {
                    self.mode = Mode::Idle;
                    self.candidate = None;
                    self.preview.clear();
                    self.notice = handoff::sanitize(&error);
                    self.invalidate_review();
                }
            },
        }
    }

    pub fn key(
        &mut self,
        key: KeyEvent,
        large_enough: bool,
        task: &store::Snapshot,
        excerpts: &[context::Excerpt],
        selected_decisions: &[String],
    ) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.kind == KeyEventKind::Repeat
            && !matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Backspace
                    | KeyCode::Delete
            )
        {
            return false;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        if self.task_id != task.id {
            self.reset(task.id);
        }
        if !large_enough {
            self.invalidate_review();
        }
        if self.pending.is_some() {
            return true;
        }
        if key.code == KeyCode::Esc && (self.busy() || self.candidate.is_some()) {
            self.mode = Mode::Idle;
            self.editing = false;
            self.candidate = None;
            self.accepted_import = false;
            self.scroll = 0;
            self.scroll_max = 0;
            self.preview.clear();
            self.invalidate_review();
            self.notice = "Closed this local review. No new write, provider request, or task change. Previously accepted planning context is unchanged.".into();
            return true;
        }
        if self.editing {
            if key.code != KeyCode::Enter {
                return self.path_key(key);
            }
            if self.path.trim().is_empty() {
                self.notice = "Choose a literal filename first.".into();
                return true;
            }
            match self.mode {
                Mode::Import => {
                    self.pending = Some(Action::Import {
                        path: PathBuf::from(&self.path),
                    });
                    self.editing = false;
                }
                Mode::Export(format) => {
                    match handoff::from_task(task, excerpts, selected_decisions)
                        .and_then(|document| self.review(document, format))
                    {
                        Ok(()) => {
                            self.editing = false;
                            self.notice = "Review the sanitized exact export bytes and chosen path. Stay is neutral; Right selects Write only after full review. No write yet.".into();
                        }
                        Err(error) => {
                            self.notice = handoff::sanitize(&error);
                        }
                    }
                }
                Mode::Idle => {}
            }
            return true;
        }
        if self.mode != Mode::Idle {
            match key.code {
                KeyCode::Left => self.affirmative = false,
                KeyCode::Right if self.reviewed && large_enough => self.affirmative = true,
                KeyCode::Down => self.scroll = self.scroll.saturating_add(1).min(self.scroll_max),
                KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                KeyCode::PageDown => {
                    self.scroll = self.scroll.saturating_add(8).min(self.scroll_max)
                }
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(8),
                KeyCode::Home => self.scroll = 0,
                KeyCode::End => self.scroll = self.scroll_max,
                KeyCode::Enter if self.affirmative && self.reviewed && large_enough => {
                    let Some(document) = self.candidate.clone() else {
                        return true;
                    };
                    match self.mode {
                        Mode::Import => {
                            self.output = Some(document);
                            self.accepted_import = true;
                            self.mode = Mode::Idle;
                            self.scroll = 0;
                            self.scroll_max = 0;
                            self.notice = "Explicitly accepted as user-supplied/unverified context only. Planning must recheck task/sources before sending; no session, grant, verification, or completion transferred.".into();
                            self.invalidate_review();
                        }
                        Mode::Export(format) => {
                            match handoff::from_task(task, excerpts, selected_decisions) {
                                Ok(current) if current == document => {
                                    self.pending = Some(Action::Export {
                                        path: PathBuf::from(&self.path),
                                        document,
                                        format,
                                    });
                                }
                                _ => {
                                    self.invalidate_review();
                                    self.notice = "Task, source basket, or selected decision changed since review. Stay and prepare a fresh export; no substituted bytes written.".into();
                                }
                            }
                        }
                        Mode::Idle => {}
                    }
                }
                KeyCode::Enter => {
                    self.notice = "Stay is selected. Review every page and deliberately select the action with Right; Esc cancels.".into();
                }
                _ => return false,
            }
            return true;
        }
        match key.code {
            KeyCode::Char('j') => self.start(Mode::Export(Format::Json)),
            KeyCode::Char('m') => self.start(Mode::Export(Format::Markdown)),
            KeyCode::Char('i') => self.start(Mode::Import),
            KeyCode::Char('c') if large_enough && self.accepted_import => {
                if let Some(decision) = self.selected_import_decision() {
                    if decision.len() > crate::app::MAX_GOAL_BYTES {
                        self.notice = "Selected decision exceeds the existing editor limit; no silent truncation or save.".into();
                    } else {
                        self.decision = Some(format!(
                            "{decision}\n\n[User-supplied imported decision; unverified. Not local execution evidence.]"
                        ));
                        // The parent editor limit applies to the final copied bytes too.
                        if self
                            .decision
                            .as_ref()
                            .is_some_and(|text| text.len() > crate::app::MAX_GOAL_BYTES)
                        {
                            self.decision = None;
                            self.notice = "Decision with required unverified label exceeds the editor limit; no copy.".into();
                        } else {
                            self.notice = "Selected unverified decision copied to the existing editor for separate review/save; task not completed.".into();
                        }
                    }
                } else {
                    self.notice = "No separately accepted imported decision selected.".into();
                }
            }
            KeyCode::Char('[') => {
                self.selected_decision = self.selected_decision.saturating_sub(1);
                self.scroll = 0;
                self.scroll_max = 0;
            }
            KeyCode::Char(']') => {
                let max = self
                    .candidate
                    .as_ref()
                    .map_or(0, |document| document.decisions.len().saturating_sub(1));
                self.selected_decision = self.selected_decision.saturating_add(1).min(max);
                self.scroll = 0;
                self.scroll_max = 0;
            }
            KeyCode::Down => self.scroll = self.scroll.saturating_add(1).min(self.scroll_max),
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(8).min(self.scroll_max),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(8),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.scroll_max,
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn task() -> store::Snapshot {
        let mut task = store::Snapshot::empty("project".into());
        task.id = 1;
        task.goal = "Reviewed task".into();
        task
    }

    #[test]
    fn export_requires_path_preview_review_and_deliberate_selection() {
        let task = task();
        let mut state = State::new();
        state.key(key(KeyCode::Char('j')), true, &task, &[], &[]);
        state.paste("chosen-new.json");
        state.key(key(KeyCode::Enter), true, &task, &[], &[]);
        assert!(state.candidate.is_some());
        assert!(state.pending.is_none());
        state.key(key(KeyCode::Enter), true, &task, &[], &[]);
        state.key(key(KeyCode::Right), true, &task, &[], &[]);
        assert!(!state.affirmative);
        state.reviewed = true;
        state.key(key(KeyCode::Right), true, &task, &[], &[]);
        state.key(
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat),
            true,
            &task,
            &[],
            &[],
        );
        assert!(
            state.pending.is_none(),
            "Repeated Enter cannot authorize a file write"
        );
        state.key(key(KeyCode::Enter), true, &task, &[], &[]);
        assert!(matches!(state.pending, Some(Action::Export { .. })));
        assert!(state.output.is_none());
    }

    #[test]
    fn import_review_is_separate_and_never_changes_task_or_evidence() {
        let task = task();
        let mut state = State::new();
        state.reset(task.id);
        state.mode = Mode::Import;
        state
            .review(
                handoff::from_task(&task, &[], &["Consider a simple design".into()]).unwrap(),
                Format::Json,
            )
            .unwrap();
        state.key(key(KeyCode::Enter), true, &task, &[], &[]);
        assert!(state.output.is_none());
        state.reviewed = true;
        state.key(key(KeyCode::Right), true, &task, &[], &[]);
        state.key(key(KeyCode::Enter), false, &task, &[], &[]);
        assert!(state.output.is_none());
        state.reviewed = true;
        state.key(key(KeyCode::Right), true, &task, &[], &[]);
        state.key(key(KeyCode::Enter), true, &task, &[], &[]);
        assert!(state.output.is_some());
    }

    #[test]
    fn changed_task_prevents_export_and_escape_is_neutral() {
        let mut task = task();
        let mut state = State::new();
        state.key(key(KeyCode::Char('m')), true, &task, &[], &[]);
        state.paste("new.md");
        state.key(key(KeyCode::Enter), true, &task, &[], &[]);
        state.reviewed = true;
        state.affirmative = true;
        task.revision += 1;
        state.key(key(KeyCode::Enter), true, &task, &[], &[]);
        assert!(state.pending.is_none());
        state.key(key(KeyCode::Esc), true, &task, &[], &[]);
        assert!(!state.busy());
        assert!(state.output.is_none());
    }

    #[test]
    fn accepted_import_copies_only_the_displayed_decision_without_saving() {
        let task = task();
        let mut state = State::new();
        state.reset(task.id);
        state.mode = Mode::Import;
        state
            .review(
                handoff::from_task(
                    &task,
                    &[],
                    &["First decision".into(), "Second decision".into()],
                )
                .unwrap(),
                Format::Json,
            )
            .unwrap();
        assert!(state.selected_import_decision().is_none());
        state.reviewed = true;
        state.affirmative = true;
        state.scroll = 80;
        state.key(key(KeyCode::Enter), true, &task, &[], &[]);
        assert_eq!(state.scroll, 0);
        assert_eq!(state.selected_import_decision(), Some("First decision"));
        state.scroll = 20;
        state.key(key(KeyCode::Char(']')), true, &task, &[], &[]);
        assert_eq!(state.scroll, 0);
        assert_eq!(state.selected_import_decision(), Some("Second decision"));
        state.key(key(KeyCode::Char('c')), true, &task, &[], &[]);
        let decision = state.decision.take().unwrap();
        assert!(decision.starts_with("Second decision"));
        assert!(decision.contains("unverified"));
        assert!(state.pending.is_none());
        assert_eq!(task.next_action, "");
        state.key(key(KeyCode::Esc), true, &task, &[], &[]);
        assert!(!state.busy());
        assert!(
            state.candidate.is_none(),
            "A second Escape must be able to close the drawer"
        );
        assert!(state.preview.is_empty());
        assert!(state.selected_import_decision().is_none());
        state.key(key(KeyCode::Char('i')), true, &task, &[], &[]);
        assert!(state.selected_import_decision().is_none());
    }

    #[test]
    fn exports_require_saved_input_but_import_review_does_not() {
        let task = task();
        let mut state = State::new();
        assert!(state.needs_saved_input(&key(KeyCode::Char('j'))));
        assert!(state.needs_saved_input(&key(KeyCode::Char('m'))));
        assert!(!state.needs_saved_input(&key(KeyCode::Char('i'))));
        state.key(key(KeyCode::Char('m')), true, &task, &[], &[]);
        assert!(state.needs_saved_input(&key(KeyCode::Enter)));
        state.key(key(KeyCode::Esc), true, &task, &[], &[]);
        state.key(key(KeyCode::Char('i')), true, &task, &[], &[]);
        assert!(!state.needs_saved_input(&key(KeyCode::Enter)));
    }

    #[test]
    fn failed_export_never_claims_a_successful_write_or_accepts_context() {
        let task = task();
        let mut state = State::new();
        state.reset(task.id);
        state.mode = Mode::Export(Format::Json);
        state.reviewed = true;
        state.affirmative = true;
        state.pending = Some(Action::Export {
            path: std::env::temp_dir(),
            document: handoff::from_task(&task, &[], &[]).unwrap(),
            format: Format::Json,
        });
        state.process_pending();
        assert_eq!(state.mode, Mode::Idle);
        assert!(!state.reviewed && !state.affirmative);
        assert!(state.output.is_none());
        assert!(state.decision.is_none());
        assert!(!state.notice.contains("Reviewed handoff written"));
    }
}
