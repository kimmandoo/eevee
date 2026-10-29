//! Observational source tools and exact edits share the reviewed file executor.
use super::*;
use serde::{Deserialize, Serialize};

const DISCOVERY_ENTRIES: usize = 10_000;
const LIST_PAGE: usize = 128;
pub(super) const SOURCE_FILES: usize = super::MAX_FILES;
const SOURCE_BYTES: usize = 16 * 1024 * 1024;
const READ_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolKind {
    Write,
    Read,
    List,
    Edit,
    Delete,
    Rename,
    StructuralSearch,
    ArtifactList,
    ArtifactRead,
    ArtifactSearch,
    ArtifactLocations,
    RunCommand,
    Todo,
    Mcp,
    Delegate,
}
impl ToolKind {
    pub(crate) fn parse(name: &str) -> Result<Self, String> {
        match name {
            TOOL_NAME => Ok(Self::Write),
            "eevee_read_file" => Ok(Self::Read),
            "eevee_list_files" => Ok(Self::List),
            "eevee_edit_file" => Ok(Self::Edit),
            "eevee_delete_file" => Ok(Self::Delete),
            "eevee_rename_file" => Ok(Self::Rename),
            "eevee_structural_search" => Ok(Self::StructuralSearch),
            "eevee_artifact_list" => Ok(Self::ArtifactList),
            "eevee_artifact_read" => Ok(Self::ArtifactRead),
            "eevee_artifact_search" => Ok(Self::ArtifactSearch),
            "eevee_artifact_locations" => Ok(Self::ArtifactLocations),
            "eevee_run_command" => Ok(Self::RunCommand),
            "todo" => Ok(Self::Todo),
            "eevee_mcp" => Ok(Self::Mcp),
            "eevee_delegate" => Ok(Self::Delegate),
            _ => Err("Unsupported file tool name".into()),
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Write => TOOL_NAME,
            Self::Read => "eevee_read_file",
            Self::List => "eevee_list_files",
            Self::Edit => "eevee_edit_file",
            Self::Delete => "eevee_delete_file",
            Self::Rename => "eevee_rename_file",
            Self::StructuralSearch => "eevee_structural_search",
            Self::ArtifactList => "eevee_artifact_list",
            Self::ArtifactRead => "eevee_artifact_read",
            Self::ArtifactSearch => "eevee_artifact_search",
            Self::ArtifactLocations => "eevee_artifact_locations",
            Self::RunCommand => "eevee_run_command",
            Self::Todo => "todo",
            Self::Mcp => "eevee_mcp",
            Self::Delegate => "eevee_delegate",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Replacement {
    pub old_text: String,
    pub new_text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "tool", content = "arguments", deny_unknown_fields)]
pub(crate) enum FileTool {
    #[serde(rename = "eevee_write_file")]
    Write { path: PathBuf, content: String },
    #[serde(rename = "eevee_read_file")]
    Read {
        path: PathBuf,
        start_line: usize,
        end_line: usize,
    },
    #[serde(rename = "eevee_list_files")]
    List {
        directory: PathBuf,
        query: String,
        offset: usize,
    },
    #[serde(rename = "eevee_edit_file")]
    Edit {
        path: PathBuf,
        sha256: String,
        replacements: Vec<Replacement>,
    },
    #[serde(rename = "eevee_delete_file")]
    Delete { path: PathBuf, sha256: String },
    #[serde(rename = "eevee_rename_file")]
    Rename {
        path: PathBuf,
        destination: PathBuf,
        sha256: String,
    },
    #[serde(rename = "eevee_structural_search")]
    StructuralSearch {
        directory: PathBuf,
        language: String,
        pattern: String,
        offset: usize,
        snapshot: String,
    },
    #[serde(rename = "eevee_artifact_list")]
    ArtifactList {},
    #[serde(rename = "eevee_artifact_read")]
    ArtifactRead {
        id: String,
        revision: String,
        offset: u64,
        limit: usize,
    },
    #[serde(rename = "eevee_artifact_search")]
    ArtifactSearch {
        id: String,
        revision: String,
        query: String,
        offset: u64,
        limit: usize,
    },
    #[serde(rename = "eevee_artifact_locations")]
    ArtifactLocations {
        id: String,
        revision: String,
        offset: u64,
    },
    #[serde(rename = "eevee_run_command")]
    RunCommand {
        cwd: PathBuf,
        command: crate::workspace::runner::CommandSpec,
    },
    #[serde(rename = "todo")]
    Todo { input: Value },
    #[serde(rename = "eevee_mcp")]
    Mcp { name: String, arguments: Value },
    #[serde(rename = "eevee_delegate")]
    Delegate {
        role: String,
        instruction: String,
        paths: Vec<PathBuf>,
    },
    #[serde(skip)]
    Invalid { kind: ToolKind },
}
impl FileTool {
    pub(crate) fn parse(name: &str, arguments: Value) -> Result<Self, String> {
        ToolKind::parse(name)?;
        // Todo owns only inert task metadata. Keep malformed original arguments
        // for a visible refusal rather than losing them in file-tool admission.
        if name == "todo" {
            return Ok(Self::Todo { input: arguments });
        }
        let tool: Self = serde_json::from_value(json!({"tool":name,"arguments":arguments}))
            .map_err(|_| {
                "File tool arguments must exactly match the advertised schema".to_owned()
            })?;
        if let Self::Delegate {
            role,
            instruction,
            paths,
        } = &tool
            && (!matches!(role.as_str(), "explore" | "review")
                || instruction.trim().is_empty()
                || instruction.len() > 4096
                || instruction
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t')
                || paths.len() > 32
                || paths.iter().any(|path| {
                    path.as_os_str().len() > 4096
                        || path
                            .components()
                            .any(|part| !matches!(part, std::path::Component::Normal(_)))
                }))
        {
            return Err("Delegation requires explore/review, bounded instructions and literal project-relative paths".into());
        }
        if tool.path().as_os_str().len() > 4096
            || tool
                .mutation_paths()
                .any(|path| path.as_os_str().len() > 4096)
        {
            return Err(
                "File tool path exceeds 4096 bytes; use a literal project-relative filename".into(),
            );
        }
        Ok(tool)
    }
    pub(crate) fn kind(&self) -> ToolKind {
        match self {
            Self::Write { .. } => ToolKind::Write,
            Self::Read { .. } => ToolKind::Read,
            Self::List { .. } => ToolKind::List,
            Self::Edit { .. } => ToolKind::Edit,
            Self::Delete { .. } => ToolKind::Delete,
            Self::Rename { .. } => ToolKind::Rename,
            Self::StructuralSearch { .. } => ToolKind::StructuralSearch,
            Self::ArtifactList { .. } => ToolKind::ArtifactList,
            Self::ArtifactRead { .. } => ToolKind::ArtifactRead,
            Self::ArtifactSearch { .. } => ToolKind::ArtifactSearch,
            Self::ArtifactLocations { .. } => ToolKind::ArtifactLocations,
            Self::RunCommand { .. } => ToolKind::RunCommand,
            Self::Todo { .. } => ToolKind::Todo,
            Self::Mcp { .. } => ToolKind::Mcp,
            Self::Delegate { .. } => ToolKind::Delegate,
            Self::Invalid { kind } => *kind,
        }
    }
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Write { path, .. }
            | Self::Read { path, .. }
            | Self::Edit { path, .. }
            | Self::Delete { path, .. }
            | Self::Rename { path, .. } => path,
            Self::List { directory, .. } | Self::StructuralSearch { directory, .. } => directory,
            Self::Invalid { .. }
            | Self::RunCommand { .. }
            | Self::Todo { .. }
            | Self::Mcp { .. }
            | Self::Delegate { .. }
            | Self::ArtifactList { .. }
            | Self::ArtifactRead { .. }
            | Self::ArtifactSearch { .. }
            | Self::ArtifactLocations { .. } => Path::new(""),
        }
    }
    pub(crate) fn mutating(&self) -> bool {
        matches!(
            self.kind(),
            ToolKind::Write | ToolKind::Edit | ToolKind::Delete | ToolKind::Rename
        )
    }
    pub(crate) fn invalid(&self) -> bool {
        matches!(self, Self::Invalid { .. })
    }
    pub(crate) fn mutation_paths(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.path())
            .filter(|_| self.mutating())
            .chain(match self {
                Self::Rename { destination, .. } => Some(destination.as_path()),
                _ => None,
            })
    }
    pub(crate) fn artifact_operation(&self) -> Option<crate::workspace::artifacts::Operation> {
        use crate::workspace::artifacts::{Handle, Operation};
        Some(match self {
            Self::ArtifactList {} => Operation::Load,
            Self::ArtifactRead {
                id,
                revision,
                offset,
                limit,
            } => Operation::Read {
                handle: Handle {
                    id: id.clone(),
                    revision: revision.clone(),
                },
                offset: *offset,
                limit: *limit,
            },
            Self::ArtifactSearch {
                id,
                revision,
                query,
                offset,
                limit,
            } => Operation::Search {
                handle: Handle {
                    id: id.clone(),
                    revision: revision.clone(),
                },
                query: query.clone(),
                offset: *offset,
                limit: *limit,
            },
            Self::ArtifactLocations {
                id,
                revision,
                offset,
            } => Operation::Locations {
                handle: Handle {
                    id: id.clone(),
                    revision: revision.clone(),
                },
                offset: *offset,
            },
            _ => return None,
        })
    }
}

