//! Task-correlated command approval, persisted execution evidence, and interruption UX.
//! This state machine performs no filesystem/process I/O; workers own those boundaries.
use std::collections::VecDeque;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use sha2::{Digest, Sha256};
use unicode_segmentation::UnicodeSegmentation;

use crate::{store, workspace::repo, workspace::runner};

const LIVE_BYTES: usize = 256 * 1024;
const LIVE_LINES: usize = 4096;
const STORE_QUEUE: usize = 16;
pub const PAGE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    EditCommand,
    SelectForm(usize),
    SwitchTask(i64),
    NewTask,
    ResetTask,
    Quit,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Output,
    History,
    Details,
}
#[derive(Clone, Debug)]
pub enum Confirmation {
    Approval,
    Cancel,
    Purge(String),
    Navigate(Action),
    UnknownExit(Action),
    UnsavedJournalExit(Action),
}
#[derive(Clone, Debug)]
pub struct LayerError {
    pub layer: &'static str,
    pub message: String,
    pub remedy: &'static str,
}
#[derive(Clone, Debug)]
pub struct OutputLine {
    pub id: u64,
    pub text: String,
}
#[derive(Default)]
pub struct OutputWindow {
    pub lines: VecDeque<OutputLine>,
    pub first: u64,
    pub next: u64,
    pub bytes: usize,
    pub evicted: u64,
}
impl OutputWindow {
    fn append(&mut self, value: &str) -> usize {
        let mut count = 0;
        for part in value.split_inclusive('\n') {
            let text = safe_text(part.trim_end_matches('\n'));
            self.bytes += text.len();
            self.lines.push_back(OutputLine {
                id: self.next,
                text,
            });
            self.next += 1;
            count += 1;
        }
        while self.bytes > LIVE_BYTES || self.lines.len() > LIVE_LINES {
            if let Some(line) = self.lines.pop_front() {
                self.bytes = self.bytes.saturating_sub(line.text.len());
                self.first = line.id + 1;
                self.evicted += 1;
            }
        }
        count
    }
}

#[derive(Clone, PartialEq, Eq)]
enum StoreJob {
    Reserve(runner::Prepared, u64),
    Update(runner::RunRecord),
    Abandon {
        task: i64,
        request: String,
        run: String,
    },
    Load(i64),
    Read {
        task: i64,
        run: String,
        offset: u64,
    },
    Purge {
        task: i64,
        run: String,
    },
}
impl StoreJob {
    fn task(&self) -> i64 {
        match self {
            Self::Reserve(p, _) => p.task_id,
            Self::Update(r) => r.prepared.task_id,
            Self::Abandon { task, .. }
            | Self::Read { task, .. }
            | Self::Purge { task, .. }
            | Self::Load(task) => *task,
        }
    }
    fn correlation(&self) -> (Option<&str>, Option<&str>) {
        match self {
            Self::Reserve(p, _) => (Some(&p.request_id), Some(&p.run_id)),
            Self::Update(r) => (Some(&r.prepared.request_id), Some(&r.prepared.run_id)),
            Self::Abandon { request, run, .. } => (Some(request), Some(run)),
            Self::Read { run, .. } | Self::Purge { run, .. } => (None, Some(run)),
            Self::Load(_) => (None, None),
        }
    }
    fn critical(&self) -> bool {
        matches!(
            self,
            Self::Reserve(..) | Self::Update(_) | Self::Abandon { .. }
        )
    }
    fn request(&self) -> store::RunRequest {
        let operation = match self {
            Self::Reserve(p, capacity) => store::RunOperation::Reserve {
                prepared: p.clone(),
                capacity: *capacity,
            },
            Self::Update(r) => store::RunOperation::Update(r.clone()),
            Self::Abandon { request, run, .. } => store::RunOperation::Abandon {
                request_id: request.clone(),
                run_id: run.clone(),
            },
            Self::Load(_) => store::RunOperation::Load,
            Self::Read { run, offset, .. } => store::RunOperation::ReadOutput {
                run_id: run.clone(),
                offset: *offset,
                limit: PAGE_BYTES,
            },
            Self::Purge { run, .. } => store::RunOperation::PurgeOutput {
                run_id: run.clone(),
            },
        };
        store::RunRequest {
            task_id: self.task(),
            operation,
        }
    }
}
#[derive(Clone, Copy, Default)]
enum RepoPurpose {
    #[default]
    Inspect,
    Before,
    After,
}

