//! Independent partner-owned request state, arrival-ordered consent and settlement.
use super::*;
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub struct WorkRow {
    pub owner: store::Attribution,
    pub status: &'static str,
    pub detail: String,
    pub local: bool,
}

pub struct CancelReview {
    pub quit: bool,
    pub reviewed: bool,
    pub affirmative: bool,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Pending {
    Provider {
        partner: i64,
        task: i64,
        generation: u64,
        request: Option<serde_json::Value>,
    },
    Command {
        partner: i64,
        task: i64,
        request: String,
        run: String,
    },
}

#[derive(Default)]
pub struct State {
    pub(super) parked: BTreeMap<i64, planning::State>,
    pub(super) executions: BTreeMap<i64, execution::State>,
    pub(super) run_owners: VecDeque<i64>,
    pub(super) execution_cursor: usize,
    pub(super) execution_actions: BTreeMap<i64, execution::Action>,
    pub(super) command_saves: BTreeMap<i64, (String, store::Attribution, u64)>,
    pub(super) command_callbacks: BTreeMap<i64, super::cooperation_execution::CommandCallback>,
    pub(super) command_calls: std::collections::HashSet<(String, String)>,
    approvals: Vec<(u64, Pending)>,
    focused: Option<Pending>,
    /// Identities already presented or explicitly deferred, not another queue.
    presented_permissions: Vec<Pending>,
    queued_response: Option<Pending>,
    preference_owners: VecDeque<i64>,
    pub(super) journal_owners: VecDeque<i64>,
    pub cancel_review: Option<CancelReview>,
    pub journal_discard: Option<super::cooperation_journal::Review>,
    cancelling: bool,
    quitting: bool,
    unknown_acknowledged: bool,
    drive_cursor: usize,
    quit_preferences_waiting: bool,
}

impl App {
    pub(super) fn has_focused_approval(&self) -> bool {
        self.cooperation.focused.is_some()
    }

    /// Replace only future launch credentials; submitted inputs own independent copies.
    pub fn set_devin_credential(
        &mut self,
        credential: Option<crate::providers::auth::SessionCredential>,
    ) {
        self.planning.set_devin_credential(credential.clone());
        for state in self.cooperation.parked.values_mut() {
            state.set_devin_credential(credential.clone());
        }
    }

    pub(super) fn planning_state(&self, partner: i64) -> Option<&planning::State> {
        if partner == self.lead.id {
            Some(&self.planning)
        } else {
            self.cooperation.parked.get(&partner).or_else(|| {
                self.delegation
                    .children
                    .get(&partner)
                    .map(|child| &child.state)
            })
        }
    }

    pub(super) fn planning_state_mut(&mut self, partner: i64) -> Option<&mut planning::State> {
        if partner == self.lead.id {
            Some(&mut self.planning)
        } else {
            self.cooperation.parked.get_mut(&partner).or_else(|| {
                self.delegation
                    .children
                    .get_mut(&partner)
                    .map(|child| &mut child.state)
            })
        }
    }

    fn planning_ids(&self) -> Vec<i64> {
        std::iter::once(self.lead.id)
            .chain(self.cooperation.parked.keys().copied())
            .collect()
    }

    pub(super) fn front_planning(&mut self, partner: &store::Partner) {
        self.clear_approval_focus();
        let incoming_execution = self
            .cooperation
            .executions
            .remove(&partner.id)
            .unwrap_or_else(|| self.execution.new_partner_state(self.task_id));
        let mut previous_execution = std::mem::replace(&mut self.execution, incoming_execution);
        previous_execution.invalidate_review_exposure();
        self.execution.invalidate_review_exposure();
        self.cooperation
            .executions
            .insert(self.lead.id, previous_execution);
        let owner = store::Attribution {
            partner: partner.id,
            name: Self::partner_label(partner),
            form: partner.form,
        };
        let incoming = self
            .cooperation
            .parked
            .remove(&partner.id)
            .unwrap_or_else(|| self.planning.new_partner(owner, self.task_id));
        let mut previous = std::mem::replace(&mut self.planning, incoming);
        previous.invalidate_review();
        self.planning.invalidate_review();
        self.cooperation.parked.insert(self.lead.id, previous);
        self.agents.active = self.planning.agent.index();
        self.agents.selected = self.agents.active;
    }

    pub fn planning_any_busy(&self) -> bool {
        self.delegation_active() > 0
            || std::iter::once(&self.planning)
                .chain(self.cooperation.parked.values())
                .any(|state| state.busy() || state.preferences_pending())
    }

    pub(super) fn cancelling_owned_work(&self) -> bool {
        self.cooperation.cancelling
    }

    pub(super) fn quit_requires_work_review(&self) -> bool {
        self.pending_request.is_some()
            || self.tool_plans.busy()
            || self.artifacts.busy()
            || self.tool_catalog.busy()
            || self.pending_command.is_some()
            || !self.cooperation.command_saves.is_empty()
            || std::iter::once(&self.planning)
                .chain(self.cooperation.parked.values())
                .any(planning::State::busy)
            || std::iter::once(&self.execution)
                .chain(self.cooperation.executions.values())
                .any(execution::State::busy)
    }

    pub fn planning_any_unknown(&self) -> bool {
        self.planning.outcome_unknown()
            || self
                .cooperation
                .parked
                .values()
                .any(planning::State::outcome_unknown)
    }

    pub fn planning_any_unknown_effects(&self) -> bool {
        std::iter::once(&self.planning)
            .chain(self.cooperation.parked.values())
            .any(planning::State::unknown_effects)
    }

    pub fn planning_any_journal_failed(&self) -> bool {
        self.planning.journal_failed()
            || self
                .cooperation
                .parked
                .values()
                .any(planning::State::journal_failed)
            || self
                .delegation
                .children
                .values()
                .any(|child| child.state.journal_failed())
    }

    pub fn planning_storage_failed(&self) -> bool {
        std::iter::once(&self.planning)
            .chain(self.cooperation.parked.values())
            .chain(self.delegation.children.values().map(|child| &child.state))
            .any(|state| {
                state.work_error.is_some()
                    || state.preference_error().is_some()
                    || state.usage.error.is_some()
            })
    }

    pub fn work_rows(&self) -> Vec<WorkRow> {
        let mut rows = Vec::new();
        for id in self.planning_ids() {
            let Some(state) = self.planning_state(id) else {
                continue;
            };
            if !state.owns_work() {
                continue;
            }
            let Some(owner) = state.owner().or(state.front.as_ref()).cloned() else {
                continue;
            };
            let waiting = state.preview.is_some() || state.request.is_some();
            rows.push(WorkRow {
                owner,
                status: if state.unknown_effects() {
                    "outcome unknown"
                } else if state.journal_failed() {
                    "journal failed"
                } else if state.usage.error.is_some() {
                    "usage save failed"
                } else if waiting {
                    "waiting"
                } else if state.cancelling {
                    "cancelling"
                } else if state.usage_pending() && !state.transport_pending() {
                    "saving usage"
                } else {
                    "working"
                },
                detail: if state.journal_failed() {
                    state.journal_notice.clone()
                } else if let Some(error) = &state.usage.error {
                    format!("Provider usage storage failed: {error}. /storage-retry retries storage only.")
                } else if state.usage_pending() && !state.transport_pending() {
                    "Provider transport settled; waiting for the exact request's reported-usage storage acknowledgement.".into()
                } else {
                    state.notice.clone()
                },
                local: false,
            });
        }
        for state in std::iter::once(&self.execution).chain(self.cooperation.executions.values()) {
            if (state.busy() || state.outcome_unknown())
                && let Some(owner) = state.owner.clone()
            {
                rows.push(WorkRow {
                    owner,
                    status: if state.outcome_unknown() {
                        "outcome unknown"
                    } else if state.approval_identity().is_some() {
                        "waiting"
                    } else {
                        "working"
                    },
                    detail: state.notice.clone(),
                    local: true,
                });
            }
        }
        for (partner, (_, owner, _)) in &self.cooperation.command_saves {
            if !rows.iter().any(|row| row.owner.partner == *partner) {
                rows.push(WorkRow { owner: owner.clone(), status: "saving", detail: "Waiting for the exact command draft's durable acknowledgement; nothing prepared.".into(), local: true });
            }
        }
        self.delegation_work_rows(&mut rows);
        rows
    }

    /// Match work-row ordering without cloning names, event text or row vectors.
    pub(crate) fn working_forms(&self) -> impl Iterator<Item = usize> + Clone + '_ {
        let providers = std::iter::once(&self.planning)
            .chain(self.cooperation.parked.values())
            .filter(|state| state.owns_work())
            .filter_map(|state| state.owner().or(state.front.as_ref()));
        let executions = std::iter::once(&self.execution)
            .chain(self.cooperation.executions.values())
            .filter(|state| state.busy() || state.outcome_unknown())
            .filter_map(|state| state.owner.as_ref());
        let active = providers.clone().chain(executions.clone());
        let saving = self
            .cooperation
            .command_saves
            .values()
            .filter(move |(_, owner, _)| {
                !active
                    .clone()
                    .any(|current| current.partner == owner.partner)
            })
            .map(|(_, owner, _)| owner);
        providers
            .chain(executions)
            .chain(saving)
            .chain(
                self.delegation
                    .children
                    .values()
                    .filter_map(|child| child.bench.as_ref().or(child.state.front.as_ref())),
            )
            .map(|owner| owner.form)
    }

    pub fn write_owner(&self) -> Option<&store::WorkClaim> {
        std::iter::once(&self.planning)
            .chain(self.cooperation.parked.values())
            .filter(|state| state.work_acquired)
            .filter_map(|state| state.work.as_ref())
            .find(|claim| claim.writing)
    }

    pub fn work_capacity_reason(&self, partner: i64) -> Option<String> {
        if self.cooperation.cancel_review.is_some()
            || self.cooperation.journal_discard.is_some()
            || self.cooperation.cancelling
            || self.cooperation.quitting
        {
            return Some("Finish the current work-safety review first; your draft is kept.".into());
        }
        if self.planning_any_unknown_effects() {
            return Some("Local effects remain unknown; inspect the owning request before starting more work. Your draft is kept.".into());
        }
        if self.partner_busy(partner) {
            return Some("This partner already owns work; finish or cancel its request first. Your draft is kept.".into());
        }
        let providers = std::iter::once(&self.planning)
            .chain(self.cooperation.parked.values())
            .filter(|state| state.owns_work())
            .count();
        let local = std::iter::once(&self.execution)
            .chain(self.cooperation.executions.values())
            .filter(|state| {
                (state.busy() || state.outcome_unknown())
                    && state.owner.as_ref().is_none_or(|owner| {
                        !self
                            .planning_state(owner.partner)
                            .is_some_and(planning::State::owns_work)
                    })
            })
            .count();
        let saving = self
            .cooperation
            .command_saves
            .keys()
            .filter(|partner| {
                !self
                    .planning_state(**partner)
                    .is_some_and(planning::State::owns_work)
                    && !self
                        .execution_state(**partner)
                        .is_some_and(execution::State::busy)
            })
            .count();
        let ordinary = providers + local + saving;
        (ordinary >= 3 || ordinary + self.delegation_active() >= 4).then(|| "Working partner/read-only child capacity is full; wait or cancel one. Your draft is kept.".into())
    }

    pub(super) fn stage_provider_work(&mut self) {
        let content = matches!(&self.planning.pending, Some(crate::providers::Request::Preview(input)) if !input.inspect_models && !input.inspect_sessions && input.work.is_none());
        if !content {
            return;
        }
        let replacement = self.record_instruction_replacement();
        if let Some(crate::providers::Request::Preview(input)) = &mut self.planning.pending {
            input.instruction_override = replacement;
        }
        if let Some(crate::providers::Request::Preview(mut input)) = self.planning.pending.take() {
            input.mcp = if input.agent.is_direct() {
                self.mcp.frozen(
                    &self.cwd,
                    input.operation == crate::providers::Operation::Implement,
                )
            } else {
                Vec::new()
            };
            if !input.local_only {
                self.profile_skip.remove(&input.task.lead_partner);
            }
            self.freeze_held_receipt(&input);
            self.planning.pending = Some(crate::providers::Request::Preview(input));
        }
        if self.planning.context_inspection() {
            self.remember_context_preparation(true);
            return;
        }
        self.remember_context_preparation(false);
        static REQUEST: AtomicU64 = AtomicU64::new(1);
        let claim = store::WorkClaim {
            task_id: self.task_id,
            owner: self
                .planning
                .front
                .clone()
                .unwrap_or_else(|| self.attribution()),
            request_id: format!(
                "provider-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |time| time.as_nanos()),
                REQUEST.fetch_add(1, Ordering::Relaxed)
            ),
            writing: match &self.planning.pending {
                Some(crate::providers::Request::Preview(input)) => {
                    crate::providers::writing_required(input.operation)
                }
                _ => false,
            },
        };
        self.freeze_delegation_parent(&claim);
        self.remember_move_request(&claim);
        self.planning.stage_work(claim);
    }

