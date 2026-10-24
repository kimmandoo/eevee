//! One shallow local review, with fixed controls and inspectable portable bytes.
use super::*;
use crate::app::formations::{Inspector, Mode, Route, Staging};

pub(super) fn render(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    frame.render_widget(Clear, area);
    let area = centered(area, 110, area.height.saturating_sub(2).min(28));
    let block = panel(paint.hint(Message::FormationTitle), config, paint, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 30 || inner.height < 6 {
        app.formations.invalidate();
        return;
    }
    let state = &app.formations;
    let bottom = inner.bottom().saturating_sub(1);
    let mut y = inner.y;
    let mut complete = true;
    let mut draw = |value: &str, style: Style| {
        let paragraph = Paragraph::new(value)
            .wrap(Wrap { trim: false })
            .style(style);
        let needed = paragraph.line_count(inner.width).min(u16::MAX as usize) as u16;
        let height = needed.min(bottom.saturating_sub(y));
        if height < needed {
            complete = false;
        }
        frame.render_widget(paragraph, Rect::new(inner.x, y, inner.width, height));
        y += height;
    };
    if state.editing() {
        draw(
            paint.hint(if state.mode == Mode::SavePath {
                Message::FormationSave
            } else {
                Message::FormationLoad
            }),
            paint.accent,
        );
        let before = &state.path[..state.cursor];
        let start = before
            .grapheme_indices(true)
            .rev()
            .nth(inner.width.saturating_sub(3) as usize)
            .map_or(0, |(offset, _)| offset);
        draw(
            &format!("{}|{}", &before[start..], &state.path[state.cursor..]),
            paint.base,
        );
    } else if state.inspect {
        draw(paint.hint(Message::FormationModes), paint.muted);
    } else {
        draw(
            paint.hint(if state.mode == Mode::Mapping {
                Message::FormationReadinessPolicy
            } else {
                Message::FormationPolicy
            }),
            paint.muted,
        );
        if state.mode == Mode::Export {
            let path = std::path::Path::new(&state.path);
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::path::Path::new(&app.cwd).join(path)
            };
            draw(
                &super::display_identity(&path.to_string_lossy()),
                paint.accent,
            );
        } else if let Some(recipe) = &state.recipe {
            draw(paint.hint(Message::FormationReadinessBoundary), paint.muted);
            let roster = &state.roster;
            for (index, slot) in recipe.roles.iter().enumerate() {
                let partner = state
                    .mapping
                    .get(index)
                    .copied()
                    .flatten()
                    .and_then(|id| roster.iter().find(|p| p.id == id));
                let name = partner.map_or_else(
                    || paint.hint(Message::FormationUnmapped).to_owned(),
                    App::partner_label,
                );
                let end = slot
                    .role
                    .grapheme_indices(true)
                    .nth(6)
                    .map_or(slot.role.len(), |(offset, _)| offset);
                draw(
                    &format!(
                        "{} {}. {} {} {} {}",
                        if index == state.selected { ">" } else { " " },
                        index + 1,
                        &slot.role[..end],
                        slot.form,
                        if config.ascii { "->" } else { "→" },
                        name
                    ),
                    if index == state.selected {
                        paint.accent
                    } else {
                        paint.base
                    },
                );
            }
            if let Some(slot) = recipe.roles.get(state.selected) {
                let readiness = app.formation_readiness(state.selected);
                let mut staging = paint
                    .hint(match readiness.staging {
                        Staging::Eligible => Message::FormationStagingEligible,
                        Staging::Unmapped => Message::FormationStagingUnmapped,
                        Staging::Duplicate => Message::FormationStagingDuplicate,
                        Staging::Changed => Message::FormationStagingChanged,
                        Staging::Busy => Message::FormationStagingBusy,
                    })
                    .to_owned();
                if let Some(writer) = readiness.writer {
                    staging.push_str(" / ");
                    staging.push_str(&paint.format(
                        Template::FormationWriter,
                        &[("partner", &writer.to_string())],
                    ));
                } else if readiness.owner_busy {
                    staging.push_str(" / ");
                    staging.push_str(paint.hint(Message::FormationOwnerBusy));
                }
                draw(&staging, paint.accent);
                let moves = readiness.moves.map_or_else(
                    || paint.hint(Message::FormationMovesUnknown).to_owned(),
                    |count| {
                        paint.format(
                            Template::FormationMovesCount,
                            &[("count", &count.to_string())],
                        )
                    },
                );
                let held = paint.hint(match readiness.held {
                    crate::app::held::Readiness::NotConfigured => Message::FormationHeldNone,
                    crate::app::held::Readiness::Available => Message::FormationHeldAvailable,
                    crate::app::held::Readiness::Unavailable => Message::FormationHeldUnavailable,
                    crate::app::held::Readiness::Unknown => Message::FormationHeldUnknown,
                });
                draw(&format!("{moves} / {held}"), paint.muted);
                draw(paint.hint(route_message(readiness.route)), paint.muted);
                let existing = readiness.draft_nonempty;
                let status = if existing {
                    Message::FormationKept
                } else if slot.request.is_empty() {
                    Message::FormationEmpty
                } else {
                    Message::FormationStage
                };
                draw(paint.hint(status), paint.muted);
                draw(paint.hint(Message::FormationWarningHint), paint.muted);
            }
        }
    }
    if state.loading {
        draw(paint.hint(Message::FormationWorking), paint.accent);
    } else if let Some(notice) = state.notice {
        draw(paint.hint(notice), paint.accent);
    }
    if !state.error.is_empty() {
        draw(&state.error, paint.accent);
    }
    if y < bottom && !state.editing() {
        let details = Rect::new(inner.x, y, inner.width, bottom - y);
        let paragraph = Paragraph::new(state.details.as_str()).wrap(Wrap { trim: false });
        let max_scroll = paragraph
            .line_count(details.width)
            .saturating_sub(details.height as usize)
            .min(u16::MAX as usize) as u16;
        frame.render_widget(
            paragraph
                .scroll((state.scroll.min(max_scroll), 0))
                .style(paint.muted),
            details,
        );
    }
    let hint = if state.editing() {
        Message::FormationPathHint
    } else if state.inspect {
        Message::FormationInspectHint
    } else if state.mode == Mode::Export {
        Message::FormationExportHint
    } else {
        Message::FormationMapHint
    };
    text(
        frame,
        Rect::new(inner.x, bottom, inner.width, 1),
        paint.hint(hint),
        paint.accent,
    );
    state.reviewed.set(
        complete
            && !state.inspect
            && !state.editing()
            && !state.loading
            && app.width >= 60
            && app.height >= 18,
    );
}

