//! `/usage`: reported tokens and labelled local cost estimates. Never billing.
use super::*;
use crate::workspace::pricing::{self, Totals};

fn scope(lines: &mut Vec<String>, label: &str, totals: &Totals) {
    let cost = pricing::cost(totals);
    let amount = if totals.priced > 0 {
        format!("{cost} USD")
    } else {
        format!("USD unknown · {cost}")
    };
    if totals.requests == 0 {
        lines.push(format!("{label} · no requests · USD unknown"));
        lines.push("  No provider counters in this scope.".into());
        lines.push(String::new());
    } else if totals.unreported == totals.requests {
        lines.push(format!("{label} · tokens unreported · {amount}"));
        lines.push("  Input / output unknown (provider unreported).".into());
        lines.push("  Cached / reasoning subsets unknown.".into());
    } else {
        let bound = if totals.lower_bound() { "≥" } else { "" };
        lines.push(format!(
            "{label} · {bound}{} tok · {amount}",
            pricing::tokens(totals.tokens())
        ));
        lines.push(format!(
            "  Input {bound}{} · Output {bound}{}",
            pricing::tokens(totals.input),
            pricing::tokens(totals.output)
        ));
        lines.push(format!(
            "  Cached read {} · write {} · Reasoning {}",
            pricing::tokens(totals.cached),
            pricing::tokens(totals.cache_write),
            pricing::tokens(totals.reasoning)
        ));
    }
}

fn details(lines: &mut Vec<String>, label: &str, totals: &Totals) {
    lines.push(format!("{label}: {} request(s)", totals.requests));
    if totals.requests > totals.unreported {
        lines.push(format!(
            "  Exact known counts: input {}; output {}; cached read {}; cache write {}; reasoning {}.",
            totals.input, totals.output, totals.cached, totals.cache_write, totals.reasoning
        ));
    }
    if totals.unreported > 0 {
        lines.push(format!(
            "  {} request(s): usage unreported.",
            totals.unreported
        ));
    }
    if totals.uncertain {
        lines.push(
            "  Partly unattributed counters or unknown subset relation; totals are a lower bound."
                .into(),
        );
    }
    if totals.unpriced > 0 {
        lines.push(format!(
            "  {} request(s): USD unknown (no price or unknown token relation).",
            totals.unpriced
        ));
    }
    if totals.plan > 0 {
        lines.push(format!(
            "  {} account-plan request(s): plan charge unknown; not API billing.",
            totals.plan
        ));
    }
    if totals.agent > 0 {
        lines.push(format!(
            "  {} external-agent request(s): agent billing unknown.",
            totals.agent
        ));
    }
    if totals.local > 0 {
        lines.push(format!(
            "  {} local-endpoint request(s): local costs not estimated.",
            totals.local
        ));
    }
}

pub(super) fn lines(app: &App) -> Vec<String> {
    let summary = app.meter_summary();
    let mut lines = Vec::new();
    match &summary.request {
        Some(request) => scope(&mut lines, "Latest request", request),
        None => {
            lines.push("Latest request · no request dispatched · USD unknown".into());
            lines.push("  No provider counters from the lead yet.".into());
            lines.push(String::new());
        }
    }
    scope(&mut lines, "Task", &summary.task);
    scope(&mut lines, "This launch", &summary.launch);
    lines.push(String::new());
    lines.push("HOW TO READ".into());
    lines.push("Input / output include known cached / reasoning subsets. Do not add subsets again; absent subset reports are not proof of zero use.".into());
    lines.push("≥ marks known token lower bounds; +? means some USD cost is unknown. Counts are provider-reported; est. USD amounts are local estimates, never billing.".into());
    lines.push("Latest request belongs to the lead. Task includes saved task usage; this launch includes all partners and tasks since startup.".into());
    lines.push(String::new());
    lines.push("SCOPE DETAILS".into());
    if let Some(request) = &summary.request {
        details(&mut lines, "Latest request", request);
    }
    details(&mut lines, "Task", &summary.task);
    details(&mut lines, "This launch", &summary.launch);
    if summary.partners.len() > 1 {
        lines.push(String::new());
        lines.push("BY PARTNER (this launch)".into());
        for (name, totals) in &summary.partners {
            scope(&mut lines, &display_identity(name), totals);
            details(&mut lines, &display_identity(name), totals);
        }
    }
    if !summary.models.is_empty() {
        lines.push(String::new());
        lines.push("BY MODEL (this launch)".into());
        for (name, totals) in &summary.models {
            scope(&mut lines, &display_identity(name), totals);
            details(&mut lines, &display_identity(name), totals);
        }
    }
    lines.push(String::new());
    lines.push("PRICE SOURCE".into());
    let table = &app.meter.table;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_secs());
    lines.push(format!(
        "Built-in list prices as of {}{}.",
        pricing::BUILT_IN_AS_OF,
        if pricing::built_in_stale(now) {
            " (stale: check current prices)"
        } else {
            ""
        }
    ));
    lines.push(format!(
        "User prices: {} row(s) in {}.",
        table.user_rows(),
        table.path.display()
    ));
    if let Some(error) = &table.error {
        lines.push(format!("Price file not used: {error}"));
    }
    lines.push(format!(
        "Home meter {} · launch budget {}.",
        if table.meter { "shown" } else { "hidden" },
        table
            .budget_usd
            .map_or_else(|| "off".to_string(), |budget| format!("${budget:.2} USD"))
    ));
    lines.push("Budget is warning-only; work never stops at the limit. /usage-budget <USD|off> changes it. No exchange-rate conversion or billing lookup.".into());
    lines
}

pub(super) fn render(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = sheet(area, 104, 30);
    frame.render_widget(Clear, area);
    let block = panel(" Usage · tokens / USD estimates ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let body = row(content, 0, content.height.saturating_sub(2));
    let raw = lines(app);
    let lines = raw
        .iter()
        .flat_map(|line| line.split('\n'))
        .map(|line| {
            let heading = line.starts_with("Latest request ·")
                || line.starts_with("Task ·")
                || line.starts_with("This launch ·");
            Line::styled(line, if heading { paint.accent } else { paint.surface })
        })
        .collect::<Vec<_>>();
    let paragraph = Paragraph::new(lines)
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    let rows = paragraph.line_count(body.width.max(1));
    app.meter.scroll_max = rows
        .saturating_sub(usize::from(body.height))
        .min(usize::from(u16::MAX)) as u16;
    app.meter.scroll = app.meter.scroll.min(app.meter.scroll_max);
    app.meter.page_rows = body.height.max(1);
    frame.render_widget(paragraph.scroll((app.meter.scroll, 0)), body);
    let position = format!(
        "↑↓ PgUp/PgDn Home/End · {}-{}/{}",
        usize::from(app.meter.scroll) + 1,
        (usize::from(app.meter.scroll) + usize::from(body.height)).min(rows),
        rows
    );
    text(
        frame,
        row(content, content.height.saturating_sub(2), 1),
        &position,
        paint.muted,
    );
    super::choices(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        "r Prices | m Meter | Esc Back",
        paint.accent,
        paint,
    );
}
