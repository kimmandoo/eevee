//! Factual cooperation surfaces. Rendering cannot dispatch or answer work.
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{BorderType, Clear, Paragraph, Wrap},
};

use super::{
    Paint, artwork::Artwork, centered, clipped, panel, row, selection_details, sheet, text,
};
use crate::{app::App, config::Config, store};

/// Extend the factual header without moving the composer or native portrait.
pub(super) fn header(frame: &mut Frame, area: Rect, app: &App, paint: &Paint) -> Rect {
    let mut offset = 0;
    let file_output = app.planning.has_current_file_output();
    if let Some(claim) = app.write_owner().filter(|claim| {
        !app.partner_approval_pending(claim.owner.partner)
            && (!file_output || claim.owner.partner != app.lead().id)
    }) {
        let owner = store::Attribution::label(Some(&claim.owner));
        let label = paint.format(
            crate::messages::Template::ProvidersPlanningPreparingChanges,
            &[("owner", &owner)],
        );
        let paragraph = Paragraph::new(label)
            .style(paint.accent)
            .wrap(Wrap { trim: false });
        let height = paragraph
            .line_count(area.width.max(1))
            .min(area.height as usize) as u16;
        frame.render_widget(paragraph, row(area, offset, height));
        offset += height;
    }
    if let Some((position, total)) = app.approval_position() {
        let owner = approval_owner(app);
        let label = format!("Approval {position} of {total} · {owner} · /approvals");
        let paragraph = Paragraph::new(label)
            .style(paint.accent)
            .wrap(Wrap { trim: false });
        let height = paragraph
            .line_count(area.width.max(1))
            .min(area.height.saturating_sub(offset) as usize) as u16;
        frame.render_widget(paragraph, row(area, offset, height));
        offset += height;
    } else if app.approval_count() > 0 {
        let label = if app.approval_shortcut_available() {
            paint.format(
                crate::messages::Template::CorePendingReviewBanner,
                &[("count", &app.approval_count().to_string())],
            )
        } else {
            paint
                .hint(crate::messages::Message::CoreWaitingForUser)
                .into()
        };
        text(frame, row(area, offset, 1), &label, paint.accent);
        offset += 1;
        if let Some((owner, path)) = app.pending_approval_detail() {
            // This clipped identity/path is a signpost, never the consent surface.
            let owner = store::Attribution::label(Some(owner));
            let detail = if let Some(path) = path {
                format!("{owner} · {path}")
            } else {
                owner
            };
            text(
                frame,
                row(area, offset, 1),
                &clipped(&detail, area.width as usize),
                paint.accent,
            );
            offset += 1;
        }
    }
    if app.approval_count() == 0
        && !file_output
        && let Some(transition) = app.evolution.transition
        && transition.from == transition.to
        && offset + 1 < area.height
    {
        let fresh = if transition.fresh {
            format!(
                " · {}",
                paint.hint(crate::messages::Message::CoreEvolutionFresh)
            )
        } else {
            String::new()
        };
        let label = paint.format(
            crate::messages::Template::CoreEvolutionTransition,
            &[
                (
                    "mode",
                    paint.hint(if app.lead().auto_form {
                        crate::messages::Message::CoreEvolutionAuto
                    } else {
                        crate::messages::Message::CoreEvolutionManual
                    }),
                ),
                ("from", crate::domain::FORMS[transition.from].name),
                ("to", crate::domain::FORMS[transition.to].name),
                ("reason", paint.hint(transition.reason)),
                ("fresh", &fresh),
            ],
        );
        let paragraph = Paragraph::new(label)
            .style(paint.muted)
            .wrap(Wrap { trim: false });
        let height = paragraph
            .line_count(area.width.max(1))
            .min(2)
            .min(area.height.saturating_sub(offset + 1) as usize) as u16;
        frame.render_widget(paragraph, row(area, offset, height));
        offset += height;
    }
    row(area, offset, area.height.saturating_sub(offset))
}

