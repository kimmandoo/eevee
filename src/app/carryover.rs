//! Ephemeral, advisory snapshots of successful form switches. No persisted state.
use std::{fmt::Write, sync::Arc};

use sha2::{Digest, Sha256};

use super::{App, Overlay, suggestions::hash_json};
use crate::{
    domain::CAPABILITIES,
    workspace::{repo, runner, workflows},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarryOver {
    pub from: usize,
    pub to: usize,
    pub criteria_met: usize,
    pub criteria_total: usize,
    pub pins: usize,
    pub open_hypotheses: usize,
    pub open_findings: usize,
    pub summary: String,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) pending: Option<(usize, usize)>,
    pub(super) pending_visible: bool,
    last: Option<CarryOver>,
    visible: bool,
    explicit: bool,
    report: Option<Arc<repo::Report>>,
    inputs: Option<[u8; 32]>,
    open_hypotheses: usize,
}

impl State {
    pub(super) fn invalidate(&mut self) {
        self.inputs = None;
    }

    pub(super) fn hide(&mut self) -> bool {
        self.pending_visible = false;
        self.explicit = false;
        std::mem::replace(&mut self.visible, false)
    }
}

impl App {
    pub fn carry_over(&self) -> Option<&CarryOver> {
        self.carry_over
            .last
            .as_ref()
            .filter(|_| self.carry_over.visible)
    }

    pub fn last_carry_over(&self) -> Option<&CarryOver> {
        self.carry_over.last.as_ref()
    }

    pub fn carry_over_explicit(&self) -> bool {
        self.carry_over.explicit
    }

    /// Reopen exactly the prior snapshot, not a newly evaluated task summary.
    pub fn reopen_carry_over(&mut self) {
        if let Some(reason) = self.command_disabled_reason("carry-over") {
            self.notice = reason;
            return;
        }
        if self.carry_over.last.is_none() {
            self.notice = "No form carry-over yet; switch forms first.".into();
            return;
        }
        if self.overlay == Some(Overlay::Companion) {
            self.close_overlay();
        }
        self.carry_over.visible = true;
        self.carry_over.explicit = true;
    }

    pub(super) fn clear_carry_over(&mut self) {
        self.carry_over = State::default();
    }

    /// Called after task/context settlement, never from render or key dispatch.
    /// The digest excludes form, drafts, nickname and presentation. Only evidence
    /// metadata changes or a fresh report re-resolve source links. Snapshot text
    /// is built once per accepted switch and borrowed by the UI thereafter.
    pub fn refresh_carry_over(&mut self) -> bool {
        if self.sources.store_busy
            || self.sources.store_failed
            || (self.sources.basket.is_empty() && !self.sources.error.is_empty())
            || self.execution.history_pending(self.task_id)
        {
            return false;
        }
        let report_changed = match (&self.carry_over.report, &self.repository) {
            (Some(previous), Some(current)) => !Arc::ptr_eq(previous, current),
            (None, None) => false,
            _ => true,
        };
        if report_changed {
            self.carry_over.report = self.repository.clone();
        }
        self.suggestion_cache.observe(self.repository.as_ref());
        let key = evidence_key(self);
        if report_changed || self.carry_over.inputs != Some(key) {
            let unavailable = runner::InputSnapshot::default();
            let current = if self.repo_loading || self.repo_refresh || !self.repo_error.is_empty() {
                &unavailable
            } else {
                &self.suggestion_cache.current
            };
            let view = workflows::EvidenceView {
                task_id: self.task_id,
                excerpts: &self.sources.basket,
                runs: &self.execution.history,
                current,
            };
            self.carry_over.open_hypotheses = self
                .workflows
                .hypotheses
                .iter()
                .filter(|hypothesis| {
                    matches!(
                        self.workflows.hypothesis_status(hypothesis.id, &view),
                        workflows::HypothesisStatus::Unresolved
                            | workflows::HypothesisStatus::Conflicting
                    )
                })
                .count();
            self.carry_over.inputs = Some(key);
        }
        let Some((from, to)) = self.carry_over.pending.take() else {
            return false;
        };
        let mut record = CarryOver {
            from,
            to,
            criteria_met: self
                .criteria
                .iter()
                .filter(|criterion| criterion.met)
                .count(),
            criteria_total: self.criteria.len(),
            pins: self
                .sources
                .basket
                .iter()
                .filter(|excerpt| excerpt.task_id == self.task_id && excerpt.id > 0)
                .count(),
            open_hypotheses: self.carry_over.open_hypotheses,
            open_findings: self
                .workflows
                .findings
                .iter()
                .filter(|finding| finding.resolution.is_none())
                .count(),
            summary: String::new(),
        };
        record.summary = summary(&record);
        self.carry_over.last = Some(record);
        self.carry_over.explicit = false;
        self.carry_over.visible = std::mem::take(&mut self.carry_over.pending_visible);
        true
    }
}

fn evidence_key(app: &App) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash_json(
        &mut hash,
        &(
            app.task_id,
            app.repo_loading,
            app.repo_refresh,
            app.repo_error.is_empty(),
        ),
    );
    for hypothesis in &app.workflows.hypotheses {
        hash_json(&mut hash, &(hypothesis.id, &hypothesis.evidence));
    }
    for excerpt in app
        .sources
        .basket
        .iter()
        .filter(|excerpt| excerpt.task_id == app.task_id)
    {
        // These are precisely the capture fields consumed by EvidenceView::status
        // and Excerpt::validate; captured source payload is never cloned/hashed.
        hash_json(
            &mut hash,
            &(
                excerpt.id,
                &excerpt.root,
                &excerpt.path,
                excerpt.start_line,
                excerpt.end_line,
                excerpt.source_sha256,
                excerpt.available,
                excerpt.text.len(),
            ),
        );
    }
    for run in app
        .execution
        .history
        .iter()
        .filter(|run| run.prepared.task_id == app.task_id)
    {
        hash_json(
            &mut hash,
            &(
                &run.prepared.run_id,
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
    hash.finalize().into()
}

fn summary(record: &CarryOver) -> String {
    let mut text = String::with_capacity(56);
    let _ = write!(
        text,
        "Carry C{}/{} P{} H{} F{} |",
        record.criteria_met,
        record.criteria_total,
        record.pins,
        record.open_hypotheses,
        record.open_findings
    );
    let before = CAPABILITIES[record.from];
    let after = CAPABILITIES[record.to];
    let mut changed = false;
    for (sign, selected, other) in [('+', after, before), ('-', before, after)] {
        for token in selected.split('·').map(str::trim) {
            if !other
                .split('·')
                .map(str::trim)
                .any(|candidate| candidate == token)
            {
                changed = true;
                let _ = write!(text, " {sign}");
                for character in token.chars() {
                    text.push(if character == ' ' { '-' } else { character });
                }
            }
        }
    }
    if !changed {
        text.push_str(" caps unchanged");
    }
    text
}

#[cfg(test)]
mod tests;
