//! A request keeps ownership until every provider usage observation is durable.
use super::State as Planning;
use crate::{
    store,
    workspace::usage::{Record, Report},
};

#[derive(Default)]
pub struct State {
    pub record: Option<Record>,
    pending: Option<store::records::Request>,
    inflight: bool,
    failed: Option<store::records::Request>,
    dirty: bool,
    pub error: Option<String>,
    rejected: Vec<Report>,
    submitted: bool,
}

impl Planning {
    pub(super) fn usage_submitted(&mut self) {
        self.usage.submitted = true;
        if self.usage.record.is_none()
            && let Some(claim) = &self.work
        {
            self.usage.record = Some(Record::new(
                claim.request_id.clone(),
                claim.task_id,
                claim.owner.clone(),
                self.agent,
            ));
            // Also persist genuinely unknown coverage (for example ACP), never
            // a fabricated zero. The receipt is durable before work release.
            self.usage.dirty = true;
        }
    }

    pub fn usage_pending(&self) -> bool {
        self.usage.dirty
            || self.usage.pending.is_some()
            || self.usage.inflight
            || self.usage.failed.is_some()
            || !self.usage.rejected.is_empty()
    }

    pub fn usage_purged(&mut self, task_id: i64) {
        if self.task == task_id && !self.usage_pending() {
            self.usage = State::default();
        }
    }

    pub(super) fn usage_report(&mut self, generation: u64, task_id: i64, report: Report) {
        let Some(claim) = self.work.as_ref().filter(|claim| {
            self.work_acquired
                && self.usage.submitted
                && self.matches(generation, task_id)
                && claim.task_id == task_id
                && !self.inspect_models
                && !self.inspect_sessions
        }) else {
            return;
        };
        let record = self.usage.record.get_or_insert_with(|| {
            Record::new(
                claim.request_id.clone(),
                claim.task_id,
                claim.owner.clone(),
                self.agent,
            )
        });
        if record.request_id != claim.request_id
            || record.task_id != claim.task_id
            || record.owner != claim.owner
            || record.agent != self.agent
        {
            self.usage.error =
                Some("Usage observation does not match its immutable work claim".into());
            self.usage.rejected.push(report);
            return;
        }
        // Retain the complete provider observation even if bounded validation refuses it.
        match record.apply_retained(report) {
            Ok(changed) => self.usage.dirty |= changed,
            Err((error, report)) => {
                self.usage.rejected.push(*report);
                self.usage.error = Some(error);
            }
        }
    }

    /// try_records consumes only on admission; both backpressure and failure keep bytes.
    pub fn try_usage(&mut self, worker: &store::Worker) -> bool {
        if self.usage.inflight || self.usage.error.is_some() {
            return false;
        }
        if self.usage.pending.is_none() && self.usage.dirty {
            let Some(record) = self.usage.record.as_ref() else {
                return false;
            };
            self.usage.pending = Some(store::records::Request {
                task_id: record.task_id,
                operation: store::records::Operation::Usage {
                    record: record.clone(),
                },
            });
            self.usage.dirty = false;
        }
        if worker.try_records(&mut self.usage.pending) {
            self.usage.inflight = true;
            true
        } else {
            false
        }
    }

    pub fn usage_event(
        &mut self,
        request: store::records::Request,
        result: Result<Box<store::records::Response>, String>,
    ) {
        let expected = match &request.operation {
            store::records::Operation::Usage { record } => record,
            _ => return,
        };
        let owned = self.usage.inflight
            && self.work.as_ref().is_some_and(|claim| {
                self.work_acquired
                    && claim.request_id == expected.request_id
                    && claim.task_id == request.task_id
                    && expected.task_id == request.task_id
                    && claim.owner == expected.owner
                    && expected.agent == self.agent
            });
        if !owned {
            return;
        }
        self.usage.inflight = false;
        let correlated = matches!(&result, Ok(response) if matches!(response.as_ref(), store::records::Response::UsageSaved(saved) if saved == expected));
        if correlated {
            self.usage.error = None;
            self.usage.dirty |= self.usage.record.as_ref() != Some(expected);
        } else {
            self.usage.error = Some(match result {
                Err(error) => error,
                Ok(_) => "Uncorrelated usage acknowledgement refused".into(),
            });
            self.usage.failed = Some(request);
        }
        self.settle_work();
    }

