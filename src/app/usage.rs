//! Route usage acknowledgements to the immutable request owner, not the front partner.
use super::*;

impl App {
    /// Call for Event::Records whose exact operation is Usage, before records-panel handling.
    pub(super) fn cooperation_usage_event(
        &mut self,
        request: store::records::Request,
        result: Result<Box<store::records::Response>, String>,
    ) {
        let store::records::Operation::Usage { record } = &request.operation else {
            return;
        };
        let partner = record.owner.partner;
        let failed = result.is_err();
        if let Some(state) = self.planning_state_mut(partner) {
            state.usage_event(request, result);
        }
        if failed {
            self.notice = format!(
                "Partner {partner}: reported usage storage failed. /storage-retry retries the exact retained storage operation only; never replays the provider turn."
            );
        }
        self.settle_cooperation();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        providers::{Agent, Event, Preview},
        workspace::usage::{Counters, Identity, Kind, Report, Scope, Semantics},
    };

    fn owned(app: &App, partner: i64, request: &str) -> planning::State {
        let owner = store::Attribution {
            partner,
            name: format!("Owner {partner}"),
            form: 0,
        };
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.lead_partner = partner;
        task.goal = "Read-only accounting fixture".into();
        let mut state = app.planning.new_partner(owner.clone(), 7);
        state.command("new", true, &task, &[], None);
        let claim = store::WorkClaim {
            task_id: 7,
            owner,
            request_id: request.into(),
            writing: false,
        };
        state.stage_work(claim.clone());
        let acquire = state.work_pending.take().unwrap();
        state.work_event(acquire, Ok(vec![claim]));
        state.pending.take();
        state.event(Event::Preview(Preview {
            comparison_scope: None,
            arrival: 1,
            generation: 11,
            task_id: 7,
            thread_id: None,
            destination: None,
            requires_connection_review: false,
            trust: Vec::new(),
            payload: "{}".into(),
            blockers: Vec::new(),
        }));
        assert!(state.dispatch_prepared());
        state.pending.take();
        state
    }

    #[test]
    fn worker_generation_collision_never_moves_usage_to_the_front_partner() {
        let mut app = App::new("/workspace".into(), 120, 36);
        for partner in [2, 3] {
            let request = format!("actual-request-{partner}");
            let state = owned(&app, partner, &request);
            app.cooperation.parked.insert(partner, state);
            app.provider_event(
                partner,
                Event::Dispatched {
                    generation: 11,
                    task_id: 7,
                    request_id: request.clone(),
                },
            );
            app.provider_event(
                partner,
                Event::Usage {
                    generation: 11,
                    task_id: 7,
                    report: Report {
                        identity: Identity {
                            response: Some("same-worker-local-response-id".into()),
                            ..Default::default()
                        },
                        scope: Scope::Turn,
                        kind: Kind::Snapshot,
                        counters: Counters {
                            input: Some(partner as u64 * 10),
                            ..Default::default()
                        },
                        baseline: None,
                        model: None,
                        tokenizer: None,
                        semantics: Semantics::default(),
                        complete: true,
                    },
                },
            );
            let record = app.cooperation.parked[&partner]
                .usage
                .record
                .as_ref()
                .unwrap();
            assert_eq!(record.request_id, request);
            assert_eq!(record.owner.partner, partner);
            assert_eq!(record.agent, Agent::Codex);
            assert_eq!(
                record.measurements[0].attributable.input,
                Some(partner as u64 * 10)
            );
        }
        assert!(app.planning.usage.record.is_none());
        assert_ne!(
            app.cooperation.parked[&2].usage.record,
            app.cooperation.parked[&3].usage.record
        );
    }
}
