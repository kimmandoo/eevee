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
pub(crate) const CODEX_RELEASE: &str = "0.152.1";
pub(crate) const OPENCODE_RELEASE: &str = "1.18.0";
pub(crate) const GEMINI_RELEASE: &str = "0.28.0";
pub(crate) const DEVIN_RELEASE: &str = "3000.11.3";
pub(crate) const DEVIN: usize = 4;
/// Official Anthropic CLI used only for Claude's Console OAuth login/status handoff.
pub(crate) const ANT_RELEASE: &str = "1.40.0";
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
        method: crate::auth::Method,
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
        result: Result<Box<crate::auth::Plan>, String>,
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
                        Event::Discovered(snapshots.iter().map(|s| s.profile.clone()).collect())
                    }
                    Request::PrepareAuth { profile, generation, method } => {
                        Event::AuthPrepared { profile, generation, result: crate::auth::Plan::prepare(profile, method, &project).map(Box::new) }
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
            "Project SDK manifest present; sidecar installation and pinned SDK/native runtime still require explicit qualification"
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
            && crate::claude::console_credentials().is_none()
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
        trust_preview.push(format!("Codex {CODEX_RELEASE} is the exact planning adapter release. This version probe does not start a planning session or establish authentication. Workspace-write startup can persist project trust; Eevee planning requests read-only instead."));
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
        version: if index == 0 {
            format!("Unknown (not probed); required planning release {CODEX_RELEASE}")
        } else {
            "Unknown (not probed); protocol/SDK version not observed".into()
        },
        authentication,
        configuration,
        os: env::consts::OS.into(),
        architecture: env::consts::ARCH.into(),
        adapter: match index {
            0 => format!("Codex app-server {CODEX_RELEASE}; Plan and POSIX controlled-file implementation"),
            1 => "Optional pinned Claude Agent SDK sidecar; e performs separate runtime/auth qualification".into(),
            2 => format!("OpenCode {OPENCODE_RELEASE} / negotiated ACP v1; provider-owned tools"),
            DEVIN => format!("Devin {DEVIN_RELEASE} / negotiated ACP v1; provider-owned tools and native account login"),
            ANT => format!("Anthropic CLI {ANT_RELEASE}; Console OAuth login/status handoff for Claude only"),
            _ => format!("Gemini {GEMINI_RELEASE} / negotiated ACP v1; absent credentials remain unavailable"),
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
                "Exact 0.152.1 JSONL adapter; immutable full outgoing payload review and dispatch revalidation",
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
                "Plan: supported command deny/allow-once; Implement: selected-file Eevee dynamic tool with exact patch approval, native edits/escalation refused",
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
                evidence: "These pinned ACP peers do not negotiate an implemented structured question/elicitation extension".into(),
                blocker: "Unknown question requests are refused. Explicit reviewed follow-up turns remain separate; no missing method is invoked".into(),
            });
            continue;
        }
        if index >= 2 && action == "Usage and billing" {
            result.push(Capability {
                action: action.into(), boundary: "Provider-owned / unavailable".into(),
                evidence: "No standardized usage accounting is negotiated by these pinned ACP adapters".into(),
                blocker: "Usage/cost remain unknown; no token total or cross-provider cost sum is inferred".into(),
            });
            continue;
        }
        result.push(Capability {
            action: action.into(),
            boundary: if action == "Filesystem/network restrictions" { "Observed/external — no Eevee confinement" } else { "Eevee-mediated protocol callbacks / provider-owned tools" }.into(),
            evidence: if index == 1 { "Pinned optional official SDK sidecar; tool/config policy separate from callbacks" }
                else { "Negotiated ACP v1; exact once-only permission IDs; fs/terminal callbacks disabled" }.into(),
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
                crate::claude::console_config_dir(),
                crate::claude::console_profile_name(),
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
                .push_str("; matches exact planning adapter release; session/authentication not yet qualified");
        } else {
            let blocker = format!(
                "Version mismatch: observed {version}, required Codex release {CODEX_RELEASE}; install the reviewed release before planning"
            );
            profile
                .version
                .push_str("; DOES NOT MATCH qualified planning release");
            profile.blockers.push(blocker);
        }
    } else if index == 1 {
        profile.blockers.push("Claude CLI version is not SDK version or permission to reuse claude.ai login. Qualify pinned sidecar and permitted API/cloud credentials in e.".into());
    } else {
        let expected = match index {
            2 => OPENCODE_RELEASE,
            DEVIN => DEVIN_RELEASE,
            ANT => ANT_RELEASE,
            _ => GEMINI_RELEASE,
        };
        if version != expected {
            profile.blockers.push(format!("Version mismatch: observed {version}, required ACP peer release {expected}; review/install compatible peer before a request"));
        } else {
            profile.version.push_str("; release matches pinned ACP profile, handshake/auth/capabilities remain unqualified");
        }
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
            args: vec![version_argument(index).into()],
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
    project_identity: String,
    pub lines: Vec<String>,
}

