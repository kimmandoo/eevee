//! Explicitly trusted Codex 0.152.1 planning. JSONL transport is owned, bounded and never retried.
//! Provider sandbox policy is not an Eevee sandbox; no provider credentials/configuration are written.
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Value, json};

const FRAME_LIMIT: usize = 2 * 1024 * 1024;
const STREAM_LIMIT: usize = 64 * 1024 * 1024;
const ITEM_LIMIT: usize = 32 * 1024;
const ITEMS_LIMIT: usize = 128;
const OUTPUT_LIMIT: usize = 512 * 1024;
const EVENTS_LIMIT: usize = 256;
const REQUESTS_LIMIT: usize = 256;
const EVENT_RESERVE: usize = 16;
const RPC_TIMEOUT: Duration = Duration::from_secs(20);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug)]
pub struct Input {
    pub task: crate::store::Snapshot,
    pub excerpts: Vec<crate::context::Excerpt>,
    pub report: Option<Arc<crate::repo::Report>>,
    pub resume: Option<String>,
}
#[derive(Debug)]
pub enum Request {
    Preview(Box<Input>),
    Run {
        generation: u64,
    },
    Answer {
        generation: u64,
        request_id: Value,
        answer: Value,
    },
    Cancel {
        generation: u64,
    },
}
#[derive(Clone, Debug)]
pub struct Preview {
    pub generation: u64,
    pub task_id: i64,
    pub thread_id: Option<String>,
    pub trust: Vec<String>,
    pub payload: String,
    pub blockers: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Message,
    Plan,
    Tool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Failed,
    Cancelled,
    Unknown,
}
#[derive(Clone, Debug)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}
#[derive(Clone, Debug)]
pub struct Question {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub other: bool,
    pub secret: bool,
}
#[derive(Clone, Debug)]
pub enum PromptKind {
    Questions(Vec<Question>),
    Approval {
        command: String,
        cwd: String,
        reason: String,
        allow_once: bool,
        deny_decision: String,
    },
}
#[derive(Clone, Debug)]
pub struct Prompt {
    pub request_id: Value,
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub kind: PromptKind,
}
#[derive(Clone, Debug)]
pub enum Event {
    Preview(Preview),
    Status {
        generation: u64,
        task_id: i64,
        message: String,
    },
    Thread {
        generation: u64,
        task_id: i64,
        thread_id: String,
        model: String,
        account: String,
    },
    Item {
        generation: u64,
        task_id: i64,
        thread_id: String,
        turn_id: String,
        item_id: String,
        kind: ItemKind,
        text: String,
        complete: bool,
    },
    Prompt {
        generation: u64,
        task_id: i64,
        request: Prompt,
    },
    PromptClosed {
        generation: u64,
        request_id: Value,
    },
    Finished {
        generation: u64,
        task_id: i64,
        outcome: Outcome,
        message: String,
    },
    Error {
        generation: Option<u64>,
        message: String,
    },
}

type Events = Arc<Mutex<VecDeque<Event>>>;
pub struct Worker {
    requests: Option<SyncSender<Request>>,
    events: Events,
    stopped: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    cancelled: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn start(
        project: PathBuf,
        data_dir: PathBuf,
        exclusions: Vec<PathBuf>,
    ) -> io::Result<Self> {
        let (requests, incoming) = mpsc::sync_channel(8);
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicU64::new(0));
        let output = Arc::clone(&events);
        let stop = Arc::clone(&stopped);
        let active = Arc::clone(&busy);
        let cancel = Arc::clone(&cancelled);
        let thread = thread::Builder::new()
            .name("ev-codex".into())
            .spawn(move || {
                worker(
                    Workspace {
                        project,
                        data_dir,
                        exclusions,
                    },
                    incoming,
                    output,
                    stop,
                    active,
                    cancel,
                );
            })?;
        Ok(Self {
            requests: Some(requests),
            events,
            stopped,
            busy,
            cancelled,
            thread: Some(thread),
        })
    }
    pub fn try_request(&self, pending: &mut Option<Request>) -> bool {
        let Some(request) = pending.take() else {
            return false;
        };
        let Some(sender) = &self.requests else {
            *pending = Some(request);
            return false;
        };
        // Backpressure admission before the event queue fills, retaining room for queued
        // command refusals and terminal events. Cancellation is never gated on UI drainage.
        if !matches!(request, Request::Cancel { .. })
            && self.events.lock().len() >= EVENTS_LIMIT - EVENT_RESERVE - 8
        {
            *pending = Some(request);
            return false;
        }
        if let Request::Cancel { generation } = &request {
            // Invalidate immediately, even while version/config validation owns the worker.
            self.cancelled.store(*generation, Ordering::Release);
        }
        let marks_busy = matches!(request, Request::Preview(_) | Request::Run { .. });
        let previous = if marks_busy {
            self.busy.swap(true, Ordering::AcqRel)
        } else {
            false
        };
        match sender.try_send(request) {
            Ok(()) => true,
            Err(TrySendError::Full(request) | TrySendError::Disconnected(request)) => {
                if marks_busy && !previous {
                    self.busy.store(false, Ordering::Release);
                }
                *pending = Some(request);
                false
            }
        }
    }
    pub fn try_event(&self) -> Option<Event> {
        self.events.lock().pop_front()
    }
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Prepared {
    generation: u64,
    input: Box<Input>,
    trust: crate::agent::PlanningTrust,
    text: String,
}
fn emit(events: &Events, event: Event) -> Result<(), String> {
    let mut queue = events.lock();
    if let Event::Item {
        generation,
        item_id,
        ..
    } = &event
        && let Some(previous) = queue.iter_mut().find(|entry| {
            matches!(entry,
            Event::Item { generation: g, item_id: id, .. } if g == generation && id == item_id)
        })
    {
        *previous = event;
        return Ok(());
    }
    let limit = if matches!(event, Event::Item { .. } | Event::Status { .. }) {
        EVENTS_LIMIT - EVENT_RESERVE
    } else {
        EVENTS_LIMIT - 1
    };
    if queue.len() >= limit {
        return Err(
            "UI event queue exhausted; transport stopped without dropping a user request".into(),
        );
    }
    queue.push_back(event);
    Ok(())
}
fn finish(events: &Events, generation: u64, task_id: i64, outcome: Outcome, message: String) {
    // Reserve terminal capacity even when an unconsumed stream filled the ordinary queue.
    let mut queue = events.lock();
    if queue.len() < EVENTS_LIMIT {
        queue.push_back(Event::Finished {
            generation,
            task_id,
            outcome,
            message: safe(&message),
        });
    }
}
struct Workspace {
    project: PathBuf,
    data_dir: PathBuf,
    exclusions: Vec<PathBuf>,
}

