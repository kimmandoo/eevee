use std::fmt::Write;

use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Clear, Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;

use super::{Paint, centered, clipped, editor_rows, panel, row, selection_details, text};
use crate::{
    app::{
        App,
        planning::{Picker, State},
    },
    config::Config,
    providers::{ItemKind, Operation, PromptKind},
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
            &clipped(&format!("Current request: {current}"), area.width as usize),
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
                &format!(
                    "{}: {request}",
                    if previous {
                        "Previous request"
                    } else {
                        "Request"
                    }
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
    if !state.quit_review {
        state.invalidate_review();
    }
    frame.render_widget(Clear, area);
    let value = if state.quit_review {
        format!(
            "Exit planning?\n{}\n{}\nTab Choose / Enter / Esc Stay",
            if state.outcome_unknown() {
                "UNKNOWN outcome; work may have run.\nEffects not rolled back."
            } else if state.cancelling {
                "Waiting for cancellation."
            } else {
                "Cancel owned work before exit."
            },
            if state.outcome_unknown() {
                if state.affirmative {
                    "Stay / [Exit with unknown outcome]"
                } else {
                    "[Stay] / Exit with unknown outcome"
                }
            } else if state.affirmative {
                "Stay / [Cancel and exit]"
            } else {
                "[Stay] / Cancel and exit"
            }
        )
    } else {
        "Planning: terminal too small\nNeed 60 columns x 18 rows.\nNo run or approval available.\nEsc Cancel / Ctrl+C Safe exit\nCancellation waits for acknowledgement.".into()
    };
    text(frame, area, &value, paint.accent);
}

pub(super) fn render(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let state = &app.planning;
    let is_review = state.preview.is_some()
        || state.request.as_ref().is_some_and(|request| {
            !matches!(request.kind, PromptKind::Questions(_)) || state.submit_review
        });
    let compact = !state.review_details
        && !state.quit_review
        && !state.journal_mode
        && state.paths_editor.is_none()
        && !state.editing
        && (state.switch_to.is_some()
            || state.selection_change.is_some()
            || (state.picker.is_none() && is_review));
    let height = if compact {
        18
    } else {
        area.height.saturating_sub(2)
    };
    let area = centered(area, 112, height);
    frame.render_widget(Clear, area);
    let state = &mut app.planning;
    let title = format!(
        " {} / {} {} ",
        match state.operation {
            Operation::Plan => "Espeon",
            Operation::Implement => "Flareon",
            Operation::Review => "Glaceon",
        },
        state.agent.label(),
        match state.operation {
            Operation::Plan => "planning",
            Operation::Implement => "implementation",
            Operation::Review => "independent review",
        },
    );
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
    if state.paths_editor.is_some() {
        paths_editor(frame, content, state, paint);
        return;
    }
    if state.picker.is_some() {
        picker(frame, content, state, paint);
        return;
    }
    if state.preview.is_none() && state.request.is_none() && state.inspection.is_none() {
        response(frame, content, state, paint);
        return;
    }
    let header = if state.preview.is_some() {
        "LOCAL PREVIEW / no provider connection authorized"
    } else if state.request.is_some() {
        "PROVIDER REQUEST / explicit response required"
    } else if state.cancelling {
        "CANCELLING / waiting; outcome not yet known"
    } else {
        "SESSION METADATA / not a selectable decision or model result"
    };
    text(
        frame,
        row(content, 0, 1),
        &clipped(header, content.width as usize),
        paint.accent,
    );
    if is_review && !state.review_details {
        let mut summary = review_summary(state);
        if state.preview.is_some() && !state.inspect_models && !state.inspect_sessions {
            let request = app
                .tasks
                .iter()
                .find(|task| task.id == app.task_id)
                .map_or("", |task| task.request_text());
            summary.insert(1, format!("Request: {request}"));
        }
        let body = row(content, 1, content.height.saturating_sub(4));
        state.reviewed = render_summary(frame, body, &summary, paint);
        state.scroll = 0;
        state.scroll_max = 0;
        review_controls(frame, content, state, paint);
        return;
    }
    let mut value = String::new();
    let _ = writeln!(value, "{}\n", super::selection_text(state));
    let mut review = false;
    if let Some(preview) = &state.preview {
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
        value.push_str("\nEevee sends the exact payload below. It is NOT the complete effective provider prompt: provider configuration, instructions, tools/MCP, hooks and resumed history may add context or effects. Provider owns authentication, usage charges and retained thread history. Read-only policy is agent-controlled, not proof of OS isolation. No always-allow grant or automatic replay.\n");
        value.push('\n');
        for line in &state.receipt {
            let _ = writeln!(value, "{line}");
        }
        for blocker in &preview.blockers {
            let _ = writeln!(value, "\nBLOCKER: {blocker}");
        }
        value.push_str("\nEXACT OUTGOING PAYLOAD BEGIN\n");
        value.push_str(&preview.payload);
        value.push_str("\nEXACT OUTGOING PAYLOAD END\n\nEND OF REVIEW. ");
        value.push_str(if state.inspect_models {
            "Run authorizes only the disclosed executable/provider model inspection. No model turn or session resume is authorized. Stay sends nothing.\n"
        } else if state.inspect_sessions {
            "Run authorizes executable connection and session metadata inspection only, not a model turn. Stay sends nothing.\n"
        } else {
            "Run authorizes this executable and one turn under the selected role and scope only. Stay sends nothing.\n"
        });
        review = true;
    } else if let Some(request) = &state.request {
        let _ = writeln!(
            value,
            "Source: {} / thread {}\nTurn {} / item {}\nRequest {}\n",
            state.agent.label(),
            request.thread_id,
            request.turn_id,
            request.item_id,
            request.request_id
        );
        match &request.kind {
            PromptKind::Permission {
                description,
                options,
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
                value.push_str("\nEND OF REQUEST. Grants apply once only to this request, not the next provider.");
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
                    "COMMAND APPROVAL\nCommand: {command}\nWorking directory: {cwd}\nReason: {reason}\n\nAllow once applies ONLY to this server request. The provider policy and callback coverage are not OS isolation. Edit boundaries depend on the reviewed adapter/operation; never approve a command you do not understand.\n\nEND OF REQUEST."
                );
                if deny_decision == "cancel" {
                    value.push_str("\nDeny (cancel turn): the provider offers cancellation, not a continue-after-denial option.\n");
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
                            "[secret answer hidden]"
                        } else {
                            &answer.text
                        }
                    );
                }
                value.push_str("\nEND OF ANSWERS. Send responds only to the current request.\n");
                review = true;
            }
            PromptKind::Questions(questions) => {
                if let Some(question) = questions.get(state.question) {
                    let _ = writeln!(
                        value,
                        "QUESTION {}/{} / {}\n{}\n",
                        state.question + 1,
                        questions.len(),
                        question.header,
                        question.question
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
                            "{} Type literal answer",
                            if state.option == question.options.len() {
                                ">"
                            } else {
                                " "
                            }
                        );
                    }
                    if let Some(answer) = state.answers.get(state.question) {
                        let _ = writeln!(
                            value,
                            "\nDraft answer (not sent): {}",
                            if question.secret && !answer.text.is_empty() {
                                "[hidden]"
                            } else {
                                &answer.text
                            }
                        );
                    }
                }
            }
        }
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

