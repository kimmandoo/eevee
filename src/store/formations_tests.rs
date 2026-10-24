use super::{Request, Response};
use crate::store::*;
use crate::workspace::workflows::{FormationSlot, Recipe, RecipeStep};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ev-formation-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
    fn open(&self) -> State {
        let mut state = State {
            data_dir: self.0.join("data"),
            project: self.0.to_str().unwrap().into(),
            policy: Policy::default(),
            connection: None,
            lock: None,
        };
        state.open_and_load().unwrap();
        state
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn recipe() -> Recipe {
    Recipe {
        schema_version: 2,
        name: "Fixture arrangement".into(),
        scope: vec![],
        context: vec![],
        validation_commands: vec![],
        completion_criteria: vec!["Review independently".into()],
        steps: vec![RecipeStep {
            form: "Vaporeon".into(),
            action: "Stage only".into(),
        }],
        roles: [
            ("scout", "Vaporeon", "Inspect source"),
            ("runner", "Jolteon", "Review a check"),
            ("reviewer", "Glaceon", "Assess evidence"),
        ]
        .into_iter()
        .map(|(role, form, request)| FormationSlot {
            role: role.into(),
            form: form.into(),
            request: request.into(),
        })
        .collect(),
    }
}
fn setup(state: &mut State) -> Snapshot {
    state.add_partner().unwrap();
    state.add_partner().unwrap();
    let mut task = Snapshot::empty(state.project.clone());
    task.goal = "Formation retains owned state".into();
    task.partner_drafts.insert(
        1,
        PartnerDrafts {
            drafts: vec![Draft {
                field: EditorField::Request,
                text: "Keep 界e\u{301} suffix".into(),
                cursor: 5,
            }],
            editing: true,
            editor_field: EditorField::Request,
            previous_form: None,
        },
    );
    state.save(task).unwrap()
}
fn request(state: &State, task: &Snapshot, mapping: Vec<Option<i64>>) -> Request {
    Request::Apply {
        expected: Box::new(task.clone()),
        roster: partners(state.connection.as_ref().unwrap()).unwrap(),
        recipe: recipe(),
        mapping,
    }
}
#[test]
fn formation_stages_three_owned_partners_atomically_and_survives_restart() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = setup(&mut state);
    let original = task.partner_drafts[&1].clone();
    let response = state
        .formation(request(&state, &task, vec![Some(1), Some(2), Some(3)]))
        .unwrap();
    let Response::Applied {
        task: applied,
        roster,
    } = response
    else {
        panic!("expected committed formation")
    };
    assert_eq!(
        roster
            .iter()
            .map(|p| (p.form, p.auto_form))
            .collect::<Vec<_>>(),
        vec![(1, false), (2, false), (7, false)]
    );
    assert_eq!(applied.partner_drafts[&1].drafts, original.drafts);
    assert_eq!(applied.partner_drafts[&2].drafts[0].text, "Review a check");
    assert_eq!(applied.partner_drafts[&3].drafts[0].text, "Assess evidence");
    let db = state.connection.as_ref().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM active_work", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM executions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM form_routes", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        3
    );
    drop(state);
    let mut state = fixture.open();
    let restored = state.open_and_load().unwrap();
    assert_eq!(restored.tasks[0], *applied);
    assert_eq!(restored.partners, roster);
}
#[test]
fn late_duplicate_mapping_rolls_back_earlier_partner_and_draft_changes() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = setup(&mut state);
    let before = load(state.connection.as_ref().unwrap(), &state.project).unwrap();
    assert!(
        state
            .formation(request(&state, &task, vec![Some(1), Some(1), Some(3)]))
            .is_err()
    );
    assert_eq!(
        load(state.connection.as_ref().unwrap(), &state.project).unwrap(),
        before
    );
}
#[test]
fn stale_task_roster_busy_owner_and_invalid_definition_refuse_without_partial_application() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = setup(&mut state);
    let stale = request(&state, &task, vec![Some(1), Some(2), None]);
    let mut updated = task.clone();
    updated.revision += 1;
    updated.next_action = "Newer exact fact".into();
    let updated = state.save(updated).unwrap();
    assert!(state.formation(stale).is_err());
    let stale = request(&state, &updated, vec![Some(1), Some(2), None]);
    state.nickname_partner(2, Some("Mizu".into())).unwrap();
    assert!(state.formation(stale).is_err());
    let claim = WorkClaim {
        task_id: updated.id,
        request_id: "formation-busy".into(),
        owner: Attribution::of(&partners(state.connection.as_ref().unwrap()).unwrap()[1]),
        writing: false,
    };
    state
        .work_request(&WorkRequest::Acquire {
            claim,
            task: Box::new(updated.clone()),
        })
        .unwrap();
    let before = load(state.connection.as_ref().unwrap(), &state.project).unwrap();
    assert!(
        state
            .formation(request(&state, &updated, vec![Some(1), Some(2), None]))
            .is_err()
    );
    assert_eq!(
        load(state.connection.as_ref().unwrap(), &state.project).unwrap(),
        before
    );
    let mut invalid = recipe();
    invalid.roles[0].request = "password=not-a-real-credential".into();
    assert!(invalid.validate().is_err());
    invalid = recipe();
    invalid.schema_version = 1;
    assert!(invalid.validate().is_err());
    invalid = recipe();
    invalid.roles[1].role = invalid.roles[0].role.clone();
    assert!(invalid.validate().is_err());
}
#[test]
fn unmapped_roles_remain_empty_and_portable_export_never_overwrites() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = setup(&mut state);
    let path = fixture.0.join("formation.toml");
    state
        .formation(Request::Export {
            path: path.clone(),
            recipe: recipe(),
        })
        .unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(
        state
            .formation(Request::Export {
                path: path.clone(),
                recipe: recipe()
            })
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let Response::Read(definition) = state.formation(Request::Read { path }).unwrap() else {
        panic!("expected definition")
    };
    assert_eq!(definition, recipe());
    let Response::Applied { task, roster } = state
        .formation(request(&state, &task, vec![None, Some(2), None]))
        .unwrap()
    else {
        panic!("expected staged partial mapping")
    };
    assert_eq!(
        roster.iter().map(|p| p.form).collect::<Vec<_>>(),
        vec![0, 2, 0]
    );
    assert!(!task.partner_drafts.contains_key(&3));
}

