//! Owner-bound, ephemeral impact recommendations on the existing specialist desk.
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::{
    app::{App, Overlay},
    workspace::{context, repo, workflows::impact},
};

static REQUEST: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub struct State {
    pub active: bool,
    pub plan: Option<impact::Plan>,
    pub error: String,
    pub scroll: u16,
    pub max_scroll: u16,
    pub page: u16,
    pending: Option<Pending>,
    basis: Option<crate::providers::ComparisonScope>,
}
struct Pending {
    request: u64,
    task: i64,
    partner: i64,
    basis: crate::providers::ComparisonScope,
    report: Arc<repo::Report>,
    cancelled: Arc<AtomicBool>,
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl State {
    pub fn cancel(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancelled.store(true, Ordering::Relaxed);
        }
        self.active = false;
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
}

impl App {
    pub(super) fn impact_command(&mut self, name: &str) -> bool {
        if !matches!(name, "impact-checks" | "stage-impact-plan") {
            return false;
        }
        if self.width < 60
            || self.height < 18
            || self.workflow.editor.is_some()
            || self.workflow.preview
            || self.workflow.chooser.is_some()
            || self.workflow.relay.is_some()
            || self.workflow.authoring.active
        {
            self.notice =
                "Finish the current editor/review and resize to60×18; nothing staged or executed."
                    .into();
            return true;
        }
        let result = if name == "impact-checks" {
            self.impact_preview()
        } else {
            self.impact_stage()
        };
        if let Err(error) = result {
            self.workflow.impact.error = error.clone();
            self.workflow.notice = error;
        }
        true
    }

    fn impact_basis(&self) -> crate::providers::ComparisonScope {
        crate::providers::ComparisonScope::from_identity(&(
            &self.workflows.recipe,
            &self.records.snapshot.signatures,
            &self.execution.history,
        ))
        .expect("bounded impact inputs serialize")
    }

    fn impact_preview(&mut self) -> Result<(), String> {
        if self.task_id <= 0
            || self.sources.busy()
            || self.planning_any_busy()
            || self.execution_any_busy()
            || self.repo_loading
            || self.repo_refresh
        {
            return Err(
                "Save a task and finish active source/execution work before impact analysis."
                    .into(),
            );
        }
        let report = self
            .repository
            .clone()
            .ok_or("Refresh the repository observation first")?;
        if self.execution.task_id != self.task_id {
            self.execution.open(self.task_id);
        }
        if self.execution.history_pending(self.task_id)
            || !self.records.loaded
            || self.records.restoring
            || self.records.failed.is_some()
            || self.records.inflight.is_some()
            || self.records.pending.is_some()
        {
            return Err("Wait for retained run/Dex evidence to load; retry failed storage before freezing check recommendations.".into());
        }
        self.open_overlay(Overlay::Workflows);
        self.workflow.impact.cancel();
        let basis = self.impact_basis();
        let state = &mut self.workflow.impact;
        let request = REQUEST.fetch_add(1, Ordering::Relaxed);
        let cancelled = Arc::new(AtomicBool::new(false));
        state.active = true;
        state.plan = None;
        state.basis = None;
        state.error.clear();
        state.scroll = 0;
        state.pending = Some(Pending {
            request,
            task: self.task_id,
            partner: self.lead.id,
            basis,
            report: Arc::clone(&report),
            cancelled: Arc::clone(&cancelled),
        });
        self.sources.pending = Some(context::Request {
            task_id: self.task_id,
            operation: context::Operation::Impact {
                report,
                request,
                partner: self.lead.id,
                runs: self.execution.history.clone(),
                failures: self.records.snapshot.signatures.clone(),
                validations: self
                    .workflows
                    .recipe
                    .as_ref()
                    .map_or_else(Vec::new, |recipe| recipe.recipe.validation_commands.clone()),
                cancelled,
            },
        });
        self.sources.impact_request = Some(request);
        self.sources.source_busy = true;
        self.workflow.notice =
            "Offline dependency/test/Dex analysis; no command is prepared or run.".into();
        Ok(())
    }

