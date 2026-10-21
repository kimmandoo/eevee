use super::State;
use crate::store;
use std::collections::VecDeque;

const GAP_MARKER: &str = "\nUnverified provider coverage gap:";

#[derive(Default)]
pub(crate) struct Recovery {
    observations: VecDeque<Observation>,
    terminal: Option<(store::WorkClaim, u64)>,
    revision: u64,
    // Memory-only coverage gaps, not saved provider records or local evidence.
    gaps: VecDeque<Gap>,
    omitted_gap_observations: usize,
}

struct Observation {
    claim: store::WorkClaim,
    record: store::ProviderRecord,
    error: Option<String>,
    phase: Phase,
    ambiguous: bool,
}

#[derive(PartialEq, Eq)]
enum Phase {
    Queued,
    Inflight,
    Failed,
}

impl Observation {
    fn matches(&self, request: &store::JournalRequest) -> bool {
        request.task_id == self.claim.task_id
            && matches!(&request.operation, store::JournalOperation::Put(record) if record.as_ref() == &self.record)
    }
}

struct Gap {
    claim: store::WorkClaim,
    ids: Vec<String>,
    errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DiscardSnapshot {
    pub claim: store::WorkClaim,
    revision: u64,
    failed: Vec<(store::ProviderRecord, String)>,
}

impl DiscardSnapshot {
    pub fn ids(&self) -> Vec<String> {
        self.failed
            .iter()
            .map(|(record, _)| record.id.clone())
            .collect()
    }

