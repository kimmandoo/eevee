//! Read-only, session-local repository evidence. No task data or repository bytes are written.
//!
//! Git is used only for metadata and raw object reads. In particular, `status` and working-tree
//! `diff` are deliberately not used: they can execute repository-configured clean/process filters.
//! The displayed diff compares raw HEAD bytes with observed working-tree bytes, without filters.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use sha2::{Digest, Sha256};

const COMMAND_CAPACITY: usize = 2;
const EVENT_CAPACITY: usize = 2;
const STDERR_LIMIT: usize = 8192;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const SCAN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryIdentity {
    /// Canonical native paths; never lowercased or lossily normalized.
    pub root: PathBuf,
    pub git_dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectIdentity {
    pub oid: String,
    pub mode: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexEntry {
    pub object: ObjectIdentity,
    /// Nonzero stages preserve unresolved-merge identities instead of inventing a single file.
    pub stage: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileState {
    Regular {
        sha256: [u8; 32],
        bytes: u64,
        /// A NUL-byte heuristic over all captured bytes, not a content-type guarantee.
        binary: bool,
        executable: bool,
    },
    /// Hashes the link text, never the target's contents. Ancestor links are not traversed.
    Symlink {
        target: PathBuf,
        sha256: [u8; 32],
    },
    Missing,
    Unavailable {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    /// Repository-relative, native path. Combine with RepositoryIdentity::root for display only.
    pub path: PathBuf,
    pub tracked: bool,
    pub head: Option<ObjectIdentity>,
    pub index: Vec<IndexEntry>,
    pub state: FileState,
    /// None means raw-byte comparison was unavailable, not clean.
    pub dirty: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct WorkingDiff {
    /// Untrusted text: the UI must sanitize controls before rendering, including file names.
    pub text: String,
    pub truncated: bool,
    /// Partial text may still be present. Never present an unavailable diff as an empty clean one.
    pub unavailable: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub head: Option<String>,
    pub files: BTreeMap<PathBuf, FileIdentity>,
    pub working_diff: WorkingDiff,
    /// Coverage excludes ignored untracked files and submodule contents.
    pub complete: bool,
    /// No race was detected; this does not promise an atomic filesystem snapshot.
    pub stable: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeOrigin {
    PreExisting,
    /// Observed since the baseline. This is never an authorship or agent attribution claim.
    Unattributed,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    MetadataChanged,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub origin: ChangeOrigin,
    pub kind: ChangeKind,
    pub was_dirty: bool,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub repository: RepositoryIdentity,
    /// Immutable for this worker's lifetime. Refresh never rebases or replaces it.
    pub baseline: Arc<Snapshot>,
    pub current: Arc<Snapshot>,
    pub changes: Vec<Change>,
    pub initial: bool,
}

#[derive(Debug)]
pub enum Event {
    Updated(Arc<Report>),
    Unsupported {
        path: PathBuf,
        reason: String,
    },
    /// The previous report and task remain valid; a subsequent refresh can retry.
    Failed {
        operation: &'static str,
        message: String,
    },
}

pub struct Worker {
    commands: Option<SyncSender<()>>,
    events: Option<Receiver<Event>>,
    stopped: Arc<AtomicBool>,
}

impl Worker {
    /// Performs no filesystem or Git I/O on the calling/UI thread. Start and refresh emit events.
    pub fn start(directory: PathBuf) -> io::Result<Self> {
        Self::start_with_limits(directory, Limits::default())
    }

    fn start_with_limits(directory: PathBuf, limits: Limits) -> io::Result<Self> {
        let (commands, incoming) = mpsc::sync_channel(COMMAND_CAPACITY);
        let (outgoing, events) = mpsc::sync_channel(EVENT_CAPACITY);
        let stopped = Arc::new(AtomicBool::new(false));
        let cancellation = Arc::clone(&stopped);
        thread::Builder::new()
            .name("ev-repo".into())
            .spawn(move || {
                let mut state = State::new(directory, limits, cancellation);
                if outgoing.send(state.refresh()).is_err() {
                    return;
                }
                while incoming.recv().is_ok() {
                    if state.stopped.load(Ordering::Relaxed)
                        || outgoing.send(state.refresh()).is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self {
            commands: Some(commands),
            events: Some(events),
            stopped,
        })
    }

    /// False means backpressure/disconnection; the caller may retain a pending refresh intention.
    pub fn refresh(&self) -> bool {
        self.commands
            .as_ref()
            .is_some_and(|sender| sender.try_send(()).is_ok())
    }

    pub fn try_event(&self) -> Option<Event> {
        self.events.as_ref()?.try_recv().ok()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        // Release a blocked event sender. Do not join a filesystem/Git worker on the UI thread.
        self.events.take();
        self.commands.take();
    }
}

#[derive(Clone, Copy)]
struct Limits {
    files: usize,
    inventory_bytes: usize,
    file_bytes: usize,
    total_file_bytes: usize,
    object_bytes: usize,
    diff_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            files: 4096,
            inventory_bytes: 2 * 1024 * 1024,
            file_bytes: 2 * 1024 * 1024,
            total_file_bytes: 32 * 1024 * 1024,
            object_bytes: 32 * 1024 * 1024,
            diff_bytes: 256 * 1024,
        }
    }
}

struct State {
    directory: PathBuf,
    limits: Limits,
    stopped: Arc<AtomicBool>,
    repository: Option<RepositoryIdentity>,
    root_stamp: Option<Stamp>,
    baseline: Option<Arc<Snapshot>>,
}

impl State {
    fn new(directory: PathBuf, limits: Limits, stopped: Arc<AtomicBool>) -> Self {
        Self {
            directory,
            limits,
            stopped,
            repository: None,
            root_stamp: None,
            baseline: None,
        }
    }

    fn refresh(&mut self) -> Event {
        match self.observe() {
            Ok(event) => event,
            Err(message) => Event::Failed {
                operation: if self.baseline.is_some() {
                    "refresh"
                } else {
                    "baseline"
                },
                message,
            },
        }
    }

    fn observe(&mut self) -> Result<Event, String> {
        let deadline = Instant::now() + SCAN_TIMEOUT;
        let directory = fs::canonicalize(&self.directory).map_err(|error| error.to_string())?;
        if !directory.is_dir() {
            return Err("Repository location is not a directory".into());
        }
        let git = Git::new(&directory, &self.stopped, deadline)?;
        let probe = git.run(&["rev-parse", "--is-inside-work-tree"], None, 1024)?;
        if !probe.success || probe.stdout.bytes != b"true\n" {
            if self.baseline.is_some() {
                return Err(format!(
                    "Repository is no longer available: {}",
                    probe.diagnostic()
                ));
            }
            // Do not turn permission/corruption/ownership failures into a non-Git success state.
            let diagnostic = probe.diagnostic();
            if probe.success || diagnostic.contains("not a git repository") {
                return Ok(Event::Unsupported {
                    path: directory,
                    reason: if probe.success {
                        "Bare Git repositories have no supported working tree".into()
                    } else {
                        "Not a Git repository; the local task remains available".into()
                    },
                });
            }
            return Err(diagnostic);
        }
        let root = git.required(&["rev-parse", "--show-toplevel"], None, 64 * 1024)?;
        let git_dir = git.required(&["rev-parse", "--absolute-git-dir"], None, 64 * 1024)?;
        if root.truncated || git_dir.truncated {
            return Err("Canonical repository path exceeded the output bound".into());
        }
        let repository = RepositoryIdentity {
            root: canonical_git_path(&root.bytes)?,
            git_dir: canonical_git_path(&git_dir.bytes)?,
        };
        let root_stamp = Stamp::from(&fs::metadata(&repository.root).map_err(|e| e.to_string())?);
        if self
            .repository
            .as_ref()
            .is_some_and(|old| old != &repository)
            || self
                .root_stamp
                .as_ref()
                .is_some_and(|old| !old.same_node(&root_stamp))
        {
            return Err("Repository identity changed; the original baseline was retained".into());
        }
        let git = Git::new(&repository.root, &self.stopped, deadline)?;
        let snapshot = Arc::new(capture(&repository, &git, self.limits)?);
        let after_root = fs::metadata(&repository.root).map_err(|e| e.to_string())?;
        if !root_stamp.same_node(&Stamp::from(&after_root))
            || fs::canonicalize(&repository.root).map_err(|e| e.to_string())? != repository.root
            || fs::canonicalize(&repository.git_dir).map_err(|e| e.to_string())?
                != repository.git_dir
        {
            return Err("Repository identity changed during capture; baseline retained".into());
        }
        let initial = self.baseline.is_none();
        let baseline = Arc::clone(self.baseline.get_or_insert_with(|| Arc::clone(&snapshot)));
        self.repository = Some(repository.clone());
        self.root_stamp = Some(root_stamp);
        let changes = classify(&baseline, &snapshot, initial);
        Ok(Event::Updated(Arc::new(Report {
            repository,
            baseline,
            current: snapshot,
            changes,
            initial,
        })))
    }
}

/// Re-observe the whole reviewed patch at provider admission, including additions
/// and deletions that are not in the selected source basket.
pub(crate) fn revalidate_review(report: &Report) -> Result<(), String> {
    let stopped = AtomicBool::new(false);
    let git = Git::new(
        &report.repository.root,
        &stopped,
        Instant::now() + SCAN_TIMEOUT,
    )?;
    let current = capture(&report.repository, &git, Limits::default())?;
    if !current.complete
        || !current.stable
        || current.files != report.current.files
        || current.head != report.current.head
        || current.working_diff.text != report.current.working_diff.text
        || current.working_diff.truncated
        || current.working_diff.unavailable.is_some()
    {
        return Err("Review patch changed or coverage is unavailable; refresh repository evidence and prepare a new review.".into());
    }
    Ok(())
}

struct Git<'a> {
    executable: PathBuf,
    cwd: &'a Path,
    stopped: &'a AtomicBool,
    deadline: Instant,
}

impl<'a> Git<'a> {
    fn new(cwd: &'a Path, stopped: &'a AtomicBool, deadline: Instant) -> Result<Self, String> {
        // Do not resolve `git` through a relative PATH entry or a binary inside this project.
        let excluded_root = cwd
            .ancestors()
            .find(|ancestor| fs::symlink_metadata(ancestor.join(".git")).is_ok())
            .unwrap_or(cwd);
        let executable = std::env::var_os("PATH")
            .into_iter()
            .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .filter(|path| path.is_absolute())
            .filter_map(|directory| {
                let candidate = directory.join(if cfg!(windows) { "git.exe" } else { "git" });
                fs::canonicalize(candidate).ok()
            })
            .find(|path| !path.starts_with(excluded_root) && path.is_file())
            .ok_or_else(|| {
                "No Git executable outside the project was found on the absolute PATH".to_string()
            })?;
        Ok(Self {
            executable,
            cwd,
            stopped,
            deadline,
        })
    }

    fn check_deadline(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Relaxed) {
            Err("Repository inspection cancelled".into())
        } else if Instant::now() >= self.deadline {
            Err("Repository inspection exceeded its 30-second deadline".into())
        } else {
            Ok(())
        }
    }

    fn run(
        &self,
        args: &[&str],
        input: Option<Vec<u8>>,
        limit: usize,
    ) -> Result<GitOutput, String> {
        self.check_deadline()?;
        let mut command = Command::new(&self.executable);
        command.current_dir(self.cwd).env_clear();
        // Windows process/runtime setup needs these. No inherited GIT_* or helper configuration.
        for key in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .env("LC_ALL", "C")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            // Git maps /dev/null on Windows too; uppercase NUL fails its access check.
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("GIT_PAGER", "")
            .env("GIT_EXTERNAL_DIFF", "")
            .args([
                "--no-pager",
                "--no-optional-locks",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.hooksPath=",
                "-c",
                "core.pager=",
                "-c",
                "diff.external=",
                "-c",
                "gc.auto=0",
                "-c",
                "maintenance.auto=false",
                "-c",
                "submodule.recurse=false",
                "-c",
                "protocol.allow=never",
                "-c",
                "protocol.file.allow=never",
            ])
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| format!("Cannot start Git: {e}"))?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        // Drain both pipes concurrently even after the capture limit; never wait with a full pipe.
        let out_reader = thread::spawn(move || drain(stdout, limit));
        let err_reader = thread::spawn(move || drain(stderr, STDERR_LIMIT));
        let input_writer = input.map(|bytes| {
            let mut stdin = child.stdin.take().expect("piped stdin");
            thread::spawn(move || stdin.write_all(&bytes))
        });
        let command_deadline = self.deadline.min(Instant::now() + COMMAND_TIMEOUT);
        let mut failure = None;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(error) => {
                    failure = Some(format!("Cannot wait for Git: {error}"));
                    let _ = child.kill();
                    break child.wait().ok();
                }
            }
            if self.stopped.load(Ordering::Relaxed) || Instant::now() >= command_deadline {
                failure = Some("Git inspection cancelled or exceeded its bounded deadline".into());
                let _ = child.kill();
                break child.wait().ok();
            }
            thread::sleep(Duration::from_millis(5));
        };
        let stdout = out_reader.join().map_err(|_| "Git stdout reader failed")?;
        let stderr = err_reader.join().map_err(|_| "Git stderr reader failed")?;
        if let Some(writer) = input_writer {
            match writer.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    failure.get_or_insert_with(|| format!("Git input failed: {error}"));
                }
                Err(_) => {
                    failure.get_or_insert_with(|| "Git input writer failed".into());
                }
            }
        }
        if let Some(message) = failure {
            return Err(format!(
                "{message}; {}",
                String::from_utf8_lossy(&stderr.bytes)
            ));
        }
        if let Some(error) = stdout.error.as_ref().or(stderr.error.as_ref()) {
            return Err(format!("Git output unavailable: {error}"));
        }
        Ok(GitOutput {
            success: status.is_some_and(|status| status.success()),
            stdout,
            stderr,
        })
    }

    fn required(
        &self,
        args: &[&str],
        input: Option<Vec<u8>>,
        limit: usize,
    ) -> Result<Captured, String> {
        let output = self.run(args, input, limit)?;
        if !output.success {
            Err(output.diagnostic())
        } else {
            Ok(output.stdout)
        }
    }
}

