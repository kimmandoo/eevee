//! Screen geometry shared by every surface; no drawing state.
use super::*;

pub(super) struct Shell {
    pub(super) header: Rect,
    pub(super) footer: Rect,
    pub(super) rail: Option<Rect>,
    pub(super) workspace: Rect,
    pub(super) composer: Rect,
    pub(super) inspector: Option<Rect>,
}

pub(super) fn shell(area: Rect, linear: bool, focus: Pane) -> Shell {
    let outer = area.inner(ratatui::layout::Margin::new(1, 0));
    let header = row(outer, 0, 2);
    let footer = row(outer, outer.height.saturating_sub(2), 2);
    let composer_height = if area.height >= 26 { 7 } else { 5 };
    let composer = row(
        outer,
        outer.height.saturating_sub(composer_height + 2),
        composer_height,
    );
    let body = row(outer, 2, composer.y.saturating_sub(outer.y + 2));
    // Keep the partner visible at roomy widths; focused details share its slot.
    let inspector = (!linear && area.width >= 120 && focus == Pane::Inspector)
        .then(|| Rect::new(body.right().saturating_sub(30), body.y, 30, body.height));
    let rail = (!linear && area.width >= 90 && inspector.is_none()).then(|| {
        Rect::new(
            body.right().saturating_sub(26),
            body.y,
            26,
            body.height.min(16),
        )
    });
    let reserved = if inspector.is_some() {
        32
    } else if rail.is_some() {
        28
    } else {
        0
    };
    Shell {
        header,
        footer,
        rail,
        workspace: Rect::new(
            body.x,
            body.y,
            body.width.saturating_sub(reserved),
            body.height,
        ),
        composer,
        inspector,
    }
}

pub(super) fn companion_portrait(area: Rect) -> Rect {
    Rect::new(
        area.x + 2,
        area.y + 3,
        area.width.saturating_sub(4),
        area.height.saturating_sub(6).min(10),
    )
}

pub(super) fn row(area: Rect, offset: u16, height: u16) -> Rect {
    let offset = offset.min(area.height);
    Rect::new(
        area.x,
        area.y.saturating_add(offset),
        area.width,
        height.min(area.height - offset),
    )
}

pub(super) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let available_height = area.height.saturating_sub(2);
    let height = height.min(available_height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// Temporary sheets leave the real status header and safe-return footer visible.
pub(super) fn sheet(area: Rect, width: u16, height: u16) -> Rect {
    centered(area, width, height.min(area.height.saturating_sub(4)))
}

pub(super) fn drawer(area: Rect) -> Rect {
    let width = 46.min(area.width.saturating_sub(2));
    Rect::new(
        area.right().saturating_sub(width + 1),
        area.y + 2,
        width,
        area.height.saturating_sub(4),
    )
}

pub(super) fn inner(area: Rect) -> Rect {
    area.inner(ratatui::layout::Margin::new(2, 1))
}

pub(super) fn portrait(area: Rect) -> Rect {
    let content = inner(area);
    let width = 32.min(content.width);
    let height = 16.min(content.height.saturating_sub(8));
    Rect::new(
        content.x + (content.width - width) / 2,
        content.y + 2,
        width,
        height,
    )
}

pub(super) fn picker(area: Rect, app: &App) -> (Rect, bool) {
    let grid = area.width >= 90 && area.height >= 30 && !app.linear;
    (
        sheet(
            area,
            if grid { 108 } else { 84 },
            if grid { 32 } else { 25 },
        ),
        grid,
    )
}

pub(super) fn grid_cell(area: Rect, index: usize) -> Rect {
    let grid = Rect::new(
        area.x,
        area.y + 2,
        area.width * 3 / 5,
        area.height.saturating_sub(2),
    );
    let col = index as u16 % 3;
    let row_index = index as u16 / 3;
    let x = grid.x + grid.width * col / 3;
    let y = grid.y + grid.height * row_index / 3;
    Rect::new(
        x,
        y,
        grid.width * (col + 1) / 3 - grid.width * col / 3,
        grid.height * (row_index + 1) / 3 - grid.height * row_index / 3,
    )
}

pub(super) fn thumbnail(area: Rect) -> Rect {
    let content = inner(area);
    Rect::new(
        content.x,
        content.y + 1,
        content.width,
        content.height.saturating_sub(2),
    )
}

pub(super) fn compact_picker_portrait(area: Rect) -> Rect {
    let content = inner(area);
    Rect::new(
        content.x + content.width * 2 / 3,
        content.y + 2,
        content.width / 3,
        5.min(content.height.saturating_sub(11)),
    )
}

/// Center choices while reserving the header and footer at supported sizes.
pub(super) fn command_popup_area(area: Rect, _app: &App) -> Rect {
    centered(area, 100, area.height.saturating_sub(4).min(22))
}

pub(super) fn composer_menu_area(area: Rect, app: &App) -> Option<Rect> {
    if app.composer_menu.is_none() || app.overlay.is_some() || app.confirmation.is_some() {
        return None;
    }
    Some(centered(area, 84, area.height.saturating_sub(4).min(12)))
}

pub(super) fn evolution_popup_area(area: Rect, app: &App) -> Option<Rect> {
    if area.width < 60 || area.height < 18 || !app.evolution_popup_ready() {
        return None;
    }
    let layout = shell(area, app.linear, app.effective_focus());
    // At roomy sizes, use the quiet space below the companion rather than
    // clearing any response text or withholding the native portrait.
    if let Some(rail) = layout.rail {
        let top = rail.bottom().saturating_add(1);
        if layout.composer.y.saturating_sub(top) >= 7 {
            return Some(Rect::new(rail.x, top, rail.width, 7));
        }
    }
    // Compact/linear layouts keep the newest response rows and the entire
    // composer visible. This receipt is anchored, never a centered modal.
    Some(Rect::new(
        layout.workspace.x,
        layout.workspace.y,
        layout.workspace.width,
        6.min(layout.workspace.height),
    ))
}

/// Native images draw above cells, so only portraits under a popup are withheld.
pub(super) fn occluded(area: Rect, app: &App, target: Rect) -> bool {
    if app.nickname_editor.is_some()
        || app.cooperation_ui.release.is_some()
        || app.cooperation.cancel_review.is_some()
        || app.cooperation.journal_discard.is_some()
    {
        return true;
    }
    app.command_popup.is_some() && command_popup_area(area, app).intersects(target)
        || composer_menu_area(area, app).is_some_and(|menu| menu.intersects(target))
        || evolution_popup_area(area, app).is_some_and(|popup| popup.intersects(target))
}
