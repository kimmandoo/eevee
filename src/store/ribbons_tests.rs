use super::*;
use crate::store::ribbons::inspect;
use crate::workspace::ribbons::{Evidence, Kind};

fn repository(fixture: &Fixture) {
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&fixture.project)
            .status()
            .unwrap()
            .success()
    );
    fs::write(
        Path::new(&fixture.project).join("source.rs"),
        "fn retained() {}\n",
    )
    .unwrap();
}

fn review_fixture() -> (
    Fixture,
    State,
    super::super::super::Snapshot,
    crate::workspace::context::Excerpt,
    RunRecord,
) {
    let fixture = Fixture::new();
    repository(&fixture);
    let mut state = fixture.open();
    let mut task = fixture.task(&mut state);
    let acting = owner(&state, 1);
    let capture = fixture.capture(&mut state, task.id, Some(acting), 1, 1);
    let current =
        crate::workspace::repo::observe_inputs(Path::new(&fixture.project), &state.data_dir)
            .unwrap();
    let run = terminal(
        &mut state,
        task.id,
        "ribbon-pass",
        current.fingerprint.as_deref().unwrap(),
        true,
        b"passed\n",
    );
    task.criteria = vec![Criterion {
        text: "Retained behavior passes its relevant check".into(),
        met: false,
    }];
    task.workflows.review = Some(workflows::Review {
        requested: workflows::Verdict::Accept,
        criteria: vec![workflows::CriterionEvidence {
            criterion: task.criteria[0].text.clone(),
            evidence: vec![
                workflows::EvidenceRef::source(&capture).unwrap(),
                workflows::EvidenceRef::run(&run).unwrap(),
            ],
        }],
    });
    task.workflows.review_generation = 1;
    (fixture, state, task, capture, run)
}

fn commit_review(
    state: &mut State,
    mut task: super::super::super::Snapshot,
    id: &str,
) -> super::super::super::Snapshot {
    let mut acting = owner(state, 1);
    acting.form = 7;
    task.revision += 1;
    task.local_records = vec![LocalIntent::Artifact {
        id: id.into(),
        owner: acting,
        kind: ArtifactKind::Review,
        reference: task.workflows.review_generation.to_string(),
        // Qualification must ignore this plausible but false caller observation.
        current: Some(observed("caller-is-not-authority")),
    }];
    state.save(task).unwrap()
}

fn handoff(
    state: &mut State,
    mut task: super::super::super::Snapshot,
    capture: &crate::workspace::context::Excerpt,
    id: &str,
    revision: u64,
) -> super::super::super::Snapshot {
    let mut acting = owner(state, 1);
    acting.form = 8;
    task.local_records = vec![LocalIntent::HandoffReady {
        id: id.into(),
        owner: acting,
        revision,
        digest: "a".repeat(64),
        evidence: vec![workflows::EvidenceRef::source(capture).unwrap()],
    }];
    state.save(task).unwrap()
}

fn taught(
    state: &mut State,
    task: &super::super::super::Snapshot,
    partner: i64,
    name: &str,
) -> crate::workspace::moves::Record {
    use crate::workspace::moves::{Proposal, Provenance, Scope, Steps};
    state
        .moves_request(crate::store::moves::Request {
            task_id: task.id,
            partner,
            form: 2,
            operation: crate::store::moves::Operation::Teach {
                proposal: Proposal {
                    partner,
                    form: 2,
                    name: name.into(),
                    scope: Scope::Project,
                    steps: Steps::Run {
                        argv: vec!["cargo".into(), "test".into()],
                        cwd: ".".into(),
                    },
                    provenance: Provenance {
                        task_id: task.id,
                        sources: Vec::new(),
                        taught_by: None,
                        teacher_name: None,
                    },
                },
                replace: None,
            },
        })
        .unwrap()
        .records
        .into_iter()
        .find(|record| record.proposal.name == name)
        .unwrap()
}

