use super::*;
use ratatui::{Terminal, backend::TestBackend};

fn claim() -> store::WorkClaim {
    store::WorkClaim {
        task_id: 7,
        owner: store::Attribution::of(&store::Partner::default()),
        request_id: "actual-ui-request".into(),
        writing: false,
    }
}
fn preview() -> providers::Preview {
    providers::Preview {
        transport: None,
        arrival: 1,
        generation: 11,
        task_id: 7,
        thread_id: None,
        destination: None,
        requires_connection_review: false,
        trust: Vec::new(),
        comparison_scope: None,
        payload: r#"{"purpose":"Plan","request":"keep exact 東京 draft"}"#.into(),
        blockers: Vec::new(),
    }
}
fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn draw(state: &mut State, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            state.render(frame, area)
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let mut screen = String::new();
    for y in 0..height {
        for x in 0..width {
            screen.push_str(buffer[(x, y)].symbol());
        }
        screen.push('\n');
    }
    screen
}
fn saved() -> State {
    let outgoing = Outgoing::frozen(
        &preview().payload,
        &claim(),
        11,
        providers::Agent::Codex,
        providers::Operation::Plan,
        1234,
    )
    .unwrap();
    let mut state = State {
        task: 7,
        partner: 1,
        dirty: true,
        ..State::default()
    };
    state.records.push(View {
        record: outgoing.record,
        source_available: true,
    });
    state
}

#[test]
fn preview_and_wrong_dispatch_never_create_durable_work_but_actual_submission_does() {
    let mut state = State::default();
    state
        .stage(
            &preview(),
            &claim(),
            providers::Agent::Codex,
            providers::Operation::Plan,
            Metadata::default(),
        )
        .unwrap();
    assert!(!state.pending());
    state.dispatched(12, 7, "actual-ui-request");
    assert!(!state.pending());
    state.dispatched(11, 8, "actual-ui-request");
    assert!(!state.pending());
    state.dispatched(11, 7, "wrong-request");
    assert!(!state.pending());
    state.dispatched(11, 7, "actual-ui-request");
    assert!(state.pending());
    state.dispatched(11, 7, "actual-ui-request");
    assert_eq!(state.queue.len(), 1);
    assert!(
        matches!(&state.queue[0].operation,Operation::Put(o) if o.record.request_id==claim().request_id && o.source.as_deref()==Some(preview().payload.as_str()))
    );
}

#[test]
fn purge_is_neutral_and_requires_visible_review_then_exact_owner_operation() {
    for (width, height) in [(60, 18), (120, 36)] {
        let mut state = saved();
        draw(&mut state, width, height);
        state.key(press(KeyCode::Char('p')));
        state.key(press(KeyCode::Right));
        state.key(press(KeyCode::Enter));
        assert!(state.queue.is_empty(), "unrendered review must not purge");
        let screen = draw(&mut state, width, height);
        assert!(screen.contains("[x]"));
        assert!(state.rendered && !state.affirmative);
        state.key(press(KeyCode::Enter));
        assert!(state.queue.is_empty(), "neutral Enter stays");
        state.key(press(KeyCode::Char('p')));
        draw(&mut state, width, height);
        state.key(press(KeyCode::Right));
        state.key(press(KeyCode::Esc));
        assert!(state.queue.is_empty(), "cancel has no effects");
        state.key(press(KeyCode::Char('p')));
        draw(&mut state, width, height);
        state.key(press(KeyCode::Right));
        state.key(press(KeyCode::Enter));
        assert_eq!(
            state.queue.front(),
            Some(&Request {
                task_id: 7,
                partner: 1,
                operation: Operation::PurgeSources {
                    request_id: Some("actual-ui-request".into())
                }
            })
        );
    }
}

#[test]
fn source_purge_removes_inspected_copy_and_keeps_count_identity_visible() {
    let mut state = saved();
    let original = state.records[0].record.clone();
    state.source = Some((
        "actual-ui-request".into(),
        "SOURCE_COPY_MUST_DISAPPEAR".into(),
    ));
    draw(&mut state, 60, 18);
    state.source_purged(7);
    assert!(state.source.is_none());
    assert!(!state.records[0].source_available);
    assert_eq!(state.records[0].record, original);
    let screen = draw(&mut state, 60, 18);
    assert!(!screen.contains("SOURCE_COPY_MUST_DISAPPEAR"));
    assert!(screen.contains("actual-ui-request"));
    assert!(screen.contains("UNAVAILABLE"));
}

