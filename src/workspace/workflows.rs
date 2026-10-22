//! Task-local specialist records. References are resolved on every consumption;
//! a saved conclusion, recipe, or provider statement is never an execution grant.
use std::{io::Read, path::Path, sync::atomic::AtomicBool};

use serde::{Deserialize, Serialize};

use crate::{workspace::context, workspace::handoff, workspace::repo, workspace::runner};

pub const MAX_ENTRIES: usize = 32;
pub const MAX_TEXT_BYTES: usize = 4096;
pub const MAX_RECIPE_BYTES: usize = 64 * 1024;
const MAX_STATE_BYTES: usize = 256 * 1024;
const MAX_LINKS: usize = 16;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct State {
    pub hypotheses: Vec<Hypothesis>,
    pub refactor: Option<Refactor>,
    pub findings: Vec<Finding>,
    pub review: Option<Review>,
    pub recipe: Option<AppliedRecipe>,
    pub chosen_decision: Option<ChosenDecision>,
    pub criterion_origins: Vec<CriterionOrigin>,
    pub relayed_findings: Vec<RelayedFinding>,
    pub review_stale: bool,
    pub command_relays: Vec<CommandRelay>,
    pub review_generation: u64,
    pub draft_relays: Vec<DraftRelay>,
    pub patch_authors: Vec<PatchAuthor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum EvidenceRef {
    Source {
        task_id: i64,
        excerpt_id: i64,
        path: String,
        start_line: usize,
        end_line: usize,
        source_sha256: [u8; 32],
    },
    Run {
        task_id: i64,
        run_id: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceStatus {
    Current,
    Stale,
    Ambiguous,
    Unavailable,
}

/// Supply committed, task-owned records and the latest repository observation.
/// Source resolution uses the existing bounded no-follow reader, so do not call
/// source-heavy evaluations from an animation/render loop.
pub struct EvidenceView<'a> {
    pub task_id: i64,
    pub excerpts: &'a [context::Excerpt],
    pub runs: &'a [runner::RunRecord],
    pub current: &'a runner::InputSnapshot,
}

impl EvidenceRef {
    pub fn source(excerpt: &context::Excerpt) -> Result<Self, String> {
        excerpt.validate()?;
        if excerpt.id <= 0 || !excerpt.available {
            return Err("Choose an available committed source excerpt".into());
        }
        Ok(Self::Source {
            task_id: excerpt.task_id,
            excerpt_id: excerpt.id,
            path: excerpt.path.clone(),
            start_line: excerpt.start_line,
            end_line: excerpt.end_line,
            source_sha256: excerpt.source_sha256,
        })
    }

    pub fn run(record: &runner::RunRecord) -> Result<Self, String> {
        let reference = Self::Run {
            task_id: record.prepared.task_id,
            run_id: record.prepared.run_id.clone(),
        };
        reference.validate(record.prepared.task_id)?;
        Ok(reference)
    }

    pub(crate) fn validate(&self, owner: i64) -> Result<(), String> {
        match self {
            Self::Source {
                task_id,
                excerpt_id,
                path,
                start_line,
                end_line,
                ..
            } => {
                if owner <= 0
                    || *task_id != owner
                    || *excerpt_id <= 0
                    || *start_line == 0
                    || end_line < start_line
                {
                    return Err("Source reference has invalid task ownership or range".into());
                }
                relative_path(path)
            }
            Self::Run { task_id, run_id } => {
                if owner <= 0 || *task_id != owner {
                    return Err("Run reference belongs to a different task".into());
                }
                run_identity(run_id)
            }
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Source {
                task_id,
                excerpt_id,
                path,
                start_line,
                end_line,
                ..
            } => format!("task:{task_id}/source:{excerpt_id} {path}:{start_line}-{end_line}"),
            Self::Run { task_id, run_id } => format!("task:{task_id}/run:{run_id}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRelay {
    pub partner: i64,
    pub run_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChosenDecision {
    pub id: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriterionOrigin {
    pub criterion: String,
    pub decision_id: String,
    pub decision_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayedFinding {
    pub finding: u64,
    pub author: i64,
    pub ranges: Vec<RangeBaseline>,
    pub addressed: bool,
    #[serde(default)]
    pub review_generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftRelay {
    pub partner: i64,
    pub form: usize,
    pub text: String,
    pub paths: Vec<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<ChosenDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_action: Option<String>,
}

pub fn role_action_form(action: &str) -> Option<usize> {
    match action {
        "command-edit" => Some(2),
        "paths" | "role-implement" => Some(3),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchAuthor {
    pub partner: i64,
    pub path: String,
    pub source_sha256: [u8; 32],
    pub record_id: String,
}

/// Only actual host-mediated write origins can establish authorship. Reads,
/// provider messages and external tools cannot supply this verified digest.
pub fn controlled_patch_author(record: &crate::store::ProviderRecord) -> Option<PatchAuthor> {
    let mediated = if record.agent == crate::providers::Agent::Codex {
        record.origin == "Codex-requested Eevee-controlled file write"
            && record.item_id.starts_with("eevee-write-")
    } else {
        record.agent.is_direct()
            && record.origin == "Eevee-mediated native file tool"
            && record.item_id.starts_with("eevee-native-write:")
    };
    if !mediated || !record.complete || record.text.len() > 8192 {
        return None;
    }
    let owner = record.partner.as_ref().filter(|owner| owner.partner > 0)?;
    let receipt: serde_json::Value = serde_json::from_str(&record.text).ok()?;
    if receipt.get("status")?.as_str()? != "success" {
        return None;
    }
    let path = receipt.get("path")?.as_str()?;
    relative_path(path).ok()?;
    let bytes = receipt.get("source_sha256")?.as_array()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut source_sha256 = [0; 32];
    for (slot, value) in source_sha256.iter_mut().zip(bytes) {
        *slot = u8::try_from(value.as_u64()?).ok()?;
    }
    Some(PatchAuthor {
        partner: owner.partner,
        path: path.into(),
        source_sha256,
        record_id: record.id.clone(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeBaseline {
    pub path: String,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Minimum line edit alignment, with ties preferring an untouched range.
/// Unlike a whole-file hash or hunk context, edits elsewhere do not address a
/// finding. Memory is linear in the new file; callers bound both files to 12 KiB.
pub fn range_touched(before: &str, after: &str, start: usize, end: usize) -> bool {
    if before == after {
        return false;
    }
    let old: Vec<_> = before.split_inclusive('\n').collect();
    let new: Vec<_> = after.split_inclusive('\n').collect();
    let mut previous: Vec<(usize, bool)> =
        (0..=new.len()).map(|n| (n, n > 0 && start == 1)).collect();
    let mut row = vec![(0, false); new.len() + 1];
    for (i, old_line) in old.iter().enumerate() {
        let inside = i + 1 >= start && i < end;
        row[0] = (i + 1, previous[0].1 || inside);
        for (j, new_line) in new.iter().enumerate() {
            if old_line == new_line {
                row[j + 1] = previous[j];
            } else {
                let deletion = (previous[j + 1].0 + 1, previous[j + 1].1 || inside);
                let insertion_inside = i + 2 >= start && i + 2 <= end;
                let insertion = (row[j].0 + 1, row[j].1 || insertion_inside);
                row[j + 1] = deletion.min(insertion);
            }
        }
        std::mem::swap(&mut row, &mut previous);
    }
    previous[new.len()].1
}

/// Positive deletion evidence only: every existing ancestor is observed as the
/// same contained directory before and after a no-follow NotFound result.
/// Links, permission errors, replaced roots and unavailable paths remain unknown.
pub fn confirmed_source_absence(root: &Path, relative: &Path) -> bool {
    use std::path::Component;
    if relative
        .to_str()
        .is_none_or(|path| relative_path(path).is_err())
    {
        return false;
    }
    let Ok(metadata) = std::fs::symlink_metadata(root) else {
        return false;
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    let Ok(canonical_root) = std::fs::canonicalize(root) else {
        return false;
    };
    if canonical_root != root {
        return false;
    }
    let Ok(identity) = runner::native::directory_identity(root) else {
        return false;
    };
    let mut directories = vec![(root.to_path_buf(), identity)];
    let mut path = root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return false;
        };
        path.push(name);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink()
                    || components.peek().is_none()
                    || !metadata.is_dir()
                {
                    return false;
                }
                let Ok(identity) = runner::native::directory_identity(&path) else {
                    return false;
                };
                directories.push((path.clone(), identity));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return directories.iter().all(|(directory, identity)| {
                    std::fs::symlink_metadata(directory).is_ok_and(|metadata| {
                        metadata.is_dir() && !metadata.file_type().is_symlink()
                    }) && runner::native::directory_identity(directory)
                        .is_ok_and(|current| current == *identity)
                });
            }
            Err(_) => return false,
        }
    }
    false
}

/// Explain ineligible retained baselines rather than silently substituting one.
pub fn baseline_eligibility(
    run: &runner::RunRecord,
    expected: &runner::Prepared,
    current: &runner::InputSnapshot,
) -> Result<(), String> {
    if run.revision_moving {
        return Err("revision moving during command execution".into());
    }
    if !passed(run) {
        return Err("not a complete retained passing run".into());
    }
    if !snapshot_known(current) || run.after.fingerprint != current.fingerprint {
        return Err("revision differs from the pre-refactor revision".into());
    }
    if run.prepared.spec != expected.spec {
        return Err("argv differs from the selected check".into());
    }
    if run.prepared.cwd != expected.cwd || run.prepared.cwd_identity != expected.cwd_identity {
        return Err("cwd identity differs".into());
    }
    if run.prepared.executable != expected.executable {
        return Err("executable identity differs".into());
    }
    Ok(())
}

fn snapshot_known(snapshot: &runner::InputSnapshot) -> bool {
    snapshot.complete
        && snapshot.stable
        && snapshot
            .fingerprint
            .as_ref()
            .is_some_and(|value| !value.is_empty())
}

fn historical_run_status(run: &runner::RunRecord) -> EvidenceStatus {
    if run.output_unavailable {
        return EvidenceStatus::Unavailable;
    }
    if !matches!(
        run.state,
        runner::RunState::Succeeded | runner::RunState::Failed
    ) || run.started_at_ms.is_none()
        || run.ended_at_ms.is_none()
        || run.started_at_ms > run.ended_at_ms
        || run.tree != runner::TreeOutcome::OwnedTreeExited
        || run.revision_moving
        || run.cancelled
        || run.truncated
        || run.discarded_bytes != 0
        || run.error.is_some()
        || (run.exit_code.is_none() && run.signal.is_none())
        || (run.state == runner::RunState::Succeeded
            && (run.exit_code != Some(0) || run.signal.is_some()))
        || (run.state == runner::RunState::Failed
            && run.exit_code == Some(0)
            && run.signal.is_none())
        || !snapshot_known(&run.prepared.before)
        || !snapshot_known(&run.after)
        || run.prepared.before.fingerprint != run.after.fingerprint
    {
        EvidenceStatus::Ambiguous
    } else {
        EvidenceStatus::Current
    }
}

fn passed(run: &runner::RunRecord) -> bool {
    run.state == runner::RunState::Succeeded
        && run.exit_code == Some(0)
        && run.signal.is_none()
        && historical_run_status(run) == EvidenceStatus::Current
}

impl<'a> EvidenceView<'a> {
    pub fn run(&self, run_id: &str) -> Option<&'a runner::RunRecord> {
        self.runs
            .iter()
            .find(|run| run.prepared.task_id == self.task_id && run.prepared.run_id == run_id)
    }

    pub fn status(&self, reference: &EvidenceRef) -> EvidenceStatus {
        if reference.validate(self.task_id).is_err() {
            return EvidenceStatus::Unavailable;
        }
        match reference {
            EvidenceRef::Run { run_id, .. } => {
                let Some(run) = self.run(run_id) else {
                    return EvidenceStatus::Unavailable;
                };
                let status = historical_run_status(run);
                if status != EvidenceStatus::Current {
                    return status;
                }
                if !snapshot_known(self.current) {
                    return EvidenceStatus::Ambiguous;
                }
                if run.after.fingerprint != self.current.fingerprint {
                    EvidenceStatus::Stale
                } else {
                    EvidenceStatus::Current
                }
            }
            EvidenceRef::Source {
                excerpt_id,
                path,
                start_line,
                end_line,
                source_sha256,
                ..
            } => {
                let Some(excerpt) = self.excerpts.iter().find(|excerpt| {
                    excerpt.task_id == self.task_id
                        && excerpt.id == *excerpt_id
                        && excerpt.path == *path
                        && excerpt.start_line == *start_line
                        && excerpt.end_line == *end_line
                        && excerpt.source_sha256 == *source_sha256
                        && excerpt.available
                        && excerpt.validate().is_ok()
                }) else {
                    return EvidenceStatus::Unavailable;
                };
                match repo::read_source(
                    Path::new(&excerpt.root),
                    Path::new(path),
                    context::MAX_SOURCE_BYTES,
                    &AtomicBool::new(false),
                ) {
                    Ok((_, hash)) if hash == *source_sha256 => EvidenceStatus::Current,
                    Ok(_) => EvidenceStatus::Stale,
                    Err(_) => EvidenceStatus::Unavailable,
                }
            }
        }
    }

    fn successful_current_run(&self, reference: &EvidenceRef) -> bool {
        match reference {
            EvidenceRef::Run { run_id, .. } => {
                self.status(reference) == EvidenceStatus::Current
                    && self.run(run_id).is_some_and(passed)
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceRelation {
    Supports,
    Refutes,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HypothesisEvidence {
    pub relation: EvidenceRelation,
    pub reference: EvidenceRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hypothesis {
    pub id: u64,
    /// An explanation is an inference, not an observed fact.
    pub explanation: String,
    pub evidence: Vec<HypothesisEvidence>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HypothesisStatus {
    Unresolved,
    Supported,
    Refuted,
    Conflicting,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refactor {
    pub contract: String,
    pub hotspot: Vec<EvidenceRef>,
    pub baseline_run: String,
    pub post_run: Option<String>,
    pub task_id: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comparison {
    pub behavior_preserved: bool,
    pub reason: String,
    pub baseline_run: Option<String>,
    pub post_run: Option<String>,
    pub baseline_ms: Option<u64>,
    pub post_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub id: u64,
    pub criterion: String,
    pub detail: String,
    pub next_action: String,
    pub evidence: Vec<EvidenceRef>,
    /// Explicit user resolution only; source edits never silently close a finding.
    pub resolution: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Accept,
    RequestChanges,
    Inconclusive,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriterionEvidence {
    pub criterion: String,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub requested: Verdict,
    pub criteria: Vec<CriterionEvidence>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewGuard {
    pub verdict: Verdict,
    pub reasons: Vec<String>,
}

impl State {
    pub fn validate(&self, task_id: i64) -> Result<(), String> {
        if self.hypotheses.len() > MAX_ENTRIES || self.findings.len() > MAX_ENTRIES {
            return Err("Workflow board exceeds 32 entries".into());
        }
        for (index, hypothesis) in self.hypotheses.iter().enumerate() {
            if hypothesis.id == 0
                || self.hypotheses[..index]
                    .iter()
                    .any(|h| h.id == hypothesis.id)
                || hypothesis.evidence.len() > MAX_LINKS
            {
                return Err("Hypothesis identity or evidence count is invalid".into());
            }
            text(&hypothesis.explanation, MAX_TEXT_BYTES)?;
            for link in &hypothesis.evidence {
                link.reference.validate(task_id)?;
            }
        }
        if let Some(refactor) = &self.refactor {
            if refactor.task_id != task_id || task_id <= 0 {
                return Err("Refactor belongs to a different task".into());
            }
            text(&refactor.contract, MAX_TEXT_BYTES)?;
            links(&refactor.hotspot, task_id)?;
            if refactor.hotspot.is_empty()
                || refactor
                    .hotspot
                    .iter()
                    .any(|r| !matches!(r, EvidenceRef::Source { .. }))
            {
                return Err("Refactor requires a bounded source hotspot".into());
            }
            run_identity(&refactor.baseline_run)?;
            if let Some(post) = &refactor.post_run {
                run_identity(post)?;
            }
        }
        for (index, finding) in self.findings.iter().enumerate() {
            if finding.id == 0 || self.findings[..index].iter().any(|f| f.id == finding.id) {
                return Err("Finding identity is invalid".into());
            }
            text(&finding.criterion, MAX_TEXT_BYTES)?;
            text(&finding.detail, MAX_TEXT_BYTES)?;
            text(&finding.next_action, MAX_TEXT_BYTES)?;
            links(&finding.evidence, task_id)?;
            if let Some(resolution) = &finding.resolution {
                text(resolution, MAX_TEXT_BYTES)?;
            }
        }
        if let Some(review) = &self.review {
            validate_criteria(&review.criteria, task_id)?;
        }
        if let Some(applied) = &self.recipe {
            if task_id <= 0 || applied.task_id != task_id {
                return Err("Recipe belongs to a different task".into());
            }
            applied.recipe.validate()?;
        }
        if self.criterion_origins.len() > 64 || self.relayed_findings.len() > MAX_ENTRIES {
            return Err("Relay provenance exceeds task bounds".into());
        }
        if let Some(decision) = &self.chosen_decision {
            text(&decision.id, 256)?;
            text(&decision.text, MAX_TEXT_BYTES)?;
        }
        for origin in &self.criterion_origins {
            text(&origin.criterion, 256)?;
            text(&origin.decision_id, 256)?;
            text(&origin.decision_text, MAX_TEXT_BYTES)?;
        }
        for relay in &self.relayed_findings {
            if relay.author <= 0
                || !self.findings.iter().any(|f| f.id == relay.finding)
                || relay.ranges.is_empty()
                || relay.ranges.len() > 16
            {
                return Err("Invalid relayed finding ownership or ranges".into());
            }
            for range in &relay.ranges {
                relative_path(&range.path)?;
                if range.start == 0 || range.end < range.start || range.text.len() > 12 * 1024 {
                    return Err("Invalid finding baseline range".into());
                }
                if crate::workspace::privacy::redact(&range.text) != range.text {
                    return Err("Credential-bearing raw range baselines cannot be retained".into());
                }
            }
        }
        if self.command_relays.len() > 6 {
            return Err("Command relay provenance exceeds six partners".into());
        }
        for relay in &self.command_relays {
            if relay.partner <= 0 {
                return Err("Invalid command relay owner".into());
            }
            run_identity(&relay.run_id)?;
        }
        if self.draft_relays.len() > 6 {
            return Err("Draft relay provenance exceeds six partners".into());
        }
        for relay in &self.draft_relays {
            if relay.partner <= 0 || relay.form > 8 || relay.paths.len() > 8 {
                return Err("Invalid draft relay scope".into());
            }
            text(&relay.text, MAX_TEXT_BYTES)?;
            if let Some(decision) = &relay.decision {
                text(&decision.id, 256)?;
                text(&decision.text, MAX_TEXT_BYTES)?;
                if relay.form != 0 {
                    return Err("Decision relay must target intake criteria".into());
                }
            }
            if let Some(action) = &relay.role_action
                && (role_action_form(action) != Some(relay.form)
                    || !relay.paths.is_empty()
                    || relay.decision.is_some())
            {
                return Err(
                    "Role-action relay transfers references only, not scope or decisions".into(),
                );
            }
            for path in &relay.paths {
                relative_path(path.to_str().ok_or("Relay path must be UTF-8")?)?;
            }
        }
        if self.patch_authors.len() > MAX_ENTRIES {
            return Err("Patch attribution exceeds 32 source paths".into());
        }
        for author in &self.patch_authors {
            if author.partner <= 0 {
                return Err("Invalid controlled patch author".into());
            }
            relative_path(&author.path)?;
            text(&author.record_id, 256)?;
        }
        if serde_json::to_vec(self)
            .map_err(|error| error.to_string())?
            .len()
            > MAX_STATE_BYTES
        {
            return Err("Workflow state exceeds 256 KiB".into());
        }
        Ok(())
    }

    pub fn add_hypothesis(&mut self, explanation: &str) -> Result<u64, String> {
        let explanation = clean(explanation)?;
        if self.hypotheses.len() >= MAX_ENTRIES {
            return Err("Hypothesis board is full".into());
        }
        let id = next_id(self.hypotheses.iter().map(|h| h.id))?;
        self.hypotheses.push(Hypothesis {
            id,
            explanation,
            evidence: Vec::new(),
        });
        Ok(id)
    }

    pub fn attach_hypothesis(
        &mut self,
        id: u64,
        relation: EvidenceRelation,
        reference: EvidenceRef,
        view: &EvidenceView<'_>,
    ) -> Result<(), String> {
        reference.validate(view.task_id)?;
        if view.status(&reference) == EvidenceStatus::Unavailable {
            return Err("Evidence reference is unavailable in this task".into());
        }
        let hypothesis = self
            .hypotheses
            .iter_mut()
            .find(|h| h.id == id)
            .ok_or("Hypothesis no longer exists")?;
        let link = HypothesisEvidence {
            relation,
            reference,
        };
        if hypothesis.evidence.contains(&link) {
            return Ok(());
        }
        if hypothesis.evidence.len() >= MAX_LINKS {
            return Err("Hypothesis has 16 evidence links".into());
        }
        hypothesis.evidence.push(link);
        Ok(())
    }

    pub fn hypothesis_status(&self, id: u64, view: &EvidenceView<'_>) -> HypothesisStatus {
        let Some(hypothesis) = self.hypotheses.iter().find(|h| h.id == id) else {
            return HypothesisStatus::Unresolved;
        };
        let mut source = false;
        let mut run = false;
        let mut refuted = false;
        for link in &hypothesis.evidence {
            if view.status(&link.reference) != EvidenceStatus::Current {
                continue;
            }
            match (link.relation, &link.reference) {
                (EvidenceRelation::Supports, EvidenceRef::Source { .. }) => source = true,
                (EvidenceRelation::Supports, EvidenceRef::Run { .. }) => run = true,
                (EvidenceRelation::Refutes, EvidenceRef::Run { .. }) => refuted = true,
                _ => {}
            }
        }
        match (source && run, refuted) {
            (true, true) => HypothesisStatus::Conflicting,
            (true, false) => HypothesisStatus::Supported,
            (false, true) => HypothesisStatus::Refuted,
            _ => HypothesisStatus::Unresolved,
        }
    }

    pub fn set_refactor(
        &mut self,
        contract: &str,
        hotspot: Vec<EvidenceRef>,
        baseline_run: &str,
        view: &EvidenceView<'_>,
    ) -> Result<(), String> {
        let contract = clean(contract)?;
        links(&hotspot, view.task_id)?;
        if hotspot.is_empty()
            || hotspot.iter().any(|r| {
                !matches!(r, EvidenceRef::Source { .. })
                    || view.status(r) != EvidenceStatus::Current
            })
        {
            return Err("Select current source evidence for the bounded hotspot".into());
        }
        let run = view
            .run(baseline_run)
            .ok_or("Select a baseline run from this task")?;
        let reference = EvidenceRef::run(run)?;
        if !view.successful_current_run(&reference) {
            return Err("Baseline must be a complete passing check of the current source".into());
        }
        self.refactor = Some(Refactor {
            contract,
            hotspot,
            baseline_run: baseline_run.into(),
            post_run: None,
            task_id: view.task_id,
        });
        Ok(())
    }

    pub fn set_post_run(&mut self, run_id: &str, view: &EvidenceView<'_>) -> Result<(), String> {
        let run = view
            .run(run_id)
            .ok_or("Post-change run is not in this task")?;
        EvidenceRef::run(run)?;
        let refactor = self
            .refactor
            .as_mut()
            .ok_or("Record the behavior contract and baseline first")?;
        if refactor.task_id != view.task_id {
            return Err("Refactor belongs to a different task".into());
        }
        refactor.post_run = Some(run_id.into());
        Ok(())
    }

    pub fn compare_refactor(&self, view: &EvidenceView<'_>) -> Comparison {
        let mut result = Comparison {
            behavior_preserved: false,
            reason: "Record a contract, source hotspot, baseline and post-change check".into(),
            baseline_run: None,
            post_run: None,
            baseline_ms: None,
            post_ms: None,
        };
        let Some(refactor) = &self.refactor else {
            return result;
        };
        result.baseline_run = Some(refactor.baseline_run.clone());
        result.post_run = refactor.post_run.clone();
        let Some(before) = view.run(&refactor.baseline_run) else {
            result.reason = "Baseline evidence is unavailable".into();
            return result;
        };
        let Some(after) = refactor.post_run.as_deref().and_then(|id| view.run(id)) else {
            result.reason = "Post-change evidence is unavailable".into();
            return result;
        };
        if refactor.task_id != view.task_id
            || before.prepared.run_id == after.prepared.run_id
            || before.prepared.spec != after.prepared.spec
            || before.prepared.cwd != after.prepared.cwd
            || before.prepared.cwd_identity != after.prepared.cwd_identity
            || before.prepared.executable != after.prepared.executable
            || before
                .ended_at_ms
                .zip(after.started_at_ms)
                .is_none_or(|(end, start)| end > start)
        {
            result.reason =
                "Comparison requires distinct ordered runs of the same check, cwd and executable"
                    .into();
            return result;
        }
        if !passed(before) || !passed(after) {
            result.reason = "Failed, incomplete, ambiguous or missing checks block a behavior-preservation claim".into();
            return result;
        }
        let reference = EvidenceRef::Run {
            task_id: view.task_id,
            run_id: after.prepared.run_id.clone(),
        };
        if view.status(&reference) != EvidenceStatus::Current {
            result.reason =
                "Post-change verification is stale or its source coverage is unknown".into();
            return result;
        }
        result.baseline_ms = before
            .ended_at_ms
            .zip(before.started_at_ms)
            .map(|(end, start)| end - start);
        result.post_ms = after
            .ended_at_ms
            .zip(after.started_at_ms)
            .map(|(end, start)| end - start);
        result.behavior_preserved = true;
        result.reason = "The selected contract check passed before and after. Inspect both linked outputs; this covers only that check. Single-run elapsed times do not establish a speedup.".into();
        result
    }

    pub fn add_finding(
        &mut self,
        criterion: &str,
        detail: &str,
        next_action: &str,
        evidence: Vec<EvidenceRef>,
        view: &EvidenceView<'_>,
    ) -> Result<u64, String> {
        let criterion = clean(criterion)?;
        let detail = clean(detail)?;
        let next_action = clean(next_action)?;
        links(&evidence, view.task_id)?;
        if evidence
            .iter()
            .any(|r| view.status(r) == EvidenceStatus::Unavailable)
        {
            return Err("Finding references unavailable evidence; record the missing proof without inventing a link".into());
        }
        if self.findings.len() >= MAX_ENTRIES {
            return Err("Review desk is full".into());
        }
        let id = next_id(self.findings.iter().map(|f| f.id))?;
        self.findings.push(Finding {
            id,
            criterion,
            detail,
            next_action,
            evidence,
            resolution: None,
        });
        Ok(id)
    }

    pub fn resolve_finding(&mut self, id: u64, resolution: &str) -> Result<(), String> {
        let resolution = clean(resolution)?;
        if self.relayed_findings.iter().any(|r| {
            r.finding == id && (self.review_stale || self.review_generation <= r.review_generation)
        }) {
            return Err(
                "Finding cannot close before a new verdict after the implementation relay.".into(),
            );
        }
        self.findings
            .iter_mut()
            .find(|f| f.id == id)
            .ok_or("Finding no longer exists")?
            .resolution = Some(resolution);
        Ok(())
    }

    pub fn update_review_evidence(
        &mut self,
        criteria: Vec<CriterionEvidence>,
        view: &EvidenceView<'_>,
    ) -> Result<(), String> {
        validate_criteria(&criteria, view.task_id)?;
        self.review
            .get_or_insert(Review {
                requested: Verdict::Inconclusive,
                criteria: Vec::new(),
            })
            .criteria = criteria;
        Ok(())
    }

    pub fn record_review(
        &mut self,
        verdict: Verdict,
        criteria: Vec<CriterionEvidence>,
        view: &EvidenceView<'_>,
    ) -> Result<(), String> {
        validate_criteria(&criteria, view.task_id)?;
        self.review = Some(Review {
            requested: verdict,
            criteria,
        });
        self.review_stale = false;
        self.review_generation = self.review_generation.saturating_add(1);
        Ok(())
    }

    /// Stored requests are not verdict authority. Re-evaluate against *current*
    /// task criteria, actual run results, and current source on every display/export.
    pub fn review_guard(&self, criteria: &[String], view: &EvidenceView<'_>) -> ReviewGuard {
        let mut guard = ReviewGuard {
            verdict: Verdict::Inconclusive,
            reasons: Vec::new(),
        };
        let Some(review) = &self.review else {
            guard
                .reasons
                .push("No independent review has been recorded".into());
            return guard;
        };
        if self.review_stale {
            guard.reasons.push("Verdict stale: addressed findings need a fresh independent re-review; no finding was closed".into());
            return guard;
        }
        if self.findings.iter().any(|f| f.resolution.is_none()) {
            guard.verdict = Verdict::RequestChanges;
            guard
                .reasons
                .push("Unresolved findings require a deliberate fix handoff".into());
        }
        if criteria.is_empty() {
            guard
                .reasons
                .push("No acceptance criteria are recorded".into());
        }
        for criterion in criteria.iter().take(MAX_ENTRIES + 1) {
            let Some(coverage) = review.criteria.iter().find(|c| c.criterion == *criterion) else {
                guard
                    .reasons
                    .push(format!("Missing criterion evidence: {criterion}"));
                continue;
            };
            let mut source = false;
            let mut run = false;
            for reference in &coverage.evidence {
                let status = view.status(reference);
                if status != EvidenceStatus::Current {
                    guard
                        .reasons
                        .push(format!("{criterion}: {status:?} {}", reference.label()));
                } else {
                    source |= matches!(reference, EvidenceRef::Source { .. });
                    run |= view.successful_current_run(reference);
                    if let EvidenceRef::Run { run_id, .. } = reference
                        && view.run(run_id).is_some_and(|r| !passed(r))
                    {
                        guard
                            .reasons
                            .push(format!("{criterion}: linked check did not pass"));
                    }
                }
            }
            if !source || !run {
                guard.reasons.push(format!(
                    "{criterion}: needs current source and a complete passing relevant check"
                ));
            }
        }
        if criteria.len() > MAX_ENTRIES {
            guard.reasons.push("Criteria exceed review bounds".into());
        }
        if guard.verdict != Verdict::RequestChanges {
            guard.verdict = if review.requested == Verdict::RequestChanges {
                Verdict::RequestChanges
            } else if review.requested == Verdict::Accept && guard.reasons.is_empty() {
                Verdict::Accept
            } else {
                Verdict::Inconclusive
            };
        }
        guard
    }

    pub fn apply_recipe(&mut self, preview: &RecipePreview, task_id: i64) -> Result<(), String> {
        if task_id <= 0 || preview.task_id != task_id {
            return Err("Recipe preview belongs to a different task; preview again".into());
        }
        preview.recipe.validate()?;
        self.recipe = Some(AppliedRecipe {
            task_id,
            recipe: preview.recipe.clone(),
        });
        Ok(())
    }

    /// Exportable context only: absence of a live evidence view cannot turn a
    /// saved diagnosis into verified fact or erase an unresolved risk.
    pub fn portable_notes(&self) -> (Vec<String>, Vec<String>) {
        let mut unresolved = Vec::new();
        for hypothesis in &self.hypotheses {
            let references = hypothesis
                .evidence
                .iter()
                .map(|link| {
                    format!(
                        "{:?} {}",
                        link.relation,
                        portable_reference(&link.reference)
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            unresolved.push(format!(
                "User/model inference, unresolved pending local evidence recheck: {}. References [{references}]. Currentness and availability are unverified.",
                portable_prose(&hypothesis.explanation)));
        }
        for finding in self
            .findings
            .iter()
            .filter(|finding| finding.resolution.is_none())
        {
            let references = finding
                .evidence
                .iter()
                .map(portable_reference)
                .collect::<Vec<_>>()
                .join("; ");
            unresolved.push(format!(
                "Unresolved user/model review finding {}: {} / {}. Next action: {}. References [{references}]. Currentness and availability are unverified; references are not portable execution receipts.",
                finding.id, portable_prose(&finding.criterion), portable_prose(&finding.detail),
                portable_prose(&finding.next_action)));
        }
        (Vec::new(), unresolved)
    }

    /// Separate selected review context, never provider history or an implicit
    /// success assertion. Actual journal observations must be supplied separately.
    pub fn review_observations(&self) -> Result<String, String> {
        #[derive(Serialize)]
        struct Observations<'a> {
            evidence_status: &'static str,
            findings: &'a [Finding],
            selected_criteria: &'a [CriterionEvidence],
            refactor: &'a Option<Refactor>,
        }
        let observations = Observations {
            evidence_status: "User/model statements and selected local references only. Availability, source freshness and check results must be rechecked locally; no grant or completion authority.",
            findings: &self.findings,
            selected_criteria: self
                .review
                .as_ref()
                .map_or(&[], |review| review.criteria.as_slice()),
            refactor: &self.refactor,
        };
        let encoded = serde_json::to_string(&observations).map_err(|error| error.to_string())?;
        if encoded.len() > MAX_RECIPE_BYTES {
            return Err("Selected review observations exceed 64 KiB; reduce the selection".into());
        }
        Ok(encoded)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub schema_version: u32,
    pub name: String,
    pub scope: Vec<String>,
    pub context: Vec<String>,
    pub steps: Vec<RecipeStep>,
    pub validation_commands: Vec<Vec<String>>,
    pub completion_criteria: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeStep {
    pub form: String,
    pub action: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedRecipe {
    pub task_id: i64,
    pub recipe: Recipe,
}

/// Private fields prevent applying an unreviewed, substituted recipe by mutating
/// its preview. This type contains no capabilities, process handles or grants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipePreview {
    task_id: i64,
    recipe: Recipe,
}

impl RecipePreview {
    pub fn recipe(&self) -> &Recipe {
        &self.recipe
    }
    pub fn text(&self) -> String {
        let mut text = format!(
            "Recipe schema {}: {}\nTask {}\nSTAGING ONLY: no commands, source/doc writes, grants or completion changes.\nAll writes and runs require normal independent approvals. Requested paths are not granted paths.\nScope: {}\nContext selection proposals: {}\n",
            self.recipe.schema_version,
            self.recipe.name,
            self.task_id,
            self.recipe.scope.join(", "),
            self.recipe.context.join(", ")
        );
        for (index, step) in self.recipe.steps.iter().enumerate() {
            text.push_str(&format!("{}. {}: {}\n", index + 1, step.form, step.action));
        }
        for command in &self.recipe.validation_commands {
            text.push_str(&format!(
                "Proposed validation argv (not executed): {}\n",
                serde_json::to_string(command).expect("string argv is serializable")
            ));
        }
        for criterion in &self.recipe.completion_criteria {
            text.push_str(&format!("Completion criterion (unverified): {criterion}\n"));
        }
        text
    }
}

impl Recipe {
    pub fn decode(input: &str) -> Result<Self, String> {
        if input.len() > MAX_RECIPE_BYTES {
            return Err("Recipe exceeds 64 KiB".into());
        }
        // Avoid reflecting arbitrary source lines/credentials in parser diagnostics.
        let recipe: Self =
            toml::from_str(input).map_err(|_| "Invalid strict recipe TOML schema".to_string())?;
        recipe.validate()?;
        Ok(recipe)
    }

    pub fn read_file(path: &Path) -> Result<Self, String> {
        let mut file = handoff::open_regular(path, false)?;
        if file
            .metadata()
            .map_err(|_| "Cannot inspect recipe file")?
            .len()
            > MAX_RECIPE_BYTES as u64
        {
            return Err("Recipe exceeds 64 KiB".into());
        }
        let mut input = String::new();
        Read::by_ref(&mut file)
            .take((MAX_RECIPE_BYTES + 1) as u64)
            .read_to_string(&mut input)
            .map_err(|_| "Cannot read recipe as bounded UTF-8 text")?;
        Self::decode(&input)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("Unsupported recipe schema; supported version is 1".into());
        }
        text(&self.name, 256)?;
        if self.scope.len() > MAX_ENTRIES
            || self.context.len() > MAX_ENTRIES
            || self.steps.is_empty()
            || self.steps.len() > MAX_ENTRIES
            || self.validation_commands.len() > MAX_ENTRIES
            || self.completion_criteria.is_empty()
            || self.completion_criteria.len() > MAX_ENTRIES
        {
            return Err(
                "Recipe requires 1–32 steps/criteria and at most 32 scope/context/command entries"
                    .into(),
            );
        }
        for path in self.scope.iter().chain(&self.context) {
            relative_path(path)?;
        }
        for step in &self.steps {
            if !crate::domain::FORMS
                .iter()
                .any(|form| form.name == step.form)
            {
                return Err("Recipe step must name an existing Eevee form".into());
            }
            text(&step.action, MAX_TEXT_BYTES)?;
        }
        for command in &self.validation_commands {
            if command.is_empty() || command.len() > 64 {
                return Err("Recipe command requires 1–64 literal argv entries".into());
            }
            for (index, argument) in command.iter().enumerate() {
                if index == 0 {
                    text(argument, MAX_TEXT_BYTES)?;
                } else {
                    safe_text(argument, MAX_TEXT_BYTES)?;
                }
            }
            // Reuse runner redaction for credentials split across argv as well.
            let joined = command.join(" ");
            if crate::workspace::privacy::redact(&joined) != joined
                || command.iter().any(|arg| {
                    matches!(
                        arg.trim_start_matches('-').to_ascii_lowercase().as_str(),
                        "password"
                            | "passwd"
                            | "token"
                            | "secret"
                            | "api-key"
                            | "api_key"
                            | "authorization"
                            | "client-secret"
                    )
                })
            {
                return Err("Recipe commands cannot contain recognizable credentials".into());
            }
        }
        for criterion in &self.completion_criteria {
            text(criterion, MAX_TEXT_BYTES)?;
        }
        if serde_json::to_vec(self)
            .map_err(|error| error.to_string())?
            .len()
            > MAX_RECIPE_BYTES
        {
            return Err("Recipe model exceeds 64 KiB".into());
        }
        Ok(())
    }

    pub fn preview(&self, task_id: i64) -> Result<RecipePreview, String> {
        if task_id <= 0 {
            return Err("Save a task before previewing a recipe".into());
        }
        self.validate()?;
        Ok(RecipePreview {
            task_id,
            recipe: self.clone(),
        })
    }
}

fn clean(value: &str) -> Result<String, String> {
    if value.len() > MAX_TEXT_BYTES {
        return Err("Workflow text exceeds 4096 bytes".into());
    }
    let value = handoff::sanitize(value);
    text(&value, MAX_TEXT_BYTES)?;
    Ok(value)
}

fn safe_text(value: &str, limit: usize) -> Result<(), String> {
    if value.len() > limit || handoff::sanitize(value) != value {
        return Err("Workflow text is oversized or contains credentials/unsafe controls".into());
    }
    Ok(())
}

fn text(value: &str, limit: usize) -> Result<(), String> {
    safe_text(value, limit)?;
    if value.trim().is_empty() {
        return Err("Workflow text must not be empty".into());
    }
    Ok(())
}

fn relative_path(value: &str) -> Result<(), String> {
    text(value, MAX_TEXT_BYTES)?;
    if value.contains([':', '\\', '*', '?', '[', ']'])
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || context::excluded(Path::new(value), &[])
    {
        return Err("Choose a literal relative non-secret path without traversal or globs".into());
    }
    Ok(())
}

fn links(references: &[EvidenceRef], task_id: i64) -> Result<(), String> {
    if references.len() > MAX_LINKS {
        return Err("At most 16 evidence links are allowed".into());
    }
    for reference in references {
        reference.validate(task_id)?;
    }
    Ok(())
}

fn validate_criteria(criteria: &[CriterionEvidence], task_id: i64) -> Result<(), String> {
    if criteria.len() > MAX_ENTRIES {
        return Err("Review exceeds 32 criteria".into());
    }
    for (index, criterion) in criteria.iter().enumerate() {
        text(&criterion.criterion, MAX_TEXT_BYTES)?;
        links(&criterion.evidence, task_id)?;
        if criteria[..index]
            .iter()
            .any(|other| other.criterion == criterion.criterion)
        {
            return Err("Review criterion is duplicated".into());
        }
    }
    Ok(())
}

fn next_id(ids: impl Iterator<Item = u64>) -> Result<u64, String> {
    ids.max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| "Workflow identity exhausted".into())
}

fn portable_reference(reference: &EvidenceRef) -> String {
    match reference {
        EvidenceRef::Source {
            task_id,
            excerpt_id,
            start_line,
            end_line,
            ..
        } => format!("task:{task_id}/source:{excerpt_id} lines:{start_line}-{end_line}"),
        EvidenceRef::Run { task_id, run_id } => format!("task:{task_id}/run:{run_id}"),
    }
}

fn portable_prose(value: &str) -> String {
    const LIMIT: usize = 2048;
    if value.len() <= LIMIT {
        return value.to_string();
    }
    let mut end = LIMIT;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} [abridged; inspect local task record]", &value[..end])
}

fn run_identity(value: &str) -> Result<(), String> {
    text(value, 96)?;
    if !crate::store::valid_run_id(value) {
        return Err("Run reference must use an exact local journal identity".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
