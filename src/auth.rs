//! Explicit provider-owned authentication. No task prompt or credential is retained here.
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::{agent, runner};

const LIFETIME: Duration = Duration::from_secs(300);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const STATUS_TIMEOUT: Duration = Duration::from_secs(15);
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    CodexBrowser,
    CodexDevice,
    OpenCode,
    GeminiNative,
    ClaudeConsole,
    ClaudeConsoleCode,
    /// Eevee-owned authorization-code + PKCE flow against the provider's
    /// website; no provider CLI is launched. The returned session token is
    /// process memory only.
    DevinWeb,
    DevinBrowser,
    DevinManualToken,
    DevinStatus,
    CodexStatus,
    OpenCodeStatus,
    ClaudeConsoleStatus,
}
impl Method {
    pub fn label(self) -> &'static str {
        match self {
            Self::CodexBrowser => "ChatGPT / browser OAuth",
            Self::CodexDevice => "ChatGPT / device authorization",
            Self::OpenCode => "Official provider and auth-method chooser",
            Self::GeminiNative => "Official Gemini CLI / Google OAuth, API key or Vertex",
            Self::ClaudeConsole => "Anthropic Console OAuth / browser (official ant CLI)",
            Self::ClaudeConsoleCode => "Anthropic Console OAuth / paste code (no browser launch)",
            Self::DevinWeb => {
                "Devin / website OAuth (browser callback or pasted redirect; session credential)"
            }
            Self::DevinBrowser => "Devin CLI / browser OAuth (stored credentials)",
            Self::DevinManualToken => "Devin CLI / manual token paste (stored credentials)",
            Self::DevinStatus => "Official Devin auth status (not eligibility proof)",
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
            Self::CodexStatus
                | Self::OpenCodeStatus
                | Self::ClaudeConsoleStatus
                | Self::DevinStatus
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
        matches!(self, Self::DevinWeb)
    }
    pub fn guidance(self) -> Option<&'static str> {
        match self {
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
                "Eevee's own authorization-code + PKCE flow against the Devin website; no Devin CLI auth subcommand runs. A browser opens (or copy the printed URL) and the site redirects to a localhost listener on 127.0.0.1:59653. Without a browser or listener (SSH/WSL/headless), paste the localhost redirect URL after sign-in — or a session token (cog_/JWT form) — and press Enter. The token stays in process memory and is injected as WINDSURF_API_KEY only into devin acp processes Eevee launches; nothing is written to Devin's credential store, so devin auth status can still report logged out and restarting Eevee requires re-login. Esc cancels; expired/cancelled URLs cannot be reused. Never share callback URLs.",
            ),
            Self::DevinBrowser | Self::DevinManualToken => Some(
                "Runs the dedicated official devin auth login, not a chat or ACP session. Browser login is owned by Devin. For remote/SSH sessions, explicitly select manual token paste to skip browser auth; this is NOT device authorization. Paste any token only into Devin's native terminal prompt, never Eevee or task context. Enterprise machine policy may pin the host/account. Eevee neither reads nor stores credentials and never infers account/model eligibility from login exit.",
            ),
            _ => None,
        }
    }
    fn args(self) -> Vec<String> {
        if self.is_claude() {
            let profile = crate::claude::console_profile_name()
                .unwrap_or_else(|| crate::claude::CONSOLE_PROFILE.into());
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
            Self::DevinWeb => &[],
            Self::DevinBrowser => &["auth", "login"],
            Self::DevinManualToken => &["auth", "login", "--force-manual-token-flow"],
            Self::DevinStatus => &["auth", "status"],
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
        agent::DEVIN => &[
            Method::DevinWeb,
            Method::DevinBrowser,
            Method::DevinManualToken,
        ],
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
    // Codex device auth and Claude paste-code sit at index 1; Devin's only
    // callback-free method with a persisted credential store is its manual
    // token CLI flow at index 2.
    if *CALLBACK_FREE && matches!(profile, 0 | 1 | agent::DEVIN) {
        if profile == agent::DEVIN { 2 } else { 1 }
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
        agent::DEVIN => Some(Method::DevinStatus),
        _ => None,
    }
}

/// A provider session token issued by an Eevee-owned website flow. Lives in
/// process memory only: never serialized, journaled or printed. Debug output
/// is redacted; the secret bytes are zeroed on drop.
pub struct SessionCredential {
    /// Environment variable the token is injected under when Eevee launches
    /// the provider's ACP server (`WINDSURF_API_KEY` for `devin acp`).
    pub env: &'static str,
    token: String,
    /// Unix seconds at which the token stops being injected (JWT `exp`
    /// minus a 5-minute skew); None when the token is not a JWT.
    pub expires_at: Option<u64>,
}
impl SessionCredential {
    pub fn token(&self) -> &str {
        &self.token
    }
    pub fn expired(&self) -> bool {
        self.expires_at.is_some_and(|at| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|now| now.as_secs() >= at)
                .unwrap_or(false)
        })
    }
}
impl Clone for SessionCredential {
    fn clone(&self) -> Self {
        Self {
            env: self.env,
            token: self.token.clone(),
            expires_at: self.expires_at,
        }
    }
}
impl std::fmt::Debug for SessionCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionCredential")
            .field("env", &self.env)
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}
impl Drop for SessionCredential {
    fn drop(&mut self) {
        // Best-effort scrub of the only copy Eevee controls.
        unsafe {
            std::ptr::write_bytes(self.token.as_mut_ptr(), 0, self.token.len());
        }
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
            if crate::claude::console_profile_name().is_none() {
                return Err("ANTHROPIC_PROFILE must be 1-64 letters, digits, '_' or '-'; no login was started.".into());
            }
            agent::ANT
        } else {
            profile
        };
        let mut trust = agent::external_trust(executable, project, method.args()).map_err(|error| {
            if method.is_claude() {
                format!("{error}. Install the official Anthropic CLI (ant {}) on PATH outside the repository, then Refresh. {CLAUDE_POLICY}", agent::ANT_RELEASE)
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
            format!("Literal argv: {:?}; first runs {} and requires exact release {}.", method.args(), agent::version_argument(executable), release(executable)),
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
            lines.push(format!("May open your browser, contact provider identity/account services, {}and write/replace provider-owned credentials and auth-method settings. Switching Gemini auth method can clear previous cached credentials. Provider-native prompts choose accounts/providers; trust their displayed scopes before consenting. Eevee never requests or persists tokens, API keys, device codes or account identifiers.", if matches!(method, Method::CodexDevice | Method::ClaudeConsoleCode | Method::DevinManualToken) { "" } else { "bind a local OAuth callback, " }));
            lines.push("Eevee sends no task context or model prompt and never replays the task. Any later model request needs a separate review and Run approval. Cancellation does not revoke already-issued credentials; use the provider's account controls to revoke them.".into());
            if matches!(method, Method::CodexBrowser | Method::CodexDevice) {
                lines.push("After the native UI returns, literal argv [\"login\", \"status\"] refreshes Codex status if reviewed identities still match. Only an exact recognized status is kept; all raw output is discarded.".into());
            }
            if method.is_claude() {
                lines.push(format!("ant writes configs/<profile>.json (first login only), credentials/<profile>.json and active_config under {} for the named profile and makes it ant's active profile. The Claude Agent SDK sidecar later reads only this explicit profile, bills API usage to its Console organization/workspace and may refresh its access token in place. Remove it with ant auth logout --profile <profile>.", crate::claude::console_config_dir().map(|p| p.display().to_string()).unwrap_or_else(|| "the ant configuration directory".into())));
            }
            if method == Method::OpenCode {
                lines.push("OpenCode --pure uses official built-in auth routes without external auth plugins. Select an eligible supported account; Claude Pro/Max OAuth is not provided or bypassed.".into());
            }
        }
        if method == Method::GeminiNative {
            lines.push("EXTERNAL NATIVE LOGIN / NOT AN AUTH-ONLY SANDBOX. Gemini 0.28.0 has no standalone login command and mixes interactive OAuth text with ACP JSON. Eevee instead opens the official CLI outside your project. Choose its supported Google OAuth (browser or manual code), API-key or Vertex route there; use /auth if already connected. The CLI may continue into chat and initialize provider services/configuration; anything you type there is handled by Gemini and can start model work. Exit/Ctrl+C to return. Eevee does not collect or forward authorization codes, select a method by injected keystrokes, or infer authentication from exit.".into());
        } else {
            lines.push(
                "Terminal is handed to the official dedicated auth UI, not a chat session.".into(),
            );
        }
        lines.push("Use the native UI's exit/cancel action or Ctrl+C. Native output can contain secrets and remains in your terminal scrollback; Eevee does not capture it. A successful process exit alone is NOT proof of authentication.".into());
        lines.push("Approval expires in five minutes; login expires after ten minutes (status after fifteen seconds). Esc cancels without launching. Back is selected by default.".into());
        let mut summary = vec![
            format!("Method: {}", method.label()),
            format!("Run: {} {:?}", trust.prepared.executable.path.display(), method.args()),
            format!("First: {} (requires {}). Cwd: {}", agent::version_argument(executable), release(executable), trust.prepared.cwd.display()),
            "Private empty cwd outside your project; removed only while empty. Inherits environment, dependencies, config/hooks and browser launcher; not sandboxed. Executable and known config identities rechecked.".into(),
        ];
        if let Some(guidance) = method.guidance() {
            summary.push(guidance.into());
        }
        if method.is_status() {
            summary.push("Reads provider credentials; native status is not model/account eligibility. Eevee never retains raw output or infers authentication from process exit.".into());
        } else {
            summary.push(format!("May open browser, contact account services, {}and replace provider credentials/settings. Choose accounts/scopes in the native UI; Eevee collects no tokens or codes.", if matches!(method, Method::CodexDevice | Method::DevinManualToken) { "" } else { "bind OAuth callback, " }));
            if matches!(method, Method::CodexBrowser | Method::CodexDevice) {
                summary.push("Then runs [\"login\", \"status\"] if identities still match; keeps recognized status only.".into());
            } else if method == Method::OpenCode {
                summary.push("--pure: official built-in auth only, no external auth plugins or Claude subscription OAuth.".into());
            } else if method.is_claude() {
                summary.push("Console OAuth for Claude API billing, not claude.ai subscription. Writes the ant profile and makes it active; the SDK sidecar reads only that profile and may refresh its token.".into());
            } else if method == Method::GeminiNative {
                summary.push("Gemini opens its full CLI, NOT auth-only. Use /auth for Google/API key/Vertex; switching can clear credentials. It may initialize services and enter chat: what YOU type can start model work. Exit/Ctrl+C to return; exit does not prove login.".into());
            }
        }
        summary.push("No task prompt sent or replayed. Native output may contain secrets in terminal scrollback. Exit/Ctrl+C returns; cancellation does not revoke issued credentials. Native exit alone is not authentication proof.".into());
        summary.push(format!(
            "Approval: 5 min. Timeout: {}. Full configuration/effects are under Details.",
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
        let mut lines = vec![
            format!("Selected method: {}", method.label()),
            "No provider executable is launched. Eevee binds a loopback HTTP listener on 127.0.0.1:59653 (/callback, /launch), opens or prints the provider's official sign-in URL, verifies the returned state, and POSTs the authorization code and PKCE verifier to https://api.devin.ai/auth/cli/token. No other host or credential is contacted.".into(),
            "The issued session token is kept in process memory only and passed to Devin solely as the WINDSURF_API_KEY environment variable of devin acp processes Eevee itself launches; it is never written to Devin's credential store, printed, journaled or replayed.".into(),
        ];
        if let Some(guidance) = method.guidance() {
            lines.push(guidance.into());
        }
        lines.push("Eevee sends no task context or model prompt and never replays the task. Any later model request needs a separate review and Run approval. Cancellation does not revoke already-issued credentials; use the provider's account controls to revoke them.".into());
        lines.push("Use Esc/Ctrl+C to cancel. Nothing is persisted by Eevee; a successful exchange alone is NOT proof of account or model eligibility.".into());
        lines.push("Approval expires in five minutes; the login wait expires after ten minutes. Esc cancels without starting. Back is selected by default.".into());
        let mut summary = vec![
            format!("Method: {}", method.label()),
            "Endpoints: https://app.devin.ai/auth/cli/continue (sign-in), http://127.0.0.1:59653/callback (loopback), https://api.devin.ai/auth/cli/token (code+verifier exchange).".into(),
            "Credential stays in Eevee process memory; injected as WINDSURF_API_KEY only into devin acp launches; never persisted by Eevee.".into(),
        ];
        if let Some(guidance) = method.guidance() {
            summary.push(guidance.into());
        }
        summary.push("No task prompt sent or replayed. Esc/Ctrl+C returns; cancellation does not revoke issued credentials. Restarting Eevee requires a fresh login.".into());
        summary
            .push("Approval: 5 min. Timeout: 10 min. Full flow details are under Details.".into());
        Ok(Self {
            profile,
            method,
            executable: profile,
            lines,
            summary,
            trust: None,
            created: Instant::now(),
            directory: None,
        })
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
        let prepared = agent::qualify_external(
            self.executable,
            project,
            trust,
            release(self.executable),
            &INTERRUPTED,
        )
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
fn release(profile: usize) -> &'static str {
    match profile {
        0 => agent::CODEX_RELEASE,
        2 => agent::OPENCODE_RELEASE,
        3 => agent::GEMINI_RELEASE,
        agent::DEVIN => agent::DEVIN_RELEASE,
        agent::ANT => agent::ANT_RELEASE,
        _ => "unsupported",
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
/// A completed authentication attempt; `credential` is only ever present for
/// Eevee-owned website flows and holds the returned session token in memory.
pub struct Completion {
    pub outcome: Outcome,
    pub credential: Option<SessionCredential>,
}
impl Completion {
    fn bare(outcome: Outcome) -> Self {
        Self {
            outcome,
            credential: None,
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
        return devin_web::login();
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
    let path = crate::claude::console_credentials()?;
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
    let Ok(mut input) = crate::input::Input::new() else {
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
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
    }
    let Ok(child) = command.spawn() else {
        return Outcome::Failed;
    };
    let mut child = NativeChild::new(child);
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
        match child.child.try_wait() {
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

#[cfg(unix)]
struct NativeSignals {
    interrupt: libc::sigaction,
    background: libc::sigaction,
}
#[cfg(unix)]
extern "C" fn record_interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Release);
}
#[cfg(unix)]
impl NativeSignals {
    fn new() -> io::Result<Self> {
        INTERRUPTED.store(false, Ordering::Release);
        // SAFETY: signal handler uses only a lock-free atomic; restore on Drop.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = record_interrupt as usize;
            libc::sigemptyset(&mut action.sa_mask);
            let mut interrupt = std::mem::zeroed();
            let mut background = std::mem::zeroed();
            if libc::sigaction(libc::SIGINT, &action, &mut interrupt) != 0 {
                return Err(io::Error::last_os_error());
            }
            action.sa_sigaction = libc::SIG_IGN;
            if libc::sigaction(libc::SIGTTOU, &action, &mut background) != 0 {
                libc::sigaction(libc::SIGINT, &interrupt, std::ptr::null_mut());
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                interrupt,
                background,
            })
        }
    }
}
#[cfg(unix)]
impl Drop for NativeSignals {
    fn drop(&mut self) {
        unsafe {
            libc::sigaction(libc::SIGINT, &self.interrupt, std::ptr::null_mut());
            libc::sigaction(libc::SIGTTOU, &self.background, std::ptr::null_mut());
        }
    }
}
#[cfg(windows)]
struct NativeSignals;
#[cfg(windows)]
unsafe extern "system" fn console_interrupt(event: u32) -> i32 {
    if event == 0 || event == 1 {
        INTERRUPTED.store(true, Ordering::Release);
        1
    } else {
        0
    }
}
#[cfg(windows)]
impl NativeSignals {
    fn new() -> io::Result<Self> {
        INTERRUPTED.store(false, Ordering::Release);
        if unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(console_interrupt), 1)
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self)
        }
    }
}
#[cfg(windows)]
impl Drop for NativeSignals {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(console_interrupt), 0);
        }
    }
}
struct NativeChild {
    child: std::process::Child,
    #[cfg(unix)]
    foreground: libc::pid_t,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}