pub(crate) fn planning_trust(project: &Path) -> Result<PlanningTrust, String> {
    let snapshot = discover(0, project);
    if !snapshot.profile.probe_allowed {
        return Err("Codex executable or project identity unavailable; inspect agent preflight before planning".into());
    }
    let project_identity = crate::runner::native::directory_identity(project)
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
            || crate::privacy::redact(text) != text
        {
            return Err("Trust path is oversized or contains private/display-control text; cannot review exact launch identity".into());
        }
    }
    let mut lines = vec![
        format!("Trust executes {} twice: literal argv [--version], then {}. Required release {CODEX_RELEASE}. Stay launches nothing.", executable.path.display(), serde_json::to_string(&planning_argv()).unwrap_or_default()),
        format!("Selected PATH candidate: {}. Process cwd: {} (outside the project). Thread/turn cwd: {}.", selected.display(), cwd.display(), project.display()),
        "The executable inherits provider authentication and the current environment. Wrappers, interpreters, libraries, configuration, MCP servers and hooks may execute or persist side effects. Eevee does not sandbox this trusted provider process or pin transitive dependencies.".into(),
        snapshot.profile.configuration.clone(),
        snapshot.profile.authentication.clone(),
        snapshot.profile.billing.clone(),
        "After trust: initialize experimental app-server API, account/read without refresh, model/list; use the available default text model and its default effort. No login, credential-store read, credential/config mutation or startup model request is performed by Eevee.".into(),
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
        || crate::runner::native::directory_identity(project)
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
) -> Result<crate::runner::Prepared, String> {
    revalidate_planning_trust(project, trust)?;
    let version = probe(0, &trust.snapshot, stopped)?;
    if version != CODEX_RELEASE {
        return Err(format!(
            "Protocol qualification refused: installed {version}, required {CODEX_RELEASE}"
        ));
    }
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
    Ok(crate::runner::Prepared {
        request_id: "codex-app-server".into(),
        run_id: "codex-app-server".into(),
        task_id: 0,
        spec: crate::runner::CommandSpec::Direct {
            program: executable
                .path
                .to_str()
                .ok_or("Non-Unicode Codex path")?
                .into(),
            args: planning_argv(),
        },
        cwd_identity: crate::runner::native::directory_identity(&cwd)
            .map_err(|_| "Codex process cwd identity unavailable")?,
        executable,
        cwd,
        prepared_at_ms: 0,
        expires_at_ms: 0,
        before: crate::runner::InputSnapshot::default(),
    })
}

/// Passive identity/configuration snapshot for a reviewed ACP launch.
pub(crate) struct ExternalTrust {
    snapshot: Snapshot,
    project_identity: String,
    index: usize,
    pub lines: Vec<String>,
    pub prepared: crate::runner::Prepared,
}

