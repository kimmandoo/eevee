//! Optional Claude Agent SDK host. No Node process exists until reviewed Run.
//! SDK tool policy/selected-path checks are external agent policy, not an OS sandbox.
use crate::providers::{
    Agent, Event, FilePhase, Input, ItemKind, ModelCatalogue, ModelOption, Operation, Outcome,
    PermissionOption, Preview, Prompt, PromptKind, Question, QuestionOption, Request,
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const FRAME_LIMIT: usize = 2 * 1024 * 1024;
const EVENTS_LIMIT: usize = 256;
const OUTPUT_LIMIT: usize = 512 * 1024;
const ITEM_LIMIT: usize = 32 * 1024;
const APPROVAL_LIMIT: usize = 1024 * 1024;
type Events = Arc<Mutex<VecDeque<Event>>>;
struct Workspace {
    project: PathBuf,
    data_dir: PathBuf,
    exclusions: Vec<PathBuf>,
}
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
        let (out, stop, active, cancel) = (
            events.clone(),
            stopped.clone(),
            busy.clone(),
            cancelled.clone(),
        );
        let thread = thread::Builder::new()
            .name("ev-claude-sdk".into())
            .spawn(move || {
                worker(
                    Workspace {
                        project,
                        data_dir,
                        exclusions,
                    },
                    incoming,
                    out,
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
        if let Request::Cancel { generation } = &request {
            self.cancelled.fetch_max(*generation, Ordering::Release);
        }
        if !matches!(request, Request::Cancel { .. })
            && self.events.lock().len() >= EVENTS_LIMIT - 32
        {
            *pending = Some(request);
            return false;
        }
        let marks_busy = matches!(request, Request::Preview(_) | Request::Run { .. });
        let previous = marks_busy && self.busy.swap(true, Ordering::AcqRel);
        let Some(sender) = &self.requests else {
            *pending = Some(request);
            return false;
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
        // Owned native process teardown is bounded; joining cannot leak an SDK worker.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn emit(events: &Events, event: Event) -> Result<(), String> {
    let mut queue = events.lock();
    if let Event::Item { generation, item_id, .. } = &event
        && let Some(old) = queue.iter_mut().find(|old| matches!(old, Event::Item { generation: g, item_id: id, .. } if g == generation && id == item_id)) {
        *old = event; return Ok(());
    }
    let limit = if matches!(
        event,
        Event::Item { .. } | Event::Status { .. } | Event::Journal { .. }
    ) {
        EVENTS_LIMIT - 24
    } else {
        EVENTS_LIMIT - 1
    };
    if queue.len() >= limit {
        return Err(
            "Claude UI backpressure bound reached; stopping without dropping permission requests"
                .into(),
        );
    }
    queue.push_back(event);
    Ok(())
}
fn finish(events: &Events, generation: u64, task_id: i64, outcome: Outcome, message: String) {
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
fn safe(text: &str) -> String {
    safe_stream(text, true)
}
fn safe_stream(text: &str, complete: bool) -> String {
    let normalized: String = text.chars().filter(|c| (!c.is_control() || matches!(c, '\n' | '\t')) &&
        !matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')).collect();
    crate::workspace::privacy::redact_stream(&normalized, complete)
        .chars()
        .take(ITEM_LIMIT)
        .collect()
}
fn env_present(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
}
/// Profile written by `ant auth login` when ANTHROPIC_PROFILE does not select another.
pub(crate) const CONSOLE_PROFILE: &str = "eevee";

/// Selected Anthropic Console OAuth profile name (ant CLI semantics), if it is safe as argv.
pub(crate) fn console_profile_name() -> Option<String> {
    let name = std::env::var("ANTHROPIC_PROFILE")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| CONSOLE_PROFILE.into());
    (name.len() <= 64
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-')))
    .then_some(name)
}

/// ant CLI configuration directory: ANTHROPIC_CONFIG_DIR, APPDATA (Windows),
/// XDG_CONFIG_HOME, then HOME/.config.
pub(crate) fn console_config_dir() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let dir = var("ANTHROPIC_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            cfg!(windows)
                .then(|| var("APPDATA").map(|p| PathBuf::from(p).join("Anthropic")))
                .flatten()
        })
        .or_else(|| var("XDG_CONFIG_HOME").map(|p| PathBuf::from(p).join("anthropic")))
        .or_else(|| var("HOME").map(|p| PathBuf::from(p).join(".config/anthropic")))?;
    dir.is_absolute().then_some(dir)
}

/// Credential file of a logged-in Console profile. Metadata only; contents are never read.
pub(crate) fn console_credentials() -> Option<PathBuf> {
    let name = console_profile_name()?;
    let dir = console_config_dir()?;
    let regular = |path: &Path| fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file());
    let credentials = dir.join("credentials").join(format!("{name}.json"));
    (regular(&dir.join("configs").join(format!("{name}.json"))) && regular(&credentials))
        .then_some(credentials)
}

fn credential_route() -> Result<String, String> {
    let profile = console_credentials().and(console_profile_name());
    if profile.is_none() && env_present("ANTHROPIC_PROFILE") {
        return Err("Selected ANTHROPIC_PROFILE has no Anthropic Console login; connect it with ant auth login or unset ANTHROPIC_PROFILE".into());
    }
    credential_route_from(
        env_present,
        |name, value| std::env::var(name).ok().as_deref() == Some(value),
        profile.as_deref(),
    )
}
fn credential_route_from(
    present: impl Fn(&str) -> bool,
    configured: impl Fn(&str, &str) -> bool,
    profile: Option<&str>,
) -> Result<String, String> {
    let api = present("ANTHROPIC_API_KEY");
    let bedrock = configured("CLAUDE_CODE_USE_BEDROCK", "1");
    let vertex = configured("CLAUDE_CODE_USE_VERTEX", "1");
    let chain = configured("EEVEE_CLAUDE_CLOUD_AUTH", "provider-chain");
    if u8::from(api) + u8::from(bedrock) + u8::from(vertex) + u8::from(profile.is_some()) != 1 {
        return Err("Claude Agent requires exactly one permitted route: ANTHROPIC_API_KEY, an Anthropic Console OAuth profile (Connect > ant auth login), or an explicitly configured Bedrock/Vertex route. Installed claude.ai subscription login is intentionally unavailable.".into());
    }
    if present("EEVEE_CLAUDE_CLOUD_AUTH") && (!chain || api || profile.is_some()) {
        return Err("EEVEE_CLAUDE_CLOUD_AUTH=provider-chain applies only to an explicit Bedrock or Vertex route, never subscription/API-key/Console-profile auth".into());
    }
    if let Some(name) = profile {
        return Ok(format!(
            "Anthropic Console OAuth profile {name} via ant auth login (API billing; validity unknown until provider response)"
        ));
    }
    if !chain && configured("GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES", "1") {
        return Err("Executable cloud credentials require explicit EEVEE_CLAUDE_CLOUD_AUTH=provider-chain consent".into());
    }
    if bedrock {
        if chain {
            return Ok("Amazon Bedrock (explicit provider credential chain)".into());
        }
        if present("AWS_BEARER_TOKEN_BEDROCK") {
            return Ok("Amazon Bedrock (provider-owned bearer credential)".into());
        }
        if [
            "AWS_PROFILE",
            "AWS_DEFAULT_PROFILE",
            "AWS_CONFIG_FILE",
            "AWS_SHARED_CREDENTIALS_FILE",
        ]
        .iter()
        .any(|name| present(name))
        {
            return Err("AWS profile/config discovery requires EEVEE_CLAUDE_CLOUD_AUTH=provider-chain; credential_process is executable external policy, not implicitly trusted".into());
        }
        if present("AWS_ACCESS_KEY_ID") && present("AWS_SECRET_ACCESS_KEY") {
            return Ok("Amazon Bedrock (provider-owned AWS credentials)".into());
        }
        if present("AWS_WEB_IDENTITY_TOKEN_FILE") && present("AWS_ROLE_ARN") {
            return Ok("Amazon Bedrock (provider-owned web identity)".into());
        }
        return Err("Bedrock requires explicit environment keys, bearer or web-identity credentials, or EEVEE_CLAUDE_CLOUD_AUTH=provider-chain for provider-owned SSO/profile/workload discovery".into());
    }
    if vertex {
        if !present("ANTHROPIC_VERTEX_PROJECT_ID") {
            return Err("Vertex requires an explicit ANTHROPIC_VERTEX_PROJECT_ID".into());
        }
        if chain {
            return Ok("Google Vertex (explicit provider ADC credential chain)".into());
        }
        if present("GOOGLE_APPLICATION_CREDENTIALS") {
            return Ok("Google Vertex (provider-owned cloud credentials)".into());
        }
        return Err("Vertex requires selected GOOGLE_APPLICATION_CREDENTIALS or EEVEE_CLAUDE_CLOUD_AUTH=provider-chain for provider-owned default ADC/workload discovery".into());
    }
    Ok("ANTHROPIC_API_KEY (permitted API usage required; validity unknown until provider response)".into())
}
fn environment_gate() -> Result<(), String> {
    for name in [
        "NODE_OPTIONS",
        "NODE_PATH",
        "BUN_OPTIONS",
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "DYLD_LIBRARY_PATH",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_API_KEY_HELPER",
    ] {
        if env_present(name) {
            return Err(format!(
                "Claude launch refused: {name} can alter runtime/auth/code identity; remove it and preview again"
            ));
        }
    }
    // System policy may outrank SDK options and execute hooks before init. Refuse rather
    // than treating disableAllHooks/config-source flags as an authoritative admin override.
    let mut managed = vec![
        PathBuf::from("/etc/claude-code/managed-settings.json"),
        PathBuf::from("/etc/claude-code/managed-settings.d"),
        PathBuf::from("/Library/Application Support/ClaudeCode/managed-settings.json"),
    ];
    if let Some(program_data) = std::env::var_os("ProgramData") {
        managed.push(PathBuf::from(program_data).join("ClaudeCode/managed-settings.json"));
    }
    if managed.iter().any(|p| fs::symlink_metadata(p).is_ok()) {
        return Err("Claude managed executable configuration exists; effective startup hooks cannot be qualified safely by this adapter; use reviewed manual external workflow".into());
    }
    Ok(())
}
/// Conservative user installation path; never implicitly execute repository/target files.
pub(crate) fn sidecar_path() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("EEVEE_CLAUDE_SIDECAR") {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err("EEVEE_CLAUDE_SIDECAR must name an absolute main.ts path".into());
        }
        return Ok(path);
    }
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|v| PathBuf::from(v).join(".local/share")))
        .ok_or("Optional sidecar absent: set EEVEE_CLAUDE_SIDECAR to an explicitly installed absolute main.ts path")?;
    Ok(base.join("eevee/claude-agent/main.ts"))
}
fn node_path() -> Result<PathBuf, String> {
    let program = std::env::var_os("EEVEE_CLAUDE_NODE").unwrap_or_else(|| {
        if cfg!(windows) {
            "node.exe".into()
        } else {
            "node".into()
        }
    });
    let path = PathBuf::from(&program);
    let candidate = if path.is_absolute() {
        Some(path)
    } else if path.components().count() == 1 {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(&program))
                .find(|p| crate::workspace::runner::native::is_executable(p))
        })
    } else {
        None
    };
    candidate
        .and_then(|p| fs::canonicalize(p).ok())
        .ok_or_else(|| {
            "Optional Node 24 runtime unavailable; Rust/local work remains available".into()
        })
}
fn open_snapshot_file(path: &Path) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(path)
}
fn identity(path: &Path) -> Result<crate::workspace::runner::ExecutableIdentity, String> {
    let mut file = open_snapshot_file(path).map_err(|_| "SDK/runtime identity cannot be read")?;
    let before = file
        .metadata()
        .map_err(|_| "SDK/runtime metadata unavailable")?;
    if !before.is_file() || before.len() > 512 * 1024 * 1024 {
        return Err("SDK/runtime identity exceeds regular-file bound".into());
    }
    let id = crate::workspace::runner::native::file_identity(&file)
        .map_err(|_| "SDK/runtime file identity unavailable")?;
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "SDK/runtime identity read failed")?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > 512 * 1024 * 1024 {
            return Err("SDK/runtime file changed while hashing".into());
        }
        digest.update(&buffer[..count]);
    }
    let after = file
        .metadata()
        .map_err(|_| "SDK/runtime metadata changed")?;
    let path_file =
        open_snapshot_file(path).map_err(|_| "SDK/runtime path replaced while hashing")?;
    if bytes != before.len()
        || after.len() != before.len()
        || after.modified().ok() != before.modified().ok()
        || fs::canonicalize(path).ok().as_deref() != Some(path)
        || crate::workspace::runner::native::file_identity(&path_file)
            .ok()
            .as_ref()
            != Some(&id)
    {
        return Err("SDK/runtime identity changed while hashing; preview again".into());
    }
    Ok(crate::workspace::runner::ExecutableIdentity {
        path: path.into(),
        sha256: format!("{:x}", digest.finalize()),
        bytes,
        modified_ns: after
            .modified()
            .ok()
            .and_then(|v| v.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |v| v.as_nanos()),
        file_id: id,
    })
}
#[derive(PartialEq, Eq)]
struct Stamp {
    digest: String,
    files: usize,
    bytes: u64,
}
fn package_stamp(root: &Path) -> Result<Stamp, String> {
    fn visit(
        root: &Path,
        dir: &Path,
        digest: &mut Sha256,
        files: &mut usize,
        total: &mut u64,
    ) -> Result<(), String> {
        let mut entries = fs::read_dir(dir).map_err(|_| "Optional sidecar package absent; explicitly run npm install --ignore-scripts --include=optional @anthropic-ai/claude-agent-sdk@latest in sidecar/")?.collect::<Result<Vec<_>, _>>().map_err(|_| "Cannot inspect sidecar package")?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            *files += 1;
            if *files > 50000 {
                return Err("Sidecar dependency entry bound exceeded".into());
            }
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).map_err(|_| "Sidecar dependency changed")?;
            let rel = path
                .strip_prefix(root)
                .map_err(|_| "Sidecar package escaped root")?;
            digest.update(rel.as_os_str().as_encoded_bytes());
            if meta.file_type().is_symlink() {
                let target =
                    fs::canonicalize(&path).map_err(|_| "Broken sidecar dependency symlink")?;
                if !target.starts_with(root) {
                    return Err("Sidecar dependency symlink escapes reviewed installation".into());
                }
                digest.update(b"link");
                digest.update(target.as_os_str().as_encoded_bytes());
            } else if meta.is_dir() {
                digest.update(b"dir");
                visit(root, &path, digest, files, total)?;
            } else if meta.is_file() {
                let path = fs::canonicalize(path).map_err(|_| "Sidecar dependency unavailable")?;
                let item = identity(&path)?;
                *total += item.bytes;
                if *total > 2 * 1024 * 1024 * 1024 {
                    return Err("Sidecar dependency byte bound exceeded".into());
                }
                digest.update(b"file");
                digest.update(item.sha256);
                digest.update(item.file_id);
                digest.update(item.modified_ns.to_le_bytes());
            } else {
                return Err("Sidecar package contains nonregular executable/config input".into());
            }
        }
        Ok(())
    }
    let mut digest = Sha256::new();
    let mut files = 0;
    let mut bytes = 0;
    visit(root, root, &mut digest, &mut files, &mut bytes)?;
    Ok(Stamp {
        digest: format!("{:x}", digest.finalize()),
        files,
        bytes,
    })
}
#[derive(Debug, PartialEq, Eq)]
struct SdkVersions {
    sdk: String,
    cli: String,
}
fn sdk_versions(package: &Value) -> Result<SdkVersions, String> {
    if package["name"] != "@anthropic-ai/claude-agent-sdk" {
        return Err("SDK package identity mismatch".into());
    }
    let version = |field: &str| -> Result<String, String> {
        let text = package[field]
            .as_str()
            .ok_or("SDK version metadata missing")?;
        let core = text.split(['-', '+']).next().unwrap_or("");
        if text.len() > 128
            || !text
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c))
            || core.split('.').count() != 3
            || !core
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit()))
        {
            return Err("SDK version metadata invalid".into());
        }
        Ok(text.into())
    };
    Ok(SdkVersions {
        sdk: version("version")?,
        cli: version("claudeCodeVersion")?,
    })
}
fn inspection_matches(value: &Value, versions: &SdkVersions) -> bool {
    value["type"] == "capabilities"
        && value["sdk_version"].as_str() == Some(versions.sdk.as_str())
        && value["cli_version"].as_str() == Some(versions.cli.as_str())
        && value["bridge_revision"] == 9
        && value["query"] == true
        && value["policy"]
            == json!({
                "tools": "closed-allowlist", "permissions": "default-host-once", "mcp": "host-discovery-only",
                "configuration": "isolated", "authentication": "api-console-cloud-only",
                "dispatch": "correlated-sdk-submission",
                "usage": "correlated-sdk-snapshots",
                "file_preview": "display-only-correlated-arguments-outcomes",
            })
}

