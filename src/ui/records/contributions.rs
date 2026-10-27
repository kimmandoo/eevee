use crate::{app::App, domain::FORMS, workspace::records::Freshness};
use ratatui::text::Line;

pub(super) fn body(app: &App) -> Vec<Line<'_>> {
    let state = &app.records.contributions;
    let producer = state.partner.map_or_else(
        || "partner unknown (legacy)".into(),
        |id| format!("partner #{id}"),
    );
    let task = state
        .task
        .map_or_else(|| "all retained tasks".into(), |id| format!("task #{id}"));
    let form = state
        .form
        .map_or("all producer forms", |form| FORMS[form].name);
    let mut lines = vec![Line::from(format!("{producer} / {task} / {form}"))];
    let count = state.entries(&app.records.snapshot).count();
    let Some(view) = state
        .entries(&app.records.snapshot)
        .nth(app.records.selected)
    else {
        lines.push(Line::from("No retained contributions match this scope."));
        lines.push(Line::from(
            "A form-only visit creates no evidence. Purged records are not reconstructed.",
        ));
        lines.push(Line::from(
            "Use f/t/p to inspect another form, task or producer.",
        ));
        return lines;
    };
    let artifact = &view.artifact;
    lines.push(Line::from(format!(
        "Item {}/{} · task #{} · {:?}",
        app.records.selected + 1,
        count,
        artifact.task_id,
        artifact.kind
    )));
    if let Some(owner) = &artifact.producer {
        lines.push(Line::from(format!(
            "Produced by {} (#{}), {}",
            owner.name,
            owner.partner,
            FORMS[owner.form.min(8)].name
        )));
        match app
            .records
            .snapshot
            .summaries
            .iter()
            .find(|summary| summary.partner.id == owner.partner)
        {
            None => lines.push(Line::from(
                "Producer released; original attribution retained.",
            )),
            Some(summary) => {
                let current = crate::store::Attribution::of(&summary.partner).name;
                if current != owner.name {
                    lines.push(Line::from(format!(
                        "Current name: {current}; original producer snapshot above is unchanged."
                    )));
                }
            }
        }
    } else {
        lines.push(Line::from(
            "Producer: partner unknown; role is not authorship.",
        ));
    }
    let freshness = match view.freshness {
        Freshness::Current => "current",
        Freshness::Stale => "stale",
        Freshness::Unavailable => "unavailable/purged",
        Freshness::Ambiguous => "unknown/ambiguous",
    };
    lines.push(Line::from(format!("Support: {freshness}")));
    lines.push(Line::from(view.explanation.as_str()));
    lines.push(Line::from(format!("Artifact: {}", artifact.id)));
    lines.push(Line::from(format!("Exact source: {}", artifact.reference)));
    lines.push(Line::from(format!(
        "Recorded role: {} · ordinal {} · time {} UTC",
        FORMS[artifact.role_form.min(8)].name,
        artifact.ordinal,
        super::utc_date(artifact.created_at)
    )));
    lines.extend(artifact.detail.lines().map(Line::from));
    for reference in &artifact.evidence {
        lines.push(Line::from(reference.label()));
    }
    lines.push(Line::from("Task/form contributions, not route-visit outputs. No visit foreign key is retained; time/ordinal proximity never proves a causal link."));
    if artifact.task_id != app.task_id {
        lines.push(Line::from("Enter/g opens the task picker only. Explicit selection uses the ordinary draft and owned-work guards."));
    }
    lines
}
