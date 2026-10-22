//! Local factual records. These metadata never enter provider packets or handoffs.
use crate::store::Attribution;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactKind {
    Capture,
    Run,
    Hypothesis,
    Refactor,
    Review,
    Handoff,
    Patch,
    Decision,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Freshness {
    Current,
    Stale,
    Unavailable,
    Ambiguous,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub task_id: i64,
    pub kind: ArtifactKind,
    pub role_form: usize,
    pub ordinal: u64,
    pub supported_diagnosis: bool,
    /// Original validated handoff document revision; absent for legacy/other artifacts.
    #[serde(default)]
    pub handoff_revision: Option<u64>,
    pub observation: Option<crate::workspace::runner::InputSnapshot>,
    pub producer: Option<Attribution>,
    pub reference: String,
    pub created_at: i64,
    pub evidence: Vec<crate::workspace::workflows::EvidenceRef>,
    pub detail: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactView {
    pub artifact: Artifact,
    pub freshness: Freshness,
    pub explanation: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub id: String,
    pub task_id: i64,
    pub ordinal: u64,
    pub owner: Attribution,
    pub from: usize,
    pub to: usize,
    pub at: i64,
    pub accepted_suggestion: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    pub id: String,
    pub task_id: i64,
    pub owner: Option<Attribution>,
    pub at: i64,
    #[serde(serialize_with = "serialize_activity_detail")]
    pub detail: String,
}

fn serialize_activity_detail<S: serde::Serializer>(
    value: &str,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    // Formatting projected fields can create new credential-shaped fragments.
    serializer.serialize_str(crate::workspace::privacy::redact(value).as_ref())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureSignature {
    pub key: String,
    pub kind: String,
    pub detail: String,
    pub first_seen: i64,
    pub seen_count: usize,
    pub runs: Vec<String>,
    pub paths: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub signature: String,
    pub task_id: i64,
    pub support_artifact: String,
    pub passing_run: String,
    pub at: i64,
    pub cause: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolutionChoice {
    pub signature: String,
    pub support_artifact: String,
    pub passing_run: String,
    pub cause: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionSource {
    pub request_id: String,
    pub task_id: i64,
    pub owner: Attribution,
    pub at: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryBinding {
    pub id: i64,
    pub revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Habit {
    pub key: String,
    pub instruction: String,
    pub sources: Vec<InstructionSource>,
    pub dismissed: bool,
    pub memory: Option<MemoryBinding>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pairing {
    pub partner: i64,
    pub form: usize,
    pub agent: crate::providers::Agent,
    pub selection: crate::providers::ModelSelection,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartnerSummary {
    pub partner: crate::store::Partner,
    pub produced_tasks: [usize; 9],
    pub moves: [usize; 9],
    pub move_names: [Vec<String>; 9],
    pub shared_bond_memories: usize,
    pub unknown_producer_artifacts: usize,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub artifacts: Vec<ArtifactView>,
    pub routes: Vec<Route>,
    pub activities: Vec<Activity>,
    pub signatures: Vec<FailureSignature>,
    pub resolutions: Vec<Resolution>,
    pub resolution_choices: Vec<ResolutionChoice>,
    pub habits: Vec<Habit>,
    pub pairings: Vec<Pairing>,
    pub held_items: Vec<super::held::Item>,
    pub usages: Vec<crate::workspace::usage::Record>,
    pub summaries: Vec<PartnerSummary>,
    pub flavor_enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalIntent {
    FormRoute {
        id: String,
        owner: Attribution,
        from: usize,
        to: usize,
        accepted_suggestion: Option<String>,
    },
    Artifact {
        id: String,
        owner: Attribution,
        kind: ArtifactKind,
        reference: String,
        current: Option<crate::workspace::runner::InputSnapshot>,
    },
    HandoffReady {
        id: String,
        owner: Attribution,
        revision: u64,
        digest: String,
        evidence: Vec<crate::workspace::workflows::EvidenceRef>,
    },
}
impl LocalIntent {
    pub fn id(&self) -> &str {
        match self {
            Self::FormRoute { id, .. }
            | Self::Artifact { id, .. }
            | Self::HandoffReady { id, .. } => id,
        }
    }
}

pub fn instruction_key(text: &str) -> Result<(String, String), String> {
    use sha2::{Digest, Sha256};
    // Only outer whitespace is equivalent. Literal codepoints inside an
    // instruction can identify different paths, strings or source bytes.
    if text.len() > crate::workspace::memory::MAX_CONTENT_BYTES {
        return Err("Instruction exceeds the project-memory byte limit".into());
    }
    let normalized = text.trim();
    if normalized.is_empty() {
        return Err("Empty instructions cannot become habits".into());
    }
    crate::workspace::memory::validate_text(
        normalized,
        crate::workspace::memory::MAX_CONTENT_BYTES,
    )?;
    if normalized.contains("[REDACTED]") {
        return Err("Masked instructions cannot become habits".into());
    }
    Ok((
        format!("{:x}", Sha256::digest(normalized.as_bytes())),
        normalized.to_owned(),
    ))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionReplacement {
    pub original: String,
    pub replacement: String,
    pub memory_id: i64,
    pub memory_revision: u64,
    pub habit_key: String,
}

impl InstructionReplacement {
    pub fn reference_text(memory_id: i64, memory_revision: u64) -> String {
        format!(
            "Use selected approved project memory #{} revision {} as the current instruction (exact-repeat substitution).",
            memory_id, memory_revision
        )
    }
}
