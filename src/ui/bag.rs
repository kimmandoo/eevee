//! The bag and item notices. Rows only describe and open existing actions.
use super::*;

pub(super) fn render(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
    artwork: &Artwork,
) {
    let area = sheet(area, 84, 24);
    frame.render_widget(Clear, area);
    let block = panel(" Bag ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let rows = app.bag_rows();
    let selected = app.bag.selected.min(rows.len().saturating_sub(1));
    app.bag.selected = selected;
    // Wide bags show the selected item's sprite beside the list.
    let wide = content.width >= 70;
    let list = if wide {
        Rect::new(content.x, content.y, content.width - 14, content.height)
    } else {
        content
    };
    let footer_rows = 3;
    let body = row(list, 0, list.height.saturating_sub(footer_rows));
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line = 0;
    let mut pocket = None;
    for (index, entry) in rows.iter().enumerate() {
        if pocket != Some(entry.pocket) {
            if pocket.is_some() {
                lines.push(Line::raw(""));
            }
            pocket = Some(entry.pocket);
            lines.push(Line::styled(entry.pocket.label(), paint.muted));
        }
        if index == selected {
            selected_line = lines.len();
        }
        let style = if index == selected {
            paint.selected
        } else {
            paint.surface
        };
        lines.push(Line::from(vec![
            Span::styled(if index == selected { "> " } else { "  " }, style),
            Span::styled(format!("{:<5}", entry.item.icon()), paint.accent),
            Span::styled(
                clipped(&entry.name, usize::from(body.width.saturating_sub(8))).into_owned(),
                style,
            ),
        ]));
        if app.bag.details && index == selected {
            lines.push(Line::styled(
                format!("       {}", entry.detail),
                paint.muted,
            ));
        }
    }
    let height = usize::from(body.height.max(1));
    let top = selected_line.saturating_sub(height.saturating_sub(2));
    frame.render_widget(
        Paragraph::new(lines.into_iter().skip(top).collect::<Vec<_>>()).style(paint.surface),
        body,
    );
    if let Some(entry) = rows.get(selected) {
        text(
            frame,
            row(list, list.height.saturating_sub(footer_rows), 2),
            &entry.detail,
            paint.muted,
        );
        if wide {
            let art = Rect::new(content.right() - 12, content.y + 1, 12, 6);
            if !artwork.render_item(entry.item, art, frame.buffer_mut()) {
                text(frame, art, entry.item.icon(), paint.accent);
            }
        }
    }
    super::choices(
        frame,
        row(list, list.height.saturating_sub(1), 1),
        "Up/Down Choose | Enter Open | d Details | Esc Back",
        paint.accent,
        paint,
    );
}

/// Same anchored slot as the evolution notice; yields to it and to consent.
pub(super) fn notice_area(area: Rect, app: &App) -> Option<Rect> {
    if app.item_notice_visible().is_none() || evolution_popup_area(area, app).is_some() {
        return None;
    }
    let layout = shell(area, app.linear, app.effective_focus());
    if let Some(rail) = layout.rail {
        let top = rail.bottom().saturating_add(1);
        if layout.composer.y.saturating_sub(top) >= 7 {
            return Some(Rect::new(rail.x, top, rail.width, 7));
        }
    }
    Some(Rect::new(
        layout.workspace.x,
        layout.workspace.y,
        layout.workspace.width,
        6.min(layout.workspace.height),
    ))
}

pub(super) fn notice(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    config: &Config,
    paint: &Paint,
    artwork: &Artwork,
) {
    let Some(notice) = app.item_notice_visible() else {
        return;
    };
    frame.render_widget(Clear, area);
    let title = format!(" {} ", notice.title);
    let mut block = panel(&title, config, paint, true)
        .title_bottom(Line::styled(" Keep typing ", paint.muted).centered());
    if !config.ascii {
        block = block.border_type(BorderType::Rounded);
    }
    let content = block.inner(area);
    frame.render_widget(block, area);
    let art_width = if content.width >= 20 { 7 } else { 0 };
    if art_width > 0 {
        let art = Rect::new(content.x, content.y, art_width, content.height.min(3));
        if !artwork.render_item(notice.item, art, frame.buffer_mut()) {
            text(frame, art, notice.item.icon(), paint.accent);
        }
    }
    let body = Rect::new(
        content.x + art_width,
        content.y,
        content.width.saturating_sub(art_width),
        content.height,
    );
    text(frame, body, &notice.text, paint.surface);
}
