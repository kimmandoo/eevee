use super::*;
use crate::workspace::moves::{Provenance, Steps};
use crate::workspace::runner::{
    CommandSpec, ExecutableIdentity, InputSnapshot, Prepared, RunRecord, RunState, TreeOutcome,
};

struct Fixture {
    root: PathBuf,
    project: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "ev-moves-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("project")).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let project = root.join("project").to_str().unwrap().to_owned();
        Self { root, project }
    }
    fn open(&self) -> State {
        let mut store = State {
            data_dir: self.root.join("data"),
            project: self.project.clone(),
            policy: Policy::default(),
            lock: None,
            connection: None,
        };
        store.open_and_load().unwrap();
        store
    }
    fn task(&self, store: &mut State, form: usize) -> Snapshot {
        let mut task = Snapshot::empty(store.project.clone());
        task.goal = "Preserve actual local facts".into();
        task.form = form;
        task.auto_form = form == 0;
        store.save(task).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn request(task: &Snapshot, partner: i64, form: usize, operation: Operation) -> Request {
    Request {
        task_id: task.id,
        partner,
        form,
        operation,
    }
}
fn call(
    store: &mut State,
    task: &Snapshot,
    partner: i64,
    form: usize,
    operation: Operation,
) -> Result<Response, String> {
    store.moves_request(request(task, partner, form, operation))
}
fn proposal(task: &Snapshot, partner: i64, name: &str, scope: Scope) -> Proposal {
    Proposal {
        partner,
        form: 2,
        name: name.into(),
        scope,
        steps: Steps::Run {
            argv: vec!["cargo".into(), "test".into(), name.into()],
            cwd: ".".into(),
        },
        provenance: Provenance {
            task_id: task.id,
            sources: vec![],
            taught_by: None,
            teacher_name: None,
        },
    }
}
fn teach(store: &mut State, task: &Snapshot, proposal: Proposal) -> Record {
    let name = proposal.name.clone();
    let partner = proposal.partner;
    let form = proposal.form;
    call(
        store,
        task,
        partner,
        form,
        Operation::Teach {
            proposal,
            replace: None,
        },
    )
    .unwrap()
    .records
    .into_iter()
    .find(|record| record.proposal.name == name)
    .unwrap()
}
fn run_record(
    task: &Snapshot,
    owner: Attribution,
    id: &str,
    args: Vec<String>,
    state: RunState,
) -> RunRecord {
    let now = timestamp().unwrap() as u64;
    let prepared = Prepared {
        request_id: format!("request-{id}"),
        run_id: format!("run-{id}"),
        task_id: task.id,
        spec: CommandSpec::Direct {
            program: "cargo".into(),
            args,
        },
        cwd: PathBuf::from(&task.project),
        cwd_identity: "fixture-cwd".into(),
        executable: ExecutableIdentity {
            path: PathBuf::from(&task.project).join("cargo"),
            sha256: "0".repeat(64),
            bytes: 1,
            modified_ns: 1,
            file_id: "fixture-file".into(),
        },
        prepared_at_ms: now,
        expires_at_ms: now + 60_000,
        approval_arrival: 1,
        before: InputSnapshot {
            fingerprint: Some("before-revision".into()),
            complete: true,
            stable: true,
        },
        partner: Some(owner),
        rerun_of: None,
    };
    let mut record = RunRecord::awaiting(&prepared);
    record.state = state;
    if state != RunState::AwaitingDispatch {
        record.started_at_ms = Some(now);
        record.tree = TreeOutcome::OwnedTreeExited;
        if !active_state(&state) {
            record.ended_at_ms = Some(now + 1);
        }
        record.exit_code = match state {
            RunState::Succeeded => Some(0),
            RunState::Failed => Some(1),
            _ => None,
        };
        record.cancelled = state == RunState::Cancelled;
        record.after = InputSnapshot {
            fingerprint: Some("after-revision".into()),
            complete: true,
            stable: true,
        };
    }
    record
}
fn commit_run(store: &mut State, record: &RunRecord) {
    store.connection.as_ref().unwrap().execute("INSERT INTO executions(run_id,task_id,request_id,record,output_name,output_identity,capacity,reserved) VALUES(?1,?2,?3,?4,?5,'1:1',1024,?6)",params![record.prepared.run_id,record.prepared.task_id,record.prepared.request_id,run_json(record).unwrap(),spool_name(&record.prepared.run_id).unwrap(),active_state(&record.state)]).unwrap();
}
fn owner(store: &State, partner: i64) -> Attribution {
    Attribution::of(
        &partners(store.connection.as_ref().unwrap())
            .unwrap()
            .into_iter()
            .find(|member| member.id == partner)
            .unwrap(),
    )
}
fn observe_run(store: &mut State, task: &Snapshot, id: &str) -> Response {
    let record = run_record(
        task,
        owner(store, 1),
        id,
        vec!["test".into(), "--workspace".into()],
        RunState::Succeeded,
    );
    commit_run(store, &record);
    call(
        store,
        task,
        1,
        2,
        Operation::ObserveRun {
            run_id: record.prepared.run_id,
        },
    )
    .unwrap()
}
fn use_for(task: &Snapshot, record: &Record, id: &str, outcome: Outcome) -> Usage {
    Usage {
        id: id.into(),
        move_id: record.id,
        task_id: task.id,
        partner: record.proposal.partner,
        run_id: None,
        request_id: None,
        outcome,
        revision: None,
    }
}

#[test]
fn four_slots_stale_atomic_replacement_and_relearn_survive_restart() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    let first = teach(
        &mut store,
        &task,
        proposal(&task, 1, "Thunderbolt", Scope::Project),
    );
    for name in ["Spark", "Charge", "Volt Switch"] {
        teach(&mut store, &task, proposal(&task, 1, name, Scope::Project));
    }
    let fifth = proposal(&task, 1, "Discharge", Scope::Project);
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            Operation::Teach {
                proposal: fifth.clone(),
                replace: None
            }
        )
        .unwrap_err()
        .contains("four")
    );
    let renamed = call(
        &mut store,
        &task,
        1,
        2,
        Operation::Rename {
            id: first.id,
            revision: first.revision,
            name: "雷電".into(),
        },
    )
    .unwrap()
    .records
    .into_iter()
    .find(|record| record.id == first.id)
    .unwrap();
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            Operation::Teach {
                proposal: fifth.clone(),
                replace: Some((first.id, first.revision))
            }
        )
        .is_err()
    );
    assert_eq!(
        call(&mut store, &task, 1, 2, Operation::Load)
            .unwrap()
            .records
            .iter()
            .filter(|record| !record.forgotten)
            .count(),
        4
    );
    let replaced = call(
        &mut store,
        &task,
        1,
        2,
        Operation::Teach {
            proposal: fifth,
            replace: Some((renamed.id, renamed.revision)),
        },
    )
    .unwrap();
    let forgotten = replaced
        .records
        .iter()
        .find(|record| record.id == first.id)
        .unwrap()
        .clone();
    assert!(forgotten.forgotten);
    let target = replaced
        .records
        .iter()
        .find(|record| record.proposal.name == "Discharge")
        .unwrap()
        .clone();
    drop(store);
    let mut store = fixture.open();
    let response = call(
        &mut store,
        &task,
        1,
        2,
        Operation::Relearn {
            id: forgotten.id,
            revision: forgotten.revision,
            replace: Some((target.id, target.revision)),
        },
    )
    .unwrap();
    let relearned = response
        .records
        .iter()
        .find(|record| record.id == first.id)
        .unwrap();
    assert!(!relearned.forgotten);
    assert_eq!(relearned.proposal.steps, first.proposal.steps);
    assert_eq!(relearned.proposal.name, "雷電");
    assert_eq!(
        response
            .records
            .iter()
            .filter(|record| !record.forgotten)
            .count(),
        4
    );
}

