use super::*;
fn hit(task: i64) -> Hit {
    Hit {
        task,
        archived: false,
        kind: "provider journal",
        when: crate::store::retained::When::Unknown,
        target: Target::Journal("immutable-journal".into()),
        digest: [0; 32],
        excerpt: "Explanation 설명".into(),
        attribution: "Original partner / original request".into(),
    }
}
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
#[test]
fn retained_inspection_landmark_edit_resize_preserve_exact_draft_and_paused_output() {
    let mut app = App::new("/project".into(), 80, 24);
    app.task_id = 7;
    app.input = "unsent 설명\r\nkeep trailing\n".into();
    app.cursor = "unsent 설명".len();
    app.editing = true;
    app.planning.following = false;
    app.planning.scroll = 9;
    let before = (
        app.lead().id,
        app.input.clone(),
        app.cursor,
        app.planning.following,
        app.planning.scroll,
    );
    assert!(app.retained_command("retained-responses", ""));
    let request = app.retained.pending.take().unwrap();
    app.retained.response(
        &request,
        Ok(Response::Opened {
            hit: hit(7),
            text: "a".repeat(1000),
            position: 80,
        }),
    );
    app.retained_key(key(KeyCode::Char('l')));
    app.retained.paste("Explanation 설명");
    app.retained_key(key(KeyCode::Enter));
    assert_eq!(app.workflows.landmarks.len(), 1);
    assert_eq!(app.workflows.landmarks[0].position, Some(80));
    app.update(Event::Resize(60, 18));
    assert_eq!(app.retained.position, 80);
    assert_eq!(
        (
            app.lead().id,
            app.input.clone(),
            app.cursor,
            app.planning.following,
            app.planning.scroll
        ),
        before
    );
    assert!(app.pending_request.is_none());
    app.retained_key(key(KeyCode::Esc));
    app.retained_command("landmarks", "");
    app.retained_key(key(KeyCode::Char('n')));
    app.retained.paste(" not saved");
    app.retained_key(key(KeyCode::Esc));
    assert_eq!(app.workflows.landmarks[0].label, "Explanation 설명");
    let mut repeated = key(KeyCode::Char('d'));
    repeated.kind = KeyEventKind::Repeat;
    app.retained_key(repeated);
    assert_eq!(
        app.workflows.landmarks.len(),
        1,
        "a held key must not delete retained landmarks"
    );
    assert_eq!(app.workflows.landmarks[0].label, "Explanation 설명");
    app.retained_key(key(KeyCode::Char('d')));
    assert!(app.workflows.landmarks.is_empty());
    assert_eq!(
        (
            app.lead().id,
            app.input.clone(),
            app.cursor,
            app.planning.following,
            app.planning.scroll
        ),
        before
    );
}
#[test]
fn cancelled_search_cannot_reopen_and_foreign_task_cannot_gain_current_task_label() {
    let mut app = App::new("/project".into(), 80, 24);
    app.task_id = 7;
    app.retained_command("project-search", "query");
    let old = app.retained.pending.take().unwrap();
    app.retained_key(key(KeyCode::Esc));
    app.retained.response(
        &old,
        Ok(Response::Hits {
            values: vec![hit(8)],
            incomplete: false,
            scanned: 1,
        }),
    );
    assert!(app.retained.hits.is_empty());
    assert_ne!(app.overlay, Some(Overlay::Retained));
    app.retained_command("project-search", "query");
    let request = app.retained.pending.take().unwrap();
    app.retained.response(
        &request,
        Ok(Response::Opened {
            hit: hit(8),
            text: "retained text".into(),
            position: 0,
        }),
    );
    app.retained_key(key(KeyCode::Char('l')));
    assert!(app.retained.editor.is_none());
    assert!(app.workflows.landmarks.is_empty());
    app.retained.invalidate_task(8);
    assert!(app.retained.text.is_empty());
    assert!(app.retained.opened.is_none());
}