pub struct State {
    pub task_id: i64,
    pub shown: bool,
    pub view: View,
    pub prepared: Option<runner::Prepared>,
    pub active: Option<runner::RunRecord>,
    pub history: Vec<runner::RunRecord>,
    pub selected_run: Option<String>,
    pub history_index: usize,
    pub live: OutputWindow,
    pub retained: OutputWindow,
    pub retained_page: Option<store::OutputPage>,
    pub showing_retained: bool,
    pub follow: bool,
    pub anchor: u64,
    pub horizontal: u16,
    pub new_output: usize,
    pub viewport_rows: usize,
    pub detail_scroll: u16,
    pub detail_max: u16,
    pub search: String,
    pub searching: bool,
    pub confirmation: Option<Confirmation>,
    pub affirmative: bool,
    pub confirmation_visible: bool,
    pub error: Option<LayerError>,
    pub notice: String,
    pub capture_capacity: u64,
    pub latest: runner::InputSnapshot,
    pub repo_waiting: bool,
    pub journal_saving: bool,
    request_id: Option<String>,
    pending_spec: Option<runner::CommandSpec>,
    before_ready: Option<runner::InputSnapshot>,
    final_record: Option<runner::RunRecord>,
    final_ack: Option<String>,
    dispatching: bool,
    reserving: bool,
    permit: Option<store::CommittedRunPermit>,
    invalidation: bool,
    cancellation: bool,
    queue: VecDeque<StoreJob>,
    inflight: Option<StoreJob>,
    failed: Option<StoreJob>,
    repo_request: bool,
    repo_purpose: RepoPurpose,
    repo_generation: u64,
    observed_generation: u64,
    page_offsets: Vec<u64>,
    live_run: Option<String>,
    deferred: Option<Action>,
    ready: Option<Action>,
    final_unknown: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            task_id: 0,
            shown: false,
            view: View::Output,
            prepared: None,
            active: None,
            history: Vec::new(),
            selected_run: None,
            history_index: 0,
            live: OutputWindow::default(),
            retained: OutputWindow::default(),
            retained_page: None,
            showing_retained: false,
            follow: true,
            anchor: 0,
            horizontal: 0,
            new_output: 0,
            viewport_rows: 1,
            detail_scroll: 0,
            detail_max: 0,
            search: String::new(),
            searching: false,
            confirmation: None,
            affirmative: false,
            confirmation_visible: false,
            error: None,
            notice: String::new(),
            capture_capacity: 64 * 1024,
            latest: runner::InputSnapshot::default(),
            repo_waiting: false,
            journal_saving: false,
            request_id: None,
            pending_spec: None,
            before_ready: None,
            final_record: None,
            final_ack: None,
            dispatching: false,
            reserving: false,
            permit: None,
            invalidation: false,
            cancellation: false,
            queue: VecDeque::new(),
            inflight: None,
            failed: None,
            repo_request: false,
            repo_purpose: RepoPurpose::Inspect,
            repo_generation: 0,
            observed_generation: 0,
            page_offsets: Vec::new(),
            live_run: None,
            deferred: None,
            ready: None,
            final_unknown: false,
        }
    }
}
impl State {
    pub fn set_capture_capacity(&mut self, bytes: u64) {
        self.capture_capacity = bytes.clamp(1024, 8 * 1024 * 1024);
    }
    pub fn activity_label(&self) -> &'static str {
        if let Some(record) = &self.active {
            return match record.state {
                runner::RunState::Running => "Local command running",
                runner::RunState::Cancelling => "Cancelling local command",
                runner::RunState::OutcomeUnknown => "Local outcome unknown",
                _ => "Finalizing local evidence",
            };
        }
        if self.final_unknown {
            return "Local outcome unknown / j Inspect";
        }
        if self.failed.is_some() {
            return "Local journal save failed";
        }
        if self.dispatching {
            return "Preparing approved command";
        }
        if self.reserving {
            return "Reserving command output";
        }
        if self.prepared.is_some() {
            return "Local approval needed";
        }
        if self.request_id.is_some() || self.pending_spec.is_some() {
            return "Preparing local command";
        }
        if self.journal_saving {
            return "Saving local journal";
        }
        "Local work idle"
    }
    pub fn visible(&self) -> bool {
        self.shown
    }
    pub fn open(&mut self, task_id: i64) {
        if self.task_id != task_id && !self.busy() {
            self.attach(task_id);
        }
        self.shown = true;
        if self.history.is_empty() {
            self.optional(StoreJob::Load(task_id));
        }
    }
    fn attach(&mut self, task_id: i64) {
        let capacity = self.capture_capacity;
        let inflight = self.inflight.take(); // A late read/load response is still consumed, never reassigned.
        *self = Self::default();
        self.task_id = task_id;
        self.capture_capacity = capacity;
        self.inflight = inflight;
        if task_id > 0 {
            self.optional(StoreJob::Load(task_id));
        }
    }
    pub fn busy(&self) -> bool {
        self.pending_spec.is_some()
            || self.request_id.is_some()
            || self.prepared.is_some()
            || self.reserving
            || self.dispatching
            || self.permit.is_some()
            || self.active.is_some()
            || self.final_record.is_some()
            || self.final_ack.is_some()
            || self.inflight.as_ref().is_some_and(StoreJob::critical)
            || self.failed.as_ref().is_some_and(StoreJob::critical)
            || self.queue.iter().any(StoreJob::critical)
    }
    pub fn invalidate(&mut self) {
        self.invalidation = true;
        self.request_id = None;
        self.prepared = None;
        self.pending_spec = None;
        self.before_ready = None;
        self.queue
            .retain(|job| !matches!(job, StoreJob::Reserve(..)));
        self.reserving = self
            .inflight
            .as_ref()
            .is_some_and(|job| matches!(job, StoreJob::Reserve(..)));
        if matches!(self.confirmation, Some(Confirmation::Approval)) {
            self.confirmation = None;
        }
        if matches!(self.repo_purpose, RepoPurpose::Before) {
            self.repo_generation += 1;
            self.repo_request = false;
        }
    }
    pub fn prepare_command(&mut self, task_id: i64, command_text: &str) -> bool {
        if task_id != self.task_id && self.busy() {
            self.fail(
                "Ownership",
                "A pending command still belongs to another task.",
                "Return to its task and deny/cancel it before switching.",
            );
            return false;
        }
        if self.active.is_some() || self.dispatching || self.final_ack.is_some() {
            self.fail(
                "Command",
                "The dispatched command cannot be edited. Its draft remains unchanged.",
                "Cancel and wait, or wait for completion before editing.",
            );
            return false;
        }
        self.invalidate();
        if task_id <= 0 {
            self.fail(
                "Storage",
                "Save a task before preparing a command.",
                "Create/save the task, then edit its command.",
            );
            return false;
        }
        match parse_command(command_text) {
            Ok(spec) => {
                self.task_id = task_id;
                self.pending_spec = Some(spec);
                self.before_ready = None;
                self.error = None;
                self.request_repo(RepoPurpose::Before);
                self.notice =
                    "Refreshing source identities before preparing; nothing has executed.".into();
                true
            }
            Err(message) => {
                self.fail("Command", message, "Use a JSON argv array or an explicit shell object; no command is split implicitly.");
                false
            }
        }
    }
    pub fn drive(&mut self, worker: &runner::Worker, task_id: i64, cwd: &Path) -> Option<Action> {
        if self.invalidation {
            worker.invalidate();
            self.invalidation = false;
        }
        if task_id != self.task_id {
            if self.busy() {
                worker.invalidate();
                if let Some(active) = &self.active {
                    worker.cancel(&active.prepared.run_id);
                }
                self.fail(
                    "Ownership",
                    "The active request still belongs to its original task.",
                    "Return to the owning task; do not transfer a running request.",
                );
                return None;
            }
            self.attach(task_id);
        }
        if let Some(permit) = self.permit.take() {
            let current = self.prepared.as_ref().is_some_and(|p| {
                p.request_id == permit.request_id
                    && p.run_id == permit.run_id
                    && p.task_id == task_id
            }) && self.request_id.as_deref() == Some(&permit.request_id)
                && self.deferred.is_none();
            if current {
                let request = permit.request_id.clone();
                let run = permit.run_id.clone();
                match worker.dispatch(&request, permit) {
                    Ok(()) => {
                        self.dispatching = true;
                        self.notice = "Committed capture reservation; dispatch is revalidating the one-shot approval.".into();
                    }
                    Err(error) => {
                        self.critical(StoreJob::Abandon {
                            task: task_id,
                            request,
                            run,
                        });
                        self.invalidate();
                        self.fail("Approval", error, "Prepare and deliberately review a new request. The command is not retried.");
                    }
                }
            } else {
                self.critical(StoreJob::Abandon {
                    task: permit.task_id,
                    request: permit.request_id.clone(),
                    run: permit.run_id.clone(),
                });
                drop(permit);
            }
        }
        if self.cancellation {
            let run = self
                .active
                .as_ref()
                .map(|r| &r.prepared.run_id)
                .or_else(|| {
                    self.prepared
                        .as_ref()
                        .filter(|_| self.dispatching)
                        .map(|p| &p.run_id)
                });
            if run.is_some_and(|run| worker.cancel(run)) {
                self.cancellation = false;
            } else if self.dispatching {
                // A queued dispatch may not have reached admission yet. Invalidate its grant,
                // and keep the cancellation intention until rejection or actual ownership.
                worker.invalidate();
            } else {
                self.cancellation = false;
            }
        }
        if let Some(before) = self.before_ready.take()
            && let Some(spec) = self.pending_spec.take()
        {
            match worker.prepare(task_id, spec, cwd.to_path_buf(), before) {
                Ok(id) => { self.request_id = Some(id); self.notice = "Preparing exact command identity; approval remains pending, not granted.".into(); }
                Err(error) => self.fail("Command", error, "Keep the draft and prepare again deliberately after resolving the prerequisite."),
            }
        }
        self.maybe_navigate();
        self.ready.take()
    }
    pub fn runner_event(&mut self, event: runner::Event) {
        match event {
            runner::Event::Prepared(p) => {
                if p.task_id == self.task_id && self.request_id.as_deref() == Some(&p.request_id) {
                    self.prepared = Some(p);
                    self.notice = "Approval pending. Use /run to inspect; opening/dismissing the view never approves.".into();
                    // Do not open a view or move focus while the user is typing elsewhere.
                }
            }
            runner::Event::Rejected {
                request_id,
                task_id,
                message,
            } => {
                if task_id != self.task_id {
                    return;
                }
                if self.request_id.as_deref() != Some(&request_id) {
                    return;
                }
                if self.dispatching
                    && let Some(p) = &self.prepared
                {
                    self.critical(StoreJob::Abandon {
                        task: task_id,
                        request: request_id.clone(),
                        run: p.run_id.clone(),
                    });
                }
                self.dispatching = false;
                self.invalidate();
                self.fail("Approval / executable", message, "Inspect the details, correct inputs, and prepare a new request; no automatic retry.");
            }
            runner::Event::Started(record) => {
                if !self.owns(&record) {
                    self.foreign_record(record);
                    return;
                }
                self.dispatching = false;
                self.reserving = false;
                self.live_run = Some(record.prepared.run_id.clone());
                self.live = OutputWindow::default();
                if !self.showing_retained && self.view == View::Output {
                    self.selected_run = self.live_run.clone();
                    self.anchor = 0;
                    self.new_output = 0;
                }
                self.active = Some(record.clone());
                self.prepared = None;
                self.critical(StoreJob::Update(record));
                self.notice = "Running with owned-tree cancellation; filesystem/network effects are not sandboxed.".into();
            }
            runner::Event::CaptureChanged(record) => {
                if self.owns(&record) {
                    self.active = Some(record);
                }
            }
            runner::Event::Cancelling(record) => {
                if !self.owns(&record) {
                    self.foreign_record(record);
                    return;
                }
                self.active = Some(record.clone());
                self.critical(StoreJob::Update(record));
                self.notice = "Cancelling; waiting for owned-tree exit with bounded escalation. Edits are not rolled back.".into();
            }
            runner::Event::Output(output) => {
                if output.task_id == self.task_id
                    && self.live_run.as_deref() == Some(&output.run_id)
                {
                    let added = self.live.append(&output.text);
                    if !self.follow || self.showing_retained || self.view != View::Output {
                        self.new_output = self.new_output.saturating_add(added);
                    }
                    if self.follow && !self.showing_retained {
                        self.anchor = self
                            .live
                            .next
                            .saturating_sub(self.viewport_rows as u64)
                            .max(self.live.first);
                    }
                }
            }
            runner::Event::Finished(record) => {
                if !self.owns(&record) {
                    self.foreign_record(record);
                    return;
                }
                self.dispatching = false;
                self.final_unknown |= record.state == runner::RunState::OutcomeUnknown;
                self.active = Some(record.clone());
                self.final_record = Some(record);
                self.request_repo(RepoPurpose::After);
                self.notice = "Process ended; refreshing after-input identities before acknowledging journal persistence.".into();
            }
        }
    }
    fn owns(&self, record: &runner::RunRecord) -> bool {
        record.prepared.task_id == self.task_id
            && (self.request_id.as_deref() == Some(&record.prepared.request_id)
                || self
                    .active
                    .as_ref()
                    .is_some_and(|r| r.prepared.run_id == record.prepared.run_id))
    }
    fn foreign_record(&mut self, record: runner::RunRecord) {
        // Fail closed on a late owner mismatch: preserve the original task/run correlation,
        // retain the ownership guard, and cancel rather than silently permitting navigation.
        self.invalidate();
        self.dispatching = false;
        self.active = Some(record.clone());
        let owner = record.prepared.task_id;
        let terminal = !matches!(
            record.state,
            runner::RunState::AwaitingDispatch
                | runner::RunState::Running
                | runner::RunState::Cancelling
        );
        if terminal {
            self.final_unknown |= record.state == runner::RunState::OutcomeUnknown;
            self.final_record = Some(record);
            self.request_repo(RepoPurpose::After);
        } else {
            self.cancellation = true;
            self.critical(StoreJob::Update(record));
        }
        self.fail("Ownership", format!("Received execution evidence owned by task {owner}; it remains under that owner and navigation stays guarded."), "Wait for owned cancellation and committed journal evidence; inspect the original task.");
    }
    pub fn take_store_request(&mut self) -> Option<store::RunRequest> {
        if self.inflight.is_some() || self.failed.as_ref().is_some_and(StoreJob::critical) {
            return None;
        }
        let job = self.queue.pop_front()?;
        let request = job.request();
        self.inflight = Some(job);
        Some(request)
    }
    pub fn requeue_store_request(&mut self, _request: store::RunRequest) {
        if let Some(job) = self.inflight.take() {
            self.queue.push_front(job);
        }
    }
    pub fn store_event(
        &mut self,
        task_id: i64,
        request_id: Option<&str>,
        run_id: Option<&str>,
        result: Result<store::RunResponse, String>,
    ) {
        let matches = self
            .inflight
            .as_ref()
            .is_some_and(|job| job.task() == task_id && job.correlation() == (request_id, run_id));
        if !matches {
            if let Ok(store::RunResponse::Reserved(permit)) = result {
                self.critical(StoreJob::Abandon {
                    task: permit.task_id,
                    request: permit.request_id.clone(),
                    run: permit.run_id.clone(),
                });
            }
            return;
        }
        let job = self.inflight.take().expect("correlated journal operation");
        match result {
            Err(message) => {
                if matches!(job, StoreJob::Reserve(..)) {
                    self.reserving = false;
                    self.fail("Storage / reservation", message, "Nothing was dispatched. Reopen v and approve again only after resolving quota/storage.");
                } else {
                    let critical = job.critical();
                    if critical {
                        self.failed = Some(job);
                    }
                    self.fail("Storage / journal", message, if critical { "Use /journal-retry to retry only this journal save. No command is re-executed; cancellation remains available." } else { "Refresh/reopen the selected journal artifact after resolving local storage." });
                }
            }
            Ok(store::RunResponse::Reserved(permit)) => {
                self.reserving = false;
                self.permit = Some(permit); // Only drive may ask the engine to dispatch.
            }
            Ok(store::RunResponse::Updated(record)) => {
                let terminal = !matches!(
                    record.state,
                    runner::RunState::AwaitingDispatch
                        | runner::RunState::Running
                        | runner::RunState::Cancelling
                );
                let run = record.prepared.run_id.clone();
                if task_id == self.task_id {
                    self.upsert(record);
                }
                if terminal && self.final_ack.as_deref() == Some(&run) {
                    self.final_ack = None;
                    self.active = None;
                    self.request_id = None;
                    self.prepared = None;
                    self.journal_saving = false;
                    self.notice =
                        "Execution journal saved. A process result is not task completion.".into();
                }
                if matches!(job, StoreJob::Abandon { .. }) {
                    self.reserving = false;
                    self.dispatching = false;
                }
            }
            Ok(store::RunResponse::Runs(records)) => {
                if task_id == self.task_id {
                    self.history = records;
                    self.history_index = self
                        .selected_run
                        .as_ref()
                        .and_then(|id| self.history.iter().position(|r| &r.prepared.run_id == id))
                        .unwrap_or(0);
                    if self.selected_run.is_none() {
                        self.selected_run = self.history.first().map(|r| r.prepared.run_id.clone());
                    }
                }
            }
            Ok(store::RunResponse::Output(page)) => {
                if task_id == self.task_id && run_id == self.selected_run.as_deref() {
                    self.retained = OutputWindow::default();
                    self.retained.append(&page.text);
                    self.retained_page = Some(page);
                    self.showing_retained = true;
                    self.follow = false;
                    self.anchor = 0;
                    self.horizontal = 0;
                    self.view = View::Output;
                }
            }
            Ok(store::RunResponse::Purged { record, warning }) => {
                if task_id == self.task_id {
                    if self.selected_run.as_deref() == Some(&record.prepared.run_id) {
                        self.retained = OutputWindow::default();
                        self.retained_page = Some(store::OutputPage {
                            offset: 0,
                            text: String::new(),
                            total_bytes: record.captured_bytes,
                            unavailable: true,
                            truncated: record.truncated,
                        });
                        self.showing_retained = true;
                    }
                    self.upsert(record);
                    self.notice = "Only this run's owned local output was purged; metadata remains explicitly unavailable. Not secure erasure.".into();
                    if let Some(warning) = warning {
                        self.notice = "Output reference is purged/unavailable; physical cleanup has a warning. Use /artifacts for full details.".into();
                        self.fail("Storage / purge cleanup", warning, "The unavailable metadata committed. Inspect the cleanup warning; do not treat this as a no-op or a verified result.");
                    }
                }
            }
        }
        self.journal_saving = self.final_ack.is_some()
            || self.queue.iter().any(StoreJob::critical)
            || self.failed.as_ref().is_some_and(StoreJob::critical);
        self.maybe_navigate();
    }
    fn upsert(&mut self, record: runner::RunRecord) {
        if let Some(existing) = self
            .history
            .iter_mut()
            .find(|r| r.prepared.run_id == record.prepared.run_id)
        {
            *existing = record;
        } else {
            self.history.insert(0, record);
            self.history.truncate(256);
        }
    }
    fn optional(&mut self, job: StoreJob) -> bool {
        if job.task() <= 0 {
            return false;
        }
        if self.queue.len() >= STORE_QUEUE - 4 {
            self.notice = "Journal queue is busy; the requested read/purge was not queued. Retry deliberately after it drains.".into();
            return false;
        }
        if self.inflight.as_ref().is_some_and(|other| other == &job) {
            return false;
        }
        if self.queue.iter().any(|other| other == &job) {
            return false;
        }
        self.queue.push_back(job);
        true
    }
    fn critical(&mut self, job: StoreJob) {
        // At most Reserve, Started, Cancelling, Finished/Abandon for the single owned run.
        // Reserve four slots independently from optional history/output reads.
        if matches!(job, StoreJob::Update(_) | StoreJob::Abandon { .. }) {
            self.journal_saving = true;
        }
        self.queue.push_back(job);
    }
    pub fn repo_requested(&self) -> bool {
        self.repo_request && !self.repo_waiting
    }
    pub fn repo_started(&mut self) {
        self.repo_waiting = true;
        self.repo_request = false;
        self.observed_generation = self.repo_generation;
    }
    fn request_repo(&mut self, purpose: RepoPurpose) {
        self.repo_generation = self.repo_generation.wrapping_add(1);
        self.repo_purpose = purpose;
        self.repo_request = true;
    }
    pub fn repo_observed(&mut self, report: Option<&repo::Report>) {
        if !self.repo_waiting {
            return;
        }
        self.repo_waiting = false;
        if self.observed_generation != self.repo_generation {
            return;
        }
        self.latest = report.map(fingerprint).unwrap_or_default();
        match self.repo_purpose {
            RepoPurpose::Before => {
                if self.pending_spec.is_some() {
                    self.before_ready = Some(self.latest.clone());
                }
            }
            RepoPurpose::After => {
                if let Some(mut record) = self.final_record.take() {
                    record.after = if record.prepared.task_id == self.task_id {
                        self.latest.clone()
                    } else {
                        runner::InputSnapshot::default()
                    };
                    self.final_ack = Some(record.prepared.run_id.clone());
                    self.active = Some(record.clone());
                    self.critical(StoreJob::Update(record));
                }
            }
            RepoPurpose::Inspect => {}
        }
        if report.is_none() {
            self.notice = "Source observation unavailable; execution evidence cannot establish current source validity.".into();
        }
    }
    pub fn request_navigation(&mut self, action: Action) -> Option<Action> {
        self.confirmation_visible = false;
        if self.confirmation.as_ref().is_some_and(|c| {
            matches!(
                c,
                Confirmation::Navigate(_)
                    | Confirmation::UnknownExit(_)
                    | Confirmation::UnsavedJournalExit(_)
            )
        }) {
            return None;
        }
        if !self.busy() {
            if !self.final_unknown {
                return Some(action);
            }
            self.confirmation = Some(Confirmation::UnknownExit(action));
            self.affirmative = false;
            self.shown = true;
            return None;
        }
        let finished = self.active.as_ref().is_some_and(|r| {
            !matches!(
                r.state,
                runner::RunState::Running
                    | runner::RunState::Cancelling
                    | runner::RunState::AwaitingDispatch
            )
        });
        self.confirmation = Some(
            if finished && self.failed.is_some() && action == Action::Quit {
                Confirmation::UnsavedJournalExit(action)
            } else {
                Confirmation::Navigate(action)
            },
        );
        self.affirmative = false;
        self.shown = true;
        None
    }
    fn maybe_navigate(&mut self) {
        if self.deferred.is_some() && !self.busy() {
            let action = self.deferred.take().expect("deferred navigation");
            if self.final_unknown {
                self.confirmation = Some(Confirmation::UnknownExit(action));
                self.confirmation_visible = false;
                self.affirmative = false;
                self.shown = true;
            } else {
                self.ready = Some(action);
            }
        }
    }
    fn cancel_or_deny(&mut self) {
        if self.active.as_ref().is_some_and(|r| {
            matches!(
                r.state,
                runner::RunState::Running | runner::RunState::Cancelling
            )
        }) || self.dispatching
        {
            self.cancellation = true;
        } else if self.active.is_none() {
            self.invalidate();
        }
    }
    pub fn command_choices(&self) -> Vec<super::commands::Choice> {
        use super::commands::Choice;
        let reviewing = self.confirmation.is_some();
        let editing = self.searching;
        let reason = if editing {
            Some("Finish or cancel the literal search first.")
        } else if reviewing {
            Some("Finish or cancel the current review first.")
        } else {
            None
        };
        let choice = |name, label, blocked: Option<&str>| {
            if let Some(reason) = blocked {
                Choice::disabled(name, label, reason)
            } else {
                Choice::new(name, label)
            }
        };
        let approval = matches!(self.confirmation, Some(Confirmation::Approval));
        vec![
            choice(
                "command-edit",
                "Edit explicit local command",
                if approval {
                    None
                } else {
                    reason.or_else(|| {
                        (self.active.is_some() || self.dispatching || self.final_ack.is_some())
                            .then_some(
                                "Cancel and wait before editing an already-dispatched command.",
                            )
                    })
                },
            ),
            choice(
                "run",
                "Review prepared command for one-shot execution",
                reason.or_else(|| {
                    (self.prepared.is_none() || self.reserving || self.dispatching)
                        .then_some("Prepare an explicit command first.")
                }),
            ),
            choice(
                "deny",
                "Deny the prepared request",
                (!approval).then_some("No approval is open."),
            ),
            choice(
                "cancel",
                "Review cancellation of owned work",
                reason.or_else(|| (!self.busy()).then_some("No owned work is pending.")),
            ),
            choice("history", "Inspect retained run history", reason),
            choice(
                "open",
                "Open selected retained run",
                reason.or_else(|| {
                    (self.view != View::History || self.history.is_empty())
                        .then_some("Select a retained run in history first.")
                }),
            ),
            choice("refresh", "Refresh repository and run history", reason),
            choice("artifacts", "Inspect command and artifact details", reason),
            choice(
                "journal-retry",
                "Retry journal persistence only",
                editing
                    .then_some("Finish or cancel the literal search first.")
                    .or_else(|| {
                        self.failed
                            .is_none()
                            .then_some("No failed journal operation to retry.")
                    }),
            ),
            choice(
                "purge",
                "Review purging selected retained output",
                reason.or_else(|| {
                    (self.busy() || self.selected_run.is_none())
                        .then_some("Select a retained run and wait for owned work to finish.")
                }),
            ),
            choice("search", "Search output literally", reason),
            choice(
                "search-next",
                "Find next output match",
                reason.or_else(|| {
                    self.search
                        .is_empty()
                        .then_some("Enter an output search first.")
                }),
            ),
            choice(
                "search-previous",
                "Find previous output match",
                reason.or_else(|| {
                    self.search
                        .is_empty()
                        .then_some("Enter an output search first.")
                }),
            ),
            choice("follow", "Toggle following live output", reason),
            choice(
                "page-next",
                "Next retained output page",
                reason
                    .or_else(|| (!self.showing_retained).then_some("Open retained output first.")),
            ),
            choice(
                "page-previous",
                "Previous retained output page",
                reason
                    .or_else(|| (!self.showing_retained).then_some("Open retained output first.")),
            ),
            Choice::new("quit", "Review exit"),
        ]
    }

    pub fn command(&mut self, name: &str) -> Option<Action> {
        self.confirmation_visible = false;
        if name == "quit" {
            return self.request_navigation(Action::Quit);
        }
        if self.searching {
            return None;
        }
        if self.confirmation.is_some() && !matches!(name, "deny" | "command-edit" | "journal-retry")
        {
            return None;
        }
        match name {
            "deny" if matches!(self.confirmation, Some(Confirmation::Approval)) => {
                self.invalidate();
                self.notice = "Request denied. Nothing executed; command draft retained.".into();
            }
            "command-edit" => {
                if matches!(self.confirmation, Some(Confirmation::Approval))
                    || (self.confirmation.is_none()
                        && self.active.is_none()
                        && !self.dispatching
                        && self.final_ack.is_none())
                {
                    self.invalidate();
                    return Some(Action::EditCommand);
                }
                if self.confirmation.is_none() {
                    self.fail("Command", "Cannot edit an already-dispatched command.",
                        "Cancel and wait before editing; the existing command draft remains available.");
                }
            }
            "run" => {
                if self.prepared.is_some() && !self.reserving && !self.dispatching {
                    self.confirmation = Some(Confirmation::Approval);
                    self.affirmative = false;
                    self.detail_scroll = 0;
                } else {
                    self.notice = "No approvable prepared request. Use /command-edit to edit an explicit JSON command.".into();
                }
            }
            "cancel" if self.busy() => {
                self.confirmation = Some(Confirmation::Cancel);
                self.affirmative = false;
                self.detail_scroll = 0;
            }
            "history" => {
                self.view = View::History;
                self.optional(StoreJob::Load(self.task_id));
            }
            "open" if self.view == View::History => self.open_history(),
            "refresh" => {
                if self.pending_spec.is_none() && self.final_record.is_none() {
                    self.request_repo(RepoPurpose::Inspect);
                }
                self.optional(StoreJob::Load(self.task_id));
                self.notice =
                    "Requested fresh source/history observation; no command is rerun.".into();
            }
            "artifacts" => {
                self.view = View::Details;
                self.detail_scroll = 0;
            }
            "journal-retry" => self.retry_journal(),
            "purge" if !self.busy() => {
                if let Some(id) = &self.selected_run {
                    self.confirmation = Some(Confirmation::Purge(id.clone()));
                    self.affirmative = false;
                    self.detail_scroll = 0;
                }
            }
            "search" => {
                self.searching = true;
                self.follow = false;
            }
            "search-next" => self.find(false),
            "search-previous" => self.find(true),
            "follow" => {
                if self.showing_retained && self.live_run.is_some() {
                    self.showing_retained = false;
                    self.selected_run = self.live_run.clone();
                }
                self.follow = !self.follow;
                if self.follow {
                    self.jump_end();
                    self.new_output = 0;
                }
            }
            "page-next" if self.showing_retained => self.next_page(),
            "page-previous" if self.showing_retained => self.previous_page(),
            _ => {}
        }
        self.ready.take()
    }

    pub fn key(&mut self, mut key: KeyEvent) -> Option<Action> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('c')
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            return self.request_navigation(Action::Quit);
        }
        if self.confirmation.is_some() {
            if key.kind != KeyEventKind::Press {
                return None;
            }
            if key.modifiers.is_empty()
                && self.confirmation_visible
                && matches!(key.code, KeyCode::Char('y' | 'n'))
            {
                self.affirmative = key.code == KeyCode::Char('y');
                key.code = KeyCode::Enter;
            }
            match key.code {
                KeyCode::Left => self.affirmative = false,
                KeyCode::Right => self.affirmative = true,
                KeyCode::Tab => self.affirmative = !self.affirmative,
                KeyCode::Esc => {
                    if matches!(
                        self.confirmation,
                        Some(
                            Confirmation::Navigate(_)
                                | Confirmation::UnknownExit(_)
                                | Confirmation::UnsavedJournalExit(_)
                        )
                    ) {
                        self.deferred = None;
                        self.ready = None;
                    }
                    self.confirmation = None;
                    self.confirmation_visible = false;
                    self.affirmative = false;
                }
                KeyCode::Enter => {
                    self.confirmation_visible = false;
                    let confirmation = self.confirmation.take().expect("open confirmation");
                    if self.affirmative {
                        self.confirm(confirmation);
                    } else if matches!(
                        confirmation,
                        Confirmation::Navigate(_)
                            | Confirmation::UnknownExit(_)
                            | Confirmation::UnsavedJournalExit(_)
                    ) {
                        self.deferred = None;
                        self.ready = None;
                    }
                    self.affirmative = false;
                }
                KeyCode::Down => {
                    self.detail_scroll = self.detail_scroll.saturating_add(1).min(self.detail_max)
                }
                KeyCode::Up => self.detail_scroll = self.detail_scroll.saturating_sub(1),
                KeyCode::PageDown => {
                    self.detail_scroll = self
                        .detail_scroll
                        .saturating_add(self.viewport_rows as u16)
                        .min(self.detail_max)
                }
                KeyCode::PageUp => {
                    self.detail_scroll =
                        self.detail_scroll.saturating_sub(self.viewport_rows as u16)
                }
                _ => {}
            }
            return self.ready.take();
        }
        if self.searching {
            match key.code {
                KeyCode::Esc => self.searching = false,
                KeyCode::Enter => {
                    self.searching = false;
                    self.find(false);
                }
                KeyCode::Backspace => {
                    if let Some((index, _)) = self.search.grapheme_indices(true).next_back() {
                        self.search.truncate(index);
                    }
                }
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && self.search.len() + ch.len_utf8() <= 512 =>
                {
                    self.search.push(ch)
                }
                _ => {}
            }
            return None;
        }
        match key.code {
            KeyCode::Esc => {
                if self.view != View::Output {
                    self.view = View::Output;
                } else {
                    self.shown = false;
                }
            }
            KeyCode::Enter if self.view == View::History => self.open_history(),
            KeyCode::Up if self.view == View::History => {
                self.history_index = self.history_index.saturating_sub(1);
                self.select_history();
            }
            KeyCode::Down if self.view == View::History => {
                self.history_index =
                    (self.history_index + 1).min(self.history.len().saturating_sub(1));
                self.select_history();
            }
            KeyCode::Up if self.view == View::Details => {
                self.detail_scroll = self.detail_scroll.saturating_sub(1)
            }
            KeyCode::Down if self.view == View::Details => {
                self.detail_scroll = self.detail_scroll.saturating_add(1).min(self.detail_max)
            }
            KeyCode::PageUp if self.view == View::Details => {
                self.detail_scroll = self.detail_scroll.saturating_sub(self.viewport_rows as u16)
            }
            KeyCode::PageDown if self.view == View::Details => {
                self.detail_scroll = self
                    .detail_scroll
                    .saturating_add(self.viewport_rows as u16)
                    .min(self.detail_max)
            }
            KeyCode::Home if self.view == View::Details => self.detail_scroll = 0,
            KeyCode::End if self.view == View::Details => self.detail_scroll = self.detail_max,
            KeyCode::Up => self.scroll(-1),
            KeyCode::Down => self.scroll(1),
            KeyCode::PageUp => self.scroll(-(self.viewport_rows as i64)),
            KeyCode::PageDown => self.scroll(self.viewport_rows as i64),
            KeyCode::Home => {
                self.follow = false;
                self.anchor = self.window().first;
                self.horizontal = 0;
            }
            KeyCode::End => {
                self.follow = false;
                self.jump_end();
            }
            KeyCode::Left => self.horizontal = self.horizontal.saturating_sub(8),
            KeyCode::Right => self.horizontal = self.horizontal.saturating_add(8),
            _ => {}
        }
        None
    }
    fn confirm(&mut self, confirmation: Confirmation) {
        match confirmation {
            Confirmation::Approval => {
                if let Some(p) = &self.prepared
                    && self.request_id.as_deref() == Some(&p.request_id)
                    && p.task_id == self.task_id
                    && !self.reserving
                    && !self.dispatching
                {
                    self.reserving = true;
                    self.critical(StoreJob::Reserve(
                        p.clone(),
                        self.capture_capacity.clamp(1024, 8 * 1024 * 1024),
                    ));
                    self.notice = "Requesting a committed private output reservation; executable has not started.".into();
                }
            }
            Confirmation::Cancel => self.cancel_or_deny(),
            Confirmation::Purge(run) => {
                self.optional(StoreJob::Purge {
                    task: self.task_id,
                    run,
                });
            }
            Confirmation::Navigate(action) => {
                self.deferred = Some(action);
                self.cancel_or_deny();
                self.maybe_navigate();
            }
            Confirmation::UnknownExit(action) => {
                if action == Action::Quit {
                    self.ready = Some(action);
                } else {
                    self.fail("Ownership", "External effects remain unknown; task switching cannot establish cancellation.", "Stay to inspect the journal, or request Quit and explicitly acknowledge the unknown outcome.");
                }
            }
            Confirmation::UnsavedJournalExit(action) => {
                if action == Action::Quit {
                    self.ready = Some(action);
                }
            }
        }
    }
    fn retry_journal(&mut self) {
        if let Some(job) = self.failed.take() {
            self.queue.push_front(job);
            self.error = None;
            self.notice =
                "Retrying only journal persistence; the executable is not replayed.".into();
        }
    }
    fn select_history(&mut self) {
        self.selected_run = self
            .history
            .get(self.history_index)
            .map(|r| r.prepared.run_id.clone());
    }
    fn open_history(&mut self) {
        self.select_history();
        self.page_offsets.clear();
        if let Some(run) = &self.selected_run {
            self.optional(StoreJob::Read {
                task: self.task_id,
                run: run.clone(),
                offset: 0,
            });
        }
    }
    fn next_page(&mut self) {
        if let (Some(page), Some(run)) = (&self.retained_page, &self.selected_run) {
            let previous = page.offset;
            let offset = previous + page.text.len() as u64;
            if offset < page.total_bytes && !page.unavailable {
                let run = run.clone();
                if self.optional(StoreJob::Read {
                    task: self.task_id,
                    run,
                    offset,
                }) && self.page_offsets.len() < 128
                {
                    self.page_offsets.push(previous);
                }
            }
        }
    }
    fn previous_page(&mut self) {
        if let Some(offset) = self.page_offsets.last().copied()
            && let Some(run) = &self.selected_run
        {
            let run = run.clone();
            if self.optional(StoreJob::Read {
                task: self.task_id,
                run,
                offset,
            }) {
                self.page_offsets.pop();
            }
        }
    }
    pub fn window(&self) -> &OutputWindow {
        if self.showing_retained {
            &self.retained
        } else {
            &self.live
        }
    }
    pub fn selected_record(&self) -> Option<&runner::RunRecord> {
        self.active
            .as_ref()
            .filter(|r| self.selected_run.as_deref() == Some(&r.prepared.run_id))
            .or_else(|| {
                self.history
                    .iter()
                    .find(|r| self.selected_run.as_deref() == Some(&r.prepared.run_id))
            })
    }
    pub fn set_viewport(&mut self, rows: usize) {
        self.viewport_rows = rows.max(1);
        if self.follow && !self.showing_retained {
            self.jump_end();
        } else {
            self.anchor = self.anchor.max(self.window().first);
        }
    }
    fn jump_end(&mut self) {
        self.anchor = self
            .window()
            .next
            .saturating_sub(self.viewport_rows as u64)
            .max(self.window().first);
    }
    fn scroll(&mut self, lines: i64) {
        self.follow = false;
        self.anchor = self.anchor.saturating_add_signed(lines).clamp(
            self.window().first,
            self.window()
                .next
                .saturating_sub(1)
                .max(self.window().first),
        );
    }
    fn find(&mut self, backwards: bool) {
        if self.search.is_empty() {
            return;
        }
        self.follow = false;
        let lines = &self.window().lines;
        let found = if backwards {
            lines
                .iter()
                .rev()
                .find(|line| line.id < self.anchor && line.text.contains(&self.search))
                .or_else(|| {
                    lines
                        .iter()
                        .rev()
                        .find(|line| line.text.contains(&self.search))
                })
        } else {
            lines
                .iter()
                .find(|line| line.id > self.anchor && line.text.contains(&self.search))
                .or_else(|| lines.iter().find(|line| line.text.contains(&self.search)))
        };
        if let Some(line) = found {
            self.anchor = line.id;
            self.horizontal = 0;
        } else {
            self.notice = "No match in this bounded live window/current retained page; use [ ] to inspect other retained pages.".into();
        }
    }
    fn fail(&mut self, layer: &'static str, message: impl Into<String>, remedy: &'static str) {
        self.error = Some(LayerError {
            layer,
            message: message.into(),
            remedy,
        });
    }
    pub fn status(&self) -> String {
        if let Some(record) = &self.active {
            return format!(
                "{:?} · {}",
                record.state,
                if self.final_ack.is_some() {
                    "journal saving"
                } else if self.final_record.is_some() {
                    "after-source observation pending"
                } else {
                    "owned local command"
                }
            );
        }
        if self.failed.is_some() {
            return "Journal save failed; retained in memory (s retries storage only)".into();
        }
        if self.dispatching {
            return "Approved reservation; dispatch revalidation pending".into();
        }
        if self.reserving {
            return "Reserving capture before dispatch".into();
        }
        if self.prepared.is_some() {
            return "Approval pending (/run reviews; no automatic execution)".into();
        }
        if self.request_id.is_some() || self.pending_spec.is_some() {
            return "Preparing command/source identities; not executed".into();
        }
        "Local command evidence; no command running".into()
    }
    pub fn details(&self) -> String {
        let mut text = String::new();
        if let Some(error) = &self.error {
            text.push_str(&format!(
                "ERROR LAYER: {}\n{}\nRemedy: {}\n\n",
                error.layer,
                safe_text(&error.message),
                error.remedy
            ));
        }
        if let Some(p) = &self.prepared {
            text.push_str(&approval_details(p, self.capture_capacity));
        }
        if let Some(record) = self.selected_record() {
            text.push_str(&format!("\nRun {}\nRequest {}\nTask {}\nState {:?}; tree {:?}\nStarted {:?} ms; ended {:?} ms\nExit {:?}; signal {:?}; cancellation requested {}\n{}\nCaptured {} bytes; discarded {} bytes; live queue skipped {} bytes\nTruncated {}; output unavailable {}\nBefore {:?}\nAfter {:?}\nLatest {:?}\n", record.prepared.run_id, record.prepared.request_id, record.prepared.task_id,
                record.state, record.tree, record.started_at_ms, record.ended_at_ms, record.exit_code, record.signal, record.cancelled,
                evidence_label(record, &self.latest), record.captured_bytes, record.discarded_bytes, record.dropped_live_bytes, record.truncated, record.output_unavailable,
                record.prepared.before, record.after, self.latest));
            text.push_str(&approval_details(&record.prepared, self.capture_capacity));
            if let Some(error) = &record.error {
                text.push_str(&format!("\nExecution details: {}\n", safe_text(error)));
            }
        }
        if text.is_empty() {
            text.push_str("No execution evidence yet. /command-edit edits a JSON argv array or explicit shell object. Preparing and inspecting never execute.\n");
        }
        text.push_str("\nSource observations exclude ignored untracked files and submodule contents. Matching hashes are observations, not an atomic snapshot or a live watch. A successful command never completes the task automatically.\n");
        text
    }
}

