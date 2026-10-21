use super::*;

fn app() -> App {
    // Navigation-focused tests explicitly leave the first-run composer.
    let mut app = App::new("/workspace".into(), 120, 36);
    app.store_event(store::Event::Loaded(store::Project {
        tasks: Vec::new(),
        partners: vec![store::Partner::default()],
        active_id: None,
        usage_bytes: 0,
    }));
    app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    app
}

fn committed(app: &App) -> store::Snapshot {
    let mut snapshot = app.snapshot("/workspace");
    snapshot.id = app.task_id.max(1);
    snapshot
}

fn key(app: &mut App, code: KeyCode) -> bool {
    modified_key(app, code, KeyModifiers::NONE)
}

fn modified_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> bool {
    // These key-only tests model a rendered full-size confirmation between keys.
    if code == KeyCode::Enter && app.confirmation.is_some() && app.width >= 60 && app.height >= 18 {
        app.confirmation_reviewed = true;
    }
    app.update(Event::Key(KeyEvent::new(code, modifiers)))
}

#[test]
fn root_provider_metadata_shortcuts_prepare_review_not_execution() {
    for (command, inspect_models) in [("sessions", false), ("models-refresh", true)] {
        let mut app = app();
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.goal = "Inspect provider metadata".into();
        app.store_event(store::Event::Loaded(store::Project {
            tasks: vec![task],
            partners: vec![store::Partner::default()],
            active_id: Some(7),
            usage_bytes: 0,
        }));
        key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, command);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.overlay, Some(Overlay::Planning));
        assert!(app.command_popup.is_none());
        assert!(matches!(
            app.planning.pending.as_ref(),
            Some(crate::providers::Request::Preview(input))
                if input.inspect_models == inspect_models
                    && input.inspect_sessions != inspect_models
                    && input.resume.is_none()
        ));
        assert!(!app.planning.affirmative);
        assert!(app.execution.active.is_none());
    }
}

#[test]
fn corrected_command_query_keeps_a_choice_but_clearing_it_is_neutral() {
    let mut app = app();
    key(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "workflow-reviewx");
    assert_eq!(app.command_popup.as_ref().unwrap().candidates().count(), 0);
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.command_popup.as_ref().unwrap().selected, Some(0));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.workflow.tab, workflows::Tab::Review);
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Backspace);
    assert!(app.command_popup.as_ref().unwrap().selected.is_none());
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none());
    assert!(app.planning.pending.is_none());
}

#[test]
fn root_commands_open_each_specialist_desk_without_a_second_palette() {
    for (command, tab) in [
        ("workflow-diagnosis", workflows::Tab::Diagnosis),
        ("workflow-refactor", workflows::Tab::Refactor),
        ("workflow-review", workflows::Tab::Review),
        ("workflow-recipe", workflows::Tab::Recipe),
    ] {
        let mut app = app();
        key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, command);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.overlay, Some(Overlay::Workflows), "{command}");
        assert_eq!(app.workflow.tab, tab);
        assert!(app.command_popup.is_none());
        assert!(app.planning.pending.is_none());
        assert!(app.execution.active.is_none());
        assert!(!app.workflow.preview);
        assert!(!app.workflow.affirmative);
        key(&mut app, KeyCode::Esc);
        assert!(app.overlay.is_none());
    }
}

#[test]
fn direct_specialist_navigation_restores_draft_caret_and_suffix() {
    let mut app = app();
    app.command("goal");
    app.update(Event::Paste("draft 한글 suffix".into()));
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Left);
    let draft = app.input.clone();
    let cursor = app.cursor;
    key(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "workflow-refactor");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.workflow.tab, workflows::Tab::Refactor);
    assert!(app.goal.is_empty());
    assert!(app.planning.pending.is_none());
    key(&mut app, KeyCode::Esc);
    assert!(app.editing);
    assert_eq!(app.input, draft);
    assert_eq!(app.cursor, cursor);
}

#[test]
fn specialist_shortcut_rechecks_hidden_review_and_popup_scope() {
    let mut app = app();
    key(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "workflow-review");
    app.workflow.preview = true;
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none());
    assert!(app.command_popup.is_some());
    assert!(app.workflow.preview);
    assert!(!app.workflow.affirmative);
    assert!(app.notice.contains("specialist"));
    app.workflow.preview = false;
    app.overlay = Some(Overlay::Help);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.overlay, Some(Overlay::Help));
    assert!(app.notice.contains("workspace changed"));
    assert!(app.planning.pending.is_none());
}

#[test]
fn root_session_inspection_keeps_saved_task_prerequisites() {
    let mut app = app();
    key(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "sessions");
    key(&mut app, KeyCode::Enter);
    assert!(app.command_popup.is_some());
    assert!(app.overlay.is_none());
    assert!(app.planning.pending.is_none());
    assert!(app.notice.contains("save a local task"));
}

#[test]
fn root_criteria_edit_opens_the_editor_without_the_list() {
    let mut app = app();
    key(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "criteria-edit");
    key(&mut app, KeyCode::Enter);
    assert!(app.editing);
    assert_eq!(app.editor_field, store::EditorField::Criteria);
    assert!(app.overlay.is_none());
    assert!(app.criteria.is_empty());
    assert!(app.planning.pending.is_none());
}

#[test]
fn command_search_keeps_exact_commands_first_and_blank_query_lists_all() {
    let mut app = app();
    app.open_commands();
    let popup = app.command_popup.as_mut().unwrap();
    popup.query = "/ROLE-REVIEW".into();
    assert_eq!(
        popup
            .candidates()
            .map(|choice| choice.command)
            .collect::<Vec<_>>(),
        vec!["role-review"]
    );
    popup.query = " \t ".into();
    assert_eq!(popup.candidates().count(), popup.choices.len());
}

#[test]
fn typed_lead_argument_wins_over_matching_search_labels() {
    let mut app = app();
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner::default(),
        store::Partner {
            id: 2,
            nickname: Some("partner".into()),
            ..store::Partner::default()
        },
    ])));
    key(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "lead partner");
    assert!(
        app.command_popup
            .as_ref()
            .unwrap()
            .candidates()
            .any(|choice| choice.command == "lead")
    );
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.lead().id, 2, "{}", app.notice);
    assert!(app.overlay.is_none());
    assert!(app.planning.pending.is_none());
}

#[test]
fn clearing_first_draft_before_insert_ack_does_not_restore_deleted_text() {
    let mut app = app();
    app.command("goal");
    key(&mut app, KeyCode::Char('x'));
    let first = committed(&app);
    app.save_submitted();
    key(&mut app, KeyCode::Backspace);
    assert!(app.revision > first.revision);
    app.store_event(store::Event::Saved(first));
    let cleared = committed(&app);
    assert!(cleared.goal.is_empty());
    assert!(
        cleared
            .partner_drafts
            .values()
            .all(|state| state.drafts.is_empty())
    );
    let mut resumed = App::new("/workspace".into(), 120, 36);
    resumed.store_event(store::Event::Loaded(store::Project {
        active_id: Some(cleared.id),
        tasks: vec![cleared],
        partners: vec![store::Partner::default()],
        usage_bytes: 0,
    }));
    assert!(resumed.input.is_empty());
    assert!(resumed.drafts.is_empty());
}

#[test]
fn passive_discovery_only_redraws_the_connections_drawer() {
    for overlay in [None, Some(Overlay::Help), Some(Overlay::Agents)] {
        let mut app = app();
        app.overlay = overlay;
        app.agents.pending = None;
        assert!(app.agents.busy());
        let redraw = app.agent_event(crate::providers::agent::Event::Discovered(Vec::new()));
        assert!(!app.agents.busy(), "hidden results must still be applied");
        assert_eq!(redraw, overlay == Some(Overlay::Agents));
    }
}

#[test]
fn first_run_typing_is_input_and_global_navigation_retains_the_draft() {
    let mut app = App::new("/workspace".into(), 120, 36);
    key(&mut app, KeyCode::Char('q'));
    app.update(Event::Paste("uick task 한글".into()));
    assert_eq!(app.input, "quick task 한글");
    assert!(!app.quit);
    assert!(app.goal.is_empty());
    app.command("connect");
    assert_eq!(app.overlay, Some(Overlay::Agents));
    assert_eq!(app.drafts[0].text, "quick task 한글");
    assert!(app.agents.pending_auth.is_none());
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.input, "quick task 한글");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.goal, "quick task 한글");
    assert_eq!(app.snapshot("/workspace").request_text(), "quick task 한글");
    assert!(app.pending_request.is_some());
}

#[test]
fn image_download_requires_visible_review_and_survives_resize_as_neutral() {
    let mut app = app();
    app.command("images");
    key(&mut app, KeyCode::Enter);
    assert!(app.images_pending.is_none());
    assert!(app.overlay.is_none());
    app.command("images");
    app.images_reviewed = true;
    key(&mut app, KeyCode::Down);
    app.update(Event::Resize(60, 18));
    key(&mut app, KeyCode::Enter);
    assert!(app.images_pending.is_none());
    app.command("images");
    app.images_reviewed = true;
    app.images_enabled = false;
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(app.images_pending.is_none());
    assert!(!app.images_busy);
}

#[test]
fn all_forms_preview_confirm_and_cancel_in_grid_list_and_linear_layouts() {
    for (width, height, linear) in [
        (120, 36, false),
        (90, 30, false),
        (90, 29, false),
        (70, 24, false),
        (120, 36, true),
    ] {
        let mut app = App::new("/workspace".into(), width, height);
        key(&mut app, KeyCode::Esc);
        app.linear = linear;
        app.goal = "Keep existing work\n한글".into();
        app.focus = Pane::Inspector;
        for selection in 0..FORMS.len() {
            let original = app.form();
            app.command("forms");
            app.selected = selection;
            assert_eq!(app.selected, selection);
            assert_eq!(app.form(), original);
            key(&mut app, KeyCode::Esc);
            assert_eq!(app.form(), original);
            app.command("forms");
            assert_eq!(app.selected, selection);
            key(&mut app, KeyCode::Enter);
            assert_eq!(app.form(), selection);
            assert_eq!(app.goal, "Keep existing work\n한글");
            assert_eq!(app.focus, Pane::Inspector);
        }
    }
}

#[test]
fn restart_restores_the_lead_partners_stored_form() {
    let mut task = store::Snapshot::empty("/workspace".into());
    task.id = 7;
    task.goal = "kept".into();
    task.form = 5;
    task.lead_partner = 1;
    let partner = store::Partner {
        form: 5,
        met_at: 172800000,
        ..store::Partner::default()
    };
    let mut resumed = App::new("/workspace".into(), 120, 36);
    resumed.store_event(store::Event::Loaded(store::Project {
        tasks: vec![task],
        partners: vec![partner],
        active_id: Some(7),
        usage_bytes: 0,
    }));
    assert_eq!(resumed.lead().id, 1);
    assert_eq!(resumed.lead().met_at, 172800000);
    assert_eq!(resumed.form(), 5);
    assert_eq!(resumed.lead_label(), "Eevee #1 (Umbreon)");
}

#[test]
fn backspace_and_back_form_toggle_between_the_last_two_forms() {
    let mut app = app();
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.form(), 0);
    assert!(app.notice.contains("No previous form"));
    app.command("vaporeon");
    assert_eq!(app.form(), 1);
    app.command("flareon");
    assert_eq!(app.form(), 3);
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.form(), 1);
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.form(), 3);
    app.command("back-form");
    assert_eq!(app.form(), 1);
    app.command("back-form");
    assert_eq!(app.form(), 3);
}

#[test]
fn previous_form_toggle_is_suppressed_where_backspace_is_text_delete() {
    let mut app = app();
    app.command("vaporeon");
    assert_eq!(app.form(), 1);
    app.command("goal");
    assert!(app.editing);
    app.update(Event::Paste("abc".into()));
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.input, "ab");
    assert_eq!(app.form(), 1);
    // The command popup's query owns Backspace while it is open.
    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.form(), 1);
    assert_eq!(app.command_popup.as_ref().unwrap().query, "");
}

#[test]
fn previous_form_survives_a_restart() {
    let mut app = app();
    app.command("vaporeon");
    app.command("flareon");
    let mut snapshot = committed(&app);
    snapshot.id = 7;
    assert_eq!(snapshot.navigation.previous_form, Some(1));
    let partner = store::Partner {
        form: 3,
        ..store::Partner::default()
    };
    let mut resumed = App::new("/workspace".into(), 120, 36);
    resumed.store_event(store::Event::Loaded(store::Project {
        tasks: vec![snapshot],
        partners: vec![partner],
        active_id: Some(7),
        usage_bytes: 0,
    }));
    assert_eq!(resumed.form(), 3);
    resumed.command("back-form");
    assert_eq!(resumed.form(), 1);
    resumed.command("back-form");
    assert_eq!(resumed.form(), 3);
}

#[test]
fn editing_is_modal_and_cancel_keeps_the_committed_goal() {
    let mut app = app();
    app.goal = "original".into();
    app.command("goal");
    key(&mut app, KeyCode::Char('q'));
    assert!(!app.quit);
    assert!(!key(&mut app, KeyCode::Tab));
    for code in [KeyCode::Left, KeyCode::Enter, KeyCode::Esc] {
        assert!(!modified_key(&mut app, code, KeyModifiers::CONTROL));
        assert!(!modified_key(&mut app, code, KeyModifiers::ALT));
    }
    assert_eq!(app.overlay, None);
    assert!(app.editing);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.goal, "original");
    assert_eq!(
        app.confirmation,
        Some(Confirmation {
            kind: ConfirmationKind::DiscardDraft,
            destructive: false
        })
    );
    assert_eq!(app.input, "originalq");
    key(&mut app, KeyCode::Esc);
    assert!(app.editing);
    assert_eq!(app.input, "originalq");
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert!(!app.editing);
    assert_eq!(app.input, "");
    app.command("goal");
    key(&mut app, KeyCode::Char('!'));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.goal, "original!");
}

#[test]
fn cursor_insertion_and_deletion_preserve_whole_unicode_graphemes() {
    let mut app = app();
    app.command("goal");
    app.update(Event::Paste("한e\u{301}👩‍💻界".into()));
    key(&mut app, KeyCode::Left);
    assert_eq!(&app.input[app.cursor..], "界");
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.input, "한e\u{301}界");
    key(&mut app, KeyCode::Left);
    assert_eq!(&app.input[app.cursor..], "e\u{301}界");
    key(&mut app, KeyCode::Delete);
    assert_eq!(app.input, "한界");
    key(&mut app, KeyCode::Char('中'));
    assert_eq!(app.input, "한中界");
    key(&mut app, KeyCode::Home);
    assert_eq!(app.cursor, 0);
    assert!(!key(&mut app, KeyCode::Backspace));
    key(&mut app, KeyCode::End);
    assert_eq!(app.cursor, app.input.len());
    assert!(!key(&mut app, KeyCode::Delete));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.goal, "한中界");
}

