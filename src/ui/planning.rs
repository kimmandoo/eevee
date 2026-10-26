use std::fmt::Write;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;

use super::{Paint, centered, clipped, editor_rows, panel, row, selection_details, text};
use crate::{
    app::{
        App,
        planning::{Picker, State, file_phase_message},
    },
    config::Config,
    messages::{Message, Template},
    providers::{FilePhase, FileReview, FileReviewKind, ItemKind, Operation, PromptKind},
};
/// Response-only home surface: reuse the cached layout and follow state.
pub(super) fn render_workspace(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    _config: &Config,
    paint: &Paint,
) {
    let (request, previous) = app.planning.response_request();
    let area = if previous && !app.planning.submitted_request.is_empty() {
        let current = app
            .planning
            .submitted_request
            .replace(['\n', '\r', '\t'], " ");
        let current = crate::workspace::privacy::redact(&current);
        text(
            frame,
            row(area, 0, 1),
            &clipped(
                &paint.format(
                    Template::ProvidersPlanningCurrentRequestCurrent,
                    &[("current", &(current))],
                ),
                area.width as usize,
            ),
            paint.muted,
        );
        row(area, 1, area.height.saturating_sub(1))
    } else {
        area
    };
    let response_area = if request.is_empty() {
        area
    } else {
        let request = request.replace(['\n', '\r', '\t'], " ");
        let request = crate::workspace::privacy::redact(&request);
        text(
            frame,
            row(area, 0, 1),
            &clipped(
                &paint.format(
                    Template::ProvidersPlanningRequest2,
                    &[
                        (
                            "label",
                            (if previous {
                                paint.hint(Message::ProvidersPlanningPreviousRequest)
                            } else {
                                paint.hint(Message::ProvidersPlanningRequest)
                            }),
                        ),
                        ("request", &(request)),
                    ],
                ),
                area.width as usize,
            ),
            paint.muted,
        );
        row(area, 1, area.height.saturating_sub(1))
    };
    response(frame, response_area, &mut app.planning, paint);
}

pub(super) fn render_small(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    state.set_response_body(Rect::default());
    if !state.quit_review {
        state.invalidate_review();
    }
    frame.render_widget(Clear, area);
    let value = if state.quit_review {
        paint.format(
            Template::ProvidersPlanningExitPlanningTabChooseEnterEscStay,
            &[
                (
                    "status",
                    (if state.outcome_unknown() {
                        paint.hint(Message::ProvidersPlanningUnknownOutcomeWorkMayHaveRunEffects)
                    } else if state.cancelling {
                        paint.hint(Message::ProvidersPlanningWaitingForCancellation)
                    } else {
                        paint.hint(Message::ProvidersPlanningCancelOwnedWorkBeforeExit)
                    }),
                ),
                (
                    "choices",
                    (if state.outcome_unknown() {
                        if state.affirmative {
                            paint.hint(Message::ProvidersPlanningStayExitWithUnknownOutcome)
                        } else {
                            paint.hint(Message::ProvidersPlanningStayExitWithUnknownOutcome2)
                        }
                    } else if state.affirmative {
                        paint.hint(Message::ProvidersPlanningStayCancelAndExit)
                    } else {
                        paint.hint(Message::ProvidersPlanningStayCancelAndExit2)
                    }),
                ),
            ],
        )
    } else {
        paint
            .hint(Message::ProvidersPlanningPlanningTerminalTooSmallNeed60Columns)
            .into()
    };
    text(frame, area, &value, paint.accent);
}

