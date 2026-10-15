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
    let rows = content.height.saturating_sub(2) as usize;
    let count = app.composer_candidates().count();
    let start = menu
        .selected
        .saturating_sub(rows / 2)
        .min(count.saturating_sub(rows));
    if count == 0 || app.sources.busy() || app.task_id <= 0 || !app.sources.error.is_empty() {
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
        row(content, content.height.saturating_sub(2), 1),
        &if count == 0 {
            "0 matches".to_owned()
        } else {
            format!("{}-{}/{count} files", start + 1, (start + rows).min(count))
        },
        paint.muted,
    );
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
    let area = super::command_popup_area(area, app);
    frame.render_widget(Clear, area);
    let block = panel(" / Commands / type to search ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let count = popup.candidates().count();
    let selected = popup
        .selected
        .and_then(|index| popup.candidates().nth(index));
    let reason = selected.and_then(|choice| {
        choice
            .reason
            .clone()
            .or_else(|| app.command_disabled_reason(choice.command))
    });
    let detail = reason
        .as_deref()
        .or_else(|| selected.map(|choice| choice.label.as_str()))
        .unwrap_or("Search actions by name or description. Esc keeps your draft.");
    let detail = if reason.is_some() {
        std::borrow::Cow::Owned(format!("Unavailable: {detail}"))
    } else {
        std::borrow::Cow::Borrowed(detail)
    };
    let detail_rows = Paragraph::new(detail.as_ref())
        .wrap(ratatui::widgets::Wrap { trim: false })
        .line_count(content.width.max(1))
        .min(2) as u16;
    let rows = content.height.saturating_sub(detail_rows + 2) as usize;
    let start = popup
        .selected
        .unwrap_or(0)
        .saturating_sub(rows / 2)
        .min(count.saturating_sub(rows));
    let end = (start + rows).min(count);
    let range = if count == 0 {
        "0 matches".to_owned()
    } else {
        format!("{}-{end}/{count}", start + 1)
    };
    let query_width = content.width.saturating_sub(range.len() as u16 + 1);
    text(
        frame,
        Rect {
            width: query_width,
            ..row(content, 0, 1)
        },
        &clipped(&format!("/{}", popup.query), query_width as usize),
        paint.accent,
    );
    text(
        frame,
        Rect {
            x: content.x + query_width + 1,
            width: content.width.saturating_sub(query_width + 1),
            ..row(content, 0, 1)
        },
        &range,
        paint.muted,
    );
    let command_width = popup
        .candidates()
        .map(|choice| choice.command.len() + 1)
        .max()
        .unwrap_or(0)
        .min(content.width.saturating_sub(5) as usize / 2);
    for (visible, choice) in popup.candidates().skip(start).take(rows).enumerate() {
        let selected = popup.selected == Some(start + visible);
        let unavailable =
            choice.reason.is_some() || app.command_disabled_reason(choice.command).is_some();
        let style = if selected {
            paint.selected
        } else if unavailable {
            paint.muted
        } else {
            paint.surface
        };
        let command = format!("/{}", choice.command);
        let command = clipped(&command, command_width);
        let label = format!(
            "{}{} {command:command_width$} {}",
            if selected { ">" } else { " " },
            if unavailable { "x" } else { " " },
            choice.label,
        );
        text(
            frame,
            row(content, visible as u16 + 1, 1),
            &clipped(&label, content.width as usize),
            style,
        );
    }
    if count == 0 {
        text(
            frame,
            row(content, 1, rows as u16),
            "No matching command. Try a shorter search.",
            paint.muted,
        );
    }
    text(
        frame,
        row(
            content,
            content.height.saturating_sub(detail_rows + 1),
            detail_rows,
        ),
        &clipped(&detail, content.width as usize * detail_rows as usize),
        paint.muted,
    );
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        "Up/Down Choose / Enter Open / Esc Back / Ctrl+K All",
        paint.muted,
    );
}