    pub fn drive_providers(
        &mut self,
        workers: &mut BTreeMap<i64, crate::providers::Worker>,
        store: &store::Worker,
        project: &Path,
        data_dir: &Path,
        exclusions: &[PathBuf],
    ) -> std::io::Result<usize> {
        let mut sent = 0;
        let mut ids = self.planning_ids();
        let len = ids.len();
        ids.rotate_left(self.cooperation.drive_cursor % len);
        self.cooperation.drive_cursor = (self.cooperation.drive_cursor + 1) % len;
        let motion_choice = self.motion_choice;
        let density_choice = self.density_choice;
        for id in ids {
            let preference_free = self.cooperation.preference_owners.is_empty();
            let move_receipt_pending = self.move_dispatch_pending(id);
            let delegated_children_pending = self
                .delegation
                .children
                .values()
                .any(|child| child.parent_partner == id);
            let nested_command_pending = self.cooperation.command_callbacks.contains_key(&id)
                || self.execution_state(id).is_some_and(|state| {
                    state.busy() || state.storage_pending() || state.outcome_unknown()
                });
            let state = self.planning_state_mut(id).expect("known partner slot");
            if let Some(reduced) = motion_choice {
                state.sync_motion_choice(reduced);
            }
            if let Some(density) = density_choice {
                state.sync_density_choice(density);
            }
            state.settle_work();
            if sent == 0
                && !state.work_inflight
                && state.work_error.is_none()
                && !((move_receipt_pending || nested_command_pending || delegated_children_pending)
                    && matches!(state.work_pending, Some(store::WorkRequest::Release(_))))
                && store.try_work(&mut state.work_pending)
            {
                state.work_inflight = true;
                sent += 1;
            }
            if sent == 0 && preference_free && store.try_preferences(state.preference_request()) {
                state.preference_started();
                self.cooperation.preference_owners.push_back(id);
                sent += 1;
            }
            let state = self.planning_state_mut(id).expect("known partner slot");
            if sent == 0 && state.receipts.try_store(store) {
                sent += 1;
            }
            if sent == 0 && state.try_usage(store) {
                sent += 1;
            }
            if sent == 0
                && let Some(request) = state.pending_journal.front().cloned()
            {
                let mut pending = Some(request);
                if store.try_journal(&mut pending) {
                    state.journal_started();
                    self.cooperation.journal_owners.push_back(id);
                    sent += 1;
                }
            }
            let state = self.planning_state_mut(id).expect("known partner slot");
            if state.pending.is_some() && !workers.contains_key(&id) {
                match crate::providers::Worker::start(
                    project.to_path_buf(),
                    data_dir.to_path_buf(),
                    exclusions.to_vec(),
                ) {
                    Ok(worker) => {
                        workers.insert(id, worker);
                    }
                    Err(error) => {
                        state.event(crate::providers::Event::Error { generation: None, message: format!("Provider worker could not start: {error}. No provider was launched.") });
                        state.failed_request = true;
                        state.settle_work();
                    }
                }
            }
            if let Some(worker) = workers.get(&id) {
                worker.try_request(&mut state.pending);
                worker.try_request(&mut state.steer_pending);
            }
        }
        sent += self.drive_delegation(store, project, data_dir, exclusions, sent == 0);
        Ok(sent)
    }

