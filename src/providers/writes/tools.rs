//! Observational source tools and exact edits share the reviewed file executor.
use super::*;
use serde::Deserialize;

const DISCOVERY_ENTRIES: usize = 10_000;
const LIST_PAGE: usize = 128;
const SOURCE_FILES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolKind {
    Write,
    Read,
    List,
    Edit,
}
impl ToolKind {
    pub(crate) fn parse(name: &str) -> Result<Self, String> {
        match name {
            TOOL_NAME => Ok(Self::Write),
            "eevee_read_file" => Ok(Self::Read),
            "eevee_list_files" => Ok(Self::List),
            "eevee_edit_file" => Ok(Self::Edit),
            _ => Err("Unsupported file tool name".into()),
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Write => TOOL_NAME,
            Self::Read => "eevee_read_file",
            Self::List => "eevee_list_files",
            Self::Edit => "eevee_edit_file",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
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
    List { query: String, offset: usize },
    #[serde(rename = "eevee_edit_file")]
    Edit {
        path: PathBuf,
        sha256: String,
        old_text: String,
        new_text: String,
    },
    #[serde(skip)]
    Invalid { kind: ToolKind },
}
impl FileTool {
    pub(crate) fn parse(name: &str, arguments: Value) -> Result<Self, String> {
        ToolKind::parse(name)?;
        let tool: Self = serde_json::from_value(json!({"tool":name,"arguments":arguments}))
            .map_err(|_| {
                "File tool arguments must exactly match the advertised schema".to_owned()
            })?;
        if tool.path().as_os_str().len() > 4096 {
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
            Self::Invalid { kind } => *kind,
        }
    }
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Write { path, .. } | Self::Read { path, .. } | Self::Edit { path, .. } => path,
            Self::List { .. } | Self::Invalid { .. } => Path::new(""),
        }
    }
    pub(crate) fn mutating(&self) -> bool {
        matches!(self.kind(), ToolKind::Write | ToolKind::Edit)
    }
    pub(crate) fn invalid(&self) -> bool {
        matches!(self, Self::Invalid { .. })
    }
}