fn row_count(state: &State) -> i64 {
    state
        .connection
        .as_ref()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM ribbons WHERE project=?1",
            [&state.project],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn three_first_recorded_sources_are_immutable_hidden_durable_and_project_wide() {
    let (fixture, mut state, task, capture, _) = review_fixture();
    let task = commit_review(&mut state, task, "first-effective-review");
    let revision = task.revision;
    let task = handoff(
        &mut state,
        task,
        &capture,
        "first-verified-handoff",
        revision,
    );
    let learned = taught(&mut state, &task, 1, "Retained check");
    let first = load(&mut state, task.id, None).ribbons;
    assert_eq!(first.len(), 3);
    assert!(
        first
            .iter()
            .all(|view| view.freshness == Freshness::Current)
    );
    assert!(first.iter().all(|view| !view.ribbon.hidden));
    assert_eq!(
        first
            .iter()
            .find(|view| view.ribbon.kind == Kind::LearnedMove)
            .unwrap()
            .ribbon
            .evidence,
        Evidence::Move {
            id: learned.id,
            revision: 1
        }
    );
    call(
        &mut state,
        task.id,
        Operation::SetRibbonHidden {
            kind: Kind::AcceptedReview,
            hidden: true,
        },
    )
    .unwrap();
    call(&mut state, task.id, Operation::SetFlavor { enabled: false }).unwrap();
    let other = fixture.task(&mut state);
    let project = load(&mut state, other.id, None);
    assert!(!project.flavor_enabled);
    assert_eq!(project.ribbons.len(), 3);
    assert!(
        project
            .ribbons
            .iter()
            .find(|view| view.ribbon.kind == Kind::AcceptedReview)
            .unwrap()
            .ribbon
            .hidden
    );
    let mut later = task.clone();
    later.workflows.review_generation += 1;
    let later = commit_review(&mut state, later, "later-effective-review");
    taught(&mut state, &later, 1, "Second check");
    assert_eq!(row_count(&state), 3);
    drop(state);
    let mut state = fixture.open();
    let reopened = load(&mut state, other.id, None);
    assert_eq!(reopened.ribbons.len(), 3);
    for view in &reopened.ribbons {
        let original = first
            .iter()
            .find(|original| original.ribbon.kind == view.ribbon.kind)
            .unwrap();
        assert_eq!(view.ribbon.evidence, original.ribbon.evidence);
        assert_eq!(view.ribbon.owner, original.ribbon.owner);
        assert_eq!(view.ribbon.at, original.ribbon.at);
    }
    assert!(
        reopened
            .ribbons
            .iter()
            .find(|view| view.ribbon.kind == Kind::AcceptedReview)
            .unwrap()
            .ribbon
            .hidden
    );
    call(
        &mut state,
        other.id,
        Operation::SetRibbonHidden {
            kind: Kind::AcceptedReview,
            hidden: false,
        },
    )
    .unwrap();
    assert!(
        load(&mut state, other.id, None)
            .ribbons
            .iter()
            .all(|view| !view.ribbon.hidden)
    );
    inspect(state.connection.as_ref().unwrap()).unwrap();
}

#[test]
fn requested_accept_and_success_shaped_unverified_sources_do_not_qualify() {
    for case in [
        "request-only",
        "no-generation",
        "stale-review",
        "unresolved",
        "missing-run",
        "failed-run",
        "moving-run",
        "partial-run",
        "unknown-input",
        "foreign-run",
        "stale-source",
        "missing-spool",
        "fake-fingerprint",
    ] {
        let (fixture, mut state, mut task, capture, run) = review_fixture();
        match case {
            "request-only" => task.workflows.review.as_mut().unwrap().criteria.clear(),
            "no-generation" => task.workflows.review_generation = 0,
            "stale-review" => task.workflows.review_stale = true,
            "unresolved" => task.workflows.findings.push(workflows::Finding {
                id: 1,
                criterion: task.criteria[0].text.clone(),
                detail: "Needs a deliberate fix".into(),
                next_action: "Inspect support".into(),
                evidence: Vec::new(),
                resolution: None,
            }),
            "missing-run" => task.workflows.review.as_mut().unwrap().criteria[0]
                .evidence
                .retain(|reference| matches!(reference, workflows::EvidenceRef::Source { .. })),
            "failed-run" | "moving-run" | "partial-run" | "unknown-input" | "fake-fingerprint" => {
                let mut stored = stored_run(
                    state.connection.as_ref().unwrap(),
                    task.id,
                    &run.prepared.run_id,
                )
                .unwrap();
                match case {
                    "failed-run" => {
                        stored.record.state = RunState::Failed;
                        stored.record.exit_code = Some(9);
                    }
                    "moving-run" => stored.record.revision_moving = true,
                    "partial-run" => stored.record.truncated = true,
                    "unknown-input" => stored.record.after = InputSnapshot::default(),
                    "fake-fingerprint" => {
                        stored.record.prepared.before = observed("fabricated-caller-revision");
                        stored.record.after = observed("fabricated-caller-revision");
                    }
                    _ => unreachable!(),
                }
                write_run(state.connection.as_ref().unwrap(), &stored).unwrap();
            }
            "foreign-run" => {
                task.workflows.review.as_mut().unwrap().criteria[0]
                    .evidence
                    .push(workflows::EvidenceRef::Run {
                        task_id: task.id + 999,
                        run_id: run.prepared.run_id.clone(),
                    });
                let mut attempt = task.clone();
                attempt.revision += 1;
                assert!(state.save(attempt).is_err());
                continue;
            }
            "stale-source" => fs::write(
                Path::new(&fixture.project).join("source.rs"),
                "fn changed() {}\n",
            )
            .unwrap(),
            "missing-spool" => {
                let stored = stored_run(
                    state.connection.as_ref().unwrap(),
                    task.id,
                    &run.prepared.run_id,
                )
                .unwrap();
                fs::remove_file(state.data_dir.join(stored.output_name)).unwrap();
            }
            _ => unreachable!(),
        }
        let task = commit_review(&mut state, task, "unqualified-review");
        let revision = task.revision;
        let task = handoff(&mut state, task, &capture, "unverified-handoff", revision);
        assert!(load(&mut state, task.id, None).ribbons.is_empty(), "{case}");
    }
}

#[test]
fn receipt_requires_current_revision_and_effective_review_not_ready_caption_or_retry() {
    let (_fixture, mut state, mut task, capture, _) = review_fixture();
    task.workflows.review.as_mut().unwrap().requested = workflows::Verdict::Inconclusive;
    task.revision += 1;
    task = state.save(task).unwrap();
    let revision = task.revision;
    task = handoff(
        &mut state,
        task,
        &capture,
        "ready-without-verification",
        revision,
    );
    assert!(load(&mut state, task.id, None).ribbons.is_empty());
    task.workflows.review.as_mut().unwrap().requested = workflows::Verdict::Accept;
    task.workflows.review_generation += 1;
    task = commit_review(&mut state, task, "actual-accept");
    task = handoff(
        &mut state,
        task,
        &capture,
        "ready-without-verification",
        revision,
    );
    assert_eq!(load(&mut state, task.id, None).ribbons.len(), 1);
    let old_revision = task.revision - 1;
    task = handoff(
        &mut state,
        task,
        &capture,
        "historical-not-current",
        old_revision,
    );
    assert_eq!(load(&mut state, task.id, None).ribbons.len(), 1);
    let current_revision = task.revision;
    task = handoff(
        &mut state,
        task,
        &capture,
        "current-verified",
        current_revision,
    );
    assert_eq!(load(&mut state, task.id, None).ribbons.len(), 2);
}

#[test]
fn source_changes_remain_historical_and_linked_purge_leaves_content_free_once_only_tombstones() {
    let (fixture, mut state, task, capture, run) = review_fixture();
    let task = commit_review(&mut state, task, "historical-accept");
    let revision = task.revision;
    let mut task = handoff(&mut state, task, &capture, "historical-handoff", revision);
    fs::write(
        Path::new(&fixture.project).join("source.rs"),
        "fn changed_after_recording() {}\n",
    )
    .unwrap();
    let stale = load(&mut state, task.id, None);
    assert_eq!(stale.ribbons.len(), 2);
    assert!(
        stale
            .ribbons
            .iter()
            .all(|view| view.freshness == Freshness::Stale)
    );
    fs::remove_file(Path::new(&fixture.project).join("source.rs")).unwrap();
    let unavailable = load(&mut state, task.id, None);
    assert!(
        unavailable
            .ribbons
            .iter()
            .all(|view| view.freshness == Freshness::Unavailable)
    );
    fs::write(
        Path::new(&fixture.project).join("source.rs"),
        "fn retained() {}\n",
    )
    .unwrap();
    state.purge_output(task.id, &run.prepared.run_id).unwrap();
    assert!(load(&mut state, task.id, None).ribbons.is_empty());
    let remaining: i64 = state
        .connection
        .as_ref()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM ribbons WHERE record IS NULL AND support IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 2);
    let current =
        crate::workspace::repo::observe_inputs(Path::new(&fixture.project), &state.data_dir)
            .unwrap();
    let fresh_run = terminal(
        &mut state,
        task.id,
        "later-pass",
        current.fingerprint.as_deref().unwrap(),
        true,
        b"passed again\n",
    );
    task.workflows.review_generation += 1;
    task.workflows.review.as_mut().unwrap().criteria[0].evidence = vec![
        workflows::EvidenceRef::source(&capture).unwrap(),
        workflows::EvidenceRef::run(&fresh_run).unwrap(),
    ];
    task = commit_review(&mut state, task, "never-reaward-review");
    let revision = task.revision;
    task = handoff(
        &mut state,
        task,
        &capture,
        "never-reaward-handoff",
        revision,
    );
    assert!(load(&mut state, task.id, None).ribbons.is_empty());
    assert!(
        call(
            &mut state,
            task.id,
            Operation::SetRibbonHidden {
                kind: Kind::AcceptedReview,
                hidden: false
            }
        )
        .is_err()
    );
    inspect(state.connection.as_ref().unwrap()).unwrap();
}