    pub fn poll_providers(
        &mut self,
        workers: &mut BTreeMap<i64, crate::providers::Worker>,
    ) -> (bool, bool) {
        let mut changed = false;
        let mut more = false;
        for (&id, worker) in workers.iter() {
            for index in 0..16 {
                let Some(event) = worker.try_event() else {
                    break;
                };
                more |= index == 15;
                changed |= self.provider_event(id, event);
            }
        }
        workers.retain(|id, worker| {
            worker.busy()
                || self.planning_state(*id).is_some_and(|state| {
                    state.transport_pending() || state.has_resumable_sessions()
                })
        });
        let ids = self.planning_ids();
        for id in ids {
            if id == self.lead.id {
                continue;
            }
            changed |= self.settle_background_provider(id);
            let failed = if let Some(state) = self.planning_state_mut(id) {
                if std::mem::take(&mut state.failed_request) {
                    state
                        .owner()
                        .cloned()
                        .map(|owner| (owner, state.submitted_request.to_string()))
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((owner, text)) = failed {
                changed = true;
                let drafts = self.partner_drafts.entry(owner.partner).or_default();
                if !drafts.drafts.iter().any(|draft| {
                    draft.field == store::EditorField::Request && !draft.text.is_empty()
                }) {
                    drafts
                        .drafts
                        .retain(|draft| draft.field != store::EditorField::Request);
                    drafts.drafts.push(store::Draft {
                        field: store::EditorField::Request,
                        cursor: text.len(),
                        text,
                    });
                    self.changed_task();
                    self.notice = format!(
                        "{}'s request was restored to its own draft; the foreground draft is unchanged.",
                        owner.name
                    );
                }
            }
        }
        if changed {
            self.refresh_approvals();
        }
        self.settle_cooperation();
        let (child_changed, child_more) = self.poll_delegation();
        changed |= child_changed;
        more |= child_more;
        changed |= self.reconcile_permission_review();
        changed |= self.observe_usage();
        (changed, more)
    }

    pub(super) fn cooperation_work_event(
        &mut self,
        request: store::WorkRequest,
        result: Result<Vec<store::WorkClaim>, String>,
    ) {
        let claim = match &request {
            store::WorkRequest::Acquire { claim, .. } | store::WorkRequest::Release(claim) => claim,
        };
        let moving = claim.writing
            && matches!(&request, store::WorkRequest::Acquire { .. })
            && result.is_ok();
        let task = claim.task_id;
        if let Some(state) = self.planning_state_mut(claim.owner.partner)
            && state.work.as_ref() == Some(claim)
        {
            state.work_event(request, result);
        }
        if moving {
            self.mark_commands_moving(task);
        }
        self.settle_cooperation();
    }

    pub(super) fn cooperation_preference_event(
        &mut self,
        result: Result<Option<store::ProviderPreferences>, String>,
    ) {
        let id = self
            .cooperation
            .preference_owners
            .pop_front()
            .unwrap_or(self.lead.id);
        let shared = result.as_ref().ok().cloned();
        let failed = result.as_ref().err().cloned();
        if let Some(state) = self.planning_state_mut(id) {
            state.preference_event(result);
        }
        if let Some(error) = failed {
            self.notice = format!(
                "Local preference storage failed for partner #{id}: {error}. /storage-retry retries storage only; the local choices and drafts are kept."
            );
        }
        if let Some(saved) = shared {
            for partner in self.planning_ids() {
                if partner != id
                    && let Some(state) = self.planning_state_mut(partner)
                {
                    state.receive_preferences(saved.as_ref());
                }
            }
        }
        self.sync_motion_choices();
        self.sync_density_choices();
        let ids: Vec<_> = self.party.iter().map(|partner| partner.id).collect();
        self.cooperation.parked.retain(|partner, state| {
            ids.contains(partner) || state.owns_work() || state.preferences_pending()
        });
    }

    pub(super) fn cooperation_journal_event(
        &mut self,
        task: i64,
        result: Result<store::JournalResponse, String>,
    ) {
        let id = self
            .cooperation
            .journal_owners
            .pop_front()
            .unwrap_or(self.lead.id);
        let failed = result.is_err();
        let purged = matches!(&result, Ok(store::JournalResponse::Purged));
        if let Some(state) = self.planning_state_mut(id) {
            state.journal_event(task, result);
            state.settle_work();
        }
        if purged {
            for owner in self.planning_ids() {
                if let Some(state) = self.planning_state_mut(owner) {
                    state.usage_purged(task);
                }
            }
        }
        if failed {
            self.notice = format!(
                "Partner {id}: journal storage failed. /history inspects retained evidence; /journal-retry retries storage only. A known-terminal failed write can be explicitly reviewed with /journal-discard."
            );
        }
        self.settle_cooperation();
    }

    pub(super) fn cancel_record_quit(&mut self) {
        self.quit = false;
        self.quit_after_save = false;
        self.records.quit_waiting = false;
        self.cooperation.quit_preferences_waiting = false;
        self.cooperation.quitting = false;
    }

    pub fn preferences_allow_quit(&mut self, requested: bool) -> bool {
        self.cooperation.quit_preferences_waiting |= requested;
        if !self.cooperation.quit_preferences_waiting {
            return false;
        }
        let mut allowed = true;
        let mut error = None;
        for id in self.planning_ids() {
            let state = self.planning_state_mut(id).expect("known partner slot");
            allowed &= state.preferences_allow_quit(true);
            if let Some(reason) = state.preference_error() {
                error = Some(reason.to_owned());
            }
        }
        if let Some(error) = error {
            self.notice = error;
            self.cooperation.quit_preferences_waiting = false;
            return false;
        }
        if allowed
            && !self.planning_any_busy()
            && !self.execution_any_pending()
            && !self.recipe_export_busy()
        {
            self.cooperation.quit_preferences_waiting = false;
            true
        } else {
            self.notice =
                "Waiting for every owned operation and provider preference commit before quitting…"
                    .into();
            false
        }
    }

    pub fn retry_cooperation_storage(&mut self) {
        for id in self.planning_ids() {
            let state = self.planning_state_mut(id).expect("known partner slot");
            state.retry_preferences();
            state.work_error = None;
            state.retry_usage();
        }
        for child in self.delegation.children.values_mut() {
            child.state.retry_usage();
            child.state.receipts.retry();
            child.state.retry_journal();
        }
    }

    pub fn refresh_approvals(&mut self) {
        self.collect_approvals();
        self.reconcile_permission_review();
    }

    fn collect_approvals(&mut self) {
        let mut pending = Vec::new();
        for id in self.planning_ids() {
            if let Some(state) = self.planning_state(id) {
                for (generation, request, arrival) in state.pending_identities() {
                    pending.push((
                        arrival,
                        Pending::Provider {
                            partner: id,
                            task: state.task_id(),
                            generation,
                            request,
                        },
                    ));
                }
            }
        }
        for state in std::iter::once(&self.execution).chain(self.cooperation.executions.values()) {
            if let Some((task, partner, request, run)) = state.approval_identity() {
                pending.push((
                    state.approval_arrival(),
                    Pending::Command {
                        partner,
                        task,
                        request: request.into(),
                        run: run.into(),
                    },
                ));
            }
        }
        pending.sort_by_key(|(arrival, _)| *arrival);
        if self
            .cooperation
            .focused
            .as_ref()
            .is_some_and(|focused| !pending.iter().any(|(_, request)| request == focused))
        {
            self.cooperation.focused = None;
        }
        self.cooperation
            .presented_permissions
            .retain(|shown| pending.iter().any(|(_, request)| request == shown));
        self.cooperation.approvals = pending;
    }

    /// Task-scoped auto-approval: answers the lead's current file/tool
    /// permission or agent command without presenting it, from any surface.
    /// Questions, other partners and settling/cancelling work are excluded.
    fn drive_auto_approval(&mut self) -> bool {
        if self.quit
            || self.cooperation.cancelling
            || self.cooperation.quitting
            || self.planning.cancelling
            || self.planning.task_id() != self.task_id
            || self
                .planning
                .owner()
                .is_none_or(|owner| owner.partner != self.lead.id)
        {
            return false;
        }
        // User-created rules (XC01) answer before the broad task-scoped switch.
        if self.drive_rules() {
            return true;
        }
        if self.mcp.effects_unknown() {
            return false;
        }
        if !self.planning.auto_approve {
            return false;
        }
        if self.planning.auto_accept() {
            self.notice = self.planning.notice.clone();
            if self.overlay == Some(Overlay::Planning) && self.planning.request.is_none() {
                self.planning.auto_permission_review = false;
            }
            return true;
        }
        let command = self
            .execution
            .approval_identity()
            .is_some_and(|(task, partner, _, _)| task == self.task_id && partner == self.lead.id)
            && self
                .cooperation
                .command_callbacks
                .contains_key(&self.lead.id)
            && self.execution.confirmation.is_none();
        if command && self.execution.auto_approve() {
            self.notice = format!(
                "Auto-approved command: {} · /auto-approve-off to review each command",
                self.execution
                    .prepared
                    .as_ref()
                    .map(execution::approval_command_summary)
                    .unwrap_or_default()
            );
            return true;
        }
        false
    }

    /// Reconcile after input and worker settlement, never before dispatching a key.
    pub(super) fn reconcile_permission_review(&mut self) -> bool {
        if self.drive_auto_approval() {
            return true;
        }
        if !self.approval_shortcut_available()
            || self.width < 60
            || self.height < 18
            || self.quit
            || self.cooperation.cancelling
            || self.cooperation.quitting
            || self.return_after_cancel.is_some()
            || self.pending_request.is_some()
            || self.pending_command.is_some()
            || self.cooperation.command_saves.contains_key(&self.lead.id)
            || self.switch_task.is_some()
            || self.management.is_some()
            || self.planning.cancelling
            || self.planning.pending.is_some()
            || self
                .planning
                .owner()
                .is_none_or(|owner| owner.partner != self.lead.id)
        {
            return false;
        }
        let file_review = self.planning.required_permission_review();
        let request = self
            .cooperation
            .approvals
            .iter()
            .find_map(|(_, pending)| match pending {
                Pending::Provider {
                    partner,
                    task,
                    generation,
                    request,
                } if file_review
                    && *partner == self.lead.id
                    && *task == self.task_id
                    && self.planning.matches(*generation, *task)
                    && self.planning.focus_prompt(*generation, request)
                    && !self.cooperation.presented_permissions.contains(pending) =>
                {
                    Some(pending.clone())
                }
                // Agent-requested commands are required tool approvals too;
                // explicitly typed local commands already open their own review.
                Pending::Command { partner, task, .. }
                    if *partner == self.lead.id
                        && *task == self.task_id
                        && self.cooperation.command_callbacks.contains_key(partner)
                        && !self.cooperation.presented_permissions.contains(pending) =>
                {
                    Some(pending.clone())
                }
                _ => None,
            });
        let Some(request) = request else {
            return false;
        };
        // open_overlay preserves the live Request editor, including its caret.
        // No editor apply, implicit send, grant or owner change occurs here.
        self.focus_pending_approval(request.clone());
        if self.cooperation.focused.as_ref() != Some(&request) {
            return false;
        }
        if matches!(request, Pending::Command { .. }) {
            self.execution.auto_presented = true;
        } else {
            self.planning.auto_permission_review = true;
        }
        self.cooperation.presented_permissions.push(request);
        true
    }

    pub fn approval_count(&self) -> usize {
        self.cooperation.approvals.len()
    }

    pub fn own_approval_pending(&self) -> bool {
        self.planning.request.is_some() || self.planning.required_permission_review()
    }

    pub fn partner_approval_pending(&self, partner: i64) -> bool {
        self.planning_state(partner)
            .is_some_and(|state| state.request.is_some() || state.required_permission_review())
    }

    /// The home shortcut never takes input from a modal or a literal editor.
    pub fn approval_shortcut_available(&self) -> bool {
        self.overlay.is_none()
            && !self.transitioning
            && !self.reset_after_save
            && !self.quit_after_save
            && self.confirmation.is_none()
            && self.command_popup.is_none()
            && self.nickname_editor.is_none()
            && self.cooperation_ui.release.is_none()
            && self.cooperation.cancel_review.is_none()
            && self.cooperation.journal_discard.is_none()
            && self.composer_menu.is_none()
            && (!self.editing || self.composer_is_request())
    }

    /// Informational only: exact permission scope is displayed in its review.
    pub fn pending_approval_detail(&self) -> Option<(&store::Attribution, Option<&str>)> {
        let (_, pending) = self.cooperation.approvals.first()?;
        match pending {
            Pending::Provider {
                partner, request, ..
            } => {
                let state = self.planning_state(*partner)?;
                let path = state.request.as_ref().and_then(|prompt| {
                    if request.as_ref() != Some(&prompt.request_id) {
                        return None;
                    }
                    match &prompt.kind {
                        crate::providers::PromptKind::Permission {
                            file: Some(file), ..
                        } => Some(file.path.as_str()),
                        _ => None,
                    }
                });
                Some((state.owner()?, path))
            }
            Pending::Command { partner, .. } => {
                let state = if *partner == self.lead.id {
                    &self.execution
                } else {
                    self.cooperation.executions.get(partner)?
                };
                Some((state.owner.as_ref()?, None))
            }
        }
    }

    pub fn approval_response_queued(&self) -> bool {
        matches!(&self.cooperation.queued_response,
            Some(Pending::Provider { partner, task, generation, .. })
                if *task == self.task_id
                    && self.planning_state(*partner)
                        .is_some_and(|state| state.matches(*generation, *task)))
    }

    pub(super) fn queue_approval_response_notice(&mut self) {
        if let Some(crate::providers::Request::Answer {
            generation,
            request_id,
            ..
        }) = &self.planning.pending
        {
            self.cooperation.queued_response = Some(Pending::Provider {
                partner: self.lead.id,
                task: self.planning.task_id(),
                generation: *generation,
                request: Some(request_id.clone()),
            });
        }
    }

    pub fn approval_position(&self) -> Option<(usize, usize)> {
        if !matches!(self.overlay, Some(Overlay::Planning | Overlay::Execution)) {
            return None;
        }
        let focused = self.cooperation.focused.as_ref();
        self.cooperation
            .approvals
            .iter()
            .position(|(_, request)| {
                if let Some(focused) = focused {
                    return request == focused;
                }
                match request {
                    Pending::Provider {
                        partner,
                        generation,
                        request,
                        ..
                    } if self.overlay == Some(Overlay::Planning) && *partner == self.lead.id => {
                        self.planning.focus_prompt(*generation, request)
                    }
                    Pending::Command {
                        task,
                        partner,
                        request,
                        run,
                    } if self.overlay == Some(Overlay::Execution) => self
                        .execution
                        .approval_identity()
                        .is_some_and(|(t, p, r, id)| {
                            t == *task && p == *partner && r == request && id == run
                        }),
                    _ => false,
                }
            })
            .map(|index| (index + 1, self.approval_count()))
    }

    pub(super) fn focus_approvals(&mut self) {
        self.collect_approvals();
        let Some((_, request)) = self.cooperation.approvals.first().cloned() else {
            self.notice = "No pending approvals or questions.".into();
            return;
        };
        self.focus_pending_approval(request);
    }

    pub(super) fn focus_own_approval(&mut self) -> bool {
        if !self.own_approval_pending() {
            return false;
        }
        self.collect_approvals();
        let request = self
            .cooperation
            .approvals
            .iter()
            .find_map(|(_, pending)| match pending {
                Pending::Provider {
                    partner,
                    task,
                    generation,
                    request,
                } if *partner == self.lead.id
                    && *task == self.task_id
                    && self.planning.focus_prompt(*generation, request) =>
                {
                    Some(pending.clone())
                }
                _ => None,
            });
        if let Some(request) = request {
            self.focus_pending_approval(request);
            true
        } else {
            false
        }
    }

    fn focus_pending_approval(&mut self, request: Pending) {
        let focused = match &request {
            Pending::Provider {
                partner,
                task,
                generation,
                request,
            } => {
                if self.lead.id != *partner {
                    self.request_lead(*partner);
                }
                if self.lead.id != *partner
                    || self.task_id != *task
                    || !self.planning.focus_prompt(*generation, request)
                {
                    false
                } else {
                    self.open_overlay(Overlay::Planning);
                    self.planning.auto_permission_review = false;
                    self.planning.invalidate_review();
                    true
                }
            }
            Pending::Command {
                partner,
                task,
                request,
                run,
            } => {
                if self.lead.id != *partner {
                    self.request_lead(*partner);
                }
                if self.lead.id == *partner
                    && self.execution.focus_approval(*task, *partner, request, run)
                {
                    self.open_overlay(Overlay::Execution);
                    true
                } else {
                    false
                }
            }
        };
        if focused {
            self.cooperation.focused = Some(request);
        }
    }

    pub(super) fn clear_approval_focus(&mut self) {
        self.planning.auto_permission_review = false;
        self.cooperation.focused = None;
    }

    pub(super) fn defer_approval(&mut self) {
        if let Some(request) = self.cooperation.focused.take()
            && self.planning.required_permission_review()
            && !self.cooperation.presented_permissions.contains(&request)
        {
            self.cooperation.presented_permissions.push(request);
        }
        self.planning.auto_permission_review = false;
        self.planning.invalidate_review();
        self.close_overlay();
        self.notice =
            "Request deferred, not answered. /approvals revisits pending requests.".into();
    }

    pub fn cancel_all_summary(&self) -> String {
        let mut lines = vec!["Owned work on this task:".into()];
        if self.tool_plans.busy() {
            lines.push("Task-owned tool plan: queued/running nodes; cancellation prevents queued admission.".into());
        }
        for row in self.work_rows() {
            lines.push(format!(
                "{} ({}): {}",
                row.owner.name,
                FORMS[row.owner.form.min(FORMS.len() - 1)].name,
                row.status
            ));
        }
        for state in std::iter::once(&self.execution).chain(self.cooperation.executions.values()) {
            if state.busy() && state.owner.is_none() {
                lines.push("Unknown owner: local command working.".into());
            }
        }
        for state in std::iter::once(&self.planning)
            .chain(self.cooperation.parked.values())
            .filter(|state| state.outcome_unknown())
        {
            let owner = state.owner().or(state.front.as_ref());
            lines.push(format!(
                "{}: provider outcome unknown.",
                owner.map_or("Unknown owner", |owner| owner.name.as_str())
            ));
        }
        for state in std::iter::once(&self.execution)
            .chain(self.cooperation.executions.values())
            .filter(|state| state.outcome_unknown())
        {
            lines.push(format!(
                "{}: command outcome unknown.",
                state
                    .owner
                    .as_ref()
                    .map_or("Unknown owner", |owner| owner.name.as_str())
            ));
        }
        if self.planning_any_journal_failed() || self.execution_any_journal_failed() {
            lines.push("Journal failed: retry storage, or explicitly review known-terminal failed provider writes with /journal-discard before leaving.".into());
        }
        if self.planning_storage_failed() {
            lines.push("Ownership/preferences unsettled: /storage-retry.".into());
        }
        lines.push("Waits for all work, journals and ownership release.".into());
        lines.push("No file rollback; remote work/billing may continue.".into());
        lines.join("\n")
    }

    pub fn request_cancel_all(&mut self) {
        self.open_cancel_review(false);
    }

    pub(super) fn open_cancel_review(&mut self, quit: bool) {
        self.cooperation.cancel_review = Some(CancelReview {
            quit,
            reviewed: false,
            affirmative: false,
            summary: self.cancel_all_summary(),
        });
    }

    pub(super) fn cancel_lead(&mut self) {
        if self.tool_plans.busy()
            && self
                .workflows
                .tool_plans
                .as_ref()
                .is_some_and(|plan| plan.partner == self.lead.id)
        {
            self.cancel_tool_plan();
            return;
        }
        if self
            .cooperation
            .command_saves
            .remove(&self.lead.id)
            .is_some()
        {
            self.notice = "Pending command cancelled; its saved draft is retained.".into();
            return;
        }
        if self
            .execution
            .request_cancel_for(self.task_id, self.lead.id)
        {
            self.refresh_approvals();
            return;
        }
        if self.planning.owns_work() {
            self.planning.cancel_owned();
        } else {
            self.notice = "The lead partner has no active work to cancel.".into();
        }
        self.refresh_approvals();
    }

    pub(super) fn cancellation_key(&mut self, key: KeyEvent) -> bool {
        let Some(review) = self.cooperation.cancel_review.as_mut() else {
            return false;
        };
        if key.kind != KeyEventKind::Press {
            return true;
        }
        if key.code == KeyCode::Esc || key.modifiers.is_empty() && key.code == KeyCode::Char('n') {
            self.cooperation.cancel_review = None;
            return true;
        }
        if !key.modifiers.is_empty() {
            return true;
        }
        if key.code == KeyCode::Enter && !review.affirmative {
            self.cooperation.cancel_review = None;
            return true;
        }
        match key.code {
            KeyCode::Left => review.affirmative = false,
            KeyCode::Right if review.reviewed => review.affirmative = true,
            KeyCode::Char('y') if review.reviewed => review.affirmative = true,
            _ => {}
        }
        if matches!(key.code, KeyCode::Enter | KeyCode::Char('y'))
            && review.reviewed
            && review.affirmative
        {
            let quit = review.quit;
            self.cooperation.cancel_review = None;
            self.cooperation.quitting = quit;
            self.cooperation.cancelling = true;
            self.cooperation.unknown_acknowledged =
                self.planning_any_unknown() || self.execution_any_unknown();
            self.pending_request = None;
            self.pending_command = None;
            self.cooperation.command_saves.clear();
            self.cancel_tool_plan();
            for id in self.planning_ids() {
                if let Some(state) = self.planning_state_mut(id) {
                    state.cancel_owned();
                }
            }
            for state in
                std::iter::once(&mut self.execution).chain(self.cooperation.executions.values_mut())
            {
                if state.busy() {
                    state.request_cancel();
                }
            }
            self.refresh_approvals();
            self.settle_cooperation();
        }
        true
    }

    pub(super) fn settle_cooperation(&mut self) {
        if self.cooperation.cancel_review.is_some() {
            let summary = self.cancel_all_summary();
            if let Some(review) = self.cooperation.cancel_review.as_mut()
                && review.summary != summary
            {
                review.summary = summary;
                review.reviewed = false;
                review.affirmative = false;
            }
        }
        if !self.cooperation.cancelling {
            return;
        }
        if self.planning_any_journal_failed() || self.execution_any_journal_failed() {
            self.notice = "Cancellation waits for retained journal storage. /journal-retry retries storage only; /journal-discard explicitly reviews known-terminal failed provider writes. No replay.".into();
            return;
        }
        if self.planning_storage_failed() {
            self.notice = "Cancellation waits for ownership release, preference, or reported-usage storage. /storage-retry retries storage only.".into();
            return;
        }
        if self.planning_any_busy() || self.execution_any_pending() {
            return;
        }
        self.cooperation.cancelling = false;
        if self.cooperation.quitting {
            self.cooperation.quitting = false;
            if (self.planning_any_unknown() || self.execution_any_unknown())
                && !self.cooperation.unknown_acknowledged
            {
                self.open_cancel_review(true);
            } else {
                self.finish_request_quit();
            }
        } else if self.planning_any_unknown() || self.execution_any_unknown() {
            self.notice = "Owned transports settled; an outcome remains unknown. No file rollback was performed.".into();
        } else {
            self.notice = "All owned work settled; no file rollback was performed.".into();
        }
    }
}

impl App {
    pub fn provider_event(&mut self, partner: i64, event: crate::providers::Event) -> bool {
        if let crate::providers::Event::DelegateCapacity {
            generation,
            task_id,
            reply,
        } = event
        {
            let _ = reply.try_send(self.delegation_capacity(partner, generation, task_id));
            return false;
        }
        if let crate::providers::Event::Preview(preview) = &event {
            self.remember_delegation_route(partner, preview);
        }
        if let crate::providers::Event::Delegate {
            generation,
            task_id,
            round,
            calls,
            reply,
            cancelled,
        } = event
        {
            return self.provider_delegate(
                partner,
                (generation, task_id),
                round,
                calls,
                reply,
                cancelled,
            );
        }
        if let crate::providers::Event::Progress {
            generation,
            task_id,
            source,
            steps,
        } = event
        {
            return self.provider_progress(partner, generation, task_id, source, steps);
        }
        if let crate::providers::Event::RunCommand {
            generation,
            task_id,
            data_dir,
            claim,
            call_id,
            cwd,
            command,
            cancelled,
            reply,
        } = event
        {
            return self.provider_command(
                partner, generation, task_id, data_dir, claim, call_id, cwd, command, cancelled,
                reply,
            );
        }
        if let crate::providers::Event::ToolArtifact {
            generation,
            task_id,
            capture,
            reply,
        } = event
        {
            let owned = self.planning_state(partner).is_some_and(|state| {
                state.matches(generation, task_id)
                    && state.work.as_ref().is_some_and(|claim| {
                        claim.request_id == capture.request_id
                            && capture.partner.as_ref() == Some(&claim.owner)
                    })
            });
            if owned {
                self.artifacts.retain_with_reply(task_id, capture, reply);
            } else {
                let _ = reply.try_send(Err(
                    "Tool artifact producer lost its exact request ownership".into(),
                ));
            }
            return owned;
        }
        if let crate::providers::Event::ArtifactLookup {
            generation,
            task_id,
            operation,
            reply,
        } = event
        {
            let owned = self.planning_state(partner).is_some_and(|state| {
                state.matches(generation, task_id)
                    && state.work.as_ref().is_some_and(|claim| {
                        claim.task_id == task_id && claim.owner.partner == partner
                    })
            });
            if owned {
                self.artifacts.enqueue_lookup(task_id, operation, reply);
            } else {
                let _ = reply.try_send(Err(
                    "Artifact lookup lost its exact task/request ownership".into(),
                ));
            }
            return owned;
        }
        if let crate::providers::Event::ConnectionFailed {
            generation,
            task_id,
            agent,
            failure,
        } = &event
        {
            let owned = self.planning_state(partner).is_some_and(|state| {
                state.agent == *agent
                    && state.matches(*generation, *task_id)
                    && state.owner().is_none_or(|owner| owner.partner == partner)
            });
            if !owned {
                return false;
            }
            // Failure evidence can update its provider's diagnostics, never
            // front another owner, reconnect, submit work or answer permission.
            self.agents.connection_failed(*agent, *failure);
            self.tool_catalog.disconnected(*agent);
            return true;
        }
        if let crate::providers::Event::Finished {
            generation,
            task_id,
            outcome: crate::providers::Outcome::Completed,
            ..
        } = &event
            && let Some(agent) = self.planning_state(partner).and_then(|state| {
                (state.matches(*generation, *task_id)
                    && !state.inspect_models
                    && !state.inspect_sessions
                    && state.owner().is_none_or(|owner| owner.partner == partner))
                .then_some(state.agent)
            })
        {
            self.agents.connection_succeeded(agent);
        }
        if partner == self.lead.id
            && self.planning.context_inspection()
            && let crate::providers::Event::Error {
                generation: None,
                message,
            } = &event
        {
            self.sources.clear_wire_request();
            self.sources.error.clone_from(message);
        }
        let content_preview = partner == self.lead.id
            && matches!(&event,
            crate::providers::Event::Preview(preview)
                if self.planning.accepts_preview(preview)
                    && (self.planning.context_inspection()
                        || !self.planning.inspect_models && !self.planning.inspect_sessions));
        if content_preview && let crate::providers::Event::Preview(preview) = &event {
            self.record_context_preview(preview);
        }
        let foreground_drawer = partner == self.lead.id && self.overlay == Some(Overlay::Planning);
        let response_settled = match (&self.cooperation.queued_response, &event) {
            (
                Some(Pending::Provider {
                    partner: owner,
                    task,
                    generation,
                    request,
                }),
                crate::providers::Event::PromptClosed {
                    generation: closed_generation,
                    request_id,
                },
            ) => {
                *owner == partner
                    && generation == closed_generation
                    && request.as_ref() == Some(request_id)
                    && self
                        .planning_state(partner)
                        .is_some_and(|state| state.matches(*generation, *task))
            }
            (
                Some(Pending::Provider {
                    partner: owner,
                    task,
                    generation,
                    ..
                }),
                crate::providers::Event::Finished {
                    generation: finished_generation,
                    task_id,
                    ..
                },
            ) => {
                *owner == partner
                    && generation == finished_generation
                    && task == task_id
                    && self
                        .planning_state(partner)
                        .is_some_and(|state| state.matches(*generation, *task))
            }
            _ => false,
        };
        let Some(state) = self.planning_state_mut(partner) else {
            return false;
        };
        if state.owner().is_some_and(|owner| owner.partner != partner) {
            return false;
        }
        let dispatched = match &event {
            crate::providers::Event::Dispatched {
                generation,
                task_id,
                request_id,
            } if state.matches(*generation, *task_id)
                && state.work.as_ref().is_some_and(|claim| {
                    claim.task_id == *task_id
                        && claim.owner.partner == partner
                        && claim.request_id == *request_id
                }) =>
            {
                Some(request_id.clone())
            }
            _ => None,
        };
        let move_finished = match &event {
            crate::providers::Event::Finished {
                generation,
                task_id,
                outcome,
                ..
            } if state.matches(*generation, *task_id) => {
                state.work.clone().map(|claim| (claim, *outcome))
            }
            _ => None,
        };
        let implemented = match &event {
            crate::providers::Event::Finished {
                generation,
                task_id,
                ..
            }
            | crate::providers::Event::LocalEffectsUnknown {
                generation,
                task_id,
                ..
            } => {
                state.matches(*generation, *task_id)
                    && state.operation == crate::providers::Operation::Implement
            }
            _ => false,
        };
        let previous = if foreground_drawer
            && matches!(
                &event,
                crate::providers::Event::PromptClosed { .. }
                    | crate::providers::Event::Finished { .. }
            ) {
            state
                .request
                .as_ref()
                .map(|request| request.request_id.clone())
        } else {
            None
        };
        state.event(event);
        let resolved = previous.as_ref().is_some_and(|previous| {
            state
                .request
                .as_ref()
                .is_none_or(|request| &request.request_id != previous)
        });
        if response_settled {
            self.cooperation.queued_response = None;
        }
        if let Some(request) = dispatched {
            self.move_request_dispatched(partner, &request);
            self.record_request_dispatched(partner, &request);
            if partner == self.lead.id {
                self.consume_berries();
            }
        }
        if resolved {
            self.defer_approval();
            self.notice = "The displayed request resolved without another answer.".into();
        }
        self.repo_refresh |= implemented;
        if let Some((claim, outcome)) = move_finished {
            self.move_provider_finished(&claim, outcome);
        }
        self.refresh_approvals();
        true
    }

    fn settle_background_provider(&mut self, partner: i64) -> bool {
        let large_enough = self.width >= 60 && self.height >= 18;
        let state = self
            .planning_state_mut(partner)
            .expect("known partner slot");
        let sending = state.send_after_prepare;
        let changed = state.settle_background(large_enough);
        let submitted = (sending
            && matches!(state.pending, Some(crate::providers::Request::Run { .. })))
        .then(|| state.submitted_request.to_string());
        if let Some(submitted) = submitted
            && let Some(drafts) = self.partner_drafts.get_mut(&partner)
        {
            let before = drafts.drafts.len();
            drafts.drafts.retain(|draft| {
                draft.field != store::EditorField::Request || draft.text != submitted
            });
            if before != drafts.drafts.len() {
                self.changed_task();
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{self, ItemKind};
    use serde_json::json;

    fn app() -> App {
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.goal = "Shared task facts".into();
        let partners = (1..=6)
            .map(|id| store::Partner {
                id,
                nickname: Some(format!("Owner{id}")),
                ..store::Partner::default()
            })
            .collect();
        let mut app = App::new("/workspace".into(), 120, 36);
        app.store_event(store::Event::Loaded(store::Project {
            tasks: vec![task],
            partners,
            active_id: Some(7),
            usage_bytes: 0,
        }));
        app
    }

    #[test]
    fn host_command_callback_requires_exact_live_implement_owner_and_fresh_call() {
        let mut app = app();
        start(&mut app, 1, 1);
        let history = app
            .execution
            .take_store_request()
            .expect("initial retained history load");
        assert!(matches!(history.operation, store::RunOperation::Load));
        app.execution
            .store_event(7, None, None, Ok(store::RunResponse::Runs(Vec::new())));
        let command = crate::workspace::runner::CommandSpec::Direct {
            program: "rustc".into(),
            args: vec!["main.rs".into()],
        };
        let mut claim = app.planning.work.clone().unwrap();
        let request = |claim: store::WorkClaim, generation, call: &str| {
            let (reply, received) = std::sync::mpsc::sync_channel(1);
            (
                providers::Event::RunCommand {
                    generation,
                    task_id: 7,
                    claim,
                    data_dir: "/not-opened-by-admission".into(),
                    call_id: call.into(),
                    cwd: "".into(),
                    command: command.clone(),
                    cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    reply,
                },
                received,
            )
        };
        let (event, received) = request(claim.clone(), 7, "readonly");
        assert!(!app.provider_event(1, event));
        assert!(received.try_recv().unwrap().is_err());
        claim.writing = true;
        app.planning.work = Some(claim.clone());
        app.planning.operation = providers::Operation::Implement;
        for (generation, cancelled) in [(8, false), (7, true)] {
            app.planning.cancelling = cancelled;
            let (event, received) = request(claim.clone(), generation, "stale");
            assert!(!app.provider_event(1, event));
            assert!(received.try_recv().unwrap().is_err());
            assert!(!app.execution.busy());
        }
        app.planning.cancelling = false;
        let (event, received) = request(claim.clone(), 7, "fresh");
        assert!(app.provider_event(1, event));
        assert!(app.execution.busy());
        assert!(
            app.execution.prepared.is_none(),
            "intent never grants dispatch"
        );
        app.execution.request_cancel();
        app.drive_executions(
            &mut std::collections::BTreeMap::new(),
            std::path::Path::new("/workspace"),
        );
        assert!(received.try_recv().unwrap().is_err());
        let (event, received) = request(claim.clone(), 7, "fresh");
        assert!(!app.provider_event(1, event));
        assert!(received.try_recv().unwrap().is_err());
        // A committed terminal-unknown run is idle, but never a fresh admission.
        let prepared = crate::workspace::runner::Prepared {
            request_id: "unknown-request".into(),
            run_id: "unknown-run".into(),
            task_id: 7,
            spec: command.clone(),
            cwd: "/workspace".into(),
            cwd_identity: "fixture".into(),
            executable: crate::workspace::runner::ExecutableIdentity {
                path: "/fixture/rustc".into(),
                sha256: "0".repeat(64),
                bytes: 1,
                modified_ns: 0,
                file_id: "fixture".into(),
            },
            prepared_at_ms: 1,
            expires_at_ms: 2,
            approval_arrival: 1,
            before: Default::default(),
            partner: Some(claim.owner.clone()),
            rerun_of: None,
        };
        let mut unknown = crate::workspace::runner::RunRecord::awaiting(&prepared);
        unknown.state = crate::workspace::runner::RunState::OutcomeUnknown;
        unknown.tree = crate::workspace::runner::TreeOutcome::OwnershipUnknown;
        unknown.started_at_ms = Some(1);
        unknown.ended_at_ms = Some(2);
        app.execution
            .runner_event(crate::workspace::runner::Event::Finished(unknown));
        app.execution.repo_started();
        app.execution.repo_observed(None);
        let update = app.execution.take_store_request().unwrap();
        let store::RunOperation::Update(record) = update.operation else {
            panic!("expected terminal journal");
        };
        app.execution.store_event(
            7,
            Some(&prepared.request_id),
            Some(&prepared.run_id),
            Ok(store::RunResponse::Updated(record)),
        );
        assert!(!app.execution.busy() && app.execution.outcome_unknown());
        let (event, received) = request(claim, 7, "new-call-after-unknown");
        assert!(!app.provider_event(1, event));
        assert!(received.try_recv().unwrap().is_err());
    }

    #[test]
    fn diagnostic_failures_keep_their_owner_and_reject_stale_observations() {
        use crate::providers::diagnostics::Failure;
        let mut app = app();
        app.agents.pending.take();
        start(&mut app, 1, 1);
        start(&mut app, 2, 2);
        let agent = app.planning_state(2).unwrap().agent;
        let other = if agent == providers::Agent::OpenAI {
            providers::Agent::Google
        } else {
            providers::Agent::OpenAI
        };
        app.input = "Future 界 draft".into();
        app.cursor = "Future ".len();
        app.overlay = Some(Overlay::Memory);
        let kept = (app.lead.id, app.input.clone(), app.cursor, app.overlay);
        for (partner, generation, task_id, route) in [
            (99, 7, 7, agent),
            (2, 8, 7, agent),
            (2, 7, 8, agent),
            (2, 7, 7, other),
        ] {
            assert!(!app.provider_event(
                partner,
                providers::Event::ConnectionFailed {
                    generation,
                    task_id,
                    agent: route,
                    failure: Failure::Http(401),
                }
            ));
            assert!(app.agents.failure(agent.index()).is_none());
        }
        assert!(app.provider_event(
            2,
            providers::Event::ConnectionFailed {
                generation: 7,
                task_id: 7,
                agent,
                failure: Failure::Http(401),
            }
        ));
        assert_eq!(app.agents.failure(agent.index()), Some(Failure::Http(401)));
        assert_eq!(
            (app.lead.id, app.input.clone(), app.cursor, app.overlay),
            kept
        );
        assert!(app.agents.pending.is_none() && app.agents.pending_auth.is_none());
    }

    #[test]
    fn external_disconnect_diagnostics_survive_owned_settlement_without_replay_or_draft_changes() {
        use providers::{Agent, Outcome, diagnostics::Failure};
        for agent in [
            Agent::Codex,
            Agent::Claude,
            Agent::OpenCode,
            Agent::Gemini,
            Agent::Devin,
        ] {
            for outcome in [Outcome::Failed, Outcome::Unknown] {
                let mut app = app();
                app.agents.pending.take();
                app.planning.agent = agent;
                start(&mut app, 1, 1);
                start(&mut app, 2, 2);
                app.open_editor(store::EditorField::Request);
                app.input = "Future 한글界e\u{301} draft".into();
                app.cursor = "Future 한글".len();
                let kept = (app.lead.id, app.input.clone(), app.cursor, app.overlay);
                let failure = |generation, task_id, route| providers::Event::ConnectionFailed {
                    generation,
                    task_id,
                    agent: route,
                    failure: Failure::Transport,
                };
                for (partner, generation, task, route) in [
                    (99, 7, 7, agent),
                    (2, 8, 7, agent),
                    (2, 7, 8, agent),
                    (2, 7, 7, Agent::OpenAI),
                ] {
                    assert!(!app.provider_event(partner, failure(generation, task, route)));
                    assert_eq!(app.agents.failure(agent.index()), None);
                }
                assert!(app.provider_event(2, failure(7, 7, agent)));
                assert_eq!(app.agents.failure(agent.index()), Some(Failure::Transport));
                assert!(
                    app.provider_event(
                        2,
                        providers::Event::Finished {
                            generation: 7,
                            task_id: 7,
                            outcome,
                            message:
                                "Owned stdio peer closed before authoritative completion; no replay"
                                    .into(),
                        }
                    )
                );
                let state = app.planning_state(2).unwrap();
                assert!(state.pending.is_none());
                assert_eq!(state.outcome_unknown(), outcome == Outcome::Unknown);
                // External opaque effects keep today's conservative blocking
                // settlement; a connection diagnostic cannot release them.
                assert_eq!(state.unknown_effects(), outcome == Outcome::Unknown);
                if outcome == Outcome::Unknown {
                    assert!(app.work_capacity_reason(3).is_some());
                }
                assert!(!app.planning.outcome_unknown());
                assert!(!app.provider_event(2, failure(7, 7, agent)));
                assert_eq!(app.agents.failure(agent.index()), Some(Failure::Transport));
                assert_eq!(
                    (app.lead.id, app.input.clone(), app.cursor, app.overlay),
                    kept
                );
                assert!(app.agents.pending.is_none() && app.agents.pending_auth.is_none());
            }
        }
    }

    #[test]
    fn only_owned_inference_success_retires_a_connection_failure() {
        use crate::providers::diagnostics::Failure;
        for metadata in [false, true] {
            let mut app = app();
            start(&mut app, 1, 1);
            let agent = app.planning.agent;
            app.agents.connection_failed(agent, Failure::Http(401));
            app.planning.inspect_models = metadata;
            app.provider_event(
                1,
                providers::Event::Finished {
                    generation: 8,
                    task_id: 7,
                    outcome: providers::Outcome::Completed,
                    message: "Foreign completion".into(),
                },
            );
            assert_eq!(app.agents.failure(agent.index()), Some(Failure::Http(401)));
            app.provider_event(
                1,
                providers::Event::Finished {
                    generation: 7,
                    task_id: 7,
                    outcome: providers::Outcome::Completed,
                    message: "Owned completion".into(),
                },
            );
            assert_eq!(
                app.agents.failure(agent.index()),
                metadata.then_some(Failure::Http(401))
            );
        }
    }

    fn start(app: &mut App, partner: i64, arrival: u64) {
        prepare_owned(app, partner, arrival, false);
    }

    fn prepare_owned(app: &mut App, partner: i64, arrival: u64, connection_review: bool) {
        let owner =
            store::Attribution::of(app.party.iter().find(|owner| owner.id == partner).unwrap());
        let mut task = app.tasks[0].clone();
        task.lead_partner = partner;
        task.form = owner.form;
        let mut state = app.planning.new_partner(owner.clone(), 7);
        state.command("new", true, &task, &[], None);
        let claim = store::WorkClaim {
            task_id: 7,
            owner,
            request_id: format!("owned-{partner}"),
            writing: false,
        };
        state.stage_work(claim.clone());
        let request = state.work_pending.take().unwrap();
        state.work_event(request, Ok(vec![claim]));
        assert!(matches!(
            state.pending.take(),
            Some(providers::Request::Preview(_))
        ));
        state.event(providers::Event::Preview(providers::Preview {
            transport: None,
            comparison_scope: None,
            arrival,
            generation: 7,
            task_id: 7,
            thread_id: None,
            destination: None,
            requires_connection_review: connection_review,
            trust: Vec::new(),
            payload: serde_json::json!({"request": format!("payload-{partner}")}).to_string(),
            blockers: Vec::new(),
        }));
        if !connection_review {
            assert!(state.dispatch_prepared());
            state.pending.take();
        }
        if partner == app.lead.id {
            app.planning = state;
        } else {
            app.cooperation.parked.insert(partner, state);
        }
    }

    fn prompt(app: &mut App, partner: i64, arrival: u64) {
        prompt_kind(
            app,
            partner,
            arrival,
            providers::PromptKind::Approval {
                command: format!("command-{partner}"),
                cwd: "/workspace".into(),
                reason: "Exact owned request".into(),
                allow_once: true,
                deny_decision: "decline".into(),
            },
        );
    }

    fn question_prompt(app: &mut App, partner: i64, arrival: u64) {
        prompt_kind(
            app,
            partner,
            arrival,
            providers::PromptKind::Questions(vec![providers::Question {
                id: "scope".into(),
                header: "Scope".into(),
                question: "Which exact file?".into(),
                options: Vec::new(),
                other: true,
                secret: false,
            }]),
        );
    }

    fn prompt_kind(app: &mut App, partner: i64, arrival: u64, kind: providers::PromptKind) {
        app.provider_event(
            partner,
            providers::Event::Prompt {
                generation: 7,
                task_id: 7,
                request: providers::Prompt {
                    arrival,
                    request_id: json!("same-provider-id"),
                    thread_id: "thread".into(),
                    turn_id: "turn".into(),
                    item_id: "tool".into(),
                    kind,
                },
            },
        );
    }

    fn file_prompt(app: &mut App, partner: i64, arrival: u64) {
        app.provider_event(
            partner,
            providers::Event::Prompt {
                generation: 7,
                task_id: 7,
                request: providers::Prompt {
                    arrival,
                    request_id: json!(format!("file-{arrival}")),
                    thread_id: "thread".into(),
                    turn_id: "turn".into(),
                    item_id: "file-tool".into(),
                    kind: providers::PromptKind::Permission {
                        description: "Exact provider scope".into(),
                        file: Some(providers::FileReview {
                            path: "src/new.rs".into(),
                            destination: None,
                            syntax: None,
                            kind: providers::FileReviewKind::Create,
                            directories: Vec::new(),
                            bytes: 0,
                            excerpt: String::new(),
                            edit: false,
                            external: false,
                        }),
                        options: vec![
                            providers::PermissionOption {
                                label: "deny".into(),
                                description: String::new(),
                                answer: json!("deny"),
                            },
                            providers::PermissionOption {
                                label: "once".into(),
                                description: String::new(),
                                answer: json!("once"),
                            },
                        ],
                    },
                },
            },
        );
    }

    fn close_file_prompt(app: &mut App, partner: i64, arrival: u64) {
        app.provider_event(
            partner,
            providers::Event::PromptClosed {
                generation: 7,
                request_id: json!(format!("file-{arrival}")),
            },
        );
    }

    #[test]
    fn automatic_generic_permission_requires_engagement_and_exact_owned_answer() {
        for allow in [false, true] {
            let mut app = app();
            start(&mut app, 1, 1);
            start(&mut app, 2, 2);
            app.open_editor(store::EditorField::Request);
            app.input = "Future 漢字 request".into();
            app.cursor = 7;
            let draft = (app.input.clone(), app.cursor);
            let permission = || providers::PromptKind::Permission {
                description: "One exact scoped call".into(),
                file: None,
                options: vec![
                    providers::PermissionOption {
                        label: "Deny".into(),
                        description: "No effects".into(),
                        answer: json!({"decision": "deny", "call": 42}),
                    },
                    providers::PermissionOption {
                        label: "Allow once".into(),
                        description: "Only call 42".into(),
                        answer: json!({"decision": "once", "call": 42}),
                    },
                ],
            };
            prompt_kind(&mut app, 2, 10, permission());
            assert!(app.overlay.is_none());
            prompt_kind(&mut app, 1, 20, permission());
            assert_eq!(app.overlay, Some(Overlay::Planning));
            assert_eq!(app.lead.id, 1);
            assert!(app.planning.auto_permission_review);
            for code in [KeyCode::Char('a'), KeyCode::Char('y'), KeyCode::Char('n')] {
                app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
            }
            app.update(Event::Paste("y\na\n".into()));
            app.update(Event::Key(KeyEvent::new_with_kind(
                KeyCode::Enter,
                KeyModifiers::NONE,
                KeyEventKind::Repeat,
            )));
            assert!(app.planning.pending.is_none());
            assert_eq!((app.input.clone(), app.cursor), draft);
            app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
            app.refresh_approvals();
            assert!(app.overlay.is_none());
            assert!(app.focus_own_approval());
            app.planning.reviewed = allow;
            app.planning_key(KeyEvent::new(
                KeyCode::Char(if allow { 'a' } else { 'n' }),
                KeyModifiers::NONE,
            ));
            assert!(matches!(
                app.planning.pending.take(),
                Some(providers::Request::Answer { generation: 7, request_id, answer })
                    if request_id == json!("same-provider-id")
                        && answer == json!({"decision": if allow { "once" } else { "deny" }, "call": 42})
            ));
            assert!(app.cooperation.parked[&2].pending.is_none());
            assert_eq!((app.input.clone(), app.cursor), draft);
        }
    }

    #[test]
    fn task_auto_approval_answers_file_permissions_without_review_but_not_other_partners() {
        let mut app = app();
        start(&mut app, 1, 1);
        start(&mut app, 2, 2);
        app.open_editor(store::EditorField::Request);
        app.input = "Unsent draft".into();
        app.cursor = 3;
        assert!(app.command("auto-approve"));
        assert!(app.planning.auto_approve);
        // Another partner's request is neither answered nor presented.
        file_prompt(&mut app, 2, 10);
        assert!(app.cooperation.parked[&2].pending.is_none());
        file_prompt(&mut app, 1, 20);
        assert!(
            app.overlay.is_none(),
            "auto-approval does not open a review"
        );
        assert!(matches!(
            app.planning.pending.take(),
            Some(providers::Request::Answer { answer, .. }) if answer == json!("once")
        ));
        assert_eq!((app.input.as_str(), app.cursor), ("Unsent draft", 3));
        assert!(app.notice.contains("src/new.rs"), "{}", app.notice);
        // Questions are never answered automatically.
        prompt_kind(
            &mut app,
            1,
            30,
            providers::PromptKind::Questions(vec![providers::Question {
                id: "q".into(),
                header: "Choice".into(),
                question: "Which?".into(),
                options: Vec::new(),
                other: true,
                secret: false,
            }]),
        );
        assert!(app.planning.pending.is_none());
        // Turning it off restores ordinary review.
        assert!(app.command("auto-approve-off"));
        assert!(!app.planning.auto_approve);
    }

    #[test]
    fn steering_is_acknowledged_or_returned_and_a_queued_send_waits_for_completion() {
        let mut app = app();
        start(&mut app, 1, 1);
        app.planning.agent = providers::Agent::OpenAI;
        app.open_editor(store::EditorField::Request);
        let key = |app: &mut App, code| {
            app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        };
        app.input = "Use src/b.rs instead".into();
        app.cursor = app.input.len();
        assert!(app.submit_request());
        assert!(
            app.steering.choice && app.notice.contains("s Steer"),
            "{}",
            app.notice
        );
        // Esc keeps the exact draft.
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.input, "Use src/b.rs instead");
        assert!(app.planning.steer_pending.is_none());
        assert!(app.submit_request());
        key(&mut app, KeyCode::Char('s'));
        assert!(app.input.is_empty());
        assert!(matches!(
            &app.planning.steer_pending,
            Some(providers::Request::Steer { generation: 7, text }) if text == "Use src/b.rs instead"
        ));
        app.planning.steer_pending = None;
        app.provider_event(
            1,
            providers::Event::Steered {
                generation: 7,
                task_id: 7,
                round: 2,
                texts: vec!["Use src/b.rs instead".into()],
            },
        );
        assert!(app.planning.steering.is_empty());
        // A second steer never reaches a boundary; the next Send is queued.
        app.input = "Also check c.rs".into();
        assert!(app.submit_request());
        key(&mut app, KeyCode::Char('s'));
        app.input = "Then write the docs".into();
        assert!(app.submit_request());
        key(&mut app, KeyCode::Char('q'));
        assert_eq!(
            app.steering
                .queued
                .get(&(7, 1))
                .map(|draft| draft.text.as_str()),
            Some("Then write the docs")
        );
        app.provider_event(
            1,
            providers::Event::Finished {
                generation: 7,
                task_id: 7,
                outcome: providers::Outcome::Failed,
                message: "fixture failure".into(),
            },
        );
        app.settle_steering();
        // Unsent steering returns at once; the queue waits for the claim release.
        assert!(app.planning.steering.is_empty());
        assert!(app.input.contains("Also check c.rs"), "{}", app.input);
        assert!(app.has_queued_send(), "still owned until release");
        app.planning.settle_work();
        if let Some(request) = app.planning.work_pending.take() {
            app.planning.work_event(request, Ok(Vec::new()));
        }
        app.planning.settle_work();
        app.settle_steering();
        assert!(app.input.contains("Also check c.rs"), "{}", app.input);
        assert!(
            app.input.contains("Then write the docs"),
            "failed run never sends the queue"
        );
        assert!(!app.has_queued_send());
        assert!(app.pending_request.is_none());
    }

    #[test]
    fn approval_rules_answer_matching_file_requests_and_deny_wins() {
        let mut app = app();
        let dir = std::env::temp_dir().join(format!("ev-rule-app-{}", std::process::id()));
        app.rules.path = dir.join(crate::workspace::rules::FILE_NAME);
        start(&mut app, 1, 1);
        file_prompt(&mut app, 1, 20);
        assert!(
            app.planning.pending.is_none(),
            "no rule yet: ordinary review"
        );
        assert!(app.command_with_argument("rule-allow src/*.rs launch"));
        assert!(app.notice.contains("Rule #1"), "{}", app.notice);
        app.refresh_approvals();
        assert!(matches!(
            app.planning.pending.take(),
            Some(providers::Request::Answer { answer, .. }) if answer == json!("once")
        ));
        assert!(app.notice.contains("Allowed by rule #1"), "{}", app.notice);
        // Another partner's request is not answered by the lead's rules.
        start(&mut app, 2, 2);
        file_prompt(&mut app, 2, 21);
        assert!(app.cooperation.parked[&2].pending.is_none());
        // A deny rule wins over the broader allow.
        let mut deny = app.rules.rules[0].clone();
        deny.effect = crate::workspace::rules::Effect::Deny;
        deny.kind = crate::workspace::rules::Kind::Write {
            ops: vec![crate::workspace::rules::WriteOp::Create],
            paths: vec!["src/new.rs".into()],
        };
        app.rules.add(deny).unwrap();
        file_prompt(&mut app, 1, 22);
        assert!(matches!(
            app.planning.pending.take(),
            Some(providers::Request::Answer { answer, .. }) if answer == json!("deny")
        ));
        // Revoking restores review before the next request.
        assert!(app.command_with_argument("rule-revoke 1"));
        assert!(app.command_with_argument("rule-revoke 2"));
        file_prompt(&mut app, 1, 23);
        assert!(app.planning.pending.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shift_a_in_an_engaged_file_review_allows_this_and_later_changes() {
        let mut app = app();
        start(&mut app, 1, 1);
        file_prompt(&mut app, 1, 20);
        assert!(app.planning.auto_permission_review);
        // Unengaged arrival ignores Shift+A like every other key.
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('A'),
            KeyModifiers::SHIFT,
        )));
        assert!(!app.planning.auto_approve && app.planning.pending.is_none());
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        app.planning.reviewed = true;
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('A'),
            KeyModifiers::SHIFT,
        )));
        assert!(app.planning.auto_approve);
        assert!(matches!(
            app.planning.pending.take(),
            Some(providers::Request::Answer { answer, .. }) if answer == json!("once")
        ));
    }

