use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Clear, Paragraph, Wrap},
};

use super::{Paint, centered, clipped, editor_rows, panel, row, selection_details, text};
use crate::{
    app::{
        App,
        handoff::{Mode, State},
    },
    config::Config,
};

pub(super) fn render_small(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    state.invalidate_review();
    frame.render_widget(Clear, area);
    text(
        frame,
        area,
        "Local handoff: terminal too small\nNeed 60 columns x 18 rows.\nNo write or acceptance available.\nEsc Stay / resize to review.\nNo provider is launched.",
        paint.accent,
    );
}

pub(super) fn render(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    if area.width < 60 || area.height < 18 {
        render_small(frame, area, &mut app.handoff, paint);
        return;
    }
    let area = centered(area, 110, area.height.saturating_sub(2));
    frame.render_widget(Clear, area);
    let block = panel(" Portable task handoff / local-only ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let state = &mut app.handoff;
    let header = if state.editing {
        "LITERAL FILE PATH / no action until review"
    } else {
        match state.mode {
            Mode::Export(_) => "SANITIZED EXPORT PREVIEW / no file written yet",
            Mode::Import => "USER-SUPPLIED IMPORT REVIEW / not applied or verified",
            Mode::Idle => "LOCAL-ONLY / no provider or model launched",
        }
    };
    text(frame, row(content, 0, 1), header, paint.accent);
    text(
        frame,
        row(content, 1, 3),
        &super::brief(&state.notice),
        paint.muted,
    );
    if state.editing {
        let input = row(content, 5, content.height.saturating_sub(8));
        let (lines, (caret_row, caret_column)) = editor_rows(
            &state.path,
            state.cursor,
            input.width.saturating_sub(1) as usize,
        );
        let top = caret_row.saturating_sub(input.height.saturating_sub(1) as usize);
        let lines: Vec<_> = lines.into_iter().map(Line::from).collect();
        frame.render_widget(
            Paragraph::new(lines)
                .style(paint.surface)
                .scroll((top.min(u16::MAX as usize) as u16, 0)),
            input,
        );
        if input.width > 0 && input.height > 0 {
            frame.set_cursor_position((
                input
                    .x
                    .saturating_add((caret_column as u16).min(input.width.saturating_sub(1))),
                input.y.saturating_add(
                    (caret_row.saturating_sub(top) as u16).min(input.height.saturating_sub(1)),
                ),
            ));
        }
        text(
            frame,
            row(content, content.height.saturating_sub(2), 2),
            "Enter Read/Preview / Esc Stay. New export file only; symlink/reparse paths refused.",
            paint.accent,
        );
        return;
    }
    let path = if state.path.is_empty() {
        "No path chosen".to_string()
    } else {
        format!("Chosen path: {}", state.path)
    };
    text(
        frame,
        row(content, 4, 1),
        &clipped(&path, content.width as usize),
        paint.surface,
    );
    let body = row(content, 6, content.height.saturating_sub(10));
    let empty = "/export-json /export-markdown /import-handoff\n\nOnly committed task facts, criteria, approved source references/excerpt bytes, selected decisions, and unresolved work are portable. No provider session IDs, permissions, credentials, or private reasoning. Imported claims are user-supplied/unverified and never become execution evidence.\n\nReview recognizable-secret redaction before sharing; detection is best-effort, not proof of secret absence. Deleting local data cannot erase prior exports, copies, provider-owned history, or backups. JSON and Markdown require the version-1 strict schema. No automatic task completion or provider request.";
    let empty = super::brief(empty);
    let value = if let Some(decision) = state.selected_import_decision() {
        format!(
            "SELECTED IMPORTED DECISION / {} / user-supplied, unverified\n\n{decision}\n\nCopy opens the existing editor for separate review/save. It creates no execution evidence.",
            state.selected_decision + 1
        )
    } else if state.preview.is_empty() {
        empty.to_string()
    } else {
        format!("Chosen literal path: {}\n\n{}", state.path, state.preview)
    };
    let max = selection_details(
        frame,
        body,
        Paragraph::new(value)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        state.scroll,
    );
    state.scroll_max = max;
    state.scroll = state.scroll.min(max);
    if state.mode != Mode::Idle
        && state.candidate.is_some()
        && state.pending.is_none()
        && body.height > 0
        && state.scroll >= max
    {
        state.reviewed = true;
    }
    let footer = if state.mode != Mode::Idle {
        let action = match state.mode {
            Mode::Export(_) => "Write new file",
            _ => "Accept unverified context",
        };
        format!(
            "{} / {}{}{} | {} | Left/Right choose / Enter / Esc Stay",
            if state.affirmative { "Stay" } else { "[Stay]" },
            if state.affirmative { "[" } else { "" },
            action,
            if state.affirmative { "]" } else { "" },
            if state.reviewed {
                "review complete"
            } else {
                "PgDn review all pages"
            }
        )
    } else if state.selected_import_decision().is_some() {
        let count = state
            .candidate
            .as_ref()
            .map_or(0, |document| document.decisions.len());
        format!(
            "/ Commands | Left/Right decision {}/{} /copy-decision | PgUp/PgDn / Esc",
            if count == 0 {
                0
            } else {
                state.selected_decision + 1
            },
            count
        )
    } else {
        "/export-json /export-markdown /import-handoff\nPgUp/PgDn Scroll / Esc Back".into()
    };
    text(
        frame,
        row(content, content.height.saturating_sub(3), 3),
        &footer,
        paint.accent,
    );
}
