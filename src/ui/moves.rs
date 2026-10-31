//! Move presentation is a pure view of local state; rendering never loads files or runs work.
use super::{Paint, centered, clipped, editor_rows, panel, row, selection_details, text};
use crate::{
    app::{
        App,
        moves::{Learning, Mode, ReviewText, State, kind, usage_outcome},
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
            paint.hint(crate::messages::Message::DesksMovesMovesNeedXDraftRetainedResizeTo),
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
        paint.hint(crate::messages::Message::DesksMovesPanel),
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
            "{} / {} / {} / Library {}/32 / Quick {}/4",
            partner,
            form.name,
            form.element,
            app.moves.active().count(),
            app.moves.quick().count()
        ),
        paint.accent,
    );
    let notice = if app.moves.failed.is_some() {
        app.moves.failed_error.as_str()
    } else if app.moves.persistence_pending() {
        paint.hint(crate::messages::Message::DesksMovesLoadingSavingLocalMoveFactsNoAction)
    } else if matches!(app.moves.mode, Mode::ImportPath | Mode::ExportPath)
        && app.moves.insertion_rejected
    {
        paint.hint(crate::messages::Message::TmPathRefused)
    } else if app.moves.portable_exported {
        paint.hint(crate::messages::Message::TmExportSaved)
    } else if app.moves.notice.is_empty() {
        paint.hint(crate::messages::Message::DesksMovesNamesAreLabelsOnlyEveryUseNeeds)
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
                paint.hint(crate::messages::Message::DesksMovesDiscardThisUnsavedMoveChangeOrUsage)
            } else {
                paint.hint(crate::messages::Message::DesksMovesMoveStorageFailedTheExactRequestIs)
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
        super::choices(
            frame,
            footer,
            if !app.moves.failure_discard {
                paint.hint(crate::messages::Message::DesksMovesRRetryExactRequestDReviewDiscard)
            } else if app.moves.affirmative {
                paint.hint(
                    crate::messages::Message::DesksMovesStayDiscardUnsavedRequestEnterDiscardLeft,
                )
            } else {
                paint.hint(
                    crate::messages::Message::DesksMovesStayDiscardUnsavedRequestEnterStayLeft,
                )
            },
            paint.accent,
            paint,
        );
        return;
    }
    match app.moves.mode {
        Mode::Menu | Mode::Library => {
            let active = app.moves.menu_records();
            if active.clone().next().is_none() {
                text(
                    frame,
                    body,
                    paint.hint(crate::messages::Message::DesksMovesNoMovesInThisFormYetTeach),
                    paint.surface,
                );
            } else if wide && app.moves.mode == Mode::Menu {
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
                        "{} Q{} {}",
                        if index == app.moves.selected {
                            ">"
                        } else {
                            " "
                        },
                        record
                            .proposal
                            .extra
                            .0
                            .get("quick_slot")
                            .and_then(serde_json::Value::as_u64)
                            .map_or(index + 1, |slot| slot as usize + 1),
                        clipped(&record.proposal.name, cell.width.saturating_sub(6) as usize)
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
                paint.hint(if app.moves.mode == Mode::Library {
                    crate::messages::Message::TmLibraryControls
                } else {
                    crate::messages::Message::TmMenuControls
                }),
                paint.accent,
            );
        }
        Mode::ImportPath | Mode::ExportPath => {
            render_name(frame, body, footer, &app.moves, paint);
        }
        Mode::ImportTarget => {
            text(
                frame,
                row(body, 0, 1),
                paint.hint(crate::messages::Message::TmImportTitle),
                paint.accent,
            );
            let partners = app.partners();
            let partner = partners
                .get(app.moves.selected)
                .map(App::partner_label)
                .unwrap_or_default();
            text(
                frame,
                row(body, 1, 1),
                &paint.format(
                    crate::messages::Template::TmImportPartner,
                    &[("partner", &partner)],
                ),
                paint.accent,
            );
            if let Some(definition) = &app.moves.portable {
                let details = format!(
                    "{}\n{}\n{}",
                    paint.format(
                        crate::messages::Template::TmImportSummary,
                        &[
                            ("name", &definition.name),
                            ("kind", kind(&definition.steps)),
                            ("form", &definition.form)
                        ]
                    ),
                    app.moves.portable_text,
                    paint.hint(crate::messages::Message::TmImportBoundary)
                );
                render_details(
                    frame,
                    row(body, 2, body.height.saturating_sub(2)),
                    &details,
                    &mut app.moves,
                    paint,
                );
            }
            text(
                frame,
                footer,
                paint.hint(crate::messages::Message::TmImportControls),
                paint.accent,
            );
        }
        Mode::ExportReview => {
            text(
                frame,
                row(body, 0, 1),
                paint.hint(crate::messages::Message::TmExportTitle),
                paint.accent,
            );
            if !app.moves.portable_text.is_empty() {
                let details = paint.format(
                    crate::messages::Template::TmExportDetails,
                    &[
                        ("path", &app.moves.file_path),
                        ("definition", &app.moves.portable_text),
                    ],
                );
                render_details(
                    frame,
                    row(body, 1, body.height.saturating_sub(1)),
                    &details,
                    &mut app.moves,
                    paint,
                );
                app.moves.reviewed = body.height >= 4;
            }
            super::choices(
                frame,
                footer,
                paint.hint(if app.moves.affirmative {
                    crate::messages::Message::TmExportArmed
                } else {
                    crate::messages::Message::TmExportControls
                }),
                paint.accent,
                paint,
            );
        }
        Mode::Name | Mode::Parameters => render_name(frame, body, footer, &app.moves, paint),
        Mode::ParameterReview => {
            let details = std::mem::take(&mut app.moves.parameter_review);
            render_details(frame, body, &details, &mut app.moves, paint);
            app.moves.parameter_review = details;
            app.moves.reviewed = body.height >= 4
                && app.moves.parameter_current()
                && app.moves.exposed_rows >= app.moves.scroll_max as usize + body.height as usize;
            if !app.moves.reviewed {
                app.moves.affirmative = false;
            }
            let controls = paint.hint(if !app.moves.reviewed {
                crate::messages::Message::TmSemanticExpose
            } else if app.moves.affirmative {
                crate::messages::Message::DesksMovesStayConfirmLocalChangeEnterConfirmLeft
            } else {
                crate::messages::Message::DesksMovesStayConfirmLocalChangeEnterStayLeft
            });
            super::choices(frame, footer, controls, paint.accent, paint);
        }
        Mode::Detail => {
            let details = app
                .moves
                .selected_record()
                .map(|record| record_details(record, &app.moves, paint))
                .unwrap_or_else(|| {
                    paint
                        .hint(crate::messages::Message::DesksMovesMoveNoLongerAvailableEscToReturn)
                        .into()
                });
            render_details(frame, body, &details, &mut app.moves, paint);
            let actions = [
                paint.hint(crate::messages::Message::DesksMovesUseNormalReview),
                paint.hint(crate::messages::Message::DesksMovesRename),
                paint.hint(crate::messages::Message::DesksMovesForget),
                paint.hint(crate::messages::Message::DesksMovesTeachToPartner),
            ];
            let selected = app.moves.selected.min(actions.len() - 1);
            text(
                frame,
                footer,
                &paint.format(
                    crate::messages::Template::DesksMovesActionControls,
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
                    .hint(
                        crate::messages::Message::DesksMovesForgottenPostponedInThisPartnerAndForm,
                    )
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
                        .hint(crate::messages::Message::DesksMovesNoForgottenOrPostponedMoves)
                        .into(),
                );
            }
            render_selection(frame, body, &lines, app.moves.selected + 1, paint);
            text(
                frame,
                footer,
                paint.hint(crate::messages::Message::DesksMovesUpDownChooseEnterRelearnOrInspect),
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
                        crate::messages::Template::DesksMovesLearnDetails,
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
                        .hint(crate::messages::Message::DesksMovesCandidateUnavailable)
                        .into()
                });
            render_details(frame, body, &details, &mut app.moves, paint);
            let actions = [
                paint.hint(crate::messages::Message::DesksMovesNotNowOfferAfterMoreUses),
                paint.hint(crate::messages::Message::DesksMovesLearnChooseName),
                paint.hint(crate::messages::Message::DesksMovesNeverForThisAction),
            ];
            text(
                frame,
                footer,
                &paint.format(
                    crate::messages::Template::DesksMovesLearnControls,
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
                    .hint(crate::messages::Message::DesksMovesCopyExactStepsIntoTheTargetS)
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
                        .hint(crate::messages::Message::DesksMovesNoOtherPartyMembersAddAPartner)
                        .into(),
                );
            }
            render_selection(frame, body, &lines, app.moves.selected + 1, paint);
            text(
                frame,
                footer,
                paint.hint(crate::messages::Message::DesksMovesUpDownTargetEnterReviewCopyEsc),
                paint.accent,
            );
        }
        Mode::LoadingTarget => {
            text(
                frame,
                body,
                paint.hint(crate::messages::Message::DesksMovesLoadingTheChosenPartnerSActualMove),
                paint.surface,
            );
            text(
                frame,
                footer,
                paint.hint(crate::messages::Message::DesksMovesEscBackWaitForTheLocalStore),
                paint.accent,
            );
        }
        Mode::Replace => {
            let active = app.moves.active();
            let mut lines = vec![
                paint
                    .hint(crate::messages::Message::DesksMovesFourSlotsAreOccupiedAcrossProjectAnd)
                    .into(),
                paint.format(
                    crate::messages::Template::DesksMovesGiveUp,
                    &[("marker", (marker(0, app.moves.selected)))],
                ),
            ];
            for (index, record) in active.enumerate() {
                lines.push(paint.format(
                    crate::messages::Template::DesksMovesReplaceRow,
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
                paint.hint(
                    crate::messages::Message::DesksMovesUpDownChooseEnterReviewExactReplacement,
                ),
                paint.accent,
            );
        }
        Mode::Review | Mode::Forget => {
            if app
                .moves
                .review_text
                .as_ref()
                .is_some_and(|cached| cached.recipient != partner)
            {
                app.moves.invalidate_review();
            }
            let review_text = app
                .moves
                .review_text
                .take()
                .filter(|cached| {
                    app.moves.mode == Mode::Review
                        && app.moves.review_current()
                        && cached.recipient == partner
                })
                .unwrap_or_else(|| {
                    let (summary, details) = review_details(&app.moves, &partner, paint);
                    ReviewText {
                        recipient: partner,
                        summary,
                        details,
                    }
                });
            let summary = &review_text.summary;
            let details = &review_text.details;
            // Effect and owner stay visible while long exact steps are scrolled.
            let summary = summary
                .lines()
                .map(|line| clipped(line, body.width as usize).into_owned())
                .collect::<Vec<_>>()
                .join("\n");
            text(frame, row(body, 0, 3), &summary, paint.accent);
            let detail_area = row(body, 3, body.height.saturating_sub(3));
            render_details(frame, detail_area, details, &mut app.moves, paint);
            app.moves.reviewed = body.height >= 4
                && (app.moves.mode != Mode::Review
                    || (app.moves.review_current()
                        && app.moves.exposed_rows
                            >= app.moves.scroll_max as usize + detail_area.height as usize));
            if !app.moves.reviewed {
                app.moves.affirmative = false;
            }
            if app.moves.mode == Mode::Review && app.moves.review_current() {
                app.moves.review_text = Some(review_text);
            }
            let controls = if !app.moves.reviewed {
                paint
                    .hint(crate::messages::Message::TmSemanticExpose)
                    .to_owned()
            } else if app.moves.affirmative {
                paint
                    .hint(
                        crate::messages::Message::DesksMovesStayConfirmLocalChangeEnterConfirmLeft,
                    )
                    .into()
            } else {
                paint
                    .hint(crate::messages::Message::DesksMovesStayConfirmLocalChangeEnterStayLeft)
                    .into()
            };
            super::choices(frame, footer, &controls, paint.accent, paint);
        }
        Mode::TaskReview => {
            text(
                frame,
                row(body, 0, 3),
                paint.hint(crate::messages::Message::DesksMovesReplaceThisTaskSGoalAndCriteria),
                paint.accent,
            );
            let details = app
                .moves
                .selected_record()
                .map(|r| proposal_details(&r.proposal, paint))
                .unwrap_or_else(|| {
                    paint
                        .hint(crate::messages::Message::DesksMovesMoveUnavailable)
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
            super::choices(
                frame,
                footer,
                if !app.moves.reviewed {
                    paint.hint(crate::messages::Message::DesksMovesResizeToReviewTheTaskChangeEnter)
                } else if app.moves.affirmative {
                    paint.hint(
                        crate::messages::Message::DesksMovesStayApplyTemplateEnterApplyLeftRight,
                    )
                } else {
                    paint.hint(
                        crate::messages::Message::DesksMovesStayApplyTemplateEnterStayLeftRight,
                    )
                },
                paint.accent,
                paint,
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
    if ((state.mode == Mode::Review && state.review_current())
        || (state.mode == Mode::ParameterReview && state.parameter_current()))
        && state.scroll as usize <= state.exposed_rows
    {
        state.exposed_rows = state
            .exposed_rows
            .max(state.scroll as usize + area.height as usize);
    }
}
fn render_name(frame: &mut Frame, body: Rect, footer: Rect, state: &State, paint: &Paint) {
    let learning = state.learning.as_ref();
    let parameter_editor = state.mode == Mode::Parameters;
    let action = if parameter_editor {
        paint.hint(crate::messages::Message::TmParametersEditor)
    } else if state.mode == Mode::ImportPath {
        paint.hint(crate::messages::Message::TmImportPath)
    } else if state.mode == Mode::ExportPath {
        paint.hint(crate::messages::Message::TmExportPath)
    } else if matches!(learning, Some(Learning::Rename(_))) {
        paint.hint(crate::messages::Message::DesksMovesRenameLabelOnly)
    } else {
        paint.hint(crate::messages::Message::DesksMovesMoveName)
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
    if state.mode != Mode::Name {
        text(
            frame,
            row(body, body.height.saturating_sub(2), 2),
            paint.hint(if parameter_editor {
                crate::messages::Message::TmParametersBoundary
            } else {
                crate::messages::Message::TmPathBoundary
            }),
            paint.muted,
        );
        text(
            frame,
            footer,
            paint.hint(if parameter_editor {
                crate::messages::Message::TmParametersControls
            } else {
                crate::messages::Message::TmPathControls
            }),
            paint.accent,
        );
        return;
    }
    text(
        frame,
        row(body, body.height.saturating_sub(2), 2),
        &paint.format(
            crate::messages::Template::DesksMovesNameScope,
            &[
                ("marker", (if state.scope_focus { ">" } else { " " })),
                ("scope", (state.scope.label())),
                (
                    "change",
                    (if matches!(
                        learning,
                        Some(Learning::Teach(_) | Learning::Import(_) | Learning::Copy(..))
                    ) {
                        paint.hint(crate::messages::Message::DesksMovesLeftRightChangesScope)
                    } else {
                        paint.hint(crate::messages::Message::DesksMovesUnchanged)
                    }),
                ),
            ],
        ),
        paint.muted,
    );
    text(
        frame,
        footer,
        paint.hint(crate::messages::Message::DesksMovesTypeNameTabNameOrScopeEnter),
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
                crate::messages::Template::DesksMovesRunDetails,
                &[("argv", &format!("{argv:?}")), ("cwd", &format!("{cwd:?}"))],
            ));
        }
        Steps::Pins { pins } => {
            text.push_str(
                paint
                    .hint(crate::messages::Message::DesksMovesPinsAreResolvedAgainAtUseUnavailable),
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
                crate::messages::Message::DesksMovesUserAuthoredInstructionOnlyNotProviderOutput,
            ));
            text.push_str(&format!("{instruction:?}"));
            text.push_str(paint.hint(crate::messages::Message::DesksMovesContextSelectors));
            for pin in pins {
                text.push_str(&format!(
                    "{}:{}-{}\n",
                    pin.path, pin.start_line, pin.end_line
                ));
            }
            text.push_str(paint.hint(
                crate::messages::Message::DesksMovesNormalFullRequestReceiptIsRequiredBefore,
            ));
        }
        Steps::Task { goal, criteria } => {
            text.push_str(&format!("Goal:\n{goal:?}\nCriteria:\n"));
            for criterion in criteria {
                text.push_str(&format!("- {criterion:?}\n"));
            }
        }
    }
    match crate::workspace::moves::parameters::Template::of(proposal) {
        Ok(Some(template)) => {
            match template.review(&proposal.steps, &template.defaults_json()) {
                Ok((_, details)) => text.push_str(&format!("\nTyped request template:\n{details}")),
                Err(error) => text.push_str(&format!("\nUnavailable parameters: {error}\n")),
            }
            text.push_str(&crate::workspace::game::variant_review(&template.variants));
        }
        Err(error) => text.push_str(&format!("\nUnavailable parameters: {error}\n")),
        Ok(None) => {}
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
        crate::messages::Template::DesksMovesUsageDetails,
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
    let quick_slot = record
        .proposal
        .extra
        .0
        .get("quick_slot")
        .and_then(serde_json::Value::as_u64)
        .map(|slot| slot as usize)
        .or_else(|| {
            (!state
                .active()
                .any(|record| record.proposal.extra.contains("quick_slot")))
            .then(|| state.quick().position(|quick| quick.id == record.id))
            .flatten()
        });
    text.push_str(&quick_slot.map_or_else(
        || "\nLibrary only (not a quick move)\n".into(),
        |slot| format!("\nQuick slot {} / library retained\n", slot + 1),
    ));
    if let Some(last) = state.last_usage(record.id) {
        text.push_str(&format!(
            "Last: {} / revision {}\n",
            usage_outcome(last),
            last.revision.as_deref().unwrap_or("unknown")
        ));
    }
    if usages.clone().next().is_none() {
        text.push_str(paint.hint(crate::messages::Message::DesksMovesNoJournalLinkedUsesRecorded));
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
                        crate::messages::Template::DesksMovesForgetSummary,
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
                        .hint(crate::messages::Message::DesksMovesMoveUnavailable)
                        .into(),
                    String::new(),
                )
            });
    }
    let Some(learning) = &state.learning else {
        return (
            paint
                .hint(crate::messages::Message::DesksMovesNoChangeSelected)
                .into(),
            String::new(),
        );
    };
    let Some(review) = &state.review else {
        return (
            paint.hint(crate::messages::Message::TmSemanticStale).into(),
            String::new(),
        );
    };
    if !state.review_current() {
        return (
            paint.hint(crate::messages::Message::TmSemanticStale).into(),
            String::new(),
        );
    }
    let proposal = &review.proposal;
    let action = match learning {
        Learning::Teach(_) | Learning::Copy(..) => {
            paint.hint(crate::messages::Message::DesksMovesTeach)
        }
        Learning::Import(_) => paint.hint(crate::messages::Message::TmImportAction),
        Learning::Relearn(_) => {
            paint.hint(crate::messages::Message::DesksMovesRelearnIdenticalSteps)
        }
        Learning::Rename(_) => paint.hint(crate::messages::Message::DesksMovesRenameLabel),
    };
    let mut summary = paint.format(
        crate::messages::Template::DesksMovesChangeSummary,
        &[
            ("action", (action)),
            ("scope", (state.scope.label())),
            ("owner", &(clipped(partner, 16))),
        ],
    );
    let mut details = paint.format(
        crate::messages::Template::TmSemanticScope,
        &[
            ("partner", partner),
            ("id", &proposal.partner.to_string()),
            ("form", FORMS[proposal.form].name),
            ("scope", proposal.scope.label()),
            ("name", &proposal.name),
        ],
    );
    details.push('\n');
    details.push_str(paint.hint(crate::messages::Message::TmSemanticLiterals));
    details.push('\n');
    details.push_str(paint.hint(if review.name_only {
        crate::messages::Message::TmSemanticNameOnly
    } else if review.same_semantics == Some(true) {
        crate::messages::Message::TmSemanticEqual
    } else if review.same_semantics.is_none() {
        crate::messages::Message::TmSemanticFresh
    } else {
        crate::messages::Message::TmSemanticChanged
    }));
    details.push('\n');
    if let Learning::Copy(_, (id, revision)) = learning {
        details.push_str(&paint.format(
            crate::messages::Template::TmSemanticTeacher,
            &[("id", &id.to_string()), ("revision", &revision.to_string())],
        ));
        details.push('\n');
    }
    if let Learning::Relearn(source) = learning {
        details.push_str(&paint.format(
            crate::messages::Template::TmSemanticReminder,
            &[
                ("id", &source.id.to_string()),
                ("revision", &source.revision.to_string()),
            ],
        ));
        details.push('\n');
    }
    if let Some(old) = &review.old {
        details.push_str(&semantic_changes(old, proposal, &review.changes, paint));
        details.push_str(&paint.format(
            crate::messages::Template::TmSemanticOld,
            &[
                ("id", &old.id.to_string()),
                ("revision", &old.revision.to_string()),
            ],
        ));
        details.push('\n');
        details.push_str(&proposal_details(&old.proposal, paint));
    }
    details.push_str(paint.hint(crate::messages::Message::TmSemanticNew));
    details.push('\n');
    details.push_str(&proposal_details(proposal, paint));
    if !review.variants.is_empty()
        && !proposal
            .extra
            .contains(crate::workspace::moves::parameters::KEY)
    {
        details.push_str(&crate::workspace::game::variant_review(&review.variants));
    }
    details.push('\n');
    details.push_str(paint.hint(crate::messages::Message::TmSemanticBoundary));
    if matches!(learning, Learning::Import(_)) {
        details.insert_str(
            0,
            &format!(
                "{}\n",
                paint.format(
                    crate::messages::Template::TmImportPartner,
                    &[("partner", partner)]
                )
            ),
        );
        if state.replacement.is_none() {
            let slot = paint.format(
                crate::messages::Template::TmFreeSlot,
                &[("slot", &(state.active().count() + 1).to_string())],
            );
            summary.push('\n');
            summary.push_str(&slot);
        }
        details.push('\n');
        details.push_str(paint.hint(crate::messages::Message::TmImportBoundary));
    }
    if let Some((id, revision)) = state.replacement
        && let Some(record) = review.old.as_ref()
    {
        summary.push_str(&paint.format(
            crate::messages::Template::DesksMovesReplacementSummary,
            &[
                ("scope", record.proposal.scope.label()),
                ("id", &id.to_string()),
            ],
        ));
        let slot = state.active().position(|r| r.id == id).map_or(0, |i| i + 1);
        details.push('\n');
        details.push_str(&paint.format(
            crate::messages::Template::TmSemanticSlot,
            &[
                ("slot", &slot.to_string()),
                ("id", &id.to_string()),
                ("revision", &revision.to_string()),
            ],
        ));
    } else if !matches!(learning, Learning::Rename(_)) {
        details.push('\n');
        details.push_str(&paint.format(
            crate::messages::Template::TmFreeSlot,
            &[("slot", &(state.active().count() + 1).to_string())],
        ));
    }
    (summary, details)
}
fn semantic_changes(
    old: &Record,
    proposed: &Proposal,
    fields: &[crate::workspace::moves::ChangedField],
    paint: &Paint,
) -> String {
    use crate::{
        messages::{Message, Template},
        workspace::moves::ChangedField,
    };
    let mut details = String::new();
    let literal = |value: Option<&String>| {
        value.map_or_else(
            || paint.hint(Message::TmSemanticAbsent).to_owned(),
            |value| format!("{value:?}"),
        )
    };
    for field in fields {
        let line = match (field, &old.proposal.steps, &proposed.steps) {
            (
                ChangedField::Arg(index),
                Steps::Run { argv: before, .. },
                Steps::Run { argv: after, .. },
            ) => paint.format(
                Template::TmSemanticArgument,
                &[
                    ("index", &index.to_string()),
                    ("old", &literal(before.get(*index))),
                    ("new", &literal(after.get(*index))),
                ],
            ),
            (ChangedField::Cwd, Steps::Run { cwd: before, .. }, Steps::Run { cwd: after, .. }) => {
                paint.format(
                    Template::TmSemanticCwd,
                    &[
                        ("old", &format!("{before:?}")),
                        ("new", &format!("{after:?}")),
                    ],
                )
            }
            (
                ChangedField::Criterion(index),
                Steps::Task {
                    criteria: before, ..
                },
                Steps::Task {
                    criteria: after, ..
                },
            ) => paint.format(
                Template::TmSemanticCriterion,
                &[
                    ("index", &index.to_string()),
                    ("old", &literal(before.get(*index))),
                    ("new", &literal(after.get(*index))),
                ],
            ),
            (ChangedField::Kind, before, after) => paint.format(
                Template::TmSemanticKind,
                &[("old", kind(before)), ("new", kind(after))],
            ),
            (ChangedField::Pins, ..) => paint.hint(Message::TmSemanticPins).into(),
            (ChangedField::Instruction, ..) => paint.hint(Message::TmSemanticInstruction).into(),
            (ChangedField::Parameters, ..) => {
                "Typed request slots/defaults/variants changed; inspect both exact templates below."
                    .into()
            }
            (ChangedField::Goal, ..) => paint.hint(Message::TmSemanticGoal).into(),
            _ => continue,
        };
        details.push_str(&line);
        details.push('\n');
    }
    details
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
                extra: Default::default(),
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
        state.partner = 1;
        state.form = 1;
        state.mode = Mode::Review;
        state.learning = Some(Learning::Teach(record.proposal.clone()));
        state.name = "Surf".into();
        state.scope = Scope::Personal;
        state.replacement = Some((1, 2));
        state.records.push(record);
        state.review = Some(state.prepare_review().unwrap());
        let (summary, details) = review_details(&state, "Bolt", paint);
        assert!(summary.contains("Bolt"));
        assert!(summary.contains("Forgets Project move #1"));
        assert!(details.contains("move #1, revision 2"));
    }
    #[test]
    fn same_name_tm_review_exposes_single_empty_argument_and_exact_local_slot() {
        let paint = &Paint::new(&config(), &FORMS[2], 1);
        let mut old = record();
        old.proposal.form = 2;
        old.proposal.steps = Steps::Run {
            argv: vec!["cargo".into(), "test".into(), "before".into()],
            cwd: ".".into(),
        };
        let mut definition =
            crate::workspace::moves::portable::Definition::from_proposal(&old.proposal).unwrap();
        if let Steps::Run { argv, .. } = &mut definition.steps {
            argv[2].clear();
        }
        let mut state = State::default();
        state.mode = Mode::Review;
        state.task_id = 3;
        state.partner = 1;
        state.form = 2;
        state.name = old.proposal.name.clone();
        state.scope = Scope::Personal;
        state.replacement = Some((old.id, old.revision));
        state.records.push(old.clone());
        state.learning = Some(Learning::Import(
            definition.proposal(3, 1, Scope::Personal).unwrap(),
        ));
        state.portable = Some(definition);
        state.review = Some(state.prepare_review().unwrap());
        let (_, details) = review_details(&state, "Exact local recipient", paint);
        assert!(details.contains("argv[2] (zero-based): \"before\" -> \"\""));
        assert!(details.contains("OLD / move #1, revision 2"));
        assert!(details.contains("PROPOSED"));
        assert!(details.contains("Exact local recipient / partner #1 / Jolteon"));
        assert!(details.contains("Scope: Personal"));
        assert!(details.contains("library slot 1 of 32 / move #1 revision 2"));
        assert!(!state.review.as_ref().unwrap().name_only);
        assert_eq!(
            state.records,
            vec![old],
            "inspection cannot change the original inventory"
        );
    }

    fn config() -> Config {
        Config {
            native_transport: crate::providers::NativeTransport::Auto,
            retry: Default::default(),
            launch_session: Default::default(),
            ascii: true,
            no_animation: true,
            animation_override: Some(false),
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
            assert!(buffer.contains(if width >= 90 { "> Q4 Surf4" } else { "> Surf4" }));
            assert!(buffer.contains("Moves"));
            assert!(buffer.is_ascii());
        }
    }
    #[test]
    fn xc07_variant_review_exposes_exact_differences_and_slot_values_at_both_sizes() {
        for (width, height) in [(120, 36), (60, 18)] {
            let mut app = App::new("project".into(), width, height);
            let mut proposal = record().proposal;
            proposal.form = 3;
            proposal.steps = Steps::Request {
                instruction: "Fix bug in src/b.rs".into(),
                pins: vec![],
            };
            app.moves.partner = 1;
            app.moves.form = 3;
            app.moves.mode = Mode::Review;
            app.moves.name = proposal.name.clone();
            app.moves.variants = vec![
                "fix the bug in src/a.rs".into(),
                "Fix bug in src/b.rs".into(),
            ];
            app.moves.learning = Some(Learning::Teach(proposal));
            app.moves.review = Some(app.moves.prepare_review().unwrap());
            let paint = Paint::new(&config(), &FORMS[3], 1);
            let (_, details) = review_details(&app.moves, "Partner", &paint);
            assert!(details.contains("Variant 1: \"fix the bug in src/a.rs\""));
            assert!(details.contains("Variant 2: \"Fix bug in src/b.rs\""));
            assert!(details.contains("path 1 = \"src/a.rs\""));
            assert!(details.contains("path 1 = \"src/b.rs\""));
            draw(&mut app, width, height);
            assert!(!app.moves.affirmative);
            if app.moves.scroll_max > 0 {
                assert!(!app.moves.reviewed);
            }
            while !app.moves.reviewed {
                app.moves.scroll = app
                    .moves
                    .scroll
                    .saturating_add(app.moves.scroll_page)
                    .min(app.moves.scroll_max);
                draw(&mut app, width, height);
            }
            app.moves.variants.push("fix the bug in src/c.rs".into());
            assert!(
                !app.moves.review_current(),
                "changed differences invalidate saved review"
            );
        }
    }

    #[test]
    fn rendered_review_requires_contiguous_full_scope_exposure_before_consent() {
        let mut app = App::new("project".into(), 60, 18);
        let record = record();
        app.moves.partner = 1;
        app.moves.form = 1;
        app.moves.mode = Mode::Review;
        app.moves.name = record.proposal.name.clone();
        app.moves.learning = Some(Learning::Teach(record.proposal));
        app.moves.review = Some(app.moves.prepare_review().unwrap());
        let buffer = draw(&mut app, 60, 18);
        assert!(!app.moves.reviewed);
        assert!(!app.moves.affirmative);
        assert!(app.moves.scroll_max > 0);
        assert!(buffer.contains("PgDn"));
        // End skips unseen content and must not arm consent.
        app.moves.scroll = app.moves.scroll_max;
        draw(&mut app, 60, 18);
        assert!(!app.moves.reviewed);
        app.moves.scroll = 0;
        draw(&mut app, 60, 18);
        while !app.moves.reviewed {
            app.moves.scroll = app
                .moves
                .scroll
                .saturating_add(app.moves.scroll_page)
                .min(app.moves.scroll_max);
            draw(&mut app, 60, 18);
        }
        assert!(!app.moves.affirmative);
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
    #[test]
    fn xc07_parameter_review_requires_full_exposure_and_revokes_stale_values() {
        for (width, height) in [(120, 36), (60, 18)] {
            let mut app = App::new("project".into(), width, height);
            let mut record = record();
            record.proposal.form = 3;
            record.proposal.steps = Steps::Request {
                instruction: "Fix bug in src/b.rs".into(),
                pins: vec![],
            };
            let template = crate::workspace::moves::parameters::Template::derive(
                &record.proposal.steps,
                vec![
                    "fix the bug in src/a.rs".into(),
                    "Fix bug in src/b.rs".into(),
                ],
            )
            .unwrap()
            .unwrap();
            template.store(&mut record.proposal).unwrap();
            app.moves.partner = 1;
            app.moves.form = 3;
            app.moves.records.push(record.clone());
            app.moves.parameter_record = Some(record.clone());
            app.moves.name = "[\"src/c.rs\"]".into();
            app.moves.parameter_values.clone_from(&app.moves.name);
            let (instruction, details) = template
                .review(&record.proposal.steps, &app.moves.name)
                .unwrap();
            app.moves.parameter_instruction = Some(instruction);
            app.moves.parameter_review = details;
            app.moves.mode = Mode::ParameterReview;
            draw(&mut app, width, height);
            assert!(!app.moves.affirmative);
            if app.moves.scroll_max > 0 {
                assert!(!app.moves.reviewed);
            }
            if app.moves.scroll_max as usize > app.moves.exposed_rows {
                app.moves.scroll = app.moves.scroll_max;
                draw(&mut app, width, height);
                assert!(
                    !app.moves.reviewed,
                    "skipping hidden slot details cannot arm consent"
                );
                app.moves.scroll = 0;
                draw(&mut app, width, height);
            }
            while !app.moves.reviewed {
                app.moves.scroll = app
                    .moves
                    .scroll
                    .saturating_add(app.moves.scroll_page)
                    .min(app.moves.scroll_max);
                draw(&mut app, width, height);
            }
            app.moves.name = "[\"src/d.rs\"]".into();
            draw(&mut app, width, height);
            assert!(!app.moves.reviewed && !app.moves.affirmative);
        }
    }
}
