//! Explicit provider authentication. No task prompt or credential is journaled here.
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::platform::native_process::{INTERRUPTED, NativeChild, NativeSignals};
use crate::{providers::agent, workspace::runner};

mod callback;
mod devin;
mod openai;
pub use devin::DevinCredential;
pub(crate) use devin::api_key as devin_api_key;
#[cfg(test)]
pub(crate) use devin::test_credential as test_devin_credential;
#[cfg(test)]
pub(crate) use openai::test_credential as test_openai_credential;
pub use openai::{OpenAiCredential, OpenAiFailure};
const INPUT_LIMIT: usize = 16 * 1024;

const LIFETIME: Duration = Duration::from_secs(300);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const STATUS_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    OpenAiBrowser,
    OpenAiDevice,
    CodexBrowser,
    CodexDevice,
    OpenCode,
    GeminiNative,
    ClaudeConsole,
    ClaudeConsoleCode,
    DevinWeb,
    CodexStatus,
    OpenCodeStatus,
    ClaudeConsoleStatus,
}
impl Method {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAiBrowser => "OpenAI account / browser sign-in",
            Self::OpenAiDevice => "OpenAI account / device authorization",
            Self::CodexBrowser => "ChatGPT / browser OAuth",
            Self::CodexDevice => "ChatGPT / device authorization",
            Self::OpenCode => "Official provider and auth-method chooser",
            Self::GeminiNative => "Official Gemini CLI / Google OAuth, API key or Vertex",
            Self::ClaudeConsole => "Anthropic Console OAuth / browser (official ant CLI)",
            Self::ClaudeConsoleCode => "Anthropic Console OAuth / paste code (no browser launch)",
            Self::DevinWeb => "Devin / browser OAuth or pasted callback",
            Self::ClaudeConsoleStatus => {
                "Official ant auth status (profile presence, not eligibility)"
            }
            Self::CodexStatus => "Official Codex login status",
            Self::OpenCodeStatus => {
                "Official OpenCode credential inventory (not eligibility proof)"
            }
        }
    }
    pub fn is_status(self) -> bool {
        matches!(
            self,
            Self::CodexStatus | Self::OpenCodeStatus | Self::ClaudeConsoleStatus
        )
    }
    fn is_claude(self) -> bool {
        matches!(
            self,
            Self::ClaudeConsole | Self::ClaudeConsoleCode | Self::ClaudeConsoleStatus
        )
    }
    /// Eevee-owned website flow: no provider executable or credential store.
    pub fn is_web(self) -> bool {
        matches!(
            self,
            Self::DevinWeb | Self::OpenAiBrowser | Self::OpenAiDevice
        )
    }
    pub fn guidance(self) -> Option<&'static str> {
        match self {
            Self::OpenAiBrowser => Some(
                "Signs in directly with OpenAI, without an installed CLI. The browser returns to http://localhost:1455/auth/callback (1457 if the first port is busy); paste the complete redirect URL into the hidden terminal input if the callback cannot reach this machine. PKCE and state bind this attempt. Esc/Ctrl+C cancels; old redirects cannot be reused. For WSL/SSH, device authorization needs no callback.",
            ),
            Self::OpenAiDevice => Some(
                "Signs in directly with OpenAI without a CLI or localhost callback. Enable device-code authorization in ChatGPT Settings > Security (workspace policy may require an administrator). Open https://auth.openai.com/codex/device and enter only the one-time code shown in this terminal for the login you initiated. Keep this terminal open; Esc/Ctrl+C cancels. No automatic browser fallback.",
            ),
            Self::CodexBrowser => Some(
                "Browser login requires the browser to reach the CLI's localhost listener (Codex uses port 1455, or 1457). On WSL, Windows reserved TCP ranges can block both ports even when the Linux listener is healthy. Use Device authorization to avoid host forwarding and administrator/network changes. Keep browser login running until its callback (10 min limit); expired/cancelled callback URLs cannot be reused. Never share callback URLs.",
            ),
            Self::CodexDevice => Some(
                "WSL/SSH: Device authorization works without a localhost listener, port forwarding, administrator privileges, or Windows network changes. Before continuing, enable device-code authorization in ChatGPT Settings > Security (workspace accounts may require an admin). Open the official link and enter the one-time code shown by Codex; keep this terminal open until it completes. Eevee never collects the code and never silently falls back to browser login.",
            ),
            Self::ClaudeConsole | Self::ClaudeConsoleCode => Some(
                "Signs in to the Claude Console (platform.claude.com) and bills Claude API usage to the chosen organization/workspace. This is NOT claude.ai Pro/Max subscription login: Anthropic does not permit third-party Agent SDK apps to use subscription OAuth. Paste-code login prints the official authorize URL and reads the returned code inside ant, without a localhost callback or browser launcher (WSL/SSH). Never share codes or URLs.",
            ),
            Self::DevinWeb => Some(
                "Signs in directly with Devin, without a CLI. PKCE and fresh state bind the localhost callback at 127.0.0.1:59653/callback; paste the complete redirect URL or code#state pair if the browser cannot reach this machine. Raw API keys belong in explicit API-key setup, not this OAuth prompt. Credentials use the selected private-save or session-only choice; expired grants require explicit reconnect. Esc/Ctrl+C cancels.",
            ),
            _ => None,
        }
    }
    fn args(self) -> Vec<String> {
        if self.is_claude() {
            let profile = crate::providers::claude::console_profile_name()
                .unwrap_or_else(|| crate::providers::claude::CONSOLE_PROFILE.into());
            let mut args: Vec<String> = match self {
                Self::ClaudeConsoleStatus => vec!["auth".into(), "status".into()],
                _ => vec!["auth".into(), "login".into()],
            };
            if self == Self::ClaudeConsoleCode {
                args.push("--no-browser".into());
            }
            args.extend(["--profile".into(), profile]);
            return args;
        }
        let args: &[&str] = match self {
            Self::CodexBrowser => &["login"],
            Self::CodexDevice => &["login", "--device-auth"],
            Self::OpenCode => &["auth", "login", "--pure"],
            Self::CodexStatus => &["login", "status"],
            Self::OpenCodeStatus => &["auth", "list"],
            Self::DevinWeb | Self::OpenAiBrowser | Self::OpenAiDevice => &[],
            Self::GeminiNative => &[],
            Self::ClaudeConsole | Self::ClaudeConsoleCode | Self::ClaudeConsoleStatus => {
                unreachable!("built above")
            }
        };
        args.iter().map(|arg| (*arg).into()).collect()
    }
}
pub fn methods(profile: usize) -> &'static [Method] {
    match profile {
        0 => &[Method::CodexBrowser, Method::CodexDevice],
        1 => &[Method::ClaudeConsole, Method::ClaudeConsoleCode],
        2 => &[Method::OpenCode],
        3 => &[Method::GeminiNative],
        profile if profile == crate::providers::Agent::Devin.index() => &[Method::DevinWeb],
        profile if profile == crate::providers::Agent::ChatGPT.index() => {
            &[Method::OpenAiBrowser, Method::OpenAiDevice]
        }
        _ => &[],
    }
}

