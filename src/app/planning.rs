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
    providers::{
        Agent, Event, Input, ItemKind, ModelCatalogue, ModelSelection, Operation, Outcome, Preview,
        Prompt, PromptKind, Request,
    },
    store,
    workspace::context,
    workspace::repo,
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
    selection: ModelSelection,
    observed_model: Option<String>,
}

#[derive(Default)]
pub struct Answer {
    pub text: String,
    pub cursor: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Picker {
    Model,
    Effort,
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
    pub handoff: Option<crate::workspace::handoff::Document>,
    sessions: HashMap<(Agent, Operation), Session>,
    pub journal_mode: bool,
    pub journal_purge: bool,
    pub journal_records: Vec<store::ProviderRecord>,
    pub journal_selected: usize,
    pub pending_journal: VecDeque<store::JournalRequest>,
    journal_inflight: VecDeque<store::JournalRequest>,
    failed_journal: VecDeque<store::JournalRequest>,
    pub journal_notice: String,
    /// Local receipt of the open preview's exact payload.
    pub receipt: Vec<String>,
    pub selection: ModelSelection,
    selections: HashMap<Agent, ModelSelection>,
    pub catalogues: HashMap<Agent, ModelCatalogue>,
    pub picker: Option<Picker>,
    pub picker_selected: usize,
    pub selection_change: Option<ModelSelection>,
    pub inspect_models: bool,
    request_selection: ModelSelection,
    session_selection: ModelSelection,
    pub observed_model: Option<String>,
    /// Website-OAuth session credential for Devin ACP launches; memory only.
    pub devin_credential: Option<crate::providers::auth::SessionCredential>,
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
            notice: "No model session yet. /new and /resume prepare an outgoing-payload review; /connect manages provider login separately.".into(),
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
            receipt: Vec::new(),
            inspection: None, inspect_sessions: false,
            selection: ModelSelection::default(), selections: HashMap::new(),
            catalogues: HashMap::new(), picker: None, picker_selected: 0,
            selection_change: None, inspect_models: false,
            request_selection: ModelSelection::default(), session_selection: ModelSelection::default(),
            observed_model: None,
            devin_credential: None,
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
            && self.preview.is_some()
            && self.affirmative
            && key.code == KeyCode::Enter
    }

    pub fn needs_saved_command(&self, name: &str) -> bool {
        !self.active
            && !self.awaiting_preview
            && !self.journal_mode
            && (matches!(name, "new" | "resume" | "sessions" | "models-refresh")
                || name.starts_with("provider-") && name != "provider-journal")
    }

    pub fn busy(&self) -> bool {
        self.busy_except_picker() || self.picker.is_some()
    }