fn review_summary(state: &State) -> Vec<String> {
    if let Some(preview) = &state.preview {
        return vec![
            format!(
                "{} · {:?} · {}",
                state.agent.label(),
                state.operation,
                state.reviewed_selection().summary()
            ),
            if state.inspect_models {
                "Scope: model catalogue only; no task/model turn.".into()
            } else if state.inspect_sessions {
                "Scope: session metadata only; no model turn.".into()
            } else {
                format!(
                    "Send {} payload bytes · {} thread · one turn.",
                    preview.payload.len(),
                    if preview.thread_id.is_some() {
                        "resumed"
                    } else {
                        "new"
                    }
                )
            },
            if state.operation == Operation::Implement {
                format!(
                    "Writes: {} selected paths; inspect exact scope (d).",
                    state.edit_paths.len()
                )
            } else {
                "No source-write grant; agent policy, not OS isolation.".into()
            },
            "Launch provider; network, charges and retained history.".into(),
            "Config, hooks, tools and history may add effects.".into(),
            "Trust executable/config; no automatic future approval.".into(),
            if preview.blockers.is_empty() {
                "d Details: exact payload, executable, scope and risks.".into()
            } else {
                format!("BLOCKED: {} (d for all prerequisites)", preview.blockers[0])
            },
        ];
    }
    let Some(request) = &state.request else {
        return Vec::new();
    };
    let mut lines = vec![format!(
        "{} · current provider request only",
        state.agent.label()
    )];
    match &request.kind {
        PromptKind::Permission {
            description,
            options,
        } => {
            lines.push(format!("Permission: {description}"));
            if let Some(option) = options.get(state.option) {
                lines.push(format!("Selected: {}", option.label));
                lines.push(format!("Effect: {}", option.description));
            }
            if options.len() == 2 {
                lines.push(format!("a Allow once: {}", options[1].description));
            }
            lines.push("One request only; no future/other-provider grants.".into());
            lines.push("Provider policy is not OS isolation.".into());
            lines.push("d Details: complete request and exact options.".into());
        }
        PromptKind::Approval {
            command,
            cwd,
            reason,
            deny_decision,
            ..
        } => {
            lines.push(format!("Command: {command}"));
            lines.push(format!("Directory: {cwd}"));
            lines.push(format!("Reason: {reason}"));
            lines.push("Allow once: this command; effects are not rolled back.".into());
            lines.push(if deny_decision == "cancel" {
                "Deny cancels this turn; no continuing denial offered.".into()
            } else {
                "Provider policy/callbacks are not OS isolation.".into()
            });
            lines.push("d Details: exact command, cwd and full reason.".into());
        }
        PromptKind::Questions(questions) => {
            lines.push(format!(
                "Send {} literal answers to this provider request.",
                state.answers.len()
            ));
            lines.push(format!(
                "{} secret answers hidden in review.",
                questions.iter().filter(|question| question.secret).count()
            ));
            lines.push("Answers may enter retained provider history.".into());
            lines.push("No permission or future request is approved.".into());
            lines.push("d Details: inspect every question and answer.".into());
        }
    }
    lines
}