    pub fn retry_usage(&mut self) {
        if let Some(request) = self.usage.failed.take() {
            self.usage.pending = Some(request);
            self.usage.error = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        providers::Agent,
        workspace::usage::{Counters, Identity, Kind, Scope, Semantics},
    };

    fn state() -> Planning {
        let mut state = Planning::new();
        state.task = 7;
        state.generation = Some(11);
        state.work_acquired = true;
        state.usage.submitted = true;
        state.work = Some(store::WorkClaim {
            request_id: "actual-work-request".into(),
            task_id: 7,
            owner: store::Attribution::of(&store::Partner::default()),
            writing: false,
        });
        state
    }
    fn report() -> Report {
        Report {
            identity: Identity {
                response: Some("wire-response".into()),
                ..Default::default()
            },
            scope: Scope::Turn,
            kind: Kind::Snapshot,
            counters: Counters {
                input: Some(10),
                output: Some(3),
                ..Default::default()
            },
            baseline: None,
            model: None,
            tokenizer: None,
            semantics: Semantics::default(),
            complete: true,
        }
    }
    fn request(state: &Planning) -> store::records::Request {
        store::records::Request {
            task_id: 7,
            operation: store::records::Operation::Usage {
                record: state.usage.record.clone().unwrap(),
            },
        }
    }
    #[test]
    fn actual_claim_identity_and_storage_failure_gate_release_after_finished() {
        let mut state = state();
        state.usage_report(11, 7, report());
        assert_eq!(
            state.usage.record.as_ref().unwrap().request_id,
            "actual-work-request"
        );
        assert_eq!(state.usage.record.as_ref().unwrap().agent, Agent::Codex);
        let request = request(&state);
        state.usage.dirty = false;
        state.usage.inflight = true;
        state.generation = None; // Finished does not surrender request ownership.
        state.usage_event(request.clone(), Err("fixture storage failure".into()));
        state.settle_work();
        assert!(state.work_pending.is_none() && state.owns_work());
        assert_eq!(state.usage.failed.as_ref(), Some(&request));
        state.retry_usage();
        assert_eq!(state.usage.pending.as_ref(), Some(&request));
        state.usage.pending = None;
        state.usage.inflight = true;
        let store::records::Operation::Usage { record } = &request.operation else {
            unreachable!()
        };
        let saved = record.clone();
        state.usage_event(
            request,
            Ok(Box::new(store::records::Response::UsageSaved(saved))),
        );
        assert!(!state.usage_pending());
        assert!(
            matches!(state.work_pending.as_ref(), Some(store::WorkRequest::Release(claim)) if claim.request_id == "actual-work-request")
        );
    }
    #[test]
    fn late_foreign_reports_and_wrong_ack_cannot_release_owner() {
        let mut state = state();
        state.usage_report(12, 7, report());
        assert!(state.usage.record.is_none());
        state.usage_report(11, 7, report());
        let request = request(&state);
        state.usage.dirty = false;
        state.usage.inflight = true;
        let mut wrong = state.usage.record.clone().unwrap();
        wrong.request_id = "other-request".into();
        state.usage_event(
            request,
            Ok(Box::new(store::records::Response::UsageSaved(wrong))),
        );
        state.settle_work();
        assert!(state.usage.error.is_some() && state.work_pending.is_none());
    }
    #[test]
    fn repeated_snapshot_does_not_create_new_write_and_partial_during_inflight_is_retained() {
        let mut state = state();
        state.usage_report(11, 7, report());
        let request = request(&state);
        state.usage.dirty = false;
        state.usage.inflight = true;
        state.usage_report(11, 7, report());
        assert!(!state.usage.dirty);
        let mut partial = report();
        partial.counters.output = Some(8);
        partial.counters.input = None;
        state.usage_report(11, 7, partial);
        let store::records::Operation::Usage { record } = &request.operation else {
            unreachable!()
        };
        let saved = record.clone();
        state.usage_event(
            request,
            Ok(Box::new(store::records::Response::UsageSaved(saved))),
        );
        assert!(state.usage_pending() && state.work_pending.is_none());
        assert_eq!(
            state.usage.record.as_ref().unwrap().measurements[0]
                .attributable
                .input,
            Some(10)
        );
        assert_eq!(
            state.usage.record.as_ref().unwrap().measurements[0]
                .attributable
                .output,
            Some(8)
        );
    }
    #[test]
    fn dispatched_unknown_coverage_is_saved_before_ownership_can_release() {
        let mut state = state();
        state.agent = crate::providers::Agent::OpenCode;
        state.usage_submitted();
        state.settle_work();
        let record = state.usage.record.as_ref().unwrap();
        assert!(
            record.measurements.is_empty(),
            "Unsupported coverage is unknown, never zero"
        );
        assert_eq!(record.request_id, "actual-work-request");
        assert!(state.usage_pending() && state.work_pending.is_none());
    }
    #[test]
    fn queue_acceptance_without_actual_dispatch_cannot_produce_usage() {
        let mut state = state();
        state.usage.submitted = false;
        state.usage_report(11, 7, report());
        assert!(state.usage.record.is_none());
        assert!(!state.usage_pending());
    }
    #[test]
    fn purge_clears_only_settled_usage_and_never_acknowledges_failed_write() {
        let mut state = state();
        state.usage_report(11, 7, report());
        let request = request(&state);
        state.usage.dirty = false;
        state.usage.inflight = true;
        state.usage_event(request, Err("retained fixture failure".into()));
        let retained = state.usage.record.clone();
        state.usage_purged(7);
        assert_eq!(state.usage.record, retained);
        assert!(state.usage_pending());
        state.usage.failed = None;
        state.usage.error = None;
        state.usage_purged(8);
        assert!(state.usage.record.is_some());
        state.usage_purged(7);
        assert!(state.usage.record.is_none());
    }
}