/// A suggestion at selection time only; an approved Plan keeps its exact method.
pub fn default_method(profile: usize) -> usize {
    static CALLBACK_FREE: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        if callback_free_environment(
            |key| std::env::var_os(key).is_some_and(|value| !value.is_empty()),
            "",
        ) {
            return true;
        }
        #[cfg(target_os = "linux")]
        let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
        #[cfg(not(target_os = "linux"))]
        let kernel = String::new();
        callback_free_environment(|_| false, &kernel)
    });
    // Codex device auth and Claude paste-code sit at index 1. Devin supports
    // a pasted callback within its single browser flow.
    if *CALLBACK_FREE
        && (matches!(profile, 0 | 1) || profile == crate::providers::Agent::ChatGPT.index())
    {
        1
    } else {
        0
    }
}
fn callback_free_environment(present: impl Fn(&str) -> bool, kernel: &str) -> bool {
    [
        "WSL_DISTRO_NAME",
        "WSL_INTEROP",
        "SSH_CONNECTION",
        "SSH_CLIENT",
        "SSH_TTY",
    ]
    .into_iter()
    .any(present)
        || kernel
            .as_bytes()
            .windows(b"microsoft".len())
            .any(|part| part.eq_ignore_ascii_case(b"microsoft"))
}
pub const CLAUDE_POLICY: &str = "Claude Agent SDK uses Anthropic API credentials: ANTHROPIC_API_KEY, an Anthropic Console OAuth profile (Connect runs the official ant CLI 1.40.0 from PATH), or configured Bedrock/Vertex cloud credentials. Install the pinned SDK sidecar described in Setup (README). Claude CLI login is not SDK setup: claude.ai subscription OAuth tokens cannot be reused here. Eevee never collects tokens. A model request still needs separate Run approval.";
pub fn status_method(profile: usize) -> Option<Method> {
    match profile {
        0 => Some(Method::CodexStatus),
        1 => Some(Method::ClaudeConsoleStatus),
        2 => Some(Method::OpenCodeStatus),
        _ => None,
    }
}