#[test]
fn failed_write_keeps_exact_bytes_for_storage_only_retry_and_blocks_release() {
    let mut state = State::default();
    state
        .stage(
            &preview(),
            &claim(),
            providers::Agent::Codex,
            providers::Operation::Plan,
            Metadata::default(),
        )
        .unwrap();
    state.dispatched(11, 7, "actual-ui-request");
    let request = state.queue.pop_front().unwrap();
    state.inflight = Some(request_identity(&request));
    state.event(request.clone(), Err("fixture disk pressure".into()));
    assert!(state.pending());
    state.retry();
    assert_eq!(state.queue.front(), Some(&request));
    assert!(state.failed.is_empty());
}

#[test]
fn committed_artifact_purge_invalidates_active_and_parked_receipt_views_before_restore() {
    use crate::{
        app::{App, Overlay, planning},
        workspace::artifacts::{
            Handle, Operation as ArtifactOperation, Request as ArtifactRequest,
            Response as ArtifactResponse,
        },
    };
    for (width, height) in [(60, 18), (120, 36)] {
        let mut app = App::new("/workspace".into(), width, height);
        let partner_two = store::Partner {
            id: 2,
            ..store::Partner::default()
        };
        let partner_three = store::Partner {
            id: 3,
            ..store::Partner::default()
        };
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.goal = "Inspect retained artifact source receipts".into();
        task.revision = 1;
        let mut other_task = task.clone();
        other_task.id = 8;
        app.store_event(store::Event::Loaded(store::Project {
            active_id: Some(task.id),
            tasks: vec![task, other_task],
            partners: vec![
                store::Partner::default(),
                partner_two.clone(),
                partner_three.clone(),
            ],
            usage_bytes: 0,
        }));

        // A previously inspected exact public envelope is cached both as
        // source text and as wrapped display rows, not merely a count view.
        let source = preview().payload;
        app.planning.receipts = saved();
        app.planning.receipts.source = Some(("actual-ui-request".into(), source.clone()));
        draw(&mut app.planning.receipts, width, height);
        assert_eq!(app.planning.receipts.display, source);
        let active_identity = app.planning.receipts.records[0].record.clone();

        let mut parked = app
            .planning
            .new_partner(store::Attribution::of(&partner_two), 7);
        parked.receipts = saved();
        parked.receipts.partner = 2;
        parked.receipts.records[0].record.owner = store::Attribution::of(&partner_two);
        parked.receipts.records[0].record.request_id = "parked-artifact-request".into();
        parked.receipts.source = Some(("parked-artifact-request".into(), source.clone()));
        draw(&mut parked.receipts, width, height);
        let parked_identity = parked.receipts.records[0].record.clone();
        app.cooperation.parked.insert(2, parked);

        let mut unrelated = planning::State::new();
        unrelated.receipts = saved();
        unrelated.receipts.task = 8;
        unrelated.receipts.partner = 3;
        unrelated.receipts.records[0].record.task_id = 8;
        unrelated.receipts.records[0].record.owner = store::Attribution::of(&partner_three);
        unrelated.receipts.source = Some(("other-task-request".into(), source.clone()));
        draw(&mut unrelated.receipts, width, height);
        app.cooperation.parked.insert(3, unrelated);

        app.open_overlay(Overlay::Receipts);
        app.open_overlay(Overlay::Artifacts);
        let handle = Handle {
            id: "a".repeat(64),
            revision: "b".repeat(64),
        };
        let request = ArtifactRequest {
            request_id: 19,
            task_id: 7,
            operation: ArtifactOperation::Purge {
                handle: handle.clone(),
            },
        };
        app.store_event(store::Event::Artifact {
            request: request.clone(),
            result: Err("purge refused before commit".into()),
        });
        assert!(app.planning.receipts.source.is_some());
        assert!(app.cooperation.parked[&2].receipts.source.is_some());

        // A committed unavailable record stays purged even when physical
        // artifact cleanup reports a warning.
        app.store_event(store::Event::Artifact {
            request,
            result: Ok(ArtifactResponse::Purged {
                handle,
                warning: Some("committed purge; physical cleanup unavailable".into()),
            }),
        });
        for receipts in [&app.planning.receipts, &app.cooperation.parked[&2].receipts] {
            assert!(receipts.source.is_none());
            assert!(receipts.display.is_empty());
            assert!(receipts.lines.is_empty());
            assert!(!receipts.records[0].source_available);
        }
        assert_eq!(app.planning.receipts.records[0].record, active_identity);
        assert_eq!(
            app.cooperation.parked[&2].receipts.records[0].record,
            parked_identity
        );
        assert!(app.cooperation.parked[&3].receipts.source.is_some());
        assert_eq!(app.cooperation.parked[&3].receipts.display, source);
        assert!(app.cooperation.parked[&3].receipts.records[0].source_available);

        app.close_overlay();
        assert_eq!(app.overlay, Some(Overlay::Receipts));
        let restored = draw(&mut app.planning.receipts, width, height);
        assert!(!restored.contains("keep exact"));
        assert!(app.planning.receipts.source.is_none());
        app.front_planning(&partner_two);
        app.lead = partner_two;
        let restored_parked = draw(&mut app.planning.receipts, width, height);
        assert!(!restored_parked.contains("keep exact"));
        assert!(app.planning.receipts.source.is_none());
        assert_eq!(app.planning.receipts.records[0].record, parked_identity);
    }
}

