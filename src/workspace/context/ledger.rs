//! Pure offline evidence compaction. Outgoing run facts exclude local approval and partner UI controls.
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::{Excerpt, Freshness, Observation};
use crate::workspace::{
    runner::{RunObservation, RunRecord},
    workflows,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ledger {
    /// Exact deterministic outgoing preview bytes; timing is deliberately outside this text.
    pub text: String,
    pub metrics: Metrics,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Metrics {
    pub elapsed_processing_us: u128,
    /// UTF-8 bytes of the canonical JSON input (including task and explicit observations).
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub tokenizer: String,
    pub input_tokens: usize,
    pub output_tokens: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceEvidence {
    pub reference: workflows::EvidenceRef,
    pub observed_status: String,
    pub observation_detail: Option<String>,
}

#[derive(Serialize)]
pub struct SupersededDecision<'a> {
    pub criterion: &'a str,
    pub decision_id: &'a str,
    pub decision_text: &'a str,
    pub superseded_by: &'a str,
}

#[derive(Serialize)]
pub struct EvidenceLedger<'a> {
    pub format: &'static str,
    pub task_id: i64,
    pub task_revision: u64,
    pub goal: &'a str,
    pub requirements: &'a [crate::store::Criterion],
    pub current_approved_decision: Option<&'a workflows::ChosenDecision>,
    pub superseded_decisions: Vec<SupersededDecision<'a>>,
    /// Retained even if there is no current decision; provenance alone is not approval.
    pub criterion_origins: &'a [workflows::CriterionOrigin],
    /// Immutable source payloads, serialized once for exact frozen-wire receipts.
    pub captures: Vec<&'a Excerpt>,
    pub source_observations: Vec<SourceEvidence>,
    /// Historical facts, including failures and incomplete output flags, without local controls.
    pub run_observations: Vec<RunObservation<'a>>,
    pub open_hypotheses_inferences: &'a [workflows::Hypothesis],
    pub next_action: &'a str,
    pub evidence_limits: &'static [&'static str],
}

