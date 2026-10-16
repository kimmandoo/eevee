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
        row(content, 0, 1),
        &clipped(
            &format!(
                "Task #{} · {} attached · {}",
                app.task_id,
                state.basket.len(),
                if state.persistence_pending() {
                    "saving"
                } else if state.store_failed {
                    "save failed"
                } else if state.busy() {
                    "validating"
                } else {
                    "local only"
                },
            ),
            content.width as usize,
        ),
        paint.accent,
    );
    text(
        frame,
        row(content, 1, 1),
        &clipped(&state.status, content.width as usize),
        paint.muted,
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
                    "{} [{}] {:?}",
                    if index == selection { ">" } else { " " },
                    if state.file_attachment(index).is_some() {
                        "+"
                    } else {
                        " "
                    },
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
                } else if state.mode == Mode::Files && state.file_attachment(index).is_some() {
                    paint.accent
                } else {
                    paint.surface
                },
            );
        }
    }
    let details = row(
        content,
        2 + list_rows,
        content.height.saturating_sub(4 + list_rows),
    );
    let lines: Vec<_> = state
        .error
        .lines()
        .map(|line| Line::styled(line, paint.accent))
        .chain(state.detail.lines().map(Line::from))
        .collect();
    let preview_reviewed = state.mode == Mode::Preview
        && state.editor.is_none()
        && state.confirmation.is_none()
        && state.preview.as_ref().is_some_and(|excerpt| {
            excerpt.task_id == app.task_id
                && excerpt.available
                && frame.area().width >= 60
                && frame.area().height >= 18
                && details.height >= 4
        });
    let max = selection_details(
        frame,
        details,
        Paragraph::new(lines)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app.scroll_for(ScrollSurface::Preview),
    );
    app.set_scroll_max(ScrollSurface::Preview, max);
    text(
        frame,
        row(content, content.height.saturating_sub(2), 2),
        if app.sources.store_failed {
            "Enter Retry retained local save · Esc Draft\nCapture kept; nothing sent to a provider"
        } else if app.sources.busy() {
            "Saving/validating locally; attachment not committed\nPgUp/Dn Details · Esc Draft"
        } else {
            match app.sources.mode {
                Mode::Files => {
                    "Enter Review/attach · s Search · r Range · Del Remove\nTab Attachments · PgUp/Dn Details · Esc Draft"
                }
                Mode::Preview => {
                    "Enter Attach locally & return · r Range\nTab Attachments · PgUp/Dn Review · Esc Cancel"
                }
                Mode::Basket => {
                    "Del Remove (review) · Tab Files · / More\nPgUp/Dn Details · Esc Draft"
                }
                Mode::Payload => "/ More · Tab Files · PgUp/Dn Details · Esc Draft",
            }
        },
        paint.muted,
    );
    app.sources.preview_reviewed = preview_reviewed;
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
    let area = centered(area, 76, if app.verbose { 20 } else { 14 });
    frame.render_widget(Clear, area);
    let block = panel(" Review local excerpt removal ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let target = app
        .sources
        .basket
        .iter()
        .find(|excerpt| excerpt.id == confirmation.id);
    let available = target.is_some();
    let capture = target
        .map(|excerpt| {
            format!(
                "{:?}:{}-{}",
                excerpt.path, excerpt.start_line, excerpt.end_line
            )
        })
        .unwrap_or_else(|| "Unavailable; reload before confirming.".into());
    let details = row(content, 0, content.height.saturating_sub(4));
    let summary = format!(
        "{} excerpt #{} from this task?\nCapture: {}\n{}\nOther files/tasks and provider data stay unchanged.\nNot secure erasure; exports remain unchanged.",
        if confirmation.purge {
            "Purge"
        } else {
            "Remove"
        },
        confirmation.id,
        super::clipped(&capture, details.width.saturating_sub(9) as usize),
        if confirmation.purge {
            "Clear local bytes; keep unavailable identity."
        } else {
            "Remove local reference and captured text."
        }
    );
    let summary = Paragraph::new(summary)
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    let rows = summary.line_count(details.width);
    if let Some(review) = &mut app.sources.confirmation {
        review.reviewed = available
            && frame.area().width >= 60
            && frame.area().height >= 18
            && rows <= usize::from(details.height);
    }
    frame.render_widget(summary, row(details, 0, (rows as u16).min(details.height)));
    let remaining = details.height.saturating_sub(rows as u16);
    let max = if app.verbose && remaining > 0 {
        selection_details(
            frame,
            row(details, rows as u16, remaining),
            Paragraph::new(format!("Exact capture: {capture}"))
                .style(paint.muted)
                .wrap(Wrap { trim: false }),
            app.scroll_for(ScrollSurface::Preview),
        )
    } else {
        0
    };
    app.set_scroll_max(ScrollSurface::Preview, max);
    text(
        frame,
        row(content, content.height.saturating_sub(3), 1),
        "n / Stay - keep the excerpt",
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
            "y / Purge selected local captured bytes"
        } else {
            "y / Remove selected basket reference"
        },
        if confirmation.destructive {
            paint.selected
        } else {
            paint.surface
        },
    );
}