struct AuthDirectory {
    path: PathBuf,
    identity: String,
}

impl AuthDirectory {
    fn create(parent: &Path) -> Result<Self, String> {
        use std::sync::atomic::AtomicU64;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..32 {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos();
            let path = parent.join(format!(
                ".ev-auth-{}-{stamp}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            #[cfg(unix)]
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(not(unix))]
            let builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => {
                    let identity = match runner::native::directory_identity(&path) {
                        Ok(identity) => identity,
                        Err(error) => {
                            let _ = std::fs::remove_dir(&path);
                            return Err(error.to_string());
                        }
                    };
                    return Ok(Self { path, identity });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!("Cannot create private authentication cwd: {error}"));
                }
            }
        }
        Err("Cannot allocate private authentication cwd".into())
    }

    fn unchanged_empty(&self) -> bool {
        std::fs::canonicalize(&self.path).ok().as_ref() == Some(&self.path)
            && runner::native::directory_identity(&self.path).ok().as_ref() == Some(&self.identity)
            && std::fs::read_dir(&self.path).is_ok_and(|mut entries| entries.next().is_none())
    }
}

impl Drop for AuthDirectory {
    fn drop(&mut self) {
        if self.unchanged_empty() {
            // Never recursively erase files created in the external native CLI.
            let _ = std::fs::remove_dir(&self.path);
        }
    }
}