struct Captured {
    bytes: Vec<u8>,
    truncated: bool,
    error: Option<String>,
}

struct GitOutput {
    success: bool,
    stdout: Captured,
    stderr: Captured,
}

impl GitOutput {
    fn diagnostic(&self) -> String {
        let suffix = if self.stderr.truncated {
            " [stderr truncated]"
        } else {
            ""
        };
        format!(
            "Git inspection failed: {}{suffix}",
            String::from_utf8_lossy(&self.stderr.bytes).trim()
        )
    }
}

fn drain(mut stream: impl Read, limit: usize) -> Captured {
    let mut captured = Captured {
        bytes: Vec::new(),
        truncated: false,
        error: None,
    };
    let mut buffer = [0; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => return captured,
            Ok(count) => {
                let keep = count.min(limit.saturating_sub(captured.bytes.len()));
                captured.bytes.extend_from_slice(&buffer[..keep]);
                captured.truncated |= keep != count;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                captured.error = Some(error.to_string());
                return captured;
            }
        }
    }
}

fn native_path(bytes: &[u8]) -> Result<PathBuf, String> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes.to_vec())
            .map(PathBuf::from)
            .map_err(|_| {
                "Git returned an unsupported path encoding; identities were not merged".into()
            })
    }
}

fn canonical_git_path(bytes: &[u8]) -> Result<PathBuf, String> {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    let bytes = if cfg!(windows) {
        bytes.strip_suffix(b"\r").unwrap_or(bytes)
    } else {
        bytes
    };
    fs::canonicalize(native_path(bytes)?).map_err(|error| error.to_string())
}

