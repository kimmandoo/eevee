//! Tool-free native provider requests. Only the immutable, reviewed HTTP body is dispatched.
pub(crate) mod config;

use std::collections::{HashSet, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::providers::{
    Event, Input, ItemKind, ModelCatalogue, ModelOption, Operation, Outcome, Preview, Request,
};
use config::{Connection, Protocol};
use parking_lot::Mutex;
use serde_json::{Value, json};

const QUEUE_LIMIT: usize = 128;
const QUEUE_RESERVE: usize = 24;
const BODY_LIMIT: usize = 1024 * 1024;
const PAGE_LIMIT: usize = 4 * 1024 * 1024;
const CATALOGUE_LIMIT: usize = 4096;
const PAGE_COUNT: usize = 128;
const FRAME_LIMIT: usize = 2 * 1024 * 1024;
const OUTPUT_LIMIT: usize = 512 * 1024;
const STREAM_LIMIT: usize = 64 * 1024 * 1024;
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

type Events = Arc<Mutex<VecDeque<Event>>>;
pub struct Worker {
    requests: Option<SyncSender<Request>>,
    events: Events,
    stopped: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    current: Arc<AtomicU64>,
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
        let current = Arc::new(AtomicU64::new(0));
        let cancelled = Arc::new(AtomicU64::new(0));
        let (output, stop, active, generation, cancel) = (
            events.clone(),
            stopped.clone(),
            busy.clone(),
            current.clone(),
            cancelled.clone(),
        );
        let thread = thread::Builder::new()
            .name("ev-direct".into())
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
                    generation,
                    cancel,
                );
            })?;
        Ok(Self {
            requests: Some(requests),
            events,
            stopped,
            busy,
            current,
            cancelled,
            thread: Some(thread),
        })
    }
    pub fn try_request(&self, pending: &mut Option<Request>) -> bool {
        let Some(request) = pending.take() else {
            return false;
        };
        if let Request::Cancel { generation } = &request
            && *generation != 0
            && self.current.load(Ordering::Acquire) == *generation
        {
            self.cancelled.store(*generation, Ordering::Release);
        }
        if !matches!(request, Request::Cancel { .. })
            && self.events.lock().len() >= QUEUE_LIMIT - QUEUE_RESERVE
        {
            *pending = Some(request);
            return false;
        }
        let Some(sender) = &self.requests else {
            *pending = Some(request);
            return false;
        };
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
struct Workspace {
    project: PathBuf,
    data_dir: PathBuf,
    exclusions: Vec<PathBuf>,
}
struct Prepared {
    generation: u64,
    input: Box<Input>,
    payload: String,
    fingerprint: String,
}

fn emit(events: &Events, event: Event) -> Result<(), String> {
    let mut queue = events.lock();
    let previous = queue
        .iter_mut()
        .find(|previous| match (&event, &**previous) {
            (
                Event::Item {
                    generation,
                    item_id,
                    ..
                },
                Event::Item {
                    generation: g,
                    item_id: id,
                    ..
                },
            ) => generation == g && item_id == id,
            (Event::Status { generation, .. }, Event::Status { generation: g, .. }) => {
                generation == g
            }
            _ => false,
        });
    if let Some(previous) = previous {
        *previous = event;
        return Ok(());
    }
    let limit = if matches!(event, Event::Item { .. } | Event::Status { .. }) {
        QUEUE_LIMIT - QUEUE_RESERVE
    } else {
        QUEUE_LIMIT
    };
    if queue.len() >= limit {
        return Err("UI event capacity reached; native transport stopped".into());
    }
    queue.push_back(event);
    Ok(())
}
fn finish(events: &Events, generation: u64, task_id: i64, outcome: Outcome, message: &str) {
    let _ = emit(
        events,
        Event::Finished {
            generation,
            task_id,
            outcome,
            message: safe(message, true),
        },
    );
}
fn worker(
    workspace: Workspace,
    incoming: Receiver<Request>,
    events: Events,
    stopped: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    current: Arc<AtomicU64>,
    cancelled: Arc<AtomicU64>,
) {
    let mut prepared: Option<Prepared> = None;
    let mut generation = 0u64;
    let mut owner = None;
    while !stopped.load(Ordering::Acquire) {
        let request = match incoming.recv_timeout(Duration::from_millis(20)) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match request {
            Request::Preview(input) => {
                prepared = None;
                generation = generation.saturating_add(1);
                current.store(generation, Ordering::Release);
                owner = Some((generation, input.task.id));
                let mut preview = Preview { destination: None, generation, task_id: input.task.id, thread_id: None, trust: vec![
                    "Native HTTPS request; no provider executable, project hooks, tools, local file writes, shell commands, or automatic retries. Fresh call only; no session or resume support.".into(),
                    "Only the exact HTTP JSON body below is sent for inference. Credentials are scoped request headers, not prompt context; no credentials are logged or persisted.".into(),
                    "Cancellation closes the local response reader; after dispatch the remote outcome remains unknown, not confirmed stopped. Connect/write/read deadlines are bounded.".into(),
                ], payload: String::new(), blockers: Vec::new() };
                if input.operation == Operation::Implement {
                    preview.trust.push("IMPLEMENT PROPOSAL ONLY: the response may propose patches, but Eevee will not apply them. Use a controlled CLI implementation route for reviewed local edits.".into());
                }
                if input.inspect_models {
                    preview.trust.push("METADATA ONLY: bounded paginated model discovery; no task/source content or inference request. Explicit/configured models are not restricted to the discovered catalogue.".into());
                }
                let result = validate_input(&workspace, &input).and_then(|()| {
                    let connection = config::resolve(input.agent)?;
                    preview.trust.push(connection.description.clone());
                    preview.destination = Some(if input.inspect_models {
                        connection.models_url()
                    } else {
                        connection.inference_url(selected_model(&input, &connection)?)
                    });
                    if !input.inspect_models {
                        preview.trust.push(format!(
                            "POST {}",
                            connection.inference_url(selected_model(&input, &connection)?)
                        ));
                    }
                    let body = payload(&workspace, &input, &connection)?;
                    Ok((body, connection.fingerprint()))
                });
                match result {
                    Ok((body, fingerprint)) => {
                        preview.payload = body.clone();
                        prepared = Some(Prepared {
                            generation,
                            input,
                            payload: body,
                            fingerprint,
                        });
                    }
                    Err(error) => preview.blockers.push(safe(&error, true)),
                }
                let _ = emit(&events, Event::Preview(preview));
                busy.store(false, Ordering::Release);
            }
            Request::Run {
                generation: requested,
            } => {
                let Some(mut run) = prepared.take() else {
                    let task_id = owner
                        .filter(|(g, _)| *g == requested)
                        .map_or(0, |(_, id)| id);
                    finish(
                        &events,
                        requested,
                        task_id,
                        Outcome::Failed,
                        "Run refused: no valid immutable native preview; review again",
                    );
                    busy.store(false, Ordering::Release);
                    continue;
                };
                if requested != run.generation {
                    prepared = Some(run);
                    finish(
                        &events,
                        requested,
                        0,
                        Outcome::Failed,
                        "Stale native consent rejected",
                    );
                    busy.store(false, Ordering::Release);
                    continue;
                }
                owner = None;
                let control = Control {
                    stopped: &stopped,
                    cancelled: &cancelled,
                    generation: requested,
                    deadline: Instant::now() + CALL_TIMEOUT,
                };
                let (outcome, message) = dispatch(&workspace, &mut run, &events, &control);
                finish(&events, requested, run.input.task.id, outcome, &message);
                busy.store(false, Ordering::Release);
            }
            Request::Cancel {
                generation: requested,
            } => {
                if owner.is_some_and(|(g, _)| g == requested) {
                    let (_, id) = owner.take().unwrap();
                    prepared = None;
                    finish(
                        &events,
                        requested,
                        id,
                        Outcome::Cancelled,
                        "Native preview discarded; no request dispatched",
                    );
                }
                busy.store(false, Ordering::Release);
            }
            Request::Answer { generation, .. } => {
                let _ = emit(&events, Event::Error { generation: Some(generation), message: "Native calls have no interactive tool or permission prompts; stale answer rejected".into() });
            }
        }
    }
    busy.store(false, Ordering::Release);
}
fn validate_input(workspace: &Workspace, input: &Input) -> Result<(), String> {
    input.validate_review()?;
    if !input.agent.is_direct() {
        return Err("Native worker refuses a CLI provider identity".into());
    }
    if input.resume.is_some() || input.inspect_sessions {
        return Err(
            "Native providers use fresh requests; session inspection and Resume are unavailable"
                .into(),
        );
    }
    if workspace.project.to_str() != Some(input.task.project.as_str()) {
        return Err("Task project differs from the native worker project".into());
    }
    Ok(())
}
fn payload(
    workspace: &Workspace,
    input: &Input,
    connection: &Connection,
) -> Result<String, String> {
    if input.inspect_models {
        return serde_json::to_string_pretty(&json!({
            "purpose": "Model metadata only; no inference, task content, session, tools or local writes",
            "method": "GET", "url": connection.models_url(), "pagination": "At most 128 pages, 4 MiB per page, 4096 unique model identities; no guessed fallback",
            "configured_model": connection.model, "explicit_model": input.selection.model,
        })).map_err(|_| "Cannot encode model inspection preview".into());
    }
    crate::store::validate_planning_input(&workspace.data_dir, &input.task, &input.excerpts)?;
    let context = if let Some(review) =
        input.review_payload(&workspace.data_dir, &workspace.exclusions)?
    {
        review
    } else {
        let sources = crate::workspace::context::planning_payload(
            input.task.id,
            input.report.as_ref(),
            &input.excerpts,
            &workspace.exclusions,
        )?;
        if let Some(document) = &input.handoff {
            crate::workspace::handoff::validate_dispatch(
                document,
                &input.task,
                &input.excerpts,
                input.report.as_ref(),
                &workspace.exclusions,
            )?;
        }
        serde_json::to_string(&json!({
            "purpose": if input.operation == Operation::Implement { "Propose implementation changes only. No tools or local writes are available. Describe patches for the user to apply; never claim files changed, commands ran, or checks passed." } else { "Plan the user's task only. Do not claim local execution or implementation; ask clarifying questions in the response when necessary." },
            "task": { "id": input.task.id, "revision": input.task.revision, "goal": input.task.goal, "acceptance_criteria": input.task.criteria, "next_action": crate::workspace::handoff::without_local_provenance(&input.task.next_action) },
            "request": input.task.request_text(), "operation": input.operation,
            "proposed_edit_paths_only": input.edit_paths, "handoff": input.handoff,
            "approved_memories": input.task.approved_memories,
            "selected_sources": serde_json::from_str::<Value>(&sources.text).map_err(|_| "Invalid selected source payload")?,
        })).map_err(|_| "Cannot encode native context")?
    };
    if context.len() > BODY_LIMIT || safe(&context, true) != context {
        return Err(
            "Native context is oversized or contains recognizable credentials/display controls"
                .into(),
        );
    }
    let model = selected_model(input, connection)?;
    let body = inference_body(
        connection.protocol,
        model,
        selected_effort(input, connection),
        &context,
    )?;
    let text = serde_json::to_string_pretty(&body).map_err(|_| "Cannot encode native request")?;
    if text.len() > BODY_LIMIT {
        return Err("Native HTTP body exceeds 1 MiB; reduce selected context".into());
    }
    Ok(text)
}
fn selected_model<'a>(input: &'a Input, connection: &'a Connection) -> Result<&'a str, String> {
    input.selection.model.as_deref().or(connection.model.as_deref()).ok_or_else(|| "Select an explicit model or configure a default before inference; discovery never guesses a fallback".into())
}
fn selected_effort<'a>(input: &'a Input, connection: &'a Connection) -> Option<&'a str> {
    input.selection.effort.as_deref().or_else(|| {
        if input.selection.model.is_none() || input.selection.model == connection.model {
            connection.effort.as_deref()
        } else {
            None
        }
    })
}
fn inference_body(
    protocol: Protocol,
    model: &str,
    effort: Option<&str>,
    context: &str,
) -> Result<Value, String> {
    let mut body = match protocol {
        Protocol::Codex => {
            json!({ "model": model, "instructions": "Respond only to the reviewed user input. No tools or local execution are available; never claim local mutations or checks. Return only user-facing output, not private reasoning.", "input": [{"role": "user", "content": [{"type": "input_text", "text": context}]}], "stream": true, "store": false })
        }
        Protocol::Responses => {
            json!({ "model": model, "input": [{"role": "user", "content": [{"type": "input_text", "text": context}]}], "stream": true, "store": false })
        }
        Protocol::ChatCompletions => {
            json!({ "model": model, "messages": [{"role": "user", "content": context}], "stream": true })
        }
        Protocol::Messages => {
            json!({ "model": model, "messages": [{"role": "user", "content": context}], "max_tokens": 16384, "stream": true })
        }
        Protocol::Gemini => json!({ "contents": [{"role": "user", "parts": [{"text": context}]}] }),
    };
    if let Some(effort) = effort {
        match protocol {
            Protocol::Responses | Protocol::Codex => body["reasoning"] = json!({ "effort": effort }),
            Protocol::ChatCompletions => body["reasoning_effort"] = json!(effort),
            Protocol::Messages | Protocol::Gemini => return Err("This native protocol has no qualified effort control; clear the explicit effort selection before Run".into()),
        }
    }
    Ok(body)
}
struct Control<'a> {
    stopped: &'a AtomicBool,
    cancelled: &'a AtomicU64,
    generation: u64,
    deadline: Instant,
}
impl Control<'_> {
    fn cancelled(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
            || self.cancelled.load(Ordering::Acquire) == self.generation
    }
    fn check(&self) -> Result<(), String> {
        if self.cancelled() {
            Err("Native call cancelled locally".into())
        } else if Instant::now() >= self.deadline {
            Err("Native call deadline exceeded".into())
        } else {
            Ok(())
        }
    }
}
fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .hickory_dns(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(CALL_TIMEOUT)
        .build()
        .map_err(|_| "Cannot initialize native HTTPS transport".into())
}
fn http_status(status: u16) -> String {
    match status {
        401 => "HTTP 401: credentials were rejected or expired; update the selected connection's authentication and review again".into(),
        403 => "HTTP 403: account, organization, model or endpoint access denied; check provider permissions".into(),
        404 => "HTTP 404: endpoint or exact model is unavailable; check base URL and model selection".into(),
        429 => "HTTP 429: provider quota or rate limit exceeded; no automatic retry was attempted".into(),
        400 | 422 => format!("HTTP {status}: provider rejected the reviewed request or its explicit model/effort; check supported request settings"),
        300..=399 => format!("HTTP {status}: redirect refused to keep authentication scoped to the reviewed endpoint"),
        _ => format!("HTTP {status}: provider request failed; no response body or credentials retained"),
    }
}
async fn wait_http<T>(
    future: impl Future<Output = Result<T, reqwest::Error>>,
    control: &Control<'_>,
) -> Result<T, String> {
    let mut future = std::pin::pin!(future);
    loop {
        control.check()?;
        // Timing out a borrow preserves the pending operation, avoiding retries/re-dispatch.
        match tokio::time::timeout(Duration::from_millis(20), &mut future).await {
            Ok(result) => return result.map_err(|_| "Native HTTP transport failed or timed out; check connectivity and endpoint. No automatic retry was attempted".into()),
            Err(_) => continue,
        }
    }
}
fn validate_prepared(
    workspace: &Workspace,
    run: &Prepared,
    connection: &Connection,
) -> Result<(), String> {
    if connection.fingerprint() != run.fingerprint {
        return Err(
            "Native connection/authentication changed since review; review a fresh preview".into(),
        );
    }
    if payload(workspace, &run.input, connection)? != run.payload {
        return Err("Reviewed task/context or HTTP body changed; review a fresh preview".into());
    }
    Ok(())
}
fn dispatch(
    workspace: &Workspace,
    run: &mut Prepared,
    events: &Events,
    control: &Control<'_>,
) -> (Outcome, String) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            return (
                Outcome::Failed,
                "Cannot initialize native HTTP runtime; no request dispatched".into(),
            );
        }
    };
    let result = runtime.block_on(dispatch_async(workspace, run, events, control));
    runtime.shutdown_timeout(Duration::from_millis(100));
    result
}
async fn dispatch_async(
    workspace: &Workspace,
    run: &mut Prepared,
    events: &Events,
    control: &Control<'_>,
) -> (Outcome, String) {
    let validated: Result<Connection, String> = (|| {
        control.check()?;
        validate_input(workspace, &run.input)?;
        let connection = config::resolve(run.input.agent)?;
        validate_prepared(workspace, run, &connection)?;
        connection.revalidate()?;
        control.check()?;
        Ok(connection)
    })();
    let connection = match validated {
        Ok(connection) => connection,
        Err(error) => {
            return (
                if control.cancelled() {
                    Outcome::Cancelled
                } else {
                    Outcome::Failed
                },
                format!("{error}; no HTTP request dispatched"),
            );
        }
    };
    let http = match http_client() {
        Ok(http) => http,
        Err(error) => return (Outcome::Failed, error),
    };
    if run.input.inspect_models {
        return match discover(
            &http,
            &connection,
            run.input.selection.model.as_deref(),
            control,
        )
        .await
        .and_then(|catalogue| {
            emit(
                events,
                Event::Models {
                    generation: run.generation,
                    task_id: run.input.task.id,
                    catalogue,
                },
            )
        }) {
            Ok(()) => (
                Outcome::Completed,
                "Native model metadata inspection completed; no inference dispatched".into(),
            ),
            Err(error) => (
                if control.cancelled() {
                    Outcome::Cancelled
                } else {
                    Outcome::Failed
                },
                error,
            ),
        };
    }
    let model = match selected_model(&run.input, &connection) {
        Ok(model) => model,
        Err(error) => return (Outcome::Failed, error),
    };
    let _ = emit(
        events,
        Event::Status {
            generation: run.generation,
            task_id: run.input.task.id,
            message: format!(
                "Dispatching fresh tool-free native request: requested model {model}; effort {}. No local writes.",
                selected_effort(&run.input, &connection).unwrap_or("provider default (not sent)")
            ),
        },
    );
    // This is the dispatch boundary: cancellation or failure afterward cannot prove the remote outcome.
    let response = connection
        .request(&http, "POST", &connection.inference_url(model))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .body(std::mem::take(&mut run.payload))
        .send();
    let mut response = match wait_http(response, control).await {
        Ok(response) => response,
        Err(error) => {
            return (
                Outcome::Unknown,
                format!("{error}; remote outcome is unknown after dispatch"),
            );
        }
    };
    if !response.status().is_success() {
        let status = response.status().as_u16();
        return (
            if (400..500).contains(&status) {
                Outcome::Failed
            } else {
                Outcome::Unknown
            },
            http_status(status),
        );
    }
    // Some provider gateways omit Content-Type even for valid SSE. The bounded
    // semantic stream parser, not that optional header, establishes completion.
    let mut stream = Stream::new(connection.protocol);
    let result = stream
        .read(&mut response, control, |stream| stream.publish(events, run))
        .await;
    match result {
        Ok(()) => {
            if let Err(error) = stream.publish_final(events, run) {
                return (Outcome::Unknown, error);
            }
            let observed = stream
                .model
                .as_deref()
                .unwrap_or("unobserved (provider supplied no model identity)");
            (
                Outcome::Completed,
                format!(
                    "Native response completed; requested model {model}; observed model {observed}. {}",
                    if run.input.operation == Operation::Implement {
                        "Proposal only; no files changed or tools executed."
                    } else {
                        "No local writes or tools executed."
                    }
                ),
            )
        }
        Err(error) => {
            let _ = stream.publish_partial(events, run);
            (
                if stream.failed {
                    Outcome::Failed
                } else {
                    Outcome::Unknown
                },
                format!(
                    "{error}; {}",
                    if control.cancelled() {
                        "local reader closed; remote cancellation/outcome is not confirmed"
                    } else {
                        "remote outcome may be incomplete; no automatic retry"
                    }
                ),
            )
        }
    }
}

