//! Fixed neutral controls with contiguous exposure of the complete install/data scope.
use super::*;
use crate::platform::distribution::Operation;

pub(super) fn render(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let obscured = app.command_popup.is_some() || app.confirmation.is_some();
    let state = &mut app.updates;
    let area = centered(area, 112, area.height.saturating_sub(2).min(32));
    frame.render_widget(Clear, area);
    let block = panel(
        paint.hint(if state.operation == Some(Operation::Rollback) {
            Message::LifecycleRollbackTitle
        } else {
            Message::LifecycleUpdateTitle
        }),
        config,
        paint,
        true,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if frame.area().width < 60 || frame.area().height < 18 || inner.height < 7 {
        state.invalidate();
        return;
    }
    let body = row(inner, 0, inner.height.saturating_sub(3));
    if state.scope.is_empty() {
        if let Some(review) = &state.review {
            let cache = paint.hint(if review.operation == Operation::Rollback {
                Message::LifecycleRollbackCache
            } else {
                Message::LifecycleUpdateCache
            });
            let data = paint.hint(if review.operation == Operation::Rollback {
                Message::LifecycleRollbackData
            } else {
                Message::LifecycleUpdateData
            });
            state.scope = paint.format(
                Template::LifecycleScope,
                &[
                    ("current", &review.current_version),
                    ("target", &review.target_version),
                    ("source", &display_identity(&review.source)),
                    (
                        "directory",
                        &display_identity(&review.installation_directory.to_string_lossy()),
                    ),
                    ("cache", cache),
                    ("data", data),
                    ("reason", &display_identity(&review.reason)),
                ],
            );
        } else if !state.error.is_empty() {
            state.scope = format!(
                "{}\n\n{}",
                paint.hint(state.message.unwrap_or(Message::LifecycleUnavailable)),
                display_identity(&state.error),
            );
        } else {
            state.scope = paint
                .hint(state.message.unwrap_or(Message::LifecycleWaiting))
                .to_owned();
        }
    }
    if state.scope_width != body.width || state.scope_rows == 0 {
        let total = Paragraph::new(state.scope.as_str())
            .wrap(Wrap { trim: false })
            .line_count(body.width.max(1));
        state.invalidate();
        state.scope_width = body.width;
        state.scope_rows = total;
    }
    let total = state.scope_rows;
    state.page_rows = body.height;
    state.scroll_max = total
        .saturating_sub(usize::from(body.height))
        .min(u16::MAX as usize) as u16;
    state.scroll = state.scroll.min(state.scroll_max);
    if !obscured && usize::from(state.scroll) <= state.seen_rows {
        state.seen_rows = state
            .seen_rows
            .max(usize::from(state.scroll) + usize::from(body.height))
            .min(total);
    }
    state.visible = !obscured && state.seen_rows >= total && state.review.is_some();
    frame.render_widget(
        Paragraph::new(state.scope.as_str())
            .style(paint.surface)
            .wrap(Wrap { trim: false })
            .scroll((state.scroll, 0)),
        body,
    );
    let eligible = state.review.as_ref().is_some_and(|review| review.can_apply);
    let controls = if state.visible && eligible {
        if state.choice {
            Message::LifecycleChooseApply
        } else {
            Message::LifecycleChooseStay
        }
    } else {
        Message::LifecycleInspect
    };
    frame.render_widget(
        Paragraph::new(paint.hint(controls))
            .wrap(Wrap { trim: false })
            .style(paint.accent),
        row(inner, inner.height.saturating_sub(3), 3),
    );
}