pub(super) fn render(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    app.planning.set_response_body(Rect::default());
    let approval_header = app.approval_position().map(|(position, total)| {
        paint.format(
            Template::ProvidersPlanningPositionOfTotal,
            &[
                ("position", &(position).to_string()),
                ("total", &(total).to_string()),
                (
                    "owner",
                    &(crate::store::Attribution::label(app.planning.owner())),
                ),
            ],
        )
    });
    let editing = if app
        .planning
        .preview
        .as_ref()
        .is_some_and(|preview| !preview.blockers.is_empty())
    {
        Some(
            paint
                .hint(Message::ProvidersPlanningBlockedConnectionNeedsAttention)
                .into(),
        )
    } else if app.planning.connection_recovery.is_some() {
        Some(
            paint
                .hint(Message::ProvidersPlanningRequestKeptNotSent)
                .into(),
        )
    } else if let Some(PromptKind::Permission {
        file: Some(file), ..
    }) = app.planning.request.as_ref().map(|request| &request.kind)
    {
        Some(format!(
            "{} · {}",
            paint.hint(file_kind_message(file.kind)),
            crate::store::Attribution::label(app.planning.owner()),
        ))
    } else if app.planning.required_permission_review() {
        Some(format!(
            "{} · {}",
            paint.hint(Message::ProvidersPlanningPermissionRequired),
            crate::store::Attribution::label(app.planning.owner()),
        ))
    } else {
        app.write_owner().map(|claim| {
            paint.format(
                Template::ProvidersPlanningPreparingChanges,
                &[(
                    "owner",
                    &crate::store::Attribution::label(Some(&claim.owner)),
                )],
            )
        })
    };
    let state = &app.planning;
    let is_review = state.preview.is_some()
        || state.request.as_ref().is_some_and(|request| {
            !matches!(request.kind, PromptKind::Questions(_)) || state.submit_review
        });
    let compact = !state.review_details
        && !state.quit_review
        && !state.journal_mode
        && state.paths_editor.is_none()
        && state.custom_model.is_none()
        && !state.editing
        && (state.switch_to.is_some()
            || state.selection_change.is_some()
            || (state.picker.is_none() && is_review));
    let local_picker = state.picker.is_some()
        && state.selection_change.is_none()
        && state.custom_model.is_none()
        && state.paths_editor.is_none()
        && state.switch_to.is_none()
        && !state.editing
        && !state.quit_review
        && !state.journal_mode;
    let compact_picker = local_picker && !state.review_details;
    let height = if compact_picker {
        u16::try_from(state.picker_len())
            .unwrap_or(u16::MAX)
            .saturating_add(7)
            .min(18)
    } else if compact {
        18
    } else {
        area.height.saturating_sub(2)
    };
    let area = super::sheet(area, if compact_picker { 80 } else { 112 }, height);
    frame.render_widget(Clear, area);
    let form = app.form();
    let state = &mut app.planning;
    let title = if local_picker {
        format!(
            " {} / {} ",
            if state.picker == Some(Picker::Model) {
                paint.hint(Message::ProvidersPlanningModel)
            } else {
                paint.hint(Message::ProvidersPlanningReasoningLevel)
            },
            state.agent.label()
        )
    } else {
        format!(
            " {} / {} {} ",
            crate::domain::FORMS[form].name,
            state.agent.label(),
            match state.operation {
                Operation::Plan => paint.hint(Message::ProvidersPlanningPlanning),
                Operation::Implement => paint.hint(Message::ProvidersPlanningImplementation),
                Operation::Review => paint.hint(Message::ProvidersPlanningIndependentReview),
            },
        )
    };
    let title = match editing.as_deref().filter(|_| !local_picker) {
        Some(editing) if area.width < 90 => format!(" {editing} "),
        Some(editing) => format!("{title} / {editing} "),
        None => title,
    };
    let block = panel(&title, config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    if state.quit_review {
        quit(frame, content, state, paint);
        return;
    }
    if state.switch_to.is_some() {
        switch(frame, content, state, paint);
        return;
    }
    if state.journal_mode {
        journal(frame, content, state, paint);
        return;
    }
    if state.paths_editor.is_some() || state.custom_model.is_some() {
        paths_editor(frame, content, state, paint);
        return;
    }
    if state.picker.is_some() || state.selection_change.is_some() {
        picker(frame, content, state, paint);
        return;
    }
    if state.preview.is_none() && state.request.is_none() && state.inspection.is_none() {
        response(frame, content, state, paint);
        return;
    }
    let header = if state
        .preview
        .as_ref()
        .is_some_and(|preview| !preview.blockers.is_empty())
    {
        paint.hint(Message::ProvidersPlanningBlockedConnectionNeedsAttention)
    } else if state.connection_recovery.is_some() && state.inspection.is_some() {
        paint.hint(Message::ProvidersPlanningRequestKeptNotSent)
    } else if state.preview.is_some() {
        paint.hint(Message::ProvidersPlanningLocalPreviewNoProviderConnectionAuthorized)
    } else if state.request.is_some() {
        paint.hint(Message::ProvidersPlanningProviderRequestExplicitResponseRequired)
    } else if state.cancelling {
        paint.hint(Message::ProvidersPlanningCancellingWaitingOutcomeNotYetKnown)
    } else {
        paint.hint(Message::ProvidersPlanningSessionMetadataNotASelectableDecisionOr)
    };
    let header = approval_header.as_deref().unwrap_or(header);
    text(
        frame,
        row(content, 0, 1),
        &clipped(header, content.width as usize),
        paint.accent,
    );
    if is_review && !state.review_details {
        let body = row(content, 1, content.height.saturating_sub(4));
        if matches!(
            state.request.as_ref().map(|request| &request.kind),
            Some(PromptKind::Permission { file: Some(_), .. })
        ) {
            state.reviewed = render_file_summary(frame, body, state, paint);
        } else {
            let mut summary = review_summary(state, paint);
            let blocked = state
                .preview
                .as_ref()
                .is_some_and(|preview| !preview.blockers.is_empty());
            if !blocked
                && state.preview.is_some()
                && !state.inspect_models
                && !state.inspect_sessions
            {
                let request = state.immutable_task().map_or(
                    paint.hint(Message::ProvidersPlanningImmutableRequestUnavailableInspectExactPayloadDetails),
                    |task| task.request_text(),
                );
                summary.insert(1, format!("Request: {request}"));
            }
            if let Some(preview) = &state.preview
                && preview.requires_connection_review
            {
                summary.extend(preview.trust.iter().cloned());
            }
            if blocked || state.required_permission_review() {
                let paragraph = Paragraph::new(summary.join("\n"))
                    .style(paint.surface)
                    .wrap(Wrap { trim: false });
                let scope_rows = paragraph.line_count(body.width.max(1));
                state.scroll_max = scope_rows
                    .saturating_sub(usize::from(body.height))
                    .min(u16::MAX as usize) as u16;
                state.scroll = state.scroll.min(state.scroll_max);
                state.reviewed = frame.area().width >= 60
                    && frame.area().height >= 18
                    && body.width >= 40
                    && body.height > 0
                    && usize::from(state.scroll) + usize::from(body.height) >= scope_rows;
                frame.render_widget(paragraph.scroll((state.scroll, 0)), body);
            } else {
                state.reviewed = render_summary(frame, body, &summary, paint);
                state.scroll = 0;
                state.scroll_max = 0;
            }
        }
        review_controls(frame, content, state, paint);
        return;
    }
    let mut value = String::new();
    let _ = writeln!(value, "{}\n", super::selection_text(state));
    let mut review = false;
    if let Some(preview) = &state.preview {
        for blocker in &preview.blockers {
            let _ = writeln!(
                value,
                "{}",
                paint.format(
                    Template::ProvidersPlanningBlockerReason,
                    &[("reason", blocker)]
                )
            );
        }
        let selection = state.reviewed_selection();
        let _ = writeln!(
            value,
            "IMMUTABLE REQUEST SELECTION / model {} / effort {} / role {:?}\n",
            selection
                .model
                .as_deref()
                .unwrap_or("Default (provider configuration)"),
            selection
                .effort
                .as_deref()
                .unwrap_or("Default (provider configuration)"),
            state.operation
        );
        let _ = writeln!(
            value,
            "Task {} / {}\n",
            preview.task_id,
            if state.inspect_models {
                "MODEL CATALOGUE ONLY / no task prompt or resumed history".into()
            } else if state.inspect_sessions {
                "SESSION METADATA ONLY / no thread creation or model turn".into()
            } else {
                preview
                    .thread_id
                    .as_ref()
                    .map(|id| format!("RESUME provider thread {id}"))
                    .unwrap_or_else(|| "NEW provider thread".into())
            }
        );
        value.push_str("EXECUTABLE TRUST AND PROVIDER EFFECTS\n");
        for line in &preview.trust {
            let _ = writeln!(value, "{line}");
        }
        value.push_str(if state.agent.is_direct() {
            paint.hint(Message::ProvidersPlanningEeveeSendsTheExactHttpPayloadBelow)
        } else {
            paint.hint(Message::ProvidersPlanningEeveeSendsTheExactPayloadBelowIt)
        });
        value.push('\n');
        for line in &state.receipt {
            let _ = writeln!(value, "{line}");
        }
        value.push_str("\nEXACT OUTGOING PAYLOAD BEGIN\n");
        value.push_str(&preview.payload);
        value.push_str("\nEXACT OUTGOING PAYLOAD END\n\nEND OF REVIEW. ");
        value.push_str(if state.agent.is_direct() {
            if state.inspect_models {
                paint.hint(Message::ProvidersPlanningRunAuthorizesScopedHttpModelMetadataInspection)
            } else {
                paint.hint(Message::ProvidersPlanningRunAuthorizesThisExactHttpRequestAnd)
            }
        } else if state.inspect_models {
            paint.hint(Message::ProvidersPlanningRunAuthorizesOnlyTheDisclosedExecutableProvider)
        } else if state.inspect_sessions {
            paint
                .hint(Message::ProvidersPlanningRunAuthorizesExecutableConnectionAndSessionMetadata)
        } else {
            paint.hint(Message::ProvidersPlanningRunAuthorizesThisExecutableAndOneTurn)
        });
        review = true;
    } else if let Some(request) = &state.request {
        match &request.kind {
            PromptKind::Permission {
                description,
                options,
                ..
            } => {
                value.push_str("PERMISSION / EXACT PROVIDER RESPONSE\n");
                value.push_str(description);
                for (index, option) in options.iter().enumerate() {
                    let _ = writeln!(
                        value,
                        "\n{} {}: {}",
                        if index == state.option { ">" } else { " " },
                        option.label,
                        option.description
                    );
                }
                value.push_str(
                    paint.hint(Message::ProvidersPlanningEndOfRequestGrantsApplyOnceOnly),
                );
                review = true;
            }
            PromptKind::Approval {
                command,
                cwd,
                reason,
                deny_decision,
                ..
            } => {
                let _ = writeln!(
                    value,
                    "{}",
                    paint.format(
                        Template::ProvidersPlanningCommandApprovalCommandCommandWorkingDirectoryCwd,
                        &[("command", (command)), ("cwd", (cwd)), ("reason", (reason))]
                    )
                );
                if deny_decision == "cancel" {
                    value.push_str(paint.hint(
                        Message::ProvidersPlanningDenyCancelTurnTheProviderOffersCancellation,
                    ));
                }
                review = true;
            }
            PromptKind::Questions(questions) if state.submit_review => {
                value.push_str("REVIEW LITERAL ANSWERS / NOT YET SENT\n");
                for (question, answer) in questions.iter().zip(&state.answers) {
                    let _ = writeln!(
                        value,
                        "\n{}: {}\nAnswer: {}",
                        question.header,
                        question.question,
                        if question.secret {
                            paint.hint(Message::ProvidersPlanningSecretAnswerHidden)
                        } else {
                            &answer.text
                        }
                    );
                }
                value
                    .push_str(paint.hint(Message::ProvidersPlanningEndOfAnswersSendRespondsOnlyTo));
                review = true;
            }
            PromptKind::Questions(questions) => {
                if let Some(question) = questions.get(state.question) {
                    let _ = writeln!(
                        value,
                        "{}",
                        paint.format(
                            Template::ProvidersPlanningAsksQuestion,
                            &[
                                (
                                    "owner",
                                    &(crate::workspace::handoff::sanitize(
                                        &crate::store::Attribution::label(state.owner())
                                    ))
                                ),
                                ("index", &(state.question + 1).to_string()),
                                ("count", &(questions.len()).to_string()),
                                ("header", &(question.header)),
                                ("question", &(question.question))
                            ]
                        )
                    );
                    for (index, option) in question.options.iter().enumerate() {
                        let _ = writeln!(
                            value,
                            "{} {}\n  {}",
                            if state.option == index { ">" } else { " " },
                            option.label,
                            option.description
                        );
                    }
                    if question.other || question.options.is_empty() {
                        let _ = writeln!(
                            value,
                            "{}",
                            paint.format(
                                Template::ProvidersPlanningTypeLiteralAnswer,
                                &[(
                                    "marker",
                                    (if state.option == question.options.len() {
                                        ">"
                                    } else {
                                        " "
                                    })
                                )]
                            )
                        );
                    }
                    if let Some(answer) = state.answers.get(state.question) {
                        let _ = writeln!(
                            value,
                            "{}",
                            paint.format(
                                Template::ProvidersPlanningDraftAnswerNotSent,
                                &[(
                                    "answer",
                                    (if question.secret && !answer.text.is_empty() {
                                        paint.hint(Message::ProvidersPlanningHidden)
                                    } else {
                                        &answer.text
                                    })
                                )]
                            )
                        );
                    }
                }
            }
        }
        let _ = writeln!(
            value,
            "Source: {} / thread {}\nTurn {} / item {}\nRequest {}\n",
            state.agent.label(),
            request.thread_id,
            request.turn_id,
            request.item_id,
            request.request_id
        );
    } else if let Some(inspection) = &state.inspection {
        value.push_str(inspection);
    }
    let details = row(content, 1, content.height.saturating_sub(6));
    state.scroll_max = selection_details(
        frame,
        details,
        Paragraph::new(value)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        state.scroll,
    );
    state.scroll = state.scroll.min(state.scroll_max);
    // Details remain inspectable, but never arm consent by scrolling.
    if review {
        state.reviewed = false;
    }
    text(
        frame,
        row(content, content.height.saturating_sub(5), 2),
        &super::brief(&state.notice),
        paint.muted,
    );
    review_controls(frame, content, state, paint);
    if state.editing {
        editor(frame, area, state, config, paint);
    }
}

/// Each summary row is deliberately bounded; exact values remain in Details.
pub(super) fn render_summary(
    frame: &mut Frame,
    area: Rect,
    lines: &[String],
    paint: &Paint,
) -> bool {
    for (index, line) in lines.iter().enumerate().take(area.height as usize) {
        let single_line = line.replace(['\n', '\r', '\t'], " ");
        let single_line = crate::workspace::privacy::redact(&single_line);
        text(
            frame,
            row(area, index as u16, 1),
            &clipped(&single_line, area.width as usize),
            paint.surface,
        );
    }
    area.width >= 40 && area.height as usize >= lines.len()
}

fn file_kind_message(kind: FileReviewKind) -> Message {
    match kind {
        FileReviewKind::Create => Message::ProvidersPlanningFileCreate,
        FileReviewKind::Modify => Message::ProvidersPlanningFileModify,
        FileReviewKind::Delete => Message::ProvidersPlanningFileDelete,
        FileReviewKind::Rename => Message::ProvidersPlanningFileRename,
    }
}

fn syntax_message(syntax: crate::providers::RustSyntaxReview) -> Message {
    use crate::providers::RustSyntaxReview;
    match syntax {
        RustSyntaxReview::NewlyInvalid => Message::ProvidersPlanningFileSyntaxNewlyInvalid,
        RustSyntaxReview::PreExistingInvalid => {
            Message::ProvidersPlanningFileSyntaxPreExistingInvalid
        }
        RustSyntaxReview::InvalidToValid => Message::ProvidersPlanningFileSyntaxInvalidToValid,
        RustSyntaxReview::Parseable => Message::ProvidersPlanningFileSyntaxParseable,
        RustSyntaxReview::CreatedParseable => Message::ProvidersPlanningFileSyntaxCreatedParseable,
        RustSyntaxReview::CreatedInvalid => Message::ProvidersPlanningFileSyntaxCreatedInvalid,
        RustSyntaxReview::Unsupported => Message::ProvidersPlanningFileSyntaxUnsupported,
        RustSyntaxReview::Unavailable => Message::ProvidersPlanningFileSyntaxUnavailable,
    }
}

/// Scope can wrap and scroll without hiding the fixed decision controls.
/// Consent requires contiguous exposure of every immutable scope row.
fn render_file_summary(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) -> bool {
    let file: &FileReview = match state.request.as_ref().map(|request| &request.kind) {
        Some(PromptKind::Permission {
            file: Some(file), ..
        }) => file,
        _ => return false,
    };
    let mut scope = [
        paint.hint(file_kind_message(file.kind)).to_owned(),
        paint.format(Template::ProvidersPlanningFilePath, &[("path", &file.path)]),
        if file.directories.is_empty() {
            paint
                .hint(Message::ProvidersPlanningFileNoParents)
                .to_owned()
        } else {
            paint.format(
                Template::ProvidersPlanningFileParents,
                &[("paths", &file.directories.join(", "))],
            )
        },
        paint.format(
            match file.kind {
                FileReviewKind::Delete => Template::ProvidersPlanningFileRemovedBytes,
                FileReviewKind::Rename => Template::ProvidersPlanningFileMovedBytes,
                _ if file.edit => Template::ProvidersPlanningFileReplacementBytes,
                _ => Template::ProvidersPlanningFileContentBytes,
            },
            &[("bytes", &file.bytes.to_string())],
        ),
    ]
    .join("\n");
    if let Some(destination) = &file.destination {
        scope.push('\n');
        scope.push_str(&paint.format(
            Template::ProvidersPlanningFileDestination,
            &[("path", destination)],
        ));
    }
    if let Some(syntax) = file.syntax {
        scope.push('\n');
        scope.push_str(paint.hint(syntax_message(syntax)));
        if syntax != crate::providers::RustSyntaxReview::Unsupported {
            scope.push('\n');
            scope.push_str(paint.hint(Message::ProvidersPlanningFileSyntaxAdvisory));
        }
    }
    let scope_paragraph = Paragraph::new(scope.as_str())
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    let scope_rows = scope_paragraph.line_count(area.width.max(1));
    let mut footer = String::new();
    if file.external {
        footer.push_str(paint.hint(Message::ProvidersPlanningFileExternal));
        footer.push('\n');
    }
    footer.push_str(paint.hint(Message::ProvidersPlanningFileDetails));
    let footer_paragraph = Paragraph::new(footer.as_str())
        .style(paint.muted)
        .wrap(Wrap { trim: false });
    let footer_rows = footer_paragraph.line_count(area.width.max(1));
    let required_rows = scope_rows + footer_rows;
    if state.file_consent.width != area.width || state.file_consent.scope_rows != required_rows {
        if state.file_consent.width != 0 {
            state.file_consent.choice = None;
        }
        state.file_consent.width = area.width;
        state.file_consent.scope_rows = required_rows;
        state.file_consent.seen_rows = 0;
        state.scroll = 0;
    }
    if frame.area().width < 60 || frame.area().height < 18 || area.width < 40 || area.height == 0 {
        state.scroll_max = 0;
        return false;
    }
    if required_rows >= usize::from(area.height) {
        scope.push('\n');
        scope.push_str(&footer);
        for line in file.excerpt.lines().take(4) {
            scope.push('\n');
            scope.push_str(&clipped(line, area.width as usize));
        }
        let paragraph = Paragraph::new(scope)
            .style(paint.surface)
            .wrap(Wrap { trim: false });
        state.scroll_max = paragraph
            .line_count(area.width)
            .saturating_sub(usize::from(area.height))
            .min(u16::MAX as usize) as u16;
        state.scroll = state.scroll.min(state.scroll_max);
        if usize::from(state.scroll) <= state.file_consent.seen_rows {
            state.file_consent.seen_rows = state
                .file_consent
                .seen_rows
                .max(usize::from(state.scroll) + usize::from(area.height))
                .min(required_rows);
        }
        frame.render_widget(paragraph.scroll((state.scroll, 0)), area);
        return state.file_consent.seen_rows == required_rows;
    }
    state.scroll = 0;
    state.scroll_max = 0;
    state.file_consent.seen_rows = required_rows;
    frame.render_widget(scope_paragraph, row(area, 0, scope_rows as u16));
    let preview_rows = usize::from(area.height) - required_rows;
    if file.excerpt.is_empty() {
        text(
            frame,
            row(area, scope_rows as u16, 1),
            paint.hint(Message::ProvidersPlanningFileEmpty),
            paint.surface,
        );
    } else {
        for (index, line) in file.excerpt.lines().take(preview_rows.min(4)).enumerate() {
            text(
                frame,
                row(area, (scope_rows + index) as u16, 1),
                &clipped(line, area.width as usize),
                paint.surface,
            );
        }
    }
    frame.render_widget(
        footer_paragraph,
        row(area, area.height - footer_rows as u16, footer_rows as u16),
    );
    true
}

fn write_scope_summary(state: &State, paint: &Paint) -> String {
    if state.edit_paths.is_empty() {
        paint
            .hint(Message::ProvidersPlanningWorkspacePathProposals)
            .into()
    } else {
        paint.format(
            Template::ProvidersPlanningWritesSelectedPathsInspectExactScopeD,
            &[("count", &state.edit_paths.len().to_string())],
        )
    }
}

fn review_summary(state: &State, paint: &Paint) -> Vec<String> {
    if let Some(preview) = &state.preview {
        if !preview.blockers.is_empty() {
            let mut lines = vec![format!(
                "{} · {:?} · {}",
                state.agent.label(),
                state.operation,
                state.reviewed_selection().summary()
            )];
            lines.extend(preview.blockers.iter().map(|reason| {
                paint.format(
                    Template::ProvidersPlanningBlockerReason,
                    &[("reason", reason)],
                )
            }));
            lines.push(
                paint
                    .hint(Message::ProvidersPlanningSetupKeepsRequestNoAutomaticRetry)
                    .into(),
            );
            if !state.inspect_models && !state.inspect_sessions {
                let request = state
                    .immutable_task()
                    .map_or(state.submitted_request.as_ref(), |task| task.request_text());
                lines.push(format!("Request: {request}"));
            }
            return lines;
        }
        if state.agent.is_direct() {
            return vec![
                format!(
                    "{} · {:?} · {}",
                    state.agent.label(),
                    state.operation,
                    state.reviewed_selection().summary()
                ),
                if state.inspect_models {
                    paint
                        .hint(Message::ProvidersPlanningScopeModelMetadataOnlyNoTaskPrompt)
                        .into()
                } else {
                    paint.format(
                        Template::ProvidersPlanningSendPayloadBytesFreshHttpRequest,
                        &[("bytes", &(preview.payload.len()).to_string())],
                    )
                },
                format!(
                    "To: {}",
                    preview
                        .destination
                        .as_deref()
                        .unwrap_or("unavailable; inspect blocker")
                ),
                if !state.inspect_models
                    && !state.inspect_sessions
                    && crate::providers::writing_required(state.operation)
                {
                    write_scope_summary(state, paint)
                } else if state.inspect_models || state.inspect_sessions {
                    paint
                        .hint(Message::ProvidersPlanningScopeModelMetadataOnlyNoTaskPrompt)
                        .into()
                } else {
                    paint
                        .hint(Message::ProvidersPlanningReadListOnlyNoMutationToolsOrPermission)
                        .into()
                },
                paint
                    .hint(Message::ProvidersPlanningScopedCredentialHeadersProviderNetworkBilling)
                    .into(),
                paint
                    .hint(Message::ProvidersPlanningNoRetriesLocalCancelCannotProveRemote)
                    .into(),
                paint
                    .hint(Message::ProvidersPlanningDDetailsExactPayloadEndpointAndCredentials)
                    .into(),
            ];
        }
        return vec![
            format!(
                "{} · {:?} · {}",
                state.agent.label(),
                state.operation,
                state.reviewed_selection().summary()
            ),
            if state.inspect_models {
                paint
                    .hint(Message::ProvidersPlanningScopeModelCatalogueOnlyNoTaskModel)
                    .into()
            } else if state.inspect_sessions {
                paint
                    .hint(Message::ProvidersPlanningScopeSessionMetadataOnlyNoModelTurn)
                    .into()
            } else {
                paint.format(
                    Template::ProvidersPlanningSendPayloadBytesThreadOneTurn,
                    &[
                        ("bytes", &(preview.payload.len()).to_string()),
                        (
                            "thread",
                            (if preview.thread_id.is_some() {
                                paint.hint(Message::ProvidersPlanningResumed)
                            } else {
                                paint.hint(Message::ProvidersPlanningNew)
                            }),
                        ),
                    ],
                )
            },
            if !state.inspect_models
                && !state.inspect_sessions
                && state.operation == Operation::Implement
            {
                write_scope_summary(state, paint)
            } else if state.agent == crate::providers::Agent::Codex
                && !state.inspect_models
                && !state.inspect_sessions
            {
                paint
                    .hint(Message::ProvidersPlanningReadListOnlyNoMutationToolsOrPermission)
                    .into()
            } else {
                paint
                    .hint(Message::ProvidersPlanningNoSourceWriteGrantAgentPolicyNot)
                    .into()
            },
            paint
                .hint(Message::ProvidersPlanningLaunchProviderNetworkChargesAndRetainedHistory)
                .into(),
            paint
                .hint(Message::ProvidersPlanningConfigHooksToolsAndHistoryMayAdd)
                .into(),
            paint
                .hint(Message::ProvidersPlanningTrustExecutableConfigNoAutomaticFutureApproval)
                .into(),
            paint
                .hint(Message::ProvidersPlanningDDetailsExactPayloadExecutableScopeAnd)
                .into(),
        ];
    }
    let Some(request) = &state.request else {
        return Vec::new();
    };
    let who = crate::workspace::handoff::sanitize(&crate::store::Attribution::label(state.owner()));
    let mut lines = vec![paint.format(
        Template::ProvidersPlanningWhoAsksThisProviderRequestOnly,
        &[("owner", &(who)), ("provider", (state.agent.label()))],
    )];
    match &request.kind {
        PromptKind::Permission {
            description,
            options,
            file,
        } => {
            if file.is_none() {
                lines.extend(
                    description
                        .lines()
                        .filter(|line| !line.trim().is_empty())
                        .map(str::to_owned),
                );
                for option in options {
                    lines.push(format!("{}: {}", option.label, option.description));
                }
                lines.push(
                    paint
                        .hint(Message::ProvidersPlanningDDetailsCompleteRequestAndExactOptions)
                        .into(),
                );
            }
        }
        PromptKind::Approval {
            command,
            cwd,
            reason,
            deny_decision,
            ..
        } => {
            lines.push(paint.format(
                Template::ProvidersPlanningWhoWantsToRunCommand,
                &[("owner", &(who)), ("command", (command))],
            ));
            lines.push(format!("Directory: {cwd}"));
            lines.push(format!("Reason: {reason}"));
            lines.push(
                paint
                    .hint(Message::ProvidersPlanningAllowOnceThisCommandEffectsAreNot)
                    .into(),
            );
            lines.push(if deny_decision == "cancel" {
                paint
                    .hint(Message::ProvidersPlanningDenyCancelsThisTurnNoContinuingDenial)
                    .into()
            } else {
                paint
                    .hint(Message::ProvidersPlanningProviderPolicyCallbacksAreNotOsIsolation)
                    .into()
            });
            lines.push(
                paint
                    .hint(Message::ProvidersPlanningDDetailsExactCommandCwdAndFull)
                    .into(),
            );
        }
        PromptKind::Questions(questions) => {
            lines.push(paint.format(
                Template::ProvidersPlanningSendLiteralAnswersToThisProviderRequest,
                &[("count", &(state.answers.len()).to_string())],
            ));
            lines.push(paint.format(
                Template::ProvidersPlanningSecretAnswersHiddenInReview,
                &[(
                    "count",
                    &(questions.iter().filter(|question| question.secret).count()).to_string(),
                )],
            ));
            lines.push(
                paint
                    .hint(Message::ProvidersPlanningAnswersMayEnterRetainedProviderHistory)
                    .into(),
            );
            lines.push(
                paint
                    .hint(Message::ProvidersPlanningNoPermissionOrFutureRequestIsApproved)
                    .into(),
            );
            lines.push(
                paint
                    .hint(Message::ProvidersPlanningDDetailsInspectEveryQuestionAndAnswer)
                    .into(),
            );
        }
    }
    lines
}

fn review_controls(frame: &mut Frame, area: Rect, state: &State, paint: &Paint) {
    let file_details = state.review_details
        && matches!(
            state.request.as_ref().map(|request| &request.kind),
            Some(PromptKind::Permission { file: Some(_), .. })
        );
    let controls = if state
        .preview
        .as_ref()
        .is_some_and(|preview| !preview.blockers.is_empty())
    {
        paint.hint(Message::ProvidersPlanningRunBlockedNCancelEscCancelD)
    } else if state.auto_permission_review {
        paint.hint(
            if matches!(
                state.request.as_ref().map(|request| &request.kind),
                Some(PromptKind::Permission { file: Some(_), .. })
            ) {
                Message::ProvidersPlanningFileAutoReviewActions
            } else {
                Message::ProvidersPlanningPermissionAutoReviewActions
            },
        )
    } else if file_details {
        paint.hint(Message::ProvidersPlanningFileDetailsActions)
    } else if state.review_details {
        paint.hint(Message::ProvidersPlanningDSummaryPgupDnDetailsEscCancel)
    } else {
        controls(state, paint)
    };
    let file_summary = !state.review_details
        && matches!(
            state.request.as_ref().map(|request| &request.kind),
            Some(PromptKind::Permission { file: Some(_), .. })
        );
    let permission_choice = if !state.review_details && !file_summary {
        match state.request.as_ref().map(|request| &request.kind) {
            Some(PromptKind::Permission { options, .. }) => options.get(state.option),
            _ => None,
        }
    } else {
        None
    };
    if let Some(option) = permission_choice {
        text(
            frame,
            row(area, area.height.saturating_sub(3), 1),
            &clipped(
                &paint.format(
                    Template::ProvidersPlanningSelectedPermission,
                    &[("label", &option.label)],
                ),
                area.width as usize,
            ),
            paint.accent,
        );
    }
    if file_summary {
        let button = |index, message| {
            Span::styled(
                paint.hint(message),
                if state.file_consent.choice == Some(index) {
                    paint
                        .accent
                        .add_modifier(Modifier::REVERSED | Modifier::BOLD)
                } else {
                    paint.surface
                },
            )
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(if state.file_consent.choice == Some(0) {
                    "> "
                } else {
                    "  "
                }),
                button(0, Message::ProvidersPlanningFileDenyButton),
                Span::raw("  "),
                Span::raw(if state.file_consent.choice == Some(1) {
                    "> "
                } else {
                    "  "
                }),
                button(1, Message::ProvidersPlanningFileAllowButton),
            ])),
            row(area, area.height.saturating_sub(3), 1),
        );
    }
    let controls_rows = if file_summary || permission_choice.is_some() {
        2
    } else {
        3
    };
    text(
        frame,
        row(
            area,
            area.height.saturating_sub(controls_rows),
            controls_rows,
        ),
        controls,
        paint.accent,
    );
}