#[test]
fn learned_move_uses_actual_teach_not_load_rename_relearn_and_release_never_reattributes() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let second = state.add_partner().unwrap();
    let partner = second.last().unwrap().id;
    let learned = taught(&mut state, &task, partner, "Original learned action");
    let original = load(&mut state, task.id, None).ribbons[0].ribbon.clone();
    assert_eq!(original.owner.partner, partner);
    assert_eq!(original.owner.form, 2);
    assert_eq!(
        original.evidence,
        Evidence::Move {
            id: learned.id,
            revision: 1
        }
    );
    let request = |operation| crate::store::moves::Request {
        task_id: task.id,
        partner,
        form: 2,
        operation,
    };
    state
        .moves_request(request(crate::store::moves::Operation::Rename {
            id: learned.id,
            revision: 1,
            name: "Renamed action".into(),
        }))
        .unwrap();
    assert_eq!(
        load(&mut state, task.id, None).ribbons[0].freshness,
        Freshness::Stale
    );
    assert_eq!(load(&mut state, task.id, None).ribbons[0].ribbon, original);
    state.release_partner(partner).unwrap();
    let released = load(&mut state, task.id, None).ribbons.remove(0);
    assert_eq!(released.freshness, Freshness::Unavailable);
    assert_eq!(released.ribbon, original);
    taught(&mut state, &task, 1, "Replacement partner action");
    assert_eq!(load(&mut state, task.id, None).ribbons[0].ribbon, original);
    assert_eq!(row_count(&state), 1);
    inspect(state.connection.as_ref().unwrap()).unwrap();
}