const QUOTA_ROLLBACK: &str = "Local storage quota exceeded (live SQLite + retained output + committed reservations); nothing was evicted and no command was started";

fn failed_receipt(error: &str) -> State {
    let mut state = State {
        task: 7,
        partner: 1,
        ..State::default()
    };
    state
        .stage(
            &preview(),
            &claim(),
            providers::Agent::Codex,
            providers::Operation::Plan,
            Metadata::default(),
        )
        .unwrap();
    state.dispatched(11, 7, "actual-ui-request");
    let request = state.queue.pop_front().unwrap();
    state.inflight = Some(request_identity(&request));
    state.event(request, Err(error.into()));
    state
}

fn read_discard_review(state: &mut State, width: u16, height: u16) {
    draw(state, width, height);
    for _ in 0..32 {
        if state.rendered {
            return;
        }
        state.key(press(KeyCode::PageDown));
        draw(state, width, height);
    }
    panic!("bounded receipt recovery review never exposed all scope rows");
}

#[test]
fn receipt_discard_refuses_running_wrong_owner_generation_and_ambiguous_commits() {
    let mut state = failed_receipt(QUOTA_ROLLBACK);
    state.key(press(KeyCode::Char('d')));
    assert!(state.discard.is_none(), "no definite terminal observation");
    let mut wrong = claim();
    wrong.owner.partner = 2;
    state.finished(Some(&(wrong, 11)));
    state.key(press(KeyCode::Char('d')));
    assert!(state.discard.is_none(), "wrong terminal owner");
    state.finished(Some(&(claim(), 12)));
    state.key(press(KeyCode::Char('d')));
    assert!(state.discard.is_none(), "wrong terminal generation");
    state.finished(Some(&(claim(), 11)));
    state.key(press(KeyCode::Char('d')));
    assert!(state.discard.is_some());
    state.finished(None);
    read_discard_review(&mut state, 60, 18);
    state.key(press(KeyCode::Right));
    state.key(press(KeyCode::Enter));
    assert!(state.pending(), "lost terminal proof must not discard");
    let mut ambiguous =
        failed_receipt("Outgoing receipt commit failed; persistence is unconfirmed");
    ambiguous.finished(Some(&(claim(), 11)));
    ambiguous.key(press(KeyCode::Char('d')));
    assert!(
        ambiguous.discard.is_none(),
        "ambiguous COMMIT is not proof of rollback"
    );
    assert!(ambiguous.pending());
}

#[test]
fn reviewed_quota_gap_recovers_only_unsaved_exact_attempt_and_allows_work_settlement() {
    for (width, height) in [(60, 18), (120, 36)] {
        let mut state = failed_receipt(QUOTA_ROLLBACK);
        state.finished(Some(&(claim(), 11)));
        state.key(press(KeyCode::Char('d')));
        state.key(press(KeyCode::Right));
        state.key(press(KeyCode::Enter));
        assert!(state.pending(), "unrendered recovery cannot discard");
        read_discard_review(&mut state, width, height);
        state.key(press(KeyCode::Enter));
        assert!(state.pending(), "neutral Enter keeps failed bytes");
        state.key(press(KeyCode::Char('d')));
        read_discard_review(&mut state, width, height);
        state.key(press(KeyCode::Right));
        state.key(press(KeyCode::Esc));
        assert!(state.pending(), "cancel keeps failed bytes");
        state.key(press(KeyCode::Char('d')));
        read_discard_review(&mut state, width, height);
        state.key(press(KeyCode::Right));
        state.rollback.insert(
            "actual-ui-request".into(),
            format!("{QUOTA_ROLLBACK}; changed sample"),
        );
        state.key(press(KeyCode::Enter));
        assert!(
            state.pending(),
            "changed exact error invalidates the review"
        );
        state.key(press(KeyCode::Char('d')));
        read_discard_review(&mut state, width, height);
        state.key(press(KeyCode::Right));
        state.key(press(KeyCode::Enter));
        assert!(!state.pending());
        assert!(state.failed.is_empty());
        assert!(state.records.is_empty(), "never fabricate a saved receipt");
        assert!(state.source.is_none());
        let gap = state.gap.as_ref().unwrap();
        assert_eq!(gap.request_id, "actual-ui-request");
        assert_eq!(gap.bytes, preview().payload.len());
        let body = state.body();
        assert!(body.contains(Message::ReceiptsGapNotice.text()));
        let mut planning = super::super::State {
            work: Some(claim()),
            work_acquired: true,
            receipts: state,
            ..super::super::State::new()
        };
        planning.settle_work();
        assert_eq!(
            planning.work_pending,
            Some(store::WorkRequest::Release(claim()))
        );
    }
}

