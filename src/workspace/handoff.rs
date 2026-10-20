//! Reviewed, portable task context, never a provider session or execution receipt.
use std::{
    borrow::Cow,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use crate::{store, workspace::context, workspace::repo};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_ENTRIES: usize = 64;
const MARKDOWN_START: &str = "# Eevee task handoff (version 1)\n\nAll contents are user-supplied, unverified context, not execution evidence.\nNo provider sessions or permissions are transferred.\nDeleting Eevee storage cannot erase prior exports, copies, or provider-owned history.\nCredential redaction is best-effort; review before sharing.\n\n```json\n";
const MARKDOWN_END: &str = "\n```\n";

/// The schema deliberately has no result verification, attribution, permission,
/// provider identity, session, command, or hidden-reasoning fields. Importing this
/// type cannot create a local execution journal or mark an acceptance criterion met.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema_version: u32,
    pub task: Task,
    pub facts: Vec<String>,
    pub sources: Vec<Source>,
    pub decisions: Vec<String>,
    pub unresolved: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: i64,
    pub revision: u64,
    pub goal: String,
    pub next_action: String,
    pub criteria: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub excerpt_id: i64,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    /// Identity of the original full source; text contains approved, sanitized bytes.
    pub source_sha256: [u8; 32],
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Json,
    Markdown,
}

