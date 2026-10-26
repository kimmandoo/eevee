use super::*;
use crate::config::{ImageMode, LayoutMode};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

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
fn project_header_escapes_controls_without_changing_task_identity() {
    use ratatui::{Terminal, backend::TestBackend};
    let project = "/workspace/한글\nfolder\u{202e}".to_owned();
    let app = App::new(project.clone(), 120, 36);
    let config = home_config();
    let paint = Paint::new(&config, &FORMS[0], 0);
    let mut terminal = Terminal::new(TestBackend::new(120, 3)).unwrap();
    terminal
        .draw(|frame| header(frame, frame.area(), &app, &config, &paint))
        .unwrap();
    let rendered = screen_text(&terminal);
    assert!(rendered.contains("한글\\nfolder"), "{rendered}");
    assert!(!rendered.contains('\u{202e}'));
    assert_eq!(app.cwd, project);
}

#[test]
fn all_true_color_forms_keep_readable_text_and_selection() {
    for theme in [Theme::Dark, Theme::Light] {
        let config = Config {
            language: crate::i18n::Language::ENGLISH,
            ascii: false,
            no_animation: false,
            animation_override: None,
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
fn crlf_is_one_visible_break_with_exact_grapheme_byte_carets() {
    let input = "한\r\ne\u{301}\r\n";
    let (rows, caret) = editor_rows(input, input.len(), 8);
    assert_eq!(rows, ["한", "e\u{301}", ""]);
    assert_eq!(caret, (2, 0));
    assert_eq!(editor_rows(input, "한".len(), 8).1, (0, 2));
    assert_eq!(editor_rows(input, "한\r\n".len(), 8).1, (1, 0));
    assert_eq!(editor_rows(input, "한\r\ne\u{301}".len(), 8).1, (1, 1));
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
        language: crate::i18n::Language::ENGLISH,
        ascii: true,
        no_animation: false,
        animation_override: None,
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
        language: crate::i18n::Language::ENGLISH,
        ascii: true,
        no_animation: true,
        animation_override: Some(false),
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
    let buffer = terminal.backend().buffer();
    let mut text = String::with_capacity(buffer.content.len());
    for row in buffer.content.chunks(usize::from(buffer.area.width).max(1)) {
        let mut column = 0;
        while column < row.len() {
            let symbol = row[column].symbol();
            text.push_str(symbol);
            // Continuation cells of a wide glyph are not independently painted.
            column += ratatui::text::Span::raw(symbol).width().max(1);
        }
    }
    text
}

#[test]
fn model_picker_keeps_focus_current_and_optional_provenance_distinct() {
    use ratatui::{Terminal, backend::TestBackend};
    for color in [ColorMode::Mono, ColorMode::TrueColor] {
        let mut config = home_config();
        config.color = color;
        for (width, height) in [(120, 36), (60, 18)] {
            let mut app = App::new("/workspace".into(), width, height);
            app.planning.catalogues.insert(
                crate::providers::Agent::Codex,
                crate::providers::ModelCatalogue {
                    models: vec![crate::providers::ModelOption {
                        id: "native-new-model".into(),
                        name: "native-new-model".into(),
                        description: String::new(),
                        efforts: vec!["max".into()],
                        default_effort: Some("max".into()),
                    }],
                    current: Some("native-new-model".into()),
                    source: "SOURCE_SENTINEL · Update the provider explicitly, then Refresh."
                        .into(),
                    effort_note: None,
                },
            );
            app.planning.selection = crate::providers::ModelSelection {
                model: Some("native-new-model".into()),
                effort: Some("max".into()),
            };
            app.command("model");
            app.update(Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)));
            let paint = Paint::new(&config, &FORMS[0], 0);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    planning::render(frame, area, &mut app, &config, &paint);
                })
                .unwrap();
            let rendered = screen_text(&terminal);
            assert!(rendered.contains(">  Default"));
            assert!(rendered.contains(" * native-new-model"));
            assert!(rendered.contains("native-new-model / max"));
            assert!(!rendered.contains("native-new-model · native-new-model"));
            assert!(!rendered.contains("SOURCE_SENTINEL"));
            let cells = &terminal.backend().buffer().content;
            let focused = cells.iter().find(|cell| cell.symbol() == ">").unwrap();
            let committed = cells.iter().find(|cell| cell.symbol() == "*").unwrap();
            if color == ColorMode::Mono {
                assert!(focused.modifier.contains(Modifier::REVERSED));
                assert!(!committed.modifier.contains(Modifier::REVERSED));
            } else {
                assert_eq!(Some(focused.bg), paint.selected.bg);
                assert_eq!(Some(committed.fg), paint.accent.fg);
                assert_ne!(focused.bg, committed.bg);
            }
            app.update(Event::Key(KeyEvent::new(
                KeyCode::Char('d'),
                KeyModifiers::CONTROL,
            )));
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    planning::render(frame, area, &mut app, &config, &paint);
                })
                .unwrap();
            let max = app.planning.scroll_max;
            let mut inspected = screen_text(&terminal);
            for scroll in 1..=max {
                app.planning.scroll = scroll;
                terminal
                    .draw(|frame| {
                        let area = frame.area();
                        planning::render(frame, area, &mut app, &config, &paint);
                    })
                    .unwrap();
                inspected.push_str(&screen_text(&terminal));
            }
            assert!(inspected.contains("SOURCE_SENTINEL"));
            assert!(inspected.contains("explicitly"));
            assert!(app.planning.pending.is_none());
            assert_eq!(app.planning.selection.effort.as_deref(), Some("max"));
        }
    }
}

#[test]
fn attachment_picker_exposes_saved_state_and_direct_removal_without_mutating_draft() {
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
        app.sources.basket.push(crate::workspace::context::Excerpt {
            origin_run: None,
            id: 91,
            task_id: 1,
            root: "/workspace".into(),
            path: "context-22.rs".into(),
            start_line: 1,
            end_line: 1,
            source_sha256: [0; 32],
            text: "saved capture".into(),
            available: true,
        });
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
        assert!(rendered.contains("[+] \"context-22.rs\""));
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Delete,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.overlay, Some(Overlay::Sources));
        assert_eq!(app.sources.confirmation.as_ref().unwrap().id, 91);
        assert!(app.sources.pending_store.is_none());
        // Opening removal never approves it; neutral Enter keeps the capture.
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(app.sources.pending_store.is_none());
        assert_eq!(app.sources.basket.len(), 1);
        app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert_eq!(app.input, "@");
        assert!(app.editing);
        assert_eq!(app.cursor, 1);
        assert!(app.planning.pending.is_none());
    }
}

#[test]
fn attachment_capture_requires_visible_review_and_fresh_render_after_resize() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    let paint = Paint::new(&config, &FORMS[0], 0);
    let mut app = App::new("/workspace".into(), 120, 36);
    app.task_id = 7;
    app.editing = false;
    app.overlay = Some(Overlay::Sources);
    app.sources.event(
        7,
        crate::workspace::context::Event {
            task_id: 7,
            result: Ok(crate::workspace::context::Response::Inspected(
                crate::workspace::context::Excerpt {
                    origin_run: None,
                    id: 0,
                    task_id: 7,
                    root: "/workspace".into(),
                    path: "sample.rs".into(),
                    start_line: 1,
                    end_line: 1,
                    source_sha256: [7; 32],
                    text: "safe capture\n".into(),
                    available: true,
                },
            )),
        },
    );
    let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.update(enter.clone());
    assert!(app.sources.pending_store.is_none());
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal
        .draw(|frame| sources::render(frame, frame.area(), &mut app, &config, &paint))
        .unwrap();
    assert!(app.sources.preview_reviewed);
    app.update(Event::Resize(39, 10));
    app.update(enter.clone());
    assert!(app.sources.pending_store.is_none());
    app.update(Event::Resize(60, 18));
    app.update(enter.clone());
    assert!(app.sources.pending_store.is_none());
    let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
    terminal
        .draw(|frame| sources::render(frame, frame.area(), &mut app, &config, &paint))
        .unwrap();
    assert!(app.sources.preview_reviewed);
    app.update(enter);
    assert!(matches!(&app.sources.pending_store,
        Some(crate::store::ContextRequest {
            task_id: 7, operation: crate::store::ContextOperation::Pin(capture),
            ..
        }) if capture.text == "safe capture\n"));
    assert!(app.sources.basket.is_empty());
    assert!(app.planning.pending.is_none());
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
            owner: None,
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
fn task_auto_approval_stays_visible_in_the_header_at_every_supported_size() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        let mut artwork = Artwork::load(&config);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let draw = |app: &mut App, terminal: &mut Terminal<TestBackend>, artwork: &mut Artwork| {
            terminal
                .draw(|frame| render(frame, app, &config, artwork, "off"))
                .unwrap();
            screen_text(terminal)
        };
        assert!(!draw(&mut app, &mut terminal, &mut artwork).contains("AUTO-APPROVE"));
        app.planning.auto_approve = true;
        let rendered = draw(&mut app, &mut terminal, &mut artwork);
        assert!(
            rendered.contains("AUTO-APPROVE"),
            "{width}x{height}\n{rendered}"
        );
        if width == 120 {
            assert!(rendered.contains("[code · auto-approve]"), "{rendered}");
        }
    }
}