pub struct Plan {
    pub profile: usize,
    pub method: Method,
    /// Trust index of the launched executable (`ant` for Claude, else the provider).
    executable: usize,
    pub lines: Vec<String>,
    pub summary: Vec<String>,
    /// None for Eevee-owned website flows: no provider executable is launched.
    trust: Option<agent::ExternalTrust>,
    created: Instant,
    directory: Option<AuthDirectory>,
}
impl std::fmt::Debug for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthPlan")
            .field("profile", &self.profile)
            .field("method", &self.method)
            .finish_non_exhaustive()
    }
}
impl Plan {
    pub fn prepare(profile: usize, method: Method, project: &Path) -> Result<Self, String> {
        if !methods(profile).contains(&method) && status_method(profile) != Some(method) {
            return Err("Authentication method does not belong to this provider".into());
        }
        if method == Method::GeminiNative
            && ["CI", "GITHUB_ACTIONS"]
                .iter()
                .any(|key| std::env::var(key).is_ok_and(|value| value == "true"))
        {
            return Err("Gemini native login is disabled by CI/GITHUB_ACTIONS=true. Launch Eevee from an interactive environment without those headless flags; no login was started.".into());
        }
        if method.is_web() {
            return Self::web(profile, method);
        }
        let executable = if method.is_claude() {
            if crate::providers::claude::console_profile_name().is_none() {
                return Err("ANTHROPIC_PROFILE must be 1-64 letters, digits, '_' or '-'; no login was started.".into());
            }
            agent::ANT
        } else {
            profile
        };
        let mut trust = agent::external_trust(executable, project, method.args()).map_err(|error| {
            if method.is_claude() {
                format!("{error}. Install/update the official Anthropic CLI on PATH outside the repository: go install github.com/anthropics/anthropic-cli/cmd/ant@latest, then Refresh. {CLAUDE_POLICY}")
            } else {
                error
            }
        })?;
        let directory = AuthDirectory::create(&trust.prepared.cwd)?;
        trust.prepared.cwd = directory.path.clone();
        trust.prepared.cwd_identity = directory.identity.clone();
        let mut lines = vec![
            format!("Selected method: {}", method.label()),
            format!("Executable: {}", trust.prepared.executable.path.display()),
            format!("Literal argv: {:?}; first runs {} and binds the recognized stable version to this reviewed launch; no exact release pin.", method.args(), agent::version_argument(executable)),
            format!("Working directory: {} (private empty directory outside task project; removed only if still empty, never recursively).", trust.prepared.cwd.display()),
            trust.authentication_configuration().to_owned(),
            "Trust includes inherited environment, interpreter, dependencies, provider configuration/hooks/plugins and browser launcher. Executable and known configuration/project identities are checked immediately before launch. This is not sandboxing or atomic dependency pinning.".into(),
        ];
        if let Some(guidance) = method.guidance() {
            lines.push(guidance.into());
        }
        if method.is_status() {
            lines.push("This launches the official local status command; the provider may read its own credentials. Output is transient, not copied into task state. A credential inventory is not proof of account eligibility or a successful model request.".into());
        } else {
            lines.push(format!("May open your browser, contact provider identity/account services, {}and write/replace provider-owned credentials and auth-method settings. Switching Gemini auth method can clear previous cached credentials. Provider-native prompts choose accounts/providers; trust their displayed scopes before consenting. Eevee never requests or persists tokens, API keys, device codes or account identifiers.", if matches!(method, Method::CodexDevice | Method::ClaudeConsoleCode) { "" } else { "bind a local OAuth callback, " }));
            lines.push("Eevee sends no task context or model prompt and never replays the task. Any later model request needs a separate review and Run approval. Cancellation does not revoke already-issued credentials; use the provider's account controls to revoke them.".into());
            if matches!(method, Method::CodexBrowser | Method::CodexDevice) {
                lines.push("After the native UI returns, literal argv [\"login\", \"status\"] refreshes Codex status if reviewed identities still match. Only an exact recognized status is kept; all raw output is discarded.".into());
            }
            if method.is_claude() {
                lines.push(format!("ant writes configs/<profile>.json (first login only), credentials/<profile>.json and active_config under {} for the named profile and makes it ant's active profile. The Claude Agent SDK sidecar later reads only this explicit profile, bills API usage to its Console organization/workspace and may refresh its access token in place. Remove it with ant auth logout --profile <profile>.", crate::providers::claude::console_config_dir().map(|p| p.display().to_string()).unwrap_or_else(|| "the ant configuration directory".into())));
            }
            if method == Method::OpenCode {
                lines.push("OpenCode --pure uses official built-in auth routes without external auth plugins. Select an eligible supported account; Claude Pro/Max OAuth is not provided or bypassed.".into());
            }
        }
        if method == Method::GeminiNative {
            lines.push("EXTERNAL NATIVE LOGIN / NOT AN AUTH-ONLY SANDBOX. Eevee opens the official Gemini CLI outside your project, not its ACP authentication channel. Choose its supported Google OAuth (browser or manual code), API-key or Vertex route there; use /auth if already connected. The CLI may continue into chat and initialize provider services/configuration; anything you type there is handled by Gemini and can start model work. Exit/Ctrl+C to return. Eevee does not collect or forward authorization codes, select a method by injected keystrokes, or infer authentication from exit.".into());
        } else {
            lines.push(
                "Terminal is handed to the official dedicated auth UI, not a chat session.".into(),
            );
        }
        lines.push("Use the native UI's exit/cancel action or Ctrl+C. Native output can contain secrets and remains in your terminal scrollback; Eevee does not capture it. A successful process exit alone is NOT proof of authentication.".into());
        lines.push("Approval expires in five minutes; login expires after ten minutes (status after fifteen seconds). Esc cancels without launching. Back is selected by default.".into());
        let mut summary = vec![
            format!("Method: {}", method.label()),
            format!(
                "Run: {} {:?}",
                trust.prepared.executable.path.display(),
                method.args()
            ),
            "Private empty cwd; version checked before auth.".into(),
            "Inherits environment/config/hooks; NOT sandboxed.".into(),
        ];
        if method.is_status() {
            summary.push("Reads credentials; status is not eligibility proof.".into());
        } else {
            summary.push("Contacts account services; may replace credentials.".into());
            summary.push(
                if matches!(method, Method::CodexDevice | Method::ClaudeConsoleCode) {
                    "Native code/token UI; no local OAuth callback.".into()
                } else {
                    "May open browser and bind a local OAuth callback.".into()
                },
            );
            if matches!(method, Method::CodexBrowser | Method::CodexDevice) {
                summary.push("Then checks login status; no model turn.".into());
            } else if method == Method::OpenCode {
                summary.push("Built-in auth only (--pure); no external plugins.".into());
            } else if method.is_claude() {
                summary.push("Claude API billing, NOT Pro/Max subscription.".into());
                summary.push("Writes active ant profile; SDK may refresh tokens.".into());
            } else if method == Method::GeminiNative {
                summary.push("FULL Gemini CLI: /auth may clear credentials.".into());
                summary.push("What YOU type can start model work; exit to return.".into());
            }
        }
        summary.push("No task sent. Native scrollback may contain secrets.".into());
        summary.push("Exit is not login proof; cancel won't revoke tokens.".into());
        summary.push(format!(
            "Approval 5 min · timeout {} · d exact config/effects",
            if method.is_status() {
                "15 sec"
            } else {
                "10 min"
            }
        ));
        Ok(Self {
            profile,
            method,
            executable,
            lines,
            summary,
            trust: Some(trust),
            created: Instant::now(),
            directory: Some(directory),
        })
    }
    /// Eevee-owned website flow: no provider executable or private cwd is
    /// involved, so trust is the fixed endpoint contract alone.
    fn web(profile: usize, method: Method) -> Result<Self, String> {
        if !method.is_web() {
            return Err("Authentication method does not belong to this provider".into());
        }
        if matches!(method, Method::OpenAiBrowser | Method::OpenAiDevice) {
            return Ok(openai::plan(profile, method));
        }
        Ok(devin::plan(profile, method))
    }
    fn qualify(&self, project: &Path) -> Result<runner::Prepared, Outcome> {
        let (Some(directory), Some(trust)) = (&self.directory, &self.trust) else {
            return Err(Outcome::Refused);
        };
        if !directory.unchanged_empty() {
            return Err(Outcome::Refused);
        }
        if self.created.elapsed() >= LIFETIME {
            return Err(Outcome::Expired);
        }
        let prepared = agent::qualify_external(self.executable, project, trust, &INTERRUPTED)
            .map_err(|_| {
                if INTERRUPTED.load(Ordering::Acquire) {
                    Outcome::Cancelled
                } else {
                    Outcome::Refused
                }
            })?;
        if !directory.unchanged_empty() {
            return Err(Outcome::Refused);
        }
        if self.created.elapsed() >= LIFETIME {
            return Err(Outcome::Expired);
        }
        Ok(prepared)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Authenticated,
    NotAuthenticated,
    Cancelled,
    Expired,
    Failed,
    Unknown,
    Refused,
}
impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Authenticated => {
                "Provider reports authentication configured; account/model eligibility still requires a separately consented request."
            }
            Self::NotAuthenticated => {
                "Provider reports not logged in. Review a login method to connect."
            }
            Self::Cancelled => {
                "Authentication cancelled; provider-side changes may already have happened. No task was replayed."
            }
            Self::Expired => {
                "Authentication approval or operation expired. Review again; provider-side changes may already have happened."
            }
            Self::Failed => {
                "Authentication/status operation failed. Provider diagnostics were transient; no credential output was retained."
            }
            Self::Unknown => {
                "Authentication status unknown. Native UI returned, but normal exit/credential inventory is not authentication proof. Review provider status or separately qualify e; no task was replayed."
            }
            Self::Refused => {
                "Authentication not launched: executable/version/configuration/project no longer matches reviewed trust, or launch unavailable. Refresh and review again."
            }
        }
    }
}
/// A completed authentication attempt. Typed credentials are transient and may
/// be applied only by the owner of the matching, still-approved attempt.
pub struct Completion {
    pub outcome: Outcome,
    pub devin_credential: Option<DevinCredential>,
    pub openai_credential: Option<OpenAiCredential>,
    pub openai_failure: Option<OpenAiFailure>,
}
impl Completion {
    fn bare(outcome: Outcome) -> Self {
        Self {
            outcome,
            devin_credential: None,
            openai_credential: None,
            openai_failure: None,
        }
    }
}

