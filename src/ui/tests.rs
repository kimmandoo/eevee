use super::*;
use crate::config::{ImageMode, LayoutMode};

fn luminance(color: Color) -> f64 {
    let Color::Rgb(r, g, b) = color else {
        panic!("true-color theme expected")
    };
    let linear = |value: u8| {
        let v = f64::from(value) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

fn contrast(foreground: Color, background: Color) -> f64 {
    let a = luminance(foreground);
    let b = luminance(background);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn all_true_color_forms_keep_readable_text_and_selection() {
    for theme in [Theme::Dark, Theme::Light] {
        let config = Config {
            ascii: false,
            no_animation: false,
            theme,
            color: ColorMode::TrueColor,
            images: ImageMode::Off,
            assets_dir: "unused".into(),
            layout: LayoutMode::Sanctuary,
            layout_explicit: false,
            data_dir: "unused-data".into(),
            storage_policy: crate::store::Policy::default(),
            context_exclusions: Vec::new(),
            capture_capacity: 64 * 1024,
        };
        for (index, form) in FORMS.iter().enumerate() {
            let paint = Paint::new(&config, form, index);
            for style in [
                paint.base,
                paint.surface,
                paint.muted,
                paint.accent,
                paint.selected,
            ] {
                assert!(
                    contrast(style.fg.unwrap(), style.bg.unwrap()) >= 4.5,
                    "{} {theme:?}: {style:?}",
                    form.name
                );
            }
            assert!(contrast(paint.accent.fg.unwrap(), paint.surface.bg.unwrap()) >= 3.0);
        }
    }
}

#[test]
fn caret_tracks_wide_wrapped_and_combining_graphemes() {
    let input = "ab한e\u{301}\n끝";
    assert_eq!(editor_rows(input, 2, 3).1, (1, 0));
    assert_eq!(editor_rows(input, "ab한".len(), 3).1, (1, 2));
    assert_eq!(editor_rows(input, input.len(), 3).1, (2, 2));
    assert_eq!(editor_rows("한글", 6, 2).1, (2, 0));
}

#[test]
fn home_keeps_partner_visible_and_primary_surfaces_usable() {
    for (width, height) in [(120, 36), (100, 30), (90, 24), (80, 24), (70, 24), (60, 18)] {
        let area = Rect::new(0, 0, width, height);
        let layout = shell(area, false, Pane::Workspace);
        assert_eq!(layout.composer.width, width - 2);
        assert!(
            inner(layout.composer).height > 2,
            "input remains above hints"
        );
        assert!(layout.workspace.height > 0);
        assert_eq!(layout.workspace.y, layout.header.bottom());
        assert_eq!(layout.workspace.bottom(), layout.composer.y);
        assert_eq!(layout.composer.bottom(), layout.footer.y);
        assert!(layout.inspector.is_none());
        if width >= 90 {
            let partner = layout.rail.expect("roomy home has a partner");
            assert_eq!(partner.width, 26);
            assert!(partner.height <= 16);
            assert!(!partner.intersects(layout.workspace));
            assert!(!partner.intersects(layout.composer));
            assert!(layout.workspace.width >= 60);
            let portrait = companion_portrait(partner);
            assert_eq!(portrait.width, 22);
            assert_eq!(portrait.height, partner.height.saturating_sub(6).min(10));
        } else {
            assert!(layout.rail.is_none());
            assert_eq!(layout.workspace.width, layout.composer.width);
        }
        assert!(inner(layout.workspace).height >= 7);
        let linear = shell(area, true, Pane::Inspector);
        assert!(linear.rail.is_none());
        assert!(linear.inspector.is_none());
        assert_eq!(linear.workspace.width, linear.composer.width);
    }
    let focused = shell(Rect::new(0, 0, 120, 36), false, Pane::Inspector);
    assert!(focused.inspector.is_some());
    assert!(focused.rail.is_none());
}

#[test]
fn home_portrait_targets_follow_layout_and_hide_under_modals() {
    let mut app = App::new("/workspace".into(), 120, 36);
    let roomy = Rect::new(0, 0, 120, 36);
    let mut targets = Vec::new();
    portrait_targets(&app, roomy, &mut targets);
    assert_eq!(
        targets,
        vec![(
            app.form(),
            companion_portrait(shell(roomy, false, Pane::Workspace).rail.unwrap())
        )]
    );
    assert_eq!(targets[0].1.width, 22);
    assert_eq!(targets[0].1.height, 10);
    app.command("model");
    portrait_targets(&app, roomy, &mut targets);
    assert!(
        targets.is_empty(),
        "native images cannot cover the model picker"
    );

    let mut compact = App::new("/workspace".into(), 60, 18);
    let area = Rect::new(0, 0, 60, 18);
    portrait_targets(&compact, area, &mut targets);
    assert!(targets.is_empty());
    compact.command("companion");
    portrait_targets(&compact, area, &mut targets);
    assert_eq!(targets, vec![(compact.form(), portrait(drawer(area)))]);
}

#[test]
fn choices_are_centered_without_covering_header_or_footer() {
    for (width, height) in [(120, 36), (90, 24), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        let area = Rect::new(0, 0, width, height);
        let layout = shell(area, false, Pane::Workspace);
        app.composer_menu = Some(crate::app::composer::Menu {
            kind: crate::app::composer::Kind::Files,
            start: 0,
            selected: 0,
        });
        for popup in [
            command_popup_area(area, &app),
            composer_menu_area(area, &app).unwrap(),
        ] {
            assert!((popup.x * 2 + popup.width).abs_diff(width) <= 1);
            assert!((popup.y * 2 + popup.height).abs_diff(height) <= 1);
            assert!(!popup.intersects(layout.header));
            assert!(!popup.intersects(layout.footer));
            assert!(inner(popup).height >= 7, "choices and help remain usable");
        }
    }
}

#[test]
fn compact_palette_shows_blocker_and_keys_without_mutating_draft() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    let config = Config {
        ascii: true,
        no_animation: false,
        theme: Theme::Dark,
        color: ColorMode::Mono,
        images: ImageMode::Off,
        assets_dir: "unused".into(),
        layout: LayoutMode::Sanctuary,
        layout_explicit: false,
        data_dir: "unused-data".into(),
        storage_policy: crate::store::Policy::default(),
        context_exclusions: Vec::new(),
        capture_capacity: 64 * 1024,
    };
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        app.input = "남겨 둔 초안\nlocal draft".into();
        app.cursor = app.input.len();
        app.editing = true;
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        )));
        let popup = app.command_popup.as_mut().unwrap();
        popup.query = "storage-retry".into();
        popup.selected = Some(0);
        let paint = Paint::new(&config, &FORMS[0], 0);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                let layout = shell(area, false, Pane::Workspace);
                composer_input(frame, layout.composer, &app, &config, &paint);
                composer::commands(frame, area, &app, &config, &paint);
            })
            .unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        for expected in [
            "/storage-retry",
            "Unavailable:",
            "No failed task save to retry.",
            "Esc Back",
            "1-1/1",
        ] {
            assert!(rendered.contains(expected), "{width}x{height}: {expected}");
        }
        assert_eq!(app.input, "남겨 둔 초안\nlocal draft");
        assert_eq!(app.cursor, app.input.len());
    }
}

