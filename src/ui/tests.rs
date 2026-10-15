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
fn task_layout_keeps_composer_full_width_and_details_progressive() {
    for (width, height) in [(120, 36), (100, 30), (80, 24), (70, 24), (60, 18)] {
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
        assert_eq!(layout.rail.is_some(), width >= 90);
        if let Some(rail) = layout.rail {
            assert!(rail.bottom() <= layout.composer.y);
            assert!(layout.workspace.right() < rail.x);
            let image = companion_portrait(rail);
            assert!(image.right() <= rail.right());
            assert!(image.bottom() < rail.bottom());
        }
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
fn command_palette_keeps_the_companion_portrait_visible() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    for (width, height) in [(120, 36), (90, 24)] {
        let mut app = App::new("/workspace".into(), width, height);
        let area = Rect::new(0, 0, width, height);
        let mut targets = Vec::new();
        portrait_targets(&app, area, &mut targets);
        assert_eq!(targets.len(), 1);
        let before = targets.clone();
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.command_popup.is_some());
        portrait_targets(&app, area, &mut targets);
        assert_eq!(targets, before, "opening / must not erase the portrait");
        assert!(!command_popup_area(area, &app).intersects(before[0].1));
    }
}

#[test]
fn command_palette_preserves_root_draft_and_status_at_compact_sizes() {
    for (width, height) in [(120, 36), (90, 24), (60, 18)] {
        let app = App::new("/workspace".into(), width, height);
        let area = Rect::new(0, 0, width, height);
        let layout = shell(area, false, Pane::Workspace);
        let popup = command_popup_area(area, &app);
        assert!(popup.y >= layout.header.bottom());
        assert_eq!(popup.bottom(), layout.composer.y);
        assert!(!popup.intersects(layout.composer));
        assert!(!popup.intersects(layout.footer));
        assert!(
            inner(popup).height >= 7,
            "query, choices, reason and keys fit"
        );
        assert!(inner(layout.workspace).height >= 7);
    }
}

#[test]
fn compact_palette_shows_blocker_keys_and_uncovered_draft() {
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
            "local draft",
            "Ctrl+N Newline",
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