#[test]
fn rename_is_unicode_caseless_and_partner_form_identity_is_persistent() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    store.add_partner().unwrap();
    let first = teach(
        &mut store,
        &task,
        proposal(&task, 1, "Straße", Scope::Project),
    );
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            Operation::Teach {
                proposal: proposal(&task, 1, "STRASSE", Scope::Project),
                replace: None
            }
        )
        .is_err()
    );
    let renamed = call(
        &mut store,
        &task,
        1,
        2,
        Operation::Rename {
            id: first.id,
            revision: 1,
            name: "水の舞".into(),
        },
    )
    .unwrap();
    assert_eq!(renamed.records[0].proposal.name, "水の舞");
    assert!(
        call(&mut store, &task, 2, 2, Operation::Load)
            .unwrap()
            .records
            .is_empty()
    );
    assert!(
        call(&mut store, &task, 1, 1, Operation::Load)
            .unwrap()
            .records
            .is_empty()
    );
    let mut switched = task.clone();
    switched.form = 1;
    switched.revision += 1;
    let mut switched = store.save(switched).unwrap();
    switched.form = 2;
    switched.revision += 1;
    store.save(switched).unwrap();
    drop(store);
    let mut store = fixture.open();
    assert_eq!(
        call(&mut store, &task, 1, 2, Operation::Load)
            .unwrap()
            .records[0]
            .proposal
            .name,
        "水の舞"
    );
    assert_eq!(name_key("Σςσ"), name_key("σσσ"));
    assert_eq!(name_key("ＳＰＡＲＫ"), name_key("spark"));
    assert_ne!(name_key("ı"), name_key("i"));
}

#[test]
fn quota_failure_rolls_back_selected_forget_and_new_record() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    let first = teach(
        &mut store,
        &task,
        proposal(&task, 1, "Thunderbolt", Scope::Project),
    );
    store.policy.quota_bytes = 1;
    let error = call(
        &mut store,
        &task,
        1,
        2,
        Operation::Teach {
            proposal: proposal(&task, 1, "Spark", Scope::Project),
            replace: Some((first.id, first.revision)),
        },
    )
    .unwrap_err();
    assert!(error.contains("quota"));
    let response = call(&mut store, &task, 1, 2, Operation::Load).unwrap();
    assert_eq!(response.records, vec![first]);
}

#[test]
fn load_backfill_obeys_quota_atomically_but_unchanged_load_remains_available() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    for id in ["history-1", "history-2", "history-3"] {
        let record = run_record(
            &task,
            owner(&store, 1),
            id,
            vec!["test".into(), "--workspace".into()],
            RunState::Succeeded,
        );
        commit_run(&mut store, &record);
    }
    store.policy.quota_bytes = 1;
    assert!(
        call(&mut store, &task, 1, 2, Operation::Load)
            .unwrap_err()
            .contains("quota")
    );
    let counts: (u64, u64, u64) = store
        .connection
        .as_ref()
        .unwrap()
        .query_row(
            "SELECT (SELECT count(*) FROM move_sources),
                    (SELECT count(*) FROM move_candidates),
                    (SELECT count(*) FROM executions)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(counts, (0, 0, 3));
    store.policy = Policy::default();
    let backfilled = call(&mut store, &task, 1, 2, Operation::Load).unwrap();
    assert_eq!(backfilled.candidates.len(), 1);
    assert_eq!(backfilled.candidates[0].uses, 3);
    store.policy.quota_bytes = 1;
    let unchanged = call(&mut store, &task, 1, 2, Operation::Load).unwrap();
    assert_eq!(unchanged.candidates, backfilled.candidates);
    assert_eq!(
        store
            .connection
            .as_ref()
            .unwrap()
            .query_row("SELECT count(*) FROM executions", [], |row| row
                .get::<_, u64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn durable_offer_ack_notnow_plus_three_and_never_do_not_count_duplicate_runs() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    assert!(observe_run(&mut store, &task, "1").candidates.is_empty());
    assert!(observe_run(&mut store, &task, "2").candidates.is_empty());
    let response = observe_run(&mut store, &task, "3");
    let candidate = response.candidates[0].clone();
    assert_eq!(candidate.uses, 3);
    assert_eq!(candidate.sources.len(), 3);
    assert!(!candidate.postponed);
    let duplicate = call(
        &mut store,
        &task,
        1,
        2,
        Operation::ObserveRun {
            run_id: "run-3".into(),
        },
    )
    .unwrap();
    assert_eq!(duplicate.candidates[0].uses, 3);
    // Merely loading while busy does not acknowledge or lose an offer.
    assert!(
        !call(&mut store, &task, 1, 2, Operation::Load)
            .unwrap()
            .candidates[0]
            .postponed
    );
    call(
        &mut store,
        &task,
        1,
        2,
        Operation::CandidateDecision {
            key: candidate.key.clone(),
            decision: Decision::Offered,
        },
    )
    .unwrap();
    drop(store);
    let mut store = fixture.open();
    assert!(
        call(&mut store, &task, 1, 2, Operation::Load)
            .unwrap()
            .candidates[0]
            .postponed
    );
    call(
        &mut store,
        &task,
        1,
        2,
        Operation::CandidateDecision {
            key: candidate.key.clone(),
            decision: Decision::NotNow,
        },
    )
    .unwrap();
    assert!(observe_run(&mut store, &task, "4").candidates[0].postponed);
    assert!(observe_run(&mut store, &task, "5").candidates[0].postponed);
    assert!(!observe_run(&mut store, &task, "6").candidates[0].postponed);
    call(
        &mut store,
        &task,
        1,
        2,
        Operation::CandidateDecision {
            key: candidate.key,
            decision: Decision::Never,
        },
    )
    .unwrap();
    drop(store);
    let mut store = fixture.open();
    assert!(observe_run(&mut store, &task, "7").candidates.is_empty());
    assert!(
        call(&mut store, &task, 1, 2, Operation::Load)
            .unwrap()
            .candidates
            .is_empty()
    );
}

#[test]
fn run_observations_refuse_fabrication_wrong_owner_and_unstarted_shell_or_redacted_history() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    store.add_partner().unwrap();
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            Operation::ObserveRun {
                run_id: "made-up".into()
            }
        )
        .is_err()
    );
    let pending = run_record(
        &task,
        owner(&store, 1),
        "pending",
        vec!["test".into()],
        RunState::AwaitingDispatch,
    );
    commit_run(&mut store, &pending);
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            Operation::ObserveRun {
                run_id: pending.prepared.run_id.clone()
            }
        )
        .is_err()
    );
    let mut redacted = run_record(
        &task,
        owner(&store, 1),
        "redacted",
        vec!["[REDACTED]".into()],
        RunState::Succeeded,
    );
    commit_run(&mut store, &redacted);
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            Operation::ObserveRun {
                run_id: redacted.prepared.run_id.clone()
            }
        )
        .is_err()
    );
    redacted.prepared.run_id = "run-shell".into();
    redacted.prepared.request_id = "request-shell".into();
    redacted.prepared.spec = CommandSpec::Shell {
        executable: "/bin/sh".into(),
        dialect: crate::workspace::runner::ShellDialect::Posix,
        text: "cargo test".into(),
    };
    commit_run(&mut store, &redacted);
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            Operation::ObserveRun {
                run_id: redacted.prepared.run_id.clone()
            }
        )
        .is_err()
    );
    let response = observe_run(&mut store, &task, "valid");
    assert!(response.candidates.is_empty());
    assert!(
        call(
            &mut store,
            &task,
            2,
            2,
            Operation::ObserveRun {
                run_id: "run-valid".into()
            }
        )
        .is_err()
    );
    let different = run_record(
        &task,
        owner(&store, 1),
        "different",
        vec!["test".into(), "--lib".into()],
        RunState::Succeeded,
    );
    commit_run(&mut store, &different);
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            Operation::ObserveRun {
                run_id: different.prepared.run_id
            }
        )
        .unwrap()
        .candidates
        .is_empty()
    );
}

