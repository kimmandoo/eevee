use super::*;
use crate::store::{Draft, EditorField, Partner, PartnerDrafts};

fn app() -> App {
    let mut app = App::new(
        std::env::temp_dir()
            .join("ev-formation-app")
            .to_string_lossy()
            .into_owned(),
        60,
        18,
    );
    app.task_id = 7;
    app.party = vec![
        Partner::default(),
        Partner {
            id: 2,
            ..Partner::default()
        },
    ];
    app.storage = StorageState::Saved;
    app.planning.preference_event(Ok(None));
    app.sources = super::super::sources::State::default();
    app.overlay = Some(Overlay::Formations);
    app.formations.mode = Mode::Mapping;
    app.formations.mapping = vec![Some(1), Some(2)];
    app.formations.recipe = Some(Recipe {
        schema_version: 2,
        name: "Two roles".into(),
        scope: vec![],
        context: vec![],
        steps: vec![RecipeStep {
            form: "Jolteon".into(),
            action: "Stage only".into(),
        }],
        validation_commands: vec![],
        completion_criteria: vec!["Review".into()],
        roles: vec![
            FormationSlot {
                role: "first".into(),
                form: "Jolteon".into(),
                request: "Inspect".into(),
            },
            FormationSlot {
                role: "second".into(),
                form: "Glaceon".into(),
                request: String::new(),
            },
        ],
    });
    app.formations.roster = app.partners();
    app.formations.owner = (app.task_id, app.lead.id);
    app
}
#[test]
fn formation_neutral_input_never_grants_and_mapping_changes_expire_the_rendered_review() {
    let mut app = app();
    let original = app.snapshot(&app.cwd);
    for key in [
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
    ] {
        app.formation_key(key);
        assert!(app.party_request.is_none());
    }
    app.formation_paste("y");
    let mut repeat = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
    repeat.kind = KeyEventKind::Repeat;
    app.formations.reviewed.set(true);
    app.formation_key(repeat);
    assert!(app.party_request.is_none());
    app.formation_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert!(!app.formations.reviewed.get());
    app.formation_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.party_request.is_none());
    assert_eq!(app.snapshot(&app.cwd), original);
}
#[test]
fn formation_acknowledgment_updates_only_owned_drafts_and_retains_other_task_facts() {
    let mut app = app();
    app.goal = "Original goal".into();
    let original_draft = PartnerDrafts {
        drafts: vec![Draft {
            field: EditorField::Request,
            text: "Draft 界 suffix".into(),
            cursor: 6,
        }],
        editing: true,
        editor_field: EditorField::Request,
        previous_form: None,
    };
    let mut task = app.snapshot(&app.cwd);
    task.partner_drafts.insert(1, original_draft.clone());
    task.form = 2;
    task.auto_form = false;
    task.revision = 1;
    task.partner_drafts.insert(
        2,
        PartnerDrafts {
            drafts: vec![Draft {
                field: EditorField::Request,
                text: "Independent review".into(),
                cursor: 18,
            }],
            ..PartnerDrafts::default()
        },
    );
    let roster = vec![
        Partner {
            form: 2,
            auto_form: false,
            ..Partner::default()
        },
        Partner {
            id: 2,
            form: 7,
            auto_form: false,
            ..Partner::default()
        },
    ];
    app.formation_event(Ok(store::formations::Response::Applied {
        task: Box::new(task),
        roster,
    }));
    assert_eq!(app.goal, "Original goal");
    assert_eq!(app.drafts, original_draft.drafts);
    assert_eq!(app.partner_drafts[&2].drafts[0].text, "Independent review");
    assert_eq!(app.form(), 2);
    assert!(!app.lead.auto_form);
    assert_eq!(app.form_selection, 2);
    assert!(app.planning.pending.is_none());
    assert!(app.evolution.transition.is_none());
    assert!(app.pending_command.is_none());
    assert!(app.pending_request.is_none());
}

#[test]
fn reviewed_formation_waits_for_its_exact_save_and_cancellation_or_new_revision_revokes_it() {
    for outcome in 0..3 {
        let mut app = app();
        app.revision = 1;
        app.formations.revision = 1;
        app.saved_revision = 0;
        app.save_requested = true;
        app.storage = StorageState::Pending;
        app.formations.reviewed.set(true);
        app.formation_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.formations.apply_pending);
        assert!(app.party_request.is_none());
        let saved = app.snapshot(&app.cwd);
        if outcome == 1 {
            app.formation_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        } else if outcome == 2 {
            app.revision += 1;
        }
        app.store_event(store::Event::Saved(saved));
        app.settle_formations();
        assert_eq!(app.party_request.is_some(), outcome == 0);
        assert!(!app.formations.apply_pending);
        assert!(app.pending_request.is_none());
        assert!(app.pending_command.is_none());
    }
}

#[test]
fn unchanged_formation_role_keeps_an_explicit_read_only_operation() {
    let mut app = app();
    app.lead.form = 3;
    app.lead.auto_form = false;
    app.party[0] = app.lead.clone();
    app.formations.roster = app.partners();
    app.planning
        .set_operation(crate::providers::Operation::Plan);
    let mut task = app.snapshot(&app.cwd);
    task.revision += 1;
    let roster = app.partners();
    app.formation_event(Ok(store::formations::Response::Applied {
        task: Box::new(task),
        roster,
    }));
    assert_eq!(
        app.planning.explicit_operation(),
        Some(crate::providers::Operation::Plan)
    );
    assert!(app.planning.pending.is_none());
    assert_eq!(app.form(), 3);
}

#[test]
fn queued_formation_cannot_apply_to_a_new_task_or_after_search_or_quit() {
    for action in 0..3 {
        let mut app = app();
        app.revision = 1;
        app.formations.revision = 1;
        app.saved_revision = 0;
        app.save_requested = true;
        app.storage = StorageState::Pending;
        app.formations.reviewed.set(true);
        app.formation_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        let saved = app.snapshot(&app.cwd);
        match action {
            0 => app.task_id += 1,
            1 => {
                app.update(Event::Key(KeyEvent::new(
                    KeyCode::Char('k'),
                    KeyModifiers::CONTROL,
                )));
            }
            _ => app.quit = true,
        }
        app.store_event(store::Event::Saved(saved));
        app.settle_formations();
        assert!(!app.formations.apply_pending);
        assert!(app.party_request.is_none());
        assert!(app.planning.pending.is_none());
    }
}
