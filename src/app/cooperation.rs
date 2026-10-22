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
    approvals: Vec<(u64, Pending)>,
    focused: Option<Pending>,
    /// Identities already presented or explicitly deferred, not another queue.
    presented_files: Vec<Pending>,
    queued_response: Option<Pending>,
    preference_owners: VecDeque<i64>,
    journal_owners: VecDeque<i64>,
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
            self.cooperation.parked.get(&partner)
        }
    }

    pub(super) fn planning_state_mut(&mut self, partner: i64) -> Option<&mut planning::State> {
        if partner == self.lead.id {
            Some(&mut self.planning)
        } else {
            self.cooperation.parked.get_mut(&partner)
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
        previous_execution.confirmation_visible = false;
        previous_execution.affirmative = false;
        self.execution.confirmation_visible = false;
        self.execution.affirmative = false;
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
        std::iter::once(&self.planning)
            .chain(self.cooperation.parked.values())
            .any(|state| state.busy() || state.preferences_pending())
    }

    pub(super) fn quit_requires_work_review(&self) -> bool {
        self.pending_request.is_some()
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
    }

    pub fn planning_storage_failed(&self) -> bool {
        std::iter::once(&self.planning)
            .chain(self.cooperation.parked.values())
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
        (providers + local + saving >= 3).then(|| "Three partners are already working on this task; wait or cancel one. Your draft is kept.".into())
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
        for id in ids {
            let preference_free = self.cooperation.preference_owners.is_empty();
            let move_receipt_pending = self.move_dispatch_pending(id);
            let state = self.planning_state_mut(id).expect("known partner slot");
            state.settle_work();
            if sent == 0
                && !state.work_inflight
                && state.work_error.is_none()
                && !(move_receipt_pending
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
            }
        }
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
        changed |= self.reconcile_file_review();
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
        let saved = result.as_ref().ok().and_then(|saved| saved.clone());
        let failed = result.as_ref().err().cloned();
        if let Some(state) = self.planning_state_mut(id) {
            state.preference_event(result);
        }
        if let Some(error) = failed {
            self.notice = format!(
                "Provider preference storage failed for partner #{id}: {error}. /storage-retry retries storage only; the local selection and drafts are kept."
            );
        }
        if let Some(saved) = saved {
            for partner in self.planning_ids() {
                if partner != id
                    && let Some(state) = self.planning_state_mut(partner)
                {
                    state.merge_preferences(&saved);
                }
            }
        }
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
        if allowed && !self.planning_any_busy() && !self.execution_any_pending() {
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
    }

    pub fn refresh_approvals(&mut self) {
        self.collect_approvals();
        self.reconcile_file_review();
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
            .presented_files
            .retain(|shown| pending.iter().any(|(_, request)| request == shown));
        self.cooperation.approvals = pending;
    }

    /// Reconcile after input and worker settlement, never before dispatching a key.
    pub(super) fn reconcile_file_review(&mut self) -> bool {
        if !self.approval_shortcut_available()
            || (!self.composer_is_request() && self.focus != Pane::Workspace)
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
            || !self.planning.request.as_ref().is_some_and(|request| {
                matches!(
                    request.kind,
                    crate::providers::PromptKind::Permission { file: Some(_), .. }
                )
            })
            || self
                .planning
                .owner()
                .is_none_or(|owner| owner.partner != self.lead.id)
        {
            return false;
        }
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
                    && self.planning.matches(*generation, *task)
                    && self.planning.focus_prompt(*generation, request)
                    && !self.cooperation.presented_files.contains(pending) =>
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
        self.cooperation.presented_files.push(request);
        self.planning.auto_file_review = true;
        true
    }

    pub fn approval_count(&self) -> usize {
        self.cooperation.approvals.len()
    }

    pub fn own_approval_pending(&self) -> bool {
        self.planning.request.is_some()
    }

    pub fn partner_approval_pending(&self, partner: i64) -> bool {
        self.planning_state(partner)
            .is_some_and(|state| state.request.is_some())
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
                    && request.is_some()
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
                    self.planning.auto_file_review = false;
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
        self.planning.auto_file_review = false;
        self.cooperation.focused = None;
    }

    pub(super) fn defer_approval(&mut self) {
        if let Some(request) = self.cooperation.focused.take()
            && matches!(
                self.planning.request.as_ref().map(|request| &request.kind),
                Some(crate::providers::PromptKind::Permission { file: Some(_), .. })
            )
            && !self.cooperation.presented_files.contains(&request)
        {
            self.cooperation.presented_files.push(request);
        }
        self.planning.auto_file_review = false;
        self.planning.invalidate_review();
        self.close_overlay();
        self.notice =
            "Request deferred, not answered. /approvals revisits pending requests.".into();
    }

    pub fn cancel_all_summary(&self) -> String {
        let mut lines = vec!["Owned work on this task:".into()];
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
        let content_preview = partner == self.lead.id
            && matches!(&event,
            crate::providers::Event::Preview(preview)
                if self.planning.accepts_preview(preview)
                    && !self.planning.inspect_models && !self.planning.inspect_sessions);
        if content_preview
            && let crate::providers::Event::Preview(preview) = &event
            && let Err(error) = self.sources.set_wire_request(&preview.payload)
        {
            self.sources.status = format!("Exact request receipt unavailable: {error}");
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

    fn start(app: &mut App, partner: i64, arrival: u64) {
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
            arrival,
            generation: 7,
            task_id: 7,
            thread_id: None,
            destination: None,
            requires_connection_review: false,
            trust: Vec::new(),
            payload: format!("payload-{partner}"),
            blockers: Vec::new(),
        }));
        assert!(state.dispatch_prepared());
        state.pending.take();
        if partner == app.lead.id {
            app.planning = state;
        } else {
            app.cooperation.parked.insert(partner, state);
        }
    }

    fn prompt(app: &mut App, partner: i64, arrival: u64) {
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
                    kind: providers::PromptKind::Approval {
                        command: format!("command-{partner}"),
                        cwd: "/workspace".into(),
                        reason: "Exact owned request".into(),
                        allow_once: true,
                        deny_decision: "decline".into(),
                    },
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
    fn automatic_file_review_preserves_live_future_draft_and_defers_once() {
        let mut app = app();
        start(&mut app, 1, 1);
        app.open_editor(store::EditorField::Request);
        app.input = "Future 漢字 draft".into();
        app.cursor = "Future ".len();
        let draft = (app.input.clone(), app.cursor);
        file_prompt(&mut app, 1, 2);
        assert_eq!(app.overlay, Some(Overlay::Planning));
        assert!(app.planning.auto_file_review);
        assert_eq!((app.input.clone(), app.cursor), draft);
        for code in [
            KeyCode::Char('a'),
            KeyCode::Char('y'),
            KeyCode::Char('n'),
            KeyCode::Left,
            KeyCode::Right,
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
        assert!(app.planning.auto_file_review && !app.planning.review_details);
        assert!(app.planning.pending.is_none() && app.pending_request.is_none());
        assert_eq!((app.input.clone(), app.cursor), draft);
        app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(app.overlay.is_none() && !app.planning.auto_file_review);
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
            !app.planning.auto_file_review,
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
            assert!(app.planning.auto_file_review);
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
            assert!(!app.planning.auto_file_review);
            app.close_overlay();
            assert!(app.settle_planning());
            assert!(app.planning.auto_file_review);
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
        assert!(app.overlay.is_none() && !app.planning.auto_file_review);
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
                3 => app.pending_request = Some((app.revision, false)),
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
            assert!(app.overlay.is_none() && !app.planning.auto_file_review);
            app.transitioning = false;
            app.reset_after_save = false;
            app.quit_after_save = false;
            app.pending_request = None;
            app.cooperation.cancel_review = None;
            app.composer_menu = None;
            app.width = 120;
            assert!(app.settle_planning());
            assert!(app.planning.auto_file_review && app.planning.pending.is_none());
        }
        let mut app = app();
        start(&mut app, 1, 1);
        app.open_editor(store::EditorField::Request);
        prompt(&mut app, 1, 10);
        app.planning.request.as_mut().unwrap().kind =
            providers::PromptKind::Questions(vec![providers::Question {
                id: "scope".into(),
                header: "Scope".into(),
                question: "Which file?".into(),
                options: Vec::new(),
                other: true,
                secret: false,
            }]);
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
        assert!(app.planning.auto_file_review && app.planning.pending.is_none());
    }

    #[test]
    fn consecutive_owned_file_reviews_reconcile_without_opening_foreign_or_generic_work() {
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
            app.planning.auto_file_review,
            "stale closure cannot affect live review"
        );
        close_file_prompt(&mut app, 1, 20);
        assert_eq!(app.overlay, Some(Overlay::Planning));
        assert!(app.planning.auto_file_review);
        assert_eq!(
            app.planning.request.as_ref().unwrap().request_id,
            json!("file-30")
        );
        assert!(app.planning.pending.is_none());
        prompt(&mut app, 1, 40);
        close_file_prompt(&mut app, 1, 30);
        assert!(
            app.overlay.is_none(),
            "generic approvals still require deliberate opening"
        );
        assert!(!app.planning.auto_file_review);
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
                assert!(app.overlay.is_none(), "arrival must not interrupt typing");
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
        prompt(&mut app, 1, 20);
        app.planning.request.as_mut().unwrap().kind =
            providers::PromptKind::Questions(vec![providers::Question {
                id: "scope".into(),
                header: "Scope".into(),
                question: "Which exact file?".into(),
                options: Vec::new(),
                other: true,
                secret: false,
            }]);
        app.planning.answers = vec![planning::Answer::default()];

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
            prompt(&mut app, 1, 2);
            app.editing = true;
            app.editor_field = field;
            app.input = "literal draft".into();
            app.cursor = 4;
            app.update(shortcut.clone());
            assert!(app.overlay.is_none());
            assert_eq!((app.input.as_str(), app.cursor), ("literal draft", 4));
            assert!(app.planning.pending.is_none());
        }
        for overlay in [Overlay::Help, Overlay::Sources, Overlay::Planning] {
            let mut app = app();
            start(&mut app, 1, 1);
            prompt(&mut app, 1, 2);
            app.open_overlay(overlay);
            app.update(shortcut.clone());
            assert_eq!(app.overlay, Some(overlay));
            assert!(app.planning.pending.is_none());
        }
        let mut app = app();
        start(&mut app, 1, 1);
        prompt(&mut app, 1, 2);
        app.open_commands();
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
            prompt(&mut app, 1, 10);
            prompt(&mut app, 2, 20);
            app.open_editor(store::EditorField::Request);
            app.input = "future draft".into();
            app.cursor = 3;
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
            i18n::{Language, Message},
        };
        use ratatui::{Terminal, backend::TestBackend};

        for tag in ["en", "ko"] {
            let language = Language::parse(tag).unwrap();
            let config = Config {
                language,
                ascii: true,
                no_animation: true,
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
                    assert!(app.planning.auto_file_review);
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
                    assert!(
                        screen.contains(
                            language.text(Message::ProvidersPlanningFileAutoReviewActions)
                        ),
                        "{screen}"
                    );
                    assert!(screen.contains("src/new.rs"), "{screen}");
                    assert_eq!((app.input.clone(), app.cursor), draft);
                    for code in [KeyCode::Char('a'), KeyCode::Char('y'), KeyCode::Char('n')] {
                        app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
                    }
                    app.update(Event::Paste("a\ny\nn\n".into()));
                    assert!(app.planning.auto_file_review && app.planning.pending.is_none());
                    app.update(Event::Key(KeyEvent::new(
                        KeyCode::Enter,
                        KeyModifiers::NONE,
                    )));
                    assert!(!app.planning.auto_file_review && !app.planning.review_details);
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
                        !settled.contains(
                            language.text(Message::ProvidersPlanningFileAutoReviewActions)
                        )
                    );
                    assert!(
                        !settled.contains(language.text(Message::CoreWaitingForUser)),
                        "{settled}"
                    );
                }
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
        assert!(
            app.work_capacity_reason(4)
                .unwrap()
                .contains("Three partners")
        );
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
}