fn committed_pins(store: &mut State, task: &Snapshot) -> Vec<Pin> {
    ["src/lib.rs", "src/main.rs"]
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let id = index as i64 + 1;
            let excerpt = crate::workspace::context::Excerpt {
                id,
                task_id: task.id,
                root: task.project.clone(),
                path: (*path).into(),
                start_line: 1,
                end_line: 3,
                source_sha256: [0; 32],
                text: "public fixture\npublic fixture\npublic fixture".into(),
                available: true,
                origin_run: None,
            };
            store
                .connection
                .as_ref()
                .unwrap()
                .execute(
                    "INSERT INTO excerpts(id,task_id,record) VALUES(?1,?2,?3)",
                    params![id, task.id, bounded_json(&excerpt, 512 * 1024).unwrap()],
                )
                .unwrap();
            Pin {
                path: (*path).into(),
                start_line: 1,
                end_line: 3,
            }
        })
        .collect()
}
fn dispatch(
    store: &mut State,
    task: &Snapshot,
    id: &str,
    pins: Vec<Pin>,
) -> Result<Response, String> {
    let claim = WorkClaim {
        task_id: task.id,
        owner: owner(store, 1),
        request_id: id.into(),
        writing: false,
    };
    store
        .work_request(&WorkRequest::Acquire {
            claim: claim.clone(),
            task: Box::new(task.clone()),
        })
        .unwrap();
    let result = call(
        store,
        task,
        1,
        1,
        Operation::DispatchPins {
            request_id: id.into(),
            pins,
        },
    );
    store.work_request(&WorkRequest::Release(claim)).unwrap();
    result
}

#[test]
fn dispatched_request_ids_are_immutable_deduped_and_pin_sets_ignore_order() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 3);
    let pins = committed_pins(&mut store, &task);
    assert!(
        call(
            &mut store,
            &task,
            1,
            1,
            Operation::DispatchPins {
                request_id: "fake".into(),
                pins: pins.clone()
            }
        )
        .is_err()
    );
    assert!(
        call(
            &mut store,
            &task,
            1,
            1,
            Operation::DispatchPins {
                request_id: "unclaimed".into(),
                pins: pins.clone()
            }
        )
        .is_err()
    );
    assert!(
        dispatch(&mut store, &task, "request-1", pins.clone())
            .unwrap()
            .candidates
            .is_empty()
    );
    let mut reordered = pins.clone();
    reordered.reverse();
    assert!(
        dispatch(&mut store, &task, "request-2", reordered.clone())
            .unwrap()
            .candidates
            .is_empty()
    );
    let response = dispatch(&mut store, &task, "request-3", reordered.clone()).unwrap();
    assert_eq!(response.candidates[0].uses, 3);
    assert_eq!(
        call(
            &mut store,
            &task,
            1,
            1,
            Operation::DispatchPins {
                request_id: "request-3".into(),
                pins: pins.clone()
            }
        )
        .unwrap()
        .candidates[0]
            .uses,
        3
    );
    assert!(
        call(
            &mut store,
            &task,
            1,
            1,
            Operation::DispatchPins {
                request_id: "request-3".into(),
                pins: vec![pins[0].clone()]
            }
        )
        .is_err()
    );
    let mut invalid = pins;
    invalid[0].path = ".env".into();
    assert!(dispatch(&mut store, &task, "request-invalid", invalid).is_err());
    drop(store);
    let mut store = fixture.open();
    assert_eq!(
        call(&mut store, &task, 1, 1, Operation::Load)
            .unwrap()
            .candidates[0]
            .uses,
        3
    );
}

