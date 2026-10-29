//! `/rules`: user-created approval rules for this project. Browsing never
//! approves anything; `x` revokes and Space disables/enables.
use super::*;
use crate::workspace::rules::{Effect, Expiry};

pub(super) fn render(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let area = sheet(area, 104, 24);
    frame.render_widget(Clear, area);
    let block = panel(" Approval rules ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let body = row(content, 0, content.height.saturating_sub(2));
    let width = usize::from(body.width);
    let rules = app.project_rules();
    let mut lines = Vec::new();
    if rules.is_empty() {
        lines.push(Line::styled(
            "No rules for this project. While a direct command or file create/edit is under review, /rule-allow or /rule-deny makes one.",
            paint.muted,
        ));
    }
    let selected = app.rule_state.selected.min(rules.len().saturating_sub(1));
    for (index, rule) in rules.iter().enumerate() {
        let style = if index == selected {
            paint.selected
        } else {
            paint.surface
        };
        let state = match (rule.enabled, rule.effect, rule.expires) {
            (false, ..) => "off  ",
            (true, Effect::Deny, _) => "deny ",
            (true, Effect::Allow, Expiry::ThisLaunch) => "now  ",
            (true, Effect::Allow, Expiry::Never) => "allow",
        };
        lines.push(Line::styled(
            clipped(
                &format!(
                    "{} #{:<3} {state} {}",
                    if index == selected { ">" } else { " " },
                    rule.id,
                    rule.sentence()
                ),
                width,
            )
            .into_owned(),
            style,
        ));
    }
    if let Some(error) = &app.rules.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            clipped(error, width).into_owned(),
            paint.accent,
        ));
    }
    let top = selected.saturating_sub(usize::from(body.height.max(1)).saturating_sub(2));
    frame.render_widget(
        Paragraph::new(lines.into_iter().skip(top).collect::<Vec<_>>()).style(paint.surface),
        body,
    );
    text(
        frame,
        row(content, content.height.saturating_sub(2), 1),
        "Rules answer only the lead's matching requests; deletes, renames, shells and unknown effects are always reviewed.",
        paint.muted,
    );
    super::choices(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        "Up/Down Choose | Space On/off | x Revoke | Esc Back",
        paint.accent,
        paint,
    );
}
