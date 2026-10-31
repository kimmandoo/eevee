//! Real local execution review and evidence inspector. No action is dispatched by rendering.
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};

use super::{Paint, panel, row, selection_details, text};
use crate::{
    app::execution::{self, Confirmation, State, View},
    config::Config,
    domain::FORMS,
    messages::{Message, Template},
};

pub(super) fn render_small_confirmation(
    frame: &mut Frame,
    area: Rect,
    state: &mut State,
    config: &Config,
) {
    state.confirmation_visible = false;
    let paint = Paint::new(config, &FORMS[2], 2);
    frame.render_widget(Clear, area);
    if matches!(state.confirmation, Some(Confirmation::Approval)) {
        text(
            frame,
            area,
            paint.hint(Message::ProvidersExecutionResizeTo60x18ToReviewApprovalNo),
            paint.surface,
        );
        return;
    }
    let body = row(area, 0, area.height.saturating_sub(2));
    state.set_viewport(body.height.saturating_sub(3) as usize);
    confirmation(frame, body, state, &paint, None, false);
    text(
        frame,
        row(area, area.height.saturating_sub(2), 2),
        paint.hint(Message::ProvidersExecutionLeftStayRightActionEnterPgupDn),
        paint.muted,
    );
}

pub(super) fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut State,
    config: &Config,
    queue: Option<&str>,
    editing: Option<&str>,
    allow_all: bool,
) {
    if !state.visible() {
        return;
    }
    let height =
        if matches!(state.confirmation, Some(Confirmation::Approval)) && !state.approval_full {
            area.height.min(24)
        } else {
            area.height
        };
    let area = super::sheet(area, 104, height);
    let paint = Paint::new(config, &FORMS[2], 2);
    frame.render_widget(Clear, area);
    let block = panel(
        queue.unwrap_or(paint.hint(Message::ProvidersExecutionJolteonLocalCommandEvidence)),
        config,
        &paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    if state.confirmation.is_some() {
        confirmation(frame, content, state, &paint, editing, allow_all);
        return;
    }
    if content.height < 4 || content.width < 12 {
        text(
            frame,
            content,
            paint.hint(Message::ProvidersExecutionExecutionStateRetainedResizeEscStaysCtrl),
            paint.surface,
        );
        return;
    }
    let status = state.status();
    text(
        frame,
        row(content, 0, 1),
        &execution::safe_text(&status),
        paint.accent,
    );
    let banner = if let Some(error) = &state.error {
        paint.format(
            Template::ProvidersExecutionErrorRunDetailsJournalRetryStorageOnly,
            &[("layer", (error.layer))],
        )
    } else if state.searching {
        paint.format(
            Template::ProvidersExecutionSearchCurrentWindowPageEnterFindsEsc,
            &[("query", &(state.search))],
        )
    } else {
        super::brief(&state.notice).into_owned()
    };
    text(
        frame,
        row(content, 1, 2),
        &execution::safe_text(&banner),
        if state.error.is_some() {
            paint.output_style(crate::output_style::Kind::Error, paint.surface)
        } else {
            paint.muted
        },
    );
    let footer_rows = if content.height >= 12 { 3 } else { 2 };
    let body_height = content.height.saturating_sub(3 + footer_rows);
    let body = row(content, 3, body_height);
    state.set_viewport(body.height as usize);
    match state.view {
        View::History => history(frame, body, state, &paint),
        View::Details => details(frame, body, state, &paint),
        View::Locations => locations(frame, body, state, &paint),
        View::Output => output(frame, body, state, &paint),
    }
    let footer = row(
        content,
        content.height.saturating_sub(footer_rows),
        footer_rows,
    );
    let hints = if state.view == View::History {
        paint.hint(Message::ProvidersExecutionUpDownSelectEnterInspectRetainedOutput)
    } else if state.view == View::Locations {
        paint.hint(Message::ProvidersExecutionUpDownSelectEnterPinRangeReview)
    } else if state.view == View::Details {
        paint.hint(Message::ProvidersExecutionUpDownPgupPgdnHomeEndInspect)
    } else {
        paint.hint(Message::ProvidersExecutionChoicesCommandEditExecuteLocationsRunDetailsArrows)
    };
    text(frame, footer, hints, paint.muted);
}

fn locations(frame: &mut Frame, area: Rect, state: &State, paint: &Paint) {
    if state.locations.is_empty() {
        text(
            frame,
            area,
            paint.hint(Message::ProvidersExecutionNoParseableLocationsInRetainedOutputNo),
            paint.muted,
        );
        return;
    }
    let start = state
        .location_index
        .saturating_sub(area.height.saturating_sub(1) as usize);
    let rows: Vec<Line<'_>> = state
        .locations
        .iter()
        .enumerate()
        .skip(start)
        .take(area.height as usize)
        .map(|(index, location)| {
            Line::styled(
                format!(
                    "{} {}",
                    if index == state.location_index {
                        ">"
                    } else {
                        " "
                    },
                    location
                ),
                if index == state.location_index {
                    paint.selected
                } else {
                    paint.surface
                },
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(rows), area);
}

fn confirmation(
    frame: &mut Frame,
    area: Rect,
    state: &mut State,
    paint: &Paint,
    editing: Option<&str>,
    allow_all: bool,
) {
    let Some(confirm) = state.confirmation.as_ref() else {
        return;
    };
    if matches!(confirm, Confirmation::Approval)
        && state.prepared.as_ref().is_some_and(|p| {
            matches!(
                p.spec,
                crate::workspace::runner::CommandSpec::SelectedCommit { .. }
            )
        })
    {
        selected_commit_confirmation(frame, area, state, paint);
        return;
    }
    let (title, affirmative, description) = match confirm {
        Confirmation::Approval => (
            paint.hint(Message::ProvidersExecutionReviewOneShotExecutionPermission),
            paint.hint(Message::ProvidersExecutionApproveThisExactCommand),
            state
                .prepared
                .as_ref()
                .map(|p| {
                    if state.approval_full {
                        format!(
                            "{}\n\n{}",
                            paint.hint(Message::ProvidersExecutionBriefDetailsHint),
                            execution::approval_details(p, state.capture_capacity)
                        )
                    } else {
                        approval_brief(p, paint)
                    }
                })
                .unwrap_or_else(|| {
                    paint
                        .hint(Message::ProvidersExecutionThisRequestWasInvalidatedReturnAndPrepare)
                        .into()
                }),
        ),
        Confirmation::Cancel => (
            paint.hint(Message::ProvidersExecutionCancelOwnedWorkDenyUnstartedApproval),
            paint.hint(Message::ProvidersExecutionCancelAndWait),
            paint
                .hint(Message::ProvidersExecutionCancellationWaitsForOwnedTreeExitWith)
                .to_owned()
                + &state.details(),
        ),
        Confirmation::Purge(run) => (
            paint.hint(Message::ProvidersExecutionPurgeRetainedLocalOutput),
            paint.hint(Message::ProvidersExecutionPurgeThisRunSOutput),
            paint.format(
                Template::ProvidersExecutionRunOnlyThisRunSOwnedLocal,
                &[
                    ("run", &(execution::safe_text(run))),
                    ("details", &(state.details())),
                ],
            ),
        ),
        Confirmation::Navigate(action) => (
            paint.hint(Message::ProvidersExecutionStayWithOwnedWorkOrCancelAnd),
            if *action == execution::Action::Quit {
                paint.hint(Message::ProvidersExecutionCancelAndExitAfterSaving)
            } else {
                paint.hint(Message::ProvidersExecutionCancelAndWaitThenNavigate)
            },
            paint.format(
                Template::ProvidersExecutionRequestedActionStayKeepsThisTaskAnd,
                &[
                    ("action", &format!("{action:?}")),
                    ("details", &(state.details())),
                ],
            ),
        ),
        Confirmation::UnknownExit(action) => (
            paint.hint(Message::ProvidersExecutionOutcomeUnknownExplicitInformedExitRequired),
            paint.hint(Message::ProvidersExecutionExitKnowingEffectsAreUnknown),
            paint.format(
                Template::ProvidersExecutionRequestedActionOwnedTreeOutputClosureCould,
                &[
                    ("action", &format!("{action:?}")),
                    ("details", &(state.details())),
                ],
            ),
        ),
        Confirmation::UnsavedJournalExit(action) => (
            paint.hint(Message::ProvidersExecutionProcessEndedButJournalPersistenceFailed),
            paint.hint(Message::ProvidersExecutionExitWithoutSavedFinalJournal),
            paint.format(
                Template::ProvidersExecutionRequestedActionTheFinalExecutionRecordIs,
                &[
                    ("action", &format!("{action:?}")),
                    ("details", &(state.details())),
                ],
            ),
        ),
    };
    text(frame, row(area, 0, 1), title, paint.accent);
    let choices = Line::from(vec![
        Span::styled(
            paint.hint(Message::ProvidersExecutionStayReturn),
            if state.affirmative {
                paint.surface
            } else {
                paint.selected
            },
        ),
        Span::raw("  "),
        Span::styled(
            format!(" {affirmative} "),
            if state.affirmative {
                paint.selected
            } else {
                paint.surface
            },
        ),
    ]);
    // Keep the decision below the immutable scope, with the same total row
    // budget as the former top-of-dialog buttons at every supported width.
    let choice_rows = if choices.width() <= area.width as usize {
        1
    } else {
        2
    };
    let footer_rows = choice_rows + 2;
    let summary_start = 1;
    frame.render_widget(
        Paragraph::new(choices).wrap(Wrap { trim: false }),
        row(area, area.height.saturating_sub(footer_rows), choice_rows),
    );
    let mut summary = match confirm {
        Confirmation::Approval => state.prepared.as_ref().map_or_else(
            || {
                vec![
                    paint
                        .hint(Message::ProvidersExecutionBlockedPreparedCommandNoLongerExists)
                        .into(),
                ]
            },
            |prepared| {
                vec![
                    paint.format(
                        Template::ProvidersExecutionWantsToRun,
                        &[
                            ("owner", &(execution::requester(prepared.partner.as_ref()))),
                            ("command", &(execution::approval_command_summary(prepared))),
                        ],
                    ),
                    paint.format(
                        Template::ProvidersExecutionDirectory,
                        &[("path", &prepared.cwd.display().to_string())],
                    ),
                    paint.format(
                        Template::ProvidersExecutionExecutable,
                        &[("path", &prepared.executable.path.display().to_string())],
                    ),
                    paint
                        .hint(Message::ProvidersExecutionOneLocalCommandMayWriteFilesAnd)
                        .into(),
                    paint
                        .hint(Message::ProvidersExecutionNotSandboxedEffectsAreNotRolledBack)
                        .into(),
                    paint
                        .hint(Message::ProvidersExecutionExactCommandConfigAndOutputLimitsBelow)
                        .into(),
                ]
            },
        ),
        Confirmation::Cancel | Confirmation::Navigate(_) => vec![
            paint.format(
                Template::ProvidersExecutionStopSCommandDenyItOrCancel,
                &[("owner", &(execution::requester(state.owner.as_ref())))],
            ),
            paint
                .hint(Message::ProvidersExecutionWaitForBoundedTeardownAndJournalAcknowledgement)
                .into(),
            paint
                .hint(Message::ProvidersExecutionCancellationIsNotRollbackEditsNetworkMay)
                .into(),
            paint
                .hint(Message::ProvidersExecutionPartialOutputAndTaskDraftsAreRetained)
                .into(),
            paint
                .hint(Message::ProvidersExecutionUnknownOutcomeNeedsASeparateExplicitExit)
                .into(),
        ],
        Confirmation::Purge(run) => vec![
            format!("Run: {}", execution::safe_text(run)),
            paint
                .hint(Message::ProvidersExecutionDeletesOnlyThisRunSRetainedLocal)
                .into(),
            paint
                .hint(Message::ProvidersExecutionMetadataStaysDeletedEvidenceCannotVerifyResults)
                .into(),
            paint
                .hint(Message::ProvidersExecutionNotSecureErasureRepositoryProviderDataUnchanged)
                .into(),
        ],
        Confirmation::UnknownExit(_) => vec![
            paint
                .hint(Message::ProvidersExecutionUnknownProcessTreeOutputClosureNotEstablished)
                .into(),
            paint
                .hint(Message::ProvidersExecutionEffectsMayRemainCancellationWasNotRollback)
                .into(),
            paint
                .hint(Message::ProvidersExecutionQuitAcknowledgesUnknownEffectsNotSuccess)
                .into(),
            paint
                .hint(Message::ProvidersExecutionTaskSwitchingCannotEstablishTermination)
                .into(),
        ],
        Confirmation::UnsavedJournalExit(_) => vec![
            paint
                .hint(Message::ProvidersExecutionFinalExecutionJournalIsOnlyInMemory)
                .into(),
            paint
                .hint(Message::ProvidersExecutionExitLosesUnsavedFinalMetadataDraftsStay)
                .into(),
            paint
                .hint(Message::ProvidersExecutionRestartSeesInterruptionNoCommandIsReplayed)
                .into(),
            paint
                .hint(Message::ProvidersExecutionStayAndJournalRetryRetriesStorageOnly)
                .into(),
        ],
    };
    if matches!(confirm, Confirmation::Approval) && state.prepared.is_some() {
        // The details navigation is already disclosed in the footer. Keep
        // the consent budget at 60x18 without painting over the permission
        // title or any effect/identity disclosure: the trailing hint/editing
        // line is optional, and spare rows let the command wrap instead of
        // being clipped to one row.
        let last = summary.last_mut().expect("approval summary");
        if let Some(editing) = editing {
            last.clear();
            last.push_str(editing);
        }
        let available =
            (area.height as usize).saturating_sub((summary_start + footer_rows) as usize);
        let required = summary.len() - 1;
        let spare = available.saturating_sub(required);
        let command =
            crate::workspace::privacy::redact(&summary[0].replace(['\n', '\r', '\t'], " "))
                .into_owned();
        let needed = wrapped_rows(&command, area.width as usize, usize::MAX).len();
        let rows = needed.clamp(1, spare.clamp(1, APPROVAL_COMMAND_ROWS));
        if rows + required > available {
            summary.pop();
        }
        summary.splice(0..1, wrapped_rows(&command, area.width as usize, rows));
    } else if let Some(editing) = editing {
        summary.push(editing.to_owned());
    }
    let summary_height = summary.len() as u16;
    state.confirmation_visible = super::planning::render_summary(
        frame,
        row(
            area,
            summary_start,
            summary_height.min(area.height.saturating_sub(summary_start + footer_rows)),
        ),
        &summary,
        paint,
    );
    let body = row(
        area,
        summary_start + summary_height,
        area.height
            .saturating_sub(summary_start + footer_rows + summary_height),
    );
    let paragraph = Paragraph::new(description)
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    state.detail_max = selection_details(frame, body, paragraph, state.detail_scroll);
    state.detail_scroll = state.detail_scroll.min(state.detail_max);
    text(
        frame,
        row(area, area.height.saturating_sub(2), 2),
        paint.hint(
            if state.auto_presented && matches!(confirm, Confirmation::Approval) {
                Message::ProvidersExecutionAutoReviewActions
            } else if allow_all && matches!(confirm, Confirmation::Approval) {
                Message::ProvidersExecutionAllowAllReviewActions
            } else {
                Message::ProvidersExecutionYConfirmNStayTabChooseEnter
            },
        ),
        paint.muted,
    );
}

/// GP07 reviews every exact wrapped row through bounded contiguous exposure.
/// A skip, hidden surface, geometry change or replacement cannot provide consent.
fn selected_commit_confirmation(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    state.confirmation_visible = false;
    let Some(p) = state.prepared.as_ref() else {
        return;
    };
    let body = execution::safe_text(&execution::approval_details(p, state.capture_capacity));
    let lines: Vec<String> = body
        .lines()
        .flat_map(|line| wrapped_rows(line, area.width as usize, usize::MAX))
        .collect();
    text(
        frame,
        row(area, 0, 1),
        "Review one-shot selected LOCAL commit — not verification",
        paint.accent,
    );
    let choices = if state.affirmative {
        " Stay    [Approve exact selected commit] "
    } else {
        " [Stay]    Approve exact selected commit "
    };
    let confirm_hint = "Left Stay · Right Approve · Enter confirms ONCE";
    super::choices(frame, row(area, 1, 1), choices, paint.surface, paint);
    let available = area.height.saturating_sub(5);
    state.set_viewport(available as usize);
    state.expose_selected_review(area.width, available, lines.len(), false);
    state.detail_max = lines
        .len()
        .saturating_sub(available as usize)
        .min(u16::MAX as usize) as u16;
    state.detail_scroll = state.detail_scroll.min(state.detail_max);
    let start = state.detail_scroll as usize;
    let end = (start + available as usize).min(lines.len());
    let rendered = available > 0
        // The 60-column terminal has a 54-column padded review body.
        // Require the actual decision controls to fit, not a guessed outer width.
        && usize::from(area.width)
            >= Span::raw(choices).width().max(Span::raw(confirm_hint).width())
        && super::planning::render_summary(
            frame,
            row(area, 2, available),
            &lines[start..end],
            paint,
        );
    let reviewed = state.expose_selected_review(area.width, available, lines.len(), rendered);
    text(
        frame,
        row(area, area.height.saturating_sub(3), 1),
        &format!(
            "Reviewed {reviewed}/{} rows · PgUp/PgDn inspect",
            lines.len()
        ),
        paint.muted,
    );
    text(
        frame,
        row(area, area.height.saturating_sub(2), 2),
        if state.auto_presented {
            "Enter engages; Esc stays"
        } else if !state.confirmation_visible {
            "Review every row first; Esc stays. No standing grant."
        } else {
            confirm_hint
        },
        paint.accent,
    );
}

/// Long commands wrap up to this many summary rows; exact argv is below.
const APPROVAL_COMMAND_ROWS: usize = 4;

/// Split by display width at spaces when possible; the final permitted row
/// is clipped with `…`.
fn wrapped_rows(value: &str, columns: usize, rows: usize) -> Vec<String> {
    let columns = columns.max(2);
    let width = |text: &str| Span::raw(text).width();
    let mut lines = vec![String::new()];
    for grapheme in unicode_segmentation::UnicodeSegmentation::graphemes(value, true) {
        let last = lines.last().expect("row");
        if width(last) + width(grapheme) > columns {
            if lines.len() == rows {
                let last = lines.last_mut().expect("row");
                *last = super::clipped(&format!("{last}{grapheme}…"), columns).into_owned();
                return lines;
            }
            // Carry an unfinished word to the next row instead of splitting it.
            let carried = match last.rfind(' ') {
                Some(space) if grapheme != " " && space > 0 => {
                    let tail = last[space + 1..].to_owned();
                    let last = lines.last_mut().expect("row");
                    last.truncate(space + 1);
                    tail
                }
                _ => String::new(),
            };
            lines.push(carried);
            if grapheme == " " {
                continue;
            }
        }
        lines.last_mut().expect("row").push_str(grapheme);
    }
    lines
}

/// What the approval actually runs plus decision-relevant limits. Identity
/// hashes, IDs, timestamps, quotas and enforcement prose stay behind `d`.
fn approval_brief(p: &crate::workspace::runner::Prepared, paint: &Paint) -> String {
    let mut text = execution::exact_command_lines(&p.spec);
    if execution::source_coverage_partial(p) {
        text.push_str(paint.hint(Message::ProvidersExecutionSourceCoveragePartial));
        text.push('\n');
    }
    let remaining = p
        .expires_at_ms
        .saturating_sub(crate::workspace::tool_plans::now_ms());
    text.push_str(&paint.format(
        Template::ProvidersExecutionApprovalExpiresIn,
        &[("minutes", &remaining.div_ceil(60_000).to_string())],
    ));
    text.push_str("\n\n");
    text.push_str(paint.hint(Message::ProvidersExecutionFullDetailsHint));
    text
}

fn details(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    let paragraph = Paragraph::new(state.details())
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    state.detail_max = selection_details(frame, area, paragraph, state.detail_scroll);
    state.detail_scroll = state.detail_scroll.min(state.detail_max);
}

fn history(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    if state.history.is_empty() {
        text(
            frame,
            area,
            paint.hint(Message::ProvidersExecutionNoRetainedExecutionsForThisTaskCommand),
            paint.muted,
        );
        return;
    }
    let rows = (area.height as usize / 3).max(1);
    let start = state.history_index.saturating_sub(rows.saturating_sub(1));
    for (slot, (index, record)) in state
        .history
        .iter()
        .enumerate()
        .skip(start)
        .take(rows)
        .enumerate()
    {
        let line = format!(
            "{} {:?} · task {} · ended {:?}\n{} · {}\n{}",
            if index == state.history_index {
                ">"
            } else {
                " "
            },
            record.state,
            record.prepared.task_id,
            record.ended_at_ms,
            record.prepared.run_id,
            crate::store::Attribution::label(record.prepared.partner.as_ref()),
            execution::evidence_label(record, &state.latest)
        );
        text(
            frame,
            row(area, (slot * 3) as u16, 3),
            &execution::safe_text(&line),
            if index == state.history_index {
                paint.selected
            } else {
                paint.surface
            },
        );
    }
}

fn output(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    let label = if state.showing_retained {
        if let Some(page) = &state.retained_page {
            paint.format(
                Template::ProvidersExecutionRetainedBytesPages,
                &[
                    ("start", &(page.offset).to_string()),
                    ("end", &(page.offset + page.text.len() as u64).to_string()),
                    ("total", &(page.total_bytes).to_string()),
                    (
                        "unavailable",
                        (if page.unavailable {
                            paint.hint(Message::ProvidersExecutionUnavailable)
                        } else {
                            ""
                        }),
                    ),
                    (
                        "truncated",
                        (if page.truncated {
                            paint.hint(Message::ProvidersExecutionTruncated)
                        } else {
                            ""
                        }),
                    ),
                ],
            )
        } else {
            paint
                .hint(Message::ProvidersExecutionLoadingRetainedOutput)
                .into()
        }
    } else {
        paint.format(
            Template::ProvidersExecutionNewLinesLiveLinesEvictedColumn,
            &[
                (
                    "status",
                    (if state.follow {
                        paint.hint(Message::ProvidersExecutionFollowOn)
                    } else {
                        paint.hint(Message::ProvidersExecutionFollowPaused)
                    }),
                ),
                ("new", &(state.new_output).to_string()),
                ("evicted", &(state.live.evicted).to_string()),
                ("column", &(state.horizontal).to_string()),
            ],
        )
    };
    let warning = if state.showing_retained {
        ""
    } else {
        match state.active.as_ref() {
            Some(record) if record.output_unavailable => {
                paint.hint(Message::ProvidersExecutionOutputUnavailableInspectDetails)
            }
            Some(record) if record.truncated => {
                paint.hint(Message::ProvidersExecutionCaptureTruncatedExcessDrainedDiscarded)
            }
            Some(record) if record.dropped_live_bytes > 0 => {
                paint.hint(Message::ProvidersExecutionLiveQueueGapInspectRetainedOutput)
            }
            _ => "",
        }
    };
    text(
        frame,
        row(area, 0, if warning.is_empty() { 2 } else { 1 }),
        &label,
        paint.muted,
    );
    if !warning.is_empty() {
        text(frame, row(area, 1, 1), warning, paint.accent);
    }
    let body = row(area, 2, area.height.saturating_sub(2));
    state.set_viewport(body.height as usize);
    if state.showing_retained
        && state
            .retained_page
            .as_ref()
            .is_some_and(|page| page.unavailable)
    {
        text(
            frame,
            body,
            paint.hint(Message::ProvidersExecutionOutputUnavailablePurgedMissingOrReplacedEmpty),
            paint.accent,
        );
        return;
    }
    if state.window().lines.is_empty() {
        text(
            frame,
            body,
            paint.hint(Message::ProvidersExecutionNoOutputYetCommandEditWritesA),
            paint.muted,
        );
        return;
    }
    let lines: Vec<Line<'_>> = state
        .window()
        .lines
        .iter()
        .filter(|line| line.id >= state.anchor)
        .take(body.height as usize)
        .map(|line| {
            let style = if !state.search.is_empty() && line.text.contains(&state.search) {
                paint.selected
            } else {
                crate::output_style::diagnostic(&line.text).map_or(paint.surface, |kind| {
                    paint.output_style(kind, paint.surface)
                })
            };
            Line::from(Span::styled(line.text.as_str(), style))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .style(paint.surface)
            .scroll((0, state.horizontal)),
        body,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        Terminal,
        backend::TestBackend,
        style::{Modifier, Style},
    };

    fn paint() -> Paint {
        Paint {
            color: crate::config::ColorMode::Mono,
            ascii: true,
            base: Style::default(),
            surface: Style::default(),
            muted: Style::default(),
            border: Style::default(),
            accent: Style::default().add_modifier(Modifier::BOLD),
            selected: Style::default().add_modifier(Modifier::REVERSED),
        }
    }
    fn prepared() -> crate::workspace::runner::Prepared {
        crate::workspace::runner::Prepared {
            approval_arrival: crate::providers::approval_arrival(),
            request_id: "r-review".into(),
            run_id: "r-review-run".into(),
            task_id: 1,
            spec: crate::workspace::runner::CommandSpec::Direct {
                program: "tool".into(),
                args: vec![
                    "中文 long exact argument ".repeat(120),
                    "\u{1b}]52;not-a-terminal-operation".into(),
                ],
            },
            cwd: "/project 中文".into(),
            cwd_identity: "directory-id".into(),
            executable: crate::workspace::runner::ExecutableIdentity {
                path: "/tool path".into(),
                sha256: "hash".into(),
                bytes: 1,
                modified_ns: 1,
                file_id: "file-id".into(),
            },
            prepared_at_ms: 1,
            expires_at_ms: 100,
            before: crate::workspace::runner::InputSnapshot::default(),
            partner: None,
            rerun_of: None,
        }
    }

    #[test]
    fn compact_run_approval_and_cancel_name_the_requesting_partner() {
        let bolt = crate::store::Attribution {
            partner: 1,
            name: "Bolt".into(),
            form: 2,
        };
        let mut p = prepared();
        p.spec = crate::workspace::runner::CommandSpec::Direct {
            program: "cargo".into(),
            args: vec!["test".into()],
        };
        p.partner = Some(bolt.clone());
        let mut state = State::default();
        state.prepared = Some(p);
        state.owner = Some(bolt);
        for confirmation in [Confirmation::Approval, Confirmation::Cancel] {
            state.confirmation = Some(confirmation);
            let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    confirmation_frame(frame, area, &mut state);
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let text: String = (0..18)
                .map(|y| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(state.confirmation_visible, "{text}");
            assert!(text.contains("Stay / return"), "{text}");
            match state.confirmation {
                Some(Confirmation::Approval) => {
                    assert!(
                        text.contains("Bolt (Jolteon) wants to run: cargo test"),
                        "{text}"
                    );
                    assert!(text.contains("Approve this exact command"), "{text}");
                }
                _ => {
                    assert!(text.contains("Stop Bolt (Jolteon)'s command"), "{text}");
                    assert!(text.contains("Cancel and wait"), "{text}");
                }
            }
        }
    }

    fn confirmation_frame(frame: &mut ratatui::Frame, area: Rect, state: &mut State) {
        confirmation(frame, area, state, &paint(), None, false);
    }

    #[test]
    fn compact_approval_is_neutral_and_full_exact_details_remain_scrollable() {
        let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
        let mut state = State::default();
        state.prepared = Some(prepared());
        state.confirmation = Some(Confirmation::Approval);
        terminal
            .draw(|frame| {
                let area = frame.area();
                confirmation(frame, area, &mut state, &paint(), None, false);
            })
            .unwrap();
        assert!(!state.affirmative);
        assert!(
            state.confirmation_visible,
            "scope is visible without End or PgDown"
        );
        assert!(state.detail_max > 0);
        state.detail_scroll = state.detail_max;
        terminal
            .draw(|frame| {
                let area = frame.area();
                confirmation(frame, area, &mut state, &paint(), None, false);
            })
            .unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!rendered.contains('\u{1b}'));
    }

    #[test]
    fn compact_approval_wraps_the_command_and_hides_identity_until_details() {
        let mut p = prepared();
        p.spec = crate::workspace::runner::CommandSpec::Direct {
            program: "/tool path".into(),
            args: (0..12).map(|i| format!("--flag-{i}=value")).collect(),
        };
        p.expires_at_ms = u64::MAX;
        let mut state = State::default();
        state.prepared = Some(p);
        state.confirmation = Some(Confirmation::Approval);
        let draw = |state: &mut State| {
            let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    confirmation(frame, area, state, &paint(), Some("Bolt is editing"), false);
                })
                .unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..18)
                .map(|y| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let text = draw(&mut state);
        assert!(state.confirmation_visible, "{text}");
        // Wrapped rows show arguments a single clipped row could not.
        assert!(text.contains("--flag-0=value"), "{text}");
        assert!(text.contains("--flag-6=value"), "{text}");
        assert_eq!(
            wrapped_rows("rustc --edition=2021 --color=never", 24, 4),
            ["rustc --edition=2021 ", "--color=never"]
        );
        assert_eq!(wrapped_rows("abcdefgh", 4, 1), ["abc…"]);
        assert!(text.contains("One local command"), "{text}");
        assert!(text.contains("Not sandboxed"), "{text}");
        let full = execution::approval_details(state.prepared.as_ref().unwrap(), 4096);
        let brief = approval_brief(state.prepared.as_ref().unwrap(), &paint());
        assert!(full.contains("Executable SHA-256") && !brief.contains("SHA-256"));
        assert!(brief.contains("argv[11] = \"--flag-11=value\""), "{brief}");
        assert!(brief.contains("d Full identity"), "{brief}");
        // `d` reveals the full record without changing the neutral choice.
        state.shown = true;
        state.key(ratatui::crossterm::event::KeyEvent::from(
            ratatui::crossterm::event::KeyCode::Char('d'),
        ));
        assert!(state.approval_full && !state.affirmative);
        draw(&mut state);
        assert!(state.confirmation_visible);
    }

    #[test]
    fn unavailable_retained_output_never_renders_as_empty_success() {
        let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
        let mut state = State::default();
        state.showing_retained = true;
        state.retained_page = Some(crate::store::OutputPage {
            offset: 0,
            text: String::new(),
            total_bytes: 100,
            unavailable: true,
            truncated: true,
        });
        terminal
            .draw(|frame| {
                let area = frame.area();
                output(frame, area, &mut state, &paint());
            })
            .unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("UNAVAILABLE") && rendered.contains("TRUNCATED"));
        assert!(rendered.contains("NOT an empty"));
    }
}
