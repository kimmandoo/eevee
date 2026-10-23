use super::*;
use crate::app::StorageState;

fn app() -> App {
    let mut app = App::new("/fixture".into(), 60, 18);
    app.task_id = 7;
    app.storage = StorageState::Saved;
    app.planning.preference_event(Ok(None));
    app.sources = crate::app::sources::State::default();
    app.overlay = Some(Overlay::Workflows);
    app.workflow.authoring.active = true;
    app.workflow.authoring.owner = (7, app.lead.id);
    app
}
fn key(app: &mut App, code: KeyCode) {
    app.recipe_authoring_key(KeyEvent::new(code, KeyModifiers::NONE));
}
#[test]
fn neutral_review_input_cancel_and_unrendered_confirmation_start_no_effect() {
    let mut app = app();
    app.workflow.authoring.mode = Mode::Review;
    key(&mut app, KeyCode::Char('y'));
    assert!(app.party_request.is_none());
    key(&mut app, KeyCode::Enter);
    assert!(app.workflow.authoring.mode == Mode::Menu);
    app.workflow.authoring.mode = Mode::Review;
    key(&mut app, KeyCode::Esc);
    assert!(app.workflow.authoring.mode == Mode::Menu);
    assert!(app.workflow.authoring.active);
    assert!(app.party_request.is_none() && !app.recipe_export_busy());
    assert!(app.pending_request.is_none() && app.pending_command.is_none());
}
#[test]
fn authoring_editor_is_grapheme_safe_retains_caret_and_never_changes_request_draft() {
    let mut app = app();
    let original = app.drafts.clone();
    app.workflow.authoring.edit(Field::Steps);
    assert!(app.workflow.paste("Flareon: 界e\u{301}"));
    key(&mut app, KeyCode::Left);
    let cursor = app.workflow.authoring.cursor;
    key(&mut app, KeyCode::Delete);
    assert_eq!(app.workflow.authoring.draft.steps, "Flareon: 界");
    assert_eq!(app.workflow.authoring.cursor, cursor);
    key(&mut app, KeyCode::Esc);
    assert!(!app.workflow.authoring.editing());
    assert_eq!(app.drafts, original);
    key(&mut app, KeyCode::Esc);
    assert!(!app.workflow.authoring.active);
    app.recipe_save();
    assert_eq!(app.workflow.authoring.draft.steps, "Flareon: 界");
}
#[test]
fn ephemeral_authoring_never_changes_serialized_specialist_admission() {
    let mut state = super::super::State::default();
    let original = state.draft_snapshot();
    state.authoring.active = true;
    state.authoring.draft.steps = "Flareon: Private draft 界".into();
    state.authoring.draft.runs = vec!["run-local".into()];
    state.authoring.generation = 4;
    state.authoring.reviewed = true;
    assert_eq!(state.draft_snapshot(), original);
    state.restore_draft(original);
    assert!(!state.authoring.active && !state.authoring.reviewed);
}
#[test]
fn inflight_publication_keeps_ownership_until_result_and_failure_retains_authoring_bytes() {
    let mut app = app();
    app.workflow.authoring.draft.steps = "Flareon: Keep this exact draft".into();
    app.workflow.authoring.inflight = true;
    app.party_inflight = true;
    key(&mut app, KeyCode::Esc);
    assert!(app.recipe_export_busy());
    assert!(app.workflow.authoring.active);
    app.formation_event(Err("Create-only collision".into()));
    assert!(!app.recipe_export_busy() && !app.party_inflight);
    assert_eq!(
        app.workflow.authoring.draft.steps,
        "Flareon: Keep this exact draft"
    );
    assert_eq!(app.workflow.authoring.error, "Create-only collision");
}
#[test]
fn small_terminal_revokes_review_and_cancellation_does_not_publish() {
    let mut app = app();
    app.workflow.authoring.mode = Mode::Review;
    app.workflow.authoring.reviewed = true;
    app.width = 59;
    key(&mut app, KeyCode::Char('y'));
    assert!(!app.workflow.authoring.reviewed);
    assert!(app.party_request.is_none());
    key(&mut app, KeyCode::Esc);
    assert!(!app.workflow.authoring.active);
    assert!(app.workflow.authoring.mode == Mode::Menu);
}
