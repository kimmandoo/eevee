//! Request-private read/search reuse. Fingerprinting is real work, never counted as avoided.
use super::*;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub(super) const VERSION: &str = "eevee-observation-v1/rust-expression-v1";
const CACHE_ENTRIES: usize = 32;
const CACHE_BYTES: usize = 512 * 1024;
pub(super) const CANDIDATES: usize = 128;
pub(super) const SCANNED_BYTES: usize = 2 * 1024 * 1024;
pub(super) const FILE_BYTES: usize = 256 * 1024;

#[derive(Default)]
pub(super) struct Cache {
    entries: BTreeMap<String, Entry>,
    bytes: usize,
}
impl fmt::Debug for Cache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObservationCache")
            .field("entries", &self.entries.len())
            .field("bytes", &self.bytes)
            .finish()
    }
}
#[derive(Clone)]
struct Entry {
    fingerprint: String,
    value: Value,
    observed_at: u128,
    execution_us: u128,
    bytes: usize,
}
pub(super) struct Captured {
    pub path: PathBuf,
    pub stamp: platform::FileStamp,
    pub text: String,
    pub digest: String,
    pub hash: [u8; 32],
}
pub(super) struct Frozen {
    pub sources: Vec<Captured>,
    pub names: Vec<String>,
    pub omissions: Vec<Value>,
    pub fingerprint: String,
    pub hashed_bytes: usize,
    pub reusable: bool,
    pub snapshot_available: bool,
}
struct Costs {
    fingerprint_us: u128,
    hashed_bytes: usize,
    lookup_us: u128,
    store_us: u128,
}

impl WriteSet {
    // Only local rules under retained project handles are cacheable. Linked Git
    // worktrees, enclosing repositories and external ignore/include files retain
    // ordinary readers but deliberately bypass reuse, rather than trusting an
    // unobserved policy. No external file bodies are acquired for this cache.
    fn policy_fingerprint(
        &self,
        paths: &[PathBuf],
        hash: &mut Sha256,
        control: &ObservationControl<'_>,
    ) -> Result<usize, String> {
        control.check()?;
        hash.update(VERSION);
        hash.update(self.root.as_os_str().as_encoded_bytes());
        for policy in [&self.exclusions, &self.protected_paths] {
            hash.update(format!("{policy:?}"));
        }
        for ancestor in self.root.ancestors().skip(1) {
            control.check()?;
            if std::fs::symlink_metadata(ancestor.join(".git")).is_ok() {
                return Err(
                    "Enclosing Git policy is outside the project-private cache boundary".into(),
                );
            }
        }
        let mut directories = BTreeSet::from([PathBuf::new()]);
        for path in paths {
            control.check()?;
            for parent in path.ancestors().skip(1) {
                directories.insert(parent.to_path_buf());
                if directories.len() > 512 {
                    return Err("Ignore policy directory bound exceeds 512".into());
                }
            }
        }
        let mut bytes = 0;
        for parent in directories {
            control.check()?;
            let directory = self.source_directory(&parent)?;
            hash.update(parent.as_os_str().as_encoded_bytes());
            hash.update(directory.observation_identity());
            bytes += hash_rule(&directory, ".gitignore", hash, false, control)?;
            if bytes > 256 * 1024 {
                return Err("Ignore policy aggregate exceeds 256 KiB".into());
            }
            directory.verify()?;
        }
        let root = self.source_directory(Path::new(""))?;
        let names = root.names(10_000)?;
        if names.iter().any(|name| name == ".git") {
            let git = root
                .child(std::ffi::OsStr::new(".git"))?
                .ok_or("Git worktree policy is not a project-local directory")?;
            bytes += hash_rule(&git, "config", hash, true, control)?;
            let names = git.names(10_000)?;
            if names.iter().any(|name| name == "config.worktree") {
                bytes += hash_rule(&git, "config.worktree", hash, true, control)?;
            }
            if names.iter().any(|name| name == "info") {
                let info = git
                    .child(std::ffi::OsStr::new("info"))?
                    .ok_or("Git info policy unavailable")?;
                bytes += hash_rule(&info, "exclude", hash, false, control)?;
                info.verify()?;
            }
            git.verify()?;
        }
        self.verify()?;
        Ok(bytes)
    }