fn relative_path(bytes: &[u8]) -> Result<PathBuf, String> {
    let path = native_path(bytes)?;
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("Git returned a path outside the repository-relative namespace".into());
    }
    Ok(path)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Entry {
    head: Option<ObjectIdentity>,
    head_size: Option<usize>,
    index: Vec<IndexEntry>,
    untracked: bool,
}

#[derive(PartialEq, Eq)]
struct Inventory {
    head: Option<String>,
    entries: BTreeMap<PathBuf, Entry>,
    complete: bool,
}

fn inventory(git: &Git<'_>, limits: Limits) -> Result<Inventory, String> {
    let head = git.run(&["rev-parse", "--verify", "-q", "HEAD"], None, 1024)?;
    let head = if head.success {
        Some(
            String::from_utf8(head.stdout.bytes)
                .map_err(|e| e.to_string())?
                .trim()
                .to_string(),
        )
    } else if head.stderr.bytes.is_empty() {
        None // Unborn repository; not an error or an empty-HEAD fiction.
    } else {
        return Err(head.diagnostic());
    };
    let mut inventory = Inventory {
        head,
        entries: BTreeMap::new(),
        complete: true,
    };
    let index = git.required(&["ls-files", "--stage", "-z"], None, limits.inventory_bytes)?;
    inventory.complete &= !index.truncated;
    for record in complete_records(&index.bytes) {
        let (header, path) = split_record(record)?;
        let fields = ascii_fields(header)?;
        if fields.len() != 3 || !valid_oid(fields[1]) {
            return Err("Invalid Git index record".into());
        }
        let entry = bounded_entry(&mut inventory, relative_path(path)?, limits.files);
        if let Some(entry) = entry {
            entry.index.push(IndexEntry {
                object: ObjectIdentity {
                    mode: fields[0].into(),
                    oid: fields[1].into(),
                },
                stage: fields[2].parse().map_err(|_| "Invalid Git index stage")?,
            });
        }
    }
    if let Some(head) = inventory.head.clone() {
        let tree = git.required(
            &["ls-tree", "-r", "-l", "-z", "--full-tree", &head],
            None,
            limits.inventory_bytes,
        )?;
        inventory.complete &= !tree.truncated;
        for record in complete_records(&tree.bytes) {
            let (header, path) = split_record(record)?;
            let fields = ascii_fields(header)?;
            if fields.len() != 4 || !valid_oid(fields[2]) {
                return Err("Invalid Git tree record".into());
            }
            if let Some(entry) = bounded_entry(&mut inventory, relative_path(path)?, limits.files) {
                entry.head = Some(ObjectIdentity {
                    mode: fields[0].into(),
                    oid: fields[2].into(),
                });
                entry.head_size = if fields[1] == "blob" {
                    fields[3].parse().ok()
                } else {
                    None
                };
            }
        }
    }
    let untracked = git.required(
        &["ls-files", "--others", "--exclude-standard", "-z"],
        None,
        limits.inventory_bytes,
    )?;
    inventory.complete &= !untracked.truncated;
    for path in complete_records(&untracked.bytes) {
        if let Some(entry) = bounded_entry(&mut inventory, relative_path(path)?, limits.files) {
            entry.untracked = true;
        }
    }
    Ok(inventory)
}

