//! Local held preferences are identities, never selected context or permission.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub partner: i64,
    pub form: usize,
    pub target: Target,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    File { path: String },
    Memory { id: i64, revision: u64 },
}

/// Frozen, transient source-worker review and its current policy. Never stored.
/// Construct only after the actual source preview has been explicitly reviewed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileProof {
    pub(crate) excerpt: super::context::Excerpt,
    pub(crate) exclusions: Vec<PathBuf>,
}

impl FileProof {
    pub(crate) fn new(excerpt: super::context::Excerpt, exclusions: Vec<PathBuf>) -> Self {
        Self {
            excerpt,
            exclusions,
        }
    }
}