fn checked_identity(text: &str) -> Result<String, String> {
    if text.is_empty()
        || text.len() > 256
        || safe(text, true) != text
        || text.chars().any(char::is_control)
    {
        return Err("Provider supplied an invalid model identity".into());
    }
    Ok(text.into())
}
fn metadata(text: Option<&str>, fallback: &str, limit: usize) -> String {
    let text = safe(text.unwrap_or(fallback), true).replace(['\n', '\t'], " ");
    bounded(&text, limit).to_string()
}
fn model_option(value: &Value, protocol: Protocol) -> Result<ModelOption, String> {
    let id = if let Some(text) = value.as_str() {
        checked_identity(text)?
    } else {
        let field = match protocol {
            Protocol::Gemini => "name",
            Protocol::Codex => "slug",
            _ => "id",
        };
        checked_identity(
            value
                .get(field)
                .or_else(|| value.get("id"))
                .or_else(|| value.get("model"))
                .and_then(Value::as_str)
                .ok_or("Model metadata entry has no request identity")?,
        )?
    };
    let mut name = metadata(
        value
            .get("display_name")
            .or_else(|| value.get("displayName"))
            .or_else(|| value.get("name"))
            .and_then(Value::as_str),
        &id,
        512,
    );
    if name.is_empty() {
        name = id.clone();
    }
    let description = metadata(value.get("description").and_then(Value::as_str), "", 4096);
    let mut efforts = Vec::new();
    if let Some(options) = value
        .get("supported_reasoning_levels")
        .or_else(|| value.get("supportedReasoningEfforts"))
        .and_then(Value::as_array)
    {
        for option in options.iter().take(32) {
            if let Some(effort) = option.as_str().or_else(|| {
                option
                    .get("effort")
                    .or_else(|| option.get("reasoningEffort"))
                    .and_then(Value::as_str)
            }) {
                let effort = checked_identity(effort)?;
                if !efforts.contains(&effort) {
                    efforts.push(effort);
                }
            }
        }
    }
    let default_effort = value
        .get("default_reasoning_level")
        .or_else(|| value.get("defaultReasoningEffort"))
        .and_then(Value::as_str)
        .filter(|effort| efforts.iter().any(|v| v == effort))
        .map(str::to_string);
    Ok(ModelOption {
        id,
        name,
        description,
        efforts,
        default_effort,
    })
}
fn catalogue_page(
    value: &Value,
    protocol: Protocol,
) -> Result<(&Vec<Value>, Option<String>), String> {
    let models = value
        .as_array()
        .or_else(|| value.get("data").and_then(Value::as_array))
        .or_else(|| value.get("models").and_then(Value::as_array))
        .ok_or("Model metadata response has no models array; no fallback catalogue accepted")?;
    let next = match protocol {
        Protocol::Messages if value.get("has_more").and_then(Value::as_bool) == Some(true) => Some(
            value
                .get("last_id")
                .and_then(Value::as_str)
                .ok_or("Anthropic model page missing last_id")?
                .to_string(),
        ),
        Protocol::Gemini => value
            .get("nextPageToken")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string),
        _ => value
            .get("next_cursor")
            .or_else(|| value.get("nextCursor"))
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string),
    };
    if next
        .as_ref()
        .is_some_and(|value| value.len() > 4096 || value.chars().any(char::is_control))
    {
        return Err("Invalid model pagination cursor".into());
    }
    Ok((models, next))
}
async fn discover(
    http: &reqwest::Client,
    connection: &Connection,
    explicit: Option<&str>,
    control: &Control<'_>,
) -> Result<ModelCatalogue, String> {
    discover_pages(connection, explicit, control, |cursor| async move {
        control.check()?;
        connection.revalidate()?;
        let mut request = connection
            .request(http, "GET", &connection.models_url())
            .header("Accept", "application/json");
        if let Some(cursor) = cursor {
            let key = match connection.protocol {
                Protocol::Messages => "after_id",
                Protocol::Gemini => "pageToken",
                _ => "cursor",
            };
            request = request.query(&[(key, cursor)]);
        }
        let mut response = wait_http(request.send(), control).await?;
        if !response.status().is_success() {
            return Err(http_status(response.status().as_u16()));
        }
        let bytes = read_bounded(&mut response, PAGE_LIMIT, control).await?;
        serde_json::from_slice(&bytes).map_err(|_| {
            "Provider models response is not valid JSON; no fallback catalogue accepted".into()
        })
    })
    .await
}
async fn discover_pages<F, Fut>(
    connection: &Connection,
    explicit: Option<&str>,
    control: &Control<'_>,
    mut fetch: F,
) -> Result<ModelCatalogue, String>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    let mut models = Vec::new();
    let mut identities = HashSet::new();
    let mut cursors = HashSet::new();
    let mut cursor: Option<String> = None;
    let mut completed = false;
    let mut omitted = 0usize;
    for _ in 0..PAGE_COUNT {
        control.check()?;
        let value = fetch(cursor.clone()).await?;
        let (entries, next) = catalogue_page(&value, connection.protocol)?;
        for entry in entries {
            let hidden = entry.get("hidden").and_then(Value::as_bool) == Some(true)
                || entry.get("visibility").and_then(Value::as_str) == Some("hide");
            let nontext = entry
                .get("input_modalities")
                .or_else(|| entry.get("inputModalities"))
                .and_then(Value::as_array)
                .is_some_and(|modes| !modes.iter().any(|mode| mode == "text"));
            let non_generating = connection.protocol == Protocol::Gemini
                && entry
                    .get("supportedGenerationMethods")
                    .and_then(Value::as_array)
                    .is_some_and(|methods| {
                        !methods.iter().any(|method| {
                            method == "generateContent" || method == "streamGenerateContent"
                        })
                    });
            if hidden || nontext || non_generating {
                omitted += 1;
                continue;
            }
            let option = match model_option(entry, connection.protocol) {
                Ok(option) => option,
                Err(_) => {
                    omitted += 1;
                    continue;
                }
            };
            if identities.insert(option.id.clone()) {
                if models.len() == CATALOGUE_LIMIT {
                    return Err(
                        "Model catalogue exceeds 4096 entries; incomplete catalogue rejected"
                            .into(),
                    );
                }
                models.push(option);
            }
        }
        match next {
            Some(next) if cursors.insert(next.clone()) => cursor = Some(next),
            Some(_) => {
                return Err(
                    "Model catalogue pagination repeated a cursor; incomplete catalogue rejected"
                        .into(),
                );
            }
            None => {
                completed = true;
                break;
            }
        }
    }
    if !completed {
        return Err(
            "Model catalogue exceeds pagination bound; incomplete catalogue rejected".into(),
        );
    }
    // Configuration and deliberate custom IDs remain selectable even when not advertised.
    for id in connection.model.as_deref().into_iter().chain(explicit) {
        let id = checked_identity(id)?;
        if identities.insert(id.clone()) {
            if models.len() == CATALOGUE_LIMIT {
                return Err("Configured/custom model exceeds catalogue capacity".into());
            }
            models.push(ModelOption {
                name: id.clone(),
                id,
                description: "Explicit/configured model; not advertised by this metadata response"
                    .into(),
                efforts: Vec::new(),
                default_effort: None,
            });
        }
    }
    let catalogue = ModelCatalogue { models, current: connection.model.clone(), source: format!("{} native model metadata; {omitted} hidden, non-text or malformed entries omitted", connection.description), effort_note: Some("Explicit effort, or the configured effort for its configured model, is sent unchanged. Discovery is informational, not a request allowlist; the provider validates custom IDs and reasoning values.".into()) };
    catalogue.validate()?;
    Ok(catalogue)
}
async fn read_bounded(
    response: &mut reqwest::Response,
    limit: usize,
    control: &Control<'_>,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    loop {
        let Some(chunk) = wait_http(response.chunk(), control).await? else {
            return Ok(bytes);
        };
        if bytes.len() + chunk.len() > limit {
            return Err("Native HTTP response exceeds bounded byte capacity".into());
        }
        bytes.extend_from_slice(&chunk);
    }
}

