//! Explicit local commands with approval-at-dispatch and owned-tree cancellation.
//! Process groups and Jobs are cancellation ownership, NOT filesystem/network sandboxes.
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const COMMAND_CAPACITY: usize = 4;
const EVENT_CAPACITY: usize = 8;
const OUTPUT_CAPACITY: usize = 32;
const LINE_LIMIT: usize = 16 * 1024;
const SPEC_LIMIT: usize = 64 * 1024;
const APPROVAL_LIFETIME_MS: u64 = 5 * 60 * 1000;
const SOFT_STOP: Duration = Duration::from_millis(750);
const HARD_STOP: Duration = Duration::from_secs(3);
static SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CommandSpec {
    Direct {
        program: String,
        args: Vec<String>,
    },
    Shell {
        executable: String,
        dialect: ShellDialect,
        text: String,
    },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ShellDialect {
    Posix,
    Cmd,
    PowerShell,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputSnapshot {
    pub fingerprint: Option<String>,
    pub complete: bool,
    pub stable: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutableIdentity {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
    pub modified_ns: u128,
    pub file_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Prepared {
    pub request_id: String,
    pub run_id: String,
    pub task_id: i64,
    pub spec: CommandSpec,
    pub cwd: PathBuf,
    pub cwd_identity: String,
    pub executable: ExecutableIdentity,
    pub prepared_at_ms: u64,
    pub expires_at_ms: u64,
    /// Process-wide preparation arrival order; legacy records have no ordering stamp.
    #[serde(default)]
    pub approval_arrival: u64,
    pub before: InputSnapshot,
    /// Requesting partner (PT06A); absent on runs recorded before attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partner: Option<crate::store::Attribution>,
    /// A deliberate new approval may cite a previous task-owned run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rerun_of: Option<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RunState {
    AwaitingDispatch,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
    OutcomeUnknown,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TreeOutcome {
    NotStarted,
    OwnedTreeExited,
    OwnershipUnknown,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunRecord {
    pub prepared: Prepared,
    pub state: RunState,
    pub started_at_ms: Option<u64>,
    pub ended_at_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub tree: TreeOutcome,
    pub cancelled: bool,
    pub truncated: bool,
    pub output_unavailable: bool,
    /// Overlapped task write ownership; never stable verification, even after restart.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub revision_moving: bool,
    pub captured_bytes: u64,
    pub discarded_bytes: u64,
    pub dropped_live_bytes: u64,
    pub error: Option<String>,
    pub after: InputSnapshot,
}
impl RunRecord {
    pub fn awaiting(prepared: &Prepared) -> Self {
        Self {
            prepared: prepared.clone(),
            state: RunState::AwaitingDispatch,
            started_at_ms: None,
            ended_at_ms: None,
            exit_code: None,
            signal: None,
            tree: TreeOutcome::NotStarted,
            cancelled: false,
            truncated: false,
            output_unavailable: false,
            revision_moving: false,
            captured_bytes: 0,
            discarded_bytes: 0,
            dropped_live_bytes: 0,
            error: None,
            after: InputSnapshot::default(),
        }
    }
    /// Never apply this to the live request: executed arguments must remain exact.
    pub fn redact_for_storage(&mut self) {
        fn redact(value: &mut String) {
            *value = crate::workspace::privacy::redact(value).into_owned();
        }
        match &mut self.prepared.spec {
            CommandSpec::Direct { program, args } => {
                redact(program);
                // Also cover credentials split across argv, including PEM blocks and flag values.
                let mut secret_next = false;
                let mut private_key = false;
                for arg in args {
                    let name = arg.trim_start_matches('-').to_ascii_lowercase();
                    let secret_flag = matches!(
                        name.as_str(),
                        "password"
                            | "passwd"
                            | "token"
                            | "secret"
                            | "api-key"
                            | "api_key"
                            | "authorization"
                            | "client-secret"
                    );
                    let begins_key =
                        arg.contains("-----BEGIN ") && arg.contains("PRIVATE KEY-----");
                    let ends_key = arg.contains("-----END ") && arg.contains("PRIVATE KEY-----");
                    if secret_next || private_key || begins_key {
                        *arg = "[REDACTED]".into();
                    } else {
                        let prefix = arg.len() - arg.trim_start_matches('-').len();
                        let value = crate::workspace::privacy::redact(&arg[prefix..]);
                        if let std::borrow::Cow::Owned(value) = value {
                            arg.replace_range(prefix.., &value);
                        }
                    }
                    private_key = (private_key || begins_key) && !ends_key;
                    secret_next = secret_flag;
                }
            }
            CommandSpec::Shell {
                executable, text, ..
            } => {
                redact(executable);
                redact(text);
            }
        }
        if let Some(error) = &mut self.error {
            redact(error);
        }
        for path in [&mut self.prepared.cwd, &mut self.prepared.executable.path] {
            if let Some(text) = path.to_str() {
                *path = PathBuf::from(crate::workspace::privacy::redact(text).as_ref());
            }
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Output {
    pub request_id: String,
    pub run_id: String,
    pub task_id: i64,
    pub stream: OutputStream,
    pub at_ms: u64,
    pub text: String,
}
#[derive(Clone, Debug)]
pub enum Event {
    Prepared(Prepared),
    Rejected {
        request_id: String,
        task_id: i64,
        message: String,
    },
    Started(RunRecord),
    Cancelling(RunRecord),
    CaptureChanged(RunRecord),
    Output(Output),
    Finished(RunRecord),
}

#[derive(Default)]
struct Gate {
    pending: Option<String>,
    active: Option<String>,
    cancelled: bool,
}
enum Command {
    Prepare {
        request_id: String,
        task_id: i64,
        spec: CommandSpec,
        cwd: PathBuf,
        before: InputSnapshot,
        partner: Option<crate::store::Attribution>,
        move_root: Option<PathBuf>,
    },
    Dispatch {
        request_id: String,
        permit: crate::store::CommittedRunPermit,
    },
}
pub struct Worker {
    commands: Option<SyncSender<Command>>,
    events: Arc<Mutex<VecDeque<Event>>>,
    output: Receiver<Output>,
    gate: Arc<Mutex<Gate>>,
    stopped: Arc<AtomicBool>,
    session: String,
}
impl Worker {
    pub fn start() -> io::Result<Self> {
        let (commands, incoming) = mpsc::sync_channel(COMMAND_CAPACITY);
        let events = Arc::new(Mutex::new(VecDeque::with_capacity(EVENT_CAPACITY)));
        let outgoing = Arc::clone(&events);
        let (live, output) = mpsc::sync_channel(OUTPUT_CAPACITY);
        let gate = Arc::new(Mutex::new(Gate::default()));
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_gate = Arc::clone(&gate);
        let worker_stopped = Arc::clone(&stopped);
        thread::Builder::new()
            .name("ev-runner".into())
            .spawn(move || {
                worker(incoming, outgoing, live, worker_gate, worker_stopped);
            })?;
        Ok(Self {
            commands: Some(commands),
            events,
            output,
            gate,
            stopped,
            session: unique_id(),
        })
    }
    /// Queues filesystem/executable discovery, never an execution. Replaces any pending grant.
    #[cfg(test)]
    pub fn prepare(
        &self,
        task_id: i64,
        spec: CommandSpec,
        cwd: PathBuf,
        before: InputSnapshot,
    ) -> Result<String, String> {
        self.prepare_for(task_id, None, spec, cwd, before)
    }
    /// `prepare` on behalf of a partner, whose snapshot the run record keeps (PT06A).
    pub fn prepare_for(
        &self,
        task_id: i64,
        partner: Option<crate::store::Attribution>,
        spec: CommandSpec,
        cwd: PathBuf,
        before: InputSnapshot,
    ) -> Result<String, String> {
        self.prepare_for_root(task_id, partner, spec, cwd, before, None)
    }

    pub fn prepare_move_for(
        &self,
        task_id: i64,
        partner: Option<crate::store::Attribution>,
        spec: CommandSpec,
        root: &Path,
        relative: &Path,
        before: InputSnapshot,
    ) -> Result<String, String> {
        if relative.as_os_str().is_empty()
            || !relative.components().all(|component| {
                matches!(
                    component,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
        {
            return Err("Move cwd must be a literal project-relative directory".into());
        }
        let cwd = if relative == Path::new(".") {
            root.to_path_buf()
        } else {
            root.join(relative)
        };
        self.prepare_for_root(
            task_id,
            partner,
            spec,
            cwd,
            before,
            Some(root.to_path_buf()),
        )
    }

    fn prepare_for_root(
        &self,
        task_id: i64,
        partner: Option<crate::store::Attribution>,
        spec: CommandSpec,
        cwd: PathBuf,
        before: InputSnapshot,
        move_root: Option<PathBuf>,
    ) -> Result<String, String> {
        validate_spec(&spec)?;
        if let Some(partner) = &partner {
            partner.validate()?;
        }
        if task_id <= 0 {
            return Err("A saved task is required".into());
        }
        if before
            .fingerprint
            .as_ref()
            .is_some_and(|value| value.len() > 256)
        {
            return Err("Source fingerprint exceeds 256 bytes".into());
        }
        let mut gate = self.gate.lock();
        if gate.active.is_some() {
            return Err("Wait for the owned run to finish".into());
        }
        let request_id = format!(
            "{}-{:x}",
            self.session,
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        // Invalidation happens even when the bounded queue refuses the replacement.
        gate.pending = None;
        self.commands
            .as_ref()
            .ok_or("Runner is stopped")?
            .try_send(Command::Prepare {
                request_id: request_id.clone(),
                task_id,
                spec,
                cwd,
                before,
                partner,
                move_root,
            })
            .map_err(|_| "Runner queue is full or disconnected")?;
        gate.pending = Some(request_id.clone());
        Ok(request_id)
    }
    pub fn invalidate(&self) {
        self.gate.lock().pending = None;
    }
    /// The only dispatch entry point requires a committed, non-forgeable storage permit.
    pub fn dispatch(
        &self,
        request_id: &str,
        permit: crate::store::CommittedRunPermit,
    ) -> Result<(), String> {
        let gate = self.gate.lock();
        if gate.pending.as_deref() != Some(request_id) || gate.active.is_some() {
            return Err("Approval is stale, already used, or another run is active".into());
        }
        if permit.request_id != request_id {
            return Err("Reservation does not match approval".into());
        }
        self.commands
            .as_ref()
            .ok_or("Runner is stopped")?
            .try_send(Command::Dispatch {
                request_id: request_id.into(),
                permit,
            })
            .map_err(|_| "Runner queue is full or disconnected".into())
    }
    pub fn cancel(&self, run_id: &str) -> bool {
        let mut gate = self.gate.lock();
        if gate.active.as_deref() != Some(run_id) {
            return false;
        }
        gate.cancelled = true;
        true
    }
    pub fn is_active(&self) -> bool {
        self.gate.lock().active.is_some()
    }
    pub fn try_event(&self) -> Option<Event> {
        // Lifecycle notifications cannot be starved by a busy output stream.
        let event = self.events.lock().pop_front();
        if matches!(event, Some(Event::Finished(_))) {
            let mut gate = self.gate.lock();
            gate.active = None;
            gate.cancelled = false;
        }
        event.or_else(|| self.output.try_recv().ok().map(Event::Output))
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let mut gate = self.gate.lock();
        gate.pending = None;
        gate.cancelled = true;
        self.commands.take();
        // The worker owns teardown and all native handles; never join it on the render thread.
    }
}

fn publish(events: &Mutex<VecDeque<Event>>, event: Event) {
    let mut queue = events.lock();
    // Only one run can be admitted until its Finished is consumed. Keep its <=5 lifecycle/limit
    // notices; superseded prepare/reject notices may be evicted, never pipe-draining blocked.
    if queue.len() >= EVENT_CAPACITY
        && let Some(index) = queue
            .iter()
            .position(|e| matches!(e, Event::Prepared(_) | Event::Rejected { .. }))
    {
        queue.remove(index);
    }
    queue.push_back(event);
}

fn worker(
    incoming: Receiver<Command>,
    events: Arc<Mutex<VecDeque<Event>>>,
    live: SyncSender<Output>,
    gate: Arc<Mutex<Gate>>,
    stopped: Arc<AtomicBool>,
) {
    let mut pending: Option<Prepared> = None;
    while let Ok(command) = incoming.recv() {
        if stopped.load(Ordering::Acquire) {
            break;
        }
        match command {
            Command::Prepare {
                request_id,
                task_id,
                spec,
                cwd,
                before,
                partner,
                move_root,
            } => {
                let result = (|| {
                    if let Some(root) = move_root.as_deref() {
                        validate_move_cwd(root, &cwd)?;
                    }
                    let mut prepared = prepare(request_id.clone(), task_id, spec, cwd, before)?;
                    if let Some(root) = move_root.as_deref() {
                        validate_move_cwd(root, &prepared.cwd)?;
                    }
                    prepared.partner = partner;
                    Ok(prepared)
                })();
                let current = gate.lock().pending.as_deref() == Some(&request_id);
                if !current {
                    continue;
                }
                match result {
                    Ok(prepared) => {
                        pending = Some(prepared.clone());
                        publish(&events, Event::Prepared(prepared));
                    }
                    Err(message) => {
                        pending = None;
                        let mut g = gate.lock();
                        if g.pending.as_deref() == Some(&request_id) {
                            g.pending = None;
                        }
                        drop(g);
                        publish(
                            &events,
                            Event::Rejected {
                                request_id,
                                task_id,
                                message,
                            },
                        );
                    }
                }
            }
            Command::Dispatch { request_id, permit } => {
                let task_id = permit.task_id;
                let result = pending
                    .as_ref()
                    .filter(|p| p.request_id == request_id)
                    .ok_or_else(|| "Approval has no current prepared request".to_owned())
                    .and_then(|prepared| admit(prepared, &permit, &gate, &stopped));
                match result {
                    Ok(process) => {
                        let mut prepared = pending.take().expect("admitted prepared request");
                        prepared.rerun_of = permit.rerun_of;
                        let mut record = RunRecord::awaiting(&prepared);
                        record.revision_moving = permit.revision_moving;
                        run(
                            process,
                            record,
                            (permit.file, permit.capacity),
                            &events,
                            &live,
                            &gate,
                            &stopped,
                        );
                    }
                    Err(message) => {
                        publish(
                            &events,
                            Event::Rejected {
                                request_id,
                                task_id,
                                message,
                            },
                        );
                    }
                }
            }
        }
    }
}

fn validate_move_cwd(root: &Path, cwd: &Path) -> Result<(), String> {
    let relative = cwd
        .strip_prefix(root)
        .map_err(|_| "Move cwd is outside the project root")?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err("Move cwd has a nonliteral component".into());
        }
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("Move cwd is unavailable: {error}"))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("Move cwd must not traverse linked directories".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("Move cwd must not traverse reparse points".into());
            }
        }
    }
    if fs::canonicalize(cwd).map_err(|error| format!("Move cwd: {error}"))? != cwd {
        return Err("Move cwd is not the exact contained directory".into());
    }
    Ok(())
}

fn unique_id() -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "r{:x}-{:x}-{:x}",
        std::process::id(),
        time,
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn validate_spec(spec: &CommandSpec) -> Result<(), String> {
    let (program, bytes, contains_nul, count) = match spec {
        CommandSpec::Direct { program, args } => (
            program,
            args.iter()
                .fold(program.len(), |n, a| n.saturating_add(a.len() + 1)),
            args.iter().any(|a| a.contains('\0')),
            args.len(),
        ),
        CommandSpec::Shell {
            executable,
            dialect,
            text,
        } => {
            if cfg!(unix) && *dialect == ShellDialect::Cmd {
                return Err("cmd dialect is only supported on Windows".into());
            }
            (
                executable,
                executable.len().saturating_add(text.len()),
                text.contains('\0'),
                1,
            )
        }
    };
    if program.is_empty()
        || program.contains('\0')
        || contains_nul
        || bytes > SPEC_LIMIT
        || count > 4096
    {
        return Err(
            "Command is empty, contains NUL, or exceeds the 64-KiB/4096-argument limit".into(),
        );
    }
    Ok(())
}
fn prepare(
    request_id: String,
    task_id: i64,
    spec: CommandSpec,
    cwd: PathBuf,
    before: InputSnapshot,
) -> Result<Prepared, String> {
    validate_spec(&spec)?;
    let cwd = fs::canonicalize(cwd).map_err(|e| format!("Working directory: {e}"))?;
    if cwd.to_str().is_none() || !cwd.is_dir() {
        return Err("Working directory must be a Unicode directory".into());
    }
    let program = match &spec {
        CommandSpec::Direct { program, .. } => program,
        CommandSpec::Shell { executable, .. } => executable,
    };
    let path = resolve_executable(program, &cwd)?;
    #[cfg(windows)]
    if path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("bat") || s.eq_ignore_ascii_case("cmd"))
    {
        return Err(
            "Windows batch files require an explicit cmd.exe shell route with displayed text"
                .into(),
        );
    }
    let executable = executable_identity(&path).map_err(|e| format!("Executable identity: {e}"))?;
    let cwd_identity =
        native::directory_identity(&cwd).map_err(|e| format!("Working directory identity: {e}"))?;
    let prepared_at_ms = now_ms();
    let prepared = Prepared {
        run_id: format!("{request_id}-run"),
        request_id,
        task_id,
        spec,
        cwd,
        cwd_identity,
        executable,
        prepared_at_ms,
        expires_at_ms: prepared_at_ms.saturating_add(APPROVAL_LIFETIME_MS),
        approval_arrival: crate::providers::approval_arrival(),
        before,
        partner: None,
        rerun_of: None,
    };
    if serde_json::to_vec(&prepared)
        .map_err(|e| e.to_string())?
        .len()
        > 96 * 1024
    {
        return Err("Prepared command metadata exceeds 96 KiB".into());
    }
    Ok(prepared)
}
pub(crate) fn resolve_executable(program: &str, cwd: &Path) -> Result<PathBuf, String> {
    let value = Path::new(program);
    let explicit = value.is_absolute()
        || value.components().count() > 1
        || program.contains(std::path::MAIN_SEPARATOR);
    let candidates = if explicit {
        vec![if value.is_absolute() {
            value.to_path_buf()
        } else {
            cwd.join(value)
        }]
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|p| {
                if p.is_absolute() {
                    p.join(value)
                } else {
                    cwd.join(p).join(value)
                }
            })
            .collect()
    };
    for candidate in candidates {
        #[cfg(windows)]
        let candidates = if candidate.extension().is_none() {
            vec![candidate.with_extension("exe"), candidate]
        } else {
            vec![candidate]
        };
        #[cfg(not(windows))]
        let candidates = [candidate];
        for candidate in candidates {
            if let Ok(path) = fs::canonicalize(candidate) {
                if path.to_str().is_none() {
                    return Err("Executable path is not Unicode".into());
                }
                if native::is_executable(&path) {
                    return Ok(path);
                }
            }
        }
    }
    Err(format!("Executable not found or not executable: {program}"))
}
pub(crate) fn executable_identity(path: &Path) -> io::Result<ExecutableIdentity> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A replaced executable must fail closed, not block the admission worker on
        // a FIFO/device or follow a newly substituted link before cancellation can run.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(path)?;
    let before = file.metadata()?;
    if !before.is_file() || before.len() > 512 * 1024 * 1024 {
        return Err(io::Error::other(
            "Executable must be a regular file no larger than 512 MiB",
        ));
    }
    let file_id = native::file_identity(&file)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > 512 * 1024 * 1024 {
            return Err(io::Error::other("Executable changed while reading"));
        }
        hash.update(&buffer[..count]);
    }
    let after = file.metadata()?;
    if total != before.len()
        || before.len() != after.len()
        || before.modified()? != after.modified()?
    {
        return Err(io::Error::other("Executable changed while reading"));
    }
    Ok(ExecutableIdentity {
        path: path.into(),
        sha256: format!("{:x}", hash.finalize()),
        bytes: total,
        modified_ns: after
            .modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        file_id,
    })
}
fn admit(
    prepared: &Prepared,
    permit: &crate::store::CommittedRunPermit,
    gate: &Mutex<Gate>,
    stopped: &AtomicBool,
) -> Result<native::Process, String> {
    if permit.request_id != prepared.request_id
        || permit.run_id != prepared.run_id
        || permit.task_id != prepared.task_id
        || permit.capacity == 0
    {
        return Err("Committed reservation does not match this command".into());
    }
    let metadata = permit
        .file
        .metadata()
        .map_err(|e| format!("Output reservation: {e}"))?;
    if !metadata.is_file() || metadata.len() != 0 {
        return Err("Output reservation is not an empty regular file".into());
    }
    if now_ms() > prepared.expires_at_ms {
        return Err("Approval expired; prepare the command again".into());
    }
    if fs::canonicalize(&prepared.cwd).map_err(|e| e.to_string())? != prepared.cwd
        || native::directory_identity(&prepared.cwd).map_err(|e| e.to_string())?
            != prepared.cwd_identity
        || fs::canonicalize(&prepared.executable.path).map_err(|e| e.to_string())?
            != prepared.executable.path
        || executable_identity(&prepared.executable.path).map_err(|e| e.to_string())?
            != prepared.executable
    {
        return Err("Working directory or executable changed since preview; prepare again".into());
    }
    // This is the one-shot dispatch linearization point, after asynchronous revalidation.
    // Do not hold the UI's gate across an OS launch: a slow loader must not freeze cancellation.
    {
        let mut state = gate.lock();
        if stopped.load(Ordering::Acquire)
            || now_ms() > prepared.expires_at_ms
            || state.pending.as_deref() != Some(&prepared.request_id)
            || state.active.is_some()
        {
            return Err("Approval was invalidated or already used".into());
        }
        state.pending = None; // A failed launch also consumes this approval.
        state.active = Some(prepared.run_id.clone());
        state.cancelled = false;
    }
    match native::Process::spawn(prepared) {
        Ok(process) => Ok(process),
        Err(error) => {
            let mut state = gate.lock();
            state.active = None;
            state.cancelled = false;
            Err(format!("Owned command launch refused: {error}"))
        }
    }
}
fn arguments(spec: &CommandSpec) -> impl Iterator<Item = &str> {
    let (direct, prefix, text): (&[String], &[&str], Option<&str>) = match spec {
        CommandSpec::Direct { args, .. } => (args, &[], None),
        CommandSpec::Shell { dialect, text, .. } => {
            let prefix: &[&str] = match dialect {
                ShellDialect::Posix => &["-c"],
                ShellDialect::Cmd => &["/d", "/s", "/c"],
                ShellDialect::PowerShell => {
                    &["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"]
                }
            };
            (&[], prefix, Some(text))
        }
    };
    direct
        .iter()
        .map(String::as_str)
        .chain(prefix.iter().copied())
        .chain(text)
}

fn run(
    mut process: native::Process,
    record: RunRecord,
    spool: (File, u64),
    events: &Mutex<VecDeque<Event>>,
    live: &SyncSender<Output>,
    gate: &Mutex<Gate>,
    stopped: &AtomicBool,
) {
    let (file, capacity) = spool;
    let mut capture = Capture {
        file,
        capacity,
        record,
        live,
        events,
    };
    capture.record.started_at_ms = Some(now_ms());
    capture.record.state = RunState::Running;
    capture.record.tree = TreeOutcome::OwnershipUnknown;
    publish(events, Event::Started(capture.record.clone()));
    let mut stop_at = None;
    let mut forced = false;
    let mut stdout = Sanitizer::default();
    let mut stderr = Sanitizer::default();
    let mut eof = [false; 2];
    let mut status = None;
    let mut buffer = [0u8; 8192];
    loop {
        let cancelled = stopped.load(Ordering::Acquire) || gate.lock().cancelled;
        if cancelled && !capture.record.cancelled {
            capture.record.cancelled = true;
            capture.record.state = RunState::Cancelling;
            // At most one cancelling event per run; lifecycle channel has room independently of output.
            publish(events, Event::Cancelling(capture.record.clone()));
            if stop_at.is_none() {
                stop_at = Some(Instant::now());
            }
            if let Err(error) = process.terminate(false) {
                capture.error(format!("Cancellation: {error}"));
            }
        }
        for (index, sanitizer, stream) in [
            (0, &mut stdout, OutputStream::Stdout),
            (1, &mut stderr, OutputStream::Stderr),
        ] {
            // A hot pipe cannot starve the other pipe, cancellation, or lifecycle observation.
            for _ in 0..8 {
                if eof[index] {
                    break;
                }
                match process.read(index, &mut buffer) {
                    Ok(0) => {
                        eof[index] = true;
                        sanitizer.finish(&mut capture, stream);
                        break;
                    }
                    Ok(count) => sanitizer.push(&buffer[..count], &mut capture, stream),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        capture.error(format!("Output pipe: {error}"));
                        capture.record.truncated = true;
                        eof[index] = true;
                        sanitizer.finish(&mut capture, stream);
                        break;
                    }
                }
            }
        }
        if status.is_none() {
            match process.try_wait() {
                Ok(value) => status = value,
                Err(error) => {
                    capture.error(format!("Process outcome: {error}"));
                    if stop_at.is_none() {
                        stop_at = Some(Instant::now());
                        let _ = process.terminate(false);
                    }
                }
            }
        }
        // A normally exited root is not permission to leave its owned descendants running.
        if status.is_some() && stop_at.is_none() {
            stop_at = Some(Instant::now());
            if let Err(error) = process.terminate(false) {
                capture.error(format!("Descendant teardown: {error}"));
            }
        }
        let tree_empty = process.tree_empty();
        if status.is_some() && tree_empty.as_ref().is_ok_and(|empty| *empty) {
            capture.record.tree = TreeOutcome::OwnedTreeExited;
            if eof.iter().all(|value| *value) {
                break;
            }
        }
        if let Some(start) = stop_at {
            if start.elapsed() >= SOFT_STOP && !forced {
                forced = true;
                if let Err(error) = process.terminate(true) {
                    capture.error(format!("Forced teardown: {error}"));
                }
            }
            if start.elapsed() >= HARD_STOP {
                if !eof.iter().all(|value| *value) {
                    capture.record.truncated = true;
                }
                if !tree_empty.unwrap_or(false)
                    || status.is_none()
                    || !eof.iter().all(|value| *value)
                {
                    capture.record.tree = TreeOutcome::OwnershipUnknown;
                    capture.error("Owned-tree/output closure could not be established within the cancellation deadline; effects may remain unknown".into());
                }
                break;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    stdout.finish(&mut capture, OutputStream::Stdout);
    stderr.finish(&mut capture, OutputStream::Stderr);
    if let Some((code, signal)) = status {
        capture.record.exit_code = code;
        capture.record.signal = signal;
    }
    capture.record.ended_at_ms = Some(now_ms());
    capture.record.state =
        if capture.record.tree != TreeOutcome::OwnedTreeExited || status.is_none() {
            RunState::OutcomeUnknown
        } else if capture.record.cancelled {
            RunState::Cancelled
        } else if capture.record.exit_code == Some(0) {
            RunState::Succeeded
        } else {
            RunState::Failed
        };
    if let Err(error) = capture.file.sync_all() {
        capture.record.output_unavailable = true;
        capture.record.truncated = true;
        capture.error(format!("Persisting captured output: {error}"));
    }
    // Native RAII does the final kill-on-drop before Finished can authorize task switching.
    drop(process);
    publish(events, Event::Finished(capture.record));
}

struct Capture<'a> {
    file: File,
    capacity: u64,
    record: RunRecord,
    live: &'a SyncSender<Output>,
    events: &'a Mutex<VecDeque<Event>>,
}
impl Capture<'_> {
    fn error(&mut self, error: String) {
        if self.record.error.is_none() {
            self.record.error = Some(crate::workspace::privacy::redact(&error).into_owned());
        }
    }
    fn discard(&mut self, count: usize) {
        let first = !self.record.truncated;
        self.record.truncated = true;
        self.record.discarded_bytes = self.record.discarded_bytes.saturating_add(count as u64);
        if first {
            publish(self.events, Event::CaptureChanged(self.record.clone()));
        }
    }
    fn emit(&mut self, text: &str, stream: OutputStream) {
        if text.is_empty() {
            return;
        }
        let available = self
            .capacity
            .saturating_sub(self.record.captured_bytes)
            .min(usize::MAX as u64) as usize;
        let mut limit = text.len().min(available);
        while !text.is_char_boundary(limit) {
            limit -= 1;
        }
        if self.record.output_unavailable {
            limit = 0;
        }
        let mut written = 0;
        while written < limit {
            match self.file.write(&text.as_bytes()[written..limit]) {
                Ok(0) => {
                    self.record.output_unavailable = true;
                    self.error("Captured output write made no progress".into());
                    break;
                }
                Ok(count) => written += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    self.record.output_unavailable = true;
                    self.error(format!("Captured output write failed: {error}"));
                    break;
                }
            }
        }
        self.record.captured_bytes = self.record.captured_bytes.saturating_add(written as u64);
        if written < text.len() {
            self.discard(text.len() - written);
        }
        // Live output follows the same redacted capture boundary, never raw child bytes.
        let mut visible = written;
        while !text.is_char_boundary(visible) {
            visible -= 1;
        }
        if visible > 0 {
            let output = Output {
                request_id: self.record.prepared.request_id.clone(),
                run_id: self.record.prepared.run_id.clone(),
                task_id: self.record.prepared.task_id,
                stream,
                at_ms: now_ms(),
                text: text[..visible].into(),
            };
            if self.live.try_send(output).is_err() {
                let first = self.record.dropped_live_bytes == 0;
                self.record.dropped_live_bytes = self
                    .record
                    .dropped_live_bytes
                    .saturating_add(visible as u64);
                if first {
                    publish(self.events, Event::CaptureChanged(self.record.clone()));
                }
            }
        }
    }
}
#[derive(Default)]
struct Sanitizer {
    line: Vec<u8>,
    suppress: bool,
    private_key: bool,
    quoted_secret: Option<char>,
    escape: Escape,
}
#[derive(Default)]
enum Escape {
    #[default]
    Text,
    Start,
    Csi,
    Osc,
    OscTerminator,
}
impl Sanitizer {
    fn push(&mut self, bytes: &[u8], capture: &mut Capture<'_>, stream: OutputStream) {
        for &byte in bytes {
            if self.suppress {
                capture.discard(1);
                continue;
            }
            if self.line.len() >= LINE_LIMIT {
                // Never emit a prefix whose credential/PEM meaning depends on a discarded suffix.
                capture.discard(self.line.len() + 1);
                self.line.clear();
                self.suppress = true;
                continue;
            }
            self.line.push(byte);
            if byte == b'\n' {
                self.flush(capture, stream);
            }
        }
    }
    fn finish(&mut self, capture: &mut Capture<'_>, stream: OutputStream) {
        if !self.line.is_empty() {
            self.flush(capture, stream);
        }
    }
    fn flush(&mut self, capture: &mut Capture<'_>, stream: OutputStream) {
        let Ok(text) = std::str::from_utf8(&self.line) else {
            capture.discard(self.line.len());
            self.line.clear();
            self.suppress = true;
            return;
        };
        let mut clean = String::with_capacity(text.len());
        for ch in text.chars() {
            match self.escape {
                Escape::Text => match ch {
                    '\u{1b}' => self.escape = Escape::Start,
                    '\u{9b}' => self.escape = Escape::Csi,
                    '\u{9d}' => self.escape = Escape::Osc,
                    '\n' | '\t' => clean.push(ch),
                    c if c.is_control()
                        || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') => {}
                    c => clean.push(c),
                },
                Escape::Start => {
                    self.escape = match ch {
                        '[' => Escape::Csi,
                        ']' | 'P' | '^' | '_' => Escape::Osc,
                        _ => Escape::Text,
                    }
                }
                Escape::Csi => {
                    if ('@'..='~').contains(&ch) {
                        self.escape = Escape::Text;
                    }
                }
                Escape::Osc => {
                    if ch == '\u{7}' || ch == '\u{9c}' {
                        self.escape = Escape::Text;
                    } else if ch == '\u{1b}' {
                        self.escape = Escape::OscTerminator;
                    }
                }
                Escape::OscTerminator => {
                    self.escape = if ch == '\\' {
                        Escape::Text
                    } else {
                        Escape::Osc
                    }
                }
            }
        }
        self.line.clear();
        if self.private_key {
            if clean.contains("-----END ") && clean.contains("PRIVATE KEY-----") {
                self.private_key = false;
            }
            return;
        }
        if let Some(quote) = self.quoted_secret {
            if clean.contains(quote) {
                self.quoted_secret = None;
            }
            return;
        }
        if let Some(start) = clean.find("-----BEGIN ") {
            let rest = &clean[start..];
            if rest.contains("PRIVATE KEY-----") {
                self.private_key = !rest.contains("-----END ");
            }
        }
        let redacted = crate::workspace::privacy::redact(&clean);
        if redacted.as_ref() != clean {
            // Conservative multiline quoted-value handling; deliberately over-redact rather than
            // persist the unrecognizable continuation of a recognizable credential assignment.
            for quote in ['\'', '"'] {
                if clean.chars().filter(|c| *c == quote).count() % 2 != 0 {
                    self.quoted_secret = Some(quote);
                }
            }
        }
        capture.emit(&redacted, stream);
    }
}

#[cfg(unix)]
pub(crate) mod native {
    use super::*;
    use std::io::{PipeReader, PipeWriter};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Child, Command, Stdio};

    pub fn is_executable(path: &Path) -> bool {
        fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    pub fn file_identity(file: &File) -> io::Result<String> {
        let m = file.metadata()?;
        Ok(format!("{}:{}", m.dev(), m.ino()))
    }
    pub fn directory_identity(path: &Path) -> io::Result<String> {
        file_identity(&File::open(path)?)
    }
    pub struct Process {
        child: Option<Child>,
        stdout: PipeReader,
        stderr: PipeReader,
        stdin: Option<PipeWriter>,
        group: libc::pid_t,
        gone: bool,
    }
    impl Process {
        pub fn spawn(prepared: &Prepared) -> io::Result<Self> {
            Self::spawn_inner(prepared, false, &[])
        }
        /// Owned bidirectional transport; stdin writes are nonblocking, like capture reads.
        pub fn spawn_piped(prepared: &Prepared) -> io::Result<Self> {
            Self::spawn_inner(prepared, true, &[])
        }
        /// Like `spawn_piped` with extra environment variables applied to the
        /// child only; values are not recorded in the reviewed spec.
        pub fn spawn_piped_env(prepared: &Prepared, env: &[(String, String)]) -> io::Result<Self> {
            Self::spawn_inner(prepared, true, env)
        }
        fn spawn_inner(
            prepared: &Prepared,
            piped: bool,
            env: &[(String, String)],
        ) -> io::Result<Self> {
            let (stdout, stdout_writer) = io::pipe()?;
            let (stderr, stderr_writer) = io::pipe()?;
            let (stdin_reader, stdin) = if piped {
                let (reader, writer) = io::pipe()?;
                let fd = writer.as_raw_fd();
                let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                if flags < 0
                    || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
                {
                    return Err(io::Error::last_os_error());
                }
                (Stdio::from(reader), Some(writer))
            } else {
                (Stdio::null(), None)
            };
            // Establish all fallible capture setup before exec, so a setup refusal cannot hide
            // a command that already ran. Only read ends are nonblocking; child writes may block.
            for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
                let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                if flags < 0
                    || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
                {
                    return Err(io::Error::last_os_error());
                }
            }
            let mut command = Command::new(&prepared.executable.path);
            command
                .args(arguments(&prepared.spec))
                .current_dir(&prepared.cwd)
                .stdin(stdin_reader)
                .stdout(stdout_writer)
                .stderr(stderr_writer)
                .process_group(0);
            if !env.is_empty() {
                command.envs(
                    env.iter()
                        .map(|(key, value)| (key.as_str(), value.as_str())),
                );
            }
            // std establishes setpgid(0,0) in the child before exec; spawn reports failures.
            let child = command.spawn()?;
            let group = child.id() as libc::pid_t;
            drop(command); // Close the parent's writer copies so EOF remains observable.
            Ok(Self {
                child: Some(child),
                stdout,
                stderr,
                stdin,
                group,
                gone: false,
            })
        }
        pub fn write_stdin(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.stdin
                .as_mut()
                .ok_or(io::ErrorKind::BrokenPipe)?
                .write(bytes)
        }
        pub fn close_stdin(&mut self) {
            self.stdin.take();
        }
        pub fn read(&mut self, index: usize, bytes: &mut [u8]) -> io::Result<usize> {
            if index == 0 {
                self.stdout.read(bytes)
            } else {
                self.stderr.read(bytes)
            }
        }
        pub fn try_wait(&mut self) -> io::Result<Option<(Option<i32>, Option<i32>)>> {
            self.child
                .as_mut()
                .expect("owned child")
                .try_wait()
                .map(|s| s.map(|s| (s.code(), s.signal())))
        }
        pub fn terminate(&mut self, force: bool) -> io::Result<()> {
            if self.gone {
                return Ok(());
            }
            let result = unsafe {
                libc::kill(
                    -self.group,
                    if force { libc::SIGKILL } else { libc::SIGTERM },
                )
            };
            if result == 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                self.gone = true;
                Ok(())
            } else {
                Err(error)
            }
        }
        pub fn tree_empty(&mut self) -> io::Result<bool> {
            if self.gone {
                return Ok(true);
            }
            if unsafe { libc::kill(-self.group, 0) } == 0 {
                return Ok(false);
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                self.gone = true;
                Ok(true)
            } else {
                Err(error)
            }
        }
    }
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.terminate(true);
            // A stuck child cannot hold up cancellation forever; retain a reaper for its eventual
            // wait status instead of losing the Child and leaking a zombie while Eevee remains open.
            if let Some(mut child) = self.child.take()
                && matches!(child.try_wait(), Ok(None))
            {
                let _ = child.kill();
                let deadline = Instant::now() + Duration::from_millis(100);
                while Instant::now() < deadline {
                    if !matches!(child.try_wait(), Ok(None)) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                let _ = thread::Builder::new()
                    .name("ev-run-reaper".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
            }
        }
    }
}

#[cfg(windows)]
pub(crate) mod native {
    use super::*;
    use std::mem::{size_of, size_of_val};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::Storage::FileSystem::*;
    use windows_sys::Win32::System::JobObjects::*;
    use windows_sys::Win32::System::Pipes::*;
    use windows_sys::Win32::System::Threading::*;

    fn wide(path: &std::ffi::OsStr) -> Vec<u16> {
        path.encode_wide().chain(Some(0)).collect()
    }
    fn handle(raw: HANDLE) -> io::Result<OwnedHandle> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
        }
    }
    pub fn is_executable(path: &Path) -> bool {
        path.is_file()
    }
    pub fn file_identity(file: &File) -> io::Result<String> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::other("Reparse identity refused"));
        }
        Ok(format!(
            "{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ))
    }
    pub fn directory_identity(path: &Path) -> io::Result<String> {
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        file_identity(&file)
    }

    /// Merges overrides into the parent environment as the sorted,
    /// NUL-delimited UTF-16 block `CreateProcessW` expects. Names are compared
    /// case-insensitively like the Win32 resolver; embedded NULs are refused.
    fn environment_block(overrides: &[(String, String)]) -> io::Result<Vec<u16>> {
        let mut merged: Vec<(String, String)> = std::env::vars()
            .filter(|(name, _)| {
                !name.is_empty()
                    && !name.contains('=')
                    && !overrides
                        .iter()
                        .any(|(key, _)| key.eq_ignore_ascii_case(name))
            })
            .collect();
        for (name, value) in overrides {
            if name.is_empty()
                || name.contains('=')
                || [name, value]
                    .iter()
                    .any(|part| part.contains('\0') || part.contains('='))
            {
                return Err(io::Error::other("Invalid environment override"));
            }
            merged.push((name.clone(), value.clone()));
        }
        merged.sort_by(|a, b| a.0.to_uppercase().cmp(&b.0.to_uppercase()));
        let mut block = Vec::new();
        for (name, value) in merged {
            block.extend(wide(std::ffi::OsStr::new(&format!("{name}={value}"))));
            block.push(0);
        }
        block.push(0);
        Ok(block)
    }

    fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
        let mut read = null_mut();
        let mut write = null_mut();
        let security = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        if unsafe { CreatePipe(&mut read, &mut write, &security, 65536) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let read = handle(read)?;
        let write = handle(write)?;
        if unsafe { SetHandleInformation(read.as_raw_handle(), HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((read, write))
    }
    // Microsoft CRT argv encoding, NOT shell escaping. The executable path is separately supplied.
    fn quote(arg: &str) -> String {
        let mut result = String::from("\"");
        let mut slashes = 0;
        for ch in arg.chars() {
            if ch == '\\' {
                slashes += 1;
                continue;
            }
            if ch == '"' {
                result.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
            } else {
                result.extend(std::iter::repeat_n('\\', slashes));
            }
            slashes = 0;
            result.push(ch);
        }
        result.extend(std::iter::repeat_n('\\', slashes * 2));
        result.push('"');
        result
    }
    struct Attributes {
        memory: Vec<usize>,
        list: LPPROC_THREAD_ATTRIBUTE_LIST,
    }
    impl Attributes {
        fn new(handles: &[HANDLE]) -> io::Result<Self> {
            let mut bytes = 0;
            unsafe {
                InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes);
            }
            if bytes == 0 || bytes > 65536 {
                return Err(io::Error::last_os_error());
            }
            let mut memory = vec![0usize; bytes.div_ceil(size_of::<usize>())];
            let list = memory.as_mut_ptr().cast();
            if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut bytes) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let result = Self { memory, list };
            if unsafe {
                UpdateProcThreadAttribute(
                    result.list,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    handles.as_ptr().cast(),
                    size_of_val(handles),
                    null_mut(),
                    null(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(result)
        }
    }
    impl Drop for Attributes {
        fn drop(&mut self) {
            unsafe {
                DeleteProcThreadAttributeList(self.list);
            }
            // Allocation must outlive DeleteProcThreadAttributeList.
            let _ = &self.memory;
        }
    }
    pub struct Process {
        process: OwnedHandle,
        job: OwnedHandle,
        stdout: OwnedHandle,
        stderr: OwnedHandle,
        stdin: Option<OwnedHandle>,
    }
    impl Process {
        pub fn spawn(prepared: &Prepared) -> io::Result<Self> {
            Self::spawn_inner(prepared, false, &[])
        }
        /// Keep the same suspended-before-Job admission as local commands.
        pub fn spawn_piped(prepared: &Prepared) -> io::Result<Self> {
            Self::spawn_inner(prepared, true, &[])
        }
        /// Like `spawn_piped` with extra environment variables applied to the
        /// child only; values are not recorded in the reviewed spec.
        pub fn spawn_piped_env(prepared: &Prepared, env: &[(String, String)]) -> io::Result<Self> {
            Self::spawn_inner(prepared, true, env)
        }
        fn spawn_inner(
            prepared: &Prepared,
            piped: bool,
            env: &[(String, String)],
        ) -> io::Result<Self> {
            let job = handle(unsafe { CreateJobObjectW(null(), null()) })?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if unsafe {
                SetInformationJobObject(
                    job.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    size_of_val(&limits) as u32,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            let (stdout, stdout_write) = pipe()?;
            let (stderr, stderr_write) = pipe()?;
            let security = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: null_mut(),
                bInheritHandle: 1,
            };
            let (stdin, stdin_write) = if piped {
                // Named byte pipe with nonblocking parent writes. Do not rely on undocumented
                // mode changes to CreatePipe's anonymous handles; cancellation must stay bounded.
                let name = wide(std::ffi::OsStr::new(&format!(
                    r"\\.\pipe\eevee-stdin-{}",
                    unique_id()
                )));
                let write = handle(unsafe {
                    CreateNamedPipeW(
                        name.as_ptr(),
                        PIPE_ACCESS_OUTBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                        PIPE_TYPE_BYTE
                            | PIPE_READMODE_BYTE
                            | PIPE_NOWAIT
                            | PIPE_REJECT_REMOTE_CLIENTS,
                        1,
                        65536,
                        65536,
                        0,
                        null(),
                    )
                })?;
                let read = handle(unsafe {
                    CreateFileW(
                        name.as_ptr(),
                        GENERIC_READ,
                        0,
                        &security,
                        OPEN_EXISTING,
                        FILE_ATTRIBUTE_NORMAL,
                        null_mut(),
                    )
                })?;
                if unsafe { ConnectNamedPipe(write.as_raw_handle(), null_mut()) } == 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) {
                        return Err(error);
                    }
                }
                (read, Some(write))
            } else {
                let nul = wide(std::ffi::OsStr::new("NUL"));
                let read = handle(unsafe {
                    CreateFileW(
                        nul.as_ptr(),
                        GENERIC_READ,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        &security,
                        OPEN_EXISTING,
                        FILE_ATTRIBUTE_NORMAL,
                        null_mut(),
                    )
                })?;
                (read, None)
            };
            let inherited = [
                stdin.as_raw_handle(),
                stdout_write.as_raw_handle(),
                stderr_write.as_raw_handle(),
            ];
            let attributes = Attributes::new(&inherited)?;
            let mut startup = STARTUPINFOEXW::default();
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdInput = inherited[0];
            startup.StartupInfo.hStdOutput = inherited[1];
            startup.StartupInfo.hStdError = inherited[2];
            startup.lpAttributeList = attributes.list;
            let executable = wide(prepared.executable.path.as_os_str());
            let cwd = wide(prepared.cwd.as_os_str());
            let mut line = quote(
                prepared
                    .executable
                    .path
                    .to_str()
                    .ok_or_else(|| io::Error::other("Non-Unicode executable"))?,
            );
            if let CommandSpec::Shell {
                dialect: ShellDialect::Cmd,
                text,
                ..
            } = &prepared.spec
            {
                // cmd's /s /c outer quotes are shell syntax; preserve exact reviewed inner text.
                line.push_str(" /d /s /c \"");
                line.push_str(text);
                line.push('"');
            } else {
                for arg in arguments(&prepared.spec) {
                    line.push(' ');
                    line.push_str(&quote(arg));
                }
            }
            let mut line = wide(std::ffi::OsStr::new(&line));
            if line.len() > 32767 {
                return Err(io::Error::other(
                    "Windows command line exceeds 32767 UTF-16 units",
                ));
            }
            let mut info = PROCESS_INFORMATION::default();
            // Child-only environment block when overrides were supplied;
            // CREATE_UNICODE_ENVIRONMENT pairs with the UTF-16 encoding.
            let environment = if env.is_empty() {
                None
            } else {
                Some(environment_block(env)?)
            };
            let creation = CREATE_SUSPENDED
                | CREATE_NO_WINDOW
                | EXTENDED_STARTUPINFO_PRESENT
                | if environment.is_some() {
                    CREATE_UNICODE_ENVIRONMENT
                } else {
                    0
                };
            if unsafe {
                CreateProcessW(
                    executable.as_ptr(),
                    line.as_mut_ptr(),
                    null(),
                    null(),
                    1,
                    creation,
                    environment
                        .as_ref()
                        .map_or(null(), |block| block.as_ptr().cast()),
                    cwd.as_ptr(),
                    &startup.StartupInfo,
                    &mut info,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            let process = handle(info.hProcess)?;
            let thread = handle(info.hThread)?;
            // Never resume executable code unless the kill-on-close Job already owns the process.
            if unsafe { AssignProcessToJobObject(job.as_raw_handle(), process.as_raw_handle()) }
                == 0
            {
                let error = io::Error::last_os_error();
                unsafe {
                    TerminateProcess(process.as_raw_handle(), 1);
                    WaitForSingleObject(process.as_raw_handle(), 1000);
                }
                return Err(error);
            }
            let result = Self {
                process,
                job,
                stdout,
                stderr,
                stdin: stdin_write,
            };
            if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            Ok(result)
        }
        pub fn write_stdin(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let pipe = self.stdin.as_ref().ok_or(io::ErrorKind::BrokenPipe)?;
            let mut written = 0;
            if unsafe {
                WriteFile(
                    pipe.as_raw_handle(),
                    bytes.as_ptr(),
                    bytes.len().min(u32::MAX as usize) as u32,
                    &mut written,
                    null_mut(),
                )
            } == 0
            {
                let error = io::Error::last_os_error();
                return if error.raw_os_error() == Some(ERROR_NO_DATA as i32) {
                    Err(io::ErrorKind::WouldBlock.into())
                } else {
                    Err(error)
                };
            }
            if written == 0 && !bytes.is_empty() {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            Ok(written as usize)
        }
        pub fn close_stdin(&mut self) {
            self.stdin.take();
        }
        pub fn read(&mut self, index: usize, bytes: &mut [u8]) -> io::Result<usize> {
            let pipe = if index == 0 {
                &self.stdout
            } else {
                &self.stderr
            };
            let mut available = 0;
            if unsafe {
                PeekNamedPipe(
                    pipe.as_raw_handle(),
                    null_mut(),
                    0,
                    null_mut(),
                    &mut available,
                    null_mut(),
                )
            } == 0
            {
                let error = io::Error::last_os_error();
                return if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
                    Ok(0)
                } else {
                    Err(error)
                };
            }
            if available == 0 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let mut read = 0;
            if unsafe {
                ReadFile(
                    pipe.as_raw_handle(),
                    bytes.as_mut_ptr(),
                    available.min(bytes.len() as u32),
                    &mut read,
                    null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(read as usize)
        }
        pub fn try_wait(&mut self) -> io::Result<Option<(Option<i32>, Option<i32>)>> {
            match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
                WAIT_TIMEOUT => Ok(None),
                WAIT_OBJECT_0 => {
                    let mut code = 0;
                    if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) } == 0 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(Some((Some(code as i32), None)))
                }
                _ => Err(io::Error::last_os_error()),
            }
        }
        pub fn terminate(&mut self, _force: bool) -> io::Result<()> {
            if unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) } == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
        pub fn tree_empty(&mut self) -> io::Result<bool> {
            let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            if unsafe {
                QueryInformationJobObject(
                    self.job.as_raw_handle(),
                    JobObjectBasicAccountingInformation,
                    (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    size_of_val(&info) as u32,
                    null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(info.ActiveProcesses == 0)
        }
    }
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.terminate(true);
        }
    }
}

#[cfg(test)]
mod tests;
