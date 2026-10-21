//! Move presentation is a pure view of local state; rendering never loads files or runs work.
use super::{Paint, centered, clipped, editor_rows, panel, row, selection_details, text};
use crate::{
    app::{
        App,
        moves::{Learning, Mode, State, kind, usage_outcome},
    },
    config::Config,
    domain::FORMS,
    workspace::moves::{Outcome, Proposal, Record, Steps},
};
use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Clear, Paragraph, Wrap},
};

pub(super) fn render(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    if area.width < 60 || area.height < 18 {
        app.moves.invalidate_review();
        frame.render_widget(Clear, area);
        text(
            frame,
            area,
            paint.hint(crate::i18n::Message::DesksMovesMovesNeedXDraftRetainedResizeTo),
            paint.accent,
        );
        return;
    }
    let wide = area.width >= 90;
    let area = centered(
        row(area, 2, area.height.saturating_sub(4)),
        116,
        area.height.saturating_sub(4),
    );
    frame.render_widget(Clear, area);
    let block = panel(
        paint.hint(crate::i18n::Message::DesksMovesPanel),
        config,
        paint,
        true,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let partner = app
        .partners()
        .iter()
        .find(|p| p.id == app.moves.partner)
        .map(App::partner_label)
        .unwrap_or_else(|| format!("Partner #{}", app.moves.partner));
    let form = &FORMS[app.moves.form.min(FORMS.len() - 1)];
    text(
        frame,
        row(inner, 0, 1),
        &format!(
            "{} / {} / {} / {} of 4 slots",
            partner,
            form.name,
            form.element,
            app.moves.active().count()
        ),
        paint.accent,
    );
    let notice = if app.moves.failed.is_some() {
        app.moves.failed_error.as_str()
    } else if app.moves.persistence_pending() {
        paint.hint(crate::i18n::Message::DesksMovesLoadingSavingLocalMoveFactsNoAction)
    } else if app.moves.notice.is_empty() {
        paint.hint(crate::i18n::Message::DesksMovesNamesAreLabelsOnlyEveryUseNeeds)
    } else {
        &app.moves.notice
    };
    text(frame, row(inner, 1, 2), notice, paint.muted);
    let body = row(inner, 3, inner.height.saturating_sub(5));
    let footer = row(inner, inner.height.saturating_sub(2), 2);
    app.moves.scroll_page = body.height.max(1);
    app.moves.reviewed = false;
    if app.moves.failed.is_some() {
        text(
            frame,
            row(body, 0, 3),
            if app.moves.failure_discard {
                paint.hint(crate::i18n::Message::DesksMovesDiscardThisUnsavedMoveChangeOrUsage)
            } else {
                paint.hint(crate::i18n::Message::DesksMovesMoveStorageFailedTheExactRequestIs)
            },
            paint.accent,
        );
        let details = format!(
            "{}\n{:#?}",
            app.moves.failed_error,
            app.moves.failed.as_ref().unwrap()
        );
        render_details(
            frame,
            row(body, 3, body.height.saturating_sub(3)),
            &details,
            &mut app.moves,
            paint,
        );
        app.moves.reviewed = body.height >= 4;
        text(
            frame,
            footer,
            if !app.moves.failure_discard {
                paint.hint(crate::i18n::Message::DesksMovesRRetryExactRequestDReviewDiscard)
            } else if app.moves.affirmative {
                paint
                    .hint(crate::i18n::Message::DesksMovesStayDiscardUnsavedRequestEnterDiscardLeft)
            } else {
                paint.hint(crate::i18n::Message::DesksMovesStayDiscardUnsavedRequestEnterStayLeft)
            },
            paint.accent,
        );
        return;
    }
    match app.moves.mode {
        Mode::Menu => {
            let active = app.moves.active();
            if active.clone().next().is_none() {
                text(
                    frame,
                    body,
                    paint.hint(crate::i18n::Message::DesksMovesNoMovesInThisFormYetTeach),
                    paint.surface,
                );
            } else if wide {
                let half = body.width / 2;
                let height = (body.height / 2).max(1);
                for (index, record) in active.enumerate().take(4) {
                    let cell = Rect::new(
                        body.x + (index % 2) as u16 * half,
                        body.y + (index / 2) as u16 * height,
                        if index % 2 == 0 {
                            half
                        } else {
                            body.width - half
                        },
                        height,
                    );
                    let style = if index == app.moves.selected {
                        paint.selected
                    } else {
                        paint.accent
                    };
                    let title = format!(
                        "{} {}",
                        if index == app.moves.selected {
                            ">"
                        } else {
                            " "
                        },
                        clipped(&record.proposal.name, cell.width.saturating_sub(3) as usize)
                    );
                    let last = app
                        .moves
                        .last_usage(record.id)
                        .map_or("not used", usage_outcome);
                    text(frame, row(cell, 0, 1), &title, style);
                    text(
                        frame,
                        row(cell, 1, cell.height.saturating_sub(1)),
                        &format!(
                            "{} / {} / {}\nLast: {}",
                            form.element,
                            kind(&record.proposal.steps),
                            record.proposal.scope.label(),
                            last
                        ),
                        paint.muted,
                    );
                }
            } else {
                let lines: Vec<String> = active
                    .enumerate()
                    .map(|(i, record)| {
                        let last = app
                            .moves
                            .last_usage(record.id)
                            .map_or("not used", usage_outcome);
                        format!(
                            "{} {}\n  {} / {} / {} / {}",
                            if i == app.moves.selected { ">" } else { " " },
                            clipped(&record.proposal.name, body.width.saturating_sub(3) as usize),
                            form.element,
                            kind(&record.proposal.steps),
                            record.proposal.scope.label(),
                            last
                        )
                    })
                    .collect();
                render_selection(frame, body, &lines, app.moves.selected, paint);
            }
            text(
                frame,
                footer,
                paint.hint(crate::i18n::Message::DesksMovesArrowsChooseEnterReviewUseDDetails),
                paint.accent,
            );
        }
        Mode::Name => render_name(frame, body, footer, &app.moves, paint),
        Mode::Detail => {
            let details = app
                .moves
                .selected_record()
                .map(|record| record_details(record, &app.moves, paint))
                .unwrap_or_else(|| {
                    paint
                        .hint(crate::i18n::Message::DesksMovesMoveNoLongerAvailableEscToReturn)
                        .into()
                });
            render_details(frame, body, &details, &mut app.moves, paint);
            let actions = [
                paint.hint(crate::i18n::Message::DesksMovesUseNormalReview),
                paint.hint(crate::i18n::Message::DesksMovesRename),
                paint.hint(crate::i18n::Message::DesksMovesForget),
                paint.hint(crate::i18n::Message::DesksMovesTeachToPartner),
            ];
            let selected = app.moves.selected.min(actions.len() - 1);
            text(
                frame,
                footer,
                &paint.format(
                    crate::i18n::Template::DesksMovesActionControls,
                    &[("action", (actions[selected]))],
                ),
                paint.accent,
            );
        }
        Mode::Reminder => {
            let forgotten_count = app.moves.reminders().count();
            let postponed_count = app.moves.postponed().count();
            let mut lines = vec![
                paint
                    .hint(crate::i18n::Message::DesksMovesForgottenPostponedInThisPartnerAndForm)
                    .into(),
            ];
            for (index, record) in app.moves.reminders().enumerate() {
                lines.push(format!(
                    "{} {} / {} / forgotten",
                    marker(index, app.moves.selected),
                    record.proposal.name,
                    record.proposal.scope.label()
                ));
            }
            for (index, candidate) in app.moves.postponed().enumerate() {
                lines.push(format!(
                    "{} {} / {} / postponed / {} observed sources",
                    marker(index + forgotten_count, app.moves.selected),
                    candidate.proposal.name,
                    candidate.proposal.scope.label(),
                    candidate.uses
                ));
            }
            if forgotten_count == 0 && postponed_count == 0 {
                lines.push(
                    paint
                        .hint(crate::i18n::Message::DesksMovesNoForgottenOrPostponedMoves)
                        .into(),
                );
            }
            render_selection(frame, body, &lines, app.moves.selected + 1, paint);
            text(
                frame,
                footer,
                paint.hint(crate::i18n::Message::DesksMovesUpDownChooseEnterRelearnOrInspect),
                paint.accent,
            );
        }
        Mode::Learn => {
            let details = app
                .moves
                .offered
                .as_ref()
                .map(|candidate| {
                    paint.format(
                        crate::i18n::Template::DesksMovesLearnDetails,
                        &[
                            ("partner", &(partner)),
                            ("sources-count", &(candidate.uses).to_string()),
                            ("proposal", &(proposal_details(&candidate.proposal, paint))),
                            (
                                "sources",
                                &(candidate
                                    .sources
                                    .iter()
                                    .map(|s| format!("- {s}"))
                                    .collect::<Vec<_>>()
                                    .join("\n")),
                            ),
                        ],
                    )
                })
                .unwrap_or_else(|| {
                    paint
                        .hint(crate::i18n::Message::DesksMovesCandidateUnavailable)
                        .into()
                });
            render_details(frame, body, &details, &mut app.moves, paint);
            let actions = [
                paint.hint(crate::i18n::Message::DesksMovesNotNowOfferAfterMoreUses),
                paint.hint(crate::i18n::Message::DesksMovesLearnChooseName),
                paint.hint(crate::i18n::Message::DesksMovesNeverForThisAction),
            ];
            text(
                frame,
                footer,
                &paint.format(
                    crate::i18n::Template::DesksMovesLearnControls,
                    &[("action", (actions[app.moves.selected.min(2)]))],
                ),
                paint.accent,
            );
        }
        Mode::TeachTarget => {
            let targets: Vec<_> = app
                .partners()
                .into_iter()
                .filter(|p| p.id != app.lead().id)
                .collect();
            let mut lines = vec![
                paint
                    .hint(crate::i18n::Message::DesksMovesCopyExactStepsIntoTheTargetS)
                    .into(),
            ];
            for (index, target) in targets.iter().enumerate() {
                lines.push(format!(
                    "{} {}",
                    marker(index, app.moves.selected),
                    App::partner_label(target)
                ));
            }
            if targets.is_empty() {
                lines.push(
                    paint
                        .hint(crate::i18n::Message::DesksMovesNoOtherPartyMembersAddAPartner)
                        .into(),
                );
            }
            render_selection(frame, body, &lines, app.moves.selected + 1, paint);
            text(
                frame,
                footer,
                paint.hint(crate::i18n::Message::DesksMovesUpDownTargetEnterReviewCopyEsc),
                paint.accent,
            );
        }
        Mode::LoadingTarget => {
            text(
                frame,
                body,
                paint.hint(crate::i18n::Message::DesksMovesLoadingTheChosenPartnerSActualMove),
                paint.surface,
            );
            text(
                frame,
                footer,
                paint.hint(crate::i18n::Message::DesksMovesEscBackWaitForTheLocalStore),
                paint.accent,
            );
        }
        Mode::Replace => {
            let active = app.moves.active();
            let mut lines = vec![
                paint
                    .hint(crate::i18n::Message::DesksMovesFourSlotsAreOccupiedAcrossProjectAnd)
                    .into(),
                paint.format(
                    crate::i18n::Template::DesksMovesGiveUp,
                    &[("marker", (marker(0, app.moves.selected)))],
                ),
            ];
            for (index, record) in active.enumerate() {
                lines.push(paint.format(
                    crate::i18n::Template::DesksMovesReplaceRow,
                    &[
                        ("marker", (marker(index + 1, app.moves.selected))),
                        ("name", &(record.proposal.name)),
                        ("scope", (record.proposal.scope.label())),
                        ("revision", &(record.revision).to_string()),
                    ],
                ));
            }
            if let Some(learning) = &app.moves.learning {
                lines.push(format!(
                    "New: {} / {} / {}",
                    app.moves.name,
                    app.moves.scope.label(),
                    kind(&learning.proposal().steps)
                ));
            }
            render_selection(frame, body, &lines, app.moves.selected + 1, paint);
            text(
                frame,
                footer,
                paint.hint(crate::i18n::Message::DesksMovesUpDownChooseEnterReviewExactReplacement),
                paint.accent,
            );
        }
        Mode::Review | Mode::Forget => {
            let (summary, details) = review_details(&app.moves, &partner, paint);
            // Effect and owner stay visible while long exact steps are scrolled.
            let summary = summary
                .lines()
                .map(|line| clipped(line, body.width as usize).into_owned())
                .collect::<Vec<_>>()
                .join("\n");
            text(frame, row(body, 0, 3), &summary, paint.accent);
            let detail_area = row(body, 3, body.height.saturating_sub(3));
            render_details(frame, detail_area, &details, &mut app.moves, paint);
            app.moves.reviewed = body.height >= 4;
            if !app.moves.reviewed {
                app.moves.affirmative = false;
            }
            let controls = if !app.moves.reviewed {
                paint
                    .hint(crate::i18n::Message::DesksMovesResizeToSeeTheLocalChangeSummary)
                    .to_owned()
            } else if app.moves.affirmative {
                paint
                    .hint(crate::i18n::Message::DesksMovesStayConfirmLocalChangeEnterConfirmLeft)
                    .into()
            } else {
                paint
                    .hint(crate::i18n::Message::DesksMovesStayConfirmLocalChangeEnterStayLeft)
                    .into()
            };
            text(frame, footer, &controls, paint.accent);
        }
        Mode::TaskReview => {
            text(
                frame,
                row(body, 0, 3),
                paint.hint(crate::i18n::Message::DesksMovesReplaceThisTaskSGoalAndCriteria),
                paint.accent,
            );
            let details = app
                .moves
                .selected_record()
                .map(|r| proposal_details(&r.proposal, paint))
                .unwrap_or_else(|| {
                    paint
                        .hint(crate::i18n::Message::DesksMovesMoveUnavailable)
                        .into()
                });
            render_details(
                frame,
                row(body, 3, body.height.saturating_sub(3)),
                &details,
                &mut app.moves,
                paint,
            );
            app.moves.reviewed = body.height >= 4;
            if !app.moves.reviewed {
                app.moves.affirmative = false;
            }
            text(
                frame,
                footer,
                if !app.moves.reviewed {
                    paint.hint(crate::i18n::Message::DesksMovesResizeToReviewTheTaskChangeEnter)
                } else if app.moves.affirmative {
                    paint.hint(crate::i18n::Message::DesksMovesStayApplyTemplateEnterApplyLeftRight)
                } else {
                    paint.hint(crate::i18n::Message::DesksMovesStayApplyTemplateEnterStayLeftRight)
                },
                paint.accent,
            );
        }
    }
}

fn marker(index: usize, selected: usize) -> &'static str {
    if index == selected { ">" } else { " " }
}
fn render_selection(
    frame: &mut Frame,
    area: Rect,
    lines: &[String],
    selected: usize,
    paint: &Paint,
) {
    let selected = selected.min(lines.len().saturating_sub(1));
    let selected_end = Paragraph::new(lines[..=selected].join("\n"))
        .wrap(Wrap { trim: false })
        .line_count(area.width.max(1));
    let top = selected_end.saturating_sub(area.height as usize);
    frame.render_widget(
        Paragraph::new(lines.join("\n"))
            .style(paint.surface)
            .wrap(Wrap { trim: false })
            .scroll((top.min(u16::MAX as usize) as u16, 0)),
        area,
    );
}
fn render_details(frame: &mut Frame, area: Rect, details: &str, state: &mut State, paint: &Paint) {
    state.scroll_page = area.height.max(1);
    let max = selection_details(
        frame,
        area,
        Paragraph::new(details)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        state.scroll,
    );
    state.scroll_max = max;
    state.scroll = state.scroll.min(max);
}
fn render_name(frame: &mut Frame, body: Rect, footer: Rect, state: &State, paint: &Paint) {
    let learning = state.learning.as_ref();
    let action = if matches!(learning, Some(Learning::Rename(_))) {
        paint.hint(crate::i18n::Message::DesksMovesRenameLabelOnly)
    } else {
        paint.hint(crate::i18n::Message::DesksMovesMoveName)
    };
    text(frame, row(body, 0, 1), action, paint.accent);
    let input = row(body, 1, body.height.saturating_sub(3).max(1));
    let (lines, (caret_row, caret_column)) = editor_rows(
        &state.name,
        state.cursor,
        input.width.saturating_sub(1) as usize,
    );
    let top = caret_row.saturating_sub(input.height.saturating_sub(1) as usize);
    frame.render_widget(
        Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
            .style(if state.scope_focus {
                paint.surface
            } else {
                paint.accent
            })
            .scroll((top.min(u16::MAX as usize) as u16, 0)),
        input,
    );
    if !state.scope_focus && input.width > 0 && input.height > 0 {
        frame.set_cursor_position((
            input
                .x
                .saturating_add((caret_column as u16).min(input.width - 1)),
            input
                .y
                .saturating_add((caret_row.saturating_sub(top) as u16).min(input.height - 1)),
        ));
    }
    text(
        frame,
        row(body, body.height.saturating_sub(2), 2),
        &paint.format(
            crate::i18n::Template::DesksMovesNameScope,
            &[
                ("marker", (if state.scope_focus { ">" } else { " " })),
                ("scope", (state.scope.label())),
                (
                    "change",
                    (if matches!(learning, Some(Learning::Teach(_))) {
                        paint.hint(crate::i18n::Message::DesksMovesLeftRightChangesScope)
                    } else {
                        paint.hint(crate::i18n::Message::DesksMovesUnchanged)
                    }),
                ),
            ],
        ),
        paint.muted,
    );
    text(
        frame,
        footer,
        paint.hint(crate::i18n::Message::DesksMovesTypeNameTabNameOrScopeEnter),
        paint.accent,
    );
}

