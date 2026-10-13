//! Passive discovery and explicitly consented version probes; never an agent session.
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, UNIX_EPOCH};

use sha2::{Digest, Sha256};

const REVISION: u32 = 1;
const CODEX_RELEASE: &str = "0.147.0";
const OUTPUT_LIMIT: usize = 16 * 1024;
const CONFIG_LIMIT: u64 = 64 * 1024;
const EXECUTABLE_LIMIT: u64 = 256 * 1024 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const NAMES: [&str; 4] = ["Codex", "Claude Agent SDK", "OpenCode ACP", "Gemini ACP"];
const PROGRAMS: [&str; 4] = ["codex", "claude", "opencode", "gemini"];

#[derive(Clone, Debug)]
pub struct Capability {
    pub action: String,
    pub boundary: String,
    pub evidence: String,
    pub blocker: String,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub revision: u32,
    pub name: String,
    pub executable: Option<PathBuf>,
    pub argv: Vec<String>,
    pub installed: String,
    pub version: String,
    pub authentication: String,
    pub configuration: String,
    pub os: String,
    pub architecture: String,
    pub adapter: String,
    pub action: String,
    pub billing: String,
    pub probe_allowed: bool,
    pub trust_preview: Vec<String>,
    pub blockers: Vec<String>,
    pub capabilities: Vec<Capability>,
}

#[derive(Debug)]
pub enum Request {
    Discover,
    /// Only send after consent to the current profile's complete trust preview.
    Probe {
        profile: usize,
        generation: u64,
    },
}

#[derive(Debug)]
pub enum Event {
    Discovered(Vec<Profile>),
    Probed {
        profile: usize,
        generation: u64,
        result: Box<Profile>,
    },
}

