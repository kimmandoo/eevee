//! Retained evidence navigation is an independent read-only surface, not an owner switch.
use super::*;
use crate::store::retained::{Hit, Landmark, Operation, Request, Response, Target};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use unicode_segmentation::UnicodeSegmentation;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Search,
    Responses,
    Landmarks,
    Evidence,
}
#[derive(Default)]
pub struct State {
    pub mode: Mode,
    pub query: String,
    pub archived: bool,
    pub hits: Vec<Hit>,
    pub selected: usize,
    pub incomplete: bool,
    pub scanned: usize,
    pub pending: Option<Request>,
    pub inflight: bool,
    pub error: String,
    pub notice: String,
    pub text: String,
    pub opened: Option<Hit>,
    pub position: usize,
    pub editor: Option<String>,
    pub rename: Option<usize>,
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
    task: i64,
    return_mode: Mode,
}
impl State {
    fn cancel(&mut self) {
        if let Some(flag) = self.cancel.take() {
            flag.store(true, Ordering::Relaxed);
        }
        self.pending = None;
        self.generation = self.generation.wrapping_add(1);
        self.inflight = false;
    }
    fn queue(&mut self, operation: Operation) {
        self.cancel();
        self.error.clear();
        self.notice.clear();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        self.pending = Some(Request {
            generation: self.generation,
            cancel,
            operation,
        });
    }
    pub fn response(&mut self, request: &Request, result: Result<Response, String>) {
        if request.generation != self.generation || request.cancel.load(Ordering::Relaxed) {
            return;
        }
        self.inflight = false;
        match result {
            Err(error) => {
                self.error = error;
                self.text.clear();
                self.opened = None;
            }
            Ok(Response::Hits {
                values,
                incomplete,
                scanned,
            }) => {
                self.hits = values;
                self.incomplete = incomplete;
                self.scanned = scanned;
                self.selected = 0;
            }
            Ok(Response::Opened {
                hit,
                text,
                position,
            }) => {
                self.return_mode = self.mode;
                self.mode = Mode::Evidence;
                self.position = position.min(text.chars().count().saturating_sub(1));
                self.text = text;
                self.opened = Some(hit);
            }
        }
    }
    pub fn invalidate_task(&mut self, task: i64) {
        if self.task == task
            || self.hits.iter().any(|h| h.task == task)
            || self.opened.as_ref().is_some_and(|h| h.task == task)
        {
            self.cancel();
            self.hits.clear();
            self.text.clear();
            self.opened = None;
            self.error =
                "Retained data changed or was purged; refresh before opening evidence".into();
        }
    }
    pub fn paste(&mut self, text: &str) {
        if self.editor.is_none() && self.mode == Mode::Search {
            self.cancel();
            self.hits.clear();
        }
        let (target, limit) = if let Some(editor) = &mut self.editor {
            (editor, 128)
        } else if self.mode == Mode::Search {
            (&mut self.query, 256)
        } else {
            return;
        };
        if target.len() + text.len() <= limit && !text.chars().any(char::is_control) {
            target.push_str(text);
        }
    }
}
impl App {
    pub(super) fn retained_command(&mut self, name: &str, argument: &str) -> bool {
        let mode = match name {
            "project-search" => Mode::Search,
            "landmarks" => Mode::Landmarks,
            "retained-responses" => Mode::Responses,
            _ => return false,
        };
        self.retained.cancel();
        self.retained.task = self.task_id;
        self.retained.mode = mode;
        self.retained.selected = 0;
        self.retained.editor = None;
        self.retained.error.clear();
        self.retained.opened = None;
        self.retained.text.clear();
        self.retained.hits.clear();
        self.retained.notice =
            "Local retained data only; no context selection, replay or owner switch".into();
        if mode == Mode::Search {
            self.retained.query = argument.into();
            if !argument.is_empty() {
                self.retained.queue(Operation::Search {
                    literal: argument.into(),
                    archived: self.retained.archived,
                });
            }
        }
        if matches!(mode, Mode::Responses | Mode::Landmarks) {
            self.retained
                .queue(Operation::Journals { task: self.task_id });
        }
        self.open_overlay(Overlay::Retained);
        true
    }
    pub(super) fn retained_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('k')
            && key.modifiers == KeyModifiers::CONTROL
        {
            self.open_commands();
            return true;
        }
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.code == KeyCode::Esc {
            if self.retained.editor.take().is_some() {
                self.retained.rename = None;
                return true;
            }
            if self.retained.mode == Mode::Evidence {
                self.retained.text.clear();
                self.retained.opened = None;
                self.retained.mode = self.retained.return_mode;
                return true;
            }
            self.retained.cancel();
            self.close_overlay();
            return true;
        }
        if self.width < 60 || self.height < 18 {
            return false;
        }
        if self.retained.task != self.task_id {
            self.retained.cancel();
            self.retained.error = "Task changed; reopen retained navigation".into();
            return true;
        }
        if let Some(editor) = &mut self.retained.editor {
            match key.code {
                KeyCode::Backspace => {
                    if let Some((i, _)) = editor.grapheme_indices(true).next_back() {
                        editor.truncate(i);
                    }
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    if editor.len() + c.len_utf8() <= 128 && !c.is_control() {
                        editor.push(c);
                    }
                }
                KeyCode::Enter if key.kind != KeyEventKind::Repeat => {
                    let mut values = self.workflows.landmarks.clone();
                    if let Some(index) = self.retained.rename {
                        if let Some(value) = values.get_mut(index) {
                            value.label = editor.clone();
                        }
                    } else if let Some(hit) = &self.retained.opened
                        && let Target::Journal(id) = &hit.target
                    {
                        values.push(Landmark {
                            label: editor.clone(),
                            journal_id: id.clone(),
                            position: Some(self.retained.position),
                        });
                    }
                    match crate::store::retained::validate_landmarks(&values) {
                        Err(error) => self.retained.error = error,
                        Ok(()) => {
                            self.workflows.landmarks = values;
                            self.retained.editor = None;
                            self.retained.rename = None;
                            self.changed_implicitly(std::time::Instant::now());
                            self.flush_save();
                            self.retained.notice="Landmark metadata staged for durable save; no transcript copied or request sent".into();
                        }
                    }
                }
                _ => {}
            }
            return true;
        }
        match key.code {
            KeyCode::Up => {
                if self.retained.mode == Mode::Evidence {
                    self.retained.position = self.retained.position.saturating_sub(40);
                } else {
                    self.retained.selected = self.retained.selected.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if self.retained.mode == Mode::Evidence {
                    self.retained.position = (self.retained.position + 40)
                        .min(self.retained.text.chars().count().saturating_sub(1));
                } else {
                    let len = if self.retained.mode == Mode::Landmarks {
                        self.workflows.landmarks.len()
                    } else {
                        self.retained.hits.len()
                    };
                    self.retained.selected =
                        (self.retained.selected + 1).min(len.saturating_sub(1));
                }
            }
            KeyCode::PageDown if self.retained.mode == Mode::Evidence => {
                self.retained.position = (self.retained.position + 400)
                    .min(self.retained.text.chars().count().saturating_sub(1))
            }
            KeyCode::PageUp if self.retained.mode == Mode::Evidence => {
                self.retained.position = self.retained.position.saturating_sub(400)
            }
            KeyCode::Tab if self.retained.mode == Mode::Search => {
                self.retained.cancel();
                self.retained.hits.clear();
                self.retained.archived = !self.retained.archived;
            }
            KeyCode::Enter if key.kind != KeyEventKind::Repeat => {
                let operation = match self.retained.mode {
                    Mode::Search if self.retained.hits.is_empty() => Some(Operation::Search {
                        literal: self.retained.query.clone(),
                        archived: self.retained.archived,
                    }),
                    Mode::Search | Mode::Responses => self
                        .retained
                        .hits
                        .get(self.retained.selected)
                        .cloned()
                        .map(|hit| Operation::Open { hit }),
                    Mode::Landmarks => self
                        .workflows
                        .landmarks
                        .get(self.retained.selected)
                        .cloned()
                        .map(|landmark| Operation::Jump {
                            task: self.task_id,
                            landmark,
                        }),
                    _ => None,
                };
                if let Some(operation) = operation {
                    self.retained.queue(operation);
                }
            }
            KeyCode::Char('l')
                if self.retained.mode == Mode::Evidence
                    && self.retained.opened.as_ref().is_some_and(|h| {
                        h.task == self.task_id && matches!(h.target, Target::Journal(_))
                    }) =>
            {
                self.retained.editor = Some(String::new());
                self.retained.rename = None;
            }
            KeyCode::Char('n') if self.retained.mode == Mode::Landmarks => {
                if let Some(value) = self.workflows.landmarks.get(self.retained.selected) {
                    self.retained.editor = Some(value.label.clone());
                    self.retained.rename = Some(self.retained.selected);
                }
            }
            KeyCode::Char('d')
                if self.retained.mode == Mode::Landmarks && key.kind == KeyEventKind::Press =>
            {
                if self.retained.selected < self.workflows.landmarks.len() {
                    self.workflows.landmarks.remove(self.retained.selected);
                    self.retained.selected = self
                        .retained
                        .selected
                        .min(self.workflows.landmarks.len().saturating_sub(1));
                    self.changed_implicitly(std::time::Instant::now());
                    self.flush_save();
                    self.retained.notice = "Landmark removed; retained response unchanged".into();
                }
            }
            KeyCode::Backspace if self.retained.mode == Mode::Search => {
                self.retained.cancel();
                self.retained.hits.clear();
                if let Some((i, _)) = self.retained.query.grapheme_indices(true).next_back() {
                    self.retained.query.truncate(i);
                }
            }
            KeyCode::Char(c)
                if self.retained.mode == Mode::Search
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.retained.cancel();
                self.retained.hits.clear();
                if self.retained.query.len() + c.len_utf8() <= 256 && !c.is_control() {
                    self.retained.query.push(c);
                }
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
#[path = "retained_tests.rs"]
mod tests;
