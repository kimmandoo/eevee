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
            .any(|state| state.work_error.is_some() || state.preference_error().is_some())
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
                } else if waiting {
                    "waiting"
                } else if state.cancelling {
                    "cancelling"
                } else {
                    "working"
                },
                detail: if state.journal_failed() {
                    state.journal_notice.clone()
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
            writing: self.planning.operation == crate::providers::Operation::Implement
                && !self.planning.agent.is_direct(),
        };
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
            let state = self.planning_state_mut(id).expect("known partner slot");
            state.settle_work();
            if sent == 0
                && !state.work_inflight
                && state.work_error.is_none()
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
        if let Some(state) = self.planning_state_mut(id) {
            state.journal_event(task, result);
            state.settle_work();
        }
        if failed {
            self.notice = format!(
                "Partner {id}: journal storage failed. /history inspects retained evidence; /journal-retry retries storage only. A known-terminal failed write can be explicitly reviewed with /journal-discard."
            );
        }
        self.settle_cooperation();
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
        }
    }

    pub fn refresh_approvals(&mut self) {
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
        self.cooperation.approvals = pending;
    }

    pub fn approval_count(&self) -> usize {
        self.cooperation.approvals.len()
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
        self.refresh_approvals();
        let Some((_, request)) = self.cooperation.approvals.first().cloned() else {
            self.notice = "No pending approvals or questions.".into();
            return;
        };
        let focused = match &request {
            Pending::Provider {
                partner,
                task,
                generation,
                request,
            } => {
                self.request_lead(*partner);
                if self.task_id != *task || !self.planning.focus_prompt(*generation, request) {
                    false
                } else {
                    self.open_overlay(Overlay::Planning);
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
                self.request_lead(*partner);
                if self.execution.focus_approval(*task, *partner, request, run) {
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
        self.cooperation.focused = None;
    }

    pub(super) fn defer_approval(&mut self) {
        self.cooperation.focused = None;
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
            self.notice = "Cancellation waits for ownership release or preference storage. /storage-retry retries storage only.".into();
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
        let foreground_drawer = partner == self.lead.id && self.overlay == Some(Overlay::Planning);
        let Some(state) = self.planning_state_mut(partner) else {
            return false;
        };
        if state.owner().is_some_and(|owner| owner.partner != partner) {
            return false;
        }
        let implemented = matches!(&event, crate::providers::Event::Finished { .. })
            && state.operation == crate::providers::Operation::Implement;
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
        if resolved {
            self.defer_approval();
            self.notice = "The displayed request resolved; /approvals deliberately opens the next pending request.".into();
        }
        self.repo_refresh |= implemented;
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