#[test]
fn previewed_three_party_apply_revalidates_a_late_real_writer_claim_and_then_stages_only_after_release()
 {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = setup(&mut state);
    let reviewed = request(&state, &task, vec![Some(1), Some(2), Some(3)]);
    let original = load(state.connection.as_ref().unwrap(), &state.project).unwrap();
    // The preview has already been constructed. Another partner now obtains
    // actual durable ownership through the same work admission as Submit.
    let claim = WorkClaim {
        task_id: task.id,
        request_id: "late-formation-writer".into(),
        owner: Attribution::of(&original.partners[2]),
        writing: true,
    };
    state
        .work_request(&WorkRequest::Acquire {
            claim: claim.clone(),
            task: Box::new(task.clone()),
        })
        .unwrap();
    let before_refusal = load(state.connection.as_ref().unwrap(), &state.project).unwrap();
    assert!(state.formation(reviewed.clone()).is_err());
    let refused = load(state.connection.as_ref().unwrap(), &state.project).unwrap();
    assert_eq!(refused, before_refusal);
    assert_eq!(refused.tasks, original.tasks);
    assert_eq!(refused.partners, original.partners);
    let db = state.connection.as_ref().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM form_routes", [], |row| row
            .get::<_, usize>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM executions", [], |row| row
            .get::<_, usize>(0))
            .unwrap(),
        0
    );
    state.work_request(&WorkRequest::Release(claim)).unwrap();
    let Response::Applied {
        task: staged,
        roster,
    } = state.formation(reviewed).unwrap()
    else {
        panic!("expected independently revalidated staging after release");
    };
    assert_eq!(staged.lead_partner, task.lead_partner);
    assert_eq!(
        staged.partner_drafts[&1].drafts,
        task.partner_drafts[&1].drafts
    );
    assert_eq!(staged.partner_drafts[&2].drafts[0].text, "Review a check");
    assert_eq!(staged.partner_drafts[&3].drafts[0].text, "Assess evidence");
    assert_eq!(
        roster
            .iter()
            .map(|partner| partner.form)
            .collect::<Vec<_>>(),
        vec![1, 2, 7]
    );
    let db = state.connection.as_ref().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM active_work", [], |row| row
            .get::<_, usize>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM executions", [], |row| row
            .get::<_, usize>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM provider_journal", [], |row| row
            .get::<_, usize>(0))
            .unwrap(),
        0
    );
}