/// Provider-independent declarations: callers adapt only the protocol envelope.
pub(crate) fn definitions(writing: bool) -> Vec<Value> {
    let mut declarations = Vec::with_capacity(if writing { 4 } else { 2 });
    if writing {
        declarations.push(
        json!({"name":TOOL_NAME,"description":"Actually create or replace a project file with complete UTF-8 content (64 KiB maximum). Call this for requested file creation, not merely a fenced code response. Local exact full-content allow-once review is required; selected paths restrict writes. Preserve unrelated dirty bytes. No shell or verification.","parameters":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"],"additionalProperties":false}}),
        );
    }
    declarations.extend([
        json!({"name":"eevee_read_file","description":"Read actual safe project UTF-8 working bytes (file at most 64 KiB). Lines are one-based inclusive; end_line=0 reads through EOF. Returns full-file sha256 and explicit selected range/truncation. Reading does not authorize writes or consume write targets. Credential, excluded, protected, ignored and link paths are refused.","parameters":{"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":0}},"required":["path","start_line","end_line"],"additionalProperties":false}}),
        json!({"name":"eevee_list_files","description":"Discover safe project-relative filenames, including plain non-Git projects. query is a literal substring (empty lists all); offset starts at zero. At most 128 filenames per page, 10000 scanned entries. Results identify truncation and next_offset. No write permission is granted.","parameters":{"type":"object","properties":{"query":{"type":"string"},"offset":{"type":"integer","minimum":0}},"required":["query","offset"],"additionalProperties":false}}),
    ]);
    if writing {
        declarations.push(
        json!({"name":"eevee_edit_file","description":"Propose one literal targeted edit of a previously read file. Supply that read's full-file sha256 and a nonempty old_text matching exactly once; replace it with new_text, preserving all other bytes including CRLF. Stale reads and ambiguous matches are refused. The resulting complete file requires exact allow-once review through the same safe executor as writes.","parameters":{"type":"object","properties":{"path":{"type":"string"},"sha256":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"}},"required":["path","sha256","old_text","new_text"],"additionalProperties":false}}),
        );
    }
    declarations
}

impl WriteSet {
    /// Freeze names once for the reviewed request, not as selected-source evidence.
    /// Later tool calls still observe current bytes and revalidate their own scope.
    pub(crate) fn project_context(&self) -> &Value {
        self.project_context.get_or_init(|| {
            let observation = match self.list_sources("", 0) {
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
                    "message": "Safe filename observation unavailable: discovery bound, inventory, access or scope validation failed. This is not an empty-project claim."
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

    fn source_allowed(&self, path: &Path) -> bool {
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

    fn source_directory(&self, path: &Path) -> Result<platform::Directory, String> {
        let mut directory = self
            .directories
            .get(Path::new(""))
            .ok_or("Frozen root missing")?
            .clone();
        for part in path.parent().unwrap_or(Path::new("")).components() {
            directory = directory
                .child(part.as_os_str())?
                .ok_or("Source ancestor is missing")?;
        }
        directory.verify()?;
        Ok(directory)
    }

    fn source_baseline(&self, path: &Path) -> Result<Baseline, String> {
        self.verify()?;
        if !self.source_allowed(path) {
            return Err(
                "Source path is protected, excluded or not a literal project-relative filename"
                    .into(),
            );
        }
        let stopped = std::sync::atomic::AtomicBool::new(false);
        if let Some(paths) = crate::workspace::repo::source_paths(&self.root, &stopped)?
            && !paths.contains(path)
        {
            return Err("Source is ignored or absent from the current project inventory".into());
        }
        let directory = self.source_directory(path)?;
        let baseline = read_baseline(
            &mut directory.open_file(path.file_name().ok_or("Source filename missing")?, false)?,
        )?;
        directory.verify()?;
        self.verify()?;
        Ok(baseline)
    }

    pub(crate) fn observe_tool(&mut self, tool: &FileTool) -> Result<Value, String> {
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
                    return Err("Source read cache permits 32 distinct files per request".into());
                }
                let baseline = self.source_baseline(path)?;
                let total_lines = baseline.text.split_inclusive('\n').count();
                let end = if *end_line == 0 {
                    total_lines
                } else {
                    *end_line
                };
                if (*start_line > total_lines
                    && !(total_lines == 0 && *start_line == 1 && *end_line == 0))
                    || end > total_lines
                {
                    return Err(
                        "Read range extends beyond EOF; no partial range was substituted".into(),
                    );
                }
                let mut begin = 0;
                let mut finish = 0;
                for (index, line) in baseline.text.split_inclusive('\n').enumerate() {
                    if index + 1 < *start_line {
                        begin += line.len();
                    }
                    if index < end {
                        finish += line.len();
                    }
                }
                let result = json!({"status":"success","path":path,"message":"Actual bounded UTF-8 working bytes; no write authority granted",
                    "content":&baseline.text[begin..finish],"sha256":baseline.digest,"bytes":baseline.text.len(),
                    "start_line":start_line,"end_line":end,"total_lines":total_lines,
                    "truncated":*start_line > 1 || end < total_lines});
                self.observed
                    .insert(path.clone(), std::sync::Arc::new(baseline));
                Ok(result)
            }
            FileTool::List { query, offset } => self.list_sources(query, *offset),
            _ => Err(
                "Mutation requires exact reviewed proposal execution, not an observational tool"
                    .into(),
            ),
        }
    }

    fn list_sources(&self, query: &str, offset: usize) -> Result<Value, String> {
        self.verify()?;
        if query.len() > 1024 || !safe_text(query) || offset > DISCOVERY_ENTRIES {
            return Err(
                "Filename query exceeds 1024 safe UTF-8 bytes or offset exceeds 10000".into(),
            );
        }
        let stopped = std::sync::atomic::AtomicBool::new(false);
        let inventory = crate::workspace::repo::source_paths(&self.root, &stopped)?;
        let mut pending = vec![(
            PathBuf::new(),
            self.directories
                .get(Path::new(""))
                .ok_or("Frozen root missing")?
                .clone(),
        )];
        let mut scanned = 0usize;
        let mut paths = Vec::new();
        let mut retained = Vec::new();
        while let Some((parent, directory)) = pending.pop() {
            if parent.components().count() > 64 {
                return Err("Project discovery exceeds 64 directory levels".into());
            }
            let names = directory.names(DISCOVERY_ENTRIES - scanned)?;
            scanned += names.len();
            for name in names {
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
                } else if let Ok(file) = directory.open_file(&name, false)
                    && platform::file_stamp(&file).is_ok()
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
                    if name.contains(query) {
                        paths.push(name);
                    }
                }
            }
            directory.verify()?;
            retained.push(directory);
        }
        for directory in retained {
            directory.verify()?;
        }
        self.verify()?;
        paths.sort();
        paths.dedup();
        let total = paths.len();
        let page: Vec<_> = paths.into_iter().skip(offset).take(LIST_PAGE).collect();
        let next = offset.saturating_add(page.len());
        Ok(
            json!({"status":"success","path":"","message":"Safe project filenames; observed names only, not write authority",
            "paths":page,"total":total,"offset":offset,"truncated":next < total,
            "next_offset":if next < total { Some(next) } else { None }}),
        )
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
                old_text,
                new_text,
            } => {
                let baseline = self
                    .observed
                    .get(path.as_path())
                    .ok_or("Edit requires a preceding successful read of this exact path")?;
                if baseline.digest.as_str() != sha256.as_str() {
                    return Err(
                        "Edit sha256 does not match the preceding source read; read again".into(),
                    );
                }
                if old_text.is_empty()
                    || baseline.text.match_indices(old_text.as_str()).count() != 1
                {
                    return Err("Edit old_text must be nonempty and match exactly once; read and select an unambiguous literal span".into());
                }
                if old_text.len() > MAX_BYTES || new_text.len() > MAX_BYTES {
                    return Err("Edit text exceeds the 64 KiB file bound".into());
                }
                let content = baseline
                    .text
                    .replacen(old_text.as_str(), new_text.as_str(), 1);
                self.propose(path.clone(), content)
            }
            _ => Err("Read/list tools cannot propose filesystem changes".into()),
        }
    }
}