#[test]
fn request_uses_require_dispatch_receipts_and_settle_without_claiming_tool_evidence() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 3);
    let mut proposal = proposal(&task, 1, "Future Sight", Scope::Project);
    proposal.form = 3;
    proposal.steps = Steps::Request {
        instruction: "Review public design constraints".into(),
        pins: vec![],
    };
    let record = teach(&mut store, &task, proposal);
    let mut usage = use_for(&task, &record, "provider-use", Outcome::Pending);
    usage.request_id = Some("native-request".into());
    assert!(
        call(
            &mut store,
            &task,
            1,
            3,
            Operation::Use {
                usage: usage.clone()
            }
        )
        .is_err()
    );
    dispatch(&mut store, &task, "native-request", vec![]).unwrap();
    assert_eq!(
        call(
            &mut store,
            &task,
            1,
            3,
            Operation::Use {
                usage: usage.clone()
            }
        )
        .unwrap()
        .usages[0]
            .outcome,
        Outcome::Pending
    );
    usage.outcome = Outcome::Completed;
    assert_eq!(
        call(
            &mut store,
            &task,
            1,
            3,
            Operation::Use {
                usage: usage.clone()
            }
        )
        .unwrap()
        .usages[0]
            .outcome,
        Outcome::Completed
    );
    usage.outcome = Outcome::Succeeded;
    assert!(call(&mut store, &task, 1, 3, Operation::Use { usage }).is_err());
    assert_eq!(
        store
            .connection
            .as_ref()
            .unwrap()
            .query_row("SELECT count(*) FROM provider_journal", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn usage_counts_follow_actual_journal_and_survive_restart_with_denials_separate() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    let mut proposal = proposal(&task, 1, "Thunderbolt", Scope::Project);
    proposal.steps = Steps::Run {
        argv: vec!["cargo".into(), "test".into(), "--workspace".into()],
        cwd: ".".into(),
    };
    let record = teach(&mut store, &task, proposal);
    for (index, state) in [
        RunState::Succeeded,
        RunState::Failed,
        RunState::Cancelled,
        RunState::OutcomeUnknown,
    ]
    .into_iter()
    .enumerate()
    {
        let run = run_record(
            &task,
            owner(&store, 1),
            &format!("usage-{index}"),
            vec!["test".into(), "--workspace".into()],
            state,
        );
        commit_run(&mut store, &run);
        let mut usage = use_for(&task, &record, &format!("use-{index}"), Outcome::Succeeded);
        usage.run_id = Some(run.prepared.run_id.clone());
        usage.revision = Some("caller-cannot-invent-revision".into());
        let response = call(&mut store, &task, 1, 2, Operation::Use { usage }).unwrap();
        assert_eq!(response.usages[index].outcome, actual_outcome(&run));
        assert_eq!(
            response.usages[index].revision.as_deref(),
            Some("after-revision")
        );
    }
    let mut denied = use_for(&task, &record, "denied-use", Outcome::Denied);
    denied.request_id = Some("denied-before-dispatch".into());
    call(&mut store, &task, 1, 2, Operation::Use { usage: denied }).unwrap();
    let mut duplicate = use_for(&task, &record, "another-id", Outcome::Succeeded);
    duplicate.run_id = Some("run-usage-0".into());
    assert!(call(&mut store, &task, 1, 2, Operation::Use { usage: duplicate }).is_err());
    drop(store);
    let mut store = fixture.open();
    let usages = call(&mut store, &task, 1, 2, Operation::Load)
        .unwrap()
        .usages;
    assert_eq!(usages.len(), 5);
    assert_eq!(
        usages
            .iter()
            .filter(|usage| usage.outcome == Outcome::Succeeded)
            .count(),
        1
    );
    assert_eq!(
        usages
            .iter()
            .filter(|usage| usage.outcome == Outcome::Denied)
            .count(),
        1
    );
    assert_eq!(
        usages
            .iter()
            .filter(|usage| usage.outcome == Outcome::Cancelled)
            .count(),
        1
    );
    assert_eq!(usages.last().unwrap().id, "denied-use");
}

#[test]
fn personal_slots_are_counted_in_other_projects_and_project_purge_preserves_personal_reminders() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    let personal = teach(
        &mut store,
        &task,
        proposal(&task, 1, "Personal Spark", Scope::Personal),
    );
    let project = teach(
        &mut store,
        &task,
        proposal(&task, 1, "Project Spark", Scope::Project),
    );
    call(
        &mut store,
        &task,
        1,
        2,
        Operation::Forget {
            id: personal.id,
            revision: personal.revision,
        },
    )
    .unwrap();
    call(
        &mut store,
        &task,
        1,
        2,
        Operation::Forget {
            id: project.id,
            revision: project.revision,
        },
    )
    .unwrap();
    purge_project(store.connection.as_ref().unwrap(), &task.project).unwrap();
    let response = call(&mut store, &task, 1, 2, Operation::Load).unwrap();
    assert_eq!(response.records.len(), 1);
    assert_eq!(response.records[0].id, personal.id);
    assert!(response.records[0].forgotten);
    let other = fixture.root.join("other");
    fs::create_dir(&other).unwrap();
    store.project = other.to_str().unwrap().into();
    let other_task = fixture.task(&mut store, 2);
    call(
        &mut store,
        &other_task,
        1,
        2,
        Operation::Relearn {
            id: personal.id,
            revision: 2,
            replace: None,
        },
    )
    .unwrap();
    for name in ["Other A", "Other B", "Other C"] {
        teach(
            &mut store,
            &other_task,
            proposal(&other_task, 1, name, Scope::Project),
        );
    }
    store.project = fixture.project.clone();
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            Operation::Teach {
                proposal: proposal(&task, 1, "New Personal", Scope::Personal),
                replace: None
            }
        )
        .is_err()
    );
    assert_eq!(
        call(&mut store, &task, 1, 2, Operation::Load)
            .unwrap()
            .records
            .len(),
        1
    );
}

#[test]
fn copy_provenance_is_checked_and_release_preserves_journal_usage_evidence() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    store.add_partner().unwrap();
    let source = teach(
        &mut store,
        &task,
        proposal(&task, 1, "Thunderbolt", Scope::Project),
    );
    let mut copy = source.proposal.clone();
    copy.partner = 2;
    copy.provenance.taught_by = Some(1);
    copy.provenance.teacher_name = Some(owner(&store, 1).name);
    let copied = teach(&mut store, &task, copy.clone());
    assert_eq!(copied.proposal.steps, source.proposal.steps);
    copy.name = "Fabricated copy".into();
    if let Steps::Run { argv, .. } = &mut copy.steps {
        argv.push("--lib".into());
    }
    assert!(
        call(
            &mut store,
            &task,
            2,
            2,
            Operation::Teach {
                proposal: copy,
                replace: None
            }
        )
        .is_err()
    );
    let record = run_record(
        &task,
        owner(&store, 1),
        "release",
        vec!["test".into(), "Thunderbolt".into()],
        RunState::Succeeded,
    );
    commit_run(&mut store, &record);
    let mut usage = use_for(&task, &source, "release-use", Outcome::Succeeded);
    usage.run_id = Some(record.prepared.run_id.clone());
    call(&mut store, &task, 1, 2, Operation::Use { usage }).unwrap();
    store.release_partner(1).unwrap();
    let connection = store.connection.as_ref().unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM moves WHERE partner=1", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM move_usages WHERE partner=1",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        journal_run(connection, task.id, 1, &record.prepared.run_id)
            .unwrap()
            .prepared
            .partner
            .unwrap()
            .partner,
        1
    );
    assert_eq!(
        call(&mut store, &task, 2, 2, Operation::Load)
            .unwrap()
            .records[0]
            .proposal
            .steps,
        source.proposal.steps
    );
    drop(store);
    fixture.open();
}