#[test]
fn vt_backspace_deletes_a_combining_grapheme_without_leaving_its_mark() {
    let mut app = app();
    app.command("goal");
    app.update(Event::Paste("cafe\u{301}".into()));
    assert!(modified_key(
        &mut app,
        KeyCode::Char('h'),
        KeyModifiers::CONTROL
    ));
    assert_eq!(app.input, "caf");
    assert_eq!(app.cursor, 3);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.goal, "caf");
}

#[test]
fn multiline_movement_retains_column_across_short_rows_and_shift_enter() {
    let mut app = app();
    app.command("goal");
    app.update(Event::Paste("한e\u{301}界\nx\n👩‍💻中文".into()));
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Up);
    assert_eq!(&app.input[..app.cursor], "한e\u{301}界\nx");
    key(&mut app, KeyCode::Up);
    assert_eq!(&app.input[..app.cursor], "한e\u{301}");
    key(&mut app, KeyCode::Home);
    assert_eq!(app.cursor, 0);
    key(&mut app, KeyCode::Down);
    assert_eq!(&app.input[app.cursor..], "x\n👩‍💻中文");
    key(&mut app, KeyCode::End);
    modified_key(&mut app, KeyCode::Enter, KeyModifiers::SHIFT);
    assert!(app.editing);
    assert!(app.input.contains("x\n\n"));
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Delete);
    assert!(app.input.contains("x👩‍💻"));
}

#[test]
fn portable_newline_key_keeps_the_draft_local_until_explicit_apply() {
    let mut app = app();
    app.command("goal");
    app.update(Event::Paste("first".into()));
    modified_key(&mut app, KeyCode::Char('n'), KeyModifiers::CONTROL);
    app.update(Event::Paste("second".into()));
    assert!(app.editing);
    assert_eq!(app.goal, "");
    assert_eq!(app.input, "first\nsecond");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.goal, "first\nsecond");
}

#[test]
fn pasted_newlines_and_commands_are_bounded_local_text_not_actions() {
    let mut app = app();
    assert!(!app.update(Event::Paste("q\n/run".into())));
    app.command("goal");
    app.update(Event::Paste("q\r\n/run\r한\x1b\x07\u{202e}\t".into()));
    assert_eq!(app.input, "q\n/run\n한");
    assert!(!app.quit);
    assert!(app.editing);
    assert_eq!(app.goal, "");
    app.update(Event::Paste("界".repeat(MAX_GOAL_BYTES)));
    assert!(app.input.len() <= MAX_GOAL_BYTES);
    assert!(app.input.len() > MAX_GOAL_BYTES - 3);
    assert!(app.input_truncated);
    assert!(!app.update(Event::Paste("界".into())));
    key(&mut app, KeyCode::Enter);
    assert!(app.goal.starts_with("q\n/run\n한"));
    assert!(!app.quit);
}

#[test]
fn resizing_preserves_preferred_focus_overlay_selection_and_unicode_draft() {
    let mut app = app();
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Pane::Inspector);
    app.command("forms");
    app.selected = 8;
    app.update(Event::Resize(70, 24));
    assert_eq!(app.focus, Pane::Inspector);
    assert_eq!(app.effective_focus(), Pane::Workspace);
    assert_eq!(app.selected, 8);
    key(&mut app, KeyCode::Esc);
    assert!(!key(&mut app, KeyCode::Tab));
    app.update(Event::Resize(120, 36));
    assert_eq!(app.effective_focus(), Pane::Inspector);
    app.command("goal");
    app.update(Event::Paste("draft\n한e\u{301}".into()));
    key(&mut app, KeyCode::Left);
    let cursor = app.cursor;
    app.update(Event::Resize(10, 3));
    assert_eq!(app.input, "draft\n한e\u{301}");
    assert_eq!(app.cursor, cursor);
    key(&mut app, KeyCode::Esc);
    assert!(app.editing);
    assert_eq!(
        app.confirmation.unwrap().kind,
        ConfirmationKind::DiscardDraft
    );
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.cursor, cursor);
    assert_eq!(app.input, "draft\n한e\u{301}");
    app.update(Event::Resize(120, 36));
    assert_eq!(app.effective_focus(), Pane::Inspector);
}

#[test]
fn overlays_restore_invoking_surface_and_keep_independent_selections() {
    let mut app = app();
    app.focus = Pane::Companion;
    app.command("forms");
    app.selected = 7;
    modified_key(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
    assert!(app.command_popup.as_ref().unwrap().selected.is_none());
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.overlay, Some(Overlay::Forms));
    assert_eq!(app.selected, 7);
    app.command("goal");
    assert!(app.editing);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.overlay, Some(Overlay::Forms));
    assert_eq!(app.selected, 7);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.effective_focus(), Pane::Companion);
}

#[test]
fn each_nonselecting_surface_retains_its_own_scroll() {
    let mut app = app();
    for surface in [
        ScrollSurface::Companion,
        ScrollSurface::Workspace,
        ScrollSurface::Inspector,
        ScrollSurface::Help,
        ScrollSurface::Preview,
    ] {
        app.set_scroll_viewport(surface, 100, 10);
    }
    key(&mut app, KeyCode::Down);
    assert_eq!(app.scroll(), 1);
    app.command("help");
    key(&mut app, KeyCode::PageDown);
    let help_scroll = app.scroll();
    assert!(help_scroll > 1);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.scroll(), 1);
    app.command("inspect");
    assert_eq!(app.scroll(), 0);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Esc);
    app.command("help");
    assert_eq!(app.scroll(), help_scroll);
    key(&mut app, KeyCode::PageUp);
    assert_eq!(app.scroll(), 0);
    key(&mut app, KeyCode::Esc);
    app.command("inspect");
    assert_eq!(app.scroll(), 2);
    key(&mut app, KeyCode::Home);
    assert_eq!(app.scroll(), 0);
    key(&mut app, KeyCode::Esc);
    app.command("companion");
    key(&mut app, KeyCode::Down);
    assert_eq!(app.scroll(), 1);
    key(&mut app, KeyCode::Esc);
    app.command("preview");
    assert_eq!(app.scroll(), 0);
}

#[test]
fn preview_is_navigation_only_and_preserves_picker_preview() {
    let mut app = app();
    app.goal = "local goal".into();
    app.command("forms");
    app.selected = 8;
    app.command("preview");
    assert_eq!(app.overlay, Some(Overlay::Preview));
    assert_eq!(app.selected, 8);
    assert_eq!(app.preview_form(), 8);
    assert!(!key(&mut app, KeyCode::Enter));
    assert!(!key(&mut app, KeyCode::Char('r')));
    assert_eq!(app.form(), 0);
    assert_eq!(app.goal, "local goal");
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.overlay, Some(Overlay::Forms));
    assert_eq!(app.selected, 8);
    key(&mut app, KeyCode::Esc);
    app.command("preview");
    assert_eq!(app.overlay, Some(Overlay::Preview));
    assert_eq!(app.preview_form(), 0);
    key(&mut app, KeyCode::Esc);
    app.command("preview");
    assert_eq!(app.overlay, Some(Overlay::Preview));
    assert_eq!(app.preview_form(), 0);
}

#[test]
fn linear_toggle_and_semantic_actions_remain_available() {
    let mut app = app();
    app.command("layout");
    assert!(app.linear);
    app.command("goal");
    app.update(Event::Paste("linear goal".into()));
    key(&mut app, KeyCode::Enter);
    app.command("forms");
    app.selected = 2;
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.form(), 2);
    app.command("preview");
    key(&mut app, KeyCode::Esc);
    app.command("layout");
    assert!(!app.linear);
    assert_eq!(app.goal, "linear goal");
    app.store_event(store::Event::Saved(committed(&app)));
    app.command("quit");
    assert!(
        app.confirmation.is_none(),
        "a committed task exits directly"
    );
    assert!(app.quit);
}

#[test]
fn imported_decision_preserves_exact_reviewed_bytes_and_refuses_overflow() {
    let mut app = app();
    let decision = format!("{} [unverified imported decision]", "한".repeat(1300));
    app.handoff.decision = Some(decision.clone());
    assert!(app.settle_planning());
    assert_eq!(app.input, decision);
    assert!(app.editing);
    assert!(app.next_action.is_empty());

    let mut app = super::App::new("project".into(), 120, 36);
    key(&mut app, KeyCode::Esc);
    app.handoff.decision = Some("x".repeat(4097));
    assert!(app.settle_planning());
    assert!(!app.editing);
    assert!(app.input.is_empty());
    assert!(app.next_action.is_empty());
}

#[test]
fn provider_exit_acknowledgement_cannot_bypass_local_work_confirmation() {
    let mut app = app();
    assert!(
        app.execution
            .prepare_command(1, r#"["printf","not-dispatched"]"#)
    );
    app.planning.quit_ready = true;
    assert!(app.settle_planning());
    assert!(!app.quit);
    assert!(app.execution.busy());
    let review = app
        .cooperation
        .cancel_review
        .as_ref()
        .expect("every quit route reaches the all-owner review");
    assert!(!review.affirmative);
    assert!(!app.authentication_ready());
}

#[test]
fn tiny_terminal_preserves_overlay_back_and_control_c_exit() {
    let mut app = app();
    app.command("help");
    app.update(Event::Resize(10, 3));
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.overlay, None);
    app.update(Event::Resize(120, 36));
    app.command("goal");
    app.update(Event::Resize(10, 3));
    modified_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(app.quit);
}

#[test]
fn dirty_draft_confirmation_restores_invoking_overlay_and_focus() {
    let mut app = app();
    app.goal = "committed".into();
    app.focus = Pane::Inspector;
    app.command("forms");
    app.selected = 7;
    app.command("goal");
    app.update(Event::Paste("\r\n한글".into()));
    key(&mut app, KeyCode::Esc);
    assert_eq!(
        app.confirmation.unwrap().kind,
        ConfirmationKind::DiscardDraft
    );
    assert!(!app.update(Event::Paste("must not enter modal draft".into())));
    key(&mut app, KeyCode::Enter); // Neutral Keep.
    assert_eq!(app.input, "committed\n한글");
    assert!(app.editing);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.goal, "committed\n한글");
    assert_eq!(app.overlay, Some(Overlay::Forms));
    assert_eq!(app.selected, 7);
    assert_eq!(app.focus, Pane::Inspector);
    app.command("goal");
    app.update(Event::Paste("discard only this".into()));
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Enter);
    assert!(!app.editing);
    assert_eq!(app.input, "");
    assert_eq!(app.goal, "committed\n한글");
    assert_eq!(app.overlay, Some(Overlay::Forms));
    assert_eq!(app.selected, 7);
    assert_eq!(app.focus, Pane::Inspector);
}

#[test]
fn repeat_enter_cannot_approve_or_apply_a_draft() {
    let mut app = app();
    app.command("goal");
    app.update(Event::Paste("keep".into()));
    let repeat = KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat);
    assert!(!app.update(Event::Key(repeat)));
    assert_eq!(app.goal, "");
    key(&mut app, KeyCode::Esc);
    assert!(!app.update(Event::Key(repeat)));
    assert!(!app.confirmation.unwrap().destructive);
    key(&mut app, KeyCode::Right);
    assert!(!app.update(Event::Key(repeat)));
    assert_eq!(app.input, "keep");
    key(&mut app, KeyCode::Enter);
    assert!(!app.editing);
    assert_eq!(app.input, "");
}

#[test]
fn quit_protects_goal_and_dirty_draft_even_below_minimum_size() {
    for editing in [false, true] {
        let mut app = app();
        app.goal = "session data".into();
        app.changed_task();
        if editing {
            app.command("goal");
            app.update(Event::Paste("\n未保存".into()));
        }
        let input = app.input.clone();
        let cursor = app.cursor;
        app.update(Event::Resize(1, 1));
        if editing {
            modified_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        } else {
            app.command("quit");
        }
        assert!(!app.quit);
        assert_eq!(
            app.confirmation,
            Some(Confirmation {
                kind: ConfirmationKind::Quit,
                destructive: false
            })
        );
        key(&mut app, KeyCode::Enter);
        assert!(!app.quit);
        assert_eq!(app.input, input);
        assert_eq!(app.cursor, cursor);
        modified_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.input, input);
        app.update(Event::Resize(120, 36));
        assert_eq!(app.editing, editing);
        modified_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Enter);
        assert!(app.quit_after_save);
        assert!(!app.quit);
        app.store_event(store::Event::Saved(committed(&app)));
        assert!(app.quit);
    }
    let mut app = app();
    app.command("goal");
    app.update(Event::Paste("first unsaved goal".into()));
    modified_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(app.confirmation.unwrap().kind, ConfirmationKind::Quit);
    assert!(!app.quit);
}

#[test]
fn popup_quit_stay_restores_exact_nested_selection() {
    let mut app = app();
    app.goal = "retained goal".into();
    app.changed_task();
    app.command("forms");
    app.selected = 5;
    modified_key(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::End);
    app.command("quit");
    assert_eq!(app.confirmation.unwrap().kind, ConfirmationKind::Quit);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.overlay, Some(Overlay::Forms));
    assert_eq!(app.selected, 5);
    assert!(!app.quit);
}

#[test]
fn vertical_cursor_uses_terminal_cells_and_keeps_column_across_short_lines() {
    let mut app = app();
    app.command("goal");
    app.update(Event::Paste("abcd\n界界\nx\nabcd".into()));
    key(&mut app, KeyCode::Home);
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Right);
    assert_eq!(&app.input[..app.cursor], "abcd\n界");
    key(&mut app, KeyCode::Up);
    assert_eq!(&app.input[..app.cursor], "ab");
    key(&mut app, KeyCode::Down);
    assert_eq!(&app.input[..app.cursor], "abcd\n界");
    key(&mut app, KeyCode::Down);
    assert_eq!(&app.input[..app.cursor], "abcd\n界界\nx");
    key(&mut app, KeyCode::Down);
    assert_eq!(&app.input[app.cursor..], "cd");
}

#[test]
fn paste_limit_warning_is_visible_without_text_change_and_survives_reflow() {
    let mut app = app();
    app.command("goal");
    app.update(Event::Paste("a".repeat(MAX_GOAL_BYTES)));
    assert!(!app.input_truncated);
    assert!(app.update(Event::Paste("界\r\nrest".into())));
    assert!(app.input_truncated);
    assert_eq!(app.input, "a".repeat(MAX_GOAL_BYTES));
    app.update(Event::Resize(60, 18));
    assert!(app.input_truncated);
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Backspace);
    assert!(app.input_truncated);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.goal.len(), MAX_GOAL_BYTES - 1);
    assert!(!app.input_truncated);
}