/// Called only with terminal ownership suspended and fresh App admission.
pub fn run(plan: &Plan, project: &Path) -> Completion {
    let Ok(_signals) = NativeSignals::new() else {
        return Completion::bare(Outcome::Failed);
    };
    if plan.method.is_web() {
        if plan.created.elapsed() >= LIFETIME {
            return Completion::bare(Outcome::Expired);
        }
        if INTERRUPTED.load(Ordering::Acquire) {
            return Completion::bare(Outcome::Cancelled);
        }
        return match plan.method {
            Method::OpenAiBrowser | Method::OpenAiDevice => openai::login(plan.method),
            _ => devin::login(),
        };
    }
    let prepared = match plan.qualify(project) {
        Ok(value) => value,
        Err(outcome) => return Completion::bare(outcome),
    };
    if INTERRUPTED.load(Ordering::Acquire) {
        return Completion::bare(Outcome::Cancelled);
    }
    if plan.method == Method::CodexStatus {
        return Completion::bare(codex_status(&prepared));
    }
    let console_before = plan.method.is_claude().then(console_stamp);
    let result = native_ui(
        &prepared,
        if plan.method.is_status() {
            STATUS_TIMEOUT
        } else {
            LOGIN_TIMEOUT
        },
    );
    if matches!(plan.method, Method::CodexBrowser | Method::CodexDevice)
        && result == Outcome::Unknown
    {
        // Login may replace known configuration; never reuse stale executable trust.
        if plan.directory.as_ref().is_none_or(|d| !d.unchanged_empty())
            || plan
                .trust
                .as_ref()
                .is_none_or(|t| agent::revalidate_external_trust(plan.profile, project, t).is_err())
        {
            return Completion::bare(Outcome::Unknown);
        }
        let mut status = prepared;
        if let runner::CommandSpec::Direct { args, .. } = &mut status.spec {
            *args = Method::CodexStatus.args();
        }
        return Completion::bare(codex_status(&status));
    }
    if let Some(before) = console_before
        && result == Outcome::Unknown
    {
        return Completion::bare(console_outcome(plan.method, &before, &console_stamp()));
    }
    Completion::bare(result)
}