    pub fn error(&self) -> String {
        self.failed
            .iter()
            .enumerate()
            .map(|(index, (record, error))| {
                format!(
                    "Write {} / {}: {}",
                    index + 1,
                    record.id,
                    crate::workspace::handoff::sanitize(error)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn matches_ids(&self, ids: &[String]) -> bool {
        ids.len() == self.failed.len()
            && ids
                .iter()
                .zip(&self.failed)
                .all(|(id, (record, _))| id == &record.id)
    }
}

// Err(String) alone does not prove that an attempted version was not saved.
// These errors occur before insertion, or abort the transaction before commit.
// A committed prior version stays untouched; an ambiguous COMMIT is refused.
fn definitely_unsaved(error: &str) -> bool {
    [
        "Provider journal reached 256 records for this task;",
        "Local storage quota exceeded (live SQLite + retained output + committed reservations);",
        "Provider evidence was not saved:",
    ]
    .iter()
    .any(|prefix| error.starts_with(prefix))
}

impl State {
    pub fn journal_started(&mut self) {
        if let Some(request) = self.pending_journal.pop_front() {
            self.journal_recovery.revision += 1;
            if let Some(observation) =
                self.journal_recovery
                    .observations
                    .iter_mut()
                    .find(|observation| {
                        observation.phase == Phase::Queued && observation.matches(&request)
                    })
            {
                observation.phase = Phase::Inflight;
            }
            self.journal_inflight.push_back(request);
        }
    }

    /// Called only for the generation-correlated provider Finished event.
    pub(super) fn journal_terminal(&mut self, generation: u64, task: i64, definite: bool) {
        self.journal_recovery.revision += 1;
        self.journal_recovery.terminal = self
            .work
            .as_ref()
            .filter(|claim| {
                definite
                    && self.work_acquired
                    && claim.task_id == task
                    && self.generation == Some(generation)
            })
            .map(|claim| (claim.clone(), generation));
    }

    pub fn journal_event(&mut self, task_id: i64, result: Result<store::JournalResponse, String>) {
        self.journal_recovery.revision += 1;
        let Some(request) = self.journal_inflight.pop_front() else {
            self.journal_notice = "Unsolicited journal acknowledgement ignored.".into();
            self.refresh_journal_gap_notice();
            return;
        };
        let correlated = request.task_id == task_id
            && match (&request.operation, &result) {
                (
                    store::JournalOperation::Put(expected),
                    Ok(store::JournalResponse::Saved(record)),
                ) => {
                    expected.id == record.id
                        && expected.agent == record.agent
                        && expected.generation == record.generation
                        && expected.thread_id == record.thread_id
                        && expected.turn_id == record.turn_id
                        && expected.item_id == record.item_id
                        && expected.complete == record.complete
                        && expected.partner == record.partner
                }
                (store::JournalOperation::List, Ok(store::JournalResponse::Records(_)))
                | (store::JournalOperation::Purge, Ok(store::JournalResponse::Purged))
                | (_, Err(_)) => true,
                _ => false,
            };
        let mut observation = self
            .journal_recovery
            .observations
            .iter()
            .position(|observation| {
                observation.phase == Phase::Inflight && observation.matches(&request)
            })
            .and_then(|index| self.journal_recovery.observations.remove(index));
        if !correlated {
            if let Some(mut observation) = observation.take() {
                observation.ambiguous = true;
                observation.phase = Phase::Failed;
                self.journal_recovery.observations.push_back(observation);
            }
            self.failed_journal.push_back(request);
            self.journal_notice = "Uncorrelated journal acknowledgement refused; retained operation requires storage-only retry.".into();
            self.refresh_journal_gap_notice();
            return;
        }
        match result {
            Ok(store::JournalResponse::Saved(_)) => {
                self.journal_notice =
                    "Provider-origin journal committed; not local execution evidence.".into();
            }
            Ok(store::JournalResponse::Records(records)) if task_id == self.task => {
                self.journal_records = records;
                self.journal_selected = self
                    .journal_selected
                    .min(self.journal_records.len().saturating_sub(1));
                self.scroll = 0;
                self.scroll_max = 0;
                self.journal_notice =
                    "Retained provider observations, with actual origin and coverage.".into();
            }
            Ok(store::JournalResponse::Purged) if task_id == self.task => {
                self.journal_records.clear();
                self.journal_selected = 0;
                self.scroll = 0;
                self.scroll_max = 0;
                // Purge is not an acknowledgement of failed in-memory writes.
                self.usage_purged(task_id);
                self.journal_notice = "This task's local provider observations purged; provider history and exports remain.".into();
            }
            Ok(_) => {}
            Err(error) => {
                if let Some(mut observation) = observation.take() {
                    observation.error = Some(error.clone());
                    observation.phase = Phase::Failed;
                    self.journal_recovery.observations.push_back(observation);
                }
                self.failed_journal.push_back(request);
                self.journal_notice = format!(
                    "Provider journal save/read failed: {}. r retries storage only; never replays a turn. /journal-discard reviews only definitively unsaved observations after provider termination.",
                    crate::workspace::handoff::sanitize(&error)
                );
            }
        }
        self.refresh_journal_gap_notice();
    }

    pub fn journal_failed(&self) -> bool {
        !self.failed_journal.is_empty()
    }

    pub fn retry_journal(&mut self) {
        self.journal_recovery.revision += 1;
        for observation in &mut self.journal_recovery.observations {
            if observation.phase == Phase::Failed {
                observation.phase = Phase::Queued;
            }
        }
        self.pending_journal.extend(self.failed_journal.drain(..));
        self.journal_notice =
            "Retrying retained journal operation only; no provider request.".into();
        self.refresh_journal_gap_notice();
    }

    fn validate_journal_discard(&self) -> Result<(&store::WorkClaim, u64), String> {
        if self.transport_pending()
            || self.outcome_unknown()
            || self.request.is_some()
            || !self.queued_prompts.is_empty()
            || self.held_input.is_some()
        {
            return Err(
                "Provider work must be definitively terminal, not running or outcome unknown."
                    .into(),
            );
        }
        if !self.pending_journal.is_empty()
            || !self.journal_inflight.is_empty()
            || self.work_inflight
            || self.work_pending.is_some()
        {
            return Err(
                "Wait for journal operations and ownership acquire/release acknowledgements."
                    .into(),
            );
        }
        let claim = self
            .work
            .as_ref()
            .filter(|claim| self.work_acquired && claim.task_id == self.task)
            .ok_or("No exact acquired work claim owns these failures.")?;
        let Some((terminal, generation)) = &self.journal_recovery.terminal else {
            return Err(
                "No definitive provider terminal event was observed for this request.".into(),
            );
        };
        if terminal != claim {
            return Err("The terminal provider identity no longer matches the work claim.".into());
        }
        if self.failed_journal.is_empty() {
            return Err("No failed unsaved provider observations to acknowledge.".into());
        }
        if self.journal_recovery.observations.len() != self.failed_journal.len() {
            return Err(
                "The failed observation attempts have no exact request-owned provenance.".into(),
            );
        }
        for (request, observation) in self
            .failed_journal
            .iter()
            .zip(&self.journal_recovery.observations)
        {
            let store::JournalOperation::Put(record) = &request.operation else {
                return Err("Failed journal reads or purges require storage retry; they cannot be discarded.".into());
            };
            let error = observation
                .error
                .as_ref()
                .ok_or("The failed observation has no definite storage error.")?;
            if request.task_id != claim.task_id
                || observation.claim != *claim
                || observation.phase != Phase::Failed
                || observation.record != **record
                || record.partner.as_ref() != Some(&claim.owner)
                || record.generation != *generation
                || observation.ambiguous
                || !definitely_unsaved(error)
            {
                return Err("Only exact, definitely unsaved failed Put attempts of this terminal request can be acknowledged; ambiguous observations are retained.".into());
            }
        }
        Ok((claim, *generation))
    }

    pub fn journal_discard_reason(&self) -> Option<String> {
        self.validate_journal_discard().err()
    }

    pub(crate) fn journal_discard_snapshot(&self) -> Result<DiscardSnapshot, String> {
        let (claim, _) = self.validate_journal_discard()?;
        let failed = self
            .journal_recovery
            .observations
            .iter()
            .map(|observation| {
                (
                    observation.record.clone(),
                    observation.error.clone().expect("validated storage error"),
                )
            })
            .collect();
        Ok(DiscardSnapshot {
            claim: claim.clone(),
            revision: self.journal_recovery.revision,
            failed,
        })
    }

    pub(crate) fn acknowledge_journal_discard(
        &mut self,
        snapshot: &DiscardSnapshot,
    ) -> Result<(), String> {
        let (claim, _) = self.validate_journal_discard()?;
        let exact_failures = self.failed_journal.len() == snapshot.failed.len()
            && self
                .journal_recovery
                .observations
                .iter()
                .zip(&snapshot.failed)
                .all(|(observation, (record, error))| {
                    &observation.record == record && observation.error.as_ref() == Some(error)
                });
        if claim != &snapshot.claim
            || self.journal_recovery.revision != snapshot.revision
            || !exact_failures
        {
            return Err("Journal recovery identity, failures or storage errors changed; review the current failures again.".into());
        }
        let ids = snapshot.ids();
        // All failures were compared byte-for-byte above. No saved record or
        // database operation is touched, and no provider request is created.
        self.failed_journal.clear();
        self.journal_recovery.observations.clear();
        if self.journal_recovery.gaps.len() == 16
            && let Some(oldest) = self.journal_recovery.gaps.pop_front()
        {
            self.journal_recovery.omitted_gap_observations = self
                .journal_recovery
                .omitted_gap_observations
                .saturating_add(oldest.ids.len());
        }
        self.journal_recovery.gaps.push_back(Gap {
            claim: snapshot.claim.clone(),
            ids,
            errors: snapshot
                .failed
                .iter()
                .map(|(_, error)| error.clone())
                .collect(),
        });
        self.journal_recovery.revision += 1;
        self.journal_notice = "Explicitly acknowledged unsaved provider observations only; committed observations remain. No retry, purge or provider replay.".into();
        self.refresh_journal_gap_notice();
        self.settle_work();
        Ok(())
    }

    /// Coverage loss takes precedence over ordinary storage acknowledgements.
    pub fn journal_gap_notice(&self) -> Option<&str> {
        self.journal_notice
            .find(GAP_MARKER)
            .map(|index| &self.journal_notice[index + 1..])
    }

    pub(super) fn refresh_journal_gap_notice(&mut self) {
        // Replace our own suffix, preserving the current storage status.
        if let Some(index) = self.journal_notice.find(GAP_MARKER) {
            self.journal_notice.truncate(index);
        }
        let mut gaps = self
            .journal_recovery
            .gaps
            .iter()
            .filter(|gap| gap.claim.task_id == self.task);
        let count: usize = gaps.clone().map(|gap| gap.ids.len()).sum();
        if let Some(last) = gaps.next_back() {
            self.journal_notice.push_str(&format!("{GAP_MARKER} {count} write attempts not saved; not verification.\nLatest owner {}, request {}. Storage error: {}. Not successful persistence.",
                last.claim.owner.name, last.claim.request_id,
                crate::workspace::handoff::sanitize(last.errors.last().map_or("unknown", String::as_str))));
        }
        if self.journal_recovery.omitted_gap_observations > 0 {
            if count == 0 {
                self.journal_notice.push_str(GAP_MARKER);
            }
            self.journal_notice.push_str(&format!(" Earlier recovery gaps across tasks: {} unsaved observation(s); older exact details omitted, never verified or persisted.",
                self.journal_recovery.omitted_gap_observations));
        }
    }

    pub(super) fn queue_record(&mut self, task_id: i64, mut record: store::ProviderRecord) {
        record.partner = self
            .work
            .as_ref()
            .map(|claim| claim.owner.clone())
            .or_else(|| self.owner().cloned());
        if self.pending_journal.len() + self.journal_inflight.len() + self.failed_journal.len()
            >= 128
        {
            self.journal_notice = "Provider journal queue limit reached; further observations cannot substantiate retained evidence.".into();
            self.refresh_journal_gap_notice();
            return;
        }
        self.journal_recovery.revision += 1;
        if let Some(claim) = self
            .work
            .as_ref()
            .filter(|claim| self.work_acquired && claim.task_id == task_id)
        {
            let mut ambiguous = false;
            for existing in &mut self.journal_recovery.observations {
                if existing.record.id == record.id
                    && (existing.claim != *claim
                        || existing.record.agent != record.agent
                        || existing.record.generation != record.generation
                        || existing.record.thread_id != record.thread_id
                        || existing.record.turn_id != record.turn_id
                        || existing.record.item_id != record.item_id
                        || existing.record.partner != record.partner)
                {
                    existing.ambiguous = true;
                    ambiguous = true;
                }
            }
            self.journal_recovery.observations.push_back(Observation {
                claim: claim.clone(),
                record: record.clone(),
                error: None,
                phase: Phase::Queued,
                ambiguous,
            });
        }
        self.pending_journal.push_back(store::JournalRequest {
            task_id,
            operation: store::JournalOperation::Put(Box::new(record)),
        });
    }
}

#[cfg(test)]
impl State {
    pub(crate) fn terminal_failure_for_test(count: usize) -> Self {
        let mut state = Self::new();
        state.task = 7;
        state.front = Some(store::Attribution {
            partner: 1,
            name: "Bolt".into(),
            form: 2,
        });
        state.work = Some(store::WorkClaim {
            task_id: 7,
            owner: state.front.clone().unwrap(),
            request_id: "provider-test-1".into(),
            writing: true,
        });
        state.work_acquired = true;
        state.generation = Some(11);
        state.active = true;
        state.bind_slot(state.task);
        state.slots.values_mut().next().unwrap().generation = Some(11);
        for index in 0..count {
            state.queue_record(7, tests::record(&format!("unsaved-{index}")));
            state.journal_started();
            state.journal_event(7, Err(tests::QUOTA.into()));
        }
        tests::finish(&mut state, crate::providers::Outcome::Completed);
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{Agent, Event, Outcome};
    use std::time::{Duration, Instant};

    pub(super) const QUOTA: &str = "Provider journal reached 256 records for this task; explicitly purge before capturing more";

    pub(super) fn record(id: &str) -> store::ProviderRecord {
        store::ProviderRecord {
            id: id.into(),
            agent: Agent::Codex,
            generation: 11,
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: id.into(),
            origin: "Codex provider-native".into(),
            coverage: "Public observed result only; not local verification".into(),
            text: "Provider output".into(),
            complete: true,
            partner: None,
        }
    }

    pub(super) fn finish(state: &mut State, outcome: Outcome) {
        state.event(Event::Finished {
            generation: 11,
            task_id: state.task,
            outcome,
            message: "Provider transport finished.".into(),
        });
    }

    #[test]
    fn reviewed_discard_retains_gap_and_releases_only_exact_claim() {
        let mut state = State::terminal_failure_for_test(2);
        let claim = state.work.clone().unwrap();
        state.items.clear();
        state.thread_id = Some("retained-thread".into());
        state.settle_work();
        assert!(state.journal_failed() && state.work_pending.is_none());
        let snapshot = state.journal_discard_snapshot().unwrap();
        assert_eq!(snapshot.ids(), ["unsaved-0", "unsaved-1"]);
        assert!(snapshot.error().contains(QUOTA));
        state.acknowledge_journal_discard(&snapshot).unwrap();
        assert!(!state.journal_failed());
        assert!(
            matches!(state.work_pending.as_ref(), Some(store::WorkRequest::Release(owned)) if owned == &claim)
        );
        assert_eq!(state.work, Some(claim));
        assert_eq!(state.thread_id.as_deref(), Some("retained-thread"));
        assert!(state.pending.is_none());
        assert!(
            state
                .journal_notice
                .contains("Unverified provider coverage gap")
        );
        assert!(state.journal_notice.contains(QUOTA));
        assert_eq!(state.journal_recovery.gaps[0].ids, snapshot.ids());
        assert!(state.acknowledge_journal_discard(&snapshot).is_err());
    }

    #[test]
    fn running_unknown_missing_terminal_and_inflight_operations_are_refused() {
        for case in 0..9 {
            let mut state = State::terminal_failure_for_test(1);
            match case {
                0 => state.active = true,
                1 => state.unknown = true,
                2 => state.journal_recovery.terminal = None,
                3 => state.work_inflight = true,
                4 => {
                    state.work_pending =
                        Some(store::WorkRequest::Release(state.work.clone().unwrap()))
                }
                5 => state.pending_journal.push_back(store::JournalRequest {
                    task_id: 7,
                    operation: store::JournalOperation::List,
                }),
                6 => state.journal_inflight.push_back(store::JournalRequest {
                    task_id: 7,
                    operation: store::JournalOperation::List,
                }),
                7 => state.work_acquired = false,
                8 => {
                    state.work_pending = Some(store::WorkRequest::Acquire {
                        claim: state.work.clone().unwrap(),
                        task: Box::new(store::Snapshot::empty("test".into())),
                    })
                }
                _ => unreachable!(),
            }
            assert!(state.journal_discard_snapshot().is_err(), "case {case}");
            assert!(state.journal_failed());
        }
        let mut state = State::terminal_failure_for_test(1);
        state.generation = Some(11);
        finish(&mut state, Outcome::Unknown);
        assert!(state.journal_discard_snapshot().is_err());
    }

    #[test]
    fn changed_snapshot_identity_errors_and_retry_cannot_acknowledge() {
        for case in 0..8 {
            let mut state = State::terminal_failure_for_test(1);
            let snapshot = state.journal_discard_snapshot().unwrap();
            match case {
                0 => state.work.as_mut().unwrap().owner.name = "Changed".into(),
                1 => state.work.as_mut().unwrap().task_id = 8,
                2 => state.work.as_mut().unwrap().request_id = "other-request".into(),
                3 => {
                    state.journal_recovery.observations[0].error =
                        Some(format!("{QUOTA}. New error"))
                }
                4 => {
                    let store::JournalOperation::Put(record) =
                        &mut state.failed_journal.front_mut().unwrap().operation
                    else {
                        panic!()
                    };
                    record.text = "changed content".into();
                }
                5 => {
                    state.retry_journal();
                    state.journal_started();
                    state.journal_event(7, Err(QUOTA.into()));
                }
                6 => state.journal_recovery.observations[0].ambiguous = true,
                7 => state.failed_journal.push_back(store::JournalRequest {
                    task_id: 7,
                    operation: store::JournalOperation::List,
                }),
                _ => unreachable!(),
            }
            assert!(
                state.acknowledge_journal_discard(&snapshot).is_err(),
                "case {case}"
            );
            assert!(state.journal_failed());
            assert!(state.work_pending.is_none());
        }
    }

    #[test]
    fn failed_reads_purges_and_ambiguous_failures_are_never_discarded() {
        for operation in [
            store::JournalOperation::List,
            store::JournalOperation::Purge,
        ] {
            let mut state = State::terminal_failure_for_test(1);
            state.failed_journal.clear();
            state.failed_journal.push_back(store::JournalRequest {
                task_id: 7,
                operation,
            });
            assert!(state.journal_discard_snapshot().is_err());
            state.journal_inflight.push_back(store::JournalRequest {
                task_id: 7,
                operation: store::JournalOperation::Purge,
            });
            state.journal_event(7, Ok(store::JournalResponse::Purged));
            assert!(
                state.journal_failed(),
                "purge cannot silently clear a failed operation"
            );
        }
        for error in [
            "Provider journal commit failed; no persistence claimed: disk error",
            "Provider record ID cannot overwrite different task/origin/content; append a new identity",
            "Uncorrelated journal acknowledgement",
            "Storage unavailable; provider evidence was not saved",
        ] {
            let mut state = State::terminal_failure_for_test(1);
            state.journal_recovery.observations[0].error = Some(error.into());
            assert!(state.journal_discard_snapshot().is_err(), "{error}");
        }
    }

    #[test]
    fn repeated_partial_final_and_identical_failed_writes_keep_exact_attempt_order() {
        let mut state = State::terminal_failure_for_test(0);
        state.generation = Some(11);
        state.active = true;
        let mut partial = record("same-item");
        partial.complete = false;
        partial.text = "Partial".into();
        let mut final_record = partial.clone();
        final_record.complete = true;
        final_record.text = "Final".into();
        for (index, record) in [partial, final_record.clone(), final_record]
            .into_iter()
            .enumerate()
        {
            state.queue_record(7, record);
            state.journal_started();
            state.journal_event(7, Err(format!("{QUOTA}. Attempt {index}")));
        }
        finish(&mut state, Outcome::Completed);
        let snapshot = state.journal_discard_snapshot().unwrap();
        assert_eq!(snapshot.ids(), ["same-item", "same-item", "same-item"]);
        assert_eq!(snapshot.failed[0].0.text, "Partial");
        assert_eq!(snapshot.failed[1].0.text, "Final");
        assert!(snapshot.failed[1].1.ends_with("Attempt 1"));
        assert!(snapshot.failed[2].1.ends_with("Attempt 2"));
        state.failed_journal.swap(0, 1);
        assert!(state.acknowledge_journal_discard(&snapshot).is_err());
        state.failed_journal.swap(0, 1);
        state.acknowledge_journal_discard(&snapshot).unwrap();
        assert!(!state.journal_failed());
        assert_eq!(state.journal_recovery.gaps[0].ids.len(), 3);
    }

    #[test]
    fn failed_update_acknowledgement_preserves_previously_committed_observation() {
        let mut state = State::terminal_failure_for_test(0);
        state.generation = Some(11);
        state.active = true;
        let mut partial = record("committed-item");
        partial.complete = false;
        partial.text = "Saved partial observation".into();
        state.queue_record(7, partial);
        state.journal_started();
        let store::JournalOperation::Put(saved) =
            &state.journal_inflight.front().unwrap().operation
        else {
            panic!()
        };
        let saved = (**saved).clone();
        state.journal_event(7, Ok(store::JournalResponse::Saved(saved.clone())));
        state.journal_records = vec![saved.clone()];
        let mut update = saved.clone();
        update.text = "Unsaved later observation".into();
        update.complete = true;
        state.queue_record(7, update);
        state.journal_started();
        state.journal_event(7, Err("Local storage quota exceeded (live SQLite + retained output + committed reservations); insertion rolled back".into()));
        finish(&mut state, Outcome::Completed);
        let snapshot = state.journal_discard_snapshot().unwrap();
        assert_eq!(snapshot.ids(), ["committed-item"]);
        assert_eq!(snapshot.failed[0].0.text, "Unsaved later observation");
        state.acknowledge_journal_discard(&snapshot).unwrap();
        assert_eq!(state.journal_records, vec![saved]);
        assert!(
            state
                .journal_notice
                .contains("Unverified provider coverage gap")
        );
        assert!(matches!(
            state.work_pending.as_ref(),
            Some(store::WorkRequest::Release(_))
        ));
    }

    #[test]
    fn acknowledged_gap_survives_reads_purge_reset_and_bounded_retention() {
        let mut state = State::terminal_failure_for_test(1);
        let snapshot = state.journal_discard_snapshot().unwrap();
        state.acknowledge_journal_discard(&snapshot).unwrap();
        let release = state.work_pending.take().unwrap();
        state.work_event(release, Ok(Vec::new()));
        for response in [
            store::JournalResponse::Records(Vec::new()),
            store::JournalResponse::Purged,
        ] {
            let operation = if matches!(response, store::JournalResponse::Purged) {
                store::JournalOperation::Purge
            } else {
                store::JournalOperation::List
            };
            state.pending_journal.push_back(store::JournalRequest {
                task_id: 7,
                operation,
            });
            state.journal_started();
            state.journal_event(7, Ok(response));
            assert!(
                state
                    .journal_notice
                    .contains("Unverified provider coverage gap")
            );
        }
        state.reset(8);
        state.reset(7);
        assert_eq!(state.journal_recovery.gaps.len(), 1);
        assert!(state.journal_notice.contains(QUOTA));
        for _ in 0..20 {
            let mut next = State::terminal_failure_for_test(1);
            next.journal_recovery.gaps = std::mem::take(&mut state.journal_recovery.gaps);
            next.journal_recovery.omitted_gap_observations =
                state.journal_recovery.omitted_gap_observations;
            let snapshot = next.journal_discard_snapshot().unwrap();
            next.acknowledge_journal_discard(&snapshot).unwrap();
            state = next;
        }
        assert_eq!(state.journal_recovery.gaps.len(), 16);
        assert_eq!(state.journal_recovery.omitted_gap_observations, 5);
        assert!(state.journal_notice.contains("older exact details omitted"));
    }

    fn store_event(worker: &store::Worker) -> store::Event {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(event) = worker.try_event() {
                return event;
            }
            assert!(Instant::now() < deadline, "storage event timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn real_257th_record_quota_requires_discard_release_then_ordinary_purge() {
        let path = std::env::temp_dir().join(format!(
            "eevee-journal-discard-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        let directory = Directory(path);
        let worker = store::Worker::start(
            directory.0.clone(),
            "journal-discard".into(),
            store::Policy::default(),
        )
        .unwrap();
        let store::Event::Loaded(project) = store_event(&worker) else {
            panic!("expected loaded project")
        };
        let owner = store::Attribution::of(&project.partners[0]);
        let mut input = store::Snapshot::empty("journal-discard".into());
        input.goal = "Capture provider observations".into();
        let mut pending = Some(input);
        assert!(worker.try_save(&mut pending));
        let store::Event::Saved(task) = store_event(&worker) else {
            panic!("expected saved task")
        };
        let mut state = State::new();
        state.task = task.id;
        state.front = Some(owner);
        let claim = store::WorkClaim {
            task_id: task.id,
            owner: state.front.clone().unwrap(),
            request_id: "quota-request".into(),
            writing: false,
        };
        state.work = Some(claim.clone());
        let mut pending = Some(store::WorkRequest::Acquire {
            claim: claim.clone(),
            task: Box::new(task.clone()),
        });
        assert!(worker.try_work(&mut pending));
        let store::Event::Work { request, result } = store_event(&worker) else {
            panic!("expected acquisition")
        };
        assert!(result.is_ok());
        state.work_event(request, result);
        state.generation = Some(11);
        state.active = true;
        state.bind_slot(state.task);
        state.slots.values_mut().next().unwrap().generation = Some(11);
        for index in 0..257 {
            state.queue_record(task.id, record(&format!("quota-{index}")));
            let mut request = state.pending_journal.front().cloned();
            assert!(worker.try_journal(&mut request));
            state.journal_started();
            let store::Event::Journal { task_id, result } = store_event(&worker) else {
                panic!("expected journal result")
            };
            if index < 256 {
                assert!(result.is_ok(), "{result:?}");
            } else {
                assert!(result.as_ref().unwrap_err().contains("256 records"));
            }
            state.journal_event(task_id, result);
        }
        assert!(state.journal_discard_snapshot().is_err(), "still running");
        finish(&mut state, Outcome::Completed);
        state.settle_work();
        assert!(state.work_pending.is_none() && state.journal_failed());
        let snapshot = state.journal_discard_snapshot().unwrap();
        assert_eq!(snapshot.ids(), ["quota-256"]);
        state.acknowledge_journal_discard(&snapshot).unwrap();
        assert!(
            matches!(state.work_pending.as_ref(), Some(store::WorkRequest::Release(owned)) if owned == &claim)
        );
        // DB observations and the durable claim are unchanged by acknowledgement.
        let mut purge = Some(store::JournalRequest {
            task_id: task.id,
            operation: store::JournalOperation::Purge,
        });
        assert!(worker.try_journal(&mut purge));
        assert!(matches!(
            store_event(&worker),
            store::Event::Journal { result: Err(_), .. }
        ));
        assert!(worker.try_work(&mut state.work_pending));
        let store::Event::Work { request, result } = store_event(&worker) else {
            panic!("expected release")
        };
        assert!(result.is_ok());
        state.work_event(request, result);
        assert!(state.work.is_none());
        let mut list = Some(store::JournalRequest {
            task_id: task.id,
            operation: store::JournalOperation::List,
        });
        assert!(worker.try_journal(&mut list));
        assert!(
            matches!(store_event(&worker), store::Event::Journal { result: Ok(store::JournalResponse::Records(records)), .. } if records.len() == 256)
        );
        let mut purge = Some(store::JournalRequest {
            task_id: task.id,
            operation: store::JournalOperation::Purge,
        });
        assert!(worker.try_journal(&mut purge));
        assert!(matches!(
            store_event(&worker),
            store::Event::Journal {
                result: Ok(store::JournalResponse::Purged),
                ..
            }
        ));
        assert!(
            state
                .journal_notice
                .contains("Unverified provider coverage gap")
        );
        assert!(state.pending.is_none());
        drop(worker);
    }
}
