//! Scoped GP07 mutation. Runs only inside the existing owned runner and its one-shot permit.
//! No status/diff/add/commit, filters, shell, editor, signer, remote, hooks or helper execution.
use super::*;
use crate::workspace::repo::validate_git_version;
use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::path::Component;

const FILE_LIMIT: u64 = 8 * 1024 * 1024;
const TOTAL_LIMIT: usize = 32 * 1024 * 1024;
const OUTPUT_LIMIT: usize = 64 * 1024 * 1024;
const DISCLOSURE: &str = "LOCAL ONLY; not content verification. Qualified Git 2.36+ required. Raw whole-file bytes and native executable modes: bypasses attributes, clean/smudge/process filters, CRLF conversion, hooks (including reference-transaction), signing, editors and executable helpers. Exact UTF-8 message; ordinary version-2 isolated index; plumbing rather than porcelain/reflog defaults. Existing native single-link HEAD/branch reflogs may append. No push/authentication. Unselected worktree bytes are never written. Initially staged work is refused. Objects may remain unreachable after failure/cancellation. Ref and index publication are separate: a partial/unknown outcome is possible; no retry, reset or claimed rollback.";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SelectedFile {
    pub path: PathBuf,
    pub sha256: Option<String>,
    pub bytes: u64,
    pub mode: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub head: String,
    pub reference: String,
    pub index_sha256: String,
    pub config_sha256: String,
    pub author: String,
    pub email: String,
    pub timestamp: u64,
    pub git_version: String,
    pub files: Vec<SelectedFile>,
    /// Authority and source/config bytes are deliberately not persisted or projected.
    #[serde(skip)]
    frozen: Option<Arc<Frozen>>,
}
impl std::fmt::Debug for Review {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SelectedCommitReview")
            .field("head", &self.head)
            .field("reference", &self.reference)
            .field("git_version", &self.git_version)
            .field("files", &self.files)
            .finish_non_exhaustive()
    }
}
impl PartialEq for Review {
    fn eq(&self, other: &Self) -> bool {
        self.head == other.head
            && self.reference == other.reference
            && self.index_sha256 == other.index_sha256
            && self.config_sha256 == other.config_sha256
            && self.author == other.author
            && self.email == other.email
            && self.timestamp == other.timestamp
            && self.files == other.files
            && self.git_version == other.git_version
    }
}
impl Eq for Review {}
impl Review {
    pub(crate) fn discard_authority(&mut self) {
        self.frozen = None;
    }
    pub(crate) fn has_authority(&self) -> bool {
        self.frozen.is_some()
    }
}
struct Frozen {
    config: Vec<u8>,
    index: Vec<u8>,
    head_file: Vec<u8>,
    reference_file: Option<Vec<u8>>,
    git_identity: String,
    sources: Vec<Option<Vec<u8>>>,
    source_nodes: Vec<Option<NodeStamp>>,
    reflogs: Vec<ReflogNode>,
    environment: Vec<(String, String)>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub created_commit: Option<String>,
    pub observed_head: Option<String>,
    pub observed_reference: Option<String>,
    pub observed_index_sha256: Option<String>,
    pub expected_index_sha256: Option<String>,
    pub ref_published: bool,
    pub index_published: bool,
    pub publication_started: bool,
    pub owned_tree_exited: bool,
    pub error: Option<String>,
}
impl Outcome {
    fn new() -> Self {
        Self {
            created_commit: None,
            observed_head: None,
            observed_reference: None,
            observed_index_sha256: None,
            expected_index_sha256: None,
            ref_published: false,
            index_published: false,
            publication_started: false,
            error: None,
            owned_tree_exited: true,
        }
    }
    pub fn summary(&self) -> String {
        format!(
            "Selected local commit: created {:?}; observed HEAD {:?}; ref {:?}; index SHA-256 {:?}; expected index {:?}; ref published {}; index published {}; publication attempted {}. {}\nA local commit is not verification; objects may remain unreachable. No rollback or retry was performed.\n",
            self.created_commit,
            self.observed_head,
            self.observed_reference,
            self.observed_index_sha256,
            self.expected_index_sha256,
            self.ref_published,
            self.index_published,
            self.publication_started,
            self.error.as_deref().unwrap_or("")
        )
    }
}
pub(super) fn resolve_git(root: &Path) -> Result<PathBuf, String> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .filter(|path| path.is_absolute())
        .filter_map(|directory| {
            fs::canonicalize(directory.join(if cfg!(windows) { "git.exe" } else { "git" })).ok()
        })
        .find(|path| !path.starts_with(root) && native::is_executable(path))
        .ok_or_else(|| "No Git executable outside the project exists on the absolute PATH".into())
}