fn dispatch_receipt(
    input: &Input,
    generation: u64,
    value: &Value,
    seen: &mut Option<String>,
) -> Result<Event, String> {
    if seen.is_some()
        || value["type"] != "dispatched"
        || value["generation"].as_u64() != Some(generation)
        || value["task_id"].as_i64() != Some(input.task.id)
        || value["request_id"].as_str() != input.work.as_ref().map(|work| work.request_id.as_str())
        || value["submission_id"]
            .as_str()
            .is_none_or(|id| !sdk_uuid(id))
    {
        return Err("SDK dispatch receipt correlation mismatch or duplicate".into());
    }
    let event = input
        .dispatched(generation)
        .ok_or("SDK dispatch receipt requires a claimed inference request")?;
    *seen = Some(value["submission_id"].as_str().unwrap().into());
    Ok(event)
}

fn sdk_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn usage_event(
    input: &Input,
    generation: u64,
    session: &str,
    submission: Option<&str>,
    value: &Value,
) -> Result<Event, String> {
    use crate::workspace::usage::{Kind, Report, Scope};
    if input.inspect_models
        || input.inspect_sessions
        || input.work.is_none()
        || session.is_empty()
        || value["type"] != "usage"
        || value["generation"].as_u64() != Some(generation)
        || value["task_id"].as_i64() != Some(input.task.id)
        || value["session_id"].as_str() != Some(session)
        || value["request_id"].as_str() != input.work.as_ref().map(|work| work.request_id.as_str())
        || submission.is_none()
        || value["submission_id"].as_str() != submission
    {
        return Err("SDK usage requires an acknowledged current submitted request".into());
    }
    let report: Report =
        serde_json::from_value(value["report"].clone()).map_err(|_| "Invalid SDK usage report")?;
    report.validate()?;
    if report.identity.session.as_deref() != Some(session)
        || report.identity.turn.as_deref() != submission
        || report.identity.response.is_some()
        || report.kind != Kind::Snapshot
        || !matches!(report.scope, Scope::MainLoopTurn | Scope::ModelPipeline)
        || (report.scope == Scope::ModelPipeline && report.model.is_none())
        || (report.scope == Scope::MainLoopTurn
            && (report.model.is_some() || report.baseline.is_some()))
        || report.tokenizer.is_some()
    {
        return Err("SDK usage widened its reported boundary or identity".into());
    }
    Ok(Event::Usage {
        generation,
        task_id: input.task.id,
        report,
    })
}

#[derive(Default)]
struct SdkSession {
    task_id: i64,
    model_totals: HashMap<String, crate::workspace::usage::Counters>,
}

#[derive(Default)]
struct SdkTurnUsage {
    baseline: HashMap<String, crate::workspace::usage::Counters>,
    completed: HashMap<String, crate::workspace::usage::Counters>,
    result: Option<String>,
}

impl SdkTurnUsage {
    fn begin(input: &Input, sessions: &mut HashMap<String, SdkSession>) -> Result<Self, String> {
        // SDK 0.3.300 SDKResultSuccess/Error.modelUsage explicitly continues
        // saved transcript totals on resume; this is not inferred from the
        // session ID or from counters merely increasing between queries.
        // https://unpkg.com/@anthropic-ai/claude-agent-sdk@0.3.300/sdk.d.ts
        // Retain only observed completed totals, never seed external history.
        let baseline = if let Some(id) = &input.resume {
            let session = sessions
                .get_mut(id)
                .filter(|session| session.task_id == input.task.id)
                .ok_or("SDK usage resume requires the owned task/session")?;
            // A submitted turn with no complete result makes the next baseline
            // unknown, rather than charging its consumption to a later turn.
            std::mem::take(&mut session.model_totals)
        } else {
            HashMap::new()
        };
        Ok(Self {
            baseline,
            ..Self::default()
        })
    }

