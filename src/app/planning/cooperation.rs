use super::*;

impl State {
    pub fn new_partner(&self, owner: store::Attribution, task: i64) -> Self {
        let mut state = Self::new();
        state.task = task;
        state.front = Some(owner);
        let (agent, selection) = self.global_route();
        state.agent = agent;
        let mut selections = self.selections.clone();
        if selections.get(&agent) != Some(selection) {
            selections.insert(agent, selection.clone());
        }
        state.preference_event(Ok(Some(store::ProviderPreferences {
            provider: state.agent,
            selections,
        })));
        state.catalogues = self.catalogues.clone();
        state.devin_credential = self.devin_credential.clone();
        state
    }

    pub fn transport_pending(&self) -> bool {
        self.active || self.awaiting_preview || self.preview.is_some() || self.pending.is_some()
    }

    pub fn owns_work(&self) -> bool {
        self.transport_pending()
            || self.work.is_some()
            || self.work_pending.is_some()
            || self.work_inflight
            || !self.pending_journal.is_empty()
            || !self.journal_inflight.is_empty()
            || self.journal_failed()
            || self.usage_pending()
            || self.unknown_effects()
    }

    pub fn immutable_task(&self) -> Option<&store::Snapshot> {
        self.held_input
            .as_ref()
            .map(|input| &input.task)
            .or(self.committed_input.as_deref())
    }

    /// Idempotent hold for the dispatched instruction's one local record.
    /// The records runtime owns the bytes; this retains only its existing claim.
    pub fn hold_local_record(&mut self, request_id: &str) -> bool {
        if !self.work_acquired
            || self.work_inflight
            || self.work_pending.is_some()
            || self
                .work
                .as_ref()
                .is_none_or(|claim| claim.request_id != request_id)
        {
            return false;
        }
        self.local_record_hold = true;
        true
    }

    pub fn release_local_record(&mut self, request_id: &str) {
        if self
            .work
            .as_ref()
            .is_some_and(|claim| claim.request_id == request_id)
        {
            self.local_record_hold = false;
            self.settle_work();
        }
    }

    pub fn stage_work(&mut self, claim: store::WorkClaim) {
        let Some(Request::Preview(mut input)) = self.pending.take() else {
            return;
        };
        input.work = Some(claim.clone());
        self.submitted_instruction = input
            .instruction_override
            .as_ref()
            .map(|replacement| replacement.replacement.as_str().into());
        self.usage = usage::State::default();
        self.local_record_hold = false;
        self.committed_input = None;
        self.work_pending = Some(store::WorkRequest::Acquire {
            claim: claim.clone(),
            task: Box::new(input.task.clone()),
        });
        self.work = Some(claim);
        self.work_acquired = false;
        self.receipt_claim = self.work.clone();
        self.work_error = None;
        self.claim_cancelled = false;
        self.held_input = Some(input);
        self.notice = "Reserving this partner's work before provider preparation…".into();
    }

    pub fn work_event(
        &mut self,
        request: store::WorkRequest,
        result: Result<Vec<store::WorkClaim>, String>,
    ) {
        self.work_inflight = false;
        match result {
            Ok(_) => match request {
                store::WorkRequest::Acquire { claim, task }
                    if self.work.as_ref() == Some(&claim) =>
                {
                    self.committed_input = Some(task);
                    self.work_acquired = true;
                    self.work_error = None;
                    if self.claim_cancelled {
                        self.awaiting_preview = false;
                        self.held_input = None;
                        self.work_pending = Some(store::WorkRequest::Release(claim));
                    } else if let Some(input) = self.held_input.take() {
                        self.pending = Some(Request::Preview(input));
                    }
                }
                store::WorkRequest::Release(claim) if self.work.as_ref() == Some(&claim) => {
                    self.work = None;
                    self.work_acquired = false;
                    self.work_error = None;
                    self.committed_input = None;
                    self.claim_cancelled = false;
                }
                _ => {}
            },
            Err(error) => match request {
                store::WorkRequest::Acquire { .. } => {
                    self.work = None;
                    self.work_acquired = false;
                    self.held_input = None;
                    self.awaiting_preview = false;
                    self.send_after_prepare = false;
                    self.failed_request = true;
                    self.notice =
                        format!("Work reservation refused: {error}. Your request draft is kept.");
                }
                release @ store::WorkRequest::Release(_) => {
                    self.work_pending = Some(release);
                    self.work_error = Some(error.clone());
                    self.notice = format!(
                        "Work ownership release failed: {error}. /storage-retry retries settlement only."
                    );
                }
            },
        }
    }