#[test]
fn mapped_draft_or_roster_races_invalidate_the_complete_review_without_partial_writes() {
    for roster_race in [false, true] {
        let fixture = Fixture::new();
        let mut state = fixture.open();
        let task = setup(&mut state);
        let reviewed = request(&state, &task, vec![Some(1), Some(2), Some(3)]);
        if roster_race {
            state.nickname_partner(3, Some("NewName".into())).unwrap();
        } else {
            let mut newer = task.clone();
            newer.revision += 1;
            newer.partner_drafts.insert(
                3,
                PartnerDrafts {
                    drafts: vec![Draft {
                        field: EditorField::Request,
                        text: "Newer 界 exact draft".into(),
                        cursor: 6,
                    }],
                    ..PartnerDrafts::default()
                },
            );
            state.save(newer).unwrap();
        }
        let before = load(state.connection.as_ref().unwrap(), &state.project).unwrap();
        assert!(state.formation(reviewed).is_err());
        assert_eq!(
            load(state.connection.as_ref().unwrap(), &state.project).unwrap(),
            before
        );
        assert_eq!(
            state
                .connection
                .as_ref()
                .unwrap()
                .query_row("SELECT count(*) FROM form_routes", [], |row| row
                    .get::<_, usize>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn portable_formation_tables_remain_semantic_only_after_local_party_staging() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = setup(&mut state);
    state
        .formation(request(&state, &task, vec![Some(1), Some(2), Some(3)]))
        .unwrap();
    let definition = recipe();
    let path = fixture.0.join("portable-only.toml");
    state
        .formation(Request::Export {
            path: path.clone(),
            recipe: definition.clone(),
        })
        .unwrap();
    let text = fs::read_to_string(path).unwrap();
    assert_eq!(toml::from_str::<Recipe>(&text).unwrap(), definition);
    let value: toml::Value = toml::from_str(&text).unwrap();
    let table = value.as_table().unwrap();
    let allowed = [
        "schema_version",
        "name",
        "scope",
        "context",
        "steps",
        "validation_commands",
        "completion_criteria",
        "roles",
    ];
    assert!(table.keys().all(|key| allowed.contains(&key.as_str())));
    for slot in table["roles"].as_array().unwrap() {
        let slot = slot.as_table().unwrap();
        assert_eq!(slot.len(), 3);
        assert!(
            slot.keys()
                .all(|key| ["role", "form", "request"].contains(&key.as_str()))
        );
    }
    assert!(toml::from_str::<Recipe>(&format!("readiness = 'configured'\n{text}")).is_err());
}

fn workflow_run(store: &mut State, task: &Snapshot) -> crate::workspace::runner::RunRecord {
    use crate::workspace::runner::*;
    let now = timestamp().unwrap() as u64;
    let snapshot = InputSnapshot {
        fingerprint: Some("prior-reviewed-source".into()),
        complete: true,
        stable: true,
    };
    let prepared = Prepared {
        request_id: "request-workflow".into(),
        run_id: "run-workflow".into(),
        task_id: task.id,
        spec: CommandSpec::Direct {
            program: "cargo".into(),
            args: vec!["test".into()],
        },
        cwd: PathBuf::from(&task.project),
        cwd_identity: "fixture-cwd".into(),
        executable: ExecutableIdentity {
            path: PathBuf::from("/fixture/cargo"),
            sha256: "0".repeat(64),
            bytes: 1,
            modified_ns: 1,
            file_id: "fixture-executable".into(),
        },
        prepared_at_ms: now,
        expires_at_ms: now + 60_000,
        approval_arrival: 1,
        before: snapshot.clone(),
        partner: Some(Attribution::of(&Partner::default())),
        rerun_of: None,
    };
    store.reserve_run(task.id, prepared.clone(), 1024).unwrap();
    let mut record = RunRecord::awaiting(&prepared);
    record.state = RunState::Succeeded;
    record.started_at_ms = Some(now);
    record.ended_at_ms = Some(now + 1);
    record.exit_code = Some(0);
    record.tree = TreeOutcome::OwnedTreeExited;
    record.after = snapshot;
    store.update_run(task.id, record).unwrap()
}
fn workflow_export(
    task: &Snapshot,
    run: &crate::workspace::runner::RunRecord,
    path: &str,
) -> crate::workspace::workflows::authoring::Export {
    crate::workspace::workflows::authoring::Draft {
        name: "Reviewed workflow".into(),
        scope: "missing.rs".into(),
        criteria: "Inspect actual results".into(),
        steps: "Flareon: Request a reviewed change".into(),
        path: path.into(),
        runs: vec![run.prepared.run_id.clone()],
    }
    .preview(
        9,
        task.id,
        task.lead_partner,
        task.revision,
        Path::new(&task.project),
        std::slice::from_ref(run),
    )
    .unwrap()
}
#[test]
fn workflow_publication_requires_exact_committed_owner_revision_and_retained_output() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = setup(&mut state);
    let run = workflow_run(&mut state, &task);
    let export = workflow_export(&task, &run, "workflow.toml");
    let before = load(state.connection.as_ref().unwrap(), &state.project).unwrap();
    assert!(matches!(
        state
            .formation(Request::ExportWorkflow(Box::new(export.clone())))
            .unwrap(),
        Response::WorkflowExported { generation: 9 }
    ));
    let bytes = fs::read(export.path()).unwrap();
    assert!(
        state
            .formation(Request::ExportWorkflow(Box::new(export)))
            .is_err()
    );
    assert_eq!(fs::read(fixture.0.join("workflow.toml")).unwrap(), bytes);
    assert_eq!(
        load(state.connection.as_ref().unwrap(), &state.project).unwrap(),
        before
    );

    let stale = workflow_export(&task, &run, "stale.toml");
    let mut updated = task.clone();
    updated.revision += 1;
    let updated = state.save(updated).unwrap();
    assert!(
        state
            .formation(Request::ExportWorkflow(Box::new(stale)))
            .is_err()
    );
    assert!(!fixture.0.join("stale.toml").exists());

    let stale = workflow_export(&updated, &run, "changed-evidence.toml");
    let mut changed = run.clone();
    changed.output_unavailable = true;
    state
        .connection
        .as_ref()
        .unwrap()
        .execute(
            "UPDATE executions SET record=?1 WHERE run_id=?2",
            params![run_json(&changed).unwrap(), run.prepared.run_id],
        )
        .unwrap();
    assert!(
        state
            .formation(Request::ExportWorkflow(Box::new(stale)))
            .is_err()
    );
    assert!(!fixture.0.join("changed-evidence.toml").exists());
    state
        .connection
        .as_ref()
        .unwrap()
        .execute(
            "UPDATE executions SET record=?1 WHERE run_id=?2",
            params![run_json(&run).unwrap(), run.prepared.run_id],
        )
        .unwrap();

    let missing_output = workflow_export(&updated, &run, "missing-output.toml");
    let retained = stored_run(
        state.connection.as_ref().unwrap(),
        task.id,
        &run.prepared.run_id,
    )
    .unwrap();
    state
        .connection
        .as_ref()
        .unwrap()
        .execute(
            "UPDATE executions SET output_identity='foreign-output' WHERE run_id=?1",
            [&run.prepared.run_id],
        )
        .unwrap();
    assert!(
        state
            .formation(Request::ExportWorkflow(Box::new(missing_output)))
            .is_err()
    );
    assert!(!fixture.0.join("missing-output.toml").exists());
    state
        .connection
        .as_ref()
        .unwrap()
        .execute(
            "UPDATE executions SET output_identity=?1 WHERE run_id=?2",
            params![retained.output_identity, run.prepared.run_id],
        )
        .unwrap();

    let mut foreign_owner = updated.clone();
    foreign_owner.lead_partner = 2;
    let export = workflow_export(&foreign_owner, &run, "foreign-owner.toml");
    assert!(
        state
            .formation(Request::ExportWorkflow(Box::new(export)))
            .is_err()
    );
    assert!(!fixture.0.join("foreign-owner.toml").exists());

    let export = workflow_export(&updated, &run, "busy.toml");
    let owner = Attribution::of(&partners(state.connection.as_ref().unwrap()).unwrap()[1]);
    state
        .work_request(&WorkRequest::Acquire {
            claim: WorkClaim {
                task_id: task.id,
                request_id: "workflow-busy".into(),
                owner,
                writing: false,
            },
            task: Box::new(updated),
        })
        .unwrap();
    assert!(
        state
            .formation(Request::ExportWorkflow(Box::new(export)))
            .is_err()
    );
    assert!(!fixture.0.join("busy.toml").exists());
}

#[test]
fn apply_rejects_same_revision_draft_or_project_mismatch_instead_of_trusting_the_caller_snapshot() {
    for project_mismatch in [false, true] {
        let fixture = Fixture::new();
        let mut state = fixture.open();
        let task = setup(&mut state);
        let mut reviewed = request(&state, &task, vec![Some(1), Some(2), Some(3)]);
        let Request::Apply { expected, .. } = &mut reviewed else {
            unreachable!()
        };
        if project_mismatch {
            expected.project = fixture
                .0
                .join("another-project")
                .to_string_lossy()
                .into_owned();
        } else {
            expected.partner_drafts.get_mut(&1).unwrap().drafts[0].cursor = 4;
        }
        let before = load(state.connection.as_ref().unwrap(), &state.project).unwrap();
        assert!(state.formation(reviewed).is_err());
        assert_eq!(
            load(state.connection.as_ref().unwrap(), &state.project).unwrap(),
            before
        );
    }
}
