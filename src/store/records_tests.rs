use super::*;
use crate::workspace::runner::{
    CommandSpec, ExecutableIdentity, InputSnapshot, Prepared, RunRecord, RunState, TreeOutcome,
};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn generated_activity_is_projected_before_storage_and_reopens_safely() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let secret = "sk-activity-fixture-private-value";
    let detail = format!("Command [\"password={secret}\"]: failed");
    activity(
        state.connection.as_ref().unwrap(),
        &fixture.project,
        &Activity {
            id: "privacy-boundary".into(),
            task_id: task.id,
            owner: None,
            at: timestamp().unwrap(),
            detail,
        },
    )
    .unwrap();
    let stored: String = state
        .connection
        .as_ref()
        .unwrap()
        .query_row(
            "SELECT record FROM local_activity WHERE id='privacy-boundary'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!stored.contains(secret));
    drop(state);
    let reopened = fixture.open();
    let persisted = rows::<Activity>(
        reopened.connection.as_ref().unwrap(),
        "local_activity",
        &fixture.project,
    )
    .unwrap();
    assert!(persisted[0].detail.ends_with("failed"));
}
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    project: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "ev-records-{}-{}-{}",
            std::process::id(),
            timestamp().unwrap(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("project")).unwrap();
        let root = fs::canonicalize(root).unwrap();
        Self {
            project: root.join("project").to_str().unwrap().into(),
            root,
        }
    }
    fn open(&self) -> State {
        let mut state = State {
            data_dir: self.root.join("data"),
            project: self.project.clone(),
            policy: Policy::default(),
            connection: None,
            lock: None,
        };
        state.open_and_load().unwrap();
        state
    }
    fn task(&self, state: &mut State) -> super::super::Snapshot {
        let mut task = super::super::Snapshot::empty(self.project.clone());
        task.goal = "Preserve factual local records".into();
        task.form = 5;
        task.revision = 1;
        state.save(task).unwrap()
    }
    fn capture(
        &self,
        state: &mut State,
        task: i64,
        producer: Option<Attribution>,
        start: usize,
        end: usize,
    ) -> crate::workspace::context::Excerpt {
        use sha2::{Digest, Sha256};
        let bytes = fs::read(self.root.join("project/source.rs")).unwrap();
        let excerpt = crate::workspace::context::Excerpt {
            id: 0,
            task_id: task,
            root: self.project.clone(),
            path: "source.rs".into(),
            start_line: start,
            end_line: end,
            source_sha256: Sha256::digest(&bytes).into(),
            text: String::from_utf8(bytes)
                .unwrap()
                .lines()
                .skip(start - 1)
                .take(end - start + 1)
                .collect::<Vec<_>>()
                .join("\n"),
            available: true,
            origin_run: None,
        };
        state
            .context(ContextRequest {
                task_id: task,
                producer,
                operation: ContextOperation::Pin(excerpt),
            })
            .unwrap()
            .pop()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn observed(revision: &str) -> InputSnapshot {
    InputSnapshot {
        fingerprint: Some(revision.into()),
        complete: true,
        stable: true,
    }
}
fn owner(state: &State, id: i64) -> Attribution {
    Attribution::of(
        &partners(state.connection.as_ref().unwrap())
            .unwrap()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap(),
    )
}
fn call(state: &mut State, task: i64, operation: Operation) -> Result<Response, String> {
    state.records(&Request {
        task_id: task,
        operation,
    })
}
fn load(
    state: &mut State,
    task: i64,
    current: Option<InputSnapshot>,
) -> crate::workspace::records::Snapshot {
    match call(state, task, Operation::Load { current }).unwrap() {
        Response::Snapshot(snapshot) => *snapshot,
        other => panic!("Unexpected response {other:?}"),
    }
}
fn terminal(
    state: &mut State,
    task: i64,
    id: &str,
    revision: &str,
    passed: bool,
    output: &[u8],
) -> RunRecord {
    let now = timestamp().unwrap() as u64;
    let prepared = Prepared {
        request_id: format!("request-{id}"),
        run_id: format!("run-{id}"),
        task_id: task,
        spec: CommandSpec::Direct {
            program: "cargo".into(),
            args: vec!["test".into()],
        },
        cwd: PathBuf::from(&state.project),
        cwd_identity: "fixture-directory".into(),
        executable: ExecutableIdentity {
            path: PathBuf::from(&state.project).join("cargo"),
            sha256: "0".repeat(64),
            bytes: 1,
            modified_ns: 1,
            file_id: "fixture-executable".into(),
        },
        prepared_at_ms: now,
        expires_at_ms: now + 60_000,
        approval_arrival: 1,
        before: observed(revision),
        partner: Some(owner(state, 1)),
        rerun_of: None,
    };
    let capacity = (output.len().max(1024) as u64).next_power_of_two();
    let mut permit = state.reserve_run(task, prepared.clone(), capacity).unwrap();
    permit.file.write_all(output).unwrap();
    permit.file.sync_all().unwrap();
    drop(permit);
    let mut run = RunRecord::awaiting(&prepared);
    run.state = if passed {
        RunState::Succeeded
    } else {
        RunState::Failed
    };
    run.started_at_ms = Some(timestamp().unwrap() as u64);
    run.ended_at_ms = Some(timestamp().unwrap() as u64);
    run.exit_code = Some(if passed { 0 } else { 101 });
    run.tree = TreeOutcome::OwnedTreeExited;
    run.captured_bytes = output.len() as u64;
    run.after = observed(revision);
    let run = state.update_run(task, run).unwrap();
    let tx = state.connection.as_mut().unwrap().transaction().unwrap();
    let stored = stored_run(&tx, task, &run.prepared.run_id).unwrap();
    observe_run(&tx, &state.data_dir, &state.project, &stored).unwrap();
    tx.commit().unwrap();
    run
}
fn claim(state: &mut State, task: &super::super::Snapshot, id: &str) -> WorkClaim {
    let claim = WorkClaim {
        task_id: task.id,
        owner: owner(state, 1),
        request_id: id.into(),
        writing: false,
    };
    state
        .work_request(&WorkRequest::Acquire {
            claim: claim.clone(),
            task: Box::new(task.clone()),
        })
        .unwrap();
    claim
}
fn release(state: &mut State, claim: WorkClaim) {
    state.work_request(&WorkRequest::Release(claim)).unwrap();
}

#[test]
fn captures_keep_exact_ids_frozen_producer_and_range_after_reopen_and_removal() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("project/source.rs"), "one\ntwo\nthree\n").unwrap();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let frozen = owner(&state, 1);
    let first = fixture.capture(&mut state, task.id, Some(frozen.clone()), 1, 1);
    let second = fixture.capture(&mut state, task.id, Some(frozen.clone()), 2, 3);
    assert_ne!(first.id, second.id);
    let mut updated = task.clone();
    updated.form = 2;
    updated.revision += 1;
    state.save(updated).unwrap();
    drop(state);
    let mut state = fixture.open();
    let basket = state
        .context(ContextRequest {
            task_id: task.id,
            producer: None,
            operation: ContextOperation::Load,
        })
        .unwrap();
    assert_eq!(basket, vec![first.clone(), second.clone()]);
    let records = load(&mut state, task.id, None);
    assert_eq!(records.artifacts.len(), 2);
    assert!(
        records
            .artifacts
            .iter()
            .all(|a| a.artifact.producer.as_ref() == Some(&frozen))
    );
    state
        .context(ContextRequest {
            task_id: task.id,
            producer: None,
            operation: ContextOperation::Remove(second.id),
        })
        .unwrap();
    drop(state);
    let mut state = fixture.open();
    let basket = state
        .context(ContextRequest {
            task_id: task.id,
            producer: None,
            operation: ContextOperation::Load,
        })
        .unwrap();
    assert_eq!(basket, vec![first]);
    assert_eq!(load(&mut state, task.id, None).artifacts.len(), 1);
}

