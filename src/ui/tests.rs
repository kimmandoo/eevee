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
        assert!(layout.workspace.bottom() < layout.composer.y);
        assert_eq!(layout.composer.bottom(), layout.footer.y);
        assert!(layout.inspector.is_none());
        assert_eq!(layout.rail.is_some(), width >= 90);
        if let Some(rail) = layout.rail {
            assert!(rail.bottom() < layout.composer.y);
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