/// A transient, nonmodal receipt. It grants nothing and leaves the editor intact.
pub(super) fn evolution_popup(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
) {
    if !app.evolution_popup_ready() {
        return;
    }
    let Some(transition) = app.evolution.transition else {
        return;
    };
    frame.render_widget(Clear, area);
    let title = format!(
        " {} ",
        paint.hint(crate::messages::Message::CoreEvolutionPopupTitle)
    );
    let mode = paint.hint(if app.lead().auto_form {
        crate::messages::Message::CoreEvolutionAuto
    } else {
        crate::messages::Message::CoreEvolutionManual
    });
    let mut block = panel(&title, config, paint, true)
        .title_top(Line::styled(format!(" {mode} "), paint.muted).right_aligned())
        .title_bottom(
            Line::styled(
                format!(
                    " {} ",
                    paint.hint(crate::messages::Message::CoreEvolutionPopupContinue)
                ),
                paint.muted,
            )
            .centered(),
        );
    if !config.ascii {
        block = block.border_type(BorderType::Rounded);
    }
    let content = block.inner(area);
    frame.render_widget(block, area);
    // Only the connector moves; form names keep their cells throughout the
    // bounded picker animation. The existing portrait keeps its own renderer.
    let arrow = match app.evolution_frame() {
        Some(0) => " >  ",
        Some(1) => " >> ",
        Some(_) => "  > ",
        None if config.ascii => " -> ",
        None => " →  ",
    };
    let hero = Line::from(vec![
        Span::styled(crate::domain::FORMS[transition.from].name, paint.muted),
        Span::styled(arrow, paint.accent),
        Span::styled(crate::domain::FORMS[transition.to].name, paint.selected),
    ])
    .centered();
    frame.render_widget(Paragraph::new(hero), row(content, 0, 1));
    // Why it changed, then what the new form does, in plain words.
    let form = &crate::domain::FORMS[transition.to];
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(paint.hint(transition.reason), paint.muted),
            Line::styled(form.summary, paint.surface),
        ])
        .wrap(Wrap { trim: true }),
        row(content, 1, content.height.saturating_sub(1)),
    );
    app.evolution.popup_shown = true;
}

pub(super) fn approval_owner(app: &App) -> String {
    if app.overlay == Some(crate::app::Overlay::Execution) {
        store::Attribution::label(app.execution.owner.as_ref())
    } else {
        store::Attribution::label(app.planning.owner())
    }
}

/// Only real working/waiting identities take space above the composer.
pub(super) fn strip(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    paint: &Paint,
    artwork: &Artwork,
) -> Rect {
    let rows = app.work_rows();
    if rows.is_empty()
        || rows.len() == 1 && rows[0].owner.partner == app.lead().id && app.own_approval_pending()
    {
        return area;
    }
    let height = if app.width < 90 {
        1
    } else {
        rows.len().min(4) as u16
    }
    .min(area.height);
    let board = row(area, area.height.saturating_sub(height), height);
    if app.width < 90 {
        text(
            frame,
            board,
            &format!("{} partners working / waiting · /party-board", rows.len()),
            paint.accent,
        );
    } else {
        for (index, work) in rows.iter().take(4).enumerate() {
            let line = row(board, index as u16, 1);
            let line = if artwork.original(work.owner.form).is_some() {
                let sprite = working_sprite(line);
                artwork.render(work.owner.form, sprite, frame.buffer_mut());
                Rect::new(
                    line.x + sprite.width + 1,
                    line.y,
                    line.width.saturating_sub(sprite.width + 1),
                    1,
                )
            } else {
                line
            };
            let label = format!(
                "{} · {} · {}",
                store::Attribution::label(Some(&work.owner)),
                work.status,
                work.detail
            );
            text(
                frame,
                line,
                &clipped(&label, line.width as usize),
                paint.muted,
            );
        }
    }
    row(area, 0, area.height.saturating_sub(height))
}

