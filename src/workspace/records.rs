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

/// PK07 is an inspection of retained identities, never a new resolution record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureDisposition {
    UnresolvedNew,
    UnresolvedRecurring,
    HistoricalSupportedPass,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupportStatus {
    Retained,
    Stale,
    Missing,
    Unavailable,
    /// The existing run flag covers purge, missing and replaced spool bytes.
    PurgedOrUnavailable,
    DifferentCommand,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoricalSupport {
    pub support_artifact: String,
    pub passing_run: String,
    pub diagnosis: SupportStatus,
    pub pass: SupportStatus,
    pub source: Freshness,
    pub source_fingerprint: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailureComparison {
    pub signature: String,
    pub failure_run: String,
    pub command: super::runner::CommandSpec,
    pub cwd: std::path::PathBuf,
    pub source: Freshness,
    pub source_fingerprint: Option<String>,
    pub failure_support: SupportStatus,
    pub disposition: FailureDisposition,
    pub earlier_failures: Vec<String>,
    pub historical: Vec<HistoricalSupport>,
}

fn exact_command(spec: &super::runner::CommandSpec) -> bool {
    match spec {
        super::runner::CommandSpec::Direct { program, args } => {
            !program.contains("[REDACTED]") && args.iter().all(|arg| !arg.contains("[REDACTED]"))
        }
        super::runner::CommandSpec::Shell {
            executable, text, ..
        } => !executable.contains("[REDACTED]") && !text.contains("[REDACTED]"),
        super::runner::CommandSpec::SelectedCommit { .. } => false,
    }
}
fn same_check(a: &super::runner::RunRecord, b: &super::runner::RunRecord) -> bool {
    a.prepared.task_id == b.prepared.task_id
        && exact_command(&a.prepared.spec)
        && !a.prepared.cwd_identity.is_empty()
        && a.prepared.spec == b.prepared.spec
        && a.prepared.cwd == b.prepared.cwd
        && a.prepared.cwd_identity == b.prepared.cwd_identity
        && a.prepared.executable == b.prepared.executable
}
fn source_freshness(
    observed: &super::runner::InputSnapshot,
    current: &super::runner::InputSnapshot,
) -> Freshness {
    if !observed.complete
        || !observed.stable
        || observed.fingerprint.as_deref().is_none_or(str::is_empty)
        || !current.complete
        || !current.stable
        || current.fingerprint.as_deref().is_none_or(str::is_empty)
    {
        Freshness::Ambiguous
    } else if observed.fingerprint == current.fingerprint {
        Freshness::Current
    } else {
        Freshness::Stale
    }
}
fn retained_run(run: &super::runner::RunRecord) -> SupportStatus {
    use super::workflows::{EvidenceRef, EvidenceStatus, EvidenceView};
    if run.output_unavailable {
        return SupportStatus::PurgedOrUnavailable;
    }
    let view = EvidenceView {
        task_id: run.prepared.task_id,
        excerpts: &[],
        runs: std::slice::from_ref(run),
        current: &run.after,
    };
    match view.status(&EvidenceRef::Run {
        task_id: run.prepared.task_id,
        run_id: run.prepared.run_id.clone(),
    }) {
        EvidenceStatus::Current => SupportStatus::Retained,
        _ => SupportStatus::Unavailable,
    }
}

/// Exact task-local command/cwd/executable comparison. Signature text only chooses
/// candidate observations; it cannot merge command identities or project roots.
/// A retained historical pass remains historical even when its source matches.
pub fn compare_failures(
    task_id: i64,
    signatures: &[FailureSignature],
    resolutions: &[Resolution],
    artifacts: &[ArtifactView],
    runs: &[super::runner::RunRecord],
    current: &super::runner::InputSnapshot,
) -> Vec<FailureComparison> {
    use super::{runner::RunState, workflows::EvidenceRef};
    let mut comparisons = Vec::new();
    for signature in signatures {
        let mut failures: Vec<_> = runs
            .iter()
            .filter(|run| {
                run.prepared.task_id == task_id
                    && run.state == RunState::Failed
                    && signature.runs.contains(&run.prepared.run_id)
            })
            .collect();
        failures.sort_by(|a, b| {
            a.started_at_ms
                .cmp(&b.started_at_ms)
                .then_with(|| a.prepared.run_id.cmp(&b.prepared.run_id))
        });
        for (index, failure) in failures.iter().enumerate() {
            let earlier_failures: Vec<_> = failures[..index]
                .iter()
                .filter(|earlier| same_check(earlier, failure))
                .map(|earlier| earlier.prepared.run_id.clone())
                .collect();
            let mut historical = Vec::new();
            let mut supported_pass_after = false;
            for resolution in resolutions.iter().filter(|resolution| {
                resolution.signature == signature.key && resolution.task_id == task_id
            }) {
                let support = artifacts.iter().find(|view| {
                    view.artifact.id == resolution.support_artifact
                        && view.artifact.task_id == task_id
                });
                let pass = runs.iter().find(|run| {
                    run.prepared.run_id == resolution.passing_run && run.prepared.task_id == task_id
                });
                // Stale current diagnosis is expected after an observed fix, but
                // unavailable/purged supporting references cannot establish support.
                let references_retained = support.is_some_and(|view| {
                    view.artifact
                        .evidence
                        .iter()
                        .all(|reference| match reference {
                            EvidenceRef::Run {
                                task_id: owner,
                                run_id,
                            } => {
                                *owner == task_id
                                    && runs.iter().any(|run| {
                                        run.prepared.run_id == *run_id
                                            && retained_run(run) == SupportStatus::Retained
                                    })
                            }
                            EvidenceRef::Source { .. } => view.freshness != Freshness::Unavailable,
                        })
                });
                let diagnosis = match support {
                    None => SupportStatus::Missing,
                    Some(view) if !view.artifact.supported_diagnosis => SupportStatus::Unavailable,
                    Some(view) => {
                        let linked = view.artifact.evidence.iter().any(|reference| {
                            matches!(reference, EvidenceRef::Run { task_id: owner, run_id }
                                if *owner == task_id && failures.iter().any(|run|
                                    run.prepared.run_id == *run_id && same_check(run, failure)))
                        });
                        if !linked {
                            SupportStatus::DifferentCommand
                        } else if !references_retained {
                            SupportStatus::Unavailable
                        } else if view.freshness == Freshness::Stale {
                            SupportStatus::Stale
                        } else {
                            SupportStatus::Retained
                        }
                    }
                };
                let pass_status = pass.map_or(SupportStatus::Missing, |run| {
                    if !same_check(run, failure) {
                        SupportStatus::DifferentCommand
                    } else if run.state != RunState::Succeeded || run.exit_code != Some(0) {
                        SupportStatus::Unavailable
                    } else {
                        retained_run(run)
                    }
                });
                if pass_status == SupportStatus::Retained
                    && references_retained
                    && matches!(diagnosis, SupportStatus::Retained | SupportStatus::Stale)
                    && pass.is_some_and(|pass| {
                        failure.ended_at_ms.is_some() && pass.started_at_ms > failure.ended_at_ms
                    })
                {
                    supported_pass_after = true;
                }
                historical.push(HistoricalSupport {
                    support_artifact: resolution.support_artifact.clone(),
                    passing_run: resolution.passing_run.clone(),
                    diagnosis,
                    pass: pass_status,
                    source: pass.map_or(Freshness::Unavailable, |pass| {
                        source_freshness(&pass.after, current)
                    }),
                    source_fingerprint: pass.and_then(|pass| pass.after.fingerprint.clone()),
                });
            }
            let disposition = if supported_pass_after {
                FailureDisposition::HistoricalSupportedPass
            } else if !earlier_failures.is_empty() {
                FailureDisposition::UnresolvedRecurring
            } else {
                FailureDisposition::UnresolvedNew
            };
            comparisons.push(FailureComparison {
                signature: signature.key.clone(),
                failure_run: failure.prepared.run_id.clone(),
                command: failure.prepared.spec.clone(),
                cwd: failure.prepared.cwd.clone(),
                source: source_freshness(&failure.after, current),
                source_fingerprint: failure.after.fingerprint.clone(),
                failure_support: retained_run(failure),
                disposition,
                earlier_failures,
                historical,
            });
        }
    }
    comparisons
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
    pub failure_comparisons: Vec<FailureComparison>,
    pub habits: Vec<Habit>,
    pub pairings: Vec<Pairing>,
    pub held_items: Vec<super::held::Item>,
    pub ribbons: Vec<super::ribbons::View>,
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