#[test]
fn deduped_failures_scan_all_spool_pages_and_lines_deliver_once_across_restart() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let mut bytes = vec![b'\n'; 65_533];
    bytes.extend_from_slice(b"error[E0308]: no output is copied\n  --> src/lib.rs:12:9\ntest tests::broken ... FAILED\nthread 'tests::broken' panicked at src/lib.rs:12:9:\n");
    let first = terminal(&mut state, task.id, "first", "old", false, &bytes);
    let records = load(&mut state, task.id, Some(observed("old")));
    assert_eq!(records.signatures.len(), 3);
    assert!(records.signatures.iter().all(|s| s.seen_count == 1));
    assert!(records.signatures.iter().all(|signature| {
        signature.first_seen == first.ended_at_ms.unwrap().min(i64::MAX as u64) as i64
    }));
    assert_eq!(records.activities.len(), 1);
    assert!(records.signatures.iter().all(|signature| {
        signature.paths.len() == 1 && Path::new(&signature.paths[0]) == Path::new("src/lib.rs")
    }));
    call(
        &mut state,
        task.id,
        Operation::ObserveRun {
            run_id: first.prepared.run_id.clone(),
        },
    )
    .unwrap();
    assert!(load(&mut state, task.id, None).activities.is_empty());
    terminal(&mut state, task.id, "second", "old", false, &bytes);
    let records = load(&mut state, task.id, None);
    assert!(records.signatures.iter().all(|s| s.seen_count == 2));
    assert_eq!(records.activities.len(), 1);
    terminal(&mut state, task.id, "baseline", "old", true, b"ok\n");
    drop(state);
    let mut state = fixture.open();
    let records = load(&mut state, task.id, None);
    assert!(records.activities.is_empty());
    assert!(records.signatures.iter().all(|s| s.seen_count == 2));
    call(&mut state, 0, Operation::SetFlavor { enabled: false }).unwrap();
    terminal(&mut state, task.id, "off", "old", true, b"ok\n");
    let enabled = match call(&mut state, 0, Operation::SetFlavor { enabled: true }).unwrap() {
        Response::Snapshot(s) => s,
        _ => unreachable!(),
    };
    assert!(enabled.activities.is_empty());
    let before = load(&mut state, task.id, Some(observed("old")));
    assert!(before.activities.is_empty());
    let stale = load(&mut state, task.id, Some(observed("new")));
    assert_eq!(stale.activities.len(), 4);
    assert!(
        load(&mut state, task.id, Some(observed("new")))
            .activities
            .is_empty()
    );
    state.purge_output(task.id, &first.prepared.run_id).unwrap();
    call(
        &mut state,
        task.id,
        Operation::ObserveRun {
            run_id: first.prepared.run_id.clone(),
        },
    )
    .unwrap();
    let records = load(&mut state, task.id, None);
    assert!(records.signatures.iter().all(|s| s.seen_count == 1));
    assert!(
        !records
            .artifacts
            .iter()
            .any(|a| a.artifact.reference == first.prepared.run_id)
    );
    drop(state);
    let mut other = State {
        data_dir: fixture.root.join("data"),
        project: fixture.root.join("other").to_str().unwrap().into(),
        policy: Policy::default(),
        connection: None,
        lock: None,
    };
    other.open_and_load().unwrap();
    assert!(load(&mut other, 0, None).signatures.is_empty());
    assert!(
        call(
            &mut other,
            task.id,
            Operation::ObserveRun {
                run_id: first.prepared.run_id
            }
        )
        .is_err()
    );
}

