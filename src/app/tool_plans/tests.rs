use super::*;

fn staged() -> App {
    let mut app = App::new("/project".into(), 120, 36);
    app.task_id = 7;
    app.editing = false;
    app.storage = StorageState::Saved;
    app.revision = 1;
    app.saved_revision = 1;
    let candidate = Plan::new(
        "Inspect".into(),
        vec![NodeSpec {
            id: "read".into(),
            dependencies: vec![],
            operation: Operation::Read {
                path: "src/lib.rs".into(),
                start_line: 1,
                end_line: 1,
            },
            reason: "Inspect source".into(),
        }],
        7,
        app.lead.id,
        app.cwd.clone(),
        Default::default(),
    )
    .unwrap();
    app.workflows.tool_plans = Some(candidate);
    let mut task = store::Snapshot::empty(app.cwd.clone());
    task.id = 7;
    task.revision = 1;
    task.workflows = app.workflows.clone();
    app.tasks = vec![task];
    app
}
fn review(app: &mut App) {
    app.preview_tool_plan(Review::Start);
    assert!(app.tool_plans.review == Some(Review::Start));
    app.tool_plans.reviewed.set(true);
}
#[test]
fn no_persisted_plan_ack_no_start_and_stale_review_starts_nothing() {
    let mut app = staged();
    review(&mut app);
    app.workflows.tool_plans.as_mut().unwrap().nodes[0]
        .spec
        .reason = "Changed exact reason".into();
    app.start_tool_plan();
    assert!(!app.tool_plans.active);
    assert!(app.tool_plans.worker.is_none());
    assert!(!app.execution.busy());
    let mut app = staged();
    review(&mut app);
    app.storage = StorageState::Failed("Writer failed".into());
    app.start_tool_plan();
    assert!(!app.tool_plans.active);
    assert_eq!(app.workflows.tool_plans.as_ref().unwrap().attempts, 0);
}
#[test]
fn undersize_or_unexposed_graph_never_admits_and_restart_never_schedules() {
    let mut app = staged();
    review(&mut app);
    app.height = 10;
    app.start_tool_plan();
    assert!(!app.tool_plans.active);
    app.height = 36;
    app.preview_tool_plan(Review::Start);
    app.start_tool_plan();
    assert!(!app.tool_plans.active);
    app.workflows.tool_plans.as_mut().unwrap().nodes[0].status = Status::Running;
    app.restore_tool_plan();
    assert!(!app.tool_plans.active);
    assert!(app.tool_plans.worker.is_none());
    assert_eq!(
        app.workflows.tool_plans.as_ref().unwrap().nodes[0].status,
        Status::Failed
    );
}
#[test]
fn history_inspection_does_not_transfer_execution_authority() {
    let mut app = staged();
    app.workflows
        .tool_plan_history
        .push(app.workflows.tool_plans.clone().unwrap());
    app.tool_plans.history_index = Some(0);
    for code in ['s', 'r', 'b', 'y', 'c', 'j'] {
        app.tool_plan_key(KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE));
        assert!(!app.tool_plans.active);
        assert!(app.tool_plans.review.is_none());
    }
    assert_eq!(
        app.workflows.tool_plan_history[0].nodes[0].status,
        Status::Pending
    );
}
#[test]
fn bounded_history_preserves_completed_plans_and_refuses_unresolved_windows() {
    let mut app = staged();
    let p = app.workflows.tool_plans.as_mut().unwrap();
    p.attempts = 1;
    p.nodes[0].status = Status::Succeeded;
    let nodes = p.nodes.iter().map(|node| node.spec.clone()).collect();
    app.stage_tool_plan("Next".into(), nodes).unwrap();
    assert_eq!(app.workflows.tool_plan_history.len(), 1);
    assert_eq!(
        app.workflows.tool_plan_history[0].nodes[0].status,
        Status::Succeeded
    );
    app.storage = StorageState::Saved;
    app.saved_revision = app.revision;
    app.save_requested = false;
    let p = app.workflows.tool_plans.as_mut().unwrap();
    p.attempts = 1;
    p.nodes[0].status = Status::Unknown;
    p.nodes[0].certainty = Certainty::Unknown;
    let nodes = p.nodes.iter().map(|node| node.spec.clone()).collect();
    assert!(
        app.stage_tool_plan("Unsafe replacement".into(), nodes)
            .is_err()
    );
}

#[test]
fn quitting_an_active_read_plan_requires_cancel_review_instead_of_save_deadlock() {
    let mut app = staged();
    review(&mut app);
    app.start_tool_plan();
    assert!(app.tool_plans.active);
    app.request_quit();
    assert!(app.cooperation.cancel_review.is_some());
    assert!(!app.quit_after_save);
    assert!(!app.quit);
    app.cancel_tool_plan();
    assert!(!app.tool_plans.active);
}

