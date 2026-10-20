//! Provider requests with durable nonsecret selection; provider text is never local execution evidence.
//! Authentication revokes future session ownership without rewriting submitted request inputs.
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::PathBuf,
    sync::Arc,
};

#[cfg(test)]
use crossterm::event::KeyModifiers;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
#[cfg(test)]
use serde_json::json;

use crate::{
    providers::{
        Agent, Event, ItemKind, ModelCatalogue, ModelSelection, Operation, Outcome, Preview,
        Prompt, PromptKind, Request,
    },
    store,
    workspace::context,
    workspace::repo,
};

mod cooperation;
pub(crate) mod journal;
mod picker;
mod preferences;
mod response;
mod review;

const ANSWER_BYTES: usize = 4096;
const ITEM_LIMIT: usize = 128;

pub struct Item {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub kind: ItemKind,
    pub text: String,
    pub complete: bool,
    /// Shared immutable request ownership; streaming snapshots never copy it.
    pub request: Arc<str>,
}

#[derive(Default)]
struct Session {
    thread_id: Option<String>,
    items: Vec<Item>,
    selected: usize,
    omitted: usize,
    selection: ModelSelection,
    observed_model: Option<String>,
    submitted_request: Arc<str>,
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

enum ModelPickerFocus {
    Default,
    Model,
    Custom,
}

/// A request's immutable task, partner and worker-local generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub owner: store::Attribution,
    pub task: i64,
    pub generation: Option<u64>,
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
    pub following: bool,
    pub new_output: bool,
    response_cache: response::Layout,
    activity: &'static str,
    response_scroll: usize,
    response_max: usize,
    pub reviewed: bool,
    /// Full technical review is optional; summary visibility still gates consent.
    pub review_details: bool,
    pub submitted_request: Arc<str>,
    pub(super) failed_request: bool,
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
    /// An old-account transport may finish, but none of its sessions may resume.
    authentication_invalidated: bool,
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
    pub(super) journal_recovery: journal::Recovery,
    /// Local receipt of the open preview's exact payload.
    pub receipt: Vec<String>,
    pub selection: ModelSelection,
    pub receipt_claim: Option<store::WorkClaim>,
    selections: HashMap<Agent, ModelSelection>,
    preferences: preferences::Preferences,
    pub catalogues: HashMap<Agent, ModelCatalogue>,
    catalogue_errors: HashMap<Agent, String>,
    pub picker: Option<Picker>,
    pub picker_selected: usize,
    pub picker_query: Answer,
    pub custom_model: Option<Answer>,
    /// Candidate model stays uncommitted until its reasoning choice is accepted.
    pending_model: Option<ModelSelection>,
    /// Query/caret remain in picker_query while reasoning owns the picker.
    model_picker_return: Option<ModelPickerFocus>,
    picker_models: Vec<usize>,
    catalogue_pending: Option<ModelCatalogue>,
    pub selection_change: Option<ModelSelection>,
    pub inspect_models: bool,
    /// A catalogue fetch is wanted (after a connection or an empty `/model`);
    /// the app starts it once the metadata-only request slot is free.
    pub auto_models: bool,
    /// The in-flight catalogue inspection was started automatically: its
    /// metadata-only preview runs without a review step.
    auto_run: bool,
    /// A deliberate composer Enter requests dispatch after local preparation, not a popup.
    pub(super) send_after_prepare: bool,
    /// Reopen the model picker when the automatic fetch completes.
    auto_picker: bool,
    /// Explicit dismissal owns cancellation; a late result must not reopen a hidden picker.
    catalogue_dismissed: bool,
    /// Why the automatic fetch stopped before running, shown after its cancel.
    auto_blocked: Option<String>,
    request_selection: ModelSelection,
    session_selection: ModelSelection,
    pub observed_model: Option<String>,
    /// Website-OAuth session credential for Devin ACP launches; memory only.
    pub devin_credential: Option<crate::providers::auth::SessionCredential>,
    /// The partner in front of the task; a prepared request binds to it.
    pub front: Option<store::Attribution>,
    /// Request slots keyed by partner id; journal records carry the owner (PT06A).
    slots: BTreeMap<i64, Slot>,
    pub work: Option<store::WorkClaim>,
    pub work_pending: Option<store::WorkRequest>,
    pub work_inflight: bool,
    pub work_acquired: bool,
    pub work_error: Option<String>,
    pub(super) committed_input: Option<Box<store::Snapshot>>,
    pub(super) held_input: Option<Box<crate::providers::Input>>,
    pub(super) claim_cancelled: bool,
    queued_prompts: VecDeque<Prompt>,
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
            review_details: false,
            submitted_request: Arc::default(),
            failed_request: false,
            following: true, new_output: false, response_cache: response::Layout::default(),
            activity: "Connecting",
            response_scroll: 0, response_max: 0,
            question: 0, option: 0, answers: Vec::new(), editing: false, submit_review: false,
            quit_review: false, quit_ready: false, cancelling: false, omitted: 0,
            task: 0, revision: 0, generation: None, awaiting_preview: false, active: false,
            authentication_invalidated: false,
            cancel_when_ready: false, quit_after_cancel: false, unknown: false, saved_scroll: 0,
            agent: Agent::Codex, operation: Operation::Plan, edit_paths: Vec::new(),
            paths_editor: None, switch_to: None, handoff: None, sessions: HashMap::new(),
            journal_mode: false, journal_purge: false, journal_records: Vec::new(), journal_selected: 0,
            pending_journal: VecDeque::new(), journal_inflight: VecDeque::new(),
            failed_journal: VecDeque::new(), journal_notice: String::new(),
            journal_recovery: journal::Recovery::default(),
            receipt: Vec::new(),
            inspection: None, inspect_sessions: false,
            selection: ModelSelection::default(), selections: HashMap::new(),
            preferences: preferences::Preferences::default(),
            catalogues: HashMap::new(), catalogue_errors: HashMap::new(), picker: None, picker_selected: 0,
            picker_query: Answer::default(), custom_model: None, pending_model: None, model_picker_return: None, picker_models: Vec::new(),
            receipt_claim: None,
            catalogue_pending: None,
            selection_change: None, inspect_models: false,
            auto_models: false, auto_run: false, auto_picker: false, catalogue_dismissed: false, auto_blocked: None, send_after_prepare: false,
            request_selection: ModelSelection::default(), session_selection: ModelSelection::default(),
            observed_model: None,
            devin_credential: None,
            front: None,
            slots: BTreeMap::new(),
            work: None, work_pending: None, work_inflight: false, work_error: None,
            committed_input: None, held_input: None, claim_cancelled: false,
            work_acquired: false,
            queued_prompts: VecDeque::new(),
        }
    }

    pub fn matches_task(&self, task_id: i64, revision: u64) -> bool {
        self.task == task_id && self.revision == revision
    }

    pub fn needs_saved_input(&self, key: &KeyEvent) -> bool {
        key.kind == KeyEventKind::Press
            && !self.inspect_models
            && !self.editing
            && !self.quit_review
            && !self.journal_mode
            && self.paths_editor.is_none()
            && self.custom_model.is_none()
            && self.preview.is_some()
            && ((self.affirmative && key.code == KeyCode::Enter)
                || (key.modifiers.is_empty()
                    && key.code == KeyCode::Char('y')
                    && self.reviewed
                    && !self.review_details))
    }

    pub fn needs_saved_command(&self, name: &str) -> bool {
        !self.active
            && !self.awaiting_preview
            && !self.journal_mode
            && (matches!(name, "new" | "resume" | "sessions")
                || name.starts_with("provider-")
                    && name != "provider-journal"
                    && (self.thread_id.is_some()
                        || !self.items.is_empty()
                        || self.handoff.is_some()))
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
            || self.selection_change.is_some()
            || self.paths_editor.is_some()
            || self.custom_model.is_some()
            || !self.pending_journal.is_empty()
            || !self.journal_inflight.is_empty()
            || self.work.is_some()
            || self.work_pending.is_some()
            || self.work_inflight
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
            let preferences = std::mem::take(&mut self.preferences);
            let catalogues = std::mem::take(&mut self.catalogues);
            let catalogue_errors = std::mem::take(&mut self.catalogue_errors);
            let journal_recovery = std::mem::take(&mut self.journal_recovery);
            let selection = self.selection.clone();
            let credential = self.devin_credential.take();
            let front = self.front.take();
            let agent = self.agent;
            let (auto_models, auto_picker) = (self.auto_models, self.auto_picker);
            *self = Self::new();
            self.auto_models = auto_models;
            self.auto_picker = auto_picker;
            self.task = task_id;
            self.journal_recovery = journal_recovery;
            self.refresh_journal_gap_notice();
            self.agent = agent;
            self.selection = selection;
            self.selections = selections;
            self.preferences = preferences;
            self.catalogues = catalogues;
            self.catalogue_errors = catalogue_errors;
            self.devin_credential = credential;
            self.front = front;
        }
    }

    /// Replaces only the credential for future Devin ACP preparations; `None`
    /// detaches it. Held inputs and launched transports retain their exact bytes.
    pub fn set_devin_credential(
        &mut self,
        credential: Option<crate::providers::auth::SessionCredential>,
    ) {
        self.devin_credential = credential;
    }

    /// Revoke account-scoped caches and grants in every state, including parked
    /// states still settling journals or claims. An already submitted transport
    /// keeps its immutable input and settles before its visible session clears.
    pub fn invalidate_authentication(&mut self) {
        self.picker = None;
        self.custom_model = None;
        self.selection_change = None;
        self.pending_model = None;
        self.model_picker_return = None;
        self.switch_to = None;
        self.catalogues.clear();
        self.catalogue_errors.clear();
        self.catalogue_pending = None;
        self.picker_models.clear();
        self.devin_credential = None;
        self.auto_models = false;
        self.auto_picker = false;
        self.auto_run = false;
        self.auto_blocked = None;
        self.catalogue_dismissed = true;
        self.send_after_prepare = false;
        self.invalidate_review();
        self.sessions.clear();
        if self.inspect_models && self.transport_pending() {
            self.catalogue_errors.insert(
                self.agent,
                "Authentication changed; refresh models with the current account.".into(),
            );
        }
        if self.active {
            self.authentication_invalidated = true;
            self.notice = "Authentication changed. The submitted request keeps its original input; its old-account session cannot resume after settlement.".into();
            return;
        }
        // Acquisition may already be in flight. Keep the held input unchanged
        // until its matching acknowledgment releases the exact owned claim.
        if self.held_input.is_some() {
            self.claim_cancelled = true;
            self.awaiting_preview = false;
        }
        self.clear_provider_sessions();
        if self.transport_pending() {
            self.authentication_invalidated = true;
            self.cancel();
            self.notice = "Authentication changed. Cancelling the stale preparation; prepare a fresh request after settlement.".into();
            return;
        }
        self.authentication_invalidated = false;
        self.inspect_sessions = false;
        self.inspect_models = false;
        self.decision = None;
        self.generation = None;
        self.close_request();
        self.review_start();
        self.notice = "Authentication may have changed. Provider sessions and consent cleared; prepare a fresh request.".into();
    }

    /// Keep the provider worker registry while any current-account session,
    /// including another agent/operation's cached session, still owns it.
    pub fn has_resumable_sessions(&self) -> bool {
        !self.authentication_invalidated
            && (self.thread_id.is_some()
                || self
                    .sessions
                    .values()
                    .any(|session| session.thread_id.is_some()))
    }

    fn clear_provider_sessions(&mut self) {
        self.sessions.clear();
        self.thread_id = None;
        self.observed_model = None;
        self.items.clear();
        self.submitted_request = Arc::default();
        self.selected = 0;
        self.omitted = 0;
        self.inspection = None;
        self.response_cache.invalidate();
    }

    fn finish_authentication_invalidation(&mut self) {
        if !self.authentication_invalidated {
            return;
        }
        self.authentication_invalidated = false;
        self.clear_provider_sessions();
        self.inspect_sessions = false;
        self.inspect_models = false;
        self.decision = None;
        self.review_start();
        self.notice.push_str(" Authentication changed; the old-account session was revoked. Prepare a fresh request.");
    }

    fn matches(&self, generation: u64, task: i64) -> bool {
        self.generation == Some(generation)
            && self.task == task
            && (self.front.is_none()
                || self
                    .slots
                    .values()
                    .any(|slot| slot.task == task && slot.generation == Some(generation)))
    }

    /// Bind the next request to the partner in front; an unattributed state
    /// (no lead known yet) routes by task and generation alone.
    fn bind_slot(&mut self, task: i64) {
        self.slots.clear();
        self.queued_prompts.clear();
        if let Some(owner) = self.front.clone() {
            self.slots.insert(
                owner.partner,
                Slot {
                    owner,
                    task,
                    generation: None,
                },
            );
        }
    }

    /// The partner whose slot holds the current or most recent request.
    pub fn owner(&self) -> Option<&store::Attribution> {
        self.slots.values().next().map(|slot| &slot.owner)
    }

    /// Drop slots whose partner left the party: their late events no longer
    /// match any slot and are refused instead of reaching another partner.
    /// Live work keeps its slot so its own completion still settles; releasing
    /// a working partner is refused before it reaches here.
    pub fn retain_partners(&mut self, partners: &[i64]) {
        if self.busy_except_picker() {
            return;
        }
        let before = self.slots.len();
        self.slots.retain(|id, _| partners.contains(id));
        if self.slots.len() != before {
            self.notice =
                "The partner that owned this request left the party; its late events are refused."
                    .into();
        }
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
                submitted_request: std::mem::take(&mut self.submitted_request),
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
        self.submitted_request = session.submitted_request;
        self.notice = if operation == Operation::Review {
            "Independent Review selected locally. New always constructs fresh requirements/patch/evidence context and a separate provider session. Resume and source writes are unavailable; fixes require a deliberate implementation handoff.".into()
        } else {
            "Role policy selected locally. New/Resume requires fresh scope and executable consent; no permission carried.".into()
        };
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
                submitted_request: std::mem::take(&mut self.submitted_request),
            },
        );
        self.selections.insert(self.agent, self.selection.clone());
        self.agent = next;
        self.selection = self.selections.get(&next).cloned().unwrap_or_default();
        self.preference_changed();
        // A queued inspection belongs to the old provider, not the new selection.
        self.auto_models = false;
        self.auto_picker = false;
        self.auto_run = false;
        self.auto_blocked = None;
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
        self.submitted_request = session.submitted_request;
        self.generation = None;
        self.close_request();
        self.review_start();
        self.notice = format!(
            "{} selected locally. No connection or request authorized; sending still requires a fresh review.",
            next.label()
        );
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
                for slot in self.slots.values_mut() {
                    slot.generation = Some(preview.generation);
                }
                self.receipt = crate::workspace::receipt::receipt(&preview.payload).lines();
                self.preview = Some(preview);
                self.notice = "Review request scope and effects; Stay is the default. Details remain inspectable.".into();
                self.review_start();
                if self.auto_run && self.inspect_models && !self.cancel_when_ready {
                    self.run_automatic_catalogue();
                }
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
                && !self.cancelling
                && !self.authentication_invalidated =>
            {
                match catalogue.validate() {
                    Ok(()) => {
                        // Publish only after successful completion; a late failure
                        // must not replace the last successful account-scoped snapshot.
                        self.catalogue_pending = Some(catalogue);
                    }
                    Err(reason) => {
                        self.notice = format!("Model catalogue rejected: {reason}");
                        self.catalogue_errors
                            .insert(self.agent, self.notice.clone());
                    }
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
            Event::ModelObserved {
                generation,
                task_id,
                model,
            } if self.matches(generation, task_id)
                && self.active
                && !self.inspect_sessions
                && !self.inspect_models =>
            {
                self.activity = "Waiting for response";
                self.observed_model = Some(model.clone());
                self.notice = format!(
                    "Provider acknowledged model {model}; fresh HTTP request, not a resumable session."
                );
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
                self.activity = "Waiting for response";
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
                self.activity = "Receiving response";
                let position = self.items.iter().position(|item| {
                    item.thread_id == thread_id
                        && item.turn_id == turn_id
                        && item.item_id == item_id
                });
                let index = if let Some(index) = position {
                    let item = &mut self.items[index];
                    item.text = text;
                    item.complete = complete;
                    item.kind = kind;
                    index
                } else {
                    // A replayed observation from an already retained turn
                    // belongs to that turn, not the newly submitted request.
                    let request = Arc::clone(
                        self.items
                            .iter()
                            .find(|item| item.thread_id == thread_id && item.turn_id == turn_id)
                            .map_or(&self.submitted_request, |item| &item.request),
                    );
                    if self.items.len() == ITEM_LIMIT {
                        // Keep an explicitly inspected item even when the bounded
                        // display fills; discard the oldest other observation.
                        let removed = usize::from(
                            self.selected == 0 && (!self.following || !self.response_surface()),
                        );
                        self.items.remove(removed);
                        if self.selected > removed {
                            self.selected -= 1;
                        }
                        self.response_cache.invalidate();
                        self.omitted += 1;
                    }
                    self.items.push(Item {
                        thread_id,
                        turn_id,
                        item_id,
                        kind,
                        text,
                        complete,
                        request,
                    });
                    self.items.len() - 1
                };
                self.response_cache.item_changed(index);
                self.response_arrived(index);
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
                if self.request.is_none() {
                    self.install_prompt(request);
                } else if self
                    .request
                    .as_ref()
                    .is_none_or(|open| open.request_id != request.request_id)
                    && !self
                        .queued_prompts
                        .iter()
                        .any(|open| open.request_id == request.request_id)
                {
                    self.queued_prompts.push_back(request);
                }
            }
            Event::PromptClosed {
                generation,
                request_id,
            } if self.matches(generation, self.task)
                && (self
                    .request
                    .as_ref()
                    .is_some_and(|request| request.request_id == request_id)
                    || self
                        .queued_prompts
                        .iter()
                        .any(|request| request.request_id == request_id)) =>
            {
                if self
                    .request
                    .as_ref()
                    .is_some_and(|request| request.request_id == request_id)
                {
                    self.close_request();
                    self.promote_prompt();
                } else {
                    self.queued_prompts
                        .retain(|request| request.request_id != request_id);
                }
            }
            Event::Finished {
                generation,
                task_id,
                outcome,
                message,
            } if self.matches(generation, task_id) => {
                self.failed_request = self.active
                    && !self.inspect_models
                    && !self.inspect_sessions
                    && !matches!(outcome, Outcome::Completed)
                    && !self
                        .items
                        .iter()
                        .any(|item| Arc::ptr_eq(&item.request, &self.submitted_request));
                self.active = false;
                self.awaiting_preview = false;
                self.cancelling = false;
                self.preview = None;
                self.pending = None;
                self.close_request();
                self.queued_prompts.clear();
                self.journal_terminal(generation, task_id, !matches!(outcome, Outcome::Unknown));
                self.generation = None;
                self.notice = message;
                if self.inspect_models {
                    if let Some(blocked) = self.auto_blocked.take() {
                        self.catalogue_errors.insert(self.agent, blocked);
                    } else if !matches!(outcome, Outcome::Completed) {
                        self.catalogue_errors
                            .entry(self.agent)
                            .or_insert_with(|| self.notice.clone());
                    } else if self.catalogue_pending.is_none() {
                        self.catalogue_errors.entry(self.agent).or_insert_with(|| {
                            "Provider completed without a usable model catalogue.".into()
                        });
                    }
                    if let Some(catalogue) = self.catalogue_pending.take()
                        && !self.catalogue_errors.contains_key(&self.agent)
                    {
                        self.publish_catalogue(catalogue);
                    }
                    if let Some(reason) = self.catalogue_errors.get(&self.agent) {
                        self.notice = format!(
                            "{reason} Cached models, if any, are stale. Retry with /model or review /models-refresh."
                        );
                    } else if self.auto_run {
                        self.notice = "Models loaded.".into();
                    }
                    if !self.quit_after_cancel
                        && !self.catalogue_dismissed
                        && (!self.auto_run || self.auto_picker)
                        && self.picker.is_none()
                    {
                        self.open_picker(Picker::Model);
                    }
                    self.auto_run = false;
                    self.auto_picker = false;
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
                self.finish_authentication_invalidation();
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
                if self.inspect_models {
                    self.catalogue_errors
                        .insert(self.agent, self.notice.clone());
                    self.notice
                        .push_str(" Retry with /model or review /models-refresh.");
                    if !self.active {
                        if !self.quit_after_cancel
                            && !self.catalogue_dismissed
                            && (!self.auto_run || self.auto_picker)
                            && self.picker.is_none()
                        {
                            self.open_picker(Picker::Model);
                        }
                        self.auto_run = false;
                        self.auto_picker = false;
                        self.auto_blocked = None;
                    }
                }
                if !self.active {
                    self.generation = None;
                    self.cancel_when_ready = false;
                    if self.quit_after_cancel {
                        self.quit_after_cancel = false;
                        self.quit_ready = true;
                    }
                    self.finish_authentication_invalidation();
                }
            }
            _ => {}
        }
    }

    pub fn cancel(&mut self) -> bool {
        self.send_after_prepare = false;
        if self.held_input.is_some() {
            self.claim_cancelled = true;
            self.held_input = None;
            self.awaiting_preview = false;
            self.notice = "Cancelling the owned reservation; no provider was started. Waiting for storage settlement.".into();
            return true;
        }
        self.invalidate_review();
        if self.inspect_models && self.auto_blocked.is_none() {
            self.catalogue_dismissed = true;
            self.auto_picker = false;
        }
        // Closing the metadata surface must not swallow owned-request cancellation.
        if self.inspect_models && (self.active || self.awaiting_preview) {
            self.picker = None;
            self.pending_model = None;
            self.model_picker_return = None;
        }
        if self.switch_to.take().is_some()
            || self.paths_editor.take().is_some()
            || self.custom_model.take().is_some()
            || self.selection_change.take().is_some()
            || self.picker.take().is_some()
        {
            if self.picker.is_none() {
                self.pending_model = None;
                self.model_picker_return = None;
            }
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
        if self.preview.take().is_some() {
            self.scroll = self.saved_scroll;
        }
        self.close_request();
        self.queued_prompts.clear();
        self.active = true;
        self.cancelling = true;
        self.notice = "Cancellation requested. Waiting for provider acknowledgement or bounded owned teardown; outcome not yet known.".into();
        true
    }

    pub fn command_choices(&self) -> Vec<super::commands::Choice> {
        use super::commands::Choice;
        if self.editing
            || self.paths_editor.is_some()
            || self.custom_model.is_some()
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
                match self.journal_discard_reason() {
                    Some(reason) => Choice::disabled(
                        "journal-discard",
                        "Review exact failed observation writes",
                        reason,
                    ),
                    None => {
                        Choice::new("journal-discard", "Review exact failed observation writes")
                    }
                },
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
                Choice::new("model", "Choose models / retry metadata loading"),
                Choice::new("effort", "Choose reasoning effort"),
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
            ("models-refresh", "Refresh provider model metadata"),
            ("sessions", "Inspect provider session metadata"),
            ("paths", "Edit selected write paths"),
            ("provider-journal", "Open local provider journal"),
            ("handoff-clear", "Deselect portable handoff"),
            ("provider-codex", "Codex CLI method (tools / edits)"),
            (
                "provider-chatgpt",
                "Codex native method (responses / no tools)",
            ),
            ("provider-claude", "Choose Claude"),
            ("provider-opencode", "Choose OpenCode"),
            ("provider-gemini", "Choose Gemini"),
            ("provider-devin", "Choose Devin"),
            ("provider-openai", "Choose OpenAI API"),
            ("provider-anthropic", "Choose Anthropic API"),
            ("provider-google", "Choose Google Gemini API"),
            ("provider-compatible", "Choose custom/local API"),
            ("role-plan", "Planning role"),
            ("role-implement", "Implementation role"),
            ("role-review", "Independent review role"),
        ] {
            let reason = if !idle {
                Some("Wait for pending local/provider work.".to_string())
            } else if self.agent.is_direct() && matches!(command, "resume" | "sessions") {
                Some("Direct API requests are fresh and stateless here; no owned provider session can be resumed or inspected. Choose /new.".into())
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
        if name == "journal-retry" {
            self.retry_journal();
            return true;
        }
        if name == "provider-journal" && large_enough {
            self.journal_mode = true;
            self.journal_purge = false;
            self.review_start();
            self.pending_journal.push_back(store::JournalRequest {
                task_id: self.task,
                operation: store::JournalOperation::List,
            });
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
                self.notice = if self.catalogue_errors.contains_key(&self.agent) {
                    self.auto_models = true;
                    self.auto_picker = true;
                    "Cached models are stale. Retrying provider metadata only…".into()
                } else if self.catalogues.contains_key(&self.agent) {
                    "Choose a model, then reasoning level. Esc keeps current settings.".into()
                } else {
                    // Nothing cached yet: fetch the catalogue and reopen here.
                    self.auto_models = true;
                    self.auto_picker = true;
                    "Loading models…".into()
                };
            }
            "models-refresh" => {
                if self.preview.is_some() {
                    self.notice = "Close the existing preview before inspecting models.".into();
                } else {
                    self.auto_picker = true;
                    self.start_automatic_catalogue(task, excerpts, report);
                }
            }
            "new" => self.prepare(false, false, false, task, excerpts, report),
            "resume" => self.prepare(true, false, false, task, excerpts, report),
            "sessions" => self.prepare(false, true, false, task, excerpts, report),
            "goal" => self.edit_goal = true,
            _ if Agent::from_command(name).is_some() => {
                let next = Agent::from_command(name).expect("provider command");
                if next != self.agent {
                    if self.thread_id.is_none() && self.items.is_empty() && self.handoff.is_none() {
                        self.switch_to = Some(next);
                        self.complete_switch();
                        return true;
                    }
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
                } else {
                    self.preference_changed();
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
            "journal-purge" => {
                self.journal_purge = true;
                self.invalidate_review();
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
}

#[cfg(test)]
mod tests;