    pub(super) fn freeze_observation(
        &self,
        tool: &FileTool,
        control: &ObservationControl<'_>,
    ) -> Result<Frozen, String> {
        control.check()?;
        let mut frozen = Frozen {
            sources: Vec::new(),
            names: Vec::new(),
            omissions: Vec::new(),
            fingerprint: String::new(),
            hashed_bytes: 0,
            reusable: true,
            snapshot_available: true,
        };
        let mut hash = Sha256::new();
        hash.update(VERSION);
        hash.update(self.root.as_os_str().as_encoded_bytes());
        let mut paths = match tool {
            FileTool::Read { path, .. } => vec![path.clone()],
            FileTool::StructuralSearch { directory, .. } => {
                match self.source_names(directory, control) {
                    Ok(names) => frozen.names = names,
                    Err(_) => {
                        control.check()?;
                        frozen.omissions.push(json!({"reason":"discovery_unavailable","message":"Safe source discovery unavailable; inventory, access, policy or scan bounds prevented a complete observation"}));
                        frozen.reusable = false;
                        frozen.snapshot_available = false;
                    }
                }
                frozen.names.iter().map(PathBuf::from).collect()
            }
            _ => return Err("Only allowlisted project-local file reads and Rust structural searches are cacheable".into()),
        };
        for name in &frozen.names {
            hash.update(name.as_bytes());
            hash.update([0]);
        }
        let policy_paths = paths.clone();
        let mut policy = Sha256::new();
        let policy_guard = match self.policy_fingerprint(&policy_paths, &mut policy, control) {
            Ok(bytes) => {
                frozen.hashed_bytes += bytes;
                let identity: [u8; 32] = policy.finalize().into();
                hash.update(identity);
                Some(identity)
            }
            Err(_) => {
                control.check()?;
                frozen.reusable = false;
                None
            }
        };
        if matches!(tool, FileTool::StructuralSearch { .. }) {
            paths.retain(|path| path.extension().is_some_and(|extension| extension == "rs"));
            if paths.len() > CANDIDATES {
                frozen.omissions.push(
                    json!({"reason":"candidate_limit","omitted_candidates":paths.len()-CANDIDATES}),
                );
                paths.truncate(CANDIDATES);
            }
        }
        let mut scanned = 0usize;
        for path in paths {
            control.check()?;
            let limit = if matches!(tool, FileTool::Read { .. }) {
                16 * 1024 * 1024
            } else {
                FILE_BYTES.min(SCANNED_BYTES.saturating_sub(scanned))
            };
            if limit == 0 {
                frozen
                    .omissions
                    .push(json!({"reason":"scanned_byte_limit"}));
                break;
            }
            match self.source_capture_limit(&path, limit, control) {
                Ok((stamp, text)) => {
                    let source_hash = Sha256::digest(text.as_bytes());
                    let digest = format!("{source_hash:x}");
                    scanned += text.len();
                    frozen.hashed_bytes += text.len();
                    hash.update(path.as_os_str().as_encoded_bytes());
                    hash.update(format!("{stamp:?}"));
                    hash.update(&digest);
                    frozen.sources.push(Captured {
                        path,
                        stamp,
                        text,
                        digest,
                        hash: source_hash.into(),
                    });
                }
                Err(_) if matches!(tool, FileTool::StructuralSearch { .. }) => {
                    control.check()?;
                    // No filename/content from a refused source is reflected.
                    hash.update(b"unavailable-source");
                    frozen
                        .omissions
                        .push(json!({"reason":"source_unavailable","message":"Source admission or complete capture unavailable; no source content or diagnostic was retained"}));
                    frozen.reusable = false;
                    frozen.snapshot_available = false;
                }
                Err(error) => return Err(error),
            }
        }
        if let Some(expected) = policy_guard {
            let mut policy = Sha256::new();
            frozen.hashed_bytes += self.policy_fingerprint(&policy_paths, &mut policy, control)?;
            let actual: [u8; 32] = policy.finalize().into();
            if expected != actual {
                return Err("Observation policy or link ancestry changed while fingerprinting; no cached result returned".into());
            }
        }
        frozen.fingerprint = format!("{:x}", hash.finalize());
        control.check()?;
        Ok(frozen)
    }

    /// Coordinator fast path: a cold/noncacheable operation does no filesystem
    /// work here and may enter the existing independent-read worker path.
    /// Known keys revalidate on the already-running coordinator; a stale key is
    /// an honest newly computed miss, never a cached hit or a second hash pass.
    pub(crate) fn observe_known_tool_controlled(
        &mut self,
        tool: &FileTool,
        control: &ObservationControl<'_>,
    ) -> Option<Result<Value, String>> {
        let control = ObservationControl {
            deadline: control
                .deadline
                .min(Instant::now() + std::time::Duration::from_secs(30)),
            ..*control
        };
        if let Err(error) = control.check() {
            return Some(Err(error));
        }
        if !matches!(
            tool,
            FileTool::Read { .. } | FileTool::StructuralSearch { .. }
        ) {
            return None;
        }
        let started = Instant::now();
        let key = match serde_json::to_string(tool) {
            Ok(key) => key,
            Err(_) => return Some(Err("Cannot identify observation arguments".into())),
        };
        let known = match self.observation_cache.lock() {
            Ok(cache) => cache.entries.contains_key(&key),
            Err(_) => return Some(Err("Observation cache unavailable".into())),
        };
        if !known {
            return None;
        }
        let probe_us = started.elapsed().as_micros();
        Some(
            self.cached_observation_key(tool, Some(key), probe_us, &control)
                .and_then(|mut value| {
                    control.check()?;
                    value["observation_cache"]["coordinator_probe_us"] = json!(probe_us);
                    Ok(value)
                }),
        )
    }

