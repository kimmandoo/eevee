//! Owner-isolated use of the existing local command state and runner.
use super::*;
use crate::workspace::runner;
use std::{collections::BTreeMap, path::Path};

impl App {
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
                self.execution_state_mut(id).runner_event(event);
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
