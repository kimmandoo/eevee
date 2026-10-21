//! Explicitly trusted, protocol-qualified Codex planning and client-controlled implementation.
//! Provider sandbox policy is not an Eevee sandbox; no provider credentials/configuration are written.
//! Usage retains latest-response and cumulative-session snapshots separately. Only
//! observed completed owned-turn totals provide a later request's baseline;
//! unknown resumed history is never attributed to the current instruction.
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::providers::writes::{TOOL_NAME, WriteFailure, WriteProposal, WriteSet};
use crate::providers::{
    Agent, Event, Input, ItemKind, ModelCatalogue, ModelOption, ModelSelection, Operation, Outcome,
    PermissionOption, Preview, Prompt, PromptKind, Question, QuestionOption, Request,
};
use crate::workspace::usage::{Counters, Identity, Kind, Report, Scope, Semantics};
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

fn capture_writes(
    root: &Path,
    data_dir: &Path,
    input: &Input,
    exclusions: &[PathBuf],
) -> Result<Option<WriteSet>, String> {
    if input.inspect_models {
        return Ok(None);
    }
    if input.operation != Operation::Implement {
        if !input.edit_paths.is_empty() {
            return Err("Plan and Review requests cannot carry write paths".into());
        }
        return Ok(None);
    }
    WriteSet::capture(root, data_dir, &input.edit_paths, exclusions).map(Some)
}

fn proposed_write(writes: &WriteSet, params: &Value) -> Result<WriteProposal, String> {
    if params.get("tool").and_then(Value::as_str) != Some(TOOL_NAME)
        || params
            .get("namespace")
            .is_some_and(|value| !value.is_null())
    {
        return Err("Unadvertised dynamic tool refused".into());
    }
    let arguments = params
        .get("arguments")
        .and_then(Value::as_object)
        .ok_or("Write tool requires an arguments object")?;
    if arguments.len() != 2 {
        return Err("Only exact path/content arguments are accepted".into());
    }
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .ok_or("Write path missing")?;
    let content = arguments
        .get("content")
        .and_then(Value::as_str)
        .ok_or("Write content missing")?;
    writes.propose(PathBuf::from(path), content.into())
}