    pub(super) fn cached_observation_controlled(
        &mut self,
        tool: &FileTool,
        control: &ObservationControl<'_>,
    ) -> Result<Value, String> {
        self.cached_observation_key(tool, None, 0, control)
    }

    fn cached_observation_key(
        &mut self,
        tool: &FileTool,
        supplied_key: Option<String>,
        prior_lookup_us: u128,
        control: &ObservationControl<'_>,
    ) -> Result<Value, String> {
        control.check()?;
        if !matches!(
            tool,
            FileTool::Read { .. } | FileTool::StructuralSearch { .. }
        ) {
            let started = Instant::now();
            let mut value = self.execute_observation(tool, None, control)?;
            if matches!(tool, FileTool::List { .. }) {
                let original = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| "Observation clock unavailable")?
                    .as_millis();
                label(
                    &mut value,
                    false,
                    original,
                    started.elapsed().as_micros(),
                    false,
                    &Costs {
                        fingerprint_us: 0,
                        hashed_bytes: 0,
                        lookup_us: 0,
                        store_us: 0,
                    },
                );
                value["observation_cache"]["bypass_reason"] = json!(
                    "Filename inventory is fingerprint work itself, not an avoided search execution"
                );
            }
            return Ok(value);
        }
        if let FileTool::StructuralSearch {
            directory,
            language,
            ..
        } = tool
            && language != "rust"
        {
            return Ok(
                json!({"status":"unsupported_language","path":directory,"message":"Structural search supports only Rust; no source search was performed","supported_language":"rust","complete":false}),
            );
        }
        let lookup_started = Instant::now();
        let key = match supplied_key {
            Some(key) => key,
            None => {
                serde_json::to_string(tool).map_err(|_| "Cannot identify observation arguments")?
            }
        };
        let known_query = self
            .observation_cache
            .lock()
            .map_err(|_| "Observation cache unavailable")?
            .entries
            .contains_key(&key);
        let initial_lookup_us = prior_lookup_us + lookup_started.elapsed().as_micros();
        let validation_started = Instant::now();
        let mut validated_pattern = None;
        if let FileTool::StructuralSearch {
            directory,
            pattern,
            offset,
            snapshot,
            ..
        } = tool
        {
            if !known_query {
                match super::structural::Pattern::parse(pattern) {
                    Ok(pattern) => validated_pattern = Some(pattern),
                    Err(error) => {
                        return Ok(
                            json!({"status":"invalid_pattern","path":directory,"message":error,"complete":false}),
                        );
                    }
                }
            }
            if *offset > 256 || snapshot.len() > 64 || (*offset > 0 && snapshot.is_empty()) {
                return Err("Structural continuation requires the returned snapshot and bounded next_offset".into());
            }
        }
        let validation_us = validation_started.elapsed().as_micros();
        let fingerprint_started = Instant::now();
        let mut frozen = self.freeze_observation(tool, control)?;
        let fingerprint_us = fingerprint_started.elapsed().as_micros();
        if let FileTool::StructuralSearch { snapshot, .. } = tool {
            let expected = super::structural::snapshot_identity(tool, &frozen.fingerprint);
            if !snapshot.is_empty() && (!frozen.snapshot_available || snapshot != &expected) {
                return Err("Structural snapshot is stale or its policy/source boundary is unavailable; restart at offset=0,snapshot=\"\"".into());
            }
        }
        let lookup_started = Instant::now();
        let cached = if frozen.reusable {
            self.observation_cache
                .lock()
                .map_err(|_| "Observation cache unavailable")?
                .entries
                .get(&key)
                .filter(|entry| entry.fingerprint == frozen.fingerprint)
                .cloned()
        } else {
            None
        };
        let mut costs = Costs {
            fingerprint_us,
            hashed_bytes: frozen.hashed_bytes,
            lookup_us: initial_lookup_us + lookup_started.elapsed().as_micros(),
            store_us: 0,
        };
        if let Some(entry) = cached {
            // Preserve mutation-baseline enrollment even when formatting/parsing is reused.
            if let FileTool::Read { path, .. } = tool {
                if !self.observed.contains_key(path)
                    && self.observed.len() >= super::tools::SOURCE_FILES
                {
                    return Err(format!(
                        "Source read cache permits {} distinct files per request",
                        super::tools::SOURCE_FILES
                    ));
                }
                if let Some(source) = frozen.sources.pop() {
                    self.observed.insert(
                        path.clone(),
                        std::sync::Arc::new(SourceIdentity {
                            stamp: source.stamp,
                            hash: source.hash,
                        }),
                    );
                }
            }
            let mut value = entry.value;
            label(
                &mut value,
                true,
                entry.observed_at,
                entry.execution_us,
                true,
                &costs,
            );
            control.check()?;
            return Ok(value);
        }
        let observed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "Observation clock unavailable")?
            .as_millis();
        let execution_started = Instant::now();
        let mut value = match tool {
            FileTool::StructuralSearch { .. } => {
                super::structural::search(tool, &frozen, validated_pattern, control)?
            }
            FileTool::Read { .. } => {
                let source = frozen.sources.pop().ok_or("Read source unavailable")?;
                self.execute_observation(tool, Some(source), control)?
            }
            _ => unreachable!(),
        };
        let execution_us = execution_started.elapsed().as_micros() + validation_us;
        control.check()?;
        let store_started = Instant::now();
        let bytes = serde_json::to_vec(&value)
            .map_err(|_| "Cannot bound observation")?
            .len()
            + key.len();
        if frozen.reusable && bytes <= CACHE_BYTES {
            let mut cache = self
                .observation_cache
                .lock()
                .map_err(|_| "Observation cache unavailable")?;
            if let Some(previous) = cache.entries.remove(&key) {
                cache.bytes -= previous.bytes;
            }
            // No eviction/replay policy: a full request cache simply stops admitting entries.
            if cache.entries.len() < CACHE_ENTRIES && cache.bytes + bytes <= CACHE_BYTES {
                cache.bytes += bytes;
                cache.entries.insert(
                    key,
                    Entry {
                        fingerprint: frozen.fingerprint,
                        value: value.clone(),
                        observed_at,
                        execution_us,
                        bytes,
                    },
                );
            }
        }
        costs.store_us = store_started.elapsed().as_micros();
        label(
            &mut value,
            false,
            observed_at,
            execution_us,
            frozen.reusable,
            &costs,
        );
        control.check()?;
        Ok(value)
    }
}