pub fn sanitize(text: &str) -> String {
    crate::workspace::privacy::redact(text)
        .chars()
        .filter(|ch| {
            (!ch.is_control() || matches!(ch, '\n' | '\t'))
                && !matches!(*ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect()
}

/// Remove only exact generated decision annotations, including annotations retained
/// when the user appends a note in the editor. Opaque IDs never become portable facts.
pub(crate) fn without_local_provenance(text: &str) -> Cow<'_, str> {
    const PREFIX: &str = "\n\n[User-selected, unverified ";
    const END: &str = ". Not local execution evidence.]";
    let mut output: Option<String> = None;
    let mut copied = 0;
    for (start, _) in text.match_indices(PREFIX) {
        let suffix = &text[start + PREFIX.len()..];
        let Some(end) = suffix.find(END) else {
            continue;
        };
        let ids = &suffix[..end];
        if let Some((origin, ids)) = ids.split_once("; thread ")
            && matches!(
                origin,
                "Codex planning"
                    | "Codex Plan"
                    | "Codex Implement"
                    | "Codex Review"
                    | "Codex · CLI agent Plan"
                    | "Codex · CLI agent Implement"
                    | "Codex · CLI agent Review"
                    | "Claude Agent Plan"
                    | "Claude Agent Implement"
                    | "Claude Agent Review"
                    | "OpenCode ACP Plan"
                    | "OpenCode ACP Implement"
                    | "Gemini ACP Plan"
                    | "Gemini ACP Implement"
                    | "Devin ACP Plan"
                    | "Devin ACP Implement"
            )
            && let Some((thread, rest)) = ids.split_once("; turn ")
            && let Some((turn, item)) = rest.split_once("; item ")
            && [thread, turn, item].iter().all(|id| {
                !id.is_empty()
                    && !id
                        .chars()
                        .any(|ch| ch.is_control() || matches!(ch, '[' | ']'))
            })
        {
            let after = start + PREFIX.len() + end + END.len();
            if output.is_none() && after == text.len() {
                return Cow::Borrowed(&text[..start]);
            }
            let output = output.get_or_insert_with(|| String::with_capacity(text.len()));
            output.push_str(&text[copied..start]);
            copied = after;
        }
    }
    match output {
        Some(mut output) => {
            output.push_str(&text[copied..]);
            Cow::Owned(output)
        }
        None => Cow::Borrowed(text),
    }
}
fn approved_text(text: &str) -> String {
    sanitize(&without_local_provenance(text))
}

fn portable_text(text: &str, bound: usize) -> Result<(), String> {
    if text.len() > bound || sanitize(text) != text || without_local_provenance(text) != text {
        return Err(
            "Handoff contains oversized text, a recognizable secret, or unsafe control characters"
                .into(),
        );
    }
    // Opaque provider state is not a task fact. Strict fields also reject state
    // injected as JSON properties, including nested properties.
    let lower = text.to_ascii_lowercase();
    if [
        "session_id",
        "sessionid",
        "thread_id",
        "threadid",
        "turn_id",
        "turnid",
        "permission_grant",
        "persistent_grant",
        "private_reasoning",
        "hidden_reasoning",
        "chain_of_thought",
        "chain-of-thought",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return Err(
            "Provider sessions, grants, and private reasoning cannot enter a handoff".into(),
        );
    }
    Ok(())
}

fn text_list(items: &[String]) -> Result<(), String> {
    if items.len() > MAX_ENTRIES {
        return Err("Handoff list exceeds 64 entries".into());
    }
    for item in items {
        portable_text(item, MAX_TEXT_BYTES)?;
    }
    Ok(())
}

impl Document {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "Unsupported handoff schema {}; supported version is 1",
                self.schema_version
            ));
        }
        if self.task.id <= 0 || self.task.goal.trim().is_empty() {
            return Err("Handoff requires a saved task identity and nonempty goal".into());
        }
        portable_text(&self.task.goal, crate::app::MAX_GOAL_BYTES)?;
        portable_text(&self.task.next_action, MAX_TEXT_BYTES)?;
        text_list(&self.task.criteria)?;
        text_list(&self.facts)?;
        text_list(&self.decisions)?;
        // At most 64 unmet criteria plus the committed next action.
        if self.unresolved.len() > MAX_ENTRIES + 1 {
            return Err("Handoff unresolved work exceeds 65 entries".into());
        }
        for item in &self.unresolved {
            portable_text(item, MAX_TEXT_BYTES)?;
        }
        if self.sources.len() > context::MAX_EXCERPTS {
            return Err("Handoff source count exceeds the context basket limit".into());
        }
        for (index, source) in self.sources.iter().enumerate() {
            portable_text(&source.path, 4096)?;
            portable_text(&source.text, context::MAX_EXCERPT_BYTES)?;
            let path = Path::new(&source.path);
            if source.excerpt_id <= 0
                || source.start_line == 0
                || source.end_line < source.start_line
                || path.as_os_str().is_empty()
                || source.path.contains(':')
                || !source
                    .path
                    .split(['/', '\\'])
                    .all(|part| !part.is_empty() && part != "." && part != "..")
                || context::excluded(Path::new(&source.path.replace('\\', "/")), &[])
                || self.sources[..index]
                    .iter()
                    .any(|other| other.excerpt_id == source.excerpt_id)
            {
                return Err("Invalid, excluded, duplicate, or non-relative handoff source".into());
            }
        }
        Ok(())
    }
}

/// Build only from committed task data and explicitly selected decisions. Provider
/// item metadata and tool transcripts are never examined by this module.
pub fn from_task(
    task: &store::Snapshot,
    excerpts: &[context::Excerpt],
    decisions: &[String],
) -> Result<Document, String> {
    if task.id <= 0 || task.archived {
        return Err("Save an active task before preparing a handoff".into());
    }
    if excerpts.len() > context::MAX_EXCERPTS
        || decisions.len() > MAX_ENTRIES
        || task.criteria.len() > MAX_ENTRIES
    {
        return Err("Task handoff exceeds the bounded source/decision/criteria counts".into());
    }
    let mut sources = Vec::with_capacity(excerpts.len());
    for excerpt in excerpts {
        excerpt.validate()?;
        if !excerpt.available || excerpt.task_id != task.id || excerpt.id <= 0 {
            return Err(
                "Only available, committed excerpts belonging to this task can be exported".into(),
            );
        }
        sources.push(Source {
            excerpt_id: excerpt.id,
            path: excerpt.path.clone(),
            start_line: excerpt.start_line,
            end_line: excerpt.end_line,
            source_sha256: excerpt.source_sha256,
            text: sanitize(&excerpt.text),
        });
    }
    let mut unresolved: Vec<_> = task
        .criteria
        .iter()
        .filter(|criterion| !criterion.met)
        .map(|criterion| approved_text(&criterion.text))
        .collect();
    if !task.next_action.trim().is_empty() {
        unresolved.push(approved_text(&task.next_action));
    }
    let (facts, risks) = task.workflows.portable_notes();
    unresolved.extend(risks);
    let document = Document {
        schema_version: SCHEMA_VERSION,
        task: Task {
            id: task.id,
            revision: task.revision,
            goal: approved_text(&task.goal),
            next_action: approved_text(&task.next_action),
            criteria: task
                .criteria
                .iter()
                .map(|criterion| approved_text(&criterion.text))
                .collect(),
        },
        facts,
        sources,
        decisions: decisions
            .iter()
            .map(|decision| approved_text(decision))
            .collect(),
        unresolved,
    };
    document.validate()?;
    Ok(document)
}

