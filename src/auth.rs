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
    CodexStatus,
    OpenCodeStatus,
}
impl Method {
    pub fn label(self) -> &'static str {
        match self {
            Self::CodexBrowser => "ChatGPT / browser OAuth",
            Self::CodexDevice => "ChatGPT / device authorization",
            Self::OpenCode => "Official provider and auth-method chooser",
            Self::GeminiNative => "Official Gemini CLI / Google OAuth, API key or Vertex",
            Self::CodexStatus => "Official Codex login status",
            Self::OpenCodeStatus => {
                "Official OpenCode credential inventory (not eligibility proof)"
            }
        }
    }
    pub fn is_status(self) -> bool {
        matches!(self, Self::CodexStatus | Self::OpenCodeStatus)
    }
    fn args(self) -> Vec<String> {
        let args: &[&str] = match self {
            Self::CodexBrowser => &["login"],
            Self::CodexDevice => &["login", "--device-auth"],
            Self::OpenCode => &["auth", "login", "--pure"],
            Self::CodexStatus => &["login", "status"],
            Self::OpenCodeStatus => &["auth", "list"],
            Self::GeminiNative => &[],
        };
        args.iter().map(|arg| (*arg).into()).collect()
    }
}
pub fn methods(profile: usize) -> &'static [Method] {
    match profile {
        0 => &[Method::CodexBrowser, Method::CodexDevice],
        2 => &[Method::OpenCode],
        3 => &[Method::GeminiNative],
        _ => &[],
    }
}
pub const CLAUDE_POLICY: &str = "Claude Agent SDK uses Anthropic API credentials (ANTHROPIC_API_KEY) or configured Bedrock/Vertex cloud credentials. Set these outside Eevee and install the pinned SDK sidecar described in Setup (README). Claude CLI login is not SDK setup: claude.ai subscription OAuth tokens cannot be reused here. Eevee never collects tokens. A model request still needs separate Run approval.";
pub fn status_method(profile: usize) -> Option<Method> {
    match profile {
        0 => Some(Method::CodexStatus),
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
    pub lines: Vec<String>,
    pub summary: Vec<String>,
    trust: agent::ExternalTrust,
    created: Instant,
    directory: AuthDirectory,
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
        let mut trust = agent::external_trust(profile, project, method.args())?;
        let directory = AuthDirectory::create(&trust.prepared.cwd)?;
        trust.prepared.cwd = directory.path.clone();
        trust.prepared.cwd_identity = directory.identity.clone();
        let mut lines = vec![
            format!("Selected method: {}", method.label()),
            format!("Executable: {}", trust.prepared.executable.path.display()),
            format!("Literal argv: {:?}; first runs --version and requires exact release {}.", method.args(), release(profile)),
            format!("Working directory: {} (private empty directory outside task project; removed only if still empty, never recursively).", trust.prepared.cwd.display()),
            trust.authentication_configuration().to_owned(),
            "Trust includes inherited environment, interpreter, dependencies, provider configuration/hooks/plugins and browser launcher. Executable and known configuration/project identities are checked immediately before launch. This is not sandboxing or atomic dependency pinning.".into(),
        ];
        if method.is_status() {
            lines.push("This launches the official local status command; the provider may read its own credentials. Output is transient, not copied into task state. A credential inventory is not proof of account eligibility or a successful model request.".into());
        } else {
            lines.push("May open your browser, contact provider identity/account services, bind a local OAuth callback, and write/replace provider-owned credentials and auth-method settings. Switching Gemini auth method can clear previous cached credentials. Provider-native prompts choose accounts/providers; trust their displayed scopes before consenting. Eevee never requests or persists tokens, API keys, device codes or account identifiers.".into());
            lines.push("Eevee sends no task context or model prompt and never replays the task. Any later model request needs a separate review and Run approval. Cancellation does not revoke already-issued credentials; use the provider's account controls to revoke them.".into());
            if matches!(method, Method::CodexBrowser | Method::CodexDevice) {
                lines.push("After the native UI returns, literal argv [\"login\", \"status\"] refreshes Codex status if reviewed identities still match. Only an exact recognized status is kept; all raw output is discarded.".into());
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
            format!("First: --version (requires {}). Cwd: {}", release(profile), trust.prepared.cwd.display()),
            "Private empty cwd outside your project; removed only while empty. Inherits environment, dependencies, config/hooks and browser launcher; not sandboxed. Executable and known config identities rechecked.".into(),
        ];
        if method.is_status() {
            summary.push("Reads provider credentials; shows native status, not model/account eligibility. Eevee keeps only a recognized classification, never raw output.".into());
        } else {
            summary.push("May open browser, contact account services, bind OAuth callback, and replace provider credentials/settings. Choose accounts/scopes in the native UI; Eevee collects no tokens or codes.".into());
            if matches!(method, Method::CodexBrowser | Method::CodexDevice) {
                summary.push("Then runs [\"login\", \"status\"] if identities still match; keeps recognized status only.".into());
            } else if method == Method::OpenCode {
                summary.push("--pure: official built-in auth only, no external auth plugins or Claude subscription OAuth.".into());
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
            lines,
            summary,
            trust,
            created: Instant::now(),
            directory,
        })
    }
    fn qualify(&self, project: &Path) -> Result<runner::Prepared, Outcome> {
        if !self.directory.unchanged_empty() {
            return Err(Outcome::Refused);
        }
        if self.created.elapsed() >= LIFETIME {
            return Err(Outcome::Expired);
        }
        let prepared = agent::qualify_external(
            self.profile,
            project,
            &self.trust,
            release(self.profile),
            &INTERRUPTED,
        )
        .map_err(|_| {
            if INTERRUPTED.load(Ordering::Acquire) {
                Outcome::Cancelled
            } else {
                Outcome::Refused
            }
        })?;
        if !self.directory.unchanged_empty() {
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

/// Called only with terminal ownership suspended and fresh App admission.
pub fn run(plan: &Plan, project: &Path) -> Outcome {
    let Ok(_signals) = NativeSignals::new() else {
        return Outcome::Failed;
    };
    let prepared = match plan.qualify(project) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };
    if INTERRUPTED.load(Ordering::Acquire) {
        return Outcome::Cancelled;
    }
    if plan.method == Method::CodexStatus {
        return codex_status(&prepared);
    }
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
        if !plan.directory.unchanged_empty()
            || agent::revalidate_external_trust(plan.profile, project, &plan.trust).is_err()
        {
            return Outcome::Unknown;
        }
        let mut status = prepared;
        if let runner::CommandSpec::Direct { args, .. } = &mut status.spec {
            *args = Method::CodexStatus.args();
        }
        return codex_status(&status);
    }
    result
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
    let runner::CommandSpec::Direct { program, args } = &prepared.spec else {
        return Outcome::Refused;
    };
    let mut command = Command::new(program);
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
                return if status.success() {
                    Outcome::Unknown
                } else if matches!(status.code(), Some(130) | Some(-1073741510)) {
                    Outcome::Cancelled
                } else {
                    Outcome::Failed
                };
            }
            Err(_) => return Outcome::Failed,
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(methods(1).is_empty());
        assert!(Plan::prepare(1, Method::CodexBrowser, Path::new(".")).is_err());
        assert!(Plan::prepare(0, Method::GeminiNative, Path::new(".")).is_err());
    }
}
