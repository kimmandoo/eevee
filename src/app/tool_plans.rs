//! Visible reviewed dependency plans reuse the ordinary command consent and writer.
use super::*;
use crate::workspace::{
    runner,
    tool_plans::{self as plan, Certainty, NodeSpec, Operation, Plan, Status, worker},
};
use std::{cell::Cell, path::PathBuf, time::Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Review {
    Start,
    Resume,
    Serial,
}
#[derive(Default)]
pub struct State {
    pub editor: Option<String>,
    pub review: Option<Review>,
    pub scroll: u16,
    pub reviewed: Cell<bool>,
    pub extent: Cell<usize>,
    pub notice: String,
    review_binding: String,
    pub active: bool,
    worker: Option<worker::Worker>,
    worker_ready: bool,
    read_batch: Vec<usize>,
    read_dispatched: bool,
    command: Option<usize>,
    command_preparing: bool,
    started: Option<Instant>,
    serial: bool,
    calls_before: u32,
    pending_review: Option<Review>,
    pub history_index: Option<usize>,
}
impl State {
    pub fn invalidate(&mut self) {
        self.review = None;
        self.pending_review = None;
        self.reviewed.set(false);
        self.extent.set(0);
        self.review_binding.clear();
    }
    pub fn busy(&self) -> bool {
        self.active || !self.read_batch.is_empty() || self.command.is_some()
    }
    pub fn observe_review(&self, offset: usize, total: usize, rows: usize) -> bool {
        if offset <= self.extent.get() {
            self.extent.set(
                self.extent
                    .get()
                    .max(offset.saturating_add(rows).min(total)),
            );
        }
        self.extent.get() >= total
    }
}
impl App {
    /// D01 and local authoring stage the same inert typed graph. This grants nothing.
    pub fn stage_tool_plan(&mut self, title: String, nodes: Vec<NodeSpec>) -> Result<(), String> {
        if !self.authentication_ready() || self.complete || self.archived || self.task_id <= 0 {
            return Err(
                "Save an idle open task before staging a tool plan; nothing started".into(),
            );
        }
        let source = self
            .repository
            .as_deref()
            .map(repo::fingerprint)
            .unwrap_or_default();
        let candidate = Plan::new(
            title,
            nodes,
            self.task_id,
            self.lead.id,
            self.cwd.clone(),
            source,
        )?;
        if serde_json::to_vec(&candidate)
            .map_err(|error| error.to_string())?
            .len()
            > 64 * 1024
        {
            return Err("Plan arguments and reasons exceed the 64 KiB initial record bound".into());
        }
        let mut workflows = self.workflows.clone();
        if let Some(previous) = &workflows.tool_plans
            && previous.attempts > 0
        {
            if previous.nodes.iter().any(|node| {
                matches!(node.status, Status::Running | Status::Unknown)
                    || node.certainty == Certainty::Unknown
            }) || !previous.cancelled
                && previous
                    .nodes
                    .iter()
                    .any(|node| node.status != Status::Succeeded)
            {
                return Err("Resume the previous failed/unstarted plan, or explicitly cancel its queue before staging another; unresolved mutation outcomes cannot be archived".into());
            }
            if workflows.tool_plan_history.len() >= 8 {
                return Err("Retained plan history is full (8); no record was evicted".into());
            }
            workflows.tool_plan_history.push(previous.clone());
        }
        // Reserve the complete bounded result/correlation envelope in the same
        // existing workflow quota before any node can be admitted.
        let mut reservation = candidate.clone();
        for node in &mut reservation.nodes {
            node.result = "\"".repeat(plan::MAX_RESULT_BYTES);
            node.result_sha256 = Some("0".repeat(64));
            node.observation_sha256 = Some("0".repeat(64));
            node.run_id = Some("r".repeat(256));
            node.started_at_ms = Some(u64::MAX);
            node.ended_at_ms = Some(u64::MAX);
            node.elapsed_us = u64::MAX;
            node.query_workers = u32::MAX;
        }
        reservation.wall_us = u64::MAX;
        reservation.execution_count = u32::MAX;
        reservation.attempts = u32::MAX;
        if reservation
            .nodes
            .iter()
            .all(|node| node.spec.operation.readonly())
        {
            reservation.comparison = Some(plan::Comparison {
                parallel_wall_us: u64::MAX,
                parallel_calls: u32::MAX,
                serial_wall_us: Some(u64::MAX),
                serial_calls: Some(u32::MAX),
                equal_results: Some(false),
                original_results: reservation
                    .nodes
                    .iter()
                    .map(|_| Some("0".repeat(64)))
                    .collect(),
            });
        }
        let mut reserved = workflows.clone();
        reserved.tool_plans = Some(reservation);
        reserved.validate(self.task_id)?;
        workflows.tool_plans = Some(candidate);
        workflows.validate(self.task_id)?;
        self.workflows = workflows;
        self.tool_plans = State::default();
        self.tool_plans.notice = "Typed plan staged locally. Read-only calls may overlap (at most 3); every command still requires its own exact review. A batch is not consent.".into();
        self.changed_task();
        self.flush_save();
        if self.editing {
            self.retain_action_editor();
        }
        self.open_overlay(Overlay::ToolPlans);
        Ok(())
    }
    pub(super) fn tool_plan_command(&mut self, input: &str) -> bool {
        let (name, argument) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
        if !matches!(
            name,
            "tool-plan"
                | "plan-history"
                | "plan-start"
                | "plan-resume"
                | "plan-serial"
                | "plan-cancel"
                | "plan-reconcile"
        ) {
            return false;
        }
        if self.editing {
            self.retain_action_editor();
        }
        self.open_overlay(Overlay::ToolPlans);
        self.tool_plans.history_index = if name == "plan-history" {
            self.workflows.tool_plan_history.len().checked_sub(1)
        } else {
            None
        };
        match name {
            "tool-plan" if !argument.is_empty() => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Input {
                    title: String,
                    nodes: Vec<NodeSpec>,
                }
                let parsed = if argument.len() <= 64 * 1024 {
                    serde_json::from_str::<Input>(argument).map_err(|error| error.to_string())
                } else {
                    Err("Tool plan exceeds 64 KiB input bound".into())
                };
                match parsed.and_then(|input| self.stage_tool_plan(input.title, input.nodes)) {
                    Ok(()) => {}
                    Err(error) => self.tool_plans.notice = error,
                }
            }
            "plan-start" => self.preview_tool_plan(Review::Start),
            "plan-resume" => self.preview_tool_plan(Review::Resume),
            "plan-serial" => self.preview_tool_plan(Review::Serial),
            "plan-cancel" => self.cancel_tool_plan(),
            "plan-reconcile" => self.reconcile_tool_plan(),
            _ => {}
        }
        true
    }
    fn plan_committed(&self) -> bool {
        !matches!(
            self.storage,
            StorageState::Failed(_) | StorageState::Loading
        ) && self.tasks.iter().any(|task| {
            task.id == self.task_id
                && task.project == self.cwd
                && task.workflows.tool_plans == self.workflows.tool_plans
        })
    }
    fn plan_binding(&self) -> String {
        plan::digest(
            &serde_json::to_vec(&(
                self.task_id,
                self.lead.id,
                &self.cwd,
                &self.workflows.tool_plans,
            ))
            .unwrap_or_default(),
        )
    }
    fn preview_tool_plan(&mut self, mode: Review) {
        self.tool_plans.invalidate();
        self.tool_plans.scroll = 0;
        if self.tool_plans.busy()
            || self.execution_any_pending()
            || self.planning_any_busy()
            || self.execution_any_unknown()
            || self.planning_any_unknown_effects()
            || !self.plan_committed()
        {
            self.tool_plans.notice = if self.execution_any_unknown() || self.planning_any_unknown_effects() { "Unresolved/lost response: existing owned outcome certainty blocks all scheduling. Inspect /run-details and reconcile only definite durable evidence; no replay or exactly-once claim." } else { "Wait for the exact plan and all existing work to settle durably; nothing started." }.into();
            return;
        }
        let Some(p) = &self.workflows.tool_plans else {
            self.tool_plans.notice =
                "Stage a typed plan first (e edits the bounded JSON definition).".into();
            return;
        };
        if p.partner != self.lead.id || p.task != self.task_id || p.project != self.cwd {
            self.tool_plans.notice =
                "Plan belongs to another task/project/partner; no authority transferred.".into();
            return;
        }
        if mode == Review::Start
            && (p.attempts != 0 || p.nodes.iter().any(|node| node.status != Status::Pending))
        {
            self.tool_plans.notice = "This plan already has outcomes. Use the failed/unstarted resume preview, never replay Start.".into();
            return;
        }
        if mode == Review::Resume {
            match p.resume_candidates() {
                Ok(indices) if !indices.is_empty() => {}
                Ok(_) => {
                    self.tool_plans.notice = "No safely resumable failed/unstarted nodes. Confirmed commands are never repeated.".into();
                    return;
                }
                Err(error) => {
                    self.tool_plans.notice = error;
                    return;
                }
            }
        }
        if mode == Review::Resume
            && let Some(current) = self.repository.as_deref().map(repo::fingerprint)
            && current != p.source
        {
            self.workflows
                .tool_plans
                .as_mut()
                .expect("selected plan")
                .source = current;
            self.changed_task();
            self.flush_save();
            self.tool_plans.pending_review = Some(mode);
            self.tool_plans.notice = "Refreshing the resume input binding through the existing writer. Only a new complete review may schedule failed/unstarted nodes; confirmed commands are unchanged.".into();
            return;
        }
        if mode == Review::Serial
            && (p.attempts != 1
                || p.nodes.iter().any(|node| {
                    !node.spec.operation.readonly() || node.status != Status::Succeeded
                }))
        {
            self.tool_plans.notice = "An actual serial comparison requires one complete successful all-read parallel run. Commands are never repeated for a benchmark.".into();
            return;
        }
        if p.nodes.iter().any(|node| !node.spec.operation.readonly())
            && (!p.source.complete || !p.source.stable || p.source.fingerprint.is_none())
        {
            self.tool_plans.notice = "Commands require a complete stable current source fingerprint. Refresh repository evidence and stage a new plan; no command started.".into();
            return;
        }
        self.tool_plans.review_binding = self.plan_binding();
        self.tool_plans.review = Some(mode);
        self.tool_plans.notice = "Neutral preview: scroll through the entire exact graph, then y schedules it. Each command has separate existing one-shot approval; n/Escape starts nothing.".into();
    }
    fn start_tool_plan(&mut self) {
        let Some(mode) = self.tool_plans.review else {
            return;
        };
        if !self.tool_plans.reviewed.get()
            || self.width < 60
            || self.height < 18
            || !self.plan_committed()
            || self.tool_plans.review_binding != self.plan_binding()
            || self.execution_any_pending()
            || self.planning_any_busy()
            || self.execution_any_unknown()
            || self.planning_any_unknown_effects()
        {
            self.tool_plans.invalidate();
            self.tool_plans.notice =
                "Stale, incomplete or unavailable plan review; no node started.".into();
            return;
        }
        let Some(p) = &mut self.workflows.tool_plans else {
            return;
        };
        if mode == Review::Resume {
            let Ok(indices) = p.resume_candidates() else {
                self.tool_plans.invalidate();
                return;
            };
            for index in indices {
                let node = &mut p.nodes[index];
                node.status = Status::Pending;
                node.certainty = Certainty::NotStarted;
                node.run_id = None;
                node.started_at_ms = None;
                node.ended_at_ms = None;
            }
        }
        if mode == Review::Serial {
            p.comparison = Some(plan::Comparison {
                parallel_wall_us: p.wall_us,
                parallel_calls: p.execution_count,
                serial_wall_us: None,
                serial_calls: None,
                equal_results: None,
                original_results: p
                    .nodes
                    .iter()
                    .map(|node| node.observation_sha256.clone())
                    .collect(),
            });
            for node in &mut p.nodes {
                node.status = Status::Pending;
                node.certainty = Certainty::NotStarted;
            }
        }
        self.tool_plans.serial = mode == Review::Serial;
        self.tool_plans.calls_before = p.execution_count;
        p.cancelled = false;
        p.attempts = p.attempts.saturating_add(1);
        p.started_at_ms = Some(plan::now_ms());
        p.ended_at_ms = None;
        p.block_descendants();
        self.tool_plans.worker = None;
        self.tool_plans.invalidate();
        self.tool_plans.active = true;
        self.tool_plans.started = Some(Instant::now());
        self.tool_plans.worker_ready = false;
        self.tool_plans.notice = "Waiting for durable start intent, then fresh worker-side source validation. Commands are not approved.".into();
        self.changed_task();
        self.flush_save();
    }
    pub(super) fn cancel_tool_plan(&mut self) {
        self.tool_plans.invalidate();
        if let Some(worker) = &self.tool_plans.worker {
            worker.cancel();
        }
        if !self.tool_plans.command_preparing
            && let Some(index) = self.tool_plans.command.take()
            && let Some(p) = &mut self.workflows.tool_plans
        {
            p.nodes[index].status = Status::Cancelled;
            p.nodes[index].certainty = Certainty::NotStarted;
        }
        if !self.tool_plans.read_dispatched {
            if let Some(p) = &mut self.workflows.tool_plans {
                for &index in &self.tool_plans.read_batch {
                    p.nodes[index].status = Status::Cancelled;
                    p.nodes[index].certainty = Certainty::NotStarted;
                }
            }
            self.tool_plans.read_batch.clear();
        }
        let partner = self.workflows.tool_plans.as_ref().map(|p| p.partner);
        if let Some(p) = &mut self.workflows.tool_plans {
            p.cancel();
            self.changed_task();
            self.flush_save();
        }
        if self.tool_plans.command.is_some()
            && let Some(partner) = partner
        {
            self.execution_state_mut(partner).request_cancel();
        }
        self.tool_plans.active = false;
        self.tool_plans.notice = "Plan cancelled: no queued node starts; admitted observations settle and existing command cancellation/journal retain outcome certainty.".into();
    }
    pub(super) fn restore_tool_plan(&mut self) {
        self.tool_plans = State::default();
        let interrupted = self
            .workflows
            .tool_plans
            .as_ref()
            .is_some_and(|p| p.nodes.iter().any(|node| node.status == Status::Running));
        if let Some(p) = &mut self.workflows.tool_plans {
            p.restore();
        }
        if interrupted {
            self.changed_task();
            self.flush_save();
        }
    }
    fn reconcile_tool_plan(&mut self) {
        if self.tool_plans.busy() || self.execution_any_pending() {
            self.tool_plans.notice =
                "Wait for owned work and the existing run journal to settle before reconciliation."
                    .into();
            return;
        }
        let Some(p) = &mut self.workflows.tool_plans else {
            return;
        };
        let mut changed = false;
        for node in &mut p.nodes {
            if node.status != Status::Unknown {
                continue;
            }
            let record = self.execution.history.iter().find(|record| node.run_id.as_ref() == Some(&record.prepared.run_id) && record.prepared.task_id == p.task && record.prepared.partner.as_ref().map(|owner| owner.partner) == Some(p.partner) && matches!(&node.spec.operation, Operation::Command { spec } if *spec == record.prepared.spec));
            if let Some(record) = record.filter(|record| {
                terminal(record) && record.tree != runner::TreeOutcome::OwnershipUnknown
            }) {
                apply_record(node, record);
                changed = true;
                if record.started_at_ms.is_some() {
                    p.source = record.after.clone();
                }
            }
        }
        self.tool_plans.notice = if changed { "Reconciled only matching durable terminal command evidence. Confirmed effects will never be resumed." } else { "No matching definite durable result: lost mutation response remains blocking. Inspect /run-details; no replay or exactly-once claim." }.into();
        if changed {
            self.changed_task();
            self.flush_save();
        }
    }
    /// Main loop adapter: read-only work delegates to WriteSet; commands to execution::State.
    pub fn drive_tool_plans(&mut self, data_dir: &std::path::Path, exclusions: &[PathBuf]) -> bool {
        let mut changed = false;
        if self.tool_plans.pending_review.is_some()
            && self.plan_committed()
            && self.overlay == Some(Overlay::ToolPlans)
            && let Some(mode) = self.tool_plans.pending_review.take()
        {
            self.preview_tool_plan(mode);
            changed = true;
        }
        while let Some(event) = self
            .tool_plans
            .worker
            .as_ref()
            .and_then(worker::Worker::event)
        {
            match event {
                worker::Event::Ready(result) => match result {
                    Ok(_) => {
                        self.tool_plans.worker_ready = true;
                        changed = true;
                    }
                    Err(error) => {
                        self.tool_plans.notice = error;
                        self.cancel_tool_plan();
                        changed = true;
                    }
                },
                worker::Event::Observed(outcomes) => {
                    if let Some(p) = &mut self.workflows.tool_plans {
                        for outcome in outcomes {
                            if let Some(node) = p.nodes.get_mut(outcome.index) {
                                node.status = if !outcome.started {
                                    Status::Cancelled
                                } else if outcome.result.is_ok() {
                                    Status::Succeeded
                                } else {
                                    Status::Failed
                                };
                                node.certainty = if outcome.started {
                                    Certainty::NoEffect
                                } else {
                                    Certainty::NotStarted
                                };
                                node.started_at_ms =
                                    outcome.started.then_some(outcome.started_at_ms);
                                node.ended_at_ms = Some(outcome.ended_at_ms);
                                node.elapsed_us =
                                    node.elapsed_us.saturating_add(outcome.elapsed_us);
                                node.query_workers = node
                                    .query_workers
                                    .saturating_add(u32::from(outcome.query_worker_started));
                                match outcome.result {
                                    Ok((text, identity)) => {
                                        node.observation_sha256 = Some(identity);
                                        node.retain_result(&text);
                                    }
                                    Err(error) => {
                                        node.observation_sha256 = None;
                                        node.retain_result(&error);
                                    }
                                }
                                p.execution_count =
                                    p.execution_count.saturating_add(u32::from(outcome.started));
                            }
                        }
                        p.block_descendants();
                    }
                    self.tool_plans.read_batch.clear();
                    self.tool_plans.read_dispatched = false;
                    self.changed_task();
                    self.flush_save();
                    changed = true;
                }
            }
        }
        if let Some(index) = self.tool_plans.command {
            let partner = self
                .workflows
                .tool_plans
                .as_ref()
                .map_or(self.lead.id, |p| p.partner);
            let prepared = self
                .execution_state(partner)
                .and_then(|state| state.prepared.clone());
            let run = self
                .workflows
                .tool_plans
                .as_ref()
                .and_then(|p| p.nodes.get(index))
                .and_then(|node| node.run_id.clone());
            if self.execution_state_mut(partner).take_plan_cancellation() {
                if let Some(p) = &mut self.workflows.tool_plans {
                    p.cancel();
                }
                if let Some(worker) = &self.tool_plans.worker {
                    worker.cancel();
                }
                self.tool_plans.active = false;
                self.changed_task();
                self.flush_save();
                changed = true;
            }
            if run.is_none()
                && let Some(prepared) = prepared
            {
                let p = self.workflows.tool_plans.as_mut().expect("active plan");
                if prepared.before != p.source
                    || !matches!(&p.nodes[index].spec.operation, Operation::Command { spec } if *spec == prepared.spec)
                {
                    self.execution_state_mut(partner).request_cancel();
                    self.cancel_tool_plan();
                    self.tool_plans.notice =
                        "Plan source changed before exact command review; no command started."
                            .into();
                } else {
                    p.nodes[index].run_id = Some(prepared.run_id);
                    p.nodes[index].input = prepared.before;
                    self.changed_task();
                    self.flush_save();
                }
                changed = true;
            } else if let Some(run) = &run {
                if self.plan_committed() && self.tool_plans.active {
                    self.execution_state_mut(partner)
                        .release_plan_preparation(run);
                }
                let record = self
                    .execution_state(partner)
                    .and_then(|state| {
                        state
                            .history
                            .iter()
                            .find(|record| record.prepared.run_id == *run && terminal(record))
                    })
                    .cloned();
                if let Some(record) = record
                    && !self
                        .execution_state(partner)
                        .is_some_and(|state| state.busy() || state.storage_pending())
                {
                    let p = self.workflows.tool_plans.as_mut().expect("active plan");
                    apply_record(&mut p.nodes[index], &record);
                    p.execution_count = p
                        .execution_count
                        .saturating_add(u32::from(record.started_at_ms.is_some()));
                    if record.started_at_ms.is_some() {
                        p.source = record.after.clone();
                    }
                    if record.cancelled {
                        p.cancel();
                        self.tool_plans.active = false;
                    }
                    p.block_descendants();
                    self.tool_plans.command = None;
                    self.tool_plans.command_preparing = false;
                    self.changed_task();
                    self.flush_save();
                    changed = true;
                }
            }
            if self.tool_plans.command.is_some()
                && self.tool_plans.command_preparing
                && !self
                    .execution_state(partner)
                    .is_some_and(|state| state.busy() || state.storage_pending())
            {
                let p = self.workflows.tool_plans.as_mut().expect("active plan");
                let node = &mut p.nodes[index];
                node.status = Status::Failed;
                node.certainty = Certainty::NoEffect;
                node.ended_at_ms = Some(plan::now_ms());
                node.retain_result(
                    "Command denied/refused before dispatch; existing approval granted no effect",
                );
                p.block_descendants();
                self.tool_plans.command = None;
                self.tool_plans.command_preparing = false;
                self.changed_task();
                self.flush_save();
                changed = true;
            }
        }
        if !self.tool_plans.active {
            if !self.tool_plans.busy() {
                self.tool_plans.worker = None;
                if let Some(clock) = self.tool_plans.started.take()
                    && let Some(p) = &mut self.workflows.tool_plans
                {
                    p.ended_at_ms = Some(plan::now_ms());
                    p.wall_us = p
                        .wall_us
                        .saturating_add(clock.elapsed().as_micros().min(u64::MAX as u128) as u64);
                    self.changed_task();
                    self.flush_save();
                    changed = true;
                }
            }
            return changed;
        }
        if !self.plan_committed()
            || self.quit
            || self.quit_after_save
            || self.reset_after_save
            || self.transitioning
            || self.cooperation.cancel_review.is_some()
            || self.cancelling_owned_work()
        {
            return changed;
        }
        let Some(p) = self.workflows.tool_plans.as_ref() else {
            self.tool_plans.active = false;
            return true;
        };
        if p.task != self.task_id || p.project != self.cwd || p.partner != self.lead.id {
            self.cancel_tool_plan();
            return true;
        }
        if self.tool_plans.worker.is_none() {
            match worker::Worker::start(
                self.cwd.clone().into(),
                data_dir.to_path_buf(),
                exclusions.to_vec(),
                p.source.clone(),
            ) {
                Ok(worker) => {
                    self.tool_plans.worker = Some(worker);
                    return true;
                }
                Err(error) => {
                    self.tool_plans.notice = error.to_string();
                    self.cancel_tool_plan();
                    return true;
                }
            }
        }
        if !self.tool_plans.worker_ready {
            return changed;
        }
        if !self.tool_plans.read_batch.is_empty() {
            if !self.tool_plans.read_dispatched {
                let nodes = self
                    .tool_plans
                    .read_batch
                    .iter()
                    .map(|&index| (index, p.nodes[index].spec.clone()))
                    .collect();
                if self
                    .tool_plans
                    .worker
                    .as_ref()
                    .is_some_and(|worker| worker.observe(nodes))
                {
                    self.tool_plans.read_dispatched = true;
                    return true;
                }
            }
            return changed;
        }
        if let Some(index) = self.tool_plans.command {
            if !self.tool_plans.command_preparing {
                let p = self.workflows.tool_plans.as_ref().expect("active plan");
                let Operation::Command { spec } = p.nodes[index].spec.operation.clone() else {
                    return changed;
                };
                let owner = store::Attribution::of(&self.lead);
                let task = self.task_id;
                if self.execution_state_mut(owner.partner).prepare_plan_for(
                    task,
                    owner,
                    &spec,
                    data_dir.to_path_buf(),
                ) {
                    self.tool_plans.command_preparing = true;
                    self.refresh_approvals();
                    return true;
                }
                self.cancel_tool_plan();
                return true;
            }
            return changed;
        }
        let mut ready = p.ready();
        if self.tool_plans.serial {
            ready.truncate(1);
        }
        if ready.is_empty() {
            let running = p.nodes.iter().any(|node| node.status == Status::Running);
            if !running {
                let p = self.workflows.tool_plans.as_mut().expect("active plan");
                let elapsed = self.tool_plans.started.take().map_or(0, |clock| {
                    clock.elapsed().as_micros().min(u64::MAX as u128) as u64
                });
                p.ended_at_ms = Some(plan::now_ms());
                p.wall_us = p.wall_us.saturating_add(elapsed);
                if self.tool_plans.serial
                    && let Some(comparison) = &mut p.comparison
                {
                    comparison.serial_wall_us = Some(elapsed);
                    comparison.serial_calls = Some(
                        p.execution_count
                            .saturating_sub(self.tool_plans.calls_before),
                    );
                    comparison.equal_results = Some(
                        p.nodes
                            .iter()
                            .map(|node| &node.observation_sha256)
                            .eq(comparison.original_results.iter()),
                    );
                }
                self.tool_plans.active = false;
                self.tool_plans.worker = None;
                self.tool_plans.notice = if self.tool_plans.serial { "Actual serial comparison completed: counts, wall time and observation identities are retained beside the parallel run. Timing is not a correctness/completeness guarantee." } else { "Plan settled. Summed node work is not a serial measurement; b previews a separately executed all-read serial comparison. Commands are never repeated for comparison." }.into();
                self.changed_task();
                self.flush_save();
                return true;
            }
            return changed;
        }
        let command = !p.nodes[ready[0]].spec.operation.readonly();
        let p = self.workflows.tool_plans.as_mut().expect("active plan");
        for &index in &ready {
            p.nodes[index].status = Status::Running;
            p.nodes[index].input = p.source.clone();
        }
        if command {
            self.tool_plans.command = Some(ready[0]);
            self.tool_plans.command_preparing = false;
        } else {
            // Persist the exact admitted node identities before the existing
            // read executor starts, just as for a command preparation.
            self.tool_plans.read_batch = ready;
            self.tool_plans.read_dispatched = false;
        }
        self.changed_task();
        self.flush_save();
        true
    }
    pub(super) fn tool_plan_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press || !key.modifiers.is_empty() {
            return true;
        }
        if self.tool_plans.history_index.is_some() {
            match key.code {
                KeyCode::Char('h') => {
                    self.tool_plans.history_index = None;
                    self.tool_plans.scroll = 0;
                    return true;
                }
                KeyCode::Char('[') => {
                    self.tool_plans.history_index = self
                        .tool_plans
                        .history_index
                        .map(|index| index.saturating_sub(1));
                    self.tool_plans.scroll = 0;
                }
                KeyCode::Char(']') => {
                    self.tool_plans.history_index = self.tool_plans.history_index.map(|index| {
                        (index + 1).min(self.workflows.tool_plan_history.len().saturating_sub(1))
                    });
                    self.tool_plans.scroll = 0;
                }
                KeyCode::Char('e' | 's' | 'r' | 'b' | 'y' | 'c' | 'j') => {
                    self.tool_plans.notice = "History inspection has no execution or resume authority; h returns to the current plan.".into();
                    return true;
                }
                _ => {}
            }
        }
        if let Some(text) = &mut self.tool_plans.editor {
            match key.code {
                KeyCode::Esc => self.tool_plans.editor = None,
                KeyCode::Enter => {
                    let text = text.clone();
                    self.tool_plans.editor = None;
                    self.tool_plan_command(&format!("tool-plan {text}"));
                }
                KeyCode::Backspace => {
                    if let Some((index, _)) = text.grapheme_indices(true).next_back() {
                        text.truncate(index);
                    }
                }
                KeyCode::Char(c) if text.len() + c.len_utf8() <= 64 * 1024 => text.push(c),
                _ => {}
            }
            return true;
        }
        match key.code {
            KeyCode::Esc => {
                self.tool_plans.invalidate();
                self.close_overlay();
            }
            KeyCode::Char('e') if !self.tool_plans.busy() => {
                self.tool_plans.invalidate();
                self.tool_plans.editor = Some(String::new());
                self.tool_plans.scroll = 0;
            }
            KeyCode::Char('s') => self.preview_tool_plan(Review::Start),
            KeyCode::Char('r') => self.preview_tool_plan(Review::Resume),
            KeyCode::Char('b') => self.preview_tool_plan(Review::Serial),
            KeyCode::Char('h') => {
                self.tool_plans.invalidate();
                self.tool_plans.history_index =
                    self.workflows.tool_plan_history.len().checked_sub(1);
                self.tool_plans.scroll = 0;
            }
            KeyCode::Char('y') => self.start_tool_plan(),
            KeyCode::Char('n') => self.tool_plans.invalidate(),
            KeyCode::Char('c') => self.cancel_tool_plan(),
            KeyCode::Char('j') => self.reconcile_tool_plan(),
            KeyCode::Up => self.tool_plans.scroll = self.tool_plans.scroll.saturating_sub(1),
            KeyCode::Down => self.tool_plans.scroll = self.tool_plans.scroll.saturating_add(1),
            KeyCode::PageUp => self.tool_plans.scroll = self.tool_plans.scroll.saturating_sub(8),
            KeyCode::PageDown => self.tool_plans.scroll = self.tool_plans.scroll.saturating_add(8),
            KeyCode::Home => self.tool_plans.scroll = 0,
            _ => {}
        }
        true
    }
}
fn terminal(record: &runner::RunRecord) -> bool {
    !matches!(
        record.state,
        runner::RunState::AwaitingDispatch
            | runner::RunState::Running
            | runner::RunState::Cancelling
    )
}
fn apply_record(node: &mut plan::Node, record: &runner::RunRecord) {
    node.status = match record.state {
        runner::RunState::Succeeded => Status::Succeeded,
        // Abandoning a correlated undispatched reservation is durable refusal,
        // not a lost process response. Startup recovery marks ownership unknown.
        runner::RunState::Interrupted
            if record.started_at_ms.is_none() && record.tree == runner::TreeOutcome::NotStarted =>
        {
            Status::Failed
        }
        runner::RunState::OutcomeUnknown | runner::RunState::Interrupted => Status::Unknown,
        _ => Status::Failed,
    };
    node.certainty =
        if node.status == Status::Unknown || record.tree == runner::TreeOutcome::OwnershipUnknown {
            Certainty::Unknown
        } else if record.started_at_ms.is_some() {
            Certainty::Confirmed
        } else {
            Certainty::NoEffect
        };
    node.started_at_ms = record.started_at_ms;
    node.ended_at_ms = record.ended_at_ms;
    node.elapsed_us = node.elapsed_us.saturating_add(
        record
            .ended_at_ms
            .zip(record.started_at_ms)
            .map_or(0, |(end, start)| {
                end.saturating_sub(start).saturating_mul(1000)
            }),
    );
    node.retain_result(
        &serde_json::to_string(&runner::RunObservation::from(record))
            .unwrap_or_else(|_| "Durable run evidence serialization unavailable".into()),
    );
}

#[cfg(test)]
mod tests;