/// Conservative admission: a foreign or stale document may be read/reviewed but
/// cannot be dispatched as this task's context. No source is refreshed/replaced.
/// Call store::validate_planning_input as well to check the persisted snapshot.
pub fn validate_dispatch(
    document: &Document,
    task: &store::Snapshot,
    excerpts: &[context::Excerpt],
    report: Option<&Arc<repo::Report>>,
    exclusions: &[PathBuf],
) -> Result<(), String> {
    document.validate()?;
    if task.archived
        || document.task.id != task.id
        || document.task.revision != task.revision
        || document.task.goal != approved_text(&task.goal)
        || document.task.next_action != approved_text(&task.next_action)
        || document.task.criteria
            != task
                .criteria
                .iter()
                .map(|criterion| approved_text(&criterion.text))
                .collect::<Vec<_>>()
    {
        return Err(
            "Handoff task changed or belongs to another task; prepare and accept a fresh handoff"
                .into(),
        );
    }
    for source in &document.sources {
        let excerpt = excerpts.iter().find(|excerpt| excerpt.id == source.excerpt_id)
            .ok_or("Handoff source is no longer in the approved task basket; no substitution permitted")?;
        if !excerpt.available
            || excerpt.task_id != task.id
            || source.path != excerpt.path
            || source.start_line != excerpt.start_line
            || source.end_line != excerpt.end_line
            || source.source_sha256 != excerpt.source_sha256
            || source.text != sanitize(&excerpt.text)
        {
            return Err(
                "Handoff source identity or approved bytes changed; review a new handoff".into(),
            );
        }
    }
    // The existing source admission checks full-file hashes, repository evidence,
    // current symlink/junction routes, exclusions, and exact persisted ranges.
    context::planning_payload(task.id, report, excerpts, exclusions)?;
    Ok(())
}

pub fn encode(document: &Document, format: Format) -> Result<String, String> {
    document.validate()?;
    let json = serde_json::to_string_pretty(document).map_err(|error| error.to_string())?;
    let text = match format {
        Format::Json => format!("{json}\n"),
        Format::Markdown => format!("{MARKDOWN_START}{json}{MARKDOWN_END}"),
    };
    if text.len() > MAX_FILE_BYTES {
        return Err("Handoff exceeds the 2 MiB file limit".into());
    }
    Ok(text)
}

pub fn decode(text: &str) -> Result<Document, String> {
    if text.len() > MAX_FILE_BYTES {
        return Err("Handoff exceeds the 2 MiB file limit".into());
    }
    let json = if text.trim_start().starts_with('{') {
        text
    } else {
        text.strip_prefix(MARKDOWN_START).and_then(|body| body.strip_suffix(MARKDOWN_END))
            .ok_or("Unsupported Markdown handoff; use the complete version-1 Eevee JSON/Markdown document")?
    };
    let document: Document = serde_json::from_str(json).map_err(|_| {
        "Invalid handoff JSON or unknown schema fields; no content imported".to_string()
    })?;
    document.validate()?;
    Ok(document)
}

