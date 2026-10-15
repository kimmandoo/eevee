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
    app.update(Event::Key(KeyEvent::new(code, modifiers)))
}

#[test]
fn clearing_first_draft_before_insert_ack_does_not_restore_deleted_text() {
    let mut app = app();
    app.command("goal");
    key(&mut app, KeyCode::Char('x'));
    let first = committed(&app);
    app.save_requested = false;
    key(&mut app, KeyCode::Backspace);
    assert!(app.revision > first.revision);
    app.store_event(store::Event::Saved(first));
    let cleared = committed(&app);
    assert!(cleared.goal.is_empty());
    assert!(cleared.drafts.is_empty());
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
    assert!(app.drafts.is_empty());
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
    assert_eq!(resumed.lead_label(), "Eevee #1 · Umbreon");
}

#[test]
fn grid_arrows_wrap_by_column_and_row_while_list_moves_one_item() {
    let mut app = app();
    app.command("forms");
    key(&mut app, KeyCode::Up);
    assert_eq!(app.selected, 6);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.selected, 0);
    key(&mut app, KeyCode::Left);
    assert_eq!(app.selected, 8);
    key(&mut app, KeyCode::Right);
    assert_eq!(app.selected, 0);
    app.update(Event::Resize(89, 30));
    key(&mut app, KeyCode::Up);
    assert_eq!(app.selected, 8);
    app.update(Event::Resize(120, 36));
    app.command("layout");
    key(&mut app, KeyCode::Down);
    assert_eq!(app.selected, 0);
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
        app.set_scroll_max(surface, 100);
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
    assert!(!app.quit);
    assert_eq!(app.confirmation.unwrap().kind, ConfirmationKind::Quit);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
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
    assert_eq!(app.overlay, Some(Overlay::Execution));
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
        if editing {
            assert!(app.quit_after_save);
            assert!(!app.quit);
            app.store_event(store::Event::Saved(committed(&app)));
        }
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
    app.set_scroll_max(ScrollSurface::Workspace, 2);
    key(&mut app, KeyCode::PageDown);
    assert_eq!(app.scroll(), 2);
    assert!(!key(&mut app, KeyCode::Down));
    app.set_scroll_max(ScrollSurface::Workspace, 1);
    assert_eq!(app.scroll(), 1);
    app.set_scroll_max(ScrollSurface::Help, 3);
    app.command("help");
    key(&mut app, KeyCode::End);
    assert_eq!(app.scroll(), 3);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.scroll(), 1);
    app.set_scroll_max(ScrollSurface::Workspace, 0);
    assert_eq!(app.scroll(), 0);
}

#[test]
fn nested_previews_restore_role_selection_and_hidden_focus_exactly() {
    let mut app = app();
    app.focus = Pane::Inspector;
    app.command("forms");
    app.selected = 8;
    app.command("preview");
    assert_eq!(app.preview_form(), 8);
    app.command("forms");
    app.selected = 1;
    app.command("preview");
    assert_eq!(app.preview_form(), 1);
    app.update(Event::Resize(10, 3));
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.overlay, Some(Overlay::Forms));
    assert_eq!(app.selected, 1);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.overlay, Some(Overlay::Preview));
    assert_eq!(app.preview_form(), 8);
    assert_eq!(app.selected, 8);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.overlay, Some(Overlay::Forms));
    assert_eq!(app.selected, 8);
    key(&mut app, KeyCode::Esc);
    app.update(Event::Resize(120, 36));
    assert_eq!(app.effective_focus(), Pane::Inspector);
    assert_eq!(app.form(), 0);
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