impl NativeChild {
    fn new(child: std::process::Child) -> Self {
        Self {
            child,
            #[cfg(unix)]
            foreground: unsafe { libc::tcgetpgrp(libc::STDIN_FILENO) },
            #[cfg(windows)]
            job: std::ptr::null_mut(),
        }
    }
    fn claim_terminal(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            if self.foreground < 0
                || unsafe { libc::tcsetpgrp(libc::STDIN_FILENO, self.child.id() as libc::pid_t) }
                    != 0
            {
                return Err(io::Error::last_os_error());
            }
            // A child that read before foreground handoff may have received SIGTTIN.
            unsafe {
                libc::kill(-(self.child.id() as libc::pid_t), libc::SIGCONT);
            }
            Ok(())
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
            use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
            use windows_sys::Win32::System::JobObjects::*;
            use windows_sys::Win32::System::Threading::{
                OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
            };
            self.job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if self.job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if unsafe {
                SetInformationJobObject(
                    self.job,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )
            } == 0
                || unsafe { AssignProcessToJobObject(self.job, self.child.as_raw_handle()) } == 0
            {
                return Err(io::Error::last_os_error());
            }
            // std::process owns the process handle but not its primary-thread
            // handle. Enumerate the still-suspended new process's sole thread;
            // no provider instruction can run before Job admission above.
            let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            let mut found = unsafe { Thread32First(snapshot, &mut entry) } != 0;
            let mut thread = std::ptr::null_mut();
            while found {
                if entry.th32OwnerProcessID == self.child.id() {
                    thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                    break;
                }
                entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
                found = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
            }
            unsafe {
                CloseHandle(snapshot);
            }
            if thread.is_null() {
                return Err(io::Error::other(
                    "Suspended authentication thread unavailable",
                ));
            }
            let previous = unsafe { ResumeThread(thread) };
            unsafe {
                CloseHandle(thread);
            }
            if previous != 1 {
                return Err(io::Error::other("Authentication thread admission failed"));
            }
            Ok(())
        }
    }
}
impl Drop for NativeChild {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL);
            if self.foreground >= 0 {
                libc::tcsetpgrp(libc::STDIN_FILENO, self.foreground);
            }
        }
        #[cfg(windows)]
        if !self.job.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.job);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
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

