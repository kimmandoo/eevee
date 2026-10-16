//! Advisory task recommendations, evaluated at event settlement rather than render.
use std::{fmt::Write, io, sync::Arc};

use serde::Serialize;
use sha2::{Digest, Sha256};
use unicode_segmentation::UnicodeSegmentation;

use super::{App, execution};
use crate::{
    store,
    workspace::{context, repo, runner, workflows},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub form: usize,
    pub reason: String,
    /// Local evidence identity, independent of presentation, drafts, and providers.
    pub trigger: String,
}

#[derive(Default)]
pub(super) struct Cache {
    report: Option<Arc<repo::Report>>,
    pub(super) current: runner::InputSnapshot,
    observed_diff: bool,
    inputs: Option<[u8; 32]>,
}

impl Cache {
    pub(super) fn invalidate(&mut self) {
        self.inputs = None;
    }

    /// Share one fingerprint per observed report across advisory consumers.
    pub(super) fn observe(&mut self, report: Option<&Arc<repo::Report>>) -> bool {
        let changed = match (self.report.as_ref(), report) {
            (Some(previous), Some(current)) => !Arc::ptr_eq(previous, current),
            (None, None) => false,
            _ => true,
        };
        if !changed {
            return false;
        }
        self.report = report.cloned();
        self.current = report
            .map(|report| execution::fingerprint(report))
            .unwrap_or_default();
        self.observed_diff = report.is_some_and(|report| {
            !report.changes.is_empty()
                || report
                    .current
                    .files
                    .values()
                    .any(|file| file.dirty == Some(true))
        });
        self.invalidate();
        true
    }
}

/// Borrow task state for one evaluation; the cache retains only a metadata digest,
/// never duplicate capture text, command arguments or complete run journals.
struct Inputs<'a> {
    task: i64,
    goal: &'a str,
    criteria: &'a [store::Criterion],
    excerpts: &'a [context::Excerpt],
    history: &'a [runner::RunRecord],
    active: Option<&'a runner::RunRecord>,
    review: Option<&'a workflows::Review>,
    unresolved_findings: bool,
    observation_available: bool,
}

impl<'a> Inputs<'a> {
    fn capture(app: &'a App) -> Self {
        Self {
            task: app.task_id,
            goal: &app.goal,
            criteria: &app.criteria,
            excerpts: &app.sources.basket,
            history: &app.execution.history,
            active: app
                .execution
                .active
                .as_ref()
                .filter(|run| run.prepared.task_id == app.task_id),
            review: app.workflows.review.as_ref(),
            unresolved_findings: app
                .workflows
                .findings
                .iter()
                .any(|finding| finding.resolution.is_none()),
            observation_available: app.repository.is_some()
                && app.repo_error.is_empty()
                && !app.repo_loading
                && !app.repo_refresh,
        }
    }

    fn pins(&self) -> impl Iterator<Item = &'a context::Excerpt> + '_ {
        self.excerpts
            .iter()
            .filter(|excerpt| excerpt.task_id == self.task && excerpt.id > 0)
    }

    fn latest(&self) -> Option<&'a runner::RunRecord> {
        let historical = self
            .history
            .iter()
            .find(|run| run.prepared.task_id == self.task);
        match (self.active, historical) {
            (Some(active), Some(latest)) => {
                if active.prepared.run_id == latest.prepared.run_id
                    || (!self
                        .history
                        .iter()
                        .any(|run| run.prepared.run_id == active.prepared.run_id)
                        && active.prepared.prepared_at_ms >= latest.prepared.prepared_at_ms)
                {
                    Some(active)
                } else {
                    Some(latest)
                }
            }
            (active, historical) => active.or(historical),
        }
    }

    fn key(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash_json(
            &mut hash,
            &(
                self.task,
                self.goal,
                self.criteria,
                self.review,
                self.unresolved_findings,
                self.observation_available,
            ),
        );
        for excerpt in self.pins() {
            // Source validation consumes identity, availability and bounds; review
            // checks actual source bytes rather than trusting capture display text.
            hash_json(
                &mut hash,
                &(
                    excerpt.id,
                    &excerpt.root,
                    &excerpt.path,
                    excerpt.start_line,
                    excerpt.end_line,
                    &excerpt.source_sha256,
                    excerpt.available,
                    excerpt.text.len(),
                ),
            );
        }
        for run in self
            .history
            .iter()
            .filter(|run| run.prepared.task_id == self.task)
            .chain(self.active)
        {
            hash_json(
                &mut hash,
                &(
                    &run.prepared.run_id,
                    run.prepared.prepared_at_ms,
                    &run.prepared.before,
                    &run.after,
                    run.state,
                    run.started_at_ms,
                    run.ended_at_ms,
                    run.exit_code,
                    run.signal,
                    run.tree,
                ),
            );
            hash_json(
                &mut hash,
                &(
                    run.cancelled,
                    run.truncated,
                    run.output_unavailable,
                    run.discarded_bytes,
                    &run.error,
                ),
            );
        }
        if let Some(latest) = self.latest() {
            hash_json(&mut hash, &latest.prepared.spec);
        }
        hash.finalize().into()
    }
}