#[test]
fn rendered_scroll_bounds_prevent_overscroll_and_clamp_after_reflow() {
    let mut app = app();
    assert!(!key(&mut app, KeyCode::Down));
    app.set_scroll_viewport(ScrollSurface::Workspace, 2, 10);
    key(&mut app, KeyCode::PageDown);
    assert_eq!(app.scroll(), 2);
    assert!(!key(&mut app, KeyCode::Down));
    app.set_scroll_viewport(ScrollSurface::Workspace, 1, 10);
    assert_eq!(app.scroll(), 1);
    app.set_scroll_viewport(ScrollSurface::Help, 3, 10);
    app.command("help");
    key(&mut app, KeyCode::End);
    assert_eq!(app.scroll(), 3);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.scroll(), 1);
    app.set_scroll_viewport(ScrollSurface::Workspace, 0, 10);
    assert_eq!(app.scroll(), 0);
}

#[test]
fn repeated_printable_characters_remain_text_while_editing() {
    let mut app = app();
    app.command("goal");
    for character in ['q', 'c', '界'] {
        let repeat = KeyEvent::new_with_kind(
            KeyCode::Char(character),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        );
        assert!(app.update(Event::Key(repeat)));
    }
    assert_eq!(app.input, "qc界");
    assert!(app.confirmation.is_none());
    assert!(!app.quit);
}

#[test]
fn key_release_events_never_edit_or_navigate() {
    let mut app = app();
    app.command("goal");
    let release = KeyEvent::new_with_kind(
        KeyCode::Char('x'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    );
    assert!(!app.update(Event::Key(release)));
    assert_eq!(app.input, "");
}

#[test]
fn late_acknowledgement_cannot_save_newer_edit_or_quit_early() {
    let mut app = app();
    app.command("goal");
    app.update(Event::Paste("first".into()));
    key(&mut app, KeyCode::Enter);
    let first = app.revision;
    let first_snapshot = committed(&app);
    app.command("goal");
    app.update(Event::Paste(" newer".into()));
    key(&mut app, KeyCode::Enter);
    app.command("quit");
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert!(app.quit_after_save);
    app.store_event(store::Event::Failed {
        revision: Some(first),
        message: "older failure".into(),
    });
    assert_eq!(app.storage, StorageState::Saving);
    assert!(app.quit_after_save);
    app.store_event(store::Event::Saved(first_snapshot));
    assert_eq!(app.storage, StorageState::Saving);
    assert!(!app.quit);
    app.store_event(store::Event::Saved(committed(&app)));
    assert!(app.quit);
    assert_eq!(app.goal, "first newer");
}

#[test]
fn failed_save_retains_goal_and_requires_explicit_unsaved_exit() {
    let mut app = app();
    app.command("goal");
    app.update(Event::Paste("recover me".into()));
    key(&mut app, KeyCode::Enter);
    app.store_event(store::Event::Failed {
        revision: Some(app.revision),
        message: "disk full".into(),
    });
    app.command("quit");
    key(&mut app, KeyCode::Enter);
    assert!(!app.quit);
    assert_eq!(app.goal, "recover me");
    app.command("storage-retry");
    assert!(app.retry_storage);
    app.store_event(store::Event::Loaded(store::Project {
        tasks: Vec::new(),
        partners: vec![store::Partner::default()],
        active_id: None,
        usage_bytes: 0,
    }));
    assert!(app.save_requested);
    assert_eq!(app.goal, "recover me");
}

#[test]
fn startup_load_retains_new_edit_without_overwriting_existing_task() {
    let mut app = App::new("/workspace".into(), 120, 36);
    app.update(Event::Paste("new edit".into()));
    key(&mut app, KeyCode::Enter);
    let mut existing = store::Snapshot::empty("/workspace".into());
    existing.id = 7;
    existing.goal = "old goal".into();
    existing.form = 8;
    existing.revision = 40;
    app.store_event(store::Event::Loaded(store::Project {
        tasks: vec![existing],
        partners: vec![store::Partner::default()],
        active_id: Some(7),
        usage_bytes: 0,
    }));
    assert_eq!(app.goal, "new edit");
    assert_eq!(app.task_id, 0);
    assert!(app.save_requested);
    assert_eq!(app.tasks[0].goal, "old goal");
}

#[test]
fn deleting_another_task_to_free_quota_keeps_the_failed_current_draft() {
    let mut app = app();
    let mut old = store::Snapshot::empty("/workspace".into());
    old.id = 7;
    old.goal = "eligible old task".into();
    app.tasks.push(old);
    app.goal = "new unsaved goal".into();
    app.storage = StorageState::Failed("quota reached".into());
    app.command("goal");
    app.update(Event::Paste(" retained draft".into()));
    modified_key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    app.command("tasks");
    app.command("delete");
    assert!(matches!(
        app.management.take(),
        Some(store::Management::PreviewDelete { .. })
    ));
    app.store_event(store::Event::DeletePreview {
        targets: vec![store::DeletionTarget {
            task: app.tasks[0].clone(),
            excerpts: Vec::new(),
            runs: Vec::new(),
            provider_records: Vec::new(),
            memories: Vec::new(),
        }],
        expired_before: None,
    });
    key(&mut app, KeyCode::Enter);
    assert!(app.management.is_none());
    assert_eq!(app.tasks[0].id, 7);
    app.command("delete");
    assert!(matches!(
        app.management.take(),
        Some(store::Management::PreviewDelete { .. })
    ));
    app.store_event(store::Event::DeletePreview {
        targets: vec![store::DeletionTarget {
            task: app.tasks[0].clone(),
            excerpts: Vec::new(),
            runs: Vec::new(),
            provider_records: Vec::new(),
            memories: Vec::new(),
        }],
        expired_before: None,
    });
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert!(
        matches!(app.management.take(), Some(store::Management::Delete { ids, expired_before: None }) if ids == [7])
    );
    app.store_event(store::Event::Managed {
        project: store::Project {
            tasks: Vec::new(),
            partners: vec![store::Partner::default()],
            active_id: None,
            usage_bytes: 8192,
        },
        affected: vec![7],
        warning: None,
    });
    assert_eq!(app.task_id, 0);
    assert_eq!(app.goal, "new unsaved goal");
    assert_eq!(app.drafts[0].text, "new unsaved goal retained draft");
    assert!(matches!(app.storage, StorageState::Failed(_)));
    assert!(app.save_requested && !app.transitioning);
    app.command("storage-retry");
    assert!(app.retry_storage);
}

#[test]
fn archived_view_restores_instead_of_resuming_hidden_tasks() {
    let mut app = app();
    let mut archived = store::Snapshot::empty("/workspace".into());
    archived.id = 8;
    archived.archived = true;
    archived.archived_at = 1;
    app.tasks.push(archived);
    app.command("tasks");
    assert_eq!(app.visible_tasks().count(), 0);
    app.command("archived");
    key(&mut app, KeyCode::Enter);
    assert!(app.switch_task.is_none());
    app.command("restore");
    assert!(matches!(
        app.management,
        Some(store::Management::Archive {
            id: 8,
            archived: false
        })
    ));
    assert_eq!(app.task_id, 0);
}

#[test]
fn party_roster_lists_every_partner_with_derived_idle_status() {
    let mut app = app();
    let partners = vec![
        store::Partner {
            id: 1,
            nickname: Some("Bolt".into()),
            form: 2,
            met_at: 7,
            met_derived: false,
            created_project: "/workspace".into(),
        },
        store::Partner {
            id: 2,
            nickname: None,
            form: 0,
            met_at: 9,
            met_derived: false,
            created_project: "/workspace".into(),
        },
    ];
    app.store_event(store::Event::Loaded(store::Project {
        tasks: Vec::new(),
        partners,
        active_id: None,
        usage_bytes: 0,
    }));
    let roster = app.partners();
    assert_eq!(roster.len(), 2);
    assert_eq!(App::partner_label(&roster[0]), "Bolt");
    assert_eq!(App::partner_label(&roster[1]), "Eevee #2");
    for partner in &roster {
        // Nothing is running: every partner derives idle without a request.
        assert_eq!(app.partner_status(partner), party::PartnerStatus::Idle);
        assert_eq!(app.partner_task(partner), "-");
    }
    // The roster opens and Escape restores the previous surface and focus.
    app.editing = false;
    app.command("party");
    assert_eq!(app.overlay, Some(Overlay::Party));
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.overlay, None);
}

#[test]
fn partner_status_tracks_live_request_and_approval_states() {
    let mut app = app();
    app.task_id = 7;
    app.goal = "task".into();
    app.planning.reset(7);
    let lead = app.lead().clone();
    // A provider preview awaiting the user's review is a waiting status.
    app.planning.preview = Some(crate::providers::Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: true,
        generation: 1,
        task_id: 7,
        thread_id: None,
        trust: Vec::new(),
        payload: "{}".into(),
        blockers: Vec::new(),
    });
    assert_eq!(app.partner_status(&lead), party::PartnerStatus::Waiting);
    // Another partner owns no request slot yet, so it stays derived idle.
    let other = store::Partner {
        id: 2,
        ..store::Partner::default()
    };
    assert_eq!(app.partner_status(&other), party::PartnerStatus::Idle);
}

#[test]
fn roster_add_queues_one_request_and_applies_the_committed_roster() {
    let mut app = app();
    app.editing = false;
    app.command("party");
    assert_eq!(app.overlay, Some(Overlay::Party));
    key(&mut app, KeyCode::Char('a'));
    assert_eq!(app.party_request, Some(store::PartyRequest::Add));
    // A second key press while the store write is in flight adds nothing more.
    app.party_inflight = true;
    key(&mut app, KeyCode::Char('a'));
    assert!(app.notice.contains("already joining"));
    let joined = store::Partner {
        id: 2,
        nickname: None,
        form: 0,
        met_at: 1,
        met_derived: false,
        created_project: "/workspace".into(),
    };
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner::default(),
        joined,
    ])));
    assert!(!app.party_inflight);
    assert_eq!(app.partners().len(), 2);
    assert!(app.notice.contains("Eevee #2 joined"));
}

#[test]
fn roster_add_is_disabled_with_the_capacity_reason_at_six() {
    let mut app = app();
    for id in 2..=6 {
        app.store_event(store::Event::Party(Ok((1..=id)
            .map(|id| store::Partner {
                id,
                ..store::Partner::default()
            })
            .collect())));
    }
    assert_eq!(app.partners().len(), 6);
    app.editing = false;
    app.command("party");
    assert_eq!(app.overlay, Some(Overlay::Party));
    key(&mut app, KeyCode::Char('a'));
    assert!(app.party_request.is_none());
    assert!(app.notice.contains("at most 6"), "{}", app.notice);
    app.command("add");
    assert!(app.party_request.is_none());
    assert!(app.notice.contains("at most 6"), "{}", app.notice);
}

#[test]
fn roster_reflects_unsaved_form_switch_on_the_lead_row() {
    let mut app = app();
    app.editing = false;
    app.command("jolteon");
    let roster = app.partners();
    assert_eq!(roster[0].form, 2);
    assert_eq!(app.lead().form, 2);
}

fn type_text(app: &mut App, text: &str) {
    for character in text.chars() {
        key(app, KeyCode::Char(character));
    }
}

#[test]
fn roster_enter_fronts_the_selected_partner_and_the_picker_follows() {
    let mut app = app();
    app.editing = false;
    // Lead switches to Jolteon first so the rows carry distinct live forms.
    app.command("jolteon");
    assert_eq!(app.lead().form, 2);
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner {
            id: 1,
            form: 2,
            ..store::Partner::default()
        },
        store::Partner {
            id: 2,
            nickname: Some("Bolt".into()),
            form: 1,
            ..store::Partner::default()
        },
    ])));
    app.command("party");
    assert_eq!(app.overlay, Some(Overlay::Party));
    assert_eq!(app.selected, 0, "the roster opens on the current lead row");
    key(&mut app, KeyCode::Down);
    assert_eq!(app.selected, 1);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.lead().id, 2);
    assert_eq!(app.lead().nickname.as_deref(), Some("Bolt"));
    assert_eq!(app.form(), 1, "the new lead's own form is active");
    assert_eq!(app.lead_label(), "Bolt (Vaporeon)");
    // The picker follows the lead.
    app.command("forms");
    assert_eq!(app.selected, 1);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.form(), 1);
    // Switching back re-arms the same path from the roster; it reopens on the
    // current lead row, so move back to the first partner first.
    app.command("party");
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.lead().id, 1, "notice={}", app.notice);
    assert_eq!(app.form(), 2);
    assert_eq!(app.lead_label(), "Eevee #1 (Jolteon)");
}

#[test]
fn slash_lead_resolves_nicknames_and_eevee_numbers() {
    let mut app = app();
    app.editing = false;
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner::default(),
        store::Partner {
            id: 2,
            nickname: Some("Bolt".into()),
            form: 1,
            ..store::Partner::default()
        },
        store::Partner {
            id: 3,
            form: 4,
            ..store::Partner::default()
        },
    ])));
    // A typed argument fronts the named partner without touching the roster.
    key(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "lead bolt");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.lead().id, 2, "notice={}", app.notice);
    assert_eq!(app.form(), 1);
    key(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "lead Eevee #3");
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.lead().id,
        3,
        "notice={} overlay={:?}",
        app.notice,
        app.overlay
    );
    assert_eq!(app.form(), 4);
    key(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "lead nobody");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.lead().id, 3);
    assert!(app.notice.contains("No partner named"), "{}", app.notice);
    // Pasted text never dispatches the command.
    let revision = app.revision;
    app.update(Event::Paste("/lead Bolt".into()));
    assert_eq!(app.lead().id, 3);
    assert_eq!(app.revision, revision);
    // Even inside the editor it lands as literal text, not a dispatch.
    app.command("goal");
    app.update(Event::Paste("/lead Bolt".into()));
    assert_eq!(app.input, "/lead Bolt");
    assert_eq!(app.lead().id, 3);
}

#[test]
fn lead_switch_persists_through_save_and_restart() {
    let mut app = app();
    app.editing = false;
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner::default(),
        store::Partner {
            id: 2,
            form: 1,
            ..store::Partner::default()
        },
    ])));
    app.command("lead");
    assert_eq!(app.overlay, Some(Overlay::Party));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.lead().id, 2);
    let mut committed = committed(&app);
    assert_eq!(committed.lead_partner, 2);
    assert_eq!(committed.form, 1);
    committed.id = 7;
    let mut resumed = App::new("/workspace".into(), 120, 36);
    resumed.store_event(store::Event::Loaded(store::Project {
        tasks: vec![committed],
        partners: vec![
            store::Partner::default(),
            store::Partner {
                id: 2,
                form: 1,
                met_at: 99,
                ..store::Partner::default()
            },
        ],
        active_id: Some(7),
        usage_bytes: 0,
    }));
    assert_eq!(resumed.lead().id, 2);
    assert_eq!(resumed.lead().met_at, 99);
    assert_eq!(resumed.form(), 1);
    assert_eq!(resumed.lead_label(), "Eevee #2 (Vaporeon)");
}