fn review_controls(frame: &mut Frame, area: Rect, state: &State, paint: &Paint) {
    let controls = if state.review_details {
        "d Summary / PgUp/Dn Details / Esc Cancel\nReturn to summary to choose; no approval in Details."
    } else {
        controls(state)
    };
    text(
        frame,
        row(area, area.height.saturating_sub(3), 3),
        controls,
        paint.accent,
    );
}

fn response(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    // The shell header owns live progress; this line describes only the
    // selected output, which may belong to an earlier request.
    let active = state.activity_label().is_some();
    let (_, previous) = state.response_request();
    let metadata = if let Some(item) = state.items.get(state.selected) {
        let mut metadata = format!(
            "{}{} {}/{} · {} · unverified",
            if previous { "Previous " } else { "" },
            match item.kind {
                ItemKind::Message => "Response",
                ItemKind::Plan => "Plan",
                ItemKind::Tool => "Tool output",
            },
            state.selected + 1,
            state.items.len(),
            if item.complete {
                "complete"
            } else if active && !previous && !state.cancelling {
                "streaming"
            } else {
                "partial"
            },
        );
        if !previous && let Some(model) = state.observed_model.as_deref() {
            let _ = write!(metadata, " · {model}");
        }
        metadata
    } else if state.outcome_unknown() {
        "No response · outcome unknown".into()
    } else {
        "Response · no output yet".into()
    };
    text(
        frame,
        row(area, 0, 1),
        &clipped(&metadata, area.width as usize),
        paint.accent,
    );
    let show_notice = !active && !state.notice.is_empty();
    let show_follow = !state.items.is_empty();
    let show_evidence = super::VERBOSE.get() && show_follow;
    let footer_rows = u16::from(show_notice) + u16::from(show_follow) + u16::from(show_evidence);
    let body = row(area, 1, area.height.saturating_sub(1 + footer_rows));
    if state.items.is_empty() {
        let empty = if active {
            "You can keep editing your next request below."
        } else if state.operation == Operation::Review {
            "Type a request below for an independent review. Review scope before Run; no source writes or resumed implementation history."
        } else {
            "Type a request below. Enter prepares a concise scope review; only explicit Run connects to the provider."
        };
        frame.render_widget(
            Paragraph::new(empty)
                .style(paint.surface)
                .wrap(Wrap { trim: false }),
            body,
        );
    } else {
        let lines: Vec<Line<'_>> = state
            .response_rows(body.width, body.height, super::VERBOSE.get())
            .map(Line::raw)
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
        let follow = if state.new_output {
            "New output · Alt+End: latest"
        } else if state.following {
            "Following"
        } else {
            "Paused · Alt+End: latest"
        };
        let navigation = format!("{follow} · PgUp/Dn scroll · Alt+↑↓ items");
        text(
            frame,
            row(area, footer_y, 1),
            &clipped(&navigation, area.width as usize),
            paint.accent,
        );
        footer_y += 1;
    }
    if show_evidence {
        let evidence = if state.omitted > 0 {
            format!(
                "{} earlier items omitted · /decision reviews a complete proposal",
                state.omitted
            )
        } else {
            "/decision reviews a complete proposal locally".into()
        };
        text(
            frame,
            row(area, footer_y, 1),
            &clipped(&evidence, area.width as usize),
            paint.muted,
        );
    }
}