#[test]
fn routes_ack_exact_local_intents_are_not_serialized_and_rollback_atomically() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let mut changed = task.clone();
    changed.revision += 1;
    changed.form = 2;
    let mut attribution = owner(&state, 1);
    attribution.form = 1;
    let one = LocalIntent::FormRoute {
        id: "route-one".into(),
        owner: attribution.clone(),
        from: 5,
        to: 1,
        accepted_suggestion: Some("explicit-suggestion".into()),
    };
    attribution.form = 2;
    let two = LocalIntent::FormRoute {
        id: "route-two".into(),
        owner: attribution.clone(),
        from: 1,
        to: 2,
        accepted_suggestion: None,
    };
    changed.local_records = vec![one.clone(), two.clone()];
    let serialized = serde_json::to_string(&changed).unwrap();
    assert!(!serialized.contains("route-one"));
    assert!(!serialized.contains("local_records"));
    let saved = state.save(changed.clone()).unwrap();
    assert_eq!(saved.local_records, vec![one.clone(), two.clone()]);
    let loaded = load_task(
        state.connection.as_ref().unwrap(),
        &fixture.project,
        task.id,
    )
    .unwrap()
    .unwrap();
    assert!(loaded.local_records.is_empty());
    let routes = load(&mut state, task.id, None).routes;
    assert_eq!(routes.len(), 2);
    assert!(routes[0].ordinal < routes[1].ordinal);
    let retry = state.save(changed).unwrap();
    assert_eq!(retry.local_records, vec![one, two]);
    assert_eq!(load(&mut state, task.id, None).routes, routes);
    let loaded = load_task(
        state.connection.as_ref().unwrap(),
        &fixture.project,
        task.id,
    )
    .unwrap()
    .unwrap();
    let mut rejected = loaded.clone();
    rejected.revision += 1;
    rejected.goal = "must roll back".into();
    rejected.form = 3;
    attribution.form = 3;
    rejected.local_records = vec![
        LocalIntent::FormRoute {
            id: "rollback-route".into(),
            owner: attribution.clone(),
            from: 2,
            to: 3,
            accepted_suggestion: None,
        },
        LocalIntent::FormRoute {
            id: "invalid-route".into(),
            owner: attribution,
            from: 3,
            to: 99,
            accepted_suggestion: None,
        },
    ];
    assert!(state.save(rejected).is_err());
    assert_eq!(
        load_task(
            state.connection.as_ref().unwrap(),
            &fixture.project,
            task.id
        )
        .unwrap()
        .unwrap(),
        loaded
    );
    assert_eq!(load(&mut state, task.id, None).routes, routes);
}

#[test]
fn habits_count_dispatched_requests_not_retries_masked_or_foreign_instructions() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let observation = |id: &str, owner: Attribution, text: &str| Operation::ObserveInstruction {
        request_id: id.into(),
        owner,
        instruction: text.into(),
    };
    let unclaimed_owner = owner(&state, 1);
    assert!(
        call(
            &mut state,
            task.id,
            observation("not-dispatched", unclaimed_owner, "Use focused tests.")
        )
        .is_err()
    );
    for id in ["one", "two", "three"] {
        let claim = claim(&mut state, &task, id);
        for _ in 0..2 {
            call(
                &mut state,
                task.id,
                observation(id, claim.owner.clone(), " Use focused tests.\r\n"),
            )
            .unwrap();
        }
        release(&mut state, claim);
    }
    let records = load(&mut state, task.id, None);
    assert_eq!(records.habits.len(), 1);
    assert_eq!(records.habits[0].sources.len(), 3);
    let hash = records.habits[0].key.clone();
    let claim = claim(&mut state, &task, "secret");
    assert!(
        call(
            &mut state,
            task.id,
            observation("secret", claim.owner.clone(), "password=secret-value")
        )
        .is_err()
    );
    release(&mut state, claim);
    call(
        &mut state,
        0,
        Operation::DismissInstruction { key: hash.clone() },
    )
    .unwrap();
    drop(state);
    let mut state = fixture.open();
    let records = load(&mut state, task.id, None);
    assert!(records.habits[0].dismissed);
    assert_eq!(records.habits[0].key, hash);
    assert_eq!(records.habits[0].sources.len(), 3);
    state
        .journal_request(JournalRequest {
            task_id: task.id,
            operation: JournalOperation::Purge,
        })
        .unwrap();
    assert!(load(&mut state, task.id, None).habits[0].sources.is_empty());
}

#[test]
fn pairings_remain_project_partner_form_scoped_after_restart_remove_release_and_purge() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let second = state
        .add_partner()
        .unwrap()
        .into_iter()
        .find(|p| p.id != 1)
        .unwrap()
        .id;
    let pairing = |partner, form, agent, model: &str| Pairing {
        partner,
        form,
        agent,
        selection: crate::providers::ModelSelection {
            model: Some(model.into()),
            effort: Some("high".into()),
        },
    };
    let first = pairing(1, 5, crate::providers::Agent::Codex, "model-a");
    let other = pairing(second, 2, crate::providers::Agent::Claude, "model-b");
    for p in [&first, &other] {
        call(
            &mut state,
            0,
            Operation::SetPairing {
                partner: p.partner,
                form: p.form,
                selection: Some(p.clone()),
            },
        )
        .unwrap();
    }
    drop(state);
    let mut state = fixture.open();
    assert_eq!(
        load(&mut state, task.id, None).pairings,
        vec![first.clone(), other.clone()]
    );
    call(
        &mut state,
        0,
        Operation::SetPairing {
            partner: first.partner,
            form: first.form,
            selection: None,
        },
    )
    .unwrap();
    assert_eq!(load(&mut state, task.id, None).pairings, vec![other]);
    state.release_partner(second).unwrap();
    assert!(load(&mut state, task.id, None).pairings.is_empty());
    assert!(
        call(
            &mut state,
            0,
            Operation::SetPairing {
                partner: second,
                form: 2,
                selection: Some(pairing(
                    second,
                    2,
                    crate::providers::Agent::Claude,
                    "model-b"
                ))
            }
        )
        .is_err()
    );
    call(
        &mut state,
        0,
        Operation::SetPairing {
            partner: first.partner,
            form: first.form,
            selection: Some(first),
        },
    )
    .unwrap();
    state
        .manage(Management::Delete {
            ids: vec![task.id],
            expired_before: None,
        })
        .unwrap();
    assert!(load(&mut state, 0, None).pairings.is_empty());
}