pub fn validate(paths: &[PathBuf], message: &str) -> Result<(), String> {
    if paths.is_empty()
        || paths.len() > 64
        || message.trim().is_empty()
        || message.len() > 16 * 1024
        || message.contains('\0')
    {
        return Err("Selected commit needs 1–64 literal whole-file paths and a nonempty exact message (at most 16 KiB; no NUL)".into());
    }
    let mut seen = BTreeSet::new();
    let mut bytes = message.len();
    for path in paths {
        bytes = bytes.saturating_add(path.as_os_str().len());
        if path.to_str().is_none_or(|text| text.contains('\0'))
            || path.as_os_str().is_empty()
            || !path.components().all(|c| matches!(c, Component::Normal(_)))
            || path
                .components()
                .any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case(".git"))
            || !seen.insert(path)
        {
            return Err("Selected commit paths must be unique, native Unicode, literal project-relative files outside .git".into());
        }
    }
    if bytes > SPEC_LIMIT {
        return Err("Selected paths and exact message exceed 64 KiB".into());
    }
    Ok(())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn plain(root: &Path, relative: &Path, limit: u64) -> Result<Vec<u8>, String> {
    read_contained(root, relative, limit)?
        .map(|(bytes, _)| bytes)
        .ok_or_else(|| format!("{} is unavailable", relative.display()))
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct NodeStamp {
    identity: String,
    bytes: u64,
    modified: Option<SystemTime>,
    links: u64,
    executable: bool,
    #[cfg(unix)]
    changed: (i64, i64),
    #[cfg(windows)]
    attributes: u32,
}
fn node_stamp(file: &File) -> Result<NodeStamp, String> {
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    #[cfg(unix)]
    let (identity, links, executable) = {
        use std::os::unix::fs::MetadataExt;
        (
            format!("{}:{}", metadata.dev(), metadata.ino()),
            metadata.nlink(),
            metadata.mode() & 0o111 != 0,
        )
    };
    #[cfg(windows)]
    let (identity, links, executable) = {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("Native reparse identity refused".into());
        }
        (
            format!(
                "{}:{}:{}",
                info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
            ),
            u64::from(info.nNumberOfLinks),
            false,
        )
    };
    Ok(NodeStamp {
        identity,
        bytes: metadata.len(),
        modified: metadata.modified().ok(),
        links,
        executable,
        #[cfg(unix)]
        changed: {
            use std::os::unix::fs::MetadataExt;
            (metadata.ctime(), metadata.ctime_nsec())
        },
        #[cfg(windows)]
        attributes: {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes()
        },
    })
}
fn read_contained(
    root: &Path,
    path: &Path,
    limit: u64,
) -> Result<Option<(Vec<u8>, NodeStamp)>, String> {
    let Some(mut file) = crate::workspace::repo::open_contained_node(root, path)? else {
        return Ok(None);
    };
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("Native capture requires a regular file".into());
    }
    #[cfg(windows)]
    if fs::canonicalize(root.join(path)).map_err(|error| error.to_string())? != root.join(path) {
        return Err(
            "Use the exact native filename spelling; case/short-name aliases are unsupported"
                .into(),
        );
    }
    let before = node_stamp(&file)?;
    if before.bytes > limit {
        return Err("Native capture exceeds its byte limit".into());
    }
    let mut bytes = Vec::with_capacity(before.bytes as usize);
    (&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 != before.bytes || node_stamp(&file)? != before {
        return Err("File changed during native-handle capture".into());
    }
    let reopened = crate::workspace::repo::open_contained_node(root, path)?
        .ok_or("File disappeared during capture")?;
    if !reopened
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
        || node_stamp(&reopened)? != before
    {
        return Err("File path/native identity changed during capture".into());
    }
    Ok(Some((bytes, before)))
}
type SourceCapture = (SelectedFile, Option<Vec<u8>>, Option<NodeStamp>);

