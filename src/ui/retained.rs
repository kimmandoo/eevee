//! Literal retained work, attributed without borrowing transcript navigation state.
use super::*;
use crate::app::retained::Mode;
pub(super) fn render(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    frame.render_widget(Clear, area);
    if app.width < 60 || app.height < 18 {
        text(
            frame,
            area,
            "Retained navigation requires 60×18; Esc returns without changing drafts",
            paint.accent,
        );
        return;
    }
    let area = centered(area, 112, area.height.min(32));
    let block = panel("Retained work / local only", config, paint, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let state = &app.retained;
    let mut lines = vec![Line::from(
        "Local only; drafts, owners and paused output unchanged.",
    )];
    if state.pending.is_some() || state.inflight {
        lines.push(Line::from("Reading bounded retained data… Esc cancels"));
    }
    if !state.error.is_empty() {
        lines.push(Line::styled(state.error.clone(), paint.accent));
    }
    if let Some(editor) = &state.editor {
        lines.push(Line::from(
            "Landmark label (1–128 UTF-8 bytes); Enter save metadata / Esc cancel",
        ));
        lines.push(Line::from(format!("{}|", display_identity(editor))));
        if let Some(hit) = &state.opened {
            lines.push(Line::from(format!(
                "task {} / {} / character {}",
                hit.task,
                display_identity(&hit.attribution),
                state.position
            )));
        }
    } else {
        match state.mode {
            Mode::Search | Mode::Responses => {
                if state.mode == Mode::Search {
                    lines.push(Line::from(format!(
                        "Literal: {}| / include archived: {} (Tab)",
                        display_identity(&state.query),
                        state.archived
                    )));
                }
                lines.push(Line::from(format!(
                    "{} retained matches / {} fields scanned",
                    state.hits.len(),
                    state.scanned
                )));
                if state.incomplete {
                    lines.push(Line::styled(
                        "INCOMPLETE: scan bounds or partial journal",
                        paint.accent,
                    ));
                }
                if let Some(hit) = state.hits.get(state.selected) {
                    lines.push(Line::styled(
                        format!(
                            "Match {}/{}: task {}{} / {}",
                            state.selected + 1,
                            state.hits.len(),
                            hit.task,
                            if hit.archived { " [archived]" } else { "" },
                            hit.kind
                        ),
                        paint.accent,
                    ));
                    lines.push(Line::from(hit.when.label()));
                    lines.push(Line::from(display_identity(&hit.excerpt).into_owned()));
                }
            }
            Mode::Landmarks => {
                lines.push(Line::from("Current task landmarks (32 max); Enter jump / n rename / d remove / Esc return"));
                if let Some(value) = app.workflows.landmarks.get(state.selected) {
                    lines.push(Line::styled(
                        format!(
                            "Landmark {}/{}: {} / character {}",
                            state.selected + 1,
                            app.workflows.landmarks.len(),
                            display_identity(&value.label),
                            value.position.unwrap_or(0)
                        ),
                        paint.accent,
                    ));
                    lines.push(Line::from(format!(
                        "Retained {}",
                        display_identity(&value.journal_id)
                    )));
                    let attribution = state
                        .hits
                        .iter()
                        .find(|hit| {
                            hit.target
                                == crate::store::retained::Target::Journal(value.journal_id.clone())
                        })
                        .map(|hit| hit.attribution.as_str())
                        .unwrap_or("Unavailable/purged or not yet loaded; jump revalidates");
                    lines.push(Line::from(display_identity(attribution).into_owned()));
                }
                if app.workflows.landmarks.is_empty() {
                    lines.push(Line::from("No landmarks. /retained-responses → Enter → l labels an existing durable response."));
                }
            }
            Mode::Evidence => {
                if let Some(hit) = &state.opened {
                    lines.push(Line::from(format!(
                        "Original task {}{} / {}",
                        hit.task,
                        if hit.archived { " [archived]" } else { "" },
                        hit.kind
                    )));
                    lines.push(Line::from(hit.when.label()));
                    lines.push(Line::from(display_identity(&hit.attribution).into_owned()));
                    lines.push(Line::from(format!(
                        "Character {} / Up Down ±40; PgUp PgDn ±400; l label; Esc back",
                        state.position
                    )));
                }
                let visible = inner.height.saturating_sub(8).max(1) as usize;
                let suffix: String = state
                    .text
                    .chars()
                    .skip(state.position)
                    .take(visible * usize::from(inner.width))
                    .collect();
                for line in suffix.lines() {
                    lines.push(Line::from(display_identity(line).into_owned()));
                }
            }
        }
    }
    if !state.notice.is_empty() {
        lines.push(Line::styled(state.notice.clone(), paint.muted));
    }
    let footer = inner.bottom().saturating_sub(1);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .style(paint.base),
        Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(1),
        ),
    );
    text(
        frame,
        Rect::new(inner.x, footer, inner.width, 1),
        "Enter search/open • Up/Down select • Esc cancel/back • retained data is not verification",
        paint.muted,
    );
}