#[test]
fn last_task_purge_removes_project_moves_sources_and_uses_but_not_personal_records() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    let personal = teach(
        &mut store,
        &task,
        proposal(&task, 1, "Personal", Scope::Personal),
    );
    teach(
        &mut store,
        &task,
        proposal(&task, 1, "Project", Scope::Project),
    );
    call(
        &mut store,
        &task,
        1,
        2,
        Operation::Use {
            usage: use_for(&task, &personal, "denied", Outcome::Denied),
        },
    )
    .unwrap();
    for id in ["purge-1", "purge-2", "purge-3"] {
        observe_run(&mut store, &task, id);
    }
    store
        .manage(Management::Delete {
            ids: vec![task.id],
            expired_before: None,
        })
        .unwrap();
    let connection = store.connection.as_ref().unwrap();
    for table in [
        "move_usages",
        "move_candidates",
        "move_sources",
        "move_dispatches",
    ] {
        assert_eq!(
            connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let records = records(connection, &fixture.project, 1, 2).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, personal.id);
    drop(store);
    fixture.open();
}

fn remove_move_schema(connection: &Connection) {
    super::super::records::remove_schema(connection);
    connection.execute_batch("DROP TABLE moves; DROP TABLE move_candidates; DROP TABLE move_sources; DROP TABLE move_usages; DROP TABLE move_dispatches; PRAGMA user_version=13;").unwrap();
}
#[test]
fn schema_thirteen_upgrade_is_atomic_preserves_facts_and_backfills_real_history() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    for id in ["old-1", "old-2", "old-3"] {
        let record = run_record(
            &task,
            owner(&store, 1),
            id,
            vec!["test".into(), "--workspace".into()],
            RunState::Succeeded,
        );
        commit_run(&mut store, &record);
    }
    let mut connection = store.connection.take().unwrap();
    remove_move_schema(&connection);
    let facts: String = connection
        .query_row(
            "SELECT record FROM executions ORDER BY rowid LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    {
        let transaction = connection.transaction().unwrap();
        super::super::schema::migrate(
            &transaction,
            super::super::schema::inspect(&transaction).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            super::super::schema::inspect(&transaction).unwrap(),
            Schema::Current
        ));
    }
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        13
    );
    initialize(&mut connection).unwrap();
    assert_eq!(
        connection
            .query_row::<String, _, _>(
                "SELECT record FROM executions ORDER BY rowid LIMIT 1",
                [],
                |row| row.get(0)
            )
            .unwrap(),
        facts
    );
    store.connection = Some(connection);
    let response = call(&mut store, &task, 1, 2, Operation::Load).unwrap();
    assert_eq!(response.candidates.len(), 1);
    assert_eq!(response.candidates[0].uses, 3);
}

#[test]
fn readonly_admission_refuses_older_and_newer_schema_without_migration_or_writes() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    let current: i64 = store
        .connection
        .as_ref()
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    for version in [current - 1, current + 1] {
        store
            .connection
            .as_ref()
            .unwrap()
            .pragma_update(None, "user_version", version)
            .unwrap();
        let path = store.data_dir.join(DATABASE);
        let before = fs::read(&path).unwrap();
        assert!(validate_planning_input(&store.data_dir, &task, &[]).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    store
        .connection
        .as_ref()
        .unwrap()
        .pragma_update(None, "user_version", current)
        .unwrap();
    validate_planning_input(&store.data_dir, &task, &[]).unwrap();
    store
        .connection
        .as_ref()
        .unwrap()
        .execute_batch("CREATE TABLE unexpected_move_data(value TEXT);")
        .unwrap();
    assert!(validate_planning_input(&store.data_dir, &task, &[]).is_err());
}

#[test]
fn moves_worker_retains_full_or_disconnected_queue_and_emits_correlated_results() {
    let (commands, incoming) = mpsc::sync_channel(1);
    let mut worker = Worker {
        commands: Some(commands),
        events: None,
        thread: None,
    };
    let load_request = Request {
        task_id: 1,
        partner: 1,
        form: 2,
        operation: Operation::Load,
    };
    let mut pending = Some(load_request.clone());
    assert!(worker.try_moves(&mut pending));
    assert!(pending.is_none());
    pending = Some(load_request.clone());
    assert!(!worker.try_moves(&mut pending));
    assert_eq!(pending, Some(load_request.clone()));
    drop(incoming);
    assert!(!worker.try_moves(&mut pending));
    assert_eq!(pending, Some(load_request.clone()));
    worker.commands.take();
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
    let mut task = Snapshot::empty(fixture.project.clone());
    task.goal = "Worker identity".into();
    let mut save = Some(task);
    assert!(worker.try_save(&mut save));
    let Event::Saved(task) = worker
        .events
        .as_ref()
        .unwrap()
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
    else {
        panic!("Expected save acknowledgment")
    };
    let mut pending = Some(request(&task, 1, 2, Operation::Load));
    assert!(worker.try_moves(&mut pending));
    assert!(
        matches!(worker.events.as_ref().unwrap().recv_timeout(Duration::from_secs(5)).unwrap(),Event::Moves {task_id,partner:1,form:2,result:Ok(_)} if task_id==task.id)
    );
}

#[test]
fn bounded_reminder_capacity_never_evicts_a_record() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    {
        let transaction = store.connection.as_mut().unwrap().transaction().unwrap();
        for index in 0..MAX_RECORDS {
            let proposal = proposal(&task, 1, &format!("Reminder {index}"), Scope::Project);
            transaction.execute("INSERT INTO moves(revision,project,partner,form,scope,proposal,forgotten,created) VALUES(1,?1,1,2,'project',?2,1,0)",params![task.project,bounded_json(&proposal,MAX_JSON).unwrap()]).unwrap();
        }
        transaction.commit().unwrap();
    }
    let error = call(
        &mut store,
        &task,
        1,
        2,
        Operation::Teach {
            proposal: proposal(&task, 1, "One too many", Scope::Project),
            replace: None,
        },
    )
    .unwrap_err();
    assert!(error.contains("capacity"));
    assert_eq!(
        call(&mut store, &task, 1, 2, Operation::Load)
            .unwrap()
            .records
            .len(),
        MAX_RECORDS
    );
}

#[test]
fn dispatch_quota_failure_rolls_back_both_receipt_and_observation() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 3);
    let pins = committed_pins(&mut store, &task);
    store.policy.quota_bytes = 1;
    assert!(
        dispatch(&mut store, &task, "quota-request", pins.clone())
            .unwrap_err()
            .contains("quota")
    );
    let connection = store.connection.as_ref().unwrap();
    assert!(dispatch_row(connection, "quota-request").unwrap().is_none());
    assert!(
        source_row(connection, "request:quota-request")
            .unwrap()
            .is_none()
    );
    assert!(
        call(
            &mut store,
            &task,
            1,
            1,
            Operation::DispatchPins {
                request_id: "quota-request".into(),
                pins
            }
        )
        .is_err()
    );
}