fn home_config() -> Config {
    Config {
        ascii: true,
        no_animation: true,
        theme: Theme::Dark,
        color: ColorMode::Mono,
        images: ImageMode::Off,
        assets_dir: "unused".into(),
        layout: LayoutMode::Sanctuary,
        layout_explicit: false,
        data_dir: "unused-data".into(),
        storage_policy: crate::store::Policy::default(),
        context_exclusions: Vec::new(),
        capture_capacity: 64 * 1024,
    }
}

fn screen_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn model_picker_keeps_complete_source_and_update_remedy_inspectable_without_verbose() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        app.planning.catalogues.insert(crate::providers::Agent::Codex, crate::providers::ModelCatalogue {
            models: vec![crate::providers::ModelOption {
                id: "native-new-model".into(), name: "Native model".into(),
                description: String::new(), efforts: vec!["max".into()],
                default_effort: Some("max".into()),
            }],
            current: Some("native-new-model".into()),
            source: "CLI 0.162.1 at /official/bin/codex. Configured provider: native. Catalogue freshness unknown. Update Codex explicitly in Connect, then Refresh.".into(),
            effort_note: None,
        });
        app.command("model");
        VERBOSE.set(false);
        let paint = Paint::new(&config, &FORMS[0], 0);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                planning::render(frame, area, &mut app, &config, &paint);
            })
            .unwrap();
        let max = app.planning.scroll_max;
        let mut screens = screen_text(&terminal);
        for scroll in 1..=max {
            app.planning.scroll = scroll;
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    planning::render(frame, area, &mut app, &config, &paint);
                })
                .unwrap();
            screens.push_str(&screen_text(&terminal));
        }
        for text in [
            "native-new-model",
            "0.162.1",
            "/official/bin/codex",
            "Configured",
            "explicitly",
            "Refresh",
        ] {
            assert!(screens.contains(text), "{width}x{height}: {text}");
        }
        assert!(app.planning.pending.is_none());
    }
}