fn route_message(route: Route) -> Message {
    match route {
        Route::Missing => Message::FormationRouteMissing,
        Route::Unknown => Message::FormationRouteUnknown,
        Route::Configured => Message::FormationRouteConfigured,
        Route::Observed => Message::FormationRouteObserved,
        Route::Unavailable => Message::FormationRouteUnavailable,
    }
}

/// The existing connection inspector in a mapped-owner, observational mode.
/// This branch never calls the mutable connection renderer (which can inspect
/// current configuration). It borrows only already committed/cached metadata.
pub(super) fn render_route_inspector(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    config: &Config,
    paint: &Paint,
) -> bool {
    let Some(Inspector::Route { partner, form }) = app.formations.inspector else {
        return false;
    };
    if !app.formation_inspector_active() {
        return false;
    }
    frame.render_widget(Clear, area);
    let area = centered(area, 100, area.height.saturating_sub(2).min(28));
    let block = panel(
        paint.hint(Message::ProvidersAgentsConnections),
        config,
        paint,
        true,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let name = app
        .formations
        .roster
        .iter()
        .find(|p| p.id == partner)
        .map_or_else(|| format!("#{partner}"), App::partner_label);
    let mut lines = vec![
        Line::styled(
            paint.format(
                Template::FormationInspectorOwner,
                &[("partner", &name), ("form", FORMS[form].name)],
            ),
            paint.accent,
        ),
        Line::from(paint.hint(Message::FormationReadinessBoundary)),
    ];
    if let Some(pairing) = app.formation_pairing(partner, form) {
        lines.push(Line::from(paint.format(
            Template::FormationInspectorRoute,
            &[("route", pairing.agent.label())],
        )));
        lines.push(Line::from(
            paint.format(
                Template::FormationInspectorModel,
                &[
                    (
                        "model",
                        pairing
                            .selection
                            .model
                            .as_deref()
                            .unwrap_or(paint.hint(Message::FormationProviderDefault)),
                    ),
                    (
                        "effort",
                        pairing
                            .selection
                            .effort
                            .as_deref()
                            .unwrap_or(paint.hint(Message::FormationProviderDefault)),
                    ),
                ],
            ),
        ));
        lines.push(Line::from(paint.hint(route_message(
            app.formation_readiness(app.formations.selected).route,
        ))));
        if app.agents.failure(pairing.agent.index())
            == Some(crate::providers::diagnostics::Failure::VersionCheck)
        {
            lines.push(Line::styled(
                paint.hint(Message::FormationProbeUnknown),
                paint.accent,
            ));
        }
        if let Some(profile) = app.agents.profiles.get(pairing.agent.index()) {
            lines.extend(
                profile
                    .blockers
                    .iter()
                    .map(|blocker| Line::styled(blocker.as_str(), paint.accent)),
            );
            lines.extend(
                profile
                    .trust_preview
                    .iter()
                    .map(|detail| Line::from(detail.as_str())),
            );
        }
    } else {
        lines.push(Line::from(paint.hint(
            if app.formation_readiness(app.formations.selected).route == Route::Unknown {
                Message::FormationRouteUnknown
            } else {
                Message::FormationInspectorMissing
            },
        )));
    }
    let body = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(1),
    );
    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .style(paint.base);
    let max_scroll = paragraph
        .line_count(body.width.max(1))
        .saturating_sub(body.height as usize)
        .min(u16::MAX as usize) as u16;
    frame.render_widget(
        paragraph.scroll((app.formations.inspector_scroll.min(max_scroll), 0)),
        body,
    );
    text(
        frame,
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        paint.hint(Message::FormationReadonly),
        paint.accent,
    );
    true
}