fn interrupted_command() -> (App, runner::RunRecord) {
    let mut app = staged();
    let spec = runner::CommandSpec::Direct {
        program: "fixture-command".into(),
        args: vec!["exact".into()],
    };
    let source = runner::InputSnapshot {
        fingerprint: Some("reviewed-source".into()),
        complete: true,
        stable: true,
    };
    let mut descendant = app.workflows.tool_plans.as_ref().unwrap().nodes[0]
        .spec
        .clone();
    descendant.dependencies = vec!["command".into()];
    let mut plan = Plan::new(
        "Refusal".into(),
        vec![
            NodeSpec {
                id: "command".into(),
                dependencies: vec![],
                operation: Operation::Command { spec: spec.clone() },
                reason: "Exact separate approval".into(),
            },
            descendant,
        ],
        app.task_id,
        app.lead.id,
        app.cwd.clone(),
        source.clone(),
    )
    .unwrap();
    let prepared = runner::Prepared {
        request_id: "refused-request".into(),
        run_id: "refused-run".into(),
        task_id: app.task_id,
        spec,
        cwd: app.cwd.clone().into(),
        cwd_identity: "fixture-cwd".into(),
        executable: runner::ExecutableIdentity {
            path: "fixture-command".into(),
            sha256: "fixture-digest".into(),
            bytes: 1,
            modified_ns: 0,
            file_id: "fixture-file".into(),
        },
        prepared_at_ms: 1,
        expires_at_ms: 100,
        approval_arrival: 1,
        before: source,
        partner: Some(store::Attribution::of(&app.lead)),
        rerun_of: None,
    };
    plan.nodes[0].status = Status::Running;
    plan.nodes[0].run_id = Some(prepared.run_id.clone());
    app.workflows.tool_plans = Some(plan);
    app.tool_plans.active = true;
    app.tool_plans.command = Some(0);
    app.tool_plans.command_preparing = true;
    let mut record = runner::RunRecord::awaiting(&prepared);
    record.state = runner::RunState::Interrupted;
    record.ended_at_ms = Some(2);
    record.error =
        Some("Approval was invalidated or dispatch was refused; command was not started".into());
    record.output_unavailable = true;
    (app, record)
}

#[test]
fn durable_undispatched_refusal_settles_plan_without_inventing_execution() {
    let (mut app, record) = interrupted_command();
    let source = app.workflows.tool_plans.as_ref().unwrap().source.clone();
    app.execution.history.push(record);
    assert!(app.drive_tool_plans(std::path::Path::new("/unused"), &[]));
    assert!(app.tool_plans.command.is_none());
    assert!(!app.tool_plans.command_preparing);
    let plan = app.workflows.tool_plans.as_ref().unwrap();
    let node = &plan.nodes[0];
    assert_eq!(node.status, Status::Failed);
    assert_eq!(node.certainty, Certainty::NoEffect);
    assert_eq!(node.started_at_ms, None);
    assert_eq!(node.ended_at_ms, Some(2));
    assert_eq!(node.elapsed_us, 0);
    assert_eq!(plan.execution_count, 0);
    assert_eq!(plan.source, source);
    assert_eq!(plan.nodes[1].status, Status::Blocked);
    assert!(plan.ready().is_empty());
    let evidence: serde_json::Value = serde_json::from_str(&node.result).unwrap();
    assert_eq!(evidence["state"], "Interrupted");
    assert_eq!(evidence["tree"], "NotStarted");
    assert!(evidence["exit_code"].is_null());
    assert_eq!(plan.resume_candidates().unwrap(), vec![0, 1]);
}

#[test]
fn interruption_without_definite_no_start_evidence_remains_fail_stop() {
    for (started, tree) in [
        (None, runner::TreeOutcome::OwnershipUnknown),
        (Some(1), runner::TreeOutcome::OwnershipUnknown),
        (Some(1), runner::TreeOutcome::OwnedTreeExited),
    ] {
        let (mut app, mut record) = interrupted_command();
        record.started_at_ms = started;
        record.tree = tree;
        let plan = app.workflows.tool_plans.as_mut().unwrap();
        apply_record(&mut plan.nodes[0], &record);
        plan.block_descendants();
        assert_eq!(plan.nodes[0].status, Status::Unknown);
        assert_eq!(plan.nodes[0].certainty, Certainty::Unknown);
        assert_eq!(plan.nodes[1].status, Status::Blocked);
        assert!(plan.ready().is_empty());
        assert!(plan.resume_candidates().is_err());
    }
}
