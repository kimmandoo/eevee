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
        if let Some(relay) = &mut app.workflow.relay {
            relay.reviewed = false;
        }
        app.workflow.affirmative = false;
        text(
            frame,
            area,
            "Specialist desk: need 60 x 18\nDraft retained. Resize to edit or apply.\nEsc Back / Ctrl+C quit or cancel review",
            paint.accent,
        );
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
    let block = panel(" Specialist desk / local decisions ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let tabs = match app.workflow.tab {
        Tab::Diagnosis => "[Diagnosis]  Refactor  Review  Recipe",
        Tab::Refactor => "Diagnosis  [Refactor]  Review  Recipe",
        Tab::Review => "Diagnosis  Refactor  [Review]  Recipe",
        Tab::Recipe => "Diagnosis  Refactor  Review  [Recipe]",
    };
    text(frame, row(content, 0, 1), tabs, paint.accent);
    let notice = if app.workflow.notice.is_empty() {
        "Local notes, real evidence. No automatic command or model request."
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
            format!(
                "{}\nStage references only. No call, run, permission or session.\nProposed paths (still need approval): {:?}\nSource: {:?}\n{}{}",
                targets
                    .iter()
                    .find(|t| Some(t.0) == pending.target)
                    .map_or("Chosen target no longer idle", |t| t.1.as_str()),
                pending.request.paths,
                pending.request.source,
                if pending.request.command.is_some() {
                    "Command draft replacement is included in this neutral review; no approval carries over.\n"
                } else {
                    ""
                },
                pending.request.text
            )
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
                "[Stay] / y Stage / Esc Cancel / PgUp/PgDn Details\nEnter Stay (drafts unchanged)"
            } else {
                "Up/Down Target / Enter Review / Esc Cancel\nOnly idle partners in the target form are offered"
            },
            paint.accent,
        );
        return;
    }
    if let Some(field) = app.workflow.editor {
        text(frame, row(body, 0, 1), field.label(), paint.accent);
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
                "> [Done editing / retain draft]"
            } else {
                "  Done editing / retain draft"
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
            "Ctrl+S Done / Tab control / Enter newline\nEsc Back (draft retained)",
            paint.accent,
        );
        return;
    }
    if app.workflow.preview {
        let effect = if app.workflow.criteria_preview {
            "Confirm decision-linked criteria only.\nNew items remain unverified; duplicates skipped."
        } else {
            "Stage plan, criteria and Next action only.\nNo tools, provider calls, writes or permissions."
        };
        let preview = if app.workflow.criteria_preview {
            app.workflow_criteria_preview()
        } else if super::VERBOSE.get() {
            format!("{effect}\n\n{}", app.workflow_recipe_preview())
        } else {
            app.workflow.recipe.as_ref().map_or_else(
                || "No recipe loaded.".into(),
                |recipe| {
                    format!(
                        "{effect}\n\n{} / {} steps\n/verbose Full plan and scope",
                        clipped(&recipe.name, 80),
                        recipe.steps.len()
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
        let footer_text = format!(
            "{} / {} | {}\ny Apply / n Back / Left/Right Choose / Enter",
            if app.workflow.affirmative {
                "Stay"
            } else {
                "[Stay]"
            },
            if app.workflow.affirmative {
                "[Apply plan only]"
            } else {
                "Apply plan only"
            },
            if app.workflow.reviewed {
                "/verbose Details"
            } else {
                "Enlarge to review scope"
            }
        );
        text(frame, footer, &footer_text, paint.accent);
        return;
    }
    let rows = app.workflow_rows();
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
            "/rerun Rerun with Jolteon / Commands\nUp/Down / Enter / PgUp/PgDn details / Esc Back"
        } else if app.workflow.chooser.is_some() {
            "Up/Down evidence / Enter attach /open-evidence\nLeft/Right Supporting/Refuting / Esc Back"
        } else {
            "/ Commands / Left/Right tabs / Up/Down actions / Enter\nPgUp/PgDn details / Esc Back"
        },
        paint.accent,
    );
}