/// Provider-independent declarations: callers adapt only the protocol envelope.
pub(crate) fn definitions(writing: bool) -> Vec<Value> {
    let mut declarations = Vec::with_capacity(if writing { 8 } else { 3 });
    if writing {
        declarations.push(
        json!({"name":TOOL_NAME,"description":"Create or replace an actual project file through the host file executor, even when the provider's own shell is read-only. Supply complete UTF-8 content (4 MiB maximum); do not substitute a fenced code response for a requested save. Missing parent folders are created after exact full-content allow-once approval. No path preselection is required in workspace mode; an explicit selected-path restriction still applies. Preserve unrelated dirty bytes. No shell or verification.","parameters":{"type":"object","properties":{"path":{"type":"string","description":"Literal project-relative filename, for example src/app/main.py. Nested folders and Unicode names are supported. Do not use an absolute path or parent traversal."},"content":{"type":"string"}},"required":["path","content"],"additionalProperties":false}}),
        );
        declarations.push(crate::providers::commands::definition());
    }
    declarations.extend([
        json!({"name":"eevee_read_file","description":"Read actual safe project UTF-8 working bytes, independently of the 4 MiB mutation limit. Source capture is bounded at 16 MiB for full-file privacy validation and sha256; returned content is at most 64 KiB of complete lines. Lines are one-based inclusive; end_line=0 requests EOF. Inspect total_lines, truncated and next_start_line, then continue with a narrower range as needed. sha256 always identifies the complete source, never just the returned slice; editable=false refuses mutation of files above 4 MiB. Reading grants no write authority. Credential, excluded, protected, ignored and link paths are refused.","parameters":{"type":"object","properties":{"path":{"type":"string","description":"Literal project-relative filename, not an absolute path or folder. Exact reads need no preceding discovery."},"start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":0}},"required":["path","start_line","end_line"],"additionalProperties":false}}),
        json!({"name":"eevee_list_files","description":"Inspect actual safe project filenames, including plain non-Git projects and files too large to edit. directory is a literal project-relative directory (empty means the project root), applied before enumeration and Git inventory. query is a literal substring; offset starts at zero. At most 128 filenames per page, 10000 scanned entries within that directory. Use next_offset for another page; if the scan bound is exceeded, choose a narrower directory, not another query or offset. Returned paths remain relative to the project root. Discovery grants no write authority.","parameters":{"type":"object","properties":{"directory":{"type":"string","description":"Literal project-relative directory to search recursively; empty selects project root. No absolute path or parent traversal."},"query":{"type":"string"},"offset":{"type":"integer","minimum":0}},"required":["directory","query","offset"],"additionalProperties":false}}),
        json!({"name":"eevee_structural_search","description":"Bounded Rust-only expression search; no binding or rewriting. Grammar: calls/method calls (no turbofish), paths/literals, parentheses, unary/binary/reference, field/index, tuples/arrays, await/try. $NAME captures one expression; repeated names require equal tokens ignoring whitespace/comments. Comments, string lookalikes and macro bodies are not searched. Limits: 128 Rust candidates, 256 KiB/source, 2 MiB scanned bytes, 256 matches, 32/page, 64 KiB response, 2048-byte pattern, 16 captures, 32 KiB capture text/match, 8 KiB each source/omission metadata. Conservative parser ceilings include literal/comment bytes: 64 nesting levels, 1024 opening-delimiter bytes, 32768 nonwhitespace bytes; token-tree expression paths are at most 256 tokens. Omitted sources/details make the result incomplete. First page offset=0,snapshot=\"\"; continuation copies snapshot/next_offset; changed sources or policy refuse. Incomplete is not absence; not compilation.","parameters":{"type":"object","properties":{"directory":{"type":"string"},"language":{"type":"string"},"pattern":{"type":"string","maxLength":2048},"offset":{"type":"integer","minimum":0},"snapshot":{"type":"string"}},"required":["directory","language","pattern","offset","snapshot"],"additionalProperties":false}}),
    ]);
    if writing {
        declarations.extend([
        json!({"name":"eevee_edit_file","description":"Save 1–32 literal replacements against one previously read original file and its full 64-hex SHA-256. Every nonempty old_text must match exactly once in the ORIGINAL bytes, and spans must not overlap. Replacement output is never input to another replacement. Preserve every unrelated byte, including CRLF and Unicode. Final file is at most 4 MiB and receives one complete exact allow-once review; stale source/ancestors, ambiguous anchors or overlap refuse without writing. Local Rust syntax disclosure is advisory, not compilation or verification.","parameters":{"type":"object","properties":{"path":{"type":"string"},"sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"},"replacements":{"type":"array","minItems":1,"maxItems":32,"items":{"type":"object","properties":{"old_text":{"type":"string","minLength":1},"new_text":{"type":"string"}},"required":["old_text","new_text"],"additionalProperties":false}}},"required":["path","sha256","replacements"],"additionalProperties":false}}),
        json!({"name":"eevee_delete_file","description":"Irreversibly delete one exact regular project file (at most 4 MiB), bound to its preceding eevee_read_file full SHA-256 and frozen source/ancestors. Requires separate exact allow-once approval; no recursive deletion, recycle bin, shell or rollback. Links, protected/excluded files and stale scope refuse.","parameters":{"type":"object","properties":{"path":{"type":"string"},"sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"}},"required":["path","sha256"],"additionalProperties":false}}),
        json!({"name":"eevee_rename_file","description":"Rename one exact regular project file (at most 4 MiB) to an exact unoccupied same-project destination. Bind to the preceding read full SHA-256. One approval freezes original bytes, source/destination and ancestors; approved absent destination parents may be created. Never overwrite; case-only and cross-volume moves refuse. Preserve literal bytes and supported metadata; no imports rewriting, shell or rollback.","parameters":{"type":"object","properties":{"path":{"type":"string"},"destination":{"type":"string"},"sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"}},"required":["path","destination","sha256"],"additionalProperties":false}}),
        ]);
    }
    declarations
}