#[test]
fn suggestion_wraps_without_replacing_response_or_composer() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    let reason = "Latest local run failed with exit code 127; inspect its owned output.";
    for (width, height) in [(120, 36), (60, 18)] {
        for with_response in [false, true] {
            let mut app = App::new("/workspace".into(), width, height);
            app.goal = "Current task".into();
            app.input = "Follow-up draft stays editable".into();
            app.cursor = app.input.len();
            app.editing = true;
            app.suggestion = Some(crate::app::suggestions::Suggestion {
                form: 5,
                reason: reason.into(),
                trigger: "task:7:failed:12".into(),
            });
            if with_response {
                app.planning.items.push(crate::app::planning::Item {
                    thread_id: "thread".into(),
                    turn_id: "turn".into(),
                    item_id: "item".into(),
                    kind: crate::providers::ItemKind::Message,
                    text: "Actual provider response stays visible.".into(),
                    complete: true,
                    request: "Original request".into(),
                    owner: None,
                });
            }
            let mut artwork = Artwork::load(&config);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let rendered = screen_text(&terminal);
            assert_eq!(rendered.matches("Suggested:").count(), 1);
            assert!(rendered.contains("Follow-up draft stays editable"));
            if with_response {
                assert!(rendered.contains("Actual provider response stays visible."));
            } else {
                assert!(rendered.contains("Task: Current task"));
            }
            let area = shell(Rect::new(0, 0, width, height), false, Pane::Workspace).workspace;
            let buffer = terminal.backend().buffer();
            let strip = (area.y..area.y + 2)
                .flat_map(|y| (area.x..area.right()).map(move |x| buffer[(x, y)].symbol()))
                .collect::<String>();
            let normalized = strip.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(
                normalized.contains(&format!("Suggested: Umbreon — {reason}")),
                "{width}x{height}: {normalized}"
            );
            assert_eq!(app.form(), 0, "suggestion never selects a form");
            assert!(app.planning.pending.is_none(), "suggestion never sends");
            assert_eq!(app.input, "Follow-up draft stays editable");
            app.dismissed_suggestion = app.suggestion.as_ref().map(|value| value.trigger.clone());
            terminal
                .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let dismissed = screen_text(&terminal);
            assert!(!dismissed.contains("Suggested:"));
            assert!(dismissed.contains("Follow-up draft stays editable"));
        }
    }
}

#[test]
fn automatic_evolution_keeps_advisory_chrome_out_of_the_response_viewport() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for (width, height) in [(120, 36), (60, 18)] {
        for automatic in [false, true] {
            let mut app = App::new("/workspace".into(), width, height);
            app.store_event(crate::store::Event::Loaded(crate::store::Project {
                tasks: Vec::new(),
                partners: vec![crate::store::Partner {
                    form: 3,
                    auto_form: automatic,
                    ..Default::default()
                }],
                active_id: None,
                usage_bytes: 0,
            }));
            app.suggestion = Some(crate::app::suggestions::Suggestion {
                form: 4,
                reason: "An advisory alternative remains inspectable.".into(),
                trigger: "task:0:criteria".into(),
            });
            app.input = "Retained future draft 한글".into();
            app.cursor = 9;
            let area = Rect::new(0, 0, width, height);
            let paint = Paint::new(&config, &FORMS[app.form()], app.form());
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut response_area = area;
            terminal
                .draw(|frame| response_area = suggestion_strip(frame, area, &app, &paint))
                .unwrap();
            if automatic {
                assert_eq!(
                    response_area, area,
                    "already routed requests need no competing advisory strip"
                );
            } else {
                assert!(
                    response_area.height < area.height,
                    "manual forms retain their advisory strip"
                );
            }
            assert!(
                app.visible_suggestion().is_some(),
                "rendering must not dismiss advisory evidence"
            );
            assert_eq!(app.input, "Retained future draft 한글");
            assert_eq!(app.cursor, 9);
            assert!(app.planning.pending.is_none());
        }
    }
}

#[test]
fn first_file_output_reclaims_compact_rows_without_dismissing_transition_or_draft() {
    use crate::providers::{FilePhase, ItemKind};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::Arc;
    let config = home_config();
    let mut app = App::new("/workspace".into(), 60, 18);
    app.task_id = 7;
    app.storage = crate::app::StorageState::Saved;
    app.input = "Create a Python code file.".into();
    app.cursor = app.input.len();
    app.editing = true;
    app.editor_field = EditorField::Request;
    app.update(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    app.refresh_carry_over();
    assert!(app.evolution.transition.is_some());
    assert!(app.carry_over().is_some());
    app.planning.submitted_request = Arc::from("Create a Python code file.");
    app.planning.items.push(crate::app::planning::Item {
        thread_id: String::new(),
        turn_id: "owned-turn".into(),
        item_id: "file-call".into(),
        kind: ItemKind::File(FilePhase::Generating),
        text: "generated/live.py\n1  STREAM_VISIBLE_WITHOUT_NAVIGATION".into(),
        complete: false,
        request: Arc::clone(&app.planning.submitted_request),
        owner: None,
    });
    app.input = "Future draft 한글".into();
    app.cursor = 7;
    let mut artwork = Artwork::load(&config);
    let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
    for phase in [
        FilePhase::Generating,
        FilePhase::Received,
        FilePhase::AwaitingReview,
    ] {
        app.planning.items[0].kind = ItemKind::File(phase);
        terminal
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        assert!(screen_text(&terminal).contains("STREAM_VISIBLE_WITHOUT_NAVIGATION"));
        assert!(app.evolution.transition.is_some());
        assert!(
            app.carry_over().is_some(),
            "rendering keeps inspectable carry-over evidence"
        );
        assert_eq!(app.input, "Future draft 한글");
        assert_eq!(app.cursor, 7);
    }
    app.reopen_carry_over();
    terminal
        .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
        .unwrap();
    assert!(
        screen_text(&terminal).contains(&app.last_carry_over().unwrap().summary),
        "deliberate carry-over inspection must remain available during file output"
    );
    assert_eq!(app.input, "Future draft 한글");
    assert_eq!(app.cursor, 7);
    app.planning.submitted_request = Arc::from("A different new request");
    assert!(
        !app.planning.has_current_file_output(),
        "old file cards cannot hide a new request's context"
    );
}

fn carry_over_home(width: u16, height: u16) -> App {
    use crate::workspace::{
        context::Excerpt,
        workflows::{Finding, Hypothesis},
    };

    let mut app = App::new("/workspace".into(), width, height);
    app.task_id = 7;
    app.command("jolteon");
    app.refresh_carry_over();
    app.criteria = (0..64)
        .map(|index| store::Criterion {
            text: format!("Criterion {index}"),
            met: index < 32,
        })
        .collect();
    app.sources.basket = (1..=16)
        .map(|id| Excerpt {
            origin_run: None,
            id,
            task_id: 7,
            root: "/workspace".into(),
            path: format!("source-{id}.rs"),
            start_line: 1,
            end_line: 1,
            source_sha256: [0; 32],
            text: "Committed evidence".into(),
            available: true,
        })
        .collect();
    app.workflows.hypotheses = (1..=32)
        .map(|id| Hypothesis {
            id,
            explanation: format!("Open explanation {id}"),
            evidence: Vec::new(),
        })
        .collect();
    app.workflows.findings = (1..=32)
        .map(|id| Finding {
            id,
            criterion: "Criterion 0".into(),
            detail: format!("Open finding {id}"),
            next_action: "Inspect".into(),
            evidence: Vec::new(),
            resolution: None,
        })
        .collect();
    app.input = "Retained follow-up draft".into();
    app.cursor = 8;
    app.editing = true;
    app.editor_field = EditorField::Request;
    app.planning.items.push(crate::app::planning::Item {
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: "item".into(),
        kind: crate::providers::ItemKind::Message,
        text: "Retained provider response.".into(),
        complete: true,
        request: "Original request".into(),
        owner: None,
    });
    app.planning.following = false;
    app.planning.new_output = true;
    app.command("sylveon");
    app.refresh_carry_over();
    app
}

#[test]
fn carry_over_counts_and_capabilities_share_one_row_with_response_and_draft() {
    use ratatui::{Terminal, backend::TestBackend};

    for (width, height) in [(60, 18), (120, 36)] {
        for (ascii, color, linear) in [
            (false, ColorMode::TrueColor, false),
            (true, ColorMode::Mono, false),
            (true, ColorMode::Mono, true),
        ] {
            let mut config = home_config();
            config.ascii = ascii;
            config.color = color;
            let mut app = carry_over_home(width, height);
            app.linear = linear;
            let focus = app.effective_focus();
            let mut artwork = Artwork::load(&config);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let area = shell(Rect::new(0, 0, width, height), linear, focus).workspace;
            for _ in 0..2 {
                terminal
                    .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                    .unwrap();
                if app.evolution.popup_shown && app.evolution.transition.is_some() {
                    app.update(Event::Key(KeyEvent::new(
                        KeyCode::F(12),
                        KeyModifiers::NONE,
                    )));
                    app.reopen_carry_over();
                    terminal
                        .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                        .unwrap();
                }
                let buffer = terminal.backend().buffer();
                let strip = (area.x..area.right())
                    .map(|x| buffer[(x, area.y)].symbol())
                    .collect::<String>();
                for expected in ["Carry", "C32/64", "P16", "H32", "F32", "-run"] {
                    assert!(strip.contains(expected), "{width}x{height}: {strip}");
                }
                let rendered = screen_text(&terminal);
                assert!(rendered.contains("Retained provider response."));
                assert!(rendered.contains("Retained follow-up draft"));
                assert_eq!(app.effective_focus(), focus);
                assert!(app.editing);
                assert_eq!(app.editor_field, EditorField::Request);
                assert_eq!(app.input, "Retained follow-up draft");
                assert_eq!(app.cursor, 8);
                assert!(!app.planning.following);
                assert!(app.planning.new_output);
                assert!(app.planning.pending.is_none());
                assert!(app.sources.pending_store.is_none());
                assert!(app.carry_over().is_some(), "render must not dismiss");
            }

            let paint = Paint::new(&config, &FORMS[app.form()], app.form());
            let mut visible_area = area;
            terminal
                .draw(|frame| visible_area = suggestion_strip(frame, area, &app, &paint))
                .unwrap();
            assert_eq!(visible_area.y, area.y + 1);
            assert_eq!(visible_area.bottom(), area.bottom());
            app.update(Event::Key(KeyEvent::new(
                KeyCode::Right,
                KeyModifiers::NONE,
            )));
            assert!(app.carry_over().is_none());
            assert_eq!(app.cursor, 9, "dismissal must deliver the original key");
            let mut restored_area = visible_area;
            terminal
                .draw(|frame| restored_area = suggestion_strip(frame, area, &app, &paint))
                .unwrap();
            assert_eq!(restored_area, area, "hidden strip reserves no row");
        }
    }
}

#[test]
fn companion_explains_and_reopens_carry_over_without_replacing_request() {
    use ratatui::{Terminal, backend::TestBackend};

    let config = home_config();
    for (width, height) in [(60, 18), (120, 36)] {
        let mut app = carry_over_home(width, height);
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::NONE,
        )));
        app.command("companion");
        let mut artwork = Artwork::load(&config);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        let rendered = screen_text(&terminal);
        for expected in [
            "c Carry-over",
            "C=criteria met/total",
            "P=committed pins",
            "H=open hypotheses",
            "F=open findings",
        ] {
            assert!(rendered.contains(expected), "{width}x{height}: {expected}");
        }
        assert!(app.carry_over().is_none());
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::NONE,
        )));
        assert!(app.overlay.is_none());
        assert!(app.carry_over().is_some());
        terminal
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        let rendered = screen_text(&terminal);
        assert!(rendered.contains("Carry C32/64"));
        assert!(rendered.contains("Retained provider response."));
        assert!(rendered.contains("Retained follow-up draft"));
        assert_eq!(app.input, "Retained follow-up draft");
        assert_eq!(app.cursor, 9);
        assert!(app.editing);
        assert_eq!(app.editor_field, EditorField::Request);
        assert!(app.planning.pending.is_none());
        assert!(app.sources.pending_store.is_none());
    }
}