/// Export only to a new file. No overwrite flag is intentionally provided.
/// Paths reject links except verified macOS `/var` and `/tmp` system aliases;
/// their fixed `/private` destinations are still traversed without following links.
pub fn write_new(path: &Path, document: &Document, format: Format) -> Result<(), String> {
    let text = encode(document, format)?;
    let file = open_regular(path, true)?;
    write_export(file, text.as_bytes())
}

fn write_export(mut file: File, bytes: &[u8]) -> Result<(), String> {
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("Export write failed (a partial new file may remain): {error}"))
}

pub fn read_file(path: &Path) -> Result<Document, String> {
    let mut file = open_regular(path, false)?;
    if file.metadata().map_err(|error| error.to_string())?.len() > MAX_FILE_BYTES as u64 {
        return Err("Handoff exceeds the 2 MiB file limit".into());
    }
    let mut text = String::new();
    Read::by_ref(&mut file)
        .take((MAX_FILE_BYTES + 1) as u64)
        .read_to_string(&mut text)
        .map_err(|_| "Cannot read handoff as bounded UTF-8 text".to_string())?;
    decode(&text)
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().len() > 32 * 1024
        || path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err("Choose a bounded literal path without parent-directory traversal".into());
    }
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(path)
    };
    if path.file_name().is_none() {
        return Err("Choose a handoff filename, not a directory".into());
    }
    #[cfg(windows)]
    for component in path.components() {
        if let Component::Normal(name) = component {
            let name = name.to_str().ok_or("Handoff filenames must be UTF-8")?;
            let stem = name
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            if name.contains(':')
                || name.ends_with(['.', ' '])
                || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || matches!(
                    stem.as_str(),
                    "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                )
            {
                return Err(
                    "Device names, alternate streams, and ambiguous Windows filenames are refused"
                        .into(),
                );
            }
        }
    }
    Ok(path)
}

fn ensure_regular(metadata: &std::fs::Metadata) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Handoff paths cannot be symlinks or reparse points".into());
        }
    }
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Handoff target must be a regular file, not a link or device".into());
    }
    Ok(())
}

/// Bind only the two standard macOS temporary-directory aliases, never arbitrary
/// user-selected links. The root-owned alias must have its exact system target,
/// and its parent must not be writable by other users. The destination is then
/// opened component-by-component with no-follow, just like a literal path.
#[cfg(target_os = "macos")]
fn macos_handoff_path(path: PathBuf, root: &File) -> Result<PathBuf, String> {
    use std::os::unix::fs::MetadataExt;

    for (alias, target, relative_target) in [
        ("/var", "/private/var", "private/var"),
        ("/tmp", "/private/tmp", "private/tmp"),
    ] {
        let Ok(suffix) = path.strip_prefix(alias) else {
            continue;
        };
        let metadata = std::fs::symlink_metadata(alias).map_err(|error| error.to_string())?;
        if !metadata.file_type().is_symlink() {
            return Ok(path);
        }
        let root_metadata = root.metadata().map_err(|error| error.to_string())?;
        if root_metadata.uid() != 0 || root_metadata.mode() & 0o022 != 0 || metadata.uid() != 0 {
            return Err("Handoff system aliases must be protected and owned by root".into());
        }
        let link = std::fs::read_link(alias).map_err(|error| error.to_string())?;
        if link != Path::new(target) && link != Path::new(relative_target) {
            return Err("Handoff system alias has an unexpected destination".into());
        }
        return Ok(Path::new(target).join(suffix));
    }
    Ok(path)
}