#[test]
fn lead_switch_keeps_live_work_and_its_approval_on_the_owning_partner() {
    let mut app = app();
    app.editing = false;
    app.task_id = 7;
    app.goal = "task".into();
    app.planning.reset(7);
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner {
            id: 1,
            nickname: Some("Bolt".into()),
            form: 2,
            ..store::Partner::default()
        },
        store::Partner {
            id: 2,
            nickname: Some("Mizu".into()),
            form: 1,
            ..store::Partner::default()
        },
    ])));
    app.request_lead(2);
    app.request_lead(1);
    assert_eq!(app.lead_label(), "Bolt (Jolteon)");
    let mut task = committed(&app);
    task.id = 7;
    app.planning.command("new", true, &task, &[], None);
    app.planning.pending.take();
    app.planning.event(crate::providers::Event::Preview(
        crate::providers::Preview {
            arrival: crate::providers::approval_arrival(),
            destination: None,
            requires_connection_review: true,
            generation: 11,
            task_id: 7,
            thread_id: None,
            trust: Vec::new(),
            payload: "{}".into(),
            blockers: Vec::new(),
        },
    ));
    let bolt = app.lead().clone();
    assert_eq!(app.partner_status(&bolt), party::PartnerStatus::Waiting);
    // Switching the lead away and back never moves the request or approval.
    app.request_lead(2);
    assert_eq!(app.lead().id, 2, "{}", app.notice);
    let mizu = app.lead().clone();
    assert_eq!(app.partner_status(&bolt), party::PartnerStatus::Waiting);
    assert_eq!(app.partner_status(&mizu), party::PartnerStatus::Idle);
    assert_eq!(app.partner_task(&bolt), "task");
    assert_eq!(
        app.planning_state(1)
            .and_then(planning::State::owner)
            .map(|owner| owner.partner),
        Some(1)
    );
    assert!(app.planning_state(1).unwrap().preview.is_some());
    assert!(
        app.planning.preview.is_none(),
        "fronting another owner does not transfer consent"
    );
    // The second partner can independently enter the save/send barrier.
    app.open_editor(store::EditorField::Request);
    type_text(&mut app, "second request");
    key(&mut app, KeyCode::Enter);
    assert!(app.pending_request.is_some());
    assert_eq!(app.pending_request_owner, Some(2));
    assert_eq!(
        app.input, "second request",
        "the draft is kept until dispatch"
    );
    assert!(app.planning_state(1).unwrap().preview.is_some());
    app.request_lead(1);
    assert_eq!(app.lead().id, 1);
    assert_eq!(app.partner_status(&bolt), party::PartnerStatus::Waiting);
    assert_eq!(app.planning.owner().map(|owner| owner.partner), Some(1));
    // A roster write in flight still blocks the switch.
    app.party_inflight = true;
    app.request_lead(2);
    assert_eq!(app.lead().id, 1);
    assert!(
        app.notice.contains("in-flight party change"),
        "{}",
        app.notice
    );
}

#[test]
fn failed_request_returns_to_its_senders_draft_not_the_current_lead() {
    let mut app = app();
    app.editing = false;
    app.task_id = 7;
    app.goal = "task".into();
    app.planning.reset(7);
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner::default(),
        store::Partner {
            id: 2,
            nickname: Some("Mizu".into()),
            form: 1,
            ..store::Partner::default()
        },
    ])));
    let mut task = committed(&app);
    task.id = 7;
    app.planning.command("new", true, &task, &[], None);
    app.planning.submitted_request = "original request".into();
    app.request_lead(2);
    app.open_editor(store::EditorField::Request);
    type_text(&mut app, "mizu draft");
    app.planning_state_mut(1).unwrap().failed_request = true;
    assert!(app.poll_providers(&mut std::collections::BTreeMap::new()).0);
    assert_eq!(app.input, "mizu draft", "the lead's composer is untouched");
    let restored = &app.partner_drafts[&1].drafts;
    assert!(restored.iter().any(|draft| {
        draft.field == store::EditorField::Request && draft.text == "original request"
    }));
    assert!(
        app.notice.contains("Eevee #1's request was restored"),
        "{}",
        app.notice
    );
}

#[test]
fn lead_switch_retains_each_partners_previous_form_toggle() {
    let mut app = app();
    app.editing = false;
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner::default(),
        store::Partner {
            id: 2,
            form: 3,
            ..store::Partner::default()
        },
    ])));
    app.command("jolteon"); // lead 1 records previous form Eevee
    app.request_lead(2);
    assert_eq!(app.form(), 3);
    key(&mut app, KeyCode::Backspace);
    assert_eq!(
        app.form(),
        3,
        "the old lead's previous form does not carry over"
    );
    assert!(app.notice.contains("No previous form"), "{}", app.notice);
    app.request_lead(1);
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.form(), 0, "partner 1 retains its own previous form");
}

#[test]
fn partner_cjk_drafts_and_carets_survive_lead_switch_and_restart() {
    let mut app = app();
    let party = vec![
        store::Partner::default(),
        store::Partner {
            id: 2,
            ..store::Partner::default()
        },
    ];
    app.store_event(store::Event::Party(Ok(party.clone())));
    app.open_editor(store::EditorField::Goal);
    app.update(Event::Paste("첫째 日本語 e\u{301} 끝".into()));
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Left);
    let first = (app.input.clone(), app.cursor);
    app.request_lead(2);
    assert!(app.input.is_empty());
    app.open_editor(store::EditorField::Goal);
    app.update(Event::Paste("둘째 中文 /jolteon".into()));
    key(&mut app, KeyCode::Home);
    key(&mut app, KeyCode::Right);
    let second = (app.input.clone(), app.cursor);
    app.request_lead(1);
    assert_eq!((&app.input, app.cursor), (&first.0, first.1));
    let saved = committed(&app);
    let mut resumed = App::new("/workspace".into(), 60, 18);
    resumed.store_event(store::Event::Loaded(store::Project {
        active_id: Some(saved.id),
        tasks: vec![saved],
        partners: party,
        usage_bytes: 0,
    }));
    assert_eq!((&resumed.input, resumed.cursor), (&first.0, first.1));
    resumed.request_lead(2);
    assert_eq!((&resumed.input, resumed.cursor), (&second.0, second.1));
    assert_eq!(resumed.form(), 0, "pasted slash text never changes form");
}

#[test]
fn dirty_draft_discard_only_affects_its_own_partner() {
    let mut app = app();
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner::default(),
        store::Partner {
            id: 2,
            ..store::Partner::default()
        },
    ])));
    app.open_editor(store::EditorField::Goal);
    app.update(Event::Paste("保留 첫째".into()));
    app.request_lead(2);
    app.open_editor(store::EditorField::Goal);
    key(&mut app, KeyCode::Esc);
    assert!(
        app.confirmation.is_none(),
        "partner 1's draft does not dirty partner 2"
    );
    app.open_editor(store::EditorField::Goal);
    app.update(Event::Paste("버릴 둘째".into()));
    key(&mut app, KeyCode::Esc);
    assert_eq!(
        app.confirmation.as_ref().unwrap().kind,
        ConfirmationKind::DiscardDraft
    );
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    app.request_lead(1);
    assert_eq!(app.input, "保留 첫째");
    app.request_lead(2);
    app.open_editor(store::EditorField::Goal);
    assert!(app.input.is_empty());
}

fn saved_task(app: &mut App) -> i64 {
    app.command("goal");
    app.update(Event::Paste("catalogue goal".into()));
    key(app, KeyCode::Enter);
    app.store_event(store::Event::Saved(committed(app)));
    app.task_id
}

fn catalogue() -> crate::providers::ModelCatalogue {
    crate::providers::ModelCatalogue {
        models: vec![crate::providers::ModelOption {
            id: "advertised-model".into(),
            name: "Advertised model".into(),
            description: "From provider metadata".into(),
            efforts: vec!["high".into()],
            default_effort: Some("high".into()),
        }],
        current: Some("advertised-model".into()),
        source: "Official metadata".into(),
        effort_note: None,
    }
}

/// Answer the automatic catalogue preview and run it to completion.
fn finish_catalogue(app: &mut App, task_id: i64, blockers: Vec<String>) {
    let Some(crate::providers::Request::Preview(input)) = app.planning.pending.take() else {
        panic!("catalogue preview was not requested");
    };
    assert!(input.inspect_models && input.resume.is_none());
    app.planning.event(crate::providers::Event::Preview(
        crate::providers::Preview {
            arrival: crate::providers::approval_arrival(),
            destination: None,
            requires_connection_review: false,
            generation: 3,
            task_id,
            thread_id: None,
            trust: Vec::new(),
            payload: "{}".into(),
            blockers: blockers.clone(),
        },
    ));
    if blockers.is_empty() {
        assert!(
            matches!(
                app.planning.pending.take(),
                Some(crate::providers::Request::Run { generation: 3 })
            ),
            "the metadata-only preview runs without a review"
        );
        app.planning.event(crate::providers::Event::Models {
            generation: 3,
            task_id,
            catalogue: catalogue(),
        });
    } else {
        assert!(matches!(
            app.planning.pending.take(),
            Some(crate::providers::Request::Cancel { generation: 3 })
        ));
    }
    app.planning.event(crate::providers::Event::Finished {
        generation: 3,
        task_id,
        outcome: if blockers.is_empty() {
            crate::providers::Outcome::Completed
        } else {
            crate::providers::Outcome::Cancelled
        },
        message: "done".into(),
    });
}

#[test]
fn connecting_the_task_provider_loads_models_without_a_review() {
    let mut app = app();
    // Connecting on a fresh home must not require saving a request first.
    app.connection_returned(
        app.agents.active,
        crate::providers::auth::Outcome::Authenticated,
    );
    assert!(app.drive_catalogue());
    finish_catalogue(&mut app, 0, Vec::new());
    assert!(app.planning.catalogues.contains_key(&app.planning.agent));
    assert!(
        app.planning.picker.is_none(),
        "a connect-time fetch opens nothing"
    );
    assert!(!app.planning.busy());
    assert!(!app.drive_catalogue(), "one fetch per connection");
    // A failed login never fetches.
    app.planning.catalogues.clear();
    app.connection_returned(app.agents.active, crate::providers::auth::Outcome::Failed);
    assert!(!app.drive_catalogue());
}

#[test]
fn empty_model_picker_fetches_then_reopens_and_blockers_are_reported() {
    let mut app = app();
    let task_id = saved_task(&mut app);
    app.editing = false;
    app.command("model");
    assert!(app.drive_catalogue());
    finish_catalogue(&mut app, task_id, Vec::new());
    assert!(app.planning.picker == Some(planning::Picker::Model));
    assert!(app.planning.picker_current().is_some());
    // A blocked preview is cancelled with its reason instead of a review.
    app.planning.picker = None;
    app.planning.catalogues.clear();
    app.planning.want_catalogue();
    assert!(app.drive_catalogue());
    finish_catalogue(
        &mut app,
        task_id,
        vec!["provider executable missing".into()],
    );
    assert!(app.planning.notice.contains("provider executable missing"));
    assert!(app.planning.catalogues.is_empty() && !app.planning.busy());
}

#[test]
fn fresh_model_picker_loads_without_saving_or_consuming_the_request_draft() {
    let mut app = request_app();
    app.update(Event::Paste("아직 보내지 않은 요청".into()));
    let draft = app.input.clone();
    let cursor = app.cursor;
    app.command("model");
    assert!(app.drive_catalogue());
    let Some(crate::providers::Request::Preview(input)) = app.planning.pending.as_ref() else {
        panic!("metadata inspection must start on a fresh home");
    };
    assert_eq!(input.task.id, 0);
    assert!(input.task.goal.is_empty());
    assert!(input.excerpts.is_empty() && input.report.is_none());
    app.settle_planning();
    assert_eq!(app.overlay, Some(Overlay::Planning));
    finish_catalogue(&mut app, 0, Vec::new());
    app.settle_planning();
    assert_eq!(app.overlay, Some(Overlay::Planning));
    assert!(app.planning.picker == Some(planning::Picker::Model));
    assert!(app.planning.picker_current().is_some());
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none() && app.editing);
    assert_eq!(app.input, draft);
    assert_eq!(app.cursor, cursor);
    assert_eq!(app.task_id, 0);
    assert!(!app.planning.busy());
}

#[test]
fn model_search_and_custom_ids_own_slashes_and_paste_without_opening_commands() {
    let mut app = request_app();
    app.command("model");
    key(&mut app, KeyCode::Char('/'));
    assert!(app.command_popup.is_none());
    assert_eq!(app.planning.picker_query.text, "/");
    app.update(Event::Paste("provider/model".into()));
    assert_eq!(app.planning.picker_query.text, "/provider/model");
    assert!(app.planning.selection.model.is_none());
    modified_key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Enter);
    assert!(app.planning.custom_model.is_some());
    app.update(Event::Paste("/quit".into()));
    assert!(!app.quit && app.command_popup.is_none());
    assert_eq!(app.planning.custom_model.as_ref().unwrap().text, "/quit");
    assert!(app.planning.selection.model.is_none() && app.planning.pending.is_none());
}

#[test]
fn fresh_provider_selection_preserves_unsaved_request_without_task_or_network() {
    let mut app = request_app();
    app.update(Event::Paste("Unsaved future request".into()));
    let draft = app.input.clone();
    let cursor = app.cursor;
    assert!(app.command_disabled_reason("provider-openai").is_none());
    app.command("provider-openai");
    assert_eq!(app.planning.agent, crate::providers::Agent::OpenAI);
    assert_eq!(app.task_id, 0);
    assert!(app.planning.pending.is_none() && !app.planning.busy());
    assert_eq!(app.input, draft);
    assert_eq!(app.cursor, cursor);
}

#[test]
fn codex_method_switch_changes_actual_route_and_returns_to_the_invoking_editor() {
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = request_app();
        app.width = width;
        app.height = height;
        app.update(Event::Paste("keep this method-switch draft".into()));
        app.open_overlay(Overlay::Agents);
        app.agents.use_for_task = Some(crate::providers::Agent::ChatGPT.index());
        app.use_selected_provider();
        assert_eq!(app.planning.agent, crate::providers::Agent::ChatGPT);
        assert!(app.overlay.is_none() && app.editing);
        assert_eq!(app.input, "keep this method-switch draft");
        assert!(app.planning.pending.is_none());
    }
}