#[test]
fn receipt_app_overlay_round_trip_preserves_committed_middle_caret_and_full_draft() {
    use crate::app::{App, Overlay};
    use crossterm::event::Event;
    for (width, height) in [(60, 18), (120, 36)] {
        let mut app = App::new("/workspace".into(), width, height);
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.goal = "Inspect receipts without changing the future request".into();
        task.revision = 1;
        app.store_event(store::Event::Loaded(store::Project {
            active_id: Some(7),
            tasks: vec![task],
            partners: vec![store::Partner::default()],
            usage_bytes: 0,
        }));
        app.open_editor(store::EditorField::Request);
        let draft = "Future unrelated draft 東京 exact caret marker";
        app.update(Event::Paste(draft.into()));
        for _ in 0..5 {
            app.update(Event::Key(press(KeyCode::Left)));
        }
        let expected_cursor = draft.len() - 5;
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        )));
        let committed = app.snapshot("/workspace");
        app.save_submitted();
        app.store_event(store::Event::Saved(committed.clone()));
        assert_eq!(
            committed.partner_drafts[&1]
                .drafts
                .iter()
                .find(|draft| draft.field == store::EditorField::Request)
                .unwrap()
                .cursor,
            expected_cursor
        );
        assert!(app.context_tool_command("receipts"));
        assert_eq!(app.overlay, Some(Overlay::Receipts));
        assert!(app.context_tool_key(press(KeyCode::Esc)));
        assert!(app.composer_is_request());
        assert_eq!((app.input.as_str(), app.cursor), (draft, expected_cursor));
        assert_eq!(
            app.snapshot("/workspace").partner_drafts,
            committed.partner_drafts
        );
        app.update(Event::Paste("Z".into()));
        let expected = format!(
            "{}Z{}",
            &draft[..expected_cursor],
            &draft[expected_cursor..]
        );
        assert_eq!(app.input, expected);
        assert_eq!(app.cursor, expected_cursor + 1);
    }
}

#[test]
fn automatic_effort_and_adopted_step_receipt_freeze_without_changing_payload_bytes() {
    use providers::effort::{Application, Bucket, Reason, Resolution, Source};
    let mut state = State::default();
    let wire_selection = providers::ModelSelection {
        model: Some("fixture-model".into()),
        effort: Some("high".into()),
    };
    let resolution = Resolution {
        bucket: Some(Bucket::Hard),
        resolved: Some("high".into()),
        reason: Reason::OpenCause,
        source: Source::AdoptedActiveStep,
        application: Application::RequestedNotAcknowledged,
    };
    let preview = preview();
    state
        .stage(
            &preview,
            &claim(),
            providers::Agent::Codex,
            providers::Operation::Plan,
            Metadata {
                selection: Some(&wire_selection),
                effort: Some(&resolution),
                progress: Some("adopted revision 4 / investigate stale cache"),
            },
        )
        .unwrap();
    assert!(
        !state.pending(),
        "preparation alone must never persist a dispatched receipt"
    );
    state.dispatched(11, 7, "actual-ui-request");
    let Request {
        operation: Operation::Put(outgoing),
        ..
    } = state.queue.pop_front().unwrap()
    else {
        panic!("missing actual receipt");
    };
    assert_eq!(outgoing.source.as_deref(), Some(preview.payload.as_str()));
    assert_eq!(outgoing.record.effort.as_ref(), Some(&resolution));
    assert_eq!(
        outgoing
            .record
            .selection
            .as_ref()
            .unwrap()
            .effort
            .as_deref(),
        Some("auto")
    );
    assert_eq!(
        outgoing.record.progress.as_deref(),
        Some("adopted revision 4 / investigate stale cache")
    );
    assert!(outgoing.record.validate().is_ok());
    let serialized = serde_json::to_vec(&outgoing.record).unwrap();
    assert_eq!(
        serde_json::from_slice::<crate::workspace::receipt::Durable>(&serialized).unwrap(),
        outgoing.record
    );
}
