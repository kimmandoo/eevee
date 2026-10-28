use super::*;
use crate::workspace::progress::{State, Status, Step};
fn app() -> App {
    let mut app = App::new("/workspace".into(), 120, 36);
    app.task_id = 7;
    app.storage = StorageState::Saved;
    app.planning.reset(7);
    app.planning.preference_event(Ok(None));
    app
}
fn proposed() -> State {
    let mut state = State::default();
    state
        .propose(
            "codex",
            vec![Step {
                label: "오류 조사".into(),
                phase: Some("investigate".into()),
                status: Status::Active,
            }],
        )
        .unwrap();
    state
}
#[test]
fn adoption_requires_visible_current_revision_and_never_dispatches() {
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = app();
        app.width = width;
        app.height = height;
        app.workflows.progress = Some(proposed());
        let before = (
            app.form(),
            app.input.clone(),
            app.cursor,
            app.criteria.clone(),
        );
        app.progress_command("todo-adopt", "");
        assert!(
            !app.execution_any_pending(),
            "Progress inspection must not enqueue unrelated execution-history work"
        );
        assert!(
            app.workflow
                .notice
                .contains("Inspect the whole proposed list")
        );
        assert!(
            app.workflows
                .progress
                .as_ref()
                .unwrap()
                .focus_label()
                .is_none()
        );
        app.workflow.todo_revision = Some(1);
        app.workflow.todo_reviewed = true;
        app.progress_command("todo-adopt", "");
        assert_eq!(
            app.workflows.progress.as_ref().unwrap().focus_label(),
            Some("오류 조사")
        );
        assert_eq!(
            (
                app.form(),
                app.input.clone(),
                app.cursor,
                app.criteria.clone()
            ),
            before
        );
        assert!(app.pending_request.is_none() && app.planning.pending.is_none());
        assert!(!app.complete);
    }
}
#[test]
fn editable_card_preserves_request_and_flags_duplicate_labels() {
    let mut app = app();
    app.workflows.progress = Some(proposed());
    app.input = "Keep 한글 draft".into();
    app.cursor = 5;
    let before = (
        app.input.clone(),
        app.cursor,
        app.workflows.progress.clone(),
    );
    app.progress_command("todo-edit", "");
    assert!(app.workflow.todo_editor.is_some());
    assert!(
        !app.execution_any_pending(),
        "Progress JSON editor must remain an idle task-card action"
    );
    assert!(app.command_surface_editing());
    app.workflow.todo_editor = Some(
        "[{\"label\":\"x\",\"status\":\"pending\"},{\"label\":\"x\",\"status\":\"done\"}]".into(),
    );
    app.workflow.todo_cursor = app.workflow.todo_editor.as_ref().unwrap().len();
    app.progress_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.workflow.notice.contains("Duplicate"));
    assert!(app.workflow.todo_editor.is_some());
    assert_eq!(
        (
            app.input.clone(),
            app.cursor,
            app.workflows.progress.clone()
        ),
        before
    );
    assert!(app.pending_request.is_none() && app.planning.pending.is_none());
    app.progress_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.workflow.todo_editor.is_none());
    assert_eq!((&app.input, app.cursor), (&before.0, before.1));
    app.progress_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!((&app.input, app.cursor), (&before.0, before.1));
    assert!(app.overlay.is_none());
}
#[test]
fn absent_source_is_visible_on_every_route() {
    for agent in crate::providers::Agent::ALL {
        let mut app = app();
        app.planning.agent = agent;
        app.progress_command("todo", "");
        assert!(app.progress_card().contains("No todo yet"));
        assert!(app.workflow.todo_view);
        assert!(app.pending_request.is_none() && app.planning.pending.is_none());
    }
}
#[test]
fn literal_todo_arguments_win_over_matching_catalogue_labels() {
    use crate::app::commands::{Choice, Popup};
    for (name, argument) in [
        ("todo-eager", "on"),
        (
            "todo-edit",
            "[{\"label\":\"fix code\",\"phase\":null,\"status\":\"pending\"}]",
        ),
        ("todo", "start exact label"),
    ] {
        let popup = Popup {
            query: format!("{name} {argument}"),
            selected: Some(0),
            choices: vec![Choice::new(name, "todo on off start exact label pending")],
            scope: None,
        };
        assert_eq!(popup.argument(), Some((name, argument)));
        assert_eq!(popup.candidates().count(), 1);
        if name == "todo-eager" {
            let mut app = app();
            app.command_popup = Some(popup);
            app.command_popup_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            assert!(app.workflows.progress.as_ref().unwrap().eager());
            let mut app = self::app();
            assert!(app.progress_card().contains("prelude: on"), "on by default");
            app.progress_command("todo-eager", "off");
            assert!(!app.workflows.progress.as_ref().unwrap().eager());
            assert!(app.progress_card().contains("prelude: off"));
            assert!(app.pending_request.is_none() && app.planning.pending.is_none());
        }
    }
}