    pub(crate) fn receive_impact(&mut self, event: context::Event) -> Option<context::Event> {
        let Ok(context::Response::Impact { request, .. }) = &event.result else {
            return Some(event);
        };
        if self.sources.impact_request == Some(*request) {
            self.sources.impact_request = None;
            self.sources.source_busy = false;
        }
        if self
            .workflow
            .impact
            .pending
            .as_ref()
            .is_none_or(|pending| pending.request != *request || pending.task != event.task_id)
        {
            return None;
        }
        let pending = self
            .workflow
            .impact
            .pending
            .take()
            .expect("matched impact request");
        let current = event.task_id == self.task_id
            && pending.task == self.task_id
            && pending.partner == self.lead.id
            && pending.basis == self.impact_basis()
            && self.overlay == Some(Overlay::Workflows)
            && self.workflow.impact.active
            && !pending.cancelled.load(Ordering::Relaxed)
            && !self.repo_loading
            && !self.repo_refresh
            && self
                .repository
                .as_ref()
                .is_some_and(|report| Arc::ptr_eq(report, &pending.report));
        if !current {
            self.workflow.impact.plan = None;
            self.workflow.impact.error =
                "Impact owner/source changed or inspection cancelled; refresh explicitly.".into();
            return None;
        }
        if let Ok(context::Response::Impact { result, .. }) = event.result {
            match result {
                Ok(plan)
                    if plan.request == pending.request
                        && plan.task_id == self.task_id
                        && plan.partner == self.lead.id =>
                {
                    self.workflow.impact.basis = Some(pending.basis);
                    self.workflow.impact.plan = Some(plan);
                    self.workflow.notice = "Recommendation only; stage separately, then approve each check through Jolteon.".into();
                }
                Err(error) => self.workflow.impact.error = error,
                _ => {
                    self.workflow.impact.error =
                        "Impact response identity mismatch; nothing staged.".into()
                }
            }
        }
        None
    }

    fn impact_stage(&mut self) -> Result<(), String> {
        use crate::workspace::tool_plans::{NodeSpec, Operation};
        if self.workflow.impact.pending()
            || !self.workflow.impact.active
            || self.repo_loading
            || self.repo_refresh
            || self.planning_any_busy()
            || self.execution_any_busy()
        {
            return Err("Wait for a current idle impact preview before staging.".into());
        }
        let plan = self
            .workflow
            .impact
            .plan
            .as_ref()
            .ok_or("No impact recommendation to stage")?;
        let report = self
            .repository
            .as_ref()
            .ok_or("Repository observation unavailable")?;
        if plan.task_id != self.task_id
            || plan.partner != self.lead.id
            || plan.project != report.repository.root
            || plan.before != repo::fingerprint(report)
            || self.workflow.impact.basis != Some(self.impact_basis())
        {
            return Err("Impact task/partner/source changed; refresh the proposal first.".into());
        }
        if plan.checks.is_empty() {
            return Err("No trustworthy check mapping; choose an explicit Jolteon command.".into());
        }
        let nodes = plan
            .checks
            .iter()
            .enumerate()
            .map(|(index, check)| NodeSpec {
                id: format!("check-{}", index + 1),
                dependencies: Vec::new(),
                operation: Operation::Command {
                    spec: check.command.clone(),
                },
                reason: format!(
                    "TARGETED recommendation, not complete verification: {}",
                    check.reasons.join("; ")
                ),
            })
            .collect();
        self.stage_tool_plan(
            "Impact-directed checks (targeted, not complete verification)".into(),
            nodes,
        )?;
        self.workflow.impact.cancel();
        Ok(())
    }

    pub(super) fn impact_key(&mut self, key: KeyEvent) -> bool {
        if !self.workflow.impact.active {
            return false;
        }
        if key.kind == KeyEventKind::Release {
            return true;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.workflow.impact.cancel();
            self.request_quit();
            return true;
        }
        match key.code {
            KeyCode::Esc => {
                self.workflow.impact.cancel();
            }
            KeyCode::PageDown => {
                self.workflow.impact.scroll = self
                    .workflow
                    .impact
                    .scroll
                    .saturating_add(self.workflow.impact.page.max(1))
                    .min(self.workflow.impact.max_scroll)
            }
            KeyCode::PageUp => {
                self.workflow.impact.scroll = self
                    .workflow
                    .impact
                    .scroll
                    .saturating_sub(self.workflow.impact.page.max(1))
            }
            KeyCode::Char('s') if self.width >= 60 && self.height >= 18 => {
                self.impact_command("stage-impact-plan");
            }
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discarded_impact_owner_settles_without_releasing_a_newer_source_operation() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.sources.source_busy = true;
        app.sources.impact_request = Some(17);
        let response = || context::Event {
            task_id: 7,
            result: Ok(context::Response::Impact {
                request: 17,
                result: Err("cancelled".into()),
            }),
        };
        // A released partner replaced the workflow state; the source queue
        // still owns this original in-flight request and must become usable.
        assert!(app.receive_impact(response()).is_none());
        assert!(!app.sources.source_busy);
        assert!(!app.workflow.impact.active);
        assert!(app.overlay.is_none());
        // A later source request has different ownership. A duplicate/late
        // response cannot clear its busy state or reopen the old preview.
        app.sources.source_busy = true;
        assert!(app.receive_impact(response()).is_none());
        assert!(app.sources.source_busy);
        assert!(!app.workflow.impact.active);
    }
}