fn controls(state: &State) -> &'static str {
    if let Some(preview) = &state.preview {
        return if !preview.blockers.is_empty() {
            "Run blocked / n Cancel / Esc Cancel\n d Details: prerequisites / no connection"
        } else if !state.reviewed {
            "[Cancel] / Run unavailable until summary is visible\nResize / Esc Cancel"
        } else if state.affirmative {
            "Cancel / [Run once] · y Run / n Cancel\nTab Choose / Enter Confirm / d Details / Esc Cancel"
        } else {
            "[Cancel] / Run once · y Run / n Cancel\nTab Choose / Enter Cancel / d Details / Esc Cancel"
        };
    }
    if let Some(request) = &state.request {
        return match &request.kind {
            PromptKind::Permission { .. } => {
                if !state.reviewed {
                    "[Deny] / Allow unavailable until summary is visible\nResize / Enter Deny / Esc Cancel"
                } else {
                    "a Allow once / n Deny / y Send selected / d Details\nTab Choose / Enter Send selected / Esc Cancel"
                }
            }
            PromptKind::Approval { allow_once, .. } => {
                if !allow_once {
                    "[Deny] / Allow unavailable\nn or Enter Deny / d Details / Esc Cancel"
                } else if !state.reviewed {
                    "[Deny] / Allow unavailable until summary is visible\nResize / Enter Deny / Esc Cancel"
                } else if state.affirmative {
                    "Deny / [Allow once] · a/y Allow / n Deny\nTab Choose / Enter Send / d Details / Esc Cancel"
                } else {
                    "[Deny] / Allow once · a/y Allow / n Deny\nTab Choose / Enter Deny / d Details / Esc Cancel"
                }
            }
            PromptKind::Questions(_) if state.submit_review => {
                if !state.reviewed {
                    "[Edit answers] / Send needs visible summary\nResize / Esc Cancel"
                } else if state.affirmative {
                    "Edit answers / [Send answers] · y Send / n Edit\nTab Choose / Enter Send / d Details / Esc Cancel"
                } else {
                    "[Edit answers] / Send answers · y Send / n Edit\nTab Choose / Enter Edit / d Details / Esc Cancel"
                }
            }
            PromptKind::Questions(_) => {
                "Up/Down Option / Enter Choose / /answer Type\nTab Question / /answers-review Review send / Esc Cancel turn"
            }
        };
    }
    if state.busy() {
        "PgUp/Dn Scroll session metadata\nEsc Cancel / /quit Safe exit"
    } else {
        "PgUp/Dn Scroll session metadata / Esc Messages\n/new /resume /provider /history"
    }
}

