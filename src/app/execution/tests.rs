use super::*;
use std::collections::BTreeMap;
use std::sync::Arc;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn prepared() -> runner::Prepared {
    runner::Prepared {
        request_id: "r-test-request".into(),
        run_id: "r-test-run".into(),
        task_id: 1,
        approval_arrival: 1,
        spec: runner::CommandSpec::Direct {
            program: "tool".into(),
            args: vec!["中文 space;literal".into()],
        },
        cwd: "/project 中文".into(),
        cwd_identity: "directory-identity".into(),
        executable: runner::ExecutableIdentity {
            path: "/tool path/tool".into(),
            sha256: "hash".into(),
            bytes: 10,
            modified_ns: 1,
            file_id: "file-id".into(),
        },
        prepared_at_ms: 1,
        expires_at_ms: u64::MAX,
        before: identity("same"),
        partner: None,
        rerun_of: None,
    }
}
fn identity(value: &str) -> runner::InputSnapshot {
    runner::InputSnapshot {
        fingerprint: Some(value.into()),
        complete: true,
        stable: true,
    }
}
fn pending() -> State {
    let p = prepared();
    State {
        task_id: 1,
        request_id: Some(p.request_id.clone()),
        prepared: Some(p),
        shown: true,
        ..State::default()
    }
}

#[test]
fn global_execute_from_help_reviews_the_same_prepared_request_without_dispatch() {
    let mut app = crate::app::App::new("/project".into(), 120, 36);
    app.task_id = 1;
    app.lead.form = 2;
    app.editing = false;
    app.execution = pending();
    let expected = app.execution.prepared.clone().unwrap();
    app.command("help");
    app.command("execute");
    assert_eq!(app.overlay, Some(crate::app::Overlay::Execution));
    assert_eq!(app.execution.approval_review.as_ref(), Some(&expected));
    assert!(matches!(
        app.execution.confirmation,
        Some(Confirmation::Approval)
    ));
    assert!(!app.execution.affirmative);
    assert!(app.execution.active.is_none());
    assert!(app.execution.permit.is_none());
}
fn record(state: runner::RunState) -> runner::RunRecord {
    let mut record = runner::RunRecord::awaiting(&prepared());
    record.state = state;
    record.started_at_ms = Some(10);
    record.ended_at_ms = Some(20);
    record.exit_code = Some(0);
    record.tree = runner::TreeOutcome::OwnedTreeExited;
    record.after = identity("same");
    record
}
fn report(byte: u8, complete: bool) -> repo::Report {
    let path = std::path::PathBuf::from("src/中文 file.rs");
    let mut files = BTreeMap::new();
    files.insert(
        path.clone(),
        repo::FileIdentity {
            path,
            tracked: true,
            head: None,
            index: vec![],
            state: repo::FileState::Regular {
                sha256: [byte; 32],
                bytes: 1,
                binary: false,
                executable: false,
            },
            dirty: Some(false),
        },
    );
    let snapshot = Arc::new(repo::Snapshot {
        head: Some("head".into()),
        files,
        working_diff: repo::WorkingDiff::default(),
        complete,
        stable: true,
        warnings: vec![],
    });
    repo::Report {
        repository: repo::RepositoryIdentity {
            root: "/project".into(),
            git_root: Some("/project".into()),
            git_dir: Some("/project/.git".into()),
            storage_root: None,
        },
        baseline: Arc::clone(&snapshot),
        current: snapshot,
        changes: vec![],
        initial: true,
    }
}