fn response(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    // A completed observation is not a completed turn while user input is pending.
    let active = state.activity_label().is_some();
    let (_, previous) = state.response_request();
    let file = state.items.get(state.selected).and_then(|item| {
        if let ItemKind::File(phase) = item.kind {
            Some((phase, item.text.lines().next().unwrap_or("")))
        } else {
            None
        }
    });
    let metadata = if let Some(item) = state.items.get(state.selected) {
        let mut metadata = if let ItemKind::File(phase) = item.kind {
            format!(
                "{} {}",
                paint.hint(Message::ProvidersPlanningFileLabel),
                paint.hint(file_phase_message(phase)),
            )
        } else {
            format!(
                "{}{} {}/{} · {}{}",
                if previous {
                    paint.hint(Message::ProvidersPlanningPrevious)
                } else {
                    ""
                },
                match item.kind {
                    ItemKind::Message => paint.hint(Message::ProvidersPlanningResponse),
                    ItemKind::Plan => paint.hint(Message::ProvidersPlanningPlan),
                    ItemKind::Tool => paint.hint(Message::ProvidersPlanningToolOutput),
                    ItemKind::File(_) => paint.hint(Message::ProvidersPlanningFilePreview),
                },
                state.selected + 1,
                state.items.len(),
                if let ItemKind::File(phase) = item.kind {
                    paint.hint(file_phase_message(phase))
                } else if !previous && state.request.is_some() {
                    paint.hint(Message::ProvidersPlanningWaitingForReview)
                } else if item.complete {
                    paint.hint(Message::ProvidersPlanningComplete)
                } else if active && !previous && !state.cancelling {
                    paint.hint(Message::ProvidersPlanningStreaming)
                } else {
                    paint.hint(Message::ProvidersPlanningPartial)
                },
                if matches!(item.kind, ItemKind::File(_)) {
                    ""
                } else {
                    " · unverified"
                },
            )
        };
        if !previous && let Some(model) = state.observed_model.as_deref() {
            let _ = write!(metadata, " · {model}");
        }
        let _ = write!(
            metadata,
            " · {}",
            crate::store::Attribution::label(item.owner.as_ref())
        );
        metadata
    } else if state.outcome_unknown() {
        paint
            .hint(Message::ProvidersPlanningNoResponseOutcomeUnknown)
            .into()
    } else {
        paint
            .hint(Message::ProvidersPlanningResponseNoOutputYet)
            .into()
    };
    let header_rows = if let Some((phase, path)) = file {
        text(
            frame,
            row(area, 0, 1),
            &clipped(path, area.width as usize),
            file_style(phase, paint).add_modifier(Modifier::UNDERLINED),
        );
        2
    } else {
        1
    };
    text(
        frame,
        row(area, header_rows - 1, 1),
        &clipped(&metadata, area.width as usize),
        state
            .items
            .get(state.selected)
            .and_then(|item| match item.kind {
                ItemKind::File(phase) => Some(file_style(phase, paint)),
                _ => None,
            })
            .unwrap_or(paint.accent),
    );
    let show_notice = !active && state.request.is_none() && !state.notice.is_empty();
    let show_follow = !state.items.is_empty();
    let show_evidence = show_follow && (super::VERBOSE.get() || state.omitted > 0);
    let footer_rows =
        u16::from(show_notice) + 2 * u16::from(show_follow) + u16::from(show_evidence);
    let body = row(
        area,
        header_rows,
        area.height.saturating_sub(header_rows + footer_rows),
    );
    state.set_response_body(body);
    if state.items.is_empty() {
        let empty = if active {
            paint.hint(Message::ProvidersPlanningYouCanKeepEditingYourNextRequest)
        } else if state.operation == Operation::Review {
            paint.hint(Message::ProvidersPlanningEnterSendsAnIndependentReviewRequestNo)
        } else {
            paint.hint(Message::ProvidersPlanningTypeARequestBelowEnterSendsCommands)
        };
        frame.render_widget(
            Paragraph::new(empty)
                .style(paint.surface)
                .wrap(Wrap { trim: false }),
            body,
        );
    } else {
        let lines: Vec<Line<'_>> = state
            .response_display_rows(body.width, body.height, super::VERBOSE.get())
            .map(|row| match row.kind {
                ItemKind::File(phase) => Line::from(vec![
                    Span::styled(
                        if row.header {
                            "+ "
                        } else if row.writing_edge {
                            "> "
                        } else {
                            "| "
                        },
                        file_style(phase, paint),
                    ),
                    Span::styled(
                        row.text,
                        if row.header {
                            file_style(phase, paint)
                        } else if row.writing_edge {
                            paint
                                .base
                                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                        } else {
                            paint.base
                        },
                    ),
                ])
                .style(paint.base),
                _ => Line::raw(row.text),
            })
            .collect();
        frame.render_widget(Paragraph::new(lines).style(paint.surface), body);
    }
    let mut footer_y = area.height.saturating_sub(footer_rows);
    if show_notice {
        text(
            frame,
            row(area, footer_y, 1),
            &clipped(&super::brief(&state.notice), area.width as usize),
            paint.muted,
        );
        footer_y += 1;
    }
    if show_follow {
        let (start, end, total) = state.response_position();
        let navigation = paint.format(
            Template::ProvidersPlanningStartEndTotalAltEndLatest,
            &[
                (
                    "status",
                    (if !state.following {
                        paint.hint(Message::ProvidersPlanningPaused)
                    } else if state.new_output {
                        paint.hint(Message::ProvidersPlanningNewOutput)
                    } else {
                        paint.hint(Message::ProvidersPlanningFollowing)
                    }),
                ),
                (
                    "new",
                    (if state.new_output && !state.following {
                        paint.hint(Message::ProvidersPlanningNew2)
                    } else {
                        ""
                    }),
                ),
                ("start", &(start).to_string()),
                ("end", &(end).to_string()),
                ("total", &(total).to_string()),
            ],
        );
        text(
            frame,
            row(area, footer_y, 1),
            &clipped(&navigation, area.width as usize),
            paint.accent,
        );
        footer_y += 1;
        text(
            frame,
            row(area, footer_y, 1),
            &clipped(
                paint.hint(Message::ProvidersPlanningWheelPgupDnAltUpDownItem),
                area.width as usize,
            ),
            paint.muted,
        );
        footer_y += 1;
    }
    if show_evidence {
        let evidence = if state.omitted > 0 {
            paint.format(
                Template::ProvidersPlanningEarlierItemsOmittedDecisionReviewsAComplete,
                &[("count", &(state.omitted).to_string())],
            )
        } else {
            paint
                .hint(Message::ProvidersPlanningDecisionReviewsACompleteProposalLocally)
                .into()
        };
        text(
            frame,
            row(area, footer_y, 1),
            &clipped(&evidence, area.width as usize),
            paint.muted,
        );
    }
}

/// Semantic lifecycle colors supplement, rather than replace, the file panel.
/// Monochrome keeps the same markers and uses bold for the lifecycle/header.
fn file_style(phase: FilePhase, paint: &Paint) -> Style {
    let style = paint.surface.add_modifier(Modifier::BOLD);
    if paint.surface.fg.is_none() {
        return style;
    }
    style.fg(match phase {
        FilePhase::Generating | FilePhase::Received => Color::Cyan,
        FilePhase::AwaitingReview => Color::Yellow,
        FilePhase::Applied => Color::Green,
        FilePhase::Denied
        | FilePhase::Refused
        | FilePhase::Interrupted
        | FilePhase::ReportedFailed => Color::Red,
        FilePhase::Reported | FilePhase::Unknown => Color::Magenta,
    })
}

