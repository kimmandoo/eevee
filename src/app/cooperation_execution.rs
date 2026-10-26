//! Owner-isolated use of the existing local command state and runner.
use super::*;
use crate::workspace::runner;
use std::{collections::BTreeMap, path::Path};

pub(super) struct CommandCallback {
    generation: u64,
    claim: store::WorkClaim,
    reply: std::sync::mpsc::SyncSender<Result<serde_json::Value, String>>,
    run: Option<String>,
    stdout: String,
    stderr: String,
    truncated: bool,
    artifact:
        Option<std::sync::mpsc::Receiver<Result<crate::workspace::artifacts::Response, String>>>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl App {
    fn command_callback_owned(
        &self,
        partner: i64,
        generation: u64,
        claim: &store::WorkClaim,
    ) -> bool {
        claim.writing
            && claim.task_id == self.task_id
            && claim.owner.partner == partner
            && self.planning_state(partner).is_some_and(|state| {
                state.matches(generation, claim.task_id)
                    && !state.cancelling
                    && state.work_acquired
                    && state.work.as_ref() == Some(claim)
                    && state.operation == crate::providers::Operation::Implement
            })
    }

    /// Denial may resolve this lead's exact callback, never borrow another writer.
    pub(crate) fn current_callback_command_deniable(&self) -> bool {
        if !matches!(
            self.execution.confirmation,
            Some(execution::Confirmation::Approval)
        ) {
            return false;
        }
        let Some((task, partner, _, _)) = self.execution.approval_identity() else {
            return false;
        };
        if task != self.task_id || partner != self.lead.id {
            return false;
        }
        self.cooperation
            .command_callbacks
            .get(&partner)
            .is_some_and(|callback| {
                self.command_callback_owned(partner, callback.generation, &callback.claim)
                    && !callback
                        .cancelled
                        .load(std::sync::atomic::Ordering::Acquire)
            })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn provider_command(
        &mut self,
        partner: i64,
        generation: u64,
        task: i64,
        data_dir: std::path::PathBuf,
        claim: store::WorkClaim,
        call_id: String,
        cwd: std::path::PathBuf,
        command: runner::CommandSpec,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
        reply: std::sync::mpsc::SyncSender<Result<serde_json::Value, String>>,
    ) -> bool {
        let identity = (claim.request_id.clone(), call_id);
        let ready = task == claim.task_id
            && !identity.1.is_empty()
            && identity.1.len() <= 512
            && !identity.1.chars().any(char::is_control)
            && self.command_callback_owned(partner, generation, &claim)
            && !cancelled.load(std::sync::atomic::Ordering::Acquire)
            && self.cooperation.cancel_review.is_none()
            && self.cooperation.journal_discard.is_none()
            && !self.cooperation.command_callbacks.contains_key(&partner)
            && !self.cooperation.command_calls.contains(&identity)
            && !self.tool_plans.busy()
            && !self.execution_any_unknown()
            && !self.planning_any_unknown_effects()
            && !self
                .execution_state(partner)
                .is_some_and(|state| state.busy() || state.storage_pending())
            && crate::providers::commands::validate_cwd(&cwd).is_ok();
        if !ready {
            let _ = reply.try_send(Err("Command refused before preparation: stale/cancelled/read-only ownership, duplicate call, unsafe cwd or owned work still pending".into()));
            return false;
        }
        if !self.execution_state_mut(partner).prepare_callback_for(
            claim.clone(),
            &command,
            cwd,
            data_dir,
            cancelled.clone(),
        ) {
            let _ = reply.try_send(Err("Command preparation refused; nothing executed".into()));
            return false;
        }
        self.cooperation.command_calls.insert(identity);
        self.cooperation.command_callbacks.insert(
            partner,
            CommandCallback {
                generation,
                claim,
                reply,
                run: None,
                stdout: String::new(),
                stderr: String::new(),
                truncated: false,
                artifact: None,
                cancelled,
            },
        );
        true
    }

    fn settle_command_callbacks(&mut self) {
        let ids: Vec<_> = self.cooperation.command_callbacks.keys().copied().collect();
        for id in ids {
            let callback = &self.cooperation.command_callbacks[&id];
            let authority = self.command_callback_owned(id, callback.generation, &callback.claim);
            let owned = authority
                && !callback
                    .cancelled
                    .load(std::sync::atomic::Ordering::Acquire);
            if !owned {
                self.execution_state_mut(id).request_cancel();
            }
            let Some(state) = self.execution_state(id) else {
                continue;
            };
            if state.busy() || state.storage_pending() {
                continue;
            }
            let mut artifact_result = None;
            if authority && self.cooperation.command_callbacks[&id].run.is_some() {
                let callback = self.cooperation.command_callbacks.get_mut(&id).unwrap();
                if let Some(receiver) = &callback.artifact {
                    match receiver.try_recv() {
                        Ok(result) => artifact_result = Some(result),
                        Err(std::sync::mpsc::TryRecvError::Empty) => continue,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            artifact_result =
                                Some(Err("Artifact acknowledgment unavailable".into()))
                        }
                    }
                } else {
                    let (reply, receiver) = std::sync::mpsc::sync_channel(1);
                    self.artifacts.enqueue_lookup(
                        callback.claim.task_id,
                        crate::workspace::artifacts::Operation::Load,
                        reply,
                    );
                    callback.artifact = Some(receiver);
                    continue;
                }
            }
            let callback = self.cooperation.command_callbacks.remove(&id).unwrap();
            let result = if !authority {
                Err("Owned command cancelled or authority changed; no model continuation may execute".into())
            } else if let Some(record) = callback.run.as_ref().and_then(|run| {
                self.execution_state(id)?
                    .history
                    .iter()
                    .find(|record| record.prepared.run_id == *run)
            }) {
                let mut value = crate::providers::commands::result(
                    record,
                    &callback.stdout,
                    &callback.stderr,
                    callback.truncated || record.dropped_live_bytes > 0,
                );
                if let Some(Ok(crate::workspace::artifacts::Response::Records(records))) =
                    artifact_result
                    && let Some(artifact) = records.iter().find(|artifact| matches!(&artifact.origin, crate::workspace::artifacts::Origin::OwnedRun { run_id } if run_id == &record.prepared.run_id))
                {
                    value["artifact"] = serde_json::json!(artifact.handle);
                    value["artifact_captured_bytes"] = serde_json::json!(artifact.captured_bytes);
                    value["artifact_omitted_bytes"] = serde_json::json!(artifact.omitted_bytes);
                }
                Ok(value)
            } else {
                Err("Command denied, cancelled before start or preparation unavailable; nothing was successfully executed. A new call requires new approval.".into())
            };
            let _ = callback.reply.try_send(result);
        }
        let planning = &self.planning;
        let parked = &self.cooperation.parked;
        self.cooperation.command_calls.retain(|(request, _)| {
            std::iter::once(planning)
                .chain(parked.values())
                .any(|state| {
                    state.work_acquired
                        && state
                            .work
                            .as_ref()
                            .is_some_and(|claim| &claim.request_id == request)
                })
        });
    }

    pub(super) fn execution_state(&self, partner: i64) -> Option<&execution::State> {
        if partner == self.lead.id {
            Some(&self.execution)
        } else {
            self.cooperation.executions.get(&partner)
        }
    }

    pub(super) fn execution_state_mut(&mut self, partner: i64) -> &mut execution::State {
        if partner == self.lead.id {
            return &mut self.execution;
        }
        let template = &self.execution;
        let task = self.task_id;
        self.cooperation
            .executions
            .entry(partner)
            .or_insert_with(|| template.new_partner_state(task))
    }

    pub(super) fn execution_ids(&self) -> Vec<i64> {
        std::iter::once(self.lead.id)
            .chain(self.cooperation.executions.keys().copied())
            .collect()
    }

    pub fn execution_any_busy(&self) -> bool {
        std::iter::once(&self.execution)
            .chain(self.cooperation.executions.values())
            .any(execution::State::busy)
    }

    pub fn execution_any_pending(&self) -> bool {
        if self.tool_plans.busy() {
            return true;
        }
        !self.cooperation.command_saves.is_empty()
            || std::iter::once(&self.execution)
                .chain(self.cooperation.executions.values())
                .any(|state| state.busy() || state.storage_pending())
    }

    pub fn execution_any_unknown(&self) -> bool {
        std::iter::once(&self.execution)
            .chain(self.cooperation.executions.values())
            .any(execution::State::outcome_unknown)
    }

    pub fn execution_any_journal_failed(&self) -> bool {
        std::iter::once(&self.execution)
            .chain(self.cooperation.executions.values())
            .any(execution::State::journal_failed)
    }

    pub fn drive_executions(
        &mut self,
        workers: &mut BTreeMap<i64, runner::Worker>,
        project: &Path,
    ) -> bool {
        let mut changed = false;
        self.settle_command_callbacks();
        self.track_pending_command();
        if self.cooperation.cancel_review.is_none()
            && self.cooperation.journal_discard.is_none()
            && !self.cooperation.command_saves.is_empty()
            && let Some(task) = self
                .tasks
                .iter()
                .find(|task| {
                    task.id == self.task_id
                        && self
                            .cooperation
                            .command_saves
                            .values()
                            .any(|(_, _, revision)| *revision <= task.revision)
                })
                .cloned()
        {
            self.commit_pending_commands(&task);
        }
        if !self.planning_any_busy()
            && !self.execution_any_pending()
            && let Some(action) = self.cooperation.execution_actions.remove(&self.lead.id)
        {
            self.apply_execution_action(action);
            changed = true;
        }
        let task = self.task_id;
        for id in self.execution_ids() {
            let state = self.execution_state_mut(id);
            state.set_task(task);
            let activity = state.activity_label();
            if state.needs_runner() && !workers.contains_key(&id) {
                match runner::Worker::start() {
                    Ok(worker) => {
                        workers.insert(id, worker);
                    }
                    Err(error) => {
                        state.runner_unavailable(error.to_string());
                        changed = true;
                    }
                }
            }
            let action = if let Some(worker) = workers.get(&id) {
                state.drive(worker, task, project)
            } else {
                state.take_ready_action()
            };
            changed |= activity != state.activity_label();
            if let Some(action) = action {
                if id == self.lead.id {
                    self.apply_execution_action(action);
                } else {
                    self.cooperation.execution_actions.insert(id, action);
                }
                changed = true;
            }
        }
        workers.retain(|id, worker| {
            worker.is_active()
                || self
                    .execution_state(*id)
                    .is_some_and(execution::State::needs_runner)
        });
        changed
    }

    pub fn submit_execution_stores(&mut self, store: &store::Worker) -> usize {
        let mut ids = self.execution_ids();
        let len = ids.len();
        ids.rotate_left(self.cooperation.execution_cursor % len);
        self.cooperation.execution_cursor = (self.cooperation.execution_cursor + 1) % len;
        let mut count = 0;
        for id in ids {
            let state = self.execution_state_mut(id);
            if let Some(request) = state.take_store_request() {
                let mut pending = Some(request);
                if store.try_run(&mut pending) {
                    self.cooperation.run_owners.push_back(id);
                    count += 1;
                    break;
                } else if let Some(request) = pending {
                    self.execution_state_mut(id).requeue_store_request(request);
                }
            }
        }
        count
    }

    pub fn poll_executions(&mut self, workers: &BTreeMap<i64, runner::Worker>) -> (bool, bool) {
        let mut changed = false;
        let mut more = false;
        for (&id, worker) in workers {
            for index in 0..16 {
                let Some(event) = worker.try_event() else {
                    break;
                };
                more |= index == 15;
                if let Some(callback) = self.cooperation.command_callbacks.get_mut(&id) {
                    match &event {
                        runner::Event::Prepared(prepared) => {
                            callback.run = Some(prepared.run_id.clone())
                        }
                        runner::Event::Output(output)
                            if callback.run.as_deref() == Some(&output.run_id) =>
                        {
                            let target = match output.stream {
                                runner::OutputStream::Stdout => &mut callback.stdout,
                                runner::OutputStream::Stderr => &mut callback.stderr,
                            };
                            target.push_str(&output.text);
                            if target.len() > 32 * 1024 {
                                let mut cut = target.len() - 32 * 1024;
                                while !target.is_char_boundary(cut) {
                                    cut += 1;
                                }
                                target.drain(..cut);
                                callback.truncated = true;
                            }
                        }
                        _ => {}
                    }
                }
                self.execution_state_mut(id).runner_event(event);
                self.move_run_prepared(id);
                changed = true;
            }
        }
        if changed {
            self.refresh_approvals();
        }
        self.settle_cooperation();
        (changed, more)
    }

    pub(super) fn cooperation_run_event(
        &mut self,
        task: i64,
        request: Option<&str>,
        run: Option<&str>,
        result: Result<store::RunResponse, String>,
    ) {
        let id = self
            .cooperation
            .run_owners
            .pop_front()
            .unwrap_or(self.lead.id);
        let move_updated = matches!(&result, Ok(store::RunResponse::Updated(_)));
        let observations = match &result {
            Ok(store::RunResponse::Updated(record))
            | Ok(store::RunResponse::Purged { record, .. }) => vec![record.clone()],
            Ok(store::RunResponse::Runs(records)) => records.clone(),
            _ => Vec::new(),
        };
        self.execution_state_mut(id)
            .store_event(task, request, run, result);
        for state in
            std::iter::once(&mut self.execution).chain(self.cooperation.executions.values_mut())
        {
            for record in observations.iter().rev() {
                state.observe_record(record);
            }
        }
        if !observations.is_empty() {
            self.records_invalidate();
        }
        if move_updated {
            for record in &observations {
                self.move_run_committed(record);
                if record.started_at_ms.is_some()
                    && record.ended_at_ms.is_some()
                    && !matches!(
                        record.state,
                        crate::workspace::runner::RunState::AwaitingDispatch
                            | crate::workspace::runner::RunState::Running
                            | crate::workspace::runner::RunState::Cancelling
                    )
                {
                    self.queue_record_operation(
                        record.prepared.task_id,
                        store::records::Operation::ObserveRun {
                            run_id: record.prepared.run_id.clone(),
                        },
                    );
                }
            }
        }
        let ids: Vec<_> = self.party.iter().map(|partner| partner.id).collect();
        self.cooperation.executions.retain(|partner, state| {
            ids.contains(partner)
                || state.busy()
                || state.storage_pending()
                || state.outcome_unknown()
        });
        self.settle_cooperation();
    }

    pub fn execution_repo_requested(&self) -> bool {
        std::iter::once(&self.execution)
            .chain(self.cooperation.executions.values())
            .any(execution::State::repo_requested)
    }

    pub fn execution_repo_started(&mut self) {
        for state in
            std::iter::once(&mut self.execution).chain(self.cooperation.executions.values_mut())
        {
            if state.repo_requested() {
                state.repo_started();
            }
        }
    }

    pub fn execution_repo_observed(&mut self, report: Option<&repo::Report>) {
        for state in
            std::iter::once(&mut self.execution).chain(self.cooperation.executions.values_mut())
        {
            state.repo_observed(report);
        }
    }

    pub(super) fn mark_commands_moving(&mut self, task: i64) {
        for state in
            std::iter::once(&mut self.execution).chain(self.cooperation.executions.values_mut())
        {
            state.mark_revision_moving(task);
        }
    }
}

impl App {
    pub(super) fn track_pending_command(&mut self) {
        let Some((command, owner)) = self.pending_command.take() else {
            return;
        };
        let draft = store::Draft {
            field: store::EditorField::Command,
            cursor: command.len(),
            text: command.clone(),
        };
        if owner.partner == self.lead.id {
            self.drafts
                .retain(|draft| draft.field != store::EditorField::Command);
            self.drafts.push(draft);
        } else {
            let drafts = self.partner_drafts.entry(owner.partner).or_default();
            drafts
                .drafts
                .retain(|draft| draft.field != store::EditorField::Command);
            drafts.drafts.push(draft);
        }
        self.cooperation
            .command_saves
            .insert(owner.partner, (command, owner, self.revision));
    }

    pub(super) fn commit_pending_commands(&mut self, snapshot: &store::Snapshot) {
        if snapshot.id != self.task_id {
            return;
        }
        if self.cooperation.cancel_review.is_some() || self.cooperation.journal_discard.is_some() {
            return;
        }
        self.track_pending_command();
        let ready: Vec<_> = self
            .cooperation
            .command_saves
            .iter()
            .filter_map(|(partner, (text, _, revision))| {
                let stored = snapshot.partner_drafts.get(partner).is_some_and(|drafts| {
                    drafts.drafts.iter().any(|draft| {
                        draft.field == store::EditorField::Command && &draft.text == text
                    })
                }) || snapshot.lead_partner == *partner && &snapshot.command == text;
                (snapshot.revision >= *revision && stored).then_some(*partner)
            })
            .collect();
        for partner in ready {
            if let Some((command, owner, _)) = self.cooperation.command_saves.remove(&partner) {
                self.prepare_command_relay(&command, owner);
            }
        }
    }
}