/// POSIX dirfd walking rejects links in every component after binding the macOS
/// system aliases, and anchors the final create/read against ancestor replacement.
/// New exports use private mode 0600. No arbitrary parent canonicalization occurs.
#[cfg(unix)]
pub(crate) fn open_regular(path: &Path, create: bool) -> Result<File, String> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    };
    let path = absolute_path(path)?;
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "macos")]
    let path = macos_handoff_path(path, &directory)?;
    let mut parts = path
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .peekable();
    while let Some(name) = parts.next() {
        let name =
            std::ffi::CString::new(name.as_bytes()).map_err(|_| "Path contains a NUL byte")?;
        let final_part = parts.peek().is_none();
        let flags = libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if final_part {
                if create {
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL
                } else {
                    libc::O_RDONLY
                }
            } else {
                libc::O_RDONLY | libc::O_DIRECTORY
            };
        // SAFETY: live owned dirfd and NUL-terminated single component. The new
        // descriptor is adopted exactly once below.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, 0o600) };
        if fd < 0 {
            return Err(format!(
                "Cannot {} handoff (existing exports are never overwritten; links refused): {}",
                if create { "create" } else { "read" },
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: openat succeeded and transferred ownership of a new descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        if final_part {
            ensure_regular(&file.metadata().map_err(|error| error.to_string())?)?;
            return Ok(file);
        }
        directory = file;
    }
    Err("Missing handoff filename".into())
}

/// Deny-delete ancestor handles pin the Windows path while the final no-follow
/// handle is opened. Reparse points are rejected instead of resolved.
#[cfg(windows)]
pub(crate) fn open_regular(path: &Path, create: bool) -> Result<File, String> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let path = absolute_path(path)?;
    let parent = path.parent().ok_or("Missing handoff parent directory")?;
    let mut ancestors = Vec::new();
    for ancestor in parent.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        let file = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .custom_flags(0x0200_0000 | 0x0020_0000)
            .open(ancestor)
            .map_err(|error| error.to_string())?;
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
            return Err("Handoff ancestors must be directories without reparse points".into());
        }
        ancestors.push(file);
    }
    let mut options = OpenOptions::new();
    options
        .read(!create)
        .write(create)
        .create_new(create)
        .share_mode(3)
        .custom_flags(0x0020_0000);
    let file = options
        .open(&path)
        .map_err(|error| format!("Cannot open handoff without overwrite: {error}"))?;
    ensure_regular(&file.metadata().map_err(|error| error.to_string())?)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> Document {
        let mut task = store::Snapshot::empty("project".into());
        task.id = 7;
        task.revision = 3;
        task.goal = "Continue the reviewed task".into();
        task.criteria.push(store::Criterion {
            text: "Check the result".into(),
            met: true,
        });
        from_task(&task, &[], &["Use a boring design".into()]).unwrap()
    }

    #[test]
    fn json_and_markdown_roundtrip_without_a_provider() {
        let document = document();
        for format in [Format::Json, Format::Markdown] {
            assert_eq!(
                decode(&encode(&document, format).unwrap()).unwrap(),
                document
            );
        }
        let value = serde_json::to_value(&document).unwrap();
        assert!(value["task"]["criteria"][0].is_string());
        assert!(value.get("verified").is_none());
        assert!(value["task"].get("complete").is_none());
    }

    #[test]
    fn rejects_new_schema_unknown_fields_secrets_and_session_injection() {
        let original = document();
        let mut value = serde_json::to_value(&original).unwrap();
        value["schema_version"] = 2.into();
        assert!(decode(&value.to_string()).is_err());
        value = serde_json::to_value(&original).unwrap();
        value["session_id"] = "opaque-provider-id".into();
        assert!(decode(&value.to_string()).is_err());
        value = serde_json::to_value(&original).unwrap();
        value["task"]["verified"] = true.into();
        assert!(decode(&value.to_string()).is_err());
        for text in [
            "api_key=sk-secret123456789",
            "session_id=opaque",
            "hidden_reasoning: private",
            "\u{202e}misleading",
        ] {
            let mut poisoned = original.clone();
            poisoned.facts.push(text.into());
            assert!(poisoned.validate().is_err());
        }
        assert!(decode(&"x".repeat(MAX_FILE_BYTES + 1)).is_err());
    }

    #[test]
    fn export_redacts_before_preview_and_dispatch_rejects_changed_task() {
        let mut task = store::Snapshot::empty("project".into());
        task.id = 1;
        task.goal = "password=abcdef continue".into();
        let document = from_task(&task, &[], &["token=abcdef decision".into()]).unwrap();
        let preview = encode(&document, Format::Json).unwrap();
        assert!(!preview.contains("abcdef"));
        assert!(validate_dispatch(&document, &task, &[], None, &[]).is_ok());
        task.revision += 1;
        assert!(validate_dispatch(&document, &task, &[], None, &[]).is_err());
    }

    #[test]
    fn strips_only_known_generated_decision_provenance() {
        for origin in [
            "Codex planning",
            "Codex Plan",
            "Codex · CLI agent Plan",
            "Codex · CLI agent Implement",
            "Codex · CLI agent Review",
            "Claude Agent Plan",
            "OpenCode ACP Implement",
            "Gemini ACP Plan",
            "Devin ACP Plan",
            "Devin ACP Implement",
        ] {
            let text = format!(
                "Approved decision\n\n[User-selected, unverified {origin}; thread opaque-old; turn old-turn; item old-item. Not local execution evidence.]"
            );
            assert_eq!(approved_text(&text), "Approved decision");
            assert!(portable_text(&text, MAX_TEXT_BYTES).is_err());
            let edited = format!("{text}\nUser appended another note.");
            assert_eq!(
                without_local_provenance(&edited),
                "Approved decision\nUser appended another note."
            );
            assert!(portable_text(&edited, MAX_TEXT_BYTES).is_err());
            let repeated = format!("{text}\nThen:\n{text}\nEnd");
            assert_eq!(
                without_local_provenance(&repeated),
                "Approved decision\nThen:\nApproved decision\nEnd"
            );
        }
        let narrative = "Decision\n\n[User-selected, unverified arbitrary; thread opaque; turn old; item old. Not local execution evidence.]";
        assert_eq!(approved_text(narrative), narrative);
    }
    #[test]
    fn source_bytes_and_identity_never_silently_substitute() {
        let mut task = store::Snapshot::empty("project".into());
        task.id = 1;
        task.goal = "Keep the approved source".into();
        let excerpt = context::Excerpt {
            origin_run: None,
            id: 4,
            task_id: task.id,
            root: std::env::current_dir()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned(),
            path: "src/example.rs".into(),
            start_line: 1,
            end_line: 1,
            source_sha256: [7; 32],
            text: "password=abcdef visible\n".into(),
            available: true,
        };
        let document = from_task(&task, std::slice::from_ref(&excerpt), &[]).unwrap();
        assert_eq!(document.sources[0].text, "password=[REDACTED] visible\n");
        assert_eq!(document.sources[0].source_sha256, excerpt.source_sha256);
        assert!(
            validate_dispatch(&document, &task, &[], None, &[])
                .unwrap_err()
                .contains("no substitution")
        );
        let mut changed = excerpt.clone();
        changed.text = "a replacement excerpt\n".into();
        assert!(
            validate_dispatch(&document, &task, &[changed], None, &[])
                .unwrap_err()
                .contains("approved bytes changed")
        );
        let mut changed = excerpt;
        changed.source_sha256 = [8; 32];
        assert!(
            validate_dispatch(&document, &task, &[changed], None, &[])
                .unwrap_err()
                .contains("approved bytes changed")
        );
        let mut excluded = document;
        excluded.sources[0].path = "sub/.env".into();
        assert!(excluded.validate().is_err());
        excluded.sources[0].path = "..\\outside".into();
        assert!(excluded.validate().is_err());
    }

    #[test]
    fn manual_files_are_new_only_and_bounded() {
        let path = std::env::temp_dir().join(format!(
            "ev-handoff-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        for (name, format) in [
            ("review.json", Format::Json),
            ("review.md", Format::Markdown),
        ] {
            let target = path.join(name);
            write_new(&target, &document(), format).unwrap();
            assert_eq!(read_file(&target).unwrap(), document());
            assert!(write_new(&target, &document(), format).is_err());
            assert_eq!(read_file(&target).unwrap(), document());
        }
        std::fs::write(path.join("large"), vec![b'x'; MAX_FILE_BYTES + 1]).unwrap();
        assert!(read_file(&path.join("large")).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(path.join("review.json"), path.join("link")).unwrap();
            assert!(read_file(&path.join("link")).is_err());
            assert!(write_new(&path.join("link"), &document(), Format::Json).is_err());
            std::os::unix::fs::symlink(&path, path.join("ancestor")).unwrap();
            assert!(read_file(&path.join("ancestor/review.json")).is_err());
            assert!(write_new(&path.join("ancestor/new"), &document(), Format::Json).is_err());
            std::fs::remove_file(path.join("ancestor")).unwrap();
        }
        std::fs::remove_dir_all(path).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_aliases_bind_only_their_fixed_destinations() {
        use std::os::unix::fs::symlink;

        let root = File::open("/").unwrap();
        for (alias, target) in [("/var", "/private/var"), ("/tmp", "/private/tmp")] {
            assert!(
                std::fs::symlink_metadata(alias)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            let selected = Path::new(alias).join("handoff.json");
            assert_eq!(
                macos_handoff_path(selected, &root).unwrap(),
                Path::new(target).join("handoff.json")
            );
        }
        // Component boundaries matter: a user directory merely named like an
        // alias must not be rewritten, nor may nested user-created links pass.
        let unrelated = PathBuf::from("/var-user/handoff.json");
        assert_eq!(
            macos_handoff_path(unrelated.clone(), &root).unwrap(),
            unrelated
        );

        let name = format!(
            "ev-handoff-system-alias-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let selected = Path::new("/tmp").join(name);
        std::fs::create_dir(&selected).unwrap();
        let bound = selected.canonicalize().unwrap();
        let target = selected.join("review.json");
        write_new(&target, &document(), Format::Json).unwrap();
        assert_eq!(read_file(&target).unwrap(), document());
        assert_eq!(read_file(&bound.join("review.json")).unwrap(), document());
        assert!(write_new(&bound.join("review.json"), &document(), Format::Json).is_err());
        assert_eq!(read_file(&target).unwrap(), document());
        symlink(&bound, selected.join("nested")).unwrap();
        assert!(read_file(&selected.join("nested/review.json")).is_err());
        assert!(write_new(&selected.join("nested/new.json"), &document(), Format::Json).is_err());
        assert!(!bound.join("new.json").exists());
        std::fs::remove_file(selected.join("nested")).unwrap();
        symlink(bound.join("review.json"), selected.join("leaf.json")).unwrap();
        assert!(read_file(&selected.join("leaf.json")).is_err());
        assert!(write_new(&selected.join("leaf.json"), &document(), Format::Json).is_err());
        std::fs::remove_dir_all(selected).unwrap();
    }

    #[test]
    fn imported_criteria_and_sources_cannot_supply_verification_fields() {
        let mut value = serde_json::to_value(document()).unwrap();
        value["task"]["criteria"] = serde_json::json!([{
            "text": "Provider claims it passed", "met": true
        }]);
        assert!(decode(&value.to_string()).is_err());
        value = serde_json::to_value(document()).unwrap();
        value["sources"] = serde_json::json!([{
            "excerpt_id": 1,
            "path": "src/example.rs",
            "start_line": 1,
            "end_line": 1,
            "source_sha256": vec![0_u8; 32],
            "text": "User-supplied source",
            "verified": true
        }]);
        assert!(decode(&value.to_string()).is_err());
        value = serde_json::to_value(document()).unwrap();
        value["facts"] = serde_json::json!(["The provider claims tests passed"]);
        let imported = decode(&value.to_string()).unwrap();
        let exported = serde_json::to_value(imported).unwrap();
        assert!(exported["task"]["criteria"][0].is_string());
        assert!(exported.get("verified").is_none());
    }
}