fn picker(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    let mut value = format!("{}\n\n", super::selection_text(state));
    if let Some(selection) = &state.selection_change {
        let summary = vec![
            format!(
                "Model: {} → {}",
                state.selection.model.as_deref().unwrap_or("Default"),
                selection.model.as_deref().unwrap_or("Default")
            ),
            "New session clears this role's local resume selection.".into(),
            "Provider history is not erased; no network starts.".into(),
            "Keep preserves the current model and owned session.".into(),
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
                "Keep current / [New session] · y New / n Keep\nTab Choose / Enter Confirm / Esc Back"
            } else {
                "[Keep current] / New session · y New / n Keep\nTab Choose / Enter Keep / Esc Back"
            },
            paint.accent,
        );
        return;
    } else {
        if state.picker == Some(Picker::Model) {
            let count = state
                .catalogues
                .get(&state.agent)
                .map_or(0, |catalogue| catalogue.models.len());
            let _ = writeln!(value, "MODELS · {count} advertised · local selection");
        } else {
            let _ = writeln!(value, "REASONING EFFORT · local selection");
        }
        if let Some(catalogue) = state.catalogues.get(&state.agent) {
            let _ = writeln!(
                value,
                "Provider default model: {} (not an active-turn acknowledgment).",
                catalogue.current.as_deref().unwrap_or("unobserved"),
            );
            if catalogue
                .current
                .as_ref()
                .is_some_and(|id| !catalogue.models.iter().any(|model| &model.id == id))
            {
                let _ = writeln!(
                    value,
                    "Retained model is not advertised; refresh or select another explicitly. No fallback."
                );
            }
        }
        let visible = area.height.saturating_sub(11).max(1) as usize;
        let start = state
            .picker_selected
            .saturating_sub(visible.saturating_sub(1) / 2);
        let current = state.picker_current();
        for index in start..(start + visible).min(state.picker_len()) {
            let Some(choice) = state.picker_choice(index) else {
                continue;
            };
            let _ = writeln!(
                value,
                "{} {}{}",
                if index == state.picker_selected {
                    ">"
                } else {
                    " "
                },
                clipped(&choice.label, area.width.saturating_sub(16) as usize),
                if choice.reason.is_some() {
                    " [unavailable]"
                } else if current == Some(index) {
                    " (current)"
                } else {
                    ""
                }
            );
        }
        if let Some(choice) = state.picker_choice(state.picker_selected)
            && let Some(reason) = choice.reason
        {
            let _ = writeln!(value, "\nUnavailable: {reason}");
        }
        if let Some(catalogue) = state.catalogues.get(&state.agent) {
            // Provenance and remedies must remain inspectable, not sentence-
            // shortened or permanently clipped to the terminal width.
            let _ = writeln!(value, "\nCatalogue source: {}", catalogue.source);
            if let Some(note) = &catalogue.effort_note {
                let _ = writeln!(value, "{note}");
            }
        }
        value.push_str("\nRefresh reviews the connection; selecting a model never sends content.");
    }
    let details = row(area, 0, area.height.saturating_sub(4));
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
        row(area, area.height.saturating_sub(4), 3),
        &super::brief(&state.notice),
        paint.muted,
    );
    text(
        frame,
        row(area, area.height.saturating_sub(1), 1),
        "Arrows Choose / Enter / PgUp/Dn Details / Esc Back",
        paint.accent,
    );
}