#[test]
fn historical_resolution_requires_supported_receipt_observed_transition_and_matching_pass() {
    use workflows::{EvidenceRef, EvidenceRelation, Hypothesis, HypothesisEvidence};
    let fixture = Fixture::new();
    fs::write(fixture.root.join("project/source.rs"), "broken\n").unwrap();
    let mut state = fixture.open();
    let mut task = fixture.task(&mut state);
    let producing_owner = owner(&state, 1);
    let excerpt = fixture.capture(&mut state, task.id, Some(producing_owner.clone()), 1, 1);
    let failed = terminal(
        &mut state,
        task.id,
        "diagnostic",
        "old",
        false,
        b"test tests::broken ... FAILED\n",
    );
    task.revision += 1;
    task.workflows.hypotheses.push(Hypothesis {
        id: 1,
        explanation: "The captured implementation causes this observed failure".into(),
        evidence: vec![
            HypothesisEvidence {
                relation: EvidenceRelation::Supports,
                reference: EvidenceRef::source(&excerpt).unwrap(),
            },
            HypothesisEvidence {
                relation: EvidenceRelation::Supports,
                reference: EvidenceRef::run(&failed).unwrap(),
            },
        ],
    });
    task.local_records.push(LocalIntent::Artifact {
        id: "supported-diagnosis".into(),
        owner: producing_owner,
        kind: ArtifactKind::Hypothesis,
        reference: "1".into(),
        current: Some(observed("old")),
    });
    state.save(task.clone()).unwrap();
    let records = load(&mut state, task.id, Some(observed("old")));
    assert!(
        records
            .artifacts
            .iter()
            .any(|a| a.artifact.id == "supported-diagnosis" && a.artifact.supported_diagnosis)
    );
    let signature = records.signatures[0].key.clone();
    assert!(records.resolution_choices.is_empty());
    let same = terminal(&mut state, task.id, "same-revision", "old", true, b"ok\n");
    assert!(
        call(
            &mut state,
            task.id,
            Operation::ResolveFailure {
                signature: signature.clone(),
                support_artifact: "supported-diagnosis".into(),
                passing_run: same.prepared.run_id
            }
        )
        .is_err()
    );
    fs::write(fixture.root.join("project/source.rs"), "fixed\n").unwrap();
    call(
        &mut state,
        task.id,
        Operation::Load {
            current: Some(observed("fixed")),
        },
    )
    .unwrap();
    let passed = terminal(&mut state, task.id, "fixed", "fixed", true, b"ok\n");
    let choices = load(&mut state, task.id, Some(observed("fixed"))).resolution_choices;
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].passing_run, passed.prepared.run_id);
    let resolution = Operation::ResolveFailure {
        signature: signature.clone(),
        support_artifact: "supported-diagnosis".into(),
        passing_run: passed.prepared.run_id.clone(),
    };
    call(&mut state, task.id, resolution.clone()).unwrap();
    call(&mut state, task.id, resolution).unwrap();
    assert_eq!(load(&mut state, task.id, None).resolutions.len(), 1);
    fs::write(fixture.root.join("project/source.rs"), "broken\n").unwrap();
    terminal(
        &mut state,
        task.id,
        "recurrence",
        "old",
        false,
        b"test tests::broken ... FAILED\n",
    );
    let records = load(&mut state, task.id, Some(observed("old")));
    assert_eq!(records.signatures[0].seen_count, 2);
    assert_eq!(records.resolutions.len(), 1);
    assert!(records.resolutions[0].cause.contains("Historical lead"));
    drop(state);
    let mut state = fixture.open();
    let records = load(&mut state, task.id, None);
    assert_eq!(records.resolutions.len(), 1);
    assert_eq!(records.signatures[0].seen_count, 2);
    state
        .context(ContextRequest {
            task_id: task.id,
            producer: None,
            operation: ContextOperation::Purge(vec![excerpt.id]),
        })
        .unwrap();
    assert!(load(&mut state, task.id, None).resolutions.is_empty());
    assert!(
        call(
            &mut state,
            task.id,
            Operation::ResolveFailure {
                signature,
                support_artifact: "supported-diagnosis".into(),
                passing_run: passed.prepared.run_id
            }
        )
        .is_err()
    );
}

#[test]
fn habit_binding_requires_exact_approved_project_memory_and_edited_revision_disables_it() {
    use crate::workspace::memory::{Mutation, Proposal, Provenance, Scope, SourceKind};
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    for id in ["a", "b", "c"] {
        let request = claim(&mut state, &task, id);
        call(
            &mut state,
            task.id,
            Operation::ObserveInstruction {
                request_id: id.into(),
                owner: request.owner.clone(),
                instruction: "Run focused checks.".into(),
            },
        )
        .unwrap();
        release(&mut state, request);
    }
    let habit = load(&mut state, task.id, None).habits.remove(0);
    let proposal = Proposal {
        scope: Scope::Project,
        content: habit.instruction.clone(),
        provenance: Provenance {
            kind: SourceKind::UserAuthored,
            reference: format!("habit:{}", habit.key),
            task_id: task.id,
        },
    };
    assert!(
        call(
            &mut state,
            task.id,
            Operation::BindInstructionMemory {
                key: habit.key.clone(),
                memory: 1,
                revision: 1
            }
        )
        .is_err()
    );
    let memory = state
        .memory_request(MemoryRequest {
            task_id: task.id,
            mutation: Some(Mutation::Approve(proposal.clone())),
        })
        .unwrap()
        .remove(0);
    call(
        &mut state,
        task.id,
        Operation::BindInstructionMemory {
            key: habit.key.clone(),
            memory: memory.id,
            revision: memory.revision,
        },
    )
    .unwrap();
    drop(state);
    let mut state = fixture.open();
    assert_eq!(
        load(&mut state, task.id, None).habits[0].memory,
        Some(MemoryBinding {
            id: memory.id,
            revision: memory.revision
        })
    );
    let mut edited = proposal;
    edited.content = "A different instruction.".into();
    state
        .memory_request(MemoryRequest {
            task_id: task.id,
            mutation: Some(Mutation::Edit {
                id: memory.id,
                expected_revision: memory.revision,
                proposal: edited,
            }),
        })
        .unwrap();
    assert!(load(&mut state, task.id, None).habits[0].memory.is_none());
    assert!(
        call(
            &mut state,
            task.id,
            Operation::BindInstructionMemory {
                key: habit.key,
                memory: memory.id,
                revision: memory.revision
            }
        )
        .is_err()
    );
}