fn hash_rule(
    directory: &platform::Directory,
    name: &str,
    hash: &mut Sha256,
    config: bool,
    control: &ObservationControl<'_>,
) -> Result<usize, String> {
    control.check()?;
    hash.update(name);
    if !directory.names(10_000)?.iter().any(|entry| entry == name) {
        hash.update(b"absent");
        return Ok(0);
    }
    let mut file = directory.open_file(std::ffi::OsStr::new(name), false)?;
    let stamp = platform::observation_stamp(&file)?;
    if stamp.size > 64 * 1024 {
        return Err("Ignore policy exceeds the cache fingerprint bound".into());
    }
    let mut bytes = Vec::with_capacity(stamp.size as usize);
    (&mut file)
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot fingerprint ignore policy")?;
    control.check()?;
    if bytes.len() > 64 * 1024 || platform::observation_stamp(&file)? != stamp {
        return Err("Ignore policy changed".into());
    }
    if config {
        let text = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
        if text.contains("include")
            || text.contains("excludesfile")
            || text.contains("worktreeconfig")
        {
            return Err("External Git policy bypasses project-private observation reuse".into());
        }
    }
    hash.update(format!("{stamp:?}"));
    hash.update(&bytes);
    directory.verify()?;
    Ok(bytes.len())
}

fn label(
    value: &mut Value,
    hit: bool,
    original: u128,
    execution_us: u128,
    eligible: bool,
    costs: &Costs,
) {
    value["observation_cache"] = json!({"status":if hit {"hit"} else if eligible {"miss"} else {"bypass"},
        "operation_version":VERSION,"scope":"request_private_project_bound","original_observed_at_unix_ms":original,
        "fingerprint_us":costs.fingerprint_us,"fingerprinted_bytes":costs.hashed_bytes,
        "cache_lookup_us":costs.lookup_us,"cache_store_us":costs.store_us,
        "fingerprinted_bytes_scope":"source and ignore/config body bytes; excludes identity/name framing",
        "original_execution_us":execution_us,"observation_executions_avoided":usize::from(hit),
        "result_computations":usize::from(!hit),
        "meaning":"A hit reuses the original read/search result after current fingerprint work. Fingerprinting and policy discovery are not avoided work; no test/build/network execution or billing saving is claimed."});
}
