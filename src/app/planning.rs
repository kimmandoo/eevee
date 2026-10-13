//! Explicit session-local provider requests; provider text is never local execution evidence.
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::Arc,
};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde_json::{Value, json};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    context,
    provider::{
        Agent, Event, Input, ItemKind, Operation, Outcome, Preview, Prompt, PromptKind, Request,
    },
    repo, store,
};

const ANSWER_BYTES: usize = 4096;
const ITEM_LIMIT: usize = 128;

pub struct Item {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub kind: ItemKind,
    pub text: String,
    pub complete: bool,
}

#[derive(Default)]
struct Session {
    thread_id: Option<String>,
    items: Vec<Item>,
    selected: usize,
    omitted: usize,
}

#[derive(Default)]
pub struct Answer {
    pub text: String,
    pub cursor: usize,
}

pub struct State {
    pub pending: Option<Request>,
    pub decision: Option<String>,
    pub edit_goal: bool,
    pub preview: Option<Preview>,
    pub request: Option<Prompt>,
    pub items: Vec<Item>,
    pub selected: usize,
    pub thread_id: Option<String>,
    pub notice: String,
    pub inspection: Option<String>,
    pub inspect_sessions: bool,
    pub scroll: u16,
    pub scroll_max: u16,
    pub reviewed: bool,
    pub affirmative: bool,
    pub question: usize,
    pub option: usize,
    pub answers: Vec<Answer>,
    pub editing: bool,
    pub submit_review: bool,
    pub quit_review: bool,
    pub quit_ready: bool,
    revision: u64,
    pub cancelling: bool,
    pub omitted: usize,
    task: i64,
    generation: Option<u64>,
    awaiting_preview: bool,
    active: bool,
    cancel_when_ready: bool,
    quit_after_cancel: bool,
    unknown: bool,
    saved_scroll: u16,
    pub agent: Agent,
    pub operation: Operation,
    pub edit_paths: Vec<PathBuf>,
    pub paths_editor: Option<Answer>,
    pub switch_to: Option<Agent>,
    pub handoff: Option<crate::handoff::Document>,
    sessions: HashMap<(Agent, Operation), Session>,
    pub journal_mode: bool,
    pub journal_purge: bool,
    pub journal_records: Vec<store::ProviderRecord>,
    pub journal_selected: usize,
    pub pending_journal: VecDeque<store::JournalRequest>,
    journal_inflight: VecDeque<store::JournalRequest>,
    failed_journal: VecDeque<store::JournalRequest>,
    pub journal_notice: String,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            pending: None, decision: None, edit_goal: false, preview: None, request: None,
            items: Vec::new(), selected: 0, thread_id: None,
            notice: "No model session yet. New and Resume prepare an outgoing-payload review; F2 manages provider login separately.".into(),
            scroll: 0, scroll_max: 0, reviewed: false, affirmative: false,
            question: 0, option: 0, answers: Vec::new(), editing: false, submit_review: false,
            quit_review: false, quit_ready: false, cancelling: false, omitted: 0,
            task: 0, revision: 0, generation: None, awaiting_preview: false, active: false,
            cancel_when_ready: false, quit_after_cancel: false, unknown: false, saved_scroll: 0,
            agent: Agent::Codex, operation: Operation::Plan, edit_paths: Vec::new(),
            paths_editor: None, switch_to: None, handoff: None, sessions: HashMap::new(),
            journal_mode: false, journal_purge: false, journal_records: Vec::new(), journal_selected: 0,
            pending_journal: VecDeque::new(), journal_inflight: VecDeque::new(),
            failed_journal: VecDeque::new(), journal_notice: String::new(),
            inspection: None, inspect_sessions: false,
        }
    }

    pub fn matches_task(&self, task_id: i64, revision: u64) -> bool {
        self.task == task_id && self.revision == revision
    }

    pub fn needs_saved_input(&self, key: &KeyEvent) -> bool {
        key.kind == KeyEventKind::Press
            && !self.editing
            && !self.quit_review
            && !self.journal_mode
            && self.paths_editor.is_none()
            && ((self.preview.is_some() && self.affirmative && key.code == KeyCode::Enter)
                || (!self.busy() && matches!(key.code, KeyCode::Char('n' | 'r' | 's' | '1'..='4'))))
    }

    pub fn busy(&self) -> bool {
        self.active
            || self.awaiting_preview
            || self.preview.is_some()
            || self.pending.is_some()
            || self.quit_review
            || self.switch_to.is_some()
            || self.paths_editor.is_some()
            || !self.pending_journal.is_empty()
            || !self.journal_inflight.is_empty()
    }

    pub fn task_id(&self) -> i64 {
        self.task
    }

    pub fn outcome_unknown(&self) -> bool {
        self.unknown
    }

    pub fn reset(&mut self, task_id: i64) {
        if !self.busy() && !self.journal_failed() {
            *self = Self::new();
            self.task = task_id;
        }
    }

    pub fn invalidate_authentication(&mut self) {
        if self.busy() {
            return;
        }
        self.sessions.clear();
        self.thread_id = None;
        self.items.clear();
        self.selected = 0;
        self.omitted = 0;
        self.inspection = None;
        self.inspect_sessions = false;
        self.decision = None;
        self.generation = None;
        self.close_request();
        self.review_start();
        self.notice = "Authentication may have changed. Provider sessions and consent cleared; prepare a fresh request.".into();
    }

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

    fn review_start(&mut self) {
        self.scroll = 0;
        self.scroll_max = 0;
        self.invalidate_review();
    }

    fn matches(&self, generation: u64, task: i64) -> bool {
        self.generation == Some(generation) && self.task == task
    }

    pub fn set_operation(&mut self, operation: Operation) {
        if self.operation == operation || self.busy() {
            return;
        }
        self.close_request();
        self.generation = None;
        self.review_start();
        self.sessions.insert(
            (self.agent, self.operation),
            Session {
                thread_id: self.thread_id.take(),
                items: std::mem::take(&mut self.items),
                selected: self.selected,
                omitted: self.omitted,
            },
        );
        self.operation = operation;
        self.inspection = None;
        self.inspect_sessions = false;
        let session = self
            .sessions
            .remove(&(self.agent, operation))
            .unwrap_or_default();
        self.thread_id = session.thread_id;
        self.items = session.items;
        self.selected = session.selected;
        self.omitted = session.omitted;
        self.notice = "Role policy selected locally. New/Resume requires fresh scope and executable consent; no permission carried.".into();
    }

    pub fn journal_started(&mut self) {
        if let Some(request) = self.pending_journal.pop_front() {
            self.journal_inflight.push_back(request);
        }
    }

    pub fn journal_event(&mut self, task_id: i64, result: Result<store::JournalResponse, String>) {
        let Some(request) = self.journal_inflight.pop_front() else {
            self.journal_notice = "Unsolicited journal acknowledgement ignored.".into();
            return;
        };
        let correlated = request.task_id == task_id
            && match (&request.operation, &result) {
                (
                    store::JournalOperation::Put(expected),
                    Ok(store::JournalResponse::Saved(record)),
                ) => {
                    expected.id == record.id
                        && expected.agent == record.agent
                        && expected.generation == record.generation
                        && expected.thread_id == record.thread_id
                        && expected.turn_id == record.turn_id
                        && expected.item_id == record.item_id
                        && expected.complete == record.complete
                }
                (store::JournalOperation::List, Ok(store::JournalResponse::Records(_)))
                | (store::JournalOperation::Purge, Ok(store::JournalResponse::Purged))
                | (_, Err(_)) => true,
                _ => false,
            };
        if !correlated {
            self.failed_journal.push_back(request);
            self.journal_notice = "Uncorrelated journal acknowledgement refused; retained operation requires storage-only retry.".into();
            return;
        }
        match result {
            Ok(store::JournalResponse::Saved(_)) => {
                self.journal_notice =
                    "Provider-origin journal committed; not local execution evidence.".into();
            }
            Ok(store::JournalResponse::Records(records)) if task_id == self.task => {
                self.journal_records = records;
                self.journal_selected = self
                    .journal_selected
                    .min(self.journal_records.len().saturating_sub(1));
                self.scroll = 0;
                self.scroll_max = 0;
                self.journal_notice =
                    "Retained provider observations, with actual origin and coverage.".into();
            }
            Ok(store::JournalResponse::Purged) if task_id == self.task => {
                self.journal_records.clear();
                self.journal_selected = 0;
                self.scroll = 0;
                self.scroll_max = 0;
                self.failed_journal
                    .retain(|request| request.task_id != task_id);
                self.journal_notice = "This task's local provider observations purged; provider history and exports remain.".into();
            }
            Ok(_) => {}
            Err(error) => {
                self.failed_journal.push_back(request);
                self.journal_notice = format!(
                    "Provider journal save/read failed: {}. r retries storage only; never replays a turn.",
                    crate::handoff::sanitize(&error)
                );
            }
        }
    }

    pub fn journal_failed(&self) -> bool {
        !self.failed_journal.is_empty()
    }

    pub fn retry_journal(&mut self) {
        self.pending_journal.extend(self.failed_journal.drain(..));
        self.journal_notice =
            "Retrying retained journal operation only; no provider request.".into();
    }

    fn queue_record(&mut self, task_id: i64, record: store::ProviderRecord) {
        if self.pending_journal.len() + self.journal_inflight.len() + self.failed_journal.len()
            >= 128
        {
            self.journal_notice = "Provider journal queue limit reached; further observations cannot substantiate retained evidence.".into();
            return;
        }
        self.pending_journal.push_back(store::JournalRequest {
            task_id,
            operation: store::JournalOperation::Put(record),
        });
    }

    fn complete_switch(&mut self) {
        let Some(next) = self.switch_to.take() else {
            return;
        };
        self.sessions.insert(
            (self.agent, self.operation),
            Session {
                thread_id: self.thread_id.take(),
                items: std::mem::take(&mut self.items),
                selected: self.selected,
                omitted: self.omitted,
            },
        );
        self.agent = next;
        self.inspection = None;
        self.inspect_sessions = false;
        let session = self
            .sessions
            .remove(&(next, self.operation))
            .unwrap_or_default();
        self.thread_id = session.thread_id;
        self.items = session.items;
        self.selected = session.selected;
        self.omitted = session.omitted;
        self.generation = None;
        self.close_request();
        self.review_start();
        self.notice = format!(
            "{} selected at idle. All grants reset; New/Resume still requires full executable/context review.",
            next.label()
        );
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

    pub fn event(&mut self, event: Event) {
        match event {
            Event::Journal {
                generation,
                task_id,
                record,
            } if self.matches(generation, task_id)
                && record.agent == self.agent
                && record.generation == generation =>
            {
                self.queue_record(task_id, record);
            }
            Event::Preview(preview) if self.awaiting_preview && preview.task_id == self.task => {
                self.awaiting_preview = false;
                self.generation = Some(preview.generation);
                self.preview = Some(preview);
                self.notice = "Local preview only. Review every page; Stay is the default.".into();
                self.review_start();
                if self.cancel_when_ready {
                    self.cancel_when_ready = false;
                    self.cancel();
                }
            }
            Event::Status {
                generation,
                task_id,
                message,
            } if self.matches(generation, task_id) && self.active => {
                self.notice = message;
            }
            Event::Inspection {
                generation,
                task_id,
                text,
            } if self.matches(generation, task_id)
                && self.active
                && !self.cancelling
                && self.inspect_sessions =>
            {
                self.inspection = Some(text);
                self.scroll = 0;
            }
            Event::Thread {
                generation,
                task_id,
                thread_id,
                model,
                account,
            } if self.matches(generation, task_id) && self.active && !self.inspect_sessions => {
                self.thread_id = Some(thread_id);
                self.notice = format!(
                    "Provider connected / model {model} / account {account}. Provider owns billing and history."
                );
            }
            Event::Item {
                generation,
                task_id,
                thread_id,
                turn_id,
                item_id,
                kind,
                text,
                complete,
            } if self.matches(generation, task_id)
                && self.active
                && !self.cancelling
                && !self.inspect_sessions =>
            {
                if let Some(item) = self.items.iter_mut().find(|item| {
                    item.thread_id == thread_id
                        && item.turn_id == turn_id
                        && item.item_id == item_id
                }) {
                    item.text = text;
                    item.complete = complete;
                    item.kind = kind;
                } else {
                    if self.items.len() == ITEM_LIMIT {
                        self.items.remove(0);
                        self.selected = self.selected.saturating_sub(1);
                        self.omitted += 1;
                    }
                    self.items.push(Item {
                        thread_id,
                        turn_id,
                        item_id,
                        kind,
                        text,
                        complete,
                    });
                }
            }
            Event::Prompt {
                generation,
                task_id,
                request,
            } if self.matches(generation, task_id)
                && self.active
                && !self.cancelling
                && !self.inspect_sessions =>
            {
                self.saved_scroll = self.scroll;
                self.answers = match &request.kind {
                    PromptKind::Questions(questions) => {
                        questions.iter().map(|_| Answer::default()).collect()
                    }
                    _ => Vec::new(),
                };
                self.request = Some(request);
                self.question = 0;
                self.option = 0;
                self.editing = false;
                self.submit_review = false;
                self.review_start();
            }
            Event::PromptClosed {
                generation,
                request_id,
            } if self.generation == Some(generation)
                && self
                    .request
                    .as_ref()
                    .is_some_and(|request| request.request_id == request_id) =>
            {
                self.close_request();
            }
            Event::Finished {
                generation,
                task_id,
                outcome,
                message,
            } if self.matches(generation, task_id) => {
                self.active = false;
                self.awaiting_preview = false;
                self.cancelling = false;
                self.preview = None;
                self.pending = None;
                self.close_request();
                self.generation = None;
                self.notice = message;
                self.cancel_when_ready = false;
                self.unknown |= matches!(outcome, Outcome::Unknown);
                if self.quit_after_cancel {
                    self.quit_after_cancel = false;
                    if self.unknown {
                        self.quit_review = true;
                        self.invalidate_review();
                    } else {
                        self.quit_ready = true;
                    }
                }
            }
            Event::Error {
                generation,
                message,
            } if (generation.is_some() && generation == self.generation)
                || (generation.is_none() && self.awaiting_preview) =>
            {
                self.notice = message;
                self.awaiting_preview = false;
                self.preview = None;
                self.pending = None;
                if !self.active {
                    self.generation = None;
                    self.cancel_when_ready = false;
                    if self.quit_after_cancel {
                        self.quit_after_cancel = false;
                        self.quit_ready = true;
                    }
                }
            }
            _ => {}
        }
    }

    fn close_request(&mut self) {
        self.request = None;
        self.answers.clear();
        self.editing = false;
        self.submit_review = false;
        self.scroll = self.saved_scroll;
        self.invalidate_review();
    }

    pub fn cancel(&mut self) -> bool {
        self.invalidate_review();
        if self.switch_to.take().is_some() || self.paths_editor.take().is_some() {
            return true;
        }
        if self.cancelling {
            return false;
        }
        if self.awaiting_preview {
            self.cancel_when_ready = true;
            self.notice =
                "Cancelling after local preview preparation; no launch consent given.".into();
            return true;
        }
        let Some(generation) = self.generation else {
            return false;
        };
        self.pending = Some(Request::Cancel { generation });
        self.preview = None;
        self.close_request();
        self.active = true;
        self.cancelling = true;
        self.notice = "Cancellation requested. Waiting for provider acknowledgement or bounded owned teardown; outcome not yet known.".into();
        true
    }

    pub fn request_quit(&mut self) {
        self.quit_review = true;
        self.invalidate_review();
        self.editing = false;
    }

    fn prepare(
        &mut self,
        resume: bool,
        inspect_sessions: bool,
        task: &store::Snapshot,
        excerpts: &[context::Excerpt],
        report: Option<&Arc<repo::Report>>,
    ) {
        if self.busy() {
            return;
        }
        if self.journal_failed() {
            self.notice =
                "Retry failed journal storage before preparing another provider request.".into();
            return;
        }
        if task.id <= 0 || task.archived || task.goal.trim().is_empty() {
            self.notice = "Save a nonempty goal in an active task before planning.".into();
            return;
        }
        if resume && (self.task != task.id || self.thread_id.is_none()) {
            self.notice =
                "Resume requires an explicitly selected same-task thread from this session.".into();
            return;
        }
        if self.task != task.id {
            self.reset(task.id);
        }
        self.revision = task.revision;
        self.inspect_sessions = inspect_sessions;
        self.inspection = None;
        self.pending = Some(Request::Preview(Box::new(Input {
            task: task.clone(),
            excerpts: excerpts.to_vec(),
            report: report.cloned(),
            resume: if resume { self.thread_id.clone() } else { None },
            agent: self.agent,
            operation: self.operation,
            edit_paths: self.edit_paths.clone(),
            handoff: self.handoff.clone(),
            inspect_sessions,
        })));
        self.awaiting_preview = true;
        self.notice =
            "Preparing local preview. No provider executable or connection is authorized.".into();
        self.saved_scroll = self.scroll;
        self.review_start();
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
        if key.code == KeyCode::Char('q') {
            self.request_quit();
            return true;
        }
        if key.code == KeyCode::Esc || key.code == KeyCode::Char('c') {
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
                KeyCode::Char('h') | KeyCode::Esc => {
                    self.journal_mode = false;
                    self.journal_purge = false;
                }
                KeyCode::Char('r') if !self.journal_purge => self.retry_journal(),
                KeyCode::Char('p') if !self.busy() => {
                    self.journal_purge = true;
                    self.affirmative = false;
                }
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
                        if preview.thread_id.is_none() && !self.inspect_sessions {
                            self.items.clear();
                            self.selected = 0;
                            self.omitted = 0;
                            self.thread_id = None;
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
                        KeyCode::Char('a') if question.other || question.options.is_empty() => {
                            self.editing = true
                        }
                        KeyCode::Char('s') => {
                            if self
                                .answers
                                .iter()
                                .all(|answer| !answer.text.trim().is_empty())
                            {
                                self.submit_review = true;
                                self.review_start();
                            } else {
                                self.notice = "Answer every question before reviewing submission. Nothing has been sent.".into();
                            }
                        }
                        _ => return false,
                    }
                }
            }
            return true;
        }
        match key.code {
            KeyCode::Char('1'..='4') if !self.busy() => {
                if let KeyCode::Char(digit) = key.code {
                    let next = Agent::ALL[digit as usize - '1' as usize];
                    if next != self.agent {
                        if self.handoff.is_none() {
                            match crate::handoff::from_task(task, excerpts, &[]) {
                                Ok(document) => self.handoff = Some(document),
                                Err(error) => {
                                    self.notice = error;
                                    return true;
                                }
                            }
                        }
                        self.switch_to = Some(next);
                        self.review_start();
                    }
                }
            }
            KeyCode::Char('f') if !self.busy() => {
                self.set_operation(if self.operation == Operation::Plan {
                    Operation::Implement
                } else {
                    Operation::Plan
                });
            }
            KeyCode::Char('x') if !self.busy() => {
                let paths: Vec<_> = if self.edit_paths.is_empty() {
                    excerpts
                        .iter()
                        .map(|excerpt| excerpt.path.clone())
                        .collect()
                } else {
                    self.edit_paths
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect()
                };
                let text = serde_json::to_string(&paths).unwrap_or_else(|_| "[]".into());
                self.paths_editor = Some(Answer {
                    cursor: text.len(),
                    text,
                });
            }
            KeyCode::Char('h') if !self.busy() && self.task > 0 => {
                self.journal_mode = true;
                self.journal_purge = false;
                self.review_start();
                self.pending_journal.push_back(store::JournalRequest {
                    task_id: self.task,
                    operation: store::JournalOperation::List,
                });
            }
            KeyCode::Char('b') if !self.busy() => {
                self.handoff = None;
                self.notice = "Portable handoff deselected locally. Current task/source basket still requires fresh review; no original export or provider history was erased.".into();
            }
            KeyCode::Char('n') => self.prepare(false, false, task, excerpts, report),
            KeyCode::Char('r') => self.prepare(true, false, task, excerpts, report),
            KeyCode::Char('s') => self.prepare(false, true, task, excerpts, report),
            KeyCode::Char('g') if !self.busy() => self.edit_goal = true,
            KeyCode::Up if self.inspection.is_none() => {
                self.selected = self.selected.saturating_sub(1);
                self.scroll = 0;
            }
            KeyCode::Down if self.inspection.is_none() => {
                self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
                self.scroll = 0;
            }
            KeyCode::Char('d') | KeyCode::Enter if !self.busy() && self.inspection.is_none() => {
                if let Some(item) = self.items.get(self.selected).filter(|item| {
                    item.complete && matches!(item.kind, ItemKind::Message | ItemKind::Plan)
                }) {
                    let provenance = format!(
                        "\n\n[User-selected, unverified {} {:?}; thread {}; turn {}; item {}. Not local execution evidence.]",
                        self.agent.label(),
                        self.operation,
                        item.thread_id,
                        item.turn_id,
                        item.item_id
                    );
                    if item.text.len() + provenance.len() > 4096 {
                        self.notice = "Decision exceeds the 4096-byte NextAction limit. Nothing was copied or truncated; select a shorter provider message or edit NextAction manually.".into();
                    } else {
                        self.decision = Some(format!("{}{}", item.text, provenance));
                    }
                }
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Question;

    fn task() -> store::Snapshot {
        let mut task = store::Snapshot::empty("project".into());
        task.id = 7;
        task.goal = "Plan a change".into();
        task
    }

    fn press(state: &mut State, code: KeyCode, large: bool) {
        state.key(
            KeyEvent::new(code, KeyModifiers::NONE),
            large,
            &task(),
            &[],
            None,
        );
    }

    fn preview() -> State {
        let mut state = State::new();
        press(&mut state, KeyCode::Char('n'), true);
        assert!(matches!(state.pending.take(), Some(Request::Preview(_))));
        state.event(Event::Preview(Preview {
            generation: 11,
            task_id: 7,
            thread_id: None,
            trust: vec!["Executable trust".into()],
            payload: "exact goal".into(),
            blockers: Vec::new(),
        }));
        state
    }

    fn active() -> State {
        let mut state = preview();
        state.reviewed = true;
        press(&mut state, KeyCode::Tab, true);
        press(&mut state, KeyCode::Enter, true);
        assert!(matches!(
            state.pending.take(),
            Some(Request::Run { generation: 11 })
        ));
        state
    }

    #[test]
    fn preview_is_passive_and_run_needs_last_page_and_deliberate_choice() {
        let mut state = preview();
        press(&mut state, KeyCode::Tab, true);
        assert!(!state.affirmative);
        assert!(state.pending.is_none());
        state.reviewed = true;
        press(&mut state, KeyCode::Tab, true);
        state.key(
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat),
            true,
            &task(),
            &[],
            None,
        );
        assert!(state.pending.is_none());
        press(&mut state, KeyCode::Enter, false);
        assert!(state.pending.is_none());
        assert!(!state.affirmative);
        press(&mut state, KeyCode::Enter, true);
        assert!(matches!(
            state.pending,
            Some(Request::Cancel { generation: 11 })
        ));
    }

    #[test]
    fn cancel_invalidates_prompt_and_rejects_stale_generation() {
        let mut state = active();
        state.event(Event::Status {
            generation: 12,
            task_id: 7,
            message: "stale".into(),
        });
        assert_ne!(state.notice, "stale");
        assert!(state.cancel());
        state.event(Event::Prompt {
            generation: 11,
            task_id: 7,
            request: Prompt {
                request_id: json!(1),
                thread_id: "thread".into(),
                turn_id: "turn".into(),
                item_id: "item".into(),
                kind: PromptKind::Approval {
                    command: "echo".into(),
                    cwd: ".".into(),
                    reason: "test".into(),
                    allow_once: true,
                    deny_decision: "cancel".into(),
                },
            },
        });
        assert!(state.request.is_none());
        assert!(state.busy());
        state.event(Event::Finished {
            generation: 11,
            task_id: 7,
            outcome: Outcome::Cancelled,
            message: "cancelled".into(),
        });
        assert!(!state.busy());
    }

    #[test]
    fn pasted_commands_are_literal_and_cjk_cursor_stays_on_graphemes() {
        let mut state = active();
        state.event(Event::Prompt {
            generation: 11,
            task_id: 7,
            request: Prompt {
                request_id: json!(1),
                thread_id: "thread".into(),
                turn_id: "turn".into(),
                item_id: "item".into(),
                kind: PromptKind::Questions(vec![Question {
                    id: "q".into(),
                    header: "Choice".into(),
                    question: "What next?".into(),
                    options: Vec::new(),
                    other: true,
                    secret: false,
                }]),
            },
        });
        assert!(!state.paste("accept\n"));
        press(&mut state, KeyCode::Char('a'), true);
        assert!(state.paste("計画e\u{301}"));
        press(&mut state, KeyCode::Backspace, true);
        assert_eq!(state.answers[0].text, "計画");
        assert!(state.paste("\nq\taccept\r"));
        assert_eq!(state.answers[0].text, "計画qaccept");
        assert!(state.pending.is_none());
        press(&mut state, KeyCode::Enter, true);
        assert!(state.pending.is_none());
        press(&mut state, KeyCode::Char('s'), true);
        assert!(state.submit_review);
        assert!(!state.affirmative);
    }

    #[test]
    fn small_exit_is_neutral_and_unknown_outcome_requires_second_choice() {
        let mut state = active();
        state.request_quit();
        press(&mut state, KeyCode::Enter, false);
        assert!(!state.quit_ready);
        assert!(!state.cancelling);
        state.request_quit();
        press(&mut state, KeyCode::Tab, false);
        press(&mut state, KeyCode::Enter, false);
        assert!(state.cancelling);
        assert!(!state.quit_ready);
        state.event(Event::Finished {
            generation: 11,
            task_id: 7,
            outcome: Outcome::Unknown,
            message: "unknown outcome".into(),
        });
        assert!(state.quit_review);
        assert!(!state.affirmative);
        assert!(!state.quit_ready);
        press(&mut state, KeyCode::Tab, false);
        press(&mut state, KeyCode::Enter, false);
        assert!(state.quit_ready);
    }

    #[test]
    fn denial_uses_the_server_offered_cancel_not_invented_decline() {
        let mut state = active();
        state.event(Event::Prompt {
            generation: 11,
            task_id: 7,
            request: Prompt {
                request_id: json!(23),
                thread_id: "thread".into(),
                turn_id: "turn".into(),
                item_id: "item".into(),
                kind: PromptKind::Approval {
                    command: "echo".into(),
                    cwd: ".".into(),
                    reason: "test".into(),
                    allow_once: true,
                    deny_decision: "cancel".into(),
                },
            },
        });
        assert!(!state.paste("accept"));
        press(&mut state, KeyCode::Enter, true);
        match state.pending.take() {
            Some(Request::Answer {
                generation: 11,
                request_id,
                answer,
            }) => {
                assert_eq!(request_id, json!(23));
                assert_eq!(answer, json!({"decision":"cancel"}));
            }
            _ => panic!("explicit deny should answer exactly the offered server decision"),
        }
    }
    #[test]
    fn idle_switch_preserves_old_session_without_portable_grants_or_ids() {
        let mut state = State::new();
        state.reset(7);
        state.thread_id = Some("opaque-old-thread".into());
        press(&mut state, KeyCode::Char('2'), true);
        assert_eq!(state.agent, Agent::Codex);
        assert!(state.pending.is_none());
        let document = serde_json::to_string(state.handoff.as_ref().unwrap()).unwrap();
        assert!(!document.contains("opaque-old-thread"));
        state.reviewed = true;
        press(&mut state, KeyCode::Tab, true);
        press(&mut state, KeyCode::Enter, true);
        assert_eq!(state.agent, Agent::Claude);
        assert!(state.thread_id.is_none());
        assert!(state.pending.is_none());
        press(&mut state, KeyCode::Char('1'), true);
        state.reviewed = true;
        press(&mut state, KeyCode::Tab, true);
        press(&mut state, KeyCode::Enter, true);
        assert_eq!(state.thread_id.as_deref(), Some("opaque-old-thread"));
    }

    #[test]
    fn failed_journal_is_retained_for_storage_only_retry_and_prevents_reset() {
        let mut state = State::new();
        state.reset(7);
        state.pending_journal.push_back(store::JournalRequest {
            task_id: 7,
            operation: store::JournalOperation::List,
        });
        state.journal_started();
        state.journal_event(7, Err("locked storage".into()));
        state.reset(9);
        assert_eq!(state.task_id(), 7);
        assert!(state.journal_failed());
        state.retry_journal();
        assert!(matches!(
            state.pending_journal.front().unwrap().operation,
            store::JournalOperation::List
        ));
        assert!(
            state.pending.is_none(),
            "Journal retry must not create a provider request"
        );
    }

    fn message() -> Item {
        Item {
            thread_id: "old-thread".into(),
            turn_id: "old-turn".into(),
            item_id: "old-item".into(),
            kind: ItemKind::Message,
            text: "Visible decision".into(),
            complete: true,
        }
    }

    #[test]
    fn metadata_inspection_cannot_select_a_hidden_message_or_replace_its_session() {
        let mut state = active();
        state.thread_id = Some("old-thread".into());
        state.items.push(message());
        state.inspect_sessions = true;
        state.event(Event::Thread {
            generation: 11,
            task_id: 7,
            thread_id: "listed-not-selected".into(),
            model: "metadata".into(),
            account: "metadata".into(),
        });
        state.event(Event::Item {
            generation: 11,
            task_id: 7,
            thread_id: "listed-not-selected".into(),
            turn_id: "not-a-turn".into(),
            item_id: "metadata".into(),
            kind: ItemKind::Message,
            text: "not a decision".into(),
            complete: true,
        });
        state.event(Event::Inspection {
            generation: 11,
            task_id: 7,
            text: "Metadata only".into(),
        });
        state.event(Event::Finished {
            generation: 11,
            task_id: 7,
            outcome: Outcome::Completed,
            message: "Inspection completed".into(),
        });
        assert_eq!(state.thread_id.as_deref(), Some("old-thread"));
        assert_eq!(state.items.len(), 1);
        for code in [KeyCode::Char('d'), KeyCode::Enter, KeyCode::Down] {
            press(&mut state, code, true);
            assert!(state.decision.is_none());
            assert_eq!(state.selected, 0);
        }
        press(&mut state, KeyCode::Esc, true);
        assert!(state.inspection.is_none());
        press(&mut state, KeyCode::Char('d'), true);
        assert!(
            state
                .decision
                .as_ref()
                .unwrap()
                .starts_with("Visible decision")
        );
    }

    #[test]
    fn leaving_live_metadata_inspection_cancels_and_rejects_late_metadata() {
        let mut state = active();
        state.inspect_sessions = true;
        state.inspection = Some("Capabilities; session list still loading".into());
        press(&mut state, KeyCode::Esc, true);
        assert!(state.inspection.is_none());
        assert!(state.cancelling);
        assert!(matches!(
            state.pending,
            Some(Request::Cancel { generation: 11 })
        ));
        state.event(Event::Inspection {
            generation: 11,
            task_id: 7,
            text: "Late list".into(),
        });
        assert!(state.inspection.is_none());
        assert!(
            state.inspect_sessions,
            "Cancellation must not turn metadata into model output"
        );
    }

    #[test]
    fn resize_and_quit_reset_permission_to_deny() {
        let mut state = active();
        state.event(Event::Prompt {
            generation: 11,
            task_id: 7,
            request: Prompt {
                request_id: json!(1),
                thread_id: "thread".into(),
                turn_id: "turn".into(),
                item_id: "item".into(),
                kind: PromptKind::Permission {
                    description: "Review exact scope".into(),
                    options: vec![
                        crate::provider::PermissionOption {
                            label: "Deny".into(),
                            description: "No permission".into(),
                            answer: json!({"permission":"deny"}),
                        },
                        crate::provider::PermissionOption {
                            label: "Allow once".into(),
                            description: "One request".into(),
                            answer: json!({"permission":"once"}),
                        },
                    ],
                },
            },
        });
        state.reviewed = true;
        press(&mut state, KeyCode::Tab, true);
        assert_eq!(state.option, 1);
        press(&mut state, KeyCode::Enter, false);
        assert_eq!(state.option, 0);
        assert!(state.pending.is_none());
        state.reviewed = true;
        press(&mut state, KeyCode::Tab, true);
        state.request_quit();
        press(&mut state, KeyCode::Esc, true);
        assert_eq!(state.option, 0);
        press(&mut state, KeyCode::Enter, true);
        assert!(matches!(
            state.pending,
            Some(Request::Answer { answer, .. }) if answer == json!({"permission":"deny"})
        ));
    }

    #[test]
    fn prior_unknown_outcome_survives_later_completion_and_authentication_reset() {
        let mut state = active();
        state.event(Event::Finished {
            generation: 11,
            task_id: 7,
            outcome: Outcome::Unknown,
            message: "No authoritative completion".into(),
        });
        press(&mut state, KeyCode::Char('s'), true);
        state.pending.take();
        state.event(Event::Preview(Preview {
            generation: 12,
            task_id: 7,
            thread_id: None,
            trust: Vec::new(),
            payload: String::new(),
            blockers: Vec::new(),
        }));
        state.reviewed = true;
        press(&mut state, KeyCode::Tab, true);
        press(&mut state, KeyCode::Enter, true);
        state.pending.take();
        assert!(state.outcome_unknown());
        state.event(Event::Finished {
            generation: 12,
            task_id: 7,
            outcome: Outcome::Completed,
            message: "Metadata inspection succeeded".into(),
        });
        state.thread_id = Some("old-auth-thread".into());
        state.items.push(message());
        state.sessions.insert(
            (Agent::Claude, Operation::Implement),
            Session {
                thread_id: Some("other-auth-thread".into()),
                ..Session::default()
            },
        );
        state.failed_journal.push_back(store::JournalRequest {
            task_id: 7,
            operation: store::JournalOperation::List,
        });
        state.invalidate_authentication();
        assert!(state.outcome_unknown());
        assert!(state.journal_failed());
        assert!(state.thread_id.is_none());
        assert!(state.items.is_empty());
        assert!(state.sessions.is_empty());
        assert!(!state.reviewed && !state.affirmative);
    }

    #[test]
    fn journal_retry_is_not_blocked_by_provider_save_gate_and_selection_starts_at_top() {
        let mut state = State::new();
        state.reset(7);
        state.failed_journal.push_back(store::JournalRequest {
            task_id: 7,
            operation: store::JournalOperation::List,
        });
        press(&mut state, KeyCode::Char('h'), true);
        state.journal_started();
        state.journal_event(7, Ok(store::JournalResponse::Records(Vec::new())));
        assert!(!state.needs_saved_input(&KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)));
        press(&mut state, KeyCode::Char('r'), true);
        assert!(!state.journal_failed());
        assert_eq!(state.pending_journal.len(), 1);
        assert!(state.pending.is_none());
        state.scroll = 40;
        state.scroll_max = 80;
        press(&mut state, KeyCode::Down, true);
        assert_eq!(state.scroll, 0);
        assert_eq!(state.scroll_max, 0);
    }

    #[test]
    fn unsolicited_or_wrong_kind_journal_reply_cannot_clear_retained_failure() {
        let mut state = State::new();
        state.reset(7);
        state.failed_journal.push_back(store::JournalRequest {
            task_id: 7,
            operation: store::JournalOperation::List,
        });
        state.journal_event(7, Ok(store::JournalResponse::Purged));
        assert!(state.journal_failed());
        state.pending_journal.push_back(store::JournalRequest {
            task_id: 7,
            operation: store::JournalOperation::List,
        });
        state.journal_started();
        state.journal_event(7, Ok(store::JournalResponse::Purged));
        assert_eq!(state.failed_journal.len(), 2);
        state.retry_journal();
        assert!(
            state
                .pending_journal
                .iter()
                .all(|request| matches!(request.operation, store::JournalOperation::List))
        );
    }
}
