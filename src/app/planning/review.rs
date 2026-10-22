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

impl super::Answer {
    fn line_key(&mut self, key: KeyCode) -> bool {
        let before = self.text[..self.cursor]
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .next_back()
            .unwrap_or(0);
        let after = self
            .text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|index| *index > self.cursor)
            .unwrap_or(self.text.len());
        match key {
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.len(),
            KeyCode::Left => self.cursor = before,
            KeyCode::Right => self.cursor = after,
            KeyCode::Backspace => {
                self.text.replace_range(before..self.cursor, "");
                self.cursor = before;
            }
            KeyCode::Delete => self.text.replace_range(self.cursor..after, ""),
            _ => return false,
        }
        true
    }

    fn insert_line(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.cursor = self
            .text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|index| *index >= self.cursor)
            .unwrap_or(self.text.len());
    }
}

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
        self.review_details = false;
        self.auto_file_review = false;
        self.scroll = 0;
        self.scroll_max = 0;
        self.response_cache.invalidate();
        self.invalidate_review();
    }

    fn custom_model_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Esc {
            self.custom_model = None;
            return true;
        }
        if key.code == KeyCode::Enter {
            let answer = self.custom_model.take().expect("model editor");
            if !self.select_model_id(&answer.text) {
                self.custom_model = Some(answer);
            }
            return true;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('u') {
            self.custom_model = Some(super::Answer::default());
            return true;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        if let KeyCode::Char(ch) = key.code {
            return self.paste(ch.encode_utf8(&mut [0; 4]));
        }
        self.custom_model
            .as_mut()
            .expect("model editor")
            .line_key(key.code)
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
                Ok(paths) if paths.len() <= 16
                    && paths.iter().enumerate().all(|(index, path)| {
                        !path.is_empty() && path.len() <= 4096
                            && !path.chars().any(char::is_control)
                            && PathBuf::from(path).components().all(|part| matches!(part, std::path::Component::Normal(_)))
                            && !paths[..index].contains(path)
                    }) => {
                    self.edit_paths = paths.into_iter().map(PathBuf::from).collect();
                    self.paths_editor = None;
                    self.notice = if self.edit_paths.is_empty() {
                        "Exact path restriction cleared locally; Implement may propose workspace paths. No files captured or created, request sent, or permission granted."
                    } else {
                        "Exact create/replace paths selected locally, including missing parents; no files captured or created, request sent, or permission granted."
                    }.into();
                }
                _ => self.notice = "Use a JSON array of up to 16 distinct root-relative paths, or [] for workspace proposals; no parent/absolute/control components.".into(),
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
        self.send_after_prepare = false;
        self.auto_file_review = false;
        let was_open = self.request.take().is_some();
        self.answers.clear();
        self.editing = false;
        self.submit_review = false;
        if was_open {
            self.scroll = self.saved_scroll;
        }
        self.invalidate_review();
    }

    pub fn request_quit(&mut self) {
        self.quit_review = true;
        self.auto_file_review = false;
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
        if self.busy() || self.unknown_effects() {
            return;
        }
        if self.preferences_loading() {
            self.notice =
                "Restoring the last provider/model preference before preparing a connection…"
                    .into();
            return;
        }
        if let Some(error) = self.preference_restore_error() {
            self.notice = error.to_owned();
            return;
        }
        self.catalogue_dismissed = false;
        self.send_after_prepare = false;
        if !inspect_models && self.agent.is_direct() && (resume || inspect_sessions) {
            self.notice = "Native history is process-local, not a resumable provider session. Send the next Request normally, or use /chat-new to reset local context. No inspection or resume was prepared.".into();
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
        if !inspect_models && (task.id <= 0 || task.archived || task.goal.trim().is_empty()) {
            self.notice = "Save a nonempty goal in an active task before planning.".into();
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
        self.receipt_claim = None;
        self.submitted_instruction = None;
        self.receipt.clear();
        self.observed_model = None;
        self.revision = task.revision;
        self.inspect_sessions = inspect_sessions;
        self.inspect_models = inspect_models;
        self.request_selection = if inspect_models {
            self.catalogue_pending = None;
            self.catalogue_errors.remove(&self.agent);
            ModelSelection::default()
        } else {
            self.selection.clone()
        };
        self.native_candidate = None;
        self.connection_invalidated = false;
        self.native_request_partner = if self.agent.is_direct() && !inspect_models {
            Some(task.lead_partner)
        } else {
            None
        };
        if !inspect_models && !inspect_sessions {
            self.submitted_request = if self.agent.is_direct() {
                crate::providers::native_request_text(task)
            } else {
                task.request_text()
            }
            .into();
        }
        self.inspection = None;
        let request_task = if inspect_models {
            // Keep only local event ownership and project identity, never task content.
            let mut metadata = store::Snapshot::empty(task.project.clone());
            metadata.id = task.id;
            metadata.revision = task.revision;
            metadata
        } else {
            task.clone()
        };
        self.pending = Some(Request::Preview(Box::new(Input {
            task: request_task,
            work: None,
            instruction_override: None,
            excerpts: if inspect_models {
                Vec::new()
            } else {
                excerpts.to_vec()
            },
            report: if inspect_models {
                None
            } else {
                report.cloned()
            },
            resume: if resume { self.thread_id.clone() } else { None },
            agent: self.agent,
            operation: if inspect_models {
                Operation::Plan
            } else {
                self.operation
            },
            edit_paths: if inspect_models
                || inspect_sessions
                || self.operation != Operation::Implement
            {
                Vec::new()
            } else {
                self.edit_paths.clone()
            },
            handoff: if inspect_models || self.operation == Operation::Review {
                None
            } else {
                self.handoff.clone()
            },
            inspect_sessions,
            inspect_models,
            selection: self.request_selection.clone(),
            conversation: if self.agent.is_direct()
                && !inspect_models
                && self.operation != Operation::Review
            {
                self.native_histories
                    .get(&(self.agent, self.operation))
                    .cloned()
            } else {
                None
            },
            credential: if self.agent == Agent::Devin {
                self.devin_credential.clone()
            } else {
                None
            },
        })));
        self.bind_slot(task.id);
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

    /// Whether an automatic catalogue fetch is wanted and the request slot is free.
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
        let picker = self.picker.take();
        self.prepare(false, false, true, task, excerpts, report);
        if let Some(picker) = picker {
            self.picker = Some(picker);
            self.filter_picker();
        } else if self.auto_picker {
            self.open_picker(Picker::Model);
        }
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
            let picker = self.picker.take();
            self.cancel();
            self.picker = picker;
            return;
        }
        if preview.requires_connection_review {
            self.auto_run = false;
            self.picker = None;
            self.notice =
                "Review this project-supplied runtime or unmanaged connection before it executes."
                    .into();
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

    pub(in crate::app) fn dispatch_prepared(&mut self) -> bool {
        let Some(preview) = self.preview.as_ref() else {
            return false;
        };
        if !preview.blockers.is_empty() || self.pending.is_some() {
            return false;
        }
        let generation = preview.generation;
        if preview.thread_id.is_none() && !self.inspect_sessions && !self.inspect_models {
            if self.items.is_empty() {
                self.following = true;
                self.new_output = false;
                self.saved_scroll = 0;
                self.response_scroll = 0;
                self.response_max = 0;
            }
            self.thread_id = None;
            self.observed_model = None;
        }
        if !self.inspect_models && !self.inspect_sessions {
            self.remember_used_selection();
        }
        self.pending = Some(Request::Run { generation });
        self.preview = None;
        self.send_after_prepare = false;
        self.active = true;
        self.activity = "Connecting";
        self.review_start();
        self.scroll = self.saved_scroll;
        self.notice = "Sending request to the selected provider.".into();
        if self.inspect_models {
            self.open_picker(Picker::Model);
            self.notice =
                "Loading provider metadata only; no request text or tools are sent.".into();
        }
        true
    }

    pub fn paste(&mut self, text: &str) -> bool {
        if self.auto_file_review {
            return false;
        }
        if self.custom_model.is_some() {
            let answer = self.custom_model.as_mut().expect("model editor");
            if text.chars().any(char::is_control) || answer.text.len() + text.len() > 256 {
                self.notice = "Custom model ID paste rejected: use at most 256 bytes without control characters. Nothing was submitted.".into();
                return true;
            }
            answer.insert_line(text);
            return true;
        }
        if self.picker.is_some() && self.review_details {
            return true;
        }
        if self.picker == Some(Picker::Model) && self.selection_change.is_none() {
            let remaining = 256usize.saturating_sub(self.picker_query.text.len());
            let mut end = 0;
            for (index, ch) in text.char_indices() {
                if ch.is_control() || index + ch.len_utf8() > remaining {
                    break;
                }
                end = index + ch.len_utf8();
            }
            self.picker_query.insert_line(&text[..end]);
            self.filter_picker();
            return true;
        }
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
        if self.auto_file_review || self.pending.is_some() || self.cancelling {
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
        self.promote_prompt();
        self.notice = "Explicit response queued for the current provider request.".into();
    }
    pub fn key(
        &mut self,
        mut key: KeyEvent,
        large_enough: bool,
        task: &store::Snapshot,
        excerpts: &[context::Excerpt],
        report: Option<&Arc<repo::Report>>,
    ) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if self.auto_file_review {
            if large_enough
                && key.kind == KeyEventKind::Press
                && key.modifiers.is_empty()
                && key.code == KeyCode::Enter
                && matches!(
                    self.request.as_ref().map(|request| &request.kind),
                    Some(PromptKind::Permission { file: Some(_), .. })
                )
            {
                self.auto_file_review = false;
                self.review_details = false;
                self.scroll = 0;
                self.invalidate_review();
                return true;
            }
            // Request typing cannot become an answer on asynchronous arrival.
            // App owns Esc deferral and its exact request identity.
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
            if key.kind == KeyEventKind::Press
                && key.modifiers.is_empty()
                && self.reviewed
                && large_enough
                && matches!(key.code, KeyCode::Char('y' | 'n'))
            {
                self.affirmative = key.code == KeyCode::Char('y');
                key.code = KeyCode::Enter;
            }
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
        if self.custom_model.is_some() {
            return if large_enough || key.code == KeyCode::Esc {
                self.custom_model_key(key)
            } else {
                false
            };
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
        if self.picker == Some(Picker::Model)
            && self.selection_change.is_none()
            && !self.review_details
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('u')
        {
            self.picker_query = super::Answer::default();
            self.filter_picker();
            return true;
        }
        if self.picker.is_some()
            && self.selection_change.is_none()
            && large_enough
            && key.modifiers == KeyModifiers::CONTROL
            && key.code == KeyCode::Char('d')
        {
            self.review_details = !self.review_details;
            self.scroll = 0;
            return true;
        }
        if large_enough
            && self.response_surface()
            && key.modifiers == KeyModifiers::ALT
            && matches!(
                key.code,
                KeyCode::Up | KeyCode::Down | KeyCode::Home | KeyCode::End
            )
        {
            return self.response_key(key.code);
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        if self.picker.is_some() || self.selection_change.is_some() {
            if key.code == KeyCode::Esc {
                if self.review_details && self.selection_change.is_none() {
                    self.review_details = false;
                    self.scroll = 0;
                } else if self.selection_change.take().is_none() {
                    self.picker = None;
                    self.pending_model = None;
                    self.model_picker_return = None;
                    self.notice = "Kept current model and reasoning.".into();
                }
                self.affirmative = false;
                return true;
            }
            if !large_enough {
                return false;
            }
            if self.selection_change.is_some() {
                if key.kind == KeyEventKind::Press
                    && key.modifiers.is_empty()
                    && self.reviewed
                    && matches!(key.code, KeyCode::Char('y' | 'n'))
                {
                    self.affirmative = key.code == KeyCode::Char('y');
                    key.code = KeyCode::Enter;
                }
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
                            self.pending_model = None;
                            self.model_picker_return = None;
                            self.notice = "Kept current model and session.".into();
                        }
                        self.affirmative = false;
                    }
                    _ => return false,
                }
                return true;
            }
            if self.review_details {
                match key.code {
                    KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                    KeyCode::Down => {
                        self.scroll = self.scroll.saturating_add(1).min(self.scroll_max)
                    }
                    KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(5),
                    KeyCode::PageDown => {
                        self.scroll = self.scroll.saturating_add(5).min(self.scroll_max)
                    }
                    KeyCode::Home => self.scroll = 0,
                    KeyCode::End => self.scroll = self.scroll_max,
                    _ => return false,
                }
                return true;
            }
            if key.code == KeyCode::Left && self.can_back_to_models() {
                self.back_to_models();
                return true;
            }
            if self.picker == Some(Picker::Model) {
                if let KeyCode::Char(ch) = key.code {
                    return self.paste(ch.encode_utf8(&mut [0; 4]));
                }
                if matches!(
                    key.code,
                    KeyCode::Backspace | KeyCode::Delete | KeyCode::Left | KeyCode::Right
                ) {
                    self.picker_query.line_key(key.code);
                    if matches!(key.code, KeyCode::Backspace | KeyCode::Delete) {
                        self.filter_picker();
                    }
                    return true;
                }
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
                    if self.catalogue_loading() {
                        self.notice = "Loading provider metadata only. Search remains available; wait for completion before selecting.".into();
                        return true;
                    }
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
                    if row.command == "model-custom" {
                        self.custom_model = Some(super::Answer::default());
                        self.notice = "Exact model ID · Enter Next / Esc Back".into();
                        return true;
                    }
                    let mut selection = self.picker_selection().clone();
                    match self.picker {
                        Some(Picker::Model) => {
                            selection.model = if self.picker_selected == 0 {
                                None
                            } else {
                                self.picker_model(self.picker_selected)
                                    .map(|model| model.id.clone())
                            };
                            if selection.model != self.selection.model {
                                selection.effort = None;
                            }
                            self.choose_model(selection);
                            return true;
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
                    self.choose_selection(selection);
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
        if let Some(PromptKind::Permission {
            file: Some(_),
            options,
            ..
        }) = self.request.as_ref().map(|request| &request.kind)
        {
            match key.code {
                KeyCode::Enter => {
                    if key.kind != KeyEventKind::Press
                        || !key.modifiers.is_empty()
                        || self.review_details
                    {
                        return false;
                    }
                    // Inspecting a proposed file must never submit the default Deny.
                    self.review_details = true;
                    self.scroll = 0;
                    self.invalidate_review();
                    return true;
                }
                KeyCode::Char('a' | 'y' | 'n') => {
                    if key.kind != KeyEventKind::Press
                        || !key.modifiers.is_empty()
                        || options.len() != 2
                        || key.code != KeyCode::Char('n') && (!self.reviewed || self.review_details)
                    {
                        return false;
                    }
                    let index = usize::from(key.code != KeyCode::Char('n'));
                    self.send_answer(options[index].answer.clone());
                    return true;
                }
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    return false;
                }
                _ => {}
            }
        }
        if key.code == KeyCode::Char('d')
            && (self.preview.is_some()
                || self.switch_to.is_some()
                || self.request.as_ref().is_some_and(|request| {
                    !matches!(request.kind, PromptKind::Questions(_)) || self.submit_review
                }))
        {
            self.review_details = !self.review_details;
            self.scroll = 0;
            self.invalidate_review();
            return true;
        }
        // Single-key confirmation is explicit consent, never text input or a repeat.
        if key.kind == KeyEventKind::Press
            && key.modifiers.is_empty()
            && self.reviewed
            && !self.review_details
        {
            match key.code {
                KeyCode::Char('y' | 'n')
                    if self.preview.is_some()
                        || self.switch_to.is_some()
                        || self.request.as_ref().is_some_and(|request| {
                            !matches!(request.kind, PromptKind::Questions(_)) || self.submit_review
                        }) =>
                {
                    self.affirmative = key.code == KeyCode::Char('y');
                    if key.code == KeyCode::Char('n') {
                        self.option = 0;
                    }
                    key.code = KeyCode::Enter;
                }
                KeyCode::Char('a') => {
                    if let Some(crate::providers::Prompt {
                        kind: PromptKind::Permission { options, .. },
                        ..
                    }) = &self.request
                    {
                        if options.len() == 2 {
                            self.option = 1;
                        }
                        if self.option > 0 && self.option < options.len() {
                            key.code = KeyCode::Enter;
                        }
                    } else if matches!(
                        self.request.as_ref().map(|request| &request.kind),
                        Some(PromptKind::Approval {
                            allow_once: true,
                            ..
                        })
                    ) {
                        self.affirmative = true;
                        key.code = KeyCode::Enter;
                    }
                }
                _ => {}
            }
        }
        if self.response_surface() {
            if key.modifiers.is_empty() && matches!(key.code, KeyCode::Up | KeyCode::Down) {
                return self.scroll_response_lines(if key.code == KeyCode::Up { -1 } else { 1 });
            }
            if key.modifiers.is_empty()
                && matches!(
                    key.code,
                    KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End
                )
                && self.response_key(key.code)
            {
                return true;
            }
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
            if self.journal_purge
                && self.reviewed
                && key.kind == KeyEventKind::Press
                && key.modifiers.is_empty()
                && matches!(key.code, KeyCode::Char('y' | 'n'))
            {
                self.affirmative = key.code == KeyCode::Char('y');
                key.code = KeyCode::Enter;
            }
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
                    if !self.dispatch_prepared() {
                        self.notice =
                            "Request blocked; resolve the listed prerequisites before sending."
                                .into();
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
        false
    }
}