/// Credential-file metadata of the selected Console profile; contents are never read.
type ConsoleStamp = Option<(u64, Option<std::time::SystemTime>)>;
fn console_stamp() -> ConsoleStamp {
    let path = crate::providers::claude::console_credentials()?;
    let metadata = std::fs::symlink_metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()))
}
fn console_outcome(method: Method, before: &ConsoleStamp, after: &ConsoleStamp) -> Outcome {
    match (method, after) {
        // Absent profile credentials are definitive for the SDK's explicit-profile route.
        (_, None) => Outcome::NotAuthenticated,
        // Freshly written credentials mean ant completed a login; account/model
        // eligibility still requires a separately consented request.
        (Method::ClaudeConsole | Method::ClaudeConsoleCode, Some(_)) if after != before => {
            Outcome::Authenticated
        }
        _ => Outcome::Unknown,
    }
}

fn codex_status(prepared: &runner::Prepared) -> Outcome {
    let Ok(_raw) = RawInput::new() else {
        return Outcome::Failed;
    };
    let Ok(mut input) = crate::platform::input::Input::new() else {
        return Outcome::Failed;
    };
    let Ok(mut process) = runner::native::Process::spawn(prepared) else {
        return Outcome::Failed;
    };
    let started = Instant::now();
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 1024];
    let mut eof = [false; 2];
    loop {
        if INTERRUPTED.load(Ordering::Acquire) {
            return Outcome::Cancelled;
        }
        if started.elapsed() >= STATUS_TIMEOUT {
            return Outcome::Expired;
        }
        match input.read_timeout(Duration::ZERO) {
            Ok(Some(crossterm::event::Event::Key(key)))
                if key.code == crossterm::event::KeyCode::Esc
                    || (key.code == crossterm::event::KeyCode::Char('c')
                        && key
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)) =>
            {
                return Outcome::Cancelled;
            }
            Err(_) => return Outcome::Failed,
            _ => {}
        }
        for (stream, ended) in eof.iter_mut().enumerate() {
            if *ended {
                continue;
            }
            match process.read(stream, &mut buffer) {
                Ok(0) => *ended = true,
                Ok(n) if bytes.len() + n <= 4096 => bytes.extend_from_slice(&buffer[..n]),
                Ok(_) => return Outcome::Unknown,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Outcome::Failed,
            }
        }
        match process.try_wait() {
            Ok(Some((code, _))) if eof.iter().all(|ended| *ended) => {
                return parse_codex_status(code, &bytes);
            }
            Err(_) => return Outcome::Failed,
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}
fn parse_codex_status(code: Option<i32>, bytes: &[u8]) -> Outcome {
    // The official CLI emits nonfatal startup warnings on stderr before its
    // status line (for example, refusing PATH helpers in a temporary home).
    // Recognize exactly one status line, not a substring of arbitrary output.
    let text = std::str::from_utf8(bytes).ok();
    let mut lines = text
        .into_iter()
        .flat_map(str::lines)
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("WARNING:"));
    let status = lines.next();
    if lines.next().is_some() {
        return Outcome::Unknown;
    }
    match (code, status) {
        (Some(0), Some("Logged in using ChatGPT")) => Outcome::Authenticated,
        (Some(1), Some("Not logged in")) => Outcome::NotAuthenticated,
        // API-key output may contain a masked key: intentionally do not parse or retain it.
        _ => Outcome::Unknown,
    }
}

