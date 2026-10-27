use super::*;

fn app() -> App {
    let mut app = App::new("/project".into(), 120, 36);
    app.task_id = 7;
    app.storage = StorageState::Saved;
    app.repo_loading = false;
    app
}
fn key(app: &mut App, code: KeyCode) {
    app.parked_draft_key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn park(app: &mut App, text: &str) {
    app.input = text.into();
    app.cursor = text.len();
    assert!(app.parked_draft_command("park-draft"));
    let snapshot = app.snapshot("/project");
    assert_eq!(app.input, text, "no clear before durable acknowledgment");
    app.store_event(store::Event::Saved(snapshot));
    assert!(app.input.is_empty());
    assert!(app.pending_request.is_none());
}
#[test]
fn two_parked_drafts_survive_restore_and_owner_switch_without_sends() {
    let mut app = app();
    park(&mut app, "next 한글 request\r\n");
    park(&mut app, "second separate follow-up\n");
    let snapshot = app.snapshot("/project");
    let serialized = serde_json::to_string(&snapshot.workflows).unwrap();
    let mut restored = super::tests::app();
    restored.workflows = serde_json::from_str(&serialized).unwrap();
    assert_eq!(restored.parked_entries().count(), 2);
    restored.lead.id = 2;
    assert_eq!(restored.parked_entries().count(), 0);
    restored.lead.id = 1;
    assert_eq!(restored.parked_entries().count(), 2);
    assert!(!restored.planning.busy() && restored.planning.pending.is_none());
    assert!(restored.pending_request.is_none() && restored.pending_command.is_none());
}
#[test]
fn exact_ack_only_clears_matching_revision_owner_and_caret() {
    for stale in 0..4 {
        let mut app = app();
        app.input = "original 한글".into();
        app.cursor = 8;
        app.parked_draft_command("park-draft");
        let snapshot = app.snapshot("/project");
        match stale {
            0 => {
                app.input.push('!');
                app.revision += 1;
            }
            1 => app.cursor = 0,
            2 => app.lead.id = 2,
            _ => app.revision += 1,
        }
        let before = (app.input.clone(), app.cursor);
        app.store_event(store::Event::Saved(snapshot));
        assert_eq!((app.input.clone(), app.cursor), before);
        assert!(app.pending_request.is_none());
    }
}
#[test]
fn storage_failure_retains_recoverable_original_and_retry_does_not_send() {
    let mut app = app();
    app.input = "recoverable candidate".into();
    app.cursor = 5;
    app.parked_draft_command("park-draft");
    let snapshot = app.snapshot("/project");
    app.store_event(store::Event::Failed {
        revision: Some(app.revision),
        message: "disk full".into(),
    });
    assert_eq!(
        (app.input.as_str(), app.cursor),
        ("recoverable candidate", 5)
    );
    assert!(app.parked_drafts.pending.is_some());
    assert_eq!(app.parked_entries().count(), 1);
    app.store_event(store::Event::Saved(snapshot));
    assert!(app.input.is_empty());
    assert!(app.pending_request.is_none() && !app.planning.busy());
}
#[test]
fn replacement_neutral_cancel_stale_and_small_review_preserve_both_texts_and_caret() {
    for width in [120, 60] {
        let mut app = app();
        app.width = width;
        park(&mut app, "parked first\r\n한글\n");
        app.input = "current exact text".into();
        app.cursor = 3;
        app.parked_draft_command("drafts");
        key(&mut app, KeyCode::Char('r'));
        key(&mut app, KeyCode::Enter);
        assert_eq!((app.input.as_str(), app.cursor), ("current exact text", 3));
        key(&mut app, KeyCode::Char('r'));
        key(&mut app, KeyCode::Esc);
        assert_eq!((app.input.as_str(), app.cursor), ("current exact text", 3));
        assert_eq!(app.parked_entries().count(), 1);
        key(&mut app, KeyCode::Char('r'));
        app.parked_drafts.complete.set(true);
        key(&mut app, KeyCode::Right);
        app.revision += 1;
        key(&mut app, KeyCode::Enter);
        assert_eq!((app.input.as_str(), app.cursor), ("current exact text", 3));
        key(&mut app, KeyCode::Char('r'));
        app.parked_drafts.complete.set(true);
        key(&mut app, KeyCode::Right);
        app.height = 10;
        key(&mut app, KeyCode::Enter);
        assert_eq!((app.input.as_str(), app.cursor), ("current exact text", 3));
        app.height = 36;
        key(&mut app, KeyCode::Char('r'));
        app.parked_drafts.complete.set(true);
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.input, "parked first\r\n한글\n");
        assert_eq!(
            app.parked_entries().count(),
            1,
            "restore copies; explicit discard is separate"
        );
        assert!(
            app.planning.pending.is_none() && !app.planning.busy() && app.pending_request.is_none()
        );
    }
}
#[test]
fn only_ordinary_explicit_send_acquires_current_request_intent() {
    let mut app = app();
    park(&mut app, "Explain the current project");
    app.parked_draft_command("drafts");
    key(&mut app, KeyCode::Char('r'));
    assert_eq!(app.input, "Explain the current project");
    assert!(app.pending_request.is_none());
    app.update(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(
        app.pending_request.is_some(),
        "ordinary Send, not Restore, creates current request intent"
    );
}

#[test]
fn discard_is_neutral_exact_and_does_not_change_current_request() {
    let mut app = app();
    park(&mut app, "discard only this parked text");
    app.input = "keep current".into();
    app.cursor = 4;
    app.parked_draft_command("drafts");
    key(&mut app, KeyCode::Char('d'));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.parked_entries().count(), 1);
    key(&mut app, KeyCode::Char('d'));
    app.parked_drafts.complete.set(true);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.parked_entries().count(), 0);
    assert_eq!((app.input.as_str(), app.cursor), ("keep current", 4));
    assert!(app.pending_request.is_none());
}