impl WriteSet {
    /// Freeze names once for the reviewed request, not as selected-source evidence.
    /// Later tool calls still observe current bytes and revalidate their own scope.
    pub(crate) fn project_context(&self) -> &Value {
        self.project_context.get_or_init(|| {
            let observation = match self.list_sources(Path::new(""), "", 0) {
                Ok(mut listing) => {
                    let mut bytes = 0usize;
                    let paths = listing["paths"].as_array_mut().expect("listing paths");
                    paths.retain(|path| {
                        bytes = bytes.saturating_add(path.as_str().expect("filename").len());
                        bytes <= 16 * 1024
                    });
                    let count = paths.len();
                    let truncated = listing["total"].as_u64().unwrap_or(0) > count as u64;
                    listing["truncated"] = json!(truncated);
                    listing["next_offset"] = if truncated { json!(count) } else { Value::Null };
                    listing["complete"] = json!(!truncated);
                    listing
                }
                Err(_) => json!({
                    "status": "unavailable", "paths": [], "complete": false,
                    "truncated": null, "total": null, "next_offset": null,
                    "message": "Safe filename observation unavailable: discovery bound, inventory, access or scope validation failed. This is not an empty-project claim. Use eevee_list_files with a narrower project-relative directory; exact eevee_read_file needs no complete inventory."
                }),
            };
            json!({
                "root": self.root,
                "root_kind": "canonical_current_folder",
                "filenames": observation,
                "bounds": {"filenames": 128, "filename_bytes": 16384, "scanned_entries": 10000},
                "meaning": "Untrusted filename observation at request preview; complete means only the privacy-filtered discoverable filename set, never all project files. Names may change and do not guarantee readable content. No file bodies are attached by this observation.",
                "authority": "Not selected context, mutation permission, review evidence or verification. Read actual admitted bytes before reasoning about implementation. Existing operation and exact per-call permissions remain authoritative."
            })
        })
    }

