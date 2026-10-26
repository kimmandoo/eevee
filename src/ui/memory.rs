use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Clear, Paragraph, Wrap},
};

use super::{Paint, clipped, editor_rows, panel, row, selection_details, text};
use crate::{
    app::memory::{Mode, State},
    config::Config,
    workspace::memory::{self, Proposal},
};

pub(super) fn render_small(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    state.invalidate_review();
    frame.render_widget(Clear, area);
    text(
        frame,
        area,
        paint
            .hint(crate::messages::Message::DesksMemoryApprovedMemoriesTerminalTooSmallNeedColumns),
        paint.accent,
    );
}

pub(super) fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut State,
    config: &Config,
    paint: &Paint,
) {
    if area.width < 60 || area.height < 18 {
        render_small(frame, area, state, paint);
        return;
    }
    let compact = matches!(state.mode, Mode::Preview | Mode::Delete) && !super::VERBOSE.get();
    let source_preview = state.mode == Mode::Preview
        && state.candidate.as_ref().is_some_and(|candidate| {
            candidate.provenance.kind == memory::SourceKind::UserAuthored
                && candidate
                    .provenance
                    .reference
                    .strip_prefix("task:")
                    .and_then(|reference| reference.split_once("/source:"))
                    .is_some_and(|(task, _)| {
                        task.parse::<i64>().ok() == Some(candidate.provenance.task_id)
                    })
        });
    let height = if compact {
        18
    } else {
        area.height.saturating_sub(2)
    };
    let area = super::sheet(area, 110, height);
    frame.render_widget(Clear, area);
    let block = panel(
        paint.hint(crate::messages::Message::DesksMemoryPanel),
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let heading = if state.saving {
        paint.hint(crate::messages::Message::DesksMemorySavingAwaitingCommittedAcknowledgement)
    } else {
        match state.mode {
            Mode::Browse => paint.hint(
                crate::messages::Message::DesksMemoryInspectOnlySelectedApprovedNotesEnterContext,
            ),
            Mode::Edit => {
                paint.hint(crate::messages::Message::DesksMemoryUserNoteEnterPreviewsNeverApproves)
            }
            Mode::Preview => {
                paint.hint(crate::messages::Message::DesksMemoryApprovalPreviewNoMemorySavedYet)
            }
            Mode::Delete => {
                paint.hint(crate::messages::Message::DesksMemoryLocalDeletionPreviewNoDeletionYet)
            }
            Mode::Discard => {
                paint.hint(crate::messages::Message::DesksMemoryUnsavedDraftKeepIsSelectedInitially)
            }
        }
    };
    text(frame, row(content, 0, 1), heading, paint.accent);
    // Adopted notes need the narrow review body for retention and consent.
    // Their summary already shows scope; omit the duplicate label and notice.
    if !compact || !source_preview {
        text(
            frame,
            row(content, 1, 2),
            &super::brief(&state.notice),
            paint.muted,
        );
        let label = match state.mode {
            Mode::Edit | Mode::Preview | Mode::Discard => state
                .candidate
                .as_ref()
                .map_or("No proposal", |candidate| candidate.scope.label())
                .to_owned(),
            _ => format!(
                "Memory {}/{} | {} selected for future requests",
                if state.records.is_empty() {
                    0
                } else {
                    state.selected + 1
                },
                state.records.len(),
                state.selected_ids.len()
            ),
        };
        text(frame, row(content, 3, 1), &label, paint.accent);
    }
    let footer_rows = if compact { 2 } else { 3 };
    let body_start = if compact && source_preview { 1 } else { 4 };
    let mut body = row(
        content,
        body_start,
        content.height.saturating_sub(body_start + footer_rows),
    );
    if state.editing() {
        editor(frame, body, state, paint);
    } else {
        if area.width >= 95
            && state.mode == Mode::Browse
            && !state.show_help
            && !state.records.is_empty()
        {
            let list = Rect {
                width: 29.min(body.width),
                ..body
            };
            let top = state
                .selected
                .saturating_sub(list.height.saturating_sub(1) as usize);
            for (offset, record) in state
                .records
                .iter()
                .enumerate()
                .skip(top)
                .take(list.height as usize)
            {
                let marker = if state.selected_ids.contains(&record.id) {
                    "+"
                } else {
                    "-"
                };
                let value = format!(
                    "{marker} {} {}",
                    match record.proposal.scope {
                        memory::Scope::Task => "Task",
                        memory::Scope::Project => "Project",
                        memory::Scope::Personal => "Personal",
                    },
                    record.proposal.content.lines().next().unwrap_or("")
                );
                text(
                    frame,
                    row(list, (offset - top) as u16, 1),
                    &clipped(&value, list.width as usize),
                    if offset == state.selected {
                        paint.selected
                    } else {
                        paint.surface
                    },
                );
            }
            body.x = body.x.saturating_add(31);
            body.width = body.width.saturating_sub(31);
        }
        let mut value = match state.mode {
            Mode::Browse if state.show_help => paint
                .hint(crate::messages::Message::DesksMemoryCreateMemoryNewNoteRememberNextAction)
                .into(),
            Mode::Preview => state
                .candidate
                .as_ref()
                .map_or_else(String::new, |candidate| {
                    if super::VERBOSE.get() {
                        describe(candidate, &state.project, state.task_id, paint)
                    } else {
                        summary(
                            candidate,
                            false,
                            paint,
                            source_preview.then_some(body.width as usize),
                        )
                    }
                }),
            Mode::Discard => paint.format(
                crate::messages::Template::DesksMemoryDiscardPreview,
                &[("draft", &(state.draft))],
            ),
            Mode::Browse | Mode::Delete => {
                if let Some(record) = state.selected_record() {
                    if state.mode == Mode::Delete && !super::VERBOSE.get() {
                        format!(
                            "Memory #{} / revision {}\n{}",
                            record.id,
                            record.revision,
                            summary(&record.proposal, true, paint, None)
                        )
                    } else {
                        format!(
                            "Approved memory #{} / revision {}\n{}\n\n{}\n\n{}",
                            record.id,
                            record.revision,
                            if state.selected_ids.contains(&record.id) {
                                paint.hint(crate::messages::Message::DesksMemorySelectedForFutureRequestPreviews)
                            } else {
                                paint.hint(
                                    crate::messages::Message::DesksMemoryNotSelectedForOutgoingContext,
                                )
                            },
                            describe(
                                &record.proposal,
                                &record.project,
                                record.task_id.unwrap_or(state.task_id),
                                paint
                            ),
                            paint.hint(crate::messages::Message::DesksMemoryErasureNotice)
                        )
                    }
                } else {
                    paint
                        .hint(crate::messages::Message::DesksMemoryNoApprovedMemoriesInThisScopeCreate)
                        .into()
                }
            }
            Mode::Edit => unreachable!(),
        };
        if source_preview {
            value.insert_str(0, "\n\n");
            value.insert_str(
                0,
                paint.hint(crate::messages::Message::SourceMemoryRetention),
            );
        }
        if state.mode == Mode::Preview && !state.habit_sources.is_empty() {
            value.push_str("\n\n");
            value.push_str(&paint.format(
                crate::messages::Template::RecordsHabitMemoryDisclosure,
                &[("sources", &state.habit_sources.join(", "))],
            ));
        }
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
        if matches!(state.mode, Mode::Preview | Mode::Delete) {
            state.reviewed = body.height >= 4 && !state.saving && (!compact || max == 0);
        }
    }
    let footer = if state.saving {
        paint
            .hint(crate::messages::Message::DesksMemoryWaitingForStorageDraftRetainedOnFailure)
            .into()
    } else {
        match state.mode {
            Mode::Browse => format!(
                "{}\n{}",
                paint.hint(
                    crate::messages::Message::DesksMemoryUpDownInspectCommandsSelectMemoryContext
                ),
                paint.hint(crate::messages::Message::HeldMemoryHint)
            ),
            Mode::Edit => paint
                .hint(crate::messages::Message::DesksMemoryEnterPreviewCtrlNNewlineEscKeep)
                .into(),
            Mode::Preview => paint.format(
                crate::messages::Template::DesksMemoryApproveControls,
                &[
                    (
                        "stay",
                        (if state.affirmative {
                            paint.hint(crate::messages::Message::CommonStay)
                        } else {
                            paint.hint(crate::messages::Message::CommonStaySelected)
                        }),
                    ),
                    (
                        "approve",
                        (if state.affirmative {
                            paint.hint(crate::messages::Message::CommonApproveSelected)
                        } else {
                            paint.hint(crate::messages::Message::CommonApprove)
                        }),
                    ),
                    (
                        "details",
                        (if state.reviewed {
                            paint.hint(crate::messages::Message::DesksMemoryVerboseFullDetails)
                        } else {
                            paint.hint(crate::messages::Message::CommonEnlargeReview)
                        }),
                    ),
                ],
            ),
            Mode::Delete => paint.format(
                crate::messages::Template::DesksMemoryDeleteControls,
                &[
                    (
                        "stay",
                        (if state.affirmative {
                            paint.hint(crate::messages::Message::CommonStay)
                        } else {
                            paint.hint(crate::messages::Message::CommonStaySelected)
                        }),
                    ),
                    (
                        "delete",
                        (if state.affirmative {
                            paint.hint(crate::messages::Message::DesksMemoryDeleteSelected)
                        } else {
                            paint.hint(crate::messages::Message::DesksMemoryDelete)
                        }),
                    ),
                    (
                        "details",
                        (if state.reviewed {
                            paint.hint(crate::messages::Message::DesksMemoryVerboseFullDetails)
                        } else {
                            paint.hint(crate::messages::Message::CommonEnlargeReview)
                        }),
                    ),
                ],
            ),
            Mode::Discard => paint.format(
                crate::messages::Template::DesksMemoryDiscardControls,
                &[
                    (
                        "keep",
                        (if state.affirmative {
                            paint.hint(crate::messages::Message::DesksMemoryKeepEditing)
                        } else {
                            paint.hint(crate::messages::Message::DesksMemoryKeepEditingSelected)
                        }),
                    ),
                    (
                        "discard",
                        (if state.affirmative {
                            paint.hint(crate::messages::Message::DesksMemoryDiscardDraftSelected)
                        } else {
                            paint.hint(crate::messages::Message::DesksMemoryDiscardDraft)
                        }),
                    ),
                ],
            ),
        }
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

fn summary(
    proposal: &Proposal,
    deleting: bool,
    paint: &Paint,
    source_columns: Option<usize>,
) -> String {
    let scope = match proposal.scope {
        memory::Scope::Task => paint.hint(crate::messages::Message::DesksMemoryThisTask),
        memory::Scope::Project => paint.hint(crate::messages::Message::DesksMemoryThisProject),
        memory::Scope::Personal => paint.hint(crate::messages::Message::DesksMemoryAllProjects),
    };
    paint.format(
        if source_columns.is_some() {
            crate::messages::Template::SourceMemorySummary
        } else {
            crate::messages::Template::DesksMemorySummary
        },
        &[
            ("scope", (scope)),
            (
                "effect",
                (if deleting {
                    paint.hint(crate::messages::Message::DesksMemoryDeleteLocallyPriorExportsRemain)
                } else {
                    paint.hint(
                        crate::messages::Message::DesksMemorySaveNoteOnlyContextSelectionIsSeparate,
                    )
                }),
            ),
            (
                "content",
                &(clipped(
                    proposal.content.lines().next().unwrap_or(""),
                    source_columns.map_or(100, |columns| columns.saturating_sub(6)),
                )),
            ),
            ("bytes", &(proposal.content.len()).to_string()),
        ],
    )
}

fn describe(proposal: &Proposal, project: &str, task_id: i64, paint: &Paint) -> String {
    let owner = match proposal.scope {
        memory::Scope::Task => format!("Task {task_id} in {project}"),
        memory::Scope::Project => format!("Project: {project}"),
        memory::Scope::Personal => paint
            .hint(crate::messages::Message::DesksMemoryPersonalEveryProjectUsingThisDataStore)
            .into(),
    };
    paint.format(
        crate::messages::Template::DesksMemoryDetails,
        &[
            ("scope", (proposal.scope.label())),
            ("bytes", &(proposal.content.len()).to_string()),
            ("content", &(proposal.content)),
            ("owner", &(owner)),
            ("provenance", (proposal.provenance.kind.label())),
            ("source", &(proposal.provenance.reference)),
            ("task", &(proposal.provenance.task_id).to_string()),
        ],
    )
}

fn editor(frame: &mut Frame, area: Rect, state: &State, paint: &Paint) {
    let (lines, (caret_row, caret_column)) = editor_rows(
        &state.draft,
        state.cursor,
        area.width.saturating_sub(1) as usize,
    );
    let top = caret_row.saturating_sub(area.height.saturating_sub(1) as usize);
    frame.render_widget(
        Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
            .style(paint.surface)
            .scroll((top.min(u16::MAX as usize) as u16, 0)),
        area,
    );
    if area.width > 0 && area.height > 0 {
        frame.set_cursor_position((
            area.x
                .saturating_add((caret_column as u16).min(area.width.saturating_sub(1))),
            area.y.saturating_add(
                (caret_row.saturating_sub(top) as u16).min(area.height.saturating_sub(1)),
            ),
        ));
    }
}