pub struct Worker {
    requests: Option<SyncSender<Request>>,
    events: Receiver<Event>,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(project: PathBuf) -> io::Result<Self> {
        let (requests, incoming) = mpsc::sync_channel(1);
        let (outgoing, events) = mpsc::sync_channel(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopped);
        let thread = thread::Builder::new().name("ev-agent-preflight".into()).spawn(move || {
            let mut snapshots: Vec<Snapshot> = Vec::new();
            while !stop.load(Ordering::Acquire) {
                let request = match incoming.recv_timeout(Duration::from_millis(25)) {
                    Ok(request) => request,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let event = match request {
                    Request::Discover => {
                        snapshots = (0..NAMES.len()).map(|index| discover(index, &project)).collect();
                        Event::Discovered(snapshots.iter().map(|s| s.profile.clone()).collect())
                    }
                    Request::Probe { profile, generation } => {
                        let Some(previous) = snapshots.get(profile) else { continue };
                        let mut current = discover(profile, &project);
                        if current.stamp != previous.stamp {
                            current.profile.version = "Not probed: executable/configuration identity changed; review refreshed trust preview and consent again".into();
                        } else if current.profile.probe_allowed && !stop.load(Ordering::Acquire) {
                            match probe(profile, &current, &stop) {
                                Ok(version) => apply_version(profile, &mut current.profile, &version),
                                Err(reason) => {
                                    current.profile.version = format!("Probe failed: {reason}");
                                    current.profile.blockers.push(reason);
                                }
                            }
                        }
                        let result = Box::new(current.profile.clone());
                        snapshots[profile] = current;
                        Event::Probed { profile, generation, result }
                    }
                };
                let mut pending = event;
                loop {
                    if stop.load(Ordering::Acquire) { return; }
                    match outgoing.try_send(pending) {
                        Ok(()) => break,
                        Err(TrySendError::Full(event)) => {
                            pending = event;
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(TrySendError::Disconnected(_)) => return,
                    }
                }
            }
        })?;
        Ok(Self {
            requests: Some(requests),
            events,
            stopped,
            thread: Some(thread),
        })
    }

    /// A full bounded queue retains the caller's pending request; it is not dropped.
    pub fn try_request(&self, pending: &mut Option<Request>) -> bool {
        let Some(request) = pending.take() else {
            return false;
        };
        let Some(sender) = &self.requests else {
            *pending = Some(request);
            return false;
        };
        match sender.try_send(request) {
            Ok(()) => true,
            Err(TrySendError::Full(request) | TrySendError::Disconnected(request)) => {
                *pending = Some(request);
                false
            }
        }
    }

    pub fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
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

struct Snapshot {
    profile: Profile,
    stamp: Stamp,
    identity: Option<crate::runner::ExecutableIdentity>,
    cwd: Option<PathBuf>,
}

#[derive(PartialEq, Eq)]
struct Stamp {
    executable: Option<crate::runner::ExecutableIdentity>,
    selected: Option<PathBuf>,
    sources: Vec<SourceStamp>,
    cwd: Option<(PathBuf, String)>,
    path: Option<OsString>,
    project: Option<PathBuf>,
}

#[derive(PartialEq, Eq)]
struct SourceStamp {
    path: PathBuf,
    canonical: Option<PathBuf>,
    metadata: Option<(u64, u128, String)>,
    declaration: String,
}

fn discover(index: usize, project: &Path) -> Snapshot {
    let project_canonical = fs::canonicalize(project).ok();
    let root = project_canonical.as_deref().unwrap_or(project);
    let selected = resolve(PROGRAMS[index], root);
    let executable = selected.as_deref().and_then(|p| fs::canonicalize(p).ok());
    let identity = executable
        .as_deref()
        .and_then(|p| executable_identity(p).ok());
    let cwd = fs::canonicalize(env::temp_dir())
        .ok()
        .filter(|p| p.is_dir() && p.to_str().is_some() && !p.starts_with(root));
    let cwd_stamp = cwd.as_deref().and_then(|p| {
        crate::runner::native::directory_identity(p)
            .ok()
            .map(|id| (p.to_path_buf(), id))
    });
    let mut sources = configuration_sources(index, root);
    let sdk = if index == 1 {
        let path = root.join("node_modules/@anthropic-ai/claude-agent-sdk/package.json");
        let present = path.is_file();
        sources.push(source(path, false));
        if present {
            "Project SDK manifest present; SDK version/runtime resolution unverified; Eevee SDK bridge absent"
        } else {
            "Project SDK manifest absent; global/custom SDK installation unknown; Eevee SDK bridge absent"
        }
    } else {
        ""
    };
    let mut blockers = vec![
        "No Eevee adapter implemented; agent runs remain disabled".into(),
        "Authentication/eligibility unverified; no status/login/model request was made".into(),
        "Filesystem/network enforcement and protocol capabilities unnegotiated".into(),
    ];
    let mut probe_allowed = identity.is_some() && cwd_stamp.is_some();
    if selected.is_none() {
        blockers.push(format!(
            "{} not found on PATH; install it outside the repository and refresh",
            PROGRAMS[index]
        ));
    } else if identity.is_none() {
        blockers.push("Executable identity unavailable, non-Unicode, changed during inspection, or exceeds 256 MiB; repair installation and refresh".into());
    }
    if cwd_stamp.is_none() {
        blockers.push("No canonical temporary working directory outside the project; repair the OS temporary directory and refresh".into());
    }
    #[cfg(windows)]
    if executable
        .as_ref()
        .and_then(|p| p.extension())
        .is_some_and(|e| !e.eq_ignore_ascii_case("exe"))
    {
        probe_allowed = false;
        blockers.push("Windows script/batch wrappers are not qualified for direct probing; install a native executable (no implicit shell)".into());
    }
    // Keep the cross-platform declaration immutable on Unix without duplicating admission logic.
    if project_canonical.is_none() {
        probe_allowed = false;
        blockers.push(
            "Project identity unavailable; reopen a valid project before executable consent".into(),
        );
    }
    if index == 1 {
        blockers.push(sdk.into());
    }
    let configuration = format!(
        "{}\nEffective configuration/trust policy unresolved: no provider loader executed; declarations below are not enforcement. Credential stores are never read.{}",
        sources
            .iter()
            .map(|s| format!("{}: {}", safe_path(&s.path), s.declaration))
            .collect::<Vec<_>>()
            .join("\n"),
        if index == 2 && env::var_os("OPENCODE_CONFIG_CONTENT").is_some() {
            " OPENCODE_CONFIG_CONTENT is present; inline content intentionally not inspected."
        } else {
            ""
        }
    );
    let (authentication, billing) = auth_labels(index);
    let mut trust_preview = vec![
        "Trust executes code, even --version. Stay does not execute anything. Consent is only for this version probe, not an agent session or login.".into(),
        format!("Selected PATH entry: {}; canonical executable: {}", selected.as_deref().map(safe_path).unwrap_or_else(|| "missing".into()), executable.as_deref().map(safe_path).unwrap_or_else(|| "unresolved".into())),
        "Literal argv: [--version]; stdin closed; stdout/stderr combined capture limited to 16 KiB; timeout 3 seconds. Only a strictly recognized version is retained; raw output is discarded.".into(),
        format!("Probe cwd: {} (outside project); native host: {}/{}", cwd.as_deref().map(safe_path).unwrap_or_else(|| "unavailable".into()), env::consts::OS, env::consts::ARCH),
        "The executable inherits Eevee's environment, including provider-owned authentication and PATH. Wrappers, shebang interpreters, dynamic libraries, dependencies, environment preload hooks and provider configuration can execute code or persist changes. These dependencies are NOT fully pinned or sandboxed. Trust them explicitly or Stay; Eevee never loads a hook itself.".into(),
        "Executable bytes and known config-source metadata/declarations are rechecked before launch. This is not atomic execution isolation; malicious concurrent replacement or undiscovered transitive configuration is not bounded by this preview.".into(),
        configuration.clone(),
        "No authentication flow, protocol handshake, model request, global config write, or credential-store read is performed by Eevee. Arbitrary trusted executable side effects cannot be guaranteed absent; process ownership is not a sandbox.".into(),
    ];
    if executable.as_deref().is_some_and(|p| p.starts_with(root))
        || selected.as_deref().is_some_and(|p| p.starts_with(root))
    {
        trust_preview.insert(1, "REPOSITORY-SUPPLIED executable/wrapper: explicit trust required; never automatically executed.".into());
    }
    if index == 0 {
        trust_preview.push("Codex 0.147.0 is a documented reference, not an observed installation or a supported adapter. Workspace-write thread startup may persist project trust; this preflight never starts a thread.".into());
    }
    let capabilities = capabilities();
    let profile = Profile {
        revision: REVISION,
        name: NAMES[index].into(),
        executable,
        argv: vec!["--version".into()],
        installed: match (selected.is_some(), index) {
            (false, 1) => format!("Claude CLI missing on PATH. {sdk}"),
            (true, 1) => format!("Claude CLI found (not the SDK). {sdk}"),
            (false, _) => "Missing on PATH; no executable invoked".into(),
            (true, _) => "PATH candidate found; installed is not authenticated or supported".into(),
        },
        version: if index == 0 {
            format!("Unknown (not probed); documented reference {CODEX_RELEASE}")
        } else {
            "Unknown (not probed); protocol/SDK version not observed".into()
        },
        authentication,
        configuration,
        os: env::consts::OS.into(),
        architecture: env::consts::ARCH.into(),
        adapter: "Unavailable — no adapter implemented; disconnected".into(),
        action: "Agent runs disabled; local workbench remains available".into(),
        billing,
        probe_allowed,
        trust_preview,
        blockers,
        capabilities,
    };
    let stamp = Stamp {
        executable: identity.clone(),
        selected,
        sources,
        cwd: cwd_stamp,
        path: env::var_os("PATH"),
        project: project_canonical,
    };
    Snapshot {
        profile,
        stamp,
        identity,
        cwd,
    }
}

fn capabilities() -> Vec<Capability> {
    let mut result = vec![Capability {
        action: "Version probe".into(), boundary: "Eevee-mediated".into(),
        evidence: "Explicit trust only; bounded owned child; recognized CLI version only, not protocol evidence".into(),
        blocker: "Requires reviewed executable/config trust; cannot sandbox trusted code".into(),
    }];
    for action in [
        "Text/image/excerpt input",
        "Streamed output and tool observation",
        "Command/file/permission requests",
        "Human questions",
        "Turn interruption",
        "Resume/fork",
        "Usage and billing",
        "Filesystem/network restrictions",
    ] {
        result.push(Capability {
            action: action.into(),
            boundary: "Observed/external — enforcement unknown".into(),
            evidence: "Documented interface only; no adapter or handshake, no observed capability"
                .into(),
            blocker: "Disabled: no qualified adapter/version/authentication/enforcement evidence"
                .into(),
        });
    }
    result.push(Capability {
        action: "Provider configuration and tool policy".into(),
        boundary: "Agent-policy-controlled".into(),
        evidence: "Source presence/nonsecret declarations only; effective policy unresolved".into(),
        blocker: "A cwd or permission callback is not OS isolation".into(),
    });
    result
}

fn auth_labels(index: usize) -> (String, String) {
    let keys: &[&str] = match index {
        0 => &["OPENAI_API_KEY", "CODEX_API_KEY"],
        1 => &[
            "ANTHROPIC_API_KEY",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
        ],
        2 => &[
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GOOGLE_GENERATIVE_AI_API_KEY",
        ],
        _ => &[
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "GOOGLE_GENAI_USE_VERTEXAI",
        ],
    };
    let present: Vec<&str> = keys
        .iter()
        .copied()
        .filter(|key| env::var_os(key).is_some())
        .collect();
    let label = if present.is_empty() {
        "No recognized environment source present; provider-owned login/store status unknown".into()
    } else {
        format!(
            "Environment source labels present: {}; values never inspected; validity/selection unknown",
            present.join(", ")
        )
    };
    let billing = if present.is_empty() {
        "Unknown — provider-owned account/plan or credential store not queried; no recognized environment billing source label".into()
    } else {
        format!(
            "Candidate environment source labels: {}; effective provider/account/plan and billing selection unknown (not authenticated)",
            present.join(", ")
        )
    };
    (format!("Unverified — {label}"), billing)
}

fn configuration_sources(index: usize, project: &Path) -> Vec<SourceStamp> {
    let home = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let mut paths: Vec<(PathBuf, bool)> = Vec::new();
    match index {
        0 => {
            if let Some(base) = env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .or_else(|| home.as_ref().map(|p| p.join(".codex")))
            {
                paths.push((base.join("config.toml"), true));
                paths.push((base.join("requirements.toml"), false));
            }
            #[cfg(unix)]
            for name in [
                "/etc/codex/config.toml",
                "/etc/codex/managed_config.toml",
                "/etc/codex/requirements.toml",
            ] {
                paths.push((name.into(), false));
            }
            for parent in project.ancestors().take(32) {
                paths.push((parent.join(".codex/config.toml"), true));
            }
        }
        1 => {
            if let Some(base) = env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .or_else(|| home.as_ref().map(|p| p.join(".claude")))
            {
                paths.push((base.join("settings.json"), true));
            }
            for name in [
                ".claude/settings.json",
                ".claude/settings.local.json",
                ".mcp.json",
                ".claude/commands",
                ".claude/agents",
                ".claude/plugins",
            ] {
                paths.push((
                    project.join(name),
                    name.ends_with("settings.json") || name.ends_with("settings.local.json"),
                ));
            }
        }
        2 => {
            if let Some(base) = env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| home.as_ref().map(|p| p.join(".config")))
            {
                for name in ["opencode.json", "opencode.jsonc"] {
                    paths.push((base.join("opencode").join(name), name.ends_with(".json")));
                }
            }
            for parent in project.ancestors().take(32) {
                for name in ["opencode.json", "opencode.jsonc", ".opencode"] {
                    paths.push((parent.join(name), name == "opencode.json"));
                }
            }
            if let Some(path) = env::var_os("OPENCODE_CONFIG") {
                paths.push((path.into(), false));
            }
        }
        _ => {
            if let Some(home) = home {
                paths.push((home.join(".gemini/settings.json"), true));
            }
            paths.push((project.join(".gemini/settings.json"), true));
            #[cfg(unix)]
            paths.push(("/etc/gemini-cli/settings.json".into(), true));
            for key in [
                "GEMINI_CLI_SYSTEM_SETTINGS_PATH",
                "GEMINI_CLI_SYSTEM_DEFAULTS_PATH",
            ] {
                if let Some(path) = env::var_os(key) {
                    paths.push((path.into(), false));
                }
            }
        }
    }
    paths
        .into_iter()
        .map(|(path, inspect)| {
            let path = if path.is_absolute() {
                path
            } else {
                project.join(path)
            };
            source(path, inspect)
        })
        .collect()
}

fn source(path: PathBuf, inspect: bool) -> SourceStamp {
    let canonical = fs::canonicalize(&path).ok();
    let metadata = fs::metadata(&path).ok().map(|m| metadata_stamp(&m));
    let declaration = match fs::metadata(&path) {
        Ok(m) if m.is_file() && inspect => match read_regular(&path, CONFIG_LIMIT) {
            Ok(bytes) => nonsecret_declarations(&path, &bytes),
            Err(_) => "present; content unavailable/oversized; effective policy unknown".into(),
        },
        Ok(_) => "present; metadata only (no loader or plugin executed)".into(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => "absent".into(),
        Err(_) => "unresolved/inaccessible; not assumed absent".into(),
    };
    SourceStamp {
        path,
        canonical,
        metadata,
        declaration,
    }
}

/// Only fixed policy enums/booleans escape this function. Never retain model/provider URLs,
/// credentials, command/plugin text, arbitrary strings or a hash of configuration contents.
fn nonsecret_declarations(path: &Path, bytes: &[u8]) -> String {
    let mut declarations = Vec::new();
    if path.extension().is_some_and(|s| s == "toml") {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return "present; invalid UTF-8; effective policy unknown".into();
        };
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                break;
            } // Never mistake a scoped declaration for a root policy.
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            let allowed: &[&str] = match key {
                "approval_policy" => &["untrusted", "on-failure", "on-request", "never"],
                "sandbox_mode" => &["read-only", "workspace-write", "danger-full-access"],
                _ => &[],
            };
            if allowed.contains(&value) {
                declarations.push(format!("{key}={value}"));
            }
        }
    } else if path.extension().is_some_and(|s| s == "json") {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) {
            for (pointer, label) in [
                ("/disableAllHooks", "disableAllHooks"),
                ("/security/disableYoloMode", "security.disableYoloMode"),
            ] {
                if let Some(value) = value.pointer(pointer).and_then(|v| v.as_bool()) {
                    declarations.push(format!("{label}={value}"));
                }
            }
            for key in [
                "hooks",
                "mcpServers",
                "plugin",
                "plugins",
                "permission",
                "permissions",
            ] {
                if value.get(key).is_some() {
                    declarations.push(format!("{key}: declared (contents withheld)"));
                }
            }
        } else {
            return "present; JSON unparsed; effective policy unknown".into();
        }
    }
    if declarations.is_empty() {
        "present; no safely recognized root policy; effective policy unknown".into()
    } else {
        format!(
            "present; nonsecret declarations [{}]; effective policy unknown",
            declarations.join(", ")
        )
    }
}