/// SSE framing retains at most one bounded event; private reasoning and tool arguments are never extracted.
#[derive(Default)]
struct Sse {
    pending: Vec<u8>,
    data: String,
    event: String,
}
impl Sse {
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
        let mut events = Vec::new();
        for byte in bytes {
            if *byte != b'\n' {
                if self.pending.len() + self.data.len() >= FRAME_LIMIT {
                    return Err("Native SSE event exceeds 2 MiB".into());
                }
                self.pending.push(*byte);
                continue;
            }
            let line = std::str::from_utf8(&self.pending)
                .map_err(|_| "Native SSE contains invalid UTF-8")?
                .trim_end_matches('\r');
            if line.is_empty() {
                if !self.data.is_empty() {
                    self.data.pop();
                    events.push((
                        std::mem::take(&mut self.event),
                        std::mem::take(&mut self.data),
                    ));
                } else {
                    self.event.clear();
                }
            } else if let Some(value) = line.strip_prefix("data:") {
                self.data.push_str(value.strip_prefix(' ').unwrap_or(value));
                self.data.push('\n');
            } else if let Some(value) = line.strip_prefix("event:") {
                self.event = value.trim_start_matches(' ').to_string();
            }
            self.pending.clear();
        }
        Ok(events)
    }
}
struct Stream {
    protocol: Protocol,
    text: String,
    model: Option<String>,
    model_published: bool,
    complete: bool,
    failed: bool,
    chat_finished: bool,
}
impl Stream {
    fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            text: String::new(),
            model: None,
            model_published: false,
            complete: false,
            failed: false,
            chat_finished: false,
        }
    }
    fn append(&mut self, text: &str) -> Result<(), String> {
        if self.text.len() + text.len() > OUTPUT_LIMIT {
            return Err(
                "Native response exceeds 512 KiB output capacity; partial evidence retained".into(),
            );
        }
        self.text.push_str(text);
        Ok(())
    }
    fn observe(&mut self, value: Option<&str>) -> Result<(), String> {
        if let Some(model) = value {
            let model = checked_identity(model)?;
            if self
                .model
                .as_ref()
                .is_some_and(|previous| previous != &model)
            {
                return Err(
                    "Provider changed the observed model identity within one response".into(),
                );
            }
            self.model = Some(model);
        }
        Ok(())
    }
    fn event(&mut self, event: &str, data: &str) -> Result<(), String> {
        if data.trim() == "[DONE]" {
            if matches!(self.protocol, Protocol::ChatCompletions) && self.chat_finished {
                self.complete = true;
                return Ok(());
            }
            return Err("Provider ended stream without an authoritative successful finish".into());
        }
        let value: Value =
            serde_json::from_str(data).map_err(|_| "Native SSE data is not valid JSON")?;
        let kind = value.get("type").and_then(Value::as_str).unwrap_or(event);
        if value.get("error").is_some_and(|v| !v.is_null()) || kind == "error" {
            self.failed = true;
            return Err("Provider reported an inference error; private error body omitted".into());
        }
        match self.protocol {
            Protocol::Codex | Protocol::Responses => {
                self.observe(
                    value
                        .get("response")
                        .and_then(|v| v.get("model"))
                        .or_else(|| value.get("model"))
                        .and_then(Value::as_str),
                )?;
                match kind {
                    "response.output_text.delta" | "response.refusal.delta" => {
                        if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                            self.append(delta)?;
                        }
                    }
                    "response.completed" => {
                        let response = value
                            .get("response")
                            .ok_or("Responses completion missing response")?;
                        if response
                            .get("status")
                            .and_then(Value::as_str)
                            .is_some_and(|v| v != "completed")
                        {
                            self.failed = true;
                            return Err("Provider response did not complete successfully".into());
                        }
                        if response.get("output").is_some_and(Value::is_array) {
                            let streamed = std::mem::take(&mut self.text);
                            self.responses_output(response)?;
                            if self.text.is_empty() {
                                self.text = streamed;
                            }
                        }
                        self.complete = true;
                    }
                    "response.failed" | "response.incomplete" => {
                        self.failed = true;
                        return Err("Provider response failed or was incomplete; private diagnostics omitted".into());
                    }
                    _ => {}
                }
            }
            Protocol::ChatCompletions => {
                self.observe(value.get("model").and_then(Value::as_str))?;
                if let Some(choices) = value.get("choices").and_then(Value::as_array) {
                    for choice in choices {
                        if choice.get("index").and_then(Value::as_u64).unwrap_or(0) != 0 {
                            continue;
                        }
                        if let Some(text) = choice
                            .get("delta")
                            .and_then(|v| v.get("content"))
                            .and_then(Value::as_str)
                        {
                            self.append(text)?;
                        }
                        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                            if reason != "stop" {
                                self.failed = true;
                                return Err("Chat response stopped with a non-success finish reason (length, filter, tools or error)".into());
                            }
                            self.chat_finished = true;
                        }
                    }
                }
            }
            Protocol::Messages => match kind {
                "message_start" => self.observe(
                    value
                        .get("message")
                        .and_then(|v| v.get("model"))
                        .and_then(Value::as_str),
                )?,
                "content_block_start" => {
                    if let Some(block) = value.get("content_block")
                        && block.get("type").and_then(Value::as_str) == Some("text")
                        && let Some(text) = block.get("text").and_then(Value::as_str)
                    {
                        self.append(text)?;
                    }
                }
                "content_block_delta" => {
                    if let Some(delta) = value.get("delta")
                        && delta.get("type").and_then(Value::as_str) == Some("text_delta")
                        && let Some(text) = delta.get("text").and_then(Value::as_str)
                    {
                        self.append(text)?;
                    }
                }
                "message_delta" => {
                    if let Some(reason) = value
                        .get("delta")
                        .and_then(|v| v.get("stop_reason"))
                        .and_then(Value::as_str)
                        && !matches!(reason, "end_turn" | "stop_sequence")
                    {
                        self.failed = true;
                        return Err(
                            "Anthropic response ended with a non-success stop reason".into()
                        );
                    }
                }
                "message_stop" => self.complete = true,
                _ => {}
            },
            Protocol::Gemini => {
                self.observe(value.get("modelVersion").and_then(Value::as_str))?;
                if value
                    .get("promptFeedback")
                    .and_then(|v| v.get("blockReason"))
                    .is_some()
                {
                    self.failed = true;
                    return Err("Gemini blocked the request; private diagnostics omitted".into());
                }
                if let Some(candidates) = value.get("candidates").and_then(Value::as_array) {
                    for candidate in candidates {
                        if candidate.get("index").and_then(Value::as_u64).unwrap_or(0) != 0 {
                            continue;
                        }
                        if let Some(parts) = candidate
                            .get("content")
                            .and_then(|v| v.get("parts"))
                            .and_then(Value::as_array)
                        {
                            for part in parts {
                                if part.get("thought").and_then(Value::as_bool) == Some(true) {
                                    continue;
                                }
                                if let Some(text) = part.get("text").and_then(Value::as_str) {
                                    self.append(text)?;
                                }
                            }
                        }
                        if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str)
                        {
                            if reason != "STOP" {
                                self.failed = true;
                                return Err(
                                    "Gemini response ended with a non-success finish reason".into(),
                                );
                            }
                            self.complete = true;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn responses_output(&mut self, response: &Value) -> Result<(), String> {
        if let Some(output) = response.get("output").and_then(Value::as_array) {
            for item in output {
                if item.get("type").and_then(Value::as_str) != Some("message") {
                    continue;
                }
                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for part in content {
                        let text = match part.get("type").and_then(Value::as_str) {
                            Some("output_text") => part.get("text"),
                            Some("refusal") => part.get("refusal"),
                            _ => None,
                        }
                        .and_then(Value::as_str);
                        if let Some(text) = text {
                            self.append(text)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    async fn read(
        &mut self,
        response: &mut reqwest::Response,
        control: &Control<'_>,
        mut publish: impl FnMut(&mut Self) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut sse = Sse::default();
        let mut total = 0usize;
        loop {
            let Some(chunk) = wait_http(response.chunk(), control).await? else {
                return if self.complete {
                    Ok(())
                } else {
                    Err("Native SSE ended without an authoritative completion; partial response only".into())
                };
            };
            total += chunk.len();
            if total > STREAM_LIMIT {
                return Err("Native SSE stream exceeds byte capacity".into());
            }
            for (event, data) in sse.feed(&chunk)? {
                control.check()?;
                self.event(&event, &data)?;
                publish(self)?;
                if self.complete {
                    return Ok(());
                }
            }
        }
    }
    fn publish_model(&mut self, events: &Events, run: &Prepared) -> Result<(), String> {
        if !self.model_published
            && let Some(model) = &self.model
        {
            emit(
                events,
                Event::ModelObserved {
                    generation: run.generation,
                    task_id: run.input.task.id,
                    model: model.clone(),
                },
            )?;
            self.model_published = true;
        }
        Ok(())
    }
    fn publish(&mut self, events: &Events, run: &Prepared) -> Result<(), String> {
        self.publish_model(events, run)?;
        self.publish_text(events, run, false)
    }
    fn publish_text(&self, events: &Events, run: &Prepared, complete: bool) -> Result<(), String> {
        if self.text.is_empty() {
            return Ok(());
        }
        let text = safe(&self.text, complete);
        let text = if run.input.operation == Operation::Implement {
            format!("IMPLEMENT PROPOSAL ONLY — no local files changed or tools executed.\n\n{text}")
        } else {
            text
        };
        emit(
            events,
            Event::Item {
                generation: run.generation,
                task_id: run.input.task.id,
                thread_id: String::new(),
                turn_id: String::new(),
                item_id: format!("native-response-{}", run.generation),
                kind: if run.input.operation == Operation::Plan {
                    ItemKind::Plan
                } else {
                    ItemKind::Message
                },
                text,
                complete,
            },
        )
    }
    fn publish_partial(&mut self, events: &Events, run: &Prepared) -> Result<(), String> {
        self.publish_model(events, run)?;
        // Keep the privacy redactor's incomplete-tail protection after interrupted streams.
        self.publish_text(events, run, false)
    }
    fn publish_final(&mut self, events: &Events, run: &Prepared) -> Result<(), String> {
        self.publish_model(events, run)?;
        self.publish_text(events, run, true)
    }
}
fn safe(text: &str, complete: bool) -> String {
    // Sanitize the cumulative text, not individual deltas: credentials and escape sequences can span chunks.
    crate::workspace::privacy::redact_stream(text, complete).chars().map(|c| {
        if (c.is_control() && c != '\n' && c != '\t') || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') { '\u{fffd}' } else { c }
    }).collect()
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
