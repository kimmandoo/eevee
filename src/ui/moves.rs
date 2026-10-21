//! Move presentation is a pure view of local state; rendering never loads files or runs work.
use super::{Paint, centered, clipped, editor_rows, panel, row, selection_details, text};
use crate::{
    app::{
        App,
        moves::{Learning, Mode, State, kind, usage_outcome},
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
            "Moves: need 60 x 18\nDraft retained. Resize to review or change moves.\nEsc Back / Ctrl+C quit",
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
    let block = panel(" Moves / local reusable actions ", config, paint, true);
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
            "{} / {} / {} / {} of 4 slots",
            partner,
            form.name,
            form.element,
            app.moves.active().count()
        ),
        paint.accent,
    );
    let notice = if app.moves.failed.is_some() {
        app.moves.failed_error.as_str()
    } else if app.moves.persistence_pending() {
        "Loading/saving local move facts. No action is approved."
    } else if app.moves.notice.is_empty() {
        "Names are labels only. Every use needs its normal review."
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
                "Discard this unsaved move change or usage receipt?\nIt will not be recorded. Local execution journal stays.\nStay is neutral. Exact failed request is below."
            } else {
                "Move storage failed; the exact request is retained.\nNo automatic retry or silent discard.\nRetry unchanged, or review what discarding would lose."
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
        text(
            frame,
            footer,
            if !app.moves.failure_discard {
                "r Retry exact request / d Review discard / Esc Stay\nPgUp/PgDn Exact request / Enter Stay"
            } else if app.moves.affirmative {
                "Stay / [Discard unsaved request] / Enter Discard\nLeft/Right Choose / Esc Stay / PgUp/PgDn Details"
            } else {
                "[Stay] / Discard unsaved request / Enter Stay\nLeft/Right Choose / y Discard / Esc Stay"
            },
            paint.accent,
        );
        return;
    }
    match app.moves.mode {
        Mode::Menu => {
            let active = app.moves.active();
            if active.clone().next().is_none() {
                text(
                    frame,
                    body,
                    "No moves in this form yet.\n/teach-move: teach from approved local work, selected pins, a sent user-authored request, or the task card.\nMove Reminder keeps forgotten and postponed moves.",
                    paint.surface,
                );
            } else if wide {
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
                        "{} {}",
                        if index == app.moves.selected {
                            ">"
                        } else {
                            " "
                        },
                        clipped(&record.proposal.name, cell.width.saturating_sub(3) as usize)
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
                "Arrows Choose / Enter Review/use / d Details\n t Teach / r Reminder / Esc Back (draft retained)",
                paint.accent,
            );
        }
        Mode::Name => render_name(frame, body, footer, &app.moves, paint),
        Mode::Detail => {
            let details = app
                .moves
                .selected_record()
                .map(|record| record_details(record, &app.moves))
                .unwrap_or_else(|| "Move no longer available. Esc to return.".into());
            render_details(frame, body, &details, &mut app.moves, paint);
            let actions = [
                "Use (normal review)",
                "Rename",
                "Forget",
                "Teach to partner",
            ];
            let selected = app.moves.selected.min(actions.len() - 1);
            text(
                frame,
                footer,
                &format!(
                    "Up/Down Action: > {} / Enter\nPgUp/PgDn Details / u Use / r Rename / f Forget / t Teach / Esc",
                    actions[selected]
                ),
                paint.accent,
            );
        }
        Mode::Reminder => {
            let forgotten_count = app.moves.reminders().count();
            let postponed_count = app.moves.postponed().count();
            let mut lines = vec![
                "Forgotten / postponed in this partner and form. Relearn revalidates exact steps."
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
                lines.push("No forgotten or postponed moves.".into());
            }
            render_selection(frame, body, &lines, app.moves.selected + 1, paint);
            text(
                frame,
                footer,
                "Up/Down Choose / Enter Relearn or inspect candidate\nEsc Back / no approvals are restored",
                paint.accent,
            );
        }
        Mode::Learn => {
            let details = app.moves.offered.as_ref().map(|candidate| format!("{} wants to learn a move!\n{} observed sources (not a correctness verdict)\n{}\nSources:\n{}", partner, candidate.uses, proposal_details(&candidate.proposal), candidate.sources.iter().map(|s| format!("- {s}")).collect::<Vec<_>>().join("\n"))).unwrap_or_else(|| "Candidate unavailable.".into());
            render_details(frame, body, &details, &mut app.moves, paint);
            let actions = [
                "Not now (offer after 3 more uses)",
                "Learn (choose name)",
                "Never for this action",
            ];
            text(
                frame,
                footer,
                &format!(
                    "Up/Down > {} / Enter\nPgUp/PgDn Sources / Esc Not now",
                    actions[app.moves.selected.min(2)]
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
                "Copy exact steps into the target's same form. Source copy stays unchanged.".into(),
            ];
            for (index, target) in targets.iter().enumerate() {
                lines.push(format!(
                    "{} {}",
                    marker(index, app.moves.selected),
                    App::partner_label(target)
                ));
            }
            if targets.is_empty() {
                lines.push("No other party members. Add a partner in /party first.".into());
            }
            render_selection(frame, body, &lines, app.moves.selected + 1, paint);
            text(
                frame,
                footer,
                "Up/Down Target / Enter Review copy / Esc Cancel\nTarget full? Choose an exact move to forget, or Give up.",
                paint.accent,
            );
        }
        Mode::LoadingTarget => {
            text(
                frame,
                body,
                "Loading the chosen partner's actual move slots.\nNo copy, forgetting, or execution has happened.",
                paint.surface,
            );
            text(
                frame,
                footer,
                "Esc Back / wait for the local store",
                paint.accent,
            );
        }
        Mode::Replace => {
            let active = app.moves.active();
            let mut lines = vec![
                "Four slots are occupied across project and personal scope.".into(),
                format!(
                    "{} Give up learning (changes nothing)",
                    marker(0, app.moves.selected)
                ),
            ];
            for (index, record) in active.enumerate() {
                lines.push(format!(
                    "{} Forget {} / {} / revision {}",
                    marker(index + 1, app.moves.selected),
                    record.proposal.name,
                    record.proposal.scope.label(),
                    record.revision
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
                "Up/Down Choose / Enter Review exact replacement\nEsc Give up / nothing changes until explicit confirmation",
                paint.accent,
            );
        }
        Mode::Review | Mode::Forget => {
            let (summary, details) = review_details(&app.moves, &partner);
            // Effect and owner stay visible while long exact steps are scrolled.
            let summary = summary
                .lines()
                .map(|line| clipped(line, body.width as usize).into_owned())
                .collect::<Vec<_>>()
                .join("\n");
            text(frame, row(body, 0, 3), &summary, paint.accent);
            let detail_area = row(body, 3, body.height.saturating_sub(3));
            render_details(frame, detail_area, &details, &mut app.moves, paint);
            app.moves.reviewed = body.height >= 4;
            if !app.moves.reviewed {
                app.moves.affirmative = false;
            }
            let controls = if !app.moves.reviewed {
                "Resize to see the local change summary\nEnter Stay / Esc Cancel (no change)"
                    .to_owned()
            } else if app.moves.affirmative {
                "Stay / [Confirm local change] / Enter Confirm\nLeft/Right Choose / PgUp/PgDn Details / Esc Cancel".into()
            } else {
                "[Stay] / Confirm local change / Enter Stay\nLeft/Right Choose / y Confirm / Esc Cancel".into()
            };
            text(frame, footer, &controls, paint.accent);
        }
        Mode::TaskReview => {
            text(
                frame,
                row(body, 0, 3),
                "Replace this task's goal and criteria.\nNo tools or model request. Apply changes both fields.\nFull template and source facts are below.",
                paint.accent,
            );
            let details = app
                .moves
                .selected_record()
                .map(|r| proposal_details(&r.proposal))
                .unwrap_or_else(|| "Move unavailable.".into());
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
            text(
                frame,
                footer,
                if !app.moves.reviewed {
                    "Resize to review the task change / Enter Stay\nEsc Cancel / no fields changed"
                } else if app.moves.affirmative {
                    "Stay / [Apply template] / Enter Apply\nLeft/Right Choose / Esc Cancel / PgUp/PgDn Details"
                } else {
                    "[Stay] / Apply template / Enter Stay\nLeft/Right Choose / y Apply / PgUp/PgDn Details"
                },
                paint.accent,
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
}
fn render_name(frame: &mut Frame, body: Rect, footer: Rect, state: &State, paint: &Paint) {
    let learning = state.learning.as_ref();
    let action = if matches!(learning, Some(Learning::Rename(_))) {
        "Rename (label only)"
    } else {
        "Move name"
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
    text(
        frame,
        row(body, body.height.saturating_sub(2), 2),
        &format!(
            "{} Scope: {}{}\nNames never enter provider packets or grant approval.",
            if state.scope_focus { ">" } else { " " },
            state.scope.label(),
            if matches!(learning, Some(Learning::Teach(_))) {
                " (Left/Right changes scope)"
            } else {
                " (unchanged)"
            }
        ),
        paint.muted,
    );
    text(
        frame,
        footer,
        "Type name / Tab Name or Scope / Enter Review\nEsc Give up (no change) / CJK and grapheme-safe editing",
        paint.accent,
    );
}

pub(super) fn proposal_details(proposal: &Proposal) -> String {
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
            text.push_str(&format!("Exact argv: {argv:?}\nProject-relative cwd: {cwd}\nNormal command preparation and approval are required.\n"));
        }
        Steps::Pins { pins } => {
            text.push_str(
                "Pins are resolved again at use; unavailable or excluded files are refused.\n",
            );
            for pin in pins {
                text.push_str(&format!(
                    "{}:{}-{}\n",
                    pin.path, pin.start_line, pin.end_line
                ));
            }
        }
        Steps::Request { instruction, pins } => {
            text.push_str("User-authored instruction only (not provider output):\n");
            text.push_str(instruction);
            text.push_str("\nContext selectors:\n");
            for pin in pins {
                text.push_str(&format!(
                    "{}:{}-{}\n",
                    pin.path, pin.start_line, pin.end_line
                ));
            }
            text.push_str("Normal full request receipt is required before dispatch.\n");
        }
        Steps::Task { goal, criteria } => {
            text.push_str(&format!("Goal:\n{goal}\nCriteria:\n"));
            for criterion in criteria {
                text.push_str(&format!("- {criterion}\n"));
            }
        }
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
fn record_details(record: &Record, state: &State) -> String {
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
    let mut text = format!(
        "{}\nMove #{} / revision {} / {}\nUses: {} / {}/{} exited 0\nExit 0 is not correctness. No power, accuracy, or level.\nDenied: {} / Cancelled: {} / Unknown: {}\nPending: {} / Completed without an exit verdict: {}\n",
        proposal_details(&record.proposal),
        record.id,
        record.revision,
        if record.forgotten {
            "forgotten"
        } else {
            "active"
        },
        usages.clone().count(),
        exited_zero,
        exited_zero + failed,
        count(Outcome::Denied),
        count(Outcome::Cancelled),
        count(Outcome::Unknown),
        count(Outcome::Pending),
        completed
    );
    if let Some(last) = state.last_usage(record.id) {
        text.push_str(&format!(
            "Last: {} / revision {}\n",
            usage_outcome(last),
            last.revision.as_deref().unwrap_or("unknown")
        ));
    }
    if usages.clone().next().is_none() {
        text.push_str("No journal-linked uses recorded.\n");
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
fn review_details(state: &State, partner: &str) -> (String, String) {
    if state.mode == Mode::Forget {
        return state.selected_record().map(|record| (format!("Forget {} / move #{}.\nOwner: {} / label: {}\nIdentical steps stay in Reminder; no execution.", record.proposal.scope.label(), record.id, clipped(partner, 16), clipped(&record.proposal.name, 20)), record_details(record, state))).unwrap_or_else(|| ("Move unavailable.".into(), String::new()));
    }
    let Some(learning) = &state.learning else {
        return ("No change selected.".into(), String::new());
    };
    let mut proposal = learning.proposal().clone();
    proposal.name.clone_from(&state.name);
    proposal.scope = state.scope;
    let action = match learning {
        Learning::Teach(_) => "Teach",
        Learning::Relearn(_) => "Relearn identical steps",
        Learning::Rename(_) => "Rename label",
    };
    let mut summary = format!(
        "{action} / {} / {}.\nNo execution or approvals; exact facts below.",
        state.scope.label(),
        clipped(partner, 16)
    );
    let mut details = proposal_details(&proposal);
    if let Some((id, revision)) = state.replacement
        && let Some(record) = state.records.iter().find(|r| r.id == id)
    {
        summary.push_str(&format!(
            "\nForgets {} move #{} into Reminder.",
            record.proposal.scope.label(),
            record.id
        ));
        details.push_str(&format!(
            "\nEXACT REPLACEMENT: move #{id}, revision {revision}\n{}",
            proposal_details(&record.proposal)
        ));
    }
    (summary, details)
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
        let preview = proposal_details(&record().proposal);
        assert!(preview.contains("src/lib.rs:2-4"));
        assert!(preview.contains("request:source"));
        assert!(preview.contains("Taught by Mizu (partner #2)"));
        assert!(preview.contains("水の波"));
    }
    #[test]
    fn usage_summary_separates_denied_cancelled_unknown_and_exit_zero() {
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
        let detail = record_details(&record(), &state);
        assert!(detail.contains("1/2 exited 0"));
        assert!(detail.contains("Denied: 1 / Cancelled: 1 / Unknown: 1"));
        assert!(detail.contains("run:4"));
        assert!(detail.contains("abc123"));
        assert!(detail.contains("Exit 0 is not correctness"));
    }
    #[test]
    fn replacement_summary_names_scope_and_exact_target() {
        let record = record();
        let mut state = State::default();
        state.mode = Mode::Review;
        state.learning = Some(Learning::Teach(record.proposal.clone()));
        state.name = "Surf".into();
        state.scope = Scope::Personal;
        state.replacement = Some((1, 2));
        state.records.push(record);
        let (summary, details) = review_details(&state, "Bolt");
        assert!(summary.contains("Bolt"));
        assert!(summary.contains("Forgets Project move #1"));
        assert!(details.contains("move #1, revision 2"));
    }
    fn config() -> Config {
        Config {
            ascii: true,
            no_animation: true,
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
            assert!(buffer.contains("> Surf4"));
            assert!(buffer.contains("Moves"));
            assert!(buffer.is_ascii());
        }
    }
    #[test]
    fn rendered_concise_review_is_neutral_without_forcing_scroll_to_end() {
        let mut app = App::new("project".into(), 60, 18);
        let record = record();
        app.moves.partner = 1;
        app.moves.form = 1;
        app.moves.mode = Mode::Review;
        app.moves.name = record.proposal.name.clone();
        app.moves.learning = Some(Learning::Teach(record.proposal));
        let buffer = draw(&mut app, 60, 18);
        assert!(app.moves.reviewed);
        assert!(!app.moves.affirmative);
        assert!(app.moves.scroll_max > 0);
        assert!(buffer.contains("[Stay]"));
        assert!(buffer.contains("No execution"));
        assert!(buffer.contains("Project"));
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
        let detail = record_details(&record(), &state);
        assert!(detail.contains("0/0 exited 0"));
        assert!(detail.contains("Completed without an exit verdict: 1"));
        assert_eq!(
            usage_outcome(&state.usages[0]),
            "completed (not a correctness verdict)"
        );
    }
}