    pub(super) fn source_allowed(&self, path: &Path) -> bool {
        if !allowed_path(path, &self.exclusions)
            || path.as_os_str().len() > 4096
            || self
                .exclusions
                .iter()
                .any(|excluded| protected_prefix(path, excluded))
        {
            return false;
        }
        let full = self.root.join(path);
        !self
            .protected_paths
            .iter()
            .any(|protected| protected_prefix(&full, protected))
    }

    pub(super) fn source_directory(&self, path: &Path) -> Result<platform::Directory, String> {
        let mut directory = self
            .directories
            .get(Path::new(""))
            .ok_or("Frozen root missing")?
            .clone();
        for part in path.components() {
            directory = directory
                .child(part.as_os_str())?
                .ok_or("Source ancestor is missing")?;
        }
        directory.verify()?;
        Ok(directory)
    }

    pub(super) fn source_capture_limit(
        &self,
        path: &Path,
        limit: usize,
        control: &ObservationControl<'_>,
    ) -> Result<(platform::FileStamp, String), String> {
        self.verify()?;
        control.check()?;
        if !self.source_allowed(path) {
            return Err(
                "Source path is protected, excluded or not a literal project-relative filename"
                    .into(),
            );
        }
        if !crate::workspace::repo::source_visible_controlled(&self.root, path, control)? {
            return Err("Source is ignored or absent from the current project inventory".into());
        }
        let directory = self.source_directory(path.parent().unwrap_or(Path::new("")))?;
        let mut file =
            directory.open_file(path.file_name().ok_or("Source filename missing")?, false)?;
        let before = platform::observation_stamp(&file)?;
        if before.size > limit as u64 {
            return Err(format!(
                "Source exceeds the full-source privacy capture bound ({limit} bytes; ordinary reads: 16 MiB); no partial digest was returned"
            ));
        }
        let mut bytes = Vec::with_capacity(before.size as usize + 1);
        while bytes.len() <= limit {
            control.check()?;
            let before_len = bytes.len();
            (&mut file)
                .take((limit + 1 - before_len).min(64 * 1024) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| "Source must be readable UTF-8")?;
            if bytes.len() == before_len {
                break;
            }
        }
        control.check()?;
        let text = String::from_utf8(bytes).map_err(|_| "Source must be readable UTF-8")?;
        if text.len() > limit {
            return Err(format!(
                "Source exceeds the full-source privacy capture bound ({limit} bytes; ordinary reads: 16 MiB); no partial digest was returned"
            ));
        }
        if !safe_text(&text) {
            return Err(
                "Source bytes are binary, credential-bearing or contain unsafe controls".into(),
            );
        }
        if platform::observation_stamp(&file)? != before {
            return Err("Source changed while captured; read again".into());
        }
        directory.verify()?;
        self.verify()?;
        control.check()?;
        Ok((before, text))
    }

