use ratatui::{Frame, layout::Rect};

use super::{Paint, clipped, row, text};
use crate::{app::App, i18n::Message, workspace::context};

/// A one-row viewport over durable exact-ID captures. Identity/range and action
/// columns have fixed space; a long grapheme path can consume only its own column.
pub(super) fn render(frame: &mut Frame, area: Rect, app: &App, paint: &Paint) {
    let Some((index, capture)) = app
        .sources
        .chip_focus
        .and_then(|id| {
            app.sources
                .basket
                .iter()
                .enumerate()
                .find(|(_, capture)| capture.id == id)
        })
        .or_else(|| app.sources.basket.iter().enumerate().next())
    else {
        return;
    };
    if area.height == 0 || area.width == 0 {
        return;
    }
    let focused = app.sources.chip_focus == Some(capture.id);
    let style = if focused {
        paint.selected
    } else {
        paint.accent
    };
    let controls = if focused {
        format!(
            "{}  {}",
            paint.hint(Message::ChipsInspect),
            paint.hint(Message::ChipsRemove)
        )
    } else {
        paint.hint(Message::ChipsFocus).to_owned()
    };
    let identity = format!(
        "{}#{} {}:{} [{}/{}] ",
        if focused { ">" } else { "" },
        capture.id,
        capture.start_line,
        capture.end_line,
        index + 1,
        app.sources.basket.len()
    );
    let (identity_area, path_area, controls_area) = columns(area, &identity, &controls);
    text(frame, identity_area, &identity, style);
    let path = format!("{:?}", capture.path);
    text(
        frame,
        path_area,
        &clipped(&path, path_area.width as usize),
        style,
    );
    text(frame, controls_area, &controls, style);
    if area.height > 1 {
        let freshness = app
            .sources
            .observations
            .iter()
            .find(|observation| observation.id == capture.id)
            .map(|observation| match &observation.freshness {
                context::Freshness::Current => Message::ChipsCurrent,
                context::Freshness::Stale(_) => Message::ChipsStale,
                context::Freshness::Unavailable(_) => Message::ChipsUnavailable,
            })
            .unwrap_or(if capture.available {
                Message::ChipsUnchecked
            } else {
                Message::ChipsUnavailable
            });
        let wire = app
            .sources
            .wire
            .as_ref()
            .and_then(|receipt| receipt.captures.iter().find(|entry| entry.id == capture.id))
            .map(|entry| {
                format!(
                    "{} B ~{} tok ({})",
                    entry.bytes,
                    entry.estimated_tokens,
                    crate::workspace::receipt::ESTIMATOR
                )
            })
            .unwrap_or_else(|| paint.hint(Message::ChipsWirePending).to_owned());
        text(
            frame,
            row(area, 1, 1),
            &clipped(
                &format!("{} · {wire}", paint.hint(freshness)),
                area.width as usize,
            ),
            paint.muted,
        );
    }
    if area.height > 2 {
        text(
            frame,
            row(area, 2, 1),
            paint.hint(Message::ChipsHint),
            paint.muted,
        );
    }
}

fn columns(area: Rect, identity: &str, controls: &str) -> (Rect, Rect, Rect) {
    let controls_width = ratatui::text::Span::raw(controls)
        .width()
        .min(area.width as usize) as u16;
    let remaining = area.width.saturating_sub(controls_width);
    let identity_width = ratatui::text::Span::raw(identity)
        .width()
        .min(remaining as usize) as u16;
    let controls_area = Rect {
        x: area.x + area.width - controls_width,
        width: controls_width,
        height: 1,
        ..area
    };
    let identity_area = Rect {
        width: identity_width,
        height: 1,
        ..area
    };
    let path_area = Rect {
        x: area.x + identity_width,
        width: remaining.saturating_sub(identity_width + 1),
        height: 1,
        ..area
    };
    (identity_area, path_area, controls_area)
}

pub(super) fn rank_reason(reason: crate::app::sources::RankReason, paint: &Paint) -> &'static str {
    use crate::app::sources::RankReason;
    paint.hint(match reason {
        RankReason::Failure => Message::ChipsFailure,
        RankReason::Changed => Message::ChipsChanged,
        RankReason::Tests => Message::ChipsTests,
        RankReason::ChangedTests => Message::ChipsChangedTests,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, widgets::Paragraph};

    #[test]
    fn long_unicode_paths_cannot_hide_fixed_identity_or_controls() {
        for width in [60, 80, 140] {
            let area = Rect::new(0, 0, width, 1);
            let identity = "#941 12:24 [2/3] ";
            for controls in ["Enter inspect  Del remove", "Enter 확인  Del 제거"] {
                let (id, path, actions) = columns(area, identity, controls);
                let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
                terminal
                    .draw(|frame| {
                        frame.render_widget(Paragraph::new(identity), id);
                        frame.render_widget(
                            Paragraph::new(clipped(&"資料한글👩‍💻".repeat(200), path.width as usize)),
                            path,
                        );
                        frame.render_widget(Paragraph::new(controls), actions);
                    })
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let screen: String = (0..width).map(|x| buffer[(x, 0)].symbol()).collect();
                assert!(screen.contains(identity));
                let mut column = actions.x;
                for character in controls.chars() {
                    assert_eq!(buffer[(column, 0)].symbol(), character.to_string());
                    column += ratatui::text::Span::raw(character.to_string()).width() as u16;
                }
                assert_eq!(actions.right(), area.right());
                assert!(path.right() < actions.x || path.width == 0);
            }
        }
    }
}