fn native_ui(prepared: &runner::Prepared, timeout: Duration) -> Outcome {
    // Spawn the reviewed executable identity itself, not the spec's program
    // string, so PATH re-resolution cannot substitute another binary.
    let runner::CommandSpec::Direct { args, .. } = &prepared.spec else {
        return Outcome::Refused;
    };
    let mut command = Command::new(&prepared.executable.path);
    command
        .args(args)
        .current_dir(&prepared.cwd)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let Ok(mut child) = NativeChild::spawn(&mut command) else {
        return Outcome::Failed;
    };
    if child.claim_terminal().is_err() {
        return Outcome::Failed;
    }
    let started = Instant::now();
    loop {
        if INTERRUPTED.load(Ordering::Acquire) {
            return Outcome::Cancelled;
        }
        if started.elapsed() >= timeout {
            return Outcome::Expired;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    if status.signal() == Some(libc::SIGINT) {
                        return Outcome::Cancelled;
                    }
                }
                return native_exit_outcome(status);
            }
            Err(_) => return Outcome::Failed,
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

fn native_exit_outcome(status: std::process::ExitStatus) -> Outcome {
    if status.success() {
        // Devin status can exit zero while logged out. Native output remains
        // in the terminal; Eevee has no speculative account/credential parser.
        Outcome::Unknown
    } else if matches!(status.code(), Some(130) | Some(-1073741510)) {
        Outcome::Cancelled
    } else {
        Outcome::Failed
    }
}

struct RawInput;
impl RawInput {
    fn new() -> io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }
}
impl Drop for RawInput {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}
/// Drain a buffered paste promptly, with bounded work per cancellation poll.
/// Unbracketed redirects arrive as Key events, not one Paste event; handling
/// only one per 25ms makes a long manual redirect appear stuck.
fn read_input(
    paste: &mut String,
    mut read: impl FnMut() -> std::io::Result<Option<crossterm::event::Event>>,
) -> Result<Option<String>, Outcome> {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
    for _ in 0..256 {
        match read().map_err(|_| Outcome::Failed)? {
            Some(Event::Key(key)) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Esc => return Err(Outcome::Cancelled),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err(Outcome::Cancelled);
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    paste.clear();
                }
                KeyCode::Enter => return Ok(Some(std::mem::take(paste))),
                KeyCode::Backspace => {
                    paste.pop();
                }
                KeyCode::Char(ch)
                    if !ch.is_control()
                        && !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    if paste.len() + ch.len_utf8() > INPUT_LIMIT {
                        paste.clear();
                        return Err(Outcome::Refused);
                    }
                    paste.push(ch);
                }
                _ => {}
            },
            Some(Event::Paste(value)) => {
                if paste.len() + value.len() > INPUT_LIMIT {
                    paste.clear();
                    return Err(Outcome::Refused);
                }
                paste.extend(value.chars().filter(|ch| !ch.is_control()));
                println!(
                    "Redirect input received (hidden). Enter submits; Ctrl+U clears; Esc cancels.\r"
                );
            }
            None => break,
            _ => {}
        }
    }
    Ok(None)
}
pub(super) fn open_browser(url: &str) {
    #[cfg(windows)]
    let failed = {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        // Pass the complete URL as data, not through cmd.exe: OAuth query
        // separators and other shell metacharacters must remain untouched.
        let url: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        // SAFETY: the URL is NUL-terminated and lives through this call.
        // A null operation selects the URL handler's default (open) verb.
        unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                std::ptr::null(),
                url.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            ) as isize
                <= 32
        }
    };
    #[cfg(not(windows))]
    let failed = {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(program)
            .arg(url)
            .spawn()
            .is_err()
    };
    if failed {
        println!("Could not launch a browser; open the URL above manually.");
    }
}

#[cfg(test)]
mod tests;
