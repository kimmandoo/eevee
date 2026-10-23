//! Inert, explicitly imported metadata. Search relevance never grants tool authority.
//! Catalog routes are closed Eevee-owned executors, not arbitrary MCP/SDK forwarding.
use super::{Agent, Operation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub const MAX_BYTES: usize = 512 * 1024;
pub const MAX_TOOLS: usize = 512;
pub const EXECUTOR_VERSION: &str = "eevee-file-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    ListFiles,
    ReadFile,
    WriteFile,
    EditFile,
}
impl Route {
    pub fn name(self) -> &'static str {
        match self {
            Self::ListFiles => "eevee_list_files",
            Self::ReadFile => "eevee_read_file",
            Self::WriteFile => "eevee_write_file",
            Self::EditFile => "eevee_edit_file",
        }
    }
    pub fn effects(self) -> Vec<Effect> {
        match self {
            Self::ListFiles | Self::ReadFile => vec![Effect::ReadProject],
            Self::WriteFile | Self::EditFile => vec![Effect::WriteProject, Effect::CreateParents],
        }
    }
    pub fn mutating(self) -> bool {
        matches!(self, Self::WriteFile | Self::EditFile)
    }
    fn definition(self) -> Value {
        super::writes::definitions(true)
            .into_iter()
            .find(|entry| entry["name"] == self.name())
            .expect("closed owned route")
    }
    pub fn schema(self) -> Value {
        self.definition()["parameters"].clone()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    ReadProject,
    WriteProject,
    CreateParents,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub namespace: String,
    pub name: String,
    pub version: String,
    /// Untrusted search/display data; deliberately never a model-facing description.
    pub description: String,
    pub effects: Vec<Effect>,
    pub route: Route,
    pub parameters: Value,
}
impl Entry {
    pub fn identity(&self) -> String {
        format!("{}::{}@{}", self.namespace, self.name, self.version)
    }
    pub fn wire_name(&self) -> String {
        format!("catalog__{}__{}", self.namespace, self.name)
    }
    pub fn validate(&self) -> Result<(), String> {
        let component = |text: &str| {
            !text.is_empty()
                && text.len() <= 24
                && !text.contains("__")
                && text
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        };
        if !component(&self.namespace) || !component(&self.name) || self.namespace == "eevee" {
            return Err("Catalog names require nonreserved lowercase ASCII namespace/name components of at most 24 bytes; double underscores are reserved".into());
        }
        if self.version != EXECUTOR_VERSION
            || self.parameters != self.route.schema()
            || self.effects != self.route.effects()
        {
            return Err(format!(
                "{}: schema/version/effects differ from the owned executor; reviewed setup required",
                self.identity()
            ));
        }
        safe_text(&self.description, 4096)?;
        Ok(())
    }
    pub fn declaration(&self) -> Value {
        let mut definition = self.route.definition();
        definition["name"] = json!(self.wire_name());
        // Never promote imported prose to tool instructions. Only the executor's
        // own reviewed contract is exposed; inert names/version remain identities.
        definition["description"] = json!(format!(
            "Reviewed catalog alias {}. {}",
            self.identity(),
            definition["description"].as_str().unwrap_or_default()
        ));
        definition["parameters"] = self.parameters.clone();
        definition
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub tools: Vec<Entry>,
}
impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.tools.is_empty() || self.tools.len() > MAX_TOOLS {
            return Err("Catalog schema 1 requires 1..512 tools".into());
        }
        let mut identities = BTreeSet::new();
        for tool in &self.tools {
            tool.validate()?;
            if !identities.insert(tool.wire_name()) {
                return Err("Duplicate namespaced catalog identity; versions cannot silently replace one another".into());
            }
        }
        if serde_json::to_vec(self)
            .map_err(|_| "Cannot encode catalog")?
            .len()
            > MAX_BYTES
        {
            return Err("Catalog exceeds 512 KiB".into());
        }
        Ok(())
    }
    /// Deterministic literal ranking. Imported prose is never parsed as commands.
    pub fn search(&self, query: &str) -> Vec<usize> {
        let query = query.to_lowercase();
        let words: Vec<_> = query.split_whitespace().collect();
        let mut ranked: Vec<_> = self
            .tools
            .iter()
            .enumerate()
            .map(|(index, tool)| {
                let name = tool.name.to_lowercase();
                let identity = tool.identity().to_lowercase();
                let description = tool.description.to_lowercase();
                let score = if query.is_empty() {
                    0
                } else {
                    usize::from(name == query || identity == query) * 100_000
                        + usize::from(name.starts_with(&query)) * 10_000
                        + usize::from(name.contains(&query)) * 5_000
                        + words
                            .iter()
                            .map(|word| {
                                usize::from(name.contains(word)) * 100
                                    + usize::from(identity.contains(word)) * 25
                                    + usize::from(description.contains(word)) * 5
                            })
                            .sum::<usize>()
                };
                (index, score)
            })
            .filter(|(_, score)| query.is_empty() || *score > 0)
            .collect();
        ranked.sort_by(|(ai, a), (bi, b)| {
            b.cmp(a)
                .then_with(|| self.tools[*ai].identity().cmp(&self.tools[*bi].identity()))
        });
        ranked.into_iter().map(|(index, _)| index).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema_version: u32,
    pub source_path: PathBuf,
    pub source_sha256: String,
    pub agent: Agent,
    /// Opaque connection/config identity; not credentials or prompt data.
    pub connection_scope: String,
    pub manifest: Manifest,
}
impl Document {
    pub fn import(path: &Path, agent: Agent, connection_scope: String) -> Result<Self, String> {
        supported(agent)?;
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(path)
        };
        let source = crate::workspace::handoff::read_bounded(&path, MAX_BYTES)?;
        let manifest: Manifest = serde_json::from_str(&source)
            .map_err(|e| format!("Invalid strict catalog JSON: {e}"))?;
        let document = Self {
            schema_version: 1,
            source_path: path,
            source_sha256: digest(source.as_bytes()),
            agent,
            connection_scope,
            manifest,
        };
        document.validate()?;
        Ok(document)
    }
    /// Durable inspection must not make an executor-schema upgrade corrupt the
    /// whole store. Old exact schemas remain inert and require fresh setup.
    pub fn validate_stored(&self) -> Result<(), String> {
        supported(self.agent)?;
        if self.schema_version != 1
            || !self.source_path.is_absolute()
            || !hex_digest(&self.source_sha256)
            || (!self.connection_scope.is_empty() && !hex_digest(&self.connection_scope))
            || self.manifest.schema_version != 1
            || self.manifest.tools.is_empty()
            || self.manifest.tools.len() > MAX_TOOLS
        {
            return Err("Invalid stored catalog identity or bounds".into());
        }
        safe_text(&self.source_path.to_string_lossy(), 4096)?;
        let mut names = BTreeSet::new();
        for entry in &self.manifest.tools {
            safe_text(&entry.description, 4096)?;
            safe_text(&entry.version, 128)?;
            if entry.version.is_empty()
                || !entry.parameters.is_object()
                || entry.namespace.is_empty()
                || entry.namespace.len() > 24
                || entry.name.is_empty()
                || entry.name.len() > 24
                || entry.namespace == "eevee"
                || entry.namespace.contains("__")
                || entry.name.contains("__")
                || !entry
                    .namespace
                    .bytes()
                    .chain(entry.name.bytes())
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                || !names.insert(entry.wire_name())
            {
                return Err("Invalid stored namespaced tool metadata".into());
            }
            safe_text(
                &serde_json::to_string(&entry.parameters)
                    .map_err(|_| "Cannot encode stored schema")?,
                64 * 1024,
            )?;
        }
        if serde_json::to_vec(self)
            .map_err(|_| "Cannot encode trusted catalog")?
            .len()
            > MAX_BYTES
        {
            return Err("Trusted catalog exceeds 512 KiB".into());
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        supported(self.agent)?;
        if self.schema_version != 1
            || !self.source_path.is_absolute()
            || self.source_path.as_os_str().len() > 4096
            || !hex_digest(&self.source_sha256)
            || (!self.connection_scope.is_empty() && !hex_digest(&self.connection_scope))
        {
            return Err("Invalid catalog setup origin/connection identity".into());
        }
        safe_text(&self.source_path.to_string_lossy(), 4096)?;
        self.manifest.validate()?;
        if serde_json::to_vec(self)
            .map_err(|_| "Cannot encode trusted catalog")?
            .len()
            > MAX_BYTES
        {
            return Err("Trusted catalog exceeds 512 KiB".into());
        }
        Ok(())
    }
    pub fn revalidate(&self, connection_scope: &str) -> Result<(), String> {
        self.validate()?;
        if self.connection_scope.is_empty() || self.connection_scope != connection_scope {
            return Err(
                "Catalog connection changed or disconnected; reviewed tool setup required".into(),
            );
        }
        let source = crate::workspace::handoff::read_bounded(&self.source_path, MAX_BYTES)
            .map_err(|e| format!("Catalog source unavailable; reviewed setup required: {e}"))?;
        if digest(source.as_bytes()) != self.source_sha256 {
            return Err("Catalog source/schema changed; reviewed setup required".into());
        }
        Ok(())
    }
    pub fn select(
        &self,
        indices: &[usize],
        task_id: i64,
        partner: i64,
    ) -> Result<Selection, String> {
        if indices.is_empty() || indices.len() > 16 {
            return Err("Select 1..16 exact catalog tools".into());
        }
        let mut unique = BTreeSet::new();
        let entries = indices
            .iter()
            .map(|index| {
                if !unique.insert(*index) {
                    return Err("Duplicate tool selection".into());
                }
                self.manifest
                    .tools
                    .get(*index)
                    .cloned()
                    .ok_or_else(|| "Catalog selection changed".into())
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.revalidate(&self.connection_scope)?;
        Ok(Selection {
            document_digest: digest(
                &serde_json::to_vec(self).map_err(|_| "Cannot encode catalog")?,
            ),
            source_path: self.source_path.clone(),
            source_sha256: self.source_sha256.clone(),
            connection_scope: self.connection_scope.clone(),
            agent: self.agent,
            task_id,
            partner,
            entries,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    pub document_digest: String,
    pub source_path: PathBuf,
    pub source_sha256: String,
    pub connection_scope: String,
    pub agent: Agent,
    pub task_id: i64,
    pub partner: i64,
    pub entries: Vec<Entry>,
}
impl Selection {
    pub fn validate(
        &self,
        agent: Agent,
        task_id: i64,
        partner: i64,
        operation: Operation,
        connection_scope: &str,
    ) -> Result<(), String> {
        supported(agent)?;
        if self.agent != agent
            || self.task_id != task_id
            || self.partner != partner
            || self.connection_scope != connection_scope
            || self.entries.is_empty()
            || self.entries.len() > 16
        {
            return Err(
                "Catalog selection owner or connection changed; reviewed setup/selection required"
                    .into(),
            );
        }
        for entry in &self.entries {
            entry.validate()?;
            if entry.route.mutating() && operation != Operation::Implement {
                return Err("Selected catalog mutation tool is unavailable in read-only operations; deselect it or explicitly choose Implement".into());
            }
        }
        let source = crate::workspace::handoff::read_bounded(&self.source_path, MAX_BYTES)
            .map_err(|e| {
                format!("Catalog disconnected/source unavailable; reviewed setup required: {e}")
            })?;
        if digest(source.as_bytes()) != self.source_sha256 {
            return Err("Catalog schema changed; reviewed setup required".into());
        }
        Ok(())
    }
    pub fn declarations(&self) -> Vec<Value> {
        self.entries.iter().map(Entry::declaration).collect()
    }
}

pub fn supported(agent: Agent) -> Result<(), String> {
    if agent == Agent::Compatible {
        Ok(())
    } else {
        Err("Lazy trusted catalog exposure unavailable on this adapter; custom/local Chat Completions is supported. Closed CLI/SDK/MCP tools remain unchanged".into())
    }
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hex_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn safe_text(value: &str, limit: usize) -> Result<(), String> {
    if value.len() > limit || crate::workspace::privacy::redact(value) != value || value.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) { Err("Catalog metadata contains private data, terminal controls or oversized text".into()) } else { Ok(()) }
}

pub struct TokenComparison {
    pub tokenizer: &'static str,
    pub selected: usize,
    pub full: usize,
}
pub fn token_comparison(
    manifest: &Manifest,
    selected: &[usize],
) -> Result<TokenComparison, String> {
    use crate::workspace::context::optimizer::{TOKENIZER, token_count};
    let selected = selected
        .iter()
        .map(|index| {
            manifest
                .tools
                .get(*index)
                .map(Entry::declaration)
                .ok_or("Stale catalog selection")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let full: Vec<_> = manifest.tools.iter().map(Entry::declaration).collect();
    Ok(TokenComparison {
        tokenizer: TOKENIZER,
        selected: token_count(
            &serde_json::to_string(&selected).map_err(|_| "Cannot encode selected schemas")?,
        ),
        full: token_count(
            &serde_json::to_string(&full).map_err(|_| "Cannot encode catalog schemas")?,
        ),
    })
}

#[cfg(test)]
mod tests;