fn bounded_entry(inventory: &mut Inventory, path: PathBuf, limit: usize) -> Option<&mut Entry> {
    if inventory.entries.len() >= limit && !inventory.entries.contains_key(&path) {
        inventory.complete = false;
        None
    } else {
        Some(inventory.entries.entry(path).or_default())
    }
}

fn complete_records(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split_inclusive(|byte| *byte == 0)
        .filter_map(|record| record.strip_suffix(&[0]))
}

fn split_record(record: &[u8]) -> Result<(&[u8], &[u8]), String> {
    let tab = record
        .iter()
        .position(|byte| *byte == b'\t')
        .ok_or("Invalid Git path record")?;
    Ok((&record[..tab], &record[tab + 1..]))
}

fn ascii_fields(bytes: &[u8]) -> Result<Vec<&str>, String> {
    Ok(std::str::from_utf8(bytes)
        .map_err(|_| "Invalid Git metadata encoding")?
        .split_ascii_whitespace()
        .collect())
}

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

struct Objects {
    bytes: Vec<u8>,
    ranges: BTreeMap<String, std::ops::Range<usize>>,
}

impl Objects {
    fn get(&self, object: &ObjectIdentity) -> Option<&[u8]> {
        self.ranges
            .get(&object.oid)
            .map(|range| &self.bytes[range.clone()])
    }
}