    #[cfg(test)]
    pub(crate) fn observe_tool(&mut self, tool: &FileTool) -> Result<Value, String> {
        let stopped = std::sync::atomic::AtomicBool::new(false);
        self.observe_tool_controlled(
            tool,
            &ObservationControl {
                stopped: &stopped,
                cancelled: &|| false,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
            },
        )
    }

    pub(crate) fn observe_tool_controlled(
        &mut self,
        tool: &FileTool,
        control: &ObservationControl<'_>,
    ) -> Result<Value, String> {
        let control = ObservationControl {
            deadline: control
                .deadline
                .min(std::time::Instant::now() + std::time::Duration::from_secs(30)),
            ..*control
        };
        control.check()?;
        let result = self.cached_observation_controlled(tool, &control);
        control.check()?;
        result
    }

    /// Content-only mediated protocols cannot disclose original cache time/hits.
    /// Reuse the same scoped reader but never cache an unlabelled observation.
    pub(crate) fn observe_unlabelled(&mut self, tool: &FileTool) -> Result<Value, String> {
        let stopped = std::sync::atomic::AtomicBool::new(false);
        self.execute_observation(
            tool,
            None,
            &ObservationControl {
                stopped: &stopped,
                cancelled: &|| false,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
            },
        )
    }

    pub(super) fn execute_observation(
        &mut self,
        tool: &FileTool,
        captured: Option<super::observations::Captured>,
        control: &ObservationControl<'_>,
    ) -> Result<Value, String> {
        control.check()?;
        match tool {
            FileTool::Read {
                path,
                start_line,
                end_line,
            } => {
                if *start_line == 0 || (*end_line != 0 && end_line < start_line) {
                    return Err(
                        "Read requires a one-based inclusive range; end_line=0 means EOF".into(),
                    );
                }
                if !self.observed.contains_key(path) && self.observed.len() == SOURCE_FILES {
                    return Err(format!(
                        "Source read cache permits {SOURCE_FILES} distinct files per request"
                    ));
                }
                let (stamp, text, hash, digest) = match captured {
                    Some(source) => (source.stamp, source.text, source.hash, source.digest),
                    None => {
                        let (stamp, text) =
                            self.source_capture_limit(path, SOURCE_BYTES, control)?;
                        let hash = Sha256::digest(text.as_bytes());
                        let digest = format!("{hash:x}");
                        (stamp, text, hash.into(), digest)
                    }
                };
                let total_lines = text.split_inclusive('\n').count();
                let requested_end = if *end_line == 0 {
                    total_lines
                } else {
                    *end_line
                };
                if (*start_line > total_lines
                    && !(total_lines == 0 && *start_line == 1 && *end_line == 0))
                    || requested_end > total_lines
                {
                    return Err(
                        "Read range extends beyond EOF; no partial range was substituted".into(),
                    );
                }
                let mut begin = 0;
                let mut finish = 0;
                let mut returned_end = start_line.saturating_sub(1);
                for (index, line) in text.split_inclusive('\n').enumerate() {
                    let number = index + 1;
                    control.check()?;
                    if number < *start_line {
                        begin += line.len();
                        finish = begin;
                    } else if number <= requested_end {
                        if finish - begin + line.len() > READ_BYTES {
                            break;
                        }
                        finish += line.len();
                        returned_end = number;
                    } else {
                        break;
                    }
                }
                if requested_end >= *start_line && returned_end < *start_line {
                    return Err("Requested source line exceeds the 64 KiB output bound; no partial line or digest was substituted".into());
                }
                let result = json!({"status":"success","path":path,"message":"Actual bounded UTF-8 working bytes; sha256 identifies the full source, not only this range; no write authority granted",
                    "content":&text[begin..finish],"sha256":digest,"bytes":text.len(),
                    "editable":text.len() <= MAX_BYTES,"digest_scope":"full_file",
                    "start_line":start_line,"end_line":returned_end,"requested_end_line":end_line,
                    "total_lines":total_lines,"returned_bytes":finish - begin,
                    "truncated":*start_line > 1 || returned_end < total_lines,
                    "next_start_line":if returned_end < requested_end { Some(returned_end + 1) } else { None }});
                self.observed.insert(
                    path.clone(),
                    std::sync::Arc::new(SourceIdentity { stamp, hash }),
                );
                Ok(result)
            }
            FileTool::List {
                directory,
                query,
                offset,
            } => self.list_sources_controlled(directory, query, *offset, control),
            _ => Err(
                "Mutation requires exact reviewed proposal execution, not an observational tool"
                    .into(),
            ),
        }
    }

