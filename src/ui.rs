use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Paragraph, Row, Table, TableState, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    app::{App, Overlay, PALETTE, Pane},
    config::{ColorMode, Config, Theme},
    domain::{FORMS, Form},
};

const ASCII_BORDER: border::Set = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};
const FORM_KEYS: [&str; 9] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];

struct Paint {
    base: Style,
    muted: Style,
    border: Style,
    accent: Style,
    selected: Style,
}

impl Paint {
    fn new(config: &Config, form: &Form, form_index: usize) -> Self {
        if config.color == ColorMode::Mono {
            let base = Style::default().fg(Color::Reset).bg(Color::Reset);
            return Self {
                base,
                muted: base,
                border: base,
                accent: base.add_modifier(Modifier::BOLD),
                selected: base.add_modifier(Modifier::REVERSED | Modifier::BOLD),
            };
        }
        let light = config.theme == Theme::Light;
        let (background, foreground, muted, border_color) = match (config.color, light) {
            (ColorMode::TrueColor, false) => (
                Color::Rgb(20, 24, 31),
                Color::Rgb(224, 228, 234),
                Color::Rgb(153, 164, 181),
                Color::Rgb(70, 82, 99),
            ),
            (ColorMode::TrueColor, true) => (
                Color::Rgb(248, 247, 242),
                Color::Rgb(37, 44, 55),
                Color::Rgb(88, 99, 114),
                Color::Rgb(153, 161, 169),
            ),
            (_, false) => (
                Color::Indexed(234),
                Color::Indexed(253),
                Color::Indexed(246),
                Color::Indexed(240),
            ),
            (_, true) => (
                Color::Indexed(255),
                Color::Indexed(235),
                Color::Indexed(241),
                Color::Indexed(246),
            ),
        };
        let accent = match config.color {
            ColorMode::TrueColor => {
                let (r, g, b) = form.rgb;
                if light {
                    Color::Rgb(r / 2, g / 2, b / 2)
                } else {
                    Color::Rgb(r, g, b)
                }
            }
            _ if light => Color::Indexed([94, 30, 100, 130, 97, 136, 65, 31, 132][form_index]),
            _ => Color::Indexed(form.indexed),
        };
        let base = Style::default().fg(foreground).bg(background);
        Self {
            base,
            muted: base.fg(muted),
            border: base.fg(border_color),
            accent: base.fg(accent).add_modifier(Modifier::BOLD),
            selected: base
                .fg(accent)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD),
        }
    }
}

pub fn render(frame: &mut Frame, app: &App, config: &Config) {
    let area = frame.area();
    let form_index = app.form.min(FORMS.len() - 1);
    let form = &FORMS[form_index];
    let paint = Paint::new(config, form, form_index);
    frame.render_widget(Block::default().style(paint.base), area);
    if area.width < 60 || area.height < 18 {
        frame.render_widget(
            Paragraph::new("Ctrl+C quit\nTerminal too small\nNeed 60 columns x 18 rows\nResize to continue; no work is running.")
                .style(paint.accent)
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }

    header(frame, row(area, 0, 3), app, form, &paint);
    let body = row(area, 3, area.height.saturating_sub(5));
    if area.width >= 120 {
        let rail = Rect::new(body.x, body.y, 22, body.height);
        let inspector = Rect::new(body.right().saturating_sub(30), body.y, 30, body.height);
        let workspace = Rect::new(body.x + 22, body.y, body.width - 52, body.height);
        companion(
            frame,
            rail,
            app,
            form,
            config,
            &paint,
            app.focus == Pane::Companion,
        );
        workspace_view(frame, workspace, app, form, config, &paint);
        inspector_view(
            frame,
            inspector,
            app,
            form,
            config,
            &paint,
            app.focus == Pane::Inspector,
        );
    } else if area.width >= 90 {
        let rail = Rect::new(body.x, body.y, 22, body.height);
        let workspace = Rect::new(body.x + 22, body.y, body.width - 22, body.height);
        companion(
            frame,
            rail,
            app,
            form,
            config,
            &paint,
            app.focus == Pane::Companion,
        );
        workspace_view(frame, workspace, app, form, config, &paint);
    } else {
        workspace_view(frame, body, app, form, config, &paint);
    }
    let footer = row(area, area.height - 2, 2);
    frame.render_widget(
        Paragraph::new("f Forms  g Session goal  i Inspect  b Companion  ? Help\nCtrl+K Palette  Tab Focus  Enter Open  q Quit  Ctrl+C Exit")
            .style(paint.muted),
        footer,
    );

    if let Some(overlay) = app.overlay {
        match overlay {
            Overlay::Forms => forms_overlay(frame, area, app, config, &paint),
            Overlay::Palette => palette_overlay(frame, area, app, config, &paint),
            Overlay::Help => help_overlay(frame, area, config, &paint),
            Overlay::Inspector => {
                let drawer = drawer(area);
                frame.render_widget(Clear, drawer);
                inspector_view(frame, drawer, app, form, config, &paint, true);
            }
            Overlay::Companion => {
                let drawer = drawer(area);
                frame.render_widget(Clear, drawer);
                companion(frame, drawer, app, form, config, &paint, true);
            }
        }
    } else if app.editing {
        goal_editor(frame, area, app, config, &paint);
    }
}

fn row(area: Rect, offset: u16, height: u16) -> Rect {
    let offset = offset.min(area.height);
    Rect::new(
        area.x,
        area.y.saturating_add(offset),
        area.width,
        height.min(area.height - offset),
    )
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

fn drawer(area: Rect) -> Rect {
    let width = 46.min(area.width.saturating_sub(2));
    Rect::new(
        area.right().saturating_sub(width + 1),
        area.y + 1,
        width,
        area.height.saturating_sub(2),
    )
}

fn panel<'a>(title: &'a str, config: &Config, paint: &Paint, focused: bool) -> Block<'a> {
    let block = Block::bordered()
        .title(title)
        .style(paint.base)
        .border_style(if focused { paint.accent } else { paint.border });
    if config.ascii {
        block.border_set(ASCII_BORDER)
    } else {
        block.border_type(BorderType::Rounded)
    }
}