#[test]
fn task_purge_tombstones_survive_until_final_project_cleanup() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let first = fixture.task(&mut state);
    let survivor = fixture.task(&mut state);
    taught(&mut state, &first, 1, "First teach");
    state
        .manage(Management::Delete {
            ids: vec![first.id],
            expired_before: None,
        })
        .unwrap();
    assert!(load(&mut state, survivor.id, None).ribbons.is_empty());
    assert_eq!(row_count(&state), 1);
    taught(&mut state, &survivor, 1, "Later teach");
    assert!(load(&mut state, survivor.id, None).ribbons.is_empty());
    state
        .manage(Management::Delete {
            ids: vec![survivor.id],
            expired_before: None,
        })
        .unwrap();
    assert_eq!(row_count(&state), 0);
    let new_task = fixture.task(&mut state);
    taught(&mut state, &new_task, 1, "New project history");
    assert_eq!(load(&mut state, new_task.id, None).ribbons.len(), 1);
}

#[test]
fn quota_rolls_back_teach_and_visibility_in_the_same_writer_transaction() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    state.policy.quota_bytes = 0;
    use crate::workspace::moves::{Proposal, Provenance, Scope, Steps};
    let operation = crate::store::moves::Operation::Teach {
        proposal: Proposal {
            partner: 1,
            form: 2,
            name: "No partial learning".into(),
            scope: Scope::Project,
            steps: Steps::Run {
                argv: vec!["cargo".into(), "test".into()],
                cwd: ".".into(),
            },
            provenance: Provenance {
                task_id: task.id,
                sources: Vec::new(),
                taught_by: None,
                teacher_name: None,
            },
        },
        replace: None,
    };
    assert!(
        state
            .moves_request(crate::store::moves::Request {
                task_id: task.id,
                partner: 1,
                form: 2,
                operation
            })
            .is_err()
    );
    assert_eq!(row_count(&state), 0);
    let moves: i64 = state
        .connection
        .as_ref()
        .unwrap()
        .query_row("SELECT count(*) FROM moves", [], |row| row.get(0))
        .unwrap();
    assert_eq!(moves, 0);
    state.policy = Policy::default();
    taught(&mut state, &task, 1, "Committed learning");
    state.policy.quota_bytes = 0;
    assert!(
        call(
            &mut state,
            task.id,
            Operation::SetRibbonHidden {
                kind: Kind::LearnedMove,
                hidden: true
            }
        )
        .is_err()
    );
    state.policy = Policy::default();
    assert!(!load(&mut state, task.id, None).ribbons[0].ribbon.hidden);
}