    pub fn settle_work(&mut self) {
        if !self.transport_pending()
            && self.pending_journal.is_empty()
            && self.journal_inflight.is_empty()
            && !self.journal_failed()
            && !self.usage_pending()
            && !self.local_record_hold
            && !self.work_inflight
            && self.work_pending.is_none()
            && let Some(claim) = self.work.clone()
        {
            self.work_pending = Some(store::WorkRequest::Release(claim));
        }
    }

    pub fn pending_identities(&self) -> Vec<(u64, Option<serde_json::Value>, u64)> {
        let Some(generation) = self.generation else {
            return Vec::new();
        };
        let mut requests = Vec::new();
        if let Some(preview) = &self.preview
            && !self.send_after_prepare
            && !self.inspect_models
        {
            requests.push((generation, None, preview.arrival));
        }
        if let Some(request) = &self.request {
            requests.push((
                generation,
                Some(request.request_id.clone()),
                request.arrival,
            ));
        }
        requests.extend(self.queued_prompts.iter().map(|request| {
            (
                generation,
                Some(request.request_id.clone()),
                request.arrival,
            )
        }));
        requests
    }

    pub fn focus_prompt(&self, generation: u64, identity: &Option<serde_json::Value>) -> bool {
        if self.generation != Some(generation) {
            return false;
        }
        if identity.is_none() {
            return self.preview.is_some();
        }
        self.request
            .as_ref()
            .is_some_and(|request| Some(&request.request_id) == identity.as_ref())
    }

    pub(super) fn install_prompt(&mut self, request: Prompt) {
        self.saved_scroll = self.scroll;
        self.answers = match &request.kind {
            PromptKind::Questions(questions) => {
                questions.iter().map(|_| Answer::default()).collect()
            }
            _ => Vec::new(),
        };
        self.request = Some(request);
        self.question = 0;
        self.option = 0;
        self.editing = false;
        self.submit_review = false;
        self.review_start();
    }

    pub fn cancel_owned(&mut self) -> bool {
        self.picker = None;
        self.paths_editor = None;
        self.custom_model = None;
        self.selection_change = None;
        self.switch_to = None;
        self.quit_review = false;
        self.cancel()
    }

    pub(super) fn promote_prompt(&mut self) {
        if self.active
            && !self.cancelling
            && self.request.is_none()
            && let Some(request) = self.queued_prompts.pop_front()
        {
            self.install_prompt(request);
        }
    }