fn resolve(program: &str, project: &Path) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for directory in env::split_paths(&path).take(256) {
        let directory = if directory.is_absolute() {
            directory
        } else {
            project.join(directory)
        };
        #[cfg(windows)]
        let candidates = [
            directory.join(format!("{program}.exe")),
            directory.join(format!("{program}.cmd")),
            directory.join(format!("{program}.bat")),
            directory.join(program),
        ];
        #[cfg(not(windows))]
        let candidates = [directory.join(program)];
        for candidate in candidates {
            if crate::runner::native::is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn metadata_stamp(metadata: &Metadata) -> (u64, u128, String) {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|t| t.as_nanos())
        .unwrap_or(0);
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!(
            "{}:{}:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.mode(),
            metadata.uid(),
            metadata.gid(),
            metadata.ctime(),
            metadata.ctime_nsec()
        )
    };
    #[cfg(not(unix))]
    let identity = format!(
        "{:?}:{}",
        metadata.created().ok(),
        metadata.permissions().readonly()
    );
    (metadata.len(), modified, identity)
}

fn open_regular(path: &Path, limit: u64) -> io::Result<File> {
    // Avoid blocking on a substituted FIFO/device. Symlinks are disclosed and revalidated.
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(io::Error::other("Not a bounded regular file"));
    }
    Ok(file)
}

