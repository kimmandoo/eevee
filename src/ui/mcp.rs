//! Complete wrapped unmanaged launch scope; selection is never consent.
use super::*;
pub(super) fn render(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let area = sheet(area, 108, 28);
    frame.render_widget(Clear, area);
    let block = panel(" MCP · unmanaged capabilities ", config, paint, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let body = row(inner, 0, inner.height.saturating_sub(2));
    let state = &app.mcp;
    let text = if let Some(review) = &state.review {
        review.text.clone()
    } else {
        let mut lines = vec!["Stdio only · UNMANAGED, not sandboxed. Registration/navigation/selection never launch or grant. Server text/annotations are untrusted. Only selected schemas/results are sent to the provider.".to_owned(), String::new()];
        if state.effects_unknown() {
            lines.insert(0, "UNMANAGED MCP EFFECTS UNKNOWN / PENDING. Persisted uncertainty disables every automatic effect grant (Command, Write, MCP, task auto-approval). Inference and fresh manual reviews remain available. Failure is not proof of no effects; reconnect/later success does not clear prior missing evidence.".into());
        }
        for registration in state.document.projects.get(&app.cwd).into_iter().flatten() {
            lines.push(format!(
                "Registered: {} · /mcp connect {}",
                registration.name, registration.name
            ));
        }
        if state.tools.is_empty() {
            lines.push("No connected tools. /mcp add <JSON> registers after review; /mcp connect <server> reviews launch.".into());
        }
        for tool in &state.tools {
            lines.push(format!("{} {} / {} · {} · {} B schema\nDescription (untrusted): {}\nAnnotations (not authority): {}\nInput schema (untrusted JSON): {}", if state.selected.contains(&tool.wire_name()) { "[SELECTED: eligible native Sends]" } else { "[NOT SENT]" }, tool.server, tool.name, if tool.user_read { "USER-marked read (unmanaged)" } else { "effect capability; Implement only" }, tool.schema.to_string().len(), tool.description, crate::providers::mcp::clean(&tool.annotations.to_string()), tool.schema));
        }
        lines.push("/mcp select|unselect <server> <tool> · /mcp read|effect <server> <tool> · /mcp disconnect [server]".into());
        lines.join("\n\n")
    };
    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .style(paint.surface);
    let total = paragraph.line_count(body.width.max(1));
    let offset = state
        .scroll
        .min(total.saturating_sub(usize::from(body.height)));
    state.observe(body.width, offset, usize::from(body.height), total);
    frame.render_widget(
        paragraph.scroll((offset.min(u16::MAX as usize) as u16, 0)),
        body,
    );
    text_status(
        frame,
        row(inner, inner.height.saturating_sub(2), 1),
        &state.status,
        paint,
    );
    let hint = match &state.review {
        Some(review) if !review.engaged => {
            "Enter Inspect (not approval) | Up/Down Read full scope | Esc Cancel"
        }
        Some(review) if !review.complete.get() => {
            "Read every scope row with Up/Down before confirmation | Esc Cancel"
        }
        Some(review) if review.launch => "a Launch exact unmanaged server once | Esc Cancel",
        Some(_) => "a Register only (no launch/grant) | Esc Cancel",
        None => "Up/Down Scroll | Esc Back · /mcp commands change explicit selection",
    };
    super::choices(
        frame,
        row(inner, inner.height.saturating_sub(1), 1),
        hint,
        paint.accent,
        paint,
    );
}
fn text_status(frame: &mut Frame, area: Rect, status: &str, paint: &Paint) {
    text(frame, area, status, paint.muted);
}