#[test]
fn task_card_suggestion_belongs_only_to_active_task() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        app.task_id = 7;
        app.goal = "Active task".into();
        app.tasks.push(app.snapshot("/workspace"));
        let mut other = app.snapshot("/workspace");
        other.id = 8;
        other.goal = "Other task".into();
        app.tasks.push(other);
        app.suggestion = Some(crate::app::suggestions::Suggestion {
            form: 4,
            reason: "Goal has no acceptance criteria.".into(),
            trigger: "task:7:criteria".into(),
        });
        app.overlay = Some(Overlay::Tasks);
        let mut artwork = Artwork::load(&config);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        for selected in [0, 1] {
            app.task_selection = selected;
            terminal
                .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let rendered = screen_text(&terminal);
            assert_eq!(
                rendered.matches("Suggested:").count(),
                usize::from(selected == 0),
                "{width}x{height}: selected {selected}"
            );
            if selected == 0 {
                assert!(rendered.contains("Suggested: Espeon"));
                assert!(rendered.contains("acceptance criteria."));
            }
        }
        app.task_selection = 0;
        app.dismissed_suggestion = app.suggestion.as_ref().map(|value| value.trigger.clone());
        terminal
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        assert!(!screen_text(&terminal).contains("Suggested:"));
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
        origin_run: None,
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
            ..
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

#[test]
fn header_reserves_model_and_effort_space_in_brief_and_verbose_modes() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for width in [60, 120] {
        for verbose in [false, true] {
            VERBOSE.set(verbose);
            let mut app = App::new(
                "/workspace/a long project name with 東京 and additional context".into(),
                width,
                18,
            );
            app.task_id = 123456;
            app.command("vaporeon");
            app.planning.agent = crate::providers::Agent::ChatGPT;
            app.planning.selection = crate::providers::ModelSelection {
                model: Some("gpt-6.1-sol".into()),
                effort: Some("high".into()),
            };
            let paint = Paint::new(&config, &FORMS[app.form()], app.form());
            let mut terminal = Terminal::new(TestBackend::new(width, 2)).unwrap();
            terminal
                .draw(|frame| header(frame, frame.area(), &app, &config, &paint))
                .unwrap();
            let first_row: String = (0..width)
                .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
                .collect();
            assert!(first_row.contains("gpt-6.1-sol"), "{first_row}");
            assert!(first_row.contains("high"), "{first_row}");
        }
    }
    VERBOSE.set(false);
}

#[test]
fn header_keeps_owner_storage_and_failures_readable_at_supported_sizes() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for (width, height) in [(120, 36), (60, 18)] {
        for form in FORMS.iter() {
            let mut app = App::new("/workspace".into(), width, height);
            app.store_event(crate::store::Event::Loaded(crate::store::Project {
                tasks: Vec::new(),
                partners: vec![crate::store::Partner::default()],
                active_id: None,
                usage_bytes: 0,
            }));
            app.command(&form.name.to_lowercase());
            app.storage = crate::app::StorageState::Saved;
            let mut artwork = Artwork::load(&config);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let line = |y: u16| -> String { (0..width).map(|x| buffer[(x, y)].symbol()).collect() };
            let (names, status) = (line(0), line(1));
            assert!(names.contains(form.name), "{width}: {names}");
            assert!(status.contains("Storage: saved"), "{status}");
        }
    }
    // Higher-priority activity text keeps its full width; the chip yields.
    let mut app = App::new("/workspace".into(), 60, 18);
    app.command("jolteon");
    app.planning.request = None;
    app.storage = crate::app::StorageState::Failed("disk".into());
    let mut artwork = Artwork::load(&config);
    let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
    terminal
        .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let status: String = (0..60).map(|x| buffer[(x, 1)].symbol()).collect();
    assert!(!status.contains("[read"), "{status}");
}

#[test]
fn nicknames_keep_species_visible_in_header_and_composer_at_both_sizes() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for (width, height) in [(120, 36), (60, 18)] {
        for form in FORMS {
            for nickname_value in ["月", "界界界界界界界界界界界界"] {
                let mut app = App::new("/workspace".into(), width, height);
                app.command(&form.name.to_lowercase());
                app.command("nickname");
                app.update(Event::Paste(nickname_value.into()));
                app.update(Event::Key(KeyEvent::new(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                )));
                app.party_request.take();
                app.store_event(store::Event::Party(Ok(vec![store::Partner {
                    nickname: Some(nickname_value.into()),
                    ..store::Partner::default()
                }])));
                app.planning.selection.model = Some("model-sentinel".into());
                app.planning.selection.effort = Some("high".into());
                let mut artwork = Artwork::load(&config);
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                    .unwrap();
                if app.evolution.popup_shown && app.evolution.transition.is_some() {
                    app.update(Event::Key(KeyEvent::new(
                        KeyCode::F(12),
                        KeyModifiers::NONE,
                    )));
                    terminal
                        .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                        .unwrap();
                }
                let buffer = terminal.backend().buffer();
                let line =
                    |y: u16| -> String { (0..width).map(|x| buffer[(x, y)].symbol()).collect() };
                let header = line(0);
                assert!(header.contains("model-sentinel"), "{width}: {header}");
                assert!(header.contains("high"), "{width}: {header}");
                if width < 90 {
                    assert!(header.contains("Codex/CLI"), "{width}: {header}");
                }
                let composer = line(
                    shell(
                        Rect::new(0, 0, width, height),
                        app.linear,
                        app.effective_focus(),
                    )
                    .composer
                    .y,
                );
                for visible in [&header, &composer] {
                    assert!(
                        visible.contains(&format!("({})", form.name)),
                        "{width}: {visible}"
                    );
                    assert!(
                        visible.contains(nickname_value.chars().next().unwrap()),
                        "{width}: {visible}"
                    );
                }
                assert!(
                    screen_text(&terminal).contains(form.element),
                    "{width}: {}",
                    form.name
                );
            }
        }
    }
}

#[test]
fn nickname_editor_exposes_local_scope_candidate_and_rejection() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        app.command("nickname");
        app.update(Event::Paste("月".into()));
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        app.party_request.take();
        app.store_event(store::Event::Party(Err("Duplicate nickname".into())));
        let mut artwork = Artwork::load(&config);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        let rendered = screen_text(&terminal);
        for boundary in [
            "local only",
            "0 B",
            "月",
            "Duplicate",
            "Enter Save",
            "Esc Cancel",
        ] {
            assert!(rendered.contains(boundary), "{width}: {rendered}");
        }
    }
}