#[test]
fn centered_attachment_picker_keeps_scrolled_selection_and_help_visible() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        app.task_id = 1;
        app.editing = true;
        app.editor_field = EditorField::Request;
        app.input = "@".into();
        app.cursor = 1;
        app.sources.files = (0..30)
            .map(|index| crate::workspace::context::Source {
                path: format!("context-{index:02}.rs").into(),
                bytes: 1,
                sha256: [0; 32],
            })
            .collect();
        app.composer_menu = Some(crate::app::composer::Menu {
            kind: crate::app::composer::Kind::Files,
            start: 0,
            selected: 22,
        });
        let paint = Paint::new(&config, &FORMS[0], 0);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                let area = composer_menu_area(frame.area(), &app).unwrap();
                composer::render(frame, area, &app, &config, &paint);
            })
            .unwrap();
        let rendered = screen_text(&terminal);
        for expected in [
            "> \"context-22.rs\"",
            "/30 files",
            "Enter Inspect",
            "Esc Keep text",
        ] {
            assert!(rendered.contains(expected), "{width}x{height}: {expected}");
        }
        assert_eq!(app.input, "@");
        assert_eq!(app.composer_menu.unwrap().selected, 22);
    }
}

#[test]
fn global_confirmation_exposes_shortcuts_only_with_visible_scope() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        app.confirmation = Some(crate::app::Confirmation {
            kind: ConfirmationKind::DiscardDraft,
            destructive: false,
        });
        let mut artwork = Artwork::load(&config);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        let rendered = screen_text(&terminal);
        for expected in [
            "Applied task data is kept.",
            "[n] Keep editing",
            "[y] Discard draft",
            "Enter Select",
        ] {
            assert!(rendered.contains(expected), "{width}x{height}: {expected}");
        }
        assert!(app.confirmation_reviewed);
        assert!(
            !app.confirmation.unwrap().destructive,
            "render never selects consent"
        );
        let mut tiny = Terminal::new(TestBackend::new(20, 6)).unwrap();
        tiny.draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        assert!(
            !app.confirmation_reviewed,
            "clipped scope invalidates consent"
        );
    }
}