fn head_objects(git: &Git<'_>, inventory: &Inventory, limits: Limits) -> Result<Objects, String> {
    let mut selected = BTreeMap::new();
    let mut remaining = limits.object_bytes;
    for entry in inventory.entries.values() {
        if let (Some(head), Some(size)) = (&entry.head, entry.head_size) {
            // Include protocol framing in the same aggregate bound.
            let cost = size.saturating_add(head.oid.len() + 64);
            if size <= limits.file_bytes && cost <= remaining && !selected.contains_key(&head.oid) {
                selected.insert(head.oid.clone(), size);
                remaining -= cost;
            }
        }
    }
    if selected.is_empty() {
        return Ok(Objects {
            bytes: Vec::new(),
            ranges: BTreeMap::new(),
        });
    }
    let mut input = Vec::with_capacity(selected.len() * 65);
    for oid in selected.keys() {
        input.extend_from_slice(oid.as_bytes());
        input.push(b'\n');
    }
    // No --filters/--textconv; cat-file reads only raw local objects. Lazy fetch is disabled.
    let captured = git.required(&["cat-file", "--batch"], Some(input), limits.object_bytes)?;
    if captured.truncated {
        return Err(
            "Git object output exceeded its bound; no complete comparison is available".into(),
        );
    }
    let mut ranges = BTreeMap::new();
    let mut offset = 0;
    for (oid, expected_size) in selected {
        let newline = captured.bytes[offset..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|position| position + offset)
            .ok_or("Incomplete Git object header")?;
        let fields = ascii_fields(&captured.bytes[offset..newline])?;
        if fields.len() != 3
            || fields[0] != oid
            || fields[1] != "blob"
            || fields[2].parse::<usize>().ok() != Some(expected_size)
        {
            return Err("Git object is missing or changed; raw comparison unavailable".into());
        }
        let start = newline + 1;
        let end = start
            .checked_add(expected_size)
            .ok_or("Invalid Git object length")?;
        if captured.bytes.get(end) != Some(&b'\n') {
            return Err("Incomplete Git object body".into());
        }
        ranges.insert(oid, start..end);
        offset = end + 1;
    }
    Ok(Objects {
        bytes: captured.bytes,
        ranges,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    file_type: u8,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed: (i64, i64),
    #[cfg(unix)]
    mode: u32,
    #[cfg(windows)]
    attributes: u32,
}

impl Stamp {
    fn from(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        #[cfg(windows)]
        use std::os::windows::fs::MetadataExt;
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            file_type: if metadata.is_file() {
                1
            } else if metadata.is_dir() {
                2
            } else if metadata.file_type().is_symlink() {
                3
            } else {
                4
            },
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            changed: (metadata.ctime(), metadata.ctime_nsec()),
            #[cfg(unix)]
            mode: metadata.mode(),
            #[cfg(windows)]
            attributes: metadata.file_attributes(),
        }
    }

    fn same_node(&self, other: &Self) -> bool {
        #[cfg(unix)]
        {
            self.device == other.device && self.inode == other.inode
        }
        #[cfg(not(unix))]
        {
            self.created == other.created && self.file_type == other.file_type
        }
    }
}

fn link_or_reparse(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn validate_ancestors(root: &Path, relative: &Path) -> Result<(), String> {
    let mut path = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for component in &components[..components.len().saturating_sub(1)] {
        path.push(component.as_os_str());
        match fs::symlink_metadata(&path) {
            Ok(metadata) if link_or_reparse(&metadata) => {
                return Err("Ancestor symlink/reparse point is not traversed".into());
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err("File ancestor is not a directory".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn open_regular(root: &Path, relative: &Path) -> io::Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)?;
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let name = std::ffi::CString::new(component.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | libc::O_CLOEXEC
            | if components.peek().is_some() {
                libc::O_DIRECTORY
            } else {
                0
            };
        // SAFETY: directory is live, name is NUL-terminated, and a successful fd is owned once.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat returned a new owned file descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        if components.peek().is_none() {
            return Ok(file);
        }
        directory = file;
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "Empty relative file path",
    ))
}

#[cfg(not(unix))]
fn open_regular(root: &Path, relative: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Open the final reparse point itself, never follow a substituted final-file link.
        options.custom_flags(0x0020_0000);
    }
    options.open(root.join(relative))
}

#[cfg(windows)]
fn validate_opened_path(file: &File, root: &Path, relative: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "GetFinalPathNameByHandleW"]
        fn get_final_path_name_by_handle(
            file: *mut std::ffi::c_void,
            path: *mut u16,
            capacity: u32,
            flags: u32,
        ) -> u32;
    }
    // Resolve the already-open handle before reading, not merely the mutable path. This closes
    // the ancestor-junction substitution gap left by checking the final reparse point alone.
    let mut buffer = [0u16; 32_768];
    // SAFETY: the handle remains owned by file and the output buffer has the stated capacity.
    let length = unsafe {
        get_final_path_name_by_handle(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    };
    if length == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    if length as usize >= buffer.len() {
        return Err("Resolved file path exceeded the native path bound".into());
    }
    let opened = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length as usize]));
    let expected = fs::canonicalize(root.join(relative)).map_err(|e| e.to_string())?;
    if !opened.starts_with(root) || opened != expected {
        return Err("Opened file escaped or changed its canonical repository identity".into());
    }
    Ok(())
}

struct FileCapture {
    state: FileState,
    bytes: Option<Vec<u8>>,
    stamp: Option<Stamp>,
}

fn capture_file(
    root: &Path,
    path: &Path,
    remaining: &mut usize,
    git: &Git<'_>,
    limits: Limits,
) -> FileCapture {
    let unavailable = |reason: String| FileCapture {
        state: FileState::Unavailable { reason },
        bytes: None,
        stamp: None,
    };
    match read_file(root, path, remaining, git, limits) {
        Ok(captured) => captured,
        Err(reason) => unavailable(reason),
    }
}