fn read_regular(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut file = open_regular(path, limit)?;
    let before = metadata_stamp(&file.metadata()?);
    let mut bytes = Vec::with_capacity((before.0 as usize).min(limit as usize));
    (&mut file).take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit || metadata_stamp(&file.metadata()?) != before {
        return Err(io::Error::other("Source changed during inspection"));
    }
    Ok(bytes)
}

fn executable_identity(path: &Path) -> io::Result<crate::runner::ExecutableIdentity> {
    if path.to_str().is_none() {
        return Err(io::Error::other("Non-Unicode executable"));
    }
    let mut file = open_regular(path, EXECUTABLE_LIMIT)?;
    let before = metadata_stamp(&file.metadata()?);
    let file_id = crate::runner::native::file_identity(&file)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 32 * 1024];
    let mut size = 0;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > EXECUTABLE_LIMIT {
            return Err(io::Error::other("Executable too large"));
        }
        digest.update(&buffer[..n]);
    }
    if metadata_stamp(&file.metadata()?) != before || size != before.0 {
        return Err(io::Error::other("Executable changed"));
    }
    Ok(crate::runner::ExecutableIdentity {
        path: path.to_path_buf(),
        sha256: format!("{:x}", digest.finalize()),
        bytes: size,
        modified_ns: before.1,
        file_id: format!("{file_id}:{}", before.2),
    })
}