#[test]
fn picker_marks_suggestion_textually_without_changing_selection_or_geometry() {
    use ratatui::{Terminal, backend::TestBackend};
    for color in [ColorMode::Mono, ColorMode::TrueColor] {
        let mut config = home_config();
        config.color = color;
        for (width, height, linear) in [
            (120, 36, false),
            (90, 30, false),
            (80, 24, false),
            (60, 18, false),
            (120, 36, true),
        ] {
            let mut app = App::new("/workspace".into(), width, height);
            app.editing = false;
            app.overlay = Some(Overlay::Forms);
            app.selected = 4;
            app.linear = linear;
            app.suggestion = Some(crate::app::suggestions::Suggestion {
                form: 5,
                reason: "Latest local run failed.".into(),
                trigger: format!("task:0/failed:{}", "a".repeat(64)),
            });
            let area = Rect::new(0, 0, width, height);
            let mut targets_before = Vec::new();
            portrait_targets(&app, area, &mut targets_before);
            let mut artwork = Artwork::load(&config);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let rendered = screen_text(&terminal);
            assert!(
                rendered.to_ascii_lowercase().contains("suggested"),
                "{rendered}"
            );
            assert!(rendered.contains("Umbreon"), "{rendered}");
            assert!(!rendered.contains("Latest local run failed."));
            assert!(rendered.contains("Current: Eevee"));
            assert!(rendered.contains("Return to Eevee / intake"));
            assert_eq!(app.selected, 4);
            assert_eq!(app.form(), 0);
            app.form_return_selected = true;
            app.update(Event::Key(KeyEvent::new(
                KeyCode::Char('d'),
                KeyModifiers::NONE,
            )));
            assert_eq!(app.selected, 4);
            assert!(app.form_return_selected);
            assert_eq!(app.form(), 0);
            let mut targets_after = Vec::new();
            portrait_targets(&app, area, &mut targets_after);
            assert_eq!(
                targets_before, targets_after,
                "native portraits retain their rectangles"
            );
            terminal
                .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let dismissed = screen_text(&terminal);
            assert!(!dismissed.contains("suggested"));
            assert!(!dismissed.contains("d Dismiss"));
            app.form_return_selected = false;
            app.update(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )));
            assert_eq!(app.form(), 4, "Enter selects only the explicit selection");
            assert!(app.planning.pending.is_none());
            assert!(app.execution.history.is_empty());
        }
    }
}

#[test]
fn every_decision_criterion_remains_inspectable_before_confirmation() {
    use crate::workspace::workflows::ChosenDecision;
    use ratatui::{Terminal, backend::TestBackend};
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        app.task_id = 7;
        app.editing = false;
        app.overlay = Some(Overlay::Workflows);
        let decision = ChosenDecision {
            id: "decision:scroll".into(),
            text: "Chosen design".into(),
        };
        app.workflows.chosen_decision = Some(decision.clone());
        app.workflow.criteria_decision = Some(decision);
        app.workflow.criteria_text = (1..=40)
            .map(|n| format!("User criterion {n:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.workflow.criteria_preview = true;
        app.workflow.preview = true;
        let config = home_config();
        let paint = Paint::new(&config, &FORMS[0], 0);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| super::workflows::render(frame, frame.area(), &mut app, &config, &paint))
            .unwrap();
        assert!(!screen_text(&terminal).contains("User criterion 40"));
        for _ in 0..40 {
            app.workflow_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
            terminal
                .draw(|frame| {
                    super::workflows::render(frame, frame.area(), &mut app, &config, &paint)
                })
                .unwrap();
        }
        assert!(
            screen_text(&terminal).contains("User criterion 40"),
            "{width}x{height}: every proposed item must be inspectable"
        );
        assert!(app.criteria.is_empty(), "inspection is not confirmation");
        assert!(app.planning.pending.is_none());
    }
}

#[test]
fn specialist_paging_does_not_skip_saved_hypothesis_lines() {
    use ratatui::{Terminal, backend::TestBackend};
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        app.task_id = 7;
        app.editing = false;
        app.overlay = Some(Overlay::Workflows);
        let lines = (1..=20)
            .map(|n| format!("User hypothesis observation {n:02}"))
            .collect::<Vec<_>>();
        app.workflows.add_hypothesis(&lines.join("\n")).unwrap();
        let config = home_config();
        let paint = Paint::new(&config, &FORMS[0], 0);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut seen = [false; 20];
        for _ in 0..24 {
            terminal
                .draw(|frame| {
                    super::workflows::render(frame, frame.area(), &mut app, &config, &paint)
                })
                .unwrap();
            let visible = screen_text(&terminal);
            for (line, seen) in lines.iter().zip(&mut seen) {
                *seen |= visible.contains(line);
            }
            app.workflow_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        }
        assert!(
            seen.iter().all(|seen| *seen),
            "{width}x{height}: paging skipped saved user lines: {seen:?}"
        );
    }
}

fn cooperation_roster(width: u16, height: u16) -> App {
    let mut app = App::new("/workspace".into(), width, height);
    app.store_event(crate::store::Event::Loaded(crate::store::Project {
        tasks: Vec::new(),
        partners: vec![
            crate::store::Partner {
                nickname: Some("Bolt".into()),
                form: 2,
                ..Default::default()
            },
            crate::store::Partner {
                id: 2,
                nickname: Some("Mizu".into()),
                form: 1,
                ..Default::default()
            },
            crate::store::Partner {
                id: 3,
                nickname: Some("Shade".into()),
                form: 5,
                ..Default::default()
            },
        ],
        active_id: None,
        usage_bytes: 0,
    }));
    app
}

#[test]
fn release_review_requires_visible_explicit_consent_and_preserves_refused_identity() {
    use ratatui::{Terminal, backend::TestBackend};
    for (width, height) in [(120, 36), (60, 18)] {
        let config = home_config();
        let mut artwork = Artwork::load(&config);
        let mut app = cooperation_roster(width, height);
        app.input = "unsent future draft 한글".into();
        app.cursor = "unsent future ".len();
        app.editing = true;
        app.editor_field = crate::store::EditorField::Request;
        let draft = (app.input.clone(), app.cursor);
        app.command("party");
        let retained_request = |app: &App| {
            app.drafts
                .iter()
                .find(|entry| entry.field == crate::store::EditorField::Request)
                .map(|entry| (entry.text.clone(), entry.cursor))
        };
        app.selected = 1;
        app.command("release-partner");
        assert_eq!(
            app.cooperation_ui.release.as_ref().unwrap().owner.partner,
            2
        );
        // Opening a preview or typing y before its scope is rendered grants nothing.
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
        )));
        assert!(app.party_request.is_none());
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        let shown = screen_text(&terminal);
        let disclosure = inner(centered(Rect::new(0, 0, width, height), 92, 24));
        let buffer = terminal.backend().buffer();
        let disclosure = (disclosure.y..disclosure.bottom())
            .flat_map(|y| (disclosure.x..disclosure.right()).map(move |x| buffer[(x, y)].symbol()))
            .collect::<String>();
        let disclosure = disclosure.split_whitespace().collect::<Vec<_>>().join(" ");
        for phrase in [
            "Mizu (Vaporeon)",
            "Remove:",
            "Retain:",
            "name/form attribution",
            "not stored",
            "Stay",
            "Storage:",
        ] {
            assert!(disclosure.contains(phrase), "{phrase}: {disclosure}");
        }
        assert!(
            app.cooperation_ui.release.as_ref().unwrap().reviewed,
            "{shown}"
        );
        assert!(!app.cooperation_ui.release.as_ref().unwrap().affirmative);
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::NONE,
        )));
        assert!(app.cooperation_ui.release.as_ref().unwrap().affirmative);
        for modifiers in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SHIFT,
        ] {
            for code in [KeyCode::Char('y'), KeyCode::Enter] {
                app.update(Event::Key(KeyEvent::new(code, modifiers)));
                assert!(
                    app.party_request.is_none(),
                    "{code:?} with {modifiers:?} must not consent"
                );
                assert!(app.cooperation_ui.release.is_some());
            }
        }
        for kind in [
            crossterm::event::KeyEventKind::Repeat,
            crossterm::event::KeyEventKind::Release,
        ] {
            for code in [KeyCode::Char('y'), KeyCode::Enter] {
                app.update(Event::Key(KeyEvent::new_with_kind(
                    code,
                    KeyModifiers::NONE,
                    kind,
                )));
                assert!(
                    app.party_request.is_none(),
                    "{code:?} {kind:?} must not consent"
                );
            }
        }
        app.update(Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)));
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(app.cooperation_ui.release.is_none());
        assert!(app.party_request.is_none());
        assert_eq!(retained_request(&app), Some(draft.clone()));
        app.command("release-partner");
        terminal
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
        )));
        assert_eq!(
            app.party_request,
            Some(crate::store::PartyRequest::Release { partner: 2 })
        );
        app.party_request = None;
        app.store_event(crate::store::Event::PartyReleased(Err(
            "disk full: roster retained".into(),
        )));
        let refused = app.cooperation_ui.release.as_ref().unwrap();
        assert_eq!(
            (refused.owner.partner, refused.owner.name.as_str()),
            (2, "Mizu")
        );
        assert!(!refused.pending && !refused.affirmative);
        assert_eq!(
            refused.refusal.as_deref(),
            Some("disk full: roster retained")
        );
        assert_eq!(retained_request(&app), Some(draft.clone()));
        app.update(Event::Resize(59, 17));
        let mut small = Terminal::new(TestBackend::new(59, 17)).unwrap();
        small
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        assert!(!app.cooperation_ui.release.as_ref().unwrap().reviewed);
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
        )));
        assert!(app.party_request.is_none());
        app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(app.cooperation_ui.release.is_none());
        assert_eq!(retained_request(&app), Some(draft.clone()));
    }
}