#[test]
fn command_json_preserves_literal_arguments_and_requires_explicit_shell_dialect() {
    let parsed = parse_command(r#"["tool","中文 space","$(touch nope);x|y",""]"#).unwrap();
    assert_eq!(
        parsed,
        runner::CommandSpec::Direct {
            program: "tool".into(),
            args: vec!["中文 space".into(), "$(touch nope);x|y".into(), "".into()]
        }
    );
    assert!(parse_command("tool test && other").is_err());
    assert!(parse_command(r#"{"shell":"/bin/sh","text":"echo no"}"#).is_err());
    assert!(
        parse_command(r#"{"shell":"/bin/sh","dialect":"posix","text":"echo ok","extra":1}"#)
            .is_err()
    );
    assert!(matches!(
        parse_command(r#"{"shell":"/bin/sh","dialect":"posix","text":"echo exact | cat"}"#)
            .unwrap(),
        runner::CommandSpec::Shell {
            dialect: runner::ShellDialect::Posix,
            ..
        }
    ));
}

#[test]
fn approval_is_neutral_dismissible_one_shot_and_never_focus_stealing() {
    let p = prepared();
    let mut state = State {
        task_id: 1,
        request_id: Some(p.request_id.clone()),
        ..State::default()
    };
    state.runner_event(runner::Event::Prepared(p));
    assert!(!state.visible());
    assert!(state.confirmation.is_none());
    state.command("execute");
    assert!(!state.affirmative);
    state.key(key(KeyCode::Enter));
    assert!(state.queue.is_empty());
    assert!(state.prepared.is_some());
    state.command("execute");
    state.key(key(KeyCode::Right));
    state.shown = true;
    state.confirmation_visible = true;
    state.key(key(KeyCode::Enter));
    assert!(matches!(state.queue.front(), Some(StoreJob::Reserve(..))));
    assert_eq!(state.queue.len(), 1);
    state.key(key(KeyCode::Enter));
    state.command("execute");
    assert_eq!(state.queue.len(), 1);
}

#[test]
fn confirmation_shortcuts_require_visible_scope_press_and_matching_request() {
    let mut state = pending();
    state.command("execute");
    state.key(key(KeyCode::Char('y')));
    assert!(state.queue.is_empty());
    state.confirmation_visible = true;
    for event in [
        KeyEvent::new_with_kind(KeyCode::Char('y'), KeyModifiers::NONE, KeyEventKind::Repeat),
        KeyEvent::new_with_kind(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        ),
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT),
    ] {
        state.key(event);
        assert!(state.queue.is_empty());
        assert!(!state.affirmative);
    }
    state.key(key(KeyCode::Char('n')));
    assert!(state.confirmation.is_none() && state.queue.is_empty());
    assert!(state.prepared.is_some(), "Stay does not discard command");
    state.command("execute");
    state.confirmation_visible = true;
    state.key(key(KeyCode::Char('y')));
    assert!(matches!(state.queue.front(), Some(StoreJob::Reserve(..))));
    state.key(key(KeyCode::Char('y')));
    assert_eq!(state.queue.len(), 1);

    for change_task in [false, true] {
        let mut state = pending();
        state.command("execute");
        state.confirmation_visible = true;
        if change_task {
            state.prepared.as_mut().unwrap().task_id += 1;
        } else {
            state
                .prepared
                .as_mut()
                .unwrap()
                .request_id
                .push_str("-stale");
        }
        state.key(key(KeyCode::Char('y')));
        assert!(
            state.queue.is_empty(),
            "mismatched scope cannot reserve execution"
        );
    }
}

#[test]
fn edit_and_explicit_denial_invalidate_grants_but_dismissal_does_not() {
    let mut state = pending();
    state.command("execute");
    state.key(key(KeyCode::Esc));
    assert!(state.request_id.is_some());
    state.command("execute");
    state.shown = true;
    state.command("deny");
    assert!(state.invalidation && state.request_id.is_none() && state.prepared.is_none());
    let mut state = pending();
    assert_eq!(state.command("command-edit"), Some(Action::EditCommand));
    assert!(state.invalidation && state.prepared.is_none());
}

#[test]
fn source_preflight_rejects_an_older_inflight_observation() {
    let mut state = State::default();
    assert!(state.prepare_command(1, r#"["tool","first"]"#));
    state.repo_started();
    assert!(state.prepare_command(1, r#"["tool","second"]"#));
    state.repo_observed(Some(&report(1, true)));
    assert!(state.before_ready.is_none());
    assert!(state.repo_requested());
    state.repo_started();
    state.repo_observed(Some(&report(2, true)));
    assert!(
        state
            .before_ready
            .as_ref()
            .is_some_and(|s| s.complete && s.stable)
    );
}

#[test]
fn completed_run_waits_for_postflight_and_committed_journal_before_navigation() {
    let mut state = pending();
    state.prepared = None;
    let mut finished = record(runner::RunState::Succeeded);
    state.active = Some(record(runner::RunState::Running));
    state.deferred = Some(Action::SwitchTask(2));
    state.runner_event(runner::Event::Finished(finished.clone()));
    assert!(state.busy() && state.ready.is_none());
    state.repo_started();
    state.repo_observed(None);
    assert!(state.final_ack.is_some());
    assert!(state.take_store_request().is_some());
    finished.after = runner::InputSnapshot::default();
    state.store_event(
        1,
        Some("r-test-request"),
        Some("r-test-run"),
        Ok(store::RunResponse::Updated(finished)),
    );
    assert!(!state.busy());
    assert_eq!(state.ready, Some(Action::SwitchTask(2)));
    assert!(state.history[0].after.fingerprint.is_none());
}

#[test]
fn journal_failure_is_retained_and_retry_never_enqueues_execution() {
    let mut state = pending();
    state.prepared = None;
    let running = record(runner::RunState::Running);
    state.active = Some(running.clone());
    state.critical(StoreJob::Update(running));
    assert!(state.take_store_request().is_some());
    state.store_event(
        1,
        Some("r-test-request"),
        Some("r-test-run"),
        Err("disk full".into()),
    );
    assert!(state.failed.is_some() && state.busy());
    assert!(state.take_store_request().is_none());
    state.command("journal-retry");
    let request = state.take_store_request().unwrap();
    assert!(matches!(request.operation, store::RunOperation::Update(_)));
    assert!(!state.dispatching && !state.reserving);
}

#[test]
fn quit_commands_are_neutral_and_search_accepts_literal_q() {
    let mut state = pending();
    assert_eq!(state.command("quit"), None);
    assert!(matches!(
        state.confirmation,
        Some(Confirmation::Navigate(Action::Quit))
    ));
    assert!(!state.affirmative);
    state.command("quit");
    state.key(key(KeyCode::Enter));
    assert!(state.ready.is_none() && state.deferred.is_none());
    state.searching = true;
    state.key(key(KeyCode::Char('q')));
    assert_eq!(state.search, "q");
    assert_eq!(
        state.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        None
    );
    assert!(state.confirmation.is_some());
}

#[test]
fn explicit_stay_revokes_an_already_deferred_exit() {
    let mut state = pending();
    state.deferred = Some(Action::Quit);
    state.confirmation = Some(Confirmation::Navigate(Action::Quit));
    state.key(key(KeyCode::Enter));
    assert!(state.deferred.is_none() && state.ready.is_none());
    assert!(state.prepared.is_some());
}

#[test]
fn unknown_outcome_requires_an_informed_quit_even_after_journal_ack() {
    let mut state = State {
        final_unknown: true,
        ..State::default()
    };
    assert_eq!(state.request_navigation(Action::Quit), None);
    assert!(matches!(
        state.confirmation,
        Some(Confirmation::UnknownExit(Action::Quit))
    ));
    state.key(key(KeyCode::Enter));
    assert!(state.ready.is_none());
    state.request_navigation(Action::Quit);
    state.key(key(KeyCode::Right));
    assert_eq!(state.key(key(KeyCode::Enter)), Some(Action::Quit));
}

#[test]
fn output_pause_search_and_eviction_keep_logical_anchors() {
    let mut state = State {
        task_id: 1,
        live_run: Some("r-test-run".into()),
        follow: false,
        anchor: 10,
        ..State::default()
    };
    for index in 0..5000 {
        state.runner_event(runner::Event::Output(runner::Output {
            task_id: 1,
            request_id: "r-test-request".into(),
            run_id: "r-test-run".into(),
            stream: runner::OutputStream::Stdout,
            at_ms: index,
            text: format!("line {index} 中文\n"),
        }));
    }
    assert!(state.live.lines.len() <= LIVE_LINES && state.live.bytes <= LIVE_BYTES);
    assert!(state.live.evicted > 0 && state.new_output > 0);
    assert_eq!(state.anchor, 10);
    state.set_viewport(8);
    assert_eq!(state.anchor, state.live.first);
    state.search = "line 4990".into();
    state.find(false);
    let anchor = state.anchor;
    assert!(
        state
            .live
            .lines
            .iter()
            .any(|line| line.id == anchor && line.text.contains("4990"))
    );
    state.command("follow");
    assert!(state.follow && state.new_output == 0);
}

#[test]
fn evidence_never_calls_missing_partial_interrupted_or_changed_inputs_verified() {
    let mut result = record(runner::RunState::Succeeded);
    assert!(evidence_label(&result, &identity("same")).contains("not a live watch"));
    assert!(evidence_label(&result, &identity("later")).contains("Stale"));
    result.after = identity("during");
    assert!(evidence_label(&result, &identity("during")).contains("Ambiguous"));
    result.output_unavailable = true;
    result.truncated = true;
    result.state = runner::RunState::Interrupted;
    let label = evidence_label(&result, &identity("during"));
    assert!(
        label.contains("Interrupted")
            && label.contains("unavailable")
            && label.contains("Truncated")
    );
    assert!(label.contains("cannot substantiate"));
}

#[test]
fn native_source_identity_hash_covers_content_and_honest_coverage() {
    assert_ne!(
        fingerprint(&report(1, true)).fingerprint,
        fingerprint(&report(2, true)).fingerprint
    );
    assert!(fingerprint(&report(1, true)).complete);
    assert!(!fingerprint(&report(1, false)).complete);
    assert!(!safe_text("evil\u{1b}]52;clipboard\u{7}").contains('\u{1b}'));
}

#[test]
fn active_form_changes_use_the_same_neutral_owned_work_guard() {
    let mut state = pending();
    assert_eq!(state.request_navigation(Action::SelectForm(4)), None);
    assert!(!state.affirmative);
    state.key(key(KeyCode::Enter));
    assert!(state.prepared.is_some());
    state.request_navigation(Action::SelectForm(4));
    state.key(key(KeyCode::Right));
    assert_eq!(state.key(key(KeyCode::Enter)), Some(Action::SelectForm(4)));
    assert!(state.invalidation && state.prepared.is_none());
}

#[test]
fn partial_physical_purge_keeps_committed_unavailable_metadata_and_warning() {
    let mut state = State {
        task_id: 1,
        selected_run: Some("r-test-run".into()),
        ..State::default()
    };
    state.inflight = Some(StoreJob::Purge {
        task: 1,
        run: "r-test-run".into(),
    });
    let mut record = record(runner::RunState::Succeeded);
    record.output_unavailable = true;
    state.store_event(
        1,
        None,
        Some("r-test-run"),
        Ok(store::RunResponse::Purged {
            record,
            warning: Some("output file cleanup refused a changed identity".into()),
        }),
    );
    assert!(state.history[0].output_unavailable);
    assert!(state.retained_page.as_ref().unwrap().unavailable);
    assert!(
        state
            .error
            .as_ref()
            .unwrap()
            .message
            .contains("changed identity")
    );
    assert!(!state.busy());
}

#[test]
fn late_foreign_started_stays_owned_until_terminal_journal_ack() {
    let mut state = State {
        task_id: 1,
        ..State::default()
    };
    let mut running = record(runner::RunState::Running);
    running.prepared.task_id = 2;
    state.runner_event(runner::Event::Started(running.clone()));
    assert!(state.busy() && state.cancellation);
    assert!(state.take_store_request().is_some());
    state.store_event(
        2,
        Some("r-test-request"),
        Some("r-test-run"),
        Ok(store::RunResponse::Updated(running.clone())),
    );
    assert!(state.busy());
    assert!(
        state.history.is_empty(),
        "foreign evidence leaked to displayed task"
    );
    running.state = runner::RunState::Cancelled;
    running.cancelled = true;
    state.runner_event(runner::Event::Finished(running.clone()));
    state.repo_started();
    state.repo_observed(Some(&report(1, true)));
    assert!(state.take_store_request().is_some());
    running.after = runner::InputSnapshot::default();
    state.store_event(
        2,
        Some("r-test-request"),
        Some("r-test-run"),
        Ok(store::RunResponse::Updated(running)),
    );
    assert!(!state.busy());
    assert!(state.history.is_empty());
}

#[test]
fn output_page_boundaries_are_byte_exact_and_backpressure_keeps_previous_anchor() {
    let mut state = State {
        task_id: 1,
        selected_run: Some("r-test-run".into()),
        showing_retained: true,
        ..State::default()
    };
    state.retained_page = Some(store::OutputPage {
        offset: 0,
        text: "中文".into(),
        total_bytes: 20,
        unavailable: false,
        truncated: false,
    });
    state.next_page();
    assert!(matches!(
        state.queue.front(),
        Some(StoreJob::Read { offset: 6, .. })
    ));
    assert_eq!(state.page_offsets, [0]);
    state.next_page(); // Identical pending read is not queued twice.
    assert_eq!(state.page_offsets, [0]);
    assert_eq!(state.queue.len(), 1);
}

#[test]
fn global_activity_never_claims_no_work_during_approval_or_owned_execution() {
    let mut state = pending();
    let idle = State::default().activity_label();
    assert_ne!(state.activity_label(), idle);
    state.active = Some(record(runner::RunState::Running));
    let running = state.activity_label();
    assert_ne!(running, idle);
    state.active.as_mut().unwrap().state = runner::RunState::Cancelling;
    assert_ne!(state.activity_label(), running);
    assert_ne!(state.activity_label(), idle);
    state.set_capture_capacity(1024);
    assert!(approval_details(&prepared(), state.capture_capacity).contains("1024 bytes"));
}

#[test]
fn approval_and_evidence_details_name_the_requesting_partner() {
    let mut p = prepared();
    assert!(approval_details(&p, 4096).starts_with("Partner: partner unknown\n"));
    p.partner = Some(store::Attribution {
        partner: 1,
        name: "Bolt".into(),
        form: 2,
    });
    assert!(approval_details(&p, 4096).starts_with("Partner: Bolt (Jolteon)\n"));
    let mut state = State::default();
    assert!(state.prepare_command_for(1, p.partner.clone(), r#"["tool","first"]"#));
    assert_eq!(state.owner, p.partner);
}

#[test]
fn location_selection_uses_only_the_displayed_window_and_keeps_run_ownership() {
    let mut state = State {
        task_id: 1,
        ..State::default()
    };
    let record = record(runner::RunState::Failed);
    state.selected_run = Some(record.prepared.run_id.clone());
    state.history.push(record);
    state.live.append("src/old.rs:2:1\n");
    state.showing_retained = true;
    state.retained_run = state.selected_run.clone();
    state.retained.append("src/current.rs:5:3\n");
    state.command("locations");
    assert_eq!(
        state.locations,
        vec![crate::workspace::locations::Location {
            path: "src/current.rs".into(),
            line: 5,
            column: Some(3),
        }]
    );
    assert!(matches!(state.command("pin-location"),
        Some(Action::PinLocation(location, run)) if location.line == 5 && run == "r-test-run"));
    state.selected_run = Some("different-retained-run".into());
    state.command("locations");
    assert!(matches!(state.command("pin-location"),
        Some(Action::PinLocation(_, run)) if run == "r-test-run"));
    state.selected_run = Some("r-test-run".into());
    state.retained = OutputWindow::default();
    state.command("locations");
    assert!(state.locations.is_empty());
    assert!(state.command("pin-location").is_none());
}

#[test]
fn rerun_preparation_has_fresh_approval_and_origin_is_cleared_by_manual_replacement() {
    let mut state = State::default();
    let owner = store::Attribution {
        partner: 1,
        name: "Bolt".into(),
        form: 2,
    };
    assert!(state.prepare_rerun_for(1, owner.clone(), r#"["tool","check"]"#, "old-run".into()));
    assert_eq!(state.rerun_of.as_deref(), Some("old-run"));
    assert!(!state.affirmative);
    assert!(state.permit.is_none());
    assert!(state.prepare_command_for(1, Some(owner), r#"["tool","different"]"#));
    assert!(state.rerun_of.is_none());
}

#[test]
fn ordinary_rerun_uses_project_cwd_without_requiring_git_freshness() {
    for git in [false, true] {
        let project = "/workspace/nested-project";
        let mut app = crate::app::App::new(project.into(), 120, 36);
        app.task_id = 1;
        app.storage = crate::app::StorageState::Saved;
        app.lead.form = 2;
        app.editing = false;
        if git {
            let mut observation = report(1, true);
            observation.repository.root = project.into();
            observation.repository.git_root = Some("/workspace".into());
            observation.repository.git_dir = Some("/workspace/.git".into());
            app.repository = Some(Arc::new(observation));
        }
        let mut prior = record(runner::RunState::Succeeded);
        prior.prepared.cwd = project.into();
        let run = prior.prepared.run_id.clone();
        app.execution.history.push(prior.clone());
        app.execution.selected_run = Some(run.clone());
        app.execution.task_id = 1;
        assert!(
            app.execution
                .command_choices()
                .iter()
                .find(|choice| choice.command == "rerun")
                .unwrap()
                .reason
                .is_none()
        );
        app.stage_execution_rerun(run).unwrap();
        assert_eq!(
            app.command,
            super::super::execution_relays::command_draft(&prior.prepared.spec)
        );
        assert!(app.pending_command.is_some());
        assert!(app.execution.permit.is_none() && app.execution.active.is_none());
        assert_eq!(app.execution.history[0], prior);
    }
    let mut app = crate::app::App::new("/workspace/nested-project".into(), 120, 36);
    app.task_id = 1;
    app.lead.form = 2;
    let mut prior = record(runner::RunState::Succeeded);
    prior.prepared.cwd = "/workspace".into();
    let run = prior.prepared.run_id.clone();
    app.execution.history.push(prior);
    assert!(app.stage_execution_rerun(run).is_err());
    assert!(app.pending_command.is_none() && app.execution.permit.is_none());
}

#[test]
fn ordinary_source_refresh_updates_staleness_without_settling_unstarted_capture() {
    let mut state = State {
        task_id: 1,
        ..State::default()
    };
    let initial = report(1, true);
    state.repo_observed(Some(&initial));
    let before = state.latest.clone();
    let changed = report(2, true);
    state.repo_observed(Some(&changed));
    assert_ne!(state.latest.fingerprint, before.fingerprint);
    assert_eq!(state.latest, fingerprint(&changed));
    assert!(state.prepare_command(1, r#"["tool","check"]"#));
    state.repo_observed(Some(&initial));
    assert!(
        state.before_ready.is_none(),
        "ordinary refresh cannot settle an unstarted execution observation"
    );
    state.repo_started();
    state.repo_observed(Some(&changed));
    assert_eq!(state.before_ready, Some(fingerprint(&changed)));
}

#[test]
fn irreversible_journal_masking_disables_exact_rerun_instead_of_substituting_arguments() {
    let mut prior = record(runner::RunState::Succeeded);
    prior.prepared.spec = runner::CommandSpec::Direct {
        program: "tool".into(),
        args: vec!["--password".into(), "synthetic-fixture-value".into()],
    };
    assert!(super::super::execution_relays::retained_rerun_spec(&prior).is_ok());
    prior.redact_for_storage();
    let mut state = State {
        task_id: 1,
        selected_run: Some(prior.prepared.run_id.clone()),
        latest: identity("changed"),
        history: vec![prior],
        ..State::default()
    };
    let choices = state.command_choices();
    let reason = choices
        .iter()
        .find(|choice| choice.command == "rerun")
        .unwrap()
        .reason
        .as_deref();
    assert!(reason.is_some_and(|reason| reason.contains("exact argv cannot be recovered")));
    state.history[0].prepared.spec = runner::CommandSpec::Direct {
        program: "tool".into(),
        args: vec!["check".into()],
    };
    assert!(
        state
            .command_choices()
            .iter()
            .find(|choice| choice.command == "rerun")
            .unwrap()
            .reason
            .is_none()
    );
}

fn command_owner(partner: i64) -> store::Attribution {
    store::Attribution {
        partner,
        name: format!("Eevee #{partner}"),
        form: 0,
    }
}

#[test]
fn deferred_command_consent_requires_exact_owner_request_run_and_new_visibility() {
    let mut state = pending();
    state.owner = Some(command_owner(1));
    state
        .prepared
        .as_mut()
        .unwrap()
        .partner
        .clone_from(&state.owner);
    let original = state.prepared.clone().unwrap();
    state.command("execute");
    state.confirmation_visible = true;
    state.key(key(KeyCode::Right));
    state.key(key(KeyCode::Esc));
    assert!(!state.shown && state.confirmation.is_none());
    assert!(state.busy());
    assert_eq!(state.prepared.as_ref(), Some(&original));
    for (task, partner, request, run) in [
        (2, 1, original.request_id.as_str(), original.run_id.as_str()),
        (1, 2, original.request_id.as_str(), original.run_id.as_str()),
        (1, 1, "replacement-request", original.run_id.as_str()),
        (1, 1, original.request_id.as_str(), "replacement-run"),
    ] {
        assert!(!state.focus_approval(task, partner, request, run));
    }
    assert!(state.focus_approval(1, 1, &original.request_id, &original.run_id));
    assert!(!state.affirmative && !state.confirmation_visible);
    state.key(key(KeyCode::Right));
    state.key(key(KeyCode::Enter));
    assert!(
        state.queue.is_empty(),
        "keyboard selection alone is not visible consent"
    );
    state.confirmation_visible = true;
    state.key(key(KeyCode::Enter));
    assert!(matches!(state.queue.front(), Some(StoreJob::Reserve(p, _)) if p == &original));
}

#[test]
fn replacement_owner_and_mutated_review_cannot_take_over_command_consent() {
    let mut state = pending();
    state.owner = Some(command_owner(1));
    state
        .prepared
        .as_mut()
        .unwrap()
        .partner
        .clone_from(&state.owner);
    let original = state.prepared.clone();
    assert!(!state.prepare_command_for(1, Some(command_owner(2)), r#"["other-tool"]"#));
    assert_eq!(state.prepared, original);
    assert_eq!(state.owner, Some(command_owner(1)));
    assert!(!state.request_cancel_for(1, 2));
    assert!(!state.request_cancel_for(2, 1));
    assert_eq!(state.prepared, original);
    for mutation in 0..3 {
        let mut state = pending();
        state.command("execute");
        state.confirmation_visible = true;
        match mutation {
            0 => state.prepared.as_mut().unwrap().run_id.push_str("-changed"),
            1 => state.prepared.as_mut().unwrap().partner = Some(command_owner(2)),
            _ => state.prepared.as_mut().unwrap().spec = parse_command(r#"["changed"]"#).unwrap(),
        }
        state.key(key(KeyCode::Char('y')));
        assert!(
            state.queue.is_empty(),
            "changed review scope cannot reserve"
        );
    }
    assert!(state.request_cancel_for(1, 1));
    assert!(state.prepared.is_none() && state.request_id.is_none());
}

#[test]
fn moving_revision_is_never_labelled_fresh_even_when_all_hashes_match() {
    let mut result = record(runner::RunState::Succeeded);
    result.revision_moving = true;
    let label = evidence_label(&result, &identity("same"));
    assert!(label.contains("Revision moving"));
    assert!(!label.contains("Before/after match"));
    let mut state = pending();
    state.active = Some(record(runner::RunState::Running));
    let mut stored = state.active.clone().unwrap();
    stored.revision_moving = true;
    state.critical(StoreJob::Update(stored.clone()));
    state.take_store_request().unwrap();
    state.store_event(
        1,
        Some("r-test-request"),
        Some("r-test-run"),
        Ok(store::RunResponse::Updated(stored)),
    );
    assert!(state.active.as_ref().unwrap().revision_moving);
}

fn next_store_event(worker: &store::Worker) -> store::Event {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Some(event) = worker.try_event() {
            return event;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "store event timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn settle_run_request(state: &mut State, worker: &store::Worker) -> bool {
    let Some(request) = state.take_store_request() else {
        return false;
    };
    assert!(worker.try_run(&mut Some(request)));
    match next_store_event(worker) {
        store::Event::Run {
            task_id,
            request_id,
            run_id,
            result,
        } => {
            state.store_event(
                task_id,
                request_id.as_deref(),
                run_id.as_deref(),
                result.map(|response| *response),
            );
        }
        other => panic!("expected correlated execution response: {other:?}"),
    }
    true
}

#[cfg(unix)]
#[test]
fn real_reserve_capacity_race_retains_command_then_runs_beside_provider_and_releases_owner() {
    struct Temp(std::path::PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let temp = Temp(std::env::temp_dir().join(format!(
        "eevee-command-cooperation-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    )));
    std::fs::create_dir_all(&temp.0).unwrap();
    let cwd = std::fs::canonicalize(&temp.0).unwrap();
    let project = cwd.to_str().unwrap().to_owned();
    let storage = store::Worker::start(
        cwd.join("storage"),
        project.clone(),
        store::Policy::default(),
    )
    .unwrap();
    assert!(matches!(
        next_store_event(&storage),
        store::Event::Loaded(_)
    ));
    for _ in 0..3 {
        assert!(storage.try_party(&mut Some(store::PartyRequest::Add)));
        assert!(matches!(
            next_store_event(&storage),
            store::Event::Party(Ok(_))
        ));
    }
    let mut task = store::Snapshot::empty(project);
    task.goal = "cooperative command ownership".into();
    task.revision = 1;
    task.command = r#"["/bin/echo","owned original command"]"#.into();
    assert!(storage.try_save(&mut Some(task.clone())));
    task = match next_store_event(&storage) {
        store::Event::Saved(task) => task,
        other => panic!("expected committed task: {other:?}"),
    };
    let claims: Vec<_> = (1..=3)
        .map(|partner| store::WorkClaim {
            task_id: task.id,
            owner: command_owner(partner),
            request_id: format!("provider-owned-{partner}"),
            writing: false,
        })
        .collect();
    for claim in &claims {
        assert!(storage.try_work(&mut Some(store::WorkRequest::Acquire {
            claim: claim.clone(),
            task: Box::new(task.clone())
        })));
        assert!(matches!(
            next_store_event(&storage),
            store::Event::Work { result: Ok(_), .. }
        ));
    }
    let runner = runner::Worker::start().unwrap();
    let mut state = State::default();
    assert!(state.prepare_command_for(task.id, Some(command_owner(4)), &task.command));
    state.before_ready = Some(identity("same"));
    state.repo_request = false;
    state.drive(&runner, task.id, &cwd);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while state.prepared.is_none() {
        if let Some(event) = runner.try_event() {
            state.runner_event(event);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "command preparation timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let original = state.prepared.clone().unwrap();
    state.shown = true;
    state.command("execute");
    state.confirmation_visible = true;
    state.key(key(KeyCode::Char('y')));
    assert!(settle_run_request(&mut state, &storage));
    assert!(
        state
            .error
            .as_ref()
            .is_some_and(|error| error.layer == "Storage / reservation")
    );
    assert_eq!(state.prepared.as_ref(), Some(&original));
    assert!(state.permit.is_none() && state.active.is_none());
    for claim in &claims[1..] {
        assert!(storage.try_work(&mut Some(store::WorkRequest::Release(claim.clone()))));
        assert!(matches!(
            next_store_event(&storage),
            store::Event::Work { result: Ok(_), .. }
        ));
    }
    // Another lead's saved command/form/draft changes cannot replace this approved input.
    task.lead_partner = 2;
    task.command = r#"["/bin/echo","future draft only"]"#.into();
    task.form = 1;
    task.revision += 1;
    assert!(storage.try_save(&mut Some(task)));
    task = match next_store_event(&storage) {
        store::Event::Saved(task) => task,
        other => panic!("routing save must remain available during provider work: {other:?}"),
    };
    assert!(state.focus_approval(task.id, 4, &original.request_id, &original.run_id));
    state.confirmation_visible = true;
    state.key(key(KeyCode::Char('y')));
    assert!(storage.try_run(&mut state.take_store_request()));
    let reserved = next_store_event(&storage);
    let writer = store::WorkClaim {
        task_id: task.id,
        owner: command_owner(3),
        request_id: "late-writer".into(),
        writing: true,
    };
    assert!(storage.try_work(&mut Some(store::WorkRequest::Acquire {
        claim: writer.clone(),
        task: Box::new(task.clone())
    })));
    assert!(matches!(
        next_store_event(&storage),
        store::Event::Work { result: Ok(_), .. }
    ));
    state.mark_revision_moving(task.id);
    match reserved {
        store::Event::Run {
            task_id,
            request_id,
            run_id,
            result,
        } => {
            state.store_event(
                task_id,
                request_id.as_deref(),
                run_id.as_deref(),
                result.map(|response| *response),
            );
        }
        other => panic!("expected reserved command before late writer: {other:?}"),
    }
    assert!(
        state.permit.as_ref().unwrap().revision_moving,
        "writer acquired before reservation ack must taint the permit"
    );
    state.drive(&runner, task.id, &cwd);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while state.busy() {
        while let Some(event) = runner.try_event() {
            state.runner_event(event);
        }
        if state.final_record.is_some() {
            state.repo_started();
            state.repo_observed(None);
        }
        while settle_run_request(&mut state, &storage) {}
        assert!(
            std::time::Instant::now() < deadline,
            "owned command settlement timed out: {}",
            state.details()
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let completed = state
        .history
        .iter()
        .find(|record| record.prepared.run_id == original.run_id)
        .unwrap();
    assert_eq!(completed.prepared, original);
    assert_eq!(completed.state, runner::RunState::Succeeded);
    assert_eq!(completed.prepared.partner, Some(command_owner(4)));
    assert!(completed.revision_moving);
    assert!(
        state.moving_request.is_none(),
        "terminal settlement clears only the request's pending taint"
    );
    assert!(storage.try_work(&mut Some(store::WorkRequest::Release(writer))));
    assert!(matches!(
        next_store_event(&storage),
        store::Event::Work { result: Ok(_), .. }
    ));
    assert!(storage.try_work(&mut Some(store::WorkRequest::Acquire {
        claim: claims[2].clone(),
        task: Box::new(task)
    })));
    match next_store_event(&storage) {
        store::Event::Work {
            result: Ok(active), ..
        } => {
            assert_eq!(active.len(), 2);
            assert!(active.iter().all(|claim| claim.owner.partner != 4));
            assert!(
                active.contains(&claims[0]),
                "provider ownership survives command settlement"
            );
        }
        other => panic!("terminal transaction must release command claim: {other:?}"),
    }
}

#[test]
fn command_relay_admission_counts_provider_owner_without_using_global_busy() {
    let mut task = store::Snapshot::empty("/workspace".into());
    task.id = 7;
    task.goal = "shared task".into();
    task.revision = 1;
    task.command = r#"["tool","retained draft"]"#.into();
    let mut app = super::super::App::new("/workspace".into(), 120, 36);
    app.store_event(store::Event::Loaded(store::Project {
        tasks: vec![task.clone()],
        partners: (1..=4)
            .map(|id| store::Partner {
                id,
                ..store::Partner::default()
            })
            .collect(),
        active_id: Some(task.id),
        usage_bytes: 0,
    }));
    app.planning.command("new", true, &task, &[], None);
    assert!(app.planning.busy());
    assert_eq!(app.planning.owner().map(|owner| owner.partner), Some(1));
    let draft = app.command.clone();
    app.prepare_command_relay(&draft, command_owner(1));
    assert!(
        app.execution.pending_spec.is_none(),
        "provider owner cannot acquire local work too"
    );
    assert_eq!(app.command, draft);
    app.prepare_command_relay(&draft, command_owner(2));
    let other = app.execution_state(2).unwrap();
    assert_eq!(other.owner, Some(command_owner(2)));
    assert!(
        other.pending_spec.is_some(),
        "another partner's provider does not block command admission"
    );
    assert!(
        !other.shown,
        "a background save acknowledgment cannot steal the lead's focus"
    );
    assert_eq!(app.lead().id, 1);
    assert_eq!(app.command, draft);
    assert!(
        app.planning.busy(),
        "local admission never cancels another partner"
    );
}

#[test]
fn independent_partner_states_share_evidence_without_transferring_work_or_consent() {
    let mut owner = pending();
    owner.owner = Some(command_owner(1));
    owner
        .prepared
        .as_mut()
        .unwrap()
        .partner
        .clone_from(&owner.owner);
    owner.latest = identity("shared-source");
    owner.set_capture_capacity(8192);
    let retained = record(runner::RunState::Succeeded);
    owner.history.push(retained.clone());
    owner.command("execute");
    owner.confirmation_visible = true;
    let mut other = owner.new_partner_state(1);
    assert_eq!(other.latest, owner.latest);
    assert_eq!(other.capture_capacity, 8192);
    assert_eq!(other.history, owner.history);
    assert!(other.owner.is_none() && !other.busy() && !other.needs_runner());
    assert!(other.prepared.is_none() && other.confirmation.is_none());
    assert!(!other.shown && other.live.lines.is_empty());
    assert!(other.prepare_command_for(1, Some(command_owner(2)), r#"["other-tool"]"#));
    assert_eq!(owner.owner, Some(command_owner(1)));
    assert!(owner.prepared.is_some() && owner.confirmation_visible);
    assert_eq!(other.owner, Some(command_owner(2)));
    assert!(other.request_cancel_for(1, 2));
    assert!(
        owner.prepared.is_some(),
        "another partner's cancellation is independent"
    );
    let mut updated = retained.clone();
    updated.revision_moving = true;
    let selection = owner.selected_run.clone();
    owner.observe_record(&updated);
    owner.observe_record(&updated);
    assert_eq!(owner.history.len(), 1);
    assert!(owner.history[0].revision_moving);
    assert_eq!(owner.selected_run, selection);
    assert_eq!(owner.owner, Some(command_owner(1)));
    assert!(owner.prepared.is_some());
    let foreign = owner.new_partner_state(2);
    assert!(foreign.history.is_empty());
    assert_eq!(foreign.latest, runner::InputSnapshot::default());
}

#[test]
fn parked_storage_and_late_writer_helpers_preserve_task_owned_settlement() {
    let mut state = State {
        task_id: 1,
        ..State::default()
    };
    state.optional(StoreJob::Load(1));
    assert!(state.storage_pending() && !state.needs_runner() && !state.busy());
    state.take_store_request().unwrap();
    assert!(state.storage_pending() && !state.needs_runner());
    state.store_event(1, None, None, Ok(store::RunResponse::Runs(Vec::new())));
    assert!(!state.storage_pending());
    let mut running = record(runner::RunState::Running);
    running.prepared.partner = Some(command_owner(1));
    let mut finished = running.clone();
    finished.state = runner::RunState::Succeeded;
    let mut historic = finished.clone();
    historic.prepared.run_id = "older-stable-run".into();
    state.active = Some(running.clone());
    state.final_record = Some(finished.clone());
    state.history = vec![finished, historic];
    state.mark_revision_moving(2);
    assert!(!state.active.as_ref().unwrap().revision_moving);
    state.mark_revision_moving(1);
    assert!(state.active.as_ref().unwrap().revision_moving);
    assert!(state.final_record.as_ref().unwrap().revision_moving);
    assert!(state.history[0].revision_moving);
    assert!(!state.history[1].revision_moving);
    state.set_task(2);
    assert_eq!(
        state.task_id, 1,
        "task changes cannot detach owned settlement"
    );
    state.active.as_mut().unwrap().state = runner::RunState::OutcomeUnknown;
    assert!(state.outcome_unknown());
    state.failed = Some(StoreJob::Update(running));
    assert!(state.journal_failed() && state.storage_pending());
    state.ready = Some(Action::SelectForm(2));
    assert_eq!(state.take_ready_action(), Some(Action::SelectForm(2)));
    assert!(state.take_ready_action().is_none());
}

#[test]
fn lazy_runner_failure_revokes_launch_without_automatic_retry_or_peer_mutation() {
    let mut state = State::default();
    assert!(state.prepare_command_for(1, Some(command_owner(1)), r#"["tool","saved draft"]"#));
    let mut peer = state.new_partner_state(1);
    assert!(peer.prepare_command_for(1, Some(command_owner(2)), r#"["tool","peer draft"]"#));
    let peer_spec = peer.pending_spec.clone();
    state.runner_unavailable("thread creation refused".into());
    assert!(!state.needs_runner());
    assert!(!state.busy());
    assert!(state.request_id.is_none() && state.prepared.is_none());
    assert!(!state.invalidation);
    assert_eq!(state.owner, Some(command_owner(1)));
    assert_eq!(state.error.as_ref().unwrap().layer, "Runtime");
    assert!(
        state
            .error
            .as_ref()
            .unwrap()
            .message
            .contains("no process launched")
    );
    assert_eq!(peer.pending_spec, peer_spec);
    assert_eq!(peer.owner, Some(command_owner(2)));
    assert!(peer.needs_runner());
}

#[test]
fn writer_taint_survives_older_runner_capture_and_terminal_snapshots() {
    let mut running = record(runner::RunState::Running);
    running.prepared.partner = Some(command_owner(1));
    let mut state = State {
        task_id: 1,
        owner: Some(command_owner(1)),
        request_id: Some(running.prepared.request_id.clone()),
        active: Some(running.clone()),
        ..State::default()
    };
    state.mark_revision_moving(1);
    state.runner_event(runner::Event::CaptureChanged(running.clone()));
    assert!(state.active.as_ref().unwrap().revision_moving);
    running.state = runner::RunState::Succeeded;
    state.runner_event(runner::Event::Finished(running));
    assert!(state.active.as_ref().unwrap().revision_moving);
    assert!(state.final_record.as_ref().unwrap().revision_moving);
}

#[test]
fn idle_invalidation_never_allocates_runner_and_pending_taint_is_request_scoped() {
    let mut idle = State::default();
    idle.invalidate();
    assert!(
        !idle.needs_runner(),
        "opening a local command editor cannot start a runner"
    );
    let mut state = pending();
    state.reserving = true;
    state.mark_revision_moving(2);
    assert!(state.moving_request.is_none());
    state.mark_revision_moving(1);
    assert_eq!(state.moving_request.as_deref(), Some("r-test-request"));
    let running = record(runner::RunState::Running);
    state.runner_event(runner::Event::Started(running));
    assert!(
        state.active.as_ref().unwrap().revision_moving,
        "pre-Started overlap survives the runner snapshot"
    );
    let mut idle = State {
        task_id: 1,
        moving_request: Some("old-request".into()),
        ..State::default()
    };
    assert!(idle.prepare_command_for(1, None, r#"["tool","new-request"]"#));
    assert!(
        idle.moving_request.is_none(),
        "a new explicit preparation never inherits old overlap"
    );
}

#[test]
fn rejected_move_preparation_does_not_attribute_later_manual_command() {
    let p = prepared();
    let mut state = State {
        task_id: 1,
        request_id: Some(p.request_id.clone()),
        move_origin: Some((7, 1)),
        move_cwd: Some(".".into()),
        ..State::default()
    };
    state.runner_event(runner::Event::Rejected {
        request_id: p.request_id,
        task_id: 1,
        message: "Executable disappeared".into(),
    });
    let owner = store::Attribution::of(&store::Partner::default());
    assert!(state.prepare_command_for(1, Some(owner), r#"["tool","中文 space;literal"]"#));
    assert!(state.move_origin.is_none());
    assert!(state.prepared.is_none());
    assert!(state.confirmation.is_none());
}