fn safe_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    crate::privacy::redact(&text)
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .take(1024)
        .collect()
}

fn apply_version(index: usize, profile: &mut Profile, version: &str) {
    profile.version = format!(
        "Observed CLI version {version} on {}/{}; protocol/SDK version unknown",
        profile.os, profile.architecture
    );
    if index == 0 {
        if version == CODEX_RELEASE {
            profile
                .version
                .push_str("; matches documented reference only (adapter still unavailable)");
        } else {
            let blocker = format!(
                "Version mismatch: observed {version}, documented Codex reference {CODEX_RELEASE}; install a reviewed release or qualify this release before any adapter run"
            );
            profile
                .version
                .push_str("; DOES NOT MATCH documented reference 0.147.0");
            profile.blockers.push(blocker);
        }
    } else {
        profile.blockers.push(
            "CLI version observed only; no qualified SDK/ACP protocol compatibility range exists"
                .into(),
        );
    }
}

/// Fail closed on extra output, escapes, arbitrary identifiers and credential-like payloads.
fn parse_version(index: usize, bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    let version = match index {
        0 => text.strip_prefix("codex-cli ")?,
        1 => text.strip_suffix(" (Claude Code)")?,
        2 | 3 => text,
        _ => return None,
    };
    if version.len() > 32 {
        return None;
    }
    let mut parts = version.split('.');
    for _ in 0..3 {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 8 || !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }
    if parts.next().is_some() {
        return None;
    }
    Some(version.to_owned())
}