    pub fn busy_except_picker(&self) -> bool {
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
            let selections = std::mem::take(&mut self.selections);
            let catalogues = std::mem::take(&mut self.catalogues);
            let selection = self.selection.clone();
            let credential = self.devin_credential.take();
            let agent = self.agent;
            *self = Self::new();
            self.task = task_id;
            self.agent = agent;
            self.selection = selection;
            self.selections = selections;
            self.catalogues = catalogues;
            self.devin_credential = credential;
        }
    }

    /// Stores or replaces the website-login credential used for Devin ACP
    /// launches; `None` detaches it (native login or explicit clear).
    pub fn set_devin_credential(
        &mut self,
        credential: Option<crate::providers::auth::SessionCredential>,
    ) {
        self.devin_credential = credential;
    }

    pub fn invalidate_authentication(&mut self) {
        // Any open picker must close before its stale catalogue survives a
        // credential change; a live request keeps its generation-correlated
        // state and is only flagged for re-review.
        self.picker = None;
        self.selection_change = None;
        if self.busy() {
            self.invalidate_review();
            self.notice = "Authentication changed during a provider request; continuing work is not invalidated.".into();
            return;
        }
        self.sessions.clear();
        self.catalogues.clear();
        self.thread_id = None;
        self.observed_model = None;
        self.items.clear();
        self.selected = 0;
        self.omitted = 0;
        self.inspection = None;
        self.inspect_sessions = false;
        self.inspect_models = false;
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
                selection: self.session_selection.clone(),
                observed_model: self.observed_model.take(),
            },
        );
        self.operation = operation;
        self.inspection = None;
        self.inspect_sessions = false;
        self.inspect_models = false;
        let session = self
            .sessions
            .remove(&(self.agent, operation))
            .unwrap_or_default();
        self.session_selection = session.selection;
        self.observed_model = session.observed_model;
        self.thread_id = session.thread_id;
        self.items = session.items;
        self.selected = session.selected;
        self.omitted = session.omitted;
        self.notice = if operation == Operation::Review {
            "Independent Review selected locally. New always constructs fresh requirements/patch/evidence context and a separate provider session. Resume and source writes are unavailable; fixes require a deliberate implementation handoff.".into()
        } else {
            "Role policy selected locally. New/Resume requires fresh scope and executable consent; no permission carried.".into()
        };
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
                    crate::workspace::handoff::sanitize(&error)
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
                selection: self.session_selection.clone(),
                observed_model: self.observed_model.take(),
            },
        );
        self.selections.insert(self.agent, self.selection.clone());
        self.agent = next;
        self.selection = self.selections.get(&next).cloned().unwrap_or_default();
        self.inspection = None;
        self.inspect_sessions = false;
        self.inspect_models = false;
        let session = self
            .sessions
            .remove(&(next, self.operation))
            .unwrap_or_default();
        self.session_selection = session.selection;
        self.observed_model = session.observed_model;
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
                && !self.inspect_sessions
                && !self.inspect_models
                && record.agent == self.agent
                && record.generation == generation =>
            {
                self.queue_record(task_id, record);
            }
            Event::Preview(preview) if self.awaiting_preview && preview.task_id == self.task => {
                self.awaiting_preview = false;
                self.generation = Some(preview.generation);
                self.receipt = crate::workspace::receipt::receipt(&preview.payload).lines();
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
            Event::Models {
                generation,
                task_id,
                catalogue,
            } if self.matches(generation, task_id)
                && self.active
                && self.inspect_models
                && !self.cancelling =>
            {
                match catalogue.validate() {
                    Ok(()) => {
                        // A refreshed catalogue that no longer advertises the
                        // selected model must say so; the picker's Default row
                        // is honest about a stale explicit id only here.
                        let stale = self.selection.model.as_ref().is_some_and(|model| {
                            !catalogue.models.iter().any(|entry| &entry.id == model)
                        });
                        self.catalogues.insert(self.agent, catalogue);
                        if stale {
                            self.notice = "The refreshed catalogue no longer advertises the selected model; the next request fails its advertised check unless you reselect.".into();
                        }
                    }
                    Err(reason) => self.notice = format!("Model catalogue rejected: {reason}"),
                }
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
            } if self.matches(generation, task_id)
                && self.active
                && !self.inspect_sessions
                && !self.inspect_models =>
            {
                self.thread_id = Some(thread_id);
                self.session_selection = self.request_selection.clone();
                self.observed_model = Some(model.clone());
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
                && !self.inspect_sessions
                && !self.inspect_models =>
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
                && !self.inspect_sessions
                && !self.inspect_models =>
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
                if self.inspect_models
                    && matches!(outcome, Outcome::Completed)
                    && !self.quit_after_cancel
                {
                    self.open_picker(Picker::Model);
                }
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
        if self.switch_to.take().is_some()
            || self.paths_editor.take().is_some()
            || self.selection_change.take().is_some()
            || self.picker.take().is_some()
        {
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

    pub fn selection_label(&self) -> String {
        format!(
            "{} / requested model {} / requested effort {} / role {:?} / observed session model: {} (catalogue availability is not entitlement)",
            self.agent.label(),
            self.selection
                .model
                .as_deref()
                .unwrap_or("Default (provider configuration)"),
            self.selection
                .effort
                .as_deref()
                .unwrap_or("Default (provider configuration)"),
            self.operation,
            self.observed_model.as_deref().unwrap_or("unobserved")
        )
    }

    pub fn reviewed_selection(&self) -> &ModelSelection {
        &self.request_selection
    }

    fn effort_model(&self) -> Option<&crate::providers::ModelOption> {
        let catalogue = self.catalogues.get(&self.agent)?;
        let id = self
            .selection
            .model
            .as_ref()
            .or(catalogue.current.as_ref())?;
        catalogue.models.iter().find(|model| &model.id == id)
    }

    /// Opens a local picker on the current value, or on Refresh when nothing is inspected yet.
    fn open_picker(&mut self, picker: Picker) {
        self.picker = Some(picker);
        self.scroll = 0;
        self.picker_selected = match self.picker_current() {
            Some(index) => index,
            None if picker == Picker::Model => self.picker_len() - 1,
            None => 0,
        };
    }

    /// Row of the current local selection; None when the model catalogue is uninspected.
    pub fn picker_current(&self) -> Option<usize> {
        match self.picker? {
            Picker::Model => {
                let catalogue = self.catalogues.get(&self.agent)?;
                Some(self.selection.model.as_ref().map_or(0, |id| {
                    catalogue
                        .models
                        .iter()
                        .position(|model| &model.id == id)
                        .map_or(0, |index| index + 1)
                }))
            }
            Picker::Effort => Some(self.selection.effort.as_ref().map_or(0, |effort| {
                self.effort_model()
                    .and_then(|model| model.efforts.iter().position(|value| value == effort))
                    .map_or(0, |index| index + 1)
            })),
        }
    }

    pub fn picker_len(&self) -> usize {
        match self.picker {
            Some(Picker::Model) => {
                2 + self
                    .catalogues
                    .get(&self.agent)
                    .map(|catalogue| catalogue.models.len())
                    .unwrap_or(0)
                    .max(1)
            }
            Some(Picker::Effort) => {
                1 + self
                    .effort_model()
                    .map(|model| model.efforts.len())
                    .unwrap_or(0)
                    .max(1)
            }
            None => 0,
        }
    }

    pub fn picker_choice(&self, index: usize) -> Option<super::commands::Choice> {
        use super::commands::Choice;
        if index >= self.picker_len() {
            return None;
        }
        if index == 0 {
            return Some(Choice::new(
                "selection-default",
                "Default (provider configuration)",
            ));
        }
        Some(match self.picker? {
            Picker::Model if index + 1 == self.picker_len() => Choice::new(
                "models-refresh",
                "Refresh / inspect available models (review connection first)",
            ),
            Picker::Model => {
                if let Some(catalogue) = self.catalogues.get(&self.agent) {
                    if let Some(model) = catalogue.models.get(index - 1) {
                        Choice::new("model-select", format!("{} [{}]", model.name, model.id))
                    } else {
                        Choice::disabled(
                            "model-select",
                            "Model selection unavailable",
                            format!(
                                "Provider advertised no selectable models. {}",
                                catalogue.source
                            ),
                        )
                    }
                } else {
                    Choice::disabled(
                        "model-select",
                        "No inspected models",
                        "Refresh requires a reviewed provider connection; opening this picker sends nothing.",
                    )
                }
            }
            Picker::Effort => {
                if let Some(effort) = self
                    .effort_model()
                    .and_then(|model| model.efforts.get(index - 1))
                {
                    Choice::new("effort-select", effort)
                } else {
                    Choice::disabled("effort-select", "Reasoning effort unavailable",
                        self.catalogues.get(&self.agent).and_then(|catalogue| catalogue.effort_note.clone())
                            .unwrap_or_else(|| "No supported reasoning effort values advertised for the selected/default model. Inspect models first.".into()))
                }
            }
        })
    }

    fn apply_selection(&mut self, selection: ModelSelection, fresh: bool) {
        let model_changed = selection.model != self.selection.model;
        self.picker = None;
        self.selection_change = None;
        if self.preview.is_some() {
            self.cancel();
        }
        self.selection = selection;
        self.selections.insert(self.agent, self.selection.clone());
        if fresh {
            self.thread_id = None;
            self.items.clear();
            self.selected = 0;
            self.omitted = 0;
            self.observed_model = None;
        }
        self.review_start();
        self.notice = "Selection updated locally. Previous payload consent invalidated; /new or compatible /resume requires fresh review. No connection was authorized.".into();
        // Continue straight to effort when the new model advertises choices.
        if model_changed
            && self.selection.model.is_some()
            && self
                .effort_model()
                .is_some_and(|model| !model.efforts.is_empty())
        {
            self.open_picker(Picker::Effort);
            self.notice = "Model updated locally. Choose reasoning effort, or Esc to keep the provider default.".into();
        }
    }

    pub fn command_choices(&self) -> Vec<super::commands::Choice> {
        use super::commands::Choice;
        if self.editing
            || self.paths_editor.is_some()
            || self.quit_review
            || self.switch_to.is_some()
            || self.selection_change.is_some()
        {
            return vec![Choice::new("cancel", "Back / keep existing choice")];
        }
        if self.journal_mode {
            if self.journal_purge {
                return vec![Choice::new(
                    "journal-close",
                    "Keep journal / close purge review",
                )];
            }
            return vec![
                Choice::new("journal-retry", "Retry retained journal storage"),
                if self.busy() {
                    Choice::disabled(
                        "journal-purge",
                        "Purge local provider journal",
                        "Wait for journal storage to finish.",
                    )
                } else {
                    Choice::new("journal-purge", "Review journal purge")
                },
                Choice::new("journal-close", "Back to provider session"),
            ];
        }
        if let Some(request) = &self.request {
            if let PromptKind::Questions(questions) = &request.kind
                && !self.submit_review
            {
                let mut rows = vec![Choice::new(
                    "answers-review",
                    "Review answers before sending",
                )];
                if questions
                    .get(self.question)
                    .is_some_and(|question| question.other || question.options.is_empty())
                {
                    rows.push(Choice::new("answer", "Edit literal answer"));
                }
                rows.push(Choice::new("cancel", "Cancel provider turn"));
                return rows;
            }
            return vec![Choice::new("cancel", "Cancel provider turn")];
        }
        if self.active || self.awaiting_preview {
            return vec![
                Choice::new("cancel", "Cancel owned request"),
                Choice::new("quit", "Review safe exit"),
            ];
        }
        if self.preview.is_some() {
            return vec![
                Choice::new("model", "Choose model / invalidate payload review"),
                Choice::new(
                    "effort",
                    "Choose reasoning effort / invalidate payload review",
                ),
                Choice::new("cancel", "Stay / cancel preview"),
            ];
        }
        if self.picker.is_some() {
            return vec![
                Choice::new("models-refresh", "Review model catalogue refresh"),
                Choice::new("cancel", "Close picker"),
            ];
        }
        let idle = !self.busy();
        let mut rows = Vec::new();
        for (command, label) in [
            ("new", "New provider session"),
            ("resume", "Resume owned same-task session"),
            ("goal", "Edit saved goal"),
            ("decision", "Copy selected proposal into NextAction"),
            ("model", "Choose model"),
            ("effort", "Choose reasoning effort"),
            (
                "models-refresh",
                "Review provider model catalogue inspection",
            ),
            ("sessions", "Inspect provider session metadata"),
            ("paths", "Edit selected write paths"),
            ("provider-journal", "Open local provider journal"),
            ("handoff-clear", "Deselect portable handoff"),
            ("provider-codex", "Choose Codex"),
            ("provider-claude", "Choose Claude"),
            ("provider-opencode", "Choose OpenCode"),
            ("provider-gemini", "Choose Gemini"),
            ("provider-devin", "Choose Devin"),
            ("role-plan", "Planning role"),
            ("role-implement", "Implementation role"),
            ("role-review", "Independent review role"),
        ] {
            let reason = if !idle {
                Some("Wait for pending local/provider work.".to_string())
            } else if command == "resume"
                && (self.operation == Operation::Review || self.thread_id.is_none())
            {
                Some("Resume requires an owned same-task session. Independent Review requires a fresh session; choose /new.".into())
            } else if matches!(
                command,
                "paths" | "sessions" | "role-plan" | "role-implement"
            ) && self.operation == Operation::Review
            {
                Some("Independent Review cannot authorize writes, resume history, or switch itself to an implementation role.".into())
            } else if command == "decision"
                && (self.inspection.is_some()
                    || !self.items.get(self.selected).is_some_and(|item| {
                        item.complete && matches!(item.kind, ItemKind::Message | ItemKind::Plan)
                    }))
            {
                Some("Select a complete provider message or plan; metadata/tool output is not a decision.".into())
            } else {
                None
            };
            rows.push(match reason {
                Some(reason) => Choice::disabled(command, label, reason),
                None => Choice::new(command, label),
            });
        }
        rows
    }

    pub fn command(
        &mut self,
        name: &str,
        large_enough: bool,
        task: &store::Snapshot,
        excerpts: &[context::Excerpt],
        report: Option<&Arc<repo::Report>>,
    ) -> bool {
        if name == "cancel" {
            return self.cancel();
        }
        if name == "quit" {
            self.request_quit();
            return true;
        }
        if !large_enough {
            self.invalidate_review();
            self.notice = "Terminal too small; no command action or approval available.".into();
            return true;
        }
        if self.task != task.id && !self.busy() && !self.journal_failed() {
            self.reset(task.id);
        }
        let refresh = name == "models-refresh";
        if refresh && self.picker.is_some() && !self.active && !self.awaiting_preview {
            self.picker = None;
        } else if let Some(choice) = self
            .command_choices()
            .iter()
            .find(|choice| choice.command == name)
        {
            if let Some(reason) = &choice.reason {
                self.notice = reason.clone();
                return true;
            }
        } else {
            self.notice = format!("/{name} is unavailable in the current provider context.");
            return true;
        }
        match name {
            "model" | "effort" => {
                self.open_picker(if name == "model" {
                    Picker::Model
                } else {
                    Picker::Effort
                });
                self.notice = "Local picker only. Default means provider configuration; inspection requires separate Preview/Run consent.".into();
            }
            "models-refresh" => {
                if self.preview.is_some() {
                    self.notice = "Close the existing preview before inspecting models.".into();
                } else {
                    self.prepare(false, false, true, task, excerpts, report);
                }
            }
            "new" => self.prepare(false, false, false, task, excerpts, report),
            "resume" => self.prepare(true, false, false, task, excerpts, report),
            "sessions" => self.prepare(false, true, false, task, excerpts, report),
            "goal" => self.edit_goal = true,
            _ if Agent::from_command(name).is_some() => {
                let next = Agent::from_command(name).expect("provider command");
                if next != self.agent {
                    if self.handoff.is_none() {
                        match crate::workspace::handoff::from_task(task, excerpts, &[]) {
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
            "role-plan" => self.set_operation(Operation::Plan),
            "role-implement" => self.set_operation(Operation::Implement),
            "role-review" => self.set_operation(Operation::Review),
            "paths" => {
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
            "provider-journal" => {
                self.journal_mode = true;
                self.journal_purge = false;
                self.review_start();
                self.pending_journal.push_back(store::JournalRequest {
                    task_id: self.task,
                    operation: store::JournalOperation::List,
                });
            }
            "journal-retry" => self.retry_journal(),
            "journal-purge" => {
                self.journal_purge = true;
                self.affirmative = false;
            }
            "journal-close" => {
                self.journal_mode = false;
                self.journal_purge = false;
            }
            "handoff-clear" => {
                self.handoff = None;
                self.notice = "Portable handoff deselected locally; no provider history or export was erased.".into();
            }
            "answer" => self.editing = true,
            "answers-review" => {
                if self
                    .answers
                    .iter()
                    .all(|answer| !answer.text.trim().is_empty())
                {
                    self.submit_review = true;
                    self.review_start();
                } else {
                    self.notice =
                        "Answer every question before reviewing submission. Nothing was sent."
                            .into();
                }
            }
            "decision" => self.copy_decision(),
            _ => return false,
        }
        true
    }

    fn copy_decision(&mut self) {
        if let Some(item) = self
            .items
            .get(self.selected)
            .filter(|item| item.complete && matches!(item.kind, ItemKind::Message | ItemKind::Plan))
        {
            let provenance = format!(
                "\n\n[User-selected, unverified {} {:?}; thread {}; turn {}; item {}. Not local execution evidence.]",
                self.agent.label(),
                self.operation,
                item.thread_id,
                item.turn_id,
                item.item_id
            );
            if item.text.len() + provenance.len() > 4096 {
                self.notice = "Decision exceeds the 4096-byte NextAction limit. Nothing was copied or truncated.".into();
            } else {
                self.decision = Some(format!("{}{}", item.text, provenance));
            }
        }
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

#[cfg(test)]
mod tests;