#[test]
fn usage_keeps_unknown_fields_distinct_boundaries_and_exact_request_owner_after_release() {
    use crate::workspace::usage::{Counters, Identity, Kind, Record, Report, Scope, Semantics};
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let request = claim(&mut state, &task, "usage-request");
    let mut usage = Record::new(
        request.request_id.clone(),
        task.id,
        request.owner.clone(),
        crate::providers::Agent::Codex,
    );
    call(
        &mut state,
        task.id,
        Operation::Usage {
            record: usage.clone(),
        },
    )
    .unwrap();
    release(&mut state, request);
    let report = Report {
        identity: Identity {
            session: None,
            turn: None,
            response: None,
            event: None,
        },
        scope: Scope::Turn,
        kind: Kind::Snapshot,
        counters: Counters {
            input: Some(10),
            output: None,
            ..Counters::default()
        },
        baseline: None,
        model: None,
        tokenizer: None,
        semantics: Semantics::default(),
        complete: false,
    };
    usage.apply_retained(report.clone()).unwrap();
    call(
        &mut state,
        task.id,
        Operation::Usage {
            record: usage.clone(),
        },
    )
    .unwrap();
    let mut final_report = report;
    final_report.counters = Counters {
        output: Some(4),
        ..Counters::default()
    };
    final_report.identity.session = Some("session".into());
    final_report.identity.turn = Some("turn".into());
    final_report.identity.response = Some("response".into());
    final_report.model = Some("reported-model".into());
    final_report.tokenizer = Some("reported-tokenizer".into());
    final_report.complete = true;
    usage.apply_retained(final_report.clone()).unwrap();
    assert!(!usage.apply_retained(final_report).unwrap());
    call(
        &mut state,
        task.id,
        Operation::Usage {
            record: usage.clone(),
        },
    )
    .unwrap();
    call(
        &mut state,
        task.id,
        Operation::Usage {
            record: usage.clone(),
        },
    )
    .unwrap();
    let records = load(&mut state, task.id, None);
    assert_eq!(records.usages, vec![usage.clone()]);
    assert_eq!(records.usages[0].measurements.len(), 1);
    assert_eq!(
        records.usages[0].measurements[0].attributable,
        Counters {
            input: Some(10),
            output: Some(4),
            ..Counters::default()
        }
    );
    let counters = &records.usages[0].measurements[0].report.counters;
    assert_eq!(counters.input, Some(10));
    assert_eq!(counters.output, Some(4));
    assert_eq!(counters.cache_read, None);
    assert_eq!(counters.reasoning, None);
    let mut foreign = usage.clone();
    foreign.owner.name = "Different owner".into();
    assert!(call(&mut state, task.id, Operation::Usage { record: foreign }).is_err());
    for field in 0..5 {
        for replacement in [None, Some("contradictory-known-value".to_owned())] {
            let mut contradictory = usage.clone();
            let report = &mut contradictory.measurements[0].report;
            match field {
                0 => report.identity.session = replacement,
                1 => report.identity.turn = replacement,
                2 => report.identity.response = replacement,
                3 => report.model = replacement,
                _ => report.tokenizer = replacement,
            }
            assert!(
                call(
                    &mut state,
                    task.id,
                    Operation::Usage {
                        record: contradictory
                    }
                )
                .is_err()
            );
        }
    }
    let mut erased = usage.clone();
    erased.measurements.clear();
    assert!(call(&mut state, task.id, Operation::Usage { record: erased }).is_err());
    drop(state);
    let mut state = fixture.open();
    assert_eq!(load(&mut state, task.id, None).usages, vec![usage.clone()]);
    state
        .journal_request(JournalRequest {
            task_id: task.id,
            operation: JournalOperation::Purge,
        })
        .unwrap();
    assert!(load(&mut state, task.id, None).usages.is_empty());
    assert!(call(&mut state, task.id, Operation::Usage { record: usage }).is_err());
}

#[test]
fn records_queue_retains_full_authored_bytes_on_backpressure_and_disconnect() {
    let (commands, receiver) = mpsc::sync_channel(1);
    let worker = Worker {
        commands: Some(commands),
        events: None,
        thread: None,
    };
    let request = Request {
        task_id: 7,
        operation: Operation::ObserveInstruction {
            request_id: "stable-request".into(),
            owner: Attribution {
                partner: 1,
                name: "Acting partner".into(),
                form: 5,
            },
            instruction: "Authored instruction\n  with exact whitespace.".into(),
        },
    };
    let mut first = Some(request.clone());
    assert!(worker.try_records(&mut first));
    assert!(first.is_none());
    let mut blocked = Some(request.clone());
    assert!(!worker.try_records(&mut blocked));
    assert_eq!(blocked, Some(request.clone()));
    match receiver.recv().unwrap() {
        Command::Records(original) => assert_eq!(*original, request),
        _ => panic!("Wrong admitted command"),
    };
    drop(receiver);
    assert!(!worker.try_records(&mut blocked));
    assert_eq!(blocked, Some(request.clone()));
    drop(worker);
    let missing = Worker {
        commands: None,
        events: None,
        thread: None,
    };
    assert!(!missing.try_records(&mut blocked));
    assert_eq!(blocked, Some(request));
}