fn probe(index: usize, snapshot: &Snapshot, stopped: &AtomicBool) -> Result<String, String> {
    if stopped.load(Ordering::Acquire) {
        return Err("Probe cancelled during shutdown".into());
    }
    let executable = snapshot
        .identity
        .clone()
        .ok_or("Executable identity unavailable")?;
    let cwd = snapshot
        .cwd
        .clone()
        .ok_or("Probe working directory unavailable")?;
    let prepared = crate::runner::Prepared {
        request_id: "agent-version-probe".into(),
        run_id: "agent-version-probe".into(),
        task_id: 0,
        spec: crate::runner::CommandSpec::Direct {
            program: executable
                .path
                .to_str()
                .ok_or("Non-Unicode executable")?
                .into(),
            args: vec!["--version".into()],
        },
        cwd_identity: crate::runner::native::directory_identity(&cwd)
            .map_err(|_| "Probe working directory identity unavailable")?,
        executable,
        cwd,
        prepared_at_ms: 0,
        expires_at_ms: 0,
        before: crate::runner::InputSnapshot::default(),
    };
    let mut process = crate::runner::native::Process::spawn(&prepared).map_err(|_| "Could not start the trusted executable; repair executable/interpreter installation (details withheld)".to_owned())?;
    let start = Instant::now();
    let mut stdout = Vec::with_capacity(256);
    let mut bytes_seen = 0;
    let mut eof = [false; 2];
    let mut exit = None;
    let mut buffer = [0u8; 4096];
    let result = 'capture: loop {
        if stopped.load(Ordering::Acquire) {
            break Err("Probe cancelled during shutdown".into());
        }
        if start.elapsed() >= PROBE_TIMEOUT {
            break Err("Version probe timed out after 3 seconds; no version accepted".into());
        }
        for (stream, ended) in eof.iter_mut().enumerate() {
            if *ended {
                continue;
            }
            match process.read(stream, &mut buffer) {
                Ok(0) => *ended = true,
                Ok(n) => {
                    bytes_seen += n;
                    if bytes_seen > OUTPUT_LIMIT {
                        break 'capture Err("Version output exceeded 16 KiB; discarded".into());
                    }
                    if stream == 0 {
                        stdout.extend_from_slice(&buffer[..n]);
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => break 'capture Err("Version capture failed; output discarded".into()),
            }
        }
        if exit.is_none() {
            match process.try_wait() {
                Ok(status) => exit = status,
                Err(_) => break Err("Version process status unavailable".into()),
            }
        }
        if let Some((code, _)) = exit
            && eof.iter().all(|ended| *ended)
        {
            break if code == Some(0) {
                parse_version(index, &stdout).ok_or_else(|| "Unrecognized version output; raw output withheld; compatibility remains unknown".into())
            } else {
                Err("Version executable exited unsuccessfully; diagnostics withheld".into())
            };
        }
        thread::sleep(Duration::from_millis(10));
    };
    let _ = process.terminate(true);
    let cleanup = Instant::now();
    while cleanup.elapsed() < Duration::from_millis(500) {
        let _ = process.try_wait();
        if matches!(process.tree_empty(), Ok(true)) {
            return result;
        }
        thread::sleep(Duration::from_millis(10));
    }
    // Native RAII retains kill-on-close/process-group cleanup and a bounded direct-child reaper.
    Err(
        "Owned process-tree exit could not be established; outcome unknown, no version accepted"
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_evidence_is_strict_and_never_retains_arbitrary_output() {
        assert_eq!(
            parse_version(0, b"codex-cli 0.152.1\n"),
            Some("0.152.1".into())
        );
        assert_eq!(
            parse_version(1, b"2.1.91 (Claude Code)\n"),
            Some("2.1.91".into())
        );
        assert_eq!(
            parse_version(0, b"codex-cli 0.147.0\nOPENAI_API_KEY=secret"),
            None
        );
        assert_eq!(parse_version(0, b"codex-cli \x1b[31m0.147.0"), None);
        assert_eq!(parse_version(0, b"codex-cli sk-secret"), None);
        assert_eq!(parse_version(3, b"0.1.0-beta-secret"), None);
    }

    #[test]
    fn configuration_discloses_only_fixed_nonsecret_policy_values() {
        let text = nonsecret_declarations(Path::new("config.toml"), b"api_key = 'secret-canary'\nsandbox_mode = 'read-only'\n[profiles.other]\napproval_policy = 'never'\n");
        assert!(text.contains("sandbox_mode=read-only"));
        assert!(!text.contains("secret-canary"));
        assert!(!text.contains("approval_policy"));
        let text = nonsecret_declarations(Path::new("settings.json"), br#"{"apiKey":"secret-canary","hooks":{"command":"leak-canary"},"disableAllHooks":true}"#);
        assert!(text.contains("disableAllHooks=true"));
        assert!(text.contains("hooks: declared"));
        assert!(!text.contains("canary"));
    }

    #[test]
    fn documented_release_is_not_observed_or_supported() {
        let mut profile = discover(0, Path::new("/nonexistent-eevee-project")).profile;
        assert!(profile.version.starts_with("Unknown"));
        assert!(!profile.probe_allowed);
        apply_version(0, &mut profile, "0.152.1");
        assert!(
            profile
                .blockers
                .iter()
                .any(|b| b.contains("Version mismatch"))
        );
        assert!(profile.action.contains("disabled"));
        apply_version(0, &mut profile, CODEX_RELEASE);
        assert!(profile.action.contains("disabled"));
        assert!(profile.version.contains("documented reference only"));
    }

    #[test]
    fn config_content_is_not_fingerprinted_or_exposed() {
        let a = nonsecret_declarations(Path::new("config.toml"), b"api_key='first-secret'");
        let b = nonsecret_declarations(Path::new("config.toml"), b"api_key='second-secret'");
        assert_eq!(a, b);
    }

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let path = env::temp_dir().join(format!(
                "ev-agent-test-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        #[cfg(unix)]
        fn executable(&self, script: &str) -> Snapshot {
            use std::os::unix::fs::PermissionsExt;
            let path = self.0.join("codex");
            fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            let mut snapshot = discover(0, &self.0);
            snapshot.identity = Some(executable_identity(&path).unwrap());
            snapshot.cwd = Some(self.0.clone());
            snapshot
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn configuration_changes_invalidate_the_consent_snapshot() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.toml");
        let absent = source(path.clone(), true);
        fs::write(&path, "sandbox_mode='read-only'\n").unwrap();
        let initial = source(path.clone(), true);
        assert!(initial != absent);
        fs::write(&path, "sandbox_mode='danger-full-access'\n").unwrap();
        assert!(initial != source(path, true));
    }

    #[test]
    fn executable_replacement_invalidates_identity_even_with_identical_bytes() {
        let fixture = Fixture::new();
        let path = fixture.0.join("program");
        fs::write(&path, b"same executable bytes").unwrap();
        let original = executable_identity(&path).unwrap();
        fs::rename(&path, fixture.0.join("old-program")).unwrap();
        fs::write(&path, b"same executable bytes").unwrap();
        assert_ne!(original, executable_identity(&path).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn consented_probe_keeps_only_a_recognized_version() {
        let fixture = Fixture::new();
        let snapshot = fixture.executable("printf 'codex-cli 0.152.1\\n'");
        assert_eq!(
            probe(0, &snapshot, &AtomicBool::new(false)).unwrap(),
            "0.152.1"
        );
        let snapshot = fixture.executable("printf 'codex-cli 0.152.1\\nsecret-canary\\n'");
        let error = probe(0, &snapshot, &AtomicBool::new(false)).unwrap_err();
        assert!(error.contains("withheld"));
        assert!(!error.contains("secret-canary"));
    }

    #[cfg(unix)]
    #[test]
    fn excessive_version_output_is_bounded() {
        let fixture = Fixture::new();
        let snapshot = fixture.executable("while :; do printf '0123456789012345678901234567890123456789012345678901234567890123456789'; done");
        let start = Instant::now();
        let error = probe(0, &snapshot, &AtomicBool::new(false)).unwrap_err();
        assert!(error.contains("16 KiB"));
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn hung_probe_and_shutdown_have_bounded_owned_cleanup() {
        let fixture = Fixture::new();
        let snapshot = fixture.executable("while :; do :; done");
        let start = Instant::now();
        let error = probe(0, &snapshot, &AtomicBool::new(false)).unwrap_err();
        assert!(error.contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(5));
        let start = Instant::now();
        let error = probe(0, &snapshot, &AtomicBool::new(true)).unwrap_err();
        assert!(error.contains("cancelled"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