/// No IO, model invocation, persistence, approval transfer or outgoing mutation.
/// A caller can cancel by discarding this value; explicit preview/apply owns publication.
pub fn compact(
    task: &crate::store::Snapshot,
    captures: &[Excerpt],
    runs: &[RunRecord],
    observations: &[Observation],
) -> Result<Ledger, String> {
    let started = Instant::now();
    if task.id <= 0 {
        return Err("Compaction requires a committed task identity".into());
    }
    let mut captures: Vec<_> = captures.iter().collect();
    captures.sort_by_key(|capture| capture.id);
    let mut ids = BTreeSet::new();
    for capture in &captures {
        capture.validate()?;
        if capture.task_id != task.id || capture.id <= 0 || !ids.insert(capture.id) {
            return Err("Compaction requires unique committed task-owned captures".into());
        }
    }
    let mut observed = BTreeMap::new();
    for observation in observations {
        if !ids.contains(&observation.id) || observed.insert(observation.id, observation).is_some()
        {
            return Err(
                "Compaction observation has an unknown or duplicate capture identity".into(),
            );
        }
    }
    let mut runs: Vec<_> = runs.iter().collect();
    runs.sort_by(|a, b| a.prepared.run_id.cmp(&b.prepared.run_id));
    let mut run_ids = BTreeSet::new();
    for run in &runs {
        workflows::EvidenceRef::run(run)?.validate(task.id)?;
        if !run_ids.insert(run.prepared.run_id.as_str()) {
            return Err("Compaction requires unique run identities".into());
        }
    }
    let run_observations: Vec<_> = runs.into_iter().map(RunObservation::from).collect();
    let source_observations = captures
        .iter()
        .map(|capture| {
            let (status, detail) = if !capture.available {
                (
                    "unavailable",
                    Some("Capture payload is unavailable".to_owned()),
                )
            } else {
                match observed
                    .get(&capture.id)
                    .map(|observation| &observation.freshness)
                {
                    Some(Freshness::Current) => ("current_at_last_explicit_observation", None),
                    Some(Freshness::Stale(reason)) => ("changed", Some(reason.clone())),
                    Some(Freshness::Unavailable(reason)) => ("unavailable", Some(reason.clone())),
                    None => ("not_observed", None),
                }
            };
            SourceEvidence {
                reference: workflows::EvidenceRef::Source {
                    task_id: task.id,
                    excerpt_id: capture.id,
                    path: capture.path.clone(),
                    start_line: capture.start_line,
                    end_line: capture.end_line,
                    source_sha256: capture.source_sha256,
                },
                observed_status: status.into(),
                observation_detail: detail,
            }
        })
        .collect();
    let current = task.workflows.chosen_decision.as_ref();
    let superseded_decisions = task
        .workflows
        .criterion_origins
        .iter()
        .filter_map(|origin| {
            let chosen = current?;
            (origin.decision_id != chosen.id || origin.decision_text != chosen.text).then(|| {
                SupersededDecision {
                    criterion: &origin.criterion,
                    decision_id: &origin.decision_id,
                    decision_text: &origin.decision_text,
                    superseded_by: &chosen.id,
                }
            })
        })
        .collect();
    let observed_input: Vec<_> = observed
        .values()
        .map(|observation| {
            let (status, detail) = match &observation.freshness {
                Freshness::Current => ("current", None),
                Freshness::Stale(detail) => ("changed", Some(detail)),
                Freshness::Unavailable(detail) => ("unavailable", Some(detail)),
            };
            serde_json::json!({"id": observation.id, "status": status, "detail": detail})
        })
        .collect();
    let input = serde_json::to_string(&(task, &captures, &run_observations, observed_input))
        .map_err(|error| error.to_string())?;
    let body = EvidenceLedger {
        format: "eevee-evidence-ledger-v1",
        task_id: task.id,
        task_revision: task.revision,
        goal: &task.goal,
        requirements: &task.criteria,
        current_approved_decision: current,
        superseded_decisions,
        criterion_origins: &task.workflows.criterion_origins,
        captures,
        source_observations,
        run_observations,
        open_hypotheses_inferences: &task.workflows.hypotheses,
        next_action: &task.next_action,
        evidence_limits: &[
            "Historical approvals, criteria marked met, and saved decisions are not execution grants or current verification.",
            "Source freshness is only the supplied explicit observation; immutable capture bytes are never refreshed here.",
            "Run output bytes are not supplied by RunRecord; captured/discarded byte counts are metadata, not output evidence.",
            "ProviderRecord text is provider-authored inference, never observed execution evidence; no provider records were supplied to this compaction.",
            "Hypotheses remain inferences; their references are preserved, not automatically resolved or promoted to observations.",
        ],
    };
    let text = serde_json::to_string(&body).map_err(|error| error.to_string())?;
    let input_tokens = super::optimizer::token_count(&input);
    let output_tokens = super::optimizer::token_count(&text);
    let metrics = Metrics {
        elapsed_processing_us: started.elapsed().as_micros(),
        input_bytes: input.len(),
        output_bytes: text.len(),
        input_tokens,
        output_tokens,
        tokenizer: super::optimizer::TOKENIZER.into(),
    };
    Ok(Ledger { text, metrics })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::runner;

    fn fixture() -> (
        crate::store::Snapshot,
        Vec<Excerpt>,
        Vec<RunRecord>,
        Vec<Observation>,
    ) {
        let task = serde_json::from_value(serde_json::json!({
            "project": "/offline/project", "goal": "Retain failures while changing strategy",
            "form": 0, "revision": 9, "id": 7,
            "criteria": [{"text": "offline check passes", "met": false}],
            "next_action": "Fix the failed check with the new strategy",
            "workflows": {
                "chosen_decision": {"id": "latest", "text": "Use bounded parsing"},
                "criterion_origins": [
                    {"criterion": "offline check passes", "decision_id": "prior", "decision_text": "Use unbounded parsing"}
                ],
                "hypotheses": [{"id": 1, "explanation": "The old parser caused the failure", "evidence": [
                    {"relation": "Supports", "reference": {"Run": {"task_id": 7, "run_id": "fixture-run"}}}
                ]}]
            },
            "complete": false, "navigation": crate::store::Navigation::default(),
            "updated_at": 0, "archived": false, "archived_at": 0, "redacted": false
        })).unwrap();
        let captures = vec![Excerpt {
            id: 1,
            task_id: 7,
            root: "/offline/project".into(),
            path: "parser.rs".into(),
            start_line: 1,
            end_line: 1,
            source_sha256: [1; 32],
            text: "fn old_parser() {}\n".into(),
            available: true,
            origin_run: None,
        }];
        let prepared = runner::Prepared {
            request_id: "fixture-request".into(),
            run_id: "fixture-run".into(),
            task_id: 7,
            spec: runner::CommandSpec::Direct {
                program: "local-check".into(),
                args: vec!["--offline".into()],
            },
            cwd: "/offline/project".into(),
            cwd_identity: "fixture-root".into(),
            executable: runner::ExecutableIdentity {
                path: "/offline/bin/local-check".into(),
                sha256: "0".repeat(64),
                bytes: 1,
                modified_ns: 0,
                file_id: "fixture-file".into(),
            },
            prepared_at_ms: 1,
            expires_at_ms: 200,
            approval_arrival: 42,
            before: runner::InputSnapshot {
                fingerprint: Some("before-source-hash".into()),
                complete: true,
                stable: true,
            },
            partner: Some(crate::store::Attribution {
                partner: 19,
                name: "private-partner-display-name".into(),
                form: 3,
            }),
            rerun_of: Some("prior-failed-run".into()),
        };
        let mut failed = RunRecord::awaiting(&prepared);
        failed.state = runner::RunState::Failed;
        failed.exit_code = Some(1);
        failed.error = Some("parser rejected boundary input".into());
        failed.started_at_ms = Some(101);
        failed.ended_at_ms = Some(109);
        failed.signal = Some(9);
        failed.tree = runner::TreeOutcome::OwnedTreeExited;
        failed.truncated = true;
        failed.output_unavailable = true;
        failed.revision_moving = true;
        failed.captured_bytes = 23;
        failed.discarded_bytes = 17;
        failed.dropped_live_bytes = 11;
        failed.after = runner::InputSnapshot {
            fingerprint: Some("after-source-hash".into()),
            complete: false,
            stable: false,
        };
        let observations = vec![Observation {
            id: 1,
            freshness: Freshness::Stale("source digest changed".into()),
        }];
        (task, captures, vec![failed], observations)
    }

    #[test]
    fn conflicting_decision_failed_check_and_changed_source_survive_offline() {
        let (task, captures, runs, observations) = fixture();
        let original = serde_json::to_string(&(&task, &captures, &runs)).unwrap();
        let ledger = compact(&task, &captures, &runs, &observations).unwrap();
        let body: serde_json::Value = serde_json::from_str(&ledger.text).unwrap();
        assert_eq!(body["current_approved_decision"]["id"], "latest");
        assert_eq!(body["superseded_decisions"][0]["decision_id"], "prior");
        assert_eq!(body["superseded_decisions"][0]["superseded_by"], "latest");
        assert_eq!(body["criterion_origins"][0]["decision_id"], "prior");
        assert_eq!(body["run_observations"][0]["state"], "Failed");
        assert_eq!(
            body["run_observations"][0]["error"],
            "parser rejected boundary input"
        );
        assert_eq!(body["source_observations"][0]["observed_status"], "changed");
        assert_eq!(
            body["source_observations"][0]["observation_detail"],
            "source digest changed"
        );
        assert_eq!(body["captures"], serde_json::to_value(&captures).unwrap());
        assert_eq!(body["captures"].as_array().unwrap().len(), captures.len());
        assert_eq!(
            body["open_hypotheses_inferences"].as_array().unwrap().len(),
            1
        );
        assert_eq!(body["next_action"], task.next_action);
        assert_eq!(ledger.metrics.output_bytes, ledger.text.len());
        assert_eq!(
            ledger.metrics.output_tokens,
            super::super::optimizer::token_count(&ledger.text)
        );
        assert_eq!(
            ledger.text,
            compact(&task, &captures, &runs, &observations)
                .unwrap()
                .text
        );
        // Cancelling/discarding a constructed preview cannot mutate any input.
        drop(ledger);
        assert_eq!(
            serde_json::to_string(&(&task, &captures, &runs)).unwrap(),
            original
        );
    }

    #[test]
    fn rejects_cross_task_and_ambiguous_evidence() {
        let (task, mut captures, mut runs, mut observations) = fixture();
        captures[0].task_id += 1;
        assert!(compact(&task, &captures, &runs, &observations).is_err());
        captures[0].task_id = task.id;
        observations.push(observations[0].clone());
        assert!(compact(&task, &captures, &runs, &observations).is_err());
        observations.pop();
        runs.push(runs[0].clone());
        assert!(compact(&task, &captures, &runs, &observations).is_err());
        runs.pop();
        runs[0].prepared.task_id += 1;
        assert!(compact(&task, &captures, &runs, &observations).is_err());
    }

    #[test]
    fn absent_observation_never_implies_freshness_or_approval() {
        let (mut task, captures, runs, _) = fixture();
        task.workflows.chosen_decision = None;
        let ledger = compact(&task, &captures, &runs, &[]).unwrap();
        let body: serde_json::Value = serde_json::from_str(&ledger.text).unwrap();
        assert!(body["current_approved_decision"].is_null());
        assert!(body["superseded_decisions"].as_array().unwrap().is_empty());
        assert_eq!(body["criterion_origins"].as_array().unwrap().len(), 1);
        assert_eq!(
            body["source_observations"][0]["observed_status"],
            "not_observed"
        );
    }

    #[test]
    fn wire_observations_retain_exact_facts_without_local_controls_or_flavor() {
        fn assert_no_controls(value: &serde_json::Value) {
            match value {
                serde_json::Value::Object(fields) => {
                    for (key, value) in fields {
                        assert!(
                            !matches!(
                                key.as_str(),
                                "prepared"
                                    | "spec"
                                    | "partner"
                                    | "name"
                                    | "form"
                                    | "flavor"
                                    | "cwd_identity"
                                    | "prepared_at_ms"
                                    | "expires_at_ms"
                                    | "approval_arrival"
                            ),
                            "local control leaked into outgoing evidence: {key}"
                        );
                        assert_no_controls(value);
                    }
                }
                serde_json::Value::Array(values) => {
                    for value in values {
                        assert_no_controls(value);
                    }
                }
                _ => {}
            }
        }

        let (task, captures, mut runs, observations) = fixture();
        let ledger = compact(&task, &captures, &runs, &observations).unwrap();
        let body: serde_json::Value = serde_json::from_str(&ledger.text).unwrap();
        let runs_on_wire = body["run_observations"].as_array().unwrap();
        assert_eq!(runs_on_wire.len(), 1);
        let run = &runs_on_wire[0];
        assert_eq!(run["task_id"], 7);
        assert_eq!(run["partner_id"], 19);
        assert_eq!(run["state"], "Failed");
        assert_eq!(run["exit_code"], 1);
        assert_eq!(run["error"], "parser rejected boundary input");
        assert!(run["started_at_ms"].as_u64().unwrap() < run["ended_at_ms"].as_u64().unwrap());
        assert_ne!(run["before"]["fingerprint"], run["after"]["fingerprint"]);
        assert_eq!(run["before"]["stable"], true);
        assert_eq!(run["after"]["stable"], false);
        assert_eq!(run["revision_moving"], true);
        assert_eq!(run["truncated"], true);
        assert_eq!(run["output_unavailable"], true);
        assert_eq!(run["captured_bytes"], 23);
        assert_eq!(run["discarded_bytes"], 17);
        assert_eq!(run["dropped_live_bytes"], 11);
        assert_eq!(run["rerun_of"], "prior-failed-run");
        assert_no_controls(&body);
        assert!(!ledger.text.contains("private-partner-display-name"));
        assert!(!ledger.text.contains("fixture-root"));

        // Local approval/UI changes must affect neither outgoing bytes nor input accounting.
        let prepared = &mut runs[0].prepared;
        prepared.cwd_identity = "different-private-root-identity".into();
        prepared.prepared_at_ms = 987_654;
        prepared.expires_at_ms = 999_999;
        prepared.approval_arrival = 777;
        let partner = prepared.partner.as_mut().unwrap();
        partner.name = "another-private-display-name-with-different-length".into();
        partner.form = 1;
        let changed = compact(&task, &captures, &runs, &observations).unwrap();
        assert_eq!(changed.text, ledger.text);
        assert_eq!(changed.metrics.input_bytes, ledger.metrics.input_bytes);
        assert_eq!(changed.metrics.input_tokens, ledger.metrics.input_tokens);
    }

    #[test]
    fn borrowed_runs_are_sorted_without_changing_input_order() {
        let (task, captures, mut runs, observations) = fixture();
        let mut earlier = runs[0].clone();
        earlier.prepared.run_id = "aaa-earlier-run".into();
        runs.push(earlier);
        let ledger = compact(&task, &captures, &runs, &observations).unwrap();
        let body: serde_json::Value = serde_json::from_str(&ledger.text).unwrap();
        assert_eq!(body["run_observations"].as_array().unwrap().len(), 2);
        assert_eq!(body["run_observations"][0]["run_id"], "aaa-earlier-run");
        assert_eq!(body["run_observations"][1]["run_id"], "fixture-run");
        assert_eq!(runs[0].prepared.run_id, "fixture-run");
        assert_eq!(runs[1].prepared.run_id, "aaa-earlier-run");
        runs.reverse();
        let reordered = compact(&task, &captures, &runs, &observations).unwrap();
        assert_eq!(reordered.text, ledger.text);
        assert_eq!(reordered.metrics.input_bytes, ledger.metrics.input_bytes);
        assert_eq!(reordered.metrics.input_tokens, ledger.metrics.input_tokens);
    }
}