#[test]
fn schema15_migration_and_old_writer_admission_are_strict_and_quota_rolls_back_local_writes() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let connection = state.connection.as_mut().unwrap();
    remove_schema(connection);
    assert!(
        validate_planning_input(&state.data_dir, &task, &[])
            .unwrap_err()
            .contains("schema 15")
    );
    {
        let connection = state.connection.as_mut().unwrap();
        connection.execute_batch("VACUUM;").unwrap();
        let pages: i64 = connection
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .unwrap();
        connection
            .pragma_update(None, "max_page_count", pages + 2)
            .unwrap();
        assert!(initialize(connection).is_err());
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            14
        );
        let leaked:i64=connection.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN('local_artifacts','diagnosis_transitions','form_routes')",[],|r|r.get(0)).unwrap();
        assert_eq!(leaked, 0);
        connection
            .pragma_update(None, "max_page_count", 1_000_000)
            .unwrap();
    }
    initialize(state.connection.as_mut().unwrap()).unwrap();
    assert_eq!(
        state
            .connection
            .as_ref()
            .unwrap()
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        15
    );
    validate_planning_input(&state.data_dir, &task, &[]).unwrap();
    let old = load_task(
        state.connection.as_ref().unwrap(),
        &fixture.project,
        task.id,
    )
    .unwrap()
    .unwrap();
    state.policy.quota_bytes = 1;
    let mut update = old.clone();
    update.revision += 1;
    update.goal = "Not committed under exhausted quota".into();
    let mut producer = owner(&state, 1);
    producer.form = 2;
    update.form = 2;
    update.local_records.push(LocalIntent::FormRoute {
        id: "quota-route".into(),
        owner: producer,
        from: 5,
        to: 2,
        accepted_suggestion: None,
    });
    assert!(state.save(update).is_err());
    assert_eq!(
        load_task(
            state.connection.as_ref().unwrap(),
            &fixture.project,
            task.id
        )
        .unwrap()
        .unwrap(),
        old
    );
    let count: i64 = state
        .connection
        .as_ref()
        .unwrap()
        .query_row("SELECT count(*) FROM form_routes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn shared_record_ordinals_disambiguate_repeated_visits_and_decisions_validate_exact_next_action() {
    use sha2::{Digest, Sha256};
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let mut task = fixture.task(&mut state);
    task.revision += 1;
    task.form = 4;
    task.next_action = "Use the explicitly chosen approach".into();
    let reference = format!("decision:{:x}", Sha256::digest(task.next_action.as_bytes()));
    task.workflows.chosen_decision = Some(workflows::ChosenDecision {
        id: reference.clone(),
        text: task.next_action.clone(),
    });
    let mut acting = owner(&state, 1);
    acting.form = 4;
    let mut intermediate = acting.clone();
    intermediate.form = 2;
    task.local_records = vec![
        LocalIntent::FormRoute {
            id: "visit-first-espeon".into(),
            owner: acting.clone(),
            from: 5,
            to: 4,
            accepted_suggestion: None,
        },
        LocalIntent::Artifact {
            id: "chosen-decision".into(),
            owner: acting.clone(),
            kind: ArtifactKind::Decision,
            reference: reference.clone(),
            current: None,
        },
        LocalIntent::FormRoute {
            id: "visit-jolteon".into(),
            owner: intermediate,
            from: 4,
            to: 2,
            accepted_suggestion: None,
        },
        LocalIntent::FormRoute {
            id: "visit-second-espeon".into(),
            owner: acting,
            from: 2,
            to: 4,
            accepted_suggestion: None,
        },
    ];
    let saved = state.save(task.clone()).unwrap();
    let records = load(&mut state, task.id, None);
    let decision = &records
        .artifacts
        .iter()
        .find(|a| a.artifact.kind == ArtifactKind::Decision)
        .unwrap()
        .artifact;
    assert!(records.routes[0].ordinal < decision.ordinal);
    assert!(decision.ordinal < records.routes[1].ordinal);
    assert!(records.routes[1].ordinal < records.routes[2].ordinal);
    assert_eq!(decision.reference, reference);
    assert_eq!(records.artifacts[0].freshness, Freshness::Current);
    let first_wall_time = decision.created_at;
    state.save(task).unwrap();
    let records = load(&mut state, saved.id, None);
    assert_eq!(records.artifacts[0].artifact.created_at, first_wall_time);
    let mut changed = saved;
    changed.local_records.clear();
    changed.revision += 1;
    changed.next_action = "Different next action".into();
    state.save(changed).unwrap();
    assert_eq!(
        load(&mut state, records.artifacts[0].artifact.task_id, None).artifacts[0].freshness,
        Freshness::Stale
    );
}

#[test]
fn summary_counts_distinct_tasks_by_actual_producer_form_and_labels_shared_memories() {
    use crate::workspace::memory::{Mutation, Proposal, Provenance, Scope, SourceKind};
    let fixture = Fixture::new();
    fs::write(fixture.root.join("project/source.rs"), "one\ntwo\n").unwrap();
    let mut state = fixture.open();
    let first = fixture.task(&mut state);
    let acting = owner(&state, 1);
    fixture.capture(&mut state, first.id, Some(acting.clone()), 1, 1);
    fixture.capture(&mut state, first.id, Some(acting.clone()), 2, 2);
    let second = fixture.task(&mut state);
    fixture.capture(&mut state, second.id, Some(acting.clone()), 1, 2);
    let unknown = fixture.task(&mut state);
    fixture.capture(&mut state, unknown.id, None, 1, 1);
    state
        .memory_request(MemoryRequest {
            task_id: first.id,
            mutation: Some(Mutation::Approve(Proposal {
                scope: Scope::Project,
                content: "Explicit shared project note".into(),
                provenance: Provenance {
                    kind: SourceKind::UserAuthored,
                    reference: "user approval".into(),
                    task_id: first.id,
                },
            })),
        })
        .unwrap();
    let other = state
        .add_partner()
        .unwrap()
        .into_iter()
        .find(|p| p.id != 1)
        .unwrap();
    let records = load(&mut state, first.id, None);
    let summary = records
        .summaries
        .iter()
        .find(|s| s.partner.id == acting.partner)
        .unwrap();
    assert_eq!(summary.produced_tasks[acting.form], 2);
    assert_eq!(summary.produced_tasks.iter().sum::<usize>(), 2);
    assert_eq!(summary.unknown_producer_artifacts, 1);
    assert_eq!(summary.shared_bond_memories, 1);
    assert!(summary.moves.iter().all(|n| *n == 0));
    assert!(summary.move_names.iter().all(Vec::is_empty));
    let untouched = records
        .summaries
        .iter()
        .find(|s| s.partner.id == other.id)
        .unwrap();
    assert!(untouched.produced_tasks.iter().all(|n| *n == 0));
    assert_eq!(untouched.shared_bond_memories, 1);
}

#[test]
fn schema14_backfill_retains_old_capture_workflow_unknowns_and_actual_run_producer_without_activity_replay()
 {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("project/source.rs"), "old source\n").unwrap();
    let mut state = fixture.open();
    let mut task = fixture.task(&mut state);
    let acting = owner(&state, 1);
    fixture.capture(&mut state, task.id, Some(acting.clone()), 1, 1);
    terminal(&mut state, task.id, "legacy", "old", true, b"ok\n");
    task.revision += 1;
    task.workflows.hypotheses.push(workflows::Hypothesis {
        id: 1,
        explanation: "Legacy hypothesis without current support".into(),
        evidence: Vec::new(),
    });
    state.save(task.clone()).unwrap();
    let connection = state.connection.as_ref().unwrap();
    remove_schema(connection);
    drop(state);
    let mut state = fixture.open();
    let records = load(&mut state, task.id, Some(observed("old")));
    assert!(records.activities.is_empty());
    let capture = records
        .artifacts
        .iter()
        .find(|a| a.artifact.kind == ArtifactKind::Capture)
        .unwrap();
    assert!(capture.artifact.producer.is_none());
    let run = records
        .artifacts
        .iter()
        .find(|a| a.artifact.kind == ArtifactKind::Run)
        .unwrap();
    assert_eq!(run.artifact.producer.as_ref(), Some(&acting));
    let hypothesis = records
        .artifacts
        .iter()
        .find(|a| a.artifact.kind == ArtifactKind::Hypothesis)
        .unwrap();
    assert!(hypothesis.artifact.producer.is_none());
    assert!(!hypothesis.artifact.supported_diagnosis);
    assert_eq!(hypothesis.freshness, Freshness::Ambiguous);
}