fn controls(state: &State, paint: &Paint) -> &'static str {
    if state.auto_permission_review {
        return paint.hint(Message::ProvidersPlanningPermissionAutoReviewActions);
    }
    if let Some(preview) = &state.preview {
        return if !preview.blockers.is_empty() {
            paint.hint(Message::ProvidersPlanningRunBlockedNCancelEscCancelD)
        } else if !state.reviewed {
            paint.hint(Message::ProvidersPlanningCancelRunUnavailableUntilSummaryIsVisible)
        } else if state.affirmative {
            paint.hint(Message::ProvidersPlanningCancelRunOnceYRunNCancel)
        } else {
            paint.hint(Message::ProvidersPlanningCancelRunOnceYRunNCancel2)
        };
    }
    if let Some(request) = &state.request {
        return match &request.kind {
            PromptKind::Permission { file, options, .. } => {
                if options.len() < 2 {
                    paint.hint(Message::ProvidersPlanningDenyAllowUnavailableNOrEnterDeny)
                } else if file.is_some() && !state.reviewed {
                    paint.hint(Message::ProvidersPlanningFileUnavailable)
                } else if !state.reviewed {
                    paint.hint(Message::ProvidersPlanningDenyAllowUnavailableUntilSummaryIsVisible)
                } else if file.is_some() && options.len() == 2 {
                    paint.hint(Message::ProvidersPlanningFileActions)
                } else {
                    paint.hint(Message::ProvidersPlanningAAllowOnceNDenyYSend)
                }
            }
            PromptKind::Approval { allow_once, .. } => {
                if !allow_once {
                    paint.hint(Message::ProvidersPlanningDenyAllowUnavailableNOrEnterDeny)
                } else if !state.reviewed {
                    paint.hint(Message::ProvidersPlanningDenyAllowUnavailableUntilSummaryIsVisible)
                } else if state.affirmative {
                    paint.hint(Message::ProvidersPlanningDenyAllowOnceAYAllowN)
                } else {
                    paint.hint(Message::ProvidersPlanningDenyAllowOnceAYAllowN2)
                }
            }
            PromptKind::Questions(_) if state.submit_review => {
                if !state.reviewed {
                    paint.hint(Message::ProvidersPlanningEditAnswersSendNeedsVisibleSummaryResize)
                } else if state.affirmative {
                    paint.hint(Message::ProvidersPlanningEditAnswersSendAnswersYSendN)
                } else {
                    paint.hint(Message::ProvidersPlanningEditAnswersSendAnswersYSendN2)
                }
            }
            PromptKind::Questions(_) => {
                paint.hint(Message::ProvidersPlanningUpDownOptionEnterChooseAnswerType)
            }
        };
    }
    if state.busy() {
        paint.hint(Message::ProvidersPlanningPgupDnScrollSessionMetadataEscCancel)
    } else {
        paint.hint(Message::ProvidersPlanningPgupDnScrollSessionMetadataEscMessages)
    }
}

fn picker(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    let mut value = String::new();
    if let Some(selection) = &state.selection_change {
        let summary = vec![
            format!(
                "Model: {} → {}",
                state
                    .selection
                    .model
                    .as_deref()
                    .unwrap_or(paint.hint(Message::ProvidersPlanningDefault)),
                selection
                    .model
                    .as_deref()
                    .unwrap_or(paint.hint(Message::ProvidersPlanningDefault))
            ),
            format!(
                "Reasoning: {} → {}",
                state
                    .selection
                    .effort
                    .as_deref()
                    .unwrap_or(paint.hint(Message::ProvidersPlanningDefault)),
                selection
                    .effort
                    .as_deref()
                    .unwrap_or(paint.hint(Message::ProvidersPlanningDefault))
            ),
            paint
                .hint(Message::ProvidersPlanningNewSessionClearsThisRoleSLocal)
                .into(),
            paint
                .hint(Message::ProvidersPlanningProviderHistoryIsNotErasedNoNetwork)
                .into(),
            paint
                .hint(Message::ProvidersPlanningKeepPreservesTheCurrentModelAndOwned)
                .into(),
        ];
        state.reviewed = render_summary(
            frame,
            row(area, 0, area.height.saturating_sub(3)),
            &summary,
            paint,
        );
        text(
            frame,
            row(area, area.height.saturating_sub(3), 3),
            if state.affirmative {
                paint.hint(Message::ProvidersPlanningKeepCurrentNewSessionYNewN)
            } else {
                paint.hint(Message::ProvidersPlanningKeepCurrentNewSessionYNewN2)
            },
            paint.accent,
        );
        return;
    }
    let current_model = state
        .selection
        .model
        .as_deref()
        .unwrap_or(paint.hint(Message::ProvidersPlanningDefault));
    let current_effort = state
        .selection
        .effort
        .as_deref()
        .unwrap_or(paint.hint(Message::ProvidersPlanningDefault));
    let model = state.picker == Some(Picker::Model);
    let header = if model {
        paint.format(
            Template::ProvidersPlanningModelMatches,
            &[("count", &(state.picker_matches()).to_string())],
        )
    } else {
        paint.format(
            Template::ProvidersPlanningReasoning,
            &[(
                "model",
                (state
                    .picker_selection()
                    .model
                    .as_deref()
                    .unwrap_or(paint.hint(Message::ProvidersPlanningDefault))),
            )],
        )
    };
    text(
        frame,
        row(area, 0, 1),
        &clipped(&header, area.width as usize),
        paint.accent,
    );
    let model_width = area.width.saturating_sub(24) as usize;
    text(
        frame,
        row(area, 1, 1),
        &clipped(
            &paint.format(
                Template::ProvidersPlanningCurrent,
                &[
                    ("model", &(clipped(current_model, model_width))),
                    ("effort", (current_effort)),
                ],
            ),
            area.width as usize,
        ),
        paint.accent,
    );
    if state.review_details {
        let _ = writeln!(
            value,
            "Current model: {current_model}\nCurrent reasoning: {current_effort}"
        );
        let candidate = state.picker_selection();
        let _ = writeln!(
            value,
            "{}",
            paint.format(
                Template::ProvidersPlanningCandidateDefaultUsesProviderConfiguration,
                &[
                    (
                        "model",
                        (candidate
                            .model
                            .as_deref()
                            .unwrap_or(paint.hint(Message::ProvidersPlanningDefault)))
                    ),
                    (
                        "effort",
                        (candidate
                            .effort
                            .as_deref()
                            .unwrap_or(paint.hint(Message::ProvidersPlanningDefault)))
                    )
                ]
            )
        );
        if let Some(choice) = state.picker_choice(state.picker_selected)
            && let Some(reason) = choice.reason
        {
            let _ = writeln!(value, "\nUnavailable: {reason}");
        }
        if let Some(catalogue) = state.catalogues.get(&state.agent) {
            let _ = writeln!(value, "\nCatalogue source: {}", catalogue.source);
            let _ = writeln!(
                value,
                "Provider default: {}",
                catalogue.current.as_deref().unwrap_or("unobserved")
            );
            if let Some(note) = &catalogue.effort_note {
                let _ = writeln!(value, "{note}");
            }
        }
        if let Some(reason) = state.catalogue_error() {
            let _ = writeln!(value, "\nFetch error: {reason}");
        }
        value.push_str(paint.hint(Message::ProvidersPlanningCatalogueEntriesAndCustomIdsAreNot));
        state.scroll_max = selection_details(
            frame,
            row(area, 2, area.height.saturating_sub(3)),
            Paragraph::new(value)
                .style(paint.surface)
                .wrap(Wrap { trim: false }),
            state.scroll,
        );
        state.scroll = state.scroll.min(state.scroll_max);
        text(
            frame,
            row(area, area.height.saturating_sub(1), 1),
            paint.hint(Message::ProvidersPlanningPgupDnScrollCtrlDEscBack),
            paint.accent,
        );
        return;
    }
    text(
        frame,
        row(area, 2, 1),
        &if model {
            paint.format(
                Template::ProvidersPlanningSearch,
                &[(
                    "query",
                    &(clipped(
                        &state.picker_query.text,
                        area.width.saturating_sub(8) as usize,
                    )),
                )],
            )
        } else {
            paint
                .hint(Message::ProvidersPlanningDefaultUsesProviderSettingsEscKeepsCurrent)
                .into()
        },
        paint.muted,
    );
    let visible = area.height.saturating_sub(5).max(1) as usize;
    let start = state
        .picker_selected
        .saturating_sub(visible.saturating_sub(1) / 2)
        .min(state.picker_len().saturating_sub(visible));
    let current = state.picker_current();
    for (offset, index) in (start..(start + visible).min(state.picker_len())).enumerate() {
        let Some(choice) = state.picker_choice(index) else {
            continue;
        };
        let focused = index == state.picker_selected;
        let committed = current == Some(index);
        let label = format!(
            "{}{} {}{}",
            if focused { ">" } else { " " },
            if committed { "*" } else { " " },
            choice.label,
            if choice.reason.is_some() {
                paint.hint(Message::ProvidersPlanningUnavailable)
            } else {
                ""
            },
        );
        text(
            frame,
            row(area, 3 + offset as u16, 1),
            &clipped(&label, area.width as usize),
            if focused {
                paint.selected
            } else if committed {
                paint.accent
            } else if choice.reason.is_some() {
                paint.muted
            } else {
                paint.surface
            },
        );
    }
    let status = if state.catalogue_loading() {
        paint.hint(Message::ProvidersPlanningLoadingMetadataCtrlDDetails)
    } else if state.catalogue_error().is_some() {
        if state.catalogues.contains_key(&state.agent) {
            paint.hint(Message::ProvidersPlanningRefreshFailedCachedRowsUsableCtrlD)
        } else {
            paint.hint(Message::ProvidersPlanningFetchFailedRefreshToRetryCtrlD)
        }
    } else {
        paint.hint(Message::ProvidersPlanningFocusCurrentCtrlDDetails)
    };
    text(
        frame,
        row(area, area.height.saturating_sub(2), 1),
        status,
        paint.muted,
    );
    text(
        frame,
        row(area, area.height.saturating_sub(1), 1),
        if model {
            paint.hint(Message::ProvidersPlanningChooseEnterNextEscKeepCtrlU)
        } else if state.can_back_to_models() {
            paint.hint(Message::ProvidersPlanningChooseEnterAcceptModelsEscKeep)
        } else {
            paint.hint(Message::ProvidersPlanningChooseEnterAcceptEscKeep)
        },
        paint.accent,
    );
}

fn switch(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    let next = state.switch_to.expect("switch preview");
    if !state.review_details {
        let summary = vec![
            paint.format(
                Template::ProvidersPlanningSwitch,
                &[
                    ("previous", (state.agent.label())),
                    ("next", (next.label())),
                ],
            ),
            paint
                .hint(Message::ProvidersPlanningOldProviderSessionsStayWithTheirOriginal)
                .into(),
            paint
                .hint(Message::ProvidersPlanningNoCredentialsPermissionsOrTranscriptsTransferred)
                .into(),
            paint
                .hint(Message::ProvidersPlanningGrantsResetSendingRechecksTheConnection)
                .into(),
            if state.handoff.is_some() {
                paint
                    .hint(Message::ProvidersPlanningTransfersSelectedPortableContextUnverified)
                    .into()
            } else {
                paint
                    .hint(Message::ProvidersPlanningOnlyCurrentReviewedTaskFactsNoHandoff)
                    .into()
            },
            paint
                .hint(Message::ProvidersPlanningNoProcessOrNetworkStartsFromThis)
                .into(),
            paint
                .hint(Message::ProvidersPlanningDDetailsInspectCompletePortableContext)
                .into(),
        ];
        state.reviewed = render_summary(
            frame,
            row(area, 0, area.height.saturating_sub(3)),
            &summary,
            paint,
        );
        state.scroll = 0;
        state.scroll_max = 0;
        text(
            frame,
            row(area, area.height.saturating_sub(3), 3),
            if state.affirmative {
                paint.hint(Message::ProvidersPlanningStaySwitchAgentYSwitchNStay)
            } else {
                paint.hint(Message::ProvidersPlanningStaySwitchAgentYSwitchNStay2)
            },
            paint.accent,
        );
        return;
    }
    let handoff = state
        .handoff
        .as_ref()
        .map(|document| serde_json::to_string_pretty(document).unwrap_or_default())
        .unwrap_or_else(|| {
            paint
                .hint(Message::ProvidersPlanningOnlyCurrentReviewedTaskFactsNoPortable)
                .into()
        });
    let value = paint.format(
        Template::ProvidersPlanningIdleAgentSwitchOldProviderSessionsRemain,
        &[
            ("previous", (state.agent.label())),
            ("next", (next.label())),
            ("handoff", &(handoff)),
        ],
    );
    let details = row(area, 0, area.height.saturating_sub(3));
    state.scroll_max = selection_details(
        frame,
        details,
        Paragraph::new(value)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        state.scroll,
    );
    state.scroll = state.scroll.min(state.scroll_max);
    state.reviewed = false;
    text(
        frame,
        row(area, area.height.saturating_sub(3), 3),
        paint.hint(Message::ProvidersPlanningDSummaryPgupDnDetailsEscStay),
        paint.accent,
    );
}

fn journal(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    if state.journal_purge {
        state.reviewed = render_summary(
            frame,
            row(area, 0, area.height.saturating_sub(3)),
            &[
                paint
                    .hint(Message::ProvidersPlanningPurgeThisTaskSLocalProviderJournal)
                    .into(),
                paint
                    .hint(Message::ProvidersPlanningDeletesRetainedObservationsNotProviderHistory)
                    .into(),
                paint
                    .hint(Message::ProvidersPlanningPurgedEvidenceCannotSubstantiateLaterResults)
                    .into(),
                paint
                    .hint(Message::ProvidersPlanningDoesNotUndoWorkOrChangeProvider)
                    .into(),
            ],
            paint,
        );
        text(
            frame,
            row(area, area.height.saturating_sub(3), 3),
            if state.affirmative {
                paint.hint(Message::ProvidersPlanningKeepPurgeTaskJournalYPurgeN)
            } else {
                paint.hint(Message::ProvidersPlanningKeepPurgeTaskJournalYPurgeN2)
            },
            paint.accent,
        );
        return;
    }
    let mut value = if let Some(record) = state.journal_records.get(state.journal_selected) {
        format!(
            "PROVIDER JOURNAL / {}/{} / {}\nPartner: {}\nOrigin: {}\nCoverage: {}\nThread {} / turn {} / item {}\nComplete capture: {}\n\n{}\n\nProvider-native observation, never a local runner verification. Missing/purged evidence cannot substantiate a result.",
            state.journal_selected + 1,
            state.journal_records.len(),
            record.agent.label(),
            crate::workspace::handoff::sanitize(&crate::store::Attribution::label(
                record.partner.as_ref()
            )),
            record.origin,
            record.coverage,
            record.thread_id,
            record.turn_id,
            record.item_id,
            record.complete,
            record.text
        )
    } else {
        paint
            .hint(Message::ProvidersPlanningProviderJournalNoRetainedObservationsForThis)
            .into()
    };
    if let Some(gap) = state.journal_gap_notice() {
        value.push_str(paint.hint(Message::ProvidersPlanningMemoryOnlyCoverageWarningNotAStored));
        value.push_str(gap);
    }
    let details = row(area, 0, area.height.saturating_sub(5));
    state.scroll_max = selection_details(
        frame,
        details,
        Paragraph::new(value)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        state.scroll,
    );
    state.scroll = state.scroll.min(state.scroll_max);
    text(
        frame,
        row(area, area.height.saturating_sub(5), 2),
        &super::brief(
            state
                .journal_gap_notice()
                .unwrap_or(state.journal_notice.as_str()),
        ),
        paint.muted,
    );
    text(
        frame,
        row(area, area.height.saturating_sub(3), 3),
        paint.hint(Message::ProvidersPlanningUpDownRecordPgupDnScrollJournal),
        paint.accent,
    );
}