#[test]
fn last_partner_release_refusal_is_inspectable_without_roster_mutation() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    let mut artwork = Artwork::load(&config);
    let mut app = App::new("/workspace".into(), 60, 18);
    app.store_event(crate::store::Event::Loaded(crate::store::Project {
        tasks: Vec::new(),
        partners: vec![crate::store::Partner::default()],
        active_id: None,
        usage_bytes: 0,
    }));
    app.command("release-partner");
    let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
    terminal
        .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
        .unwrap();
    let shown = screen_text(&terminal);
    assert!(shown.contains("last partner"), "{shown}");
    assert!(shown.contains("Eevee #1 (Eevee)"), "{shown}");
    app.update(Event::Key(KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
    )));
    assert!(app.party_request.is_none());
    assert_eq!(app.partners().len(), 1);
    assert!(app.cooperation_ui.release.is_some());
}

#[test]
fn cooperation_reviews_withhold_native_images_without_moving_home_artwork() {
    let mut app = cooperation_roster(120, 36);
    app.editing = true;
    let area = Rect::new(0, 0, 120, 36);
    let mut targets = Vec::new();
    portrait_targets(&app, area, &mut targets);
    let original = targets.clone();
    assert!(!original.is_empty());
    app.command("release-partner");
    portrait_targets(&app, area, &mut targets);
    assert!(
        targets.is_empty(),
        "native pictures must not cover release disclosure"
    );
    app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    portrait_targets(&app, area, &mut targets);
    assert_eq!(targets, original);
    app.request_cancel_all();
    portrait_targets(&app, area, &mut targets);
    assert!(
        targets.is_empty(),
        "native pictures must not cover owner scope"
    );
    app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    portrait_targets(&app, area, &mut targets);
    assert_eq!(targets, original);
}

#[test]
fn journal_discard_review_keeps_exact_scope_inspectable_without_hidden_consent() {
    use ratatui::{Terminal, backend::TestBackend};
    for (width, height) in [(120, 36), (60, 18)] {
        let config = home_config();
        let mut artwork = Artwork::load(&config);
        let mut app = App::new("/workspace".into(), width, height);
        app.task_id = 7;
        app.planning = crate::app::planning::State::terminal_failure_for_test(64);
        app.command("journal-discard");
        let review = app.cooperation.journal_discard.as_ref().unwrap();
        assert_eq!(review.count, 64);
        let ids = review.ids.clone();
        let claim = app.planning.work.clone();
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
        )));
        assert!(app.planning.journal_failed());
        assert_eq!(app.planning.work, claim);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        let body_text = |terminal: &Terminal<TestBackend>| screen_text(terminal);
        let summary = body_text(&terminal);
        for scope in ["Bolt (Jolteon)", "provider-test-1", "64"] {
            assert!(summary.contains(scope), "{scope}: {summary}");
        }
        for disclosure in ["committed", "gap", "replay"] {
            assert!(
                summary.to_lowercase().contains(disclosure),
                "{disclosure}: {summary}"
            );
        }
        let review = app.cooperation.journal_discard.as_ref().unwrap();
        assert!(review.reviewed, "{summary}");
        assert!(!review.affirmative);
        let mut targets = Vec::new();
        portrait_targets(&app, Rect::new(0, 0, width, height), &mut targets);
        assert!(targets.is_empty());
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::NONE,
        )));
        let mut inspected = std::collections::HashSet::new();
        for _ in 0..128 {
            terminal
                .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            inspected.extend(body_text(&terminal).split_whitespace().map(str::to_owned));
            assert!(!app.cooperation.journal_discard.as_ref().unwrap().reviewed);
            if app.cooperation_ui.journal_discard_scroll == app.cooperation_ui.journal_discard_max {
                break;
            }
            app.update(Event::Key(KeyEvent::new(
                KeyCode::PageDown,
                KeyModifiers::NONE,
            )));
        }
        for id in &ids {
            assert!(
                inspected.contains(id),
                "{width}x{height}: unreachable exact observation ID {id}; inspected {inspected:?}\n{}",
                screen_text(&terminal)
            );
        }
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.planning.work, claim);
        assert!(app.planning.journal_failed());
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::NONE,
        )));
        terminal
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        assert!(app.cooperation.journal_discard.as_ref().unwrap().reviewed);
        app.update(Event::Resize(59, 17));
        let mut small = Terminal::new(TestBackend::new(59, 17)).unwrap();
        small
            .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        assert!(!app.cooperation.journal_discard.as_ref().unwrap().reviewed);
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.planning.work, claim);
        app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(app.cooperation.journal_discard.is_none());
        assert!(app.planning.journal_failed());
        app.update(Event::Resize(width, height));
        app.command("journal-discard");
        assert_eq!(app.cooperation.journal_discard.as_ref().unwrap().ids, ids);
        app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    }
}

