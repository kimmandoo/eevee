use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Clear, Paragraph, Wrap},
};

use super::{Paint, clipped, editor_rows, panel, row, selection_details, text};
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
        paint.hint(crate::messages::Message::DesksHandoffLocalHandoffTerminalTooSmallNeedColumns),
        paint.accent,
    );
}

pub(super) fn render(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    if area.width < 60 || area.height < 18 {
        render_small(frame, area, &mut app.handoff, paint);
        return;
    }
    let compact = app.handoff.mode != Mode::Idle
        && app.handoff.candidate.is_some()
        && !app.handoff.editing
        && !super::VERBOSE.get();
    let height = if compact {
        18
    } else {
        area.height.saturating_sub(2)
    };
    let area = super::sheet(area, 110, height);
    frame.render_widget(Clear, area);
    let block = panel(
        paint.hint(crate::messages::Message::DesksHandoffPanel),
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let state = &mut app.handoff;
    let header = if state.editing {
        paint.hint(crate::messages::Message::DesksHandoffLiteralFilePathNoActionUntilReview)
    } else {
        match state.mode {
            Mode::Export(_) => paint
                .hint(crate::messages::Message::DesksHandoffSanitizedExportPreviewNoFileWrittenYet),
            Mode::Import => paint
                .hint(crate::messages::Message::DesksHandoffUserSuppliedImportReviewNotAppliedOr),
            Mode::Idle => {
                paint.hint(crate::messages::Message::DesksHandoffLocalOnlyNoProviderOrModelLaunched)
            }
        }
    };
    text(frame, row(content, 0, 1), header, paint.accent);
    text(
        frame,
        row(content, 1, if compact { 2 } else { 3 }),
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
            paint.hint(crate::messages::Message::DesksHandoffEnterReadPreviewEscBackNewExport),
            paint.accent,
        );
        return;
    }
    let path = if state.path.is_empty() {
        paint
            .hint(crate::messages::Message::DesksHandoffNoPathChosen)
            .to_string()
    } else {
        format!("Chosen path: {}", state.path)
    };
    text(
        frame,
        row(content, if compact { 3 } else { 4 }, 1),
        &clipped(&path, content.width as usize),
        paint.surface,
    );
    let footer_rows = if compact { 2 } else { 3 };
    let body_start = if compact { 4 } else { 6 };
    let body = row(
        content,
        body_start,
        content.height.saturating_sub(body_start + footer_rows),
    );
    let empty =
        paint.hint(crate::messages::Message::DesksHandoffExportJsonExportMarkdownImportHandoffOnly);
    let empty = super::brief(empty);
    let value = if state.mode != Mode::Idle && state.candidate.is_some() {
        let effect = match state.mode {
            Mode::Export(_) => {
                paint.hint(crate::messages::Message::DesksHandoffWriteANewFileOnlyNoOverwrite)
            }
            _ => paint
                .hint(crate::messages::Message::DesksHandoffAcceptUnverifiedContextOnlyNoTaskFacts),
        };
        if super::VERBOSE.get() {
            format!("{effect}\nPath: {}\n\n{}", state.path, state.preview)
        } else {
            let document = state.candidate.as_ref().expect("handoff preview");
            paint.format(
                crate::messages::Template::DesksHandoffPayloadSummary,
                &[
                    ("effect", (effect)),
                    ("sources", &(document.sources.len()).to_string()),
                    ("decisions", &(document.decisions.len()).to_string()),
                    ("bytes", &(state.preview.len()).to_string()),
                ],
            )
        }
    } else if let Some(decision) = state.selected_import_decision() {
        paint.format(
            crate::messages::Template::DesksHandoffDecisionPreview,
            &[
                ("index", &(state.selected_decision + 1).to_string()),
                ("decision", (decision)),
            ],
        )
    } else if state.preview.is_empty() {
        empty.to_string()
    } else {
        format!("Chosen literal path: {}\n\n{}", state.path, state.preview)
    };
    if compact {
        state.scroll = 0;
    }
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
        && body.height >= 3
    {
        state.reviewed = !compact || max == 0;
    }
    let footer = if state.mode != Mode::Idle {
        let action = match state.mode {
            Mode::Export(_) => paint.hint(crate::messages::Message::DesksHandoffWriteNewFile),
            _ => paint.hint(crate::messages::Message::DesksHandoffAcceptUnverifiedContext),
        };
        paint.format(
            crate::messages::Template::DesksHandoffConfirmControls,
            &[
                (
                    "stay",
                    (if state.affirmative {
                        paint.hint(crate::messages::Message::CommonStay)
                    } else {
                        paint.hint(crate::messages::Message::CommonStaySelected)
                    }),
                ),
                ("open", (if state.affirmative { "[" } else { "" })),
                ("action", (action)),
                ("close", (if state.affirmative { "]" } else { "" })),
                (
                    "details",
                    (if state.reviewed {
                        paint.hint(crate::messages::Message::DesksHandoffVerboseDetails)
                    } else {
                        paint.hint(crate::messages::Message::CommonEnlargeReview)
                    }),
                ),
            ],
        )
    } else if state.selected_import_decision().is_some() {
        let count = state
            .candidate
            .as_ref()
            .map_or(0, |document| document.decisions.len());
        paint.format(
            crate::messages::Template::DesksHandoffDecisionControls,
            &[
                (
                    "index",
                    &(if count == 0 {
                        0
                    } else {
                        state.selected_decision + 1
                    })
                    .to_string(),
                ),
                ("count", &(count).to_string()),
            ],
        )
    } else {
        paint
            .hint(crate::messages::Message::DesksHandoffExportJsonExportMarkdownImportHandoffPgup)
            .into()
    };
    text(
        frame,
        row(
            content,
            content.height.saturating_sub(footer_rows),
            footer_rows,
        ),
        &footer,
        paint.accent,
    );
}