fn read_file(
    root: &Path,
    path: &Path,
    remaining: &mut usize,
    git: &Git<'_>,
    limits: Limits,
) -> Result<FileCapture, String> {
    git.check_deadline()?;
    validate_ancestors(root, path)?;
    let full = root.join(path);
    let before = match fs::symlink_metadata(&full) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(FileCapture {
                state: FileState::Missing,
                bytes: None,
                stamp: None,
            });
        }
        Err(error) => return Err(error.to_string()),
    };
    let stamp = Stamp::from(&before);
    if before.file_type().is_symlink() {
        let target = fs::read_link(&full).map_err(|e| e.to_string())?;
        let bytes = target.as_os_str().as_encoded_bytes();
        if bytes.len() > limits.file_bytes || bytes.len() > *remaining {
            return Err("Link identity exceeds the bounded capture budget".into());
        }
        let digest = Sha256::digest(bytes).into();
        *remaining -= bytes.len();
        let captured_bytes = bytes.to_vec();
        validate_ancestors(root, path)?;
        if Stamp::from(&fs::symlink_metadata(&full).map_err(|e| e.to_string())?) != stamp {
            return Err("Link changed during capture; identity unavailable".into());
        }
        return Ok(FileCapture {
            state: FileState::Symlink {
                target,
                sha256: digest,
            },
            bytes: Some(captured_bytes),
            stamp: Some(stamp),
        });
    }
    if link_or_reparse(&before) || !before.is_file() {
        return Err("Not a regular file (directory, submodule, reparse point, or special file); contents not read".into());
    }
    let size = usize::try_from(before.len()).map_err(|_| "File size is unsupported")?;
    if size > limits.file_bytes || size > *remaining {
        return Err(
            "File exceeds the per-file or aggregate byte limit; no partial hash is claimed".into(),
        );
    }
    let mut file = open_regular(root, path).map_err(|e| e.to_string())?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    if !opened.is_file() || link_or_reparse(&opened) || Stamp::from(&opened) != stamp {
        return Err("File changed or was substituted before opening".into());
    }
    #[cfg(windows)]
    validate_opened_path(&file, root, path)?;
    let mut bytes = Vec::with_capacity(size);
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];
    let mut binary = false;
    loop {
        git.check_deadline()?;
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        if count > *remaining || bytes.len().saturating_add(count) > limits.file_bytes {
            return Err("File grew beyond the capture budget; identity unavailable".into());
        }
        *remaining -= count;
        hasher.update(&buffer[..count]);
        binary |= buffer[..count].contains(&0);
        bytes.extend_from_slice(&buffer[..count]);
    }
    validate_ancestors(root, path)?;
    let after = fs::symlink_metadata(&full).map_err(|e| e.to_string())?;
    if Stamp::from(&after) != stamp
        || Stamp::from(&file.metadata().map_err(|e| e.to_string())?) != stamp
        || bytes.len() as u64 != before.len()
    {
        return Err("File changed during capture; identity unavailable".into());
    }
    #[cfg(windows)]
    validate_opened_path(&file, root, path)?;
    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        before.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let executable = false;
    Ok(FileCapture {
        state: FileState::Regular {
            sha256: hasher.finalize().into(),
            bytes: bytes.len() as u64,
            binary,
            executable,
        },
        bytes: Some(bytes),
        stamp: Some(stamp),
    })
}

/// Rechecks current Git ignore/inventory membership, rather than trusting a stale
/// catalogue after an ignore-file edit. The path is literal, never a pathspec.
pub(crate) fn source_visible(
    root: &Path,
    relative: &Path,
    stopped: &AtomicBool,
) -> Result<bool, String> {
    let path = source_path(relative)?;
    // Git's inventory uses '/' even when the native Windows identity uses '\'.
    // Keep stored identities untouched; translate only the Git query boundary.
    #[cfg(windows)]
    let git_path = if path.contains('\\') {
        std::borrow::Cow::Owned(path.replace('\\', "/"))
    } else {
        std::borrow::Cow::Borrowed(path)
    };
    #[cfg(windows)]
    let path: &str = &git_path;
    let git = Git::new(root, stopped, Instant::now() + COMMAND_TIMEOUT)?;
    let captured = git.required(
        &[
            "--literal-pathspecs",
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            path,
        ],
        None,
        4 * (path.len() + 1),
    )?;
    if captured.truncated {
        return Err("Source inventory membership exceeded its exact-path bound".into());
    }
    Ok(complete_records(&captured.bytes).any(|record| record == path.as_bytes()))
}

/// Bounded nonignored filenames for controlled source tools. Plain directories
/// have no Git policy; repository failures never silently disable that policy.
pub(crate) fn source_paths(
    root: &Path,
    stopped: &AtomicBool,
) -> Result<Option<BTreeSet<PathBuf>>, String> {
    if !root
        .ancestors()
        .any(|ancestor| fs::symlink_metadata(ancestor.join(".git")).is_ok())
    {
        return Ok(None);
    }
    let git = Git::new(root, stopped, Instant::now() + COMMAND_TIMEOUT)?;
    let output = git.required(
        &[
            "-c",
            "core.precomposeunicode=false",
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ],
        None,
        512 * 1024,
    )?;
    if output.truncated {
        return Err("Source filename inventory exceeds 512 KiB; select a smaller project".into());
    }
    complete_records(&output.bytes)
        .map(relative_path)
        .collect::<Result<BTreeSet<_>, _>>()
        .map(Some)
}

fn source_path(relative: &Path) -> Result<&str, String> {
    let path = relative.to_str().ok_or("Source path is not Unicode")?;
    if path.is_empty()
        || path.len() > 32 * 1024
        || !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err("Source path must be a bounded normal repository-relative path".into());
    }
    Ok(path)
}