    #[test]
    fn prepared_connection_review_presents_once_and_never_dispatches_before_decision() {
        for allow in [false, true] {
            let mut app = app();
            app.open_editor(store::EditorField::Request);
            app.input = "Retained future request".into();
            app.cursor = 4;
            let draft = (app.input.clone(), app.cursor);
            prepare_owned(&mut app, 2, 1, true);
            app.refresh_approvals();
            assert!(app.overlay.is_none());
            prepare_owned(&mut app, 1, 2, true);
            app.refresh_approvals();
            assert_eq!(app.overlay, Some(Overlay::Planning));
            assert!(app.planning.auto_permission_review);
            assert!(!app.planning.dispatch_prepared());
            for code in [KeyCode::Char('y'), KeyCode::Char('n')] {
                app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
            }
            app.update(Event::Paste("y\n".into()));
            assert!(app.planning.preview.is_some() && app.planning.pending.is_none());
            app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
            app.refresh_approvals();
            assert!(app.overlay.is_none());
            assert_eq!((app.input.clone(), app.cursor), draft);
            assert!(app.focus_own_approval());
            assert!(!app.planning.dispatch_prepared());
            app.planning.reviewed = allow;
            app.planning_key(KeyEvent::new(
                KeyCode::Char(if allow { 'y' } else { 'n' }),
                KeyModifiers::NONE,
            ));
            if allow {
                assert!(matches!(
                    app.planning.pending,
                    Some(providers::Request::Run { generation: 7 })
                ));
            } else {
                assert!(app.planning.preview.is_none());
                assert!(!matches!(
                    app.planning.pending,
                    Some(providers::Request::Run { .. })
                ));
            }
            assert!(app.cooperation.parked[&2].preview.is_some());
            assert!(app.cooperation.parked[&2].pending.is_none());
            assert_eq!((app.input.clone(), app.cursor), draft);
        }
    }

