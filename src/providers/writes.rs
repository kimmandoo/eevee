//! Worker-owned bounded project observations and exact create/replace authority.
//! Retained handles and stale checks are not an OS sandbox or compare-and-swap.
//! Name admission is read-only: native IDs and known directory capabilities
//! reject established aliases before mutation. Unknown collation never merges
//! identities: exact frozen entries and no-overwrite publication remain the
//! authority, so an alias observed after a sibling write is refused as stale.
//! A known exact leaf outcome is separate from unrelated scope becoming stale.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::workspace::context;

#[cfg(unix)]
#[path = "writes/posix.rs"]
mod platform;
#[cfg(windows)]
#[path = "writes/windows.rs"]
mod platform;

mod tools;
pub(crate) use tools::{FileTool, ToolKind, definitions};

pub(crate) const TOOL_NAME: &str = "eevee_write_file";
pub(crate) const MAX_FILES: usize = 8;
pub(crate) const MAX_BYTES: usize = 64 * 1024;
const REVIEW_BYTES: usize = 256 * 1024;
const REVIEW_LINES: usize = 32_768;

// Capability-backed ASCII distinctions and conservative macOS normalization
// rejection are admission checks, never lowercased filesystem identities.
// Unknown Unicode collation still requires exact handle/absence validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NamePolicy {
    ByteSensitive,
    #[cfg(any(target_os = "macos", windows))]
    AsciiSensitive,
    AsciiInsensitive,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NameRelation {
    Distinct,
    Equivalent,
    Unknown,
}
impl NamePolicy {
    fn compare(self, left: &std::ffi::OsStr, right: &std::ffi::OsStr) -> NameRelation {
        if left == right {
            return NameRelation::Equivalent;
        }
        if self == Self::ByteSensitive {
            return NameRelation::Distinct;
        }
        let (Some(left), Some(right)) = (left.to_str(), right.to_str()) else {
            return NameRelation::Unknown;
        };
        if cfg!(target_os = "macos") && left.nfd().eq(right.nfd()) {
            return NameRelation::Equivalent;
        }
        if !left.is_ascii() || !right.is_ascii() {
            return NameRelation::Unknown;
        }
        match self {
            #[cfg(any(target_os = "macos", windows))]
            Self::AsciiSensitive => NameRelation::Distinct,
            Self::AsciiInsensitive if left.eq_ignore_ascii_case(right) => NameRelation::Equivalent,
            Self::AsciiInsensitive => NameRelation::Distinct,
            _ => NameRelation::Unknown,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriteKind {
    Create,
    Replace,
}
impl WriteKind {
    fn name(self) -> &'static str {
        match self {
            Self::Create => "Create",
            Self::Replace => "Replace",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WriteProposal {
    pub path: PathBuf,
    pub content: String,
    pub patch: String,
    pub kind: WriteKind,
    pub directories: Vec<PathBuf>,
}
impl WriteProposal {
    pub(crate) fn description(&self) -> String {
        let mut description = format!(
            "{} file\nPath: {}\n{}\nOnly this exact full UTF-8 content is allowed once. Existing baselines include dirty working bytes; preserve unrelated user changes. No provider-native write grant, commit or verification.\n",
            self.kind.name(),
            self.path.display(),
            if self.directories.is_empty() {
                "Directories: none".to_owned()
            } else {
                format!(
                    "Create directories: {}",
                    self.directories
                        .iter()
                        .map(|directory| directory.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        );
        if !self.directories.is_empty() {
            description.push_str("Only still-absent approved parents are created; unchanged parents created by this request for selected siblings may be reused.\n");
        }
        description.push_str(&self.patch);
        description
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WriteResult {
    pub message: String,
    pub kind: WriteKind,
    pub path: PathBuf,
    pub directories: Vec<PathBuf>,
    pub source_sha256: [u8; 32],
}

impl WriteResult {
    pub(crate) fn receipt(&self) -> String {
        json!({"status":"success","path":self.path,"source_sha256":self.source_sha256}).to_string()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WriteFailure {
    pub message: String,
    pub uncertain: bool,
}
impl fmt::Display for WriteFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for WriteFailure {}
impl From<String> for WriteFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            uncertain: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Baseline {
    stamp: platform::FileStamp,
    text: String,
    hash: [u8; 32],
    digest: String,
}
#[derive(Clone, Debug)]
struct Target {
    path: PathBuf,
    missing_parents: Vec<PathBuf>,
    baseline: Option<Baseline>,
}
impl Target {
    fn kind(&self) -> WriteKind {
        if self.baseline.is_some() {
            WriteKind::Replace
        } else {
            WriteKind::Create
        }
    }

    fn patch(&self, content: &str) -> String {
        let mut patch = match &self.baseline {
            Some(baseline) => format!(
                "--- a/{}\n+++ b/{}\n@@ exact full-file replacement; baseline sha256={} @@\n",
                self.path.display(),
                self.path.display(),
                baseline.digest
            ),
            None => format!(
                "--- /dev/null\n+++ b/{}\n@@ exact new UTF-8 file; reviewed target absent @@\n",
                self.path.display()
            ),
        };
        let baseline = self
            .baseline
            .as_ref()
            .map_or("", |baseline| baseline.text.as_str());
        let show_endings = baseline.contains("\r\n") || content.contains("\r\n");
        patch.push_str(&format!(
            "Line endings: baseline {}; proposed {}. When either side contains CRLF, every terminated line has a [CRLF] or [LF] review suffix (not file bytes); carriage-return controls are never rendered.\n",
            line_endings(baseline),
            line_endings(content),
        ));
        for (prefix, text) in [('-', baseline), ('+', content)] {
            for line in text.split_inclusive('\n') {
                patch.push(prefix);
                if let Some(body) = line.strip_suffix("\r\n") {
                    patch.push_str(body);
                    patch.push_str(" [CRLF]\n");
                } else if show_endings && let Some(body) = line.strip_suffix('\n') {
                    patch.push_str(body);
                    patch.push_str(" [LF]\n");
                } else {
                    patch.push_str(line);
                    if !line.ends_with('\n') {
                        patch.push_str("\n\\ No newline at end of file\n");
                    }
                }
            }
        }
        if content.is_empty() && self.kind() == WriteKind::Create {
            patch.push_str("(new empty file, 0 bytes)\n");
        }
        patch
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WriteSet {
    source_root: PathBuf,
    root: PathBuf,
    directories: BTreeMap<PathBuf, platform::Directory>,
    owned_directories: BTreeSet<PathBuf>,
    targets: Vec<Target>,
    workspace: bool,
    protected_paths: Vec<PathBuf>,
    exclusions: Vec<PathBuf>,
    observed: BTreeMap<PathBuf, std::sync::Arc<Baseline>>,
}

fn line_endings(text: &str) -> String {
    let crlf = text.matches("\r\n").count();
    let lf = text.bytes().filter(|byte| *byte == b'\n').count() - crlf;
    format!(
        "{crlf} CRLF, {lf} LF, {} bytes, {}",
        text.len(),
        if text.is_empty() {
            "empty"
        } else if text.ends_with('\n') {
            "final newline"
        } else {
            "no final newline"
        }
    )
}

fn review_fits(proposal: &WriteProposal) -> bool {
    let description = proposal.description();
    description.len() <= REVIEW_BYTES && description.lines().count() <= REVIEW_LINES
}

fn safe_text(text: &str) -> bool {
    !text.as_bytes().iter().enumerate().any(|(index, byte)| {
        *byte == b'\r' && text.as_bytes().get(index + 1) != Some(&b'\n')
    }) && !text.chars().any(|character| {
        (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
            || matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    }) && crate::workspace::privacy::redact(text) == text
}

/// Only explicit input refusals are retryable; native capture/identity failures
/// are never treated as model-correctable merely because no write occurred.
pub(crate) fn semantic_proposal_refusal(message: &str) -> bool {
    matches!(
        message,
        "Protected, excluded, duplicate, conflicting or non-literal proposed write path refused"
            | "Only an exact reviewed selected path may be written"
            | "Controlled implementation permits at most 8 distinct file targets"
            | "Selected working bytes are oversized, binary, credential-bearing or contain unsafe controls"
            | "Filesystem-equivalent selected target/absent-parent aliases refused; select unambiguous paths"
            | "Filesystem-equivalent or conflicting selected write paths refused"
            | "Unsafe or oversized proposed UTF-8 content refused"
            | "Exact content/path/parent review exceeds 256 KiB or 32768 lines; reduce the change"
            | "Source changed since read; read the file again before proposing a write"
            | "Edit requires a preceding successful read of this exact path"
            | "Edit sha256 does not match the preceding source read; read again"
            | "Edit old_text must be nonempty and match exactly once; read and select an unambiguous literal span"
            | "Edit text exceeds the 64 KiB file bound"
            | "File tool arguments must exactly match the advertised schema"
    )
}

fn allowed_path(path: &Path, exclusions: &[PathBuf]) -> bool {
    let Some(text) = path.to_str() else {
        return false;
    };
    if text.is_empty()
        || text.len() > REVIEW_BYTES
        || !safe_text(text)
        || text.chars().any(char::is_control)
        || (!cfg!(windows) && text.contains('\\'))
        || text.contains(':')
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || context::excluded(path, exclusions)
    {
        return false;
    }
    // Reject normalized aliases, ADS/device names and trailing-name aliases.
    // Both native Windows separators are valid, but no empty/dot component is.
    text.split(|character| character == '/' || cfg!(windows) && character == '\\')
        .all(|part| {
            let lower = part.to_ascii_lowercase();
            let device = lower.split('.').next().unwrap_or_default();
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !part.contains(['<', '>', '"', '|', '?', '*'])
                && !matches!(device, "con" | "prn" | "aux" | "nul" | "conin$" | "conout$")
                && !((device.starts_with("com") || device.starts_with("lpt"))
                    && matches!(
                        &device[3..],
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    ))
                && !matches!(
                    lower.as_str(),
                    ".codex" | ".claude" | ".gemini" | ".opencode" | ".ds_store" | ".ev-install"
                )
                && !lower.starts_with("id_")
        })
}

fn absolute_frozen_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| "Cannot resolve private-data scope")?
            .join(path)
    };
    let mut ancestor = absolute.as_path();
    let mut missing = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                for name in missing.into_iter().rev() {
                    resolved.push(name);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or("Cannot resolve private-data scope")?
                        .to_os_string(),
                );
                ancestor = ancestor
                    .parent()
                    .ok_or("Cannot resolve private-data scope")?;
            }
            Err(_) => return Err("Cannot safely resolve private-data scope".into()),
        }
    }
}

fn protected_prefix(path: &Path, protected: &Path) -> bool {
    let mut actual = path.components();
    protected.components().all(|part| {
        actual.next().is_some_and(|candidate| {
            candidate
                .as_os_str()
                .to_string_lossy()
                .nfd()
                .flat_map(char::to_lowercase)
                .eq(part
                    .as_os_str()
                    .to_string_lossy()
                    .nfd()
                    .flat_map(char::to_lowercase))
        })
    })
}

fn read_baseline(file: &mut File) -> Result<Baseline, String> {
    let before = platform::file_stamp(file)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "Cannot seek selected working file")?;
    let mut text = String::new();
    (&mut *file)
        .take((MAX_BYTES + 1) as u64)
        .read_to_string(&mut text)
        .map_err(|_| "Selected working file must be readable UTF-8")?;
    if text.len() > MAX_BYTES || !safe_text(&text) {
        return Err("Selected working bytes are oversized, binary, credential-bearing or contain unsafe controls".into());
    }
    if platform::file_stamp(file)? != before {
        return Err("Selected working file changed while captured; review again".into());
    }
    let hash = Sha256::digest(text.as_bytes());
    let digest = format!("{hash:x}");
    Ok(Baseline {
        stamp: before,
        text,
        hash: hash.into(),
        digest,
    })
}

/// Read-only native name admission shared with the larger SDK frozen scope.
/// Callers retain their own privacy/exclusion, literal-name and count limits.
pub(crate) fn verify_path_names(root: &Path, paths: &[PathBuf]) -> Result<(), String> {
    let root = root
        .canonicalize()
        .map_err(|_| "Cannot bind selected name-admission root")?;
    let mut directories =
        BTreeMap::from([(PathBuf::new(), platform::Directory::open_root(&root)?)]);
    for path in paths {
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("Only relative normal selected path components are admitted".into());
        }
        let mut parent = PathBuf::new();
        for part in path.parent().unwrap_or(Path::new("")).components() {
            let child = parent.join(part);
            if !directories.contains_key(&child) {
                let directory = directories
                    .get(&parent)
                    .ok_or("Selected name-admission ancestor missing")?;
                let Some(directory) = directory.child(part.as_os_str())? else {
                    break;
                };
                directories.insert(child.clone(), directory);
            }
            parent = child;
        }
    }
    for directory in directories.values() {
        directory.verify()?;
    }
    verify_name_bindings(paths.iter().map(PathBuf::as_path), &directories)
}

fn verify_name_bindings<'a>(
    paths: impl Iterator<Item = &'a Path> + Clone,
    directories: &BTreeMap<PathBuf, platform::Directory>,
) -> Result<(), String> {
    for (index, left) in paths.clone().enumerate() {
        'comparison: for right in paths.clone().take(index) {
            let mut left_parent = PathBuf::new();
            let mut right_parent = PathBuf::new();
            let mut policy = NamePolicy::Unknown;
            let mut distinct = false;
            for (left_part, right_part) in left.components().zip(right.components()) {
                if let (Some(left_directory), Some(right_directory)) = (
                    directories.get(&left_parent),
                    directories.get(&right_parent),
                ) {
                    if !left_directory.same_object(right_directory) {
                        distinct = true;
                        break;
                    }
                    policy = left_directory.name_policy();
                }
                let left_child = left_parent.join(left_part);
                let right_child = right_parent.join(right_part);
                if left_child != right_child {
                    // Existing ancestors use actual native IDs, not spelling.
                    if let (Some(left_directory), Some(right_directory)) =
                        (directories.get(&left_child), directories.get(&right_child))
                    {
                        if !left_directory.same_object(right_directory) {
                            distinct = true;
                            break;
                        }
                    } else {
                        match policy.compare(left_part.as_os_str(), right_part.as_os_str()) {
                            NameRelation::Distinct => {
                                distinct = true;
                                break;
                            }
                            NameRelation::Unknown => continue 'comparison,
                            NameRelation::Equivalent => {
                                // Missing-parent ownership remains exact-spelling.
                                return Err("Filesystem-equivalent selected target/absent-parent aliases refused; select unambiguous paths".into());
                            }
                        }
                    }
                }
                left_parent = left_child;
                right_parent = right_child;
            }
            if !distinct {
                return Err(
                    "Filesystem-equivalent or conflicting selected write paths refused".into(),
                );
            }
        }
    }
    Ok(())
}

impl WriteSet {
    pub(crate) fn capture(
        root: &Path,
        data_dir: &Path,
        paths: &[PathBuf],
        exclusions: &[PathBuf],
    ) -> Result<Self, String> {
        if paths.is_empty() || paths.len() > MAX_FILES {
            return Err(
                "Controlled implementation requires 1–8 explicitly selected create/replace paths"
                    .into(),
            );
        }
        let mut set = Self::capture_workspace(root, data_dir, exclusions)?;
        set.admit_paths(paths)?;
        set.workspace = false;
        Ok(set)
    }

    pub(crate) fn capture_workspace(
        root: &Path,
        data_dir: &Path,
        exclusions: &[PathBuf],
    ) -> Result<Self, String> {
        let canonical = root
            .canonicalize()
            .map_err(|_| "Cannot bind the selected project root")?;
        let private_data = absolute_frozen_path(data_dir)?;
        let executable = std::env::current_exe()
            .map_err(|_| "Cannot identify the running executable")?
            .canonicalize()
            .map_err(|_| "Cannot bind the running executable")?;
        let set = Self {
            source_root: root.to_path_buf(),
            root: canonical.clone(),
            directories: BTreeMap::from([(
                PathBuf::new(),
                platform::Directory::open_root(&canonical)?,
            )]),
            owned_directories: BTreeSet::new(),
            targets: Vec::new(),
            workspace: true,
            protected_paths: vec![private_data, executable],
            exclusions: exclusions.to_vec(),
            observed: BTreeMap::new(),
        };
        set.verify()?;
        Ok(set)
    }

    /// Capture a whole proposal batch without changing disk or recapturing any
    /// existing authority. The candidate is committed only after every check.
    pub(crate) fn admit_paths(&mut self, paths: &[PathBuf]) -> Result<(), String> {
        self.verify()?;
        let mut additional = 0;
        for (index, path) in paths.iter().enumerate() {
            let known = self.targets.iter().any(|target| target.path == *path);
            if !allowed_path(path, &self.exclusions)
                || self
                    .protected_paths
                    .iter()
                    .any(|protected| protected_prefix(&self.root.join(path), protected))
                || paths[..index]
                    .iter()
                    .any(|other| path.starts_with(other) || other.starts_with(path))
                || self.targets.iter().any(|target| {
                    target.path != *path
                        && (path.starts_with(&target.path) || target.path.starts_with(path))
                })
            {
                return Err("Protected, excluded, duplicate, conflicting or non-literal proposed write path refused".into());
            }
            if !known {
                if !self.workspace {
                    return Err("Only an exact reviewed selected path may be written".into());
                }
                additional += 1;
            }
        }
        if self.targets.len() + additional > MAX_FILES {
            return Err("Controlled implementation permits at most 8 distinct file targets".into());
        }
        if additional == 0 {
            return Ok(());
        }
        let mut set = self.clone();
        for path in paths {
            if set.targets.iter().any(|target| target.path == *path) {
                continue;
            }
            let mut parent = PathBuf::new();
            let mut missing_parents = Vec::new();
            for part in path.parent().unwrap_or(Path::new("")).components() {
                let child = parent.join(part.as_os_str());
                if !missing_parents.is_empty() {
                    missing_parents.push(child.clone());
                } else if !set.directories.contains_key(&child) {
                    let directory = set
                        .directories
                        .get(&parent)
                        .ok_or("Selected ancestor baseline missing")?;
                    match directory.child(part.as_os_str())? {
                        Some(directory) => {
                            set.directories.insert(child.clone(), directory);
                        }
                        None => missing_parents.push(child.clone()),
                    }
                }
                parent = child;
            }
            let baseline = if missing_parents.is_empty() {
                let directory = set
                    .directories
                    .get(&parent)
                    .ok_or("Selected parent baseline missing")?;
                let name = path.file_name().ok_or("Selected write name missing")?;
                if directory.absent(name)? {
                    None
                } else {
                    Some(read_baseline(&mut directory.open_file(name, false)?)?)
                }
            } else {
                None
            };
            set.targets.push(Target {
                path: path.clone(),
                missing_parents,
                baseline,
            });
            if let Some(observed) = set.observed.get(path)
                && set
                    .targets
                    .last()
                    .and_then(|target| target.baseline.as_ref())
                    != Some(observed.as_ref())
            {
                return Err(
                    "Source changed since read; read the file again before proposing a write"
                        .into(),
                );
            }
        }
        set.verify()?;
        *self = set;
        Ok(())
    }

    fn verify_aliases(&self) -> Result<(), String> {
        verify_name_bindings(
            self.targets.iter().map(|target| target.path.as_path()),
            &self.directories,
        )
    }

    pub(crate) fn verify(&self) -> Result<(), String> {
        if self
            .source_root
            .canonicalize()
            .map_err(|_| "Project root disappeared; selected scope is stale")?
            != self.root
        {
            return Err("Project root changed since review; selected scope is stale".into());
        }
        for directory in self.directories.values() {
            directory.verify()?;
        }
        self.verify_aliases()?;
        for target in &self.targets {
            for missing in &target.missing_parents {
                if self.owned_directories.contains(missing) {
                    continue;
                }
                let parent = missing.parent().unwrap_or(Path::new(""));
                if let Some(directory) = self.directories.get(parent)
                    && !directory
                        .absent(missing.file_name().ok_or("Selected parent name missing")?)?
                {
                    return Err(
                        "Reviewed absent parent appeared; fresh scope and approval required".into(),
                    );
                }
            }
            let parent = target.path.parent().unwrap_or(Path::new(""));
            if let Some(directory) = self.directories.get(parent) {
                let name = target
                    .path
                    .file_name()
                    .ok_or("Selected write name missing")?;
                match &target.baseline {
                    Some(expected)
                        if read_baseline(&mut directory.open_file(name, false)?)? != *expected =>
                    {
                        return Err("Selected working bytes or file identity changed; selected scope is stale".into());
                    }
                    None if !directory.absent(name)? => {
                        return Err(
                            "Reviewed absent target appeared; fresh scope and approval required"
                                .into(),
                        );
                    }
                    _ => {}
                }
            } else if target.baseline.is_some() {
                return Err(
                    "Selected existing-file parent baseline missing; selected scope is stale"
                        .into(),
                );
            }
        }
        Ok(())
    }

    pub(crate) fn payload(&self, excerpts: &[context::Excerpt]) -> Value {
        Value::Array(
            self.targets
                .iter()
                .map(|target| {
                    let mut entry = json!({
                        "path": target.path,
                        "kind": if target.baseline.is_some() { "replace" } else { "create" },
                        "missing_parents": target.missing_parents,
                    });
                    if let Some(baseline) = &target.baseline {
                        entry["sha256"] = json!(baseline.digest);
                        let capture = excerpts.iter().find(|excerpt| {
                            excerpt.available
                                && Path::new(&excerpt.root) == self.root
                                && Path::new(&excerpt.path) == target.path
                                && excerpt.start_line == 1
                                && excerpt.text == baseline.text
                                && excerpt.source_sha256 == baseline.hash
                        });
                        if let Some(capture) = capture {
                            entry["working_content_same_as_capture"] = json!(capture.id);
                        } else {
                            entry["working_content"] = json!(baseline.text);
                        }
                    } else {
                        entry["expected"] = json!("absent, not an empty existing file");
                    }
                    entry
                })
                .collect(),
        )
    }

    pub(crate) fn propose(&self, path: PathBuf, content: String) -> Result<WriteProposal, String> {
        self.verify()?;
        let target = self
            .targets
            .iter()
            .find(|target| target.path == path)
            .ok_or("Only an exact reviewed selected path may be written")?;
        if let Some(observed) = self.observed.get(&path)
            && self
                .targets
                .iter()
                .find(|target| target.path == path)
                .and_then(|target| target.baseline.as_ref())
                != Some(observed.as_ref())
        {
            return Err(
                "Source changed since read; read the file again before proposing a write".into(),
            );
        }
        if content.len() > MAX_BYTES || !safe_text(&content) {
            return Err("Unsafe or oversized proposed UTF-8 content refused".into());
        }
        let kind = target.kind();
        let patch = target.patch(&content);
        let proposal = WriteProposal {
            path,
            content,
            patch,
            kind,
            directories: target.missing_parents.clone(),
        };
        if !review_fits(&proposal) {
            return Err(
                "Exact content/path/parent review exceeds 256 KiB or 32768 lines; reduce the change".into(),
            );
        }
        Ok(proposal)
    }

    pub(crate) fn apply(&mut self, proposal: &WriteProposal) -> Result<WriteResult, WriteFailure> {
        self.verify()?;
        let target = self
            .targets
            .iter()
            .find(|target| target.path == proposal.path)
            .ok_or("Only an exact reviewed selected path may be written".to_owned())?;
        if proposal.content.len() > MAX_BYTES
            || !safe_text(&proposal.content)
            || proposal.kind != target.kind()
            || proposal.directories != target.missing_parents
            || proposal.patch != target.patch(&proposal.content)
            || !review_fits(proposal)
        {
            return Err("Exact reviewed create/replace proposal changed; no write"
                .to_owned()
                .into());
        }
        let mut created = Vec::new();
        let outcome = self.apply_exact(proposal, &mut created);
        match outcome {
            Ok(source_sha256) => Ok(WriteResult {
                message: format!(
                    "Applied reviewed {} to {}{}; inspect the working diff. No local check, commit or verification occurred.",
                    proposal.kind.name(),
                    proposal.path.display(),
                    if created.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "; created parents: {}",
                            created
                                .iter()
                                .map(|path: &PathBuf| path.to_string_lossy())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                ),
                kind: proposal.kind,
                path: proposal.path.clone(),
                directories: created,
                source_sha256,
            }),
            Err(mut failure) => {
                if !created.is_empty() {
                    failure.uncertain = true;
                    failure.message.push_str(
                        "; approved parent directories were already created and remain: ",
                    );
                    failure.message.push_str(
                        &created
                            .iter()
                            .map(|path| path.to_string_lossy())
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                }
                Err(failure)
            }
        }
    }

    fn apply_exact(
        &mut self,
        proposal: &WriteProposal,
        created: &mut Vec<PathBuf>,
    ) -> Result<[u8; 32], WriteFailure> {
        for missing in &proposal.directories {
            if self.owned_directories.contains(missing) {
                continue;
            }
            let parent = missing.parent().unwrap_or(Path::new(""));
            let directory = self
                .directories
                .get(parent)
                .ok_or("Approved parent baseline missing".to_owned())?;
            directory.verify()?;
            let child = directory.create_directory(
                missing
                    .file_name()
                    .ok_or("Approved parent name missing".to_owned())?,
            )?;
            self.directories.insert(missing.clone(), child);
            self.owned_directories.insert(missing.clone());
            created.push(missing.clone());
        }
        // New directory creation changes directory times, not their native identities.
        // Recheck every other selected baseline before publishing/writing any leaf.
        self.verify()?;
        let parent = proposal.path.parent().unwrap_or(Path::new(""));
        let directory = self
            .directories
            .get(parent)
            .ok_or("Selected parent baseline missing".to_owned())?;
        let name = proposal
            .path
            .file_name()
            .ok_or("Selected file name missing".to_owned())?;
        let mut written = if proposal.kind == WriteKind::Create {
            directory.publish(name, proposal.content.as_bytes())?
        } else {
            let expected = self
                .targets
                .iter()
                .find(|target| target.path == proposal.path)
                .and_then(|target| target.baseline.as_ref())
                .ok_or("Selected replacement baseline missing".to_owned())?;
            let mut file = directory.open_file(name, true)?;
            if read_baseline(&mut file)? != *expected {
                return Err(
                    "Writable file identity or working content changed; leaf replacement refused"
                        .to_owned()
                        .into(),
                );
            }
            self.verify()?;
            file.seek(SeekFrom::Start(0))
                .map_err(|_| "Cannot seek writable file; leaf replacement refused".to_owned())?;
            file.write_all(proposal.content.as_bytes())
                .and_then(|_| file.set_len(proposal.content.len() as u64))
                .and_then(|_| file.sync_all())
                .map_err(|error| WriteFailure { message: format!("Approved replacement failed after writing began; partial local changes possible: {error}"), uncertain: true })?;
            file
        };
        #[cfg(test)]
        tests::after_leaf_write();
        let actual = (|| -> Result<Baseline, String> {
            directory.verify()?;
            let retained = read_baseline(&mut written)?;
            let named = read_baseline(&mut directory.open_file(name, false)?)?;
            if retained != named {
                return Err("Written file identity/content no longer matches its selected name".into());
            }
            Ok(retained)
        })().map_err(|message| WriteFailure { message: format!("Write occurred but its resulting identity/content could not be established: {message}"), uncertain: true })?;
        if actual.text != proposal.content {
            return Err(WriteFailure { message: "Write occurred but post-write content differs; inspect local changes, outcome unknown".into(), uncertain: true });
        }
        let target = self
            .targets
            .iter_mut()
            .find(|target| target.path == proposal.path)
            .ok_or_else(|| WriteFailure {
                message: "Write occurred but selected baseline was lost".into(),
                uncertain: true,
            })?;
        let source_sha256 = actual.hash;
        target.baseline = Some(actual);
        target.missing_parents.clear();
        self.observed.remove(&proposal.path);
        // Other selected targets may have changed after this exact leaf write.
        // They make future proposals stale, not this established local outcome.
        Ok(source_sha256)
    }
}

#[cfg(test)]
mod tests;