#[test]
fn test_name_signatures_aggregate_contained_failure_paths_without_changing_identity() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let first=terminal(&mut state,task.id,"path-one","old",false,b"test tests::only_name ... FAILED\n  at src/first.rs:12:4\n  at ../../outside.rs:3:1\n  at .env:1:1\n");
    terminal(
        &mut state,
        task.id,
        "path-two",
        "old",
        false,
        b"test tests::only_name ... FAILED\n  at src/second.rs:20:2\n",
    );
    let records = load(&mut state, task.id, None);
    assert_eq!(records.signatures.len(), 1);
    assert_eq!(records.signatures[0].seen_count, 2);
    let paths = &records.signatures[0].paths;
    assert_eq!(paths.len(), 2);
    for expected in [Path::new("src/first.rs"), Path::new("src/second.rs")] {
        assert!(paths.iter().any(|path| Path::new(path) == expected));
    }
    state.purge_output(task.id, &first.prepared.run_id).unwrap();
    let records = load(&mut state, task.id, None);
    assert_eq!(records.signatures[0].seen_count, 1);
    assert_eq!(records.signatures[0].paths.len(), 1);
    assert_eq!(
        Path::new(&records.signatures[0].paths[0]),
        Path::new("src/second.rs")
    );
}

#[test]
fn review_generation_and_handoff_receipts_deliver_once_independently_of_intent_retries() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let mut task = fixture.task(&mut state);
    let mut acting = owner(&state, 1);
    acting.form = 7;
    task.form = 7;
    task.workflows.review = Some(workflows::Review {
        requested: workflows::Verdict::Inconclusive,
        criteria: Vec::new(),
    });
    task.workflows.review_generation = 1;
    for id in ["review-first-callback", "review-repeated-callback"] {
        task.revision += 1;
        task.local_records = vec![LocalIntent::Artifact {
            id: id.into(),
            owner: acting.clone(),
            kind: ArtifactKind::Review,
            reference: "1".into(),
            current: None,
        }];
        task = state.save(task).unwrap();
    }
    let records = load(&mut state, task.id, None);
    assert_eq!(records.activities.len(), 1);
    assert_eq!(records.activities[0].id, format!("review:{}:1", task.id));
    task.local_records = vec![LocalIntent::HandoffReady {
        id: "handoff-ready-receipt".into(),
        owner: acting,
        revision: task.revision,
        digest: "0".repeat(64),
        evidence: Vec::new(),
    }];
    state.save(task.clone()).unwrap();
    let ready = load(&mut state, task.id, None);
    assert_eq!(ready.activities.len(), 1);
    state.save(task.clone()).unwrap();
    assert!(load(&mut state, task.id, None).activities.is_empty());
    drop(state);
    let mut state = fixture.open();
    assert!(load(&mut state, task.id, None).activities.is_empty());
}

#[test]
fn worker_records_failure_returns_the_original_full_request_for_exact_owner_retry() {
    let fixture = Fixture::new();
    let worker = Worker::start(
        fixture.root.join("data"),
        fixture.project.clone(),
        Policy::default(),
    )
    .unwrap();
    assert!(matches!(
        worker
            .events
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap(),
        Event::Loaded(_)
    ));
    let original = Request {
        task_id: 0,
        operation: Operation::ObserveInstruction {
            request_id: "not-dispatched".into(),
            owner: Attribution {
                partner: 1,
                name: "Original owner".into(),
                form: 5,
            },
            instruction: "Full authored bytes survive failure\n  including indentation.".into(),
        },
    };
    let mut pending = Some(original.clone());
    assert!(worker.try_records(&mut pending));
    assert!(pending.is_none());
    match worker
        .events
        .as_ref()
        .unwrap()
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
    {
        Event::Records { request, result } => {
            assert_eq!(*request, original);
            assert!(result.is_err());
        }
        other => panic!("Expected exact records acknowledgement, got {other:?}"),
    }
}

#[test]
fn retry_reopen_preserves_fresh_undelivered_activity_but_new_process_startup_consumes_it() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    terminal(&mut state, task.id, "retry-preserves", "old", true, b"ok\n");
    assert!(matches!(state.reopen(), Event::Loaded(_)));
    assert_eq!(load(&mut state, task.id, None).activities.len(), 1);
    terminal(
        &mut state,
        task.id,
        "restart-consumes",
        "old",
        true,
        b"ok\n",
    );
    drop(state);
    let mut state = fixture.open();
    assert!(load(&mut state, task.id, None).activities.is_empty());
}

