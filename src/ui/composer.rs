use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use super::{Paint, clipped, panel, row, text};
use crate::{app::App, config::Config};

pub(super) fn render(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let Some(menu) = app.composer_menu else {
        return;
    };
    if area.height < 4 || area.width < 8 {
        return;
    }
    frame.render_widget(Clear, area);
    let block = panel(" @ Files / inspect before attaching ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let rows = content.height.saturating_sub(1) as usize;
    let start = menu.selected.saturating_sub(rows.saturating_sub(1));
    if app.composer_candidates().count() == 0
        || app.sources.busy()
        || app.task_id <= 0
        || !app.sources.error.is_empty()
    {
        let status = if app.task_id <= 0 {
            "Saving local draft before selecting context..."
        } else if app.sources.busy() {
            "Finding eligible repository files..."
        } else if !app.sources.error.is_empty() {
            &app.sources.error
        } else {
            "No eligible matches. Ignored/binary/linked files excluded."
        };
        text(frame, row(content, 0, rows as u16), status, paint.muted);
    } else {
        for (visible, index) in app.composer_candidates().skip(start).take(rows).enumerate() {
            let selected = start + visible == menu.selected;
            let style = if selected {
                paint.selected
            } else {
                paint.surface
            };
            let label = format!("{:?}", app.sources.files[index].path);
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(if selected { "> " } else { "  " }, style),
                    Span::styled(
                        clipped(&label, content.width.saturating_sub(2) as usize),
                        style,
                    ),
                ])),
                row(content, visible as u16, 1),
            );
        }
    }
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        "Arrows Choose / Enter Inspect / Esc Keep text",
        paint.muted,
    );
}

pub(super) fn commands(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let Some(popup) = &app.command_popup else {
        return;
    };
    // Keep storage/pending state visible even at the minimum supported size.
    let viewport = Rect {
        y: area.y.saturating_add(2),
        height: area.height.saturating_sub(4),
        ..area
    };
    let area = super::centered(viewport, 100, 22);
    frame.render_widget(Clear, area);
    let block = panel(" / Commands / type to search ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        row(content, 0, 1),
        &format!("/{}", popup.query),
        paint.accent,
    );
    let rows = content.height.saturating_sub(4) as usize;
    let start = popup
        .selected
        .unwrap_or(0)
        .saturating_sub(rows.saturating_sub(1));
    let mut selected_reason = None;
    for (visible, choice) in popup.candidates().skip(start).take(rows).enumerate() {
        let selected = popup.selected == Some(start + visible);
        let reason = choice
            .reason
            .clone()
            .or_else(|| app.command_disabled_reason(choice.command));
        let style = if selected {
            paint.selected
        } else if reason.is_some() {
            paint.muted
        } else {
            paint.surface
        };
        let label = format!(
            "{} /{}  {}{}",
            if selected { ">" } else { " " },
            choice.command,
            choice.label,
            if reason.is_some() {
                " [unavailable]"
            } else {
                ""
            }
        );
        text(
            frame,
            row(content, visible as u16 + 1, 1),
            &clipped(&label, content.width as usize),
            style,
        );
        if selected {
            selected_reason = reason;
        }
    }
    if popup.candidates().count() == 0 {
        text(
            frame,
            row(content, 1, 2),
            "No command on this surface matches. Esc preserves your draft; paste is never a command.",
            paint.muted,
        );
    }
    text(
        frame,
        row(content, content.height.saturating_sub(3), 2),
        selected_reason.as_deref().unwrap_or(&app.notice),
        paint.muted,
    );
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        "Type / arrows Choose • Enter Open • Esc Back • Ctrl+K All choices",
        paint.muted,
    );
}