#[test]
fn provider_switch_review_cancellation_restores_the_invoking_editor_directly() {
    for code in [KeyCode::Enter, KeyCode::Esc] {
        let mut app = request_app();
        app.goal = "Committed task".into();
        app.update(Event::Paste("keep the original suffix".into()));
        app.cursor = 5;
        modified_key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        save_request(&mut app);
        app.planning.reset(app.task_id);
        app.planning.thread_id = Some("owned-session".into());
        let owner = app.planning.agent;
        let next = if owner == crate::providers::Agent::Codex {
            crate::providers::Agent::OpenAI
        } else {
            crate::providers::Agent::Codex
        };
        app.command("provider");
        app.agents.use_for_task = Some(next.index());
        app.use_selected_provider();
        assert_eq!(app.overlay, Some(Overlay::Planning));
        assert_eq!(app.planning.switch_to, Some(next));
        key(&mut app, code);
        assert!(app.overlay.is_none() && app.editing);
        assert_eq!(
            (app.input.as_str(), app.cursor),
            ("keep the original suffix", 5)
        );
        assert_eq!(app.planning.agent, owner);
        assert_eq!(app.planning.thread_id.as_deref(), Some("owned-session"));
        assert!(app.planning.pending.is_none());
    }
}

#[test]
fn direct_custom_model_review_remains_owned_until_explicit_cancellation() {
    let mut app = request_app();
    app.goal = "Committed session".into();
    app.update(Event::Paste("retained future request".into()));
    modified_key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    save_request(&mut app);
    app.planning.reset(app.task_id);
    app.planning.thread_id = Some("owned-session".into());
    key(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "model vendor/Unlisted.Exact");
    key(&mut app, KeyCode::Enter);
    assert!(app.planning.selection_change.is_some());
    assert!(app.planning.picker.is_none());
    app.settle_planning();
    assert_eq!(app.overlay, Some(Overlay::Planning));
    assert!(app.planning.busy());
    app.command("response");
    assert_eq!(app.overlay, Some(Overlay::Planning));
    assert_eq!(app.planning.thread_id.as_deref(), Some("owned-session"));
    assert!(app.planning.selection.model.is_none());
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none() && app.editing);
    assert_eq!(app.input, "retained future request");
    assert!(app.planning.selection_change.is_none());
    assert!(app.planning.pending.is_none());
}

fn request_app() -> App {
    let mut app = App::new("/workspace".into(), 120, 36);
    app.storage = StorageState::Saved;
    app.agents.pending = None;
    app.agent_event(crate::providers::agent::Event::Discovered(Vec::new()));
    app
}

fn save_request(app: &mut App) -> store::Snapshot {
    let snapshot = committed(app);
    app.save_submitted();
    app.store_event(store::Event::Saved(snapshot.clone()));
    snapshot
}

fn acknowledge_work(app: &mut App) {
    app.planning.settle_work();
    if let Some(request) = app.planning.work_pending.take() {
        let claims = match &request {
            store::WorkRequest::Acquire { claim, .. } => vec![claim.clone()],
            store::WorkRequest::Release(_) => Vec::new(),
        };
        app.store_event(store::Event::Work {
            request,
            result: Ok(claims),
        });
    }
}

fn request_preview(app: &mut App) {
    key(app, KeyCode::Enter);
    save_request(app);
    assert!(app.settle_planning());
    acknowledge_work(app);
    assert!(matches!(
        app.planning.pending.take(),
        Some(crate::providers::Request::Preview(_))
    ));
    app.planning.event(crate::providers::Event::Preview(
        crate::providers::Preview {
            arrival: crate::providers::approval_arrival(),
            destination: None,
            requires_connection_review: true,
            generation: 12,
            task_id: app.task_id,
            thread_id: None,
            trust: vec!["Reviewed connection".into()],
            payload: "{}".into(),
            blockers: vec![],
        },
    ));
    app.settle_planning();
    app.command("approvals");
}

#[test]
fn only_deliberate_unmodified_enter_can_submit_a_request() {
    let mut app = request_app();
    app.update(Event::Paste("retained request".into()));
    for event in [
        KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
    ] {
        app.update(Event::Key(event));
        assert!(app.pending_request.is_none() && app.planning.pending.is_none());
        assert_eq!(app.input, "retained request");
    }
}

#[test]
fn composer_enter_dispatches_the_committed_request_once_without_a_popup() {
    let mut app = request_app();
    app.update(Event::Paste("Send exactly this request".into()));
    key(&mut app, KeyCode::Enter);
    save_request(&mut app);
    app.settle_planning();
    acknowledge_work(&mut app);
    let Some(crate::providers::Request::Preview(input)) = app.planning.pending.take() else {
        panic!("committed request was not prepared");
    };
    assert_eq!(input.task.request_text(), "Send exactly this request");
    assert!(app.overlay.is_none());
    app.planning.event(crate::providers::Event::Preview(
        crate::providers::Preview {
            arrival: crate::providers::approval_arrival(),
            generation: 91,
            task_id: app.task_id,
            thread_id: None,
            destination: Some("https://example.test/v1/responses".into()),
            requires_connection_review: false,
            trust: Vec::new(),
            payload: "{}".into(),
            blockers: Vec::new(),
        },
    ));
    assert!(app.settle_planning());
    assert!(app.overlay.is_none() && app.editing && app.input.is_empty());
    assert_eq!(
        &*app.planning.submitted_request,
        "Send exactly this request"
    );
    assert!(matches!(
        app.planning.pending.take(),
        Some(crate::providers::Request::Run { generation: 91 })
    ));
    app.settle_planning();
    assert!(app.planning.pending.is_none());
    app.update(Event::Paste("unsent follow-up".into()));
    assert_eq!(app.input, "unsent follow-up");
    assert!(
        !app.planning.holds_committed_input(),
        "the request owns private input rather than freezing draft saves"
    );
}

#[test]
fn terminal_native_unknown_allows_explicit_followup_and_task_switch_but_cli_unknown_does_not() {
    use crate::providers::{Agent, Event as ProviderEvent, ItemKind, Outcome, Preview, Request};
    for agent in [Agent::Compatible, Agent::Codex] {
        let mut app = request_app();
        app.planning.agent = agent;
        app.planning
            .preference_event(Ok(Some(store::ProviderPreferences {
                provider: agent,
                selections: std::collections::HashMap::new(),
            })));
        app.update(Event::Paste("Interrupted request".into()));
        key(&mut app, KeyCode::Enter);
        save_request(&mut app);
        app.settle_planning();
        acknowledge_work(&mut app);
        assert!(matches!(
            app.planning.pending.take(),
            Some(Request::Preview(_))
        ));
        app.provider_event(
            1,
            ProviderEvent::Preview(Preview {
                arrival: crate::providers::approval_arrival(),
                generation: 91,
                task_id: app.task_id,
                thread_id: None,
                destination: None,
                requires_connection_review: false,
                trust: Vec::new(),
                payload: "{}".into(),
                blockers: Vec::new(),
            }),
        );
        app.settle_planning();
        assert!(matches!(
            app.planning.pending.take(),
            Some(Request::Run { generation: 91 })
        ));
        app.provider_event(
            1,
            ProviderEvent::Item {
                generation: 91,
                task_id: app.task_id,
                thread_id: String::new(),
                turn_id: "partial".into(),
                item_id: "partial".into(),
                kind: ItemKind::Plan,
                text: "Unverified partial answer".into(),
                complete: false,
            },
        );
        let claim = app.planning.work.clone().unwrap();
        app.provider_event(
            1,
            ProviderEvent::Finished {
                generation: 91,
                task_id: app.task_id,
                outcome: Outcome::Unknown,
                message: "Local reader closed; remote outcome unknown".into(),
            },
        );
        assert!(app.planning.outcome_unknown());
        assert!(
            app.planning.owns_work(),
            "release acknowledgement is still required"
        );
        assert_eq!(app.planning.work.as_ref(), Some(&claim));
        acknowledge_work(&mut app);
        assert!(app.planning.work.is_none());
        assert!(app.planning.items.iter().any(|item| !item.complete));
        assert!(app.planning.pending.is_none(), "settlement never resends");

        if !agent.is_direct() {
            assert!(app.planning.unknown_effects() && app.partner_busy(1));
            assert!(app.work_capacity_reason(1).is_some());
            app.request_navigation(execution::Action::SwitchTask(8));
            assert!(app.switch_task.is_none());
            // Selected route is mutable; it cannot change an earlier effect's
            // uncertainty kind or permit its owner to escape the safety gate.
            app.planning.agent = Agent::Compatible;
            assert!(app.planning.unknown_effects() && app.partner_busy(1));
            app.request_navigation(execution::Action::SwitchTask(8));
            assert!(app.switch_task.is_none());
            continue;
        }

        assert!(!app.planning.unknown_effects() && !app.partner_busy(1));
        assert!(app.work_rows().is_empty());
        assert!(app.work_capacity_reason(1).is_none());
        assert!(
            app.cancel_all_summary()
                .contains("provider outcome unknown")
        );
        app.update(Event::Paste("Explicit follow-up only".into()));
        key(&mut app, KeyCode::Enter);
        save_request(&mut app);
        app.settle_planning();
        acknowledge_work(&mut app);
        let Some(Request::Preview(input)) = app.planning.pending.take() else {
            panic!("terminal native uncertainty must not block an explicit follow-up");
        };
        assert_eq!(input.task.request_text(), "Explicit follow-up only");
        assert!(
            input.conversation.is_none(),
            "partial output is not canonical history"
        );
        app.provider_event(
            1,
            ProviderEvent::Preview(Preview {
                arrival: crate::providers::approval_arrival(),
                generation: 92,
                task_id: app.task_id,
                thread_id: None,
                destination: None,
                requires_connection_review: false,
                trust: Vec::new(),
                payload: "{}".into(),
                blockers: Vec::new(),
            }),
        );
        app.settle_planning();
        assert!(matches!(
            app.planning.pending.take(),
            Some(Request::Run { generation: 92 })
        ));
        assert_eq!(app.work_rows()[0].status, "working");
        app.provider_event(
            1,
            ProviderEvent::Finished {
                generation: 92,
                task_id: app.task_id,
                outcome: Outcome::Completed,
                message: "Explicit follow-up complete".into(),
            },
        );
        acknowledge_work(&mut app);
        assert!(
            app.planning.outcome_unknown(),
            "later success does not erase uncertainty"
        );
        save_request(&mut app);
        app.request_navigation(execution::Action::SwitchTask(8));
        assert_eq!(app.switch_task, Some(8), "{}", app.notice);
    }
}

#[test]
fn idle_quit_waits_for_preferences_without_an_empty_cancel_all_review() {
    let mut app = request_app();
    app.planning.preference_event(Ok(None));
    assert!(app.planning.select_model_id("local-fixture-model"));
    assert!(app.planning.preferences_pending());
    assert!(!app.planning.owns_work() && !app.execution.busy());
    app.request_quit();
    assert!(app.cooperation.cancel_review.is_none());
    assert!(app.quit_after_save && !app.quit);
    app.settle_exit();
    assert!(
        !app.quit,
        "preference persistence still needs acknowledgement"
    );
    let Some(store::PreferenceRequest::Save(saved)) = app.planning.preference_request().take()
    else {
        panic!("missing preference save");
    };
    app.planning.preference_started();
    app.store_event(store::Event::Preferences(Ok(Some(saved))));
    app.settle_exit();
    assert!(app.quit);
    assert!(app.preferences_allow_quit(true));
}

#[test]
fn followup_draft_isolated_and_blocked_preparation_never_sends() {
    for changed in [false, true] {
        let mut app = request_app();
        app.update(Event::Paste("original".into()));
        key(&mut app, KeyCode::Enter);
        save_request(&mut app);
        app.settle_planning();
        acknowledge_work(&mut app);
        app.planning.pending.take();
        if changed {
            app.update(Event::Paste(" edited before dispatch".into()));
        }
        app.planning.event(crate::providers::Event::Preview(
            crate::providers::Preview {
                arrival: crate::providers::approval_arrival(),
                generation: 92,
                task_id: app.task_id,
                thread_id: None,
                destination: None,
                requires_connection_review: false,
                trust: Vec::new(),
                payload: "{}".into(),
                blockers: if changed {
                    Vec::new()
                } else {
                    vec!["Credentials unavailable".into()]
                },
            },
        ));
        app.settle_planning();
        if changed {
            assert!(matches!(
                app.planning.pending.take(),
                Some(crate::providers::Request::Run { generation: 92 })
            ));
            assert_eq!(
                app.planning.immutable_task().unwrap().request_text(),
                "original"
            );
            assert_eq!(app.planning.submitted_request.as_ref(), "original");
            assert_eq!(app.input, "original edited before dispatch");
        } else {
            assert!(app.planning.pending.is_none() && app.planning.preview.is_some());
            assert_eq!(
                app.planning.immutable_task().unwrap().request_text(),
                "original"
            );
            app.settle_planning();
            assert!(
                app.planning.pending.is_none(),
                "a blocker never retries or sends"
            );
            app.command("cancel");
            assert!(matches!(
                app.planning.pending.take(),
                Some(crate::providers::Request::Cancel { generation: 92 })
            ));
        }
        assert!(app.input.starts_with("original") && app.overlay.is_none());
        app.planning.event(crate::providers::Event::Finished {
            generation: 92,
            task_id: app.task_id,
            outcome: crate::providers::Outcome::Cancelled,
            message: "Prepared request cancelled".into(),
        });
        assert!(!app.planning.holds_committed_input());
    }
}

