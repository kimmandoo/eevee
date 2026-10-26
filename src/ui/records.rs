use super::{Paint, panel, row, selection_details, text};
use crate::{
    app::{
        App,
        records::{FormMarker, Mode},
    },
    config::Config,
    domain::FORMS,
    messages::{Message, Template},
    store,
    workspace::records::{self, Freshness},
};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};

pub(super) fn render_small(frame: &mut Frame, area: Rect, app: &mut App, paint: &Paint) {
    app.records.invalidate_review();
    frame.render_widget(Clear, area);
    text(frame, area, paint.hint(Message::RecordsSmall), paint.accent);
}
pub(super) fn render(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    if area.width < 60 || area.height < 18 {
        render_small(frame, area, app, paint);
        return;
    }
    let area = super::sheet(area, 110, area.height.saturating_sub(2).max(18));
    frame.render_widget(Clear, area);
    let block = panel(paint.hint(title(app.records.mode)), config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let count = app.record_selection_count();
    app.records.selected = app.records.selected.min(count.saturating_sub(1));
    let scoped = app.formation_inspector_active();
    let heading = if let Some(crate::app::formations::Inspector::Moves { partner, form }) =
        app.formations.inspector
        && scoped
    {
        let name = app
            .formations
            .roster
            .iter()
            .find(|p| p.id == partner)
            .map_or_else(|| format!("#{partner}"), App::partner_label);
        let owner_heading = paint.format(
            Template::FormationInspectorOwner,
            &[("partner", &name), ("form", FORMS[form].name)],
        );
        if let Some(index) = app
            .records
            .snapshot
            .summaries
            .iter()
            .position(|summary| summary.partner.id == partner)
            .filter(|_| {
                app.formation_readiness(app.formations.selected)
                    .moves
                    .is_some()
            })
        {
            app.records.selected = index;
        } else {
            text(frame, row(content, 0, 1), &owner_heading, paint.accent);
            text(
                frame,
                row(content, 1, 2),
                paint.hint(Message::FormationMovesUnknown),
                paint.accent,
            );
            text(
                frame,
                row(content, content.height.saturating_sub(2), 2),
                paint.hint(Message::FormationReadonly),
                paint.accent,
            );
            return;
        }
        owner_heading
    } else if app.records.mode == Mode::Ribbons {
        paint.hint(Message::RibbonsHistory).to_owned()
    } else {
        paint.format_values(
            Template::RecordsPosition,
            &[
                (
                    "index",
                    crate::messages::Argument::Number(if count == 0 {
                        0
                    } else {
                        app.records.selected + 1
                    } as f64),
                ),
                ("count", crate::messages::Argument::Number(count as f64)),
            ],
        )
    };
    text(frame, row(content, 0, 1), &heading, paint.accent);
    let summary = match app.records.mode {
        Mode::Resolve => app
            .records
            .resolution_choices()
            .nth(app.records.selected)
            .map(|choice| {
                paint.format(
                    Template::RecordsResolutionSummary,
                    &[
                        ("signature", &choice.signature),
                        ("support", &choice.support_artifact),
                        ("run", &choice.passing_run),
                    ],
                )
            }),
        Mode::Discard => app.records.failed.as_ref().map(|request| {
            paint.format(
                Template::RecordsDiscardSummary,
                &[("identity", &operation_scope(request, paint))],
            )
        }),
        _ => None,
    };
    let status = summary.as_deref().unwrap_or_else(|| {
        if !app.records.failed_error.is_empty() {
            &app.records.failed_error
        } else if !app.records.notice.is_empty() {
            &app.records.notice
        } else if let Some(hint) = app.records.hint {
            paint.hint(hint)
        } else if app.records.persistence_pending() {
            paint.hint(Message::RecordsWaiting)
        } else {
            paint.hint(Message::RecordsLocalOnly)
        }
    });
    let maximum_status_rows = if summary.is_some() {
        content.height.saturating_sub(5)
    } else {
        2
    };
    let status = Paragraph::new(status)
        .style(if app.records.failed.is_some() {
            paint.accent
        } else {
            paint.muted
        })
        .wrap(Wrap { trim: false });
    let required_status_rows = status.line_count(content.width.max(1));
    let status_rows = required_status_rows
        .max(2)
        .min(usize::from(maximum_status_rows)) as u16;
    let summary_visible = summary.is_some() && required_status_rows <= usize::from(status_rows);
    frame.render_widget(status, row(content, 1, status_rows));
    let body = row(
        content,
        1 + status_rows,
        content.height.saturating_sub(status_rows + 4),
    );
    let lines = body_lines(app, paint);
    let paragraph = Paragraph::new(lines)
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    let max = selection_details(frame, body, paragraph, app.records.scroll);
    app.records.scroll_max = max;
    app.records.scroll = app.records.scroll.min(max);
    app.records.page = body.height.saturating_sub(1).max(1);
    if matches!(app.records.mode, Mode::Resolve | Mode::Discard) {
        let waiting = app.records.mode != Mode::Discard && app.records.persistence_pending();
        app.records.reviewed = summary_visible
            && body.height > 0
            && !waiting
            && if app.records.mode == Mode::Discard {
                app.records.failed.is_some()
            } else {
                count > 0
            };
        if !app.records.reviewed {
            app.records.affirmative = false;
        }
    }
    let footer = if scoped {
        paint.hint(Message::FormationReadonly).to_owned()
    } else if matches!(app.records.mode, Mode::Resolve | Mode::Discard) {
        paint.format(
            Template::RecordsReviewControls,
            &[
                (
                    "stay",
                    paint.hint(if app.records.affirmative {
                        Message::CommonStay
                    } else {
                        Message::CommonStaySelected
                    }),
                ),
                (
                    "action",
                    paint.hint(match (app.records.mode, app.records.affirmative) {
                        (Mode::Discard, true) => Message::RecordsDiscardSelected,
                        (Mode::Discard, false) => Message::RecordsDiscardAction,
                        (_, true) => Message::RecordsLinkSelected,
                        _ => Message::RecordsLinkAction,
                    }),
                ),
                (
                    "review",
                    paint.hint(if app.records.reviewed {
                        Message::RecordsSummaryVisible
                    } else {
                        Message::RecordsSummaryUnavailable
                    }),
                ),
            ],
        )
    } else {
        paint
            .hint(match app.records.mode {
                Mode::Habits => Message::RecordsHabitsControls,
                Mode::Pairings => Message::RecordsPairingControls,
                Mode::Dex => Message::RecordsDexControls,
                Mode::Ribbons => Message::RibbonsControls,
                _ => Message::RecordsControls,
            })
            .to_owned()
    };
    text(
        frame,
        row(content, content.height.saturating_sub(3), 3),
        &footer,
        paint.muted,
    );
}
fn title(mode: Mode) -> Message {
    match mode {
        Mode::Summary => Message::RecordsSummary,
        Mode::Routes => Message::RecordsRoutes,
        Mode::Dex => Message::RecordsDex,
        Mode::Resolve => Message::RecordsResolve,
        Mode::Habits => Message::RecordsHabits,
        Mode::Pairings => Message::RecordsPairings,
        Mode::Ribbons => Message::RibbonsTitle,
        Mode::Discard => Message::RecordsDiscard,
    }
}
pub(super) fn marker(marker: &FormMarker, paint: &Paint) -> String {
    marker_text(marker, paint, false)
}
pub(super) fn compact_marker(marker: &FormMarker, paint: &Paint) -> String {
    marker_text(marker, paint, true)
}
fn marker_text(marker: &FormMarker, paint: &Paint, compact: bool) -> String {
    if marker.count == 0 {
        return paint.hint(Message::RecordsNone).to_owned();
    }
    let status = paint.hint(if marker.stale {
        Message::RecordsStale
    } else {
        Message::RecordsProduced
    });
    let mut producers = match marker.producers.as_slice() {
        [] => std::borrow::Cow::Borrowed(""),
        [name] => std::borrow::Cow::Borrowed(name.as_str()),
        names => std::borrow::Cow::Owned(names.join(", ")),
    };
    if marker.unknown > 0 {
        if producers.is_empty() {
            producers = std::borrow::Cow::Borrowed(paint.hint(Message::RecordsUnknownProducer));
        } else {
            let producers = producers.to_mut();
            producers.push_str(", ");
            producers.push_str(paint.hint(Message::RecordsUnknownProducer));
        }
    }
    let count = marker.count.to_string();
    let arguments = [
        ("status", status),
        ("count", &count),
        ("producer", &producers),
    ];
    if compact {
        paint.format(Template::RecordsCompactMarker, &arguments)
    } else {
        paint.format(
            Template::RecordsMarker,
            &[
                arguments[0],
                arguments[1],
                arguments[2],
                ("fact", &marker.fact),
            ],
        )
    }
}
fn body_lines<'a>(app: &'a App, paint: &Paint) -> Vec<Line<'a>> {
    let state = &app.records;
    let mut lines = Vec::new();
    if matches!(state.mode, Mode::Resolve | Mode::Discard) {
        lines.extend(
            state
                .failed_error
                .lines()
                .map(|line| Line::styled(line, paint.accent)),
        );
    }
    let mut push = |value: String| {
        for line in value.lines() {
            lines.push(Line::from(line.to_owned()));
        }
    };
    match state.mode {
        Mode::Summary => {
            if let Some(summary) = state.snapshot.summaries.get(state.selected) {
                let partner = &summary.partner;
                let name = store::Attribution::of(partner).name;
                let form = &FORMS[partner.form.min(8)];
                push(paint.format(
                    Template::RecordsIdentity,
                    &[
                        ("name", &name),
                        ("form", form.name),
                        ("element", form.element),
                    ],
                ));
                if state.snapshot.flavor_enabled {
                    for view in state
                        .snapshot
                        .ribbons
                        .iter()
                        .filter(|view| {
                            view.ribbon.owner.partner == partner.id && !view.ribbon.hidden
                        })
                        .take(3)
                    {
                        push(ribbon_line(view, paint));
                    }
                }
                let date = if partner.met_at > 0 {
                    utc_date(partner.met_at)
                } else {
                    paint.hint(Message::RecordsUnknownDate).into()
                };
                let project = if partner.created_project.is_empty() {
                    paint.hint(Message::RecordsUnknownProject)
                } else {
                    &partner.created_project
                };
                push(paint.format(
                    Template::RecordsMet,
                    &[
                        ("date", &date),
                        (
                            "origin",
                            if partner.met_derived {
                                paint.hint(Message::RecordsDerived)
                            } else {
                                ""
                            },
                        ),
                        ("project", project),
                    ],
                ));
                push(paint.format(
                    Template::RecordsBond,
                    &[("count", &summary.shared_bond_memories.to_string())],
                ));
                if summary.moves.iter().all(|count| *count == 0) {
                    push(paint.hint(Message::RecordsNoMoves).to_owned());
                }
                for (index, form) in FORMS.iter().enumerate() {
                    push(paint.format(
                        Template::RecordsFormCounts,
                        &[
                            ("form", form.name),
                            ("moves", &summary.moves[index].to_string()),
                            ("tasks", &summary.produced_tasks[index].to_string()),
                        ],
                    ));
                    if !summary.move_names[index].is_empty() {
                        push(summary.move_names[index].join(", "));
                    }
                }
                push(paint.format(
                    Template::RecordsUnknownArtifacts,
                    &[("count", &summary.unknown_producer_artifacts.to_string())],
                ));
                push(paint.hint(Message::RecordsTaskMarkers).to_owned());
                for (index, form) in FORMS.iter().enumerate() {
                    push(format!(
                        "{}: {}",
                        form.name,
                        marker(&state.markers[index], paint)
                    ));
                }
            } else {
                push(paint.hint(Message::RecordsEmpty).to_owned());
            }
        }
        Mode::Ribbons => {
            if state.snapshot.ribbons.is_empty() {
                push(paint.hint(Message::RibbonsEmpty).to_owned());
            }
            if let Some(view) = state.snapshot.ribbons.get(state.selected) {
                let ribbon = &view.ribbon;
                push(format!(
                    "{}{}",
                    if ribbon.hidden {
                        paint.hint(Message::RibbonsHiddenTag)
                    } else {
                        ""
                    },
                    ribbon_line(view, paint)
                ));
                push(match &ribbon.evidence {
                    crate::workspace::ribbons::Evidence::Artifact { id } => {
                        paint.format(Template::RibbonsArtifact, &[("id", id)])
                    }
                    crate::workspace::ribbons::Evidence::Move { id, revision } => paint.format(
                        Template::RibbonsMove,
                        &[("id", &id.to_string()), ("revision", &revision.to_string())],
                    ),
                });
                push(paint.format(
                    Template::RibbonsDetails,
                    &[
                        ("task", &ribbon.task_id.to_string()),
                        ("partner", &ribbon.owner.partner.to_string()),
                        ("name", &ribbon.owner.name),
                        ("form", FORMS[ribbon.owner.form.min(8)].name),
                        ("date", &utc_date(ribbon.at)),
                        ("status", paint.hint(freshness(&view.freshness))),
                        ("reason", &view.explanation),
                    ],
                ));
                push(
                    paint
                        .hint(if ribbon.hidden {
                            Message::RibbonsHidden
                        } else {
                            Message::RibbonsVisible
                        })
                        .to_owned(),
                );
                if ribbon.task_id != app.task_id {
                    push(paint.hint(Message::RibbonsForeignTask).to_owned());
                }
                match &ribbon.evidence {
                    crate::workspace::ribbons::Evidence::Artifact { id } => {
                        if let Some(source) = state.snapshot.artifacts.iter().find(|source| {
                            source.artifact.id == *id && source.artifact.task_id == ribbon.task_id
                        }) {
                            push(source.artifact.reference.clone());
                            push(source.artifact.detail.clone());
                            for evidence in &source.artifact.evidence {
                                use crate::workspace::workflows::EvidenceRef;
                                push(match evidence {
                                    EvidenceRef::Source {
                                        task_id,
                                        excerpt_id,
                                        path,
                                        start_line,
                                        end_line,
                                        ..
                                    } => paint.format(
                                        Template::RibbonsSourceEvidence,
                                        &[
                                            ("task", &task_id.to_string()),
                                            ("excerpt", &excerpt_id.to_string()),
                                            ("path", path),
                                            ("start", &start_line.to_string()),
                                            ("end", &end_line.to_string()),
                                        ],
                                    ),
                                    EvidenceRef::Run { task_id, run_id } => paint.format(
                                        Template::RibbonsRunEvidence,
                                        &[("task", &task_id.to_string()), ("run", run_id)],
                                    ),
                                });
                            }
                        }
                        push(paint.hint(Message::RibbonsOpenArtifact).to_owned());
                    }
                    crate::workspace::ribbons::Evidence::Move { .. } => {
                        if ribbon.owner.partner != app.lead().id || ribbon.owner.form != app.form()
                        {
                            push(paint.hint(Message::RibbonsForeignOwner).to_owned());
                        }
                        push(paint.hint(Message::RibbonsOpenMove).to_owned());
                    }
                }
            }
            for (index, view) in state.snapshot.ribbons.iter().enumerate() {
                if index != state.selected {
                    push(format!(
                        "{}{}",
                        if view.ribbon.hidden {
                            paint.hint(Message::RibbonsHiddenTag)
                        } else {
                            ""
                        },
                        ribbon_line(view, paint)
                    ));
                }
            }
            push(paint.hint(Message::RibbonsDisclosure).to_owned());
        }
        Mode::Routes => {
            push(paint.hint(Message::RecordsRoutesDisclosure).to_owned());
            if let Some(summary) = state.snapshot.summaries.get(state.selected) {
                let owner = store::Attribution::of(&summary.partner);
                push(owner.name);
                lines.extend(route_lines(state, app.task_id, owner.partner, paint));
            }
        }
        Mode::Dex => {
            if let Some(signature) = state.snapshot.signatures.get(state.selected) {
                push(paint.format(
                    Template::RecordsSignature,
                    &[
                        ("kind", &signature.kind),
                        ("detail", &signature.detail),
                        ("count", &signature.seen_count.to_string()),
                        ("first", &utc_date(signature.first_seen)),
                        ("key", &signature.key),
                    ],
                ));
                if let Some(latest) = state
                    .snapshot
                    .failure_comparisons
                    .iter()
                    .rev()
                    .find(|comparison| comparison.signature == signature.key)
                {
                    push(
                        paint.format(Template::DexRelaySelection, &[("run", &latest.failure_run)]),
                    );
                }
                let mut comparisons = state
                    .snapshot
                    .failure_comparisons
                    .iter()
                    .filter(|comparison| comparison.signature == signature.key)
                    .peekable();
                if comparisons.peek().is_none() {
                    push(paint.hint(Message::DexComparisonUnavailable).to_owned());
                }
                for comparison in comparisons {
                    let disposition = match comparison.disposition {
                        records::FailureDisposition::UnresolvedNew => Message::DexUnresolvedNew,
                        records::FailureDisposition::UnresolvedRecurring => {
                            Message::DexUnresolvedRecurring
                        }
                        records::FailureDisposition::HistoricalSupportedPass => {
                            Message::DexHistoricalSupportedPass
                        }
                    };
                    push(
                        paint.format(
                            Template::DexComparison,
                            &[
                                ("run", &comparison.failure_run),
                                ("state", paint.hint(disposition)),
                                (
                                    "command",
                                    &crate::app::execution_relays::command_draft(
                                        &comparison.command,
                                    ),
                                ),
                                ("cwd", &comparison.cwd.display().to_string()),
                                ("source", paint.hint(freshness(&comparison.source))),
                                (
                                    "fingerprint",
                                    comparison
                                        .source_fingerprint
                                        .as_deref()
                                        .unwrap_or(paint.hint(Message::DexSourceUnknown)),
                                ),
                                (
                                    "support",
                                    paint.hint(support_status(comparison.failure_support)),
                                ),
                                ("earlier", &comparison.earlier_failures.join(", ")),
                            ],
                        ),
                    );
                    for support in &comparison.historical {
                        push(
                            paint.format(
                                Template::DexHistoricalSupport,
                                &[
                                    ("support", &support.support_artifact),
                                    ("diagnosis", paint.hint(support_status(support.diagnosis))),
                                    ("run", &support.passing_run),
                                    ("pass", paint.hint(support_status(support.pass))),
                                    ("source", paint.hint(freshness(&support.source))),
                                    (
                                        "fingerprint",
                                        support
                                            .source_fingerprint
                                            .as_deref()
                                            .unwrap_or(paint.hint(Message::DexSourceUnknown)),
                                    ),
                                ],
                            ),
                        );
                    }
                }
                push(paint.format(
                    Template::RecordsRunRefs,
                    &[("runs", &signature.runs.join("\n"))],
                ));
                for resolution in state
                    .snapshot
                    .resolutions
                    .iter()
                    .filter(|resolution| resolution.signature == signature.key)
                {
                    if resolution.task_id != app.task_id {
                        push(paint.hint(Message::DexForeignSupport).to_owned());
                    }
                    push(paint.format(
                        Template::RecordsHistoricalLead,
                        &[
                            ("task", &resolution.task_id.to_string()),
                            ("support", &resolution.support_artifact),
                            ("run", &resolution.passing_run),
                            ("cause", &resolution.cause),
                        ],
                    ));
                }
                push(paint.hint(Message::RecordsLeadOnly).to_owned());
            } else {
                push(paint.hint(Message::RecordsNoSignatures).to_owned());
            }
        }
        Mode::Resolve => {
            if let Some(choice) = state.resolution_choices().nth(state.selected) {
                push(paint.format(
                    Template::RecordsResolutionChoice,
                    &[
                        ("signature", &choice.signature),
                        ("support", &choice.support_artifact),
                        ("run", &choice.passing_run),
                        ("cause", &choice.cause),
                    ],
                ));
            } else {
                push(paint.hint(Message::RecordsNoResolution).to_owned());
                for view in state
                    .snapshot
                    .artifacts
                    .iter()
                    .filter(|view| view.artifact.supported_diagnosis)
                {
                    push(paint.format(
                        Template::RecordsSupportReceipt,
                        &[
                            ("reference", &view.artifact.id),
                            ("freshness", paint.hint(freshness(&view.freshness))),
                            ("detail", &view.artifact.detail),
                        ],
                    ));
                }
            }
        }
        Mode::Habits => {
            push(paint.hint(Message::RecordsHabitDisclosure).to_owned());
            if let Some(habit) = state.selected_habit() {
                push(habit.instruction.clone());
                push(paint.format(Template::RecordsHabitHash, &[("key", &habit.key)]));
                for source in &habit.sources {
                    push(paint.format(
                        Template::RecordsHabitSource,
                        &[
                            ("request", &source.request_id),
                            ("task", &source.task_id.to_string()),
                            ("owner", &source.owner.name),
                        ],
                    ));
                }
            } else {
                push(paint.hint(Message::RecordsNoHabits).to_owned());
            }
        }
        Mode::Pairings => {
            push(paint.hint(Message::RecordsPairingDisclosure).to_owned());
            if let Some(offer) = state.pairings.offer.as_ref() {
                push(paint.hint(Message::RecordsPairingOffer).to_owned());
                push(pairing(offer, paint));
            }
            if let Some(saved) = state.snapshot.pairings.get(state.selected) {
                push(paint.hint(Message::RecordsPairingCommitted).to_owned());
                push(pairing(saved, paint));
            } else {
                push(paint.hint(Message::RecordsNoPairings).to_owned());
            }
        }
        Mode::Discard => {
            push(paint.hint(Message::RecordsDiscardDisclosure).to_owned());
            if let Some(request) = &state.failed {
                push(paint.format(
                    Template::RecordsFailedOwner,
                    &[
                        ("task", &request.task_id.to_string()),
                        ("operation", operation_name(&request.operation, paint)),
                    ],
                ));
                push(operation_detail(&request.operation, paint));
            }
        }
    }
    lines
}
fn ribbon_line(view: &crate::workspace::ribbons::View, paint: &Paint) -> String {
    use crate::workspace::ribbons::Kind;
    paint.format(
        Template::RibbonsLine,
        &[
            (
                "kind",
                paint.hint(match view.ribbon.kind {
                    Kind::AcceptedReview => Message::RibbonsAcceptedReview,
                    Kind::VerifiedHandoff => Message::RibbonsVerifiedHandoff,
                    Kind::LearnedMove => Message::RibbonsLearnedMove,
                }),
            ),
            ("name", &view.ribbon.owner.name),
            ("status", paint.hint(freshness(&view.freshness))),
        ],
    )
}
fn pairing(value: &records::Pairing, paint: &Paint) -> String {
    paint.format(
        Template::RecordsPairingValue,
        &[
            ("partner", &value.partner.to_string()),
            ("form", FORMS[value.form.min(8)].name),
            ("agent", value.agent.label()),
            (
                "model",
                value
                    .selection
                    .model
                    .as_deref()
                    .unwrap_or(paint.hint(Message::RecordsDefault)),
            ),
            (
                "effort",
                value
                    .selection
                    .effort
                    .as_deref()
                    .unwrap_or(paint.hint(Message::RecordsDefault)),
            ),
        ],
    )
}
fn support_status(status: records::SupportStatus) -> Message {
    match status {
        records::SupportStatus::Retained => Message::DexSupportRetained,
        records::SupportStatus::Stale => Message::DexSupportStale,
        records::SupportStatus::Missing => Message::DexSupportMissing,
        records::SupportStatus::Unavailable => Message::DexSupportUnavailable,
        records::SupportStatus::PurgedOrUnavailable => Message::DexSupportPurged,
        records::SupportStatus::DifferentCommand => Message::DexSupportDifferentCommand,
    }
}
fn freshness(value: &Freshness) -> Message {
    match value {
        Freshness::Current => Message::RecordsCurrent,
        Freshness::Stale => Message::RecordsStale,
        Freshness::Unavailable => Message::RecordsUnavailable,
        Freshness::Ambiguous => Message::RecordsUnknown,
    }
}
fn operation_name(operation: &store::records::Operation, paint: &Paint) -> &'static str {
    use store::records::Operation;
    paint.hint(match operation {
        Operation::ObserveInstruction { .. } => Message::RecordsOpInstruction,
        Operation::SetPairing { .. } => Message::RecordsOpPairing,
        Operation::SetHeld { .. } | Operation::RenewHeld { .. } => Message::HeldPanel,
        Operation::SetRibbonHidden { .. } => Message::RibbonsVisibility,
        Operation::SetFlavor { .. } => Message::RecordsOpFlavor,
        Operation::DismissInstruction { .. } => Message::RecordsOpDismiss,
        Operation::BindInstructionMemory { .. } => Message::RecordsOpBind,
        Operation::ResolveFailure { .. } => Message::RecordsOpResolve,
        Operation::ObserveRun { .. } => Message::RecordsOpRun,
        Operation::Load { .. } => Message::RecordsOpLoad,
        Operation::Usage { .. } => Message::RecordsOpUsage,
    })
}
fn operation_scope(request: &store::records::Request, paint: &Paint) -> String {
    use std::fmt::Write;
    use store::records::Operation;
    let mut identity = if request.task_id == 0 {
        format!(
            "{} · {}",
            paint.hint(Message::RecordsProjectSetting),
            operation_name(&request.operation, paint),
        )
    } else {
        paint.format(
            Template::RecordsFailedOwner,
            &[
                ("task", &request.task_id.to_string()),
                ("operation", operation_name(&request.operation, paint)),
            ],
        )
    };
    match &request.operation {
        Operation::ObserveInstruction {
            request_id, owner, ..
        } => {
            let _ = write!(
                identity,
                " · {request_id} · #{} / {}",
                owner.partner,
                FORMS[owner.form.min(8)].name
            );
        }
        Operation::ObserveRun { run_id } => {
            let _ = write!(identity, " · {run_id}");
        }
        Operation::SetPairing { partner, form, .. } | Operation::SetHeld { partner, form, .. } => {
            let _ = write!(identity, " · #{partner} / {}", FORMS[(*form).min(8)].name);
        }
        Operation::RenewHeld { review } => {
            let _ = write!(
                identity,
                " · #{} / {} · r{}",
                review.owner.partner,
                FORMS[review.owner.form.min(8)].name,
                review.task_revision,
            );
        }
        Operation::DismissInstruction { key } => {
            let _ = write!(identity, " · {key}");
        }
        Operation::BindInstructionMemory {
            key,
            memory,
            revision,
        } => {
            let _ = write!(identity, " · {key} · #{memory}@{revision}");
        }
        Operation::ResolveFailure {
            signature,
            support_artifact,
            passing_run,
        } => {
            let _ = write!(
                identity,
                " · {signature} · {support_artifact} · {passing_run}"
            );
        }
        Operation::Usage { record } => {
            let _ = write!(
                identity,
                " · {} · #{}",
                record.request_id, record.owner.partner
            );
        }
        Operation::SetFlavor { enabled } => {
            let _ = write!(identity, " = {enabled}");
        }
        Operation::SetRibbonHidden { kind, hidden } => {
            let _ = write!(identity, " / {} = {hidden}", kind.as_str());
        }
        Operation::Load { .. } => {}
    }
    identity
}
fn operation_detail(operation: &store::records::Operation, paint: &Paint) -> String {
    use store::records::Operation;
    match operation {
        Operation::SetRibbonHidden { kind, hidden } => paint.format(
            Template::RibbonsVisibilityDetail,
            &[
                ("kind", kind.as_str()),
                ("hidden", if *hidden { "true" } else { "false" }),
            ],
        ),
        Operation::ObserveInstruction {
            request_id,
            owner,
            instruction,
        } => paint.format(
            Template::RecordsFailedInstruction,
            &[
                ("request", request_id),
                ("owner", &owner.name),
                ("instruction", instruction),
            ],
        ),
        Operation::SetPairing {
            partner,
            form,
            selection,
        } => selection
            .as_ref()
            .map(|value| pairing(value, paint))
            .unwrap_or_else(|| {
                paint.format(
                    Template::RecordsRemovePairing,
                    &[
                        ("partner", &partner.to_string()),
                        ("form", FORMS[(*form).min(8)].name),
                    ],
                )
            }),
        Operation::SetHeld { target, .. } => paint.format(
            Template::HeldTarget,
            &[("target", &super::held::target_label(target.as_ref(), paint))],
        ),
        Operation::RenewHeld { review } => format!(
            "{}\n{}\n{}\n{}",
            paint.hint(Message::HeldRenewalOld),
            super::held::target_label(Some(&review.expected), paint),
            paint.hint(Message::HeldRenewalNew),
            super::held::target_label(Some(&review.proposed), paint),
        ),
        Operation::SetFlavor { enabled } => paint
            .hint(if *enabled {
                Message::RecordsFlavorEnabled
            } else {
                Message::RecordsFlavorDisabled
            })
            .to_owned(),
        Operation::DismissInstruction { key } => {
            paint.format(Template::RecordsHabitHash, &[("key", key)])
        }
        Operation::BindInstructionMemory {
            key,
            memory,
            revision,
        } => paint.format(
            Template::RecordsMemoryBinding,
            &[
                ("key", key),
                ("memory", &memory.to_string()),
                ("revision", &revision.to_string()),
            ],
        ),
        Operation::ResolveFailure {
            signature,
            support_artifact,
            passing_run,
        } => paint.format(
            Template::RecordsResolutionChoice,
            &[
                ("signature", signature),
                ("support", support_artifact),
                ("run", passing_run),
                ("cause", paint.hint(Message::RecordsUnknown)),
            ],
        ),
        Operation::ObserveRun { run_id } => {
            paint.format(Template::RecordsRunRefs, &[("runs", run_id)])
        }
        Operation::Usage { record } => paint.format(
            Template::RecordsUsageOwner,
            &[
                ("request", &record.request_id),
                ("owner", &record.owner.name),
            ],
        ),
        Operation::Load { .. } => paint.hint(Message::RecordsOpLoad).to_owned(),
    }
}
pub(super) fn task_routes<'a>(
    state: &'a crate::app::records::State,
    task: i64,
    paint: &Paint,
    lines: &mut Vec<Line<'a>>,
) {
    let traces = state.routes_for_task(task);
    if traces.is_empty() {
        lines.push(Line::from(paint.hint(Message::RecordsNoRoutes)));
    } else {
        lines.extend(traces.iter().map(|trace| route_chain(trace, paint)));
    }
}
fn route_chain<'a>(trace: &'a crate::app::records::RouteTrace, paint: &Paint) -> Line<'a> {
    let mut spans = Vec::with_capacity(trace.visits.len() * 2 + 2);
    spans.push(Span::styled(trace.name.as_str(), paint.accent));
    spans.push(Span::raw(": "));
    for (index, visit) in trace.visits.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(if visit.discontinuity { " / " } else { " -> " }));
        }
        spans.push(Span::styled(
            FORMS[visit.form.min(8)].name,
            if visit.produced {
                paint.accent
            } else {
                paint.muted
            },
        ));
    }
    Line::from(spans)
}
fn route_lines<'a>(
    state: &'a crate::app::records::State,
    task: i64,
    partner: i64,
    paint: &Paint,
) -> Vec<Line<'a>> {
    let Some(trace) = state
        .routes_for_task(task)
        .iter()
        .find(|trace| trace.partner == partner)
    else {
        return vec![Line::from(paint.hint(Message::RecordsNoRoutes))];
    };
    let mut lines = Vec::with_capacity(trace.receipts.len() * 2 + 1);
    lines.push(route_chain(trace, paint));
    for index in &trace.receipts {
        let route = &state.snapshot.routes[*index];
        if let Some(trigger) = &route.accepted_suggestion {
            lines.push(Line::from(paint.format(
                Template::RecordsAcceptedSuggestion,
                &[("trigger", trigger)],
            )));
        }
        lines.push(Line::from(paint.format(
            Template::RecordsRouteReceipt,
            &[
                ("ordinal", &route.ordinal.to_string()),
                ("date", &utc_date(route.at)),
                ("reference", &route.id),
            ],
        )));
    }
    lines
}
/// Gregorian date from the persisted Unix-millisecond fact, always UTC.
fn utc_date(milliseconds: i64) -> String {
    let days = milliseconds.div_euclid(86_400_000) + 719468;
    let era = days.div_euclid(146097);
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn met_dates_use_utc_days_not_display_clock_or_timezone() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(86_399_999), "1970-01-01");
        assert_eq!(utc_date(86_400_000), "1970-01-02");
        assert_eq!(utc_date(951_782_400_000), "2000-02-29");
    }
}