impl App {
    /// Borrow the advisory cache without exposing a dismissed identity.
    pub fn visible_suggestion(&self) -> Option<&Suggestion> {
        self.suggestion.as_ref().filter(|suggestion| {
            self.dismissed_suggestion.as_deref() != Some(suggestion.trigger.as_str())
        })
    }

    /// Save only a task-local display preference, never an evidence or work action.
    pub fn dismiss_suggestion(&mut self) -> bool {
        if let Some(reason) = self.command_disabled_reason("suggestion-dismiss") {
            self.notice = reason;
            return false;
        }
        let Some(suggestion) = self.visible_suggestion() else {
            self.notice = "No visible suggestion to dismiss.".into();
            return false;
        };
        self.dismissed_suggestion = Some(suggestion.trigger.clone());
        self.changed_task();
        self.notice = "Suggestion dismissed for this task and evidence; no work started.".into();
        true
    }

    /// Returns whether the visible recommendation changed. No grants, mutations of
    /// stored evidence, or execution are performed. A new report (including an
    /// explicit refresh with identical fingerprints) revalidates source references.
    pub fn refresh_suggestion(&mut self) -> bool {
        let report_changed = self.suggestion_cache.observe(self.repository.as_ref());
        let inputs = Inputs::capture(self);
        let key = inputs.key();
        if !report_changed && self.suggestion_cache.inputs == Some(key) {
            return false;
        }
        let next = evaluate(
            &inputs,
            &self.suggestion_cache.current,
            self.suggestion_cache.observed_diff,
            &self.workflows,
        );
        self.suggestion_cache.inputs = Some(key);
        if self.suggestion == next {
            false
        } else {
            self.suggestion = next;
            true
        }
    }
}

#[derive(Clone, Copy)]
enum Rule {
    GoalNeedsCriteria,
    CriteriaNeedPins,
    LatestRunFailed,
    DiffNeedsRun,
    PassingRunNeedsReview,
    ReviewAccepted,
}

// Precedence is product behavior: exactly one ordered table, first match wins.
const RULES: &[(Rule, usize, &str)] = &[
    (Rule::GoalNeedsCriteria, 4, "goal-needs-criteria"),
    (Rule::CriteriaNeedPins, 1, "criteria-need-pins"),
    (Rule::LatestRunFailed, 5, "latest-run-failed"),
    (Rule::DiffNeedsRun, 2, "diff-needs-run"),
    (Rule::PassingRunNeedsReview, 7, "passing-run-needs-review"),
    (Rule::ReviewAccepted, 8, "review-accepted"),
];

