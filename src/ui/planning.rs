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
    provider::{ItemKind, Operation, PromptKind},
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
    let title = format!(
        " {} / {} {} / selected model {} / selected effort {} ",
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
        state.selection.model.as_deref().unwrap_or("Default"),
        state.selection.effort.as_deref().unwrap_or("Default")
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
    let header = if state.preview.is_some() {
        "LOCAL PREVIEW / no provider connection authorized"
    } else if state.request.is_some() {
        "PROVIDER REQUEST / explicit response required"
    } else if state.cancelling {
        "CANCELLING / waiting; outcome not yet known"
    } else if state.busy() {
        if state.inspect_models {
            "INSPECTING MODELS / no model turn authorized"
        } else if state.inspect_sessions {
            "INSPECTING SESSION METADATA / no model turn authorized"
        } else {
            match state.operation {
                Operation::Plan => "PLANNING / provider-owned history and billing",
                Operation::Implement => "IMPLEMENTING / reviewed write boundary",
                Operation::Review => "REVIEWING / separate session; source writes unavailable",
            }
        }
    } else if state.inspection.is_some() {
        "SESSION METADATA / not a selectable decision or model result"
    } else if state.outcome_unknown() {
        "IDLE / prior provider outcome UNKNOWN; effects may remain"
    } else if state.operation == Operation::Review {
        "IDLE / New independent Review; Resume unavailable"
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
    let _ = writeln!(value, "{}\n", state.selection_label());
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
        if !state.inspect_sessions && !state.inspect_models {
            let _ = writeln!(
                value,
                "BUDGET: {} of {} provider turns journaled for this task{}",
                state
                    .budget_turns
                    .map(|turns| turns.to_string())
                    .unwrap_or_else(|| "loading".into()),
                state.limits.provider_turns,
                if state.budget_paused() {
                    " / PAUSED: /budget-continue lifts this preview only; Stay to revise"
                } else {
                    ""
                }
            );
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
    } else {
        if state.operation == Operation::Review {
            value.push_str("/new: independent Review in a fresh provider session, using current requirements, working patch and selected observations only. No implementation transcript or portable handoff is reused. Source-writing tools, Resume and edit grants are unavailable. ACP Review is unavailable because no-write tool policy cannot be established. Applying a fix requires a deliberate implementation handoff from the review desk.\n\n");
        } else {
            value.push_str("/new: new provider thread. /resume: this task's last explicitly created compatible thread only; provider history is additional context. Both open a local preview first. No executable is launched by opening this drawer or a model picker.\n\n");
        }
        if let Some(id) = &state.thread_id {
            let _ = writeln!(
                value,
                "Session-local {}thread: {id}\n",
                if state.operation == Operation::Review {
                    "non-resumable review "
                } else {
                    "resumable "
                }
            );
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
                "ITEM {}/{} / {kind} / {}\nSource: {} / thread {}\nTurn {} / item {}\n\n{}",
                state.selected + 1,
                state.items.len(),
                if item.complete {
                    "complete"
                } else {
                    "streaming"
                },
                state.agent.label(),
                item.thread_id,
                item.turn_id,
                item.item_id,
                item.text
            );
            value.push_str("\n\nDecision selection copies only a complete message/plan into the NextAction editor. Review and explicitly save it. It is an unverified provider proposal, not command success, a saved review verdict, or task acceptance.\n");
        } else {
            if state.operation == Operation::Review {
                value.push_str("Review compares saved acceptance criteria with the current working patch and selected evidence. Missing, stale or ambiguous evidence must be labelled inconclusive, never passed. Findings must name affected source/run references and a proposed next action; returning a response does not mark the task complete.\n\nNo review items yet. New opens a local outgoing-context preview before any provider launch.\n");
            } else {
                value.push_str("Current saved goal, acceptance criteria and selected source basket are included in the preview. An empty basket is goal-only planning. Save the goal first (/goal); use /sources before starting to change the basket.\n\nNo provider items yet. Item display is bounded and source-qualified; raw private reasoning is not displayed.\n");
            }
            let _ = writeln!(
                value,
                "\nAgent: {} / Operation: {:?}\n{}",
                state.agent.label(),
                state.operation,
                if state.operation == Operation::Review {
                    "No write paths or grants. /provider reviews provider switch; /handoff opens portable handoff.".into()
                } else {
                    format!(
                        "Selected edit paths: {:?}\n/provider reviews provider switch; /role chooses role; /paths edits selected paths; /handoff opens portable handoff.",
                        state.edit_paths
                    )
                }
            );
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
    if state.inspection.is_some() {
        return if state.busy() {
            "PgUp/Dn Scroll session metadata\nEsc Cancel / /quit Safe exit"
        } else {
            "PgUp/Dn Scroll session metadata / Esc Messages\n/new /resume /provider /history"
        };
    }
    if state.busy() {
        "Up/Down Item / PgUp/Dn Scroll\nEsc Cancel / /quit Safe exit"
    } else if state.operation == Operation::Review {
        "/goal /new /decision /provider /model /effort\n/history /handoff /commands\nResume/writes unavailable / Esc Back"
    } else {
        "/goal /new /resume /decision /provider /model /effort\n/role /paths /history /sessions /handoff\n/commands Choices / Esc Back"
    }
}

fn picker(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    let mut value = format!("{}\n\n", state.selection_label());
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
            "{} / LOCAL ONLY / no connection on open\nDefault uses provider configuration, not a guessed value.",
            if state.picker == Some(Picker::Model) {
                "MODELS"
            } else {
                "REASONING EFFORT"
            }
        );
        if let Some(catalogue) = state.catalogues.get(&state.agent) {
            let _ = writeln!(
                value,
                "Catalogue source: {}\nReported current/default: {} (not a turn acknowledgment)",
                clipped(&catalogue.source, area.width as usize),
                catalogue.current.as_deref().unwrap_or("unobserved")
            );
            if let Some(note) = &catalogue.effort_note {
                let _ = writeln!(value, "{note}\n");
            }
        }
        let visible = area.height.saturating_sub(11).max(1) as usize;
        let start = state
            .picker_selected
            .saturating_sub(visible.saturating_sub(1) / 2);
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
        value.push_str("\nUp/Down Choose / Enter Select / Esc Back\nRefresh prepares a separate reviewed connection; Run consent remains required.");
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
        &state.notice,
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
        &state.journal_notice,
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
        &state.notice,
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
