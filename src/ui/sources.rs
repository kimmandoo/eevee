use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Clear, Paragraph, Wrap},
};

use super::{Paint, centered, clipped, editor_rows, panel, row, selection_details, text};
use crate::{
    app::{
        App, ScrollSurface,
        sources::{EditKind, Mode},
    },
    config::Config,
};

pub(super) fn render(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(2));
    let area = centered(area, 104, area.height.saturating_sub(2));
    frame.render_widget(Clear, area);
    let block = panel(" Sources / task-owned context ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let state = &app.sources;
    text(
        frame,
        row(content, 0, 2),
        if state.busy() {
            "Context: working / existing capture retained"
        } else {
            &state.status
        },
        paint.accent,
    );
    let list_rows = if matches!(state.mode, Mode::Files | Mode::Basket) {
        (content.height / 3).clamp(1, 7)
    } else {
        0
    };
    if list_rows > 0 {
        let (selection, count) = if state.mode == Mode::Files {
            (state.file_selection, state.files.len())
        } else {
            (state.basket_selection, state.basket.len())
        };
        let start = selection.saturating_sub(list_rows.saturating_sub(1) as usize);
        for index in start..(start + list_rows as usize).min(count) {
            let label = if state.mode == Mode::Files {
                format!(
                    "{} {:?}",
                    if index == selection { ">" } else { " " },
                    state.files[index].path
                )
            } else {
                let excerpt = &state.basket[index];
                format!(
                    "{} #{} {} {:?}:{}-{}",
                    if index == selection { ">" } else { " " },
                    excerpt.id,
                    if excerpt.available {
                        "stored"
                    } else {
                        "UNAVAILABLE"
                    },
                    excerpt.path,
                    excerpt.start_line,
                    excerpt.end_line
                )
            };
            text(
                frame,
                row(content, 2 + (index - start) as u16, 1),
                &clipped(&label, content.width as usize),
                if index == selection {
                    paint.selected
                } else {
                    paint.surface
                },
            );
        }
    }
    let details = row(
        content,
        2 + list_rows,
        content.height.saturating_sub(2 + list_rows),
    );
    let lines: Vec<_> = state
        .error
        .lines()
        .map(|line| Line::styled(line, paint.accent))
        .chain(state.detail.lines().map(Line::from))
        .collect();
    let max = selection_details(
        frame,
        details,
        Paragraph::new(lines)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app.scroll_for(ScrollSurface::Preview),
    );
    app.set_scroll_max(ScrollSurface::Preview, max);
    if app.sources.editor.is_some() {
        editor(frame, area, app, config, paint);
    }
    if app.sources.confirmation.is_some() {
        confirmation(frame, area, app, config, paint);
    }
}

fn editor(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let Some(editor) = &app.sources.editor else {
        return;
    };
    let area = centered(area, 82, 12);
    frame.render_widget(Clear, area);
    let block = panel(
        match editor.kind {
            EditKind::Search => " Filename filter / literal ",
            EditKind::Range => " Source range / inclusive ",
        },
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let hint = if !app.sources.error.is_empty() {
        app.sources.error.as_str()
    } else {
        match editor.kind {
            EditKind::Search => "Case-sensitive filename substring; excluded paths stay excluded.",
            EditKind::Range => "One-based start:end, e.g. 2:5. Exact range; no silent clipping.",
        }
    };
    text(frame, row(content, 0, 2), hint, paint.muted);
    let input = row(content, 2, content.height.saturating_sub(2));
    let (lines, (caret_row, caret_column)) = editor_rows(
        &editor.text,
        editor.cursor,
        input.width.saturating_sub(1) as usize,
    );
    let top = caret_row.saturating_sub(input.height.saturating_sub(1) as usize);
    frame.render_widget(
        Paragraph::new(
            lines
                .iter()
                .skip(top)
                .map(|line| Line::from(line.as_str()))
                .collect::<Vec<_>>(),
        )
        .style(paint.surface),
        input,
    );
    if input.width > 0 && input.height > 0 {
        frame.set_cursor_position((
            input.x + caret_column.min(input.width.saturating_sub(1) as usize) as u16,
            input.y + (caret_row - top) as u16,
        ));
    }
}

fn confirmation(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let Some(confirmation) = app.sources.confirmation else {
        return;
    };
    let area = centered(area, 76, 20);
    frame.render_widget(Clear, area);
    let block = panel(" Review local excerpt removal ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let target = app
        .sources
        .basket
        .iter()
        .find(|excerpt| excerpt.id == confirmation.id);
    let value = format!(
        "{} excerpt #{}?\n{}\n\nSelected capture: {}\n\nSource files and other tasks' captures remain unchanged. Local purge is not forensic secure erasure and does not affect exports or provider sessions.",
        if confirmation.purge {
            "Purge stored bytes of"
        } else {
            "Remove the basket reference for"
        },
        confirmation.id,
        if confirmation.purge {
            "Keep unavailable identity metadata; future assembly must refuse it."
        } else {
            "Remove this task's reference and captured text only."
        },
        target
            .map(|excerpt| format!(
                "{:?}:{}-{}",
                excerpt.path, excerpt.start_line, excerpt.end_line
            ))
            .unwrap_or_else(|| "Unavailable; storage will refuse a stale target.".into())
    );
    let details = row(content, 0, content.height.saturating_sub(4));
    let max = selection_details(
        frame,
        details,
        Paragraph::new(value)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app.scroll_for(ScrollSurface::Preview),
    );
    app.set_scroll_max(ScrollSurface::Preview, max);
    text(
        frame,
        row(content, content.height.saturating_sub(3), 1),
        "Stay / keep the excerpt",
        if confirmation.destructive {
            paint.surface
        } else {
            paint.selected
        },
    );
    text(
        frame,
        row(content, content.height.saturating_sub(2), 1),
        if confirmation.purge {
            "Purge selected local captured bytes"
        } else {
            "Remove selected basket reference"
        },
        if confirmation.destructive {
            paint.selected
        } else {
            paint.surface
        },
    );
}