pub(super) fn proposal_details(proposal: &Proposal, paint: &Paint) -> String {
    let mut text = format!(
        "{} / {} / {}\nPartner #{} / {} form\n",
        proposal.name,
        proposal.scope.label(),
        kind(&proposal.steps),
        proposal.partner,
        FORMS[proposal.form.min(FORMS.len() - 1)].name
    );
    match &proposal.steps {
        Steps::Run { argv, cwd } => {
            text.push_str(&paint.format(
                crate::i18n::Template::DesksMovesRunDetails,
                &[("argv", &format!("{:?}", argv)), ("cwd", (cwd))],
            ));
        }
        Steps::Pins { pins } => {
            text.push_str(
                paint.hint(crate::i18n::Message::DesksMovesPinsAreResolvedAgainAtUseUnavailable),
            );
            for pin in pins {
                text.push_str(&format!(
                    "{}:{}-{}\n",
                    pin.path, pin.start_line, pin.end_line
                ));
            }
        }
        Steps::Request { instruction, pins } => {
            text.push_str(paint.hint(
                crate::i18n::Message::DesksMovesUserAuthoredInstructionOnlyNotProviderOutput,
            ));
            text.push_str(instruction);
            text.push_str(paint.hint(crate::i18n::Message::DesksMovesContextSelectors));
            for pin in pins {
                text.push_str(&format!(
                    "{}:{}-{}\n",
                    pin.path, pin.start_line, pin.end_line
                ));
            }
            text.push_str(
                paint
                    .hint(crate::i18n::Message::DesksMovesNormalFullRequestReceiptIsRequiredBefore),
            );
        }
        Steps::Task { goal, criteria } => {
            text.push_str(&format!("Goal:\n{goal}\nCriteria:\n"));
            for criterion in criteria {
                text.push_str(&format!("- {criterion}\n"));
            }
        }
    }
    text.push_str(&format!(
        "Provenance task #{}\n",
        proposal.provenance.task_id
    ));
    for source in &proposal.provenance.sources {
        text.push_str(&format!("Source: {source}\n"));
    }
    if let Some(teacher) = proposal.provenance.taught_by {
        text.push_str(&format!(
            "Taught by {} (partner #{teacher})\n",
            proposal
                .provenance
                .teacher_name
                .as_deref()
                .unwrap_or("unnamed partner")
        ));
    }
    text
}
fn record_details(record: &Record, state: &State, paint: &Paint) -> String {
    let usages = state.usages.iter().filter(|u| u.move_id == record.id);
    let count = |outcome: Outcome| usages.clone().filter(|u| u.outcome == outcome).count();
    let exited_zero = usages
        .clone()
        .filter(|u| u.run_id.is_some() && u.outcome == Outcome::Succeeded)
        .count();
    let failed = usages
        .clone()
        .filter(|u| u.run_id.is_some() && u.outcome == Outcome::Failed)
        .count();
    let completed = usages
        .clone()
        .filter(|u| {
            u.outcome == Outcome::Completed
                || (u.run_id.is_none() && u.outcome == Outcome::Succeeded)
        })
        .count();
    let mut text = paint.format(
        crate::i18n::Template::DesksMovesUsageDetails,
        &[
            ("proposal", &(proposal_details(&record.proposal, paint))),
            ("id", &(record.id).to_string()),
            ("revision", &(record.revision).to_string()),
            (
                "status",
                (if record.forgotten {
                    "forgotten"
                } else {
                    "active"
                }),
            ),
            ("uses", &(usages.clone().count()).to_string()),
            ("exited-zero", &(exited_zero).to_string()),
            ("exited-total", &(exited_zero + failed).to_string()),
            ("denied", &(count(Outcome::Denied)).to_string()),
            ("cancelled", &(count(Outcome::Cancelled)).to_string()),
            ("unknown", &(count(Outcome::Unknown)).to_string()),
            ("pending", &(count(Outcome::Pending)).to_string()),
            ("completed", &(completed).to_string()),
        ],
    );
    if let Some(last) = state.last_usage(record.id) {
        text.push_str(&format!(
            "Last: {} / revision {}\n",
            usage_outcome(last),
            last.revision.as_deref().unwrap_or("unknown")
        ));
    }
    if usages.clone().next().is_none() {
        text.push_str(paint.hint(crate::i18n::Message::DesksMovesNoJournalLinkedUsesRecorded));
    }
    for usage in usages.rev() {
        text.push_str(&format!(
            "Use {} / task #{} / partner #{} / {}\n  run: {} / request: {} / revision: {}\n",
            usage.id,
            usage.task_id,
            usage.partner,
            usage_outcome(usage),
            usage.run_id.as_deref().unwrap_or("none"),
            usage.request_id.as_deref().unwrap_or("none"),
            usage.revision.as_deref().unwrap_or("unknown")
        ));
    }
    text
}
fn review_details(state: &State, partner: &str, paint: &Paint) -> (String, String) {
    if state.mode == Mode::Forget {
        return state
            .selected_record()
            .map(|record| {
                (
                    paint.format(
                        crate::i18n::Template::DesksMovesForgetSummary,
                        &[
                            ("scope", (record.proposal.scope.label())),
                            ("id", &(record.id).to_string()),
                            ("owner", &(clipped(partner, 16))),
                            ("name", &(clipped(&record.proposal.name, 20))),
                        ],
                    ),
                    record_details(record, state, paint),
                )
            })
            .unwrap_or_else(|| {
                (
                    paint
                        .hint(crate::i18n::Message::DesksMovesMoveUnavailable)
                        .into(),
                    String::new(),
                )
            });
    }
    let Some(learning) = &state.learning else {
        return (
            paint
                .hint(crate::i18n::Message::DesksMovesNoChangeSelected)
                .into(),
            String::new(),
        );
    };
    let mut proposal = learning.proposal().clone();
    proposal.name.clone_from(&state.name);
    proposal.scope = state.scope;
    let action = match learning {
        Learning::Teach(_) => paint.hint(crate::i18n::Message::DesksMovesTeach),
        Learning::Relearn(_) => paint.hint(crate::i18n::Message::DesksMovesRelearnIdenticalSteps),
        Learning::Rename(_) => paint.hint(crate::i18n::Message::DesksMovesRenameLabel),
    };
    let mut summary = paint.format(
        crate::i18n::Template::DesksMovesChangeSummary,
        &[
            ("action", (action)),
            ("scope", (state.scope.label())),
            ("owner", &(clipped(partner, 16))),
        ],
    );
    let mut details = proposal_details(&proposal, paint);
    if let Some((id, revision)) = state.replacement
        && let Some(record) = state.records.iter().find(|r| r.id == id)
    {
        summary.push_str(&paint.format(
            crate::i18n::Template::DesksMovesReplacementSummary,
            &[
                ("scope", (record.proposal.scope.label())),
                ("id", &(record.id).to_string()),
            ],
        ));
        details.push_str(&paint.format(
            crate::i18n::Template::DesksMovesReplacementDetails,
            &[
                ("id", &(id).to_string()),
                ("revision", &(revision).to_string()),
                ("proposal", &(proposal_details(&record.proposal, paint))),
            ],
        ));
    }
    (summary, details)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::moves::{Pin, Provenance, Scope, Usage};
    fn record() -> Record {
        Record {
            id: 1,
            revision: 2,
            project: "project".into(),
            proposal: Proposal {
                partner: 1,
                form: 1,
                name: "水の波".into(),
                scope: Scope::Project,
                steps: Steps::Pins {
                    pins: vec![Pin {
                        path: "src/lib.rs".into(),
                        start_line: 2,
                        end_line: 4,
                    }],
                },
                provenance: Provenance {
                    task_id: 3,
                    sources: vec!["request:source".into()],
                    taught_by: Some(2),
                    teacher_name: Some("Mizu".into()),
                },
            },
            forgotten: false,
            created: 1,
        }
    }
    #[test]
    fn preview_keeps_exact_selectors_and_teacher_sources() {
        let paint = &Paint::new(&config(), &FORMS[1], 1);
        let preview = proposal_details(&record().proposal, paint);
        assert!(preview.contains("src/lib.rs:2-4"));
        assert!(preview.contains("request:source"));
        assert!(preview.contains("Taught by Mizu (partner #2)"));
        assert!(preview.contains("水の波"));
    }
    #[test]
    fn usage_summary_separates_denied_cancelled_unknown_and_exit_zero() {
        let paint = &Paint::new(&config(), &FORMS[1], 1);
        let mut state = State::default();
        for (index, result) in [
            Outcome::Succeeded,
            Outcome::Failed,
            Outcome::Denied,
            Outcome::Cancelled,
            Outcome::Unknown,
        ]
        .into_iter()
        .enumerate()
        {
            state.usages.push(Usage {
                id: format!("use:{index}"),
                move_id: 1,
                task_id: 3,
                partner: 1,
                run_id: Some(format!("run:{index}")),
                request_id: None,
                outcome: result,
                revision: Some("abc123".into()),
            });
        }
        let detail = record_details(&record(), &state, paint);
        assert!(detail.contains("1/2 exited 0"));
        assert!(detail.contains("Denied: 1 / Cancelled: 1 / Unknown: 1"));
        assert!(detail.contains("run:4"));
        assert!(detail.contains("abc123"));
        assert!(detail.contains("Exit 0 is not correctness"));
    }
    #[test]
    fn replacement_summary_names_scope_and_exact_target() {
        let paint = &Paint::new(&config(), &FORMS[1], 1);
        let record = record();
        let mut state = State::default();
        state.mode = Mode::Review;
        state.learning = Some(Learning::Teach(record.proposal.clone()));
        state.name = "Surf".into();
        state.scope = Scope::Personal;
        state.replacement = Some((1, 2));
        state.records.push(record);
        let (summary, details) = review_details(&state, "Bolt", paint);
        assert!(summary.contains("Bolt"));
        assert!(summary.contains("Forgets Project move #1"));
        assert!(details.contains("move #1, revision 2"));
    }
    fn config() -> Config {
        Config {
            ascii: true,
            language: crate::i18n::Language::ENGLISH,
            no_animation: true,
            theme: crate::config::Theme::Dark,
            color: crate::config::ColorMode::Mono,
            images: crate::config::ImageMode::Off,
            assets_dir: std::path::PathBuf::new(),
            layout: crate::config::LayoutMode::Linear,
            layout_explicit: true,
            data_dir: std::path::PathBuf::new(),
            storage_policy: crate::store::Policy::default(),
            context_exclusions: vec![],
            capture_capacity: 1024,
        }
    }
    fn draw(app: &mut App, width: u16, height: u16) -> String {
        let config = config();
        let paint = Paint::new(&config, &FORMS[1], 1);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), app, &config, &paint))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }
    #[test]
    fn ascii_grid_and_small_list_keep_selected_fourth_move_visible() {
        for (width, height) in [(120, 36), (60, 18)] {
            let mut app = App::new("project".into(), width, height);
            app.moves.partner = 1;
            app.moves.form = 1;
            app.moves.selected = 3;
            for id in 1..=4 {
                let mut record = record();
                record.id = id;
                record.proposal.name = format!("Surf{id}");
                app.moves.records.push(record);
                app.moves.usages.push(Usage {
                    id: format!("use-{id}"),
                    move_id: id,
                    task_id: 3,
                    partner: 1,
                    run_id: None,
                    request_id: None,
                    outcome: Outcome::Completed,
                    revision: None,
                });
            }
            let buffer = draw(&mut app, width, height);
            assert!(buffer.contains("> Surf4"));
            assert!(buffer.contains("Moves"));
            assert!(buffer.is_ascii());
        }
    }
    #[test]
    fn rendered_concise_review_is_neutral_without_forcing_scroll_to_end() {
        let mut app = App::new("project".into(), 60, 18);
        let record = record();
        app.moves.partner = 1;
        app.moves.form = 1;
        app.moves.mode = Mode::Review;
        app.moves.name = record.proposal.name.clone();
        app.moves.learning = Some(Learning::Teach(record.proposal));
        let buffer = draw(&mut app, 60, 18);
        assert!(app.moves.reviewed);
        assert!(!app.moves.affirmative);
        assert!(app.moves.scroll_max > 0);
        assert!(buffer.contains("[Stay]"));
        assert!(buffer.contains("No execution"));
        assert!(buffer.contains("Project"));
    }
    #[test]
    fn too_small_render_revokes_approval_without_losing_name() {
        let mut app = App::new("project".into(), 40, 10);
        app.moves.name = "水の波".into();
        app.moves.reviewed = true;
        app.moves.affirmative = true;
        let buffer = draw(&mut app, 40, 10);
        assert!(buffer.contains("60 x 18"));
        assert!(!app.moves.reviewed);
        assert!(!app.moves.affirmative);
        assert_eq!(app.moves.name, "水の波");
    }
    #[test]
    fn provider_success_is_never_labeled_as_local_exit_zero() {
        let paint = &Paint::new(&config(), &FORMS[1], 1);
        let mut state = State::default();
        state.usages.push(Usage {
            id: "use:request".into(),
            move_id: 1,
            task_id: 3,
            partner: 1,
            run_id: None,
            request_id: Some("request:1".into()),
            outcome: Outcome::Succeeded,
            revision: None,
        });
        let detail = record_details(&record(), &state, paint);
        assert!(detail.contains("0/0 exited 0"));
        assert!(detail.contains("Completed without an exit verdict: 1"));
        assert_eq!(
            usage_outcome(&state.usages[0]),
            "completed (not a correctness verdict)"
        );
    }
}