fn text(frame: &mut Frame, area: Rect, value: &str, style: Style) {
    frame.render_widget(
        Paragraph::new(value)
            .style(style)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn header(frame: &mut Frame, area: Rect, app: &App, form: &Form, paint: &Paint) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" EEVEE / ", paint.accent),
            Span::styled(form.name, paint.accent),
            Span::styled(" | Ready | Approvals: 0", paint.base),
        ])),
        row(area, 0, 1),
    );
    let goal = if app.goal.is_empty() {
        "Not set - press g (session only)"
    } else {
        &app.goal
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Goal: ", paint.muted),
            Span::raw(goal),
        ]))
        .style(paint.base),
        row(area, 1, 1),
    );
    text(
        frame,
        row(area, 2, 1),
        " Local shell | Backend not connected | No permissions granted",
        paint.muted,
    );
}

fn workspace_view(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    form: &Form,
    config: &Config,
    paint: &Paint,
) {
    let block = panel(form.workspace, config, paint, app.focus == Pane::Workspace);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    text(frame, row(inner, 0, 2), form.role, paint.accent);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("cwd: ", paint.muted),
            Span::raw(app.cwd.as_str()),
        ]))
        .style(paint.base)
        .wrap(Wrap { trim: false }),
        row(inner, 2, 3),
    );
    text(frame, row(inner, 6, 3), form.empty, paint.base);
    text(
        frame,
        row(inner, 9, 1),
        "No evidence recorded. No permissions granted.",
        paint.muted,
    );
    text(
        frame,
        row(inner, 10, 1),
        "g / Enter: session goal | f: forms",
        paint.muted,
    );
    if inner.height >= 15 {
        text(frame, row(inner, 12, 3), form.evidence, paint.muted);
    }
    if inner.height >= 19 {
        text(
            frame,
            row(inner, 17, 2),
            "Phase 1: local navigation and session goal only.\nForm changes do not run tools or grant permissions.",
            paint.muted,
        );
    }
}

fn companion(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    form: &Form,
    config: &Config,
    paint: &Paint,
    focused: bool,
) {
    let block = panel(" Companion ", config, paint, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(form.name)
            .style(paint.accent)
            .alignment(Alignment::Center),
        row(inner, 0, 1),
    );
    frame.render_widget(
        Paragraph::new(form.element)
            .style(paint.muted)
            .alignment(Alignment::Center),
        row(inner, 1, 1),
    );
    for (index, motif) in form.motif.iter().enumerate() {
        frame.render_widget(
            Paragraph::new(*motif)
                .style(paint.accent)
                .alignment(Alignment::Center),
            row(inner, 3 + index as u16, 1),
        );
    }
    text(frame, row(inner, 7, 3), form.role, paint.base);
    if inner.height >= 14 {
        text(
            frame,
            row(inner, 11, 2),
            "One active form.\nNo tools running.",
            paint.muted,
        );
    }
    if inner.height >= 19 {
        text(
            frame,
            row(inner, 15, 3),
            if app.goal.is_empty() {
                "No session goal yet.\ng Edit local goal"
            } else {
                "Session goal set.\ng Edit local goal"
            },
            paint.muted,
        );
    }
    text(
        frame,
        row(inner, inner.height.saturating_sub(1), 1),
        "f Forms | Esc Back",
        paint.muted,
    );
}