#[test]
fn running_request_keeps_claimed_admission_while_followup_saves_and_releases_exact_owner() {
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn next_event(worker: &store::Worker) -> store::Event {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(event) = worker.try_event() {
                return event;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "store worker timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = Directory(std::env::temp_dir().join(format!(
        "eevee-request-admission-{}-{stamp}",
        std::process::id()
    )));
    let worker = store::Worker::start(
        directory.0.clone(),
        "/workspace".into(),
        store::Policy::default(),
    )
    .unwrap();
    let mut app = request_app();
    app.store_event(next_event(&worker));
    app.update(Event::Paste("Exact reviewed request".into()));
    key(&mut app, KeyCode::Enter);
    let mut pending = Some(app.snapshot("/workspace"));
    assert!(worker.try_save(&mut pending));
    app.save_submitted();
    app.store_event(next_event(&worker));
    assert!(app.settle_planning());
    assert!(
        app.planning.pending.is_none(),
        "provider preparation waits for durable ownership"
    );
    assert!(worker.try_work(&mut app.planning.work_pending));
    app.planning.work_inflight = true;
    app.store_event(next_event(&worker));
    let Some(crate::providers::Request::Preview(input)) = app.planning.pending.take() else {
        panic!("expected acquired request preview");
    };
    let claim = input
        .work
        .clone()
        .expect("content requests own a durable claim");
    input.validate_committed(&directory.0).unwrap();
    app.provider_event(
        1,
        crate::providers::Event::Preview(crate::providers::Preview {
            arrival: crate::providers::approval_arrival(),
            destination: None,
            requires_connection_review: true,
            generation: 12,
            task_id: app.task_id,
            thread_id: None,
            trust: vec!["Reviewed connection".into()],
            payload: "{}".into(),
            blockers: Vec::new(),
        }),
    );
    app.settle_planning();
    app.command("approvals");
    app.planning.reviewed = true;
    key(&mut app, KeyCode::Char('y'));
    assert!(matches!(
        app.planning.pending.take(),
        Some(crate::providers::Request::Run { generation: 12 })
    ));
    assert!(app.input.is_empty());
    app.update(Event::Paste("Retained follow-up".into()));
    key(&mut app, KeyCode::Left);
    let caret = app.cursor;
    modified_key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert!(
        app.save_due(std::time::Instant::now()),
        "private input does not freeze future draft persistence"
    );
    let mut pending = Some(app.snapshot("/workspace"));
    assert!(worker.try_save(&mut pending));
    app.save_submitted();
    app.store_event(next_event(&worker));
    assert_eq!(app.input, "Retained follow-up");
    assert_eq!(app.cursor, caret);
    assert_eq!(
        app.planning.immutable_task().unwrap().request_text(),
        "Exact reviewed request"
    );
    input.validate_committed(&directory.0).unwrap();
    assert!(
        store::validate_planning_input(&directory.0, &input.task, &[]).is_err(),
        "ordinary unclaimed admission remains strict"
    );
    app.planning.cancel_owned();
    input.validate_committed(&directory.0).unwrap();
    assert_eq!(
        app.planning.work.as_ref(),
        Some(&claim),
        "cancel does not release or reuse live ownership"
    );
    app.provider_event(
        1,
        crate::providers::Event::Finished {
            generation: 12,
            task_id: app.task_id,
            outcome: crate::providers::Outcome::Cancelled,
            message: "Stopped before output".into(),
        },
    );
    app.settle_planning();
    app.planning.settle_work();
    assert!(
        matches!(app.planning.work_pending.as_ref(), Some(store::WorkRequest::Release(owned)) if owned == &claim)
    );
    assert!(worker.try_work(&mut app.planning.work_pending));
    app.planning.work_inflight = true;
    app.store_event(next_event(&worker));
    assert!(app.planning.work.is_none());
    assert!(
        input.validate_committed(&directory.0).is_err(),
        "released claims never authorize another request"
    );
    assert_eq!(app.input, "Retained follow-up");
    assert_eq!(app.cursor, caret);
}

#[test]
fn failure_without_output_recovers_original_request_when_followup_is_empty() {
    let mut app = request_app();
    app.update(Event::Paste("Recover this request".into()));
    request_preview(&mut app);
    app.planning.reviewed = true;
    key(&mut app, KeyCode::Char('y'));
    assert!(app.input.is_empty());
    assert!(app.planning.work.is_some());
    app.planning.event(crate::providers::Event::Finished {
        generation: 12,
        task_id: app.task_id,
        outcome: crate::providers::Outcome::Failed,
        message: "Launch refused".into(),
    });
    assert!(app.settle_planning());
    assert_eq!(app.input, "Recover this request");
    assert_eq!(app.cursor, app.input.len());
    assert!(app.composer_is_request());
    assert!(!app.planning.holds_committed_input());
    assert!(app.save_due(std::time::Instant::now()));
}

#[test]
fn explicit_run_returns_home_with_empty_followup_and_preserves_task_facts_and_output() {
    let mut app = request_app();
    app.goal = "Stable task".into();
    app.next_action = "Keep this task fact".into();
    app.update(Event::Paste("First request".into()));
    request_preview(&mut app);
    assert!(!app.planning.affirmative);
    app.planning.reviewed = true;
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.planning.pending.take(),
        Some(crate::providers::Request::Run { generation: 12 })
    ));
    assert!(app.overlay.is_none());
    assert!(app.composer_is_request());
    assert!(app.input.is_empty());
    app.update(Event::Paste("Follow-up".into()));
    key(&mut app, KeyCode::Enter);
    assert!(
        app.pending_request.is_none(),
        "busy request retains sole ownership"
    );
    app.planning.event(crate::providers::Event::Item {
        generation: 12,
        task_id: app.task_id,
        thread_id: "owned".into(),
        turn_id: "turn".into(),
        item_id: "answer".into(),
        kind: crate::providers::ItemKind::Message,
        text: "Partial answer".into(),
        complete: false,
    });
    app.settle_planning();
    assert!(app.overlay.is_none());
    assert!(app.response_on_home());
    assert_eq!(app.input, "Follow-up");
    app.planning.event(crate::providers::Event::Finished {
        generation: 12,
        task_id: app.task_id,
        outcome: crate::providers::Outcome::Completed,
        message: "Complete".into(),
    });
    app.settle_planning();
    acknowledge_work(&mut app);
    assert!(app.overlay.is_none());
    key(&mut app, KeyCode::Enter);
    let snapshot = save_request(&mut app);
    assert_eq!(snapshot.request_text(), "Follow-up");
    assert_eq!(snapshot.goal, "Stable task");
    assert_eq!(snapshot.next_action, "Keep this task fact");
    assert!(app.settle_planning());
    acknowledge_work(&mut app);
    assert!(
        matches!(app.planning.pending.as_ref(), Some(crate::providers::Request::Preview(input))
        if input.task.request_text() == "Follow-up")
    );
    assert_eq!(app.planning.items[0].text, "Partial answer");
}

#[test]
fn cancel_review_and_restart_restore_request_not_previous_prompt_or_task_goal() {
    let mut app = request_app();
    app.goal = "Saved task identity".into();
    app.update(Event::Paste("Keep my question".into()));
    request_preview(&mut app);
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());
    assert!(app.composer_is_request());
    assert_eq!(app.input, "Keep my question");
    let snapshot = app.snapshot("/workspace");
    let mut restored = request_app();
    restored.load_task(snapshot);
    assert!(restored.composer_is_request());
    assert_eq!(restored.input, "Keep my question");
    assert_eq!(restored.goal, "Saved task identity");
    assert!(restored.pending_request.is_none());
    assert!(restored.planning.pending.is_none());
}

#[test]
fn local_save_paste_and_changed_draft_do_not_prepare_provider_work() {
    let mut app = request_app();
    app.update(Event::Paste("y/n/a\n/new".into()));
    modified_key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    save_request(&mut app);
    assert!(app.composer_is_request());
    assert!(!app.settle_planning());
    assert!(app.planning.pending.is_none());
    key(&mut app, KeyCode::Enter);
    let saved = committed(&app);
    key(&mut app, KeyCode::Char('!'));
    app.store_event(store::Event::Saved(saved));
    app.settle_planning();
    assert!(app.pending_request.is_none());
    assert!(app.planning.pending.is_none());
    assert!(app.input.ends_with('!'));
}

#[test]
fn global_confirmation_shortcuts_need_visible_summary_and_unmodified_press() {
    let mut app = request_app();
    app.update(Event::Paste("Keep draft".into()));
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('y'));
    assert!(app.confirmation.is_some());
    app.confirmation_reviewed = true;
    for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
        app.update(Event::Key(KeyEvent {
            code: KeyCode::Char('y'),
            modifiers: KeyModifiers::NONE,
            kind,
            state: crossterm::event::KeyEventState::NONE,
        }));
        assert!(app.confirmation.is_some());
    }
    modified_key(&mut app, KeyCode::Char('y'), KeyModifiers::ALT);
    app.update(Event::Paste("yna".into()));
    assert!(app.confirmation.is_some());
    key(&mut app, KeyCode::Char('n'));
    assert!(app.confirmation.is_none());
    assert_eq!(app.input, "Keep draft");
    key(&mut app, KeyCode::Esc);
    app.confirmation_reviewed = true;
    key(&mut app, KeyCode::Char('y'));
    assert!(app.confirmation.is_none());
    assert!(app.input.is_empty());
}

#[test]
fn restart_restores_overlay_and_temporary_fact_editor_returns_to_request() {
    let mut original = app();
    original.open_editor(store::EditorField::Request);
    original.update(Event::Paste("Retained request 漢字".into()));
    original.command("help");
    let snapshot = committed(&original);
    let task_id = snapshot.id;
    let mut restored = App::new("/workspace".into(), 120, 36);
    restored.store_event(store::Event::Loaded(store::Project {
        tasks: vec![snapshot],
        partners: vec![store::Partner::default()],
        active_id: Some(task_id),
        usage_bytes: 0,
    }));
    assert_eq!(restored.overlay, Some(Overlay::Help));
    assert!(!restored.editing);
    key(&mut restored, KeyCode::Esc);
    assert!(restored.composer_is_request());
    assert_eq!(restored.input, "Retained request 漢字");
    restored.command("goal");
    restored.update(Event::Paste("Local task identity".into()));
    key(&mut restored, KeyCode::Enter);
    assert_eq!(restored.goal, "Local task identity");
    assert!(restored.composer_is_request());
    assert_eq!(restored.input, "Retained request 漢字");
    restored.command("help");
    key(&mut restored, KeyCode::Esc);
    assert!(restored.composer_is_request());
    assert_eq!(restored.input, "Retained request 漢字");
    restored.open_editor(store::EditorField::Request);
    assert_eq!(restored.input, "Retained request 漢字");
}

#[test]
fn reset_review_is_neutral_and_paste_never_invokes_lifecycle_commands() {
    for command in ["reset", "init"] {
        let mut app = request_app();
        app.update(Event::Paste(format!("/{command}\n")));
        assert!(app.confirmation.is_none());
        assert_eq!(app.input, format!("/{command}\n"));
        assert!(app.planning.pending.is_none());
    }
    let mut app = request_app();
    app.update(Event::Paste("keep draft suffix".into()));
    key(&mut app, KeyCode::Left);
    let before = (app.input.clone(), app.cursor, app.revision);
    app.command("reset");
    assert_eq!(app.confirmation.unwrap().kind, ConfirmationKind::ResetTask);
    assert!(!app.confirmation.unwrap().destructive);
    app.update(Event::Paste("y\n".into()));
    key(&mut app, KeyCode::Enter);
    assert!(app.confirmation.is_none());
    assert_eq!((app.input.clone(), app.cursor, app.revision), before);
    app.command("reset");
    key(&mut app, KeyCode::Char('n'));
    assert_eq!((app.input.clone(), app.cursor, app.revision), before);
    assert!(app.planning.pending.is_none());
}

#[test]
fn reset_waits_for_matching_durable_save_and_retains_old_facts_and_draft() {
    let mut app = request_app();
    app.goal = "Old task facts".into();
    app.update(Event::Paste("old request suffix".into()));
    key(&mut app, KeyCode::Left);
    let cursor = app.cursor;
    app.command("reset");
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert!(app.reset_after_save);
    assert_eq!(app.goal, "Old task facts");
    assert!(app.save_due(std::time::Instant::now()));
    let snapshot = committed(&app);
    let mut stale = snapshot.clone();
    stale.revision -= 1;
    app.store_event(store::Event::Saved(stale));
    assert!(app.reset_after_save);
    app.save_submitted();
    app.store_event(store::Event::Saved(snapshot.clone()));
    assert!(!app.reset_after_save);
    assert_eq!(app.task_id, 0);
    assert!(app.goal.is_empty());
    assert!(app.input.is_empty());
    assert!(app.composer_is_request());
    assert!(app.planning.pending.is_none());
    let saved = app
        .tasks
        .iter()
        .find(|task| task.id == snapshot.id)
        .unwrap();
    assert_eq!(saved.goal, "Old task facts");
    let draft = &saved.partner_drafts[&app.lead().id].drafts[0];
    assert_eq!(draft.text, "old request suffix");
    assert_eq!(draft.cursor, cursor);
    let saved = saved.clone();
    app.load_task(saved);
    assert_eq!(app.input, "old request suffix");
    assert_eq!(app.cursor, cursor);
}

#[test]
fn reset_clean_task_needs_no_save_and_save_failure_never_switches() {
    let mut app = request_app();
    app.command("reset");
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert!(!app.reset_after_save);
    assert!(app.composer_is_request());
    assert!(app.planning.pending.is_none());
    app.update(Event::Paste("retained despite failure".into()));
    app.command("reset");
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert!(app.reset_after_save);
    app.store_event(store::Event::Failed {
        revision: Some(app.revision),
        message: "disk full".into(),
    });
    assert!(!app.reset_after_save);
    assert!(
        app.drafts
            .iter()
            .any(|draft| draft.text == "retained despite failure")
    );
    assert!(app.notice.contains("Reset stopped"));
    assert!(app.planning.pending.is_none());
}

#[test]
fn reset_save_wait_can_be_cancelled_without_late_ack_switching_tasks() {
    let mut app = request_app();
    app.update(Event::Paste("stay here".into()));
    app.command("reset");
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    let snapshot = committed(&app);
    key(&mut app, KeyCode::Esc);
    assert!(!app.reset_after_save);
    app.store_event(store::Event::Saved(snapshot));
    assert_ne!(app.task_id, 0);
    assert!(app.drafts.iter().any(|draft| draft.text == "stay here"));
}

#[test]
fn reset_busy_work_uses_existing_guards_without_dispatch_or_abandonment() {
    let mut app = request_app();
    app.task_id = 7;
    assert!(
        app.execution
            .prepare_command(7, r#"["printf","not-dispatched"]"#)
    );
    app.command("reset");
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.task_id, 7);
    assert!(app.execution.busy());
    assert!(
        app.execution.confirmation.is_some(),
        "Owned command navigation requires an explicit Stay / cancel-and-wait review"
    );
    assert!(!app.execution.affirmative);
    assert!(!app.reset_after_save);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.task_id, 7, "Neutral Stay cannot reset the task");
    assert!(
        app.execution.busy(),
        "Neutral Stay retains the original command preparation"
    );
    assert!(
        app.execution.prepared.is_none() && app.execution.active.is_none(),
        "Reset never dispatches the pending command"
    );

    let mut app = request_app();
    app.update(Event::Paste("provider request".into()));
    request_preview(&mut app);
    let task_id = app.task_id;
    app.command("reset");
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.task_id, task_id);
    assert!(app.planning.preview.is_some());
    assert!(!app.reset_after_save);
    assert!(app.planning.pending.is_none());
    assert!(app.notice.contains("cancel"));
}

