//! `/usage`: reported tokens and labelled local cost estimates. Never billing.
use super::*;
use crate::workspace::pricing::{self, Totals};

fn columns(label: &str, totals: &Totals, wide: bool) -> String {
    let bound = if totals.lower_bound() { "≥" } else { "" };
    if wide {
        format!(
            "{label:<18} {bound}{:>8} in {:>8} cached {:>8} out {:>8} reason  {}",
            pricing::tokens(totals.input),
            pricing::tokens(totals.cached),
            pricing::tokens(totals.output),
            pricing::tokens(totals.reasoning),
            pricing::cost(totals),
        )
    } else {
        format!(
            "{label:<10} {bound}{} tok · {}",
            pricing::tokens(totals.tokens()),
            pricing::cost(totals)
        )
    }
}

pub(super) fn lines(app: &App, wide: bool) -> Vec<String> {
    let summary = app.meter_summary();
    let mut lines = vec!["SCOPE".to_string()];
    match &summary.request {
        Some(request) => lines.push(columns("Request (latest)", request, wide)),
        None => lines.push("Request (latest)   none dispatched by the lead yet".into()),
    }
    lines.push(columns("Task", &summary.task, wide));
    lines.push(columns("This launch", &summary.launch, wide));
    let launch = &summary.launch;
    let mut notes = Vec::new();
    if launch.unreported > 0 {
        notes.push(format!("{} request(s) unreported", launch.unreported));
    }
    if launch.unpriced > 0 {
        notes.push(format!(
            "{} with no price or unknown token relation",
            launch.unpriced
        ));
    }
    if launch.plan > 0 {
        notes.push(format!("{} on an account plan", launch.plan));
    }
    if launch.agent > 0 {
        notes.push(format!("{} billed by an external agent", launch.agent));
    }
    if launch.local > 0 {
        notes.push(format!("{} on a local endpoint", launch.local));
    }
    if !notes.is_empty() {
        lines.push(format!("  {}", notes.join(" · ")));
    }
    if summary.partners.len() > 1 {
        lines.push(String::new());
        lines.push("BY PARTNER (this launch)".into());
        for (name, totals) in &summary.partners {
            lines.push(columns(name, totals, wide));
        }
    }
    if !summary.models.is_empty() {
        lines.push(String::new());
        lines.push("BY MODEL (this launch)".into());
        for (name, totals) in &summary.models {
            lines.push(columns(name, totals, wide));
        }
    }
    lines.push(String::new());
    let table = &app.meter.table;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_secs());
    lines.push(format!(
        "Prices: built-in list prices as of {}{}; {} user row(s) in {}.",
        pricing::BUILT_IN_AS_OF,
        if pricing::built_in_stale(now) {
            " (stale: check current prices)"
        } else {
            ""
        },
        table.user_rows(),
        table.path.display()
    ));
    if let Some(error) = &table.error {
        lines.push(format!("Price file not used: {error}"));
    }
    lines.push(format!(
        "Meter {} · budget {}",
        if table.meter { "shown" } else { "hidden" },
        table
            .budget_usd
            .map_or_else(|| "off".to_string(), |budget| format!("${budget:.2}"))
    ));
    lines.push(
        "Counts are provider-reported. Costs are local estimates (est.), never billing.".into(),
    );
    lines
}

pub(super) fn render(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let area = sheet(area, 104, 26);
    frame.render_widget(Clear, area);
    let block = panel(" Usage ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let wide = content.width >= 90;
    let body = row(content, 0, content.height.saturating_sub(1));
    let lines = lines(app, wide)
        .into_iter()
        .map(|line| {
            let style = if line
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == ' ' || c == '(' || c == ')')
                && !line.is_empty()
            {
                paint.muted
            } else {
                paint.surface
            };
            Line::styled(clipped(&line, usize::from(body.width)).into_owned(), style)
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).style(paint.surface), body);
    super::choices(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        "r Reload prices | m Meter on/off | Esc Back",
        paint.accent,
        paint,
    );
}