#[test]
fn schema17_migration_is_atomic_preserves_rows_and_never_awards_legacy_sources() {
    let (fixture, mut state, task, _, _) = review_fixture();
    let task = commit_review(&mut state, task, "predecessor-review");
    let learned = taught(&mut state, &task, 1, "Predecessor learned action");
    let mut connection = state.connection.take().unwrap();
    super::super::super::schema::remove_context_schema(&connection);
    connection
        .execute_batch("DROP TABLE ribbons; PRAGMA user_version=17;")
        .unwrap();
    let before: String = connection
        .query_row(
            "SELECT group_concat(record,'|') FROM local_artifacts ORDER BY id",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let predecessor = super::super::super::schema::inspect(&connection).unwrap();
    {
        let transaction = connection.transaction().unwrap();
        super::super::super::schema::migrate(&transaction, predecessor).unwrap();
        inspect(&transaction).unwrap();
        let count: i64 = transaction
            .query_row("SELECT count(*) FROM ribbons", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        17
    );
    assert!(connection.prepare("SELECT * FROM ribbons").is_err());
    connection.execute_batch("VACUUM;").unwrap();
    let pages: i64 = connection
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    connection
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    assert!(initialize(&mut connection).is_err());
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        17
    );
    assert!(connection.prepare("SELECT * FROM ribbons").is_err());
    connection
        .pragma_update(None, "max_page_count", 1_000_000)
        .unwrap();
    initialize(&mut connection).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        20
    );
    let after: String = connection
        .query_row(
            "SELECT group_concat(record,'|') FROM local_artifacts ORDER BY id",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after, before);
    state.connection = Some(connection);
    assert!(load(&mut state, task.id, None).ribbons.is_empty());
    state
        .moves_request(crate::store::moves::Request {
            task_id: task.id,
            partner: 1,
            form: 2,
            operation: crate::store::moves::Operation::Rename {
                id: learned.id,
                revision: 1,
                name: "Legacy renamed".into(),
            },
        })
        .unwrap();
    state
        .moves_request(crate::store::moves::Request {
            task_id: task.id,
            partner: 1,
            form: 2,
            operation: crate::store::moves::Operation::Forget {
                id: learned.id,
                revision: 2,
            },
        })
        .unwrap();
    state
        .moves_request(crate::store::moves::Request {
            task_id: task.id,
            partner: 1,
            form: 2,
            operation: crate::store::moves::Operation::Relearn {
                id: learned.id,
                revision: 3,
                replace: None,
            },
        })
        .unwrap();
    assert!(load(&mut state, task.id, None).ribbons.is_empty());
    drop(state);
    let mut reopened = fixture.open();
    assert!(load(&mut reopened, task.id, None).ribbons.is_empty());
}