#[test]
fn first_use_invites_request_without_a_goal_prerequisite() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        app.editing = true;
        app.editor_field = EditorField::Request;
        let mut artwork = Artwork::load(&config);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        let rendered = screen_text(&terminal);
        for expected in [
            "Request",
            "What would you like to do?",
            "Enter Review",
            "Responses appear here",
        ] {
            assert!(rendered.contains(expected), "{width}x{height}: {expected}");
        }
        assert!(!rendered.contains("Enter Save"));
        assert!(app.goal.is_empty());
    }
}

#[test]
fn provider_response_and_request_share_home_at_supported_sizes() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        app.editing = true;
        app.editor_field = EditorField::Request;
        app.input = "Follow-up request remains editable".into();
        app.cursor = app.input.len();
        app.planning.items.push(crate::app::planning::Item {
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "item".into(),
            kind: crate::providers::ItemKind::Message,
            text: "The actual provider response is here.".into(),
            complete: true,
            request: "Original request".into(),
        });
        app.planning.following = false;
        app.planning.new_output = true;
        let mut artwork = Artwork::load(&config);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        let rendered = screen_text(&terminal);
        for expected in [
            "The actual provider response is here.",
            "Follow-up request remains editable",
            "Enter Review",
        ] {
            assert!(rendered.contains(expected), "{width}x{height}: {expected}");
        }
        assert!(app.overlay.is_none());
        assert!(
            !app.planning.following,
            "render must not resume paused following"
        );
        assert!(app.planning.new_output, "render must retain unread output");
    }
}

#[test]
fn image_review_has_immediate_neutral_actions_and_visible_effects() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        app.overlay = Some(Overlay::Images);
        app.images_notice = "Long inspectable technical detail. ".repeat(100);
        let mut artwork = Artwork::load(&config);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        let rendered = screen_text(&terminal);
        for expected in [
            "raw.githubusercontent.com",
            "Writes image cache only",
            "> Back",
            "Download missing",
            "Enter Select",
            "PgUp/Dn Details",
        ] {
            assert!(rendered.contains(expected), "{width}x{height}: {expected}");
        }
        assert!(
            app.images_reviewed,
            "visible scope, not end scrolling, gates consent"
        );
        assert_eq!(app.images_selected, 0);
        assert_eq!(app.scroll_for(ScrollSurface::Preview), 0);
    }
}

#[test]
fn brief_keeps_the_first_sentence_unless_verbose() {
    VERBOSE.set(false);
    assert_eq!(brief("Saved. Nothing was sent."), "Saved. …");
    assert_eq!(brief("Version 0.152.1 ready"), "Version 0.152.1 ready");
    assert_eq!(brief("Commands\n\nLong detail."), "Commands …");
    assert_eq!(brief("One sentence. "), "One sentence. ");
    assert_eq!(
        brief("Failed. Why it failed. Next: try again. More."),
        "Failed. Next: try again. …"
    );
    VERBOSE.set(true);
    assert_eq!(
        brief("Saved. Nothing was sent."),
        "Saved. Nothing was sent."
    );
    VERBOSE.set(false);
}

#[test]
fn long_memory_review_uses_explicit_shortcuts_without_a_scroll_gate() {
    use crate::workspace::memory::{Mutation, Scope};
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    let config = home_config();
    let paint = Paint::new(&config, &FORMS[0], 0);
    for (width, height) in [(60, 18), (120, 36)] {
        let mut state = crate::app::memory::State::new();
        state.reset(7, "/project");
        state.command("create-memory", true);
        state.paste(&"Keep changes scoped.\n".repeat(100));
        state.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE), true);
        assert!(state.draft.ends_with('y'));
        assert!(state.pending.is_none());
        state.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), true);
        state.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE), true);
        assert!(
            state.pending.is_none(),
            "unrendered scope cannot be approved"
        );
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        VERBOSE.set(true);
        terminal
            .draw(|frame| {
                memory::render(frame, frame.area(), &mut state, &config, &paint);
            })
            .unwrap();
        assert!(state.reviewed && state.scroll == 0 && state.scroll_max > 0);
        for event in [
            KeyEvent::new_with_kind(KeyCode::Char('y'), KeyModifiers::NONE, KeyEventKind::Repeat),
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
        ] {
            state.key(event, true);
            assert!(state.pending.is_none());
        }
        assert!(!state.paste("y"));
        state.command("memory-scope", true);
        state.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE), true);
        assert!(
            state.pending.is_none(),
            "changed scope needs a fresh visible review"
        );
        terminal
            .draw(|frame| {
                memory::render(frame, frame.area(), &mut state, &config, &paint);
            })
            .unwrap();
        state.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE), true);
        assert!(matches!(&state.pending, Some(Mutation::Approve(proposal))
            if proposal.scope == Scope::Project && proposal.content.ends_with('y')));
        VERBOSE.set(false);
    }
}

