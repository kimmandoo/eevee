//! Exact typed plan inspection, separate from ordinary command consent.
use super::*;
use crate::{app::tool_plans::Review, messages::Message, workspace::tool_plans::Operation};

pub(super) fn render(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    frame.render_widget(Clear, area);
    let state = &app.tool_plans;
    if app.width < 60 || app.height < 18 || area.width < 56 || area.height < 14 {
        state.reviewed.set(false);
        state.extent.set(0);
        text(
            frame,
            area,
            paint.hint(Message::ToolPlanSmall),
            paint.accent,
        );
        return;
    }
    let area = centered(area, 112, area.height.min(34));
    let block = panel(paint.hint(Message::ToolPlanTitle), config, paint, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = inner.height.saturating_sub(2);
    let mut lines = Vec::<Line<'static>>::new();
    if !state.notice.is_empty() {
        lines.push(Line::from(display_identity(&state.notice).into_owned()));
    }
    if let Some(editor) = &state.editor {
        lines.push(Line::from(paint.hint(Message::ToolPlanEditor).to_owned()));
        lines.push(Line::from(display_identity(editor).into_owned()));
    } else {
        lines.push(Line::from(paint.hint(Message::ToolPlanPolicy).to_owned()));
        lines.push(Line::from(paint.hint(Message::ToolPlanRecovery).to_owned()));
        let selected = state
            .history_index
            .and_then(|index| app.workflows.tool_plan_history.get(index))
            .or(app.workflows.tool_plans.as_ref());
        if let Some(plan) = selected {
            if state.history_index.is_some() {
                lines.push(Line::from(
                    paint.hint(Message::ToolPlanHistoryControls).to_owned(),
                ));
            }
            lines.push(Line::from(
                display_identity(&format!(
                    "{} · {} · task {} / partner {} · format {}",
                    plan.title, plan.id, plan.task, plan.partner, plan.version
                ))
                .into_owned(),
            ));
            lines.push(Line::from(
                display_identity(&format!("Project: {}", plan.project)).into_owned(),
            ));
            lines.push(Line::from(
                display_identity(&format!("Input: {:?}", plan.source)).into_owned(),
            ));
            lines.push(Line::from(format!("Attempts {} · recorded calls {} (lost-response work may be uncounted) · plan wall {} µs · summed measured node work {} µs (serial-work comparator, not a serial rerun)", plan.attempts, plan.execution_count, plan.wall_us, plan.serial_work_us())));
            if let Some(comparison) = &plan.comparison {
                lines.push(Line::from(format!("ACTUAL paired all-read executions: parallel {} calls / {} µs; serial {:?} calls / {:?} µs; identical observation identities {:?}. No command replay or correctness/completeness claim.", comparison.parallel_calls, comparison.parallel_wall_us, comparison.serial_calls, comparison.serial_wall_us, comparison.equal_results)));
            }
            let candidates = if state.review == Some(Review::Resume) {
                plan.resume_candidates().ok()
            } else {
                None
            };
            for (index, node) in plan.nodes.iter().enumerate() {
                let eligibility = candidates.as_ref().map_or("", |indices| {
                    if indices.contains(&index) {
                        " [RESUME]"
                    } else {
                        " [KEEP: never repeat confirmed command]"
                    }
                });
                lines.push(Line::from(format!(
                    "{}. {} · {:?} / {:?}{}",
                    index + 1,
                    node.spec.id,
                    node.status,
                    node.certainty,
                    eligibility
                )));
                lines.push(Line::from(
                    display_identity(&format!("Requires: {:?}", node.spec.dependencies))
                        .into_owned(),
                ));
                lines.push(Line::from(
                    display_identity(&format!("Reason: {}", node.spec.reason)).into_owned(),
                ));
                lines.push(Line::from(if matches!(node.spec.operation, Operation::Command { .. }) { "Effect: UNKNOWN — serialized; separate ordinary exact command approval required" } else { "Effect: READ ONLY — enforced by existing bounded file executor" }));
                lines.push(Line::from(
                    display_identity(&format!(
                        "Exact arguments: {}",
                        serde_json::to_string(&node.spec.operation).unwrap_or_default()
                    ))
                    .into_owned(),
                ));
                lines.push(Line::from(format!("Argument SHA256: {} · measured {} µs · new observation query workers {} · run: {}", node.argument_sha256, node.elapsed_us, node.query_workers, node.run_id.as_deref().unwrap_or("none"))));
                lines.push(Line::from(format!(
                    "Node input snapshot (historical, not a current observation): {:?}",
                    node.input
                )));
                if state.review.is_none() && !node.result.is_empty() {
                    lines.push(Line::from(format!(
                        "Result retained {}/{} bytes · digest {}",
                        node.result.len(),
                        node.result_bytes,
                        node.result_sha256.as_deref().unwrap_or("unknown"),
                    )));
                    lines.push(Line::from(display_identity(&node.result).into_owned()));
                }
            }
        } else {
            lines.push(Line::from(paint.hint(Message::ToolPlanEmpty).to_owned()));
            lines.push(Line::from(r#"{"title":"Inspect","nodes":[{"id":"read","dependencies":[],"operation":{"type":"Read","path":"src/main.rs","start_line":1,"end_line":5},"reason":"Inspect entry point"}]}"#));
        }
    }
    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .style(paint.base);
    let total = paragraph.line_count(inner.width);
    let scroll = usize::from(state.scroll).min(total.saturating_sub(usize::from(rows)));
    if state.review.is_some() {
        state
            .reviewed
            .set(state.observe_review(scroll, total, usize::from(rows)));
    }
    frame.render_widget(
        paragraph.scroll((scroll.min(u16::MAX as usize) as u16, 0)),
        Rect::new(inner.x, inner.y, inner.width, rows),
    );
    frame.render_widget(
        Paragraph::new(paint.hint(if state.review.is_some() {
            Message::ToolPlanReviewControls
        } else {
            Message::ToolPlanControls
        }))
        .wrap(Wrap { trim: false })
        .style(paint.muted),
        Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 2),
    );
}