#[test]
fn temporary_sheets_preserve_live_status_and_return_context() {
    use ratatui::{Terminal, backend::TestBackend};
    for (width, height) in [(60, 18), (120, 36)] {
        for linear in [false, true] {
            for overlay in [
                Overlay::Sources,
                Overlay::Memory,
                Overlay::Handoff,
                Overlay::Planning,
                Overlay::Execution,
                Overlay::Tasks,
                Overlay::Criteria,
                Overlay::Forms,
                Overlay::Party,
                Overlay::Companion,
                Overlay::Inspector,
                Overlay::Help,
                Overlay::Images,
                Overlay::Repository,
                Overlay::Preview,
            ] {
                let config = home_config();
                let mut app = App::new("/workspace".into(), width, height);
                app.linear = linear;
                app.editing = false;
                app.overlay = Some(overlay);
                app.storage = crate::app::StorageState::Failed("disk unavailable".into());
                let paint = Paint::new(&config, &FORMS[app.form()], app.form());
                let mut artwork = Artwork::load(&config);
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| {
                        render(frame, &mut app, &config, &mut artwork, "off");
                    })
                    .unwrap();
                let layout = shell(
                    Rect::new(0, 0, width, height),
                    linear,
                    app.effective_focus(),
                );
                let mut expected = Terminal::new(TestBackend::new(width, height)).unwrap();
                expected
                    .draw(|frame| {
                        header(frame, layout.header, &app, &config, &paint);
                        footer(frame, layout.footer, &app, &paint);
                    })
                    .unwrap();
                for protected in [layout.header, layout.footer] {
                    for y in protected.y..protected.bottom() {
                        for x in protected.x..protected.right() {
                            assert_eq!(
                                terminal.backend().buffer()[(x, y)].symbol(),
                                expected.backend().buffer()[(x, y)].symbol(),
                                "{overlay:?} {width}x{height} linear={linear}: live shell context obscured",
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn task_and_criteria_navigation_remains_visible_in_the_reserved_footer() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    for (width, height) in [(60, 18), (120, 36)] {
        for command in ["tasks", "criteria"] {
            let mut app = App::new("/workspace".into(), width, height);
            app.input = "retained request".into();
            app.cursor = 4;
            app.editing = true;
            app.command(command);
            let layout = shell(Rect::new(0, 0, width, height), false, app.effective_focus());
            let paint = Paint::new(&config, &FORMS[app.form()], app.form());
            for notice in ["", "Local notice remains visible"] {
                app.notice = notice.into();
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| footer(frame, layout.footer, &app, &paint))
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let hint: String = (layout.footer.x..layout.footer.right())
                    .map(|x| buffer[(x, layout.footer.bottom() - 1)].symbol())
                    .collect();
                assert!(hint.contains("Enter"), "{command}: {hint}");
                assert!(hint.contains("Esc Back"), "{command}: {hint}");
                if !notice.is_empty() {
                    let status: String = (layout.footer.x..layout.footer.right())
                        .map(|x| buffer[(x, layout.footer.y)].symbol())
                        .collect();
                    assert!(status.contains(notice), "{status}");
                }
            }
            app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
            assert!(app.overlay.is_none());
            assert!(app.editing);
            assert_eq!(app.input, "retained request");
            assert_eq!(app.cursor, 4);
            assert!(app.planning.pending.is_none());
            assert!(!app.execution.busy());
        }
    }
}

#[test]
fn source_pages_cover_every_detail_row_at_the_rendered_sheet_size() {
    use ratatui::{Terminal, backend::TestBackend};
    use std::collections::BTreeSet;

    let config = home_config();
    let paint = Paint::new(&config, &FORMS[0], 0);
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        app.overlay = Some(Overlay::Sources);
        app.sources.mode = crate::app::sources::Mode::Basket;
        app.sources.detail = (0..40)
            .map(|index| format!("detail-row-{index:03}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut seen = BTreeSet::new();
        for _ in 0..40 {
            terminal
                .draw(|frame| {
                    sources::render(
                        frame,
                        Rect::new(0, 2, width, height - 4),
                        &mut app,
                        &config,
                        &paint,
                    );
                })
                .unwrap();
            let text = screen_text(&terminal);
            for index in 0..40 {
                if text.contains(&format!("detail-row-{index:03}")) {
                    seen.insert(index);
                }
            }
            let previous = app.scroll_for(ScrollSurface::Preview);
            app.update(Event::Key(KeyEvent::new(
                KeyCode::PageDown,
                KeyModifiers::NONE,
            )));
            if previous == app.scroll_for(ScrollSurface::Preview) {
                break;
            }
        }
        assert_eq!(
            seen,
            (0..40).collect(),
            "{width}x{height} skipped detail rows"
        );
    }
}

#[test]
fn record_discard_uses_visible_summary_not_end_scroll_and_invalidates_tiny_consent() {
    use ratatui::{Terminal, backend::TestBackend};

    let config = home_config();
    let paint = Paint::new(&config, &FORMS[0], 0);
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/workspace".into(), width, height);
        app.overlay = Some(Overlay::Records);
        app.records.mode = crate::app::records::Mode::Discard;
        app.records.failed = Some(crate::store::records::Request {
            task_id: 7,
            operation: crate::store::records::Operation::ObserveInstruction {
                request_id: "request-other-owner".into(),
                owner: crate::store::Attribution {
                    partner: 2,
                    name: "Other owner".into(),
                    form: 4,
                },
                instruction: "retained authored bytes".repeat(100),
            },
        });
        app.records.failed_error = "retained failed-write detail\n".repeat(100);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| records::render(frame, frame.area(), &mut app, &config, &paint))
            .unwrap();
        assert!(app.records.reviewed);
        assert_eq!(app.records.scroll, 0);
        assert!(app.records.scroll_max > 0);
        assert!(!app.records.affirmative);
        let visible = screen_text(&terminal);
        assert!(
            visible.contains("request-other-owner"),
            "failed request identity was buried by error details"
        );
        assert!(
            visible.contains("#7") && visible.contains("#2"),
            "actual task/partner scope did not render"
        );
        app.record_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert!(app.records.affirmative);
        app.record_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(!app.records.affirmative);
        app.record_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            app.records.failed.is_some(),
            "neutral Enter discarded an owned write"
        );

        app.records.enter(crate::app::records::Mode::Discard);
        let mut tiny = Terminal::new(TestBackend::new(20, 6)).unwrap();
        tiny.draw(|frame| records::render(frame, frame.area(), &mut app, &config, &paint))
            .unwrap();
        app.record_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        app.record_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            app.records.failed.is_some(),
            "unrendered summary admitted discard"
        );

        app.records.enter(crate::app::records::Mode::Discard);
        terminal
            .draw(|frame| records::render(frame, frame.area(), &mut app, &config, &paint))
            .unwrap();
        app.record_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        app.record_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.records.failed.is_none());
    }
}

#[test]
fn evolution_popup_is_one_shot_localized_and_preserves_real_editor_input() {
    use ratatui::{Terminal, backend::TestBackend};
    for (width, height) in [(60, 18), (120, 36)] {
        for language in [
            crate::i18n::Language::ENGLISH,
            crate::i18n::Language::parse("ko").unwrap(),
        ] {
            let mut config = home_config();
            config.language = language;
            let mut app = App::new("/workspace".into(), width, height);
            app.task_id = 7;
            app.storage = crate::app::StorageState::Saved;
            app.input = "future 界draft".into();
            app.cursor = "future ".len();
            app.editing = true;
            app.editor_field = EditorField::Request;
            app.command("flareon");
            let mut artwork = Artwork::load(&config);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let screen = screen_text(&terminal);
            assert!(app.evolution.popup_shown, "{screen}");
            assert!(screen.contains("Eevee -> Flareon"), "{screen}");
            assert!(
                screen.contains(language.text(Message::CoreEvolutionPopupTitle))
                    && screen.contains(language.text(Message::CoreEvolutionPopupContinue)),
                "{screen}"
            );
            assert_eq!((app.input.as_str(), app.cursor), ("future 界draft", 7));
            assert!(app.planning.pending.is_none());
            app.update(Event::Paste("한".into()));
            assert_eq!((app.input.as_str(), app.cursor), ("future 한界draft", 10));
            terminal
                .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            assert!(!screen_text(&terminal).contains("Eevee -> Flareon"));
            app.command("flareon");
            terminal
                .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            assert!(
                !screen_text(&terminal).contains("Eevee -> Flareon"),
                "unchanged form replayed evolution"
            );
        }
    }
}

#[test]
fn evolution_popup_waits_for_safe_surface_and_live_file_code_without_being_consumed() {
    use crate::providers::{FilePhase, ItemKind};
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::Arc;
    let config = home_config();
    let mut app = App::new("/workspace".into(), 60, 18);
    app.task_id = 7;
    app.storage = crate::app::StorageState::Saved;
    app.command("flareon");
    app.command("help");
    let mut artwork = Artwork::load(&config);
    let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
    terminal
        .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
        .unwrap();
    assert!(!app.evolution.popup_shown);
    app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert!(
        app.evolution.transition.is_some(),
        "hidden notification was consumed by overlay input"
    );
    app.planning.submitted_request = Arc::from("Create generated/live.py");
    app.planning.items.push(crate::app::planning::Item {
        thread_id: String::new(),
        turn_id: "turn".into(),
        item_id: "file".into(),
        kind: ItemKind::File(FilePhase::Generating),
        text: "generated/live.py\n1  ACTUAL_LIVE_CODE".into(),
        complete: false,
        request: Arc::clone(&app.planning.submitted_request),
        owner: None,
    });
    terminal
        .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
        .unwrap();
    assert!(screen_text(&terminal).contains("ACTUAL_LIVE_CODE"));
    assert!(!app.evolution.popup_shown);
    app.planning.items[0].kind = ItemKind::File(FilePhase::Applied);
    terminal
        .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
        .unwrap();
    assert!(
        app.evolution.popup_shown,
        "completed file never released the pending evolution receipt"
    );
}

#[test]
fn evolution_popup_keeps_native_portraits_outside_its_visible_bounds() {
    let mut app = App::new("/workspace".into(), 120, 36);
    app.command("flareon");
    let window = Rect::new(0, 0, 120, 36);
    let popup = evolution_popup_area(window, &app).unwrap();
    let layout = shell(window, app.linear, app.effective_focus());
    assert!(popup.y >= layout.header.bottom() && popup.bottom() <= layout.composer.y);
    let mut targets = Vec::new();
    portrait_targets(&app, window, &mut targets);
    assert!(
        targets.iter().all(|(_, area)| !area.intersects(popup)),
        "native image would paint above the evolution receipt"
    );
    let portrait = companion_portrait(layout.rail.unwrap());
    assert!(
        !portrait.intersects(popup) && targets.contains(&(app.form(), portrait)),
        "roomy receipt must leave the existing native portrait visible"
    );
    let config = home_config();
    let mut artwork = Artwork::load(&config);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
    terminal
        .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
        .unwrap();
    app.update(Event::Key(KeyEvent::new(
        KeyCode::F(12),
        KeyModifiers::NONE,
    )));
    portrait_targets(&app, window, &mut targets);
    assert!(
        targets.contains(&(app.form(), portrait)),
        "acknowledged popup must retain the real companion portrait"
    );
}

#[test]
fn evolution_receipt_is_edge_anchored_and_reserves_response_and_composer() {
    for (width, height) in [(60, 18), (120, 36)] {
        for linear in [false, true] {
            let mut app = App::new("/workspace".into(), width, height);
            app.linear = linear;
            app.command("flareon");
            // A nonzero origin catches accidental screen-relative placement.
            let window = Rect::new(3, 4, width, height);
            let layout = shell(window, linear, app.effective_focus());
            let popup = evolution_popup_area(window, &app).unwrap();
            assert!(!popup.intersects(layout.header));
            assert!(!popup.intersects(layout.composer));
            assert!(!popup.intersects(layout.footer));
            if width == 120 && !linear {
                assert_eq!((popup.width, popup.height), (26, 7));
                assert!(!popup.intersects(layout.workspace));
                assert!(popup.y > layout.rail.unwrap().bottom());
            } else {
                assert_eq!(popup.x, layout.workspace.x);
                assert_eq!(popup.width, layout.workspace.width);
                assert_eq!(popup.y, layout.workspace.y);
                assert_eq!(popup.right(), layout.workspace.right());
                assert!(layout.workspace.bottom() - popup.bottom() >= 3);
            }
        }
    }
}

#[test]
fn evolution_receipt_keeps_localized_hierarchy_in_ascii_mono_and_color() {
    use ratatui::{Terminal, backend::TestBackend};
    for (width, height) in [(60, 18), (120, 36)] {
        for language in [Language::ENGLISH, Language::parse("ko").unwrap()] {
            for (ascii, color) in [
                (true, ColorMode::Mono),
                (false, ColorMode::Mono),
                (false, ColorMode::TrueColor),
            ] {
                let mut config = home_config();
                config.language = language;
                config.ascii = ascii;
                config.color = color;
                let mut app = App::new("/workspace".into(), width, height);
                app.command("vaporeon");
                app.command("jolteon");
                let paint = Paint::new(&config, &FORMS[app.form()], app.form());
                let area = evolution_popup_area(Rect::new(0, 0, width, height), &app).unwrap();
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| {
                        cooperation::evolution_popup(frame, area, &mut app, &config, &paint)
                    })
                    .unwrap();
                let screen = screen_text(&terminal);
                assert!(
                    screen.contains(if ascii {
                        "Vaporeon -> Jolteon"
                    } else {
                        "Vaporeon →  Jolteon"
                    }),
                    "{screen}"
                );
                assert!(
                    screen.contains(language.text(Message::CoreEvolutionPopupTitle)),
                    "{screen}"
                );
                assert!(
                    screen.contains(language.text(Message::CoreEvolutionManual)),
                    "{screen}"
                );
                assert!(
                    screen.contains(language.text(Message::CoreEvolutionPopupContinue)),
                    "{screen}"
                );
                for word in language
                    .text(Message::CoreEvolutionPopupManualReason)
                    .split_whitespace()
                {
                    assert!(screen.contains(word), "receipt clipped {word}: {screen}");
                }
                let buffer = terminal.backend().buffer();
                assert_eq!(
                    buffer[(area.x, area.y)].symbol(),
                    if ascii { "+" } else { "╭" }
                );
                assert!(
                    buffer.content.iter().any(|cell| {
                        cell.symbol() == "J" && cell.modifier.contains(Modifier::BOLD)
                    }),
                    "destination lost its visual emphasis: {screen}"
                );
                assert_eq!(app.evolution_frame(), None);
            }
        }
    }
}