    pub(super) fn list_sources(
        &self,
        scope: &Path,
        query: &str,
        offset: usize,
    ) -> Result<Value, String> {
        let stopped = std::sync::atomic::AtomicBool::new(false);
        self.list_sources_controlled(
            scope,
            query,
            offset,
            &ObservationControl {
                stopped: &stopped,
                cancelled: &|| false,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
            },
        )
    }

    fn list_sources_controlled(
        &self,
        scope: &Path,
        query: &str,
        offset: usize,
        control: &ObservationControl<'_>,
    ) -> Result<Value, String> {
        control.check()?;
        self.verify()?;
        if query.len() > 1024 || !safe_text(query) || offset > DISCOVERY_ENTRIES {
            return Err(
                "Filename query exceeds 1024 safe UTF-8 bytes or offset exceeds 10000".into(),
            );
        }
        let paths: Vec<_> = self
            .source_names(scope, control)?
            .into_iter()
            .filter(|name| name.contains(query))
            .collect();
        let total = paths.len();
        let page: Vec<_> = paths.into_iter().skip(offset).take(LIST_PAGE).collect();
        let next = offset.saturating_add(page.len());
        Ok(json!({"status":"success","path":scope,"directory":scope,
            "message":"Safe project filenames within this directory scope; observed names only, not write authority",
            "paths":page,"total":total,"offset":offset,"truncated":next < total,"complete":next >= total,
            "next_offset":if next < total { Some(next) } else { None }}))
    }

