use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use super::{Paint, clipped, panel, row, text};
use crate::{
    app::App,
    config::Config,
    messages::{Message, Template},
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
        paint.hint(Message::ProvidersComposerAttachFileLocalCaptureReview),
        config,
        paint,
        true,
    );
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
            paint.hint(Message::ProvidersComposerSavingLocalDraftBeforeSelectingContext)
        } else if app.sources.busy() {
            paint.hint(Message::ProvidersComposerFindingEligibleRepositoryFiles)
        } else if !app.sources.error.is_empty() {
            &app.sources.error
        } else {
            paint.hint(Message::ProvidersComposerNoEligibleMatchesIgnoredBinaryLinkedFiles)
        };
        text(frame, row(content, 0, rows as u16), status, paint.muted);
    } else {
        for (visible, index) in app.composer_candidates().skip(start).take(rows).enumerate() {
            let selected = start + visible == menu.selected;
            let style = if selected {
                paint.selected
            } else if app.sources.file_attachment(index).is_some() {
                paint.accent
            } else {
                paint.surface
            };
            let label = format!(
                "[{}] {:?}",
                if app.sources.file_attachment(index).is_some() {
                    "+"
                } else {
                    " "
                },
                app.sources.files[index].path
            );
            let reason = app
                .composer_rank_reason(index)
                .map(|reason| super::chips::rank_reason(reason, paint));
            let reason_width = reason.map_or(0, |reason| {
                Span::raw(reason).width().min(content.width as usize / 3) as u16
            });
            let path_width = content
                .width
                .saturating_sub(2 + reason_width + u16::from(reason.is_some()));
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(if selected { "> " } else { "  " }, style),
                    Span::styled(clipped(&label, path_width as usize), style),
                ])),
                row(content, visible as u16, 1),
            );
            if let Some(reason) = reason {
                text(
                    frame,
                    Rect {
                        x: content.right() - reason_width,
                        width: reason_width,
                        ..row(content, visible as u16, 1)
                    },
                    reason,
                    style,
                );
            }
        }
    }
    text(
        frame,
        row(content, content.height.saturating_sub(2), 1),
        &if count == 0 {
            paint.hint(Message::ProvidersComposer0Matches).to_owned()
        } else {
            let (first, last) = (start + 1, (start + rows).min(count));
            paint.format(
                Template::ProvidersComposerFilesRange,
                &[
                    ("first", &first.to_string()),
                    ("last", &last.to_string()),
                    ("count", &count.to_string()),
                ],
            )
        },
        paint.muted,
    );
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        paint.hint(Message::ProvidersComposerEnterReviewAttachDelRemoveEscKeep),
        paint.muted,
    );
}

pub(super) fn commands(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let Some(popup) = &app.command_popup else {
        return;
    };
    let area = super::command_popup_area(area, app);
    frame.render_widget(Clear, area);
    let block = panel(
        paint.hint(Message::ProvidersComposerCommandsTypeToSearch),
        config,
        paint,
        true,
    );
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
    let argument_hint = popup.argument().and_then(|(name, argument)| {
        let action = match name {
            "model" => Message::ProvidersComposerUseExactModel,
            "effort" => Message::ProvidersComposerSetReasoning,
            "connect" => Message::ProvidersComposerChooseConnection,
            "lead" => Message::ProvidersComposerChoosePartner,
            _ => return None,
        };
        Some(paint.format(
            Template::ProvidersCommandArgument,
            &[
                ("action", paint.hint(action)),
                (
                    "argument",
                    &(clipped(argument, content.width.saturating_sub(24) as usize)),
                ),
            ],
        ))
    });
    let help = selected.map(|choice| {
        crate::command_help::key(choice.command)
            .map_or(choice.label.as_str(), |message| paint.hint(message))
    });
    let detail = reason
        .as_deref()
        .or(argument_hint.as_deref())
        .or(help)
        .unwrap_or_else(|| {
            paint.hint(Message::ProvidersComposerSearchActionsByNameOrDescriptionEsc)
        });
    let detail = if reason.is_some() {
        std::borrow::Cow::Owned(
            paint.format(Template::ProvidersCommandUnavailable, &[("reason", detail)]),
        )
    } else if argument_hint.is_none()
        && let Some(choice) = selected
        && matches!(
            choice.command,
            "send-role" | "lead" | "model-select" | "effort-select"
        )
    {
        std::borrow::Cow::Owned(paint.format(
            Template::ProvidersCommandIdentity,
            &[("description", detail), ("identity", &choice.label)],
        ))
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
        paint.hint(Message::ProvidersComposer0Matches).to_owned()
    } else {
        format!("{}-{end}/{count}", start + 1)
    };
    let query_width = content
        .width
        .saturating_sub(Span::raw(range.as_str()).width() as u16 + 1);
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
        let description = if matches!(
            choice.command,
            "send-role" | "lead" | "model-select" | "effort-select"
        ) {
            choice.label.as_str()
        } else {
            crate::command_help::key(choice.command)
                .map_or(choice.label.as_str(), |message| paint.hint(message))
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(if selected { ">" } else { " " }, style),
                Span::styled(if unavailable { "x " } else { "  " }, style),
                Span::styled(
                    format!("{command:command_width$} "),
                    if selected || unavailable {
                        style
                    } else {
                        paint.accent
                    },
                ),
                Span::styled(description, if selected { style } else { paint.muted }),
            ]))
            .style(style),
            row(content, visible as u16 + 1, 1),
        );
    }
    if count == 0 {
        text(
            frame,
            row(content, 1, rows as u16),
            paint.hint(Message::ProvidersComposerNoMatchingCommandTryAShorterSearch),
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
        paint.hint(Message::ProvidersComposerUpDownChooseEnterOpenEscBack),
        paint.muted,
    );
}
