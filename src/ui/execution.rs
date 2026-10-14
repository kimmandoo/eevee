//! Real local execution review and evidence inspector. No action is dispatched by rendering.
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};

use super::{Paint, panel, row, selection_details, text};
use crate::{
    app::execution::{self, Confirmation, State, View},
    config::Config,
    domain::FORMS,
};

pub(super) fn render_small_confirmation(
    frame: &mut Frame,
    area: Rect,
    state: &mut State,
    config: &Config,
) {
    let paint = Paint::new(config, &FORMS[2], 2);
    frame.render_widget(Clear, area);
    if matches!(state.confirmation, Some(Confirmation::Approval)) {
        text(
            frame,
            area,
            "Resize to 60x18 to review approval.\nNo command can be approved here.\nEsc closes; /deny denies; Ctrl+C reviews exit.",
            paint.surface,
        );
        return;
    }
    let body = row(area, 0, area.height.saturating_sub(2));
    state.set_viewport(body.height.saturating_sub(3) as usize);
    confirmation(frame, body, state, &paint);
    text(
        frame,
        row(area, area.height.saturating_sub(2), 2),
        "Left Stay / Right action / Enter\nPgUp/Dn details / Esc Stay",
        paint.muted,
    );
}

pub fn render(frame: &mut Frame, area: Rect, state: &mut State, config: &Config) {
    if !state.visible() {
        return;
    }
    let paint = Paint::new(config, &FORMS[2], 2);
    frame.render_widget(Clear, area);
    let block = panel(" Jolteon · local command evidence ", config, &paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    if content.height < 4 || content.width < 12 {
        text(
            frame,
            content,
            "Execution state retained. Resize; Esc stays; Ctrl+C requests neutral cancel-and-exit.",
            paint.surface,
        );
        return;
    }
    let status = state.status();
    text(
        frame,
        row(content, 0, 1),
        &execution::safe_text(&status),
        paint.accent,
    );
    let banner = if let Some(error) = &state.error {
        format!(
            "{} error · /artifacts details · /journal-retry storage only",
            error.layer
        )
    } else if state.searching {
        format!(
            "Search current window/page: /{}  Enter finds · Esc keeps",
            state.search
        )
    } else {
        state.notice.clone()
    };
    text(
        frame,
        row(content, 1, 2),
        &execution::safe_text(&banner),
        paint.muted,
    );
    let footer_rows = if content.height >= 12 { 3 } else { 2 };
    let body_height = content.height.saturating_sub(3 + footer_rows);
    let body = row(content, 3, body_height);
    state.set_viewport(body.height as usize);
    if state.confirmation.is_some() {
        confirmation(frame, body, state, &paint);
    } else {
        match state.view {
            View::History => history(frame, body, state, &paint),
            View::Details => details(frame, body, state, &paint),
            View::Output => output(frame, body, state, &paint),
        }
    }
    let footer = row(
        content,
        content.height.saturating_sub(footer_rows),
        footer_rows,
    );
    let hints = if state.confirmation.is_some() {
        "Left Stay · Right/Tab select action · Enter confirm\nUp/Down/PgUp/PgDn inspect full details · Esc Stay\n/deny explicitly denies; closing never approves"
    } else if state.view == View::History {
        "Up/Down select · Enter inspect retained output\n/ choices: purge, command-edit, run, cancel, refresh\nEsc output · Ctrl+C/quit: Stay or Cancel-and-wait"
    } else if state.view == View::Details {
        "Up/Down/PgUp/PgDn/Home/End inspect full details\n/ choices: journal-retry, run, cancel · Esc output\nNo command is retried automatically; task drafts stay"
    } else {
        "/ choices: command-edit, run, cancel, history, artifacts\nArrows scroll · /search edits a literal output query\n/page-next, /page-previous · /refresh · Esc close"
    };
    text(frame, footer, hints, paint.muted);
}

fn confirmation(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    let Some(confirm) = state.confirmation.as_ref() else {
        return;
    };
    let (title, affirmative, description) = match confirm {
        Confirmation::Approval => (
            "Review one-shot execution permission", "Approve this exact command",
            state.prepared.as_ref().map(|p| execution::approval_details(p, state.capture_capacity))
                .unwrap_or_else(|| "This request was invalidated. Return and prepare a new command.".into()),
        ),
        Confirmation::Cancel => (
            "Cancel owned work / deny unstarted approval", "Cancel and wait",
            "Cancellation waits for owned-tree exit with bounded escalation. It does not roll back filesystem edits or network effects. Partial output and the task's command draft remain. An unstarted request is denied instead; it will not execute.\n\nIf termination cannot be established, the journal records outcome unknown rather than success.\n\n".to_owned() + &state.details(),
        ),
        Confirmation::Purge(run) => (
            "Purge retained local output", "Purge this run's output",
            format!("Run {}\nOnly this run's owned local output file is removed. Metadata remains explicitly unavailable and cannot substantiate a verified result. Other tasks' records/artifacts remain.\n\nThis is NOT secure erasure of SQLite/WAL/backups and does not remove repository changes or provider-owned data. Active work is refused by storage.\n\n{}", execution::safe_text(run), state.details()),
        ),
        Confirmation::Navigate(action) => (
            "Stay with owned work, or cancel and wait", if *action == execution::Action::Quit { "Cancel and exit after saving" } else { "Cancel and wait, then navigate" },
            format!("Requested: {action:?}\nStay keeps this task and all drafts. Cancel-and-wait first denies any unstarted approval or cancels the owned run, then waits for process-tree exit, after-source observation, and committed journal metadata.\n\nNo second keypress silently forces termination. Cancellation is not rollback. The parent's normal unsaved/failed-draft save review still follows.\n\n{}", state.details()),
        ),
        Confirmation::UnknownExit(action) => (
            "Outcome unknown — explicit informed exit required", "Exit knowing effects are unknown",
            format!("Requested: {action:?}\nOwned-tree/output closure could not be established. Effects may remain, and cancellation did NOT roll back edits. Partial journal evidence has been retained. This is not success or verified cancellation.\n\nStay to inspect. Only an explicit Quit acknowledgement may leave this unresolved ownership outcome; task switching cannot establish termination. Draft/save handling still follows.\n\n{}", state.details()),
        ),
        Confirmation::UnsavedJournalExit(action) => (
            "Process ended, but journal persistence failed", "Exit without saved final journal",
            format!("Requested: {action:?}\nThe final execution record is retained only in memory. On restart, the last committed reservation/run will be interrupted, not magically completed. Spool bytes may remain partial; no command will be re-executed.\n\nStay and use /journal-retry to retry ONLY storage. Leaving knowingly forfeits the unsaved final metadata, not the task's independent draft/save review.\n\n{}", state.details()),
        ),
    };
    text(frame, row(area, 0, 1), title, paint.accent);
    let choices = Line::from(vec![
        Span::styled(
            " Stay / return ",
            if state.affirmative {
                paint.surface
            } else {
                paint.selected
            },
        ),
        Span::raw("  "),
        Span::styled(
            format!(" {affirmative} "),
            if state.affirmative {
                paint.selected
            } else {
                paint.surface
            },
        ),
    ]);
    frame.render_widget(
        Paragraph::new(choices).wrap(Wrap { trim: false }),
        row(area, 1, 2),
    );
    let body = row(area, 3, area.height.saturating_sub(3));
    let paragraph = Paragraph::new(description)
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    state.detail_max = selection_details(frame, body, paragraph, state.detail_scroll);
    state.detail_scroll = state.detail_scroll.min(state.detail_max);
}

fn details(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    let paragraph = Paragraph::new(state.details())
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    state.detail_max = selection_details(frame, area, paragraph, state.detail_scroll);
    state.detail_scroll = state.detail_scroll.min(state.detail_max);
}

fn history(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    if state.history.is_empty() {
        text(
            frame,
            area,
            "No retained executions for this task. /command-edit edits a command; /run reviews a prepared request. Loading history never executes a saved command. Interrupted startup records are evidence, not live process ownership.",
            paint.muted,
        );
        return;
    }
    let rows = (area.height as usize / 3).max(1);
    let start = state.history_index.saturating_sub(rows.saturating_sub(1));
    for (slot, (index, record)) in state
        .history
        .iter()
        .enumerate()
        .skip(start)
        .take(rows)
        .enumerate()
    {
        let line = format!(
            "{} {:?} · task {} · ended {:?}\n{}\n{}",
            if index == state.history_index {
                ">"
            } else {
                " "
            },
            record.state,
            record.prepared.task_id,
            record.ended_at_ms,
            record.prepared.run_id,
            execution::evidence_label(record, &state.latest)
        );
        text(
            frame,
            row(area, (slot * 3) as u16, 3),
            &execution::safe_text(&line),
            if index == state.history_index {
                paint.selected
            } else {
                paint.surface
            },
        );
    }
}

fn output(frame: &mut Frame, area: Rect, state: &mut State, paint: &Paint) {
    let label = if state.showing_retained {
        if let Some(page) = &state.retained_page {
            format!(
                "Retained bytes {}..{} / {}{}{} · [ ] pages",
                page.offset,
                page.offset + page.text.len() as u64,
                page.total_bytes,
                if page.unavailable {
                    " · UNAVAILABLE"
                } else {
                    ""
                },
                if page.truncated { " · TRUNCATED" } else { "" }
            )
        } else {
            "Loading retained output".into()
        }
    } else {
        format!(
            "{} · {} new lines · {} live lines evicted · column {}",
            if state.follow {
                "Follow ON"
            } else {
                "Follow paused"
            },
            state.new_output,
            state.live.evicted,
            state.horizontal
        )
    };
    let warning = if state.showing_retained {
        ""
    } else {
        match state.active.as_ref() {
            Some(record) if record.output_unavailable => "OUTPUT UNAVAILABLE · inspect details",
            Some(record) if record.truncated => "CAPTURE TRUNCATED · excess drained/discarded",
            Some(record) if record.dropped_live_bytes > 0 => {
                "LIVE QUEUE GAP · inspect retained output"
            }
            _ => "",
        }
    };
    text(
        frame,
        row(area, 0, if warning.is_empty() { 2 } else { 1 }),
        &label,
        paint.muted,
    );
    if !warning.is_empty() {
        text(frame, row(area, 1, 1), warning, paint.accent);
    }
    let body = row(area, 2, area.height.saturating_sub(2));
    state.set_viewport(body.height as usize);
    if state.showing_retained
        && state
            .retained_page
            .as_ref()
            .is_some_and(|page| page.unavailable)
    {
        text(
            frame,
            body,
            "Output unavailable (purged, missing, or replaced). Empty display is NOT an empty successful command. /artifacts shows the retained metadata and source-identity limits.",
            paint.accent,
        );
        return;
    }
    if state.window().lines.is_empty() {
        text(
            frame,
            body,
            "No captured output in this window yet.\n\nDirect example: [\"cargo\",\"test\"]\nExplicit shell: {\"shell\":\"/bin/sh\",\"dialect\":\"posix\",\"text\":\"printf hello\"}\n\n/command-edit opens the task command editor. Preparing never executes; /run review is initially neutral. /history opens retained evidence. There are no model calls or automatic retries.",
            paint.muted,
        );
        return;
    }
    let lines: Vec<Line<'_>> = state
        .window()
        .lines
        .iter()
        .filter(|line| line.id >= state.anchor)
        .take(body.height as usize)
        .map(|line| {
            let style = if !state.search.is_empty() && line.text.contains(&state.search) {
                paint.selected
            } else {
                paint.surface
            };
            Line::from(Span::styled(line.text.as_str(), style))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .style(paint.surface)
            .scroll((0, state.horizontal)),
        body,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        Terminal,
        backend::TestBackend,
        style::{Modifier, Style},
    };

    fn paint() -> Paint {
        Paint {
            base: Style::default(),
            surface: Style::default(),
            muted: Style::default(),
            border: Style::default(),
            accent: Style::default().add_modifier(Modifier::BOLD),
            selected: Style::default().add_modifier(Modifier::REVERSED),
        }
    }
    fn prepared() -> crate::runner::Prepared {
        crate::runner::Prepared {
            request_id: "r-review".into(),
            run_id: "r-review-run".into(),
            task_id: 1,
            spec: crate::runner::CommandSpec::Direct {
                program: "tool".into(),
                args: vec![
                    "中文 long exact argument ".repeat(120),
                    "\u{1b}]52;not-a-terminal-operation".into(),
                ],
            },
            cwd: "/project 中文".into(),
            cwd_identity: "directory-id".into(),
            executable: crate::runner::ExecutableIdentity {
                path: "/tool path".into(),
                sha256: "hash".into(),
                bytes: 1,
                modified_ns: 1,
                file_id: "file-id".into(),
            },
            prepared_at_ms: 1,
            expires_at_ms: 100,
            before: crate::runner::InputSnapshot::default(),
        }
    }

    #[test]
    fn compact_approval_is_neutral_and_full_exact_details_remain_scrollable() {
        let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
        let mut state = State::default();
        state.prepared = Some(prepared());
        state.confirmation = Some(Confirmation::Approval);
        terminal
            .draw(|frame| {
                let area = frame.area();
                confirmation(frame, area, &mut state, &paint());
            })
            .unwrap();
        assert!(!state.affirmative);
        assert!(state.detail_max > 0);
        assert!(
            terminal.backend().buffer()[(0, 1)]
                .modifier
                .contains(Modifier::REVERSED)
        );
        state.detail_scroll = state.detail_max;
        terminal
            .draw(|frame| {
                let area = frame.area();
                confirmation(frame, area, &mut state, &paint());
            })
            .unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("headroom"));
        assert!(!rendered.contains('\u{1b}'));
    }

    #[test]
    fn unavailable_retained_output_never_renders_as_empty_success() {
        let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
        let mut state = State::default();
        state.showing_retained = true;
        state.retained_page = Some(crate::store::OutputPage {
            offset: 0,
            text: String::new(),
            total_bytes: 100,
            unavailable: true,
            truncated: true,
        });
        terminal
            .draw(|frame| {
                let area = frame.area();
                output(frame, area, &mut state, &paint());
            })
            .unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("UNAVAILABLE") && rendered.contains("TRUNCATED"));
        assert!(rendered.contains("NOT an empty"));
    }
}