fn source(root: &Path, path: &Path) -> Result<SourceCapture, String> {
    let Some((bytes, node)) = read_contained(root, path, FILE_LIMIT)? else {
        return Ok((
            SelectedFile {
                path: path.into(),
                sha256: None,
                bytes: 0,
                mode: "deleted".into(),
            },
            None,
            None,
        ));
    };
    Ok((
        SelectedFile {
            path: path.into(),
            sha256: Some(hash(&bytes)),
            bytes: node.bytes,
            mode: if node.executable { "100755" } else { "100644" }.into(),
        },
        Some(bytes),
        Some(node),
    ))
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct ReflogNode {
    path: PathBuf,
    identity: Option<String>,
    file: Option<NodeStamp>,
}
fn reflog_nodes(root: &Path, reference: &str) -> Result<Vec<ReflogNode>, String> {
    let mut nodes = Vec::new();
    // update-ref appends existing branch AND HEAD reflogs despite logAllRefUpdates=false.
    for relative in [
        PathBuf::from(".git/logs/HEAD"),
        Path::new(".git/logs").join(reference),
    ] {
        let components: Vec<_> = relative.components().collect();
        let mut path = PathBuf::new();
        for (index, component) in components.iter().enumerate() {
            path.push(component.as_os_str());
            let Some(file) = crate::workspace::repo::open_contained_node(root, &path)? else {
                nodes.push(ReflogNode {
                    path: path.clone(),
                    identity: None,
                    file: None,
                });
                break;
            };
            let metadata = file.metadata().map_err(|error| error.to_string())?;
            let stamp = node_stamp(&file)?;
            if index + 1 == components.len() {
                if !metadata.is_file() || stamp.links != 1 {
                    return Err("Writable reflogs must be native regular files with exactly one hard link; unselected-file aliases are refused".into());
                }
                nodes.push(ReflogNode {
                    path: path.clone(),
                    identity: Some(stamp.identity.clone()),
                    file: Some(stamp),
                });
            } else {
                if !metadata.is_dir() {
                    return Err("Reflog ancestor is not a native directory".into());
                }
                nodes.push(ReflogNode {
                    path: path.clone(),
                    identity: Some(stamp.identity),
                    file: None,
                });
            }
        }
    }
    Ok(nodes)
}
fn environment() -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = [
        ("LC_ALL", "C"),
        ("TZ", "UTC"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_NO_LAZY_FETCH", "1"),
        ("GIT_CONFIG_COUNT", "0"),
        ("GIT_CONFIG_SYSTEM", "/dev/null"),
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_ATTR_NOSYSTEM", "1"),
        ("GIT_NO_REPLACE_OBJECTS", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_OPTIONAL_LOCKS", "0"),
        ("GIT_PAGER", ""),
        ("GIT_EXTERNAL_DIFF", ""),
        ("GIT_PROTOCOL_FROM_USER", "0"),
        ("GIT_ALLOW_PROTOCOL", ""),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    // No inherited PATH/private variables. Builtin-only qualified Git/config
    // guards exclude helpers; missing PATH alone would not prevent execvp lookup.
    for key in ["SystemRoot", "WINDIR"] {
        if let Ok(v) = std::env::var(key) {
            result.push((key.into(), v));
        }
    }
    result
}
fn args(tail: &[String]) -> Vec<String> {
    // The owned process always starts in the frozen repository cwd. Keep Git's
    // path arguments relative: its config/index readers do not consistently
    // accept Windows canonical verbatim (\\?\) paths as Rust's filesystem does.
    let mut args = vec![
        "--no-pager".into(),
        "--no-replace-objects".into(),
        "--git-dir=.git".into(),
        "--work-tree=.".into(),
    ];
    for setting in [
        "core.hooksPath=/dev/null",
        "core.fsmonitor=false",
        "commit.gpgSign=false",
        "tag.gpgSign=false",
        "gc.auto=0",
        "maintenance.auto=false",
        "core.logAllRefUpdates=false",
        "core.untrackedCache=false",
        "core.splitIndex=false",
        "core.sparseCheckout=false",
        "core.precomposeunicode=false",
        "submodule.recurse=false",
        "protocol.allow=never",
        "protocol.file.allow=never",
        "core.pager=",
        "diff.external=",
        "core.ignoreCase=false",
        "core.protectNTFS=true",
        "core.protectHFS=true",
        "index.version=2",
        "index.sparse=false",
        "core.autocrlf=false",
        "core.eol=lf",
        "i18n.commitEncoding=UTF-8",
    ] {
        args.extend(["-c".into(), setting.into()]);
    }
    args.extend_from_slice(tail);
    args
}
fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).into()).collect()
}
fn identity_environment(
    env: &mut Vec<(String, String)>,
    author: &str,
    email: &str,
    timestamp: u64,
) {
    for (key, value) in [
        ("GIT_AUTHOR_NAME", author.to_owned()),
        ("GIT_COMMITTER_NAME", author.to_owned()),
        ("GIT_AUTHOR_EMAIL", email.to_owned()),
        ("GIT_COMMITTER_EMAIL", email.to_owned()),
        ("GIT_AUTHOR_DATE", format!("@{timestamp} +0000")),
        ("GIT_COMMITTER_DATE", format!("@{timestamp} +0000")),
    ] {
        env.push((key.into(), value));
    }
}
/// Every plumbing process uses the same native process-group/Job owner, on the runner thread.
/// No helper executor, unmanaged subprocess, shell, or inherited environment.
fn git(
    p: &Prepared,
    tail: &[String],
    input: &[u8],
    env: &[(String, String)],
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>, String> {
    if cancelled() {
        return Err("Selected commit cancelled".into());
    }
    if executable_identity(&p.executable.path).map_err(|e| e.to_string())? != p.executable {
        return Err("Git executable changed".into());
    }
    let mut launch = p.clone();
    launch.spec = CommandSpec::Direct {
        program: "git".into(),
        args: args(tail),
    };
    let mut child = native::Process::spawn_piped_clean_env(&launch, env)
        .map_err(|e| format!("Owned Git launch: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let output_limit = if tail.first().is_some_and(|arg| arg == "--version") {
        128
    } else {
        OUTPUT_LIMIT
    };
    let mut written = 0;
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let mut eof = [false; 2];
    let mut status = None;
    let mut buffer = [0u8; 8192];
    let result = (|| {
        loop {
            if cancelled() || Instant::now() >= deadline {
                return Err(
                    "Selected commit cancelled or Git deadline exceeded; effects may remain".into(),
                );
            }
            if written < input.len() {
                match child.write_stdin(&input[written..]) {
                    Ok(n) => written += n,
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(e) => return Err(format!("Git stdin: {e}")),
                }
            } else {
                child.close_stdin();
            }
            for (i, bytes) in [(0, &mut output), (1, &mut errors)] {
                for _ in 0..8 {
                    if eof[i] {
                        break;
                    }
                    match child.read(i, &mut buffer) {
                        Ok(0) => {
                            eof[i] = true;
                            break;
                        }
                        Ok(n) => {
                            if bytes.len() + n > output_limit {
                                return Err("Git output limit exceeded".into());
                            }
                            bytes.extend_from_slice(&buffer[..n]);
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(format!("Git output: {e}")),
                    }
                }
            }
            if status.is_none() {
                status = child.try_wait().map_err(|e| e.to_string())?;
            }
            if let Some((code, _)) = status
                && eof.iter().all(|v| *v)
            {
                if code == Some(0) {
                    return Ok(output);
                }
                return Err(format!(
                    "Git refused: {}",
                    String::from_utf8_lossy(&errors[..errors.len().min(8192)])
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
    })();
    // Establish owned-tree exit, not merely a killed root, before returning.
    let _ = child.terminate(true);
    let stop = Instant::now() + HARD_STOP;
    let mut closed = false;
    while Instant::now() < stop {
        let reaped = child.try_wait().is_ok_and(|status| status.is_some());
        if reaped && child.tree_empty().is_ok_and(|empty| empty) {
            closed = true;
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    drop(child);
    if !closed {
        return Err(format!(
            "Owned Git tree outcome unknown: {}",
            result.err().unwrap_or_default()
        ));
    }
    result
}
fn text(bytes: Vec<u8>) -> Result<String, String> {
    String::from_utf8(bytes)
        .map(|s| s.trim_end_matches('\n').to_owned())
        .map_err(|_| "Git returned non-Unicode metadata".into())
}
fn oid(bytes: Vec<u8>) -> Result<String, String> {
    let value = text(bytes)?;
    if value.len() != 40 || !value.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Only ordinary SHA-1 repositories are supported".into());
    }
    Ok(value)
}
fn ordinary(
    root: &Path,
    cancelled: &dyn Fn() -> bool,
    owns_index_config_locks: bool,
) -> Result<(), String> {
    let dir = root.join(".git");
    let meta = fs::symlink_metadata(&dir).map_err(|e| e.to_string())?;
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || fs::canonicalize(&dir).map_err(|e| e.to_string())? != dir
    {
        return Err("Selected commits require the ordinary repository root with a native .git directory; linked/worktree/bare repositories are unsupported".into());
    }
    for name in [
        "MERGE_HEAD",
        "MERGE_MSG",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "REBASE_HEAD",
        "BISECT_LOG",
        "rebase-apply",
        "rebase-merge",
        "sequencer",
        "commondir",
        "worktrees",
        "shallow",
        "info/grafts",
        "objects/info/alternates",
        "objects/info/http-alternates",
        "refs/replace",
    ] {
        if dir.join(name).symlink_metadata().is_ok() {
            return Err(format!("Unsupported repository operation/effect: {name}"));
        }
    }
    for name in ["index.lock", "HEAD.lock", "config.lock"] {
        if owns_index_config_locks && name != "HEAD.lock" {
            continue;
        }
        if dir.join(name).symlink_metadata().is_ok() {
            return Err(format!("Repository is locked: {name}"));
        }
    }
    for name in ["objects", "refs"] {
        let path = dir.join(name);
        if fs::canonicalize(&path).map_err(|e| e.to_string())? != path {
            return Err("Linked Git administrative directories are unsupported".into());
        }
    }
    let mut pending = vec![dir.join("objects"), dir.join("refs")];
    let mut entries = 0usize;
    let deadline = Instant::now() + Duration::from_secs(30);
    while let Some(path) = pending.pop() {
        if cancelled() || Instant::now() >= deadline {
            return Err("Git administration observation cancelled or exceeded its deadline".into());
        }
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            entries += 1;
            if entries > 262_144 {
                return Err("Git administration exceeds the supported bounded inventory".into());
            }
            let path = entry.map_err(|e| e.to_string())?.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if path
                .extension()
                .is_some_and(|extension| extension == "promisor")
            {
                return Err("Promisor/partial-clone object storage is unsupported".into());
            }
            if meta.file_type().is_symlink() || (!meta.is_file() && !meta.is_dir()) {
                return Err("Linked/nonregular Git object/ref storage is unsupported".into());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err("Git object/ref reparse points are unsupported".into());
                }
            }
            if meta.is_dir() {
                pending.push(path);
            }
        }
    }
    if dir.join("packed-refs").symlink_metadata().is_ok() {
        plain(root, Path::new(".git/packed-refs"), 16 * 1024 * 1024)?;
    }
    Ok(())
}
fn config_identity(
    p: &Prepared,
    env: &[(String, String)],
    cancelled: &dyn Fn() -> bool,
) -> Result<(Vec<u8>, String, String), String> {
    let bytes = plain(&p.cwd, Path::new(".git/config"), 1024 * 1024)?;
    // Parse the exact local file, without includes; config is data here, never an executable source.
    let raw = git(
        p,
        &[
            "config".into(),
            "--no-includes".into(),
            "--null".into(),
            "--list".into(),
            "--file=.git/config".into(),
        ],
        &[],
        env,
        cancelled,
    )?;
    let mut name = None;
    let mut email = None;
    for entry in raw.split(|b| *b == 0).filter(|v| !v.is_empty()) {
        let split = entry
            .iter()
            .position(|b| *b == b'\n')
            .ok_or("Malformed Git configuration")?;
        let (key, rest) = entry.split_at(split);
        let value = &rest[1..];
        let key = String::from_utf8_lossy(key).to_ascii_lowercase();
        if key.starts_with("include.")
            || key.starts_with("includeif.")
            || key.starts_with("extensions.")
            || key.starts_with("hook.")
            || key.starts_with("remote.") && key.ends_with(".promisor")
            || matches!(key.as_str(), "core.worktree" | "core.alternaterefscommand")
        {
            return Err(format!("Unsupported Git configuration effect: {key}"));
        }
        if key == "core.repositoryformatversion" && value != b"0" {
            return Err("Only repository format 0 is supported".into());
        }
        if key == "core.bare" && value != b"false" {
            return Err("Bare repositories are unsupported".into());
        }
        if key == "user.name" {
            name = Some(String::from_utf8(value.to_vec()).map_err(|_| "Non-Unicode user.name")?);
        }
        if key == "user.email" {
            email = Some(String::from_utf8(value.to_vec()).map_err(|_| "Non-Unicode user.email")?);
        }
    }
    let name = name.ok_or("Set repository-local user.name before authoring a selected commit")?;
    let email =
        email.ok_or("Set repository-local user.email before authoring a selected commit")?;
    for value in [&name, &email] {
        if value.trim().is_empty()
            || value.len() > 512
            || value.chars().any(|c| c.is_control() || "<>".contains(c))
        {
            return Err("Repository-local identity must be nonempty, bounded and contain no controls or angle brackets".into());
        }
    }
    if plain(&p.cwd, Path::new(".git/config"), 1024 * 1024)? != bytes {
        return Err("Git configuration changed during freeze".into());
    }
    Ok((bytes, name, email))
}
fn clean_index(
    p: &Prepared,
    env: &[(String, String)],
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    let head = git(
        p,
        &strings(&["ls-tree", "-r", "-z", "--full-tree", "HEAD"]),
        &[],
        env,
        cancelled,
    )?;
    let index = git(
        p,
        &strings(&["ls-files", "--stage", "-z"]),
        &[],
        env,
        cancelled,
    )?;
    let flags = git(p, &strings(&["ls-files", "-v", "-z"]), &[], env, cancelled)?;
    if flags
        .split(|b| *b == 0)
        .any(|entry| !entry.is_empty() && !entry.starts_with(b"H "))
    {
        return Err(
            "Assume-unchanged, skip-worktree and sparse index flags are unsupported".into(),
        );
    }
    drop(flags);
    const DIRTY: &str =
        "Initial index is staged, conflicted, sparse or differs from HEAD; nothing committed";
    let mut indexed = index
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty());
    for entry in head.split(|b| *b == 0).filter(|v| !v.is_empty()) {
        let tab = entry
            .iter()
            .position(|b| *b == b'\t')
            .ok_or("Malformed HEAD tree")?;
        let mut meta = entry[..tab].split(|byte| *byte == b' ');
        let mode = meta.next().ok_or("Malformed HEAD mode")?;
        let kind = meta.next().ok_or("Malformed HEAD object kind")?;
        let id = meta.next().ok_or("Malformed HEAD object ID")?;
        if meta.next().is_some() {
            return Err("Malformed HEAD metadata".into());
        }
        let regular = match (mode, kind) {
            (b"100644" | b"100755", b"blob") => true,
            (b"120000", b"blob") | (b"160000", b"commit") => false,
            _ => return Err("Unsupported HEAD tree mode/object kind".into()),
        };
        if !regular
            && let CommandSpec::SelectedCommit { paths, .. } = &p.spec
            && paths.iter().any(|path| {
                path.as_os_str()
                    .as_encoded_bytes()
                    .iter()
                    .map(|byte| {
                        if cfg!(windows) && *byte == b'\\' {
                            b'/'
                        } else {
                            *byte
                        }
                    })
                    .eq(entry[tab + 1..].iter().copied())
            })
        {
            return Err("Selected symlink/gitlink entries are unsupported; unselected entries remain untouched".into());
        }
        let actual = indexed.next().ok_or(DIRTY)?;
        let split = actual.iter().position(|byte| *byte == b'\t').ok_or(DIRTY)?;
        let mut fields = actual[..split].split(|byte| *byte == b' ');
        if fields.next() != Some(mode)
            || fields.next() != Some(id)
            || fields.next() != Some(&b"0"[..])
            || fields.next().is_some()
            || actual[split + 1..] != entry[tab + 1..]
        {
            return Err(DIRTY.into());
        }
    }
    if indexed.next().is_some() {
        return Err(DIRTY.into());
    }
    Ok(())
}
fn reference_file(root: &Path, relative: &Path) -> Result<Option<Vec<u8>>, String> {
    let Some((bytes, _)) = read_contained(root, relative, 4096)? else {
        return Ok(None);
    };
    let value = std::str::from_utf8(&bytes)
        .map_err(|_| "Non-Unicode local ref")?
        .trim_end_matches('\n');
    if value.len() != 40 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Local branch symrefs/nonordinary ref storage are unsupported".into());
    }
    Ok(Some(bytes))
}
pub(super) fn freeze(p: &Prepared, cancelled: &dyn Fn() -> bool) -> Result<Review, String> {
    let CommandSpec::SelectedCommit { paths, message, .. } = &p.spec else {
        return Err("Not a selected commit".into());
    };
    validate(paths, message)?;
    ordinary(&p.cwd, cancelled, false)?;
    let mut env = environment();
    let version = git(p, &strings(&["--version"]), &[], &env, cancelled)?;
    let git_version = std::str::from_utf8(&version)
        .map_err(|_| "Git version is not Unicode")?
        .trim_end_matches(['\r', '\n'])
        .to_owned();
    validate_git_version(&git_version)?;
    let (config, author, email) = config_identity(p, &env, cancelled)?;
    let timestamp = now_ms() / 1000;
    identity_environment(&mut env, &author, &email, timestamp);
    let expected = format!("{author} <{email}> {timestamp} +0000");
    for variable in ["GIT_AUTHOR_IDENT", "GIT_COMMITTER_IDENT"] {
        if text(git(
            p,
            &["var".into(), variable.into()],
            &[],
            &env,
            cancelled,
        )?)? != expected
        {
            return Err("Git would normalize the configured identity; configure an exact supported local identity before review".into());
        }
    }
    let head_file = plain(&p.cwd, Path::new(".git/HEAD"), 4096)?;
    let reference = std::str::from_utf8(&head_file)
        .map_err(|_| "Invalid HEAD")?
        .strip_prefix("ref: ")
        .ok_or("Detached/unborn HEAD is unsupported")?
        .trim_end_matches('\n')
        .to_owned();
    if !reference.starts_with("refs/heads/") || reference.contains(['\n', '\r', '\0']) {
        return Err("HEAD must name one local branch".into());
    }
    git(
        p,
        &["check-ref-format".into(), reference.clone()],
        &[],
        &env,
        cancelled,
    )?;
    let reference_file = reference_file(&p.cwd, &Path::new(".git").join(&reference))?;
    let head = oid(git(
        p,
        &strings(&["rev-parse", "--verify", "HEAD^{commit}"]),
        &[],
        &env,
        cancelled,
    )?)?;
    let index = plain(&p.cwd, Path::new(".git/index"), TOTAL_LIMIT as u64)?;
    clean_index(p, &env, cancelled)?;
    let reflogs = reflog_nodes(&p.cwd, &reference)?;
    let mut files = Vec::new();
    let mut sources = Vec::new();
    let mut source_nodes = Vec::new();
    let mut total = 0;
    for path in paths {
        let (file, bytes, node) = source(&p.cwd, path)?;
        total += bytes.as_ref().map_or(0, Vec::len);
        if total > TOTAL_LIMIT {
            return Err("Selected source bytes exceed 32 MiB".into());
        }
        files.push(file);
        sources.push(bytes);
        source_nodes.push(node);
    }
    let review = Review {
        head,
        reference,
        index_sha256: hash(&index),
        config_sha256: hash(&config),
        author,
        email,
        timestamp,
        git_version,
        files,
        frozen: Some(Arc::new(Frozen {
            config,
            index,
            head_file,
            reference_file,
            git_identity: native::directory_identity(&p.cwd.join(".git"))
                .map_err(|e| e.to_string())?,
            sources,
            source_nodes,
            reflogs,
            environment: env,
        })),
    };
    revalidate(p, &review, cancelled, false)?;
    Ok(review)
}
fn revalidate(
    p: &Prepared,
    r: &Review,
    cancelled: &dyn Fn() -> bool,
    owns_index_config_locks: bool,
) -> Result<(), String> {
    let f = r
        .frozen
        .as_ref()
        .ok_or("Restored selected-commit records contain no execution authority; author anew")?;
    ordinary(&p.cwd, cancelled, owns_index_config_locks)?;
    if native::directory_identity(&p.cwd.join(".git")).map_err(|e| e.to_string())? != f.git_identity
        || plain(&p.cwd, Path::new(".git/config"), 1024 * 1024)? != f.config
        || plain(&p.cwd, Path::new(".git/index"), TOTAL_LIMIT as u64)? != f.index
        || plain(&p.cwd, Path::new(".git/HEAD"), 4096)? != f.head_file
        || reference_file(&p.cwd, &Path::new(".git").join(&r.reference))? != f.reference_file
        || oid(git(
            p,
            &["rev-parse".into(), "--verify".into(), r.reference.clone()],
            &[],
            &f.environment,
            cancelled,
        )?)? != r.head
    {
        return Err("HEAD/ref/index/configuration drift; selected commit refused".into());
    }
    for (i, file) in r.files.iter().enumerate() {
        let (current, bytes, node) = source(&p.cwd, &file.path)?;
        if current != *file || bytes != f.sources[i] || node != f.source_nodes[i] {
            return Err(
                "Selected source path/native identity/content/mode drift; commit refused".into(),
            );
        }
    }
    if reflog_nodes(&p.cwd, &r.reference)? != f.reflogs {
        return Err(
            "Touched reflog file/ancestor native identities changed; selected commit refused"
                .into(),
        );
    }
    Ok(())
}
struct OwnedPath(Option<PathBuf>);
impl Drop for OwnedPath {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}
fn lock(path: PathBuf) -> Result<(OwnedPath, File), String> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("Git lock refused: {e}"))?;
    Ok((OwnedPath(Some(path)), file))
}
pub(super) fn execute(p: &Prepared, cancelled: &dyn Fn() -> bool) -> Outcome {
    let mut out = Outcome::new();
    let CommandSpec::SelectedCommit {
        message,
        review: Some(r),
        ..
    } = &p.spec
    else {
        out.error = Some("No frozen selected-commit review".into());
        return out;
    };
    let Some(f) = r.frozen.as_ref() else {
        out.error = Some("Restored review has no authority".into());
        return out;
    };
    let result = (|| -> Result<(), String> {
        revalidate(p, r, cancelled, false)?;
        let (mut index_guard, mut index_lock) = lock(p.cwd.join(".git/index.lock"))?;
        // Git's ref transaction must own HEAD.lock, including for --no-deref branch updates.
        // We own only index/config locks and revalidate frozen HEAD/ref before publication.
        let (_config_guard, _config_lock) = lock(p.cwd.join(".git/config.lock"))?;
        let scratch_relative = format!(".git/eevee-index-{}", p.run_id);
        let scratch = p.cwd.join(&scratch_relative);
        if scratch.symlink_metadata().is_ok() {
            return Err("Isolated index already exists".into());
        }
        let _scratch_guard = OwnedPath(Some(scratch.clone()));
        let _scratch_lock_guard = OwnedPath(Some(scratch.with_extension("lock")));
        let mut env = f.environment.clone();
        env.push(("GIT_INDEX_FILE".into(), scratch_relative));
        git(
            p,
            &["read-tree".into(), r.head.clone()],
            &[],
            &env,
            cancelled,
        )?;
        let mut updates = Vec::new();
        for (i, file) in r.files.iter().enumerate() {
            let id = if let Some(bytes) = &f.sources[i] {
                oid(git(
                    p,
                    &strings(&["hash-object", "-w", "--no-filters", "--stdin"]),
                    bytes,
                    &env,
                    cancelled,
                )?)?
            } else {
                "0".repeat(40)
            };
            updates.extend_from_slice(if f.sources[i].is_some() {
                file.mode.as_bytes()
            } else {
                b"0"
            });
            updates.push(b' ');
            updates.extend_from_slice(id.as_bytes());
            updates.push(b'\t');
            #[cfg(unix)]
            updates.extend_from_slice(
                file.path
                    .to_str()
                    .ok_or("Non-Unicode selected path")?
                    .as_bytes(),
            );
            #[cfg(windows)]
            updates.extend_from_slice(
                file.path
                    .to_str()
                    .ok_or("Non-Unicode selected path")?
                    .replace('\\', "/")
                    .as_bytes(),
            );
            updates.push(0);
        }
        git(
            p,
            &strings(&["update-index", "-z", "--index-info"]),
            &updates,
            &env,
            cancelled,
        )?;
        let tree = oid(git(p, &strings(&["write-tree"]), &[], &env, cancelled)?)?;
        let original_tree = oid(git(
            p,
            &strings(&["rev-parse", "HEAD^{tree}"]),
            &[],
            &env,
            cancelled,
        )?)?;
        if tree == original_tree {
            return Err("Selected whole-file changeset is empty".into());
        }
        let commit = oid(git(
            p,
            &[
                "commit-tree".into(),
                tree,
                "-p".into(),
                r.head.clone(),
                "--no-gpg-sign".into(),
            ],
            message.as_bytes(),
            &env,
            cancelled,
        )?)?;
        out.created_commit = Some(commit.clone());
        let next_index = plain(
            &p.cwd,
            scratch
                .strip_prefix(&p.cwd)
                .map_err(|_| "Isolated index escaped project")?,
            TOTAL_LIMIT as u64,
        )?;
        out.expected_index_sha256 = Some(hash(&next_index));
        index_lock
            .write_all(&next_index)
            .map_err(|e| e.to_string())?;
        index_lock.sync_all().map_err(|e| e.to_string())?;
        revalidate(p, r, cancelled, true)?;
        if cancelled() {
            return Err("Cancelled before publication; objects may remain unreachable".into());
        }
        out.publication_started = true;
        git(
            p,
            &[
                "update-ref".into(),
                "--no-deref".into(),
                r.reference.clone(),
                commit,
                r.head.clone(),
            ],
            &[],
            &env,
            cancelled,
        )?;
        out.ref_published = true;
        // Ref is published first. A rename failure is a truthful partial result, never rolled back.
        drop(index_lock);
        fs::rename(p.cwd.join(".git/index.lock"), p.cwd.join(".git/index"))
            .map_err(|e| format!("Ref published but index publication failed: {e}"))?;
        index_guard.0 = None; // The published lock path may now be acquired by another Git process.
        out.index_published = true;
        Ok(())
    })();
    if let Err(e) = result {
        out.owned_tree_exited = !e.contains("Owned Git tree outcome unknown:");
        out.error = Some(e);
    }
    // Observe even after cancellation. These bounded read-only plumbing queries cannot publish.
    let head = git(
        p,
        &strings(&["rev-parse", "--verify", "HEAD"]),
        &[],
        &f.environment,
        &|| false,
    )
    .and_then(oid);
    let reference = git(
        p,
        &["rev-parse".into(), "--verify".into(), r.reference.clone()],
        &[],
        &f.environment,
        &|| false,
    )
    .and_then(oid);
    if [&head, &reference].iter().any(|result| {
        result
            .as_ref()
            .is_err_and(|e| e.contains("Owned Git tree outcome unknown:"))
    }) {
        out.owned_tree_exited = false;
    }
    out.observed_head = head.ok();
    out.observed_reference = reference.ok();
    out.observed_index_sha256 = plain(&p.cwd, Path::new(".git/index"), TOTAL_LIMIT as u64)
        .ok()
        .map(|b| hash(&b));
    // Publication claims require actual observations, not only a successful command status.
    out.ref_published =
        out.created_commit.is_some() && out.observed_reference == out.created_commit;
    out.index_published = out.expected_index_sha256.is_some()
        && out.observed_index_sha256 == out.expected_index_sha256;
    out
}
pub fn disclosure(review: Option<&Review>) -> String {
    let mut text = format!("{DISCLOSURE}\n");
    if let Some(r) = review {
        text.push_str(&format!("HEAD {}\nRef {}\nInitial index SHA-256 {}\nConfiguration SHA-256 {} (local config frozen; global/system and inherited Git/environment effects excluded)\nAuthor AND committer: {} <{}>\nExact author/committer timestamp {} +0000\n",r.head,r.reference,r.index_sha256,r.config_sha256,serde_json::to_string(&r.author).unwrap(),serde_json::to_string(&r.email).unwrap(),r.timestamp));
        text.push_str(&format!(
            "Qualified {} (minimum 2.36 for non-executable fsmonitor=false semantics)\n",
            r.git_version
        ));
        for file in &r.files {
            text.push_str(&format!(
                "Whole file {} · mode {} · {} bytes · SHA-256 {}\n",
                serde_json::to_string(&file.path).unwrap(),
                file.mode,
                file.bytes,
                file.sha256.as_deref().unwrap_or("DELETED")
            ));
        }
    }
    text
}

#[cfg(test)]
mod tests;
