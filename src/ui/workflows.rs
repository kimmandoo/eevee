use super::{Paint, centered, clipped, editor_rows, panel, row, selection_details, text};
use crate::{
    app::{App, workflows::Tab},
    config::Config,
};
use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Clear, Paragraph, Wrap},
};

pub(super) fn render(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    if area.width < 60 || area.height < 18 {
        frame.render_widget(Clear, area);
        app.workflow.reviewed = false;
        app.workflow.authoring.invalidate();
        if let Some(relay) = &mut app.workflow.relay {
            relay.reviewed = false;
        }
        app.workflow.affirmative = false;
        text(
            frame,
            area,
            paint.hint(if app.workflow.authoring.active {
                crate::i18n::Message::RecipeAuthoringResize
            } else {
                crate::i18n::Message::DesksWorkflowsSpecialistDeskNeedXDraftRetainedResize
            }),
            paint.accent,
        );
        return;
    }
    if app.workflow.authoring.active {
        render_authoring(frame, area, app, config, paint);
        return;
    }
    // Keep project/storage status and the global safe-exit footer visible.
    let area = row(area, 2, area.height.saturating_sub(4));
    let compact = app.workflow.preview
        && !app.workflow.criteria_preview
        && app.workflow.editor.is_none()
        && !super::VERBOSE.get();
    let height = if compact {
        18.min(area.height)
    } else {
        area.height
    };
    let area = centered(area, 116, height);
    frame.render_widget(Clear, area);
    let block = panel(
        paint.hint(crate::i18n::Message::DesksWorkflowsPanel),
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let tabs = match app.workflow.tab {
        Tab::Diagnosis => paint.hint(crate::i18n::Message::DesksWorkflowsDiagnosisTab),
        Tab::Refactor => paint.hint(crate::i18n::Message::DesksWorkflowsRefactorTab),
        Tab::Review => paint.hint(crate::i18n::Message::DesksWorkflowsReviewTab),
        Tab::Recipe => paint.hint(crate::i18n::Message::DesksWorkflowsRecipeTab),
    };
    text(frame, row(content, 0, 1), tabs, paint.accent);
    let notice = if app.workflow.notice.is_empty() {
        paint.hint(crate::i18n::Message::DesksWorkflowsLocalNotesRealEvidenceNoAutomaticCommand)
    } else {
        &app.workflow.notice
    };
    text(
        frame,
        row(content, 1, 2),
        &super::brief(notice),
        paint.muted,
    );
    let body = row(content, 3, content.height.saturating_sub(5));
    let footer = row(content, content.height.saturating_sub(2), 2);
    app.workflow.scroll_page = body.height.max(1);
    if app.workflow.relay.is_some() {
        let targets = app.relay_targets();
        let pending = app.workflow.relay.as_ref().unwrap();
        let selected = pending.selected.min(targets.len().saturating_sub(1));
        let confirm = pending.confirm;
        let details = if confirm {
            paint.format(crate::i18n::Template::DesksWorkflowsRelayPreview, &[
    ("target", (targets
                .iter()
                .find(|t| Some(t.0) == pending.target)
                .map_or(paint.hint(crate::i18n::Message::DesksWorkflowsChosenTargetNoLongerIdle), |t| t.1.as_str()))),
    ("paths", &format!("{:?}", pending.request.paths)),
    ("source", &format!("{:?}", pending.request.source)),
    ("replacement", (if pending.request.command.is_some() {
                paint.hint(crate::i18n::Message::DesksWorkflowsCommandDraftReplacementIsIncludedInThis)
            } else {
                ""
            })),
    ("request", &(pending.request.text)),
])
        } else {
            targets
                .iter()
                .enumerate()
                .map(|(i, (_, label))| format!("{} {label}", if i == selected { ">" } else { " " }))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let max = selection_details(
            frame,
            body,
            Paragraph::new(details)
                .style(paint.surface)
                .wrap(Wrap { trim: false }),
            app.workflow.scroll,
        );
        app.workflow.scroll_max = max;
        app.workflow.scroll = app.workflow.scroll.min(max);
        if let Some(pending) = &mut app.workflow.relay {
            pending.reviewed = confirm && body.height >= 3;
        }
        text(
            frame,
            footer,
            if confirm {
                paint.hint(crate::i18n::Message::DesksWorkflowsStayYStageEscCancelPgupPgdn)
            } else {
                paint.hint(crate::i18n::Message::DesksWorkflowsUpDownTargetEnterReviewEscCancel)
            },
            paint.accent,
        );
        return;
    }
    if let Some(field) = app.workflow.editor {
        text(
            frame,
            row(body, 0, 1),
            paint.hint(match field {
                crate::app::workflows::Field::Hypothesis => {
                    crate::i18n::Message::DesksWorkflowsFieldHypothesis
                }
                crate::app::workflows::Field::Contract => {
                    crate::i18n::Message::DesksWorkflowsFieldContract
                }
                crate::app::workflows::Field::Finding => {
                    crate::i18n::Message::DesksWorkflowsFieldFinding
                }
                crate::app::workflows::Field::Action => {
                    crate::i18n::Message::DesksWorkflowsFieldAction
                }
                crate::app::workflows::Field::RecipePath => {
                    crate::i18n::Message::DesksWorkflowsFieldRecipePath
                }
                crate::app::workflows::Field::Criteria => {
                    crate::i18n::Message::DesksWorkflowsFieldCriteria
                }
            }),
            paint.accent,
        );
        let input = row(body, 1, body.height.saturating_sub(2));
        let (lines, (caret_row, caret_column)) = editor_rows(
            app.workflow.value(field),
            app.workflow.cursor,
            input.width.saturating_sub(1) as usize,
        );
        let top = caret_row.saturating_sub(input.height.saturating_sub(1) as usize);
        frame.render_widget(
            Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
                .style(paint.surface)
                .scroll((top.min(u16::MAX as usize) as u16, 0)),
            input,
        );
        if !app.workflow.editor_done && input.width > 0 && input.height > 0 {
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
            row(body, body.height.saturating_sub(1), 1),
            if app.workflow.editor_done {
                paint.hint(crate::i18n::Message::DesksWorkflowsDoneEditingSelected)
            } else {
                paint.hint(crate::i18n::Message::DesksWorkflowsDoneEditing)
            },
            if app.workflow.editor_done {
                paint.accent
            } else {
                paint.muted
            },
        );
        text(
            frame,
            footer,
            paint.hint(crate::i18n::Message::DesksWorkflowsCtrlSDoneTabControlEnterNewline),
            paint.accent,
        );
        return;
    }
    if app.workflow.preview {
        let effect = if app.workflow.criteria_preview {
            paint
                .hint(crate::i18n::Message::DesksWorkflowsConfirmDecisionLinkedCriteriaOnlyNewItems)
        } else {
            paint.hint(crate::i18n::Message::DesksWorkflowsStagePlanCriteriaAndNextActionOnly)
        };
        let preview = if app.workflow.criteria_preview {
            app.workflow_criteria_preview()
        } else if super::VERBOSE.get() {
            format!("{effect}\n\n{}", app.workflow_recipe_preview())
        } else {
            app.workflow.recipe.as_ref().map_or_else(
                || {
                    paint
                        .hint(crate::i18n::Message::DesksWorkflowsNoRecipeLoaded)
                        .into()
                },
                |recipe| {
                    paint.format(
                        crate::i18n::Template::DesksWorkflowsRecipeSummary,
                        &[
                            ("effect", (effect)),
                            ("name", &(clipped(&recipe.name, 80))),
                            ("steps", &(recipe.steps.len()).to_string()),
                        ],
                    )
                },
            )
        };
        if compact {
            app.workflow.scroll = 0;
        }
        let max = selection_details(
            frame,
            body,
            Paragraph::new(preview)
                .style(paint.surface)
                .wrap(Wrap { trim: false }),
            app.workflow.scroll,
        );
        app.workflow.scroll_max = max;
        app.workflow.scroll = app.workflow.scroll.min(max);
        if app.workflow.recipe.is_some() || app.workflow.criteria_preview {
            app.workflow.reviewed =
                body.height >= 3 && (app.workflow.criteria_preview || !compact || max == 0);
        }
        let footer_text = paint.format(
            crate::i18n::Template::DesksWorkflowsApplyControls,
            &[
                (
                    "stay",
                    (if app.workflow.affirmative {
                        paint.hint(crate::i18n::Message::CommonStay)
                    } else {
                        paint.hint(crate::i18n::Message::CommonStaySelected)
                    }),
                ),
                (
                    "apply",
                    (if app.workflow.affirmative {
                        paint.hint(crate::i18n::Message::DesksWorkflowsApplyPlanOnlySelected)
                    } else {
                        paint.hint(crate::i18n::Message::DesksWorkflowsApplyPlanOnly)
                    }),
                ),
                (
                    "details",
                    (if app.workflow.reviewed {
                        paint.hint(crate::i18n::Message::DesksHandoffVerboseDetails)
                    } else {
                        paint.hint(crate::i18n::Message::CommonEnlargeReview)
                    }),
                ),
            ],
        );
        text(frame, footer, &footer_text, paint.accent);
        return;
    }
    let rows = app.workflow_rows(paint.language);
    let selected = if app.workflow.chooser.is_some() {
        app.workflow.evidence
    } else {
        app.workflow.selected
    }
    .min(rows.len().saturating_sub(1));
    let details = app.workflow_details();
    let wide = body.width >= 96;
    let (list, detail) = if wide {
        let width = body.width / 2;
        (
            Rect { width, ..body },
            Rect {
                x: body.x + width + 2,
                width: body.width.saturating_sub(width + 2),
                ..body
            },
        )
    } else {
        let height = body.height.saturating_sub(3).max(1);
        (
            Rect { height, ..body },
            row(body, height, body.height.saturating_sub(height)),
        )
    };
    app.workflow.scroll_page = detail.height.max(1);
    let top = selected.saturating_sub(list.height.saturating_sub(1) as usize);
    for (offset, label) in rows.iter().skip(top).take(list.height as usize).enumerate() {
        let active = top + offset == selected;
        let label = format!("{} {}", if active { ">" } else { " " }, label);
        text(
            frame,
            row(list, offset as u16, 1),
            &clipped(&label, list.width as usize),
            if active { paint.accent } else { paint.surface },
        );
    }
    let max = selection_details(
        frame,
        detail,
        Paragraph::new(details)
            .style(paint.muted)
            .wrap(Wrap { trim: false }),
        app.workflow.scroll,
    );
    app.workflow.scroll_max = max;
    app.workflow.scroll = app.workflow.scroll.min(max);
    text(
        frame,
        footer,
        if app.workflow.statuses.iter().any(|(reference, status)| {
            matches!(
                reference,
                crate::workspace::workflows::EvidenceRef::Run { .. }
            ) && *status == crate::workspace::workflows::EvidenceStatus::Stale
        }) {
            paint.hint(crate::i18n::Message::DesksWorkflowsRerunRerunWithJolteonCommandsUpDown)
        } else if app.workflow.chooser.is_some() {
            paint.hint(crate::i18n::Message::DesksWorkflowsUpDownEvidenceEnterAttachOpenEvidence)
        } else {
            paint.hint(crate::i18n::Message::DesksWorkflowsCommandsLeftRightTabsUpDownActions)
        },
        paint.accent,
    );
}

fn render_authoring(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    use crate::app::workflows::authoring::{Field, Mode};
    use crate::i18n::Message;
    let review_visible = app.command_popup.is_none()
        && app.confirmation.is_none()
        && app.nickname_editor.is_none()
        && app.composer_menu.is_none()
        && app.cooperation_ui.release.is_none()
        && app.cooperation.cancel_review.is_none()
        && app.cooperation.journal_discard.is_none();
    let area = centered(
        row(area, 2, area.height.saturating_sub(4)),
        116,
        area.height.saturating_sub(4),
    );
    frame.render_widget(Clear, area);
    let block = panel(
        paint.hint(Message::RecipeAuthoringTitle),
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let state = &mut app.workflow.authoring;
    let notice = if !state.error.is_empty() {
        state.error.as_str()
    } else {
        paint.hint(state.notice.unwrap_or(Message::RecipeAuthoringPolicy))
    };
    text(
        frame,
        row(content, 0, 2),
        notice,
        if state.error.is_empty() {
            paint.muted
        } else {
            paint.accent
        },
    );
    let body = row(content, 2, content.height.saturating_sub(4));
    let footer = row(content, content.height.saturating_sub(2), 2);
    state.page = body.height.max(1);
    if let Some(field) = state.field {
        text(
            frame,
            row(body, 0, 2),
            paint.hint(field.title()),
            paint.accent,
        );
        let input = row(body, 2, body.height.saturating_sub(3));
        let (lines, (caret_row, caret_column)) = editor_rows(
            state.value(field),
            state.cursor,
            input.width.saturating_sub(1) as usize,
        );
        let top = caret_row.saturating_sub(input.height.saturating_sub(1) as usize);
        frame.render_widget(
            Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
                .style(paint.surface)
                .scroll((top.min(u16::MAX as usize) as u16, 0)),
            input,
        );
        if !state.done && input.width > 0 && input.height > 0 {
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
            row(body, body.height.saturating_sub(1), 1),
            paint.hint(Message::RecipeAuthoringDone),
            if state.done {
                paint.accent
            } else {
                paint.muted
            },
        );
        text(
            frame,
            footer,
            paint.hint(Message::RecipeAuthoringEditorHint),
            paint.accent,
        );
        return;
    }
    match state.mode {
        Mode::Menu => {
            let fields = [
                Field::Name,
                Field::Scope,
                Field::Criteria,
                Field::Steps,
                Field::Path,
            ];
            let mut rows: Vec<String> = fields
                .iter()
                .map(|field| {
                    let value = state.value(*field);
                    format!(
                        "{}: {}",
                        paint.hint(field.title()),
                        if value.is_empty() {
                            paint.hint(Message::RecipeAuthoringEmpty).into()
                        } else {
                            value.replace('\n', " ")
                        }
                    )
                })
                .collect();
            rows.push(format!(
                "{} ({})",
                paint.hint(Message::RecipeAuthoringCommands),
                state.draft.runs.len()
            ));
            rows.push(paint.hint(Message::RecipeAuthoringPreview).into());
            let selected = state.selected.min(rows.len() - 1);
            // Keep keyboard focus and its complete value reachable at the minimum size.
            let top = selected.saturating_sub(body.height.saturating_sub(1) as usize);
            for (index, label) in rows.iter().enumerate().skip(top).take(body.height as usize) {
                text(
                    frame,
                    row(body, (index - top) as u16, 1),
                    &format!("{} {label}", if index == selected { ">" } else { " " }),
                    if index == selected {
                        paint.accent
                    } else {
                        paint.muted
                    },
                );
            }
            text(
                frame,
                footer,
                paint.hint(Message::RecipeAuthoringMenuHint),
                paint.accent,
            );
        }
        Mode::Commands => {
            let runs: Vec<_> = app
                .execution
                .history
                .iter()
                .filter(|run| run.prepared.task_id == app.task_id)
                .collect();
            let details = if let Some(run) = runs.get(state.selected) {
                let eligibility = crate::workspace::workflows::authoring::command(
                    std::path::Path::new(&app.cwd),
                    app.task_id,
                    run,
                );
                let status = match &eligibility {
                    Ok(_) => paint.hint(Message::RecipeAuthoringEligible).to_string(),
                    Err(error) => format!(
                        "{}: {error}",
                        paint.hint(Message::RecipeAuthoringUnavailable)
                    ),
                };
                let mut details = format!(
                    "{}/{}  {}  {}\n{}\n",
                    state.selected + 1,
                    runs.len(),
                    run.prepared.run_id,
                    if state.draft.runs.contains(&run.prepared.run_id) {
                        paint.hint(Message::RecipeAuthoringSelected)
                    } else {
                        ""
                    },
                    status
                );
                details.push_str(&format!("{:?}\ncwd: {}\n{:?} / exit {:?} / {:?}\nstart {:?} / end {:?}\nsource before {:?}\nsource after {:?}\n",
                    run.prepared.spec, run.prepared.cwd.display(), run.state, run.exit_code, run.tree,
                    run.started_at_ms, run.ended_at_ms, run.prepared.before, run.after));
                details
            } else {
                paint.hint(Message::RecipeAuthoringNoRuns).into()
            };
            let max = selection_details(
                frame,
                body,
                Paragraph::new(details)
                    .style(paint.surface)
                    .wrap(Wrap { trim: false }),
                state.scroll,
            );
            state.scroll_max = max;
            state.scroll = state.scroll.min(max);
            text(
                frame,
                footer,
                paint.hint(Message::RecipeAuthoringCommandHint),
                paint.accent,
            );
        }
        Mode::Review => {
            let details = state
                .export
                .as_ref()
                .map(|export| {
                    let mut details = format!(
                        "{}: {}\n\n{}\n",
                        paint.hint(Message::RecipeAuthoringOutput),
                        export.path().display(),
                        paint.hint(Message::RecipeAuthoringEvidence)
                    );
                    for run in export.runs() {
                        details.push_str(&format!(
                            "{} / exit {:?} / {:?} / {:?}–{:?}\nsource {:?}\n",
                            run.prepared.run_id,
                            run.exit_code,
                            run.tree,
                            run.started_at_ms,
                            run.ended_at_ms,
                            run.after.fingerprint
                        ));
                    }
                    details.push_str(&format!(
                        "\n{}\n\n{}",
                        paint.hint(Message::RecipeAuthoringDefinition),
                        export.bytes()
                    ));
                    details
                })
                .unwrap_or_default();
            let max = selection_details(
                frame,
                body,
                Paragraph::new(details)
                    .style(paint.surface)
                    .wrap(Wrap { trim: false }),
                state.scroll,
            );
            state.scroll_max = max;
            state.scroll = state.scroll.min(max);
            state.reviewed =
                review_visible && body.height >= 3 && state.scroll == max && state.export.is_some();
            text(
                frame,
                footer,
                paint.hint(Message::RecipeAuthoringReviewHint),
                paint.accent,
            );
        }
    }
}
