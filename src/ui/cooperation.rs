//! Factual cooperation surfaces. Rendering cannot dispatch or answer work.
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};

use super::{
    Paint, artwork::Artwork, centered, clipped, panel, row, selection_details, sheet, text,
};
use crate::{app::App, config::Config, store};

/// Extend the factual header without moving the composer or native portrait.
pub(super) fn header(frame: &mut Frame, area: Rect, app: &App, paint: &Paint) -> Rect {
    let mut offset = 0;
    if let Some(claim) = app.write_owner() {
        let label = format!(
            "{} is editing",
            store::Attribution::label(Some(&claim.owner))
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
        let label = format!("{} pending approvals · /approvals", app.approval_count());
        text(frame, row(area, offset, 1), &label, paint.accent);
        offset += 1;
    }
    row(area, offset, area.height.saturating_sub(offset))
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
    if rows.is_empty() {
        return area;
    }
    let height = if app.width < 90 {
        1
    } else {
        rows.len().min(3) as u16
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
        for (index, work) in rows.iter().take(3).enumerate() {
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
    let forms = app.working_forms().take(3);
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
        " Party board / working and waiting only ",
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
            "No partner owns working or waiting work.\nIdle partners run nothing. Esc returns; /party lists everyone.",
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
    for (index, work) in rows.iter().take(3).enumerate() {
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
        "Up/Down choose owner · Enter front owner only\nPgUp/Dn selected details · Esc Back\nNo request, approval or cancellation by browsing.",
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
    let block = panel(" Release partner / local roster only ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let disclosure = format!(
        "Selected: {} · #{}\nRemove: personal roster entry, own drafts/navigation and proposed recipient intents.\nRetain: task facts, sources, runs and provider journals with original name/form attribution snapshots.\nMoves/pairings are not stored; none are removed.\nNo repository/provider erasure; not secure deletion.",
        store::Attribution::label(Some(&review.owner)),
        review.owner.partner,
    );
    let paragraph = Paragraph::new(disclosure)
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    let height = paragraph
        .line_count(content.width.max(1))
        .min(u16::MAX as usize) as u16;
    frame.render_widget(paragraph, row(content, 0, content.height.saturating_sub(4)));
    let message = if review.pending {
        "Waiting for committed roster acknowledgement; no success claimed.".to_owned()
    } else if let Some(refusal) = &review.refusal {
        format!("Refused: {refusal}")
    } else {
        "Only explicit confirmation requests removal. Esc keeps everything.".to_owned()
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
            " Stay ",
            if review.affirmative {
                paint.surface
            } else {
                paint.selected
            },
        ),
        Span::raw("  "),
        Span::styled(
            " Release selected partner ",
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
            "Waiting for storage; removal cannot be undone by Esc."
        } else if !minimum {
            "Resize to 60x18 to review. Esc Stay; no removal here."
        } else {
            "y Request / Left Stay / Right Release / Enter / Esc Stay"
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
            " Quit / ALL owned work "
        } else {
            " Cancel ALL owned work "
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
            " Stay ",
            if review.affirmative {
                paint.surface
            } else {
                paint.selected
            },
        ),
        Span::raw("  "),
        Span::styled(
            if review.quit {
                " Cancel ALL and wait, then exit "
            } else {
                " Cancel ALL and wait "
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
            "y Confirm / Left Stay / Right action / Enter / Esc Stay"
        } else {
            "Resize to inspect ALL owners. Esc Stay; none answered."
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
        " Discard exact failed journal writes / d IDs and error ",
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let body = row(content, 0, content.height.saturating_sub(3));
    if app.cooperation_ui.journal_discard_details {
        review.affirmative = false;
        let mut detail = format!(
            "DETAILS / no consent here\nOwner: {}\nTask: {}\nRequest: {}\nExact failed write attempt IDs ({}; repeats retained):\n",
            store::Attribution::label(Some(&review.owner)),
            review.task,
            review.request_id,
            review.count,
        );
        for id in &review.ids {
            let _ = writeln!(detail, "{id}");
        }
        let _ = write!(
            detail,
            "\nStorage/refusal error:\n{}\n\nCommitted observations remain. Discarding loses unsaved write attempts and leaves a coverage gap; not persistence, retry, purge or replay.",
            review.error
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
            " Stay ",
            if review.affirmative {
                paint.surface
            } else {
                paint.selected
            },
        ),
        Span::raw("  "),
        Span::styled(
            " Discard failed writes ",
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
            "d Summary / PgUp/Dn IDs and error / Esc Stay"
        } else if review.reviewed {
            "d Details / y Confirm / Left/Right / Enter / Esc Stay"
        } else {
            "Resize to inspect full scope. d Details / Esc Stay"
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
