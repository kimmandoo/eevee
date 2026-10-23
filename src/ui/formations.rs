//! One shallow local review, with fixed controls and inspectable portable bytes.
use super::*;
use crate::app::formations::Mode;

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
    } else if !state.inspect {
        draw(paint.hint(Message::FormationPolicy), paint.muted);
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
            draw(paint.hint(Message::FormationModes), paint.muted);
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
                let target = state.mapping[state.selected];
                let snapshot = app.partner_drafts.get(&target.unwrap_or(0));
                let existing = snapshot.is_some_and(|d| {
                    d.drafts
                        .iter()
                        .any(|d| d.field == EditorField::Request && !d.text.is_empty())
                });
                let status = if existing {
                    Message::FormationKept
                } else if slot.request.is_empty() {
                    Message::FormationEmpty
                } else {
                    Message::FormationStage
                };
                draw(paint.hint(status), paint.muted);
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