    pub fn settle_background(&mut self, large_enough: bool) -> bool {
        let mut changed = false;
        if large_enough
            && self.send_after_prepare
            && let Some(preview) = self.preview.as_ref()
        {
            if let Some(blocker) = preview.blockers.first() {
                self.failed_request = true;
                self.send_after_prepare = false;
                self.notice = format!("Request blocked: {blocker} /approvals Details");
                changed = true;
            } else if preview.requires_connection_review {
                self.send_after_prepare = false;
                changed = true;
            } else {
                changed |= self.dispatch_prepared();
            }
        }
        self.promote_prompt();
        self.settle_work();
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> (State, store::Snapshot, store::Attribution) {
        let owner = store::Attribution::of(&store::Partner::default());
        let state = State::new().new_partner(owner.clone(), 7);
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.goal = "Immutable task facts".into();
        task.partner_drafts.insert(
            owner.partner,
            store::PartnerDrafts {
                drafts: vec![store::Draft {
                    field: store::EditorField::Request,
                    text: "Submitted A".into(),
                    cursor: 11,
                }],
                ..Default::default()
            },
        );
        (state, task, owner)
    }

    fn claim(owner: store::Attribution, id: &str) -> store::WorkClaim {
        store::WorkClaim {
            task_id: 7,
            owner,
            request_id: id.into(),
            writing: false,
        }
    }

    #[test]
    fn cancellation_before_acquire_ack_releases_exact_claim_without_provider_preparation() {
        let (mut state, task, owner) = state();
        state.command("new", true, &task, &[], None);
        let owned = claim(owner, "request-a");
        state.stage_work(owned.clone());
        let acquire = state.work_pending.take().unwrap();
        state.work_inflight = true;
        assert!(state.cancel_owned());
        assert!(state.pending.is_none());
        state.work_event(acquire, Ok(vec![owned.clone()]));
        assert!(
            state.pending.is_none(),
            "a late acquire must never prepare a cancelled provider request"
        );
        let release = state.work_pending.take().unwrap();
        assert!(matches!(&release, store::WorkRequest::Release(claim) if claim == &owned));
        assert_eq!(state.work.as_ref(), Some(&owned));
        state.work_event(release, Ok(Vec::new()));
        assert!(state.work.is_none() && !state.owns_work());
        assert!(!state.focus_prompt(7, &None));
    }

    #[test]
    fn current_receipt_and_observed_model_never_inherit_previous_request_metadata() {
        let (mut state, task, owner) = state();
        let old = claim(owner.clone(), "old-request");
        state.receipt_claim = Some(old.clone());
        state.receipt = vec!["old exact payload receipt".into()];
        state.observed_model = Some("observed-old-model".into());
        state.submitted_request = "Old submitted request".into();
        state.command("new", true, &task, &[], None);
        assert!(state.receipt.is_empty());
        assert!(state.observed_model.is_none());
        let current = claim(owner, "new-request");
        state.stage_work(current.clone());
        assert_eq!(state.receipt_claim.as_ref(), Some(&current));
        assert!(state.receipt.is_empty() && state.observed_model.is_none());
        assert_eq!(
            state.immutable_task().unwrap().request_text(),
            "Submitted A"
        );
        let acquire = state.work_pending.take().unwrap();
        state.work_event(acquire, Ok(vec![current]));
        assert!(state.receipt.is_empty() && state.observed_model.is_none());
    }

    #[test]
    fn release_waits_for_every_pending_inflight_and_failed_journal_operation() {
        let (mut state, task, owner) = state();
        state.command("new", true, &task, &[], None);
        state.stage_work(claim(owner, "journal-owned"));
        let acquire = state.work_pending.take().unwrap();
        let owned = state.work.clone().unwrap();
        state.work_event(acquire, Ok(vec![owned.clone()]));
        state.pending.take();
        state.awaiting_preview = false;
        state.pending_journal.push_back(store::JournalRequest {
            task_id: 7,
            operation: store::JournalOperation::List,
        });
        state.settle_work();
        assert!(state.work_pending.is_none());
        state.journal_started();
        state.settle_work();
        assert!(state.work_pending.is_none());
        state.journal_event(7, Err("disk unavailable".into()));
        state.settle_work();
        assert!(state.work_pending.is_none() && state.journal_failed());
        state.retry_journal();
        state.journal_started();
        state.journal_event(7, Ok(store::JournalResponse::Records(Vec::new())));
        state.settle_work();
        assert!(
            matches!(state.work_pending.as_ref(), Some(store::WorkRequest::Release(claim)) if claim == &owned)
        );
    }

    #[test]
    fn local_instruction_record_holds_exact_claim_until_commit_or_reviewed_discard() {
        let (mut state, _, owner) = state();
        let owned = claim(owner, "actual-dispatched-request");
        state.work = Some(owned.clone());
        assert!(
            !state.hold_local_record(&owned.request_id),
            "Admission alone is not acquired ownership"
        );
        state.work_acquired = true;
        assert!(!state.hold_local_record("different-request"));
        assert!(state.hold_local_record(&owned.request_id));
        assert!(
            state.hold_local_record(&owned.request_id),
            "Repeated observed dispatch is idempotent"
        );
        state.settle_work();
        assert!(state.work_pending.is_none() && state.owns_work());
        state.release_local_record("different-request");
        assert!(state.work_pending.is_none());
        state.release_local_record(&owned.request_id);
        assert!(
            matches!(state.work_pending.as_ref(), Some(store::WorkRequest::Release(claim)) if claim == &owned)
        );
        assert!(
            !state.hold_local_record(&owned.request_id),
            "Cannot reacquire a record hold after release was queued"
        );
    }
}