/// Bounded source reads share the repository opened-handle/no-follow algorithm.
/// A final or ancestor link, substituted handle, changed root, and partial hash
/// are all refused; callers never receive silently substituted bytes.
pub(crate) fn read_source(
    root: &Path,
    relative: &Path,
    max_bytes: usize,
    stopped: &AtomicBool,
) -> Result<(Vec<u8>, [u8; 32]), String> {
    source_path(relative)?;
    let before = fs::symlink_metadata(root).map_err(|error| error.to_string())?;
    if !before.is_dir()
        || link_or_reparse(&before)
        || fs::canonicalize(root).map_err(|error| error.to_string())? != root
    {
        return Err("Repository root is linked or no longer canonical".into());
    }
    let stamp = Stamp::from(&before);
    let git = Git::new(root, stopped, Instant::now() + COMMAND_TIMEOUT)?;
    let limits = Limits {
        file_bytes: max_bytes,
        ..Limits::default()
    };
    let mut remaining = max_bytes;
    let captured = read_file(root, relative, &mut remaining, &git, limits)?;
    let after = fs::symlink_metadata(root).map_err(|error| error.to_string())?;
    if !stamp.same_node(&Stamp::from(&after))
        || link_or_reparse(&after)
        || fs::canonicalize(root).map_err(|error| error.to_string())? != root
    {
        return Err("Repository root changed while reading source".into());
    }
    match (captured.state, captured.bytes) {
        (FileState::Regular { sha256, .. }, Some(bytes)) => Ok((bytes, sha256)),
        _ => Err("Selected source is not an available regular file".into()),
    }
}

fn capture(
    repository: &RepositoryIdentity,
    git: &Git<'_>,
    limits: Limits,
) -> Result<Snapshot, String> {
    let before = inventory(git, limits)?;
    let mut diff = DiffBuilder::new(limits.diff_bytes);
    diff.push("Raw HEAD → working tree diff; no filters/textconv. Index identities are recorded separately.\n");
    let mut warnings = vec!["Ignored untracked files and submodule contents are outside coverage. Observations are not an atomic snapshot.".into()];
    let objects = match head_objects(git, &before, limits) {
        Ok(objects) => objects,
        Err(reason) => {
            diff.unavailable(&reason);
            warnings.push(reason);
            Objects {
                bytes: Vec::new(),
                ranges: BTreeMap::new(),
            }
        }
    };
    let mut files = BTreeMap::new();
    let mut stamps = BTreeMap::new();
    let mut remaining = limits.total_file_bytes;
    let mut complete = before.complete;
    for (path, entry) in &before.entries {
        git.check_deadline()?;
        let captured = capture_file(&repository.root, path, &mut remaining, git, limits);
        let old = entry.head.as_ref().and_then(|head| objects.get(head));
        let dirty = dirty_state(entry, &captured.state, captured.bytes.as_deref(), old);
        if matches!(captured.state, FileState::Unavailable { .. }) || dirty.is_none() {
            complete = false;
        }
        if dirty != Some(false) {
            diff.file(
                path,
                entry,
                &captured.state,
                old,
                captured.bytes.as_deref(),
                dirty,
            );
        }
        stamps.insert(path.clone(), captured.stamp);
        files.insert(
            path.clone(),
            FileIdentity {
                path: path.clone(),
                tracked: entry.head.is_some() || !entry.index.is_empty(),
                head: entry.head.clone(),
                index: entry.index.clone(),
                state: captured.state,
                dirty,
            },
        );
    }
    let after = inventory(git, limits)?;
    let mut stable = before == after;
    for (path, stamp) in stamps {
        let now = validate_ancestors(&repository.root, &path).and_then(|()| {
            match fs::symlink_metadata(repository.root.join(&path)) {
                Ok(metadata) => Ok(Some(Stamp::from(&metadata))),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error.to_string()),
            }
        });
        // An unavailable file is already unknown, not evidence of a new race.
        let file = files.get_mut(&path).expect("captured file");
        if !matches!(file.state, FileState::Unavailable { .. }) && now.ok().as_ref() != Some(&stamp)
        {
            stable = false;
            file.state = FileState::Unavailable {
                reason: "File changed before the scan completed".into(),
            };
            file.dirty = None;
        }
    }
    if !before.complete || !after.complete {
        complete = false;
        warnings.push(
            "File inventory reached its entry/output bound; omitted paths are unknown, not deleted"
                .into(),
        );
        diff.unavailable("File inventory is incomplete");
    }
    if !stable {
        complete = false;
        warnings.push("Concurrent repository changes detected; this capture is ambiguous. Refresh preserves the original baseline.".into());
        diff.unavailable(
            "Repository changed during capture; displayed bytes may span multiple states",
        );
    }
    if files.values().any(|file| file.dirty.is_none()) {
        diff.unavailable(
            "Some file comparisons are unavailable; inspect per-file reasons/identities",
        );
    }
    Ok(Snapshot {
        head: before.head,
        files,
        working_diff: diff.finish(),
        complete,
        stable,
        warnings,
    })
}

fn dirty_state(
    entry: &Entry,
    state: &FileState,
    current: Option<&[u8]>,
    head: Option<&[u8]>,
) -> Option<bool> {
    if matches!(state, FileState::Unavailable { .. }) {
        return None;
    }
    let index_changed = match (&entry.head, entry.index.as_slice()) {
        (None, []) => false,
        (Some(head), [index]) => index.stage != 0 || &index.object != head,
        _ => true,
    };
    if index_changed || entry.untracked {
        return Some(true);
    }
    match (&entry.head, state, current, head) {
        (None, FileState::Missing, _, _) => Some(false),
        (None, _, _, _) | (Some(_), FileState::Missing, _, _) => Some(true),
        (Some(object), FileState::Regular { executable, .. }, Some(current), Some(head)) => {
            let mode_changed = object.mode == "120000"
                || (cfg!(unix) && (*executable != (object.mode == "100755")));
            Some(mode_changed || current != head)
        }
        (Some(object), FileState::Symlink { .. }, Some(current), Some(head)) => {
            Some(object.mode != "120000" || current != head)
        }
        _ => None,
    }
}