#[test]
fn strict_rows_unknown_fields_foreign_owners_and_schema_versions_refuse_without_repair() {
    for mutation in [
        "UPDATE ribbons SET record=json_set(record,'$.unknown',1)",
        "UPDATE ribbons SET record=json_set(record,'$.kind','accepted_review')",
        "UPDATE ribbons SET record=json_set(record,'$.task_id',999999)",
        "UPDATE ribbons SET record=json_set(record,'$.owner.partner',999999)",
        "UPDATE ribbons SET record=json_set(record,'$.evidence.move.revision',0)",
        "ALTER TABLE ribbons ADD COLUMN copied_payload TEXT",
    ] {
        let fixture = Fixture::new();
        let mut state = fixture.open();
        let task = fixture.task(&mut state);
        taught(&mut state, &task, 1, "Strict history");
        state
            .connection
            .as_ref()
            .unwrap()
            .execute_batch(mutation)
            .unwrap();
        assert!(
            inspect(state.connection.as_ref().unwrap()).is_err(),
            "{mutation}"
        );
        drop(state);
        let before = fs::read(fixture.root.join("data").join(DATABASE)).unwrap();
        let mut refused = State {
            data_dir: fixture.root.join("data"),
            project: fixture.project.clone(),
            policy: Policy::default(),
            connection: None,
            lock: None,
        };
        assert!(refused.open_and_load().is_err(), "{mutation}");
        assert_eq!(
            fs::read(fixture.root.join("data").join(DATABASE)).unwrap(),
            before
        );
    }
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    for version in [18, 21] {
        state
            .connection
            .as_ref()
            .unwrap()
            .pragma_update(None, "user_version", version)
            .unwrap();
        let before = fs::read(state.data_dir.join(DATABASE)).unwrap();
        assert!(validate_planning_input(&state.data_dir, &task, &[]).is_err());
        assert!(review_run_observations(&state.data_dir, &task, None).is_err());
        assert_eq!(fs::read(state.data_dir.join(DATABASE)).unwrap(), before);
    }
}

#[test]
fn capture_purge_removes_all_linked_ribbon_metadata_and_keeps_only_kind_tombstones() {
    let (_fixture, mut state, task, capture, _) = review_fixture();
    let task = commit_review(&mut state, task, "purged-source-review");
    let revision = task.revision;
    let task = handoff(
        &mut state,
        task,
        &capture,
        "purged-source-handoff",
        revision,
    );
    state
        .context(ContextRequest {
            task_id: task.id,
            producer: None,
            operation: ContextOperation::Purge(vec![capture.id]),
        })
        .unwrap();
    assert!(load(&mut state, task.id, None).ribbons.is_empty());
    let rows: Vec<(String, Option<String>, Option<String>)> = state
        .connection
        .as_ref()
        .unwrap()
        .prepare("SELECT kind,record,support FROM ribbons ORDER BY kind")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("accepted_review".into(), None, None),
            ("verified_handoff".into(), None, None)
        ]
    );
    inspect(state.connection.as_ref().unwrap()).unwrap();
}

#[test]
fn quota_failure_rolls_back_review_artifact_and_award_without_consuming_first_recorded_slot() {
    let (_fixture, mut state, mut task, _, _) = review_fixture();
    let mut acting = owner(&state, 1);
    acting.form = 7;
    task.revision += 1;
    task.local_records = vec![LocalIntent::Artifact {
        id: "atomic-review".into(),
        owner: acting,
        kind: ArtifactKind::Review,
        reference: "1".into(),
        current: None,
    }];
    state.policy.quota_bytes = 0;
    assert!(state.save(task.clone()).is_err());
    assert_eq!(row_count(&state), 0);
    let artifact: i64 = state
        .connection
        .as_ref()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM local_artifacts WHERE id='atomic-review'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(artifact, 0);
    state.policy = Policy::default();
    let task = state.save(task).unwrap();
    assert_eq!(load(&mut state, task.id, None).ribbons.len(), 1);
}

#[test]
fn active_writer_and_unknown_legacy_artifact_never_supply_award_authority() {
    let (_fixture, mut state, mut task, _, _) = review_fixture();
    task.revision += 1;
    task = state.save(task).unwrap();
    let writing = WorkClaim {
        task_id: task.id,
        owner: owner(&state, 1),
        request_id: "ribbon-concurrent-writer".into(),
        writing: true,
    };
    state
        .work_request(&WorkRequest::Acquire {
            claim: writing.clone(),
            task: Box::new(task.clone()),
        })
        .unwrap();
    let mut task = commit_review(&mut state, task, "review-during-write");
    assert!(load(&mut state, task.id, None).ribbons.is_empty());
    release(&mut state, writing);
    state.connection.as_ref().unwrap().execute(
        "UPDATE local_artifacts SET record=json_set(record,'$.producer',NULL,'$.detail','Effective review: Accept') WHERE id='review-during-write'", []).unwrap();
    assert!(load(&mut state, task.id, None).ribbons.is_empty());
    task.local_records.clear();
    task = commit_review(&mut state, task, "new-qualified-review");
    assert_eq!(load(&mut state, task.id, None).ribbons.len(), 1);
}
