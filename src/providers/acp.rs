//! Private ACP v1 transport. Client callbacks are deliberately not filesystem/terminal sandboxes.
use crate::providers::{
    Agent, Event, Input, ItemKind, ModelCatalogue, ModelOption, ModelSelection, Operation, Outcome,
    PermissionOption, Preview, Prompt, PromptKind, Request,
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
    trust: crate::providers::agent::ExternalTrust,
    payload: String,
}
fn profile(agent: Agent) -> Result<(usize, Vec<String>), String> {
    match agent {
        Agent::OpenCode => Ok((2, vec!["acp".into(), "--pure".into()])),
        Agent::Gemini => Ok((3, vec!["--acp".into()])),
        Agent::Devin => Ok((crate::providers::agent::DEVIN, vec!["acp".into()])),
        _ => Err("ACP route accepts only OpenCode, Gemini or Devin".into()),
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
                    let (index, launch) = profile(input.agent)?;
                    let trust =
                        crate::providers::agent::external_trust(index, &workspace.project, launch)?;
                    let payload = payload(&workspace, &input)?;
                    let mut lines = trust.lines.clone();
                    lines.extend([
                        "Private owned ACPv1 stdio process; initialize and session authentication happen only after Run consent.".into(),
                        "Client filesystem and terminal callbacks are NOT supported or advertised; every such request is refused.".into(),
                        "Agent-owned tools, project instructions, MCP, configuration, hooks and history are EXTERNAL PROVIDER POLICY. ACP is not a path/network sandbox or a read-only guarantee.".into(),
                        "Only exact reject-once/allow-once provider permission IDs are offered; persistent grants are filtered. Unknown questions/elicitation extensions are refused, not advertised.".into(),
                        "No authenticate/login action is invoked. Provider-owned auth/account/product eligibility must succeed at session creation; installed is not authenticated.".into(),
                        "Resume is only sent if session/load is actually negotiated; replayed history is suppressed. No standardized token usage is negotiated by these interfaces.".into(),
                    ]);
                    match input.agent {
                        Agent::OpenCode => lines.push("OpenCode acp --pure disables external plugins; built-in tools/config/MCP/project rules remain provider-owned.".into()),
                        Agent::Gemini => lines.push("Gemini CLI uses current stable --acp. Older --experimental-acp-only releases must be updated explicitly; no fallback or automatic download. Its configured Google OAuth/API/Vertex route is provider-owned; no free-account eligibility claim.".into()),
                        Agent::Devin => lines.push("Devin CLI acp uses local stdio, without --cloud. Connect separately through the official native browser login; credentials, account eligibility, tools and configuration remain provider-owned.".into()),
                        _ => unreachable!("profile rejects non-ACP providers"),
                    }
                    lines.push(format!(
                        "Reviewed selection snapshot: model = {}; reasoning = {}. Explicit values are sent through the peer's official selectors and must be acknowledged before any prompt; catalogue membership is not an authorization gate.",
                        input.selection.model.as_deref().unwrap_or("provider default (not yet observed)"),
                        input.selection.effort.as_deref().unwrap_or("provider default (not yet observed)")
                    ));
                    if input.selection != ModelSelection::default() {
                        lines.push("Requested model/reasoning changes use official session configuration IDs. Setters can persist provider-owned session metadata; exact current-value acknowledgments are required, not membership in a potentially stale option list.".into());
                    }
                    if input.inspect_models {
                        lines.push("MODEL INSPECTION ONLY: initialize, then one temporary session/new because ACP advertises session models/configOptions there, not in initialize. Session creation can persist provider metadata, load project/provider configuration, initialize services/MCP and contact provider/auth networks; external effects are not a read-only guarantee. Reviewed model/effort selections may change this temporary session's configuration. No session/prompt/load/list/history/authenticate, task content, journal or retained resume grant. The process is closed afterward; provider-owned session metadata may remain.".into());
                    }
                    if input.inspect_sessions {
                        lines.push("CAPABILITY INSPECTION ONLY: initialize, then session/list only if negotiated. No session/new/load/prompt, model request, imported handoff or grant. Listed metadata does not authorize resuming foreign sessions.".into());
                    }
                    let mut blockers = Vec::new();
                    if input.operation == Operation::Implement
                        && !input.inspect_sessions
                        && !input.inspect_models
                    {
                        blockers.push("Controlled implementation refused: this ACP peer's agent-owned tools cannot enforce Eevee's selected-file boundary. Use the explicitly external-policy manual handoff instead; no sandbox claim.".into());
                    }
                    if let Some(id) = &input.resume
                        && (id.is_empty() || id.len() > 256 || safe(id) != *id)
                    {
                        blockers.push("Resume refused: invalid ACP session identity".into());
                    }
                    if sessions.len() >= 256
                        && input.resume.is_none()
                        && !input.inspect_models
                        && !input.inspect_sessions
                    {
                        blockers.push(
                            "ACP session bound reached; restart before another session".into(),
                        );
                    }
                    if let Some(id) = &input.resume
                        && sessions.get(&(input.agent, id.clone())) != Some(&task_id)
                    {
                        blockers.push("Resume refused: session is not an observed session of this provider and task; no cross-provider session transfer.".into());
                    }
                    if input.credential.is_some() && input.agent != Agent::Devin {
                        blockers.push(
                            "A website-login credential is attached but this provider is not Devin; injection is refused. Clear the credential or switch back.".into(),
                        );
                    } else if let Some(credential) = &input.credential {
                        if credential.expired() {
                            blockers.push(
                                "The session credential from the provider website login has expired; Connect again before launching. No launch was prepared.".into(),
                            );
                        } else {
                            lines.push(format!(
                                "Session credential: Eevee injects {} into the launched provider process only (memory-held website credential; it takes precedence over the provider CLI's stored credentials and is never persisted or printed).{}",
                                credential.env,
                                credential
                                    .expires_at
                                    .map(|at| format!(" Token expires ~unix {at}."))
                                    .unwrap_or_default()
                            ));
                        }
                    }
                    let preview = Preview {
                        destination: None,
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
    input.validate_selection()?;
    if std::path::Path::new(&input.task.project) != workspace.project.as_path()
        || !matches!(input.agent, Agent::OpenCode | Agent::Gemini | Agent::Devin)
        || (!input.inspect_models
            && input.operation == Operation::Plan
            && !input.edit_paths.is_empty())
    {
        return Err("ACP task/project/profile or planning write scope mismatch".into());
    }
    if input.inspect_models {
        return serde_json::to_string_pretty(&json!({
            "purpose":"Inspect provider-advertised model and reasoning selectors only",
            "initialize":{"protocolVersion":1,"clientCapabilities":client_capabilities()},
            "session/new":{"cwd":workspace.project,"mcpServers":[]},
            "selection":input.selection,
            "optional_configuration":"session/set_config_option for model/thought_level controls; legacy session/set_model for exact model IDs; correlated setter success is required",
            "effects":"Temporary session creation/configuration may persist provider metadata, load configuration/services and contact provider networks. Provider-owned metadata may remain after teardown.",
            "session_prompt_load_list_history":false,
            "model_prompts_sent":0,
            "retain_resume_grant":false
        })).map_err(|_| "Could not encode ACP model inspection".into());
    }
    input.validate_review()?;
    crate::store::validate_planning_input(&workspace.data_dir, &input.task, &input.excerpts)?;
    if input.inspect_sessions {
        if input.selection != ModelSelection::default() {
            return Err(
                "Session-list inspection cannot apply model or reasoning selections".into(),
            );
        }
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
        crate::workspace::handoff::validate_dispatch(
            document,
            &input.task,
            &input.excerpts,
            input.report.as_ref(),
            &workspace.exclusions,
        )?;
    }
    let sources = crate::workspace::context::planning_payload(
        input.task.id,
        input.report.as_ref(),
        &input.excerpts,
        &workspace.exclusions,
    )?;
    let text = serde_json::to_string_pretty(&json!({
        "purpose":"Plan the user's task. Do not implement or write files. Ask for decisions and report proposals as unverified planning. This instruction is not an enforced sandbox.",
        "task":{"id":input.task.id,"revision":input.task.revision,"goal":input.task.goal,"acceptance_criteria":input.task.criteria,"next_action":crate::workspace::handoff::without_local_provenance(&input.task.next_action)},
        "request":input.task.request_text(),
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
    let (index, _) = profile(run.input.agent)?;
    crate::providers::agent::revalidate_external_trust(index, &workspace.project, &run.trust)?;
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
#[derive(Clone)]
struct SelectOption {
    id: String,
    current: String,
    values: Vec<ModelOption>,
}
#[derive(Default)]
struct Models {
    model: Option<SelectOption>,
    thought: Option<SelectOption>,
    legacy: Option<SelectOption>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum SelectionChange {
    Config {
        id: String,
        value: String,
        thought: bool,
    },
    Legacy {
        model: String,
    },
}
impl SelectionChange {
    fn request(&self, session: &str) -> (&'static str, Value) {
        match self {
            Self::Config { id, value, .. } => (
                "session/set_config_option",
                json!({"sessionId":session,"configId":id,"value":value}),
            ),
            Self::Legacy { model } => (
                "session/set_model",
                json!({"sessionId":session,"modelId":model}),
            ),
        }
    }
}
fn config_text(value: &Value, key: &str, limit: usize, optional: bool) -> Result<String, String> {
    let text = match value.get(key) {
        None | Some(Value::Null) if optional => "",
        Some(Value::String(text)) => text,
        _ => return Err(format!("ACP configuration lacks valid {key}")),
    };
    if (!optional && text.is_empty())
        || text.len() > limit
        || safe(text) != text
        || text.chars().any(char::is_control)
    {
        return Err(format!("ACP configuration has unsafe or oversized {key}"));
    }
    Ok(text.into())
}
fn select_values(values: &Value, limit: usize) -> Result<Vec<ModelOption>, String> {
    let entries = values
        .as_array()
        .ok_or("ACP select options must be an array")?;
    if entries.len() > limit {
        return Err("ACP select options exceed the advertised-value bound".into());
    }
    let mut result = Vec::new();
    let mut ids = HashSet::new();
    let mut groups = HashSet::new();
    let grouped = entries
        .first()
        .is_some_and(|entry| entry.get("group").is_some());
    for entry in entries {
        if entry.get("group").is_some() != grouped {
            return Err("ACP select options mix groups and values".into());
        }
        let members = if grouped {
            let group = config_text(entry, "group", 256, false)?;
            config_text(entry, "name", 512, false)?;
            if !groups.insert(group) {
                return Err("ACP select options contain duplicate groups".into());
            }
            entry
                .get("options")
                .and_then(Value::as_array)
                .ok_or("ACP option group lacks its value array")?
                .as_slice()
        } else {
            std::slice::from_ref(entry)
        };
        if result.len() + members.len() > limit {
            return Err("ACP grouped select options exceed the advertised-value bound".into());
        }
        for member in members {
            if member.get("group").is_some() {
                return Err(
                    "Nested ACP option groups are not supported by the official contract".into(),
                );
            }
            let id = config_text(member, "value", 256, false)?;
            if !ids.insert(id.clone()) {
                return Err("ACP select options contain duplicate values".into());
            }
            result.push(ModelOption {
                id,
                name: config_text(member, "name", 512, false)?,
                description: config_text(member, "description", 4096, true)?,
                efforts: Vec::new(),
                default_effort: None,
            });
        }
    }
    Ok(result)
}
impl Models {
    fn parse(result: &Value) -> Result<Self, String> {
        if !result.is_object() {
            return Err("ACP session/configuration response is not an object".into());
        }
        let mut state = Self::default();
        if let Some(options) = result.get("configOptions").filter(|value| !value.is_null()) {
            let options = options
                .as_array()
                .ok_or("ACP configOptions must be an array")?;
            if options.len() > 128 {
                return Err("ACP configOptions exceed the 128-selector bound".into());
            }
            let mut ids = HashSet::new();
            for option in options {
                let id = config_text(option, "id", 256, false)?;
                if !ids.insert(id.clone()) {
                    return Err("ACP configOptions contain duplicate selector IDs".into());
                }
                let category = option.get("category").and_then(Value::as_str);
                // Missing/custom categories and unknown controls are not inferred from names.
                if !matches!(category, Some("model" | "thought_level"))
                    || option.get("type").and_then(Value::as_str) != Some("select")
                {
                    continue;
                }
                config_text(option, "name", 512, false)?;
                config_text(option, "description", 4096, true)?;
                let thought = category == Some("thought_level");
                let select = SelectOption {
                    id,
                    current: config_text(option, "currentValue", 256, false)?,
                    values: select_values(
                        option.get("options").ok_or("ACP select lacks options")?,
                        if thought { 32 } else { 4096 },
                    )?,
                };
                // ACP's ordered configuration list resolves category ties.
                let target = if thought {
                    &mut state.thought
                } else {
                    &mut state.model
                };
                if target.is_none() {
                    *target = Some(select);
                }
            }
        }
        // A categorized model selector is authoritative even when it has no selectable values.
        if state.model.is_none()
            && let Some(models) = result.get("models").filter(|value| !value.is_null())
        {
            let available = models
                .get("availableModels")
                .and_then(Value::as_array)
                .ok_or("ACP legacy models lack availableModels array")?;
            if available.len() > 4096 {
                return Err("ACP legacy models exceed the 4096-entry resource bound".into());
            }
            let mut values = Vec::with_capacity(available.len());
            let mut ids = HashSet::new();
            for model in available {
                let id = config_text(model, "modelId", 256, false)?;
                if !ids.insert(id.clone()) {
                    return Err("ACP legacy models contain duplicate modelId".into());
                }
                values.push(ModelOption {
                    id,
                    name: config_text(model, "name", 512, false)?,
                    description: config_text(model, "description", 4096, true)?,
                    efforts: Vec::new(),
                    default_effort: None,
                });
            }
            state.legacy = Some(SelectOption {
                id: String::new(),
                current: config_text(models, "currentModelId", 256, false)?,
                values,
            });
        }
        Ok(state)
    }
    fn updated(&self, result: &Value) -> Result<Self, String> {
        let mut next = Self::parse(result)?;
        // Legacy model state is separate from the complete configOptions snapshot.
        if self.model.is_none() && next.model.is_none() && result.get("models").is_none() {
            next.legacy = self.legacy.clone();
        }
        Ok(next)
    }
    fn active_model(&self) -> Option<&SelectOption> {
        self.model.as_ref().or(self.legacy.as_ref())
    }
    fn catalogue(&self, agent: Agent) -> Result<ModelCatalogue, String> {
        let active = self.active_model();
        let mut models = active.map_or_else(Vec::new, |select| select.values.clone());
        if let Some(thought) = &self.thought
            && let Some(current) = active
            && let Some(model) = models.iter_mut().find(|model| model.id == current.current)
        {
            model.efforts = thought
                .values
                .iter()
                .map(|value| value.id.clone())
                .collect();
            model.default_effort = model
                .efforts
                .contains(&thought.current)
                .then(|| thought.current.clone());
        }
        let catalogue = ModelCatalogue {
            models,
            current: active.map(|select| select.current.clone()),
            source: format!("{} ACP1 session/new/configuration: {}. Temporary provider session; no prompt, history or retained resume grant.{}",
                agent.label(),
                if self.model.is_some() { "official configOptions model select" }
                else if self.legacy.is_some() { "legacy models.availableModels/currentModelId" }
                else { "no supported model selector advertised" },
                if active.is_none_or(|select| select.values.is_empty()) {
                    " No model options advertised; use a known exact ID, or connect/configure and refresh."
                } else { "" }),
            effort_note: self.thought.as_ref().map(|_| "Official thought_level values apply only to the observed current model. Changing model can replace this list; requested reasoning is revalidated after model acknowledgment.".into()),
        };
        catalogue.validate()?;
        Ok(catalogue)
    }
    fn validate_selected(
        &self,
        selection: &ModelSelection,
        model_ack: bool,
        effort_ack: bool,
    ) -> Result<(), String> {
        if let Some(model) = &selection.model {
            let active = self
                .active_model()
                .ok_or("ACP peer does not advertise model selection")?;
            if model_ack && &active.current != model {
                return Err(
                    "ACP model changed or was not acknowledged; reviewed selection invalidated"
                        .into(),
                );
            }
        }
        if let Some(effort) = &selection.effort
            && (selection.model.is_none() || model_ack)
        {
            let thought = self
                .thought
                .as_ref()
                .ok_or("ACP peer does not advertise an official thought_level select control")?;
            if effort_ack && &thought.current != effort {
                return Err(
                    "ACP reasoning changed or was not acknowledged; reviewed selection invalidated"
                        .into(),
                );
            }
        }
        Ok(())
    }
    fn require_acknowledged(
        &self,
        selection: &ModelSelection,
        model_ack: bool,
        effort_ack: bool,
    ) -> Result<(), String> {
        self.validate_selected(selection, model_ack, effort_ack)?;
        if selection.model.is_some() && !model_ack || selection.effort.is_some() && !effort_ack {
            return Err(
                "ACP selection requires provider setter acknowledgment before prompt dispatch"
                    .into(),
            );
        }
        Ok(())
    }
    fn next_change(
        &self,
        selection: &ModelSelection,
        model_ack: bool,
        effort_ack: bool,
    ) -> Result<Option<SelectionChange>, String> {
        self.validate_selected(selection, model_ack, effort_ack)?;
        if let Some(model) = &selection.model
            && !model_ack
        {
            return Ok(Some(if let Some(control) = &self.model {
                SelectionChange::Config {
                    id: control.id.clone(),
                    value: model.clone(),
                    thought: false,
                }
            } else {
                SelectionChange::Legacy {
                    model: model.clone(),
                }
            }));
        }
        if let Some(effort) = &selection.effort
            && !effort_ack
        {
            let control = self
                .thought
                .as_ref()
                .ok_or("ACP thought_level selector unavailable")?;
            return Ok(Some(SelectionChange::Config {
                id: control.id.clone(),
                value: effort.clone(),
                thought: true,
            }));
        }
        Ok(None)
    }
    fn acknowledge(&mut self, result: &Value, change: &SelectionChange) -> Result<(), String> {
        match change {
            SelectionChange::Config { id, value, thought } => {
                if result.get("configOptions").is_none() {
                    return Err(
                        "ACP set_config_option lacks complete configOptions acknowledgment".into(),
                    );
                }
                let next = self.updated(result)?;
                let control = if *thought { &next.thought } else { &next.model };
                if !control
                    .as_ref()
                    .is_some_and(|control| &control.id == id && &control.current == value)
                {
                    return Err(
                        "ACP set_config_option did not acknowledge the exact requested selection"
                            .into(),
                    );
                }
                *self = next;
            }
            SelectionChange::Legacy { model } => {
                // Released legacy SetSessionModelResponse is an empty object; the correlated
                // successful response acknowledges the requested model, not an inferred fallback.
                if !result.is_object() {
                    return Err("Malformed ACP legacy set_model acknowledgment".into());
                }
                if result.get("models").is_some() || result.get("configOptions").is_some() {
                    let next = Self::parse(result)?;
                    if next
                        .active_model()
                        .is_none_or(|active| &active.current != model)
                    {
                        return Err(
                            "ACP legacy set_model did not acknowledge the exact requested model"
                                .into(),
                        );
                    }
                    *self = next;
                } else {
                    let active = self
                        .legacy
                        .as_mut()
                        .ok_or("ACP legacy model control disappeared")?;
                    active.current.clone_from(model);
                }
            }
        }
        Ok(())
    }
}
#[derive(PartialEq, Eq)]
enum Call {
    Initialize,
    Session,
    Sessions,
    Selection(SelectionChange),
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
        let mut text = safe(
            &serde_json::to_string_pretty(&self.fields).map_err(|_| "ACP tool encoding failed")?,
        );
        if text.len() > 32 * 1024 {
            // Bound oversized tool fields instead of aborting the whole run:
            // keep a head/tail slice with the exact omitted byte count so the
            // journal observation stays honest and bounded.
            const FIELD_LIMIT: usize = 28 * 1024;
            for key in ["rawOutput", "content"] {
                let Some(value) = self.fields.get_mut(key) else {
                    continue;
                };
                let rendered = match &*value {
                    Value::String(text) => text.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                if rendered.len() <= FIELD_LIMIT {
                    continue;
                }
                let total = rendered.len();
                let head_bytes = FIELD_LIMIT / 2;
                let tail_bytes = FIELD_LIMIT - head_bytes;
                let mut head_end = head_bytes.min(total);
                while !rendered.is_char_boundary(head_end) {
                    head_end -= 1;
                }
                let mut tail_start = (total - tail_bytes).max(head_end);
                while tail_start < total && !rendered.is_char_boundary(tail_start) {
                    tail_start += 1;
                }
                *value = json!({
                    "truncated_by_eevee": true,
                    "total_bytes": total,
                    "omitted_bytes": total.saturating_sub(head_end + total - tail_start),
                    "head": &rendered[..head_end],
                    "tail": &rendered[tail_start..],
                });
            }
            text = safe(
                &serde_json::to_string_pretty(&self.fields)
                    .map_err(|_| "ACP tool encoding failed")?,
            );
            if text.len() > 32 * 1024 {
                return Err("ACP tool snapshot exceeds the 32 KiB journal bound after truncation; stopping with the prior bounded observation only".into());
            }
        }
        self.text = text;
        if let Some(status) = update.get("status").and_then(Value::as_str) {
            self.complete = matches!(status, "completed" | "failed");
        }
        Ok(())
    }
}

struct Engine<'a> {
    process: crate::workspace::runner::native::Process,
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
    models: Models,
    model_ack: bool,
    effort_ack: bool,
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
        let (index, _) = profile(run.input.agent)?;
        let prepared =
            crate::providers::agent::qualify_external(index, &workspace.project, &run.trust, stop)?;
        if stop.load(Ordering::Acquire) || cancelled.load(Ordering::Acquire) == run.generation {
            return Err("Cancelled before ACP launch".into());
        }
        let env: Vec<(String, String)> = if run.input.agent == Agent::Devin {
            run.input
                .credential
                .iter()
                .filter(|credential| !credential.expired())
                .map(|credential| (credential.env.to_string(), credential.token().to_string()))
                .collect()
        } else {
            Vec::new()
        };
        crate::workspace::runner::native::Process::spawn_piped_env(&prepared, &env)
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
        models: Models::default(),
        model_ack: false,
        effort_ack: false,
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
    let result = if run.input.inspect_models || run.input.inspect_sessions {
        result
    } else {
        match engine.journal_tools() {
            Ok(()) => result,
            Err(error) => Err(error),
        }
    };
    if !run.input.inspect_models
        && !run.input.inspect_sessions
        && let Some(session) = engine.session.clone()
    {
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
            deadline: if kind == Call::Prompt {
                None
            } else {
                Some(Instant::now() + Duration::from_secs(45))
            },
            kind,
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
                // agentInfo describes the agent build, not necessarily its CLI package
                // (Devin has reported 0.0.0-dev). Protocol capabilities, not package
                // equality, qualify this peer; executable identity remains bound.
                self.status(&format!(
                    "Observed CLI {}; ACP v1 negotiated; required actions remain capability-gated",
                    self.run.trust.observed_version().unwrap_or("unavailable")
                ))?;
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
                self.models = Models::parse(result)?;
                self.configure_or_dispatch()?;
            }
            Call::Selection(change) => {
                if self.cancelling.is_some() {
                    self.terminal = Some((Outcome::Cancelled, "Cancelled before prompt dispatch; provider session/configuration effects may remain".into()));
                    return Ok(());
                }
                self.models.acknowledge(result, &change)?;
                match change {
                    SelectionChange::Config { thought: true, .. } => self.effort_ack = true,
                    _ => self.model_ack = true,
                }
                self.configure_or_dispatch()?;
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
    fn configure_or_dispatch(&mut self) -> Result<(), String> {
        revalidate(self.workspace, self.run)?;
        if self.cancelled.load(Ordering::Acquire) == self.run.generation {
            return self.cancel();
        }
        let session = self
            .session
            .clone()
            .ok_or("ACP configuration before session identity")?;
        if let Some(change) =
            self.models
                .next_change(&self.run.input.selection, self.model_ack, self.effort_ack)?
        {
            let (method, params) = change.request(&session);
            self.call(Call::Selection(change), method, params)?;
            return Ok(());
        }
        if self.run.input.inspect_models {
            let catalogue = self.models.catalogue(self.run.input.agent)?;
            let selectable = !catalogue.models.is_empty();
            emit(
                self.events,
                Event::Models {
                    generation: self.run.generation,
                    task_id: self.run.input.task.id,
                    catalogue,
                },
            )?;
            self.terminal = Some((Outcome::Completed, if selectable {
                "ACP model configuration observed; no prompt/history/journal or retained resume grant. Temporary provider session metadata/configuration effects may remain."
            } else {
                "ACP inspection completed, but no selectable models advertised; connect/configure provider and explicitly refresh. No prompt/history/journal or retained resume grant; temporary provider session metadata may remain."
            }.into()));
            return Ok(());
        }
        let model = self
            .models
            .active_model()
            .map_or("Provider default (not observed)", |select| {
                select.current.as_str()
            });
        emit(self.events, Event::Thread {
            generation: self.run.generation,
            task_id: self.run.input.task.id,
            thread_id: session.clone(),
            model: model.into(),
            account: "Provider-owned authentication accepted session; billing/eligibility not independently verified".into(),
        })?;
        self.dispatch(&session)
    }
    fn dispatch(&mut self, session: &str) -> Result<(), String> {
        revalidate(self.workspace, self.run)?;
        if self.run.input.inspect_models || self.run.input.inspect_sessions {
            return Err("ACP inspection cannot dispatch a model prompt".into());
        }
        self.models.require_acknowledged(
            &self.run.input.selection,
            self.model_ack,
            self.effort_ack,
        )?;
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
        if self.terminal.is_some() {
            return Ok(());
        }
        // session/load replays historical updates; its response is authoritative initial state.
        // Configuration notifications after setup remain meaningful before prompt dispatch.
        if self.session.is_none()
            || self
                .calls
                .iter()
                .any(|call| matches!(&call.kind, Call::Session))
        {
            return Ok(());
        }
        if params.get("sessionId").and_then(Value::as_str) != self.session.as_deref() {
            return Err("ACP update has wrong session identity".into());
        }
        let update = params.get("update").ok_or("ACP update lacks content")?;
        if update.get("sessionUpdate").and_then(Value::as_str) == Some("config_option_update") {
            if !update.get("configOptions").is_some_and(Value::is_array) {
                return Err("ACP config_option_update lacks complete configOptions".into());
            }
            let next = self.models.updated(update)?;
            next.validate_selected(&self.run.input.selection, self.model_ack, self.effort_ack)?;
            self.models = next;
            return self.status("Provider session configuration updated; reviewed model/reasoning selection revalidated");
        }
        // Cancellation withdraws grants, not observations of already-started tools.
        // Inspection and load replay never produce new-turn artifacts or journals.
        if !self.dispatched || self.run.input.inspect_models || self.run.input.inspect_sessions {
            return Ok(());
        }
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
                text: safe_stream(&item.text, item.complete || item.kind != ItemKind::Message),
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
    safe_stream(text, true)
}
fn safe_stream(text: &str, complete: bool) -> String {
    crate::workspace::privacy::redact_stream(text, complete)
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
mod tests;