fn switch(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    let next = state.switch_to.expect("switch preview");
    if !state.review_details {
        let summary = vec![
            format!("Switch {} → {}", state.agent.label(), next.label()),
            "Old provider sessions stay with their original agent.".into(),
            "No credentials, permissions or transcripts transferred.".into(),
            "All grants reset; next provider request needs Run.".into(),
            if state.handoff.is_some() {
                "Transfers selected portable context (unverified).".into()
            } else {
                "Only current reviewed task facts; no handoff selected.".into()
            },
            "No process or network starts from this switch.".into(),
            "d Details: inspect complete portable context.".into(),
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
                "Stay / [Switch agent] · y Switch / n Stay\nTab Choose / Enter Confirm / d Details / Esc Stay"
            } else {
                "[Stay] / Switch agent · y Switch / n Stay\nTab Choose / Enter Stay / d Details / Esc Stay"
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
            "Only current reviewed task facts; no portable document selected.".into()
        });
    let value = format!(
        "IDLE AGENT SWITCH / {} → {}\n\nOld provider sessions remain separate and available under their original agent. No opaque provider session IDs, permissions, credentials, hidden reasoning or transcripts cross this boundary. All grants reset; the next request still requires executable/configuration and exact-payload review.\n\nAPPROVED PORTABLE CONTEXT / UNVERIFIED\n{handoff}\n\nEND OF SWITCH REVIEW.",
        state.agent.label(),
        next.label()
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
        "d Summary / PgUp/Dn Details / Esc Stay\nReturn to summary to choose; no switch in Details.",
        paint.accent,
    );
}

fn journal(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    if state.journal_purge {
        state.reviewed = render_summary(
            frame,
            row(area, 0, area.height.saturating_sub(3)),
            &[
                "Purge this task's local provider journal?".into(),
                "Deletes retained observations, not provider history.".into(),
                "Purged evidence cannot substantiate later results.".into(),
                "Does not undo work or change provider permissions.".into(),
            ],
            paint,
        );
        text(
            frame,
            row(area, area.height.saturating_sub(3), 3),
            if state.affirmative {
                "Keep / [Purge task journal] · y Purge / n Keep\nTab Choose / Enter Confirm / Esc Keep"
            } else {
                "[Keep] / Purge task journal · y Purge / n Keep\nTab Choose / Enter Keep / Esc Keep"
            },
            paint.accent,
        );
        return;
    }
    let value = if let Some(record) = state.journal_records.get(state.journal_selected) {
        format!(
            "PROVIDER JOURNAL / {}/{} / {}\nOrigin: {}\nCoverage: {}\nThread {} / turn {} / item {}\nComplete capture: {}\n\n{}\n\nProvider-native observation, never a local runner verification. Missing/purged evidence cannot substantiate a result.",
            state.journal_selected + 1,
            state.journal_records.len(),
            record.agent.label(),
            record.origin,
            record.coverage,
            record.thread_id,
            record.turn_id,
            record.item_id,
            record.complete,
            record.text
        )
    } else {
        "PROVIDER JOURNAL\nNo retained observations for this task. Local commands remain in /execution-history; no provider result is inferred.".into()
    };
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
        &super::brief(&state.journal_notice),
        paint.muted,
    );
    text(
        frame,
        row(area, area.height.saturating_sub(3), 3),
        "Up/Down Record / PgUp/Dn Scroll\n/journal-purge /journal-retry /journal-close",
        paint.accent,
    );
}