#[test]
fn source_purge_requires_a_current_visible_scope_after_resize() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    let config = home_config();
    let mut artwork = Artwork::load(&config);
    let mut app = App::new("/workspace".into(), 120, 36);
    app.task_id = 7;
    app.overlay = Some(Overlay::Sources);
    app.sources.mode = crate::app::sources::Mode::Basket;
    app.sources.basket.push(crate::workspace::context::Excerpt {
        id: 91,
        task_id: 7,
        root: "/workspace".into(),
        path: format!("src/{}/note.txt", "한".repeat(100)),
        start_line: 1,
        end_line: 1,
        source_sha256: [9; 32],
        text: "retained source".into(),
        available: true,
    });
    app.sources.command("purge", 7, None);
    let approve = Event::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    app.update(approve.clone());
    assert!(app.sources.pending_store.is_none());
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal
        .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
        .unwrap();
    assert!(app.sources.confirmation.unwrap().reviewed);
    app.update(Event::Resize(39, 10));
    app.update(approve.clone());
    assert!(
        app.sources.pending_store.is_none(),
        "hidden scope must not authorize deletion"
    );
    app.update(Event::Resize(60, 18));
    app.update(approve.clone());
    assert!(
        app.sources.pending_store.is_none(),
        "resizing back is not a fresh review"
    );
    let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
    terminal
        .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
        .unwrap();
    assert!(app.sources.confirmation.unwrap().reviewed);
    app.update(Event::Paste("y".into()));
    app.update(Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
        KeyEventKind::Repeat,
    )));
    app.update(Event::Key(KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::CONTROL,
    )));
    assert!(app.sources.pending_store.is_none());
    app.update(approve);
    assert!(matches!(&app.sources.pending_store,
        Some(crate::store::ContextRequest {
            task_id: 7,
            operation: crate::store::ContextOperation::Purge(ids),
        }) if ids == &[91]));
}

#[test]
fn reset_and_init_reviews_keep_scope_and_neutral_actions_visible() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for (width, height) in [(120, 36), (60, 18)] {
        for (kind, title, keep, affirmative) in [
            (
                ConfirmationKind::ResetTask,
                "Reset local task",
                "[n] Stay",
                "[y] Save and reset",
            ),
            (
                ConfirmationKind::ReplaceWithInit,
                "Prepare /init request",
                "[n] Keep draft",
                "[y] Replace with /init",
            ),
        ] {
            let mut app = App::new("/workspace".into(), width, height);
            app.confirmation = Some(crate::app::Confirmation {
                kind,
                destructive: false,
            });
            let mut artwork = Artwork::load(&config);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let rendered = screen_text(&terminal);
            for expected in [title, keep, affirmative, "Enter Select"] {
                assert!(rendered.contains(expected), "{width}x{height}: {expected}");
            }
            assert!(app.confirmation_reviewed, "{width}x{height}: {kind:?}");
            assert!(!app.confirmation.unwrap().destructive);
            let mut tiny = Terminal::new(TestBackend::new(20, 6)).unwrap();
            tiny.draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            assert!(!app.confirmation_reviewed);
        }
    }
}
