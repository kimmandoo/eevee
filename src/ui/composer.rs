use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use super::{Paint, clipped, panel, row, text};
use crate::{
    app::{
        App,
        composer::{COMMANDS, Kind},
    },
    config::Config,
};

pub(super) fn render(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let Some(menu) = app.composer_menu else {
        return;
    };
    if area.height < 4 || area.width < 8 {
        return;
    }
    frame.render_widget(Clear, area);
    let block = panel(
        match menu.kind {
            Kind::Files => " @ Attach context / inspect first ",
            Kind::Commands => " / Commands ",
        },
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let count = app.composer_candidates().count();
    let rows = content.height.saturating_sub(1) as usize;
    let start = menu.selected.saturating_sub(rows.saturating_sub(1));
    if count == 0
        || (menu.kind == Kind::Files
            && (app.sources.busy() || app.task_id <= 0 || !app.sources.error.is_empty()))
    {
        let status = if menu.kind == Kind::Commands {
            "No matching command. Esc keeps literal text."
        } else if matches!(app.storage, crate::app::StorageState::Failed(_)) && app.task_id <= 0 {
            "Draft save failed. Esc keeps input; inspect storage before attaching."
        } else if app.task_id <= 0 {
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
            let label = match menu.kind {
                Kind::Commands => {
                    format!("/{}  {}", COMMANDS[index].name, COMMANDS[index].description)
                }
                Kind::Files => format!("{:?}", app.sources.files[index].path),
            };
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
        "Type to filter / Arrows Choose / Enter / Esc Keep text",
        paint.muted,
    );
}