#[test]
fn new_personal_move_in_empty_project_cannot_overfill_a_different_project() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task_a = fixture.task(&mut store, 2);
    for name in ["A1", "A2", "A3", "A4"] {
        teach(
            &mut store,
            &task_a,
            proposal(&task_a, 1, name, Scope::Project),
        );
    }
    let project_b = fixture.root.join("project-b");
    fs::create_dir(&project_b).unwrap();
    store.project = project_b.to_str().unwrap().into();
    let task_b = fixture.task(&mut store, 2);
    assert!(
        call(&mut store, &task_b, 1, 2, Operation::Load)
            .unwrap()
            .records
            .is_empty()
    );
    assert!(
        call(
            &mut store,
            &task_b,
            1,
            2,
            Operation::Teach {
                proposal: proposal(&task_b, 1, "Personal", Scope::Personal),
                replace: None
            }
        )
        .unwrap_err()
        .contains("visible")
    );
    assert!(
        call(&mut store, &task_b, 1, 2, Operation::Load)
            .unwrap()
            .records
            .is_empty()
    );
    store.project = fixture.project.clone();
    assert_eq!(
        call(&mut store, &task_a, 1, 2, Operation::Load)
            .unwrap()
            .records
            .iter()
            .filter(|record| !record.forgotten)
            .count(),
        4
    );
}

#[test]
fn run_usage_is_pending_from_committed_reservation_then_resolves_actual_outcome() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    let mut proposal = proposal(&task, 1, "Thunderbolt", Scope::Project);
    proposal.steps = Steps::Run {
        argv: vec!["cargo".into(), "test".into()],
        cwd: ".".into(),
    };
    let record = teach(&mut store, &task, proposal);
    let mut journal = run_record(
        &task,
        owner(&store, 1),
        "pending-use",
        vec!["test".into()],
        RunState::AwaitingDispatch,
    );
    commit_run(&mut store, &journal);
    let mut usage = use_for(&task, &record, "pending-use", Outcome::Succeeded);
    usage.run_id = Some(journal.prepared.run_id.clone());
    let response = call(
        &mut store,
        &task,
        1,
        2,
        Operation::Use {
            usage: usage.clone(),
        },
    )
    .unwrap();
    assert_eq!(response.usages[0].outcome, Outcome::Pending);
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            Operation::ObserveRun {
                run_id: journal.prepared.run_id.clone()
            }
        )
        .is_err()
    );
    call(
        &mut store,
        &task,
        1,
        2,
        Operation::Forget {
            id: record.id,
            revision: record.revision,
        },
    )
    .unwrap();
    journal.state = RunState::Failed;
    journal.started_at_ms = Some(1);
    journal.ended_at_ms = Some(2);
    journal.tree = TreeOutcome::OwnedTreeExited;
    journal.exit_code = Some(7);
    store
        .connection
        .as_ref()
        .unwrap()
        .execute(
            "UPDATE executions SET record=?1,reserved=0 WHERE run_id=?2",
            params![run_json(&journal).unwrap(), journal.prepared.run_id],
        )
        .unwrap();
    let response = call(&mut store, &task, 1, 2, Operation::Use { usage }).unwrap();
    assert_eq!(response.usages.len(), 1);
    assert_eq!(response.usages[0].outcome, Outcome::Failed);
}

#[test]
fn strict_schema_inspection_refuses_move_owner_tampering_and_unknown_objects() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    let record = teach(
        &mut store,
        &task,
        proposal(&task, 1, "Thunderbolt", Scope::Project),
    );
    let connection = store.connection.as_ref().unwrap();
    connection
        .execute("UPDATE moves SET form=5 WHERE id=?1", [record.id])
        .unwrap();
    assert!(
        super::super::schema::inspect(connection)
            .err()
            .unwrap()
            .contains("ownership")
    );
    connection
        .execute("UPDATE moves SET form=2 WHERE id=?1", [record.id])
        .unwrap();
    connection
        .execute_batch("CREATE INDEX unexpected_move_index ON moves(partner);")
        .unwrap();
    assert!(super::super::schema::inspect(connection).is_err());
}

#[test]
fn validation_is_pure_stale_safe_and_requires_an_active_visible_move() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    store.add_partner().unwrap();
    let record = teach(
        &mut store,
        &task,
        proposal(&task, 1, "Thunderbolt", Scope::Project),
    );
    store.policy.quota_bytes = 1;
    let response = call(
        &mut store,
        &task,
        1,
        2,
        Operation::Validate {
            id: record.id,
            revision: record.revision,
        },
    )
    .unwrap();
    assert_eq!(response.records, vec![record.clone()]);
    assert!(response.usages.is_empty());
    assert!(
        call(
            &mut store,
            &task,
            2,
            2,
            Operation::Validate {
                id: record.id,
                revision: record.revision
            }
        )
        .is_err()
    );
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            Operation::Validate {
                id: record.id,
                revision: record.revision + 1
            }
        )
        .is_err()
    );
    call(
        &mut store,
        &task,
        1,
        2,
        Operation::Forget {
            id: record.id,
            revision: record.revision,
        },
    )
    .unwrap();
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            Operation::Validate {
                id: record.id,
                revision: record.revision + 1
            }
        )
        .unwrap_err()
        .contains("Relearn")
    );
}

#[test]
fn completed_task_and_pin_uses_require_actual_committed_local_facts() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let mut task = fixture.task(&mut store, 0);
    let mut template = proposal(&task, 1, "Helping Hand", Scope::Project);
    template.form = 0;
    template.steps = Steps::Task {
        goal: "Reusable goal".into(),
        criteria: vec!["Public criterion".into()],
    };
    let record = teach(&mut store, &task, template);
    let usage = use_for(&task, &record, "template-use", Outcome::Completed);
    assert!(
        call(
            &mut store,
            &task,
            1,
            0,
            Operation::Use {
                usage: usage.clone()
            }
        )
        .is_err()
    );
    task.goal = "Reusable goal".into();
    task.criteria = vec![Criterion {
        text: "Public criterion".into(),
        met: false,
    }];
    task.revision += 1;
    task = store.save(task).unwrap();
    let response = call(&mut store, &task, 1, 0, Operation::Use { usage }).unwrap();
    assert_eq!(response.usages[0].outcome, Outcome::Completed);
    let pins = committed_pins(&mut store, &task);
    let mut proposal = proposal(&task, 1, "Surf", Scope::Project);
    proposal.form = 1;
    proposal.steps = Steps::Pins { pins: pins.clone() };
    let record = teach(&mut store, &task, proposal);
    let usage = use_for(&task, &record, "pin-use", Outcome::Completed);
    call(&mut store, &task, 1, 1, Operation::Use { usage }).unwrap();
    store
        .connection
        .as_ref()
        .unwrap()
        .execute("DELETE FROM excerpts WHERE task_id=?1", [task.id])
        .unwrap();
    assert!(
        call(
            &mut store,
            &task,
            1,
            1,
            Operation::Use {
                usage: use_for(&task, &record, "fabricated-capture", Outcome::Completed)
            }
        )
        .is_err()
    );
}

