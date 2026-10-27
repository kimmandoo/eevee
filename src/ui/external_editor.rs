//! Contiguous full-scope exposure before either external launch or draft replacement.
use super::*;
use ratatui::widgets::Borders;

pub(super) fn render(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let obscured = app.command_popup.is_some() || app.confirmation.is_some();
    let state = &mut app.external_editor;
    let area = centered(area, 112, area.height.saturating_sub(2).min(32));
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" External Request editor ")
        .borders(Borders::ALL)
        .style(paint.surface);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let _ = config;
    if frame.area().width < 60 || frame.area().height < 18 || inner.height < 6 {
        state.invalidate();
        return;
    }
    let body = row(inner, 0, inner.height.saturating_sub(3));
    if state.scope_width != body.width || state.scope_rows == 0 {
        let total = Paragraph::new(state.scope.as_str())
            .wrap(Wrap { trim: false })
            .line_count(body.width.max(1));
        state.invalidate();
        state.scope_width = body.width;
        state.scope_rows = total;
    }
    state.page_rows = body.height;
    state.scroll_max = state
        .scope_rows
        .saturating_sub(usize::from(body.height))
        .min(u16::MAX as usize) as u16;
    state.scroll = state.scroll.min(state.scroll_max);
    if !obscured && usize::from(state.scroll) <= state.seen_rows {
        state.seen_rows = state
            .seen_rows
            .max(usize::from(state.scroll) + usize::from(body.height))
            .min(state.scope_rows);
    }
    state.visible = !obscured && state.seen_rows >= state.scope_rows && state.actionable();
    frame.render_widget(
        Paragraph::new(state.scope.as_str())
            .style(paint.surface)
            .wrap(Wrap { trim: false })
            .scroll((state.scroll, 0)),
        body,
    );
    let controls = if state.visible {
        if state.choice {
            "Stay   [Proceed] — Enter confirms; n/Esc keeps original"
        } else {
            "[Stay]   Proceed — y approves; Tab chooses; Enter/n/Esc keeps original"
        }
    } else {
        "Read the complete scope: arrows/PgDn. No approval yet. Esc keeps original."
    };
    frame.render_widget(
        Paragraph::new(controls)
            .style(paint.accent)
            .wrap(Wrap { trim: false }),
        row(inner, inner.height.saturating_sub(3), 3),
    );
}