fn write_permission(proposal: &WriteProposal) -> PromptKind {
    PromptKind::Permission {
        description: proposal.description(),
        options: vec![
            PermissionOption {
                label: "Deny".into(),
                description: "Do not write this file.".into(),
                answer: json!({"decision":"decline"}),
            },
            PermissionOption {
                label: "Allow once".into(),
                description: "Only this exact file and reviewed parents.".into(),
                answer: json!({"decision":"accept"}),
            },
        ],
    }
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
            self.cancelled.fetch_max(*generation, Ordering::Release);
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
    trust: crate::providers::agent::PlanningTrust,
    text: String,
    writes: Option<WriteSet>,
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
    let mut threads: HashMap<String, OwnedThread> = HashMap::new();
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
                    arrival: super::approval_arrival(),
                    destination: None,
                    requires_connection_review: true,
                    generation,
                    task_id: input.task.id,
                    thread_id: input.resume.clone(),
                    trust: Vec::new(),
                    payload: String::new(),
                    blockers: Vec::new(),
                };
                if input.inspect_sessions {
                    preview.blockers.push("session/list inspection requires an ACP peer; no Codex turn will be dispatched".into());
                }
                if let Err(error) = input.validate_selection() {
                    preview.blockers.push(safe(&error));
                }
                let trust = crate::providers::agent::planning_trust(project);
                if let Ok(trust) = &trust {
                    preview.requires_connection_review = trust.requires_connection_review()
                        || input.operation == Operation::Implement;
                    preview.trust = trust.lines.iter().map(|line| safe(line)).collect();
                } else if let Err(error) = &trust {
                    preview.blockers.push(safe(error));
                }
                if input.agent != Agent::Codex {
                    preview
                        .blockers
                        .push("Codex worker refuses a different provider identity".into());
                }
                preview.trust.push(format!(
                    "Model: {}; reasoning effort: {}. Account/configuration and paginated metadata preserve exact selection. Configured/custom IDs are sent unchanged even when missing from a stale catalogue; the provider validates actual availability. No automatic model substitution.",
                    input.selection.model.as_deref().unwrap_or("provider configuration, otherwise advertised default"),
                    input.selection.effort.as_deref().unwrap_or("configured effort for configured model, otherwise advertised default"),
                ));
                if input.inspect_models {
                    preview.trust.push("CATALOGUE ONLY: launch the qualified Codex app-server and initialize/account/read/config/read/model/list. Retain only nonsecret configured model/provider/effort. Provider configuration/authentication and network access may be used; no login/auth refresh, thread start/resume, prompt, model turn, edit or journal.".into());
                }
                let writes = capture_writes(project, data_dir, &input, exclusions);
                if let Err(error) = &writes {
                    preview.blockers.push(safe(error));
                }
                if !input.inspect_models && input.operation == Operation::Implement {
                    preview.trust.push("Controlled route: eevee_write_file creates or replaces only exact reviewed selected files after an exact content patch and allow-once. Creation includes only the displayed missing parents. Protected/symlink/hardlink paths and appeared targets/parents are refused. No grantRoot/session grant; native file-write and command escalation denied.".into());
                    preview.trust.push("Provider readOnly/network-disabled policy is acknowledged, not an Eevee process sandbox. Independent provider/config/MCP reads or side effects are outside this tool's coverage. No workspace-write trust configuration is changed.".into());
                    preview.trust.push("Baseline distinguishes absent targets from existing current working bytes (including dirty user edits). No reset/commit/merge. Each creation or replacement is reviewed against the frozen scope; changed baselines require fresh consent.".into());
                    if input.resume.is_some() {
                        preview.blockers.push("Controlled implementation requires a fresh thread; planning/resumed threads do not carry the dynamic write tool".into());
                    }
                }
                if !input.inspect_models && input.operation == Operation::Review {
                    preview.trust.push("REVIEW: fresh requirements/patch/selected-observation context and a new provider thread. No implementation transcript, dynamic edit tool, native file-write approval or command escalation. Read-only is acknowledged provider policy, not an Eevee OS sandbox; independent provider configuration remains disclosed.".into());
                }
                if project.to_str() != Some(input.task.project.as_str()) {
                    preview
                        .blockers
                        .push("Task project differs from the trusted worker project".into());
                }
                if input.resume.as_ref().is_some_and(|id| {
                    threads.get(id).map(|thread| thread.task_id) != Some(input.task.id)
                }) {
                    preview.blockers.push("Resume refused: thread was not started for this task in the current Eevee session".into());
                }
                if !input.inspect_models
                    && threads.len() >= REQUESTS_LIMIT
                    && input.resume.is_none()
                {
                    preview.blockers.push("Session thread bound reached; restart Eevee before creating another thread".into());
                }
                match payload(
                    data_dir,
                    &input,
                    exclusions,
                    writes.as_ref().ok().and_then(Option::as_ref),
                ) {
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
                        writes: writes.unwrap_or_default(),
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
                        "Stale provider consent rejected".into(),
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
fn payload(
    data_dir: &Path,
    input: &Input,
    exclusions: &[PathBuf],
    writes: Option<&WriteSet>,
) -> Result<String, String> {
    input.validate_selection()?;
    if input.inspect_models {
        return serde_json::to_string_pretty(&json!({
            "purpose": "Inspect the complete bounded nonhidden text-model catalogue only; no thread or model turn.",
            "provider": "Codex",
            "requests": ["initialize", "account/read (refreshToken=false)", "config/read (includeLayers=false; retain model/provider/effort only)", "model/list (includeHidden=false, limit=100, at most 64 pages / 4096 entries)"],
            "selection": input.selection,
            "effects": "Qualified app-server launch; provider configuration, account and network access. No task/source payload, thread start/resume, prompt, edit or journal.",
        })).map_err(|_| "Could not encode catalogue inspection preview".into());
    }
    input.validate_review()?;
    input.validate_committed(data_dir)?;
    if let Some(text) = input.review_payload(data_dir, exclusions)? {
        return Ok(text);
    }
    let sources = crate::workspace::context::planning_payload(
        input.task.id,
        input.report.as_ref(),
        &input.excerpts,
        exclusions,
    )?;
    if let Some(document) = &input.handoff {
        crate::workspace::handoff::validate_dispatch(
            document,
            &input.task,
            &input.excerpts,
            input.report.as_ref(),
            exclusions,
        )?;
    }
    let baseline = match (input.operation, writes) {
        (Operation::Implement, Some(writes)) => writes.payload(&input.excerpts),
        (Operation::Implement, None) => {
            return Err("Implementation requires a captured selected write scope".into());
        }
        (_, _) => json!([]),
    };
    let text = serde_json::to_string_pretty(&json!({
        "purpose": if input.operation == Operation::Plan { "Plan the user's task. Do not implement changes. Ask the user when a decision is needed; report proposals as unverified planning, never as local execution evidence." } else { "Implement the approved requirement ONLY by calling eevee_write_file with an exact selected root-relative path and full proposed UTF-8 content. Create only targets captured as absent, including only their reviewed missing parents; replace only targets captured as existing. Preserve every unrelated dirty user change visible in write_baseline. Never use apply_patch, shell writes, MCP, delegation, commits or merge. Native filesystem writes and command escalations will be denied. Ask for user input if unclear. Every proposed creation or replacement requires a displayed exact patch and allow-once. Report results as unverified until the user reviews the working diff and deliberately chooses a local check." },
        "task": { "id": input.task.id, "revision": input.task.revision, "goal": input.task.goal, "acceptance_criteria": input.task.criteria, "next_action": crate::workspace::handoff::without_local_provenance(&input.task.next_action) },
        "request": input.instruction(),
        "operation": input.operation,
        "selection": input.selection,
        "edit_paths": input.edit_paths,
        "write_baseline": baseline,
        "handoff": input.handoff,
        "approved_memories": input.task.approved_memories,
        "selected_sources": serde_json::from_str::<Value>(&sources.text).map_err(|_| "Source payload is not valid JSON")?
    })).map_err(|_| "Could not encode provider payload")?;
    if text.len() > FRAME_LIMIT / 2 {
        return Err("Provider payload exceeds 1 MiB; reduce the source basket/edit files".into());
    }
    if crate::workspace::privacy::redact(&text) != text {
        return Err(
            "Provider payload contains a recognizable credential; edit it before review".into(),
        );
    }
    if text.chars().any(bidi) {
        return Err(
            "Provider payload contains directional display controls; remove them before review"
                .into(),
        );
    }
    Ok(text)
}

/// A complete bounded provider snapshot, never a guessed or cached dispatch default.
#[derive(Default)]
struct ModelListing {
    models: Vec<ModelOption>,
    default: Option<String>,
    pages: usize,
    cursors: HashSet<String>,
    configured_model: Option<String>,
    configured_effort: Option<String>,
    provider: Option<String>,
}
impl ModelListing {
    fn configuration(&mut self, result: &Value) -> Result<(), String> {
        let config = result
            .get("config")
            .filter(|value| value.is_object())
            .ok_or("Protocol config/read missing config object")?;
        // Retain only these nonsecret selection fields, never configuration layers,
        // endpoints, headers, environment values or credentials.
        for (key, target) in [
            ("model", &mut self.configured_model),
            ("model_reasoning_effort", &mut self.configured_effort),
            ("model_provider", &mut self.provider),
        ] {
            *target = match config.get(key) {
                None | Some(Value::Null) => None,
                Some(_) => Some(identity(config, key)?),
            };
        }
        Ok(())
    }

    fn refresh(&mut self) {
        self.models.clear();
        self.default = None;
        self.pages = 0;
        self.cursors.clear();
    }
    fn page(&mut self, result: &Value) -> Result<Option<String>, String> {
        self.pages += 1;
        if self.pages > 64 {
            return Err("Model catalogue pagination bound exceeded".into());
        }
        let models = result
            .get("data")
            .and_then(Value::as_array)
            .ok_or("Protocol model/list missing data")?;
        if models.len() > 100 {
            return Err("Model catalogue page exceeds requested bound".into());
        }
        for model in models {
            if model.get("hidden").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            // Codex's Model schema defaults an omitted inputModalities field
            // to ["text", "image"]; explicit non-text and malformed values
            // must not inherit that default.
            match model.get("inputModalities") {
                None => {}
                Some(Value::Array(modes)) if modes.iter().all(Value::is_string) => {
                    if !modes.iter().any(|mode| mode == "text") {
                        continue;
                    }
                }
                Some(_) => return Err("Protocol model/list has malformed input modalities".into()),
            }
            let id = identity(model, "model")?;
            if self.models.iter().any(|option| option.id == id) {
                return Err("Model catalogue contains duplicate request identities".into());
            }
            if self.models.len() >= 4096 {
                return Err("Model catalogue exceeds the 4096-entry resource bound; no partial catalogue accepted".into());
            }
            let supported = match model.get("supportedReasoningEfforts") {
                None | Some(Value::Null) => &[][..],
                Some(Value::Array(values)) if values.len() <= 32 => values.as_slice(),
                _ => return Err("Model catalogue has malformed reasoning metadata".into()),
            };
            let mut efforts = Vec::with_capacity(supported.len());
            for effort in supported {
                let effort = identity(effort, "reasoningEffort")?;
                if efforts.contains(&effort) {
                    return Err("Model catalogue contains duplicate reasoning efforts".into());
                }
                efforts.push(effort);
            }
            let default_effort = match model.get("defaultReasoningEffort") {
                None | Some(Value::Null) => None,
                Some(_) => Some(identity(model, "defaultReasoningEffort")?),
            };
            if default_effort
                .as_ref()
                .is_some_and(|effort| !efforts.contains(effort))
            {
                return Err("Model layer: advertised default effort is unsupported".into());
            }
            if model.get("isDefault").and_then(Value::as_bool) == Some(true) {
                if self.default.is_some() {
                    return Err("Model catalogue advertises conflicting defaults".into());
                }
                self.default = Some(id.clone());
            }
            let name = catalogue_text(
                model
                    .get("displayName")
                    .and_then(Value::as_str)
                    .unwrap_or(&id),
                256,
            );
            let description = catalogue_text(
                model
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                4096,
            );
            self.models.push(ModelOption {
                id,
                name,
                description,
                efforts,
                default_effort,
            });
        }
        match result.get("nextCursor") {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(cursor)) => {
                if self.pages >= 64
                    || cursor.is_empty()
                    || cursor.len() > 4096
                    || cursor.chars().any(|c| c.is_control() || bidi(c))
                    || !self.cursors.insert(cursor.clone())
                {
                    return Err("Model catalogue pagination bound or cursor cycle rejected".into());
                }
                Ok(Some(cursor.clone()))
            }
            _ => Err("Protocol model/list contains a malformed cursor".into()),
        }
    }
    fn select(&self, selection: &ModelSelection) -> Result<(String, Option<String>), String> {
        let requested = selection
            .model
            .as_ref()
            .or(self.configured_model.as_ref())
            .or(self.default.as_ref())
            .ok_or(
                "Model layer: no configured or advertised default; no guessed model or fallback",
            )?;
        let model = self.models.iter().find(|model| &model.id == requested);
        let effort = selection
            .effort
            .as_ref()
            .or_else(|| {
                (selection.model.as_ref().or(self.configured_model.as_ref())
                    == self.configured_model.as_ref())
                .then_some(self.configured_effort.as_ref())
                .flatten()
            })
            .or_else(|| model.and_then(|model| model.default_effort.as_ref()));
        if let (Some(model), Some(effort)) = (model, effort)
            && !model.efforts.is_empty()
            && !model.efforts.contains(effort)
        {
            return Err("Model layer: requested reasoning effort is unsupported by the selected model; no fallback".into());
        }
        Ok((requested.clone(), effort.cloned()))
    }
    fn revalidate(
        &self,
        selection: &ModelSelection,
        model: &str,
        effort: Option<&str>,
    ) -> Result<(), String> {
        let selected = self.select(selection)?;
        if selected.0 != model || selected.1.as_deref() != effort {
            return Err("Model catalogue changed since thread qualification; review a fresh preview before dispatch".into());
        }
        Ok(())
    }
    fn into_catalogue(self, selection: &ModelSelection) -> ModelCatalogue {
        let current = selection
            .model
            .clone()
            .or(self.configured_model.clone())
            .or(self.default.clone());
        ModelCatalogue {
            models: self.models,
            current,
            source: format!("Codex config/read + paginated model/list; provider: {}; configured model: {}; catalogue default: {}. CLI metadata may be cached. Explicit configured/custom IDs are sent unchanged even when absent from this list; the provider decides availability. For live account discovery without an installed CLI choose Codex's native responses method (no tools).", self.provider.as_deref().unwrap_or("native default"), self.configured_model.as_deref().unwrap_or("unset"), self.default.as_deref().unwrap_or("unreported")),
            effort_note: Some("Reasoning controls are optional. Default preserves configured effort for the configured model, otherwise the advertised default when present. Explicit unsupported advertised efforts fail; unknown custom-model capabilities are validated by the provider.".into()),
        }
    }
}