#[test]
fn privacy_changed_ack_does_not_destroy_original_user_text() {
    let mut app = app();
    app.input = "original private text".into();
    app.cursor = 5;
    app.parked_draft_command("park-draft");
    let mut snapshot = app.snapshot("/project");
    snapshot.workflows.parked_drafts[0].text = "[REDACTED]".into();
    app.store_event(store::Event::Saved(snapshot));
    assert_eq!(
        (app.input.as_str(), app.cursor),
        ("original private text", 5)
    );
    assert!(app.pending_request.is_none());
}

#[test]
fn restore_reviews_retained_request_bytes_and_caret_without_requiring_a_live_editor() {
    let mut app = app();
    park(&mut app, "parked replacement 한글\r\n");
    app.input = "retained original 한글".into();
    app.cursor = 3;
    app.retain_action_editor();
    app.parked_draft_command("drafts");
    key(&mut app, KeyCode::Char('r'));
    let review = app.parked_drafts.review.as_ref().unwrap();
    assert_eq!(
        (review.original.as_str(), review.cursor),
        ("retained original 한글", 3)
    );
    key(&mut app, KeyCode::Enter);
    let retained = app
        .drafts
        .iter()
        .find(|draft| draft.field == store::EditorField::Request)
        .unwrap();
    assert_eq!(
        (retained.text.as_str(), retained.cursor),
        ("retained original 한글", 3)
    );
    assert_eq!(app.parked_entries().count(), 1);

    key(&mut app, KeyCode::Char('r'));
    app.drafts
        .iter_mut()
        .find(|draft| draft.field == store::EditorField::Request)
        .unwrap()
        .cursor = 4;
    app.parked_drafts.complete.set(true);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert!(app.parked_drafts.review.is_none());
    assert_eq!(app.parked_request(), ("retained original 한글", 4));

    key(&mut app, KeyCode::Char('r'));
    app.parked_drafts.complete.set(true);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        (app.input.as_str(), app.cursor),
        (
            "parked replacement 한글\r\n",
            "parked replacement 한글\r\n".len()
        )
    );
    assert_eq!(app.parked_entries().count(), 1);
    assert!(app.pending_request.is_none() && app.planning.pending.is_none());
}