#[test]
fn reset_does_not_transfer_or_cancel_another_partners_command() {
    let mut app = request_app();
    app.task_id = 7;
    let partner = store::Partner {
        id: 2,
        form: 2,
        ..Default::default()
    };
    let owner = store::Attribution::of(&partner);
    app.party.push(partner);
    assert!(app.execution_state_mut(2).prepare_command_for(
        7,
        Some(owner.clone()),
        r#"["printf","not-dispatched"]"#,
    ));
    app.command("reset");
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.task_id, 7);
    assert_eq!(app.lead.id, 1);
    let retained = app.execution_state(2).unwrap();
    assert!(retained.busy());
    assert_eq!(retained.owner.as_ref(), Some(&owner));
    assert!(
        retained.confirmation.is_none(),
        "The lead cannot stage navigation on another owner's command"
    );
    assert!(retained.prepared.is_none() && retained.active.is_none());
    assert!(
        app.cooperation.cancel_review.is_none(),
        "Reset does not implicitly become Cancel all"
    );
    assert!(!app.reset_after_save);
}

#[test]
fn init_prepares_editable_plan_request_without_sending_or_writing() {
    let mut app = request_app();
    app.set_form(3);
    app.planning
        .set_operation(crate::providers::Operation::Implement);
    app.planning.edit_paths.push("AGENTS.md".into());
    app.command("init");
    assert!(app.composer_is_request());
    assert_eq!(app.form(), 4);
    assert_eq!(app.planning.operation, crate::providers::Operation::Plan);
    assert!(app.planning.edit_paths.is_empty());
    assert!(app.input.contains("/workspace"));
    assert!(app.input.contains("including any AGENTS.md"));
    assert!(
        app.input
            .contains("Do not create, overwrite or change AGENTS.md")
    );
    assert!(app.input.contains("later, separate explicit approval"));
    assert!(app.input.contains("do not invent commands or results"));
    assert!(app.goal.is_empty());
    assert!(app.pending_request.is_none());
    assert!(app.planning.pending.is_none());
    assert!(!app.execution.busy());
    assert!(app.management.is_none());
    key(&mut app, KeyCode::Char('!'));
    assert!(app.input.ends_with('!'));
    request_preview(&mut app);
    assert_eq!(app.planning.operation, crate::providers::Operation::Plan);
    assert!(app.planning.preview.is_some());
    assert!(app.planning.pending.is_none(), "review is not Run consent");
}

#[test]
fn init_replacement_is_neutral_and_preserves_other_fact_drafts() {
    let mut app = request_app();
    app.update(Event::Paste("original request suffix".into()));
    key(&mut app, KeyCode::Left);
    let before = (app.input.clone(), app.cursor, app.revision);
    app.command("init");
    assert_eq!(
        app.confirmation.unwrap().kind,
        ConfirmationKind::ReplaceWithInit
    );
    key(&mut app, KeyCode::Enter);
    assert_eq!((app.input.clone(), app.cursor, app.revision), before);
    app.command("init");
    key(&mut app, KeyCode::Char('n'));
    assert_eq!((app.input.clone(), app.cursor, app.revision), before);
    app.command("goal");
    app.update(Event::Paste("old goal draft".into()));
    app.command("init");
    assert!(
        app.confirmation.is_some(),
        "retained Request also needs review"
    );
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert!(
        app.input
            .starts_with("Prepare a project-initialization proposal")
    );
    assert!(app.drafts.iter().any(|draft| {
        draft.field == store::EditorField::Goal && draft.text == "old goal draft"
    }));
    assert!(app.pending_request.is_none());
    assert!(app.planning.pending.is_none());

    let mut app = request_app();
    app.command("goal");
    app.update(Event::Paste("only goal draft".into()));
    app.command("init");
    assert!(app.confirmation.is_none());
    assert!(app.composer_is_request());
    assert!(
        app.drafts
            .iter()
            .any(|draft| draft.text == "only goal draft")
    );
}

#[test]
fn lifecycle_reviews_require_rendered_unmodified_consent_and_supported_size() {
    for command in ["reset", "init"] {
        let mut app = request_app();
        app.update(Event::Paste("preserve me".into()));
        app.command(command);
        key(&mut app, KeyCode::Char('y'));
        assert!(
            app.confirmation.is_some(),
            "unrendered review is not consent"
        );
        app.confirmation_reviewed = true;
        for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
            app.update(Event::Key(KeyEvent {
                code: KeyCode::Char('y'),
                modifiers: KeyModifiers::NONE,
                kind,
                state: crossterm::event::KeyEventState::NONE,
            }));
            assert!(app.confirmation.is_some());
        }
        modified_key(&mut app, KeyCode::Char('y'), KeyModifiers::ALT);
        app.update(Event::Paste("y\n".into()));
        assert_eq!(app.input, "preserve me");
        app.update(Event::Resize(40, 10));
        app.confirmation_reviewed = true;
        key(&mut app, KeyCode::Char('y'));
        assert!(app.confirmation.is_some());
        assert_eq!(app.input, "preserve me");
        key(&mut app, KeyCode::Char('n'));
        app.command(command);
        assert!(
            app.confirmation.is_none(),
            "small terminal refuses new review"
        );
        assert!(app.pending_request.is_none());
        assert!(app.planning.pending.is_none());
    }
}

#[test]
fn init_oversized_project_identity_preserves_draft_caret_and_role() {
    for field in [store::EditorField::Request, store::EditorField::Goal] {
        let mut app = request_app();
        app.set_form(3);
        app.planning
            .set_operation(crate::providers::Operation::Implement);
        app.planning.edit_paths.push("existing.rs".into());
        app.open_editor(field);
        app.update(Event::Paste("keep this draft and suffix".into()));
        key(&mut app, KeyCode::Left);
        app.cwd = format!("/{}", "long-project/".repeat(MAX_GOAL_BYTES));
        let input = app.input.clone();
        let cursor = app.cursor;
        let revision = app.revision;
        let drafts = app.drafts.clone();
        let navigation = app.navigation();
        app.command("init");
        if field == store::EditorField::Request {
            key(&mut app, KeyCode::Right);
            key(&mut app, KeyCode::Enter);
        }
        assert_eq!(app.input, input);
        assert_eq!(app.cursor, cursor);
        assert_eq!(app.revision, revision);
        assert_eq!(app.drafts, drafts);
        assert_eq!(app.navigation(), navigation);
        assert_eq!(app.form(), 3);
        assert_eq!(
            app.planning.operation,
            crate::providers::Operation::Implement
        );
        assert_eq!(
            app.planning.edit_paths,
            vec![std::path::PathBuf::from("existing.rs")]
        );
        assert!(!app.input_truncated);
        assert!(app.pending_request.is_none());
        assert!(app.planning.pending.is_none());
        assert!(!app.execution.busy());
        assert!(
            app.notice
                .contains("shorter initialization request manually")
        );
    }
}

#[test]
fn lead_and_roster_stay_reachable_while_another_partner_requests() {
    let mut app = app();
    app.editing = false;
    app.task_id = 7;
    app.goal = "task".into();
    app.planning.reset(7);
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner::default(),
        store::Partner {
            id: 2,
            ..store::Partner::default()
        },
    ])));
    let mut task = committed(&app);
    task.id = 7;
    app.planning.command("new", true, &task, &[], None);
    assert!(app.planning.busy());
    app.command_with_argument("/lead 2");
    assert_eq!(app.lead().id, 2, "{}", app.notice);
    app.command("party");
    assert_eq!(app.overlay, Some(Overlay::Party));
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.lead().id, 1, "{}", app.notice);
    assert_eq!(app.planning.owner().map(|owner| owner.partner), Some(1));
    assert!(
        app.planning.pending.is_some(),
        "navigation never sends or cancels"
    );
}

#[test]
fn each_forms_primary_empty_action_opens_its_flow_and_executes_nothing() {
    for (index, form) in crate::domain::FORMS.iter().enumerate() {
        let mut app = app();
        app.command(&form.name.to_lowercase());
        assert_eq!(app.form(), index);
        let (_, action, _) = crate::domain::FIRST_ACTIONS[index];
        match action {
            Some(command) => {
                key(&mut app, KeyCode::Char('/'));
                type_text(&mut app, command);
                key(&mut app, KeyCode::Enter);
                let opened = match command {
                    "criteria-edit" => {
                        app.editing && app.editor_field == store::EditorField::Criteria
                    }
                    "files" => app.overlay == Some(Overlay::Sources),
                    "command-edit" => {
                        app.editing && app.editor_field == store::EditorField::Command
                    }
                    "paths" => {
                        app.overlay == Some(Overlay::Planning)
                            && app.planning.paths_editor.is_some()
                    }
                    "handoff" => app.overlay == Some(Overlay::Handoff),
                    _ => app.overlay == Some(Overlay::Workflows),
                };
                assert!(
                    opened,
                    "{}: /{command} did not open; {}",
                    form.name, app.notice
                );
            }
            None => {
                // The composer is the flow: typing stays a local draft.
                app.open_editor(store::EditorField::Request);
                type_text(&mut app, "which approach?");
                assert_eq!(app.input, "which approach?");
            }
        }
        assert!(app.planning.pending.is_none(), "{}", form.name);
        assert!(!app.execution.busy(), "{}", form.name);
        assert!(app.execution.history.is_empty(), "{}", form.name);
    }
}

#[test]
fn out_of_role_actions_name_the_owner_and_offer_a_working_switch() {
    for (form, command, owner, owner_index) in [
        ("glaceon", "role-implement", "Flareon", 3),
        ("glaceon", "paths", "Flareon", 3),
        ("espeon", "command-edit", "Jolteon", 2),
    ] {
        let mut app = app();
        app.command(form);
        let before = app.form();
        let operation = app.planning.operation;
        let reason = app.command_disabled_reason(command).expect(command);
        assert!(reason.contains(&format!("Switch to {owner}")), "{reason}");
        // Typed through the palette like a user: nothing starts.
        key(&mut app, KeyCode::Char('/'));
        type_text(&mut app, command);
        key(&mut app, KeyCode::Enter);
        assert!(app.notice.contains(owner), "{}", app.notice);
        assert_eq!(app.form(), before);
        assert_eq!(app.planning.operation, operation);
        assert!(app.planning.paths_editor.is_none());
        assert!(!app.editing || app.editor_field != store::EditorField::Command);
        assert!(app.planning.pending.is_none() && !app.execution.busy());
        let popup = app.command_popup.as_ref().expect("switch offered");
        assert_eq!(popup.choices[0].label, format!("Switch to {owner}"));
        // The offer works and only switches the form.
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.form(), owner_index, "{}", app.notice);
        assert!(app.planning.paths_editor.is_none());
        assert!(!app.editing || app.editor_field != store::EditorField::Command);
        assert!(app.planning.pending.is_none() && !app.execution.busy());
        assert!(app.command_disabled_reason(command).is_none());
    }
    // An existing neutral review retains sole input ownership.
    let mut app = app();
    app.command("espeon");
    assert!(app.execution.prepare_command(1, r#"["printf","kept"]"#));
    app.command("jolteon");
    assert!(app.execution.confirmation.is_some());
    app.command("command-edit");
    assert!(app.command_popup.is_none());
    assert_eq!(app.form(), 4);
    assert!(app.execution.confirmation.is_some());
    assert!(
        app.execution.busy(),
        "refusal must not invalidate the request"
    );
}

#[test]
fn cancelling_an_owner_switch_keeps_the_draft_and_editing_scope() {
    let mut app = app();
    app.command("glaceon");
    app.open_editor(store::EditorField::Request);
    type_text(&mut app, "review only 漢字");
    let draft = (app.input.clone(), app.cursor);
    app.planning.edit_paths.push("selected.rs".into());
    key(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "paths");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.form(), 7);
    assert_eq!((app.input.clone(), app.cursor), draft);
    assert!(app.editing);
    assert_eq!(
        app.planning.edit_paths,
        vec![std::path::PathBuf::from("selected.rs")]
    );
    assert!(app.planning.paths_editor.is_none());
    assert!(app.planning.pending.is_none() && !app.execution.busy());

    // Both implementation forms may explicitly open scope selection.
    for form in ["flareon", "leafeon"] {
        app.command(form);
        app.command("paths");
        assert!(
            app.planning.paths_editor.is_some(),
            "{form}: {}",
            app.notice
        );
        assert_eq!(
            app.planning.edit_paths,
            vec![std::path::PathBuf::from("selected.rs")]
        );
        key(&mut app, KeyCode::Esc);
    }
}

#[test]
fn selecting_new_nested_paths_preserves_request_bytes_and_caret_without_dispatch() {
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = app();
        app.width = width;
        app.height = height;
        app.command("flareon");
        app.open_editor(store::EditorField::Request);
        type_text(&mut app, "create 새 파일 without altering this draft");
        key(&mut app, KeyCode::Left);
        key(&mut app, KeyCode::Left);
        let draft = (app.input.clone(), app.cursor);
        app.command("paths");
        let editor = app.planning.paths_editor.as_mut().unwrap();
        editor.text = r#"["not-created/nested/새.rs"]"#.into();
        editor.cursor = editor.text.len();
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            app.planning.edit_paths,
            vec![std::path::PathBuf::from("not-created/nested/새.rs")]
        );
        // Scope selection retains the owned draft; reopen its normal editor.
        app.open_editor(store::EditorField::Request);
        assert_eq!((app.input.clone(), app.cursor), draft);
        assert!(app.planning.paths_editor.is_none());
        assert!(app.planning.pending.is_none() && app.planning.work.is_none());
        assert!(app.pending_request.is_none() && !app.execution.busy());
    }
}

#[test]
fn returning_to_eevee_preserves_task_content_and_drafts() {
    for (width, height) in [(120, 36), (60, 18)] {
        for form in ["flareon", "umbreon", "sylveon"] {
            for picker in [false, true] {
                let mut app = app();
                app.width = width;
                app.height = height;
                app.command(form);
                let prior_form = app.form();
                app.goal = "Keep task facts 漢字".into();
                app.criteria.push(store::Criterion {
                    text: "Keep this checked criterion".into(),
                    met: true,
                });
                app.next_action = "Inspect the existing evidence".into();
                app.command = r#"["printf","never replay"]"#.into();
                app.drafts.push(store::Draft {
                    field: store::EditorField::Request,
                    text: "retained e\u{301} draft".into(),
                    cursor: 4,
                });
                app.planning.edit_paths.push("kept.rs".into());
                let mut expected = app.snapshot("/workspace");
                if picker {
                    app.command("forms");
                    key(&mut app, KeyCode::Home);
                    assert!(app.form_return_selected);
                    app.update(Event::Resize(height * 2, height));
                    assert!(app.form_return_selected);
                    // Restore a supported size before selecting the local action.
                    app.update(Event::Resize(width, height));
                    key(&mut app, KeyCode::Enter);
                } else {
                    app.command("eevee");
                }
                assert_eq!(app.form(), 0, "{form}: {}", app.notice);
                assert!(app.overlay.is_none());
                let actual = app.snapshot("/workspace");
                // Form identity, previous-form memory and save bookkeeping must
                // change; all task payload bytes must remain identical.
                expected.form = 0;
                expected.revision = actual.revision;
                expected.updated_at = actual.updated_at;
                expected.navigation.form_selection = 0;
                expected.navigation.previous_form = Some(prior_form);
                expected
                    .partner_drafts
                    .get_mut(&app.lead.id)
                    .unwrap()
                    .previous_form = Some(prior_form);
                assert_eq!(
                    serde_json::to_vec(&actual).unwrap(),
                    serde_json::to_vec(&expected).unwrap(),
                    "{form}, picker={picker}, {width}x{height}"
                );
                assert_eq!(
                    app.planning.edit_paths,
                    vec![std::path::PathBuf::from("kept.rs")]
                );
                assert!(app.planning.pending.is_none() && !app.execution.busy());
            }
        }
    }
}