fn working_sprite(line: Rect) -> Rect {
    Rect::new(line.x, line.y, line.width.min(6), line.height.min(1))
}

/// Shares the exact HUD geometry with cell preparation and native erasure.
pub(super) fn sprite_targets(app: &App, area: Rect, targets: &mut Vec<(usize, Rect)>) {
    if app.width < 90 {
        return;
    }
    let forms = app.working_forms().take(4);
    let height = (forms.clone().count() as u16).min(area.height);
    let board = row(area, area.height.saturating_sub(height), height);
    targets.extend(
        forms
            .take(height as usize)
            .enumerate()
            .map(|(index, form)| (form, working_sprite(row(board, index as u16, 1)))),
    );
}

pub(super) fn board(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = sheet(area, 106, 26);
    frame.render_widget(Clear, area);
    let block = panel(
        paint.hint(crate::messages::Message::DesksCooperationBoardPanel),
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let rows = app.work_rows();
    if rows.is_empty() {
        text(
            frame,
            content,
            paint.hint(crate::messages::Message::DesksCooperationNoPartnerOwnsWorkingOrWaitingWork),
            paint.muted,
        );
        return;
    }
    let selected = app
        .cooperation_ui
        .board_owner
        .and_then(|owner| rows.iter().position(|row| row.owner.partner == owner))
        .unwrap_or_else(|| {
            app.cooperation_ui
                .board_selected
                .min(rows.len().saturating_sub(1))
        });
    app.cooperation_ui.board_selected = selected;
    app.cooperation_ui.board_owner = rows.get(selected).map(|row| row.owner.partner);
    let mut offset = 0;
    for (index, work) in rows.iter().take(4).enumerate() {
        let label = format!(
            "{}{}",
            if index == selected { "> " } else { "  " },
            store::Attribution::label(Some(&work.owner))
        );
        let paragraph = Paragraph::new(label)
            .style(if index == selected {
                paint.selected
            } else {
                paint.surface
            })
            .wrap(Wrap { trim: false });
        let height = paragraph.line_count(content.width.max(1)) as u16;
        frame.render_widget(paragraph, row(content, offset, height));
        offset += height;
        text(
            frame,
            row(content, offset, 1),
            &format!(
                "  {} · {}",
                work.status,
                if work.local {
                    "local command"
                } else {
                    "provider request"
                }
            ),
            paint.muted,
        );
        offset += 1;
    }
    let details = row(content, offset, content.height.saturating_sub(offset + 3));
    if let Some(work) = rows.get(selected) {
        let detail = app.cooperation_work_details(work);
        let detail = crate::app::execution::safe_text(&detail);
        let paragraph = Paragraph::new(detail)
            .style(paint.surface)
            .wrap(Wrap { trim: false });
        app.cooperation_ui.board_max =
            selection_details(frame, details, paragraph, app.cooperation_ui.board_scroll);
        app.cooperation_ui.board_scroll = app
            .cooperation_ui
            .board_scroll
            .min(app.cooperation_ui.board_max);
    }
    text(
        frame,
        row(content, content.height.saturating_sub(3), 3),
        paint.hint(crate::messages::Message::DesksCooperationUpDownChooseOwnerEnterFrontOwner),
        paint.accent,
    );
}

pub(super) fn release(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
) {
    let storage = app.storage_label().to_owned();
    let Some(review) = &mut app.cooperation_ui.release else {
        return;
    };
    review.reviewed = false;
    let minimum = area.width >= 60 && area.height >= 18;
    let area = if minimum {
        centered(area, 92, 24)
    } else {
        area
    };
    frame.render_widget(Clear, area);
    let block = panel(
        paint.hint(crate::messages::Message::DesksCooperationReleasePanel),
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let disclosure = paint.format(
        crate::messages::Template::DesksCooperationReleaseDisclosure,
        &[
            ("owner", &(store::Attribution::label(Some(&review.owner)))),
            ("partner", &(review.owner.partner).to_string()),
        ],
    );
    let paragraph = Paragraph::new(disclosure)
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    let height = paragraph
        .line_count(content.width.max(1))
        .min(u16::MAX as usize) as u16;
    frame.render_widget(paragraph, row(content, 0, content.height.saturating_sub(4)));
    let message = if review.pending {
        paint.hint(crate::messages::Message::DesksCooperationWaitingForCommittedRosterAcknowledgementNoSuccess).to_owned()
    } else if let Some(refusal) = &review.refusal {
        format!("Refused: {refusal}")
    } else {
        paint.hint(crate::messages::Message::DesksCooperationOnlyExplicitConfirmationRequestsRemovalEscKeeps).to_owned()
    };
    let status = Paragraph::new(message)
        .style(paint.accent)
        .wrap(Wrap { trim: false });
    let status_height = status
        .line_count(content.width.max(1))
        .min(u16::MAX as usize) as u16;
    frame.render_widget(
        status,
        row(content, height, content.height.saturating_sub(height + 3)),
    );
    review.reviewed = minimum && !review.pending && content.height >= height + status_height + 3;
    if !minimum {
        review.affirmative = false;
    }
    let choices = Line::from(vec![
        Span::styled(
            paint.hint(crate::messages::Message::DesksCooperationStay),
            if review.affirmative {
                paint.surface
            } else {
                paint.selected
            },
        ),
        Span::raw("  "),
        Span::styled(
            paint.hint(crate::messages::Message::DesksCooperationReleaseSelectedPartner),
            if review.affirmative {
                paint.selected
            } else {
                paint.surface
            },
        ),
    ]);
    frame.render_widget(
        Paragraph::new(choices),
        row(content, content.height.saturating_sub(3), 1),
    );
    text(
        frame,
        row(content, content.height.saturating_sub(2), 1),
        if review.pending {
            paint.hint(
                crate::messages::Message::DesksCooperationWaitingForStorageRemovalCannotBeUndone,
            )
        } else if !minimum {
            paint.hint(crate::messages::Message::DesksCooperationResizeToXToReviewEscStay)
        } else {
            paint.hint(crate::messages::Message::DesksCooperationYRequestLeftStayRightReleaseEnter)
        },
        paint.muted,
    );
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        &storage,
        paint.muted,
    );
}

pub(super) fn cancellation(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
) {
    let storage = app.storage_label().to_owned();
    let Some(review) = &mut app.cooperation.cancel_review else {
        return;
    };
    review.reviewed = false;
    let minimum = area.width >= 60 && area.height >= 18;
    let area = if minimum {
        centered(area, 108, area.height.saturating_sub(2))
    } else {
        area
    };
    frame.render_widget(Clear, area);
    let block = panel(
        if review.quit {
            paint.hint(crate::messages::Message::DesksCooperationQuitPanel)
        } else {
            paint.hint(crate::messages::Message::DesksCooperationCancelPanel)
        },
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let summary = Paragraph::new(review.summary.as_str())
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    let height = summary.line_count(content.width.max(1));
    let body = row(content, 0, content.height.saturating_sub(4));
    frame.render_widget(summary, body);
    // Never acknowledge a clipped owner scope, including below-minimum layouts.
    review.reviewed = body.width >= 20 && body.height as usize >= height;
    if !review.reviewed {
        review.affirmative = false;
    }
    let choices = Line::from(vec![
        Span::styled(
            paint.hint(crate::messages::Message::DesksCooperationStay),
            if review.affirmative {
                paint.surface
            } else {
                paint.selected
            },
        ),
        Span::raw("  "),
        Span::styled(
            if review.quit {
                paint.hint(crate::messages::Message::DesksCooperationCancelAllAndWaitThenExit)
            } else {
                paint.hint(crate::messages::Message::DesksCooperationCancelAllAndWait)
            },
            if review.affirmative {
                paint.selected
            } else {
                paint.surface
            },
        ),
    ]);
    frame.render_widget(
        Paragraph::new(choices).wrap(Wrap { trim: false }),
        row(content, content.height.saturating_sub(4), 2),
    );
    text(
        frame,
        row(content, content.height.saturating_sub(2), 1),
        if review.reviewed {
            paint.hint(crate::messages::Message::DesksCooperationYConfirmLeftStayRightActionEnter)
        } else {
            paint.hint(crate::messages::Message::DesksCooperationResizeToInspectAllOwnersEscStay)
        },
        paint.accent,
    );
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        &storage,
        paint.muted,
    );
}

pub(super) fn journal_discard(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
) {
    use std::fmt::Write;
    let storage = app.storage_label().to_owned();
    let Some(review) = &mut app.cooperation.journal_discard else {
        return;
    };
    review.reviewed = false;
    let minimum = area.width >= 60 && area.height >= 18;
    let area = if minimum {
        centered(area, 108, area.height.saturating_sub(2))
    } else {
        area
    };
    frame.render_widget(Clear, area);
    let block = panel(
        paint.hint(crate::messages::Message::DesksCooperationDiscardExactFailedJournalWritesDIds),
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let body = row(content, 0, content.height.saturating_sub(3));
    if app.cooperation_ui.journal_discard_details {
        review.affirmative = false;
        let mut detail = paint.format(
            crate::messages::Template::DesksCooperationJournalDetails,
            &[
                ("owner", &(store::Attribution::label(Some(&review.owner)))),
                ("task", &(review.task).to_string()),
                ("request", &(review.request_id)),
                ("count", &(review.count).to_string()),
            ],
        );
        for id in &review.ids {
            let _ = writeln!(detail, "{id}");
        }
        let _ = write!(
            detail,
            "{}",
            paint.format(
                crate::messages::Template::DesksCooperationJournalError,
                &[("error", &(review.error)),]
            )
        );
        let detail = crate::app::execution::safe_text(&detail);
        let paragraph = Paragraph::new(detail)
            .style(paint.surface)
            .wrap(Wrap { trim: false });
        app.cooperation_ui.journal_discard_max = selection_details(
            frame,
            body,
            paragraph,
            app.cooperation_ui.journal_discard_scroll,
        );
        app.cooperation_ui.journal_discard_scroll = app
            .cooperation_ui
            .journal_discard_scroll
            .min(app.cooperation_ui.journal_discard_max);
    } else {
        let summary = crate::app::execution::safe_text(&review.summary());
        let paragraph = Paragraph::new(summary)
            .style(paint.surface)
            .wrap(Wrap { trim: false });
        let height = paragraph.line_count(body.width.max(1));
        frame.render_widget(paragraph, body);
        review.reviewed = minimum && body.height as usize >= height;
        if !review.reviewed {
            review.affirmative = false;
        }
    }
    let choices = Line::from(vec![
        Span::styled(
            paint.hint(crate::messages::Message::DesksCooperationStay),
            if review.affirmative {
                paint.surface
            } else {
                paint.selected
            },
        ),
        Span::raw("  "),
        Span::styled(
            paint.hint(crate::messages::Message::DesksCooperationDiscardFailedWrites),
            if review.affirmative {
                paint.selected
            } else {
                paint.surface
            },
        ),
    ]);
    frame.render_widget(
        Paragraph::new(choices),
        row(content, content.height.saturating_sub(3), 1),
    );
    text(
        frame,
        row(content, content.height.saturating_sub(2), 1),
        if app.cooperation_ui.journal_discard_details {
            paint.hint(crate::messages::Message::DesksCooperationDSummaryPgupDnIdsAndError)
        } else if review.reviewed {
            paint.hint(crate::messages::Message::DesksCooperationDDetailsYConfirmLeftRightEnter)
        } else {
            paint.hint(crate::messages::Message::DesksCooperationResizeToInspectFullScopeDDetails)
        },
        paint.accent,
    );
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        &storage,
        paint.muted,
    );
}
