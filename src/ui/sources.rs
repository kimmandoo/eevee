use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Clear, Paragraph, Wrap},
};

use super::{Paint, clipped, editor_rows, panel, row, selection_details, sheet, text};
use crate::{
    app::{
        App, ScrollSurface,
        sources::{EditKind, Mode},
    },
    config::Config,
};

pub(super) fn render(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    app.sources.optimizer_reviewed = false;
    app.sources.screenshot_reviewed = false;
    if app.sources.editor.is_some() {
        editor(frame, area, app, config, paint);
        return;
    }
    if app.sources.confirmation.is_some() {
        if app
            .sources
            .confirmation
            .is_some_and(|confirmation| confirmation.id == -1)
        {
            screenshot_confirmation(frame, area, app, config, paint);
            return;
        }
        confirmation(frame, area, app, config, paint);
        return;
    }
    if app.sources.mode == Mode::Screenshot {
        screenshot(frame, area, app, config, paint);
        return;
    }
    let area = sheet(area, 104, area.height);
    frame.render_widget(Clear, area);
    let block = panel(
        paint.hint(crate::messages::Message::DesksSourcesPanel),
        config,
        paint,
        true,
    );
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
        if state.mode == Mode::Optimizer {
            paint.hint(state.optimizer_notice())
        } else if let Some(message) = state.memory_notice {
            paint.hint(message)
        } else {
            &state.status
        },
        if state
            .optimizer_preview
            .as_ref()
            .is_some_and(|plan| plan.overflow)
        {
            paint.accent
        } else {
            paint.muted
        },
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
        content.height.saturating_sub(5 + list_rows),
    );
    let comparison = match state.mode {
        Mode::Comparison => Some(comparison_lines(state, paint)),
        Mode::Optimizer => Some(optimizer_lines(state, paint)),
        _ => None,
    };
    let errors = || {
        state
            .error
            .lines()
            .map(|line| Line::styled(line, paint.accent))
    };
    let lines: Vec<_> = if let Some(detail) = &comparison {
        let mut lines = Vec::with_capacity(detail.len());
        if state.comparison.current.is_none() {
            lines.extend(errors());
        }
        lines.extend(detail.iter().map(|line| Line::from(line.as_str())));
        if state.comparison.current.is_some() {
            lines.extend(errors());
        }
        lines
    } else {
        errors()
            .chain(state.detail.lines().map(Line::from))
            .collect()
    };
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
    app.set_scroll_viewport(ScrollSurface::Preview, max, details.height);
    text(
        frame,
        row(content, content.height.saturating_sub(2), 2),
        if app.sources.store_failed {
            paint.hint(crate::messages::Message::DesksSourcesEnterRetryRetainedLocalSaveEscDraft)
        } else if app.sources.busy() {
            paint.hint(
                crate::messages::Message::DesksSourcesSavingValidatingLocallyAttachmentNotCommittedPgup,
            )
        } else {
            match app.sources.mode {
                Mode::Files => {
                    paint.hint(crate::messages::Message::DesksSourcesEnterReviewAttachSSearchRRange)
                }
                Mode::Preview => paint
                    .hint(crate::messages::Message::DesksSourcesEnterAttachLocallyReturnRRangeTab),
                Mode::Basket => paint
                    .hint(crate::messages::Message::DesksSourcesDelRemoveReviewTabFilesMorePgup),
                Mode::Payload => {
                    paint.hint(crate::messages::Message::DesksSourcesMoreTabFilesPgupDnDetailsEsc)
                }
                Mode::Comparison => paint.hint(crate::messages::Message::ContextComparisonHint),
                Mode::Screenshot => {
                    "Enter attaches reviewed image locally · PgUp/Dn disclosure · Esc cancels"
                }
                Mode::Optimizer => paint.hint(
                    if app
                        .sources
                        .optimizer_preview
                        .as_ref()
                        .is_some_and(|plan| plan.overflow)
                    {
                        crate::messages::Message::ContextOptimizerOverflowHint
                    } else {
                        crate::messages::Message::ContextOptimizerHint
                    },
                ),
            }
        },
        paint.muted,
    );
    if app.sources.mode == Mode::Preview {
        text(
            frame,
            row(content, content.height.saturating_sub(3), 1),
            paint.hint(crate::messages::Message::HeldSourceHint),
            paint.muted,
        );
    } else if app.sources.mode == Mode::Basket {
        text(
            frame,
            row(content, content.height.saturating_sub(3), 1),
            paint.hint(crate::messages::Message::SourceMemoryHint),
            paint.muted,
        );
    } else if app.sources.mode != Mode::Comparison {
        text(
            frame,
            row(content, content.height.saturating_sub(3), 1),
            paint.hint(crate::messages::Message::ContextOptimizerSourceHint),
            paint.muted,
        );
    }
    app.sources.preview_reviewed = preview_reviewed;
    app.sources.optimizer_reviewed = app.sources.mode == Mode::Optimizer
        && app.sources.optimizer_preview.is_some()
        && frame.area().width >= 60
        && frame.area().height >= 18
        && details.height >= 4;
}