struct DiffBuilder {
    diff: WorkingDiff,
    limit: usize,
}

impl DiffBuilder {
    fn new(limit: usize) -> Self {
        Self {
            diff: WorkingDiff::default(),
            limit,
        }
    }

    fn push(&mut self, text: &str) {
        let mut keep = text
            .len()
            .min(self.limit.saturating_sub(self.diff.text.len()));
        while !text.is_char_boundary(keep) {
            keep -= 1;
        }
        self.diff.text.push_str(&text[..keep]);
        self.diff.truncated |= keep < text.len();
    }

    fn unavailable(&mut self, message: &str) {
        self.diff.unavailable.get_or_insert_with(|| message.into());
    }

    fn file(
        &mut self,
        path: &Path,
        entry: &Entry,
        state: &FileState,
        old: Option<&[u8]>,
        new: Option<&[u8]>,
        dirty: Option<bool>,
    ) {
        if self.diff.truncated {
            return;
        }
        self.push(&format!(
            "\n--- HEAD/{}\n+++ working/{}\n",
            path.display(),
            path.display()
        ));
        if let FileState::Unavailable { reason } = state {
            self.push(&format!("[unavailable: {reason}]\n"));
            return;
        }
        if entry.index.len() > 1 || entry.index.iter().any(|index| index.stage != 0) {
            self.push("[unmerged index; no authorship inferred]\n");
        } else if entry.head.as_ref() != entry.index.first().map(|index| &index.object) {
            self.push("[index differs from HEAD; staged object identity recorded separately]\n");
        }
        if entry.head.is_some() && old.is_none() {
            self.push("[HEAD bytes unavailable or over capture limit]\n");
            self.unavailable("Some HEAD objects are unavailable or exceed capture limits");
            return;
        }
        let old = old.unwrap_or_default();
        let new = new.unwrap_or_default();
        if old == new {
            self.push(if matches!(state, FileState::Missing) {
                "[deleted empty file]\n"
            } else if entry.head.is_none() {
                "[new empty file]\n"
            } else {
                "[raw bytes match HEAD; index or file mode differs]\n"
            });
            return;
        }
        if old.contains(&0) || new.contains(&0) {
            self.push(&format!(
                "[binary bytes differ: HEAD {} bytes, working {} bytes; content omitted]\n",
                old.len(),
                new.len()
            ));
            return;
        }
        let (Ok(old), Ok(new)) = (std::str::from_utf8(old), std::str::from_utf8(new)) else {
            self.push("[non-UTF-8 bytes differ; text omitted, exact byte identities retained]\n");
            return;
        };
        if dirty.is_none() {
            self.push("[comparison coverage unknown]\n");
        }
        // Deliberately bounded full-file replacement hunks, not a quadratic LCS allocation.
        self.push(&format!(
            "@@ -1,{} +1,{} @@ (full-file raw comparison)\n",
            old.lines().count(),
            new.lines().count()
        ));
        for (prefix, contents) in [("-", old), ("+", new)] {
            for line in contents.split_inclusive('\n') {
                if self.diff.truncated {
                    return;
                }
                self.push(prefix);
                self.push(line);
                if !line.ends_with('\n') {
                    self.push("\n\\ No newline at end of file\n");
                }
            }
        }
    }

    fn finish(self) -> WorkingDiff {
        self.diff
    }
}

fn classify(baseline: &Snapshot, current: &Snapshot, initial: bool) -> Vec<Change> {
    let paths: BTreeSet<_> = baseline.files.keys().chain(current.files.keys()).collect();
    let mut changes = Vec::new();
    for path in paths {
        let before = baseline.files.get(path);
        let after = current.files.get(path);
        let was_dirty = before.is_some_and(|file| file.dirty == Some(true));
        let unknown = !baseline.stable
            || !current.stable
            || (before.is_none() && !baseline.complete)
            || (after.is_none() && !current.complete)
            || before.is_some_and(|file| file.dirty.is_none())
            || after.is_some_and(|file| file.dirty.is_none());
        let changed = before != after;
        if !changed && !was_dirty && !unknown {
            continue;
        }
        let origin = if unknown {
            ChangeOrigin::Unknown
        } else if initial || !changed {
            ChangeOrigin::PreExisting
        } else {
            ChangeOrigin::Unattributed
        };
        let kind = if unknown {
            ChangeKind::Unknown
        } else if initial || !changed {
            match after {
                Some(file) if matches!(file.state, FileState::Missing) => ChangeKind::Deleted,
                Some(file) if file.head.is_none() => ChangeKind::Added,
                _ => ChangeKind::Modified,
            }
        } else {
            match (before, after) {
                (None, _) => ChangeKind::Added,
                (_, None) => ChangeKind::Deleted,
                (_, Some(file)) if matches!(file.state, FileState::Missing) => ChangeKind::Deleted,
                (Some(old), Some(new)) if old.state == new.state => ChangeKind::MetadataChanged,
                (Some(old), _) if matches!(old.state, FileState::Missing) => ChangeKind::Added,
                _ => ChangeKind::Modified,
            }
        };
        changes.push(Change {
            path: path.clone(),
            origin,
            kind,
            was_dirty,
        });
    }
    changes
}

#[cfg(test)]
mod tests;
