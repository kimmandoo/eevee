//! Passive discovery, consented version probes, and immutable Codex planning trust.
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
pub(crate) const DEVIN: usize = 4;
/// Trust index of `ant`; not a provider row in the connection drawer.
pub(crate) const ANT: usize = 5;
const PROVIDERS: usize = 5;
const OUTPUT_LIMIT: usize = 16 * 1024;
const CONFIG_LIMIT: u64 = 64 * 1024;
const EXECUTABLE_LIMIT: u64 = 256 * 1024 * 1024;
fn probe_timeout(index: usize) -> Duration {
    // Gemini's Node dependency graph exceeded three seconds on the qualified host.
    Duration::from_secs(if index == 3 { 10 } else { 3 })
}
const NAMES: [&str; 6] = [
    "Codex",
    "Claude Agent SDK",
    "OpenCode ACP",
    "Gemini ACP",
    "Devin ACP",
    "Anthropic CLI (ant)",
];
const PROGRAMS: [&str; 6] = ["codex", "claude", "opencode", "gemini", "devin", "ant"];

pub(crate) fn version_argument(index: usize) -> &'static str {
    if index == DEVIN {
        "version"
    } else {
        "--version"
    }
}

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
    PrepareAuth {
        profile: usize,
        generation: u64,
        method: crate::providers::auth::Method,
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
    AuthPrepared {
        profile: usize,
        generation: u64,
        result: Result<Box<crate::providers::auth::Plan>, String>,
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
                        snapshots = (0..PROVIDERS).map(|index| discover(index, &project)).collect();
                        let mut profiles: Vec<_> = snapshots.iter().map(|s| s.profile.clone()).collect();
                        profiles.extend(crate::providers::Agent::ALL.into_iter().filter(|agent| agent.is_direct()).map(crate::providers::direct::config::profile));
                        Event::Discovered(profiles)
                    }
                    Request::PrepareAuth { profile, generation, method } => {
                        Event::AuthPrepared { profile, generation, result: crate::providers::auth::Plan::prepare(profile, method, &project).map(Box::new) }
                    }
                    Request::Probe { profile, generation }
                        if crate::providers::Agent::from_index(profile).is_some_and(crate::providers::Agent::is_direct) => {
                        let agent = crate::providers::Agent::from_index(profile).expect("guarded provider");
                        Event::Probed { profile, generation, result: Box::new(crate::providers::direct::config::profile(agent)) }
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
    identity: Option<crate::workspace::runner::ExecutableIdentity>,
    cwd: Option<PathBuf>,
}

#[derive(PartialEq, Eq, serde::Serialize)]
struct Stamp {
    executable: Option<crate::workspace::runner::ExecutableIdentity>,
    selected: Option<PathBuf>,
    sources: Vec<SourceStamp>,
    cwd: Option<(PathBuf, String)>,
    path: Option<OsString>,
    project: Option<PathBuf>,
}

#[derive(PartialEq, Eq, serde::Serialize)]
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
        crate::workspace::runner::native::directory_identity(p)
            .ok()
            .map(|id| (p.to_path_buf(), id))
    });
    let mut sources = configuration_sources(index, root);
    let sdk = if index == 1 {
        let path = root.join("node_modules/@anthropic-ai/claude-agent-sdk/package.json");
        let present = path.is_file();
        sources.push(source(path, false));
        if present {
            "Project SDK manifest present; sidecar installation and SDK/native runtime still require explicit qualification"
        } else {
            "Project SDK manifest absent; optional user-installed SDK sidecar is inspected separately, not established by Claude CLI"
        }
    } else {
        ""
    };
    let mut blockers = vec![
        "Adapter disconnected; e separately previews executable/context trust and runtime qualification".into(),
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
        if ![
            "ANTHROPIC_API_KEY",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
        ]
        .iter()
        .any(|key| env::var_os(key).is_some())
            && crate::providers::claude::console_credentials().is_none()
        {
            blockers.push("Claude Agent requires permitted API/cloud credentials or an Anthropic Console OAuth profile (Connect > ant auth login). Existing claude.ai subscription login is not reused by this integration.".into());
        }
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
        "Trust executes code, even a version command. Stay does not execute anything. Consent is only for this version probe, not an agent session or login.".into(),
        format!("Selected PATH entry: {}; canonical executable: {}", selected.as_deref().map(safe_path).unwrap_or_else(|| "missing".into()), executable.as_deref().map(safe_path).unwrap_or_else(|| "unresolved".into())),
        format!("Literal argv: [{}]; stdin closed; stdout/stderr combined capture limited to 16 KiB; timeout {} seconds. Only a strictly recognized version is retained; raw output is discarded.", version_argument(index), probe_timeout(index).as_secs()),
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
        trust_preview.push("Codex compatibility is qualified by the app-server protocol and acknowledged policy, not an exact CLI release. This version probe does not start a planning session or establish authentication. Workspace-write startup can persist project trust; Eevee planning requests read-only instead.".into());
    }
    let capabilities = capabilities(index);
    let profile = Profile {
        revision: REVISION,
        name: NAMES[index].into(),
        executable,
        argv: vec![version_argument(index).into()],
        installed: match (selected.is_some(), index) {
            (false, 1) => format!("Claude CLI missing on PATH. {sdk}"),
            (true, 1) => format!("Claude CLI found (not the SDK). {sdk}"),
            (false, _) => "Missing on PATH; no executable invoked".into(),
            (true, _) => "PATH candidate found; installed is not authenticated or supported".into(),
        },
        version: "Unknown (not probed); protocol/SDK version not observed".into(),
        authentication,
        configuration,
        os: env::consts::OS.into(),
        architecture: env::consts::ARCH.into(),
        adapter: match index {
            0 => "Codex app-server; Plan/Review and native controlled-file implementation".into(),
            1 => "Optional Claude Agent SDK sidecar; e performs separate runtime/auth qualification".into(),
            2 => "OpenCode / negotiated ACP v1; provider-owned tools".into(),
            DEVIN => "Devin / negotiated ACP v1; provider-owned tools and native account login".into(),
            ANT => "Anthropic CLI; Console OAuth login/status handoff for Claude only".into(),
            _ => "Gemini / negotiated ACP v1; absent credentials remain unavailable".into(),
        },
        action: "Requires explicit e request preview, supported installed release, authentication, negotiated action and control boundary; local workbench always remains available".into(),
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

fn capabilities(index: usize) -> Vec<Capability> {
    let mut result = vec![Capability {
        action: "Version probe".into(), boundary: "Eevee-mediated".into(),
        evidence: "Explicit trust only; bounded owned child; recognized CLI version only, not protocol evidence".into(),
        blocker: "Requires reviewed executable/config trust; cannot sandbox trusted code".into(),
    }];
    if index == 0 {
        for (action, boundary, evidence, blocker) in [
            (
                "Text/excerpt planning",
                "Eevee-mediated",
                "Policy-acknowledged JSONL adapter; immutable full outgoing payload review and dispatch revalidation",
                "Requires separate trust, account/model read and protocol qualification; images unsupported",
            ),
            (
                "Streamed output and human questions",
                "Eevee-mediated",
                "Bounded item-ID snapshots; Plan-mode question responses; private reasoning excluded",
                "Only an explicitly dispatched session supplies live evidence",
            ),
            (
                "Command/file/permission requests",
                "Eevee-mediated callbacks",
                "Implement: model-proposed workspace paths, optionally restricted by /paths; exact creation/replacement and missing parents via Eevee tool with one-shot patch approval; at most 1024 files, 4 MiB per file; native edits/escalation refused",
                "Not every provider/MCP/hook action necessarily reaches a callback",
            ),
            (
                "Turn interruption",
                "Eevee-owned transport",
                "Provider interrupt plus bounded process-group/Job teardown",
                "Without authoritative completion, turn outcome remains unknown",
            ),
            (
                "Same-task session resume",
                "Provider-owned history",
                "Explicit same-task thread resume only; no automatic replay",
                "Provider persists history; Eevee session-local ownership only; fork unsupported",
            ),
            (
                "Filesystem/network restrictions",
                "Agent-policy-controlled",
                "Requests read-only/restricted-network policy and checks thread acknowledgment",
                "Not Eevee OS isolation; independent provider reads/MCP/hooks are outside source exclusions",
            ),
        ] {
            result.push(Capability {
                action: action.into(),
                boundary: boundary.into(),
                evidence: evidence.into(),
                blocker: blocker.into(),
            });
        }
        return result;
    }
    for action in [
        "Text/excerpt requests (images/audio unavailable)",
        "Streamed output and tool observation",
        "Command/file/permission requests",
        "Human questions",
        "Turn interruption",
        "Explicit same-task resume (fork unavailable)",
        "Usage and billing",
        "Filesystem/network restrictions",
    ] {
        if index >= 2 && action == "Human questions" {
            result.push(Capability {
                action: action.into(), boundary: "Unsupported client callback".into(),
                evidence: "These ACP adapters do not implement a negotiated structured question/elicitation extension".into(),
                blocker: "Unknown question requests are refused. Explicit reviewed follow-up turns remain separate; no missing method is invoked".into(),
            });
            continue;
        }
        if index >= 2 && action == "Usage and billing" {
            result.push(Capability {
                action: action.into(), boundary: "Provider-owned / unavailable".into(),
                evidence: "No standardized usage accounting is negotiated by these ACP adapters".into(),
                blocker: "Usage/cost remain unknown; no token total or cross-provider cost sum is inferred".into(),
            });
            continue;
        }
        result.push(Capability {
            action: action.into(),
            boundary: if action == "Filesystem/network restrictions" { "Observed/external — no Eevee confinement" } else { "Eevee-mediated protocol callbacks / provider-owned tools" }.into(),
            evidence: if index == 1 { "Optional official SDK sidecar bridge revision 10; display-only file previews never grant effects. Host discovery and frozen read/protection/exclusion checks precede source tools. Implement alone exposes the in-process host command MCP route; each request uses the shared owned runner with separate exact command/cwd/executable consent, durable settlement, real exit/stdout/stderr and retained artifact handles. Routing approval never authorizes execution. Command cancellation withdraws the exact pending consent or stops its owned process tree; unknown outcomes are never replayed. Source writes and commands cannot overlap. Plan/Review/inspection exclude host commands independently of callbacks. Native Bash/Agent/external MCP/configuration/hooks remain disabled. SDK file writes retain exact content/parent consent and verified outcomes, but are external policy, not OS confinement. SDK-message UUID acknowledgment proves claimed submission, not completion" }
                else { "Negotiated ACP v1; bounded project read callbacks and exactly reviewed client file writes. Implement alone exposes owned terminal create/output/wait/kill/release through the shared runner with separate exact command consent, durable exit/output and per-call cancellation. Plan/Review/inspection do not expose host execution. Pending or uncertain commands block client writes and overlapping terminal creation. Provider-owned tools remain external policy, not confined by client callbacks or /paths; no OS sandbox. Actual host effects and provider-reported results remain distinct." }.into(),
            blocker: "Requires supported runtime/auth/capability qualification after explicit consent; installation alone is not support".into(),
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
            "ANTHROPIC_PROFILE",
        ],
        2 => &[
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GOOGLE_GENERATIVE_AI_API_KEY",
        ],
        DEVIN => &["WINDSURF_API_KEY"],
        ANT => &[
            "ANTHROPIC_PROFILE",
            "ANTHROPIC_CONFIG_DIR",
            "ANTHROPIC_API_KEY",
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
        ANT => {
            // Profile configuration metadata only; credentials/ is never read or stamped.
            if let (Some(base), Some(name)) = (
                crate::providers::claude::console_config_dir(),
                crate::providers::claude::console_profile_name(),
            ) {
                paths.push((base.join("configs").join(format!("{name}.json")), false));
                paths.push((base.join("active_config"), false));
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
        DEVIN => {
            // Documented Devin config/import locations. Metadata only: JSONC and local
            // overrides can carry secrets; no credentials, instructions or hook bodies read.
            if let Some(base) = if cfg!(windows) {
                env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .map(|p| p.join("devin"))
            } else {
                home.as_ref().map(|p| p.join(".config/devin"))
            } {
                for name in ["config.json", "mcp_config.json", "AGENTS.md", "skills"] {
                    paths.push((base.join(name), false));
                }
            }
            #[cfg(target_os = "linux")]
            paths.push(("/etc/devin/system.json".into(), false));
            #[cfg(target_os = "macos")]
            paths.push((
                "/Library/Application Support/Devin/system.json".into(),
                false,
            ));
            #[cfg(windows)]
            paths.push((PathBuf::from(r"C:\ProgramData\Devin\system.json"), false));
            for name in [
                ".devin/config.json",
                ".devin/config.local.json",
                ".devin/mcp_config.json",
                ".devin/mcp_config.local.json",
                ".devin/hooks.v1.json",
                ".devin/skills",
                ".devin/agents",
                "AGENTS.md",
                "AGENTS.local.md",
                "AGENT.md",
                ".windsurfrules",
                ".cursor/rules",
                ".cursor/mcp.json",
                ".windsurf/rules",
                ".windsurf/global_rules.md",
                ".windsurf/skills",
                "CLAUDE.md",
                ".claude/skills",
                ".claude/commands",
                ".claude/settings.json",
                ".claude/settings.local.json",
                ".mcp.json",
                ".github/skills",
                "opencode.json",
                ".zed/settings.json",
            ] {
                paths.push((project.join(name), false));
            }
            // Hooks, rules, skills and repo plugin declarations can be inherited.
            // Bounded ancestor metadata is not an effective configuration loader.
            for parent in project.ancestors().skip(1).take(31) {
                for name in [
                    ".devin/config.json",
                    ".devin/config.local.json",
                    ".devin/hooks.v1.json",
                    ".devin/skills",
                    ".devin/agents",
                    ".claude/settings.json",
                    ".claude/settings.local.json",
                    "AGENTS.md",
                    "AGENTS.local.md",
                    "AGENT.md",
                ] {
                    paths.push((parent.join(name), false));
                }
            }
            if let Some(home) = home.as_ref() {
                for name in [
                    ".codeium/windsurf/skills",
                    ".codeium/windsurf/mcp_config.json",
                    ".claude/CLAUDE.md",
                    ".claude.json",
                    ".claude/settings.json",
                    ".claude/settings.local.json",
                    ".claude/mcp_servers.json",
                    ".config/opencode/opencode.json",
                    ".config/zed/settings.json",
                ] {
                    paths.push((home.join(name), false));
                }
                let copilot = env::var_os("COPILOT_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".copilot"));
                paths.push((copilot.join("skills"), false));
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
    #[allow(unused_mut)]
    let metadata = fs::metadata(&path).ok().map(|m| {
        let mut stamp = metadata_stamp(&m);
        // NTFS tunneling restores the creation time of a same-name replacement
        // within seconds; only the file ID reliably reveals the swap.
        #[cfg(windows)]
        if let Some(id) = windows_file_id(&path) {
            stamp.2.push_str(&id);
        }
        stamp
    });
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
            if crate::workspace::runner::native::is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Volume serial and file index without reading contents or following reparse points.
#[cfg(windows)]
fn windows_file_id(path: &Path) -> Option<String> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
    };
    let file = OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the handle is owned by `file` for the duration of the call.
    (unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } != 0).then(|| {
        format!(
            ":{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        )
    })
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

fn executable_identity(path: &Path) -> io::Result<crate::workspace::runner::ExecutableIdentity> {
    if path.to_str().is_none() {
        return Err(io::Error::other("Non-Unicode executable"));
    }
    let mut file = open_regular(path, EXECUTABLE_LIMIT)?;
    let before = metadata_stamp(&file.metadata()?);
    let file_id = crate::workspace::runner::native::file_identity(&file)?;
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
    Ok(crate::workspace::runner::ExecutableIdentity {
        path: path.to_path_buf(),
        sha256: format!("{:x}", digest.finalize()),
        bytes: size,
        modified_ns: before.1,
        file_id: format!("{file_id}:{}", before.2),
    })
}

fn safe_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    crate::workspace::privacy::redact(&text)
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
    if index == 1 {
        profile.blockers.push("Claude CLI version is not SDK version or permission to reuse claude.ai login. Qualify the sidecar and permitted API/cloud credentials in e.".into());
    } else {
        profile
            .version
            .push_str("; version recorded, handshake/auth/capabilities remain unqualified");
    }
}

/// Fail closed on extra output, escapes, arbitrary identifiers and credential-like payloads.
fn parse_version(index: usize, bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    let version = match index {
        0 => text.strip_prefix("codex-cli ")?,
        1 => text.strip_suffix(" (Claude Code)")?,
        2 | 3 => text,
        DEVIN => {
            let (version, build) = text.strip_prefix("devin ")?.split_once(" (")?;
            let build = build.strip_suffix(')')?;
            if build.len() != 12 || !build.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }
            version
        }
        ANT => text.strip_prefix("ant version ")?,
        _ => return None,
    };
    if version.len() > 96 || crate::workspace::privacy::redact(version) != version {
        return None;
    }
    let core_end = version.find(['-', '+']).unwrap_or(version.len());
    if core_end < version.len() {
        let suffix = &version[core_end + 1..];
        if suffix.is_empty()
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
        {
            return None;
        }
    }
    let mut parts = version[..core_end].split('.');
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
    let prepared = crate::workspace::runner::Prepared {
        approval_arrival: crate::providers::approval_arrival(),
        request_id: "agent-version-probe".into(),
        run_id: "agent-version-probe".into(),
        task_id: 0,
        spec: crate::workspace::runner::CommandSpec::Direct {
            program: executable
                .path
                .to_str()
                .ok_or("Non-Unicode executable")?
                .into(),
            args: vec![version_argument(index).into()],
        },
        cwd_identity: crate::workspace::runner::native::directory_identity(&cwd)
            .map_err(|_| "Probe working directory identity unavailable")?,
        executable,
        cwd,
        prepared_at_ms: 0,
        expires_at_ms: 0,
        before: crate::workspace::runner::InputSnapshot::default(),
        partner: None,
        rerun_of: None,
    };
    let mut process = crate::workspace::runner::native::Process::spawn(&prepared).map_err(|_| "Could not start the trusted executable; repair executable/interpreter installation (details withheld)".to_owned())?;
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
        if start.elapsed() >= probe_timeout(index) {
            break Err(format!(
                "Version probe timed out after {} seconds; no version accepted",
                probe_timeout(index).as_secs()
            ));
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

/// Retains only existing nonsecret discovery identities; never auth store contents.
pub(crate) struct PlanningTrust {
    snapshot: Snapshot,
    observed_version: std::sync::OnceLock<String>,
    project_identity: String,
    pub lines: Vec<String>,
}

impl PlanningTrust {
    pub(crate) fn comparison_scope(&self) -> Option<super::ComparisonScope> {
        super::ComparisonScope::from_identity(&(&self.snapshot.stamp, &self.project_identity))
    }

    pub(crate) fn requires_connection_review(&self) -> bool {
        let Some(project) = self.snapshot.stamp.project.as_deref() else {
            return true;
        };
        self.snapshot
            .stamp
            .selected
            .as_ref()
            .is_some_and(|path| path.starts_with(project))
            || self
                .snapshot
                .profile
                .executable
                .as_ref()
                .is_some_and(|path| path.starts_with(project))
    }

    pub(crate) fn observed_version(&self) -> Option<&str> {
        self.observed_version.get().map(String::as_str)
    }

    pub(crate) fn catalogue_source(&self) -> String {
        format!(
            "CLI {} at {}",
            self.observed_version().unwrap_or("unobserved"),
            self.snapshot
                .profile
                .executable
                .as_deref()
                .map(safe_path)
                .unwrap_or_else(|| "unresolved".into())
        )
    }
}

pub(crate) fn planning_trust(project: &Path) -> Result<PlanningTrust, String> {
    let snapshot = discover(0, project);
    if !snapshot.profile.probe_allowed {
        return Err("Codex executable or project identity unavailable; inspect agent preflight before planning".into());
    }
    let project_identity = crate::workspace::runner::native::directory_identity(project)
        .map_err(|_| "Planning project identity unavailable")?;
    let executable = snapshot
        .identity
        .as_ref()
        .ok_or("Codex executable identity unavailable")?;
    let selected = snapshot
        .stamp
        .selected
        .as_deref()
        .ok_or("Selected Codex path unavailable")?;
    let cwd = snapshot
        .cwd
        .as_deref()
        .ok_or("Codex process cwd unavailable")?;
    for path in [executable.path.as_path(), selected, cwd, project] {
        let text = path
            .to_str()
            .ok_or("Non-Unicode trust path cannot be reviewed exactly")?;
        if text.len() > 32 * 1024 || text.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
            || crate::workspace::privacy::redact(text) != text
        {
            return Err("Trust path is oversized or contains private/display-control text; cannot review exact launch identity".into());
        }
    }
    let mut lines = vec![
        format!("Trust executes {} twice: literal argv [--version], then {}. Record recognizable version evidence; live protocol and policy acknowledgment determine compatibility. Stay launches nothing.", executable.path.display(), serde_json::to_string(&planning_argv()).unwrap_or_default()),
        format!("Selected PATH candidate: {}. Process cwd: {} (outside the project). Thread/turn cwd: {}.", selected.display(), cwd.display(), project.display()),
        "The executable inherits provider authentication and the current environment. Wrappers, interpreters, libraries, configuration, MCP servers and hooks may execute or persist side effects. Eevee does not sandbox this trusted provider process or pin transitive dependencies.".into(),
        snapshot.profile.configuration.clone(),
        snapshot.profile.authentication.clone(),
        snapshot.profile.billing.clone(),
        "After trust: initialize experimental app-server API, account/read without refresh, config/read (retain only model/provider/effort), model/list. Preserve exact configured/custom selection even when absent from stale metadata; the provider decides availability and cannot silently substitute. No login, credential-store read, credential/config mutation or startup model request is performed by this CLI adapter.".into(),
        "Only Run sends the displayed immutable payload. Codex may additionally load project instructions, skills, provider configuration, MCP context and prior provider history. This is NOT a preview of the full effective model prompt. Provider account/plan owns billing and eligibility; a present account does not guarantee quota.".into(),
        "Provider persists thread history and thread IDs; Eevee keeps IDs/outputs only for this session. Resume is explicit and restricted to a previously started same-task thread; no automatic replay/restart. New thread does not mean provider forgets prior data.".into(),
        "Thread/turn request read-only, restricted network, approvalsReviewer=user and on-request approval policy; disable multi-agent delegation, web search and login shells by session config overrides (not global writes). These are AGENT-POLICY-CONTROLLED boundaries, not Eevee OS isolation. Provider config, MCP startup/tool actions and hook coverage is unknown. Process groups/Jobs own cancellation only. Commands the provider deems safe can run without callbacks.".into(),
        "Command callbacks offer only user deny or supported allow-once, never persistent/session approval. File edits are denied; unsupported permission/tool/auth/elicitation requests are explicitly refused. Plan-mode human questions require explicit answers. No private reasoning is displayed.".into(),
        "Executable bytes, known config source metadata/nonsecret declarations, task revision and exact selected-source payload are checked again before turn dispatch. Concurrent malicious replacement and undiscovered transitive config are not atomically isolated. Source exclusions constrain Eevee's payload only, not independent provider reads.".into(),
        "No retries or automatic sends. Provider-internal retries are outside Eevee control; an observed retry fails closed. Interruption waits for authoritative turn completion or bounded owned teardown; absent completion means outcome unknown, not successful cancellation.".into(),
        "Transport bounds: 2 MiB JSONL frames, 1 MiB outgoing payload, 64 MiB captured protocol/diagnostics, 128 displayed items at 32 KiB each / 512 KiB total. Overflow stops explicitly. Stdin is nonblocking with a 5-second send deadline; interrupt waits up to 3 seconds, then owned teardown is bounded. Raw stderr and private reasoning are never retained.".into(),
    ];
    if executable.path.starts_with(project)
        || snapshot
            .stamp
            .selected
            .as_ref()
            .is_some_and(|path| path.starts_with(project))
    {
        lines.insert(
            0,
            "REPOSITORY-SUPPLIED executable/wrapper: explicit trust required.".into(),
        );
    }
    Ok(PlanningTrust {
        snapshot,
        observed_version: std::sync::OnceLock::new(),
        project_identity,
        lines,
    })
}

fn planning_argv() -> Vec<String> {
    [
        "app-server",
        "--listen",
        "stdio://",
        "-c",
        "sandbox_mode=\"read-only\"",
        "-c",
        "approval_policy=\"on-request\"",
        "-c",
        "allow_login_shell=false",
        "-c",
        "features.multi_agent=false",
        "-c",
        "web_search=\"disabled\"",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub(crate) fn revalidate_planning_trust(
    project: &Path,
    trust: &PlanningTrust,
) -> Result<(), String> {
    let current = discover(0, project);
    if !current.profile.probe_allowed
        || current.stamp != trust.snapshot.stamp
        || crate::workspace::runner::native::directory_identity(project)
            .ok()
            .as_ref()
            != Some(&trust.project_identity)
    {
        return Err(
            "Executable/configuration/project identity changed; review a fresh trust preview"
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn qualify_planning(
    project: &Path,
    trust: &PlanningTrust,
    stopped: &AtomicBool,
) -> Result<crate::workspace::runner::Prepared, String> {
    revalidate_planning_trust(project, trust)?;
    let version = probe(0, &trust.snapshot, stopped)?;
    bind_version(&trust.observed_version, version)?;
    revalidate_planning_trust(project, trust)?;
    let executable = trust
        .snapshot
        .identity
        .clone()
        .ok_or("Codex executable missing")?;
    let cwd = trust
        .snapshot
        .cwd
        .clone()
        .ok_or("Codex process cwd missing")?;
    Ok(crate::workspace::runner::Prepared {
        approval_arrival: crate::providers::approval_arrival(),
        request_id: "codex-app-server".into(),
        run_id: "codex-app-server".into(),
        task_id: 0,
        spec: crate::workspace::runner::CommandSpec::Direct {
            program: executable
                .path
                .to_str()
                .ok_or("Non-Unicode Codex path")?
                .into(),
            args: planning_argv(),
        },
        cwd_identity: crate::workspace::runner::native::directory_identity(&cwd)
            .map_err(|_| "Codex process cwd identity unavailable")?,
        executable,
        cwd,
        prepared_at_ms: 0,
        expires_at_ms: 0,
        before: crate::workspace::runner::InputSnapshot::default(),
        partner: None,
        rerun_of: None,
    })
}

/// Passive identity/configuration snapshot for a reviewed ACP launch.
pub(crate) struct ExternalTrust {
    snapshot: Snapshot,
    project_identity: String,
    observed_version: std::sync::OnceLock<String>,
    index: usize,
    pub lines: Vec<String>,
    pub prepared: crate::workspace::runner::Prepared,
}

impl ExternalTrust {
    pub(crate) fn comparison_scope(
        &self,
        credential: Option<&str>,
    ) -> Option<super::ComparisonScope> {
        super::ComparisonScope::from_identity(&(
            &self.snapshot.stamp,
            &self.project_identity,
            &self.prepared.spec,
            credential,
        ))
    }

    pub(crate) fn observed_version(&self) -> Option<&str> {
        self.observed_version.get().map(String::as_str)
    }

    pub(crate) fn authentication_configuration(&self) -> &str {
        &self.snapshot.profile.configuration
    }
}

pub(crate) fn external_trust(
    index: usize,
    project: &Path,
    launch: Vec<String>,
) -> Result<ExternalTrust, String> {
    if index >= NAMES.len()
        || launch.len() > 64
        || launch.iter().any(|arg| {
            arg.len() > 32 * 1024
                || arg.chars().any(char::is_control)
                || crate::workspace::privacy::redact(arg) != arg.as_str()
        })
    {
        return Err("Provider launch arguments cannot be reviewed safely".into());
    }
    let snapshot = discover(index, project);
    if !snapshot.profile.probe_allowed {
        return Err(format!(
            "{} executable/config identity unavailable; inspect o preflight",
            NAMES[index]
        ));
    }
    let executable = snapshot
        .identity
        .clone()
        .ok_or("Executable identity unavailable")?;
    let cwd = snapshot
        .cwd
        .clone()
        .ok_or("Outside-project temporary directory unavailable")?;
    for path in [&executable.path, &cwd, project] {
        let value = path.to_str().ok_or("Non-Unicode provider path")?;
        if value.len() > 32 * 1024 || value.chars().any(|ch| ch.is_control()
            || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
            || crate::workspace::privacy::redact(value) != value
        {
            return Err("Provider path contains private or directional display text".into());
        }
    }
    let project_identity = crate::workspace::runner::native::directory_identity(project)
        .map_err(|_| "Provider project identity unavailable")?;
    let lines = vec![
        format!("{}: explicit trust runs {} with {}, then literal argv {}.",
            NAMES[index], executable.path.display(), version_argument(index), serde_json::to_string(&launch).map_err(|error| error.to_string())?),
        format!("Owned process cwd {} outside project; requested session cwd {}.", cwd.display(), project.display()),
        snapshot.profile.configuration.clone(),
        snapshot.profile.authentication.clone(),
        snapshot.profile.billing.clone(),
        "Executable, known configuration identities and project identity are revalidated. Inherited environment, interpreters, dependencies, provider instructions, hooks, plugins, MCP and provider-owned history may add context or execute effects. This is not a sandbox or atomic transitive-dependency pin.".into(),
        "Consent authorizes this provider process and one reviewed request. Provider authentication/billing remain provider-owned; no login/config mutation or credential-store read by Eevee. Permission callbacks are not universal interception of provider-owned tools. Grants never cross provider switches.".into(),
        "Process-tree ownership provides bounded cancellation, not filesystem/network isolation or rollback. Unsupported callback/capability is refused; missing authoritative completion remains outcome unknown. No automatic resend.".into(),
    ];
    let prepared = crate::workspace::runner::Prepared {
        approval_arrival: crate::providers::approval_arrival(),
        request_id: "provider-session".into(),
        run_id: "provider-session".into(),
        task_id: 0,
        spec: crate::workspace::runner::CommandSpec::Direct {
            program: executable
                .path
                .to_str()
                .ok_or("Non-Unicode executable")?
                .into(),
            args: launch,
        },
        cwd_identity: crate::workspace::runner::native::directory_identity(&cwd)
            .map_err(|_| "Provider process cwd identity unavailable")?,
        executable,
        cwd,
        prepared_at_ms: 0,
        expires_at_ms: 0,
        before: crate::workspace::runner::InputSnapshot::default(),
        partner: None,
        rerun_of: None,
    };
    Ok(ExternalTrust {
        snapshot,
        project_identity,
        observed_version: std::sync::OnceLock::new(),
        index,
        lines,
        prepared,
    })
}

pub(crate) fn revalidate_external_trust(
    index: usize,
    project: &Path,
    trust: &ExternalTrust,
) -> Result<(), String> {
    if index != trust.index {
        return Err("Provider identity differs from the reviewed executable".into());
    }
    let current = discover(index, project);
    if !current.profile.probe_allowed
        || current.stamp != trust.snapshot.stamp
        || crate::workspace::runner::native::directory_identity(project)
            .ok()
            .as_ref()
            != Some(&trust.project_identity)
    {
        return Err("Provider executable/configuration/project changed; preview again".into());
    }
    Ok(())
}

pub(crate) fn qualify_external(
    index: usize,
    project: &Path,
    trust: &ExternalTrust,
    stopped: &AtomicBool,
) -> Result<crate::workspace::runner::Prepared, String> {
    revalidate_external_trust(index, project, trust)?;
    let version = probe(index, &trust.snapshot, stopped)?;
    bind_version(&trust.observed_version, version)?;
    revalidate_external_trust(index, project, trust)?;
    Ok(trust.prepared.clone())
}

/// Bind the first consented observation for this immutable reviewed launch.
/// A package update requires a fresh preview, even if both releases are compatible.
fn bind_version(observed: &std::sync::OnceLock<String>, version: String) -> Result<(), String> {
    if let Err(version) = observed.set(version)
        && observed.get() != Some(&version)
    {
        return Err("Observed provider version changed; review a fresh trust preview".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