fn screenshot(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = sheet(area, 104, area.height);
    frame.render_widget(Clear, area);
    let block = panel(
        "Reviewed screenshot input · local capture",
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let capability = app
        .sources
        .screenshot_route
        .as_ref()
        .filter(|review| {
            review.matches(
                app.task_id,
                app.lead().id,
                app.planning.agent,
                &app.planning.selection,
            )
        })
        .map(|review| &review.capability);
    let supported = matches!(capability, Some(Ok(_)))
        && app.command_popup.is_none()
        && app.nickname_editor.is_none()
        && app.confirmation.is_none();
    let route = match capability {
        Some(Ok(label)) => label.as_str(),
        Some(Err(reason)) => reason.as_str(),
        None => {
            "Route not reviewed for this task/partner/model. Use /screenshot to capture and review again."
        }
    };
    let selected = if app.sources.screenshot.is_some() {
        "selected"
    } else {
        "not selected"
    };
    text(
        frame,
        row(content, 0, 1),
        &format!(
            "Task #{} · image {selected} · no provider request",
            app.task_id
        ),
        paint.accent,
    );
    let disclosure = format!(
        "{}\nSelected route: {}\n{}\n{}",
        app.sources.detail, route, app.sources.status, app.sources.error
    );
    let details = row(content, 1, content.height.saturating_sub(4));
    let paragraph = Paragraph::new(disclosure.as_str())
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    let rows = paragraph.line_count(details.width.max(1));
    let offset = app.scroll_for(ScrollSurface::Preview);
    let max = selection_details(frame, details, paragraph, offset);
    app.set_scroll_viewport(ScrollSurface::Preview, max, details.height);
    app.sources.expose_screenshot(
        disclosure,
        details.width,
        details.height,
        usize::from(offset.min(max)),
        rows,
        supported && !app.sources.busy(),
    );
    text(
        frame,
        row(content, content.height.saturating_sub(3), 3),
        if app.sources.screenshot_reviewed {
            "Enter: attach exact image locally · Esc: cancel\n/remove via /screenshot-remove · /screenshot-purge deletes retained bytes"
        } else {
            "PgUp/Dn: read complete disclosure · Enter blocked until reviewed\nEsc: cancel without attachment · /screenshot-show inspects retained metadata"
        },
        paint.muted,
    );
}

fn screenshot_confirmation(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
) {
    let Some(confirmation) = app.sources.confirmation else {
        return;
    };
    let area = sheet(area, 90, 18);
    frame.render_widget(Clear, area);
    let block = panel("Review screenshot removal", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let target = app.sources.screenshot_removal.as_ref();
    let available = target.is_some();
    let body = format!(
        "Task #{} · partner #{}\nRoot {:?}\nImage {:?} · {} compressed bytes\n{} · {} × {}\nSHA-256 {}\n{}\nNo provider request; historical receipt counts remain.",
        app.task_id,
        target.map_or(0, |capture| capture.partner),
        target.map(|capture| &capture.root),
        target.map(|capture| &capture.path),
        target.map_or(0, |capture| capture.bytes),
        target.map_or("unavailable", |capture| capture.format.label()),
        target.map_or(0, |capture| capture.width),
        target.map_or(0, |capture| capture.height),
        target
            .map(|capture| capture.sha256_hex())
            .unwrap_or_default(),
        if confirmation.purge {
            "Permanently delete retained compressed bytes; no undo."
        } else {
            "Deselect only. Exact compressed bytes remain retained until purge/task deletion."
        }
    );
    let details = row(content, 0, content.height.saturating_sub(2));
    let paragraph = Paragraph::new(body.as_str())
        .wrap(Wrap { trim: false })
        .style(paint.surface);
    let rows = paragraph.line_count(details.width.max(1));
    let offset = app.scroll_for(ScrollSurface::Preview);
    let maximum = selection_details(frame, details, paragraph, offset);
    app.set_scroll_viewport(ScrollSurface::Preview, maximum, details.height);
    let supported = available
        && app.command_popup.is_none()
        && app.nickname_editor.is_none()
        && app.confirmation.is_none();
    app.sources.expose_screenshot(
        body,
        details.width,
        details.height,
        usize::from(offset.min(maximum)),
        rows,
        supported,
    );
    let reviewed = app.sources.screenshot_reviewed;
    if let Some(review) = app.sources.confirmation.as_mut() {
        review.reviewed = reviewed;
        if !reviewed {
            review.destructive = false;
        }
    }
    text(
        frame,
        row(content, content.height.saturating_sub(2), 2),
        if !reviewed {
            "PgUp/Dn: read complete exact scope · Esc cancels"
        } else if confirmation.destructive {
            "[ Keep / cancel ]     [ > Confirm removal ]"
        } else {
            "[ > Keep / cancel ]     [ Confirm removal ]"
        },
        paint.muted,
    );
}

fn comparison_lines(state: &crate::app::sources::State, paint: &Paint) -> Vec<String> {
    use crate::messages::{Message, Template};
    let comparison = &state.comparison;
    let mut lines = vec![paint.hint(Message::ContextComparisonTitle).into()];
    if let Some(turn) = &comparison.turn {
        lines.push(paint.format(
            Template::ContextComparisonScope,
            &[
                ("task", &turn.task_id.to_string()),
                ("partner", &turn.partner.to_string()),
                ("provider", turn.agent.label()),
                ("operation", &format!("{:?}", turn.operation)),
                ("turn", &turn.request.to_string()),
            ],
        ));
    }
    if let Some(delta) = comparison.delta() {
        lines.push(
            paint.format(
                Template::ContextComparisonCounts,
                &[
                    (
                        "previous",
                        &comparison
                            .previous
                            .as_ref()
                            .expect("delta baseline")
                            .captures
                            .to_string(),
                    ),
                    (
                        "current",
                        &comparison
                            .current
                            .as_ref()
                            .expect("delta current")
                            .captures
                            .to_string(),
                    ),
                ],
            ),
        );
        let counts = |template, change: crate::workspace::receipt::CountChange| {
            paint.format(
                template,
                &[
                    ("previous", &change.previous.to_string()),
                    ("current", &change.current.to_string()),
                    ("delta", &format!("{:+}", change.delta())),
                ],
            )
        };
        // The full-payload result is first, including at minimum terminal size.
        lines.push(counts(Template::ContextComparisonTotal, delta.bytes));
        lines.push(counts(Template::ContextComparisonEstimate, delta.tokens));
        for section in delta.sections {
            let label = match section.name {
                "Instructions" => Message::ContextComparisonInstructions,
                "Task facts" => Message::ContextComparisonFacts,
                "Attachments" => Message::ContextComparisonAttachments,
                "Handoff" => Message::ContextComparisonHandoff,
                "Approved memories" => Message::ContextComparisonMemories,
                _ => Message::ContextComparisonOther,
            };
            lines.push(paint.format(
                Template::ContextComparisonSection,
                &[
                    ("section", paint.hint(label)),
                    ("previous", &section.bytes.previous.to_string()),
                    ("current", &section.bytes.current.to_string()),
                    ("delta", &format!("{:+}", section.bytes.delta())),
                ],
            ));
            lines.push(counts(Template::ContextComparisonEstimate, section.tokens));
        }
        lines.push(counts(
            Template::ContextComparisonEnvelope,
            delta.envelope_bytes,
        ));
        lines.push(paint.format(
            Template::ContextComparisonDedup,
            &[
                ("previous", &delta.deduplicated.previous.to_string()),
                ("current", &delta.deduplicated.current.to_string()),
            ],
        ));
    } else {
        let status = if state.comparison_refresh {
            Message::ContextComparisonWaiting
        } else if comparison.previous.is_some() {
            Message::ContextComparisonBeforeOnly
        } else if comparison.current.is_some() && state.comparison_scope.is_none() {
            Message::ContextComparisonUnavailable
        } else if comparison.current.is_some() {
            Message::ContextComparisonBaseline
        } else {
            Message::ContextComparisonEmpty
        };
        lines.push(paint.hint(status).into());
        if let Some(current) = &comparison.current {
            lines.push(paint.format(
                Template::ContextComparisonTotal,
                &[
                    ("previous", &current.receipt.bytes.to_string()),
                    ("current", &current.receipt.bytes.to_string()),
                    ("delta", "0"),
                ],
            ));
        }
    }
    lines.push(paint.hint(Message::ContextComparisonLocal).into());
    lines.push(paint.hint(Message::ContextComparisonEstimator).into());
    lines.push(paint.hint(Message::ContextComparisonBoundary).into());
    lines
}
fn optimizer_lines(state: &crate::app::sources::State, paint: &Paint) -> Vec<String> {
    use crate::messages::{Message, Template};
    let Some(plan) = &state.optimizer_preview else {
        return vec![paint.hint(Message::ContextOptimizerTitle).into()];
    };
    let mut lines = Vec::with_capacity(8 + plan.included.len() + plan.omissions.len());
    if plan.overflow {
        lines.push(paint.hint(Message::ContextOptimizerOverflow).into());
    }
    lines.extend([
        paint.hint(Message::ContextOptimizerTitle).into(),
        paint.format(
            Template::ContextOptimizerSummary,
            &[
                ("kind", &plan.kind),
                ("tokens", &plan.tokens.to_string()),
                ("budget", &plan.budget.to_string()),
                ("baseline", &plan.baseline_tokens.to_string()),
            ],
        ),
        paint.format(
            Template::ContextOptimizerContributions,
            &[
                ("packet", &plan.packet_tokens.to_string()),
                ("notes", &plan.mandatory_note_tokens.to_string()),
            ],
        ),
        paint.format(
            Template::ContextOptimizerMeasurements,
            &[
                ("bytes", &plan.packet.len().to_string()),
                ("micros", &plan.processing_micros.to_string()),
                ("hits", &plan.cache_hits.to_string()),
            ],
        ),
        paint.format(
            Template::ContextOptimizerTokenizer,
            &[("tokenizer", &plan.tokenizer)],
        ),
    ]);
    lines.push(paint.hint(Message::ContextOptimizerApplyNotice).into());
    for item in &plan.included {
        lines.push(format!(
            "{} {}:{}-{} — {}",
            paint.hint(if item.mandatory {
                Message::ContextOptimizerPin
            } else {
                Message::ContextOptimizerIncluded
            }),
            item.path,
            item.start_line,
            item.end_line,
            item.reason
        ));
    }
    lines.extend(
        plan.omissions
            .iter()
            .map(|reason| format!("{}: {reason}", paint.hint(Message::ContextOptimizerOmitted))),
    );
    lines.push(plan.packet.clone());
    lines
}

fn editor(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let Some(editor) = &app.sources.editor else {
        return;
    };
    let area = sheet(area, 82, 12);
    frame.render_widget(Clear, area);
    let block = panel(
        match editor.kind {
            EditKind::Search => {
                paint.hint(crate::messages::Message::DesksSourcesFilenameFilterLiteral)
            }
            EditKind::Range => {
                paint.hint(crate::messages::Message::DesksSourcesSourceRangeInclusive)
            }
            EditKind::Budget => paint.hint(crate::messages::Message::ContextOptimizerBudgetTitle),
            EditKind::ScreenshotPath => "Explicit screenshot path · PNG/JPEG",
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
            EditKind::Search => paint.hint(
                crate::messages::Message::DesksSourcesCaseSensitiveFilenameSubstringExcludedPathsStay,
            ),
            EditKind::Range => {
                paint.hint(crate::messages::Message::DesksSourcesOneBasedStartEndEGExact)
            }
            EditKind::Budget => paint.hint(crate::messages::Message::ContextOptimizerBudgetHint),
            EditKind::ScreenshotPath => "Literal project-relative file only; no URL, clipboard, OCR or automatic capture.",
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
    let area = sheet(area, 76, if app.verbose { 20 } else { 14 });
    frame.render_widget(Clear, area);
    let block = panel(
        paint.hint(crate::messages::Message::DesksSourcesReviewLocalExcerptRemoval),
        config,
        paint,
        true,
    );
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
        .unwrap_or_else(|| {
            paint
                .hint(crate::messages::Message::DesksSourcesUnavailableReloadBeforeConfirming)
                .into()
        });
    let details = row(content, 0, content.height.saturating_sub(4));
    let summary = paint.format(
        crate::messages::Template::DesksSourcesRemovalSummary,
        &[
            (
                "action",
                (if confirmation.purge {
                    paint.hint(crate::messages::Message::DesksSourcesPurge)
                } else {
                    paint.hint(crate::messages::Message::DesksSourcesRemove)
                }),
            ),
            ("id", &(confirmation.id).to_string()),
            (
                "capture",
                &(super::clipped(&capture, details.width.saturating_sub(9) as usize)),
            ),
            (
                "effect",
                (if confirmation.purge {
                    paint.hint(
                        crate::messages::Message::DesksSourcesClearLocalBytesKeepUnavailableIdentity,
                    )
                } else {
                    paint
                        .hint(crate::messages::Message::DesksSourcesRemoveLocalReferenceAndCapturedText)
                }),
            ),
        ],
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
            Paragraph::new(paint.format(
                crate::messages::Template::DesksSourcesExactCapture,
                &[("capture", &(capture))],
            ))
            .style(paint.muted)
            .wrap(Wrap { trim: false }),
            app.scroll_for(ScrollSurface::Preview),
        )
    } else {
        0
    };
    app.set_scroll_viewport(ScrollSurface::Preview, max, remaining);
    text(
        frame,
        row(content, content.height.saturating_sub(3), 1),
        paint.hint(crate::messages::Message::DesksSourcesNStayKeepTheExcerpt),
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
            paint.hint(crate::messages::Message::DesksSourcesYPurgeSelectedLocalCapturedBytes)
        } else {
            paint.hint(crate::messages::Message::DesksSourcesYRemoveSelectedBasketReference)
        },
        if confirmation.destructive {
            paint.selected
        } else {
            paint.surface
        },
    );
}
