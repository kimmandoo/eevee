//! Provider-reported counters. These are neither local estimates nor prices.
use crate::{providers::Agent, store::Attribution};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    pub reasoning: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    Turn,
    MainLoopTurn,
    Response,
    Session,
    Pipeline,
    ModelPipeline,
}

impl Scope {
    pub fn is_request_scoped(self) -> bool {
        matches!(self, Self::Turn | Self::MainLoopTurn | Self::Response)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Snapshot,
    Delta,
}

/// None means that the provider did not establish the subset relationship.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Semantics {
    pub cache_read_in_input: Option<bool>,
    pub cache_write_in_input: Option<bool>,
    pub reasoning_in_output: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub session: Option<String>,
    pub turn: Option<String>,
    pub response: Option<String>,
    /// Required for genuine deltas; snapshots do not use it as an additive key.
    pub event: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub identity: Identity,
    pub scope: Scope,
    pub kind: Kind,
    pub counters: Counters,
    /// An observed cumulative value immediately before this submitted request.
    /// Missing baseline on resumed history is unknown, never implicitly zero.
    pub baseline: Option<Counters>,
    pub model: Option<String>,
    pub tokenizer: Option<String>,
    pub semantics: Semantics,
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Measurement {
    /// Latest merged raw provider snapshot (or merged genuine deltas).
    pub report: Report,
    /// Request-attributable counts. Cumulative scopes require a known baseline.
    pub attributable: Counters,
    pub delta_events: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub request_id: String,
    pub task_id: i64,
    pub owner: Attribution,
    pub agent: Agent,
    pub measurements: Vec<Measurement>,
}

impl Counters {
    fn merge(&mut self, other: &Self) {
        macro_rules! field {
            ($name:ident) => {
                if other.$name.is_some() {
                    self.$name = other.$name;
                }
            };
        }
        field!(input);
        field!(output);
        field!(cache_read);
        field!(cache_write);
        field!(reasoning);
    }
    fn add(&mut self, other: &Self) -> Result<(), String> {
        macro_rules! field {
            ($name:ident) => {
                if let Some(value) = other.$name {
                    self.$name = Some(
                        self.$name
                            .unwrap_or(0)
                            .checked_add(value)
                            .ok_or("Provider usage counter overflow")?,
                    );
                }
            };
        }
        field!(input);
        field!(output);
        field!(cache_read);
        field!(cache_write);
        field!(reasoning);
        Ok(())
    }
    pub fn difference(&self, baseline: &Self) -> Self {
        fn sub(value: Option<u64>, base: Option<u64>) -> Option<u64> {
            value
                .zip(base)
                .and_then(|(value, base)| value.checked_sub(base))
        }
        Self {
            input: sub(self.input, baseline.input),
            output: sub(self.output, baseline.output),
            cache_read: sub(self.cache_read, baseline.cache_read),
            cache_write: sub(self.cache_write, baseline.cache_write),
            reasoning: sub(self.reasoning, baseline.reasoning),
        }
    }
    pub fn is_unknown(&self) -> bool {
        self == &Self::default()
    }
}

impl Report {
    pub fn validate(&self) -> Result<(), String> {
        fn text(value: &str) -> bool {
            !value.is_empty()
                && value.len() <= 512
                && !value.chars().any(char::is_control)
                && crate::workspace::privacy::redact(value) == value
        }
        if [
            &self.identity.session,
            &self.identity.turn,
            &self.identity.response,
            &self.identity.event,
            &self.model,
            &self.tokenizer,
        ]
        .into_iter()
        .flatten()
        .any(|value| !text(value))
        {
            return Err("Invalid provider usage identity/model/tokenizer".into());
        }
        if self.kind == Kind::Delta && self.identity.event.is_none() {
            return Err("Provider usage delta requires a stable event identity".into());
        }
        Ok(())
    }
}

impl Record {
    pub fn new(request_id: String, task_id: i64, owner: Attribution, agent: Agent) -> Self {
        Self {
            request_id,
            task_id,
            owner,
            agent,
            measurements: Vec::new(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.owner.validate()?;
        if self.request_id.is_empty()
            || self.request_id.len() > 512
            || self.task_id <= 0
            || self.measurements.len() > 128
            || self.request_id.chars().any(char::is_control)
            || crate::workspace::privacy::redact(&self.request_id) != self.request_id
        {
            return Err("Invalid provider usage request".into());
        }
        for measurement in &self.measurements {
            measurement.report.validate()?;
            let attributable = if measurement.report.scope.is_request_scoped() {
                measurement.report.counters
            } else {
                measurement
                    .report
                    .baseline
                    .as_ref()
                    .map(|base| measurement.report.counters.difference(base))
                    .unwrap_or_default()
            };
            if measurement.attributable != attributable {
                return Err(
                    "Provider usage attribution differs from its reported boundary/baseline".into(),
                );
            }
            if measurement.delta_events.len() > 4096
                || measurement
                    .delta_events
                    .iter()
                    .any(|id| id.is_empty() || id.len() > 512 || id.chars().any(char::is_control))
            {
                return Err("Invalid provider usage delta identities".into());
            }
        }
        Ok(())
    }
    /// The owner can retain a refused observation without cloning every event.
    pub fn apply_retained(&mut self, report: Report) -> Result<bool, (String, Box<Report>)> {
        if let Err(error) = report.validate() {
            return Err((error, Box::new(report)));
        }
        // A final snapshot can report identity/model fields absent in a partial
        // event. Refine one compatible boundary; never guess among several.
        let compatible = |left: &Option<String>, right: &Option<String>| {
            left.is_none() || right.is_none() || left == right
        };
        let mut index = None;
        for (candidate, entry) in self.measurements.iter().enumerate() {
            let previous = &entry.report;
            if previous.scope == report.scope
                && compatible(&previous.identity.session, &report.identity.session)
                && compatible(&previous.identity.turn, &report.identity.turn)
                && compatible(&previous.identity.response, &report.identity.response)
                && compatible(&previous.model, &report.model)
                && compatible(&previous.tokenizer, &report.tokenizer)
            {
                if index.is_some() {
                    return Err((
                        "Ambiguous provider usage boundary without reported identity".into(),
                        Box::new(report),
                    ));
                }
                index = Some(candidate);
            }
        }
        let Some(index) = index else {
            if self.measurements.len() >= 128 {
                return Err((
                    "Provider usage reached 128 measurement boundaries for this request".into(),
                    Box::new(report),
                ));
            }
            let attributable = if report.scope.is_request_scoped() {
                report.counters
            } else {
                report
                    .baseline
                    .as_ref()
                    .map(|base| report.counters.difference(base))
                    .unwrap_or_default()
            };
            let delta_events = if report.kind == Kind::Delta {
                vec![report.identity.event.clone().unwrap()]
            } else {
                Vec::new()
            };
            self.measurements.push(Measurement {
                report,
                attributable,
                delta_events,
            });
            return Ok(true);
        };
        let entry = &mut self.measurements[index];
        if report.kind == Kind::Delta
            && entry
                .delta_events
                .contains(report.identity.event.as_ref().unwrap())
        {
            return Ok(false);
        }
        // A final cumulative response snapshot already includes earlier deltas.
        if report.kind == Kind::Delta
            && entry.report.kind == Kind::Snapshot
            && entry.report.complete
        {
            return Ok(false);
        }
        if let (Some(base), Some(next)) = (entry.report.baseline, report.baseline) {
            let changed = base.input.zip(next.input).is_some_and(|(a, b)| a != b)
                || base.output.zip(next.output).is_some_and(|(a, b)| a != b)
                || base
                    .cache_read
                    .zip(next.cache_read)
                    .is_some_and(|(a, b)| a != b)
                || base
                    .cache_write
                    .zip(next.cache_write)
                    .is_some_and(|(a, b)| a != b)
                || base
                    .reasoning
                    .zip(next.reasoning)
                    .is_some_and(|(a, b)| a != b);
            if changed {
                return Err((
                    "Provider usage baseline changed within a submitted request".into(),
                    Box::new(report),
                ));
            }
        }
        let previous = (
            entry.report.counters,
            entry.report.baseline,
            entry.report.kind,
            entry.report.complete,
            entry.report.semantics,
        );
        let metadata_changed = (entry.report.identity.session.is_none()
            && report.identity.session.is_some())
            || (entry.report.identity.turn.is_none() && report.identity.turn.is_some())
            || (entry.report.identity.response.is_none() && report.identity.response.is_some())
            || (entry.report.model.is_none() && report.model.is_some())
            || (entry.report.tokenizer.is_none() && report.tokenizer.is_some());
        let delta = report.kind == Kind::Delta;
        if delta {
            if entry.delta_events.len() >= 4096 {
                return Err((
                    "Provider usage reached 4096 delta events for this boundary".into(),
                    Box::new(report),
                ));
            }
            let mut counters = entry.report.counters;
            if let Err(error) = counters.add(&report.counters) {
                return Err((error, Box::new(report)));
            }
            entry.report.counters = counters;
            entry.delta_events.push(report.identity.event.unwrap());
        } else {
            entry.report.counters.merge(&report.counters);
        }
        if report.kind == Kind::Snapshot {
            entry.report.kind = Kind::Snapshot;
        }
        if entry.report.identity.session.is_none() {
            entry.report.identity.session = report.identity.session;
        }
        if entry.report.identity.turn.is_none() {
            entry.report.identity.turn = report.identity.turn;
        }
        if entry.report.identity.response.is_none() {
            entry.report.identity.response = report.identity.response;
        }
        if entry.report.model.is_none() {
            entry.report.model = report.model;
        }
        if entry.report.tokenizer.is_none() {
            entry.report.tokenizer = report.tokenizer;
        }
        if let Some(base) = report.baseline {
            entry
                .report
                .baseline
                .get_or_insert_with(Counters::default)
                .merge(&base);
        }
        entry.report.complete |= report.complete;
        if report.semantics.cache_read_in_input.is_some() {
            entry.report.semantics.cache_read_in_input = report.semantics.cache_read_in_input;
        }
        if report.semantics.cache_write_in_input.is_some() {
            entry.report.semantics.cache_write_in_input = report.semantics.cache_write_in_input;
        }
        if report.semantics.reasoning_in_output.is_some() {
            entry.report.semantics.reasoning_in_output = report.semantics.reasoning_in_output;
        }
        entry.attributable = if entry.report.scope.is_request_scoped() {
            entry.report.counters
        } else {
            entry
                .report
                .baseline
                .as_ref()
                .map(|base| entry.report.counters.difference(base))
                .unwrap_or_default()
        };
        Ok(delta
            || metadata_changed
            || previous
                != (
                    entry.report.counters,
                    entry.report.baseline,
                    entry.report.kind,
                    entry.report.complete,
                    entry.report.semantics,
                ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(request: &str) -> Record {
        Record::new(
            request.into(),
            7,
            Attribution::of(&crate::store::Partner::default()),
            Agent::Codex,
        )
    }
    fn report(scope: Scope, input: Option<u64>, output: Option<u64>) -> Report {
        Report {
            identity: Identity {
                session: Some("wire-session".into()),
                turn: Some("wire-turn".into()),
                ..Default::default()
            },
            scope,
            kind: Kind::Snapshot,
            counters: Counters {
                input,
                output,
                ..Default::default()
            },
            baseline: None,
            model: None,
            tokenizer: None,
            semantics: Semantics::default(),
            complete: false,
        }
    }
    #[test]
    fn partial_final_and_repeated_snapshots_replace_without_zeroing_unknowns() {
        let mut record = record("request");
        assert!(
            record
                .apply_retained(report(Scope::Turn, Some(120), None))
                .unwrap()
        );
        let mut final_report = report(Scope::Turn, None, Some(14));
        final_report.complete = true;
        final_report.counters.cache_read = Some(40);
        final_report.counters.reasoning = Some(6);
        final_report.semantics.cache_read_in_input = Some(true);
        final_report.semantics.reasoning_in_output = Some(true);
        assert!(record.apply_retained(final_report.clone()).unwrap());
        assert!(!record.apply_retained(final_report).unwrap());
        assert_eq!(record.measurements.len(), 1);
        let counts = record.measurements[0].attributable;
        assert_eq!(
            (
                counts.input,
                counts.output,
                counts.cache_read,
                counts.reasoning
            ),
            (Some(120), Some(14), Some(40), Some(6))
        );
        assert_eq!(counts.cache_write, None);
        assert_eq!(record.measurements[0].report.model, None);
        assert_eq!(record.measurements[0].report.tokenizer, None);
        record.validate().unwrap();
    }
    #[test]
    fn stable_delta_events_deduplicate_and_final_snapshot_supersedes_deltas() {
        let mut record = record("request");
        let mut delta = report(Scope::Turn, None, Some(3));
        delta.kind = Kind::Delta;
        assert!(record.apply_retained(delta.clone()).is_err());
        delta.identity.event = Some("event-1".into());
        assert!(record.apply_retained(delta.clone()).unwrap());
        assert!(!record.apply_retained(delta.clone()).unwrap());
        delta.identity.event = Some("event-2".into());
        assert!(record.apply_retained(delta.clone()).unwrap());
        assert_eq!(record.measurements[0].attributable.output, Some(6));
        let mut final_report = report(Scope::Turn, Some(10), Some(6));
        final_report.complete = true;
        record.apply_retained(final_report.clone()).unwrap();
        assert!(!record.apply_retained(final_report).unwrap());
        assert!(!record.apply_retained(delta).unwrap());
        assert_eq!(record.measurements.len(), 1);
        assert_eq!(record.measurements[0].attributable.output, Some(6));
    }
    #[test]
    fn two_turns_and_resume_subtract_only_observed_cumulative_baseline() {
        let first_total = Counters {
            input: Some(100),
            output: Some(10),
            ..Default::default()
        };
        let mut first = record("first-actual-request");
        let mut first_report = report(Scope::Session, first_total.input, first_total.output);
        first_report.baseline = Some(Counters {
            input: Some(0),
            output: Some(0),
            ..Default::default()
        });
        first_report.complete = true;
        first.apply_retained(first_report.clone()).unwrap();
        assert!(!first.apply_retained(first_report).unwrap());
        assert_eq!(first.measurements[0].attributable, first_total);
        let mut second = record("second-actual-request");
        let mut second_report = report(Scope::Session, Some(250), Some(25));
        second_report.identity.turn = Some("wire-turn-2".into());
        second_report.baseline = Some(first_total);
        second_report.complete = true;
        second.apply_retained(second_report.clone()).unwrap();
        assert!(!second.apply_retained(second_report).unwrap());
        assert_eq!(
            (
                second.measurements[0].attributable.input,
                second.measurements[0].attributable.output
            ),
            (Some(150), Some(15))
        );
        let mut resumed = record("resumed-actual-request");
        let mut resumed_report = report(Scope::Session, Some(400), Some(42));
        resumed_report.identity.turn = Some("resumed-wire-turn".into());
        resumed.apply_retained(resumed_report.clone()).unwrap();
        resumed.apply_retained(resumed_report).unwrap();
        assert!(
            resumed.measurements[0].attributable.is_unknown(),
            "Unknown restored baseline cannot charge previous turns"
        );
    }
    #[test]
    fn cumulative_pipeline_and_main_loop_boundaries_never_add_together() {
        let mut record = record("request");
        record
            .apply_retained(report(Scope::MainLoopTurn, Some(10), Some(4)))
            .unwrap();
        let mut pipeline = report(Scope::ModelPipeline, Some(900), Some(80));
        pipeline.model = Some("reported-model".into());
        record.apply_retained(pipeline).unwrap();
        assert_eq!(record.measurements.len(), 2);
        assert_eq!(record.measurements[0].attributable.input, Some(10));
        assert!(record.measurements[1].attributable.is_unknown());
    }
    #[test]
    fn missing_or_decreasing_baseline_fields_remain_unknown_independently() {
        let mut record = record("request");
        let mut report = report(Scope::Session, Some(5), Some(20));
        report.baseline = Some(Counters {
            input: Some(10),
            output: Some(7),
            cache_read: Some(0),
            ..Default::default()
        });
        record.apply_retained(report).unwrap();
        let counts = record.measurements[0].attributable;
        assert_eq!(
            (counts.input, counts.output, counts.cache_read),
            (None, Some(13), None)
        );
    }
    #[test]
    fn overflowing_delta_is_atomic_and_original_observation_is_returned() {
        let mut record = record("request");
        let mut first = report(Scope::Turn, Some(1), Some(u64::MAX));
        first.kind = Kind::Delta;
        first.identity.event = Some("first-event".into());
        record.apply_retained(first).unwrap();
        let before = record.clone();
        let mut second = report(Scope::Turn, Some(5), Some(1));
        second.kind = Kind::Delta;
        second.identity.event = Some("second-event".into());
        let (_, refused) = record.apply_retained(second.clone()).unwrap_err();
        assert_eq!(*refused, second);
        assert_eq!(record, before);
    }
    #[test]
    fn forged_attribution_is_rejected_before_persistence() {
        let mut record = record("request");
        record
            .apply_retained(report(Scope::Session, Some(900), Some(80)))
            .unwrap();
        record.measurements[0].attributable.input = Some(900);
        assert!(record.validate().is_err());
    }
    #[test]
    fn final_report_refines_reported_identity_without_duplicating_partial_counts() {
        let mut record = record("request");
        record
            .apply_retained(report(Scope::Turn, Some(50), Some(1)))
            .unwrap();
        let mut final_report = report(Scope::Turn, None, Some(8));
        final_report.identity.response = Some("reported-response".into());
        final_report.model = Some("reported-model".into());
        final_report.complete = true;
        record.apply_retained(final_report.clone()).unwrap();
        assert!(!record.apply_retained(final_report).unwrap());
        assert_eq!(record.measurements.len(), 1);
        assert_eq!(record.measurements[0].attributable.input, Some(50));
        assert_eq!(
            record.measurements[0].report.model.as_deref(),
            Some("reported-model")
        );
    }
    #[test]
    fn conflicting_baseline_is_refused_without_changing_owned_record() {
        let mut record = record("request");
        let mut first = report(Scope::Session, Some(100), Some(10));
        first.baseline = Some(Counters {
            input: Some(20),
            ..Default::default()
        });
        record.apply_retained(first.clone()).unwrap();
        let before = record.clone();
        first.baseline.as_mut().unwrap().input = Some(30);
        assert!(record.apply_retained(first).is_err());
        assert_eq!(record, before);
    }
}