fn inspector_view(
    frame: &mut Frame,
    area: Rect,
    _app: &App,
    form: &Form,
    config: &Config,
    paint: &Paint,
    focused: bool,
) {
    let block = panel(" Session inspector ", config, paint, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    text(frame, row(inner, 0, 1), "PERMISSIONS", paint.accent);
    text(
        frame,
        row(inner, 1, 2),
        "No permissions granted.\nPending approvals: 0",
        paint.base,
    );
    text(frame, row(inner, 4, 1), "EVIDENCE", paint.accent);
    text(
        frame,
        row(inner, 5, 2),
        "No evidence recorded.\nRepository not inspected.",
        paint.base,
    );
    text(
        frame,
        row(inner, 8, 2),
        "Backend not connected.\nGoal is session-local only.",
        paint.muted,
    );
    if inner.height >= 19 {
        text(
            frame,
            row(inner, 12, inner.height.saturating_sub(14)),
            form.evidence,
            paint.muted,
        );
    }
    text(
        frame,
        row(inner, inner.height.saturating_sub(1), 1),
        "Esc Back | Ctrl+C Exit",
        paint.muted,
    );
}

fn forms_overlay(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let area = centered(area, 84, 16);
    frame.render_widget(Clear, area);
    let block = panel(" Choose evolution ", config, paint, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Current: ", paint.muted),
            Span::styled(FORMS[app.form.min(FORMS.len() - 1)].name, paint.accent),
            Span::raw(" | Switching grants no permissions"),
        ]))
        .style(paint.base),
        row(inner, 0, 1),
    );
    let rows = FORMS
        .iter()
        .enumerate()
        .map(|(index, form)| Row::new([FORM_KEYS[index], form.name, form.role]).style(paint.base));
    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Length(10),
            Constraint::Min(1),
        ],
    )
    .column_spacing(1)
    .header(Row::new(["Key", "Form", "Responsibility"]).style(paint.muted))
    .row_highlight_style(paint.selected);
    let mut state = TableState::default().with_selected(Some(app.selected.min(FORMS.len() - 1)));
    frame.render_stateful_widget(table, row(inner, 2, 10), &mut state);
    text(
        frame,
        row(inner, 12, 1),
        "1-9 / Up/Down Select | Enter Confirm | Esc Back",
        paint.muted,
    );
    text(
        frame,
        row(inner, 13, 1),
        "q Quit | Ctrl+C Exit",
        paint.muted,
    );
}

fn palette_overlay(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let area = centered(area, 62, 12);
    frame.render_widget(Clear, area);
    let block = panel(" Command palette ", config, paint, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    for (index, command) in PALETTE.iter().enumerate() {
        let style = if index == app.selected {
            paint.selected
        } else {
            paint.base
        };
        text(frame, row(inner, index as u16 + 1, 1), command, style);
    }
    text(
        frame,
        row(inner, 8, 1),
        "Up/Down Select | Enter Open | Esc Back",
        paint.muted,
    );
    text(frame, row(inner, 9, 1), "q Quit | Ctrl+C Exit", paint.muted);
}

fn help_overlay(frame: &mut Frame, area: Rect, config: &Config, paint: &Paint) {
    let area = centered(area, 72, 16);
    frame.render_widget(Clear, area);
    let block = panel(" Keyboard / local shell ", config, paint, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        inner,
        "f           Choose a form (1-9 inside picker)\ng           Edit the session-local goal\ni / b       Inspector / companion drawer\nCtrl+K      Command palette\nTab         Cycle visible panes\nUp/Down     Move selection\nEnter       Open / confirm\nEsc         Back / cancel goal edit\n?           Keyboard help\nq           Quit outside text entry\nCtrl+C      Exit (no active work in this phase)\n\nNo backend, execution, persistence, or grants.\nForm selection is local and never starts work.",
        paint.base,
    );
}

fn goal_editor(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let area = centered(area, 78, 8);
    frame.render_widget(Clear, area);
    let block = panel(" Edit session goal ", config, paint, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        row(inner, 0, 1),
        "Session only; not persisted or sent to a backend.",
        paint.muted,
    );
    let input = row(inner, 2, 1);
    let (tail, width) = input_tail(&app.input, input.width.saturating_sub(1) as usize);
    frame.render_widget(Paragraph::new(tail).style(paint.base), input);
    if input.width > 0 && input.height > 0 {
        frame.set_cursor_position((input.x + width as u16, input.y));
    }
    text(
        frame,
        row(inner, 4, 1),
        "Enter Apply goal | Esc Cancel | Backspace Delete",
        paint.muted,
    );
    text(
        frame,
        row(inner, 5, 1),
        "Ctrl+C Exit | q types text here",
        paint.muted,
    );
}

fn input_tail(input: &str, columns: usize) -> (&str, usize) {
    let mut start = input.len();
    let mut width = 0;
    for (index, grapheme) in input.grapheme_indices(true).rev() {
        let next_width = Span::raw(grapheme).width();
        if width + next_width > columns {
            break;
        }
        start = index;
        width += next_width;
    }
    (&input[start..], width)
}

#[cfg(test)]
mod tests {
    use super::input_tail;

    #[test]
    fn input_clipping_keeps_graphemes_whole_and_respects_display_columns() {
        assert_eq!(input_tail("前한e\u{301}", 3), ("한e\u{301}", 3));
        assert_eq!(input_tail("前한e\u{301}", 2), ("e\u{301}", 1));
        assert_eq!(input_tail("x👩‍💻", 2), ("👩‍💻", 2));
        assert_eq!(input_tail("한", 1), ("", 0));
        assert_eq!(input_tail("e\u{301}", 0), ("", 0));
    }
}