struct OwnedThread {
    task_id: i64,
    usage: Option<UsageBaseline>,
}

#[derive(Clone)]
struct UsageBaseline {
    turn_id: String,
    counters: Counters,
}

/// Codex's `last` is the latest model response, not an agentic turn total.
/// Both wire counters are snapshots; repeated notifications never add tokens.
#[derive(Default)]
struct UsageSnapshots {
    baseline: Option<UsageBaseline>,
    last: Option<Counters>,
    total: Option<Counters>,
    complete: bool,
}

fn usage_counters(value: &Value) -> Result<Counters, String> {
    if !value.is_object() {
        return Err("Protocol token usage breakdown is not an object".into());
    }
    let count = |key: &str| match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| "Protocol token usage counter is not a nonnegative integer".to_owned()),
    };
    Ok(Counters {
        input: count("inputTokens")?,
        output: count("outputTokens")?,
        cache_read: count("cachedInputTokens")?,
        cache_write: count("cacheWriteInputTokens")?,
        reasoning: count("reasoningOutputTokens")?,
    })
}

fn merge_usage(target: &mut Option<Counters>, update: Counters) {
    if update.is_unknown() {
        return;
    }
    let target = target.get_or_insert_with(Counters::default);
    macro_rules! field {
        ($name:ident) => {
            if update.$name.is_some() {
                target.$name = update.$name;
            }
        };
    }
    field!(input);
    field!(output);
    field!(cache_read);
    field!(cache_write);
    field!(reasoning);
}