#[test]
fn purging_one_task_reconciles_sources_without_dropping_other_task_or_project_moves() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let first = fixture.task(&mut store, 2);
    let second = fixture.task(&mut store, 2);
    teach(
        &mut store,
        &first,
        proposal(&first, 1, "Shared project move", Scope::Project),
    );
    observe_run(&mut store, &first, "first-1");
    observe_run(&mut store, &first, "first-2");
    observe_run(&mut store, &second, "second-1");
    store
        .manage(Management::Delete {
            ids: vec![first.id],
            expired_before: None,
        })
        .unwrap();
    let response = call(&mut store, &second, 1, 2, Operation::Load).unwrap();
    assert_eq!(response.records.len(), 1);
    assert_eq!(response.candidates.len(), 1);
    assert_eq!(response.candidates[0].uses, 1);
    let remaining_run: String = store
        .connection
        .as_ref()
        .unwrap()
        .query_row(
            "SELECT run_id FROM executions WHERE task_id=?1",
            [second.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        response.candidates[0].sources,
        vec![format!("run:{remaining_run}")]
    );
    assert!(response.candidates[0].postponed);
    drop(store);
    let mut store = fixture.open();
    assert_eq!(
        call(&mut store, &second, 1, 2, Operation::Load)
            .unwrap()
            .candidates[0]
            .uses,
        1
    );
}

