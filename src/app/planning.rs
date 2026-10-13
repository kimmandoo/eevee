//! Explicit, session-local Codex planning; provider text is never local execution evidence.
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde_json::{Value, json};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    codex::{Event, Input, ItemKind, Outcome, Preview, Prompt, PromptKind, Request},
    context, repo, store,
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
            notice: "No connection. New and Resume first prepare a local trust and outgoing-payload preview.".into(),
            scroll: 0, scroll_max: 0, reviewed: false, affirmative: false,
            question: 0, option: 0, answers: Vec::new(), editing: false, submit_review: false,
            quit_review: false, quit_ready: false, cancelling: false, omitted: 0,
            task: 0, revision: 0, generation: None, awaiting_preview: false, active: false,
            cancel_when_ready: false, quit_after_cancel: false, unknown: false, saved_scroll: 0,
        }
    }

    pub fn matches_task(&self, task_id: i64, revision: u64) -> bool {
        self.task == task_id && self.revision == revision
    }

    pub fn needs_saved_input(&self, key: &KeyEvent) -> bool {
        key.kind == KeyEventKind::Press
            && !self.editing
            && !self.quit_review
            && ((self.preview.is_some() && self.affirmative && key.code == KeyCode::Enter)
                || (!self.busy() && matches!(key.code, KeyCode::Char('n' | 'r'))))
    }

    pub fn busy(&self) -> bool {
        self.active
            || self.awaiting_preview
            || self.preview.is_some()
            || self.pending.is_some()
            || self.quit_review
    }

    pub fn task_id(&self) -> i64 {
        self.task
    }

    pub fn outcome_unknown(&self) -> bool {
        self.unknown
    }

    pub fn reset(&mut self, task_id: i64) {
        if !self.busy() {
            *self = Self::new();
            self.task = task_id;
        }
    }

    pub fn invalidate_review(&mut self) {
        self.reviewed = false;
        self.affirmative = false;
    }

    fn review_start(&mut self) {
        self.scroll = 0;
        self.scroll_max = 0;
        self.invalidate_review();
    }

    fn matches(&self, generation: u64, task: i64) -> bool {
        self.generation == Some(generation) && self.task == task
    }

    pub fn event(&mut self, event: Event) {
        match event {
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
            Event::Thread {
                generation,
                task_id,
                thread_id,
                model,
                account,
            } if self.matches(generation, task_id) && self.active => {
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
            } if self.matches(generation, task_id) && self.active && !self.cancelling => {
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
            } if self.matches(generation, task_id) && self.active && !self.cancelling => {
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
                self.unknown = matches!(outcome, Outcome::Unknown);
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
            } if generation == self.generation
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
        self.affirmative = false;
        self.editing = false;
    }

    fn prepare(
        &mut self,
        resume: bool,
        task: &store::Snapshot,
        excerpts: &[context::Excerpt],
        report: Option<&Arc<repo::Report>>,
    ) {
        if self.busy() {
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
        self.pending = Some(Request::Preview(Box::new(Input {
            task: task.clone(),
            excerpts: excerpts.to_vec(),
            report: report.cloned(),
            resume: if resume { self.thread_id.clone() } else { None },
        })));
        self.awaiting_preview = true;
        self.notice =
            "Preparing local preview. No provider executable or connection is authorized.".into();
        self.saved_scroll = self.scroll;
        self.review_start();
    }

    pub fn paste(&mut self, text: &str) -> bool {
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
                        if !self.cancel() && !self.cancelling {
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
                        if preview.thread_id.is_none() {
                            self.items.clear();
                            self.selected = 0;
                            self.omitted = 0;
                            self.thread_id = None;
                        }
                        self.pending = Some(Request::Run { generation });
                        self.preview = None;
                        self.active = true;
                        self.unknown = false;
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
            KeyCode::Char('n') => self.prepare(false, task, excerpts, report),
            KeyCode::Char('r') => self.prepare(true, task, excerpts, report),
            KeyCode::Char('g') if !self.busy() => self.edit_goal = true,
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                self.scroll = 0;
            }
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
                self.scroll = 0;
            }
            KeyCode::Char('d') | KeyCode::Enter if !self.busy() => {
                if let Some(item) = self.items.get(self.selected).filter(|item| {
                    item.complete && matches!(item.kind, ItemKind::Message | ItemKind::Plan)
                }) {
                    let provenance = format!(
                        "\n\n[User-selected, unverified Codex planning; thread {}; turn {}; item {}. Not local execution evidence.]",
                        item.thread_id, item.turn_id, item.item_id
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
    use crate::codex::Question;

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
}
