use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Clear, Paragraph, Wrap},
};

use super::{Paint, centered, clipped, editor_rows, panel, row, selection_details, text};
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
        "Approved memories / terminal too small\nNeed 60 columns x 18 rows.\nDraft and selection retained.\nEsc Back / Ctrl+C safe quit.\nNo approval or deletion at this size.",
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
    let height = if compact {
        18
    } else {
        area.height.saturating_sub(2)
    };
    let area = centered(area, 110, height);
    frame.render_widget(Clear, area);
    let block = panel(
        " Approved memories / explicit local curation ",
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let heading = if state.saving {
        "SAVING / awaiting committed acknowledgement"
    } else {
        match state.mode {
            Mode::Browse => "INSPECT / only selected approved notes enter context",
            Mode::Edit => "USER NOTE / Enter previews, never approves",
            Mode::Preview => "APPROVAL PREVIEW / no memory saved yet",
            Mode::Delete => "LOCAL DELETION PREVIEW / no deletion yet",
            Mode::Discard => "UNSAVED DRAFT / keep is selected initially",
        }
    };
    text(frame, row(content, 0, 1), heading, paint.accent);
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
    let mut body = row(content, 4, content.height.saturating_sub(7));
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
        let value = match state.mode {
            Mode::Browse if state.show_help => "/create-memory New note / /remember Next-action note\n/edit-memory Edit / /delete-memory Delete\n/select-memory Include in future requests\n/memory-scope Task / Project / Personal\n/verbose Full content and provenance\n\nRight then Enter confirms the displayed scope; changing scope resets consent. No tools run and context selection stays separate. Esc keeps the draft; failed saves retain it. Deletion does not erase exports or provider history.".into(),
            Mode::Preview => state.candidate.as_ref().map_or_else(String::new, |candidate| {
                if super::VERBOSE.get() {
                    describe(candidate, &state.project, state.task_id)
                } else {
                    summary(candidate, false)
                }
            }),
            Mode::Discard => format!("Keep this editable draft or discard it?\n\n{}\n\nDiscard affects only this draft, not existing approved memories.", state.draft),
            Mode::Browse | Mode::Delete => {
                if let Some(record) = state.selected_record() {
                    if state.mode == Mode::Delete && !super::VERBOSE.get() {
                        format!("Memory #{} / revision {}\n{}", record.id, record.revision, summary(&record.proposal, true))
                    } else {
                    format!("Approved memory #{} / revision {}\n{}\n\n{}\n\n{}", record.id, record.revision,
                        if state.selected_ids.contains(&record.id) { "Selected for future request previews" } else { "Not selected for outgoing context" },
                        describe(&record.proposal, &record.project, record.task_id.unwrap_or(state.task_id)), memory::ERASURE_NOTICE)
                    }
                } else {
                    "No approved memories in this scope.\n\n/create-memory authors a note; /remember proposes the task's saved next-action note. Both open a separate scope/approval preview.\n\nProject memories stay in their project; personal memories are visible in every project. /select-memory deliberately selects approved notes for request context.\n\nNo transcript mining or automatic habit inference. Recognizable credentials and excluded captured notes are refused; detection is best-effort.".into()
                }
            }
            Mode::Edit => unreachable!(),
        };
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
        "Waiting for storage / draft retained on failure".into()
    } else {
        match state.mode {
            Mode::Browse => {
                "Up/Down Inspect | / Commands\n/select-memory Context | PgUp/PgDn Scroll | Esc Back"
                    .into()
            }
            Mode::Edit => {
                "Enter Preview | Ctrl+N Newline | Esc Keep/Discard\nLiteral text only.".into()
            }
            Mode::Preview => format!(
                "{} {} | y Approve / n Back\n{} / /memory-scope / /edit-memory",
                if state.affirmative { "Stay" } else { "[Stay]" },
                if state.affirmative {
                    "[Approve]"
                } else {
                    "Approve"
                },
                if state.reviewed {
                    "/verbose Full details"
                } else {
                    "Enlarge to review scope"
                }
            ),
            Mode::Delete => format!(
                "{} {} | y Delete / n Back\n{} / Left/Right Choose / Enter",
                if state.affirmative { "Stay" } else { "[Stay]" },
                if state.affirmative {
                    "[Delete]"
                } else {
                    "Delete"
                },
                if state.reviewed {
                    "/verbose Full details"
                } else {
                    "Enlarge to review scope"
                }
            ),
            Mode::Discard => format!(
                "{} {}\ny Discard draft / n Keep editing",
                if state.affirmative {
                    "Keep editing"
                } else {
                    "[Keep editing]"
                },
                if state.affirmative {
                    "[Discard draft]"
                } else {
                    "Discard draft"
                }
            ),
        }
    };
    text(
        frame,
        row(content, content.height.saturating_sub(3), 3),
        &footer,
        paint.accent,
    );
}

fn summary(proposal: &Proposal, deleting: bool) -> String {
    let scope = match proposal.scope {
        memory::Scope::Task => "This task",
        memory::Scope::Project => "This project",
        memory::Scope::Personal => "All projects",
    };
    format!(
        "{scope}\n{}\n\n{}\n{} bytes / /verbose Full note and source",
        if deleting {
            "Delete locally; prior exports remain."
        } else {
            "Save note only; context selection is separate."
        },
        clipped(proposal.content.lines().next().unwrap_or(""), 100),
        proposal.content.len()
    )
}

fn describe(proposal: &Proposal, project: &str, task_id: i64) -> String {
    let owner = match proposal.scope {
        memory::Scope::Task => format!("Task {task_id} in {project}"),
        memory::Scope::Project => format!("Project: {project}"),
        memory::Scope::Personal => "Personal: every project using this data store".into(),
    };
    format!(
        "SCOPE: {}\nLocal memory only; no tools or grants.\nEXACT CONTENT ({} UTF-8 bytes):\n{}\n\nOWNER: {owner}\nPROVENANCE: {}\nSOURCE: {} (task {})\n\nContext selection is separate. Deletion does not erase prior exports or provider history.",
        proposal.scope.label(),
        proposal.content.len(),
        proposal.content,
        proposal.provenance.kind.label(),
        proposal.provenance.reference,
        proposal.provenance.task_id
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
