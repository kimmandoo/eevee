use super::*;

#[test]
fn semantic_specialist_actions_keep_editor_text_and_context_guards() {
    let mut app = App::new("/project".into(), 120, 36);
    app.task_id = 7;
    assert!(app.workflow_command("edit-hypothesis"));
    app.workflow_paste("Literal /diagnostic, not a command.");
    assert!(!app.workflow_command("diagnostic"));
    assert_eq!(
        app.workflow.value(Field::Hypothesis),
        "Literal /diagnostic, not a command."
    );
    app.workflow_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.workflow_command("add-hypothesis"));
    assert_eq!(app.workflows.hypotheses.len(), 1);
    assert!(app.workflow_command("hypothesis-evidence"));
    assert!(!app.workflow_command("specialist-fix"));
    assert!(app.workflow_command("attach-evidence"));
    assert!(app.workflows.hypotheses[0].evidence.is_empty());
    assert!(app.workflow.chooser.is_some());
    assert!(app.workflow.notice.contains("No evidence selected"));
}
