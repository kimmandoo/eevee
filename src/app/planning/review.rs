use std::{path::PathBuf, sync::Arc};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde_json::{Value, json};
use unicode_segmentation::UnicodeSegmentation;

use super::{ANSWER_BYTES, Agent, ModelSelection, Operation, Picker, PromptKind, Request, State};
use crate::{
    providers::Input,
    store,
    workspace::{context, repo},
};

impl State {
    pub fn invalidate_review(&mut self) {
        self.reviewed = false;
        self.affirmative = false;
        if matches!(
            self.request.as_ref().map(|request| &request.kind),
            Some(PromptKind::Permission { .. })
        ) {
            self.option = 0;
        }
    }

    pub(super) fn review_start(&mut self) {
        self.scroll = 0;
        self.scroll_max = 0;
        self.invalidate_review();
    }

    fn paths_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Esc {
            self.paths_editor = None;
            return true;
        }
        if key.code == KeyCode::Enter {
            let value = &self.paths_editor.as_ref().expect("path editor").text;
            let paths: Result<Vec<String>, _> = serde_json::from_str(value);
            match paths {
                Ok(paths) if !paths.is_empty() && paths.len() <= 16
                    && paths.iter().enumerate().all(|(index, path)| {
                        !path.is_empty() && path.len() <= 4096
                            && !path.chars().any(char::is_control)
                            && PathBuf::from(path).components().all(|part| matches!(part, std::path::Component::Normal(_)))
                            && !paths[..index].contains(path)
                    }) => {
                    self.edit_paths = paths.into_iter().map(PathBuf::from).collect();
                    self.paths_editor = None;
                    self.notice = "Edit paths selected locally; no write or permission granted. Review exact scope before Run.".into();
                }
                _ => self.notice = "Use a JSON array of 1–16 distinct root-relative paths; no parent/absolute/control components.".into(),
            }
            return true;
        }
        if let KeyCode::Char(ch) = key.code
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return self.paste(ch.encode_utf8(&mut [0; 4]));
        }
        let answer = self.paths_editor.as_mut().expect("path editor");
        let before = answer.text[..answer.cursor]
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .next_back()
            .unwrap_or(0);
        let after = answer
            .text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|index| *index > answer.cursor)
            .unwrap_or(answer.text.len());
        match key.code {
            KeyCode::Home => answer.cursor = 0,
            KeyCode::End => answer.cursor = answer.text.len(),
            KeyCode::Left => answer.cursor = before,
            KeyCode::Right => answer.cursor = after,
            KeyCode::Backspace => {
                answer.text.replace_range(before..answer.cursor, "");
                answer.cursor = before;
            }
            KeyCode::Delete => {
                answer.text.replace_range(answer.cursor..after, "");
            }
            _ => return false,
        }
        true
    }

    pub(super) fn close_request(&mut self) {
        self.request = None;
        self.answers.clear();
        self.editing = false;
        self.submit_review = false;
        self.scroll = self.saved_scroll;
        self.invalidate_review();
    }

    pub fn request_quit(&mut self) {
        self.quit_review = true;
        self.invalidate_review();
        self.editing = false;
    }

    pub(super) fn prepare(
        &mut self,
        resume: bool,
        inspect_sessions: bool,
        inspect_models: bool,
        task: &store::Snapshot,
        excerpts: &[context::Excerpt],
        report: Option<&Arc<repo::Report>>,
    ) {
        if self.busy() {
            return;
        }
        if !inspect_models && self.operation == Operation::Review && (resume || inspect_sessions) {
            self.notice = "Review requires a fresh session and freshly constructed evidence context. Resume/session inspection is unavailable here; choose New.".into();
            return;
        }
        if self.journal_failed() {
            self.notice =
                "Retry failed journal storage before preparing another provider request.".into();
            return;
        }
        if task.id <= 0 || task.archived || (!inspect_models && task.goal.trim().is_empty()) {
            self.notice = if inspect_models {
                "Save an active task before model inspection.".into()
            } else {
                "Save a nonempty goal in an active task before planning.".into()
            };
            return;
        }
        if resume && (self.task != task.id || self.thread_id.is_none()) {
            self.notice =
                "Resume requires an explicitly selected same-task thread from this session.".into();
            return;
        }
        if resume && self.session_selection.model != self.selection.model {
            self.notice = "This session belongs to a different model selection. Choose /new for a fresh session or /model to keep the session's original selection; no resume was prepared.".into();
            return;
        }
        if self.task != task.id {
            self.reset(task.id);
        }
        self.revision = task.revision;
        self.inspect_sessions = inspect_sessions;
        self.inspect_models = inspect_models;
        self.request_selection = if inspect_models {
            ModelSelection {
                model: if matches!(self.agent, Agent::OpenCode | Agent::Gemini | Agent::Devin) {
                    self.selection.model.clone()
                } else {
                    None
                },
                effort: None,
            }
        } else {
            self.selection.clone()
        };
        self.inspection = None;
        self.pending = Some(Request::Preview(Box::new(Input {
            task: task.clone(),
            excerpts: excerpts.to_vec(),
            report: report.cloned(),
            resume: if resume { self.thread_id.clone() } else { None },
            agent: self.agent,
            operation: self.operation,
            edit_paths: if self.operation == Operation::Review {
                Vec::new()
            } else {
                self.edit_paths.clone()
            },
            handoff: if self.operation == Operation::Review {
                None
            } else {
                self.handoff.clone()
            },
            inspect_sessions,
            inspect_models,
            selection: self.request_selection.clone(),
            credential: if self.agent == Agent::Devin {
                self.devin_credential.clone()
            } else {
                None
            },
        })));
        self.awaiting_preview = true;
        self.notice =
            "Preparing local preview. No provider executable or connection is authorized.".into();
        self.saved_scroll = self.scroll;
        self.review_start();
    }

    /// Request the automatic catalogue fetch; the next call to
    /// `start_automatic_catalogue` runs it once nothing else is in flight.
    pub fn want_catalogue(&mut self) {
        self.auto_models = true;
    }

    /// Whether an automatic catalogue fetch is wanted and the request slot
    /// is free (an open picker is replaced by the fetch, then reopened).
    pub fn catalogue_due(&self) -> bool {
        self.auto_models && !self.busy_except_picker() && !self.journal_failed()
    }

    /// Prepare the metadata-only catalogue preview without opening a review;
    /// `Event::Preview` then runs it directly unless it reports blockers.
    pub fn start_automatic_catalogue(
        &mut self,
        task: &store::Snapshot,
        excerpts: &[context::Excerpt],
        report: Option<&Arc<repo::Report>>,
    ) {
        self.auto_models = false;
        self.picker = None;
        self.prepare(false, false, true, task, excerpts, report);
        if self.awaiting_preview {
            self.auto_run = true;
            self.notice = "Loading models…".into();
        } else {
            self.auto_picker = false;
        }
    }

    pub(super) fn run_automatic_catalogue(&mut self) {
        let Some(preview) = self.preview.as_ref() else {
            return;
        };
        if let Some(blocker) = preview.blockers.first() {
            self.auto_blocked = Some(format!("Models not loaded: {blocker}"));
            self.auto_picker = false;
            self.cancel();
            return;
        }
        self.pending = Some(Request::Run {
            generation: preview.generation,
        });
        self.preview = None;
        self.active = true;
        self.review_start();
        self.notice = "Loading models…".into();
    }

    pub fn paste(&mut self, text: &str) -> bool {
        if let Some(answer) = &mut self.paths_editor {
            let remaining = ANSWER_BYTES.saturating_sub(answer.text.len());
            let mut inserted = String::new();
            for ch in text.chars().filter(|ch| !ch.is_control()) {
                if inserted.len() + ch.len_utf8() > remaining {
                    break;
                }
                inserted.push(ch);
            }
            answer.text.insert_str(answer.cursor, &inserted);
            answer.cursor += inserted.len();
            return !inserted.is_empty();
        }
        if !self.editing || self.quit_review || self.cancelling {
            return false;
        }
        let Some(answer) = self.answers.get_mut(self.question) else {
            return false;
        };
        let remaining = ANSWER_BYTES.saturating_sub(answer.text.len());
        let mut inserted = String::new();
        let mut truncated = false;
        for ch in text.chars() {
            if ch.is_control() {
                continue;
            }
            if inserted.len() + ch.len_utf8() > remaining {
                truncated = true;
                break;
            }
            inserted.push(ch);
        }
        answer.text.insert_str(answer.cursor, &inserted);
        answer.cursor += inserted.len();
        if answer.cursor < answer.text.len() {
            answer.cursor = answer
                .text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .find(|i| *i >= answer.cursor)
                .unwrap_or(answer.text.len());
        }
        if truncated {
            self.notice =
                "Answer limit: 4096 UTF-8 bytes. Excess pasted text was not inserted.".into();
        }
        self.affirmative = false;
        !inserted.is_empty() || truncated
    }

    fn editor_key(&mut self, key: KeyEvent) -> bool {
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
            self.editing = false;
            self.affirmative = false;
            return true;
        }
        if let KeyCode::Char(ch) = key.code
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return self.paste(ch.encode_utf8(&mut [0; 4]));
        }
        let Some(answer) = self.answers.get_mut(self.question) else {
            return false;
        };
        let before = answer.text[..answer.cursor]
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .next_back()
            .unwrap_or(0);
        let after = answer
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i > answer.cursor)
            .unwrap_or(answer.text.len());
        match key.code {
            KeyCode::Left => answer.cursor = before,
            KeyCode::Right => answer.cursor = after,
            KeyCode::Home => answer.cursor = 0,
            KeyCode::End => answer.cursor = answer.text.len(),
            KeyCode::Backspace => {
                answer.text.replace_range(before..answer.cursor, "");
                answer.cursor = before;
            }
            KeyCode::Delete => {
                answer.text.replace_range(answer.cursor..after, "");
            }
            _ => return false,
        }
        if answer.cursor < answer.text.len() {
            answer.cursor = answer
                .text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .take_while(|i| *i <= answer.cursor)
                .last()
                .unwrap_or(0);
        }
        true
    }

    fn send_answer(&mut self, answer: Value) {
        if self.pending.is_some() || self.cancelling {
            return;
        }
        let (Some(generation), Some(request)) = (self.generation, self.request.as_ref()) else {
            return;
        };
        self.pending = Some(Request::Answer {
            generation,
            request_id: request.request_id.clone(),
            answer,
        });
        self.close_request();
        self.notice = "Explicit response queued for the current provider request.".into();
    }
    pub fn key(
        &mut self,
        key: KeyEvent,
        large_enough: bool,
        task: &store::Snapshot,
        excerpts: &[context::Excerpt],
        report: Option<&Arc<repo::Report>>,
    ) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if self.task != task.id && !self.busy() && !self.journal_failed() {
            self.reset(task.id);
        }
        if !large_enough && !self.quit_review {
            self.invalidate_review();
        }
        // Repeated navigation may scroll, but cannot authorize or terminate a request.
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
        if self.quit_review {
            match key.code {
                KeyCode::Esc => {
                    self.quit_review = false;
                    self.affirmative = false;
                }
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    self.affirmative = !self.affirmative
                }
                KeyCode::Enter if key.kind == KeyEventKind::Press => {
                    self.quit_review = false;
                    if self.affirmative {
                        self.quit_after_cancel = true;
                        self.cancel();
                        if !self.active && !self.awaiting_preview && !self.cancelling {
                            self.quit_after_cancel = false;
                            self.quit_ready = true;
                        }
                    }
                    self.affirmative = false;
                }
                _ => return false,
            }
            return true;
        }
        if self.paths_editor.is_some() {
            return if large_enough || key.code == KeyCode::Esc {
                self.paths_key(key)
            } else {
                false
            };
        }
        if key.code == KeyCode::Esc && self.journal_mode {
            if self.journal_purge {
                self.journal_purge = false;
                self.affirmative = false;
            } else {
                self.journal_mode = false;
            }
            return true;
        }
        if key.code == KeyCode::Esc && self.inspection.is_some() {
            self.inspection = None;
            if self.active {
                self.cancel();
            } else {
                self.inspect_sessions = false;
            }
            self.review_start();
            return true;
        }
        if self.editing {
            return self.editor_key(key);
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        if self.picker.is_some() {
            if key.code == KeyCode::Esc {
                if self.selection_change.take().is_none() {
                    self.picker = None;
                }
                self.affirmative = false;
                return true;
            }
            if !large_enough {
                return false;
            }
            if self.selection_change.is_some() {
                match key.code {
                    KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                        self.affirmative = !self.affirmative
                    }
                    KeyCode::Enter => {
                        let selection = self.selection_change.take().expect("selection change");
                        if self.affirmative {
                            self.apply_selection(selection, true);
                        } else {
                            self.picker = None;
                        }
                        self.affirmative = false;
                    }
                    _ => return false,
                }
                return true;
            }
            let count = self.picker_len();
            match key.code {
                KeyCode::Up => self.picker_selected = self.picker_selected.saturating_sub(1),
                KeyCode::Down => {
                    self.picker_selected = (self.picker_selected + 1).min(count.saturating_sub(1))
                }
                KeyCode::Home => self.picker_selected = 0,
                KeyCode::End => self.picker_selected = count.saturating_sub(1),
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(5),
                KeyCode::PageDown => {
                    self.scroll = self.scroll.saturating_add(5).min(self.scroll_max)
                }
                KeyCode::Enter => {
                    let Some(row) = self.picker_choice(self.picker_selected) else {
                        return false;
                    };
                    if let Some(reason) = &row.reason {
                        self.notice = reason.clone();
                        return true;
                    }
                    if row.command == "models-refresh" {
                        return self.command(
                            "models-refresh",
                            large_enough,
                            task,
                            excerpts,
                            report,
                        );
                    }
                    let mut selection = self.selection.clone();
                    match self.picker {
                        Some(Picker::Model) => {
                            selection.model = if self.picker_selected == 0 {
                                None
                            } else {
                                self.catalogues
                                    .get(&self.agent)
                                    .and_then(|catalogue| {
                                        catalogue.models.get(self.picker_selected - 1)
                                    })
                                    .map(|model| model.id.clone())
                            };
                            if selection.model != self.selection.model {
                                selection.effort = None;
                            }
                        }
                        Some(Picker::Effort) => {
                            selection.effort = if self.picker_selected == 0 {
                                None
                            } else {
                                self.effort_model()
                                    .and_then(|model| model.efforts.get(self.picker_selected - 1))
                                    .cloned()
                            };
                        }
                        None => return false,
                    }
                    if selection == self.selection {
                        self.picker = None;
                        return true;
                    }
                    if selection.model != self.selection.model && self.thread_id.is_some() {
                        self.selection_change = Some(selection);
                        self.affirmative = false;
                    } else {
                        self.apply_selection(selection, false);
                    }
                }
                _ => return false,
            }
            return true;
        }
        if key.code == KeyCode::Esc {
            return self.cancel();
        }
        if !large_enough {
            return false;
        }
        match key.code {
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(5);
                return true;
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(5).min(self.scroll_max);
                return true;
            }
            KeyCode::Home => {
                self.scroll = 0;
                return true;
            }
            KeyCode::End => {
                self.scroll = self.scroll_max;
                return true;
            }
            _ => {}
        }
        if self.switch_to.is_some() {
            match key.code {
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right
                    if self.reviewed =>
                {
                    self.affirmative = !self.affirmative
                }
                KeyCode::Enter if self.reviewed && self.affirmative => self.complete_switch(),
                KeyCode::Enter => {
                    self.switch_to = None;
                    self.invalidate_review();
                }
                KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                KeyCode::Down => self.scroll = self.scroll.saturating_add(1).min(self.scroll_max),
                _ => return false,
            }
            return true;
        }
        if self.journal_mode {
            match key.code {
                KeyCode::Tab | KeyCode::Left | KeyCode::Right if self.journal_purge => {
                    self.affirmative = !self.affirmative
                }
                KeyCode::Enter if self.journal_purge => {
                    if self.affirmative {
                        self.pending_journal.push_back(store::JournalRequest {
                            task_id: self.task,
                            operation: store::JournalOperation::Purge,
                        });
                    }
                    self.journal_purge = false;
                    self.affirmative = false;
                }
                KeyCode::Up if !self.journal_purge => {
                    self.journal_selected = self.journal_selected.saturating_sub(1);
                    self.scroll = 0;
                    self.scroll_max = 0;
                }
                KeyCode::Down if !self.journal_purge => {
                    self.journal_selected = (self.journal_selected + 1)
                        .min(self.journal_records.len().saturating_sub(1));
                    self.scroll = 0;
                    self.scroll_max = 0;
                }
                _ => return false,
            }
            return true;
        }
        if self.preview.is_some() {
            match key.code {
                KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                KeyCode::Down => self.scroll = self.scroll.saturating_add(1).min(self.scroll_max),
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right
                    if self.reviewed =>
                {
                    self.affirmative = !self.affirmative
                }
                KeyCode::Enter if !self.affirmative => {
                    self.cancel();
                }
                KeyCode::Enter if self.reviewed && self.pending.is_none() => {
                    let preview = self.preview.as_ref().expect("preview is open");
                    if preview.blockers.is_empty() {
                        let generation = preview.generation;
                        if preview.thread_id.is_none()
                            && !self.inspect_sessions
                            && !self.inspect_models
                        {
                            self.items.clear();
                            self.selected = 0;
                            self.omitted = 0;
                            self.thread_id = None;
                            self.observed_model = None;
                        }
                        self.pending = Some(Request::Run { generation });
                        self.preview = None;
                        self.active = true;
                        self.review_start();
                        self.notice = "Explicit trust and run consent given for this exact preview. Connecting to provider.".into();
                    } else {
                        self.notice = "Run blocked. Review prerequisites; no executable launch is authorized.".into();
                    }
                }
                _ => return false,
            }
            return true;
        }
        if let Some(request) = &self.request {
            match &request.kind {
                PromptKind::Permission { options, .. } => match key.code {
                    KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right
                        if self.reviewed =>
                    {
                        self.option = if options.is_empty() {
                            0
                        } else {
                            (self.option + 1) % options.len()
                        };
                    }
                    KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                    KeyCode::Down => {
                        self.scroll = self.scroll.saturating_add(1).min(self.scroll_max)
                    }
                    KeyCode::Enter if self.option == 0 || self.reviewed => {
                        if let Some(option) = options.get(self.option) {
                            self.send_answer(option.answer.clone());
                        }
                    }
                    _ => return false,
                },
                PromptKind::Approval {
                    allow_once,
                    deny_decision,
                    ..
                } => match key.code {
                    KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                    KeyCode::Down => {
                        self.scroll = self.scroll.saturating_add(1).min(self.scroll_max)
                    }
                    KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right
                        if self.reviewed && *allow_once =>
                    {
                        self.affirmative = !self.affirmative
                    }
                    KeyCode::Enter if !self.affirmative => {
                        self.send_answer(json!({"decision":deny_decision}))
                    }
                    KeyCode::Enter if self.reviewed && *allow_once => {
                        self.send_answer(json!({"decision":"accept"}))
                    }
                    _ => return false,
                },
                PromptKind::Questions(questions) => {
                    if self.submit_review {
                        match key.code {
                            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right
                                if self.reviewed =>
                            {
                                self.affirmative = !self.affirmative
                            }
                            KeyCode::Enter if !self.affirmative => {
                                self.submit_review = false;
                                self.review_start();
                            }
                            KeyCode::Enter if self.reviewed => {
                                let mut values = serde_json::Map::new();
                                for (question, answer) in questions.iter().zip(&self.answers) {
                                    values.insert(
                                        question.id.clone(),
                                        json!({"answers":[answer.text]}),
                                    );
                                }
                                self.send_answer(json!({"answers": values}));
                            }
                            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                            KeyCode::Down => {
                                self.scroll = self.scroll.saturating_add(1).min(self.scroll_max)
                            }
                            _ => return false,
                        }
                        return true;
                    }
                    let Some(question) = questions.get(self.question) else {
                        return false;
                    };
                    let choices = question.options.len()
                        + usize::from(question.other || question.options.is_empty());
                    match key.code {
                        KeyCode::Up => self.option = self.option.saturating_sub(1),
                        KeyCode::Down => {
                            self.option =
                                self.option.saturating_add(1).min(choices.saturating_sub(1))
                        }
                        KeyCode::Left | KeyCode::BackTab => {
                            self.question = self.question.saturating_sub(1);
                            self.option = 0;
                            self.review_start();
                        }
                        KeyCode::Right | KeyCode::Tab => {
                            self.question =
                                (self.question + 1).min(questions.len().saturating_sub(1));
                            self.option = 0;
                            self.review_start();
                        }
                        KeyCode::Enter => {
                            if let Some(option) = question.options.get(self.option) {
                                self.answers[self.question].text = option.label.clone();
                                self.answers[self.question].cursor = option.label.len();
                            } else {
                                self.editing = true;
                            }
                        }
                        _ => return false,
                    }
                }
            }
            return true;
        }
        match key.code {
            KeyCode::Up if self.inspection.is_none() => {
                self.selected = self.selected.saturating_sub(1);
                self.scroll = 0;
            }
            KeyCode::Down if self.inspection.is_none() => {
                self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
                self.scroll = 0;
            }
            _ => return false,
        }
        true
    }
}
