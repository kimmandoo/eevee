//! Worker-owned bounded project observations and exact single-file mutation authority.
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
#[cfg(test)]
pub(crate) use tools::Replacement;
pub(crate) use tools::{FileTool, ToolKind, definitions};

pub(crate) const TOOL_NAME: &str = "eevee_write_file";
pub(crate) const MAX_FILES: usize = 32;
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
    Delete,
    Rename,
}
impl WriteKind {
    fn name(self) -> &'static str {
        match self {
            Self::Create => "Create",
            Self::Replace => "Replace",
            Self::Delete => "Delete",
            Self::Rename => "Rename",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WriteProposal {
    pub path: PathBuf,
    pub content: String,
    pub patch: String,
    pub kind: WriteKind,
    pub destination: Option<PathBuf>,
    syntax: Option<SyntaxAnalysis>,
    binding: [u8; 32],
    pub directories: Vec<PathBuf>,
}
impl WriteProposal {
    fn fingerprint(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update([
            self.kind as u8,
            self.syntax
                .as_ref()
                .map_or(0, |analysis| analysis.review as u8 + 1),
        ]);
        let fields = [
            self.path.as_os_str().as_encoded_bytes(),
            self.destination
                .as_ref()
                .map_or(&[][..], |path| path.as_os_str().as_encoded_bytes()),
            self.content.as_bytes(),
            self.patch.as_bytes(),
        ];
        for field in fields.into_iter().chain(
            self.directories
                .iter()
                .map(|path| path.as_os_str().as_encoded_bytes()),
        ) {
            hash.update((field.len() as u64).to_le_bytes());
            hash.update(field);
        }
        hash.finalize().into()
    }

    fn sealed(mut self) -> Self {
        self.binding = self.fingerprint();
        self
    }

    pub(crate) fn file_review(&self) -> super::FileReview {
        super::FileReview {
            path: self.path.to_string_lossy().into_owned(),
            kind: match self.kind {
                WriteKind::Create => super::FileReviewKind::Create,
                WriteKind::Replace => super::FileReviewKind::Modify,
                WriteKind::Delete => super::FileReviewKind::Delete,
                WriteKind::Rename => super::FileReviewKind::Rename,
            },
            destination: self
                .destination
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            syntax: self.syntax.as_ref().map(|analysis| analysis.review),
            directories: self
                .directories
                .iter()
                .map(|directory| directory.to_string_lossy().into_owned())
                .collect(),
            bytes: self.content.len(),
            excerpt: super::FileReview::excerpt(&self.content),
            edit: false,
            external: false,
        }
    }
    pub(crate) fn preview(&self, complete: bool) -> String {
        let content =
            super::file_preview::display(&self.path.to_string_lossy(), &self.content, complete);
        match self.kind {
            WriteKind::Delete => format!(
                "DELETE exact regular file: {}\nIrreversible removal. Content below is the reviewed ORIGINAL source, not a proposed new file.\n{content}",
                self.path.display()
            ),
            WriteKind::Rename => format!(
                "RENAME exact regular file: {} -> {}\nNever overwrite. Content below is the unchanged ORIGINAL source.\n{content}",
                self.path.display(),
                self.destination
                    .as_ref()
                    .map_or_else(String::new, |path| path.display().to_string())
            ),
            _ => content,
        }
    }
    pub(crate) fn description(&self) -> String {
        let mut description = format!(
            "{} file\nPath: {}\n{}\nOnly this exact reviewed file operation is allowed once. Existing baselines include dirty working bytes; preserve unrelated user changes. No provider-native write grant, commit or verification.\n",
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
        match self.kind {
            WriteKind::Delete => description.push_str("IRREVERSIBLE: remove only this exact regular file. No recycle bin, recursive deletion or rollback is provided.\n"),
            WriteKind::Rename => {
                description.push_str(&format!("Destination: {}\nMove only this exact regular file without overwriting any destination. Literal content and supported file metadata are retained; no import rewriting or rollback.\n", self.destination.as_ref().map_or_else(String::new, |path| path.display().to_string())));
            }
            _ => {}
        }
        if let Some(analysis) = &self.syntax {
            description.push_str(analysis.description());
            description.push_str("Local parsing is advisory only: not compiled, type-correct or verified. No automatic repair or subprocess is started.\n");
        }
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
    pub destination: Option<PathBuf>,
    pub directories: Vec<PathBuf>,
    pub source_sha256: Option<[u8; 32]>,
    pub destination_sha256: Option<[u8; 32]>,
    pub original_source_sha256: Option<[u8; 32]>,
}

impl WriteResult {
    pub(crate) fn receipt(&self) -> String {
        json!({"status":"success","effect":"applied","kind":format!("{:?}",self.kind),"path":self.path,"destination":self.destination,"source_state":if matches!(self.kind,WriteKind::Delete | WriteKind::Rename) {"absent"} else {"present"},"original_source_sha256":self.original_source_sha256,"source_sha256":self.source_sha256,"destination_sha256":self.destination_sha256,"message":self.message}).to_string()
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceIdentity {
    stamp: platform::FileStamp,
    hash: [u8; 32],
}
impl SourceIdentity {
    fn matches(&self, baseline: &Baseline) -> bool {
        self.stamp == baseline.stamp && self.hash == baseline.hash
    }
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

impl Target {
    fn mutation_patch(&self, kind: WriteKind, destination: Option<&Path>, content: &str) -> String {
        match kind {
            WriteKind::Delete => format!(
                "DELETE exact regular file {}; baseline sha256={}; irreversible, no recycle bin.\n{}",
                self.path.display(),
                self.baseline
                    .as_ref()
                    .map_or("", |baseline| baseline.digest.as_str()),
                self.patch(""),
            ),
            WriteKind::Rename => format!(
                "RENAME exact regular file {} -> {}; destination must remain absent; no overwrite. Content below is unchanged.\n{}",
                self.path.display(),
                destination.map_or_else(String::new, |path| path.display().to_string()),
                self.patch(content),
            ),
            _ => self.patch(content),
        }
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
    readonly: bool,
    protected_paths: Vec<PathBuf>,
    exclusions: Vec<PathBuf>,
    observed: BTreeMap<PathBuf, std::sync::Arc<SourceIdentity>>,
    /// Request-local filename observation; shared by cloned proposal scopes.
    project_context: std::sync::Arc<std::sync::OnceLock<Value>>,
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

#[derive(Clone, Debug, PartialEq)]
struct SyntaxAnalysis {
    review: super::RustSyntaxReview,
}
impl SyntaxAnalysis {
    fn prepare(path: &Path, baseline: Option<&str>, content: &str) -> Self {
        use super::RustSyntaxReview::*;
        let review = if path.extension().is_none_or(|extension| extension != "rs") {
            Unsupported
        } else {
            std::panic::catch_unwind(|| {
                let proposed = syn::parse_file(content).is_ok();
                match baseline.map(|text| syn::parse_file(text).is_ok()) {
                    Some(true) if proposed => Parseable,
                    Some(true) => NewlyInvalid,
                    Some(false) if proposed => InvalidToValid,
                    Some(false) => PreExistingInvalid,
                    None if proposed => CreatedParseable,
                    None => CreatedInvalid,
                }
            })
            .unwrap_or(Unavailable)
        };
        Self { review }
    }
    fn description(&self) -> &'static str {
        use super::RustSyntaxReview::*;
        match self.review {
            NewlyInvalid => {
                "Rust syntax: NEWLY INVALID — baseline parses; proposed bytes fail parsing.\n"
            }
            PreExistingInvalid => {
                "Rust syntax: pre-existing invalid — baseline and proposed bytes fail parsing.\n"
            }
            InvalidToValid => {
                "Rust syntax: invalid-to-valid — baseline fails parsing; proposed bytes parse.\n"
            }
            Parseable => {
                "Rust syntax: baseline and proposed bytes parse. Advisory only: not compiled, type-correct or verified.\n"
            }
            CreatedParseable => {
                "Rust syntax: proposed new file parses; no baseline exists. Advisory only: not compiled, type-correct or verified.\n"
            }
            CreatedInvalid => "Rust syntax: proposed new file fails parsing; no baseline exists.\n",
            Unsupported => {
                "Syntax analysis: unsupported file type; no syntax, compilation or verification claim.\n"
            }
            Unavailable => {
                "Rust syntax analysis unavailable; no syntax, compilation or verification claim.\n"
            }
        }
    }
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
            | "Read-only file scope refuses mutation"
            | "Only an exact reviewed selected path may be written"
            | "Controlled implementation permits at most 32 distinct file targets"
            | "Selected working bytes are oversized, binary, credential-bearing or contain unsafe controls"
            | "Selected target must be a regular single-link UTF-8 file at most 64 KiB"
            | "Windows file exceeds the controlled-write byte limit."
            | "Filesystem-equivalent selected target/absent-parent aliases refused; select unambiguous paths"
            | "Filesystem-equivalent or conflicting selected write paths refused"
            | "Unsafe or oversized proposed UTF-8 content refused"
            | "Exact content/path/parent review exceeds 256 KiB or 32768 lines; reduce the change"
            | "Source changed since read; read the file again before proposing a write"
            | "Edit requires a preceding successful read of this exact path"
            | "Edit requires an unchanged admitted file at most 64 KiB; read again before proposing a write"
            | "Edit sha256 does not match the preceding source read; read again"
            | "Edit requires 1–32 literal replacements"
            | "Edit old_text must be nonempty and replacement text at most 64 KiB"
            | "Edit replacement spans overlap in the original source"
            | "Same-path and case-only rename are unsupported; no move occurred"
            | "Rename destination must be absent; existing entries are never overwritten"
            | "Cross-volume rename is unsupported; no move occurred"
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
                "Controlled implementation requires 1–32 explicitly selected create/replace paths"
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
        context::validate_project_root(&canonical)?;
        if canonical
            .to_str()
            .is_none_or(|path| !safe_text(path) || path.chars().any(char::is_control))
        {
            return Err("Current project identity contains private or unsafe text; it cannot be sent as context".into());
        }
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
            readonly: false,
            protected_paths: vec![private_data, executable],
            exclusions: exclusions.to_vec(),
            observed: BTreeMap::new(),
            project_context: Default::default(),
        };
        set.verify()?;
        Ok(set)
    }

    /// Frozen source access using the same root/privacy boundaries, with no
    /// mutation enrollment or execution authority.
    pub(crate) fn capture_readonly(
        root: &Path,
        data_dir: &Path,
        exclusions: &[PathBuf],
    ) -> Result<Self, String> {
        let mut set = Self::capture_workspace(root, data_dir, exclusions)?;
        set.readonly = true;
        set.workspace = false;
        Ok(set)
    }

    pub(crate) fn writing(&self) -> bool {
        !self.readonly
    }

    fn require_writing(&self) -> Result<(), String> {
        if self.readonly {
            Err("Read-only file scope refuses mutation".into())
        } else {
            Ok(())
        }
    }

    /// Capture a whole proposal batch without changing disk or recapturing any
    /// existing authority. The candidate is committed only after every check.
    pub(crate) fn admit_paths(&mut self, paths: &[PathBuf]) -> Result<(), String> {
        self.verify()?;
        if !paths.is_empty() {
            self.require_writing()?;
        }
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
            return Err(
                "Controlled implementation permits at most 32 distinct file targets".into(),
            );
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
                    .is_none_or(|baseline| !observed.matches(baseline))
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
        self.require_writing()?;
        let target = self
            .targets
            .iter()
            .find(|target| target.path == path)
            .ok_or("Only an exact reviewed selected path may be written")?;
        if let Some(observed) = self.observed.get(&path)
            && target
                .baseline
                .as_ref()
                .is_none_or(|baseline| !observed.matches(baseline))
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
        let syntax = Some(SyntaxAnalysis::prepare(
            &path,
            target
                .baseline
                .as_ref()
                .map(|baseline| baseline.text.as_str()),
            &content,
        ));
        let proposal = WriteProposal {
            path,
            content,
            patch,
            kind,
            destination: None,
            syntax,
            binding: [0; 32],
            directories: target.missing_parents.clone(),
        }
        .sealed();
        if !review_fits(&proposal) {
            return Err(
                "Exact content/path/parent review exceeds 256 KiB or 32768 lines; reduce the change".into(),
            );
        }
        Ok(proposal)
    }

    fn mutation_baseline(&self, path: &Path, digest: &str) -> Result<&Baseline, String> {
        let observed = self
            .observed
            .get(path)
            .ok_or("Edit requires a preceding successful read of this exact path")?;
        let baseline = self.targets.iter().find(|target| target.path == path)
            .and_then(|target| target.baseline.as_ref())
            .filter(|baseline| observed.matches(baseline))
            .ok_or("Edit requires an unchanged admitted file at most 64 KiB; read again before proposing a write")?;
        if baseline.digest != digest {
            return Err("Edit sha256 does not match the preceding source read; read again".into());
        }
        Ok(baseline)
    }

    fn propose_mutation(
        &self,
        path: PathBuf,
        digest: &str,
        destination: Option<PathBuf>,
    ) -> Result<WriteProposal, String> {
        self.verify()?;
        self.require_writing()?;
        let baseline = self.mutation_baseline(&path, digest)?;
        let target = self
            .targets
            .iter()
            .find(|target| target.path == path)
            .ok_or("Only an exact reviewed selected path may be written")?;
        let (kind, directories, syntax) = if let Some(destination) = &destination {
            if path
                .to_string_lossy()
                .chars()
                .flat_map(char::to_lowercase)
                .eq(destination
                    .to_string_lossy()
                    .chars()
                    .flat_map(char::to_lowercase))
            {
                return Err(
                    "Same-path and case-only rename are unsupported; no move occurred".into(),
                );
            }
            let destination_target = self
                .targets
                .iter()
                .find(|target| target.path == *destination)
                .ok_or("Only an exact reviewed selected path may be written")?;
            if destination_target.baseline.is_some() {
                return Err(
                    "Rename destination must be absent; existing entries are never overwritten"
                        .into(),
                );
            }
            let source_parent = self
                .directories
                .get(path.parent().unwrap_or(Path::new("")))
                .ok_or("Selected parent baseline missing")?;
            let mut parent = destination.parent().unwrap_or(Path::new(""));
            let destination_parent = loop {
                if let Some(directory) = self.directories.get(parent) {
                    break directory;
                }
                parent = parent
                    .parent()
                    .ok_or("Destination parent baseline missing")?;
            };
            if !source_parent.same_volume(destination_parent) {
                return Err("Cross-volume rename is unsupported; no move occurred".into());
            }
            let syntax = Some(SyntaxAnalysis::prepare(
                destination,
                (path.extension().is_some_and(|extension| extension == "rs"))
                    .then_some(baseline.text.as_str()),
                &baseline.text,
            ));
            (
                WriteKind::Rename,
                destination_target.missing_parents.clone(),
                syntax,
            )
        } else {
            (WriteKind::Delete, Vec::new(), None)
        };
        let proposal = WriteProposal {
            patch: target.mutation_patch(kind, destination.as_deref(), &baseline.text),
            content: baseline.text.clone(),
            path,
            kind,
            destination,
            syntax,
            directories,
            binding: [0; 32],
        }
        .sealed();
        if !review_fits(&proposal) {
            return Err("Exact content/path/parent review exceeds 256 KiB or 32768 lines; reduce the change".into());
        }
        Ok(proposal)
    }

    pub(crate) fn apply(&mut self, proposal: &WriteProposal) -> Result<WriteResult, WriteFailure> {
        self.verify()?;
        self.require_writing()?;
        let target = self
            .targets
            .iter()
            .find(|target| target.path == proposal.path)
            .ok_or("Only an exact reviewed selected path may be written".to_owned())?;
        let expected_directories = if proposal.kind == WriteKind::Rename {
            proposal
                .destination
                .as_ref()
                .and_then(|path| self.targets.iter().find(|target| target.path == *path))
                .filter(|target| target.baseline.is_none())
                .map(|target| target.missing_parents.as_slice())
        } else if proposal.kind == WriteKind::Delete {
            Some([].as_slice())
        } else {
            Some(target.missing_parents.as_slice())
        };
        let mutation = matches!(proposal.kind, WriteKind::Delete | WriteKind::Rename);
        if proposal.content.len() > MAX_BYTES
            || !safe_text(&proposal.content)
            || (!mutation && proposal.kind != target.kind())
            || (mutation
                && target
                    .baseline
                    .as_ref()
                    .is_none_or(|baseline| baseline.text != proposal.content))
            || (proposal.kind == WriteKind::Rename) != proposal.destination.is_some()
            || expected_directories != Some(proposal.directories.as_slice())
            || proposal.patch
                != target.mutation_patch(
                    proposal.kind,
                    proposal.destination.as_deref(),
                    &proposal.content,
                )
            || proposal.binding != proposal.fingerprint()
            || !review_fits(proposal)
        {
            return Err("Exact reviewed file proposal changed; no write"
                .to_owned()
                .into());
        }
        let original_source_sha256 = target.baseline.as_ref().map(|baseline| baseline.hash);
        let mut created = Vec::new();
        let outcome = self.apply_exact(proposal, &mut created);
        match outcome {
            Ok(source_sha256) => Ok(WriteResult {
                message: format!(
                    "Applied reviewed {} to {}{}{}; inspect the working diff. No compilation, type check, commit or verification occurred.",
                    proposal.kind.name(),
                    proposal.path.display(),
                    proposal
                        .destination
                        .as_ref()
                        .map_or_else(String::new, |path| format!(" -> {}", path.display())),
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
                destination: proposal.destination.clone(),
                directories: created,
                source_sha256: matches!(proposal.kind, WriteKind::Create | WriteKind::Replace)
                    .then_some(source_sha256),
                destination_sha256: (proposal.kind == WriteKind::Rename).then_some(source_sha256),
                original_source_sha256,
            }),
            Err(mut failure) => {
                failure.message = format!(
                    "{} {}{}: {}",
                    proposal.kind.name(),
                    proposal.path.display(),
                    proposal
                        .destination
                        .as_ref()
                        .map_or_else(String::new, |path| format!(" -> {}", path.display())),
                    failure.message
                );
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
        if matches!(proposal.kind, WriteKind::Delete | WriteKind::Rename) {
            return self.apply_mutation(proposal);
        }
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

    fn apply_mutation(&mut self, proposal: &WriteProposal) -> Result<[u8; 32], WriteFailure> {
        let path = &proposal.path;
        let directory = self
            .directories
            .get(path.parent().unwrap_or(Path::new("")))
            .ok_or("Selected parent baseline missing".to_owned())?;
        let name = path
            .file_name()
            .ok_or("Selected file name missing".to_owned())?;
        let expected = self
            .targets
            .iter()
            .find(|target| target.path == *path)
            .and_then(|target| target.baseline.as_ref())
            .ok_or("Selected mutation baseline missing".to_owned())?;
        let mut file = directory.mutation_file(name)?;
        if read_baseline(&mut file)? != *expected {
            return Err(
                "Mutation source identity or working bytes changed; no effect"
                    .to_owned()
                    .into(),
            );
        }
        self.verify()?;
        let result = if let Some(destination) = &proposal.destination {
            let parent = self
                .directories
                .get(destination.parent().unwrap_or(Path::new("")))
                .ok_or("Destination parent baseline missing".to_owned())?;
            if !directory.same_volume(parent) {
                return Err("Cross-volume rename is unsupported; no move occurred"
                    .to_owned()
                    .into());
            }
            let destination_name = destination
                .file_name()
                .ok_or("Destination filename missing".to_owned())?;
            directory.rename(name, &file, parent, destination_name)
        } else {
            directory.remove(name, &file)
        };
        result?;
        // Windows deletion completes only when this DELETE handle closes.
        drop(file);
        #[cfg(test)]
        tests::after_leaf_write();
        let actual = (|| -> Result<Option<Baseline>, String> {
            directory.verify()?;
            if !directory.absent(name)? {
                return Err("Mutation source name is not absent".into());
            }
            if let Some(destination) = &proposal.destination {
                let parent = self.directories.get(destination.parent().unwrap_or(Path::new("")))
                    .ok_or("Destination parent baseline missing")?;
                parent.verify()?;
                let actual = read_baseline(&mut parent.open_file(destination.file_name().ok_or("Destination filename missing")?, false)?)?;
                if actual.text != proposal.content || !platform::same_file(&expected.stamp, &actual.stamp) {
                    return Err("Renamed destination identity or bytes differ from the reviewed source".into());
                }
                Ok(Some(actual))
            } else { Ok(None) }
        })().map_err(|message| WriteFailure {
            message: format!("File mutation occurred but its resulting scope could not be established; inspect local changes, no rollback claimed: {message}"),
            uncertain: true,
        })?;
        let hash = actual
            .as_ref()
            .map_or(expected.hash, |baseline| baseline.hash);
        for target in &mut self.targets {
            if target.path == *path {
                target.baseline = None;
                target.missing_parents.clear();
            } else if proposal.destination.as_ref() == Some(&target.path) {
                target.baseline = actual.clone();
                target.missing_parents.clear();
            }
        }
        self.observed.remove(path);
        if let Some(destination) = &proposal.destination {
            self.observed.remove(destination);
        }
        Ok(hash)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn test_after_leaf_write(hook: impl FnOnce() + 'static) {
    tests::on_after_leaf_write(hook);
}
