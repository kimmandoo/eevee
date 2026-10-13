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
    app::{App, planning::State},
    codex::{ItemKind, PromptKind},
    config::Config,
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
    let state = &mut app.planning;
    let block = panel(" Espeon / Codex planning ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    if state.quit_review {
        quit(frame, content, state, paint);
        return;
    }
    let header = if state.preview.is_some() {
        "LOCAL PREVIEW / no provider connection authorized"
    } else if state.request.is_some() {
        "PROVIDER REQUEST / explicit response required"
    } else if state.cancelling {
        "CANCELLING / waiting; outcome not yet known"
    } else if state.busy() {
        "PLANNING / provider-owned history and billing"
    } else {
        "IDLE / explicit New or same-task Resume"
    };
    text(
        frame,
        row(content, 0, 1),
        &clipped(header, content.width as usize),
        paint.accent,
    );
    let mut value = String::new();
    let mut review = false;
    if let Some(preview) = &state.preview {
        let _ = writeln!(
            value,
            "Task {} / {}\n",
            preview.task_id,
            preview
                .thread_id
                .as_ref()
                .map(|id| format!("RESUME provider thread {id}"))
                .unwrap_or_else(|| "NEW provider thread".into())
        );
        value.push_str("EXECUTABLE TRUST AND PROVIDER EFFECTS\n");
        for line in &preview.trust {
            let _ = writeln!(value, "{line}");
        }
        value.push_str("\nEevee sends the exact payload below. It is NOT the complete effective provider prompt: provider configuration, instructions, tools/MCP, hooks and resumed history may add context or effects. Provider owns authentication, usage charges and retained thread history. Read-only policy is agent-controlled, not proof of OS isolation. No always-allow grant or automatic replay.\n");
        for blocker in &preview.blockers {
            let _ = writeln!(value, "\nBLOCKER: {blocker}");
        }
        value.push_str("\nEXACT OUTGOING PAYLOAD BEGIN\n");
        value.push_str(&preview.payload);
        value.push_str("\nEXACT OUTGOING PAYLOAD END\n\nEND OF REVIEW. Run authorizes this executable and one planning turn only. Stay sends nothing.\n");
        review = true;
    } else if let Some(request) = &state.request {
        let _ = writeln!(
            value,
            "Source: Codex / thread {}\nTurn {} / item {}\nRequest {}\n",
            request.thread_id, request.turn_id, request.item_id, request.request_id
        );
        match &request.kind {
            PromptKind::Approval {
                command,
                cwd,
                reason,
                deny_decision,
                ..
            } => {
                let _ = writeln!(
                    value,
                    "COMMAND APPROVAL\nCommand: {command}\nWorking directory: {cwd}\nReason: {reason}\n\nAllow once applies ONLY to this server request. Eevee denies file-edit approvals. Provider policy/sandbox and callback coverage are not OS isolation. Never allow a command you do not understand.\n\nEND OF REQUEST."
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
    } else {
        value.push_str("New (n): new provider thread. Resume (r): this task's last explicitly created thread only; provider history is additional context. Both open a local preview first. No executable is launched by opening this drawer.\n\n");
        if let Some(id) = &state.thread_id {
            let _ = writeln!(value, "Session-local resumable thread: {id}\n");
        }
        if state.omitted > 0 {
            let _ = writeln!(
                value,
                "Display bound: {} earlier items omitted; no local execution evidence is implied.\n",
                state.omitted
            );
        }
        if let Some(item) = state.items.get(state.selected) {
            let kind = match item.kind {
                ItemKind::Message => "agent message",
                ItemKind::Plan => "plan",
                ItemKind::Tool => "tool output (not a decision)",
            };
            let _ = writeln!(
                value,
                "ITEM {}/{} / {kind} / {}\nSource: Codex / thread {}\nTurn {} / item {}\n\n{}",
                state.selected + 1,
                state.items.len(),
                if item.complete {
                    "complete"
                } else {
                    "streaming"
                },
                item.thread_id,
                item.turn_id,
                item.item_id,
                item.text
            );
            value.push_str("\n\nDecision selection copies only a complete message/plan into the NextAction editor. Review and explicitly save it. It is unverified provider planning, not command success or acceptance evidence.\n");
        } else {
            value.push_str("Current saved goal, acceptance criteria and selected source basket are included in the preview. An empty basket is goal-only planning. Save the goal first (g); use the Sources drawer before starting to change the basket.\n\nNo provider items yet. Item display is bounded and source-qualified; raw private reasoning is not displayed.\n");
        }
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
        &state.notice,
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
                "Up/Down Option / Enter Choose / a Type\nTab Question / s Review send / Esc Cancel turn"
            }
        };
    }
    if state.busy() {
        "Up/Down Item / PgUp/Dn Scroll\nEsc or c Cancel / q Safe exit"
    } else {
        "g Goal / n New / r Resume / d Select decision\nUp/Down Item / PgUp/Dn Scroll / Esc Back"
    }
}

fn quit(frame: &mut Frame, area: Rect, state: &State, paint: &Paint) {
    let text_value = if state.outcome_unknown() {
        "UNKNOWN OUTCOME\nNo authoritative provider completion was received. Work may have run. Exiting does not undo side effects or erase provider history."
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
        &state.notice,
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