impl ExternalTrust {
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
                || crate::privacy::redact(arg) != arg.as_str()
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
            || crate::privacy::redact(value) != value
        {
            return Err("Provider path contains private or directional display text".into());
        }
    }
    let project_identity = crate::runner::native::directory_identity(project)
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
    let prepared = crate::runner::Prepared {
        request_id: "provider-session".into(),
        run_id: "provider-session".into(),
        task_id: 0,
        spec: crate::runner::CommandSpec::Direct {
            program: executable
                .path
                .to_str()
                .ok_or("Non-Unicode executable")?
                .into(),
            args: launch,
        },
        cwd_identity: crate::runner::native::directory_identity(&cwd)
            .map_err(|_| "Provider process cwd identity unavailable")?,
        executable,
        cwd,
        prepared_at_ms: 0,
        expires_at_ms: 0,
        before: crate::runner::InputSnapshot::default(),
    };
    Ok(ExternalTrust {
        snapshot,
        project_identity,
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
        || crate::runner::native::directory_identity(project)
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
    expected: &str,
    stopped: &AtomicBool,
) -> Result<crate::runner::Prepared, String> {
    revalidate_external_trust(index, project, trust)?;
    let version = probe(index, &trust.snapshot, stopped)?;
    if version != expected {
        return Err(format!(
            "Provider version mismatch: observed {version}, required {expected}"
        ));
    }
    revalidate_external_trust(index, project, trust)?;
    Ok(trust.prepared.clone())
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
        assert_eq!(
            parse_version(ANT, b"ant version 1.40.0\n"),
            Some("1.40.0".into())
        );
        assert_eq!(parse_version(ANT, b"1.40.0\n"), None);
        assert_eq!(
            parse_version(DEVIN, b"devin 3000.11.3 (9c803229faa4)\n"),
            Some(DEVIN_RELEASE.into())
        );
        for output in [
            b"3000.11.3".as_slice(),
            b"devin 3000.11.3 (secret)",
            b"devin 3000.11.3 (9c803229faa4)\nWINDSURF_API_KEY=secret",
            b"devin \x1b[31m3000.11.3 (9c803229faa4)",
        ] {
            assert_eq!(parse_version(DEVIN, output), None);
        }
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
    fn devin_config_metadata_tracks_replacement_without_reading_secrets_or_credentials() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join(".devin")).unwrap();
        let path = fixture.0.join(".devin/config.local.json");
        let absent = configuration_sources(DEVIN, &fixture.0);
        fs::write(
            &path,
            b"{\"token\":\"private-canary\",\"hooks\":{\"command\":\"hook-canary\"}}",
        )
        .unwrap();
        let initial = configuration_sources(DEVIN, &fixture.0);
        assert!(initial != absent);
        let config = initial.iter().find(|entry| entry.path == path).unwrap();
        assert_eq!(
            config.declaration,
            "present; metadata only (no loader or plugin executed)"
        );
        assert!(
            initial
                .iter()
                .all(|entry| !entry.declaration.contains("canary"))
        );
        assert!(
            initial
                .iter()
                .all(|entry| !entry.path.ends_with("credentials.toml"))
        );
        fs::rename(&path, fixture.0.join("old-config")).unwrap();
        fs::write(
            &path,
            b"{\"token\":\"private-canary\",\"hooks\":{\"command\":\"hook-canary\"}}",
        )
        .unwrap();
        assert!(initial != configuration_sources(DEVIN, &fixture.0));
    }

    #[cfg(unix)]
    #[test]
    fn devin_version_probe_rejects_extra_output_without_exposing_it() {
        let fixture = Fixture::new();
        let stop = AtomicBool::new(false);
        let snapshot =
            fixture.executable("printf 'devin 3000.11.3 (9c803229faa4)\\nprivate-canary\\n'");
        let error = probe(DEVIN, &snapshot, &stop).unwrap_err();
        assert!(!error.contains("canary"));
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