pub fn parse_command(text: &str) -> Result<runner::CommandSpec, String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Shell {
        shell: String,
        dialect: String,
        text: String,
    }
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Input {
        Direct(Vec<String>),
        Shell(Shell),
    }
    if text.len() > 64 * 1024 {
        return Err("Command exceeds 64 KiB".into());
    }
    let input: Input = serde_json::from_str(text).map_err(|error| {
        format!("Command JSON (string argv array or exactly shell/dialect/text): {error}")
    })?;
    match input {
        Input::Direct(strings) => {
            if strings.is_empty() || strings.len() > 4097 {
                return Err(
                    "Direct argv requires a program and at most 4096 literal arguments".into(),
                );
            }
            if strings[0].is_empty() || strings.iter().any(|s| s.contains('\0')) {
                return Err("Program must be nonempty; NUL is not allowed".into());
            }
            let mut strings = strings.into_iter();
            let program = strings.next().expect("nonempty argv");
            Ok(runner::CommandSpec::Direct {
                program,
                args: strings.collect(),
            })
        }
        Input::Shell(shell) => {
            let dialect = match shell.dialect.as_str() {
                "posix" => runner::ShellDialect::Posix,
                "cmd" => runner::ShellDialect::Cmd,
                "powershell" => runner::ShellDialect::PowerShell,
                _ => return Err("dialect must be posix, cmd, or powershell".into()),
            };
            if shell.shell.is_empty() || shell.shell.contains('\0') || shell.text.contains('\0') {
                return Err("Shell executable must be nonempty; NUL is not allowed".into());
            }
            Ok(runner::CommandSpec::Shell {
                executable: shell.shell,
                dialect,
                text: shell.text,
            })
        }
    }
}
pub fn safe_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\n' || ch == '\t' {
            result.push(ch);
        } else if ch.is_control() || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            result.extend(ch.escape_unicode());
        } else {
            result.push(ch);
        }
    }
    result
}
pub fn approval_details(p: &runner::Prepared, capacity: u64) -> String {
    let mut text = format!(
        "Task {}\nRequest {}\nRun {}\nCanonical cwd: {}\nCanonical executable: {}\nExecutable SHA-256: {}\nExecutable identity: {} · {} bytes\nPrepared {} ms; approval expires {} ms\n",
        p.task_id,
        p.request_id,
        p.run_id,
        serde_json::to_string(&p.cwd).unwrap_or_else(|_| "[unavailable path]".into()),
        serde_json::to_string(&p.executable.path).unwrap_or_else(|_| "[unavailable path]".into()),
        p.executable.sha256,
        p.executable.file_id,
        p.executable.bytes,
        p.prepared_at_ms,
        p.expires_at_ms
    );
    match &p.spec {
        runner::CommandSpec::Direct { program, args } => {
            text.push_str("Route: direct argv; no implicit shell\n");
            text.push_str(&format!("program = {}\n", serde_json::to_string(program).unwrap()));
            for (index, arg) in args.iter().enumerate() { text.push_str(&format!("argv[{index}] = {}\n", serde_json::to_string(arg).unwrap())); }
        }
        runner::CommandSpec::Shell { executable, dialect, text: shell } => text.push_str(&format!("Route: explicit shell {:?}\nShell executable = {}\nExact shell text (JSON escaped) = {}\n", dialect, serde_json::to_string(executable).unwrap(), serde_json::to_string(shell).unwrap())),
    }
    text.push_str(&format!("\nPermission duration: ONE command; no reusable grant or automatic retry. Runtime duration: until exit or deliberate cancellation; no promised completion time.\nScope: arbitrary filesystem/network effects UNKNOWN. Not read-only, not sandboxed.\nEnforcement: Eevee dispatch checks + {} cancellation ownership before user execution. Escaped POSIX sessions/external effects are not contained.\nNull stdin; no interactive-shell inheritance.\nPrivate output capacity: {} bytes, committed under local quota BEFORE dispatch. Sanitized/redacted best-effort capture; full capture/queues drain and discard with explicit partial-evidence notices.\nBefore source identity: {:?}\n", if cfg!(windows) { "Windows kill-on-close Job" } else { "POSIX process group" }, capacity.clamp(1024, 8 * 1024 * 1024), p.before));
    text.push_str("Storage additionally reserves 256 KiB of bounded journal/metadata headroom; shared quota may refuse the request before any execution.\n");
    text
}
pub fn evidence_label(record: &runner::RunRecord, latest: &runner::InputSnapshot) -> String {
    let mut labels = Vec::new();
    match record.state {
        runner::RunState::Interrupted => {
            labels.push("Interrupted; prior process ownership is not restored")
        }
        runner::RunState::OutcomeUnknown => labels.push("Outcome unknown; effects may remain"),
        runner::RunState::Failed => labels.push("Failed command"),
        runner::RunState::Cancelled => labels.push("Cancelled; edits not rolled back"),
        runner::RunState::AwaitingDispatch
        | runner::RunState::Running
        | runner::RunState::Cancelling => labels.push("Incomplete execution"),
        runner::RunState::Succeeded => {}
    }
    if record.output_unavailable {
        labels.push("Output unavailable; cannot substantiate a verified result");
    }
    if record.truncated {
        labels.push("Truncated/partial evidence");
    }
    let valid = |s: &runner::InputSnapshot| s.complete && s.stable && s.fingerprint.is_some();
    if !valid(&record.prepared.before) || !valid(&record.after) {
        labels.push("Source coverage unknown/incomplete");
    } else if record.prepared.before.fingerprint != record.after.fingerprint {
        labels.push("Ambiguous: inputs changed between before/after observations");
    } else if !valid(latest) {
        labels.push("Current source state unavailable");
    } else if record.after.fingerprint != latest.fingerprint {
        labels.push("Stale: inputs changed after this run");
    } else if labels.is_empty() {
        labels.push(
            "Before/after match last source observation; not a live watch or task verification",
        );
    }
    labels.join(" · ")
}
pub fn fingerprint(report: &repo::Report) -> runner::InputSnapshot {
    fn part(hash: &mut Sha256, bytes: &[u8]) {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    fn path(hash: &mut Sha256, path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            part(hash, path.as_os_str().as_bytes());
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            hash.update((path.as_os_str().encode_wide().count() as u64).to_le_bytes());
            for unit in path.as_os_str().encode_wide() {
                hash.update(unit.to_le_bytes());
            }
        }
    }
    let snapshot = &report.current;
    let mut hash = Sha256::new();
    part(&mut hash, b"eevee-source-v1");
    path(&mut hash, &report.repository.root);
    path(&mut hash, &report.repository.git_dir);
    part(&mut hash, snapshot.head.as_deref().unwrap_or("").as_bytes());
    let mut complete = snapshot.complete;
    for (name, file) in &snapshot.files {
        path(&mut hash, name);
        if name.to_str().is_none() {
            complete = false;
        }
        hash.update([u8::from(file.tracked)]);
        if let Some(head) = &file.head {
            hash.update([1]);
            part(&mut hash, head.oid.as_bytes());
            part(&mut hash, head.mode.as_bytes());
        } else {
            hash.update([0]);
        }
        hash.update((file.index.len() as u64).to_le_bytes());
        for index in &file.index {
            hash.update([index.stage]);
            part(&mut hash, index.object.oid.as_bytes());
            part(&mut hash, index.object.mode.as_bytes());
        }
        match &file.state {
            repo::FileState::Regular {
                sha256,
                bytes,
                binary,
                executable,
            } => {
                hash.update([1, u8::from(*binary), u8::from(*executable)]);
                hash.update(bytes.to_le_bytes());
                hash.update(sha256);
            }
            repo::FileState::Symlink { target, sha256 } => {
                hash.update([2]);
                path(&mut hash, target);
                hash.update(sha256);
            }
            repo::FileState::Missing => hash.update([3]),
            repo::FileState::Unavailable { .. } => {
                hash.update([4]);
                complete = false;
            }
        }
    }
    runner::InputSnapshot {
        fingerprint: Some(format!("{:x}", hash.finalize())),
        complete,
        stable: snapshot.stable,
    }
}

#[cfg(test)]
mod tests;