fn worker(
    workspace: Workspace,
    incoming: Receiver<Request>,
    events: Events,
    stop: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    cancelled: Arc<AtomicU64>,
) {
    let Workspace {
        project,
        data_dir,
        exclusions,
    } = &workspace;
    let mut prepared: Option<Prepared> = None;
    let mut generation = 0u64;
    let mut preview_owner = None;
    let mut threads: HashMap<String, i64> = HashMap::new();
    while !stop.load(Ordering::Acquire) {
        let request = match incoming.recv_timeout(Duration::from_millis(20)) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match request {
            Request::Preview(input) => {
                prepared = None;
                generation = generation.saturating_add(1);
                preview_owner = Some((generation, input.task.id));
                let mut preview = Preview {
                    generation,
                    task_id: input.task.id,
                    thread_id: input.resume.clone(),
                    trust: Vec::new(),
                    payload: String::new(),
                    blockers: Vec::new(),
                };
                let trust = crate::agent::planning_trust(project);
                if let Ok(trust) = &trust {
                    preview.trust = trust.lines.iter().map(|line| safe(line)).collect();
                } else if let Err(error) = &trust {
                    preview.blockers.push(safe(error));
                }
                if project.to_str() != Some(input.task.project.as_str()) {
                    preview
                        .blockers
                        .push("Task project differs from the trusted worker project".into());
                }
                if input
                    .resume
                    .as_ref()
                    .is_some_and(|id| threads.get(id) != Some(&input.task.id))
                {
                    preview.blockers.push("Resume refused: thread was not started for this task in the current Eevee session".into());
                }
                if threads.len() >= REQUESTS_LIMIT && input.resume.is_none() {
                    preview.blockers.push("Session thread bound reached; restart Eevee before creating another thread".into());
                }
                match payload(data_dir, &input, exclusions) {
                    Ok(text) => preview.payload = text,
                    Err(error) => preview.blockers.push(safe(&error)),
                }
                if preview.blockers.is_empty()
                    && let Ok(trust) = trust
                {
                    prepared = Some(Prepared {
                        generation,
                        input,
                        trust,
                        text: preview.payload.clone(),
                    });
                }
                let _ = emit(&events, Event::Preview(preview));
                busy.store(false, Ordering::Release);
            }
            Request::Run {
                generation: requested,
            } => {
                let Some(run) = prepared.take() else {
                    let task_id = preview_owner
                        .filter(|(generation, _)| *generation == requested)
                        .map_or(0, |(_, task_id)| task_id);
                    finish(
                        &events,
                        requested,
                        task_id,
                        Outcome::Failed,
                        "Run refused: no valid immutable preview; review again".into(),
                    );
                    busy.store(false, Ordering::Release);
                    continue;
                };
                if run.generation != requested {
                    prepared = Some(run);
                    finish(
                        &events,
                        requested,
                        0,
                        Outcome::Failed,
                        "Stale planning consent rejected".into(),
                    );
                    busy.store(false, Ordering::Release);
                    continue;
                }
                let task_id = run.input.task.id;
                preview_owner = None;
                let result = run_transport(
                    &workspace,
                    &run,
                    &incoming,
                    &events,
                    &stop,
                    &cancelled,
                    &mut threads,
                );
                finish(&events, requested, task_id, result.0, result.1);
                busy.store(false, Ordering::Release);
            }
            Request::Cancel {
                generation: requested,
            } => {
                if preview_owner.is_some_and(|(generation, _)| generation == requested) {
                    let (_, task_id) = preview_owner.take().unwrap();
                    prepared = None;
                    finish(
                        &events,
                        requested,
                        task_id,
                        Outcome::Cancelled,
                        "Preview discarded; no executable or model launched".into(),
                    );
                }
                busy.store(false, Ordering::Release);
            }
            Request::Answer { generation, .. } => {
                let _ = emit(
                    &events,
                    Event::Error {
                        generation: Some(generation),
                        message: "Stale answer rejected: there is no pending provider request"
                            .into(),
                    },
                );
            }
        }
    }
    busy.store(false, Ordering::Release);
}
fn payload(data_dir: &Path, input: &Input, exclusions: &[PathBuf]) -> Result<String, String> {
    crate::store::validate_planning_input(data_dir, &input.task, &input.excerpts)?;
    let sources = crate::context::planning_payload(
        input.task.id,
        input.report.as_ref(),
        &input.excerpts,
        exclusions,
    )?;
    let text = serde_json::to_string_pretty(&json!({
        "purpose": "Plan the user's task. Do not implement changes. Ask the user when a decision is needed; report proposals as unverified planning, never as local execution evidence.",
        "task": { "id": input.task.id, "revision": input.task.revision, "goal": input.task.goal, "acceptance_criteria": input.task.criteria, "next_action": input.task.next_action },
        "selected_sources": serde_json::from_str::<Value>(&sources.text).map_err(|_| "Source payload is not valid JSON")?
    })).map_err(|_| "Could not encode planning payload")?;
    if text.len() > FRAME_LIMIT / 2 {
        return Err("Planning payload exceeds 1 MiB; reduce the source basket".into());
    }
    if crate::privacy::redact(&text) != text {
        return Err(
            "Planning payload contains a recognizable credential; edit it before review".into(),
        );
    }
    if text.chars().any(bidi) {
        return Err(
            "Planning payload contains directional display controls; remove them before review"
                .into(),
        );
    }
    Ok(text)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    Initialize,
    Account,
    Models,
    Thread,
    Turn,
    Interrupt,
}
struct Wire {
    bytes: Vec<u8>,
    refusal: bool,
}
struct PendingCall {
    id: String,
    kind: Call,
    deadline: Instant,
}
struct Item {
    kind: ItemKind,
    raw: String,
    complete: bool,
}
struct Engine<'a> {
    process: crate::runner::native::Process,
    run: &'a Prepared,
    project: &'a Path,
    data_dir: &'a Path,
    exclusions: &'a [PathBuf],
    events: &'a Events,
    stop: &'a AtomicBool,
    cancelled: &'a AtomicU64,
    wire: VecDeque<Wire>,
    offset: usize,
    write_deadline: Option<Instant>,
    framer: Framer,
    calls: Vec<PendingCall>,
    sequence: u64,
    thread_id: Option<String>,
    turn_id: Option<String>,
    model: String,
    effort: String,
    account: String,
    model_pages: usize,
    prompts: Vec<Prompt>,
    seen_requests: HashSet<String>,
    items: HashMap<String, Item>,
    output_bytes: usize,
    received: usize,
    process_exit: Option<Instant>,
    dispatched: bool,
    interrupt_sent: bool,
    cancelling: Option<Instant>,
    terminal: Option<(Outcome, String)>,
}
fn run_transport(
    workspace: &Workspace,
    run: &Prepared,
    incoming: &Receiver<Request>,
    events: &Events,
    stop: &AtomicBool,
    cancelled: &AtomicU64,
    threads: &mut HashMap<String, i64>,
) -> (Outcome, String) {
    let Workspace {
        project,
        data_dir,
        exclusions,
    } = workspace;
    let setup = (|| {
        if cancelled.load(Ordering::Acquire) == run.generation {
            return Err("Cancelled before launch".into());
        }
        revalidate(project, data_dir, exclusions, run)?;
        let prepared = crate::agent::qualify_planning(project, &run.trust, stop)?;
        if stop.load(Ordering::Acquire) || cancelled.load(Ordering::Acquire) == run.generation {
            return Err("Stopped before transport launch".into());
        }
        crate::runner::native::Process::spawn_piped(&prepared).map_err(|_| {
            "Process layer: trusted Codex app-server could not launch (OS details withheld)"
                .to_owned()
        })
    })();
    let process = match setup {
        Ok(process) => process,
        Err(error) => {
            return (
                if cancelled.load(Ordering::Acquire) == run.generation
                    || stop.load(Ordering::Acquire)
                {
                    Outcome::Cancelled
                } else {
                    Outcome::Failed
                },
                error,
            );
        }
    };
    let mut engine = Engine {
        process,
        run,
        project,
        data_dir,
        exclusions,
        events,
        stop,
        cancelled,
        wire: VecDeque::new(),
        offset: 0,
        write_deadline: None,
        framer: Framer::default(),
        calls: Vec::new(),
        sequence: 0,
        thread_id: None,
        turn_id: None,
        model: String::new(),
        effort: String::new(),
        account: String::new(),
        model_pages: 0,
        prompts: Vec::new(),
        seen_requests: HashSet::new(),
        items: HashMap::new(),
        output_bytes: 0,
        received: 0,
        process_exit: None,
        dispatched: false,
        interrupt_sent: false,
        cancelling: None,
        terminal: None,
    };
    let result: Result<(Outcome, String), String> = (|| {
        engine.status("Trusted executable version qualified; initializing app-server (no model dispatched yet)")?;
        engine.call(Call::Initialize, "initialize", json!({"clientInfo":{"name":"eevee","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true,"optOutNotificationMethods":["item/reasoning/textDelta","item/reasoning/summaryTextDelta","item/reasoning/summaryPartAdded"]}}))?;
        loop {
            if stop.load(Ordering::Acquire) {
                return Err("Shutdown: owned provider transport stopped".into());
            }
            if cancelled.load(Ordering::Acquire) == run.generation {
                engine.cancel()?;
            }
            for _ in 0..8 {
                match incoming.try_recv() {
                    Ok(Request::Cancel { generation }) if generation == run.generation => {
                        engine.cancel()?
                    }
                    Ok(Request::Answer {
                        generation,
                        request_id,
                        answer,
                    }) => engine.answer(generation, request_id, answer)?,
                    Ok(_) => engine.status(
                        "Stale or overlapping planning request rejected; no replay performed",
                    )?,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Err("Client disconnected".into());
                    }
                }
            }
            if let Some(result) = engine.terminal.take() {
                return Ok(result);
            }
            engine.flush()?;
            let mut buffer = [0u8; 8192];
            for _ in 0..32 {
                match engine.process.read(0, &mut buffer) {
                    Ok(0) => {
                        if engine.terminal.is_none() {
                            return Err(if engine.framer.partial() {
                                "Protocol layer: EOF inside a JSONL frame"
                            } else {
                                "Process layer: provider EOF without authoritative turn completion"
                            }
                            .into());
                        }
                        break;
                    }
                    Ok(n) => {
                        engine.received += n;
                        if engine.received > STREAM_LIMIT {
                            return Err("Protocol stream exceeded 64 MiB; no further provider input accepted".into());
                        }
                        for chunk in buffer[..n].split_inclusive(|byte| *byte == b'\n') {
                            for value in engine.framer.push(chunk)? {
                                engine.message(value)?;
                            }
                            if engine.terminal.is_some() {
                                break;
                            }
                        }
                        if engine.terminal.is_some() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => return Err("Process layer: provider stdout capture failed".into()),
                }
            }
            if let Some(result) = engine.terminal.take() {
                return Ok(result);
            }
            // Drain bounded stderr without retaining/logging provider secrets or private diagnostics.
            for _ in 0..8 {
                match engine.process.read(1, &mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        engine.received += n;
                        if engine.received > STREAM_LIMIT {
                            return Err(
                                "Provider diagnostics/stream exceeded the 64 MiB capture bound"
                                    .into(),
                            );
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) =>
                    {
                        break;
                    }
                    Err(_) => return Err("Process layer: provider stderr capture failed".into()),
                }
            }
            if let Some(result) = engine.terminal.take() {
                return Ok(result);
            }
            if engine
                .calls
                .iter()
                .any(|call| Instant::now() >= call.deadline)
            {
                return Err("Protocol layer: request timed out; no automatic retry".into());
            }
            if engine
                .cancelling
                .is_some_and(|start| start.elapsed() >= CANCEL_TIMEOUT)
            {
                return Err(
                    "Interrupt deadline expired without authoritative turn completion".into(),
                );
            }
            if engine
                .process
                .try_wait()
                .map_err(|_| "Process layer: app-server status unavailable")?
                .is_some()
            {
                let exited = engine.process_exit.get_or_insert_with(Instant::now);
                if exited.elapsed() >= Duration::from_millis(500) {
                    return Err(
                        "Process layer: app-server exited without authoritative turn completion"
                            .into(),
                    );
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
    })();
    if let Some(id) = &engine.thread_id {
        threads.insert(id.clone(), run.input.task.id);
    }
    engine.close_prompts();
    let tree_exited = engine.teardown();
    match result {
        Ok((outcome, message)) => (
            outcome,
            if tree_exited {
                message
            } else {
                format!("{message}; owned process-tree cleanup could not be established")
            },
        ),
        Err(error) => (
            if engine.dispatched {
                Outcome::Unknown
            } else if engine.cancelling.is_some() || stop.load(Ordering::Acquire) {
                Outcome::Cancelled
            } else {
                Outcome::Failed
            },
            format!(
                "{error}. {} No resend or retry.",
                if engine.dispatched {
                    "Turn outcome unknown; provider may have acted"
                } else {
                    "No turn was dispatched"
                }
            ),
        ),
    }
}
fn revalidate(
    project: &Path,
    data_dir: &Path,
    exclusions: &[PathBuf],
    run: &Prepared,
) -> Result<(), String> {
    if payload(data_dir, &run.input, exclusions)? != run.text {
        return Err(
            "Outgoing payload changed since review; no dispatch. Review a fresh preview".into(),
        );
    }
    crate::agent::revalidate_planning_trust(project, &run.trust)
}

impl Engine<'_> {
    fn status(&self, text: &str) -> Result<(), String> {
        emit(
            self.events,
            Event::Status {
                generation: self.run.generation,
                task_id: self.run.input.task.id,
                message: safe(text),
            },
        )
    }
    fn queue(&mut self, value: Value) -> Result<(), String> {
        let mut bytes =
            serde_json::to_vec(&value).map_err(|_| "Protocol layer: JSON encoding failed")?;
        if bytes.len() >= FRAME_LIMIT
            || self.wire.len() >= 32
            || self
                .wire
                .iter()
                .map(|frame| frame.bytes.len())
                .sum::<usize>()
                + bytes.len()
                > FRAME_LIMIT * 2
        {
            return Err("Protocol layer: outgoing queue/frame bound exceeded".into());
        }
        bytes.push(b'\n');
        if self.wire.is_empty() {
            self.write_deadline = Some(Instant::now() + Duration::from_secs(5));
        }
        let refusal = value.get("error").is_some()
            || value.get("method").and_then(Value::as_str) == Some("turn/interrupt")
            || matches!(
                value.pointer("/result/decision").and_then(Value::as_str),
                Some("decline" | "cancel")
            );
        self.wire.push_back(Wire { bytes, refusal });
        Ok(())
    }
    fn call(&mut self, kind: Call, method: &str, params: Value) -> Result<(), String> {
        self.sequence += 1;
        let id = format!("ev-{}-{}", self.run.generation, self.sequence);
        self.queue(json!({"id":id,"method":method,"params":params}))?;
        self.calls.push(PendingCall {
            id,
            kind,
            deadline: Instant::now() + RPC_TIMEOUT,
        });
        Ok(())
    }
    fn flush(&mut self) -> Result<(), String> {
        for _ in 0..32 {
            let Some(frame) = self.wire.front() else {
                return Ok(());
            };
            let bytes = &frame.bytes;
            if self
                .write_deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                return Err("Process layer: stdin write deadline exceeded".into());
            }
            match self.process.write_stdin(&bytes[self.offset..]) {
                Ok(0) => return Err("Process layer: provider stdin closed".into()),
                Ok(n) => {
                    self.offset += n;
                    if self.offset == bytes.len() {
                        self.wire.pop_front();
                        self.offset = 0;
                        self.write_deadline = (!self.wire.is_empty())
                            .then(|| Instant::now() + Duration::from_secs(5));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return Err("Process layer: provider stdin write failed".into()),
            }
        }
        Ok(())
    }
    fn message(&mut self, value: Value) -> Result<(), String> {
        if !value.is_object() {
            return Err("Protocol layer: JSONL message is not an object".into());
        }
        if let Some(method) = value.get("method").and_then(Value::as_str) {
            let params = value.get("params").unwrap_or(&Value::Null);
            if let Some(id) = value.get("id") {
                return self.server_request(id.clone(), method, params);
            }
            return self.notification(method, params);
        }
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or("Protocol layer: malformed response identity")?;
        let index = self
            .calls
            .iter()
            .position(|call| call.id == id)
            .ok_or("Protocol layer: stale/unknown response rejected")?;
        let call = self.calls.remove(index);
        if let Some(error) = value.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("provider RPC error");
            return Err(format!(
                "Protocol {:?} rejected: {}",
                call.kind,
                safe(message)
            ));
        }
        let result = value
            .get("result")
            .ok_or("Protocol layer: response has neither result nor error")?;
        if self.cancelling.is_some() && call.kind != Call::Turn && call.kind != Call::Interrupt {
            return Ok(());
        }
        self.response(call.kind, result)
    }
    fn response(&mut self, kind: Call, result: &Value) -> Result<(), String> {
        match kind {
            Call::Initialize => {
                if !result.get("userAgent").is_some_and(Value::is_string) { return Err("Protocol initialize response missing userAgent".into()); }
                self.queue(json!({"method":"initialized"}))?;
                self.call(Call::Account, "account/read", json!({"refreshToken":false}))
            }
            Call::Account => {
                self.account = match result.pointer("/account/type").and_then(Value::as_str) {
                    Some("chatgpt") => "ChatGPT account present; provider-owned subscription/billing and quota".into(),
                    Some("apiKey") => "API-key account present; provider-owned API billing and quota".into(),
                    Some("amazonBedrock") => "Amazon Bedrock account present; provider-owned cloud billing and quota".into(),
                    None if result.get("requiresOpenaiAuth").and_then(Value::as_bool) == Some(false) => "Provider does not require OpenAI auth; external credentials/billing unverified".into(),
                    _ => return Err("Authentication layer: no usable provider account; authenticate outside Eevee and review a fresh preview".into()),
                };
                self.call(Call::Models, "model/list", json!({"includeHidden":false,"limit":100}))
            }
            Call::Models => {
                self.model_pages += 1;
                let models = result.get("data").and_then(Value::as_array).ok_or("Protocol model/list missing data")?;
                if models.len() > 100 { return Err("Model catalogue page exceeds requested bound".into()); }
                if let Some(model) = models.iter().find(|model| model.get("isDefault").and_then(Value::as_bool) == Some(true) && model.get("hidden").and_then(Value::as_bool) != Some(true)) {
                    self.model = identity(model, "model")?;
                    self.effort = identity(model, "defaultReasoningEffort")?;
                    if model.get("inputModalities").and_then(Value::as_array).is_some_and(|modes| !modes.iter().any(|mode| mode == "text")) {
                        return Err("Model layer: default model does not support text planning".into());
                    }
                    if !model.get("supportedReasoningEfforts").and_then(Value::as_array).is_some_and(|efforts| efforts.iter().any(|effort| effort.get("reasoningEffort").and_then(Value::as_str) == Some(self.effort.as_str()))) {
                        return Err("Model layer: default effort is not supported by the selected model".into());
                    }
                    let mut params = json!({"cwd":self.project,"model":self.model,"sandbox":"read-only","approvalPolicy":"on-request","approvalsReviewer":"user","config":{"features.multi_agent":false,"web_search":"disabled","allow_login_shell":false}});
                    let method = if let Some(id) = &self.run.input.resume {
                        params["threadId"] = json!(id); params["excludeTurns"] = json!(true); "thread/resume"
                    } else {
                        params["experimentalRawEvents"] = json!(false); params["allowProviderModelFallback"] = json!(false); "thread/start"
                    };
                    self.call(Call::Thread, method, params)
                } else if let Some(cursor) = result.get("nextCursor").and_then(Value::as_str) {
                    if self.model_pages >= 10 || cursor.len() > 4096 { return Err("Model catalogue pagination bound exceeded; no default selected".into()); }
                    self.call(Call::Models, "model/list", json!({"includeHidden":false,"limit":100,"cursor":cursor}))
                } else { Err("Model layer: no available default model; no guessed model or fallback".into()) }
            }
            Call::Thread => {
                self.validate_policy(result)?;
                let thread = result.get("thread").ok_or("Protocol thread response missing thread")?;
                let id = identity(thread, "id")?;
                if self.run.input.resume.as_ref().is_some_and(|expected| expected != &id) { return Err("Protocol layer: resume returned a different thread".into()); }
                self.thread_id = Some(id.clone());
                emit(self.events, Event::Thread { generation: self.run.generation, task_id: self.run.input.task.id, thread_id: id.clone(), model: safe(&self.model), account: self.account.clone() })?;
                // Last read-only validation occurs after potentially slow startup and immediately
                // before the one model request; no outgoing bytes are silently replaced.
                revalidate(self.project, self.data_dir, self.exclusions, self.run)?;
                if self.stop.load(Ordering::Acquire) { return Err("Stopped before turn dispatch".into()); }
                if self.cancelled.load(Ordering::Acquire) == self.run.generation { return self.cancel(); }
                self.call(Call::Turn, "turn/start", json!({
                    "threadId":id,"input":[{"type":"text","text":self.run.text,"text_elements":[]}],
                    "cwd":self.project,"model":self.model,"approvalPolicy":"on-request","approvalsReviewer":"user",
                    "sandboxPolicy":{"type":"readOnly","networkAccess":false},"summary":"none",
                    "collaborationMode":{"mode":"plan","settings":{"model":self.model,"reasoning_effort":self.effort,"developer_instructions":null}}
                }))?;
                // Enqueueing is conservatively treated as dispatch: partial writes are outcome-unknown.
                self.dispatched = true;
                self.status("Plan turn dispatched once; read-only policy acknowledged by provider")
            }
            Call::Turn => {
                let turn = result.get("turn").ok_or("Protocol turn/start response missing turn")?;
                self.bind_turn(turn)?;
                if self.cancelling.is_some() { self.interrupt()?; }
                self.turn_terminal(turn)
            }
            Call::Interrupt => self.status("Provider acknowledged interrupt request; waiting for authoritative turn completion"),
        }
    }
    fn validate_policy(&self, result: &Value) -> Result<(), String> {
        if result.get("approvalPolicy").and_then(Value::as_str) != Some("on-request")
            || result.get("approvalsReviewer").and_then(Value::as_str) != Some("user")
            || result.pointer("/sandbox/type").and_then(Value::as_str) != Some("readOnly")
            || result
                .pointer("/sandbox/networkAccess")
                .and_then(Value::as_bool)
                == Some(true)
            || result.get("model").and_then(Value::as_str) != Some(self.model.as_str())
            || result.get("cwd").and_then(Value::as_str) != self.project.to_str()
        {
            return Err("Policy qualification refused: provider did not acknowledge exact model/cwd/read-only/user-approval boundaries".into());
        }
        Ok(())
    }
    fn bind_turn(&mut self, turn: &Value) -> Result<(), String> {
        let id = identity(turn, "id")?;
        if self
            .turn_id
            .as_ref()
            .is_some_and(|expected| expected != &id)
        {
            return Err("Protocol layer: conflicting turn identity".into());
        }
        self.turn_id = Some(id);
        Ok(())
    }
    fn correlated(&self, params: &Value) -> bool {
        self.thread_id.as_deref() == params.get("threadId").and_then(Value::as_str)
            && self.turn_id.as_deref() == params.get("turnId").and_then(Value::as_str)
            && self.thread_id.is_some()
            && self.turn_id.is_some()
    }
    fn notification(&mut self, method: &str, params: &Value) -> Result<(), String> {
        if method == "turn/started" || method == "turn/completed" {
            if !self.dispatched
                || self.thread_id.as_deref() != params.get("threadId").and_then(Value::as_str)
            {
                return Ok(());
            }
            let turn = params
                .get("turn")
                .ok_or("Protocol turn notification missing turn")?;
            self.bind_turn(turn)?;
            if method == "turn/completed" {
                return self.turn_terminal(turn);
            }
            if self.cancelling.is_some() {
                self.interrupt()?;
            }
            return Ok(());
        }
        if method == "serverRequest/resolved" {
            if self.thread_id.as_deref() == params.get("threadId").and_then(Value::as_str)
                && let Some(index) = self
                    .prompts
                    .iter()
                    .position(|prompt| Some(&prompt.request_id) == params.get("requestId"))
            {
                let prompt = self.prompts.remove(index);
                emit(
                    self.events,
                    Event::PromptClosed {
                        generation: self.run.generation,
                        request_id: prompt.request_id,
                    },
                )?;
                self.status("Provider resolved a pending request without a client answer; its draft is invalidated")?;
            }
            return Ok(());
        }
        if method == "thread/settings/updated"
            && self.thread_id.as_deref() == params.get("threadId").and_then(Value::as_str)
        {
            // Exact installed ThreadSettings schema; a notification cannot silently weaken
            // the boundary acknowledged by thread/start or switch out of Plan mode.
            let settings = &params["threadSettings"];
            if settings.get("approvalPolicy").and_then(Value::as_str) != Some("on-request")
                || settings.get("approvalsReviewer").and_then(Value::as_str) != Some("user")
                || settings
                    .pointer("/sandboxPolicy/type")
                    .and_then(Value::as_str)
                    != Some("readOnly")
                || settings
                    .pointer("/sandboxPolicy/networkAccess")
                    .and_then(Value::as_bool)
                    == Some(true)
                || settings.get("model").and_then(Value::as_str) != Some(self.model.as_str())
                || settings.get("cwd").and_then(Value::as_str) != self.project.to_str()
                || self.dispatched
                    && settings
                        .pointer("/collaborationMode/mode")
                        .and_then(Value::as_str)
                        != Some("plan")
            {
                return Err("Policy changed during planning; dispatch/continuation refused".into());
            }
        }
        if matches!(
            method,
            "modelProvider/authRecoveryStarted" | "model/rerouted"
        ) {
            return Err("Provider attempted authentication recovery/model reroute; no automatic retry or fallback accepted".into());
        }
        if method == "warning" || method == "configWarning" || method == "guardianWarning" {
            if let Some(message) = params.get("message").and_then(Value::as_str) {
                self.status(&format!("Provider warning: {}", bounded(message, 4096)))?;
            }
            return Ok(());
        }
        if !self.correlated(params) {
            return Ok(());
        }
        match method {
            "error" => {
                let message = params
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("provider turn error");
                if params.get("willRetry").and_then(Value::as_bool) == Some(true) {
                    return Err(format!("Provider attempted retry: {}", safe(message)));
                }
                self.status(&format!(
                    "Provider error (awaiting authoritative completion): {}",
                    bounded(message, 4096)
                ))
            }
            "item/agentMessage/delta" | "item/plan/delta" | "item/commandExecution/outputDelta" => {
                let kind = match method {
                    "item/agentMessage/delta" => ItemKind::Message,
                    "item/plan/delta" => ItemKind::Plan,
                    _ => ItemKind::Tool,
                };
                let id = identity(params, "itemId")?;
                let text = params
                    .get("delta")
                    .and_then(Value::as_str)
                    .ok_or("Protocol delta missing text")?;
                self.item(id, kind, text, false, true)
            }
            "item/started" | "item/completed" => {
                let item = params
                    .get("item")
                    .ok_or("Protocol item notification missing item")?;
                let complete = method == "item/completed";
                self.full_item(item, complete)
            }
            "turn/plan/updated" => {
                let plan = params
                    .get("plan")
                    .and_then(Value::as_array)
                    .ok_or("Protocol plan missing steps")?;
                if plan.len() > 64 {
                    return Err("Plan step bound exceeded".into());
                }
                let mut text = params
                    .get("explanation")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                for step in plan {
                    text.push('\n');
                    text.push_str(
                        step.get("status")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown"),
                    );
                    text.push_str(": ");
                    text.push_str(
                        step.get("step")
                            .and_then(Value::as_str)
                            .ok_or("Plan step missing text")?,
                    );
                }
                self.item(
                    "eevee-turn-plan".into(),
                    ItemKind::Plan,
                    &text,
                    false,
                    false,
                )
            }
            // Private reasoning, raw response events, arbitrary MCP outputs and user echoes
            // deliberately never enter the display or decision-selection stream.
            _ => Ok(()),
        }
    }
    fn full_item(&mut self, item: &Value, complete: bool) -> Result<(), String> {
        let kind = match item.get("type").and_then(Value::as_str) {
            Some("agentMessage") => ItemKind::Message,
            Some("plan") => ItemKind::Plan,
            Some("commandExecution") => ItemKind::Tool,
            Some("fileChange") => {
                if complete && item.get("status").and_then(Value::as_str) == Some("completed") {
                    return Err("Policy boundary: provider reported applied file edits during read-only planning".into());
                }
                let status = item
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                return self.item(identity(item, "id")?, ItemKind::Tool,
                    &format!("File-edit activity: {status}. Eevee denies file-change approval callbacks; no diff retained."),
                    complete, false);
            }
            Some("collabAgentToolCall" | "subAgentActivity" | "webSearch") => {
                return Err(
                    "Policy boundary: provider attempted disabled delegation/web search".into(),
                );
            }
            Some("mcpToolCall") => {
                return self.status("Provider-owned MCP tool activity observed; its enforcement/side effects are outside Eevee's callback coverage");
            }
            _ => return Ok(()),
        };
        let id = identity(item, "id")?;
        if kind == ItemKind::Tool {
            let text = format!(
                "Command: {}\nCwd: {}\nStatus: {}\n{}",
                item.get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("(not supplied)"),
                item.get("cwd")
                    .and_then(Value::as_str)
                    .unwrap_or("(not supplied)"),
                item.get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
                item.get("aggregatedOutput")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            );
            self.item(id, kind, &text, complete, false)
        } else {
            self.item(
                id,
                kind,
                item.get("text").and_then(Value::as_str).unwrap_or(""),
                complete,
                false,
            )
        }
    }
    fn item(
        &mut self,
        id: String,
        kind: ItemKind,
        text: &str,
        complete: bool,
        delta: bool,
    ) -> Result<(), String> {
        if !self.items.contains_key(&id) && self.items.len() >= ITEMS_LIMIT {
            return Err(
                "Output exceeded 128 items; transport stopped rather than silently omit decisions"
                    .into(),
            );
        }
        let item = self.items.entry(id.clone()).or_insert_with(|| Item {
            kind,
            raw: String::new(),
            complete: false,
        });
        if item.kind != kind {
            return Err("Protocol item identity changed type".into());
        }
        if item.complete {
            return Ok(());
        }
        let previous = item.raw.len();
        let length = if delta {
            previous.saturating_add(text.len())
        } else {
            text.len()
        };
        if length > ITEM_LIMIT || self.output_bytes - previous + length > OUTPUT_LIMIT {
            return Err("Output exceeded 32 KiB per item / 512 KiB total; stopped with explicit overflow, no silent truncation".into());
        }
        if delta {
            item.raw.push_str(text);
        } else {
            item.raw.clear();
            item.raw.push_str(text);
        }
        self.output_bytes = self.output_bytes - previous + length;
        item.complete = complete;
        // Redact the cumulative item, not individual deltas. Withhold a partial final line so
        // credentials split across JSONL deltas are not exposed before their prefix is complete.
        let visible = if complete || !delta {
            item.raw.as_str()
        } else {
            &item.raw[..item.raw.rfind('\n').map_or(0, |end| end + 1)]
        };
        if visible.is_empty() && !complete {
            return Ok(());
        }
        emit(
            self.events,
            Event::Item {
                generation: self.run.generation,
                task_id: self.run.input.task.id,
                thread_id: self.thread_id.clone().ok_or("Missing item thread")?,
                turn_id: self.turn_id.clone().ok_or("Missing item turn")?,
                item_id: id,
                kind,
                text: safe(visible),
                complete,
            },
        )
    }
    fn turn_terminal(&mut self, turn: &Value) -> Result<(), String> {
        let status = turn
            .get("status")
            .and_then(Value::as_str)
            .ok_or("Protocol turn missing status")?;
        let mut outcome = match status {
            "completed" => Outcome::Completed,
            "interrupted" => Outcome::Cancelled,
            "failed" => Outcome::Failed,
            "inProgress" => return Ok(()),
            _ => return Err("Protocol turn has unknown status".into()),
        };
        let mut capture_error = None;
        if let Some(items) = turn.get("items").and_then(Value::as_array) {
            for item in items {
                if let Err(error) = self.full_item(item, true) {
                    capture_error = Some(error);
                    break;
                }
            }
        }
        let mut message = if outcome == Outcome::Failed {
            format!(
                "Provider turn failed: {}",
                safe(
                    turn.pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("No public provider error supplied")
                )
            )
        } else if outcome == Outcome::Cancelled {
            "Provider authoritatively reported interrupted turn; prior provider actions are not rolled back".into()
        } else {
            "Provider authoritatively completed the turn. Planning text is unverified, not local execution evidence".into()
        };
        if let Some(error) = capture_error {
            message.push_str(&format!(
                " Eevee output/policy failure: {error}; displayed output is incomplete."
            ));
            if outcome == Outcome::Completed {
                outcome = Outcome::Failed;
            }
        }
        self.terminal = Some((outcome, message));
        Ok(())
    }
    fn refuse(&mut self, id: Value, code: i64, message: &str) -> Result<(), String> {
        self.queue(json!({"id":id,"error":{"code":code,"message":message}}))
    }
    fn server_request(&mut self, id: Value, method: &str, params: &Value) -> Result<(), String> {
        let key = request_key(&id)?;
        if !self.seen_requests.insert(key) {
            self.refuse(id, -32600, "Duplicate server request identity refused")?;
            return Err("Duplicate provider request identity; transport stopped".into());
        }
        if self.seen_requests.len() > REQUESTS_LIMIT {
            self.refuse(id, -32600, "Server request bound exceeded")?;
            return Err("More than 256 server requests in one turn".into());
        }
        if !self.correlated(params)
            || self.cancelling.is_some()
            || self.terminal.is_some()
            || self.cancelled.load(Ordering::Acquire) == self.run.generation
        {
            self.refuse(
                id,
                -32800,
                "Stale, uncorrelated, cancelled or completed turn request refused",
            )?;
            return self.status("Stale/uncorrelated server request explicitly refused");
        }
        if method == "item/fileChange/requestApproval" {
            self.queue(json!({"id":id,"result":{"decision":"decline"}}))?;
            return self.status("File-edit request denied by Eevee read-only planning policy");
        }
        if !matches!(
            method,
            "item/commandExecution/requestApproval" | "item/tool/requestUserInput"
        ) {
            self.refuse(id, -32601, "Unsupported server request; Eevee does not grant permissions, authenticate, run dynamic tools or accept MCP elicitation")?;
            return self.status(
                "Unsupported provider request explicitly refused; no permission was granted",
            );
        }
        if !self.prompts.is_empty() {
            self.refuse(
                id,
                -32600,
                "Another user request is already pending; concurrent request refused",
            )?;
            return self.status(
                "Concurrent provider request refused; the original prompt remains pending",
            );
        }
        let kind = if method == "item/tool/requestUserInput" {
            match questions(params) {
                Ok(questions) => PromptKind::Questions(questions),
                Err(error) => {
                    self.refuse(id, -32602, "Invalid or unsupported user-input request")?;
                    return self.status(&error);
                }
            }
        } else {
            if params
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind != "command")
                || params
                    .get("environmentId")
                    .and_then(Value::as_str)
                    .is_some_and(|environment| environment != "local")
            {
                self.refuse(id, -32602, "Only local command approval is supported; terminal-input/remote requests refused")?;
                return self.status(
                    "Unsupported terminal-input or remote command approval explicitly refused",
                );
            }
            let decisions = params.get("availableDecisions").and_then(Value::as_array);
            let deny_decision = if decisions
                .is_none_or(|values| values.iter().any(|value| value == "decline"))
            {
                "decline"
            } else if decisions.is_some_and(|values| values.iter().any(|value| value == "cancel")) {
                "cancel"
            } else {
                self.refuse(id, -32602, "Required deny decision not supported")?;
                return self.status("Command request refused: provider did not offer deny/cancel");
            };
            let command = params
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("(command not supplied; allow-once unavailable)");
            let cwd = params
                .get("cwd")
                .and_then(Value::as_str)
                .unwrap_or("(cwd not supplied; allow-once unavailable)");
            let reason = format!(
                "{}\nAdditional permissions: {}\nNetwork context: {}\nThis approval may run the command outside the requested sandbox. It is once only, not a persisted rule.",
                params
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("No reason supplied"),
                params.get("additionalPermissions").unwrap_or(&Value::Null),
                params.get("networkApprovalContext").unwrap_or(&Value::Null)
            );
            if command.len() + cwd.len() + reason.len() > ITEM_LIMIT {
                self.refuse(id, -32602, "Approval details exceed review bound")?;
                return self
                    .status("Oversized command approval refused; no hidden approval details");
            }
            let shown_command = safe(command);
            let shown_cwd = safe(cwd);
            let allow_once = params.get("command").is_some_and(Value::is_string)
                && params.get("cwd").is_some_and(Value::is_string)
                && shown_command == command
                && shown_cwd == cwd
                && decisions
                    .is_none_or(|decisions| decisions.iter().any(|decision| decision == "accept"));
            PromptKind::Approval {
                command: shown_command,
                cwd: shown_cwd,
                reason: safe(&reason),
                allow_once,
                deny_decision: deny_decision.into(),
            }
        };
        let item_id = match identity(params, "itemId") {
            Ok(id) => id,
            Err(error) => {
                self.refuse(id, -32602, "Missing or invalid item identity")?;
                return Err(error);
            }
        };
        let prompt = Prompt {
            request_id: id,
            thread_id: self.thread_id.clone().unwrap(),
            turn_id: self.turn_id.clone().unwrap(),
            item_id,
            kind,
        };
        emit(
            self.events,
            Event::Prompt {
                generation: self.run.generation,
                task_id: self.run.input.task.id,
                request: prompt.clone(),
            },
        )?;
        self.prompts.push(prompt);
        Ok(())
    }
    fn answer(&mut self, generation: u64, id: Value, answer: Value) -> Result<(), String> {
        if generation != self.run.generation
            || self.cancelling.is_some()
            || self.terminal.is_some()
            || self.cancelled.load(Ordering::Acquire) == self.run.generation
        {
            return self.status("Stale/cancelled answer rejected; nothing sent to provider");
        }
        let Some(index) = self.prompts.iter().position(|prompt| {
            prompt.request_id == id
                && Some(&prompt.thread_id) == self.thread_id.as_ref()
                && Some(&prompt.turn_id) == self.turn_id.as_ref()
        }) else {
            return self.status("Answer rejected: server request no longer pending for this turn");
        };
        if let Err(error) = validate_answer(&self.prompts[index].kind, &answer) {
            return self.status(&format!("Answer not sent: {error}"));
        }
        // Grant scope and immutable input/config remain valid at approval/answer dispatch as well.
        revalidate(self.project, self.data_dir, self.exclusions, self.run)?;
        if self.cancelled.load(Ordering::Acquire) == self.run.generation {
            return self.cancel();
        }
        self.queue(json!({"id":id,"result":answer}))?;
        let prompt = self.prompts.remove(index);
        emit(
            self.events,
            Event::PromptClosed {
                generation,
                request_id: prompt.request_id,
            },
        )
    }
    fn close_prompts(&mut self) {
        for prompt in self.prompts.drain(..) {
            let _ = emit(
                self.events,
                Event::PromptClosed {
                    generation: self.run.generation,
                    request_id: prompt.request_id,
                },
            );
        }
    }
    fn cancel(&mut self) -> Result<(), String> {
        if self.cancelling.is_some() {
            return Ok(());
        }
        self.cancelling = Some(Instant::now());
        self.discard_grants()?;
        let prompts = std::mem::take(&mut self.prompts);
        for prompt in prompts {
            self.refuse(
                prompt.request_id.clone(),
                -32800,
                "User cancelled; no approval or answer granted",
            )?;
            emit(
                self.events,
                Event::PromptClosed {
                    generation: self.run.generation,
                    request_id: prompt.request_id,
                },
            )?;
        }
        if !self.dispatched {
            self.terminal = Some((
                Outcome::Cancelled,
                "Cancelled before turn dispatch; no model request sent".into(),
            ));
            return Ok(());
        }
        self.status("Cancellation invalidated pending answers; interrupting provider, then bounded owned teardown")?;
        self.interrupt()
    }
    fn interrupt(&mut self) -> Result<(), String> {
        if self.interrupt_sent {
            return Ok(());
        }
        if let (Some(thread), Some(turn)) = (&self.thread_id, &self.turn_id) {
            self.call(
                Call::Interrupt,
                "turn/interrupt",
                json!({"threadId":thread,"turnId":turn}),
            )?;
            self.interrupt_sent = true;
        }
        Ok(())
    }
    fn discard_grants(&mut self) -> Result<(), String> {
        if self.offset != 0 && self.wire.front().is_some_and(|frame| !frame.refusal) {
            // Never complete a partially written approval/turn after cancellation or failure.
            self.wire.clear();
            return Err(
                "Cancelled during a partial protocol write; provider receipt is unknown".into(),
            );
        }
        self.wire.retain(|frame| frame.refusal);
        if self.wire.is_empty() {
            self.offset = 0;
            self.write_deadline = None;
        }
        Ok(())
    }
    fn teardown(&mut self) -> bool {
        let _ = self.discard_grants();
        // Give an explicit refusal a bounded chance to reach the server before closing the pipe.
        let deadline = Instant::now() + Duration::from_millis(100);
        while !self.wire.is_empty() && Instant::now() < deadline {
            if self.flush().is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        self.process.close_stdin();
        let _ = self.process.terminate(false);
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(750) {
            let _ = self.process.try_wait();
            if self.process.tree_empty().unwrap_or(false) {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.process.terminate(true);
        let deadline = Instant::now() + Duration::from_millis(750);
        while Instant::now() < deadline {
            let _ = self.process.try_wait();
            if self.process.tree_empty().unwrap_or(false) {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }
}

#[derive(Default)]
struct Framer {
    line: Vec<u8>,
}
impl Framer {
    fn partial(&self) -> bool {
        !self.line.is_empty()
    }
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>, String> {
        let mut frames = Vec::new();
        for chunk in bytes.split_inclusive(|byte| *byte == b'\n') {
            if self.line.len() + chunk.len() > FRAME_LIMIT {
                return Err("Protocol layer: JSONL frame exceeds 2 MiB".into());
            }
            self.line.extend_from_slice(chunk);
            if chunk.last() == Some(&b'\n') {
                if self.line.iter().all(u8::is_ascii_whitespace) {
                    return Err("Protocol layer: empty JSONL frame".into());
                }
                let value = serde_json::from_slice(&self.line).map_err(
                    |_| "Protocol layer: malformed JSON/UTF-8; raw provider bytes withheld",
                )?;
                frames.push(value);
                self.line.clear();
            }
        }
        Ok(frames)
    }
}
fn identity(value: &Value, field: &str) -> Result<String, String> {
    let text = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Protocol layer: missing {field} identity"))?;
    if text.is_empty()
        || text.len() > 256
        || text.chars().any(|c| c.is_control() || bidi(c))
        || crate::privacy::redact(text) != text
    {
        return Err(format!("Protocol layer: invalid {field} identity"));
    }
    Ok(text.to_owned())
}
fn request_key(id: &Value) -> Result<String, String> {
    match id {
        Value::String(text)
            if !text.is_empty()
                && text.len() <= 256
                && !text.chars().any(|c| c.is_control() || bidi(c))
                && crate::privacy::redact(text) == text.as_str() =>
        {
            Ok(format!("s:{text}"))
        }
        Value::Number(number) if number.is_i64() || number.is_u64() => Ok(format!("n:{number}")),
        _ => Err("Protocol layer: invalid server request ID".into()),
    }
}
fn questions(params: &Value) -> Result<Vec<Question>, String> {
    if params.get("isBlocking").and_then(Value::as_bool) != Some(true) {
        return Err(
            "Nonblocking/auto-resolving user-input request refused; explicit user answer required"
                .into(),
        );
    }
    let values = params
        .get("questions")
        .and_then(Value::as_array)
        .ok_or("Question request missing questions")?;
    if values.is_empty() || values.len() > 8 {
        return Err("Question request must contain 1–8 questions".into());
    }
    let mut result = Vec::with_capacity(values.len());
    let mut ids = HashSet::new();
    let mut bytes = 0;
    for value in values {
        let id = identity(value, "id")?;
        if !ids.insert(id.clone()) {
            return Err("Duplicate question identity refused".into());
        }
        let header = value
            .get("header")
            .and_then(Value::as_str)
            .ok_or("Question missing header")?;
        let question = value
            .get("question")
            .and_then(Value::as_str)
            .ok_or("Question missing text")?;
        bytes += header.len() + question.len();
        let mut options = Vec::new();
        if let Some(values) = value.get("options").and_then(Value::as_array) {
            if values.len() > 16 {
                return Err("Question has more than 16 options".into());
            }
            for value in values {
                let label = value
                    .get("label")
                    .and_then(Value::as_str)
                    .ok_or("Question option missing label")?;
                let description = value
                    .get("description")
                    .and_then(Value::as_str)
                    .ok_or("Question option missing description")?;
                // Option labels are wire answers. Never silently replace them with a masked value.
                if safe(label) != label {
                    return Err(
                        "Question option contains private/terminal-control text; request refused"
                            .into(),
                    );
                }
                bytes += label.len() + description.len();
                options.push(QuestionOption {
                    label: label.into(),
                    description: safe(description),
                });
            }
        }
        if bytes > ITEM_LIMIT {
            return Err("Question request exceeds 32 KiB review bound".into());
        }
        result.push(Question {
            id,
            header: safe(header),
            question: safe(question),
            other: value
                .get("isOther")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || options.is_empty(),
            secret: value
                .get("isSecret")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            options,
        });
    }
    Ok(result)
}
fn validate_answer(kind: &PromptKind, answer: &Value) -> Result<(), String> {
    let object = answer.as_object().ok_or("Answer must be an object")?;
    if object.len() != 1 {
        return Err("Answer contains unsupported fields".into());
    }
    match kind {
        PromptKind::Approval {
            allow_once,
            deny_decision,
            ..
        } => match answer.get("decision").and_then(Value::as_str) {
            Some(decision) if decision == deny_decision => Ok(()),
            Some("accept") if *allow_once => Ok(()),
            _ => Err("Only deny or server-supported allow-once is permitted".into()),
        },
        PromptKind::Questions(questions) => {
            let answers = answer
                .get("answers")
                .and_then(Value::as_object)
                .ok_or("Missing answers map")?;
            if answers.len() != questions.len() {
                return Err("Every current question requires an explicit answer".into());
            }
            let mut bytes = 0;
            for question in questions {
                let entry = answers
                    .get(&question.id)
                    .and_then(Value::as_object)
                    .ok_or("Question identity missing from answer")?;
                if entry.len() != 1 {
                    return Err("Question answer contains unsupported fields".into());
                }
                let values = entry
                    .get("answers")
                    .and_then(Value::as_array)
                    .ok_or("Missing answer array")?;
                if values.len() != 1 {
                    return Err(
                        "Exactly one explicit option or literal answer required per question"
                            .into(),
                    );
                }
                let text = values[0].as_str().ok_or("Answer is not literal text")?;
                bytes += text.len();
                if text.is_empty()
                    || bytes > 16 * 1024
                    || text
                        .chars()
                        .any(|c| c.is_control() && c != '\n' && c != '\t' || bidi(c))
                {
                    return Err("Empty, oversized or control-bearing answer refused".into());
                }
                if !question.other && !question.options.iter().any(|option| option.label == text) {
                    return Err("Question requires one of the offered exact options".into());
                }
                if crate::privacy::redact(text) != text {
                    return Err("Recognizable credential in answer; nothing sent".into());
                }
            }
            Ok(())
        }
    }
}
fn bidi(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}
fn safe(text: &str) -> String {
    crate::privacy::redact(text)
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' || bidi(c) {
                '\u{fffd}'
            } else {
                c
            }
        })
        .collect()
}
fn bounded(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_events_replace_item_snapshots_and_reserve_terminal_capacity() {
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let make_item = |text: &str, complete| Event::Item {
            generation: 7,
            task_id: 3,
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "item".into(),
            kind: ItemKind::Message,
            text: text.into(),
            complete,
        };
        emit(&events, make_item("first delta snapshot", false)).unwrap();
        emit(&events, make_item("completed authoritative text", true)).unwrap();
        assert_eq!(events.lock().len(), 1);
        assert!(
            matches!(events.lock().front(), Some(Event::Item { text, complete: true, .. }) if text == "completed authoritative text")
        );
        for _ in 1..EVENTS_LIMIT - EVENT_RESERVE {
            emit(
                &events,
                Event::Status {
                    generation: 7,
                    task_id: 3,
                    message: "bounded".into(),
                },
            )
            .unwrap();
        }
        assert!(
            emit(
                &events,
                Event::Status {
                    generation: 7,
                    task_id: 3,
                    message: "overflow".into()
                }
            )
            .is_err()
        );
        finish(
            &events,
            7,
            3,
            Outcome::Unknown,
            "Event overflow; no authoritative completion".into(),
        );
        assert!(matches!(
            events.lock().back(),
            Some(Event::Finished {
                outcome: Outcome::Unknown,
                ..
            })
        ));
    }
    #[test]
    fn nonblocking_and_duplicate_question_requests_are_refused() {
        let question = json!({"id":"q","header":"Choice","question":"Pick one","options":[{"label":"A","description":"literal"}],"isOther":true,"isSecret":false});
        assert!(questions(&json!({"isBlocking":false,"questions":[question.clone()]})).is_err());
        assert!(
            questions(&json!({"isBlocking":true,"questions":[question.clone(),question.clone()]}))
                .is_err()
        );
        assert_eq!(
            questions(&json!({"isBlocking":true,"questions":[question]}))
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn jsonl_handles_utf8_and_multiple_frames_without_accepting_partial_eof() {
        let mut framer = Framer::default();
        let bytes = "{\"result\":\"문자\"}\n{\"id\":1}\n".as_bytes();
        assert!(framer.push(&bytes[..13]).unwrap().is_empty());
        assert!(framer.partial());
        let frames = framer.push(&bytes[13..]).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["result"], "문자");
        assert!(!framer.partial());
        assert!(framer.push(b"{\"id\":").unwrap().is_empty());
        assert!(framer.partial());
    }
    #[test]
    fn malformed_and_oversized_frames_fail_closed_without_raw_diagnostics() {
        let error = Framer::default().push(b"secret-canary\n").unwrap_err();
        assert!(!error.contains("secret-canary"));
        assert!(
            Framer::default()
                .push(&vec![b' '; FRAME_LIMIT + 1])
                .is_err()
        );
        assert!(Framer::default().push(b"\n").is_err());
    }
    #[test]
    fn request_identity_preserves_string_vs_number_and_refuses_unsafe_ids() {
        assert_ne!(
            request_key(&json!(1)).unwrap(),
            request_key(&json!("1")).unwrap()
        );
        for id in [
            Value::Null,
            json!(true),
            json!(1.5),
            json!("\u{001b}[31m"),
            json!("sk-sensitive123"),
        ] {
            assert!(request_key(&id).is_err());
        }
    }
    #[test]
    fn approvals_never_accept_session_rules_or_hidden_choices() {
        let kind = PromptKind::Approval {
            command: "echo hello".into(),
            cwd: "/tmp".into(),
            reason: String::new(),
            allow_once: true,
            deny_decision: "decline".into(),
        };
        assert!(validate_answer(&kind, &json!({"decision":"accept"})).is_ok());
        assert!(validate_answer(&kind, &json!({"decision":"decline"})).is_ok());
        for decision in [
            json!("acceptForSession"),
            json!({"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["echo"]}}),
        ] {
            assert!(validate_answer(&kind, &json!({"decision":decision})).is_err());
        }
        assert!(validate_answer(&kind, &json!({"decision":"accept","extra":true})).is_err());
        let kind = PromptKind::Approval {
            command: "printf literal".into(),
            cwd: "/tmp".into(),
            reason: String::new(),
            allow_once: true,
            deny_decision: "cancel".into(),
        };
        assert!(validate_answer(&kind, &json!({"decision":"cancel"})).is_ok());
        assert!(validate_answer(&kind, &json!({"decision":"decline"})).is_err());
    }
    #[test]
    fn answers_require_exact_current_question_ids_and_literal_user_input() {
        let kind = PromptKind::Questions(vec![Question {
            id: "current".into(),
            header: String::new(),
            question: String::new(),
            options: vec![],
            other: true,
            secret: false,
        }]);
        assert!(
            validate_answer(
                &kind,
                &json!({"answers":{"current":{"answers":["literal custom choice"]}}})
            )
            .is_ok()
        );
        assert!(
            validate_answer(
                &kind,
                &json!({"answers":{"stale":{"answers":["literal custom choice"]}}})
            )
            .is_err()
        );
        assert!(validate_answer(&kind, &json!({"answers":{"current":{"answers":[""]}}})).is_err());
        assert!(
            validate_answer(
                &kind,
                &json!({"answers":{"current":{"answers":["token=secret-canary"]}}})
            )
            .is_err()
        );
    }
    #[test]
    fn terminal_strings_and_cumulative_credentials_are_masked() {
        assert!(!safe("\u{001b}[31m\u{202e} token=secret-canary").contains("secret-canary"));
        assert!(!safe("\u{001b}[31m\u{202e}").contains('\u{001b}'));
        assert!(!safe("\u{001b}[31m\u{202e}").contains('\u{202e}'));
    }
}