impl UsageSnapshots {
    fn observe(
        &mut self,
        params: &Value,
        thread_id: Option<&str>,
        turn_id: Option<&str>,
        dispatched: bool,
    ) -> Result<(), String> {
        if thread_id.is_none() || thread_id != params.get("threadId").and_then(Value::as_str) {
            return Ok(());
        }
        let expected_turn = if dispatched {
            turn_id
        } else {
            self.baseline
                .as_ref()
                .map(|baseline| baseline.turn_id.as_str())
        };
        if expected_turn.is_none() || expected_turn != params.get("turnId").and_then(Value::as_str)
        {
            return Ok(());
        }
        let usage = params
            .get("tokenUsage")
            .filter(|value| value.is_object())
            .ok_or("Protocol token usage notification missing tokenUsage")?;
        let parse = |key: &str| match usage.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => usage_counters(value).map(Some),
        };
        let last = parse("last")?;
        let total = parse("total")?;
        if !dispatched {
            // Only an already-observed historical turn can refresh its baseline.
            // A notification from an unknown resumed turn cannot establish ownership.
            if let Some(total) = total {
                let mut baseline = self.baseline.as_ref().map(|base| base.counters);
                merge_usage(&mut baseline, total);
                if let Some(counters) = baseline {
                    self.baseline.as_mut().unwrap().counters = counters;
                }
            }
            return Ok(());
        }
        if let Some(last) = last {
            merge_usage(&mut self.last, last);
        }
        if let Some(total) = total {
            merge_usage(&mut self.total, total);
        }
        Ok(())
    }

    fn emit(
        &self,
        events: &Events,
        generation: u64,
        task_id: i64,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), String> {
        for (scope, counters) in [(Scope::Response, &self.last), (Scope::Session, &self.total)] {
            let Some(counters) = counters else { continue };
            emit(
                events,
                Event::Usage {
                    generation,
                    task_id,
                    report: Report {
                        identity: Identity {
                            session: Some(thread_id.to_owned()),
                            turn: Some(turn_id.to_owned()),
                            ..Identity::default()
                        },
                        scope,
                        kind: Kind::Snapshot,
                        counters: *counters,
                        baseline: if scope == Scope::Session {
                            self.baseline.as_ref().map(|base| base.counters)
                        } else {
                            None
                        },
                        // The usage notification does not report a model or tokenizer.
                        // Selected/acknowledged thread settings are not response metadata.
                        model: None,
                        tokenizer: None,
                        semantics: Semantics {
                            cache_read_in_input: Some(true),
                            cache_write_in_input: None,
                            reasoning_in_output: Some(true),
                        },
                        complete: self.complete,
                    },
                },
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    Initialize,
    Account,
    Config,
    Models,
    VerifyModels,
    Thread,
    Turn,
    Interrupt,
}
struct Wire {
    bytes: Vec<u8>,
    refusal: bool,
    dispatch_receipt: bool,
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
    process: crate::workspace::runner::native::Process,
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
    threads: &'a mut HashMap<String, OwnedThread>,
    turn_id: Option<String>,
    usage: UsageSnapshots,
    model: String,
    effort: Option<String>,
    account: String,
    catalogue: ModelListing,
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
    writes: Option<WriteSet>,
    pending_write: Option<WriteProposal>,
    journal_session: String,
}
fn run_transport(
    workspace: &Workspace,
    run: &Prepared,
    incoming: &Receiver<Request>,
    events: &Events,
    stop: &AtomicBool,
    cancelled: &AtomicU64,
    threads: &mut HashMap<String, OwnedThread>,
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
        let prepared = crate::providers::agent::qualify_planning(project, &run.trust, stop)?;
        if stop.load(Ordering::Acquire) || cancelled.load(Ordering::Acquire) == run.generation {
            return Err("Stopped before transport launch".into());
        }
        crate::workspace::runner::native::Process::spawn_piped(&prepared).map_err(|_| {
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
        threads,
        turn_id: None,
        usage: UsageSnapshots::default(),
        model: String::new(),
        effort: None,
        account: String::new(),
        catalogue: ModelListing::default(),
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
        writes: run.writes.clone(),
        pending_write: None,
        journal_session: format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ),
    };
    let result: Result<(Outcome, String), String> = (|| {
        engine.status(&format!(
            "Observed Codex CLI {}; initializing app-server (policy/capabilities not yet qualified; no model dispatched)",
            run.trust.observed_version().unwrap_or("unavailable")
        ))?;
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
                        "Stale or overlapping provider request rejected; no replay performed",
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
    let mut result = if run.input.inspect_models {
        result
    } else {
        engine.journal_partial_tools().and(result)
    };
    if let (Some(thread_id), Some(turn_id)) = (&engine.thread_id, &engine.turn_id) {
        result = engine
            .usage
            .emit(
                events,
                run.generation,
                run.input.task.id,
                thread_id,
                turn_id,
            )
            .and(result);
    }
    if let Some(id) = &engine.thread_id {
        let thread = engine
            .threads
            .entry(id.clone())
            .or_insert_with(|| OwnedThread {
                task_id: run.input.task.id,
                usage: None,
            });
        if engine.dispatched {
            // Failed/unknown transport cannot provide a final baseline for a later request.
            thread.usage = if engine.usage.complete {
                engine
                    .turn_id
                    .as_ref()
                    .zip(engine.usage.total.as_ref())
                    .map(|(turn_id, counters)| UsageBaseline {
                        turn_id: turn_id.clone(),
                        counters: *counters,
                    })
            } else {
                None
            };
        }
    }
    engine.close_prompts();
    let tree_exited = engine.teardown();
    match result {
        Ok((outcome, message)) => (
            if tree_exited {
                outcome
            } else {
                Outcome::Unknown
            },
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
    run.input.validate_selection()?;
    if let Some(writes) = &run.writes {
        writes.verify()?;
    }
    if payload(data_dir, &run.input, exclusions, run.writes.as_ref())? != run.text {
        return Err(
            "Outgoing payload changed since review; no dispatch. Review a fresh preview".into(),
        );
    }
    crate::providers::agent::revalidate_planning_trust(project, &run.trust)
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
        let dispatch_receipt = value.get("method").and_then(Value::as_str) == Some("turn/start");
        self.wire.push_back(Wire {
            bytes,
            refusal,
            dispatch_receipt,
        });
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
            if self.cancelling.is_none()
                && self.cancelled.load(Ordering::Acquire) == self.run.generation
            {
                self.cancel()?;
            }
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
                        let frame = self.wire.pop_front().expect("front frame was just written");
                        self.offset = 0;
                        self.write_deadline = (!self.wire.is_empty())
                            .then(|| Instant::now() + Duration::from_secs(5));
                        if frame.dispatch_receipt
                            && let Some(event) = self.run.input.dispatched(self.run.generation)
                        {
                            emit(self.events, event)?;
                        }
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
        if value.get("error").is_some()
            && matches!(call.kind, Call::Models | Call::VerifyModels)
            && !self.run.input.inspect_models
            && (self.run.input.selection.model.is_some()
                || self.catalogue.configured_model.is_some())
        {
            self.status("Model discovery unavailable; preserving the exact explicit/configured selection for provider validation")?;
            return if call.kind == Call::VerifyModels {
                self.catalogue.revalidate(
                    &self.run.input.selection,
                    &self.model,
                    self.effort.as_deref(),
                )?;
                self.dispatch_turn()
            } else {
                self.start_selected_thread()
            };
        }
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
                    _ if self.run.input.inspect_models => "No authenticated account; catalogue metadata is not proof of access".into(),
                    _ => return Err("Authentication layer: no usable provider account; authenticate outside Eevee and review a fresh preview".into()),
                };
                self.call(Call::Config, "config/read", json!({"includeLayers":false,"cwd":self.project}))
            }
            Call::Config => {
                self.catalogue.configuration(result)?;
                self.call(Call::Models, "model/list", json!({"includeHidden":false,"limit":100}))
            }
            Call::Models | Call::VerifyModels => {
                if let Some(cursor) = self.catalogue.page(result)? {
                    return self.call(kind, "model/list", json!({"includeHidden":false,"limit":100,"cursor":cursor}));
                }
                if kind == Call::VerifyModels {
                    self.catalogue.revalidate(&self.run.input.selection, &self.model, self.effort.as_deref())?;
                    return self.dispatch_turn();
                }
                if self.run.input.inspect_models {
                    let mut catalogue = std::mem::take(&mut self.catalogue)
                        .into_catalogue(&self.run.input.selection);
                    catalogue.source = catalogue_text(&format!("{}; {}. {}", self.run.trust.catalogue_source(), self.account, catalogue.source), 2048);
                    catalogue.validate()?;
                    emit(self.events, Event::Models {
                        generation: self.run.generation,
                        task_id: self.run.input.task.id,
                        catalogue,
                    })?;
                    self.terminal = Some((Outcome::Completed, "Model catalogue inspection completed; no thread or model turn dispatched".into()));
                    return Ok(());
                }
                self.start_selected_thread()
            }
            Call::Thread => {
                self.validate_policy(result)?;
                let thread = result.get("thread").ok_or("Protocol thread response missing thread")?;
                let id = identity(thread, "id")?;
                if self.run.input.resume.as_ref().is_some_and(|expected| expected != &id) { return Err("Protocol layer: resume returned a different thread".into()); }
                if self.run.input.resume.is_none() && self.threads.contains_key(&id) {
                    return Err("Provider new thread reused an existing identity; no ownership transfer".into());
                }
                self.thread_id = Some(id.clone());
                self.usage.baseline = self.threads.get(&id).and_then(|thread| thread.usage.clone());
                emit(self.events, Event::Thread { generation: self.run.generation, task_id: self.run.input.task.id, thread_id: id.clone(), model: safe(&self.model), account: self.account.clone() })?;
                // Refresh the complete provider snapshot after startup, before the one turn.
                self.catalogue.refresh();
                self.call(Call::VerifyModels, "model/list", json!({"includeHidden":false,"limit":100}))
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
    fn start_selected_thread(&mut self) -> Result<(), String> {
        revalidate(self.project, self.data_dir, self.exclusions, self.run)?;
        (self.model, self.effort) = self.catalogue.select(&self.run.input.selection)?;
        let mut params = json!({"cwd":self.project,"model":self.model,"sandbox":"read-only","approvalPolicy":"on-request","approvalsReviewer":"user","config":{"model_reasoning_effort":self.effort,"features.multi_agent":false,"web_search":"disabled","allow_login_shell":false}});
        if self.run.input.operation == Operation::Implement {
            params["dynamicTools"] = json!([{"type":"function","name":TOOL_NAME,"description":"The ONLY allowed implementation write route. Propose creating or replacing an exact selected UTF-8 file with full content, preserving unrelated dirty changes. Creation includes only reviewed missing parents. User reviews exact create/replace scope and patch and must allow once. Protected/unselected/stale/link paths and appeared targets/parents refused.","inputSchema":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"],"additionalProperties":false}}]);
        }
        let method = if let Some(id) = &self.run.input.resume {
            params["threadId"] = json!(id);
            params["excludeTurns"] = json!(true);
            "thread/resume"
        } else {
            params["experimentalRawEvents"] = json!(false);
            params["allowProviderModelFallback"] = json!(false);
            "thread/start"
        };
        self.call(Call::Thread, method, params)
    }
    fn dispatch_turn(&mut self) -> Result<(), String> {
        revalidate(self.project, self.data_dir, self.exclusions, self.run)?;
        if self.stop.load(Ordering::Acquire) {
            return Err("Stopped before turn dispatch".into());
        }
        if self.cancelled.load(Ordering::Acquire) == self.run.generation {
            return self.cancel();
        }
        let id = self
            .thread_id
            .as_ref()
            .ok_or("No qualified thread before turn")?;
        self.call(Call::Turn, "turn/start", json!({
            "threadId":id,"input":[{"type":"text","text":self.run.text,"text_elements":[]}],
            "cwd":self.project,"model":self.model,"effort":self.effort,
            "approvalPolicy":"on-request","approvalsReviewer":"user",
            "sandboxPolicy":{"type":"readOnly","networkAccess":false},"summary":"none",
            "collaborationMode":{"mode":if self.run.input.operation == Operation::Plan { "plan" } else { "default" },"settings":{"model":self.model,"reasoning_effort":self.effort,"developer_instructions":null}}
        }))?;
        // Enqueueing is conservatively treated as dispatch: partial writes are outcome-unknown.
        self.dispatched = true;
        self.status(&format!("Turn dispatched once with model {} and reasoning effort {}; provider readOnly acknowledged. Implementation writes only through Eevee's reviewed selected-file tool.", self.model, self.effort.as_deref().unwrap_or("provider default")))
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
            || self.effort.as_deref().is_some_and(|effort| {
                result.get("reasoningEffort").and_then(Value::as_str) != Some(effort)
            })
            || result.get("cwd").and_then(Value::as_str) != self.project.to_str()
        {
            return Err("Policy qualification refused: provider did not acknowledge exact model/reasoning effort/cwd/read-only/user-approval boundaries".into());
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
        if self.run.input.inspect_models {
            if matches!(
                method,
                "modelProvider/authRecoveryStarted" | "model/rerouted"
            ) {
                return Err("Catalogue inspection refused provider authentication recovery or model reroute".into());
            }
            return Ok(());
        }
        if method == "thread/tokenUsage/updated" {
            return self.usage.observe(
                params,
                self.thread_id.as_deref(),
                self.turn_id.as_deref(),
                self.dispatched,
            );
        }
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
                self.pending_write = None;
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
            // the boundary acknowledged by thread/start or the explicitly requested operation.
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
                || self.effort.as_deref().is_some_and(|effort| {
                    settings.get("effort").and_then(Value::as_str) != Some(effort)
                })
                || settings.get("cwd").and_then(Value::as_str) != self.project.to_str()
                || self.dispatched
                    && settings
                        .pointer("/collaborationMode/mode")
                        .and_then(Value::as_str)
                        != Some(if self.run.input.operation == Operation::Plan {
                            "plan"
                        } else {
                            "default"
                        })
                || self.dispatched
                    && (settings
                        .pointer("/collaborationMode/settings/model")
                        .and_then(Value::as_str)
                        != Some(self.model.as_str())
                        || self.effort.as_deref().is_some_and(|effort| {
                            settings
                                .pointer("/collaborationMode/settings/reasoning_effort")
                                .and_then(Value::as_str)
                                != Some(effort)
                        }))
            {
                return Err(
                    "Provider operation/policy changed; dispatch/continuation refused".into(),
                );
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
            // Private reasoning, raw response events and user echoes never enter the
            // display or decision-selection stream. Tool finals use explicit public coverage.
            _ => Ok(()),
        }
    }
    fn full_item(&mut self, item: &Value, complete: bool) -> Result<(), String> {
        let complete = complete && provider_item_terminal(item);
        let kind = match item.get("type").and_then(Value::as_str) {
            Some("agentMessage") => ItemKind::Message,
            Some("plan") => ItemKind::Plan,
            Some("commandExecution") => ItemKind::Tool,
            Some("fileChange") => {
                if complete && item.get("status").and_then(Value::as_str) == Some("completed") {
                    return Err("Policy boundary: provider reported native applied file edits despite readOnly; controlled route did not authorize them".into());
                }
                let status = item
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                return self.item(identity(item, "id")?, ItemKind::Tool,
                    &format!("Native file-write activity: {status}. Eevee denies native file-change approval; only reviewed eevee_write_file writes are permitted."),
                    complete, false);
            }
            Some("collabAgentToolCall" | "subAgentActivity" | "webSearch") => {
                return Err(
                    "Policy boundary: provider attempted disabled delegation/web search".into(),
                );
            }
            Some("mcpToolCall" | "dynamicToolCall") => {
                let contents = item
                    .get("contentItems")
                    .or_else(|| item.pointer("/result/content"))
                    .and_then(Value::as_array);
                let public_text: Vec<&str> = contents
                    .into_iter()
                    .flatten()
                    .filter(|part| {
                        matches!(
                            part.get("type").and_then(Value::as_str),
                            Some("text" | "inputText")
                        )
                    })
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect();
                return self.item(identity(item, "id")?, ItemKind::Tool,
                    &format!("Provider-native {} (public text result only; binary/structured results and arguments omitted): {}",
                        item["type"], serde_json::to_string(&json!({"tool":item.get("tool"),"server":item.get("server"),"status":item.get("status"),"success":item.get("success"),"text":public_text,"error":item.pointer("/error/message")})).map_err(|_| "Cannot encode public tool metadata")?),
                    complete, false);
            }
            _ => return Ok(()),
        };
        let id = identity(item, "id")?;
        if kind == ItemKind::Tool {
            let text = format!(
                "Command: {}\nCwd: {}\nStatus: {}\nExit code: {}\n{}",
                item.get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("(not supplied)"),
                item.get("cwd")
                    .and_then(Value::as_str)
                    .unwrap_or("(not supplied)"),
                item.get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
                item.get("exitCode").unwrap_or(&Value::Null),
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
        // Cumulative context protects credentials split across arbitrary deltas;
        // ordinary words become visible without waiting for a newline.
        let visible = safe_stream(&item.raw, complete || !delta && kind == ItemKind::Tool);
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
                item_id: id.clone(),
                kind,
                text: visible.clone(),
                complete,
            },
        )?;
        if kind == ItemKind::Tool && complete {
            self.journal(&id, "Codex provider-native tool observation", "Public final provider item only; provider-owned execution/sandbox/config effects are not Eevee local execution evidence", &visible, true)?;
        }
        Ok(())
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
        self.usage.complete = true;
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
            "Provider authoritatively completed the turn. Output and patch remain unverified; review working diff and deliberately choose a local check. Provider observations are not local execution evidence".into()
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
    fn journal_partial_tools(&self) -> Result<(), String> {
        for (id, item) in &self.items {
            if item.kind == ItemKind::Tool && !item.complete {
                self.journal(id, "Codex provider-native partial tool observation",
                    "Transport ended without a terminal tool status. Side effects/outcome unknown; not Eevee local execution evidence",
                    &safe(&item.raw), false)?;
            }
        }
        Ok(())
    }
    fn journal(
        &self,
        item_id: &str,
        origin: &str,
        coverage: &str,
        text: &str,
        complete: bool,
    ) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        emit(
            self.events,
            Event::Journal {
                generation: self.run.generation,
                task_id: self.run.input.task.id,
                record: crate::store::ProviderRecord {
                    id: format!(
                        "codex-{}-{}-{:x}",
                        self.journal_session,
                        self.run.generation,
                        Sha256::digest(item_id.as_bytes())
                    ),
                    agent: Agent::Codex,
                    generation: self.run.generation,
                    thread_id: self.thread_id.clone().ok_or("Journal thread missing")?,
                    turn_id: self.turn_id.clone().ok_or("Journal turn missing")?,
                    item_id: item_id.into(),
                    origin: origin.into(),
                    coverage: coverage.into(),
                    text: text.into(),
                    complete,
                    partner: self
                        .run
                        .input
                        .work
                        .as_ref()
                        .map(|claim| claim.owner.clone()),
                },
            },
        )
    }
    fn revalidate_write_session(&self) -> Result<(), String> {
        self.run.input.validate_committed(self.data_dir)?;
        crate::providers::agent::revalidate_planning_trust(self.project, &self.run.trust)?;
        self.writes
            .as_ref()
            .ok_or("Selected write scope missing")?
            .verify()
    }
    fn request_write(&mut self, id: Value, params: &Value) -> Result<(), String> {
        let prepared = (|| -> Result<(Prompt, WriteProposal), String> {
            if self.run.input.operation != Operation::Implement {
                return Err("This operation does not permit dynamic writes".into());
            }
            if !self.prompts.is_empty() || self.pending_write.is_some() {
                return Err("Concurrent write request refused".into());
            }
            self.revalidate_write_session()?;
            let proposal = proposed_write(
                self.writes.as_ref().ok_or("Selected write scope missing")?,
                params,
            )?;
            let prompt = Prompt {
                arrival: super::approval_arrival(),
                request_id: id.clone(),
                thread_id: self.thread_id.clone().unwrap(),
                turn_id: self.turn_id.clone().unwrap(),
                item_id: identity(params, "callId")?,
                kind: write_permission(&proposal),
            };
            Ok((prompt, proposal))
        })();
        match prepared {
            Ok((prompt, proposal)) => {
                emit(
                    self.events,
                    Event::Prompt {
                        generation: self.run.generation,
                        task_id: self.run.input.task.id,
                        request: prompt.clone(),
                    },
                )?;
                self.prompts.push(prompt);
                self.pending_write = Some(proposal);
                Ok(())
            }
            Err(error) => {
                self.queue(json!({"id":id,"result":{"success":false,"contentItems":[{"type":"inputText","text":error}]}}))?;
                self.status("Dynamic write tool call refused; this tool call made no Eevee filesystem changes")
            }
        }
    }
    fn answer_write(
        &mut self,
        generation: u64,
        id: Value,
        answer: Value,
        index: usize,
    ) -> Result<(), String> {
        let proposal = self
            .pending_write
            .take()
            .ok_or("Write request is no longer pending")?;
        let prompt = self.prompts.remove(index);
        let mut success = false;
        let mut effects_notification = Ok(());
        let text = if answer.get("decision").and_then(Value::as_str) == Some("accept") {
            let ready = (|| -> Result<(), String> {
                self.revalidate_write_session()?;
                if self.cancelled.load(Ordering::Acquire) == generation
                    || self.stop.load(Ordering::Acquire)
                {
                    return Err("Cancelled write refused before filesystem effects".into());
                }
                if self.events.lock().len() > EVENTS_LIMIT - 5 {
                    return Err("Write refused before filesystem effects: event queue cannot retain the local result and terminal state".into());
                }
                Ok(())
            })();
            let applied = match ready {
                Ok(()) => self.writes.as_mut().unwrap().apply(&proposal),
                Err(message) => Err(WriteFailure {
                    message,
                    uncertain: false,
                }),
            };
            match applied {
                Ok(result) => {
                    success = true;
                    result.message
                }
                Err(error) => {
                    if error.uncertain {
                        effects_notification = emit(
                            self.events,
                            Event::LocalEffectsUnknown {
                                generation,
                                task_id: self.run.input.task.id,
                                message: error.message.clone(),
                            },
                        );
                        self.terminal = Some((Outcome::Unknown, error.message.clone()));
                    }
                    format!("Reviewed write failed: {}", error.message)
                }
            }
        } else {
            format!(
                "This tool call to {} was denied; it made no Eevee filesystem changes",
                proposal.path.display()
            )
        };
        // Record actual filesystem results even when queuing the provider reply later fails.
        self.journal(&format!("eevee-write-{}", prompt.item_id), "Codex-requested Eevee-controlled file write", "Only exact selected create/replace scope and missing parents are mediated; frozen baseline, path identity, current authority and exact patch revalidated. Provider-owned independent effects outside coverage; not a local check", &safe(&text), true)?;
        effects_notification?;
        self.queue(json!({"id":id,"result":{"success":success,"contentItems":[{"type":"inputText","text":text}]}}))?;
        emit(
            self.events,
            Event::PromptClosed {
                generation,
                request_id: prompt.request_id,
            },
        )?;
        self.status(&text)
    }
    fn refuse(&mut self, id: Value, code: i64, message: &str) -> Result<(), String> {
        self.queue(json!({"id":id,"error":{"code":code,"message":message}}))
    }
    fn server_request(&mut self, id: Value, method: &str, params: &Value) -> Result<(), String> {
        if self.run.input.inspect_models {
            self.refuse(
                id,
                -32601,
                "Catalogue inspection does not permit provider prompts or tool requests",
            )?;
            return Ok(());
        }
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
        self.run.input.validate_committed(self.data_dir)?;
        if method == "item/tool/call" {
            return self.request_write(id, params);
        }
        if method == "item/commandExecution/requestApproval"
            && self.run.input.operation != Operation::Plan
        {
            self.queue(json!({"id":id,"result":{"decision":"decline"}}))?;
            return self.status("Command escalation denied: implementation/review cannot grant command effects; choose an explicitly approved local check separately");
        }
        if method == "item/fileChange/requestApproval" {
            self.queue(json!({"id":id,"result":{"decision":"decline"}}))?;
            return self.status("Native file-edit request denied; no native write grant. Implementation uses the exact Eevee-controlled dynamic file tool");
        }
        if !matches!(
            method,
            "item/commandExecution/requestApproval" | "item/tool/requestUserInput"
        ) {
            self.refuse(id, -32601, "Unsupported server request; no permissions, authentication, unadvertised dynamic tools or MCP elicitation")?;
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
            let shown_reason = safe(&reason);
            let allow_once = params.get("command").is_some_and(Value::is_string)
                && params.get("cwd").is_some_and(Value::is_string)
                && shown_command == command
                && shown_cwd == cwd
                && shown_reason == reason
                && decisions
                    .is_none_or(|decisions| decisions.iter().any(|decision| decision == "accept"));
            PromptKind::Approval {
                command: shown_command,
                cwd: shown_cwd,
                reason: shown_reason,
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
            arrival: super::approval_arrival(),
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
        self.run.input.validate_committed(self.data_dir)?;
        if self.pending_write.is_some() {
            return self.answer_write(generation, id, answer, index);
        }
        if self.run.input.operation != Operation::Implement {
            revalidate(self.project, self.data_dir, self.exclusions, self.run)?;
        } else {
            self.revalidate_write_session()?;
        }
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
        self.pending_write = None;
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
        self.pending_write = None;
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

fn provider_item_terminal(item: &Value) -> bool {
    match item.get("type").and_then(Value::as_str) {
        Some("commandExecution" | "fileChange") => matches!(
            item.get("status").and_then(Value::as_str),
            Some("completed" | "failed" | "declined")
        ),
        Some("mcpToolCall") => matches!(
            item.get("status").and_then(Value::as_str),
            Some("completed" | "failed")
        ),
        Some("dynamicToolCall") => {
            matches!(
                item.get("status").and_then(Value::as_str),
                Some("completed" | "failed")
            ) || item.get("success").is_some_and(Value::is_boolean)
        }
        _ => true,
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
        || crate::workspace::privacy::redact(text) != text
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
                && crate::workspace::privacy::redact(text) == text.as_str() =>
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
        PromptKind::Permission { options, .. } => {
            if options.iter().any(|option| option.answer == *answer) {
                Ok(())
            } else {
                Err("Only the exact currently offered deny/allow-once choice is permitted".into())
            }
        }
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
                if crate::workspace::privacy::redact(text) != text {
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
fn catalogue_text(text: &str, limit: usize) -> String {
    let text = crate::workspace::privacy::redact(bounded(text, limit));
    let mut visible: String = text
        .chars()
        .map(|c| {
            if c.is_control() {
                ' '
            } else if bidi(c) {
                '\u{fffd}'
            } else {
                c
            }
        })
        .collect();
    let end = bounded(&visible, limit).len();
    visible.truncate(end);
    visible
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