    #[test]
    fn file_popup_reaches_every_safe_home_pane_and_restores_focus() {
        for pane in [Pane::Companion, Pane::Workspace, Pane::Inspector] {
            let mut app = app();
            start(&mut app, 1, 1);
            app.editing = false;
            app.focus = pane;
            let draft = (app.input.clone(), app.cursor);
            file_prompt(&mut app, 1, 2);
            assert_eq!(app.overlay, Some(Overlay::Planning));
            assert!(app.planning.auto_permission_review);
            assert!(app.planning.pending.is_none());
            app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
            assert!(app.overlay.is_none());
            assert_eq!(app.focus, pane);
            assert_eq!((app.input.clone(), app.cursor), draft);
            assert!(app.planning.request.is_some() && app.planning.pending.is_none());
        }
    }

    #[test]
    fn automatic_file_review_preserves_live_future_draft_and_defers_once() {
        let mut app = app();
        start(&mut app, 1, 1);
        app.open_editor(store::EditorField::Request);
        app.input = "Future 漢字 draft".into();
        app.cursor = "Future ".len();
        let draft = (app.input.clone(), app.cursor);
        file_prompt(&mut app, 1, 2);
        assert_eq!(app.overlay, Some(Overlay::Planning));
        assert!(app.planning.auto_permission_review);
        assert_eq!((app.input.clone(), app.cursor), draft);
        for code in [
            KeyCode::Char('a'),
            KeyCode::Char('y'),
            KeyCode::Char('n'),
            KeyCode::Backspace,
            KeyCode::Char('/'),
            KeyCode::Down,
        ] {
            app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        }
        for (code, modifiers, kind) in [
            (KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat),
            (KeyCode::Enter, KeyModifiers::CONTROL, KeyEventKind::Press),
            (KeyCode::Char('y'), KeyModifiers::NONE, KeyEventKind::Repeat),
            (
                KeyCode::Char('y'),
                KeyModifiers::CONTROL,
                KeyEventKind::Press,
            ),
        ] {
            app.update(Event::Key(KeyEvent::new_with_kind(code, modifiers, kind)));
        }
        app.update(Event::Paste("a\ny\nn\n".into()));
        app.planning_command("answers-review");
        assert!(app.planning.auto_permission_review && !app.planning.review_details);
        assert!(app.planning.pending.is_none() && app.pending_request.is_none());
        assert_eq!((app.input.clone(), app.cursor), draft);
        app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(app.overlay.is_none() && !app.planning.auto_permission_review);
        app.refresh_approvals();
        assert!(!app.settle_planning());
        assert!(app.overlay.is_none() && app.planning.request.is_some());
        assert_eq!((app.input.clone(), app.cursor), draft);
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.overlay, Some(Overlay::Planning));
        assert!(
            !app.planning.auto_permission_review,
            "manual reopen is deliberate"
        );
        assert!(app.planning.pending.is_none());
    }

    #[test]
    fn blocked_file_review_waits_for_home_and_closed_prompts_never_reappear() {
        for field in [
            store::EditorField::Goal,
            store::EditorField::Criteria,
            store::EditorField::Command,
            store::EditorField::NextAction,
        ] {
            let mut app = app();
            start(&mut app, 1, 1);
            app.open_editor(field);
            app.input = "literal future 漢字".into();
            app.cursor = 8;
            file_prompt(&mut app, 1, 2);
            assert!(app.overlay.is_none());
            assert_eq!((app.input.as_str(), app.cursor), ("literal future 漢字", 8));
            app.open_editor(store::EditorField::Request);
            assert!(app.settle_planning());
            assert!(app.planning.auto_permission_review);
        }
        for overlay in [
            Overlay::Help,
            Overlay::Party,
            Overlay::Planning,
            Overlay::Execution,
        ] {
            let mut app = app();
            start(&mut app, 1, 1);
            app.open_editor(store::EditorField::Request);
            app.open_overlay(overlay);
            file_prompt(&mut app, 1, 2);
            assert_eq!(app.overlay, Some(overlay));
            assert!(!app.planning.auto_permission_review);
            app.close_overlay();
            assert!(app.settle_planning());
            assert!(app.planning.auto_permission_review);
        }
        let mut app = app();
        start(&mut app, 1, 1);
        app.open_editor(store::EditorField::Request);
        app.open_overlay(Overlay::Help);
        file_prompt(&mut app, 1, 2);
        close_file_prompt(&mut app, 1, 2);
        app.close_overlay();
        assert!(!app.settle_planning());
        assert!(app.planning.request.is_none() && app.planning.pending.is_none());
        assert!(app.overlay.is_none() && !app.planning.auto_permission_review);
    }

    #[test]
    fn file_review_waits_for_save_cancel_and_queue_question_transitions() {
        for blocker in 0..7 {
            let mut app = app();
            start(&mut app, 1, 1);
            app.open_editor(store::EditorField::Request);
            match blocker {
                0 => app.transitioning = true,
                1 => app.reset_after_save = true,
                2 => app.quit_after_save = true,
                3 => app.pending_request = Some((app.revision, false, false)),
                4 => app.open_cancel_review(false),
                5 => {
                    app.composer_menu = Some(composer::Menu {
                        kind: composer::Kind::Files,
                        start: 0,
                        selected: 0,
                    })
                }
                _ => app.width = 59,
            }
            file_prompt(&mut app, 1, 2);
            assert!(app.overlay.is_none() && !app.planning.auto_permission_review);
            app.transitioning = false;
            app.reset_after_save = false;
            app.quit_after_save = false;
            app.pending_request = None;
            app.cooperation.cancel_review = None;
            app.composer_menu = None;
            app.width = 120;
            assert!(app.settle_planning());
            assert!(app.planning.auto_permission_review && app.planning.pending.is_none());
        }
        let mut app = app();
        start(&mut app, 1, 1);
        app.open_editor(store::EditorField::Request);
        question_prompt(&mut app, 1, 10);
        file_prompt(&mut app, 1, 20);
        assert!(
            app.overlay.is_none(),
            "file must not displace a queued question"
        );
        app.provider_event(
            1,
            providers::Event::PromptClosed {
                generation: 7,
                request_id: json!("same-provider-id"),
            },
        );
        assert_eq!(app.overlay, Some(Overlay::Planning));
        assert!(app.planning.auto_permission_review && app.planning.pending.is_none());
    }

    #[test]
    fn consecutive_owned_reviews_reconcile_without_opening_foreign_work() {
        let mut app = app();
        start(&mut app, 1, 1);
        start(&mut app, 2, 2);
        app.open_editor(store::EditorField::Request);
        file_prompt(&mut app, 2, 10);
        assert!(app.overlay.is_none());
        assert_eq!(app.lead.id, 1);
        file_prompt(&mut app, 1, 20);
        file_prompt(&mut app, 1, 30);
        assert_eq!(app.approval_position(), Some((2, 3)));
        assert_eq!(
            app.planning.request.as_ref().unwrap().request_id,
            json!("file-20")
        );
        close_file_prompt(&mut app, 2, 10);
        assert_eq!(
            app.planning.request.as_ref().unwrap().request_id,
            json!("file-20")
        );
        app.provider_event(
            1,
            providers::Event::PromptClosed {
                generation: 8,
                request_id: json!("file-20"),
            },
        );
        assert!(
            app.planning.auto_permission_review,
            "stale closure cannot affect live review"
        );
        close_file_prompt(&mut app, 1, 20);
        assert_eq!(app.overlay, Some(Overlay::Planning));
        assert!(app.planning.auto_permission_review);
        assert_eq!(
            app.planning.request.as_ref().unwrap().request_id,
            json!("file-30")
        );
        assert!(app.planning.pending.is_none());
        prompt(&mut app, 1, 40);
        close_file_prompt(&mut app, 1, 30);
        assert_eq!(app.overlay, Some(Overlay::Planning));
        assert!(app.planning.auto_permission_review);
        assert!(app.planning.request.is_some() && app.planning.pending.is_none());
        assert_eq!(app.lead.id, 1);
    }

    #[test]
    fn request_enter_reviews_own_prompt_before_empty_or_busy_checks() {
        for agent in [providers::Agent::ChatGPT, providers::Agent::Codex] {
            for text in ["", "Future 漢字 request"] {
                let mut app = app();
                app.planning.agent = agent;
                start(&mut app, 1, 1);
                app.open_editor(store::EditorField::Request);
                app.input = text.into();
                app.cursor = text.find('漢').unwrap_or(text.len());
                let draft = (app.input.clone(), app.cursor);
                prompt(&mut app, 1, 2);
                assert_eq!(app.overlay, Some(Overlay::Planning));
                assert_eq!((app.input.clone(), app.cursor), draft);

                app.update(Event::Key(KeyEvent::new(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                )));
                assert_eq!(app.overlay, Some(Overlay::Planning));
                assert_eq!(app.lead.id, 1);
                assert!(!app.planning.reviewed && !app.planning.affirmative);
                assert!(app.pending_request.is_none() && app.planning.pending.is_none());
                app.update(Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                )));
                assert!(app.planning.pending.is_none(), "held Enter cannot answer");
                app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
                assert!(app.overlay.is_none());
                assert_eq!((app.input.clone(), app.cursor), draft);
                assert!(app.editing && app.planning.request.is_some());
            }
        }
    }

    #[test]
    fn output_enter_reviews_own_question_while_alt_a_keeps_global_fifo() {
        let mut app = app();
        start(&mut app, 1, 1);
        start(&mut app, 2, 2);
        app.open_editor(store::EditorField::Request);
        app.input = "future draft".into();
        app.cursor = 3;
        app.retain_editor();
        app.focus = Pane::Workspace;
        prompt(&mut app, 2, 10);
        question_prompt(&mut app, 1, 20);

        app.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.lead.id, 1, "Enter belongs to the current request");
        assert_eq!(app.overlay, Some(Overlay::Planning));
        assert!(app.planning.pending.is_none() && app.pending_request.is_none());
        app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::ALT,
        )));
        assert_eq!(app.lead.id, 2, "Alt+A follows producer arrival order");
        assert_eq!(app.approval_position(), Some((1, 2)));
        assert!(app.planning.pending.is_none());
        app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        app.request_lead(1);
        app.open_editor(store::EditorField::Request);
        assert_eq!((app.input.as_str(), app.cursor), ("future draft", 3));
    }

    #[test]
    fn alt_a_requires_exact_press_and_never_interprets_pasted_text() {
        let mut app = app();
        start(&mut app, 1, 1);
        prompt(&mut app, 1, 2);
        app.defer_approval();
        app.open_editor(store::EditorField::Request);
        for (modifiers, kind) in [
            (KeyModifiers::ALT, KeyEventKind::Repeat),
            (KeyModifiers::ALT, KeyEventKind::Release),
            (KeyModifiers::ALT | KeyModifiers::SHIFT, KeyEventKind::Press),
            (
                KeyModifiers::ALT | KeyModifiers::CONTROL,
                KeyEventKind::Press,
            ),
            (KeyModifiers::ALT | KeyModifiers::SUPER, KeyEventKind::Press),
        ] {
            app.update(Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('a'),
                modifiers,
                kind,
            )));
            assert!(app.overlay.is_none());
            assert!(app.planning.pending.is_none());
        }
        app.update(Event::Paste("Alt+A\n".into()));
        assert!(app.overlay.is_none());
        assert_eq!(app.input, "Alt+A\n");
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::ALT,
        )));
        assert_eq!(app.overlay, Some(Overlay::Planning));
        assert_eq!(app.input, "Alt+A\n");
        assert!(app.planning.pending.is_none() && app.pending_request.is_none());
    }

    #[test]
    fn pending_review_shortcut_respects_literal_editors_and_overlay_ownership() {
        let shortcut = Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT));
        for field in [
            store::EditorField::Goal,
            store::EditorField::Criteria,
            store::EditorField::Command,
            store::EditorField::NextAction,
        ] {
            let mut app = app();
            start(&mut app, 1, 1);
            app.editing = true;
            app.editor_field = field;
            app.input = "literal draft".into();
            app.cursor = 4;
            prompt(&mut app, 1, 2);
            app.update(shortcut.clone());
            assert!(app.overlay.is_none());
            assert_eq!((app.input.as_str(), app.cursor), ("literal draft", 4));
            assert!(app.planning.pending.is_none());
        }
        for overlay in [Overlay::Help, Overlay::Sources, Overlay::Planning] {
            let mut app = app();
            start(&mut app, 1, 1);
            app.open_overlay(overlay);
            prompt(&mut app, 1, 2);
            app.update(shortcut.clone());
            assert_eq!(app.overlay, Some(overlay));
            assert!(app.planning.pending.is_none());
        }
        let mut app = app();
        start(&mut app, 1, 1);
        app.open_commands();
        prompt(&mut app, 1, 2);
        app.update(shortcut.clone());
        assert!(app.command_popup.is_some() && app.overlay.is_none());
        assert!(app.planning.pending.is_none());
        app.command_popup = None;
        app.confirmation = Some(Confirmation {
            kind: ConfirmationKind::DiscardDraft,
            destructive: false,
        });
        app.update(shortcut);
        assert!(app.confirmation.is_some() && app.overlay.is_none());
        assert!(app.planning.pending.is_none());
    }

    #[test]
    fn queued_response_notice_settles_only_for_its_correlated_callback_or_turn() {
        for finish_turn in [false, true] {
            let mut app = app();
            start(&mut app, 1, 1);
            start(&mut app, 2, 2);
            app.open_editor(store::EditorField::Request);
            app.input = "future draft".into();
            app.cursor = 3;
            prompt(&mut app, 1, 10);
            prompt(&mut app, 2, 20);
            app.focus_approvals();
            app.planning.reviewed = true;
            app.planning_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
            assert!(matches!(
                app.planning.pending.take(),
                Some(providers::Request::Answer { .. })
            ));
            assert!(app.approval_response_queued());
            assert_eq!((app.input.as_str(), app.cursor), ("future draft", 3));
            assert!(app.editing && app.overlay.is_none());
            app.notice = "An unrelated storage error".into();
            app.provider_event(
                2,
                providers::Event::PromptClosed {
                    generation: 7,
                    request_id: json!("same-provider-id"),
                },
            );
            assert!(
                app.approval_response_queued(),
                "same provider id, different owner"
            );
            app.provider_event(
                1,
                providers::Event::PromptClosed {
                    generation: 8,
                    request_id: json!("same-provider-id"),
                },
            );
            assert!(app.approval_response_queued(), "stale generation");
            app.provider_event(
                1,
                providers::Event::PromptClosed {
                    generation: 7,
                    request_id: json!("another-request"),
                },
            );
            assert!(app.approval_response_queued(), "different callback");
            if finish_turn {
                app.provider_event(
                    1,
                    providers::Event::Finished {
                        generation: 7,
                        task_id: 7,
                        outcome: providers::Outcome::Completed,
                        message: "Completed".into(),
                    },
                );
            } else {
                app.provider_event(
                    1,
                    providers::Event::PromptClosed {
                        generation: 7,
                        request_id: json!("same-provider-id"),
                    },
                );
            }
            assert!(!app.approval_response_queued());
            assert_eq!(app.notice, "An unrelated storage error");
        }
    }

    #[test]
    fn actual_auto_file_review_requires_engagement_then_rendered_scope_for_once() {
        use crate::{
            config::{ColorMode, Config, ImageMode, LayoutMode, Theme},
            messages::Message,
        };
        use ratatui::{Terminal, backend::TestBackend};

        let config = Config {
            native_transport: crate::providers::NativeTransport::Auto,
            retry: Default::default(),
            ascii: true,
            no_animation: true,
            animation_override: Some(false),
            theme: Theme::Dark,
            color: ColorMode::Mono,
            images: ImageMode::Off,
            assets_dir: "unused".into(),
            layout: LayoutMode::Sanctuary,
            layout_explicit: false,
            data_dir: "unused-data".into(),
            storage_policy: store::Policy::default(),
            context_exclusions: Vec::new(),
            capture_capacity: 64 * 1024,
        };
        for (width, height) in [(60, 18), (120, 36)] {
            for editing in [true, false] {
                let mut app = app();
                app.width = width;
                app.height = height;
                start(&mut app, 1, 1);
                app.planning.work.as_mut().unwrap().writing = true;
                app.open_editor(store::EditorField::Request);
                app.input = "Future 漢字 request".into();
                app.cursor = "Future ".len();
                app.editing = editing;
                app.focus = Pane::Workspace;
                let draft = (app.input.clone(), app.cursor);
                file_prompt(&mut app, 1, 2);
                assert_eq!(app.overlay, Some(Overlay::Planning));
                assert!(app.planning.auto_permission_review);
                let mut artwork = crate::ui::artwork::Artwork::load(&config);
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                let mut draw = |app: &mut App| {
                    terminal
                        .draw(|frame| {
                            crate::ui::render(frame, app, &config, &mut artwork, "off");
                        })
                        .unwrap();
                    let buffer = terminal.backend().buffer();
                    let mut screen = String::new();
                    for y in 0..height {
                        let mut x = 0;
                        while x < width {
                            let symbol = buffer[(x, y)].symbol();
                            screen.push_str(symbol);
                            x += ratatui::text::Span::raw(symbol).width().max(1) as u16;
                        }
                        screen.push('\n');
                    }
                    screen
                };
                let screen = draw(&mut app);
                assert!(screen.contains("src/new.rs"), "{screen}");
                assert_eq!((app.input.clone(), app.cursor), draft);
                for code in [KeyCode::Char('a'), KeyCode::Char('y'), KeyCode::Char('n')] {
                    app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
                }
                app.update(Event::Paste("a\ny\nn\n".into()));
                assert!(app.planning.auto_permission_review && app.planning.pending.is_none());
                app.update(Event::Key(KeyEvent::new(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                )));
                assert!(!app.planning.auto_permission_review && app.planning.review_details);
                assert!(!app.planning.reviewed && app.planning.pending.is_none());
                app.update(Event::Key(KeyEvent::new(
                    KeyCode::Char('a'),
                    KeyModifiers::NONE,
                )));
                assert!(
                    app.planning.pending.is_none(),
                    "engagement does not reuse prior render authority"
                );
                let engaged = draw(&mut app);
                app.update(Event::Key(KeyEvent::new(
                    KeyCode::Char('d'),
                    KeyModifiers::NONE,
                )));
                let engaged = format!("{engaged}{}", draw(&mut app));
                assert!(app.planning.reviewed, "{engaged}");
                app.update(Event::Key(KeyEvent::new(
                    KeyCode::Char('a'),
                    KeyModifiers::NONE,
                )));
                assert!(matches!(
                    app.planning.pending.take(),
                    Some(providers::Request::Answer { request_id, answer, .. })
                        if request_id == json!("file-2") && answer == json!("once")
                ));
                assert!(app.overlay.is_none() && app.planning.request.is_none());
                assert_eq!((app.input.clone(), app.cursor), draft);
                app.update(Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('a'),
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                )));
                assert!(
                    app.planning.pending.is_none(),
                    "one review grants once only"
                );
                app.provider_event(
                    1,
                    providers::Event::Finished {
                        generation: 7,
                        task_id: 7,
                        outcome: providers::Outcome::Completed,
                        message: "Completed".into(),
                    },
                );
                app.refresh_approvals();
                let settled = draw(&mut app);
                assert!(
                    !settled.contains(Message::ProvidersPlanningPermissionAutoReviewActions.text())
                );
                assert!(
                    !settled.contains(Message::CoreWaitingForUser.text()),
                    "{settled}"
                );
            }
        }
    }

    #[test]
    fn clearing_exact_paths_preserves_draft_caret_and_has_no_dispatch_or_grant() {
        for agent in [
            providers::Agent::Compatible,
            providers::Agent::Codex,
            providers::Agent::Claude,
        ] {
            let mut app = app();
            app.planning.agent = agent;
            app.planning.set_operation(providers::Operation::Implement);
            app.planning.edit_paths = vec!["missing/nested/새.rs".into()];
            app.input = "보존할 초안\nliteral /paths []\n".into();
            app.cursor = "보존할 ".len();
            let before = (app.input.clone(), app.cursor);
            assert!(app.planning_command("paths"));
            let characters = app
                .planning
                .paths_editor
                .as_ref()
                .unwrap()
                .text
                .chars()
                .count();
            app.planning_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
            for _ in 0..characters {
                app.planning_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
            }
            assert!(app.planning.paste("[]"));
            assert_eq!(
                app.planning.edit_paths,
                vec![PathBuf::from("missing/nested/새.rs")]
            );
            assert_eq!((app.input.clone(), app.cursor), before);
            assert!(app.planning_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
            assert!(app.planning.edit_paths.is_empty() && app.planning.paths_editor.is_none());
            assert_eq!((app.input.clone(), app.cursor), before);
            assert_eq!(app.planning.operation, providers::Operation::Implement);
            assert!(app.planning.pending.is_none() && app.planning.work_pending.is_none());
            assert!(app.planning.work.is_none() && app.planning.preview.is_none());
            assert!(app.planning.pending_journal.is_empty() && !app.planning.busy());
            assert_eq!(app.approval_count(), 0);
        }
    }

    #[test]
    fn native_scope_takes_writer_claim_but_selection_and_metadata_do_not() {
        for agent in providers::Agent::ALL {
            for operation in [
                providers::Operation::Plan,
                providers::Operation::Implement,
                providers::Operation::Review,
            ] {
                for has_paths in [false, true] {
                    let mut app = app();
                    app.planning.agent = agent;
                    app.planning.set_operation(operation);
                    if has_paths {
                        app.planning.edit_paths.push("missing/nested/new.rs".into());
                    }
                    assert!(app.planning.pending.is_none() && app.planning.work.is_none());
                    let task = app.tasks[0].clone();
                    app.planning.command("new", true, &task, &[], None);
                    let Some(providers::Request::Preview(input)) = &app.planning.pending else {
                        panic!("content preparation must freeze an input");
                    };
                    assert_eq!(input.operation, operation);
                    assert_eq!(
                        input.edit_paths.is_empty(),
                        !has_paths || operation != providers::Operation::Implement
                    );
                    // Mutable role/selection changes cannot change the frozen request's lease.
                    app.planning.operation = if operation == providers::Operation::Implement {
                        providers::Operation::Plan
                    } else {
                        providers::Operation::Implement
                    };
                    app.planning.edit_paths.clear();
                    app.stage_provider_work();
                    let Some(store::WorkRequest::Acquire { claim, .. }) =
                        &app.planning.work_pending
                    else {
                        panic!("content preparation must stage a durable claim");
                    };
                    assert_eq!(claim.writing, operation == providers::Operation::Implement);
                    let request = app.planning.work_pending.take().unwrap();
                    app.planning.work_event(
                        request,
                        Err("Another partner holds the task writer lease".into()),
                    );
                    assert!(app.planning.pending.is_none());
                    assert!(app.planning.work.is_none() && app.planning.held_input.is_none());
                    assert!(!app.planning.transport_pending());
                }
            }
            let mut app = app();
            app.planning.agent = agent;
            app.planning.set_operation(providers::Operation::Implement);
            app.planning.edit_paths.push("missing/new.rs".into());
            let task = app.tasks[0].clone();
            app.planning
                .command("models-refresh", true, &task, &[], None);
            app.stage_provider_work();
            assert!(app.planning.work_pending.is_none() && app.planning.work.is_none());
        }
    }

    #[test]
    fn unknown_local_effects_cannot_move_to_foreign_owner_or_allow_more_work() {
        let mut app = app();
        start(&mut app, 1, 1);
        start(&mut app, 2, 2);
        app.provider_event(
            3,
            providers::Event::LocalEffectsUnknown {
                generation: 7,
                task_id: 7,
                message: "foreign owner".into(),
            },
        );
        assert!(!app.planning_any_unknown_effects());
        app.provider_event(
            2,
            providers::Event::LocalEffectsUnknown {
                generation: 7,
                task_id: 7,
                message: "own local effects unknown".into(),
            },
        );
        assert!(!app.planning.unknown_effects());
        assert!(app.cooperation.parked[&2].unknown_effects());
        app.provider_event(
            2,
            providers::Event::Finished {
                generation: 7,
                task_id: 7,
                outcome: providers::Outcome::Cancelled,
                message: "settled".into(),
            },
        );
        assert!(app.work_capacity_reason(3).is_some());
        assert!(app.planning_any_unknown_effects());
    }

    #[test]
    fn same_agent_generations_and_arrival_fifo_are_owner_isolated() {
        let mut app = app();
        for partner in 1..=3 {
            start(&mut app, partner, partner as u64);
        }
        app.request_lead(6);
        app.open_editor(store::EditorField::Request);
        app.update(Event::Paste("Future 漢字 draft".into()));
        app.cursor = "Future ".len();
        let draft = (app.input.clone(), app.cursor);
        for partner in [3, 1, 2] {
            app.provider_event(
                partner,
                providers::Event::Item {
                    generation: 7,
                    task_id: 7,
                    thread_id: "thread".into(),
                    turn_id: "turn".into(),
                    item_id: "same-item".into(),
                    kind: ItemKind::Message,
                    text: format!("response-{partner}"),
                    complete: true,
                },
            );
        }
        // Deliberately ingest in the reverse order: producer stamps, not map polling, own FIFO.
        prompt(&mut app, 1, 30);
        prompt(&mut app, 3, 10);
        prompt(&mut app, 2, 20);
        app.refresh_approvals();
        assert_eq!((app.input.clone(), app.cursor), draft);
        assert_eq!(app.approval_count(), 3);
        assert_eq!(app.overlay, None, "background prompts do not steal focus");
        app.focus_approvals();
        assert_eq!(app.lead.id, 3);
        assert_eq!(app.approval_position(), Some((1, 3)));
        app.planning_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.approval_count(), 3);
        for partner in 1..=3 {
            let state = app.planning_state(partner).unwrap();
            assert_eq!(state.items[0].text, format!("response-{partner}"));
            assert!(state.request.is_some());
            assert!(state.pending.is_none(), "deferral never sends an answer");
            assert_eq!(state.work.as_ref().unwrap().owner.partner, partner);
        }
        app.request_lead(6);
        assert_eq!((app.input.clone(), app.cursor), draft);
        assert!(!app.provider_event(
            99,
            providers::Event::Error {
                generation: Some(7),
                message: "late foreign event".into()
            }
        ));
    }

    #[test]
    fn lead_cancel_and_neutral_all_review_preserve_other_live_claims() {
        let mut app = app();
        for partner in 1..=3 {
            start(&mut app, partner, partner as u64);
        }
        app.request_lead(2);
        app.cancel_lead();
        assert!(app.planning.cancelling);
        assert!(!app.planning_state(1).unwrap().cancelling);
        assert!(!app.planning_state(3).unwrap().cancelling);
        app.request_cancel_all();
        app.cancellation_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.cooperation.cancel_review.is_none());
        assert!(!app.planning_state(1).unwrap().cancelling);
        app.request_cancel_all();
        app.cooperation.cancel_review.as_mut().unwrap().reviewed = true;
        app.cancellation_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        for partner in 1..=3 {
            let state = app.planning_state(partner).unwrap();
            assert!(state.cancelling);
            assert_eq!(
                state.work.as_ref().unwrap().request_id,
                format!("owned-{partner}")
            );
        }
        assert!(
            !app.quit,
            "cancellation waits for terminal events and exact durable releases"
        );
    }

    #[test]
    fn idle_release_preserves_live_slots_and_newer_foreground_draft() {
        let mut app = app();
        for partner in 1..=3 {
            start(&mut app, partner, partner as u64);
        }
        prompt(&mut app, 1, 10);
        prompt(&mut app, 2, 20);
        app.refresh_approvals();
        app.request_lead(6);
        app.open_editor(store::EditorField::Request);
        app.update(Event::Paste("Keep draft 六".into()));
        app.cursor = 5;
        let before = (app.input.clone(), app.cursor, app.revision);
        let project = store::Project {
            tasks: app.tasks.clone(),
            partners: app
                .party
                .iter()
                .filter(|partner| partner.id != 4)
                .cloned()
                .collect(),
            active_id: Some(7),
            usage_bytes: 0,
        };
        app.store_event(store::Event::PartyReleased(Ok(project)));
        assert_eq!(app.lead.id, 6);
        assert_eq!((app.input.clone(), app.cursor), (before.0, before.1));
        assert!(app.revision >= before.2);
        assert_eq!(app.work_rows().len(), 3);
        assert_eq!(app.approval_count(), 2);
        for partner in 1..=3 {
            assert_eq!(
                app.planning_state(partner)
                    .unwrap()
                    .work
                    .as_ref()
                    .unwrap()
                    .request_id,
                format!("owned-{partner}")
            );
        }
        assert!(!app.party.iter().any(|partner| partner.id == 4));
    }

    #[test]
    fn independently_saved_commands_prepare_without_fronting_the_saved_owner() {
        let mut app = app();
        let owner = app.attribution();
        app.command = "[\"echo\",\"first\"]".into();
        app.pending_command = Some((app.command.clone(), owner));
        app.changed_task();
        app.track_pending_command();
        app.request_lead(2);
        app.open_editor(store::EditorField::Request);
        app.update(Event::Paste("Keep this future request".into()));
        let before = (app.input.clone(), app.cursor);
        app.command = "[\"echo\",\"second\"]".into();
        app.pending_command = Some((app.command.clone(), app.attribution()));
        app.changed_task();
        app.track_pending_command();
        let snapshot = app.snapshot("/workspace");
        app.store_event(store::Event::Saved(snapshot));
        assert_eq!(app.lead.id, 2);
        assert_eq!((app.input.clone(), app.cursor), before);
        for partner in 1..=2 {
            let state = app.execution_state(partner).unwrap();
            assert!(state.busy());
            assert_eq!(state.owner.as_ref().unwrap().partner, partner);
        }
        assert_eq!(app.work_rows().iter().filter(|row| row.local).count(), 2);
        assert!(app.work_capacity_reason(3).is_none());
    }

    #[test]
    fn released_current_lead_reconciles_intents_above_committed_revision() {
        use crate::workspace::workflows::{CommandRelay, DraftRelay};

        let mut app = app();
        for partner in 1..=3 {
            start(&mut app, partner, partner as u64);
            app.planning_state_mut(partner).unwrap().thread_id =
                Some(format!("private-session-{partner}"));
        }
        app.request_lead(6);
        app.open_editor(store::EditorField::Request);
        app.update(Event::Paste("Keep partner six's independent draft".into()));
        app.cursor = 8;
        app.request_lead(4);
        let sixth = app.partner_drafts.get(&6).cloned().unwrap();
        app.memory.draft = "Released lead's private memory proposal".into();
        app.workflow.hypothesis_text = "Released lead's private workflow proposal".into();
        for partner in [4, 6] {
            app.workflows.command_relays.push(CommandRelay {
                partner,
                run_id: format!("retained-run-{partner}"),
            });
            app.workflows.draft_relays.push(DraftRelay {
                partner,
                form: 3,
                text: format!("Recipient {partner}"),
                paths: Vec::new(),
                decision: None,
                role_action: None,
            });
        }
        let claims: Vec<_> = (1..=3)
            .map(|partner| app.planning_state(partner).unwrap().work.clone().unwrap())
            .collect();
        let mut committed = app.snapshot("/workspace");
        committed.lead_partner = 1;
        committed.form = 0;
        committed.revision += 1;
        committed.partner_drafts.remove(&4);
        committed.memory_draft = serde_json::Value::Null;
        committed.workflow_draft = serde_json::Value::Null;
        committed
            .workflows
            .command_relays
            .retain(|relay| relay.partner != 4);
        committed
            .workflows
            .draft_relays
            .retain(|relay| relay.partner != 4);
        committed.navigation.form_selection = 0;
        committed.navigation.previous_form = None;
        committed.navigation.overlay = None;
        committed.navigation.editing = false;
        committed.navigation.editor_field = store::EditorField::default();
        let released_revision = committed.revision;
        let partners = app
            .party
            .iter()
            .filter(|partner| partner.id != 4)
            .cloned()
            .collect();
        app.store_event(store::Event::PartyReleased(Ok(store::Project {
            tasks: vec![committed],
            partners,
            active_id: Some(7),
            usage_bytes: 0,
        })));

        let next_save = app.snapshot("/workspace");
        assert_eq!(app.lead.id, 1);
        assert_eq!(app.saved_revision, released_revision);
        assert!(app.revision > released_revision);
        assert_eq!(next_save.revision, app.revision);
        assert!(app.save_requested);
        assert_eq!(app.partner_drafts.get(&6), Some(&sixth));
        assert!(!next_save.partner_drafts.contains_key(&4));
        assert_eq!(next_save.workflows.command_relays.len(), 1);
        assert_eq!(next_save.workflows.command_relays[0].partner, 6);
        assert_eq!(next_save.workflows.draft_relays.len(), 1);
        assert_eq!(next_save.workflows.draft_relays[0].partner, 6);
        assert!(app.memory.draft.is_empty());
        assert!(app.workflow.hypothesis_text.is_empty());
        for claim in claims {
            let state = app.planning_state(claim.owner.partner).unwrap();
            assert_eq!(state.work.as_ref(), Some(&claim));
            assert_eq!(
                state.thread_id.as_deref(),
                Some(format!("private-session-{}", claim.owner.partner).as_str())
            );
        }
        assert_eq!(app.work_rows().len(), 3);
    }

    #[test]
    fn idle_owned_session_retains_backend_worker_until_account_revocation() {
        let mut app = app();
        app.planning.thread_id = Some("backend-owned-resume-session".into());
        let worker =
            providers::Worker::start(std::env::temp_dir(), std::env::temp_dir(), Vec::new())
                .unwrap();
        let mut workers = BTreeMap::from([(app.lead.id, worker)]);
        app.poll_providers(&mut workers);
        assert!(workers.contains_key(&app.lead.id));
        app.authentication_returned();
        app.poll_providers(&mut workers);
        assert!(workers.is_empty());
    }

    #[test]
    fn authentication_and_future_credentials_reach_every_parked_owner() {
        let mut app = app();
        app.set_devin_credential(Some(providers::auth::test_credential("old-account")));
        for partner in 1..=3 {
            start(&mut app, partner, partner as u64);
            app.planning_state_mut(partner).unwrap().thread_id =
                Some(format!("old-account-{partner}"));
        }
        app.authentication_returned();
        for partner in 1..=3 {
            let state = app.planning_state(partner).unwrap();
            assert!(!state.has_resumable_sessions());
            assert!(state.devin_credential.is_none());
            assert!(
                state.transport_pending(),
                "Submitted work still owns its transport."
            );
        }
        app.set_devin_credential(Some(providers::auth::test_credential("replacement")));
        for partner in 1..=3 {
            assert_eq!(
                app.planning_state(partner)
                    .unwrap()
                    .devin_credential
                    .as_ref()
                    .unwrap()
                    .token(),
                "replacement"
            );
        }
        app.set_devin_credential(None);
        for partner in 1..=3 {
            assert!(
                app.planning_state(partner)
                    .unwrap()
                    .devin_credential
                    .is_none()
            );
        }
    }
    #[test]
    fn global_motion_latest_choice_rewrites_older_pending_owner_snapshots() {
        fn submit(app: &mut App, id: i64) -> store::ProviderPreferences {
            assert!(app.cooperation.preference_owners.is_empty());
            let reduced = app.motion_choice.unwrap();
            let state = app.planning_state_mut(id).unwrap();
            state.sync_motion_choice(reduced);
            let Some(store::PreferenceRequest::Save(snapshot)) = state.preference_request().take()
            else {
                panic!("expected actual preference save");
            };
            state.preference_started();
            app.cooperation.preference_owners.push_back(id);
            snapshot
        }
        let mut app = app();
        let original = store::ProviderPreferences {
            provider: providers::Agent::ChatGPT,
            selections: Default::default(),
            reduced_motion: false,
            text_density: store::TextDensity::Compact,
        };
        app.cooperation_preference_event(Ok(Some(original)));
        app.motion_command("motion-off");
        let first = submit(&mut app, 1);
        app.request_lead(2);
        assert_eq!(app.lead.id, 2);
        app.motion_command("motion-on");
        assert!(
            matches!(app.planning.preference_request(), Some(store::PreferenceRequest::Save(saved)) if !saved.reduced_motion)
        );
        app.request_lead(1);
        assert_eq!(app.lead.id, 1);
        app.motion_command("motion-off");
        app.cooperation_preference_event(Ok(Some(first)));
        let last = submit(&mut app, 2);
        assert!(
            last.reduced_motion,
            "B's queued on cannot follow the latest global off"
        );
        assert_eq!(last.provider, providers::Agent::ChatGPT);
        app.cooperation_preference_event(Ok(Some(last.clone())));
        for id in app.planning_ids() {
            let state = app.planning_state_mut(id).unwrap();
            assert!(state.reduced_motion());
            assert!(state.preference_request().is_none());
        }
        let mut restarted = planning::State::new();
        restarted.preference_event(Ok(Some(last)));
        assert!(
            restarted.reduced_motion(),
            "restart observes the latest durable snapshot"
        );
    }

    #[test]
    fn global_density_latest_intent_survives_parked_saves_without_losing_model_or_motion() {
        fn submit(app: &mut App, id: i64) -> store::ProviderPreferences {
            assert!(app.cooperation.preference_owners.is_empty());
            let density = app.density_choice.unwrap();
            let motion = app.motion_choice;
            let state = app.planning_state_mut(id).unwrap();
            state.sync_density_choice(density);
            if let Some(reduced) = motion {
                state.sync_motion_choice(reduced);
            }
            let Some(store::PreferenceRequest::Save(snapshot)) = state.preference_request().take()
            else {
                panic!("expected the actual preference owner snapshot");
            };
            state.preference_started();
            app.cooperation.preference_owners.push_back(id);
            snapshot
        }
        let mut app = app();
        let original = store::ProviderPreferences {
            provider: providers::Agent::ChatGPT,
            selections: Default::default(),
            reduced_motion: false,
            text_density: store::TextDensity::Compact,
        };
        app.cooperation_preference_event(Ok(Some(original)));
        app.density_command("density-comfortable");
        let first = submit(&mut app, 1);
        app.request_lead(2);
        assert_eq!(app.lead.id, 2);
        let model = providers::ModelSelection {
            model: Some("latest/Exact".into()),
            effort: Some("high".into()),
        };
        app.planning.selection = model.clone();
        app.planning.remember_used_selection();
        app.motion_command("motion-off");
        app.density_command("density-compact");
        assert!(matches!(
            app.planning.preference_request(),
            Some(store::PreferenceRequest::Save(saved))
                if saved.text_density == store::TextDensity::Compact
        ));
        app.request_lead(1);
        assert_eq!(app.lead.id, 1);
        app.density_command("density-comfortable");
        app.cooperation_preference_event(Ok(Some(first)));
        let last = submit(&mut app, 2);
        assert_eq!(last.text_density, store::TextDensity::Comfortable);
        assert!(last.reduced_motion);
        assert_eq!(last.provider, providers::Agent::ChatGPT);
        assert_eq!(last.selections[&providers::Agent::ChatGPT], model);
        app.cooperation_preference_event(Ok(Some(last.clone())));
        let settled = submit(&mut app, 1);
        assert_eq!(
            settled, last,
            "the earlier owner settles only the latest shared fields"
        );
        app.cooperation_preference_event(Ok(Some(settled)));
        for id in app.planning_ids() {
            let state = app.planning_state_mut(id).unwrap();
            assert_eq!(state.text_density(), store::TextDensity::Comfortable);
            assert!(state.reduced_motion());
            assert!(state.preference_request().is_none());
        }
        let mut restarted = planning::State::new();
        restarted.preference_event(Ok(Some(last)));
        assert_eq!(restarted.text_density(), store::TextDensity::Comfortable);
        assert_eq!(restarted.selection, model);
        assert!(restarted.reduced_motion());
    }

    #[test]
    fn restored_second_lead_receives_empty_global_preference_hydration() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.planning
            .set_startup_provider(providers::Agent::Codex, false);
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.lead_partner = 2;
        app.store_event(store::Event::Loaded(store::Project {
            tasks: vec![task],
            partners: (1..=2)
                .map(|id| store::Partner {
                    id,
                    ..Default::default()
                })
                .collect(),
            active_id: Some(7),
            usage_bytes: 0,
        }));
        assert_eq!(app.lead.id, 2);
        assert!(
            app.planning.reduced_motion(),
            "unhydrated launch is temporarily static"
        );
        app.cooperation.preference_owners.push_back(1);
        app.cooperation_preference_event(Ok(None));
        assert!(
            !app.planning.reduced_motion(),
            "no persisted row must not become reduced motion"
        );
        assert!(app.motion_enabled());
        for id in app.planning_ids() {
            let state = app.planning_state_mut(id).unwrap();
            assert!(!state.reduced_motion());
            assert!(!state.preferences_loading());
            assert!(state.preference_request().is_none());
        }
    }
}