/// Eevee-owned website authorization-code + PKCE login for Devin. No provider
/// CLI runs: a loopback listener receives the website redirect, and the code
/// plus verifier are exchanged for a session token that exists only in this
/// process's memory.
mod devin_web {
    use super::{Completion, INTERRUPTED, LOGIN_TIMEOUT, Outcome, SessionCredential};
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    /// Registered redirect for the provider's CLI sign-in page. Fixed: a
    /// random port would produce a redirect URI the website refuses.
    const CALLBACK_PORT: u16 = 59653;
    const AUTHORIZE_URL: &str = "https://app.devin.ai/auth/cli/continue";
    const TOKEN_URL: &str = "https://api.devin.ai/auth/cli/token";
    const REQUEST_LIMIT: usize = 8 * 1024;
    const PASTE_LIMIT: usize = 8 * 1024;
    /// `exp` claim minus this skew decides when an injected credential stops
    /// being used; the token itself is never refreshed.
    pub(super) const EXPIRY_SKEW: u64 = 300;

    fn random_bytes(buffer: &mut [u8]) -> Result<(), Outcome> {
        getrandom::getrandom(buffer).map_err(|_| Outcome::Failed)
    }

    fn base64url_encode(bytes: &[u8]) -> String {
        base64_simd::URL_SAFE_NO_PAD.encode_to_string(bytes)
    }