#[test]
fn post_purge_observations_refresh_below_threshold_without_resetting_decisions() {
    for decision in [
        None,
        Some(Decision::Offered),
        Some(Decision::NotNow),
        Some(Decision::Never),
    ] {
        let fixture = Fixture::new();
        let mut store = fixture.open();
        let first = fixture.task(&mut store, 2);
        let second = fixture.task(&mut store, 2);
        observe_run(&mut store, &first, "first-1");
        observe_run(&mut store, &first, "first-2");
        let candidate = observe_run(&mut store, &second, "second-1")
            .candidates
            .remove(0);
        if let Some(decision) = decision {
            call(
                &mut store,
                &second,
                1,
                2,
                Operation::CandidateDecision {
                    key: candidate.key.clone(),
                    decision,
                },
            )
            .unwrap();
        }
        store
            .manage(Management::Delete {
                ids: vec![first.id],
                expired_before: None,
            })
            .unwrap();
        let response = observe_run(&mut store, &second, "second-2");
        let connection = store.connection.as_ref().unwrap();
        let (stored, threshold, offered, never): (Candidate, u64, bool, bool) = connection
            .query_row(
                "SELECT record,threshold,offered,never FROM move_candidates WHERE key=?1",
                [&candidate.key],
                |row| {
                    Ok((
                        json_column(row, 0, MAX_JSON)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                    ))
                },
            )
            .unwrap();
        let expected_sources = connection
            .prepare(
                "SELECT id FROM move_sources WHERE task_id=?1 AND observed=1 ORDER BY rowid DESC",
            )
            .unwrap()
            .query_map([second.id], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(stored.uses, 2);
        assert_eq!(expected_sources.len(), 2);
        assert_eq!(stored.sources, expected_sources);
        assert_eq!(stored.proposal.provenance.sources, stored.sources);
        assert!(stored.postponed);
        assert_eq!(
            threshold,
            if decision == Some(Decision::NotNow) {
                4
            } else {
                3
            }
        );
        assert_eq!(
            offered,
            matches!(decision, Some(Decision::Offered | Decision::Never))
        );
        assert_eq!(never, decision == Some(Decision::Never));
        if never {
            assert!(response.candidates.is_empty());
        } else {
            assert_eq!(response.candidates, vec![stored.clone()]);
        }
        let readonly = Connection::open_with_flags(
            connection.path().unwrap(),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        super::super::schema::inspect(&readonly).unwrap();
        drop(readonly);
        drop(store);
        let mut store = fixture.open();
        let reopened = call(&mut store, &second, 1, 2, Operation::Load).unwrap();
        if never {
            assert!(reopened.candidates.is_empty());
        } else {
            assert_eq!(reopened.candidates, vec![stored]);
        }
        for uses in 3..=6 {
            let response = observe_run(&mut store, &second, &format!("second-{uses}"));
            if never {
                assert!(response.candidates.is_empty());
            } else {
                assert_eq!(response.candidates[0].uses, uses);
                assert_eq!(
                    response.candidates[0].postponed,
                    decision == Some(Decision::Offered) || uses < threshold,
                );
            }
        }
        super::super::schema::inspect(store.connection.as_ref().unwrap()).unwrap();
    }
}

#[test]
fn project_reminders_survive_zero_sources_and_preserve_their_offer_decisions() {
    for decision in [
        None,
        Some(Decision::Offered),
        Some(Decision::NotNow),
        Some(Decision::Never),
    ] {
        let fixture = Fixture::new();
        let mut store = fixture.open();
        let first = fixture.task(&mut store, 2);
        let second = fixture.task(&mut store, 2);
        observe_run(&mut store, &first, "first-1");
        observe_run(&mut store, &first, "first-2");
        let candidate = observe_run(&mut store, &first, "first-3")
            .candidates
            .remove(0);
        if let Some(decision) = decision {
            call(
                &mut store,
                &first,
                1,
                2,
                Operation::CandidateDecision {
                    key: candidate.key.clone(),
                    decision,
                },
            )
            .unwrap();
        }
        store
            .manage(Management::Delete {
                ids: vec![first.id],
                expired_before: None,
            })
            .unwrap();
        for uses in 0..=3 {
            if uses > 0 {
                observe_run(&mut store, &second, &format!("second-{uses}"));
            }
            let connection = store.connection.as_ref().unwrap();
            let (stored, threshold, offered, never): (Candidate, u64, bool, bool) = connection
                .query_row(
                    "SELECT record,threshold,offered,never FROM move_candidates WHERE key=?1",
                    [&candidate.key],
                    |row| {
                        Ok((
                            json_column(row, 0, MAX_JSON)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(stored.uses, uses);
            assert_eq!(stored.sources.len(), uses as usize);
            assert_eq!(stored.proposal.provenance.sources, stored.sources);
            assert_eq!(stored.proposal.steps, candidate.proposal.steps);
            assert_eq!(threshold, 3);
            assert_eq!(
                offered,
                matches!(decision, Some(Decision::Offered | Decision::Never))
            );
            assert_eq!(never, decision == Some(Decision::Never));
            assert_eq!(stored.postponed, offered || uses < 3);
            let readonly = Connection::open_with_flags(
                connection.path().unwrap(),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            super::super::schema::inspect(&readonly).unwrap();
            drop(readonly);
            drop(store);
            store = fixture.open();
            let response = call(&mut store, &second, 1, 2, Operation::Load).unwrap();
            if never {
                assert!(response.candidates.is_empty());
            } else {
                assert_eq!(response.candidates, vec![stored]);
            }
        }
    }
}

#[test]
fn explicit_project_purge_cannot_resurrect_candidates_from_retained_journals() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    for id in ["history-1", "history-2", "history-3"] {
        observe_run(&mut store, &task, id);
    }
    purge_project(store.connection.as_ref().unwrap(), &task.project).unwrap();
    assert!(
        call(&mut store, &task, 1, 2, Operation::Load)
            .unwrap()
            .candidates
            .is_empty()
    );
    drop(store);
    let mut store = fixture.open();
    assert!(
        call(&mut store, &task, 1, 2, Operation::Load)
            .unwrap()
            .candidates
            .is_empty()
    );
    assert!(
        observe_run(&mut store, &task, "fresh-1")
            .candidates
            .is_empty()
    );
    assert!(
        observe_run(&mut store, &task, "fresh-2")
            .candidates
            .is_empty()
    );
    assert_eq!(
        observe_run(&mut store, &task, "fresh-3").candidates[0].uses,
        3
    );
    assert_eq!(
        store
            .connection
            .as_ref()
            .unwrap()
            .query_row("SELECT count(*) FROM executions", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        6
    );
}

#[test]
fn portable_import_is_local_inert_restartable_and_atomic() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 2);
    let foreign = proposal(&task, 1, "Portable", Scope::Project);
    let definition = domain::portable::Definition::from_proposal(&foreign).unwrap();
    let path = fixture.root.join("portable.toml");
    domain::portable::write_new(&path, &definition).unwrap();
    let preview = call(
        &mut store,
        &task,
        1,
        2,
        Operation::ReadPortable {
            path: path.to_str().unwrap().into(),
        },
    )
    .unwrap();
    assert_eq!(preview.portable, Some(definition.clone()));
    assert!(preview.records.is_empty());
    assert!(preview.usages.is_empty());
    let import = |name: &str, scope, replace| Operation::ImportPortable {
        definition: definition.clone(),
        name: name.into(),
        scope,
        replace,
    };
    let learned = call(
        &mut store,
        &task,
        1,
        2,
        import("Portable", Scope::Personal, None),
    )
    .unwrap();
    let first = learned.records[0].clone();
    assert_eq!(first.proposal.provenance.task_id, task.id);
    assert_eq!(first.proposal.provenance.taught_by, None);
    assert_eq!(first.proposal.provenance.teacher_name, None);
    assert!(first.proposal.provenance.sources[0].starts_with("tm:"));
    assert!(learned.usages.is_empty());
    assert!(learned.candidates.is_empty());
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            import("PORTABLE", Scope::Project, None)
        )
        .is_err()
    );
    for name in ["Second", "Third", "Fourth"] {
        call(&mut store, &task, 1, 2, import(name, Scope::Project, None)).unwrap();
    }
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            import("Fifth", Scope::Project, None)
        )
        .is_err()
    );
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            import(
                "Fifth",
                Scope::Project,
                Some((first.id, first.revision + 1))
            )
        )
        .is_err()
    );
    let snapshot = call(&mut store, &task, 1, 2, Operation::Load).unwrap();
    assert_eq!(snapshot.records.iter().filter(|r| !r.forgotten).count(), 4);
    let replaced = call(
        &mut store,
        &task,
        1,
        2,
        import("Fifth", Scope::Personal, Some((first.id, first.revision))),
    )
    .unwrap();
    assert!(
        replaced
            .records
            .iter()
            .find(|r| r.id == first.id)
            .unwrap()
            .forgotten
    );
    let fifth = replaced
        .records
        .iter()
        .find(|r| r.proposal.name == "Fifth")
        .unwrap()
        .clone();
    let exported = fixture.root.join("exported.toml");
    call(
        &mut store,
        &task,
        1,
        2,
        Operation::ExportPortable {
            id: fifth.id,
            revision: fifth.revision,
            path: exported.to_str().unwrap().into(),
        },
    )
    .unwrap();
    let bytes = fs::read(&exported).unwrap();
    assert!(
        call(
            &mut store,
            &task,
            1,
            2,
            Operation::ExportPortable {
                id: fifth.id,
                revision: fifth.revision,
                path: exported.to_str().unwrap().into(),
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(&exported).unwrap(), bytes);
    drop(store);
    let mut reopened = fixture.open();
    let restored = call(&mut reopened, &task, 1, 2, Operation::Load).unwrap();
    assert_eq!(restored.records, replaced.records);
    assert!(restored.usages.is_empty());
    fs::create_dir(fixture.root.join("other-project")).unwrap();
    reopened.project = fixture.root.join("other-project").to_str().unwrap().into();
    let other_task = fixture.task(&mut reopened, 2);
    let other = call(&mut reopened, &other_task, 1, 2, Operation::Load).unwrap();
    assert_eq!(other.records.iter().filter(|r| !r.forgotten).count(), 1);
    assert_eq!(
        other.records.iter().find(|r| !r.forgotten).unwrap().id,
        fifth.id
    );
    assert!(other.usages.is_empty());
}

#[test]
fn every_tm_form_imports_across_projects_without_evidence_or_capture() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    let task = fixture.task(&mut store, 0);
    for form in 0..9 {
        let steps = match form {
            0 => Steps::Task {
                goal: "Local task".into(),
                criteria: vec!["Review result".into()],
            },
            1 => Steps::Pins {
                pins: vec![Pin {
                    path: "not-present.txt".into(),
                    start_line: 1,
                    end_line: 3,
                }],
            },
            2 | 5 | 6 => Steps::Run {
                argv: vec!["cargo".into(), "test".into(), "".into()],
                cwd: ".".into(),
            },
            _ => Steps::Request {
                instruction: "Inspect the change".into(),
                pins: vec![],
            },
        };
        let definition = domain::portable::Definition {
            schema_version: 1,
            name: format!("TM {form}"),
            form: crate::domain::FORMS[form].name.into(),
            steps,
        };
        let imported = call(
            &mut store,
            &task,
            1,
            form,
            Operation::ImportPortable {
                definition: domain::portable::Definition::decode(&definition.encode().unwrap())
                    .unwrap(),
                name: definition.name.clone(),
                scope: Scope::Project,
                replace: None,
            },
        )
        .unwrap();
        assert_eq!(imported.records[0].proposal.steps, definition.steps);
        assert!(imported.usages.is_empty());
        assert!(imported.candidates.is_empty());
    }
    let connection = store.connection.as_ref().unwrap();
    for table in [
        "executions",
        "excerpts",
        "provider_journal",
        "move_sources",
        "move_dispatches",
        "move_usages",
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    let snapshot = load_task(connection, &store.project, task.id)
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.goal, task.goal);
    assert_eq!(snapshot.criteria, task.criteria);
}
