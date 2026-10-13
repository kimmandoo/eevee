//! Private ACP v1 transport. Client callbacks are deliberately not filesystem/terminal sandboxes.
use crate::provider::{
    Agent, Event, Input, ItemKind, Operation, Outcome, PermissionOption, Preview, Prompt,
    PromptKind, Request,
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt::Write as _,
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const FRAME_LIMIT: usize = 2 * 1024 * 1024;
const EVENT_LIMIT: usize = 512;
const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const ITEM_LIMIT: usize = 128;
type Events = Arc<Mutex<VecDeque<Event>>>;

pub struct Worker {
    requests: Option<SyncSender<Request>>,
    events: Events,
    stop: Arc<AtomicBool>,
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
        let (sender, incoming) = mpsc::sync_channel(8);
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicU64::new(0));
        let (output, stopped, active, cancel) = (
            events.clone(),
            stop.clone(),
            busy.clone(),
            cancelled.clone(),
        );
        let thread = thread::Builder::new()
            .name("ev-acp".into())
            .spawn(move || {
                worker(
                    Workspace {
                        project,
                        data_dir,
                        exclusions,
                    },
                    incoming,
                    output,
                    stopped,
                    active,
                    cancel,
                );
            })?;
        Ok(Self {
            requests: Some(sender),
            events,
            stop,
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
        if !matches!(request, Request::Cancel { .. })
            && self.events.lock().len() >= EVENT_LIMIT - 32
        {
            *pending = Some(request);
            return false;
        }
        if let Request::Cancel { generation } = &request {
            self.cancelled.fetch_max(*generation, Ordering::Release);
        }
        let marks_busy = matches!(request, Request::Preview(_) | Request::Run { .. });
        let previous = marks_busy && self.busy.swap(true, Ordering::AcqRel);
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
        self.stop.store(true, Ordering::Release);
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
struct Workspace {
    project: PathBuf,
    data_dir: PathBuf,
    exclusions: Vec<PathBuf>,
}
struct Reviewed {
    generation: u64,
    input: Box<Input>,
    trust: crate::agent::ExternalTrust,
    payload: String,
}
fn profile(agent: Agent) -> Result<(usize, &'static str, Vec<String>), String> {
    match agent {
        Agent::OpenCode => Ok((
            2,
            crate::agent::OPENCODE_RELEASE,
            vec!["acp".into(), "--pure".into()],
        )),
        Agent::Gemini => Ok((3, "0.28.0", vec!["--experimental-acp".into()])),
        _ => Err("ACP route accepts only OpenCode or Gemini".into()),
    }
}
fn worker(
    workspace: Workspace,
    incoming: Receiver<Request>,
    events: Events,
    stop: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    cancelled: Arc<AtomicU64>,
) {
    let mut generation = 0u64;
    let mut reviewed: Option<Reviewed> = None;
    let mut preview_owner: Option<(u64, i64)> = None;
    let mut sessions: HashMap<(Agent, String), i64> = HashMap::new();
    while !stop.load(Ordering::Acquire) {
        let request = match incoming.recv_timeout(Duration::from_millis(50)) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        match request {
            Request::Preview(input) => {
                generation = generation.saturating_add(1);
                reviewed = None;
                let task_id = input.task.id;
                preview_owner = Some((generation, task_id));
                let result = (|| {
                    let (index, _, launch) = profile(input.agent)?;
                    let trust = crate::agent::external_trust(index, &workspace.project, launch)?;
                    let payload = payload(&workspace, &input)?;
                    let mut lines = trust.lines.clone();
                    lines.extend([
                        "Private owned ACPv1 stdio process; initialize and session authentication happen only after Run consent.".into(),
                        "Client filesystem and terminal callbacks are NOT supported or advertised; every such request is refused.".into(),
                        "Agent-owned tools, project instructions, MCP, configuration, hooks and history are EXTERNAL PROVIDER POLICY. ACP is not a path/network sandbox or a read-only guarantee.".into(),
                        "Only exact reject-once/allow-once provider permission IDs are offered; persistent grants are filtered. Unknown questions/elicitation extensions are refused, not advertised.".into(),
                        "No authenticate/login action is invoked. Provider-owned auth/account/product eligibility must succeed at session creation; installed is not authenticated.".into(),
                        "Resume is only sent if session/load is actually negotiated; replayed history is suppressed. No standardized token usage is negotiated by these pinned interfaces.".into(),
                    ]);
                    if input.agent == Agent::OpenCode {
                        lines.push(format!("OpenCode {} acp --pure disables external plugins; built-in tools/config/MCP/project rules remain provider-owned.", crate::agent::OPENCODE_RELEASE));
                    } else {
                        lines.push("Gemini CLI 0.28.0 uses --experimental-acp (not newer releases' --acp). Its configured Google OAuth/API/Vertex route is provider-owned; no free-account eligibility claim.".into());
                    }
                    if input.inspect_sessions {
                        lines.push("CAPABILITY INSPECTION ONLY: initialize, then session/list only if negotiated. No session/new/load/prompt, model request, imported handoff or grant. Listed metadata does not authorize resuming foreign sessions.".into());
                    }
                    let mut blockers = Vec::new();
                    if input.operation == Operation::Implement && !input.inspect_sessions {
                        blockers.push("Controlled implementation refused: this ACP peer's agent-owned tools cannot enforce Eevee's selected-file boundary. Use the explicitly external-policy manual handoff instead; no sandbox claim.".into());
                    }
                    if let Some(id) = &input.resume
                        && (id.is_empty() || id.len() > 256 || safe(id) != *id)
                    {
                        blockers.push("Resume refused: invalid ACP session identity".into());
                    }
                    if sessions.len() >= 256 && input.resume.is_none() {
                        blockers.push(
                            "ACP session bound reached; restart before another session".into(),
                        );
                    }
                    if let Some(id) = &input.resume
                        && sessions.get(&(input.agent, id.clone())) != Some(&task_id)
                    {
                        blockers.push("Resume refused: session is not an observed session of this provider and task; no cross-provider session transfer.".into());
                    }
                    let preview = Preview {
                        generation,
                        task_id,
                        thread_id: input.resume.clone(),
                        trust: lines,
                        payload: payload.clone(),
                        blockers: blockers.clone(),
                    };
                    if blockers.is_empty() {
                        reviewed = Some(Reviewed {
                            generation,
                            input,
                            trust,
                            payload,
                        });
                    }
                    emit(&events, Event::Preview(preview))
                })();
                if let Err(message) = result {
                    let _ = emit(
                        &events,
                        Event::Error {
                            generation: None,
                            message: safe(&message),
                        },
                    );
                }
                busy.store(false, Ordering::Release);
            }
            Request::Run {
                generation: requested,
            } => {
                if reviewed
                    .as_ref()
                    .is_some_and(|run| run.generation == requested)
                {
                    let run = reviewed.take().expect("matched reviewed generation");
                    let task_id = run.input.task.id;
                    preview_owner = None;
                    let (outcome, message) = transport(
                        &workspace,
                        &run,
                        &incoming,
                        &events,
                        &stop,
                        &cancelled,
                        &mut sessions,
                    );
                    finish(&events, requested, task_id, outcome, &message);
                } else {
                    finish(
                        &events,
                        requested,
                        preview_owner
                            .filter(|(owner, _)| *owner == requested)
                            .map_or(0, |(_, task_id)| task_id),
                        Outcome::Failed,
                        "Run refused: no matching valid ACP preview; review again",
                    );
                }
                busy.store(false, Ordering::Release);
            }
            Request::Cancel {
                generation: requested,
            } => {
                if preview_owner.is_some_and(|(owner, _)| owner == requested) {
                    let (_, task_id) = preview_owner.take().expect("matched preview");
                    reviewed = None;
                    finish(
                        &events,
                        requested,
                        task_id,
                        Outcome::Cancelled,
                        "Preview discarded; no provider launched",
                    );
                }
                busy.store(false, Ordering::Release);
            }
            Request::Answer { generation, .. } => {
                let _ = emit(
                    &events,
                    Event::Error {
                        generation: Some(generation),
                        message: "Stale ACP answer rejected: no pending request".into(),
                    },
                );
            }
        }
    }
    busy.store(false, Ordering::Release);
}
fn payload(workspace: &Workspace, input: &Input) -> Result<String, String> {
    input.validate_review()?;
    if std::path::Path::new(&input.task.project) != workspace.project.as_path()
        || !matches!(input.agent, Agent::OpenCode | Agent::Gemini)
        || (input.operation == Operation::Plan && !input.edit_paths.is_empty())
    {
        return Err("ACP task/project/profile or planning write scope mismatch".into());
    }
    crate::store::validate_planning_input(&workspace.data_dir, &input.task, &input.excerpts)?;
    if input.inspect_sessions {
        if input.resume.is_some() {
            return Err("Session-list inspection must not resume a provider session".into());
        }
        return serde_json::to_string_pretty(&json!({
            "initialize":{"protocolVersion":1,"clientCapabilities":client_capabilities()},
            "optional_method":"session/list",
            "params":{"cwd":workspace.project},
            "dispatch_condition":"only if agentCapabilities.sessionCapabilities.list was negotiated",
            "model_prompts_sent":0
        })).map_err(|_| "Could not encode ACP inspection".into());
    }
    if let Some(document) = &input.handoff {
        crate::handoff::validate_dispatch(
            document,
            &input.task,
            &input.excerpts,
            input.report.as_ref(),
            &workspace.exclusions,
        )?;
    }
    let sources = crate::context::planning_payload(
        input.task.id,
        input.report.as_ref(),
        &input.excerpts,
        &workspace.exclusions,
    )?;
    let text = serde_json::to_string_pretty(&json!({
        "purpose":"Plan the user's task. Do not implement or write files. Ask for decisions and report proposals as unverified planning. This instruction is not an enforced sandbox.",
        "task":{"id":input.task.id,"revision":input.task.revision,"goal":input.task.goal,"acceptance_criteria":input.task.criteria,"next_action":crate::handoff::without_local_provenance(&input.task.next_action)},
        "approved_memories":input.task.approved_memories,
        "selected_sources":serde_json::from_str::<Value>(&sources.text).map_err(|_| "Invalid selected source JSON")?,
        "accepted_handoff":input.handoff
    })).map_err(|_| "Could not encode ACP payload")?;
    if text.len() > FRAME_LIMIT / 2 {
        return Err("ACP payload exceeds 1 MiB; reduce the reviewed sources".into());
    }
    if safe(&text) != text {
        return Err("Payload contains credential-like text or terminal/directional controls; remove before review".into());
    }
    Ok(text)
}
fn revalidate(workspace: &Workspace, run: &Reviewed) -> Result<(), String> {
    let (index, _, _) = profile(run.input.agent)?;
    crate::agent::revalidate_external_trust(index, &workspace.project, &run.trust)?;
    if payload(workspace, &run.input)? != run.payload {
        return Err("Task/source/handoff changed after preview; no substituted prompt sent".into());
    }
    Ok(())
}
fn emit(events: &Events, event: Event) -> Result<(), String> {
    let mut queue = events.lock();
    if let Event::Item { generation, item_id, .. } = &event
        && let Some(previous) = queue.iter_mut().find(|entry| matches!(entry, Event::Item { generation:g,item_id:id,.. } if g==generation && id==item_id)) {
        *previous = event; return Ok(());
    }
    let limit = if matches!(
        event,
        Event::Item { .. } | Event::Status { .. } | Event::Journal { .. }
    ) {
        EVENT_LIMIT - 24
    } else {
        EVENT_LIMIT - 1
    };
    if queue.len() >= limit {
        return Err(
            "ACP UI queue exhausted; stopping without dropping a pending permission".into(),
        );
    }
    queue.push_back(event);
    Ok(())
}
fn finish(events: &Events, generation: u64, task_id: i64, outcome: Outcome, message: &str) {
    let mut queue = events.lock();
    if queue.len() < EVENT_LIMIT {
        queue.push_back(Event::Finished {
            generation,
            task_id,
            outcome,
            message: safe(message),
        });
    }
}

/// Optional methods must be supported by negotiated capability data, never profile assumptions.
pub fn supports_method(capabilities: &Value, method: &str) -> bool {
    match method {
        "session/new" | "session/prompt" | "session/cancel" => true,
        "session/load" => capabilities.get("loadSession").and_then(Value::as_bool) == Some(true),
        "session/list" => capabilities
            .pointer("/sessionCapabilities/list")
            .is_some_and(Value::is_object),
        "session/fork" => capabilities
            .pointer("/sessionCapabilities/fork")
            .is_some_and(Value::is_object),
        "session/resume" => capabilities
            .pointer("/sessionCapabilities/resume")
            .is_some_and(Value::is_object),
        _ => false,
    }
}
fn negotiated(result: &Value) -> Result<Value, String> {
    if result.get("protocolVersion").and_then(Value::as_u64) != Some(1) {
        return Err("ACP protocol mismatch: only negotiated version 1 is supported".into());
    }
    let capabilities = result
        .get("agentCapabilities")
        .cloned()
        .unwrap_or(json!({}));
    if !capabilities.is_object() {
        return Err("Malformed ACP agentCapabilities".into());
    }
    // Unknown extensions are accepted, but not echoed as trusted capability or credential-bearing metadata.
    let mut normalized = json!({});
    for key in ["loadSession"] {
        if let Some(value) = capabilities.get(key).and_then(Value::as_bool) {
            normalized[key] = json!(value);
        }
    }
    for (group, keys) in [
        (
            "promptCapabilities",
            &["image", "audio", "embeddedContext"][..],
        ),
        ("mcpCapabilities", &["http", "sse"][..]),
    ] {
        for key in keys {
            if let Some(value) = capabilities
                .get(group)
                .and_then(|group| group.get(*key))
                .and_then(Value::as_bool)
            {
                if normalized.get(group).is_none() {
                    normalized[group] = json!({});
                }
                normalized[group][*key] = json!(value);
            }
        }
    }
    for key in ["list", "fork", "resume", "close"] {
        if capabilities
            .get("sessionCapabilities")
            .and_then(|group| group.get(key))
            .is_some_and(Value::is_object)
        {
            if normalized.get("sessionCapabilities").is_none() {
                normalized["sessionCapabilities"] = json!({});
            }
            normalized["sessionCapabilities"][key] = json!({});
        }
    }
    Ok(normalized)
}
fn session_list_report(
    capabilities: &Value,
    project: &std::path::Path,
    result: &Value,
) -> Result<String, String> {
    let sessions = result
        .get("sessions")
        .and_then(Value::as_array)
        .ok_or("ACP session/list response lacks sessions array")?;
    if sessions.len() > 256 {
        return Err("ACP session/list metadata exceeds the 256-entry inspection bound".into());
    }
    let cwd = project.to_str().ok_or("ACP cwd is not UTF-8")?;
    let mut text = format!(
        "PEER CAPABILITY INSPECTION / no model request\nSource: ACP1 initialize + negotiated session/list\nNegotiated capabilities: {capabilities}\nProject: {cwd}\n\nProvider-owned session metadata for this exact project (first page, at most 64 displayed):\n"
    );
    let mut shown = 0;
    for session in sessions {
        if session.get("cwd").and_then(Value::as_str) != Some(cwd) {
            continue;
        }
        if shown == 64 {
            text.push_str("Display bound reached; remaining entries withheld.\n");
            break;
        }
        let id = session
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 512)
            .ok_or("ACP session/list entry lacks bounded sessionId")?;
        let title = session
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("(no title)");
        let _ = writeln!(text, "{} / {}", safe(id), safe(bounded(title, 256)));
        shown += 1;
    }
    let _ = writeln!(
        text,
        "\nDisplayed {shown} project sessions. External metadata is unverified; not a resume grant. No session/new/load/prompt, model request, permission grant or execution evidence."
    );
    Ok(text)
}
fn client_capabilities() -> Value {
    json!({"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false})
}
fn permission_options(params: &Value) -> Result<Vec<PermissionOption>, String> {
    let mut options = vec![PermissionOption {
        label: "Deny / cancel".into(),
        description: "Refuse without saving a persistent rule".into(),
        answer: json!({"outcome":{"outcome":"cancelled"}}),
    }];
    let values = params
        .get("options")
        .and_then(Value::as_array)
        .ok_or("Malformed ACP permission options")?;
    if values.len() > 32 {
        return Err("Too many ACP permission options".into());
    }
    let mut ids = HashSet::new();
    let mut rejects = Vec::new();
    let mut allows = Vec::new();
    for option in values {
        let id = identity(option, "optionId")?;
        if !ids.insert(id.clone()) {
            return Err("Duplicate ACP permission option ID".into());
        }
        let kind = option
            .get("kind")
            .and_then(Value::as_str)
            .ok_or("Malformed ACP permission kind")?;
        if !matches!(kind, "allow_once" | "reject_once") {
            continue;
        }
        let entry = PermissionOption {
            label: bounded(
                &safe(option.get("name").and_then(Value::as_str).unwrap_or(kind)),
                1024,
            )
            .to_owned(),
            description: if kind == "allow_once" {
                "Provider policy approval for this operation only; not file/network confinement"
            } else {
                "Reject this operation only; no saved rule"
            }
            .into(),
            answer: json!({"outcome":{"outcome":"selected","optionId":id}}),
        };
        if kind == "reject_once" {
            rejects.push(entry);
        } else {
            allows.push(entry);
        }
    }
    if !rejects.is_empty() {
        options = rejects;
    }
    options.extend(allows);
    Ok(options)
}
#[derive(Default)]
struct Permissions {
    pending: Vec<(Value, Vec<Value>)>,
    seen: HashSet<String>,
}
impl Permissions {
    fn insert(&mut self, id: &Value, options: &[PermissionOption]) -> Result<(), String> {
        if self.pending.len() >= 16 || self.seen.len() >= 4096 {
            return Err("ACP permission request limit exceeded".into());
        }
        if !self.seen.insert(id.to_string()) {
            return Err("Repeated ACP request ID refused".into());
        }
        self.pending.push((
            id.clone(),
            options.iter().map(|o| o.answer.clone()).collect(),
        ));
        Ok(())
    }
    fn take_answer(&mut self, id: &Value, answer: &Value) -> bool {
        let Some(index) = self
            .pending
            .iter()
            .position(|(pending, options)| pending == id && options.contains(answer))
        else {
            return false;
        };
        self.pending.remove(index);
        true
    }
    fn invalidate(&mut self) -> Vec<Value> {
        self.pending.drain(..).map(|(id, _)| id).collect()
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Call {
    Initialize,
    Session,
    Sessions,
    Prompt,
}
struct PendingCall {
    id: u64,
    kind: Call,
    deadline: Option<Instant>,
}
struct Item {
    kind: ItemKind,
    text: String,
    complete: bool,
    fields: serde_json::Map<String, Value>,
}
impl Item {
    fn new(kind: ItemKind) -> Self {
        Self {
            kind,
            text: String::new(),
            complete: false,
            fields: serde_json::Map::new(),
        }
    }
    fn tool_snapshot(&mut self, update: &Value) -> Result<(), String> {
        if self.complete
            && update
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|status| !matches!(status, "completed" | "failed"))
        {
            return Err("ACP terminal tool identity cannot return to an in-progress state".into());
        }
        // ACP tool updates patch named fields. Content is a snapshot, not a delta or usage total.
        for key in [
            "title",
            "kind",
            "status",
            "content",
            "locations",
            "rawInput",
            "rawOutput",
        ] {
            if let Some(value) = update.get(key) {
                self.fields.insert(key.into(), value.clone());
            }
        }
        let text = safe(
            &serde_json::to_string_pretty(&self.fields).map_err(|_| "ACP tool encoding failed")?,
        );
        if text.len() > 32 * 1024 {
            return Err("ACP tool snapshot exceeds the 32 KiB journal bound; stopping with the prior bounded observation only".into());
        }
        self.text = text;
        if let Some(status) = update.get("status").and_then(Value::as_str) {
            self.complete = matches!(status, "completed" | "failed");
        }
        Ok(())
    }
}
struct Engine<'a> {
    process: crate::runner::native::Process,
    workspace: &'a Workspace,
    run: &'a Reviewed,
    events: &'a Events,
    stop: &'a AtomicBool,
    cancelled: &'a AtomicU64,
    wire: VecDeque<Vec<u8>>,
    wire_offset: usize,
    write_started: Option<Instant>,
    framer: Framer,
    sequence: u64,
    calls: Vec<PendingCall>,
    capabilities: Value,
    sessions: &'a mut HashMap<(Agent, String), i64>,
    session: Option<String>,
    dispatched: bool,
    cancelling: Option<Instant>,
    permissions: Permissions,
    items: HashMap<String, Item>,
    message_id: Option<String>,
    item_sequence: u64,
    received: usize,
    terminal: Option<(Outcome, String)>,
    model: String,
    journal_nonce: u128,
}
fn transport(
    workspace: &Workspace,
    run: &Reviewed,
    incoming: &Receiver<Request>,
    events: &Events,
    stop: &AtomicBool,
    cancelled: &AtomicU64,
    sessions: &mut HashMap<(Agent, String), i64>,
) -> (Outcome, String) {
    let setup = (|| {
        revalidate(workspace, run)?;
        if stop.load(Ordering::Acquire) || cancelled.load(Ordering::Acquire) == run.generation {
            return Err("Cancelled before ACP launch".into());
        }
        let (index, expected, _) = profile(run.input.agent)?;
        let prepared =
            crate::agent::qualify_external(index, &workspace.project, &run.trust, expected, stop)?;
        if stop.load(Ordering::Acquire) || cancelled.load(Ordering::Acquire) == run.generation {
            return Err("Cancelled before ACP launch".into());
        }
        crate::runner::native::Process::spawn_piped(&prepared)
            .map_err(|_| "Trusted ACP peer failed to launch; OS details withheld".to_owned())
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
        workspace,
        run,
        events,
        stop,
        cancelled,
        wire: VecDeque::new(),
        wire_offset: 0,
        write_started: None,
        framer: Framer::default(),
        sequence: 0,
        calls: Vec::new(),
        capabilities: json!({}),
        sessions,
        session: None,
        dispatched: false,
        cancelling: None,
        permissions: Permissions::default(),
        items: HashMap::new(),
        message_id: None,
        item_sequence: 0,
        received: 0,
        terminal: None,
        model: "Provider default (not yet observed)".into(),
        journal_nonce: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    };
    let result: Result<(Outcome, String), String> = (|| {
        engine.call(Call::Initialize,"initialize",json!({"protocolVersion":1,"clientCapabilities":client_capabilities(),"clientInfo":{"name":"eevee","version":env!("CARGO_PKG_VERSION")}}))?;
        loop {
            if engine.stop.load(Ordering::Acquire) {
                return Err("ACP client shutdown".into());
            }
            if engine.cancelled.load(Ordering::Acquire) == run.generation {
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
                    Ok(_) => engine.status("Stale/overlapping ACP request refused; no replay")?,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(_) => return Err("ACP request owner disconnected".into()),
                }
            }
            engine.flush()?;
            let mut buffer = [0u8; 8192];
            for _ in 0..8 {
                match engine.process.read(0, &mut buffer) {
                    Ok(0) => {
                        if engine.terminal.is_some() {
                            break;
                        }
                        return Err(if engine.framer.line.is_empty() {
                            "ACP stdout closed without authoritative prompt completion"
                        } else {
                            "ACP stdout closed inside a JSONL frame"
                        }
                        .into());
                    }
                    Ok(count) => {
                        engine.received += count;
                        if engine.received > 32 * 1024 * 1024 {
                            return Err("ACP transport byte limit exceeded".into());
                        }
                        for chunk in buffer[..count].split_inclusive(|byte| *byte == b'\n') {
                            for frame in engine.framer.push(chunk)? {
                                engine.frame(frame)?;
                            }
                            if engine.terminal.is_some() {
                                break;
                            }
                        }
                        if engine.terminal.is_some() {
                            break;
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
                    Err(_) => return Err("ACP stdout transport failed".into()),
                }
            }
            // Provider stderr is drained but never copied into snapshots/logs: it may contain credentials.
            for _ in 0..8 {
                match engine.process.read(1, &mut buffer) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) =>
                    {
                        break;
                    }
                    Err(_) => return Err("ACP stderr capture failed".into()),
                }
            }
            if let Some(result) = engine.terminal.take() {
                return Ok(result);
            }
            if engine.calls.iter().any(|call| {
                call.deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
            }) {
                return Err(
                    "ACP initialization/session deadline expired; no automatic retry".into(),
                );
            }
            if engine
                .cancelling
                .is_some_and(|start| start.elapsed() > Duration::from_secs(3))
            {
                return Err(
                    "ACP cancellation deadline expired without authoritative completion".into(),
                );
            }
            if engine
                .process
                .try_wait()
                .map_err(|_| "ACP process status unavailable")?
                .is_some()
            {
                return Err("ACP peer exited without authoritative completion".into());
            }
            thread::sleep(Duration::from_millis(5));
        }
    })();
    let result = match engine.journal_tools() {
        Ok(()) => result,
        Err(error) => Err(error),
    };
    let observed_session = engine.session.clone();
    if let Some(session) = observed_session {
        engine
            .sessions
            .entry((run.input.agent, session))
            .or_insert(run.input.task.id);
    }
    let _ = engine.close_permissions();
    let cleanup = engine.teardown();
    match result {
        Ok((outcome, message)) => (
            if cleanup { outcome } else { Outcome::Unknown },
            if cleanup {
                message
            } else {
                format!("{message}; process-tree cleanup could not be established")
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
                "{error}. {}; no retry/resend.{}",
                if engine.dispatched {
                    "Provider may have acted; turn outcome unknown"
                } else {
                    "No model prompt dispatched"
                },
                if cleanup {
                    ""
                } else {
                    " Owned process-tree cleanup could not be established."
                }
            ),
        ),
    }
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
    fn send(&mut self, value: Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&value).map_err(|_| "ACP encoding failed")?;
        if bytes.len() > FRAME_LIMIT || self.wire.len() >= 64 {
            return Err("ACP output frame/queue limit exceeded".into());
        }
        bytes.push(b'\n');
        self.wire.push_back(bytes);
        Ok(())
    }
    fn call(&mut self, kind: Call, method: &str, params: Value) -> Result<(), String> {
        self.sequence += 1;
        let id = self.sequence;
        self.calls.push(PendingCall {
            id,
            kind,
            deadline: if kind == Call::Prompt {
                None
            } else {
                Some(Instant::now() + Duration::from_secs(45))
            },
        });
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
    }
    fn flush(&mut self) -> Result<(), String> {
        loop {
            if self.cancelling.is_none()
                && self.cancelled.load(Ordering::Acquire) == self.run.generation
            {
                self.cancel()?;
            }
            let Some(bytes) = self.wire.front() else {
                break;
            };
            self.write_started.get_or_insert_with(Instant::now);
            match self.process.write_stdin(&bytes[self.wire_offset..]) {
                Ok(0) => return Err("ACP stdin closed".into()),
                Ok(count) => {
                    self.wire_offset += count;
                    if self.wire_offset == bytes.len() {
                        self.wire.pop_front();
                        self.wire_offset = 0;
                        self.write_started = None;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => return Err("ACP stdin transport failed".into()),
            }
        }
        if self
            .write_started
            .is_some_and(|start| start.elapsed() > Duration::from_secs(5))
        {
            return Err("ACP stdin write deadline expired".into());
        }
        Ok(())
    }
    fn frame(&mut self, frame: Value) -> Result<(), String> {
        if frame.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err("Malformed ACP JSON-RPC envelope".into());
        }
        if let Some(method) = frame.get("method").and_then(Value::as_str) {
            let params = frame.get("params").cloned().unwrap_or(json!({}));
            if let Some(id) = frame.get("id") {
                if !valid_rpc_id(id) {
                    return Err("Invalid ACP request identity".into());
                }
                if method == "session/request_permission" {
                    return self.permission(id.clone(), params);
                }
                // No client callbacks/extensions have been advertised. Never execute provider-supplied commands/paths.
                self.status(&format!(
                    "Refused unsupported ACP client method: {}",
                    safe(method)
                ))?;
                return self.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Eevee does not implement or advertise this client callback; no filesystem/terminal/elicitation action performed"}}));
            }
            if method == "session/update" {
                self.update(&params)?;
            }
            return Ok(()); // Unknown optional notifications are safely ignored.
        }
        let id = frame
            .get("id")
            .and_then(Value::as_u64)
            .ok_or("ACP response lacks valid correlation ID")?;
        let Some(index) = self.calls.iter().position(|call| call.id == id) else {
            self.status("Late/unknown ACP response ignored; no session or approval replay")?;
            return Ok(());
        };
        let call = self.calls.remove(index);
        if let Some(error) = frame.get("error") {
            let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
            let message = safe(
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Provider request failed"),
            );
            return Err(format!(
                "ACP provider error {code}: {}",
                bounded(&message, 2048)
            ));
        }
        let result = frame
            .get("result")
            .ok_or("ACP response lacks result/error")?;
        match call.kind {
            Call::Initialize => {
                self.capabilities = negotiated(result)?;
                if let Some(version) = result.pointer("/agentInfo/version").and_then(Value::as_str)
                    && version != profile(self.run.input.agent)?.1
                {
                    return Err(
                        "ACP peer-reported version differs from qualified executable".into(),
                    );
                }
                let auth = result
                    .get("authMethods")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|v| v.get("id").and_then(Value::as_str))
                            .map(safe)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_else(|| "not advertised".into());
                self.status(&format!("Negotiated ACP1; capabilities {}; auth routes [{auth}] (availability not authentication); client fs/terminal disabled; question/elicitation unsupported; usage coverage unavailable",self.capabilities))?;
                if self.cancelling.is_some() {
                    self.terminal = Some((
                        Outcome::Cancelled,
                        "Cancelled before session creation".into(),
                    ));
                    return Ok(());
                }
                revalidate(self.workspace, self.run)?;
                let cwd = self
                    .workspace
                    .project
                    .to_str()
                    .ok_or("ACP cwd is not UTF-8")?;
                if self.run.input.inspect_sessions {
                    let supported = supports_method(&self.capabilities, "session/list");
                    emit(
                        self.events,
                        Event::Inspection {
                            generation: self.run.generation,
                            task_id: self.run.input.task.id,
                            text: format!(
                                "PEER CAPABILITY INSPECTION / no model request\nSource: {} ACP1 initialize\nNegotiated capabilities: {}\n{}\nClient fs/terminal disabled. No session/new/load/prompt; listed sessions never authorize resume.",
                                self.run.input.agent.label(),
                                self.capabilities,
                                if supported {
                                    "session/list negotiated; invoking only for the reviewed project."
                                } else {
                                    "session/list unavailable: peer did not negotiate sessionCapabilities.list. Method NOT invoked."
                                }
                            ),
                        },
                    )?;
                    if supported {
                        self.call(Call::Sessions, "session/list", json!({"cwd":cwd}))?;
                    } else {
                        self.terminal = Some((Outcome::Failed, "session/list unavailable: peer did not negotiate the method; NOT invoked; no model prompt".into()));
                    }
                    return Ok(());
                }
                if let Some(id) = &self.run.input.resume {
                    if !supports_method(&self.capabilities, "session/load") {
                        return Err("Resume refused: peer did not negotiate session/load; method not invoked".into());
                    }
                    self.session = Some(id.clone());
                    self.call(
                        Call::Session,
                        "session/load",
                        json!({"sessionId":id,"cwd":cwd,"mcpServers":[]}),
                    )?;
                } else {
                    self.call(
                        Call::Session,
                        "session/new",
                        json!({"cwd":cwd,"mcpServers":[]}),
                    )?;
                }
            }
            Call::Sessions => {
                let text =
                    session_list_report(&self.capabilities, &self.workspace.project, result)?;
                emit(
                    self.events,
                    Event::Inspection {
                        generation: self.run.generation,
                        task_id: self.run.input.task.id,
                        text,
                    },
                )?;
                self.terminal = Some((Outcome::Completed, "Negotiated session/list observed; metadata only, no model prompt or resume grant".into()));
            }
            Call::Session => {
                if self.run.input.resume.is_none() {
                    let id = identity(result, "sessionId")?;
                    if self
                        .sessions
                        .contains_key(&(self.run.input.agent, id.clone()))
                    {
                        return Err("ACP new session reused an existing session identity; no ownership transfer".into());
                    }
                    self.session = Some(id);
                }
                if self.cancelling.is_some() {
                    self.terminal = Some((
                        Outcome::Cancelled,
                        "Cancelled before prompt dispatch".into(),
                    ));
                    return Ok(());
                }
                if let Some(options) = result.get("configOptions").and_then(Value::as_array) {
                    for option in options {
                        if option.get("category").and_then(Value::as_str) == Some("model") {
                            self.model = safe(
                                option
                                    .get("currentValue")
                                    .and_then(Value::as_str)
                                    .unwrap_or("Provider default"),
                            );
                        }
                    }
                }
                let session = self
                    .session
                    .clone()
                    .ok_or("ACP session creation lacks identity")?;
                emit(self.events,Event::Thread { generation:self.run.generation,task_id:self.run.input.task.id,thread_id:session.clone(),model:self.model.clone(),account:"Provider-owned authentication accepted session; billing/eligibility not independently verified".into() })?;
                self.dispatch(&session)?;
            }
            Call::Prompt => {
                let reason = result
                    .get("stopReason")
                    .and_then(Value::as_str)
                    .ok_or("ACP prompt response lacks stopReason")?;
                let outcome = prompt_outcome(reason);
                self.close_permissions()?;
                let ids = self.items.keys().cloned().collect::<Vec<_>>();
                for id in ids {
                    if let Some(item) = self.items.get_mut(&id)
                        && item.kind != ItemKind::Tool
                        && outcome == Outcome::Completed
                    {
                        item.complete = true;
                    }
                    self.emit_item(&id)?;
                }
                self.terminal = Some((
                    outcome,
                    format!(
                        "ACP prompt stopped: {}; output is provider-origin, not verified local execution; token usage unavailable",
                        safe(reason)
                    ),
                ));
            }
        }
        Ok(())
    }
    fn dispatch(&mut self, session: &str) -> Result<(), String> {
        revalidate(self.workspace, self.run)?;
        if self.cancelled.load(Ordering::Acquire) == self.run.generation {
            self.cancel()?;
            return Ok(());
        }
        self.call(
            Call::Prompt,
            "session/prompt",
            json!({"sessionId":session,"prompt":[{"type":"text","text":self.run.payload}]}),
        )?;
        self.dispatched = true;
        self.status("Literal reviewed payload dispatched once; external provider policy, not Eevee confinement")
    }
    fn turn(&self) -> String {
        format!("acp-turn-{}", self.run.generation)
    }
    fn permission(&mut self, id: Value, params: Value) -> Result<(), String> {
        if self.terminal.is_some()
            || self.cancelling.is_some()
            || self.cancelled.load(Ordering::Acquire) == self.run.generation
            || !self.dispatched
        {
            return self.send(
                json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}}),
            );
        }
        let session = self
            .session
            .clone()
            .ok_or("ACP permission before session")?;
        if params.get("sessionId").and_then(Value::as_str) != Some(session.as_str()) {
            return Err("ACP permission has wrong session identity".into());
        }
        if !self.permissions.pending.is_empty() {
            if self.permissions.seen.len() >= 4096 || !self.permissions.seen.insert(id.to_string())
            {
                return Err("Repeated/excessive ACP permission request identity".into());
            }
            return self.send(
                json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}}),
            );
        }
        let options = permission_options(&params)?;
        self.permissions.insert(&id, &options)?;
        let tool = params.get("toolCall").unwrap_or(&Value::Null);
        let item = if tool.get("toolCallId").is_some() {
            format!("acp-tool-{}", identity(tool, "toolCallId")?)
        } else {
            format!("acp-permission-{id}")
        };
        let details =
            serde_json::to_string(tool).map_err(|_| "ACP tool description encoding failed")?;
        if details.len() > 16000 || safe(&details) != details {
            return Err("ACP permission details are oversized or require redaction; no partially visible grant offered".into());
        }
        let description = format!("External provider policy approval (not sandbox): {details}");
        emit(
            self.events,
            Event::Prompt {
                generation: self.run.generation,
                task_id: self.run.input.task.id,
                request: Prompt {
                    request_id: id,
                    thread_id: session,
                    turn_id: self.turn(),
                    item_id: item,
                    kind: PromptKind::Permission {
                        description: bounded(&description, 16384).to_owned(),
                        options,
                    },
                },
            },
        )
    }
    fn answer(&mut self, generation: u64, id: Value, answer: Value) -> Result<(), String> {
        if generation != self.run.generation
            || self.cancelling.is_some()
            || self.cancelled.load(Ordering::Acquire) == self.run.generation
            || !self.permissions.take_answer(&id, &answer)
        {
            return emit(
                self.events,
                Event::Error {
                    generation: Some(generation),
                    message: "Stale/invalid ACP answer rejected; no permission sent or persisted"
                        .into(),
                },
            );
        }
        revalidate(self.workspace, self.run)?;
        // Check again immediately before queueing an allow; cancellation closes every pending request.
        if self.cancelled.load(Ordering::Acquire) == self.run.generation {
            self.send(
                json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}}),
            )?;
        } else {
            self.send(json!({"jsonrpc":"2.0","id":id,"result":answer}))?;
        }
        emit(
            self.events,
            Event::PromptClosed {
                generation,
                request_id: id,
            },
        )
    }
    fn close_permissions(&mut self) -> Result<(), String> {
        for id in self.permissions.invalidate() {
            self.send(
                json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}}),
            )?;
            emit(
                self.events,
                Event::PromptClosed {
                    generation: self.run.generation,
                    request_id: id,
                },
            )?;
        }
        Ok(())
    }
    fn cancel(&mut self) -> Result<(), String> {
        if self.cancelling.is_some() {
            return Ok(());
        }
        self.cancelling = Some(Instant::now());
        // Withdraw unsent approvals/prompts. A partially written frame must finish or the transport is invalid.
        if self.wire_offset != 0 {
            return Err("Cancelled during partial ACP frame write; transport terminated, outcome not guessed".into());
        }
        self.wire.clear();
        self.write_started = None;
        self.close_permissions()?;
        if let Some(session) = &self.session {
            self.send(
                json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}}),
            )?;
        }
        if !self.dispatched {
            self.terminal = Some((
                Outcome::Cancelled,
                "Cancelled before model prompt dispatch".into(),
            ));
        }
        self.status("ACP cancellation invalidated pending approvals; late updates/replies cannot grant permission")
    }
    fn update(&mut self, params: &Value) -> Result<(), String> {
        // session/load replays historical updates; never duplicate them as new-turn artifacts.
        if self.terminal.is_some() || !self.dispatched
        // Cancellation withdraws grants, not observations of already-started tools.
        {
            return Ok(());
        }
        if params.get("sessionId").and_then(Value::as_str) != self.session.as_deref() {
            return Err("ACP update has wrong session identity".into());
        }
        let update = params.get("update").ok_or("ACP update lacks content")?;
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("agent_message_chunk") => {
                if update.pointer("/content/type").and_then(Value::as_str) != Some("text") {
                    return self.status("Unsupported non-text ACP response content withheld");
                }
                let text = update
                    .pointer("/content/text")
                    .and_then(Value::as_str)
                    .ok_or("ACP message chunk lacks text")?;
                let id = if let Some(id) = &self.message_id {
                    id.clone()
                } else {
                    self.item_sequence += 1;
                    let id = format!("acp-message-{}-{}", self.run.generation, self.item_sequence);
                    self.message_id = Some(id.clone());
                    id
                };
                let item = self
                    .items
                    .entry(id.clone())
                    .or_insert_with(|| Item::new(ItemKind::Message));
                item.text.push_str(text);
                self.check_items()?;
                self.emit_item(&id)?;
            }
            Some("tool_call" | "tool_call_update") => {
                self.message_id = None;
                let native_id = identity(update, "toolCallId")?;
                let id = format!("acp-tool-{native_id}");
                let item = self
                    .items
                    .entry(id.clone())
                    .or_insert_with(|| Item::new(ItemKind::Tool));
                item.tool_snapshot(update)?;
                self.check_items()?;
                self.emit_item(&id)?;
            }
            Some("plan") => {
                self.message_id = None;
                let id = format!("acp-plan-{}", self.run.generation);
                let text = safe(
                    &serde_json::to_string_pretty(update.get("entries").unwrap_or(&Value::Null))
                        .map_err(|_| "ACP plan encoding failed")?,
                );
                let mut item = Item::new(ItemKind::Plan);
                item.text = text;
                self.items.insert(id.clone(), item);
                self.check_items()?;
                self.emit_item(&id)?;
            }
            Some("agent_thought_chunk" | "user_message_chunk") => {} // Never show or journal private reasoning/user replay.
            Some("current_mode_update") => {
                self.status("Provider session mode changed under external policy; no client sandbox guarantee")?;
            }
            _ => {} // Unknown optional fields/updates do not invent capabilities or totals.
        }
        Ok(())
    }
    fn check_items(&self) -> Result<(), String> {
        if self.items.len() > ITEM_LIMIT
            || self
                .items
                .values()
                .map(|item| item.text.len())
                .sum::<usize>()
                > OUTPUT_LIMIT
        {
            return Err("ACP normalized output limit exceeded".into());
        }
        Ok(())
    }
    fn emit_item(&self, id: &str) -> Result<(), String> {
        let item = self.items.get(id).ok_or("ACP item state missing")?;
        emit(
            self.events,
            Event::Item {
                generation: self.run.generation,
                task_id: self.run.input.task.id,
                thread_id: self.session.clone().ok_or("ACP item before session")?,
                turn_id: self.turn(),
                item_id: id.into(),
                kind: item.kind,
                text: safe(if item.kind == ItemKind::Message && !item.complete {
                    &item.text[..item.text.rfind('\n').map_or(0, |end| end + 1)]
                } else {
                    &item.text
                }),
                complete: item.complete,
            },
        )
    }
    fn journal_tools(&self) -> Result<(), String> {
        // Store records are append-only. Publish once at transport finalization, never
        // successive mutable snapshots under the same ID. Live Item events remain updates.
        for (item_id, item) in &self.items {
            if item.kind != ItemKind::Tool {
                continue;
            }
            let native_id = item_id
                .strip_prefix("acp-tool-")
                .ok_or("ACP journal tool identity missing")?;
            let thread_id = self.session.clone().ok_or("ACP journal session missing")?;
            let state = if item.complete { "final" } else { "partial" };
            emit(
                self.events,
                Event::Journal {
                    generation: self.run.generation,
                    task_id: self.run.input.task.id,
                    record: crate::store::ProviderRecord {
                        id: format!(
                            "acp-{}-{}-{}-{native_id}-{state}",
                            std::process::id(),
                            self.journal_nonce,
                            self.run.generation
                        ),
                        agent: self.run.input.agent,
                        generation: self.run.generation,
                        thread_id,
                        turn_id: self.turn(),
                        item_id: item_id.clone(),
                        origin: format!(
                            "{} ACP session/update tool report",
                            self.run.input.agent.label()
                        ),
                        coverage: format!(
                            "Provider-reported {state} tool snapshot only. {} Agent-owned execution/config/MCP is external policy; client fs/terminal callbacks disabled. Not local runner evidence, sandbox proof, or independent side-effect verification.",
                            if item.complete {
                                "Provider reported terminal tool status."
                            } else {
                                "Transport ended without a terminal tool status; side effects/outcome unknown."
                            }
                        ),
                        text: safe(&item.text),
                        complete: item.complete,
                    },
                },
            )?;
        }
        Ok(())
    }
    fn teardown(&mut self) -> bool {
        // Teardown is never another chance to deliver a queued grant or model prompt.
        self.wire.clear();
        self.wire_offset = 0;
        self.process.close_stdin();
        let _ = self.process.terminate(false);
        let deadline = Instant::now() + Duration::from_millis(300);
        while Instant::now() < deadline {
            let _ = self.process.try_wait();
            if self.process.tree_empty().unwrap_or(false) {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.process.terminate(true);
        let deadline = Instant::now() + Duration::from_secs(1);
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
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>, String> {
        let mut frames = Vec::new();
        for chunk in bytes.split_inclusive(|byte| *byte == b'\n') {
            if self.line.len() + chunk.len() > FRAME_LIMIT {
                return Err("ACP JSONL frame exceeds 2 MiB".into());
            }
            self.line.extend_from_slice(chunk);
            if chunk.last() == Some(&b'\n') {
                if self.line.iter().all(u8::is_ascii_whitespace) {
                    return Err("ACP stdout contains an empty/nonprotocol frame".into());
                }
                frames.push(
                    serde_json::from_slice(&self.line)
                        .map_err(|_| "Malformed ACP JSON/UTF-8; raw bytes withheld")?,
                );
                self.line.clear();
            }
        }
        Ok(frames)
    }
}
fn prompt_outcome(reason: &str) -> Outcome {
    match reason {
        "end_turn" => Outcome::Completed,
        "cancelled" => Outcome::Cancelled,
        "refusal" | "max_tokens" | "max_turn_requests" => Outcome::Failed,
        _ => Outcome::Unknown,
    }
}

fn identity(value: &Value, key: &str) -> Result<String, String> {
    let text = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("ACP missing {key} identity"))?;
    if text.is_empty() || text.len() > 256 || safe(text) != text {
        return Err(format!("ACP invalid {key} identity"));
    }
    Ok(text.into())
}
fn valid_rpc_id(id: &Value) -> bool {
    if let Some(text) = id.as_str() {
        !text.is_empty() && text.len() <= 256 && safe(text) == text
    } else {
        id.as_i64().is_some() || id.as_u64().is_some()
    }
}
fn bidi(c: char) -> bool {
    matches!(c,'\u{061c}'|'\u{200e}'|'\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')
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
    fn managed_review_is_refused_before_loading_or_dispatching_an_acp_session() {
        let workspace = Workspace {
            project: PathBuf::from("unopened-project"),
            data_dir: PathBuf::from("unopened-storage"),
            exclusions: Vec::new(),
        };
        for agent in [Agent::OpenCode, Agent::Gemini] {
            let input = Input {
                task: crate::store::Snapshot::empty("unopened-project".into()),
                excerpts: Vec::new(),
                report: None,
                resume: None,
                agent,
                operation: Operation::Review,
                edit_paths: Vec::new(),
                handoff: None,
                inspect_sessions: false,
            };
            assert!(
                payload(&workspace, &input)
                    .unwrap_err()
                    .contains("no-source-write")
            );
        }
    }

    #[test]
    fn cancellation_requests_do_not_override_authoritative_stop_reason() {
        assert_eq!(prompt_outcome("end_turn"), Outcome::Completed);
        assert_eq!(prompt_outcome("cancelled"), Outcome::Cancelled);
        assert_eq!(prompt_outcome("max_tokens"), Outcome::Failed);
        assert_eq!(prompt_outcome("unknown"), Outcome::Unknown);
    }
    #[test]
    fn session_list_does_not_expose_foreign_project_metadata_or_accept_malformed_results() {
        let project = std::path::Path::new("/reviewed-project");
        let report = session_list_report(&json!({}), project, &json!({"sessions":[
            {"sessionId":"owned-session","cwd":"/reviewed-project","title":"Approved workflow"},
            {"sessionId":"foreign-session","cwd":"/unreviewed-project","title":"Private foreign task"}
        ]})).unwrap();
        assert!(report.contains("owned-session"));
        assert!(report.contains("Approved workflow"));
        assert!(!report.contains("foreign-session"));
        assert!(!report.contains("Private foreign task"));
        assert!(session_list_report(&json!({}), project, &json!({})).is_err());
        assert!(
            session_list_report(
                &json!({}),
                project,
                &json!({"sessions":[{"cwd":"/reviewed-project","title":"Missing identity"}]})
            )
            .is_err()
        );
        assert!(
            session_list_report(
                &json!({}),
                project,
                &json!({"sessions":vec![json!({});257]})
            )
            .is_err()
        );
    }
    #[test]
    fn negotiated_capabilities_are_not_profile_assumptions() {
        let open=negotiated(&json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"list":{}},"future":true},"futureField":123})).unwrap();
        let gemini =
            negotiated(&json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}}))
                .unwrap();
        assert!(supports_method(&open, "session/list"));
        assert!(!supports_method(&gemini, "session/list"));
        assert!(supports_method(&gemini, "session/load"));
        assert!(!supports_method(&json!({}), "session/load"));
        assert!(negotiated(&json!({"protocolVersion":2})).is_err());
        assert!(!supports_method(&open, "session/elicitation"));
    }
    #[test]
    fn persistent_options_are_filtered_exact_ids_survive() {
        let params = json!({"options":[{"optionId":"forever","name":"Always","kind":"allow_always"},{"optionId":"no-future","name":"Never","kind":"reject_always"},{"optionId":"deny exact","name":"No","kind":"reject_once"},{"optionId":"yes exact","name":"Yes","kind":"allow_once"}]});
        let options = permission_options(&params).unwrap();
        assert_eq!(options.len(), 2);
        assert_eq!(
            options[0].answer,
            json!({"outcome":{"outcome":"selected","optionId":"deny exact"}})
        );
        assert_eq!(
            options[1].answer,
            json!({"outcome":{"outcome":"selected","optionId":"yes exact"}})
        );
        let fallback = permission_options(
            &json!({"options":[{"optionId":"always","name":"Always","kind":"allow_always"}]}),
        )
        .unwrap();
        assert_eq!(fallback.len(), 1);
        assert_eq!(
            fallback[0].answer,
            json!({"outcome":{"outcome":"cancelled"}})
        );
    }
    #[test]
    fn cancellation_invalidates_pending_and_late_grants() {
        let options = permission_options(
            &json!({"options":[{"optionId":"one","name":"Once","kind":"allow_once"}]}),
        )
        .unwrap();
        let mut ledger = Permissions::default();
        ledger.insert(&json!(7), &options).unwrap();
        assert_eq!(ledger.invalidate(), vec![json!(7)]);
        assert!(!ledger.take_answer(&json!(7), &options[1].answer));
        assert!(ledger.insert(&json!(7), &options).is_err());
        ledger.insert(&json!(8), &options).unwrap();
        assert!(!ledger.take_answer(
            &json!(8),
            &json!({"outcome":{"outcome":"selected","optionId":"always"}})
        ));
        assert!(ledger.take_answer(&json!(8), &options[1].answer));
        assert!(!ledger.take_answer(&json!(8), &options[1].answer));
    }
    #[test]
    fn malformed_stdout_is_not_completion_and_optional_fields_survive() {
        let mut framer = Framer::default();
        assert!(framer.push(b"{\"jsonrpc\":\"2.0\",").unwrap().is_empty());
        assert_eq!(
            framer
                .push(b"\"id\":1,\"result\":{\"future\":1}}\n")
                .unwrap()
                .len(),
            1
        );
        assert!(framer.push(b"provider log\n").is_err());
        assert!(
            Framer::default()
                .push(&vec![b'x'; FRAME_LIMIT + 1])
                .is_err()
        );
    }
    #[test]
    fn oversized_tool_final_preserves_bounded_partial_observation() {
        let mut item = Item::new(ItemKind::Tool);
        item.tool_snapshot(&json!({"title":"Read","status":"in_progress"}))
            .unwrap();
        let partial = item.text.clone();
        assert!(
            item.tool_snapshot(&json!({"status":"completed","rawOutput":"x".repeat(32 * 1024)}))
                .is_err()
        );
        assert_eq!(item.text, partial);
        assert!(!item.complete);
    }
    #[test]
    fn tool_final_snapshots_do_not_duplicate_delta_or_completion() {
        let mut item = Item::new(ItemKind::Tool);
        item.tool_snapshot(&json!({"title":"Read","status":"in_progress","content":[{"type":"content","content":{"type":"text","text":"part"}}]})).unwrap();
        item.tool_snapshot(&json!({"status":"completed","content":[{"type":"content","content":{"type":"text","text":"part final"}}]})).unwrap();
        let final_text = item.text.clone();
        item.tool_snapshot(&json!({"status":"completed","content":[{"type":"content","content":{"type":"text","text":"part final"}}],"futureOptional":true})).unwrap();
        assert_eq!(item.text, final_text);
        assert!(item.complete);
        item.tool_snapshot(&json!({"locations":[]})).unwrap();
        assert!(item.complete);
        assert!(
            item.tool_snapshot(&json!({"status":"in_progress"}))
                .is_err()
        );
        assert!(item.complete);
    }
}