fn evaluate(
    inputs: &Inputs<'_>,
    observed: &runner::InputSnapshot,
    diff: bool,
    state: &workflows::State,
) -> Option<Suggestion> {
    if inputs.task <= 0 {
        return None;
    }
    let unavailable = runner::InputSnapshot::default();
    let current = if inputs.observation_available {
        observed
    } else {
        &unavailable
    };
    let latest = inputs.latest();
    let view = workflows::EvidenceView {
        task_id: inputs.task,
        excerpts: inputs.excerpts,
        runs: latest.map(std::slice::from_ref).unwrap_or_default(),
        current,
    };
    let complete_current = latest.is_some_and(|run| {
        workflows::EvidenceRef::run(run)
            .is_ok_and(|reference| view.status(&reference) == workflows::EvidenceStatus::Current)
    });
    for &(rule, form, name) in RULES {
        let (reason, evidence) = match rule {
            Rule::GoalNeedsCriteria
                if !inputs.goal.trim().is_empty() && inputs.criteria.is_empty() =>
            {
                (
                    "The task goal has no acceptance criteria.".into(),
                    evidence_hash(&inputs.goal),
                )
            }
            Rule::CriteriaNeedPins
                if !inputs.criteria.is_empty() && inputs.pins().next().is_none() =>
            {
                (
                    "Acceptance criteria have no committed task source pins.".into(),
                    evidence_hash(&inputs.criteria),
                )
            }
            Rule::LatestRunFailed
                if latest.is_some_and(|run| run.state == runner::RunState::Failed) =>
            {
                let run = latest.expect("matched failed local run");
                (
                    failure_reason(run),
                    evidence_hash(&(
                        &run.prepared.run_id,
                        &run.prepared.spec,
                        run.exit_code,
                        run.signal,
                        &run.error,
                    )),
                )
            }
            Rule::DiffNeedsRun if diff && !complete_current => (
                "Observed source changes have no complete run at this revision.".into(),
                evidence_hash(&(observed, inputs.observation_available)),
            ),
            Rule::PassingRunNeedsReview
                if complete_current
                    && latest.is_some_and(|run| run.state == runner::RunState::Succeeded)
                    && inputs.review.is_none() =>
            {
                (
                    "A complete passing current run has no recorded review.".into(),
                    evidence_hash(&(
                        current,
                        &latest.expect("complete current run").prepared.run_id,
                    )),
                )
            }
            Rule::ReviewAccepted
                if inputs.review.is_some()
                    && inputs.active.is_none()
                    && current.complete
                    && current.stable
                    && current.fingerprint.is_some() =>
            {
                let view = workflows::EvidenceView {
                    runs: inputs.history,
                    ..view
                };
                let criteria: Vec<_> = inputs
                    .criteria
                    .iter()
                    .map(|criterion| criterion.text.clone())
                    .collect();
                if state.review_guard(&criteria, &view).verdict != workflows::Verdict::Accept {
                    continue;
                }
                (
                    "Independent review accepts the current task evidence.".into(),
                    evidence_hash(&(current, &criteria, &inputs.review)),
                )
            }
            _ => continue,
        };
        return Some(Suggestion {
            form,
            reason,
            trigger: format!("task:{}/{}:{}", inputs.task, name, evidence),
        });
    }
    None
}

struct HashWriter<'a>(&'a mut Sha256);

impl io::Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn hash_json(hash: &mut Sha256, value: &impl Serialize) {
    hash.update([0]);
    serde_json::to_writer(HashWriter(hash), value).expect("serializable local suggestion evidence");
}

fn evidence_hash(value: &impl Serialize) -> String {
    let mut hash = Sha256::new();
    hash_json(&mut hash, value);
    format!("{:x}", hash.finalize())
}

fn bound_summary(text: &mut String, graphemes: usize) {
    if let Some((end, _)) = text.grapheme_indices(true).nth(graphemes) {
        text.truncate(end);
        text.push('…');
    }
}

fn failure_reason(run: &runner::RunRecord) -> String {
    let mut command = execution::safe_text(&execution::command_summary(&run.prepared.spec));
    bound_summary(&mut command, 32);
    let mut reason = format!("Local {command} failed (");
    let mut separator = "";
    if let Some(code) = run.exit_code {
        write!(reason, "exit {code}").expect("String write");
        separator = "; ";
    }
    if let Some(signal) = run.signal {
        write!(reason, "{separator}signal {signal}").expect("String write");
        separator = "; ";
    }
    if let Some(error) = &run.error {
        let mut error = serde_json::to_string(error).expect("JSON string");
        bound_summary(&mut error, 48);
        write!(reason, "{separator}error {error}").expect("String write");
    } else if separator.is_empty() {
        reason.push_str("outcome details unavailable");
    }
    reason.push_str(").");
    reason
}

#[cfg(test)]
mod tests;