    fn observe(&mut self, mut event: Event) -> Result<Event, String> {
        use crate::workspace::usage::Scope;
        let Event::Usage { report, .. } = &mut event else {
            return Err("Expected correlated SDK usage".into());
        };
        let result = report
            .identity
            .event
            .as_ref()
            .ok_or("SDK usage lacks result identity")?;
        if self
            .result
            .as_ref()
            .is_some_and(|previous| previous != result)
            || report.baseline.is_some()
        {
            return Err("SDK usage changed result identity or supplied an unowned baseline".into());
        }
        if self.result.is_none() {
            self.result = Some(result.clone());
        }
        if report.scope == Scope::ModelPipeline {
            let model = report
                .model
                .as_ref()
                .ok_or("SDK pipeline usage lacks model")?;
            report.baseline = self.baseline.get(model).copied();
            if report.complete {
                if self.completed.len() >= 128 && !self.completed.contains_key(model) {
                    return Err("SDK usage model bound exceeded".into());
                }
                let totals = self.completed.entry(model.clone()).or_default();
                for (field, value) in [
                    (&mut totals.input, report.counters.input),
                    (&mut totals.output, report.counters.output),
                    (&mut totals.cache_read, report.counters.cache_read),
                    (&mut totals.cache_write, report.counters.cache_write),
                    (&mut totals.reasoning, report.counters.reasoning),
                ] {
                    if value.is_some() {
                        *field = value;
                    }
                }
            }
        }
        Ok(event)
    }

    fn finish(self, session: &mut SdkSession, outcome: Outcome) {
        if outcome == Outcome::Completed {
            session.model_totals = self.completed;
        }
    }
}