#[test]
fn return_picker_entry_preserves_form_preview_and_nested_focus() {
    let mut app = app();
    app.command("sylveon");
    app.command("forms");
    key(&mut app, KeyCode::Home);
    assert_eq!(app.form(), 8, "preview is not a switch");
    app.command("help");
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.overlay, Some(Overlay::Forms));
    assert!(app.form_return_selected);
    key(&mut app, KeyCode::Down);
    assert!(!app.form_return_selected);
    key(&mut app, KeyCode::Up);
    assert!(app.form_return_selected);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.form(), 8);
    assert!(app.planning.pending.is_none() && !app.execution.busy());
}

#[test]
fn return_to_eevee_waits_for_explicit_provider_cancellation_and_known_outcome() {
    use crate::providers::{Outcome, Preview, Request};
    for unknown in [false, true] {
        let mut app = app();
        app.command("espeon");
        app.task_id = 7;
        app.goal = "Keep the current request".into();
        app.planning.reset(7);
        let task = committed(&app);
        app.planning.command("new", true, &task, &[], None);
        app.planning.pending.take();
        app.planning
            .event(crate::providers::Event::Preview(Preview {
                arrival: crate::providers::approval_arrival(),
                destination: None,
                requires_connection_review: false,
                generation: 12,
                task_id: 7,
                thread_id: None,
                trust: vec![],
                payload: "{}".into(),
                blockers: vec![],
            }));
        assert!(app.planning.dispatch_prepared());
        app.planning.pending.take();
        app.open_editor(store::EditorField::Request);
        type_text(&mut app, "Keep this unsent follow-up 漢字");
        let draft = (app.input.clone(), app.cursor);
        app.command("eevee");
        assert!(app.confirmation.is_some());
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.form(), 4);
        assert!(!app.planning.cancelling, "neutral Enter must stay");
        assert_eq!((app.input.clone(), app.cursor), draft);
        assert!(app.editing);
        app.command("eevee");
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Enter);
        assert!(matches!(
            app.planning.pending.take(),
            Some(Request::Cancel { generation: 12 })
        ));
        app.settle_planning();
        assert_eq!(app.form(), 4, "a cancellation request is not completion");
        app.planning.event(crate::providers::Event::Finished {
            generation: 12,
            task_id: 7,
            outcome: if unknown {
                Outcome::Unknown
            } else {
                Outcome::Cancelled
            },
            message: "Fixture cancellation outcome".into(),
        });
        app.settle_planning();
        assert_eq!(app.form(), if unknown { 4 } else { 0 });
        assert!(app.return_after_cancel.is_none());
        assert!(app.planning.pending.is_none());
        assert_eq!((app.input.clone(), app.cursor), draft);
        assert!(app.editing);
    }
}

#[test]
fn return_to_eevee_does_not_cancel_another_partners_request() {
    let mut app = app();
    app.task_id = 7;
    app.goal = "Partner-owned work".into();
    app.planning.reset(7);
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner::default(),
        store::Partner {
            id: 2,
            form: 5,
            ..store::Partner::default()
        },
    ])));
    let task = committed(&app);
    app.planning.command("new", true, &task, &[], None);
    app.command_with_argument("/lead 2");
    assert_eq!(app.lead.id, 2);
    app.command("eevee");
    assert_eq!(
        app.form(),
        0,
        "an idle partner can change form independently"
    );
    assert!(app.confirmation.is_none());
    assert!(app.return_after_cancel.is_none());
    assert!(!app.planning_state(1).unwrap().cancelling);
    assert!(matches!(
        app.planning_state(1).unwrap().pending,
        Some(crate::providers::Request::Preview(_))
    ));
}

#[test]
fn nickname_editor_is_local_grapheme_safe_and_cancel_keeps_composer() {
    let mut app = app();
    app.open_editor(store::EditorField::Request);
    app.update(Event::Paste("Request 界e\u{301}".into()));
    key(&mut app, KeyCode::Left);
    let draft = (
        app.input.clone(),
        app.cursor,
        app.editor_field,
        app.goal.clone(),
    );
    app.command("nickname");
    assert_eq!(app.nickname_editor.as_ref().unwrap().text, "");
    app.update(Event::Paste("界e\u{301}👩‍💻".into()));
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.nickname_editor.as_ref().unwrap().text, "界👩‍💻");
    app.update(Event::Paste("界".repeat(13)));
    assert!(!app.nickname_editor.as_ref().unwrap().error.is_empty());
    assert_eq!(app.nickname_editor.as_ref().unwrap().text, "界👩‍💻");
    key(&mut app, KeyCode::Esc);
    assert!(app.nickname_editor.is_none());
    assert_eq!(
        (
            app.input.clone(),
            app.cursor,
            app.editor_field,
            app.goal.clone()
        ),
        draft
    );
    assert!(app.party_request.is_none());
    assert!(app.planning.pending.is_none());
    assert!(!app.execution.busy());
}

#[test]
fn nickname_commit_failure_retry_and_clear_preserve_live_task_and_roster() {
    let mut app = app();
    app.open_editor(store::EditorField::Request);
    app.update(Event::Paste("task draft".into()));
    key(&mut app, KeyCode::Left);
    let draft = (app.input.clone(), app.cursor);
    app.set_form(7);
    let old_owner = app.attribution();
    app.planning.front = Some(old_owner.clone());
    app.command("nickname");
    app.update(Event::Paste("  月  ".into()));
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.party_request.take(),
        Some(store::PartyRequest::Nickname {
            partner: 1,
            nickname: Some("月".into()),
        })
    );
    assert!(app.lead().nickname.is_none());
    assert_eq!(app.planning.front.as_ref(), Some(&old_owner));
    app.store_event(store::Event::Party(Err("Duplicate nickname".into())));
    assert_eq!(app.nickname_editor.as_ref().unwrap().text, "  月  ");
    assert!(!app.nickname_editor.as_ref().unwrap().error.is_empty());
    assert!(app.lead().nickname.is_none());
    key(&mut app, KeyCode::Enter);
    app.party_request.take();
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner {
            nickname: Some("月".into()),
            form: 0,
            ..store::Partner::default()
        },
        store::Partner {
            id: 2,
            nickname: Some("星".into()),
            form: 3,
            ..store::Partner::default()
        },
    ])));
    assert!(app.nickname_editor.is_none());
    assert_eq!(app.lead_label(), "月 (Glaceon)");
    assert_eq!(
        app.form(),
        7,
        "a stale committed form must not replace the live form"
    );
    assert_eq!(app.partners()[1].nickname.as_deref(), Some("星"));
    assert_eq!(old_owner.name, "Eevee #1");
    assert_eq!(app.planning.front.as_ref().unwrap().name, "月");
    assert_eq!((app.input.clone(), app.cursor), draft);
    assert!(app.planning.pending.is_none());
    assert!(!app.execution.busy());
    app.command("nickname");
    key(&mut app, KeyCode::Backspace);
    app.update(Event::Paste(String::new()));
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.party_request.take(),
        Some(store::PartyRequest::Nickname {
            partner: 1,
            nickname: None,
        })
    );
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner::default(),
        store::Partner {
            id: 2,
            nickname: Some("星".into()),
            ..store::Partner::default()
        },
    ])));
    assert_eq!(app.lead_label(), "Eevee #1 (Glaceon)");
    assert_eq!((app.input.clone(), app.cursor), draft);
}

#[test]
fn nickname_party_action_targets_selected_partner_without_fronting_it() {
    let mut app = app();
    app.editing = false;
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner::default(),
        store::Partner {
            id: 2,
            ..store::Partner::default()
        },
    ])));
    app.command("party");
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(app.nickname_editor.as_ref().unwrap().partner, 2);
    assert_eq!(app.lead().id, 1);
    app.update(Event::Paste("星".into()));
    key(&mut app, KeyCode::Enter);
    app.party_request.take();
    app.store_event(store::Event::Party(Ok(vec![
        store::Partner::default(),
        store::Partner {
            id: 2,
            nickname: Some("星".into()),
            ..store::Partner::default()
        },
    ])));
    assert_eq!(app.lead().id, 1);
    assert!(app.lead().nickname.is_none());
    assert_eq!(app.partners()[1].nickname.as_deref(), Some("星"));
}

#[test]
fn nickname_ack_does_not_rewrite_prepared_input_or_request_ownership() {
    let mut app = app();
    app.goal = "Only the task goal".into();
    app.set_form(2);
    app.command("nickname");
    app.update(Event::Paste("LocalOnly月".into()));
    key(&mut app, KeyCode::Enter);
    app.party_request.take();
    // A request can finish binding while the independent store write is pending.
    let task = committed(&app);
    app.planning.command("new", true, &task, &[], None);
    let Some(crate::providers::Request::Preview(input)) = app.planning.pending.as_ref() else {
        panic!("request must be prepared");
    };
    let before = serde_json::to_value(&input.task).unwrap();
    let owner = app.planning.owner().cloned();
    app.store_event(store::Event::Party(Ok(vec![store::Partner {
        nickname: Some("LocalOnly月".into()),
        ..store::Partner::default()
    }])));
    let Some(crate::providers::Request::Preview(input)) = app.planning.pending.as_ref() else {
        panic!("nickname acknowledgement must not replace the request");
    };
    assert_eq!(serde_json::to_value(&input.task).unwrap(), before);
    assert!(!before.to_string().contains("LocalOnly"));
    assert_eq!(app.planning.owner(), owner.as_ref());
    assert_eq!(owner.unwrap().name, "Eevee #1");
    assert_eq!(app.planning.front.as_ref().unwrap().name, "LocalOnly月");
    assert!(!app.execution.busy());
}

#[test]
fn rejected_nickname_paste_cannot_clear_or_save_a_different_name() {
    let mut app = app();
    app.store_event(store::Event::Loaded(store::Project {
        tasks: vec![],
        partners: vec![store::Partner {
            nickname: Some("前".into()),
            ..store::Partner::default()
        }],
        active_id: None,
        usage_bytes: 0,
    }));
    app.command("nickname");
    key(&mut app, KeyCode::Home);
    key(&mut app, KeyCode::Delete);
    app.update(Event::Paste("界".repeat(13)));
    key(&mut app, KeyCode::Enter);
    assert!(app.party_request.is_none());
    assert!(app.nickname_editor.is_some());
    assert_eq!(app.lead().nickname.as_deref(), Some("前"));
    // Editing explicitly corrects the refused insertion; a valid name can save.
    app.update(Event::Paste("月".into()));
    key(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.party_request.take(),
        Some(store::PartyRequest::Nickname { nickname: Some(name), .. }) if name == "月"
    ));
}

fn response_scroll_app() -> App {
    let mut app = request_app();
    let text = (0..40)
        .map(|row| format!("row{row:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    for id in ["older", "newer"] {
        app.planning.items.push(planning::Item {
            thread_id: "conversation".into(),
            turn_id: id.into(),
            item_id: id.into(),
            kind: crate::providers::ItemKind::Message,
            text: text.clone(),
            complete: true,
            request: std::sync::Arc::from("original request"),
        });
    }
    app.planning.selected = 1;
    app.planning.response_rows(40, 4, false).count();
    app.planning
        .set_response_body(ratatui::layout::Rect::new(2, 5, 40, 4));
    app
}

fn response_wheel(
    app: &mut App,
    kind: crossterm::event::MouseEventKind,
    column: u16,
    row: u16,
) -> bool {
    app.update(Event::Mouse(crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }))
}

#[test]
fn response_wheel_and_pages_preserve_unicode_draft_caret_focus_and_reply() {
    let mut app = response_scroll_app();
    app.update(Event::Paste("draft界e\u{301}Z".into()));
    key(&mut app, KeyCode::Left);
    let draft = (app.input.clone(), app.cursor, app.navigation());
    assert!(response_wheel(
        &mut app,
        crossterm::event::MouseEventKind::ScrollUp,
        3,
        6
    ));
    assert_eq!(
        app.planning.response_rows(40, 4, false).collect::<Vec<_>>(),
        ["row33", "row34", "row35", "row36"]
    );
    assert!(!app.planning.following);
    assert_eq!(app.planning.selected, 1);
    key(&mut app, KeyCode::PageUp);
    assert_eq!(
        app.planning.response_rows(40, 4, false).collect::<Vec<_>>(),
        ["row30", "row31", "row32", "row33"]
    );
    assert_eq!((app.input.clone(), app.cursor, app.navigation()), draft);
    assert!(app.composer_is_request());
    assert!(app.pending_request.is_none() && app.planning.pending.is_none());
    modified_key(&mut app, KeyCode::End, KeyModifiers::ALT);
    assert_eq!(
        app.planning.response_rows(40, 4, false).collect::<Vec<_>>(),
        ["row36", "row37", "row38", "row39"]
    );
    assert!(app.planning.following);
    modified_key(&mut app, KeyCode::Up, KeyModifiers::ALT);
    assert_eq!(app.planning.items[app.planning.selected].item_id, "older");
    assert_eq!((app.input.clone(), app.cursor, app.navigation()), draft);
}

#[test]
fn response_reading_arrows_scroll_lines_and_mouse_cannot_reach_occluded_output() {
    let mut app = response_scroll_app();
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.effective_focus(), Pane::Workspace);
    key(&mut app, KeyCode::Up);
    assert_eq!(
        app.planning.response_rows(40, 4, false).collect::<Vec<_>>(),
        ["row35", "row36", "row37", "row38"]
    );
    assert_eq!(app.planning.items[app.planning.selected].item_id, "newer");
    let held = app.planning.scroll;
    assert!(!response_wheel(
        &mut app,
        crossterm::event::MouseEventKind::ScrollUp,
        0,
        0
    ));
    app.command("help");
    assert!(!response_wheel(
        &mut app,
        crossterm::event::MouseEventKind::ScrollUp,
        3,
        6
    ));
    assert_eq!(app.planning.scroll, held);
    assert_eq!(app.overlay, Some(Overlay::Help));
    assert!(app.pending_request.is_none() && app.planning.pending.is_none());
}
