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
            git_dir: "/project/.git".into(),
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
    state.command("run");
    assert!(!state.affirmative);
    state.key(key(KeyCode::Enter));
    assert!(state.queue.is_empty());
    assert!(state.prepared.is_some());
    state.command("run");
    state.key(key(KeyCode::Right));
    state.key(key(KeyCode::Enter));
    assert!(matches!(state.queue.front(), Some(StoreJob::Reserve(..))));
    assert_eq!(state.queue.len(), 1);
    state.key(key(KeyCode::Enter));
    state.command("run");
    assert_eq!(state.queue.len(), 1);
}

#[test]
fn confirmation_shortcuts_require_visible_scope_press_and_matching_request() {
    let mut state = pending();
    state.command("run");
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
    state.command("run");
    state.confirmation_visible = true;
    state.key(key(KeyCode::Char('y')));
    assert!(matches!(state.queue.front(), Some(StoreJob::Reserve(..))));
    state.key(key(KeyCode::Char('y')));
    assert_eq!(state.queue.len(), 1);

    for change_task in [false, true] {
        let mut state = pending();
        state.command("run");
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
    state.command("run");
    state.key(key(KeyCode::Esc));
    assert!(state.request_id.is_some());
    state.command("run");
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