#[test]
fn evolution_receipt_survives_resize_without_consuming_the_next_typed_key() {
    use ratatui::{Terminal, backend::TestBackend};
    let config = home_config();
    let mut app = App::new("/workspace".into(), 120, 36);
    app.task_id = 7;
    app.storage = crate::app::StorageState::Saved;
    app.input = "한界draft".into();
    app.cursor = "한".len();
    app.editing = true;
    app.editor_field = EditorField::Request;
    app.command("flareon");
    let mut artwork = Artwork::load(&config);
    app.update(Event::Resize(59, 17));
    let mut tiny = Terminal::new(TestBackend::new(59, 17)).unwrap();
    tiny.draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
        .unwrap();
    assert!(!app.evolution.popup_shown);
    app.update(Event::Resize(60, 18));
    let mut compact = Terminal::new(TestBackend::new(60, 18)).unwrap();
    compact
        .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
        .unwrap();
    assert!(app.evolution.popup_shown);
    app.update(Event::Key(KeyEvent::new(
        KeyCode::Char('x'),
        KeyModifiers::NONE,
    )));
    assert_eq!((app.input.as_str(), app.cursor), ("한x界draft", 4));
    assert!(app.planning.pending.is_none());
    app.update(Event::Resize(120, 36));
    assert!(evolution_popup_area(Rect::new(0, 0, 120, 36), &app).is_none());
}

#[test]
fn held_sheet_keeps_frozen_scope_and_membership_without_mutating_draft() {
    use crate::{
        app::held::{Notice, Receipt},
        workspace::held::Target,
    };
    use ratatui::{Terminal, backend::TestBackend};
    for language in [Language::ENGLISH, Language::parse("ko").unwrap()] {
        for (width, height) in [(60, 18), (120, 36)] {
            let mut config = home_config();
            config.language = language;
            let mut app = App::new("/workspace".into(), width, height);
            app.task_id = 92;
            app.input = "Future draft 한글".into();
            app.cursor = 7;
            app.held.target = Some(Target::File {
                path: "future.rs".into(),
            });
            app.held.notice = Some(Notice::Saved);
            app.held.receipt = Some(Receipt {
                task: 7,
                owner: store::Attribution {
                    partner: 18,
                    name: "FrozenOwner".into(),
                    form: 2,
                },
                target: Some(Target::Memory {
                    id: 81,
                    revision: 4,
                }),
                included: false,
            });
            app.overlay = Some(Overlay::Held);
            let mut artwork = Artwork::load(&config);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for _ in 0..2 {
                terminal
                    .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                    .unwrap();
                let rendered = screen_text(&terminal);
                for value in [
                    "FrozenOwner",
                    "future.rs",
                    "#81 @ 4",
                    "/held-accept",
                    "/held-dismiss",
                    "/held-clear",
                    "/held-set-file",
                    "/held-set-memory",
                ] {
                    assert!(rendered.contains(value), "{value}: {rendered}");
                }
                assert!(
                    rendered.contains(language.text(Message::HeldReceiptExcluded)),
                    "{width}×{height}: {rendered}"
                );
                assert_eq!(app.input, "Future draft 한글");
                assert_eq!(app.cursor, 7);
                assert_eq!(app.held.receipt.as_ref().unwrap().task, 7);
                assert!(!app.held.receipt.as_ref().unwrap().included);
            }
            app.held.receipt.as_mut().unwrap().included = true;
            app.held.target = Some(Target::File {
                path: "changed-current.rs".into(),
            });
            app.task_id = 93;
            terminal
                .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let rendered = screen_text(&terminal);
            assert!(
                rendered.contains(language.text(Message::HeldReceiptIncluded)),
                "{width}×{height}: {rendered}"
            );
            assert!(rendered.contains("#81 @ 4"));
            assert_eq!(app.held.receipt.as_ref().unwrap().task, 7);
        }
    }
}

#[test]
fn held_optional_strip_yields_to_real_live_file_output_without_hiding_draft() {
    use crate::{
        providers::{FilePhase, ItemKind},
        workspace::held::Target,
    };
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::Arc;
    for language in [Language::ENGLISH, Language::parse("ko").unwrap()] {
        for (width, height) in [(60, 18), (120, 36)] {
            let mut config = home_config();
            config.language = language;
            let mut app = App::new("/workspace".into(), width, height);
            app.held.target = Some(Target::File {
                path: "held.rs".into(),
            });
            app.input = "Draft remains".into();
            app.cursor = 5;
            app.planning.submitted_request = Arc::from("Write a file");
            app.planning.items.push(crate::app::planning::Item {
                thread_id: String::new(),
                turn_id: "turn".into(),
                item_id: "file".into(),
                kind: ItemKind::File(FilePhase::Generating),
                text: "live.rs\n1  HELD_PRIORITY_LIVE_CODE".into(),
                complete: false,
                request: Arc::clone(&app.planning.submitted_request),
                owner: None,
            });
            let mut artwork = Artwork::load(&config);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for phase in [
                FilePhase::Generating,
                FilePhase::Received,
                FilePhase::AwaitingReview,
            ] {
                app.planning.items[0].kind = ItemKind::File(phase);
                terminal
                    .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                    .unwrap();
                let rendered = screen_text(&terminal);
                assert!(rendered.contains("HELD_PRIORITY_LIVE_CODE"), "{rendered}");
                assert!(!rendered.contains("/held"));
                assert!(rendered.contains("Draft remains"));
                assert_eq!(app.cursor, 5);
            }
        }
    }
}

#[test]
fn held_preference_does_not_occlude_source_approval_at_supported_sizes() {
    use ratatui::{Terminal, backend::TestBackend};
    for language in [Language::ENGLISH, Language::parse("ko").unwrap()] {
        for (width, height) in [(60, 18), (120, 36)] {
            let mut config = home_config();
            config.language = language;
            let mut app = App::new("/workspace".into(), width, height);
            app.task_id = 7;
            app.input = "Unsent draft".into();
            app.cursor = 3;
            app.held.target = Some(crate::workspace::held::Target::File {
                path: "other.rs".into(),
            });
            app.overlay = Some(Overlay::Sources);
            app.sources.mode = crate::app::sources::Mode::Basket;
            app.sources.basket.push(crate::workspace::context::Excerpt {
                origin_run: None,
                id: 91,
                task_id: 7,
                root: "/workspace".into(),
                path: "APPROVAL_SCOPE.rs".into(),
                start_line: 1,
                end_line: 1,
                source_sha256: [9; 32],
                text: "retained source".into(),
                available: true,
            });
            app.sources.command("purge", 7, None);
            let mut artwork = Artwork::load(&config);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let rendered = screen_text(&terminal);
            assert!(rendered.contains("APPROVAL_SCOPE.rs"), "{rendered}");
            assert!(!rendered.contains("/held"));
            assert!(app.sources.confirmation.as_ref().unwrap().reviewed);
            assert_eq!(app.input, "Unsent draft");
            assert_eq!(app.cursor, 3);
        }
    }
}

#[test]
fn held_workspace_keeps_item_identity_and_controls_at_minimum_size() {
    use ratatui::{Terminal, backend::TestBackend};
    for language in [Language::ENGLISH, Language::parse("ko").unwrap()] {
        for (width, height) in [(60, 18), (120, 36)] {
            for path in [
                "資料_한글.rs".to_owned(),
                format!("資料_{}.rs", "한글".repeat(80)),
            ] {
                let mut config = home_config();
                config.language = language;
                let mut app = App::new("/workspace".into(), width, height);
                app.task_id = 7;
                app.goal = "Keep the saved context visible".into();
                app.input = "Future draft".into();
                app.cursor = 7;
                app.editor_field = store::EditorField::Request;
                app.held.target = Some(crate::workspace::held::Target::File { path });
                let mut artwork = Artwork::load(&config);
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                    .unwrap();
                let rendered = screen_text(&terminal);
                assert!(rendered.contains("資料_"), "{width}×{height}: {rendered}");
                assert!(rendered.contains("/held"), "{width}×{height}: {rendered}");
                assert!(rendered.contains("Future draft"));
                assert_eq!(app.cursor, 7);
                assert!(app.sources.basket.is_empty());
                app.held.dismissed = true;
                for _ in 0..2 {
                    terminal
                        .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                        .unwrap();
                    let rendered = screen_text(&terminal);
                    assert!(!rendered.contains("資料_"), "{rendered}");
                    assert!(!rendered.contains("/held"), "{rendered}");
                    assert!(app.held.target.is_some());
                    assert_eq!(app.input, "Future draft");
                    assert_eq!(app.cursor, 7);
                }
            }
        }
    }
}