fn paths_editor(frame: &mut Frame, area: Rect, state: &State, paint: &Paint) {
    let answer = state.paths_editor.as_ref().expect("path editor");
    text(
        frame,
        row(area, 0, 3),
        "SELECT EDIT PATHS / LOCAL ONLY\nJSON array of existing root-relative files\nEnter Apply selection / Esc Cancel / 4096 bytes",
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
                "UNKNOWN OUTCOME: work may have run.".into()
            } else {
                "Cancel owned work and exit planning?".into()
            },
            "Effects are not undone; provider history is retained.".into(),
            "Waits for acknowledgement or bounded teardown.".into(),
            "Unknown outcome needs a second explicit exit choice.".into(),
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
            "Stay / [Exit with unknown outcome]"
        } else {
            "[Stay] / Exit with unknown outcome"
        }
    } else if state.affirmative {
        "Stay / [Cancel remaining work and exit]"
    } else {
        "[Stay] / Cancel remaining work and exit"
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
        "y Confirm exit / n Stay / Tab Choose / Enter / Esc Stay",
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
    let block = panel(" Literal answer / not sent ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        row(content, 0, 2),
        "Enter/Esc Keep draft / no submission\n4096 UTF-8 bytes; paste is text, never approval",
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
            generation: 11,
            task_id: 7,
            thread_id: None,
            trust: vec!["exact executable/configuration\n".repeat(100)],
            payload: "exact payload\n".repeat(1000),
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
        (area.x..area.right())
            .map(|x| buffer[(x, area.y + offset)].symbol())
            .collect::<String>()
            .trim_end()
            .to_owned()
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
        });
        let screen = draw_home(&mut app);
        assert!(screen.contains("Earlier interrupted answer"), "{screen}");
        assert!(screen.contains("partial"), "{screen}");
        assert!(!screen.contains("streaming"), "{screen}");
        assert!(!screen.contains("current-model-only"), "{screen}");
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
        assert!((16..=20).contains(&bounds.height), "{bounds:?}");
        assert!((left - screen.x).abs_diff(screen.right() - right) <= 1);
        assert!((top - screen.y).abs_diff(screen.bottom() - bottom) <= 1);
        bounds
    }

    #[test]
    fn request_summary_is_compact_centered_and_fully_visible() {
        for (width, height) in [(60, 18), (120, 36)] {
            let (mut app, task) = preview(width, height);
            let buffer = draw(&mut app);
            let bounds = compact_bounds(&buffer);
            let config = config();
            let paint = Paint::new(&config, &crate::domain::FORMS[0], 0);
            let content = panel("", &config, &paint, true).inner(bounds);
            let mut summary = review_summary(&app.planning);
            summary.insert(1, "Request: ".into());
            assert!(summary.len() <= content.height.saturating_sub(4) as usize);
            for (index, line) in summary.iter().enumerate() {
                let expected = clipped(line, content.width as usize);
                assert_eq!(
                    rendered_row(&buffer, content, 1 + index as u16),
                    expected.trim_end()
                );
            }
            for (index, line) in controls(&app.planning).lines().enumerate() {
                assert_eq!(
                    rendered_row(&buffer, content, content.height - 3 + index as u16),
                    line
                );
            }
            assert!(app.planning.reviewed);
            key(&mut app, &task, KeyCode::Char('d'));
            draw(&mut app);
            assert!(app.planning.scroll_max > 0);
            assert!(!app.planning.reviewed);
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
                compact_bounds(terminal.backend().buffer());
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
    fn long_permission_can_allow_once_without_scrolling_and_cannot_repeat() {
        for size in [(60, 18), (120, 36)] {
            let (mut app, task) = preview(size.0, size.1);
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
                    request_id: serde_json::json!(42),
                    thread_id: "thread".into(),
                    turn_id: "turn".into(),
                    item_id: "item".into(),
                    kind: PromptKind::Permission {
                        description: "exact scope\n".repeat(1000),
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
            draw(&mut app);
            assert!(app.planning.reviewed);
            assert_eq!(app.planning.scroll, 0);
            assert_eq!(app.planning.option, 0);
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
}