fn paths_editor(frame: &mut Frame, area: Rect, state: &State, paint: &Paint) {
    let custom = state.custom_model.is_some();
    let answer = state
        .custom_model
        .as_ref()
        .or(state.paths_editor.as_ref())
        .expect("local editor");
    text(
        frame,
        row(area, 0, 3),
        if custom {
            paint.hint(Message::ProvidersPlanningCustomModelIdLocalOnlyExactId)
        } else {
            paint.hint(Message::ProvidersPlanningSelectEditPathsLocalOnlyJsonArray)
        },
        paint.accent,
    );
    let input = row(area, 3, area.height.saturating_sub(5));
    let (lines, (caret_row, caret_column)) = editor_rows(
        &answer.text,
        answer.cursor,
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
    text(
        frame,
        row(area, area.height.saturating_sub(2), 2),
        &super::brief(&state.notice),
        paint.muted,
    );
}

fn quit(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    state.reviewed = render_summary(
        frame,
        row(area, 0, area.height.saturating_sub(5)),
        &[
            if state.outcome_unknown() {
                paint
                    .hint(Message::ProvidersPlanningUnknownOutcomeWorkMayHaveRun)
                    .into()
            } else {
                paint
                    .hint(Message::ProvidersPlanningCancelOwnedWorkAndExitPlanning)
                    .into()
            },
            paint
                .hint(Message::ProvidersPlanningEffectsAreNotUndoneProviderHistoryIs)
                .into(),
            paint
                .hint(Message::ProvidersPlanningWaitsForAcknowledgementOrBoundedTeardown)
                .into(),
            paint
                .hint(Message::ProvidersPlanningUnknownOutcomeNeedsASecondExplicitExit)
                .into(),
        ],
        paint,
    );
    text(
        frame,
        row(area, area.height.saturating_sub(5), 2),
        &super::brief(&state.notice),
        paint.muted,
    );
    let choice = if state.outcome_unknown() {
        if state.affirmative {
            paint.hint(Message::ProvidersPlanningStayExitWithUnknownOutcome)
        } else {
            paint.hint(Message::ProvidersPlanningStayExitWithUnknownOutcome2)
        }
    } else if state.affirmative {
        paint.hint(Message::ProvidersPlanningStayCancelRemainingWorkAndExit)
    } else {
        paint.hint(Message::ProvidersPlanningStayCancelRemainingWorkAndExit2)
    };
    text(
        frame,
        row(area, area.height.saturating_sub(3), 1),
        choice,
        paint.accent,
    );
    text(
        frame,
        row(area, area.height.saturating_sub(2), 2),
        paint.hint(Message::ProvidersPlanningYConfirmExitNStayTabChoose),
        paint.accent,
    );
}

fn editor(frame: &mut Frame, area: Rect, state: &State, config: &Config, paint: &Paint) {
    let Some(answer) = state.answers.get(state.question) else {
        return;
    };
    let secret = state
        .request
        .as_ref()
        .and_then(|request| match &request.kind {
            PromptKind::Questions(questions) => questions
                .get(state.question)
                .map(|question| question.secret),
            _ => None,
        })
        .unwrap_or(false);
    let area = centered(area, 86, 12);
    frame.render_widget(Clear, area);
    let block = panel(
        paint.hint(Message::ProvidersPlanningLiteralAnswerNotSent),
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        row(content, 0, 2),
        paint.hint(Message::ProvidersPlanningEnterEscKeepDraftNoSubmission4096),
        paint.muted,
    );
    let input = row(content, 2, content.height.saturating_sub(2));
    let hidden;
    let (value, cursor) = if secret {
        hidden = "*".repeat(answer.text.graphemes(true).count());
        (
            hidden.as_str(),
            answer.text[..answer.cursor].graphemes(true).count(),
        )
    } else {
        (answer.text.as_str(), answer.cursor)
    };
    let (lines, (caret_row, caret_column)) =
        editor_rows(value, cursor, input.width.saturating_sub(1) as usize);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{ColorMode, ImageMode, LayoutMode, Theme},
        providers::{Event, PermissionOption, Preview, Prompt, Request},
        store,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    fn config() -> Config {
        Config {
            ascii: true,
            no_animation: true,
            animation_override: Some(false),
            theme: Theme::Dark,
            color: ColorMode::TrueColor,
            images: ImageMode::Off,
            assets_dir: "unused".into(),
            layout: LayoutMode::Sanctuary,
            layout_explicit: false,
            data_dir: "unused".into(),
            storage_policy: store::Policy::default(),
            context_exclusions: vec![],
            capture_capacity: 64 * 1024,
        }
    }

    fn preview(width: u16, height: u16) -> (App, store::Snapshot) {
        let mut app = App::new("/workspace".into(), width, height);
        let mut task = store::Snapshot::empty("project".into());
        task.id = 7;
        task.goal = "A request".into();
        app.planning.command("new", true, &task, &[], None);
        assert!(matches!(
            app.planning.pending.take(),
            Some(Request::Preview(_))
        ));
        app.planning.event(Event::Preview(Preview {
            comparison_scope: None,
            arrival: crate::providers::approval_arrival(),
            destination: None,
            requires_connection_review: false,
            generation: 11,
            task_id: 7,
            thread_id: None,
            trust: vec!["exact executable/configuration\n".repeat(100)],
            payload: serde_json::json!({"request": "exact payload\n".repeat(1000)}).to_string(),
            blockers: vec![],
        }));
        (app, task)
    }

    fn draw(app: &mut App) -> ratatui::buffer::Buffer {
        let config = config();
        let paint = Paint::new(&config, &crate::domain::FORMS[0], 0);
        let mut terminal = Terminal::new(TestBackend::new(app.width, app.height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), app, &config, &paint))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn key(app: &mut App, task: &store::Snapshot, code: KeyCode) {
        app.planning.key(
            KeyEvent::new(code, KeyModifiers::NONE),
            true,
            task,
            &[],
            None,
        );
    }

    fn rendered_row(buffer: &ratatui::buffer::Buffer, area: Rect, offset: u16) -> String {
        let mut row = String::new();
        let mut x = area.x;
        while x < area.right() {
            let symbol = buffer[(x, area.y + offset)].symbol();
            row.push_str(symbol);
            x += ratatui::text::Span::raw(symbol).width().max(1) as u16;
        }
        row.trim_end().to_owned()
    }

    fn draw_home(app: &mut App) -> String {
        let config = config();
        let mut artwork = super::super::Artwork::load(&config);
        let mut terminal = Terminal::new(TestBackend::new(app.width, app.height)).unwrap();
        terminal
            .draw(|frame| super::super::render(frame, app, &config, &mut artwork, "off"))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..app.height)
            .map(|y| rendered_row(buffer, buffer.area, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn model_search_and_custom_entry_remain_visible_in_compact_picker() {
        for (width, height) in [(60, 18), (120, 36)] {
            let (mut app, task) = preview(width, height);
            app.planning.catalogues.insert(
                app.planning.agent,
                crate::providers::ModelCatalogue {
                    models: vec![crate::providers::ModelOption {
                        id: "vendor/exact".into(),
                        name: "Searchable display".into(),
                        description: String::new(),
                        efforts: vec![],
                        default_effort: None,
                    }],
                    current: Some("vendor/exact".into()),
                    source: "Metadata provenance ".repeat(80),
                    effort_note: None,
                },
            );
            app.planning.command("model", true, &task, &[], None);
            app.planning.paste("searchable");
            let buffer = draw(&mut app);
            let visible = (0..height)
                .map(|y| rendered_row(&buffer, buffer.area, y))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(visible.contains("Search: searchable"), "{visible}");
            assert!(visible.contains("Searchable display"), "{visible}");
            assert!(visible.contains("Ctrl+U Clear"), "{visible}");
            key(&mut app, &task, KeyCode::End);
            key(&mut app, &task, KeyCode::Up);
            key(&mut app, &task, KeyCode::Enter);
            app.planning.paste("vendor/Exact:2026");
            let buffer = draw(&mut app);
            let visible = (0..height)
                .map(|y| rendered_row(&buffer, buffer.area, y))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(visible.contains("vendor/Exact:2026"), "{visible}");
            assert!(app.planning.pending.is_none(), "typing never dispatches");
        }
    }

    #[test]
    fn compact_metadata_loading_and_stale_notice_do_not_hide_cached_rows() {
        let (mut app, task) = preview(60, 18);
        app.planning.cancel();
        app.planning.pending.take();
        app.planning.event(Event::Finished {
            generation: 11,
            task_id: 7,
            outcome: crate::providers::Outcome::Cancelled,
            message: "Cancelled".into(),
        });
        app.planning.catalogues.insert(
            app.planning.agent,
            crate::providers::ModelCatalogue {
                models: vec![crate::providers::ModelOption {
                    id: "cached-id".into(),
                    name: "Cached model".into(),
                    description: String::new(),
                    efforts: vec![],
                    default_effort: None,
                }],
                current: None,
                source: "Prior successful fetch".into(),
                effort_note: None,
            },
        );
        app.planning.command("model", true, &task, &[], None);
        app.planning.want_catalogue();
        app.planning.start_automatic_catalogue(&task, &[], None);
        let buffer = draw(&mut app);
        let visible = (0..app.height)
            .map(|y| rendered_row(&buffer, buffer.area, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(visible.contains("Cached model"), "{visible}");
        app.planning.pending.take();
        app.planning.event(Event::Error {
            generation: None,
            message: "Offline".into(),
        });
        let buffer = draw(&mut app);
        let visible = (0..app.height)
            .map(|y| rendered_row(&buffer, buffer.area, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(visible.contains("Cached model"), "{visible}");
    }

    #[test]
    fn home_progress_has_one_owner_before_and_during_streaming() {
        for (width, height) in [(60, 18), (120, 36)] {
            let (mut app, task) = preview(width, height);
            draw(&mut app);
            key(&mut app, &task, KeyCode::Char('y'));
            app.planning.pending.take();
            let label = app.planning.activity_label().unwrap();
            let screen = draw_home(&mut app);
            assert_eq!(screen.matches(label).count(), 1, "{screen}");
            app.planning.event(Event::Item {
                generation: 11,
                task_id: 7,
                thread_id: "thread".into(),
                turn_id: "turn".into(),
                item_id: "response".into(),
                kind: ItemKind::Message,
                text: "Partial answer stays readable".into(),
                complete: false,
            });
            let label = app.planning.activity_label().unwrap();
            let screen = draw_home(&mut app);
            assert_eq!(screen.matches(label).count(), 1, "{screen}");
            assert!(screen.contains("Partial answer stays readable"), "{screen}");
            assert!(screen.contains("streaming"), "{screen}");
        }
    }

    #[test]
    fn growing_file_previews_share_home_without_taking_the_future_draft_or_caret() {
        for (width, height, ascii, color) in [
            (60, 18, true, ColorMode::TrueColor),
            (120, 36, false, ColorMode::TrueColor),
            (60, 18, true, ColorMode::Mono),
            (120, 36, false, ColorMode::Mono),
        ] {
            let (mut app, task) = preview(width, height);
            draw(&mut app);
            key(&mut app, &task, KeyCode::Char('y'));
            app.planning.pending.take();
            app.overlay = None;
            app.editing = true;
            app.editor_field = store::EditorField::Request;
            app.input = "Future draft 東京 suffix".into();
            app.cursor = "Future draft 東京".len();
            let original = (app.input.clone(), app.cursor, app.focus);
            let mut config = config();

            config.ascii = ascii;
            config.color = color;
            let mut artwork = super::super::Artwork::load(&config);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for (phase, code) in [
                (
                    crate::providers::FilePhase::Generating,
                    "draft.rs\n1 | first_code",
                ),
                (
                    crate::providers::FilePhase::Generating,
                    "draft.rs\n1 | first_code\n2 | next_code",
                ),
                (
                    crate::providers::FilePhase::Received,
                    "draft.rs\n1 | first_code\n2 | next_code",
                ),
                (
                    crate::providers::FilePhase::Denied,
                    "draft.rs\n1 | first_code\n2 | next_code",
                ),
            ] {
                app.planning.event(Event::Item {
                    generation: 11,
                    task_id: 7,
                    thread_id: "thread".into(),
                    turn_id: "turn".into(),
                    item_id: "file".into(),
                    kind: ItemKind::File(phase),
                    text: code.into(),
                    complete: phase != crate::providers::FilePhase::Generating,
                });
                terminal
                    .draw(|frame| {
                        super::super::render(frame, &mut app, &config, &mut artwork, "off")
                    })
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let screen = (0..height)
                    .map(|y| rendered_row(buffer, buffer.area, y))
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(
                    screen.contains(file_phase_message(phase).text()),
                    "{screen}"
                );
                assert!(
                    screen.contains(if code.contains("next_code") {
                        "next_code"
                    } else {
                        "first_code"
                    }),
                    "{screen}"
                );
                let edge = buffer
                    .content
                    .iter()
                    .any(|cell| cell.symbol() == ">" && cell.modifier.contains(Modifier::BOLD));
                assert_eq!(edge, phase == FilePhase::Generating, "{screen}");
                assert!(screen.contains("Future draft"), "{screen}");
                assert_eq!(
                    (&app.input, app.cursor, app.focus),
                    (&original.0, original.1, original.2)
                );
                assert!(app.editing && app.overlay.is_none());
                assert_eq!(app.planning.items.len(), 1);
                assert!(app.planning.pending.is_none() && app.planning.decision.is_none());
            }
        }
    }

    fn draw_response(
        state: &mut State,
        width: u16,
        height: u16,
        paint: &Paint,
    ) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| response(frame, frame.area(), state, paint))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn file_panel_is_typed_not_inferred_from_prose_in_color_and_monochrome() {
        for (width, height) in [(60, 18), (120, 36)] {
            for color in [ColorMode::TrueColor, ColorMode::Mono] {
                let mut config = config();

                config.color = color;
                let paint = Paint::new(&config, &crate::domain::FORMS[0], 0);
                let content =
                    "src/새.rs\n1 | let 인사 = \"안녕 e\u{301}\";\n2 | ordinary-looking prose";
                for phase in [
                    FilePhase::Generating,
                    FilePhase::Received,
                    FilePhase::AwaitingReview,
                    FilePhase::Applied,
                    FilePhase::Denied,
                    FilePhase::Refused,
                    FilePhase::Interrupted,
                    FilePhase::Reported,
                    FilePhase::ReportedFailed,
                    FilePhase::Unknown,
                ] {
                    let mut state = State::default();
                    state.items.push(crate::app::planning::Item {
                        thread_id: "private-thread".into(),
                        turn_id: "private-turn".into(),
                        item_id: "private-item".into(),
                        kind: ItemKind::File(phase),
                        text: content.into(),
                        complete: phase != FilePhase::Generating,
                        request: "".into(),
                        owner: None,
                    });
                    let buffer = draw_response(&mut state, width, height, &paint);
                    let file = screen(&buffer);
                    assert!(
                        file.contains(paint.hint(Message::ProvidersPlanningFileLabel))
                            && file.contains(paint.hint(file_phase_message(phase)))
                            && file.contains("| src/새.rs")
                            && file.contains("| 1 | let 인사")
                            && file.contains("e\u{301}"),
                        "{file}"
                    );
                    assert!(!file.contains("private-"), "{file}");
                    let verbose = state
                        .response_display_rows(width, height, true)
                        .map(|row| row.text)
                        .collect::<Vec<_>>()
                        .join("\n");
                    assert!(!verbose.contains("private-"), "{verbose}");

                    let mut plain_state = State::default();
                    let mut item = state.items.pop().unwrap();
                    item.kind = ItemKind::Message;
                    plain_state.items.push(item);
                    let plain = screen(&draw_response(&mut plain_state, width, height, &paint));
                    assert!(
                        plain.contains("1 | let 인사")
                            && !plain.contains("| 1 | let 인사")
                            && !plain.contains(paint.hint(Message::ProvidersPlanningFileLabel)),
                        "{plain}"
                    );
                    assert_eq!(
                        plain_state
                            .response_rows(width, height, false)
                            .collect::<Vec<_>>(),
                        content.lines().collect::<Vec<_>>(),
                        "ordinary prose retains its original rows"
                    );
                }
            }
        }
    }

    #[test]
    fn paused_mid_file_rows_keep_gutter_identity_and_unicode_after_stage_and_resize() {
        for color in [ColorMode::TrueColor, ColorMode::Mono] {
            let (mut app, task) = preview(60, 18);
            draw(&mut app);
            key(&mut app, &task, KeyCode::Char('y'));
            app.planning.pending.take();
            let source = format!(
                "src/새.rs\n{}",
                (1..=100)
                    .map(|n| format!("{n} | held_{n:03} 界 e\u{301}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            let snapshot = |phase, text: String| Event::Item {
                generation: 11,
                task_id: 7,
                thread_id: "thread".into(),
                turn_id: "turn".into(),
                item_id: "file".into(),
                kind: ItemKind::File(phase),
                text,
                complete: phase != FilePhase::Generating,
            };
            app.planning
                .event(snapshot(FilePhase::Generating, source.clone()));
            let mut config = config();
            config.color = color;
            let paint = Paint::new(&config, &crate::domain::FORMS[0], 0);
            draw_response(&mut app.planning, 60, 18, &paint);
            app.planning.home_response_key(KeyCode::Home);
            app.planning.scroll_response_lines(20);
            let before = draw_response(&mut app.planning, 60, 18, &paint);
            let first = screen(&before)
                .lines()
                .find(|line| line.contains("held_"))
                .expect("paused source landmark remains visible")
                .trim_end()
                .to_owned();
            assert!(
                first.starts_with("| ") && first.contains("held_"),
                "{first}"
            );
            app.planning.event(snapshot(
                FilePhase::AwaitingReview,
                format!("{source}\n101 | newly arrived"),
            ));
            for (width, height) in [(60, 18), (120, 36), (60, 18)] {
                let buffer = draw_response(&mut app.planning, width, height, &paint);
                let visible = screen(&buffer);
                assert_eq!(
                    visible
                        .lines()
                        .find(|line| line.contains("held_"))
                        .map(str::trim_end),
                    Some(first.as_str()),
                    "{visible}"
                );
                assert!(
                    visible.contains("[FILE]") && visible.contains("src/새.rs"),
                    "{visible}"
                );
                assert!(
                    visible.contains("Awaiting review") && !visible.contains("+ [FILE]"),
                    "{visible}"
                );
                assert!(!app.planning.following && app.planning.new_output);
                assert!(app.planning.pending.is_none() && app.planning.decision.is_none());
            }
        }
    }

    #[test]
    fn prior_partial_response_is_not_labelled_as_current_stream_or_model() {
        let (mut app, task) = preview(120, 36);
        draw(&mut app);
        key(&mut app, &task, KeyCode::Char('y'));
        app.planning.pending.take();
        app.planning.observed_model = Some("current-model-only".into());
        app.planning.items.push(crate::app::planning::Item {
            thread_id: "old-thread".into(),
            turn_id: "old-turn".into(),
            item_id: "old-response".into(),
            kind: ItemKind::Message,
            text: "Earlier interrupted answer".into(),
            complete: false,
            request: "Earlier request".into(),
            owner: None,
        });
        let screen = draw_home(&mut app);
        assert!(screen.contains("Earlier interrupted answer"), "{screen}");
        assert!(screen.contains("partial"), "{screen}");
        assert!(!screen.contains("streaming"), "{screen}");
        assert!(!screen.contains("current-model-only"), "{screen}");
    }

    #[test]
    fn rendered_response_hitbox_excludes_chrome_and_is_cleared_by_reviews() {
        let (mut app, _) = preview(60, 18);
        app.planning.items.push(crate::app::planning::Item {
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "retained".into(),
            kind: ItemKind::Message,
            text: "0\n1\n2\n3\n4\n5\n6\n7\n8\n9".into(),
            complete: true,
            request: "earlier request".into(),
            owner: None,
        });
        app.planning.notice.clear();
        let config = config();
        let paint = Paint::new(&config, &crate::domain::FORMS[0], 0);
        let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
        terminal
            .draw(|frame| response(frame, Rect::new(2, 2, 56, 8), &mut app.planning, &paint))
            .unwrap();
        assert!(
            !app.planning.response_wheel(2, 2, -1),
            "metadata is not scrollable"
        );
        assert!(
            !app.planning.response_wheel(2, 8, -1),
            "status is not scrollable"
        );
        assert!(
            !app.planning.response_wheel(2, 9, -1),
            "hints are not scrollable"
        );
        assert!(app.planning.response_wheel(2, 3, -1));
        draw(&mut app);
        assert!(
            !app.planning.response_wheel(2, 3, -1),
            "review occludes home body"
        );
        assert!(app.planning.decision.is_none() && !app.planning.affirmative);
        app.planning.set_response_body(Rect::new(2, 3, 56, 5));
        terminal
            .draw(|frame| render_small(frame, frame.area(), &mut app.planning, &paint))
            .unwrap();
        assert!(!app.planning.response_wheel(2, 3, -1));
    }

    fn compact_bounds(buffer: &ratatui::buffer::Buffer) -> Rect {
        let screen = buffer.area;
        let mut left = screen.right();
        let mut top = screen.bottom();
        let mut right = screen.x;
        let mut bottom = screen.y;
        for y in screen.y..screen.bottom() {
            for x in screen.x..screen.right() {
                if !buffer[(x, y)].symbol().trim().is_empty() {
                    left = left.min(x);
                    top = top.min(y);
                    right = right.max(x + 1);
                    bottom = bottom.max(y + 1);
                }
            }
        }
        let bounds = Rect::new(left, top, right - left, bottom - top);
        let shell = super::super::shell(screen, false, crate::app::Pane::Workspace);
        assert!(!bounds.intersects(shell.header), "{bounds:?}");
        assert!(!bounds.intersects(shell.footer), "{bounds:?}");
        assert!((left - screen.x).abs_diff(screen.right() - right) <= 1);
        assert!((top - screen.y).abs_diff(screen.bottom() - bottom) <= 1);
        bounds
    }

    #[test]
    fn request_summary_keeps_frozen_input_and_consent_readable_at_supported_sizes() {
        for (width, height) in [(60, 18), (120, 36)] {
            let (mut app, mut task) = preview(width, height);
            let prepared = app.planning.preview.take().unwrap();
            task.partner_drafts
                .entry(task.lead_partner)
                .or_default()
                .drafts
                .push(store::Draft {
                    field: store::EditorField::Request,
                    text: "Reviewed immutable request".into(),
                    cursor: "Reviewed immutable request".len(),
                });
            let owner = store::Attribution::of(app.lead());
            app.planning = State::new();
            app.planning.command("new", true, &task, &[], None);
            app.planning.stage_work(store::WorkClaim {
                task_id: task.id,
                owner,
                request_id: "readable-immutable-preview".into(),
                writing: false,
            });
            app.planning.event(Event::Preview(prepared));
            draw(&mut app);
            assert!(app.planning.reviewed);
            key(&mut app, &task, KeyCode::Char('d'));
            draw(&mut app);
            assert!(app.planning.scroll_max > 0);
            assert!(!app.planning.reviewed);
        }
    }

    #[test]
    fn blocked_preview_reason_scrolls_without_arming_run() {
        for (width, height) in [(60, 18), (120, 36)] {
            let (mut app, task) = preview(width, height);
            app.planning.agent = crate::providers::Agent::ChatGPT;
            let prepared = app.planning.preview.as_mut().unwrap();
            prepared.blockers = vec![
                format!(
                    "Cannot read user provider config safely: {}REMEDY_END",
                    "actual reason ".repeat(60)
                ),
                "SECOND_BLOCKER_END".into(),
            ];
            let shown = screen(&draw(&mut app));
            let setup = Message::ProvidersPlanningRunBlockedNCancelEscCancelD.text();
            assert!(shown.contains(setup.lines().next().unwrap()), "{shown}");
            assert!(shown.contains("Cannot read user provider"), "{shown}");
            assert!(!shown.contains("preparing changes"), "{shown}");
            assert!(app.planning.scroll_max > 0, "{shown}");
            let mut inspected = shown;
            while app.planning.scroll < app.planning.scroll_max {
                key(&mut app, &task, KeyCode::PageDown);
                inspected.push_str(&screen(&draw(&mut app)));
            }
            assert!(inspected.contains("REMEDY_END"), "{inspected}");
            assert!(inspected.contains("SECOND_BLOCKER_END"), "{inspected}");
            for code in [KeyCode::Char('y'), KeyCode::Tab, KeyCode::Char('a')] {
                key(&mut app, &task, code);
                assert!(app.planning.pending.is_none() && !app.planning.affirmative);
            }
            key(&mut app, &task, KeyCode::Char('d'));
            let details = screen(&draw(&mut app));
            assert!(details.contains("Cannot read user provider"), "{details}");
            assert!(details.contains(setup.lines().next().unwrap()), "{details}");
            key(&mut app, &task, KeyCode::Char('n'));
            assert!(matches!(
                app.planning.pending.take(),
                Some(Request::Cancel { generation: 11 })
            ));
        }
    }

    #[test]
    fn summary_gate_requires_every_row_to_be_rendered() {
        let config = config();
        let paint = Paint::new(&config, &crate::domain::FORMS[0], 0);
        let summary: Vec<_> = (0..8).map(|index| format!("Scope {index}")).collect();
        for height in [7, 8] {
            let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
            let area = Rect::new(2, 2, 54, height);
            let mut reviewed = false;
            terminal
                .draw(|frame| {
                    reviewed = render_summary(frame, area, &summary, &paint);
                })
                .unwrap();
            assert_eq!(reviewed, height as usize == summary.len());
            for (index, expected) in summary.iter().take(height as usize).enumerate() {
                assert_eq!(
                    rendered_row(terminal.backend().buffer(), area, index as u16),
                    *expected
                );
            }
        }
    }

    #[test]
    fn local_confirmation_summaries_fit_compact_modals() {
        use crate::workspace::{handoff, workflows::Recipe};

        let config = config();
        let paint = Paint::new(&config, &crate::domain::FORMS[0], 0);
        super::super::VERBOSE.set(false);
        for (width, height) in [(60, 18), (120, 36)] {
            for review in 0..3 {
                let mut app = App::new("/workspace".into(), width, height);
                let mut task = store::Snapshot::empty("project".into());
                task.id = 7;
                task.goal = "Continue the saved task".into();
                app.memory.reset(7, "/project");
                app.memory.command("create-memory", true);
                app.memory.paste(&"Bounded note.\n".repeat(100));
                app.memory
                    .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), true);
                app.handoff.mode = crate::app::handoff::Mode::Export(handoff::Format::Json);
                app.handoff.candidate = Some(handoff::from_task(&task, &[], &[]).unwrap());
                app.handoff.preview = "Exact handoff payload\n".repeat(100);
                app.workflow.preview = true;
                app.workflow.recipe = Some(Recipe {
                    schema_version: 1,
                    name: "Scoped plan".into(),
                    scope: vec![],
                    context: vec![],
                    steps: vec![],
                    validation_commands: vec![],
                    completion_criteria: vec![],
                    roles: vec![],
                });
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| match review {
                        0 => super::super::memory::render(
                            frame,
                            frame.area(),
                            &mut app.memory,
                            &config,
                            &paint,
                        ),
                        1 => super::super::handoff::render(
                            frame,
                            frame.area(),
                            &mut app,
                            &config,
                            &paint,
                        ),
                        _ => super::super::workflows::render(
                            frame,
                            frame.area(),
                            &mut app,
                            &config,
                            &paint,
                        ),
                    })
                    .unwrap();
                let (reviewed, scroll_max) = match review {
                    0 => (app.memory.reviewed, app.memory.scroll_max),
                    1 => (app.handoff.reviewed, app.handoff.scroll_max),
                    _ => (app.workflow.reviewed, app.workflow.scroll_max),
                };
                assert!(reviewed, "review {review} at {width}x{height}");
                assert_eq!(scroll_max, 0, "summary must fit without scrolling");
            }
        }
    }

    #[test]
    fn workspace_labels_selected_response_with_its_originating_request() {
        let config = config();
        let paint = Paint::new(&config, &crate::domain::FORMS[0], 0);
        let mut app = App::new("/workspace".into(), 120, 36);
        app.planning.submitted_request = "new request".into();
        app.planning.items.push(crate::app::planning::Item {
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "item".into(),
            kind: ItemKind::Message,
            text: "Retained response".into(),
            complete: true,
            request: "original request".into(),
            owner: None,
        });
        let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
        terminal
            .draw(|frame| render_workspace(frame, frame.area(), &mut app, &config, &paint))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let current = rendered_row(buffer, buffer.area, 0);
        assert!(current.contains("new request"));
        assert!(!current.contains("original request"));
        let label = rendered_row(buffer, buffer.area, 1);
        assert!(label.contains("original request"));
        assert!(!label.contains("new request"));
    }

    #[test]
    fn long_request_needs_visible_summary_and_deliberate_confirmation_not_end() {
        for size in [(60, 18), (120, 36)] {
            let (mut app, task) = preview(size.0, size.1);
            key(&mut app, &task, KeyCode::Char('y'));
            assert!(
                app.planning.pending.is_none(),
                "unrendered summary cannot authorize"
            );
            draw(&mut app);
            assert!(app.planning.reviewed);
            assert_eq!(app.planning.scroll, 0);
            assert!(!app.planning.affirmative);
            key(&mut app, &task, KeyCode::Char('d'));
            draw(&mut app);
            assert!(app.planning.scroll_max > 0);
            assert!(!app.planning.reviewed);
            key(&mut app, &task, KeyCode::Char('y'));
            assert!(app.planning.pending.is_none());
            key(&mut app, &task, KeyCode::Char('d'));
            draw(&mut app);
            for event in [
                KeyEvent::new_with_kind(
                    KeyCode::Char('y'),
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                ),
                KeyEvent::new_with_kind(
                    KeyCode::Char('y'),
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                ),
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::SHIFT),
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
            ] {
                app.planning.key(event, true, &task, &[], None);
                assert!(app.planning.pending.is_none());
            }
            assert!(!app.planning.paste("yna"));
            key(&mut app, &task, KeyCode::Tab);
            assert!(app.planning.affirmative);
            app.planning.key(
                KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat),
                true,
                &task,
                &[],
                None,
            );
            assert!(app.planning.pending.is_none());
            key(&mut app, &task, KeyCode::Enter);
            assert!(matches!(
                app.planning.pending,
                Some(Request::Run { generation: 11 })
            ));
        }
    }

    #[test]
    fn neutral_enter_resize_and_blockers_never_run() {
        for size in [(60, 18), (120, 36)] {
            let (mut app, task) = preview(size.0, size.1);
            draw(&mut app);
            key(&mut app, &task, KeyCode::Enter);
            assert!(!matches!(app.planning.pending, Some(Request::Run { .. })));
            let (mut app, task) = preview(size.0, size.1);
            draw(&mut app);
            key(&mut app, &task, KeyCode::Tab);
            app.planning.invalidate_review();
            key(&mut app, &task, KeyCode::Char('y'));
            assert!(app.planning.pending.is_none());
            draw(&mut app);
            app.planning
                .preview
                .as_mut()
                .unwrap()
                .blockers
                .push("stale source".into());
            key(&mut app, &task, KeyCode::Char('y'));
            assert!(app.planning.pending.is_none());
        }
    }

    #[test]
    fn compact_generation_scope_contrasts_read_only_roles_and_metadata() {
        let config = config();

        let paint = Paint::new(&config, &crate::domain::FORMS[0], 0);
        for agent in [
            crate::providers::Agent::Compatible,
            crate::providers::Agent::Codex,
            crate::providers::Agent::Claude,
        ] {
            for operation in [Operation::Plan, Operation::Implement, Operation::Review] {
                let (mut app, _) = preview(60, 18);
                app.planning.agent = agent;
                app.planning.operation = operation;
                let scope = if operation == Operation::Implement {
                    paint.hint(Message::ProvidersPlanningWorkspacePathProposals)
                } else if agent.is_direct() || agent == crate::providers::Agent::Codex {
                    paint.hint(Message::ProvidersPlanningReadListOnlyNoMutationToolsOrPermission)
                } else {
                    paint.hint(Message::ProvidersPlanningNoSourceWriteGrantAgentPolicyNot)
                };
                assert!(
                    review_summary(&app.planning, &paint)
                        .iter()
                        .any(|line| line == scope)
                );
                let buffer = draw(&mut app);
                let rendered = (0..buffer.area.height)
                    .map(|y| rendered_row(&buffer, buffer.area, y))
                    .collect::<Vec<_>>()
                    .join("\n");
                if operation == Operation::Implement {
                    assert!(rendered.contains(scope), "{rendered}");
                }
                assert!(app.planning.reviewed && !app.planning.affirmative);
                assert!(app.planning.pending.is_none());
                app.planning.inspect_models = true;
                let lines = review_summary(&app.planning, &paint);
                assert!(!lines.iter().any(
                    |line| line == paint.hint(Message::ProvidersPlanningWorkspacePathProposals)
                ));
                assert!(app.planning.pending.is_none());
            }
        }
    }

    #[test]
    fn complete_creation_scope_is_visible_and_permission_is_one_shot() {
        use crate::store::TextDensity::{Comfortable, Compact};
        for (width, height, density) in [
            (60, 18, Compact),
            (120, 36, Compact),
            (60, 18, Comfortable),
            (120, 36, Comfortable),
        ] {
            let (mut app, task) = preview(width, height);
            app.planning.set_text_density(density);
            draw(&mut app);
            key(&mut app, &task, KeyCode::Char('y'));
            assert!(matches!(
                app.planning.pending.take(),
                Some(Request::Run { .. })
            ));
            app.planning.event(Event::Prompt {
                generation: 11,
                task_id: 7,
                request: Prompt {
                    arrival: crate::providers::approval_arrival(),
                    request_id: serde_json::json!(42),
                    thread_id: "thread".into(),
                    turn_id: "turn".into(),
                    item_id: "item".into(),
                    kind: PromptKind::Permission {
                        description: format!("Create\nPath: new/새.rs\nCreate directories: new\n--- /dev/null\n+++ new/새.rs\n{}", "+exact content\n".repeat(1000)),
                        file: Some(FileReview {
                            path: "new/새.rs".into(),
                            destination: None,
                            syntax: None,
                            kind: FileReviewKind::Create,
                            directories: vec!["new".into()],
                            bytes: 14000,
                            excerpt: FileReview::excerpt(&"exact content\n".repeat(1000)),
                            edit: false,
                            external: false,
                        }),
                        options: vec![
                            PermissionOption {
                                label: "Deny".into(),
                                description: "No grant".into(),
                                answer: serde_json::json!({"permission":"deny"}),
                            },
                            PermissionOption {
                                label: "Allow once".into(),
                                description: "One request only".into(),
                                answer: serde_json::json!({"permission":"once"}),
                            },
                        ],
                    },
                },
            });
            let buffer = draw(&mut app);
            let screen = (0..buffer.area.height)
                .map(|y| rendered_row(&buffer, buffer.area, y))
                .collect::<Vec<_>>()
                .join("\n");
            for scope in ["new/새.rs", "new", "exact content", "a/y"] {
                assert!(screen.contains(scope), "{screen}");
            }
            assert!(app.planning.reviewed);
            key(&mut app, &task, KeyCode::Char('d'));
            draw(&mut app);
            key(&mut app, &task, KeyCode::PageDown);
            let buffer = draw(&mut app);
            let screen = (0..buffer.area.height)
                .map(|y| rendered_row(&buffer, buffer.area, y))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(screen.contains("--- /dev/null"), "{screen}");
            assert!(app.planning.scroll_max > 0 && !app.planning.reviewed);
            key(&mut app, &task, KeyCode::Char('a'));
            assert!(app.planning.pending.is_none());
            key(&mut app, &task, KeyCode::Char('d'));
            draw(&mut app);
            let saved_scope = match &mut app.planning.request.as_mut().unwrap().kind {
                PromptKind::Permission {
                    file: Some(file), ..
                } => std::mem::replace(&mut file.path, format!("{}file.rs", "nested/".repeat(200))),
                _ => unreachable!(),
            };
            draw(&mut app);
            assert!(
                !app.planning.reviewed,
                "clipped scope must never arm permission"
            );
            key(&mut app, &task, KeyCode::Char('a'));
            assert!(app.planning.pending.is_none());
            if let PromptKind::Permission {
                file: Some(file), ..
            } = &mut app.planning.request.as_mut().unwrap().kind
            {
                file.path = saved_scope;
            }
            app.width = 59;
            draw(&mut app);
            key(&mut app, &task, KeyCode::Char('a'));
            assert!(!app.planning.reviewed && app.planning.pending.is_none());
            app.width = width;
            draw(&mut app);
            assert!(!app.planning.paste("a"));
            app.planning.key(
                KeyEvent::new_with_kind(
                    KeyCode::Char('a'),
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                ),
                true,
                &task,
                &[],
                None,
            );
            assert!(app.planning.pending.is_none());
            key(&mut app, &task, KeyCode::Char('a'));
            assert!(
                matches!(app.planning.pending.take(), Some(Request::Answer { answer, .. }) if answer == serde_json::json!({"permission":"once"}))
            );
            key(&mut app, &task, KeyCode::Char('a'));
            assert!(
                app.planning.pending.is_none(),
                "closed request cannot replay approval"
            );
        }
    }

    #[test]
    fn auth_and_version_long_details_do_not_gate_explicit_shortcuts() {
        use crate::providers::{
            agent::Profile,
            auth::{Method, Plan},
        };
        for (width, height) in [(60, 18), (120, 36)] {
            for auth in [false, true] {
                let mut app = App::new("/workspace".into(), width, height);
                app.agents.pending.take();
                app.agents
                    .event(crate::providers::agent::Event::Discovered(vec![Profile {
                        name: "Test".into(),
                        executable: Some("/trusted/agent".into()),
                        argv: vec!["--version".into()],
                        installed: "present".into(),
                        version: "unknown".into(),
                        authentication: "unknown".into(),
                        configuration: "unknown".into(),
                        os: "test".into(),
                        adapter: "none".into(),
                        action: "none".into(),
                        probe_allowed: true,
                        trust_preview: vec!["exact configuration\n".repeat(1000)],
                        revision: 1,
                        architecture: "test".into(),
                        billing: "unknown".into(),
                        blockers: vec![],
                        capabilities: vec![],
                    }]));
                if auth {
                    let mut plan =
                        Plan::prepare(4, Method::DevinWeb, std::path::Path::new("/workspace"))
                            .unwrap();
                    plan.lines.push("exact auth effects\n".repeat(1000));
                    app.agents.auth_preview = Some(plan);
                } else {
                    app.agents.command("version", true);
                }
                let config = config();
                let paint = Paint::new(&config, &crate::domain::FORMS[0], 0);
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| {
                        super::super::agents::render(frame, frame.area(), &mut app, &config, &paint)
                    })
                    .unwrap();
                compact_bounds(terminal.backend().buffer());
                assert!(app.agents.reviewed);
                assert_eq!(app.agents.scroll, 0);
                assert!(!app.agents.affirmative);
                app.agents.key(
                    KeyEvent::new_with_kind(
                        KeyCode::Char('y'),
                        KeyModifiers::NONE,
                        KeyEventKind::Repeat,
                    ),
                    true,
                );
                app.agents
                    .key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT), true);
                assert!(app.agents.pending.is_none() && app.agents.pending_auth.is_none());
                app.agents
                    .key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE), true);
                terminal
                    .draw(|frame| {
                        super::super::agents::render(frame, frame.area(), &mut app, &config, &paint)
                    })
                    .unwrap();
                assert!(app.agents.scroll_max > 0);
                app.agents
                    .key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE), true);
                assert!(app.agents.pending.is_none() && app.agents.pending_auth.is_none());
                app.agents
                    .key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE), true);
                terminal
                    .draw(|frame| {
                        super::super::agents::render(frame, frame.area(), &mut app, &config, &paint)
                    })
                    .unwrap();
                app.agents
                    .key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE), true);
                if auth {
                    assert!(app.agents.pending_auth.is_some());
                } else {
                    assert!(matches!(
                        app.agents.pending,
                        Some(crate::providers::agent::Request::Probe { .. })
                    ));
                }
            }
        }
    }

    #[test]
    fn long_answer_and_switch_reviews_confirm_from_initial_summary() {
        for (width, height) in [(60, 18), (120, 36)] {
            let (mut app, task) = preview(width, height);
            draw(&mut app);
            key(&mut app, &task, KeyCode::Char('y'));
            app.planning.pending.take();
            app.planning.event(Event::Prompt {
                generation: 11,
                task_id: 7,
                request: Prompt {
                    arrival: crate::providers::approval_arrival(),
                    request_id: serde_json::json!(43),
                    thread_id: "thread".into(),
                    turn_id: "turn".into(),
                    item_id: "question".into(),
                    kind: PromptKind::Questions(vec![crate::providers::Question {
                        id: "q".into(),
                        header: "Question".into(),
                        question: "long question\n".repeat(100),
                        options: vec![],
                        other: true,
                        secret: false,
                    }]),
                },
            });
            app.planning.editing = true;
            key(&mut app, &task, KeyCode::Char('y'));
            assert_eq!(
                app.planning.answers[0].text, "y",
                "editor receives literal shortcut letters"
            );
            app.planning.paste(&"answer".repeat(100));
            app.planning.editing = false;
            app.planning
                .command("answers-review", true, &task, &[], None);
            draw(&mut app);
            assert!(app.planning.reviewed);
            assert!(!app.planning.affirmative);
            key(&mut app, &task, KeyCode::Char('y'));
            assert!(matches!(
                app.planning.pending.take(),
                Some(Request::Answer { .. })
            ));

            let mut app = App::new("/workspace".into(), width, height);
            app.planning.reset(task.id);
            app.planning.switch_to = Some(crate::providers::Agent::Claude);
            draw(&mut app);
            assert!(app.planning.reviewed);
            key(&mut app, &task, KeyCode::Char('y'));
            assert_eq!(app.planning.agent, crate::providers::Agent::Claude);
            assert!(
                app.planning.pending.is_none(),
                "switch grants no process/network consent"
            );
        }
    }

    fn bolt_prompt(kind: PromptKind) -> (App, store::Snapshot) {
        let mut app = App::new("/workspace".into(), 60, 18);
        app.planning.front = Some(store::Attribution {
            partner: 1,
            name: "Bolt".into(),
            form: 2,
        });
        let mut task = store::Snapshot::empty("project".into());
        task.id = 7;
        task.goal = "A request".into();
        app.planning.command("new", true, &task, &[], None);
        app.planning.pending.take();
        app.planning.event(Event::Preview(Preview {
            comparison_scope: None,
            arrival: crate::providers::approval_arrival(),
            destination: None,
            requires_connection_review: false,
            generation: 11,
            task_id: 7,
            thread_id: None,
            trust: vec![],
            payload: "{}".into(),
            blockers: vec![],
        }));
        draw(&mut app);
        key(&mut app, &task, KeyCode::Char('y'));
        app.planning.pending.take();
        app.planning.event(Event::Prompt {
            generation: 11,
            task_id: 7,
            request: Prompt {
                arrival: crate::providers::approval_arrival(),
                request_id: serde_json::json!(44),
                thread_id: "thread".into(),
                turn_id: "turn".into(),
                item_id: "item".into(),
                kind,
            },
        });
        (app, task)
    }

    fn screen(buffer: &ratatui::buffer::Buffer) -> String {
        (0..buffer.area.height)
            .map(|y| rendered_row(buffer, buffer.area, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn required_permission_and_connection_scope_scroll_with_explicit_controls() {
        for (width, height) in [(60, 18), (120, 36)] {
            for connection in [false, true] {
                let (mut app, task) = if connection {
                    let (mut app, task) = preview(width, height);
                    app.planning
                        .preview
                        .as_mut()
                        .unwrap()
                        .requires_connection_review = true;
                    (app, task)
                } else {
                    let (mut app, task) = bolt_prompt(PromptKind::Permission {
                        description: format!("{}final-exact-scope", "scope row\n".repeat(50)),
                        file: None,
                        options: vec![
                            PermissionOption {
                                label: "Deny".into(),
                                description: "No effects".into(),
                                answer: serde_json::json!("deny"),
                            },
                            PermissionOption {
                                label: "Allow once".into(),
                                description: "This call only".into(),
                                answer: serde_json::json!("once"),
                            },
                        ],
                    });
                    app.width = width;
                    app.height = height;
                    (app, task)
                };
                app.planning.auto_permission_review = true;
                let shown = screen(&draw(&mut app));
                assert!(shown.contains("Enter"), "{shown}");
                for code in [KeyCode::Char('a'), KeyCode::Char('y'), KeyCode::Char('n')] {
                    key(&mut app, &task, code);
                }
                app.planning.paste("y\na\n");
                assert!(app.planning.pending.is_none());
                key(&mut app, &task, KeyCode::Enter);
                assert!(!app.planning.auto_permission_review);
                draw(&mut app);
                assert!(!app.planning.reviewed);
                key(
                    &mut app,
                    &task,
                    KeyCode::Char(if connection { 'y' } else { 'a' }),
                );
                assert!(app.planning.pending.is_none());
                let mut shown = String::new();
                while app.planning.scroll < app.planning.scroll_max {
                    key(&mut app, &task, KeyCode::PageDown);
                    shown = screen(&draw(&mut app));
                }
                assert!(app.planning.reviewed, "{shown}");
                if connection {
                    assert!(shown.contains("executable/configuration"), "{shown}");
                } else {
                    assert!(shown.contains("final-exact-scope"), "{shown}");
                    assert!(shown.contains("Deny"), "{shown}");
                }
                for (modifiers, kind) in [
                    (KeyModifiers::NONE, KeyEventKind::Repeat),
                    (KeyModifiers::CONTROL, KeyEventKind::Press),
                ] {
                    app.planning.key(
                        KeyEvent::new_with_kind(KeyCode::Char('y'), modifiers, kind),
                        true,
                        &task,
                        &[],
                        None,
                    );
                }
                assert!(app.planning.pending.is_none());
                key(
                    &mut app,
                    &task,
                    KeyCode::Char(if connection { 'y' } else { 'a' }),
                );
                if connection {
                    assert!(matches!(
                        app.planning.pending,
                        Some(Request::Run { generation: 11 })
                    ));
                } else {
                    assert!(matches!(
                        app.planning.pending.as_ref(),
                        Some(Request::Answer { generation: 11, answer, .. })
                            if answer == &serde_json::json!("once")
                    ));
                }
            }
        }
    }

    #[test]
    fn long_file_scope_scrolls_without_skipping_consent_or_hiding_buttons() {
        let directories: Vec<String> = (1..=8)
            .map(|count| {
                "long_component/"
                    .repeat(count)
                    .trim_end_matches('/')
                    .to_owned()
            })
            .collect();
        let path = format!("{}/new.rs", directories.last().unwrap());
        let (mut app, task) = bolt_prompt(PromptKind::Permission {
            description: format!("Create {path} with exact content"),
            file: Some(FileReview {
                path,
                destination: None,
                syntax: None,
                directories,
                kind: FileReviewKind::Create,
                bytes: 12,
                excerpt: "exact content".into(),
                edit: false,
                external: false,
            }),
            options: vec![
                PermissionOption {
                    label: "Deny".into(),
                    description: String::new(),
                    answer: serde_json::json!("deny"),
                },
                PermissionOption {
                    label: "Allow once".into(),
                    description: String::new(),
                    answer: serde_json::json!("once"),
                },
            ],
        });
        app.planning.auto_permission_review = true;
        draw(&mut app);
        key(&mut app, &task, KeyCode::Right);
        draw(&mut app);
        assert!(!app.planning.reviewed);
        key(&mut app, &task, KeyCode::End);
        draw(&mut app);
        key(&mut app, &task, KeyCode::Enter);
        assert!(
            app.planning.pending.is_none(),
            "jumping past unseen scope cannot grant"
        );
        key(&mut app, &task, KeyCode::Home);
        draw(&mut app);
        for _ in 0..100 {
            if app.planning.scroll == app.planning.scroll_max {
                break;
            }
            key(&mut app, &task, KeyCode::PageDown);
            let visible = screen(&draw(&mut app));
            assert!(visible.contains("[ Allow once ]"));
            assert!(app.planning.pending.is_none());
        }
        assert!(app.planning.reviewed);
        key(&mut app, &task, KeyCode::Enter);
        assert!(matches!(
            app.planning.pending.as_ref(),
            Some(Request::Answer { request_id, answer, .. })
                if request_id == &serde_json::json!(44) && answer == &serde_json::json!("once")
        ));
    }

    #[test]
    fn rename_destination_and_syntax_warning_are_part_of_required_consent_scope() {
        let destination = format!("{}new.rs", "long_destination/".repeat(12));
        let (mut app, task) = bolt_prompt(PromptKind::Permission {
            description: "Rename exact source".into(),
            file: Some(FileReview {
                path: "old.rs".into(),
                destination: Some(destination.clone()),
                syntax: Some(crate::providers::RustSyntaxReview::NewlyInvalid),
                kind: FileReviewKind::Rename,
                directories: Vec::new(),
                bytes: 8,
                excerpt: "fn main(".into(),
                edit: false,
                external: false,
            }),
            options: vec![
                PermissionOption {
                    label: "Deny".into(),
                    description: String::new(),
                    answer: serde_json::json!("deny"),
                },
                PermissionOption {
                    label: "Allow once".into(),
                    description: String::new(),
                    answer: serde_json::json!("once"),
                },
            ],
        });
        app.planning.auto_permission_review = true;
        let mut observed = screen(&draw(&mut app));
        key(&mut app, &task, KeyCode::Char('y'));
        assert!(
            app.planning.pending.is_none(),
            "unseen destination must block permission"
        );
        key(&mut app, &task, KeyCode::Right);
        observed.push_str(&screen(&draw(&mut app)));
        for _ in 0..100 {
            if app.planning.scroll == app.planning.scroll_max {
                break;
            }
            key(&mut app, &task, KeyCode::PageDown);
            observed.push_str(&screen(&draw(&mut app)));
        }
        assert!(app.planning.reviewed, "{observed}");
        assert!(observed.contains("new.rs"), "{observed}");
        assert!(
            observed.contains(Message::ProvidersPlanningFileSyntaxNewlyInvalid.text()),
            "{observed}"
        );
        key(&mut app, &task, KeyCode::Char('y'));
        assert!(matches!(app.planning.pending, Some(Request::Answer { .. })));
    }

    #[test]
    fn typed_file_summary_shows_operation_empty_and_unicode_source_at_minimum_size() {
        let config = config();

        let paint = Paint::new(&config, &crate::domain::FORMS[0], 0);
        for kind in [FileReviewKind::Create, FileReviewKind::Modify] {
            for content in ["", "let 인사 = \"안녕\";\n"] {
                for external in [false, true] {
                    let (mut app, _) = bolt_prompt(PromptKind::Permission {
                        description: "Untrusted generic description is not file metadata".into(),
                        file: Some(FileReview {
                            path: "src/새.rs".into(),
                            destination: None,
                            syntax: None,
                            kind,
                            directories: Vec::new(),
                            bytes: content.len(),
                            excerpt: FileReview::excerpt(content),
                            edit: false,
                            external,
                        }),
                        options: vec![
                            PermissionOption {
                                label: "provider denial".into(),
                                description: "provider text".into(),
                                answer: serde_json::json!({"permission":"deny"}),
                            },
                            PermissionOption {
                                label: "provider permission".into(),
                                description: "provider text".into(),
                                answer: serde_json::json!({"permission":"once"}),
                            },
                        ],
                    });
                    let text = screen(&draw(&mut app));
                    assert!(app.planning.reviewed, "{text}");
                    assert!(text.contains(paint.hint(file_kind_message(kind))), "{text}");
                    assert!(text.contains("src/새.rs") && text.contains("a/y"), "{text}");
                    assert!(
                        !text.contains("Untrusted generic") && !text.contains("provider denial"),
                        "{text}"
                    );
                    if content.is_empty() {
                        assert!(
                            text.contains(paint.hint(Message::ProvidersPlanningFileEmpty)),
                            "{text}"
                        );
                    } else {
                        assert!(text.contains("안녕"), "{text}");
                    }
                    if external {
                        assert!(
                            text.contains(paint.hint(Message::ProvidersPlanningFileExternal)),
                            "{text}"
                        );
                    }
                    assert!(app.planning.pending.is_none());
                }
            }
        }
    }

    #[test]
    fn compact_provider_dialogs_name_the_asking_partner_and_keep_choices() {
        let options = vec![
            PermissionOption {
                label: "Deny".into(),
                description: "No grant".into(),
                answer: serde_json::json!({"permission":"deny"}),
            },
            PermissionOption {
                label: "Allow once".into(),
                description: "One request only".into(),
                answer: serde_json::json!({"permission":"once"}),
            },
        ];
        let (mut app, _) = bolt_prompt(PromptKind::Permission {
            description: "write src/lib.rs".into(),
            file: None,
            options,
        });
        let text = screen(&draw(&mut app));
        assert!(app.planning.reviewed, "summary fits 60x18:\n{text}");
        assert!(text.contains("Bolt (Jolteon)"), "{text}");
        assert!(text.contains("Deny") && text.contains("Allow"), "{text}");

        let (mut app, _) = bolt_prompt(PromptKind::Approval {
            command: "cargo test".into(),
            cwd: "/workspace".into(),
            reason: "verify".into(),
            allow_once: true,
            deny_decision: "decline".into(),
        });
        let text = screen(&draw(&mut app));
        assert!(app.planning.reviewed, "summary fits 60x18:\n{text}");
        assert!(
            text.contains("Bolt (Jolteon) wants to run: cargo test"),
            "{text}"
        );

        let (mut app, _) = bolt_prompt(PromptKind::Questions(vec![crate::providers::Question {
            id: "q".into(),
            header: "Scope".into(),
            question: "Which module?".into(),
            options: vec![],
            other: true,
            secret: false,
        }]));
        let text = screen(&draw(&mut app));
        assert!(text.contains("Bolt (Jolteon) asks"), "{text}");
        assert!(text.contains("Which module?"), "{text}");
    }

    #[test]
    fn deferred_preview_keeps_submitted_request_after_next_draft_is_saved() {
        for (width, height) in [(120, 36), (60, 18)] {
            let mut app = App::new("/workspace".into(), width, height);
            let mut task = store::Snapshot::empty("/workspace".into());
            task.id = 7;
            task.goal = "Immutable request review".into();
            task.partner_drafts
                .entry(task.lead_partner)
                .or_default()
                .drafts
                .push(store::Draft {
                    field: store::EditorField::Request,
                    text: "submitted-immutable-A".into(),
                    cursor: "submitted-immutable-A".len(),
                });
            app.planning.command("new", true, &task, &[], None);
            app.planning.stage_work(store::WorkClaim {
                task_id: 7,
                owner: store::Attribution::of(app.lead()),
                request_id: "immutable-preview-review".into(),
                writing: false,
            });
            app.planning.event(Event::Preview(Preview {
                comparison_scope: None,
                arrival: crate::providers::approval_arrival(),
                destination: None,
                requires_connection_review: true,
                generation: 11,
                task_id: 7,
                thread_id: None,
                trust: Vec::new(),
                payload: "{}".into(),
                blockers: Vec::new(),
            }));
            let mut saved_next = task.clone();
            saved_next.revision += 1;
            saved_next
                .partner_drafts
                .get_mut(&task.lead_partner)
                .unwrap()
                .drafts[0]
                .text = "saved-next-request-B".into();
            app.task_id = 7;
            app.tasks = vec![saved_next];
            let shown = screen(&draw(&mut app));
            assert!(shown.contains("Request: submitted-immutable-A"), "{shown}");
            assert!(!shown.contains("saved-next-request-B"), "{shown}");
            assert_eq!(
                app.planning.immutable_task().unwrap().request_text(),
                "submitted-immutable-A"
            );
        }
    }
}