    pub(super) fn source_names(
        &self,
        scope: &Path,
        control: &ObservationControl<'_>,
    ) -> Result<Vec<String>, String> {
        control.check()?;
        self.verify()?;
        if !scope.as_os_str().is_empty() && !self.source_allowed(scope) {
            return Err(
                "Discovery directory must be a safe literal project-relative directory".into(),
            );
        }
        let directory = self.source_directory(scope)?;
        #[cfg(target_os = "macos")]
        let native_scope = directory.relative_path(
            self.directories
                .get(Path::new(""))
                .ok_or("Frozen root missing")?,
        )?;
        #[cfg(target_os = "macos")]
        let scope = native_scope.as_path();
        #[cfg(target_os = "macos")]
        if !scope.as_os_str().is_empty() && !self.source_allowed(scope) {
            return Err("Native discovery directory is protected or excluded".into());
        }
        let inventory =
            crate::workspace::repo::source_paths_controlled(&self.root, scope, control)?;
        let mut pending = vec![(scope.to_path_buf(), directory)];
        let mut scanned = 0usize;
        let mut paths = Vec::new();
        let mut retained = Vec::new();
        while let Some((parent, directory)) = pending.pop() {
            control.check()?;
            if parent.components().count() - scope.components().count() > 64 {
                return Err("Project discovery exceeds 64 directory levels; choose a narrower directory scope".into());
            }
            let names = directory.names(DISCOVERY_ENTRIES - scanned)?;
            scanned += names.len();
            for name in names {
                control.check()?;
                let path = parent.join(&name);
                if !self.source_allowed(&path) {
                    continue;
                }
                if let Some(inventory) = &inventory
                    && inventory
                        .range::<Path, _>((
                            std::ops::Bound::Included(path.as_path()),
                            std::ops::Bound::Unbounded,
                        ))
                        .next()
                        .is_none_or(|entry| !entry.starts_with(&path))
                {
                    continue;
                }
                if let Ok(Some(child)) = directory.child(&name) {
                    pending.push((path, child));
                } else if let Ok(_file) = directory.open_file(&name, false)
                    && let Ok(name) = path.into_os_string().into_string()
                {
                    #[cfg(windows)]
                    let name = {
                        let mut name = name;
                        // Only the wire spelling changes; native identity stays untouched.
                        // ASCII separator replacement preserves length and UTF-8 boundaries.
                        for index in 0..name.len() {
                            if name.as_bytes()[index] == b'\\' {
                                name.replace_range(index..index + 1, "/");
                            }
                        }
                        name
                    };
                    paths.push(name);
                }
            }
            directory.verify()?;
            retained.push(directory);
        }
        for directory in retained {
            directory.verify()?;
        }
        self.verify()?;
        control.check()?;
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    /// Called on a batch candidate; enrollment is committed only after all proposals validate.
    pub(crate) fn propose_tool(&self, tool: &mut FileTool) -> Result<WriteProposal, String> {
        self.verify()?;
        self.require_writing()?;
        match tool {
            FileTool::Write { path, content } => {
                self.propose(path.clone(), std::mem::take(content))
            }
            FileTool::Edit {
                path,
                sha256,
                replacements,
            } => {
                let baseline = self.mutation_baseline(path, sha256)?;
                if replacements.is_empty() || replacements.len() > 32 {
                    return Err("Edit requires 1–32 literal replacements".into());
                }
                let mut spans = Vec::with_capacity(replacements.len());
                let mut size = baseline.text.len();
                for replacement in replacements {
                    let old = replacement.old_text.as_str();
                    if old.is_empty()
                        || old.len() > MAX_BYTES
                        || replacement.new_text.len() > MAX_BYTES
                    {
                        return Err(
                            "Edit old_text must be nonempty and replacement text at most 4 MiB"
                                .into(),
                        );
                    }
                    let Some(start) = baseline.text.find(old) else {
                        return Err("Edit old_text must be nonempty and match exactly once; read and select an unambiguous literal span".into());
                    };
                    // Advance one Unicode scalar, not the whole match, so
                    // overlapping occurrences are ambiguous too ("aa"/"aaa").
                    let next = start + old.chars().next().expect("nonempty anchor").len_utf8();
                    if baseline.text[next..].contains(old) {
                        return Err("Edit old_text must be nonempty and match exactly once; read and select an unambiguous literal span".into());
                    }
                    size = size
                        .checked_sub(old.len())
                        .and_then(|size| size.checked_add(replacement.new_text.len()))
                        .ok_or("Edit text exceeds the 4 MiB file bound")?;
                    spans.push((start, start + old.len(), replacement.new_text.as_str()));
                }
                spans.sort_unstable_by_key(|span| span.0);
                if spans.windows(2).any(|pair| pair[0].1 > pair[1].0) {
                    return Err("Edit replacement spans overlap in the original source".into());
                }
                if size > MAX_BYTES {
                    return Err("Edit text exceeds the 4 MiB file bound".into());
                }
                let mut content = String::with_capacity(size);
                let mut offset = 0;
                for (start, end, replacement) in spans {
                    content.push_str(&baseline.text[offset..start]);
                    content.push_str(replacement);
                    offset = end;
                }
                content.push_str(&baseline.text[offset..]);
                self.propose(path.clone(), content)
            }
            FileTool::Delete { path, sha256 } => self.propose_mutation(path.clone(), sha256, None),
            FileTool::Rename {
                path,
                destination,
                sha256,
            } => self.propose_mutation(path.clone(), sha256, Some(destination.clone())),
            _ => Err("Read/list tools cannot propose filesystem changes".into()),
        }
    }
}