    fn uuid() -> Result<String, Outcome> {
        let mut bytes = [0u8; 16];
        random_bytes(&mut bytes)?;
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let mut text = String::with_capacity(36);
        for (index, byte) in bytes.iter().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                text.push('-');
            }
            use std::fmt::Write as _;
            let _ = write!(text, "{byte:02x}");
        }
        Ok(text)
    }

    fn url_encode(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for byte in value.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                out.push(byte as char);
            } else {
                use std::fmt::Write as _;
                let _ = write!(out, "%{byte:02X}");
            }
        }
        out
    }

    fn url_decode(value: &str) -> String {
        let bytes = value.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'+' => {
                    out.push(b' ');
                    index += 1;
                }
                b'%' if index + 2 < bytes.len() => {
                    let hi = (bytes[index + 1] as char).to_digit(16);
                    let lo = (bytes[index + 2] as char).to_digit(16);
                    if let (Some(hi), Some(lo)) = (hi, lo) {
                        out.push((hi * 16 + lo) as u8);
                        index += 3;
                    } else {
                        out.push(b'%');
                        index += 1;
                    }
                }
                byte => {
                    out.push(byte);
                    index += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Minimal `?k=v&…` reader; first occurrence wins, names are decoded.
    fn query_param(query: &str, name: &str) -> Option<String> {
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            if url_decode(key) == name {
                return Some(url_decode(value));
            }
        }
        None
    }

    /// Accepts the full redirect URL, a bare `?…`/`code=…` query string, or a
    /// `code#state` fragment pair (the form the site shows for manual flow).
    pub(super) fn parse_callback_input(input: &str) -> (Option<String>, Option<String>) {
        let value = input.trim();
        if value.is_empty() {
            return (None, None);
        }
        if let Some(query) = value
            .split_once("://")
            .and_then(|(_, rest)| rest.split_once(['?', '#']))
            .map(|(_, rest)| rest.split('#').next().unwrap_or(rest))
        {
            let query = query.trim_end_matches('/');
            return (query_param(query, "code"), query_param(query, "state"));
        }
        if value.contains("code=") {
            let query = value.trim_start_matches(['?', '#']);
            return (query_param(query, "code"), query_param(query, "state"));
        }
        let mut parts = value.splitn(2, '#');
        let code = parts.next().map(str::trim).filter(|s| !s.is_empty());
        let state = parts.next().map(str::trim).filter(|s| !s.is_empty());
        (code.map(str::to_string), state.map(str::to_string))
    }

    /// Only printable, shell/environment-safe token bytes are ever injected
    /// into a child environment.
    pub(super) fn plausible_token(token: &str) -> bool {
        (16..=4096).contains(&token.len())
            && token.bytes().all(|byte| {
                byte.is_ascii_graphic()
                    && !matches!(
                        byte,
                        b'"' | b'\''
                            | b'`'
                            | b'\\'
                            | b';'
                            | b'|'
                            | b'&'
                            | b'<'
                            | b'>'
                            | b'$'
                            | b'!'
                            | b'('
                            | b')'
                            | b'{'
                            | b'}'
                            | b'['
                            | b']'
                            | b'*'
                            | b'?'
                            | b'~'
                            | b'^'
                    )
            })
    }

    /// JWT `exp` (seconds) minus skew; opaque tokens get no expiry.
    pub(super) fn jwt_expiry(token: &str) -> Option<u64> {
        let mut parts = token.split('.');
        parts.next()?;
        let payload = parts.next()?;
        parts.next()?;
        if parts.next().is_some() {
            return None;
        }
        let mut padded = payload.to_string();
        while padded.len() % 4 != 0 {
            padded.push('=');
        }
        let bytes = base64_simd::URL_SAFE
            .decode_to_vec(padded.as_bytes())
            .ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        let exp = value.get("exp")?.as_u64()?;
        Some(exp.saturating_sub(EXPIRY_SKEW))
    }

    struct Callback {
        code: Option<String>,
        denied: Option<String>,
    }

    /// Serves `/callback` (code/state/error intake) and `/launch` (302 to the
    /// live authorize URL for viewport-safe copying) on one loopback listener.
    fn serve(mut stream: std::net::TcpStream, state: &str, launch: &str) -> Option<Callback> {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
        let mut buffer = Vec::with_capacity(1024);
        let mut chunk = [0u8; 1024];
        while buffer.len() < REQUEST_LIMIT {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    buffer.extend_from_slice(&chunk[..n]);
                    if buffer.windows(4).any(|w| w == b"\r\n\r\n") || buffer.contains(&b'\n') {
                        break;
                    }
                }
            }
        }
        let head = String::from_utf8_lossy(&buffer);
        let mut request = head.split_whitespace();
        let target = match (request.next(), request.next()) {
            (Some("GET"), Some(target)) => target,
            _ => {
                let _ = stream
                    .write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n");
                return None;
            }
        };
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let respond = |stream: &mut std::net::TcpStream, status: &str, body: &str| {
            let _ = stream.write_all(
                format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes(),
            );
        };
        match path {
            "/launch" => {
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: {launch}\r\nContent-Length: 0\r\n\r\n"
                    )
                    .as_bytes(),
                );
                None
            }
            "/callback" => {
                let error = query_param(query, "error");
                let description = query_param(query, "error_description").or(error.clone());
                let code = query_param(query, "code");
                let got_state = query_param(query, "state").unwrap_or_default();
                if let Some(detail) = description {
                    if got_state == state {
                        respond(
                            &mut stream,
                            "200 OK",
                            "<html><body>Sign-in finished; return to Eevee.</body></html>",
                        );
                        return Some(Callback {
                            code: None,
                            denied: Some(detail),
                        });
                    }
                    respond(
                        &mut stream,
                        "400 Bad Request",
                        "<html><body>Not recognized; return to Eevee and retry.</body></html>",
                    );
                    return None;
                }
                let Some(code) = code.filter(|code| !code.is_empty()) else {
                    respond(
                        &mut stream,
                        "400 Bad Request",
                        "<html><body>Missing authorization code.</body></html>",
                    );
                    return None;
                };
                if got_state != state {
                    respond(
                        &mut stream,
                        "400 Bad Request",
                        "<html><body>State mismatch; return to Eevee and retry.</body></html>",
                    );
                    return None;
                }
                respond(
                    &mut stream,
                    "200 OK",
                    "<html><body>Sign-in finished; return to Eevee.</body></html>",
                );
                Some(Callback {
                    code: Some(code),
                    denied: None,
                })
            }
            _ => {
                respond(
                    &mut stream,
                    "404 Not Found",
                    "<html><body>Not found.</body></html>",
                );
                None
            }
        }
    }

    fn open_browser(url: &str) {
        let result = if cfg!(target_os = "macos") {
            std::process::Command::new("open").arg(url).spawn()
        } else if cfg!(target_os = "windows") {
            std::process::Command::new("cmd")
                .args(["/c", "start", "", url])
                .spawn()
        } else {
            std::process::Command::new("xdg-open").arg(url).spawn()
        };
        if result.is_err() {
            println!("Could not launch a browser; open the URL above manually.");
        }
    }

    fn exchange(code: &str, verifier: &str) -> Result<SessionCredential, String> {
        let agent = ureq::AgentBuilder::new()
            .https_only(true)
            .redirects(0)
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(15))
            .timeout_write(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build();
        let body = serde_json::to_string(&serde_json::json!({
            "code": code,
            "code_verifier": verifier,
        }))
        .map_err(|_| "could not encode the token request".to_string())?;
        let response = agent
            .post(TOKEN_URL)
            .set("Accept", "application/json")
            .set("Content-Type", "application/json")
            .send_string(&body)
            .map_err(|_| "token exchange failed or was refused".to_string())?;
        if !(200..300).contains(&response.status()) {
            return Err(format!(
                "token endpoint returned HTTP {}",
                response.status()
            ));
        }
        let body: serde_json::Value =
            serde_json::from_reader(response.into_reader().take(64 * 1024))
                .map_err(|_| "token endpoint returned malformed or oversized JSON".to_string())?;
        let token = body
            .get("token")
            .and_then(serde_json::Value::as_str)
            .filter(|token| plausible_token(token))
            .ok_or_else(|| "token endpoint returned no usable token".to_string())?;
        Ok(SessionCredential {
            env: "WINDSURF_API_KEY",
            token: token.to_string(),
            expires_at: jwt_expiry(token),
        })
    }

    pub fn login() -> Completion {
        let state = match uuid() {
            Ok(state) => state,
            Err(outcome) => return Completion::bare(outcome),
        };
        let mut verifier_bytes = [0u8; 96];
        if let Err(outcome) = random_bytes(&mut verifier_bytes) {
            return Completion::bare(outcome);
        }
        let verifier = base64url_encode(&verifier_bytes);
        let challenge = base64url_encode(&Sha256::digest(verifier.as_bytes()));
        let listener = match TcpListener::bind(("127.0.0.1", CALLBACK_PORT)) {
            Ok(listener) => {
                let _ = listener.set_nonblocking(true);
                Some(listener)
            }
            Err(_) => {
                println!(
                    "Port 127.0.0.1:{CALLBACK_PORT} is unavailable; continuing without a local callback listener.\r"
                );
                None
            }
        };
        let redirect = format!("http://127.0.0.1:{CALLBACK_PORT}/callback");
        let url = format!(
            "{AUTHORIZE_URL}?response_type=code&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256&state={state}&prompt=select_account",
            url_encode(&redirect)
        );
        let launch = format!("http://127.0.0.1:{CALLBACK_PORT}/launch");
        println!("Devin website sign-in.\r");
        println!("Open: {url}\r");
        if listener.is_some() {
            println!("Short redirect: {launch}\r");
        }
        println!(
            "After sign-in, the site returns to localhost automatically. Without a browser, paste the http://127.0.0.1:{CALLBACK_PORT}/callback?code=… redirect URL, a code#state pair, or a session token, then Enter. Esc cancels.\r"
        );
        let Ok(_raw) = super::RawInput::new() else {
            return Completion::bare(Outcome::Failed);
        };
        let Ok(mut input) = crate::input::Input::new() else {
            return Completion::bare(Outcome::Failed);
        };
        open_browser(&url);
        let started = Instant::now();
        let mut paste = String::new();
        loop {
            if INTERRUPTED.load(Ordering::Acquire) {
                return Completion::bare(Outcome::Cancelled);
            }
            if started.elapsed() >= LOGIN_TIMEOUT {
                return Completion::bare(Outcome::Expired);
            }
            if let Some(listener) = &listener {
                loop {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let Some(callback) = serve(stream, &state, &launch) else {
                                continue;
                            };
                            if callback.denied.is_some() {
                                println!("Devin sign-in was declined or failed.\r");
                                return Completion::bare(Outcome::NotAuthenticated);
                            }
                            let Some(code) = callback.code else {
                                continue;
                            };
                            println!("Exchanging the authorization code…\r");
                            return match exchange(&code, &verifier) {
                                Ok(credential) => Completion {
                                    outcome: Outcome::Authenticated,
                                    credential: Some(credential),
                                },
                                Err(reason) => {
                                    println!("Devin token exchange failed: {reason}.\r");
                                    Completion::bare(Outcome::Failed)
                                }
                            };
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => break,
                    }
                }
            }
            match input.read_timeout(Duration::from_millis(25)) {
                Ok(Some(crossterm::event::Event::Key(key))) => {
                    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
                    if matches!(key.kind, KeyEventKind::Release) {
                        continue;
                    }
                    match key.code {
                        KeyCode::Esc => return Completion::bare(Outcome::Cancelled),
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Completion::bare(Outcome::Cancelled);
                        }
                        KeyCode::Enter => {
                            let pasted = std::mem::take(&mut paste);
                            let trimmed = pasted.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            if plausible_token(trimmed) {
                                return Completion {
                                    outcome: Outcome::Authenticated,
                                    credential: Some(SessionCredential {
                                        env: "WINDSURF_API_KEY",
                                        token: trimmed.to_string(),
                                        expires_at: jwt_expiry(trimmed),
                                    }),
                                };
                            }
                            let (code, got_state) = parse_callback_input(&pasted);
                            match (code, got_state) {
                                (Some(code), Some(got)) if got == state => {
                                    println!("Exchanging the authorization code…\r");
                                    return match exchange(&code, &verifier) {
                                        Ok(credential) => Completion {
                                            outcome: Outcome::Authenticated,
                                            credential: Some(credential),
                                        },
                                        Err(reason) => {
                                            println!("Devin token exchange failed: {reason}.\r");
                                            Completion::bare(Outcome::Failed)
                                        }
                                    };
                                }
                                (Some(_), _) => println!(
                                    "State mismatch; paste the exact redirect URL from this sign-in.\r"
                                ),
                                _ => println!(
                                    "Paste the redirect URL (…/callback?code=…&state=…), a code#state pair, or a session token.\r"
                                ),
                            }
                        }
                        KeyCode::Char(ch)
                            if !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                                && paste.len() + ch.len_utf8() <= PASTE_LIMIT =>
                        {
                            paste.push(ch);
                        }
                        KeyCode::Backspace => {
                            paste.pop();
                        }
                        _ => {}
                    }
                }
                Ok(Some(crossterm::event::Event::Paste(text))) => {
                    for ch in text.chars() {
                        if paste.len() + ch.len_utf8() > PASTE_LIMIT {
                            break;
                        }
                        if !ch.is_control() {
                            paste.push(ch);
                        }
                    }
                }
                Ok(_) => {}
                Err(_) => return Completion::bare(Outcome::Failed),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_free_default_covers_wsl_and_remote_without_changing_provider_contracts() {
        for key in [
            "WSL_DISTRO_NAME",
            "WSL_INTEROP",
            "SSH_CONNECTION",
            "SSH_CLIENT",
            "SSH_TTY",
        ] {
            assert!(callback_free_environment(|name| name == key, ""));
        }
        assert!(callback_free_environment(
            |_| false,
            "6.18.54.1-microsoft-standard-WSL2"
        ));
        assert!(callback_free_environment(|_| false, "4.4.0-Microsoft"));
        assert!(!callback_free_environment(|_| false, "6.18.1-generic"));
        assert!(!callback_free_environment(|_| false, ""));
        for profile in 2..4 {
            assert_eq!(default_method(profile), 0);
        }
        assert_eq!(
            methods(1),
            &[Method::ClaudeConsole, Method::ClaudeConsoleCode]
        );
        assert_eq!(methods(0), &[Method::CodexBrowser, Method::CodexDevice]);
        assert_eq!(Method::CodexBrowser.args(), ["login"]);
        assert_eq!(Method::CodexDevice.args(), ["login", "--device-auth"]);
        assert_eq!(Method::CodexStatus.args(), ["login", "status"]);
    }

    #[test]
    fn status_requires_exact_provider_evidence_not_success_exit() {
        assert_eq!(parse_codex_status(Some(0), b""), Outcome::Unknown);
        assert_eq!(
            parse_codex_status(Some(0), b"Logged in using ChatGPT\n"),
            Outcome::Authenticated
        );
        assert_eq!(
            parse_codex_status(Some(1), b"Logged in using ChatGPT"),
            Outcome::Unknown
        );
        assert_eq!(
            parse_codex_status(Some(1), b"Not logged in\n"),
            Outcome::NotAuthenticated
        );
        assert_eq!(
            parse_codex_status(
                Some(1),
                b"WARNING: refusing temporary PATH helpers\nNot logged in\n"
            ),
            Outcome::NotAuthenticated
        );
        assert_eq!(
            parse_codex_status(Some(0), b"Not logged in\nLogged in using ChatGPT\n"),
            Outcome::Unknown
        );
        assert_eq!(
            parse_codex_status(Some(0), b"Logged in using ChatGPT\nsecret"),
            Outcome::Unknown
        );
        assert_eq!(
            parse_codex_status(Some(0), b"Logged in using an API key - sk-secret"),
            Outcome::Unknown
        );
    }
    #[test]
    fn authentication_cwd_rejects_inserted_configuration_and_preserves_native_work() {
        let parent = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let directory = AuthDirectory::create(&parent).unwrap();
        let path = directory.path.clone();
        assert!(directory.unchanged_empty());
        let artifact = path.join("native-work.txt");
        std::fs::write(&artifact, b"User work created in the provider").unwrap();
        assert!(!directory.unchanged_empty());
        drop(directory);
        assert_eq!(
            std::fs::read(&artifact).unwrap(),
            b"User work created in the provider"
        );
        std::fs::remove_file(artifact).unwrap();
        std::fs::remove_dir(path).unwrap();
    }

    #[test]
    fn authentication_methods_cannot_cross_provider() {
        assert!(Plan::prepare(1, Method::CodexBrowser, Path::new(".")).is_err());
        assert!(Plan::prepare(0, Method::GeminiNative, Path::new(".")).is_err());
        assert!(Plan::prepare(0, Method::ClaudeConsole, Path::new(".")).is_err());
        assert!(Plan::prepare(3, Method::ClaudeConsoleStatus, Path::new(".")).is_err());
        assert!(Plan::prepare(0, Method::DevinBrowser, Path::new(".")).is_err());
        assert!(Plan::prepare(agent::DEVIN, Method::CodexBrowser, Path::new(".")).is_err());
        assert!(Plan::prepare(agent::ANT, Method::DevinBrowser, Path::new(".")).is_err());
        assert!(methods(1).iter().all(|method| method.is_claude()));
    }

    #[test]
    fn claude_uses_console_oauth_profile_never_subscription_login() {
        let profile = crate::claude::console_profile_name().unwrap();
        assert_eq!(
            Method::ClaudeConsole.args(),
            ["auth", "login", "--profile", profile.as_str()]
        );
        assert_eq!(
            Method::ClaudeConsoleCode.args(),
            [
                "auth",
                "login",
                "--no-browser",
                "--profile",
                profile.as_str()
            ]
        );
        assert_eq!(
            Method::ClaudeConsoleStatus.args(),
            ["auth", "status", "--profile", profile.as_str()]
        );
        assert_eq!(status_method(1), Some(Method::ClaudeConsoleStatus));
        let at = Some(std::time::SystemTime::UNIX_EPOCH);
        let old = Some((10, at));
        let fresh = Some((12, at));
        assert_eq!(
            console_outcome(Method::ClaudeConsole, &None, &fresh),
            Outcome::Authenticated
        );
        assert_eq!(
            console_outcome(Method::ClaudeConsoleCode, &old, &old),
            Outcome::Unknown
        );
        assert_eq!(
            console_outcome(Method::ClaudeConsole, &old, &None),
            Outcome::NotAuthenticated
        );
        assert_eq!(
            console_outcome(Method::ClaudeConsoleStatus, &old, &fresh),
            Outcome::Unknown
        );
    }

    #[cfg(unix)]
    #[test]
    fn native_status_success_is_not_authentication_evidence() {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            native_exit_outcome(std::process::ExitStatus::from_raw(0)),
            Outcome::Unknown
        );
        assert_eq!(
            native_exit_outcome(std::process::ExitStatus::from_raw(1 << 8)),
            Outcome::Failed
        );
        assert_eq!(
            native_exit_outcome(std::process::ExitStatus::from_raw(130 << 8)),
            Outcome::Cancelled
        );
    }

    #[test]
    fn devin_web_callback_input_parses_redirect_query_and_fragment_forms() {
        assert_eq!(
            devin_web::parse_callback_input(
                "http://127.0.0.1:8765/callback?code=abc123&state=xyz789"
            ),
            (Some("abc123".into()), Some("xyz789".into()))
        );
        assert_eq!(
            devin_web::parse_callback_input("?code=abc%20123&state=x"),
            (Some("abc 123".into()), Some("x".into()))
        );
        assert_eq!(
            devin_web::parse_callback_input("abc123#xyz789"),
            (Some("abc123".into()), Some("xyz789".into()))
        );
        assert_eq!(devin_web::parse_callback_input(""), (None, None));
    }

    #[test]
    fn devin_web_token_plausibility_rejects_shell_and_whitespace_bytes() {
        assert!(!devin_web::plausible_token(""));
        assert!(!devin_web::plausible_token(&"x".repeat(15)));
        assert!(!devin_web::plausible_token(&"a".repeat(4097)));
        assert!(!devin_web::plausible_token(&format!(
            "{};rm",
            "a".repeat(16)
        )));
        assert!(!devin_web::plausible_token(&format!("{} ", "a".repeat(16))));
        assert!(!devin_web::plausible_token(&format!(
            "{}$x",
            "a".repeat(16)
        )));
        assert!(devin_web::plausible_token(&"a".repeat(32)));
        let jwt = format!("{}.{}.{}", "a".repeat(8), "b".repeat(8), "c".repeat(8));
        assert!(devin_web::plausible_token(&jwt));
    }

    #[test]
    fn devin_web_jwt_expiry_reads_exp_minus_skew_and_rejects_malformed() {
        let segment = base64_simd::URL_SAFE
            .encode_to_string(br#"{"exp":2000}"#)
            .trim_end_matches('=')
            .to_string();
        let token = format!("header.{segment}.signature");
        assert_eq!(
            devin_web::jwt_expiry(&token),
            Some(2000 - devin_web::EXPIRY_SKEW)
        );
        assert_eq!(devin_web::jwt_expiry("opaque-token-no-dots"), None);
        assert_eq!(devin_web::jwt_expiry("a.b"), None);
        assert_eq!(devin_web::jwt_expiry("a.b.c.d"), None);
        assert_eq!(devin_web::jwt_expiry("a.@@@.c"), None);
    }

    #[test]
    fn session_credential_redacts_debug_and_reports_expiry() {
        let mut credential = crate::auth::SessionCredential {
            env: "WINDSURF_API_KEY",
            token: "opaque-session-secret".into(),
            expires_at: None,
        };
        assert!(!format!("{credential:?}").contains("opaque-session-secret"));
        assert!(!credential.expired());
        credential.expires_at = Some(0);
        assert!(credential.expired());
        credential.expires_at = Some(u64::MAX);
        assert!(!credential.expired());
        assert_eq!(credential.env, "WINDSURF_API_KEY");
        assert_eq!(credential.token(), "opaque-session-secret");
    }
}