#[test]
fn coalesced_handoff_save_preserves_historical_revision_and_drafts_without_replay() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("project/source.rs"), "retained source\n").unwrap();
    let mut state = fixture.open();
    let mut task = fixture.task(&mut state);
    let acting = owner(&state, 1);
    let excerpt = fixture.capture(&mut state, task.id, Some(acting.clone()), 1, 1);
    let evidence = vec![workflows::EvidenceRef::source(&excerpt).unwrap()];
    let receipt = LocalIntent::HandoffReady {
        id: "current-handoff".into(),
        owner: acting.clone(),
        revision: task.revision,
        digest: "1".repeat(64),
        evidence: evidence.clone(),
    };
    task.local_records = vec![receipt];
    task = state.save(task).unwrap();
    let current = load(&mut state, task.id, None);
    assert_eq!(current.activities.len(), 1);
    assert_eq!(
        current
            .artifacts
            .iter()
            .find(|a| a.artifact.id == "current-handoff")
            .unwrap()
            .freshness,
        Freshness::Current
    );

    // A second validated receipt is still queued when a draft/caret edit is
    // coalesced into the next save. Its document revision must stay unchanged.
    let revision = task.revision;
    let retained = LocalIntent::HandoffReady {
        id: "backpressured-handoff".into(),
        owner: acting,
        revision,
        digest: "2".repeat(64),
        evidence,
    };
    task.local_records = vec![retained.clone()];
    task.revision += 1;
    task.partner_drafts.insert(
        1,
        super::super::PartnerDrafts {
            drafts: vec![super::super::Draft {
                field: super::super::EditorField::Request,
                text: "new draft after the handoff".into(),
                cursor: 7,
            }],
            editor_field: super::super::EditorField::Request,
            editing: true,
            previous_form: None,
        },
    );
    let saved = state.save(task).unwrap();
    assert_eq!(saved.local_records, vec![retained.clone()]);
    let historical = load(&mut state, saved.id, None);
    assert_eq!(historical.activities.len(), 1);
    assert_eq!(historical.artifacts.len(), 3); // Capture plus two receipts.
    for artifact in historical
        .artifacts
        .iter()
        .filter(|a| a.artifact.kind == ArtifactKind::Handoff)
    {
        assert_eq!(artifact.freshness, Freshness::Stale);
        assert_eq!(artifact.artifact.handoff_revision, Some(revision));
    }
    state.save(saved.clone()).unwrap();
    assert!(load(&mut state, saved.id, None).activities.is_empty());
    drop(state);
    let mut state = fixture.open();
    let mut loaded = load_task(
        state.connection.as_ref().unwrap(),
        &fixture.project,
        saved.id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(loaded.revision, revision + 1);
    assert_eq!(loaded.partner_drafts, saved.partner_drafts);
    assert!(load(&mut state, saved.id, None).activities.is_empty());

    loaded.revision += 1;
    loaded.local_records = vec![retained.clone()];
    let saved = state.save(loaded).unwrap();
    assert!(load(&mut state, saved.id, None).activities.is_empty());
    state
        .context(ContextRequest {
            task_id: saved.id,
            producer: None,
            operation: ContextOperation::Purge(vec![excerpt.id]),
        })
        .unwrap();
    // Exact delayed acknowledgements after purge neither poison the task save
    // nor resurrect deleted receipt metadata or activity.
    state.save(saved.clone()).unwrap();
    let purged = load(&mut state, saved.id, None);
    assert!(purged.artifacts.is_empty());
    assert!(purged.activities.is_empty());
    drop(state);
    let mut state = fixture.open();
    state.save(saved.clone()).unwrap();
    assert!(load(&mut state, saved.id, None).artifacts.is_empty());
    let mut contradictory = saved;
    if let LocalIntent::HandoffReady { owner, .. } = &mut contradictory.local_records[0] {
        owner.name = "Different producer".into();
    }
    assert!(state.save(contradictory).is_err());
}

#[test]
fn handoff_rejects_future_foreign_invalid_and_never_committed_missing_references_atomically() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("project/source.rs"), "owned source\n").unwrap();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let acting = owner(&state, 1);
    let excerpt = fixture.capture(&mut state, task.id, Some(acting.clone()), 1, 1);
    let source = workflows::EvidenceRef::source(&excerpt).unwrap();
    let mut invalid_identity = source.clone();
    if let workflows::EvidenceRef::Source { source_sha256, .. } = &mut invalid_identity {
        *source_sha256 = [0; 32];
    }
    let mut foreign = source.clone();
    if let workflows::EvidenceRef::Source { task_id, .. } = &mut foreign {
        *task_id += 1;
    }
    for (revision, evidence) in [
        (task.revision + 2, vec![source.clone()]),
        (task.revision, vec![foreign]),
        (task.revision, vec![invalid_identity]),
        (
            task.revision,
            vec![workflows::EvidenceRef::Run {
                task_id: task.id + 1,
                run_id: "foreign-run".into(),
            }],
        ),
    ] {
        let mut changed = task.clone();
        changed.revision += 1;
        changed.goal = "must not commit".into();
        changed.local_records = vec![LocalIntent::HandoffReady {
            id: "invalid-handoff".into(),
            owner: acting.clone(),
            revision,
            digest: "3".repeat(64),
            evidence,
        }];
        assert!(state.save(changed).is_err());
        let loaded = load_task(
            state.connection.as_ref().unwrap(),
            &fixture.project,
            task.id,
        )
        .unwrap()
        .unwrap();
        assert_eq!(loaded, task);
    }
    state
        .context(ContextRequest {
            task_id: task.id,
            producer: None,
            operation: ContextOperation::Purge(vec![excerpt.id]),
        })
        .unwrap();
    let mut changed = task.clone();
    changed.revision += 1;
    changed.local_records = vec![LocalIntent::HandoffReady {
        id: "never-committed-handoff".into(),
        owner: acting,
        revision: task.revision,
        digest: "4".repeat(64),
        evidence: vec![source],
    }];
    assert!(state.save(changed).is_err());
    let records = load(&mut state, task.id, None);
    assert!(records.artifacts.is_empty());
    assert!(records.activities.is_empty());
}

#[test]
fn legacy_handoff_without_typed_revision_remains_unknown_despite_retained_detail() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("project/source.rs"), "retained source\n").unwrap();
    let mut state = fixture.open();
    let mut task = fixture.task(&mut state);
    let acting = owner(&state, 1);
    let excerpt = fixture.capture(&mut state, task.id, Some(acting.clone()), 1, 1);
    task.local_records = vec![LocalIntent::HandoffReady {
        id: "legacy-handoff".into(),
        owner: acting,
        revision: task.revision,
        digest: "5".repeat(64),
        evidence: vec![workflows::EvidenceRef::source(&excerpt).unwrap()],
    }];
    state.save(task.clone()).unwrap();
    let current = load(&mut state, task.id, None);
    let receipt = current
        .artifacts
        .iter()
        .find(|a| a.artifact.id == "legacy-handoff")
        .unwrap();
    assert_eq!(receipt.freshness, Freshness::Current);
    assert_eq!(receipt.artifact.handoff_revision, Some(task.revision));
    // Legacy persisted records lack the typed field; human-readable detail is
    // not revision authority, even when it happens to contain today's revision.
    state
        .connection
        .as_ref()
        .unwrap()
        .execute(
            "UPDATE local_artifacts SET record=json_remove(record,'$.handoff_revision') WHERE id=?1",
            ["legacy-handoff"],
        )
        .unwrap();
    drop(state);
    let mut state = fixture.open();
    let legacy = load(&mut state, task.id, None);
    let receipt = legacy
        .artifacts
        .iter()
        .find(|a| a.artifact.id == "legacy-handoff")
        .unwrap();
    assert_eq!(receipt.artifact.handoff_revision, None);
    assert_eq!(receipt.freshness, Freshness::Ambiguous);
}