struct Trust {
    launch: crate::workspace::runner::Prepared,
    root: PathBuf,
    entry: PathBuf,
    stamp: Stamp,
    versions: SdkVersions,
    project_id: String,
    account: String,
    state_dir: PathBuf,
}
fn trust(project: &Path, data_dir: &Path) -> Result<Trust, String> {
    environment_gate()?;
    let entry_requested = sidecar_path()?;
    let entry = fs::canonicalize(&entry_requested).map_err(|_| "Optional Claude sidecar is not installed. Explicitly run npm install --ignore-scripts --include=optional @anthropic-ai/claude-agent-sdk@latest in sidecar/; set EEVEE_CLAUDE_SIDECAR. Core needs no Node/SDK.".to_string())?;
    let root = entry
        .parent()
        .ok_or("Sidecar package root unavailable")?
        .to_path_buf();
    for name in [".npmrc", ".env", ".credentials.json"] {
        if fs::symlink_metadata(root.join(name)).is_ok() {
            return Err("Optional sidecar installation contains a credential/config store; move it outside the executable package before review".into());
        }
    }
    for name in ["main.ts", "policy.ts", "package.json", "package-lock.json"] {
        if !root.join(name).is_file() {
            return Err(format!(
                "Sidecar installation missing {name}; install the complete sidecar package"
            ));
        }
    }
    if entry.file_name().and_then(|n| n.to_str()) != Some("main.ts") {
        return Err("Sidecar entry must be the installed main.ts".into());
    }
    let sdk = root.join("node_modules/@anthropic-ai/claude-agent-sdk/package.json");
    let mut sdk_file = File::open(&sdk).map_err(|_| "Optional SDK dependency absent; explicitly run npm install --ignore-scripts --include=optional @anthropic-ai/claude-agent-sdk@latest in the sidecar installation")?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut sdk_file)
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "SDK metadata unavailable")?;
    if bytes.len() > 64 * 1024 {
        return Err("SDK metadata exceeds bound".into());
    }
    let package: Value = serde_json::from_slice(&bytes).map_err(|_| "SDK metadata invalid")?;
    let versions = sdk_versions(&package)?;
    let node = node_path()?;
    for path in [&entry, &node] {
        let value = path.to_str().ok_or("SDK/runtime paths must be Unicode")?;
        if value.len() > 4096 || safe(value) != value {
            return Err(
                "SDK/runtime path cannot be reviewed literally; remove private/control text".into(),
            );
        }
    }
    if !crate::workspace::runner::native::is_executable(&node) {
        return Err("Selected Node runtime is not executable".into());
    }
    let executable = identity(&node)?;
    let stamp = package_stamp(&root)?;
    let cwd = fs::canonicalize(std::env::temp_dir())
        .map_err(|_| "Outside-project process directory unavailable")?;
    if cwd.starts_with(project) {
        return Err("SDK process discovery cwd must be outside project".into());
    }
    let state_dir = data_dir.join("claude-agent-state");
    if fs::symlink_metadata(&state_dir).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("SDK state symlink refused".into());
    }
    for name in [
        "settings.json",
        "settings.local.json",
        ".credentials.json",
        "plugins",
        "hooks",
        "mcp.json",
    ] {
        if fs::symlink_metadata(state_dir.join(name)).is_ok() {
            return Err("Isolated SDK state contains executable/auth configuration; review and remove it outside Eevee".into());
        }
    }
    if fs::symlink_metadata(state_dir.join("home")).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("SDK isolated home symlink refused".into());
    }
    for name in [".claude", ".claude.json"] {
        if fs::symlink_metadata(state_dir.join("home").join(name)).is_ok() {
            return Err("SDK isolated home contains a provider store/configuration; it cannot reuse existing authentication".into());
        }
    }
    let account = credential_route().unwrap_or_else(|_| {
        "Unavailable: permitted API key, Console OAuth profile or cloud credentials required; subscription auth is disabled"
            .into()
    });
    let launch = crate::workspace::runner::Prepared {
        approval_arrival: crate::providers::approval_arrival(),
        request_id: "claude-agent".into(),
        run_id: "claude-agent".into(),
        task_id: 0,
        spec: crate::workspace::runner::CommandSpec::Direct {
            program: node.to_string_lossy().into_owned(),
            args: vec![entry.to_string_lossy().into_owned()],
        },
        cwd_identity: crate::workspace::runner::native::directory_identity(&cwd)
            .map_err(|_| "SDK process cwd identity unavailable")?,
        executable,
        cwd,
        prepared_at_ms: 0,
        expires_at_ms: 0,
        before: crate::workspace::runner::InputSnapshot::default(),
        partner: None,
        rerun_of: None,
    };
    Ok(Trust {
        launch,
        root,
        entry,
        stamp,
        versions,
        project_id: crate::workspace::runner::native::directory_identity(project)
            .map_err(|_| "Project identity unavailable")?,
        account,
        state_dir,
    })
}
fn revalidate(project: &Path, data_dir: &Path, old: &Trust) -> Result<(), String> {
    let now = trust(project, data_dir)?;
    if now.launch != old.launch
        || now.entry != old.entry
        || now.stamp != old.stamp
        || now.versions != old.versions
        || now.project_id != old.project_id
        || now.account != old.account
        || now.state_dir != old.state_dir
    {
        return Err(
            "Claude runtime/SDK/code/config/auth/project identity changed; preview again".into(),
        );
    }
    Ok(())
}
// The sidecar's "plan" policy exposes only read/question tools and
// mandatory-disables writes, shell, delegation and external MCP. Review deliberately
// reuses that policy, not its prompt or provider session.
fn sdk_tool_policy(operation: Operation) -> &'static str {
    match operation {
        Operation::Plan | Operation::Review => "plan",
        Operation::Implement => "implement",
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SdkWriteScope {
    mode: SdkWriteMode,
    protected_paths: Vec<PathBuf>,
    excluded_paths: Vec<PathBuf>,
    root_identity: Option<String>,
    files: Vec<SdkWriteFile>,
    /// Host-only preview observation; never part of the sidecar write policy.
    #[serde(skip)]
    project_context: Option<Arc<Value>>,
}
impl SdkWriteScope {
    fn empty_selected() -> Self {
        Self {
            mode: SdkWriteMode::Selected,
            protected_paths: Vec::new(),
            excluded_paths: Vec::new(),
            root_identity: None,
            files: Vec::new(),
            project_context: None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum SdkWriteMode {
    Selected,
    Workspace,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SdkWriteFile {
    path: String,
    kind: SdkWriteKind,
    identity: Option<String>,
    sha256: Option<String>,
    working_content: Option<String>,
    ancestors: Vec<SdkWriteAncestor>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum SdkWriteKind {
    Create,
    Replace,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SdkWriteAncestor {
    path: String,
    identity: Option<String>,
}
fn sdk_directory_identity(path: &Path) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "SDK ancestor unavailable")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("SDK ancestor must be a non-link directory".into());
    }
    crate::workspace::runner::native::directory_identity(path)
        .map_err(|_| "SDK ancestor identity/reparse validation failed".into())
}
fn sdk_regular(file: &File) -> Result<(), String> {
    let metadata = file
        .metadata()
        .map_err(|_| "SDK file metadata unavailable")?;
    if !metadata.is_file() || metadata.len() > 256 * 1024 {
        return Err("SDK edit baseline exceeds bounded regular-file policy".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("SDK hard-link alias refused".into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0
            || info.nNumberOfLinks != 1
            || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err("SDK hard-link/reparse alias refused".into());
        }
    }
    Ok(())
}
fn capture_write_scope(
    data_dir: &Path,
    input: &Input,
    exclusions: &[PathBuf],
) -> Result<SdkWriteScope, String> {
    if input.edit_paths.len() > 256
        || (input.operation != Operation::Implement && !input.edit_paths.is_empty())
    {
        return Err("Claude edit paths are bounded and available only for Implement".into());
    }
    let root = Path::new(&input.task.project);
    let root_identity = sdk_directory_identity(root)?;
    let data = fs::canonicalize(data_dir).map_err(|_| "SDK data protection unavailable")?;
    let executable = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|_| "SDK executable protection unavailable")?;
    if exclusions.iter().any(|path| {
        path.as_os_str().is_empty()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || path.to_str().is_none()
    }) {
        return Err("SDK exclusion must be an exact root-relative path".into());
    }
    let mut files = Vec::with_capacity(input.edit_paths.len());
    for path in &input.edit_paths {
        let text = path.to_str().ok_or("SDK selected path is not UTF-8")?;
        if text.is_empty()
            || text.len() > 4096
            || text
                .chars()
                .any(|c| c.is_control() || matches!(c, '\\' | ':'))
            || text.split('/').any(|part| {
                let lower = part.to_ascii_lowercase();
                let stem = lower.split('.').next().unwrap_or_default();
                part.is_empty()
                    || matches!(part, "." | "..")
                    || part.ends_with('.')
                    || part.ends_with(' ')
                    || matches!(
                        stem,
                        "con"
                            | "prn"
                            | "aux"
                            | "nul"
                            | "com1"
                            | "com2"
                            | "com3"
                            | "com4"
                            | "com5"
                            | "com6"
                            | "com7"
                            | "com8"
                            | "com9"
                            | "lpt1"
                            | "lpt2"
                            | "lpt3"
                            | "lpt4"
                            | "lpt5"
                            | "lpt6"
                            | "lpt7"
                            | "lpt8"
                            | "lpt9"
                    )
                    || matches!(
                        lower.as_str(),
                        ".claude"
                            | ".codex"
                            | ".gemini"
                            | ".opencode"
                            | ".ds_store"
                            | ".ev-install"
                    )
            })
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || crate::workspace::context::excluded(path, exclusions)
        {
            return Err(
                "Implementation path is not a permitted root-relative selected file".into(),
            );
        }
        let full = root.join(path);
        if full.starts_with(&data) || full == executable {
            return Err("SDK own data/runtime target refused".into());
        }
        let mut ancestors = Vec::new();
        let mut relative = PathBuf::new();
        let mut missing = false;
        let parts: Vec<_> = path.components().collect();
        for component in &parts[..parts.len() - 1] {
            relative.push(component.as_os_str());
            let current = root.join(&relative);
            let identity = match fs::symlink_metadata(&current) {
                Ok(_) if !missing => Some(sdk_directory_identity(&current)?),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    missing = true;
                    None
                }
                _ => {
                    return Err("SDK selected parent appeared or is not a regular directory".into());
                }
            };
            ancestors.push(SdkWriteAncestor {
                path: relative
                    .to_str()
                    .ok_or("SDK ancestor is not UTF-8")?
                    .replace('\\', "/"),
                identity,
            });
        }
        let (kind, identity, sha256, working_content) = match fs::symlink_metadata(&full) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                (SdkWriteKind::Create, None, None, None)
            }
            Ok(metadata) if !missing && !metadata.file_type().is_symlink() => {
                let mut file =
                    open_snapshot_file(&full).map_err(|_| "SDK baseline open refused")?;
                sdk_regular(&file)?;
                let before = file.metadata().map_err(|_| "SDK baseline unavailable")?;
                let identity = crate::workspace::runner::native::file_identity(&file)
                    .map_err(|_| "SDK file identity unavailable")?;
                let mut content = String::new();
                Read::by_ref(&mut file)
                    .take(256 * 1024 + 1)
                    .read_to_string(&mut content)
                    .map_err(|_| "SDK baseline is not bounded UTF-8")?;
                let after = file.metadata().map_err(|_| "SDK baseline changed")?;
                let path_file =
                    open_snapshot_file(&full).map_err(|_| "SDK baseline path changed")?;
                sdk_regular(&path_file)?;
                if content.len() > 256 * 1024
                    || content.len() as u64 != before.len()
                    || before.len() != after.len()
                    || before.modified().ok() != after.modified().ok()
                    || crate::workspace::runner::native::file_identity(&path_file)
                        .ok()
                        .as_ref()
                        != Some(&identity)
                {
                    return Err("SDK baseline changed while reading".into());
                }
                (
                    SdkWriteKind::Replace,
                    Some(identity),
                    Some(format!("{:x}", Sha256::digest(content.as_bytes()))),
                    Some(content),
                )
            }
            _ => {
                return Err("SDK baseline target is not an absent or regular selected file".into());
            }
        };
        files.push(SdkWriteFile {
            path: text.replace('\\', "/"),
            kind,
            identity,
            sha256,
            working_content,
            ancestors,
        });
    }
    crate::providers::writes::verify_path_names(root, &input.edit_paths)?;
    if sdk_directory_identity(root)? != root_identity {
        return Err("SDK root changed during capture".into());
    }
    Ok(SdkWriteScope {
        mode: if input.operation == Operation::Implement && input.edit_paths.is_empty() {
            SdkWriteMode::Workspace
        } else {
            SdkWriteMode::Selected
        },
        protected_paths: vec![data, executable],
        excluded_paths: exclusions
            .iter()
            .map(|path| {
                let mut text = String::with_capacity(path.as_os_str().len());
                for part in path.components() {
                    if !text.is_empty() {
                        text.push('/');
                    }
                    text.push_str(
                        part.as_os_str()
                            .to_str()
                            .expect("validated UTF-8 exclusion"),
                    );
                }
                PathBuf::from(text)
            })
            .collect(),
        root_identity: Some(root_identity),
        files,
        project_context: None,
    })
}
fn payload(
    data_dir: &Path,
    input: &Input,
    exclusions: &[PathBuf],
    frozen_context: Option<&Arc<Value>>,
) -> Result<(String, SdkWriteScope), String> {
    input.validate_selection()?;
    if input.inspect_models {
        return serde_json::to_string(&json!({
            "intent": "inspect_models", "selection": input.selection,
            "effects": "Initialize the reviewed SDK/backend and query supportedModels(); may contact provider/auth services and create isolated SDK state. No task prompt, prior session, model turn or model journal.",
        })).map(|text| (text, SdkWriteScope::empty_selected())).map_err(|_| "Could not encode Claude model inspection".into());
    }
    input.validate_review()?;
    input.validate_committed(data_dir)?;
    let mut write_scope = capture_write_scope(data_dir, input, exclusions)?;
    let project_context = if let Some(context) = frozen_context {
        Arc::clone(context)
    } else {
        let scope = super::writes::WriteSet::capture_readonly(
            Path::new(&input.task.project),
            data_dir,
            exclusions,
        )?;
        Arc::new(
            serde_json::to_value(super::project_context(&scope, input.agent))
                .map_err(|_| "Cannot encode SDK project context")?,
        )
    };
    write_scope.project_context = Some(project_context);
    let project_context = write_scope
        .project_context
        .as_deref()
        .expect("captured project context");
    if let Some(text) = input.review_payload(data_dir, exclusions)? {
        let mut review: Value =
            serde_json::from_str(&text).map_err(|_| "Cannot decode SDK review context")?;
        review["project_context"] = project_context.clone();
        return Ok((
            serde_json::to_string_pretty(&review)
                .map_err(|_| "Cannot encode SDK review context")?,
            write_scope,
        ));
    }
    if let Some(document) = &input.handoff {
        crate::workspace::handoff::validate_dispatch(
            document,
            &input.task,
            &input.excerpts,
            input.report.as_ref(),
            exclusions,
        )?;
    }
    let sources = input.source_payload(exclusions)?;
    let text = crate::workspace::context::optimizer::encode_context(json!({
        "purpose": if input.operation == Operation::Plan { "Answer the request directly; inspect safe source when useful. Produce a plan only when requested. No implementation tools are exposed." } else { "Answer the request directly and inspect safe source when useful. When changes are requested, implement using exact per-operation approval of each file and missing parent. Empty selected paths permit safe project-relative file proposals; nonempty selected paths are an exact restriction. Preserve existing dirty working content. This is external SDK agent policy, not strict OS confinement. Do not commit/merge. Report unverified patch for human diff review." },
        "task": { "id": input.task.id, "revision": input.task.revision, "goal": input.task.goal, "acceptance_criteria": input.task.criteria, "next_action": crate::workspace::handoff::without_local_provenance(&input.task.next_action) },
        "project_context": project_context,
        "request": input.instruction(),
        "prior_plan": input.prior_plan_payload(),
        "approved_memories": input.task.approved_memories,
        "selected_sources": serde_json::from_str::<Value>(&sources.text).map_err(|_| "Invalid source envelope")?, "write_mode": write_scope.mode, "edit_baselines": write_scope.files, "handoff": input.handoff,
    })).map_err(|_| "Could not encode Claude payload")?;
    if text.len() > FRAME_LIMIT / 2 || text.chars().any(|c| (c.is_control() && !matches!(c, '\n' | '\t')) ||
        matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) || crate::workspace::privacy::redact(&text) != text {
        return Err("Claude payload exceeds bound or contains credential/display-control text; review source/task content".into());
    }
    Ok((text, write_scope))
}
struct Prepared {
    generation: u64,
    input: Box<Input>,
    trust: Trust,
    text: String,
    write_scope: SdkWriteScope,
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
    let mut generation = 0u64;
    let mut prepared: Option<Prepared> = None;
    let mut preview_owner: Option<(u64, i64)> = None;
    let mut sessions: HashMap<String, SdkSession> = HashMap::new();
    while !stop.load(Ordering::Acquire) {
        let request = match incoming.recv_timeout(Duration::from_millis(20)) {
            Ok(r) => r,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        match request {
            Request::Preview(input) => {
                prepared = None;
                generation = generation.saturating_add(1);
                preview_owner = Some((generation, input.task.id));
                let mut preview = Preview {
                    comparison_scope: None,
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
                    preview.blockers.push("session/list inspection requires an ACP peer; no SDK query will be dispatched".into());
                }
                if input.agent != Agent::Claude
                    || Path::new(&input.task.project) != project.as_path()
                {
                    preview
                        .blockers
                        .push("Claude task/project/profile mismatch".into());
                }
                if input.resume.as_ref().is_some_and(|id| {
                    sessions.get(id).map(|session| session.task_id) != Some(input.task.id)
                }) {
                    preview.blockers.push("Resume refused: explicit SDK session must have been started for this task in this Eevee instance".into());
                }
                if !input.inspect_models && sessions.len() >= 256 && input.resume.is_none() {
                    preview
                        .blockers
                        .push("SDK session bound reached; restart before another session".into());
                }
                let observed = trust(project, data_dir);
                if let Ok(t) = &observed {
                    preview.comparison_scope = super::ComparisonScope::from_identity(&(
                        &t.launch.spec,
                        &t.launch.cwd_identity,
                        &t.launch.executable,
                        &t.stamp.digest,
                        &t.project_id,
                        &t.account,
                        &t.state_dir,
                    ));
                    preview.requires_connection_review =
                        t.launch.executable.path.starts_with(project)
                            || t.entry.starts_with(project)
                            || t.root.starts_with(project)
                            || input.operation == Operation::Implement
                            || std::env::var("EEVEE_CLAUDE_CLOUD_AUTH").ok().as_deref()
                                == Some("provider-chain");
                }
                match &observed {
                    Ok(t) => preview.trust.extend([
                        format!("Consent executes Node {} sha256 {}, native-TypeScript sidecar {} and complete dependency tree sha256 {} ({} entries / {} bytes). Installed SDK {} / embedded CLI {} are bound to this review, not release-pinned; --inspect loads the real SDK and checks bridge policy without a model before one query. Unsupported protocol/capabilities fail closed.", t.launch.executable.path.display(), t.launch.executable.sha256, t.entry.display(), t.stamp.digest, t.stamp.files, t.stamp.bytes, t.versions.sdk, t.versions.cli),
                        format!("Authentication/billing source: {}. Credential values are never read/copied/stored by Eevee; installed claude.ai subscription auth is not used. SDK state/transcripts persist separately at {}.", t.account, t.state_dir.display()),
                        if t.account.starts_with("Anthropic Console OAuth profile") {
                            "CONSOLE OAUTH PROFILE: the SDK reads only the selected ant profile (explicit ANTHROPIC_CONFIG_DIR/ANTHROPIC_PROFILE) and bills API usage to its Console organization/workspace, not a claude.ai subscription. The embedded CLI may refresh that profile's access token in place; a profile base_url is provider configuration. HOME and Claude configuration stay isolated. The effective backend must report profile credentials (no API-key/subscription source) before prompt release.".into()
                        } else if std::env::var("EEVEE_CLAUDE_CLOUD_AUTH").ok().as_deref() == Some("provider-chain") {
                            "EXPLICIT EXTERNAL CLOUD AUTH: inherited HOME is available only for selected Bedrock/Vertex profile/SSO/ADC/workload credentials. Provider credential libraries may read cloud stores, refresh caches, execute credential_process or configured external-account helpers, and contact identity/metadata services. Eevee does not inspect/copy those stores or qualify helper executables. Claude's CLAUDE_CONFIG_DIR stays isolated; effective SDK backend is checked before prompt release. No live account/SSO/ADC validity claim.".into()
                        } else {
                            "CLOUD AUTH: only explicitly selected environment/bearer/web-identity/credential-file routes are admitted; HOME and Claude configuration are isolated. Profile/SSO/default ADC discovery requires EEVEE_CLAUDE_CLOUD_AUTH=provider-chain and a fresh preview. Effective SDK backend is checked before prompt release.".into()
                        },
                        "Filesystem settingSources=[]; no project/user CLAUDE.md/hooks/plugins/skills/external MCP are loaded. MCP is strict host-only: one in-process bounded read-only source discovery tool; no external servers. Agent/Task/Bash/web/worktree/delegation tools are mandatory-disabled. System executable configuration is refused. Mandatory host policy hooks cover reads and writes in every noninspection operation. Inherited OS/runtime and provider cloud credential libraries remain external dependencies.".into(),
                        "Read/Glob/Grep are provider-native agent-policy tools, not excerpt-only reads or Eevee confinement; preapproved reads may bypass canUseTool but not mandatory host admission. Root discovery uses host-filtered safe filenames/literal matches, omitting protected/excluded/linked/credential-bearing entries without weakening native subtree checks. Explicit ask rules mediate supported requests. Deny/allow once only; no permission updates, login flows or automatic queue/resend. Interrupt requires observed terminal result; forced cleanup is outcome unknown, never rollback. One query is bounded to 16 SDK turns and estimated incremental USD 5; estimates are not invoices.".into(),
                        match input.operation {
                            Operation::Implement => "EXTERNAL-POLICY IMPLEMENTATION: Write/Edit proposals, mandatory frozen-target hook and exact one-shot user approval. Optional selected paths restrict targets; otherwise safe project-relative targets are captured before approval. No shell/delegation/MCP mutations. Symlinks are refused at check; SDK path checks are NOT atomic OS confinement. Consent accepts that external-policy boundary; inspect dirty baseline/current diff and deliberately verify; no auto commit/merge.".into(),
                            Operation::Plan => "PLAN: SDK tool exposure excludes all file-write/shell/delegation routes. This is verified configured agent tool policy, not filesystem/network isolation.".into(),
                            Operation::Review => "REVIEW: fresh requirements/patch/selected-observation context in a new session; no implementation history or Resume. Uses the reviewed SDK read-only tool policy: Write/Edit/Bash/delegation/external MCP are unavailable independently of approval callbacks. Only host-owned safe source discovery is exposed through MCP. Agent tool policy is not OS isolation.".into(),
                        },
                        if input.operation == Operation::Review { "Review never sends a provider session ID or resumes prior history. A fix requires a deliberate separate implementation handoff.".into() } else { "Resume includes provider-owned prior transcript, not just the selected context. Same-task/session/current policy only. UUID-correlated main-loop per-turn usage is separate from cumulative model-pipeline snapshots. Restored pipeline totals are never current charges without an observed pre-submission baseline; this one-query sidecar cannot infer that baseline from prior history.".into() },
                    ]),
                    Err(error) => preview.blockers.push(safe(error)),
                }
                if let Err(error) = credential_route() {
                    preview.blockers.push(error);
                }
                preview.trust.push(format!(
                    "Model: {}; requested reasoning effort: {}. Explicit choices use official SDK Options. Catalogue metadata is informational; exact configured/custom IDs are validated by the SDK control path and provider, not a local list. Effort is requested through supported SDK Options; organization caps/downgrades may apply and applied effort is not reported. {}",
                    input.selection.model.as_deref().unwrap_or("provider default (not yet observed)"),
                    input.selection.effort.as_deref().unwrap_or("provider default (not advertised)"),
                    if input.inspect_models { "MODEL INSPECTION ONLY: no prompt/history/model turn or journal; SDK initialization may contact provider/auth services, refresh credentials and create isolated SDK state. All tools are disabled; the SDK closes after bounded capabilities and terminal result." } else { "The selected model and effort belong to this immutable reviewed request." },
                ));
                let mut write_scope = SdkWriteScope::empty_selected();
                match payload(data_dir, &input, exclusions, None) {
                    Ok((text, scope)) => {
                        preview.payload = text;
                        write_scope = scope;
                    }
                    Err(error) => preview.blockers.push(safe(&error)),
                }
                if preview.blockers.is_empty()
                    && !input.local_only
                    && let Ok(trust) = observed
                {
                    prepared = Some(Prepared {
                        generation,
                        input,
                        trust,
                        text: preview.payload.clone(),
                        write_scope,
                    });
                }
                for line in &mut preview.trust {
                    *line = safe(line);
                }
                let _ = emit(&events, Event::Preview(preview));
                busy.store(false, Ordering::Release);
            }
            Request::Run {
                generation: requested,
            } => {
                if let Some(run) = prepared.take() {
                    if requested != run.generation {
                        prepared = Some(run);
                        finish(
                            &events,
                            requested,
                            0,
                            Outcome::Failed,
                            "Stale Claude Run consent refused".into(),
                        );
                    } else {
                        preview_owner = None;
                        let result = transport(
                            &workspace,
                            &run,
                            &incoming,
                            &events,
                            &stop,
                            &cancelled,
                            &mut sessions,
                        );
                        finish(&events, requested, run.input.task.id, result.0, result.1);
                    }
                } else {
                    finish(
                        &events,
                        requested,
                        preview_owner
                            .filter(|(owner, _)| *owner == requested)
                            .map_or(0, |(_, task_id)| task_id),
                        Outcome::Failed,
                        "Claude Run requires a valid reviewed immutable preview".into(),
                    );
                }
                busy.store(false, Ordering::Release);
            }
            Request::Cancel {
                generation: requested,
            } => {
                if preview_owner.is_some_and(|(owner, _)| owner == requested) {
                    let (_, task_id) = preview_owner.take().expect("matched preview");
                    prepared = None;
                    finish(
                        &events,
                        requested,
                        task_id,
                        Outcome::Cancelled,
                        "Claude preview discarded before launch".into(),
                    );
                }
                busy.store(false, Ordering::Release);
            }
            Request::Answer { generation, .. } => {
                let _ = emit(
                    &events,
                    Event::Error {
                        generation: Some(generation),
                        message: "Stale Claude answer refused; no live callback".into(),
                    },
                );
            }
        }
    }
    busy.store(false, Ordering::Release);
}
fn preserve_observations(
    messages: Vec<Value>,
    fault: &mut Option<String>,
    error: &str,
) -> Result<Vec<Value>, String> {
    if messages.is_empty() {
        Err(error.into())
    } else {
        *fault = Some(error.into());
        Ok(messages)
    }
}

struct Pipe {
    cancelled: bool,
    process: crate::workspace::runner::native::Process,
    frame: Vec<u8>,
    received: usize,
    outgoing: VecDeque<Vec<u8>>,
    offset: usize,
    deadline: Option<Instant>,
    eof: bool,
    fault: Option<String>,
}
impl Pipe {
    fn spawn(prepared: &crate::workspace::runner::Prepared) -> Result<Self, String> {
        Ok(Self {
            cancelled: false,
            process: crate::workspace::runner::native::Process::spawn_piped(prepared)
                .map_err(|_| "Trusted optional Claude runtime could not launch")?,
            frame: Vec::new(),
            received: 0,
            outgoing: VecDeque::new(),
            offset: 0,
            deadline: None,
            eof: false,
            fault: None,
        })
    }
    fn cancel(&mut self, generation: u64) -> Result<(), String> {
        if self.cancelled {
            return Ok(());
        }
        self.cancelled = true;
        self.outgoing.clear();
        if self.offset != 0 {
            return Err("Claude cancelled during partial IPC write; no remaining grant bytes sent; outcome unknown".into());
        }
        self.deadline = None;
        self.send(json!({"protocol":1,"type":"cancel","generation":generation}))
    }
    fn send(&mut self, value: Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&value).map_err(|_| "Claude IPC encoding failed")?;
        bytes.push(b'\n');
        if bytes.len() > FRAME_LIMIT || self.outgoing.len() >= 32 {
            return Err("Claude IPC write bound exceeded".into());
        }
        self.outgoing.push_back(bytes);
        Ok(())
    }
    fn pump(&mut self, cancellation: Option<(&AtomicU64, u64)>) -> Result<Vec<Value>, String> {
        if let Some(error) = self.fault.take() {
            return Err(error);
        }
        if let Some((cancelled, generation)) = cancellation
            && cancelled.load(Ordering::Acquire) == generation
        {
            self.cancel(generation)?;
        }
        if let Some(front) = self.outgoing.front() {
            let deadline = *self
                .deadline
                .get_or_insert_with(|| Instant::now() + Duration::from_secs(20));
            if Instant::now() > deadline {
                return Err("Claude IPC write deadline exceeded".into());
            }
            match self.process.write_stdin(&front[self.offset..]) {
                Ok(0) => return Err("Claude sidecar stdin closed".into()),
                Ok(n) => {
                    self.offset += n;
                    if self.offset == front.len() {
                        self.outgoing.pop_front();
                        self.offset = 0;
                        self.deadline = None;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Err("Claude IPC write failed".into()),
            }
        }
        let mut messages = Vec::new();
        let mut buffer = [0u8; 8192];
        for index in 0..2 {
            for _ in 0..8 {
                match self.process.read(index, &mut buffer) {
                    Ok(0) => {
                        if index == 0 {
                            self.eof = true;
                        }
                        break;
                    }
                    Ok(n) => {
                        self.received += n;
                        if self.received > 64 * 1024 * 1024 {
                            return preserve_observations(
                                messages,
                                &mut self.fault,
                                "Claude stdio stream bound exceeded",
                            );
                        }
                        if index == 1 {
                            continue;
                        } // Never disclose SDK stderr/auth/reasoning diagnostics.
                        for &byte in &buffer[..n] {
                            if byte == b'\n' {
                                let value: Value = match serde_json::from_slice(&self.frame) {
                                    Ok(value) => value,
                                    Err(_) => {
                                        return preserve_observations(
                                            messages,
                                            &mut self.fault,
                                            "Malformed Claude sidecar frame; outcome unknown",
                                        );
                                    }
                                };
                                self.frame.clear();
                                if value["protocol"] != 1 {
                                    return preserve_observations(
                                        messages,
                                        &mut self.fault,
                                        "Unsupported Claude IPC revision",
                                    );
                                }
                                messages.push(value);
                            } else {
                                self.frame.push(byte);
                                if self.frame.len() > FRAME_LIMIT {
                                    return preserve_observations(
                                        messages,
                                        &mut self.fault,
                                        "Claude IPC frame bound exceeded",
                                    );
                                }
                            }
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
                    Err(_) => {
                        return preserve_observations(
                            messages,
                            &mut self.fault,
                            "Claude IPC read failed",
                        );
                    }
                }
            }
        }
        Ok(messages)
    }
    fn teardown(&mut self) -> bool {
        self.process.close_stdin();
        let _ = self.process.terminate(false);
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            let _ = self.process.try_wait();
            if self.process.tree_empty().unwrap_or(false) {
                return true;
            }
            if Instant::now() + Duration::from_secs(2) > deadline {
                let _ = self.process.terminate(true);
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }
}
struct SdkItem {
    kind: ItemKind,
    text: String,
    complete: bool,
}
#[derive(Default)]
struct SdkItems {
    items: HashMap<String, SdkItem>,
    retained: usize,
}
impl SdkItems {
    fn observe(&mut self, value: &Value) -> Result<&SdkItem, String> {
        let id = value["item_id"]
            .as_str()
            .ok_or("Missing SDK item identity")?;
        let text = value["text"].as_str().ok_or("Missing SDK item text")?;
        let kind = match value["kind"].as_str() {
            Some("tool") => ItemKind::Tool,
            Some("message") => ItemKind::Message,
            _ => return Err("Unsupported SDK item kind".into()),
        };
        let complete = value["complete"]
            .as_bool()
            .ok_or("Missing SDK item completion state")?;
        if id.is_empty()
            || id.len() > 256
            || safe(id) != id
            || text.len() > ITEM_LIMIT
            || (self.items.len() >= 128 && !self.items.contains_key(id))
        {
            return Err("Claude retained item bound/identity invalid".into());
        }
        let text = safe_stream(text, complete || kind != ItemKind::Message);
        let previous = self.items.get(id);
        if previous.is_some_and(|item| {
            item.kind != kind || item.complete && (!complete || item.text != text)
        }) {
            return Err("SDK item changed type or rewrote a terminal observation".into());
        }
        let retained = self.retained - previous.map_or(0, |item| item.text.len()) + text.len();
        if retained > OUTPUT_LIMIT {
            return Err("Claude retained output bound exceeded".into());
        }
        self.retained = retained;
        Ok(self
            .items
            .entry(id.into())
            .insert_entry(SdkItem {
                kind,
                text,
                complete,
            })
            .into_mut())
    }
    fn journal(
        &self,
        events: &Events,
        run: &Prepared,
        thread_id: &str,
        turn: &str,
        nonce: u128,
    ) -> Result<(), String> {
        for (id, item) in &self.items {
            if item.kind != ItemKind::Tool {
                continue;
            }
            let state = if item.complete { "final" } else { "partial" };
            emit(
                events,
                Event::Journal {
                    generation: run.generation,
                    task_id: run.input.task.id,
                    record: crate::store::ProviderRecord {
                        id: format!(
                            "claude-{}-{nonce}-{}-{id}",
                            std::process::id(),
                            run.generation
                        ),
                        agent: Agent::Claude,
                        generation: run.generation,
                        thread_id: thread_id.into(),
                        turn_id: turn.into(),
                        item_id: id.clone(),
                        origin: "Claude Agent SDK provider-native tool observation".into(),
                        coverage: format!(
                            "Provider-reported {state} snapshot only; not local runner evidence or confinement proof. A partial tool has unknown side effects/outcome; cancellation does not roll back prior actions."
                        ),
                        text: item.text.clone(),
                        complete: item.complete,
                        partner: run.input.work.as_ref().map(|claim| claim.owner.clone()),
                    },
                },
            )?;
        }
        Ok(())
    }
}

struct SdkFilePreview {
    id: String,
    edit: bool,
    stream: super::file_preview::Stream,
    phase: FilePhase,
    bytes: usize,
}
struct SdkFilePreviews {
    enabled: bool,
    generation: u64,
    calls: HashMap<String, SdkFilePreview>,
    bytes: usize,
}
impl SdkFilePreviews {
    fn new(input: &Input, generation: u64) -> Self {
        Self {
            enabled: input.operation == Operation::Implement
                && !input.inspect_models
                && !input.inspect_sessions,
            generation,
            calls: HashMap::new(),
            bytes: 0,
        }
    }
    fn observe(&mut self, value: &Value) -> Result<Option<(String, FilePhase, String)>, String> {
        if !self.enabled {
            return Ok(None);
        }
        let id = value["tool_id"]
            .as_str()
            .ok_or("Missing SDK file preview identity")?;
        let edit = match value["tool"].as_str() {
            Some("Write") => false,
            Some("Edit") => true,
            _ => return Err("Unsupported SDK file preview tool".into()),
        };
        let stage = value["stage"]
            .as_str()
            .ok_or("Missing SDK file preview stage")?;
        if id.is_empty() || id.len() > 256 || safe(id) != id {
            return Err("Invalid SDK file preview identity".into());
        }
        if !self.calls.contains_key(id) {
            if !matches!(stage, "start" | "received") {
                return Ok(None);
            }
            if self.calls.len() >= 128 {
                return Ok(None);
            }
            self.calls.insert(
                id.into(),
                SdkFilePreview {
                    id: format!(
                        "eevee-file-sdk-{}-{}",
                        self.generation,
                        self.calls.len() + 1
                    ),
                    edit,
                    stream: super::file_preview::Stream::new(edit),
                    phase: FilePhase::Generating,
                    bytes: 0,
                },
            );
        }
        let call = self.calls.get_mut(id).unwrap();
        if call.edit != edit {
            return Err("SDK preview changed tool identity".into());
        }
        if matches!(
            call.phase,
            FilePhase::Denied
                | FilePhase::Refused
                | FilePhase::Reported
                | FilePhase::ReportedFailed
                | FilePhase::Unknown
                | FilePhase::Interrupted
        ) {
            return Ok(None);
        }
        match stage {
            "start" => return Ok(None),
            "delta" => {
                if call.phase != FilePhase::Generating {
                    return Ok(None);
                }
                let fragment = value["fragment"]
                    .as_str()
                    .ok_or("Missing SDK preview fragment")?;
                if call.bytes.saturating_add(fragment.len()) > FRAME_LIMIT / 2
                    || self.bytes.saturating_add(fragment.len()) > 4 * FRAME_LIMIT
                {
                    call.phase = FilePhase::Interrupted;
                } else {
                    call.bytes += fragment.len();
                    self.bytes += fragment.len();
                    call.stream.push(fragment);
                }
            }
            "received" => {
                if call.phase == FilePhase::AwaitingReview {
                    return Ok(None);
                }
                let fragment = value["fragment"]
                    .as_str()
                    .ok_or("Missing SDK complete preview")?;
                if fragment.len() > FRAME_LIMIT / 2
                    || self.bytes.saturating_add(fragment.len()) > 4 * FRAME_LIMIT
                {
                    return Ok(None);
                }
                self.bytes += fragment.len();
                // A real complete input is a separate observation, never a
                // rewind of the authoritative tool decoder or journal state.
                call.stream = super::file_preview::Stream::new(edit);
                call.stream.push(fragment);
                call.phase = FilePhase::Received;
            }
            "stop" => {
                if call.phase != FilePhase::Generating {
                    return Ok(None);
                }
                call.phase = FilePhase::Received;
            }
            "review" => call.phase = FilePhase::AwaitingReview,
            "approved" => call.phase = FilePhase::Received,
            "denied" => call.phase = FilePhase::Denied,
            "refused" => call.phase = FilePhase::Refused,
            "reported" => call.phase = FilePhase::Reported,
            "reported_failed" => call.phase = FilePhase::ReportedFailed,
            "unknown" => call.phase = FilePhase::Unknown,
            "interrupted" => call.phase = FilePhase::Interrupted,
            _ => return Err("Unsupported SDK file preview stage".into()),
        }
        let complete = !matches!(call.phase, FilePhase::Generating | FilePhase::Interrupted);
        Ok(call
            .stream
            .text(complete)
            .map(|text| (call.id.clone(), call.phase, text)))
    }
}

fn transport(
    workspace: &Workspace,
    run: &Prepared,
    incoming: &Receiver<Request>,
    events: &Events,
    stop: &AtomicBool,
    cancelled: &AtomicU64,
    sessions: &mut HashMap<String, SdkSession>,
) -> (Outcome, String) {
    let Workspace {
        project,
        data_dir,
        exclusions,
    } = workspace;
    let mut dispatched = false;
    let result = (|| {
        if stop.load(Ordering::Acquire) || cancelled.load(Ordering::Acquire) == run.generation {
            return Ok((
                Outcome::Cancelled,
                "Claude cancelled before dispatch".into(),
            ));
        }
        revalidate(project, data_dir, &run.trust)?;
        credential_route()?;
        let (current_text, current_scope) = payload(
            data_dir,
            &run.input,
            exclusions,
            run.write_scope.project_context.as_ref(),
        )?;
        if current_text != run.text || current_scope != run.write_scope {
            return Err(
                "Task/source/edit baseline/handoff changed; preview Claude again before dispatch"
                    .into(),
            );
        }
        let mut probe_launch = run.trust.launch.clone();
        if let crate::workspace::runner::CommandSpec::Direct { args, .. } = &mut probe_launch.spec {
            args.push("--inspect".into());
        }
        let mut probe = Pipe::spawn(&probe_launch)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let observed: Result<(), String> = (|| {
            while Instant::now() < deadline
                && !stop.load(Ordering::Acquire)
                && cancelled.load(Ordering::Acquire) != run.generation
            {
                let messages = probe.pump(None)?;
                if messages.len() > 1 {
                    return Err("SDK inspect emitted unexpected additional frames".into());
                }
                if let Some(value) = messages.first() {
                    if inspection_matches(value, &run.trust.versions) {
                        let native = value["cli"]
                            .as_str()
                            .ok_or("SDK inspect omitted native executable identity")?;
                        if !Path::new(native).starts_with(&run.trust.root)
                            || fs::canonicalize(native).ok().as_deref() != Some(Path::new(native))
                        {
                            return Err(
                                "SDK inspect resolved native code outside reviewed dependency tree"
                                    .into(),
                            );
                        }
                        let runtime = value["node_executable"]
                            .as_str()
                            .ok_or("SDK inspect omitted actual Node runtime")?;
                        let sdk_entry = value["sdk_entry"]
                            .as_str()
                            .ok_or("SDK inspect omitted loaded SDK entry")?;
                        if Path::new(runtime) != run.trust.launch.executable.path.as_path()
                            || !Path::new(sdk_entry).starts_with(&run.trust.root)
                            || fs::canonicalize(sdk_entry).ok().as_deref()
                                != Some(Path::new(sdk_entry))
                        {
                            return Err("SDK/Node inspect resolved code outside reviewed runtime/package identity".into());
                        }
                        return Ok(());
                    }
                    let _ = emit(
                        events,
                        Event::ConnectionFailed {
                            generation: run.generation,
                            task_id: run.input.task.id,
                            agent: run.input.agent,
                            failure: super::diagnostics::Failure::Incompatible,
                        },
                    );
                    return Err("Installed SDK identity/bridge capability inspection failed".into());
                }
                if probe.eof {
                    return Err("SDK inspection ended without loaded capability evidence".into());
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err("SDK inspection cancelled or timed out; no model sent".into())
        })();
        let clean = probe.teardown();
        observed?;
        if !clean {
            return Err("SDK inspect process-tree cleanup unknown; no model sent".into());
        }
        revalidate(project, data_dir, &run.trust)?;
        let (current_text, current_scope) = payload(
            data_dir,
            &run.input,
            exclusions,
            run.write_scope.project_context.as_ref(),
        )?;
        if current_text != run.text || current_scope != run.write_scope {
            return Err("Context changed during SDK inspection; preview again".into());
        }
        if stop.load(Ordering::Acquire) || cancelled.load(Ordering::Acquire) == run.generation {
            return Ok((
                Outcome::Cancelled,
                "Claude cancelled before model dispatch".into(),
            ));
        }
        let mut pipe = Pipe::spawn(&run.trust.launch)?;
        let turn = if run.input.inspect_models {
            String::new()
        } else {
            format!("claude-query-{}", run.generation)
        };
        let mut prompts: BTreeMap<String, Prompt> = BTreeMap::new();
        let mut thread_id = String::new();
        let mut items = SdkItems::default();
        let mut file_previews = SdkFilePreviews::new(&run.input, run.generation);
        let mut seen_prompts = HashSet::new();
        let mut dispatch_receipt_seen = None;
        let journal_nonce = if run.input.inspect_models {
            0
        } else {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        };
        let mut usage = SdkTurnUsage::begin(&run.input, sessions)?;
        pipe.send(json!({"protocol":1,"bridge_revision":9,"type":"run","generation":run.generation,"task_id":run.input.task.id,"cwd":project,"state_dir":run.trust.state_dir,
            "request_id":if run.input.inspect_models { None } else { run.input.work.as_ref().map(|work| work.request_id.as_str()) },
            "prompt":if run.input.inspect_models { "" } else { run.text.as_str() },
            "operation":if run.input.inspect_models { "plan" } else { sdk_tool_policy(run.input.operation) },
            "write_scope":run.write_scope,
            "resume":run.input.resume,"inspect_models":run.input.inspect_models,"selection":run.input.selection}))?;
        dispatched = true;
        let mut cancelling: Option<Instant> = None;
        let mut startup = Some(Instant::now() + Duration::from_secs(30));
        let mut models_seen = false;
        let processing: Result<(Outcome, String), String> = (|| {
            loop {
                if (stop.load(Ordering::Acquire)
                    || cancelled.load(Ordering::Acquire) == run.generation)
                    && cancelling.is_none()
                {
                    cancelling = Some(Instant::now());
                    for (id, _) in std::mem::take(&mut prompts) {
                        emit(
                            events,
                            Event::PromptClosed {
                                generation: run.generation,
                                request_id: json!(id),
                            },
                        )?;
                    }
                    pipe.cancel(run.generation)?;
                }
                if cancelling.is_some_and(|at| at.elapsed() > Duration::from_secs(3)) {
                    return Ok((Outcome::Unknown, "Claude interrupt deadline: forced teardown without authoritative SDK terminal result; no rollback/replay".into()));
                }
                for request in incoming.try_iter() {
                    match request {
                        Request::Answer {
                            generation,
                            request_id,
                            answer,
                        } if generation == run.generation
                            && cancelling.is_none()
                            && cancelled.load(Ordering::Acquire) != run.generation =>
                        {
                            if let Some(id) = request_id.as_str()
                                && let Some(prompt) = prompts.get(id)
                            {
                                match provider_answer(&prompt.kind, answer) {
                                    Ok(answer) => {
                                        run.input.validate_committed(data_dir)?;
                                        revalidate(project, data_dir, &run.trust)?;
                                        if cancelled.load(Ordering::Acquire) == run.generation {
                                            pipe.cancel(run.generation)?;
                                            continue;
                                        }
                                        pipe.send(json!({"protocol":1,"type":"answer","generation":generation,"request_id":request_id,"answer":answer}))?;
                                        prompts.remove(id);
                                    }
                                    Err(message) => emit(
                                        events,
                                        Event::Status {
                                            generation,
                                            task_id: run.input.task.id,
                                            message,
                                        },
                                    )?,
                                }
                            } else {
                                emit(
                                    events,
                                    Event::Status {
                                        generation,
                                        task_id: run.input.task.id,
                                        message: "Stale Claude callback answer refused".into(),
                                    },
                                )?;
                            }
                        }
                        Request::Cancel { generation } if generation == run.generation => {
                            cancelled.fetch_max(generation, Ordering::Release);
                        }
                        _ => {
                            emit(events, Event::Status { generation: run.generation, task_id: run.input.task.id, message: "Claude is active: additional submissions/stale answers rejected, never queued".into() })?;
                        }
                    }
                }
                for value in pipe.pump(Some((cancelled, run.generation)))? {
                    if value["generation"].as_u64() != Some(run.generation)
                        || value["task_id"].as_i64() != Some(run.input.task.id)
                    {
                        return Err("Claude event correlation mismatch".into());
                    }
                    match value["type"].as_str().unwrap_or("") {
                        "dispatched" => {
                            let event = dispatch_receipt(&run.input, run.generation, &value, &mut dispatch_receipt_seen)?;
                            emit(events, event)?;
                        }
                        "models" => {
                            if !run.input.inspect_models || models_seen || !thread_id.is_empty() {
                                return Err("Unexpected/duplicate SDK model catalogue".into());
                            }
                            let catalogue = decode_catalogue(&value["catalogue"])?;
                            emit(events, Event::Models { generation: run.generation, task_id: run.input.task.id, catalogue })?;
                            models_seen = true;
                        }
                        "thread" => {
                            if run.input.inspect_models { return Err("Model inspection unexpectedly started a session".into()); }
                            let id = value["session_id"].as_str().ok_or("Missing SDK session ID")?;
                            if !thread_id.is_empty() || id.is_empty() || id.len() > 128 || safe(id) != id
                                || run.input.resume.as_ref().is_some_and(|resume| resume != id)
                                || (run.input.resume.is_none() && sessions.contains_key(id))
                            { return Err("SDK session identity mismatch/reuse; no ownership transfer".into()); }
                            thread_id = id.into(); sessions.entry(id.into()).or_insert_with(|| SdkSession { task_id: run.input.task.id, ..SdkSession::default() }); startup = None;
                            emit(events, Event::Thread { generation: run.generation, task_id: run.input.task.id, thread_id: id.into(), model: safe(value["model"].as_str().unwrap_or("unknown")), account: safe(value["account"].as_str().unwrap_or("unknown")) })?;
                        }
                        "file_preview" => {
                            if thread_id.is_empty() || value["session_id"].as_str() != Some(thread_id.as_str()) { return Err("Claude file preview precedes/widens qualified session".into()); }
                            if let Some((item_id, phase, text)) = file_previews.observe(&value)? {
                                emit(events, Event::Item { generation: run.generation, task_id: run.input.task.id, thread_id: thread_id.clone(), turn_id: turn.clone(), item_id, kind: ItemKind::File(phase), text, complete: phase != FilePhase::Generating })?;
                            }
                        }
                        "item" => {
                            if thread_id.is_empty() || value["session_id"].as_str() != Some(thread_id.as_str()) { return Err("Claude item precedes/widens qualified session".into()); }
                            let item = items.observe(&value)?;
                            emit(events, Event::Item { generation: run.generation, task_id: run.input.task.id, thread_id: thread_id.clone(), turn_id: turn.clone(), item_id: value["item_id"].as_str().unwrap().into(), kind: item.kind, text: item.text.clone(), complete: item.complete })?;
                        }
                        "prompt" => {
                            let id = value["request_id"].as_str().ok_or("Missing callback request ID")?;
                            if cancelling.is_some() || cancelled.load(Ordering::Acquire) == run.generation { pipe.cancel(run.generation)?; continue; }
                            if !prompts.is_empty() || seen_prompts.len() >= 256 || !seen_prompts.insert(id.to_owned()) || id.is_empty() || id.len() > 256 || safe(id) != id || thread_id.is_empty() || value["session_id"].as_str() != Some(thread_id.as_str()) { return Err("Stale/duplicate/concurrent/unqualified SDK callback refused".into()); }
                            run.input.validate_committed(data_dir)?;
                            let kind = prompt_kind(&value)?;
                            let prompt = Prompt { arrival: super::approval_arrival(), request_id: json!(id), thread_id: thread_id.clone(), turn_id: turn.clone(), item_id: safe(value["item_id"].as_str().unwrap_or("unknown")), kind };
                            emit(events, Event::Prompt { generation: run.generation, task_id: run.input.task.id, request: prompt.clone() })?; prompts.insert(id.into(), prompt);
                        }
                        "prompt_closed" => { let id = value["request_id"].as_str().ok_or("Invalid prompt closure")?; prompts.remove(id); emit(events, Event::PromptClosed { generation: run.generation, request_id: json!(id) })?; }
                        "status" => emit(events, Event::Status { generation: run.generation, task_id: run.input.task.id, message: safe(value["message"].as_str().unwrap_or("SDK status")) })?,
                        "usage" => {
                            let event = usage_event(&run.input, run.generation, &thread_id, dispatch_receipt_seen.as_deref(), &value)?;
                            emit(events, usage.observe(event)?)?;
                        }
                        "finished" => {
                            if run.input.inspect_models {
                                if !models_seen || value["session_id"].as_str() != Some("") || value["outcome"].as_str() != Some("completed") {
                                    return Err("Unqualified SDK model inspection terminal event".into());
                                }
                                return Ok((Outcome::Completed, "Live SDK model capabilities inspected; no model turn or journal".into()));
                            }
                            if thread_id.is_empty() || value["session_id"].as_str() != Some(thread_id.as_str()) { return Err("Unqualified SDK terminal event".into()); }
                            let outcome = match value["outcome"].as_str() { Some("completed") => Outcome::Completed, Some("failed") => Outcome::Failed, _ => Outcome::Unknown };
                            return Ok((outcome, safe(value["message"].as_str().unwrap_or("SDK terminal result observed"))));
                        }
                        "rejected" => {
                            if !thread_id.is_empty() || models_seen || value["session_id"].as_str() != Some("") {
                                return Err("Unqualified SDK pre-prompt selection rejection".into());
                            }
                            return Ok((Outcome::Failed, safe(value["message"].as_str().unwrap_or("SDK model/effort selection unsupported; no prompt sent"))));
                        }
                        "fatal" => return Err("SDK fatal transport/config/provider error; raw diagnostics withheld; outcome unknown".into()),
                        _ => return Err("Unsupported Claude sidecar event; fail closed without replay".into()),
                    }
                }
                if pipe.eof {
                    return Err(
                        "Claude sidecar EOF without terminal SDK result; outcome unknown".into(),
                    );
                }
                if startup.is_some_and(|deadline| Instant::now() > deadline) {
                    return Err("Claude SDK startup deadline exceeded; outcome unknown".into());
                }
                thread::sleep(Duration::from_millis(10));
            }
        })();
        let processing = if run.input.inspect_models {
            processing
        } else {
            items
                .journal(events, run, &thread_id, &turn, journal_nonce)
                .and(processing)
        };
        for (id, _) in prompts {
            let _ = emit(
                events,
                Event::PromptClosed {
                    generation: run.generation,
                    request_id: json!(id),
                },
            );
        }
        let clean = pipe.teardown();
        if !clean {
            return Ok((Outcome::Unknown, "Claude owned process-tree cleanup could not be confirmed; inspect external changes before any new run".into()));
        }
        if let Ok((outcome, _)) = &processing
            && let Some(session) = sessions
                .get_mut(&thread_id)
                .filter(|session| session.task_id == run.input.task.id)
        {
            usage.finish(session, *outcome);
        }
        processing
    })();
    result.unwrap_or_else(|error: String| {
        (
            if dispatched {
                Outcome::Unknown
            } else if stop.load(Ordering::Acquire)
                || cancelled.load(Ordering::Acquire) == run.generation
            {
                Outcome::Cancelled
            } else {
                Outcome::Failed
            },
            safe(&error),
        )
    })
}

fn decode_catalogue(value: &Value) -> Result<ModelCatalogue, String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireModel {
        id: String,
        name: String,
        description: String,
        efforts: Vec<String>,
        default_effort: Option<String>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireCatalogue {
        models: Vec<WireModel>,
        current: Option<String>,
        source: String,
        effort_note: Option<String>,
    }
    let wire: WireCatalogue = serde::Deserialize::deserialize(value)
        .map_err(|_| "Malformed SDK model catalogue".to_string())?;
    let catalogue = ModelCatalogue {
        models: wire
            .models
            .into_iter()
            .map(|model| ModelOption {
                id: model.id,
                name: model.name,
                description: model.description,
                efforts: model.efforts,
                default_effort: model.default_effort,
            })
            .collect(),
        current: wire.current,
        source: wire.source,
        effort_note: wire.effort_note,
    };
    catalogue.validate()?;
    Ok(catalogue)
}
fn prompt_kind(value: &Value) -> Result<PromptKind, String> {
    let input = &value["input"];
    if serde_json::to_vec(input)
        .map_err(|_| "Invalid SDK prompt")?
        .len()
        > APPROVAL_LIMIT
    {
        return Err("SDK prompt exceeds bound".into());
    }
    if value["kind"] == "questions" {
        let questions = input["questions"]
            .as_array()
            .ok_or("Invalid SDK questions")?;
        if questions.is_empty() || questions.len() > 8 {
            return Err("SDK question count unsupported".into());
        }
        let mut result = Vec::new();
        for (index, q) in questions.iter().enumerate() {
            if q["multiSelect"] == true {
                return Err("Multi-select SDK question unsupported".into());
            }
            let text = q["question"].as_str().ok_or("Invalid SDK question text")?;
            if safe(text) != text || text.len() > 4096 {
                return Err("SDK question contains private/control text".into());
            }
            let options = q["options"]
                .as_array()
                .ok_or("Invalid SDK question options")?;
            if options.len() > 16 {
                return Err("SDK question option count unsupported".into());
            }
            let mut offered = Vec::new();
            for option in options {
                let label = option["label"]
                    .as_str()
                    .ok_or("SDK option label unavailable")?;
                if safe(label) != label {
                    return Err(
                        "SDK literal option is private/control text; refused rather than rewritten"
                            .into(),
                    );
                }
                offered.push(QuestionOption {
                    label: label.into(),
                    description: safe(option["description"].as_str().unwrap_or("")),
                });
            }
            result.push(Question {
                id: index.to_string(),
                header: safe(q["header"].as_str().unwrap_or("Question")),
                question: text.into(),
                options: offered,
                other: true,
                secret: false,
            });
        }
        return Ok(PromptKind::Questions(result));
    }
    if !matches!(value["kind"].as_str(), Some("approval" | "elicitation")) {
        return Err("Unsupported SDK callback kind".into());
    }
    let mut file = None;
    let description = if matches!(input["tool"].as_str(), Some("Write" | "Edit")) {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Scope {
            kind: SdkWriteKind,
            path: String,
            directories: Vec<String>,
        }
        let scope: Scope = serde_json::from_value(input["scope"].clone())
            .map_err(|_| "SDK write prompt lacks typed create/replace scope")?;
        if std::iter::once(&scope.path)
            .chain(scope.directories.iter())
            .any(|path| path.chars().any(char::is_control) || safe(path) != *path)
        {
            return Err("SDK write scope cannot be displayed exactly".into());
        }
        let tool_input = &input["input"];
        let edit = input["tool"] == "Edit";
        let (bytes, excerpt) = if edit {
            let old = tool_input["old_string"]
                .as_str()
                .ok_or("SDK edit lacks old text")?;
            let new = tool_input["new_string"]
                .as_str()
                .ok_or("SDK edit lacks replacement text")?;
            (
                new.len(),
                format!(
                    "- {}\n+ {}",
                    super::FileReview::excerpt(old.lines().next().unwrap_or("")),
                    super::FileReview::excerpt(new.lines().next().unwrap_or(""))
                ),
            )
        } else {
            let content = tool_input["content"]
                .as_str()
                .ok_or("SDK write lacks content")?;
            (content.len(), super::FileReview::excerpt(content))
        };
        file = Some(super::FileReview {
            path: scope.path.clone(),
            destination: None,
            syntax: Some(super::RustSyntaxReview::Unavailable),
            kind: match scope.kind {
                SdkWriteKind::Create => super::FileReviewKind::Create,
                SdkWriteKind::Replace => super::FileReviewKind::Modify,
            },
            directories: scope.directories.clone(),
            bytes,
            excerpt,
            edit,
            external: true,
        });
        format!(
            "Kind: {:?}\nPath: {}\nMissing parents: {}\nExternal SDK policy, not OS confinement.\nClaude approval request: {input}",
            scope.kind,
            scope.path,
            if scope.directories.is_empty() {
                "none".into()
            } else {
                scope.directories.join(", ")
            }
        )
    } else {
        format!(
            "Claude {} request: {input}",
            value["kind"].as_str().unwrap()
        )
    };
    if description.len() > APPROVAL_LIMIT
        || description.lines().count() > 32768
        || description.chars().any(|c| {
            (c.is_control() && !matches!(c, '\n' | '\t'))
                || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        || crate::workspace::privacy::redact(&description) != description
    {
        return Err(
            "SDK permission details require truncation/redaction; no hidden grant offered".into(),
        );
    }
    let options = input["options"]
        .as_array()
        .ok_or("Missing exact SDK permission options")?;
    if options.is_empty() || options.len() > 2 {
        return Err("Unsupported SDK permission options".into());
    }
    let mut offered = Vec::new();
    for (index, option) in options.iter().enumerate() {
        let answer = &option["answer"];
        let valid = answer.as_object().is_some_and(|fields| {
            if value["kind"] == "elicitation" {
                fields.len() == 1 && matches!(answer["action"].as_str(), Some("decline" | "cancel"))
            } else {
                fields.len() == 2
                    && match answer["behavior"].as_str() {
                        Some("deny") => answer["message"].is_string(),
                        Some("allow") if index != 0 => answer["updatedInput"].is_object(),
                        _ => false,
                    }
            }
        });
        if !valid {
            return Err("SDK permission default/schema/persistent rule rejected".into());
        }
        offered.push(PermissionOption {
            label: safe(option["label"].as_str().unwrap_or("Deny")),
            description: if index == 0 {
                "No grant; no saved rule.".into()
            } else {
                "Only this exact tool input, once.".into()
            },
            answer: answer.clone(),
        });
    }
    if offered.len() != 2
        || offered[0].answer["behavior"] != "deny"
        || offered[1].answer["behavior"] != "allow"
        || offered[1].answer["updatedInput"] != input["input"]
    {
        file = None;
    }
    Ok(PromptKind::Permission {
        description,
        file,
        options: offered,
    })
}
fn provider_answer(kind: &PromptKind, answer: Value) -> Result<Value, String> {
    match kind {
        PromptKind::Permission { options, .. } => {
            if options.iter().any(|option| option.answer == answer) {
                Ok(answer)
            } else {
                Err("SDK permission answer must equal an exact current option; persistent grants refused".into())
            }
        }
        PromptKind::Questions(questions) => {
            let answers = answer["answers"]
                .as_object()
                .ok_or("SDK questions require an answers map")?;
            if answers.len() != questions.len() {
                return Err("Answer every current SDK question".into());
            }
            let mut result = serde_json::Map::new();
            for question in questions {
                let values = answers
                    .get(&question.id)
                    .and_then(|v| v["answers"].as_array())
                    .ok_or("Stale SDK question identity")?;
                if values.len() != 1 {
                    return Err("SDK questions require one explicit answer each".into());
                }
                let value = values[0]
                    .as_str()
                    .ok_or("SDK question answer must be literal text")?;
                if value.trim().is_empty() || value.len() > 8192 || safe(value) != value {
                    return Err(
                        "SDK answer is empty, oversized or contains private/display-control text"
                            .into(),
                    );
                }
                result.insert(question.question.clone(), json!(value));
            }
            Ok(json!({"behavior":"allow","updatedInput":{"answers":result}}))
        }
        _ => Err("Unsupported SDK response kind".into()),
    }
}
#[cfg(test)]
mod tests;
