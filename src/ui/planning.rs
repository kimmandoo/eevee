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
    let area = centered(area, 112, area.height.saturating_sub(2));
    frame.render_widget(Clear, area);
    let marker = super::activity_marker(app.activity_frame, config);
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
        response(frame, content, state, marker, paint);
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
    if review && details.height > 0 && state.scroll >= state.scroll_max {
        state.reviewed = true;
    }
    text(
        frame,
        row(content, content.height.saturating_sub(5), 2),
        &super::brief(&state.notice),
        paint.muted,
    );
    text(
        frame,
        row(content, content.height.saturating_sub(3), 3),
        controls(state),
        paint.accent,
    );
    if state.editing {
        editor(frame, area, state, config, paint);
    }
}

fn response(frame: &mut Frame, area: Rect, state: &mut State, marker: &str, paint: &Paint) {
    let label = state.activity_label();
    let status = if let Some(label) = label {
        format!("{marker} {label}")
    } else if state.outcome_unknown() {
        "Outcome unknown".into()
    } else {
        "IDLE / explicit New".into()
    };
    let model = state.observed_model.as_deref().unwrap_or("not observed");
    let metadata = if let Some(item) = state.items.get(state.selected) {
        format!(
            "{status} · {} {}/{} {} · model {}",
            match item.kind {
                ItemKind::Message => "Response",
                ItemKind::Plan => "Plan",
                ItemKind::Tool => "Tool output",
            },
            state.selected + 1,
            state.items.len(),
            if item.complete {
                "complete"
            } else if label.is_some() && !state.cancelling {
                "streaming"
            } else {
                "partial"
            },
            model,
        )
    } else {
        format!("{status} · observed model {model}")
    };
    text(
        frame,
        row(area, 0, 1),
        &clipped(&metadata, area.width as usize),
        paint.accent,
    );
    let body = row(area, 1, area.height.saturating_sub(4));
    if state.items.is_empty() {
        let empty = if label.is_some() {
            "Waiting for provider output. Partial responses appear here as they arrive."
        } else if state.operation == Operation::Review {
            "/new prepares an independent review. Review the outgoing context before Run; no source writes or resumed implementation history."
        } else {
            "/new prepares a response from your saved goal and selected sources. Review the outgoing context before Run."
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
    text(
        frame,
        row(area, area.height.saturating_sub(3), 1),
        &clipped(&super::brief(&state.notice), area.width as usize),
        paint.muted,
    );
    let follow = if state.new_output {
        "New output · End: latest / follow"
    } else if state.following {
        "Following · PgUp: pause"
    } else {
        "Follow paused · End: latest / follow"
    };
    let navigation = format!("{follow} · ↑↓ items · PgUp/Dn · /verbose details");
    text(
        frame,
        row(area, area.height.saturating_sub(2), 1),
        &clipped(&navigation, area.width as usize),
        paint.accent,
    );
    let evidence = if state.omitted > 0 {
        format!(
            "Unverified provider output · {} earlier items omitted · /decision copies complete proposal",
            state.omitted
        )
    } else {
        "Unverified provider output · /decision copies a complete proposal for local review".into()
    };
    text(
        frame,
        row(area, area.height.saturating_sub(1), 1),
        &clipped(&evidence, area.width as usize),
        paint.muted,
    );
}

fn controls(state: &State) -> &'static str {
    if let Some(preview) = &state.preview {
        return if !preview.blockers.is_empty() {
            "Run blocked / Enter or Esc Stay\nPgUp/Dn Review / no connection"
        } else if !state.reviewed {
            "[Stay] / Run locked until last page\nPgUp/Dn or End Review / Esc Cancel"
        } else if state.affirmative {
            "Stay / [Trust and run once]\nTab Choose / Enter Confirm / Esc Cancel"
        } else {
            "[Stay] / Trust and run once\nTab Choose / Enter Stay / Esc Cancel"
        };
    }
    if let Some(request) = &state.request {
        return match &request.kind {
            PromptKind::Permission { .. } => {
                if !state.reviewed {
                    "[Deny] / Allow locked until end\nEnter Deny / PgUp/Dn Review / Esc Cancel"
                } else {
                    "Tab Choose exact option / Enter Send\nPgUp/Dn Review / Esc Cancel turn"
                }
            }
            PromptKind::Approval { allow_once, .. } => {
                if !allow_once {
                    "[Deny] / Allow unavailable\nEnter Deny / Esc Cancel turn / PgUp/Dn Review"
                } else if !state.reviewed {
                    "[Deny] / Allow locked until last page\nEnter Deny / PgUp/Dn Review / Esc Cancel turn"
                } else if state.affirmative {
                    "Deny / [Allow once]\nTab Choose / Enter Send / Esc Cancel turn"
                } else {
                    "[Deny] / Allow once\nTab Choose / Enter Deny / Esc Cancel turn"
                }
            }
            PromptKind::Questions(_) if state.submit_review => {
                if !state.reviewed {
                    "[Edit answers] / Send locked until end\nPgUp/Dn Review / Esc Cancel turn"
                } else if state.affirmative {
                    "Edit answers / [Send answers]\nTab Choose / Enter Send / Esc Cancel turn"
                } else {
                    "[Edit answers] / Send answers\nTab Choose / Enter Edit / Esc Cancel turn"
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
        let _ = writeln!(
            value,
            "MODEL CHANGE / {} → {}\n\nThe owned session belongs to the previous model selection. It cannot be silently resumed with another model. New session clears only this role's local resume selection; provider history is not erased. Keep current preserves the owned session and current selection. Back returns to the picker. No process/network is authorized.\n",
            state.selection.model.as_deref().unwrap_or("Default"),
            selection.model.as_deref().unwrap_or("Default")
        );
        value.push_str(if state.affirmative {
            "Keep current / [New session]\nTab Choose / Enter Confirm / Esc Back"
        } else {
            "[Keep current] / New session\nTab Choose / Enter Keep / Esc Back"
        });
    } else {
        let _ = writeln!(
            value,
            "{}",
            super::brief(&format!(
                "{}. Local only; nothing connects on open.\n\nDefault uses provider configuration, not a guessed value.",
                if state.picker == Some(Picker::Model) {
                    "MODELS"
                } else {
                    "REASONING EFFORT"
                }
            ))
        );
        if let Some(catalogue) = state.catalogues.get(&state.agent) {
            let _ = writeln!(
                value,
                "{}",
                super::brief(&format!(
                    "Reported current/default: {}.\n\nCatalogue source: {} (not a turn acknowledgment)",
                    catalogue.current.as_deref().unwrap_or("unobserved"),
                    clipped(&catalogue.source, area.width as usize)
                ))
            );
            if let Some(note) = &catalogue.effort_note {
                let _ = writeln!(value, "{}\n", super::brief(note));
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
        value.push_str(&super::brief("\nUp/Down Choose / Enter Select / Esc Back.\n\nRefresh prepares a separate reviewed connection; Run consent remains required."));
    }
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
    text(
        frame,
        row(area, area.height.saturating_sub(3), 3),
        &super::brief(&state.notice),
        paint.muted,
    );
}

fn switch(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    let next = state.switch_to.expect("switch preview");
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
    if details.height > 0 && state.scroll >= state.scroll_max {
        state.reviewed = true;
    }
    text(
        frame,
        row(area, area.height.saturating_sub(3), 3),
        if !state.reviewed {
            "[Stay] / Switch locked until end\nPgDn/End Review / Esc Stay"
        } else if state.affirmative {
            "Stay / [Switch agent; reset grants]\nTab Choose / Enter Confirm / Esc Stay"
        } else {
            "[Stay] / Switch agent; reset grants\nTab Choose / Enter Stay / Esc Stay"
        },
        paint.accent,
    );
}

fn journal(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
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
        if state.journal_purge && state.affirmative {
            "Keep / [Purge this task's local journal]\nTab Choose / Enter Confirm / Esc Keep"
        } else if state.journal_purge {
            "[Keep] / Purge this task's local journal\nTab Choose / Enter Keep / Esc Keep"
        } else {
            "Up/Down Record / PgUp/Dn Scroll\n/journal-purge /journal-retry /journal-close"
        },
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

fn quit(frame: &mut Frame, area: Rect, state: &State, paint: &Paint) {
    let text_value = if state.outcome_unknown() {
        "UNKNOWN OUTCOME\nAt least one provider request ended without authoritative completion. Work may have run. Later requests or metadata inspection do not resolve it. Exiting does not undo side effects or erase provider history."
    } else {
        "Exit planning?\nCancellation does not undo side effects or erase provider history. Eevee waits for acknowledgement or bounded teardown. Unknown outcome requires a second explicit exit choice."
    };
    text(
        frame,
        row(area, 0, area.height.saturating_sub(5)),
        text_value,
        paint.surface,
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
        "Tab Choose / Enter Confirm / Esc Stay",
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