fn ribbon_fixture(
    kind: crate::workspace::ribbons::Kind,
    partner: i64,
    name: &str,
    hidden: bool,
) -> crate::workspace::ribbons::View {
    crate::workspace::ribbons::View {
        ribbon: crate::workspace::ribbons::Ribbon {
            kind,
            task_id: 7,
            owner: crate::store::Attribution {
                partner,
                name: name.into(),
                form: 7,
            },
            at: 1,
            evidence: crate::workspace::ribbons::Evidence::Artifact {
                id: format!("source-{partner}"),
            },
            hidden,
        },
        freshness: crate::workspace::records::Freshness::Stale,
        explanation: String::new(),
    }
}

#[test]
fn compact_ribbon_inspection_shows_original_evidence_without_paging() {
    use crate::workspace::ribbons::Kind;
    use ratatui::{Terminal, backend::TestBackend};
    for language in [Language::ENGLISH, Language::parse("ko").unwrap()] {
        let mut config = home_config();
        config.language = language;
        let paint = Paint::new(&config, &FORMS[0], 0);
        let mut app = App::new("/workspace".into(), 60, 18);
        app.records.loaded = true;
        app.records.snapshot.ribbons.push(ribbon_fixture(
            Kind::AcceptedReview,
            22,
            "Original producer",
            false,
        ));
        app.command("ribbons");
        let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
        terminal
            .draw(|frame| records::render(frame, frame.area(), &mut app, &config, &paint))
            .unwrap();
        assert!(screen_text(&terminal).contains("source-22"));
    }
}

#[test]
fn ribbon_inspector_pages_every_evidence_row_in_both_locales_and_restores_editor() {
    use crate::workspace::ribbons::Kind;
    use ratatui::{Terminal, backend::TestBackend};
    use std::collections::BTreeSet;
    for language in [Language::ENGLISH, Language::parse("ko").unwrap()] {
        for (width, height) in [(60, 18), (120, 36)] {
            let mut config = home_config();
            config.language = language;
            let paint = Paint::new(&config, &FORMS[0], 0);
            let mut app = App::new("/workspace".into(), width, height);
            app.task_id = 9;
            app.input = "draft 한글\nliteral".into();
            app.cursor = 6;
            app.editing = true;
            app.records.loaded = true;
            let mut view = ribbon_fixture(Kind::AcceptedReview, 22, "Old producer", true);
            view.explanation = (0..48)
                .map(|index| format!("source-row-{index:03}"))
                .collect::<Vec<_>>()
                .join("\n");
            app.records.snapshot.ribbons.push(view);
            app.command("ribbons");
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut seen = BTreeSet::new();
            for _ in 0..100 {
                terminal
                    .draw(|frame| records::render(frame, frame.area(), &mut app, &config, &paint))
                    .unwrap();
                let rendered = screen_text(&terminal);
                for index in 0..48 {
                    if rendered.contains(&format!("source-row-{index:03}")) {
                        seen.insert(index);
                    }
                }
                let old = app.records.scroll;
                app.record_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
                if app.records.scroll == old {
                    break;
                }
            }
            assert_eq!(
                seen,
                (0..48).collect(),
                "{width}x{height} {}",
                language.tag()
            );
            assert!(app.records.scroll_max > 0);
            assert_eq!(app.task_id, 9);
            assert!(!app.execution.busy());
            assert!(app.planning.pending.is_none());
            assert!(app.handoff.pending.is_none());
            app.record_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            assert!(app.editing);
            assert_eq!(app.input, "draft 한글\nliteral");
            assert_eq!(app.cursor, 6);
        }
    }
}

#[test]
fn partner_summary_ribbons_filter_historical_producer_hidden_and_flavor_without_hiding_inspector() {
    use crate::workspace::{records::PartnerSummary, ribbons::Kind};
    use ratatui::{Terminal, backend::TestBackend};
    for language in [Language::ENGLISH, Language::parse("ko").unwrap()] {
        for (width, height) in [(60, 18), (120, 36)] {
            let mut config = home_config();
            config.language = language;
            let paint = Paint::new(&config, &FORMS[0], 0);
            let mut app = App::new("/workspace".into(), width, height);
            let partner = app.lead().clone();
            app.records.snapshot.summaries.push(PartnerSummary {
                partner: partner.clone(),
                produced_tasks: [0; 9],
                moves: [0; 9],
                move_names: Default::default(),
                shared_bond_memories: 0,
                unknown_producer_artifacts: 0,
            });
            app.records.snapshot.ribbons = vec![
                ribbon_fixture(Kind::AcceptedReview, partner.id, "frozen-visible", false),
                ribbon_fixture(Kind::VerifiedHandoff, partner.id, "frozen-hidden", true),
                ribbon_fixture(Kind::LearnedMove, partner.id + 100, "other-producer", false),
            ];
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            app.open_records(crate::app::records::Mode::Summary);
            terminal
                .draw(|frame| records::render(frame, frame.area(), &mut app, &config, &paint))
                .unwrap();
            let rendered = screen_text(&terminal);
            assert!(rendered.contains("frozen-visible"), "{rendered}");
            assert!(!rendered.contains("frozen-hidden"));
            assert!(!rendered.contains("other-producer"));
            app.records.snapshot.flavor_enabled = false;
            terminal
                .draw(|frame| records::render(frame, frame.area(), &mut app, &config, &paint))
                .unwrap();
            assert!(!screen_text(&terminal).contains("frozen-visible"));
            app.open_records(crate::app::records::Mode::Ribbons);
            app.records.selected = 1;
            let mut history = String::new();
            loop {
                terminal
                    .draw(|frame| records::render(frame, frame.area(), &mut app, &config, &paint))
                    .unwrap();
                history.push_str(&screen_text(&terminal));
                let old = app.records.scroll;
                app.record_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
                if app.records.scroll == old {
                    break;
                }
            }
            assert!(history.contains("frozen-hidden"));
            assert!(history.contains("other-producer"));
            assert!(history.contains("source-"));
        }
    }
}

#[test]
fn both_densities_keep_storage_error_and_focused_cjk_editor_at_supported_sizes() {
    use ratatui::{Terminal, backend::TestBackend};
    for language in [Language::ENGLISH, Language::parse("ko").unwrap()] {
        for (width, height) in [(60, 18), (120, 36)] {
            let mut config = home_config();
            config.language = language;
            let mut app = App::new("/workspace".into(), width, height);
            app.editing = true;
            app.editor_field = EditorField::Request;
            app.input = "界e\u{301} 끝".into();
            app.cursor = "界e\u{301}".len();
            app.storage = crate::app::StorageState::Failed("disk full".into());
            app.notice = "Local write refused".into();
            let before = (app.input.clone(), app.cursor, app.effective_focus());
            let mut artwork = Artwork::load(&config);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for density in [store::TextDensity::Compact, store::TextDensity::Comfortable] {
                app.planning.set_text_density(density);
                terminal
                    .draw(|frame| render(frame, &mut app, &config, &mut artwork, "off"))
                    .unwrap();
                let cursor = terminal.get_cursor_position().unwrap();
                let rendered = screen_text(&terminal);
                assert!(rendered.contains("界e\u{301} 끝"), "{rendered}");
                assert!(rendered.contains(app.storage_label()), "{rendered}");
                assert!(rendered.contains("Local write refused"), "{rendered}");
                let composer = shell(
                    Rect::new(0, 0, width, height),
                    app.linear,
                    app.effective_focus(),
                )
                .composer;
                assert!(cursor.x >= composer.x && cursor.x < composer.right());
                assert!(cursor.y > composer.y && cursor.y < composer.bottom());
                assert_eq!(
                    (app.input.clone(), app.cursor, app.effective_focus()),
                    before
                );
                assert!(app.editing);
                assert!(app.planning.pending.is_none());
            }
        }
    }
}

#[test]
fn comfortable_task_rows_add_space_without_hiding_the_keyboard_selection() {
    use ratatui::{Terminal, backend::TestBackend};
    for (width, height) in [(60, 18), (120, 36)] {
        let config = home_config();
        let paint = Paint::new(&config, &FORMS[0], 0);
        let mut app = App::new("/workspace".into(), width, height);
        app.tasks = (1..=6)
            .map(|id| {
                let mut task = store::Snapshot::empty("/workspace".into());
                task.id = id;
                task.goal = format!("Task {id} 界");
                task
            })
            .collect();
        app.task_selection = 5;
        for density in [store::TextDensity::Compact, store::TextDensity::Comfortable] {
            app.planning.set_text_density(density);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| tasks(frame, frame.area(), &mut app, &config, &paint))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let row_of = |needle: &str| {
                (0..height)
                    .find(|y| {
                        let line: String = (0..width).map(|x| buffer[(x, *y)].symbol()).collect();
                        line.contains(needle)
                    })
                    .expect("selected and preceding card remain visible")
            };
            assert_eq!(
                row_of(">  #6 ") - row_of("  #5 "),
                if density.is_compact() { 1 } else { 2 },
            );
            assert_eq!(app.task_selection, 5);
            assert!(app.planning.pending.is_none());
        }
    }
}
