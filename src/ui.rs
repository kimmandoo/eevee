mod agents;
mod execution;
mod handoff;
mod planning;
mod sources;

use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    app::{App, ConfirmationKind, Overlay, PALETTE, Pane, ScrollSurface},
    artwork::Artwork,
    config::{ColorMode, Config, Theme},
    domain::{FORMS, Form},
    store::EditorField,
};

#[cfg(windows)]
pub fn compose_console_graphemes(buffer: &mut ratatui::buffer::Buffer) {
    use unicode_normalization::UnicodeNormalization;
    // The legacy console cell buffer advances separately for decomposed
    // scalars. Compose displayed glyphs, never the editable source text.
    for cell in &mut buffer.content {
        let symbol = cell.symbol();
        if symbol.chars().nth(1).is_some() && !symbol.nfc().eq(symbol.chars()) {
            let composed: String = symbol.nfc().collect();
            cell.set_symbol(&composed);
        }
    }
}

const ASCII_BORDER: border::Set = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

struct Paint {
    base: Style,
    surface: Style,
    muted: Style,
    border: Style,
    accent: Style,
    selected: Style,
}

impl Paint {
    fn new(config: &Config, form: &Form, index: usize) -> Self {
        if config.color == ColorMode::Mono {
            let base = Style::default();
            return Self {
                base,
                surface: base,
                muted: base,
                border: base,
                accent: base.add_modifier(Modifier::BOLD),
                selected: base.add_modifier(Modifier::REVERSED | Modifier::BOLD),
            };
        }
        let light = config.theme == Theme::Light;
        let (canvas, surface, foreground, muted, border) = match (config.color, light) {
            (ColorMode::TrueColor, false) => (
                Color::Rgb(15, 19, 27),
                Color::Rgb(24, 33, 45),
                Color::Rgb(232, 237, 244),
                Color::Rgb(174, 184, 200),
                Color::Rgb(70, 80, 99),
            ),
            (ColorMode::TrueColor, true) => (
                Color::Rgb(249, 250, 252),
                Color::Rgb(240, 243, 248),
                Color::Rgb(23, 35, 55),
                Color::Rgb(82, 96, 121),
                Color::Rgb(154, 165, 181),
            ),
            (_, false) => (
                Color::Indexed(234),
                Color::Indexed(235),
                Color::Indexed(255),
                Color::Indexed(250),
                Color::Indexed(242),
            ),
            (_, true) => (
                Color::Indexed(255),
                Color::Indexed(254),
                Color::Indexed(234),
                Color::Indexed(240),
                Color::Indexed(246),
            ),
        };
        let accent = match config.color {
            ColorMode::TrueColor => {
                let (r, g, b) = form.rgb;
                if light {
                    Color::Rgb(r / 2, g / 2, b / 2)
                } else {
                    Color::Rgb(r, g, b)
                }
            }
            _ if light => Color::Indexed([94, 30, 100, 130, 97, 136, 65, 31, 132][index]),
            _ => Color::Indexed(form.indexed),
        };
        let base = Style::default().fg(foreground).bg(canvas);
        let surface_style = base.bg(surface);
        Self {
            base,
            surface: surface_style,
            muted: surface_style.fg(muted),
            border: surface_style.fg(border),
            accent: surface_style.fg(accent).add_modifier(Modifier::BOLD),
            selected: Style::default()
                .fg(canvas)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
        }
    }
}

pub fn image_background(config: &Config) -> Color {
    Paint::new(config, &FORMS[0], 0)
        .surface
        .bg
        .unwrap_or(Color::Reset)
}

struct Shell {
    header: Rect,
    footer: Rect,
    rail: Option<Rect>,
    workspace: Rect,
    inspector: Option<Rect>,
}

fn shell(area: Rect, linear: bool) -> Shell {
    let roomy = area.height >= 26;
    let outer = area.inner(ratatui::layout::Margin::new(1, u16::from(roomy)));
    let gap = u16::from(roomy);
    let footer_height = if roomy { 3 } else { 2 };
    let header = row(outer, 0, 3);
    let body = row(
        outer,
        3 + gap,
        outer.height.saturating_sub(3 + footer_height + 2 * gap),
    );
    let footer = Rect::new(outer.x, area.bottom().saturating_sub(2), outer.width, 2);
    let (rail, workspace, inspector) = if !linear && area.width >= 120 {
        (
            Some(Rect::new(body.x, body.y, 22, body.height)),
            Rect::new(
                body.x + 23,
                body.y,
                body.width.saturating_sub(54),
                body.height,
            ),
            Some(Rect::new(
                body.right().saturating_sub(30),
                body.y,
                30,
                body.height,
            )),
        )
    } else if !linear && area.width >= 90 {
        (
            Some(Rect::new(body.x, body.y, 22, body.height)),
            Rect::new(
                body.x + 23,
                body.y,
                body.width.saturating_sub(23),
                body.height,
            ),
            None,
        )
    } else {
        (None, body, None)
    };
    Shell {
        header,
        footer,
        rail,
        workspace,
        inspector,
    }
}

fn row(area: Rect, offset: u16, height: u16) -> Rect {
    let offset = offset.min(area.height);
    Rect::new(
        area.x,
        area.y.saturating_add(offset),
        area.width,
        height.min(area.height - offset),
    )
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let available_height = area.height.saturating_sub(2);
    let height = height.min(available_height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (available_height - height) / 2,
        width,
        height,
    )
}

fn drawer(area: Rect) -> Rect {
    let width = 46.min(area.width.saturating_sub(2));
    Rect::new(
        area.right().saturating_sub(width + 1),
        area.y + 1,
        width,
        area.height.saturating_sub(3),
    )
}

fn panel<'a>(title: &'a str, config: &Config, paint: &Paint, focused: bool) -> Block<'a> {
    let block = Block::bordered()
        .title(title)
        .title_style(if focused { paint.accent } else { paint.muted })
        .padding(Padding::horizontal(1))
        .style(paint.surface)
        .border_style(if focused { paint.accent } else { paint.border });
    if config.ascii {
        block.border_set(ASCII_BORDER)
    } else {
        block.border_type(BorderType::Rounded)
    }
}

fn inner(area: Rect) -> Rect {
    area.inner(ratatui::layout::Margin::new(2, 1))
}
fn portrait(area: Rect) -> Rect {
    let content = inner(area);
    let width = 18.min(content.width);
    let height = 9.min(content.height.saturating_sub(8));
    Rect::new(
        content.x + (content.width - width) / 2,
        content.y + 2,
        width,
        height,
    )
}

fn picker(area: Rect, app: &App) -> (Rect, bool) {
    let grid = area.width >= 90 && area.height >= 30 && !app.linear;
    (
        centered(
            area,
            if grid { 108 } else { 84 },
            if grid { 32 } else { 25 },
        ),
        grid,
    )
}

fn grid_cell(area: Rect, index: usize) -> Rect {
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

fn thumbnail(area: Rect) -> Rect {
    let content = inner(area);
    Rect::new(
        content.x,
        content.y + 1,
        content.width,
        content.height.saturating_sub(2),
    )
}

fn compact_picker_portrait(area: Rect) -> Rect {
    let content = inner(area);
    Rect::new(
        content.x + content.width * 2 / 3,
        content.y + 2,
        content.width / 3,
        5.min(content.height.saturating_sub(11)),
    )
}

/// This layout pass runs before drawing. Only visible images are prepared, never
/// underlying native graphics behind an overlay.
pub fn portrait_targets(app: &App, area: Rect, targets: &mut Vec<(usize, Rect)>) {
    targets.clear();
    if area.width < 60 || area.height < 18 || app.editing || app.confirmation.is_some() {
        return;
    }
    match app.overlay {
        Some(Overlay::Forms) => {
            let (picker, grid) = picker(area, app);
            if grid {
                targets
                    .extend((0..9).map(|index| (index, thumbnail(grid_cell(inner(picker), index)))))
            } else {
                targets.push((app.selected.min(8), compact_picker_portrait(picker)));
            }
        }
        Some(Overlay::Companion) => targets.push((app.form, portrait(drawer(area)))),
        Some(_) => {}
        None => {
            if let Some(rail) = shell(area, app.linear).rail {
                targets.push((app.form, portrait(rail)));
            }
        }
    }
}

pub fn render(
    frame: &mut Frame,
    app: &mut App,
    config: &Config,
    artwork: &mut Artwork,
    graphics_status: &str,
) {
    let area = frame.area();
    let form = &FORMS[app.form.min(8)];
    let paint = Paint::new(config, form, app.form.min(8));
    frame.render_widget(Block::default().style(paint.base), area);
    if area.width < 60 || area.height < 18 {
        app.agents.affirmative = false;
        app.agents.reviewed = false;
        text(
            frame,
            area,
            "Terminal too small\nNeed 60 columns x 18 rows\nResize keeps your draft. Ctrl+C opens safe exit.",
            paint.accent,
        );
        if app.confirmation.is_some() {
            confirmation(frame, area, app, config, &paint);
        } else if app.overlay == Some(Overlay::Execution) && app.execution.confirmation.is_some() {
            execution::render_small_confirmation(frame, area, &mut app.execution, config);
        } else if app.overlay == Some(Overlay::Planning) {
            planning::render_small(frame, area, &mut app.planning, &paint);
        } else if app.overlay == Some(Overlay::Handoff) {
            handoff::render_small(frame, area, &mut app.handoff, &paint);
        }
        return;
    }
    let shell = shell(area, app.linear);
    header(frame, shell.header, app, form, &paint);
    if let Some(rail) = shell.rail {
        companion(
            frame,
            rail,
            app,
            config,
            &paint,
            artwork,
            app.overlay.is_none() && !app.editing,
        );
    }
    workspace(frame, shell.workspace, app, form, config, &paint);
    if let Some(inspector) = shell.inspector {
        inspect(
            frame,
            inspector,
            app,
            config,
            &paint,
            artwork,
            graphics_status,
        );
    }
    footer(frame, shell.footer, app, &paint);
    if let Some(overlay) = app.overlay {
        match overlay {
            Overlay::Forms => forms(frame, area, app, config, &paint, artwork),
            Overlay::Palette => palette(frame, area, app, config, &paint),
            Overlay::Help => help(frame, area, app, config, &paint),
            Overlay::Preview => preview(frame, area, app, config, &paint),
            Overlay::Tasks => tasks(frame, area, app, config, &paint),
            Overlay::Criteria => criteria(frame, area, app, config, &paint),
            Overlay::Repository => repository(frame, area, app, config, &paint),
            Overlay::Sources => sources::render(frame, area, app, config, &paint),
            Overlay::Execution => execution::render(frame, area, &mut app.execution, config),
            Overlay::Agents => agents::render(frame, area, app, config, &paint),
            Overlay::Planning => planning::render(frame, area, app, config, &paint),
            Overlay::Handoff => handoff::render(frame, area, app, config, &paint),
            Overlay::Inspector => {
                let area = drawer(area);
                frame.render_widget(Clear, area);
                inspect(frame, area, app, config, &paint, artwork, graphics_status);
            }
            Overlay::Companion => {
                let area = drawer(area);
                frame.render_widget(Clear, area);
                companion(frame, area, app, config, &paint, artwork, true);
            }
        }
    } else if app.editing {
        editor(frame, area, app, config, &paint);
    }
    if app.confirmation.is_some() {
        confirmation(frame, area, app, config, &paint);
    }
}

fn text(frame: &mut Frame, area: Rect, value: &str, style: Style) {
    frame.render_widget(
        Paragraph::new(value)
            .style(style)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn header(frame: &mut Frame, area: Rect, app: &App, form: &Form, paint: &Paint) {
    let title = if app.goal.is_empty() {
        "Your next task starts here"
    } else {
        app.goal.lines().next().unwrap_or("")
    };
    let title = clipped(title, area.width.saturating_sub(1) as usize);
    text(
        frame,
        row(area, 0, 1),
        &title,
        paint.base.add_modifier(Modifier::BOLD),
    );
    let state = if app.linear || app.width < 90 {
        format!("{} / {}", form.name, app.storage_label())
    } else {
        app.storage_label().to_owned()
    };
    text(frame, row(area, 1, 1), &state, paint.accent);
    text(
        frame,
        row(area, 2, 1),
        if app.planning.cancelling {
            "Provider cancellation pending / e status"
        } else if app.planning.preview.is_some() {
            "Local provider preview / no run consent yet"
        } else if app.planning.busy() {
            "Specialist request or journal save active / e status"
        } else if app.planning.thread_id.is_some() {
            "Provider session-local ID available / e status"
        } else {
            app.execution.activity_label()
        },
        paint.base,
    );
}

fn workspace(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    form: &Form,
    config: &Config,
    paint: &Paint,
) {
    let block = panel(
        if app.linear {
            " Task / linear "
        } else {
            form.workspace
        },
        config,
        paint,
        app.effective_focus() == Pane::Workspace,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(frame, row(content, 0, 1), "LOCAL TASK", paint.muted);
    text(
        frame,
        row(content, 1, 2),
        if app.task_id == 0 && app.goal.is_empty() {
            "g / Enter  Create task"
        } else {
            "g / Enter  Resume/edit task"
        },
        paint.accent,
    );
    let artifact = row(content, 3, content.height.saturating_sub(3));
    let offset = app.scroll_for(ScrollSurface::Workspace);
    let met = app
        .criteria
        .iter()
        .filter(|criterion| criterion.met)
        .count();
    let mut lines = vec![
        Line::styled(
            format!(
                "{} / {met}/{} criteria met / {}",
                if app.complete { "Complete" } else { "Open" },
                app.criteria.len(),
                form.name
            ),
            paint.accent,
        ),
        Line::from(""),
        Line::styled("GOAL", paint.muted),
    ];
    if app.goal.is_empty() {
        lines.push(Line::from(
            "No goal applied. Give this task a bounded goal.",
        ));
    } else {
        lines.extend(app.goal.lines().map(Line::from));
    }
    lines.push(Line::from(""));
    lines.push(Line::styled("NEXT ACTION", paint.muted));
    if app.next_action.is_empty() {
        lines.push(Line::from(
            "Not set. Press a to write the next local action.",
        ));
    } else {
        lines.extend(app.next_action.lines().map(Line::from));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(if app.criteria.is_empty() {
        "No acceptance criteria. Press c, then e to add a checklist."
    } else {
        "c Review checklist / d Review task completion"
    }));
    lines.push(Line::from(format!(
        "{} retained draft(s). Completion is your review, not verified execution.",
        app.drafts.len()
    )));
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "EXPECTED ARTIFACT / FORM GUIDANCE",
        paint.muted,
    ));
    lines.push(Line::from(form.evidence));
    lines.push(Line::styled(
        "e reviews specialist requests; j local checks; z portable handoff.",
        paint.muted,
    ));
    let paragraph = Paragraph::new(lines)
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    let max = paragraph
        .line_count(artifact.width.max(1))
        .saturating_sub(artifact.height as usize)
        .min(u16::MAX as usize) as u16;
    frame.render_widget(paragraph.scroll((offset.min(max), 0)), artifact);
    // Consume the borrowed goal before reporting view-only wrapping bounds.
    app.set_scroll_max(ScrollSurface::Workspace, max);
}

fn companion(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
    artwork: &mut Artwork,
    show_image: bool,
) {
    let block = panel(
        " Companion ",
        config,
        paint,
        app.effective_focus() == Pane::Companion || app.overlay == Some(Overlay::Companion),
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let form = &FORMS[app.form];
    frame.render_widget(
        Paragraph::new(form.name)
            .style(paint.accent)
            .alignment(Alignment::Center),
        row(content, 0, 1),
    );
    let image = portrait(area);
    if show_image {
        artwork.render(app.form, image, frame.buffer_mut());
    }
    let details = row(
        content,
        2 + image.height,
        content.height.saturating_sub(2 + image.height),
    );
    let lines = vec![
        Line::styled(form.element, paint.muted),
        Line::from(""),
        Line::from(form.role),
        Line::from(""),
        Line::styled(artwork.status(app.form), paint.muted),
        Line::from(""),
        Line::styled("f Choose evolution", paint.accent),
    ];
    scroll_text(
        frame,
        details,
        Paragraph::new(lines)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app,
        ScrollSurface::Companion,
    );
}

fn inspect(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
    artwork: &Artwork,
    graphics_status: &str,
) {
    let drawer = app.overlay == Some(Overlay::Inspector);
    let block = panel(
        " Task and storage inspector ",
        config,
        paint,
        drawer || app.effective_focus() == Pane::Inspector,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let project = std::path::Path::new(&app.cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&app.cwd);
    let task = if app.task_id > 0 {
        format!("#{}", app.task_id)
    } else {
        "New / not committed".to_owned()
    };
    let value = if drawer {
        let path = config.assets_dir.to_string_lossy();
        let safe_path: String = path.chars().filter(|c| !c.is_control()).collect();
        let updated = if app.updated_at > 0 {
            format!("{} Unix ms", app.updated_at)
        } else {
            "Not committed yet".to_owned()
        };
        let quota = app.policy.quota_bytes;
        let sampled = app.usage_bytes;
        let retention = app.policy.retention_days;
        format!(
            "TASK\n{task} / {}\nUpdated: {updated}\nCriteria: {}/{} met\nRetained drafts: {}\nComplete means user-reviewed criteria, not verified command evidence.\n\nPROJECT DIRECTORY\n{}\n\nSTORAGE\n{}\nRevision {} / committed {}\nDirectory: {}\n{}\nRecovery: r retries storage; --export-data FILE exports readable local records, not provider history.\nQuota: {quota} bytes. Last sample: {sampled} live SQLite, retained output and committed reservations; not physical database size.\nRetention: archived at least {retention} days; explicit purge only, no automatic eviction.\nSecret detection is best-effort. Volatile editor text may remain unmasked.\nArchive is reversible. Deletion affects selected local records and owned artifacts, not repository files, exports or provider sessions. Not forensic secure erasure.\n\nEXECUTION\nSpecialist request and provider-origin journal: e. Local command approval/evidence: j. Portable handoff: z. Cancellation does not roll back files; approval is not a sandbox.\n\nARTWORK\n{}\n{}\n{}\n\nUp/Down scroll / Esc back",
            if app.complete { "Complete" } else { "Open" },
            app.criteria
                .iter()
                .filter(|criterion| criterion.met)
                .count(),
            app.criteria.len(),
            app.drafts.len(),
            app.cwd,
            app.storage_label(),
            app.revision,
            app.saved_revision,
            config.data_dir.to_string_lossy().escape_debug(),
            match &app.storage {
                crate::app::StorageState::Failed(error) => error.as_str(),
                _ => "",
            }
            .escape_debug(),
            artwork.status(app.form),
            graphics_status,
            safe_path
        )
    } else {
        format!(
            "PROJECT\n{project}\n\nTASK\n{task} / {}\nCriteria: {}/{} met\nRetained drafts: {}\nComplete is user review, not verified execution.\n\nSTORAGE\n{}\n{}\n\nCONTROL\nSpecialist requests/journal: e. Local checks: j. Portable handoff: z. Exact control coverage is adapter/action-specific; approval is not a sandbox.\n\ni Details: saved timestamp, revisions, paths and recovery\nUp/Down scroll",
            if app.complete { "Complete" } else { "Open" },
            app.criteria
                .iter()
                .filter(|criterion| criterion.met)
                .count(),
            app.criteria.len(),
            app.drafts.len(),
            app.storage_label(),
            match &app.storage {
                crate::app::StorageState::Failed(error) => error.as_str(),
                _ => "",
            }
            .escape_debug(),
        )
    };
    scroll_text(
        frame,
        content,
        Paragraph::new(value)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app,
        ScrollSurface::Inspector,
    );
}

fn forms(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    config: &Config,
    paint: &Paint,
    artwork: &mut Artwork,
) {
    let (area, grid) = picker(area, app);
    frame.render_widget(Clear, area);
    let block = panel(" Choose evolution / preview only ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        row(content, 0, 1),
        &format!(
            "Current: {} / Enter selects; no work starts",
            FORMS[app.form].name
        ),
        paint.muted,
    );
    let selected = app.selected.min(8);
    if grid {
        for (index, form) in FORMS.iter().enumerate() {
            let cell = grid_cell(content, index);
            let cell_paint = Paint::new(config, form, index);
            let block = panel(form.name, config, &cell_paint, index == selected);
            let inside = block.inner(cell);
            frame.render_widget(block, cell);
            text(
                frame,
                row(inside, 0, 1),
                &format!(
                    "{} {}{}",
                    if index == selected { ">" } else { " " },
                    index + 1,
                    if index == app.form { " current" } else { "" }
                ),
                if index == selected {
                    cell_paint.selected
                } else {
                    cell_paint.muted
                },
            );
            artwork.render(index, thumbnail(cell), frame.buffer_mut());
        }
        let x = content.x + content.width * 3 / 5 + 1;
        let detail = Rect::new(
            x,
            content.y + 2,
            content.right().saturating_sub(x),
            content.height.saturating_sub(2),
        );
        form_details(frame, detail, selected, config, artwork);
    } else {
        let list_width = content.width * 2 / 3;
        let visible = if content.height < 20 { 5 } else { 9 };
        let start = selected.saturating_sub(visible / 2).min(9 - visible);
        for (row_index, index) in (start..start + visible).enumerate() {
            let form = &FORMS[index];
            let item = Rect::new(content.x, content.y + 2 + row_index as u16, list_width, 1);
            let label = format!(
                "{} {} {:<9}{}",
                if index == selected { ">" } else { " " },
                index + 1,
                form.name,
                if index == app.form { " *" } else { "" }
            );
            text(
                frame,
                item,
                &label,
                if index == selected {
                    paint.selected
                } else {
                    paint.surface
                },
            );
        }
        artwork.render(selected, compact_picker_portrait(area), frame.buffer_mut());
        let info = row(
            content,
            3 + visible as u16,
            content.height.saturating_sub(3 + visible as u16),
        );
        let value = format!(
            "{} / {}\nLocal selection grants nothing\np Inspect role / v Sources / j Commands",
            FORMS[selected].name, FORMS[selected].role
        );
        text(frame, info, &value, paint.muted);
    }
}

fn form_details(frame: &mut Frame, area: Rect, index: usize, config: &Config, artwork: &Artwork) {
    let form = &FORMS[index];
    let paint = Paint::new(config, form, index);
    let value = format!(
        "{} / {}\n{}\n\n{}\n\nLocal selection: available\n{}\n{}\n\nArrows / 1-9 preview\nEnter select / Esc keep current",
        form.name,
        form.element,
        form.role,
        form.evidence,
        match index {
            0 => "Task cards and resumption: available",
            1 => "Source ranges/context basket: v",
            2 => "Approved local commands/evidence: j",
            3 => "Reviewed selected-file implementation: e / f",
            4 => "Reviewed multi-agent planning: e",
            _ => "Provider specialist workflow: unavailable",
        },
        artwork.status(index)
    );
    frame.render_widget(
        Paragraph::new(value)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn repository(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = centered(area, 106, area.height.saturating_sub(2));
    frame.render_widget(Clear, area);
    let block = panel(" Repository / read-only evidence ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(frame, row(content, 0, 1), &app.repo_status, paint.accent);
    let details = row(content, 1, content.height.saturating_sub(1));
    let value = if app.repo_text.is_empty() {
        if app.repo_loading {
            "Reading repository on a bounded worker; task editing remains available after Esc."
        } else {
            "No repository evidence available. A non-Git directory still supports durable tasks."
        }
    } else {
        &app.repo_text
    };
    let lines: Vec<_> = app
        .repo_error
        .lines()
        .map(|line| Line::styled(line, paint.muted))
        .chain(value.lines().map(Line::from))
        .collect();
    let max = selection_details(
        frame,
        details,
        Paragraph::new(lines)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app.scroll_for(ScrollSurface::Preview),
    );
    app.set_scroll_max(ScrollSurface::Preview, max);
}

fn tasks(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = centered(area, 88, 28);
    frame.render_widget(Clear, area);
    let block = panel(
        if app.show_archived {
            " Archived task cards "
        } else {
            " Task cards / committed snapshots "
        },
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let count = app.visible_tasks().count();
    if count == 0 {
        text(
            frame,
            content,
            "No task cards in this view.\n\nh Active/archived / n New / Esc Back\nArchive is reversible; deletion requires review.",
            paint.surface,
        );
        app.set_scroll_max(ScrollSurface::Preview, 0);
        return;
    }
    let selected = app.task_selection.min(count - 1);
    let visible = (content.height / 3).clamp(1, 6) as usize;
    let start = selected.saturating_sub(visible - 1);
    for (index, task) in app.visible_tasks().enumerate().skip(start).take(visible) {
        let title = task
            .goal
            .lines()
            .next()
            .filter(|title| !title.is_empty())
            .unwrap_or("Untitled task");
        let label = format!(
            "{}{} #{} {} / {} / {}",
            if index == selected { ">" } else { " " },
            if task.id == app.task_id { "*" } else { " " },
            task.id,
            if task.complete { "Complete" } else { "Open" },
            FORMS[task.form.min(FORMS.len() - 1)].name,
            title
        );
        text(
            frame,
            row(content, (index - start) as u16, 1),
            &clipped(&label, content.width as usize),
            if index == selected {
                paint.selected
            } else {
                paint.surface
            },
        );
    }
    text(
        frame,
        row(content, visible as u16, 1),
        &format!(
            "{} cards / h View / {} / Del Delete / x Retention",
            count,
            if app.show_archived {
                "r Restore"
            } else {
                "a Archive"
            }
        ),
        paint.muted,
    );
    let details = row(
        content,
        visible as u16 + 2,
        content.height.saturating_sub(visible as u16 + 2),
    );
    let task = app
        .visible_tasks()
        .nth(selected)
        .expect("selected visible task");
    let mut lines = vec![
        Line::styled(
            format!(
                "#{} / {} / {}",
                task.id,
                if task.complete { "Complete" } else { "Open" },
                FORMS[task.form.min(FORMS.len() - 1)].name,
            ),
            paint.accent,
        ),
        Line::from(if task.updated_at > 0 {
            format!("Updated: {} Unix ms", task.updated_at)
        } else {
            "Updated: unavailable".to_owned()
        }),
        Line::from(format!(
            "Criteria: {}/{} met / Retained drafts: {}",
            task.criteria
                .iter()
                .filter(|criterion| criterion.met)
                .count(),
            task.criteria.len(),
            task.drafts.len(),
        )),
        Line::from(format!(
            "Archive: {} / Masked secrets: {}",
            task.archived, task.redacted
        )),
        Line::styled("GOAL", paint.muted),
    ];
    if task.goal.is_empty() {
        lines.push(Line::from("No goal applied."));
    } else {
        lines.extend(task.goal.lines().map(Line::from));
    }
    lines.push(Line::styled("NEXT ACTION", paint.muted));
    if task.next_action.is_empty() {
        lines.push(Line::from("Not set."));
    } else {
        lines.extend(task.next_action.lines().map(Line::from));
    }
    lines.push(Line::styled("ACCEPTANCE CRITERIA", paint.muted));
    if task.criteria.is_empty() {
        lines.push(Line::from("No criteria applied."));
    } else {
        lines.extend(task.criteria.iter().map(|criterion| {
            Line::from(format!(
                "[{}] {}",
                if criterion.met { "x" } else { " " },
                criterion.text,
            ))
        }));
    }
    lines.push(Line::from(
        "Complete is user-reviewed, not verified execution.",
    ));
    let max = selection_details(
        frame,
        details,
        Paragraph::new(lines)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app.scroll_for(ScrollSurface::Preview),
    );
    app.set_scroll_max(ScrollSurface::Preview, max);
}

fn criteria(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = centered(area, 88, 26);
    frame.render_widget(Clear, area);
    let block = panel(" Acceptance checklist / your review ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        row(content, 0, 2),
        &format!(
            "{} / {}/{} met. Enter does not complete or execute.",
            if app.complete { "Complete" } else { "Open" },
            app.criteria
                .iter()
                .filter(|criterion| criterion.met)
                .count(),
            app.criteria.len(),
        ),
        paint.muted,
    );
    if app.criteria.is_empty() {
        text(
            frame,
            row(content, 3, content.height.saturating_sub(3)),
            "No acceptance criteria yet.\n\ne Edit criteria, one per line.\nComplete requires a nonempty, all-met checklist.",
            paint.surface,
        );
        app.set_scroll_max(ScrollSurface::Preview, 0);
        return;
    }
    let selected = app.criterion_selection.min(app.criteria.len() - 1);
    let visible = (content.height / 3).clamp(1, 6) as usize;
    let start = selected.saturating_sub(visible - 1);
    for (index, criterion) in app.criteria.iter().enumerate().skip(start).take(visible) {
        let label = format!(
            "{} [{}] {}. {}",
            if index == selected { ">" } else { " " },
            if criterion.met { "x" } else { " " },
            index + 1,
            criterion.text,
        );
        text(
            frame,
            row(content, (index - start) as u16 + 2, 1),
            &clipped(&label, content.width as usize),
            if index == selected {
                paint.selected
            } else {
                paint.surface
            },
        );
    }
    text(
        frame,
        row(content, visible as u16 + 2, 1),
        "Selected criterion / PgUp/Dn details",
        paint.muted,
    );
    let criterion = &app.criteria[selected];
    let lines = vec![
        Line::from(format!(
            "{}. [{}] {}",
            selected + 1,
            if criterion.met { "x" } else { " " },
            criterion.text,
        )),
        Line::from(""),
        Line::from(if app.complete {
            "Completed task. Editing criteria requires explicit Reopen."
        } else {
            "Space toggles your review. e edits criteria; d reviews Complete."
        }),
        Line::from("No command evidence is collected or verified here."),
    ];
    let details = row(
        content,
        visible as u16 + 3,
        content.height.saturating_sub(visible as u16 + 3),
    );
    let max = selection_details(
        frame,
        details,
        Paragraph::new(lines)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app.scroll_for(ScrollSurface::Preview),
    );
    app.set_scroll_max(ScrollSurface::Preview, max);
}

fn selection_details(frame: &mut Frame, area: Rect, paragraph: Paragraph<'_>, offset: u16) -> u16 {
    let max = paragraph
        .line_count(area.width.max(1))
        .saturating_sub(area.height as usize)
        .min(u16::MAX as usize) as u16;
    frame.render_widget(paragraph.scroll((offset.min(max), 0)), area);
    max
}

fn palette(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let area = centered(area, 66, 19);
    frame.render_widget(Clear, area);
    let block = panel(" Local command palette ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let visible = content.height.saturating_sub(2) as usize;
    let start = app.selected.saturating_sub(visible.saturating_sub(1));
    for (index, command) in PALETTE.iter().enumerate().skip(start).take(visible) {
        text(
            frame,
            row(content, (index - start) as u16, 1),
            &format!(
                "{} {}",
                if index == app.selected { ">" } else { " " },
                command
            ),
            if index == app.selected {
                paint.selected
            } else {
                paint.surface
            },
        );
    }
    text(
        frame,
        row(content, content.height.saturating_sub(2), 2),
        "Actions open separate reviews; no automatic provider request.",
        paint.muted,
    );
}

fn help(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = centered(area, 78, 24);
    frame.render_widget(Clear, area);
    let block = panel(" Keyboard / local actions ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let value = "TASKS\n g / Enter  Edit goal; Enter in workspace resumes\n t          Task cards; Enter resumes selection\n n          New task after saves and owned work settle\n c          Criteria; Space toggles, e edits lines\n a          Next action (descriptive, never execution)\n d          Review Complete; requires all criteria met\n w          Repository baseline; r refreshes current evidence\nExternal changes are unattributed, not agent-authored.\nTask cards: h active/archived; a archive; r restore.\nDelete reviews a task; x reviews eligible retention purge.\nDeletion: PgUp/Dn scroll; n/p artifact pages; neutral Stay.\nPurge leaves repository files, exports and provider sessions alone. Not forensic secure erasure.\nComplete is user review, not verified execution.\nGoal/criteria changes require explicit Reopen.\n\nSOURCES / v\n / filters filenames; r chooses inclusive start:end.\n Enter inspects line 1; p pins an inspected range.\n b toggles files/basket; Delete removes; x purges capture.\n a previews exact escaped envelope; d revalidates without sending.\n u refreshes repository; n/p pages a large envelope.\n s retries a failed retained storage operation explicitly.\nIgnore/exclusion, containment and source identity are rechecked. Provider history/config/independent reads are not constrained.\n\nCOMMANDS / j\n e edits JSON argv or explicit shell object; Apply prepares only.\n v opens neutral approval; closing it grants nothing.\n c reviews cancellation; h history; r fresh evidence.\n / searches output; n/N next/previous; f follows.\n Arrows/PgUp/Dn/Home/End scroll without following.\n [/] pages retained output; p reviews purge; d error details.\n s retries journal persistence, never command execution.\nApproval is not a sandbox. Cancellation does not roll back edits.\nTask/form changes and quit wait for owned work or explicit unknown-outcome review. Restart never replays a command.\n\nEDITOR\nEnter applies locally; Ctrl+S retains draft without applying.\nEsc reviews changed draft; unchanged drafts close directly.\nLeft/Right and deletion operate on graphemes.\nUp/Down move lines; Home/End line boundaries.\nCtrl+N newline; Shift+Enter only if terminal distinguishes it.\nPaste never applies or approves. Drafts survive restart.\nSecret masking is best-effort; volatile text may be unmasked.\n\nNAVIGATION\n f Evolution: arrows/1-9 preview, Enter activates\n p Local handoff preview; sends nothing\n i / b Inspector / companion\n r Retry failed task storage outside editor\n l Linear/sanctuary; Ctrl+K actions; Tab pane focus\n Esc closes overlay and restores focus; ? help\n q / Ctrl+C safe exit with neutral Stay\nDuring save-and-exit, Esc cancels exit.\nFailed persistence requires informed explicit exit.\nArtwork: --images cells|off. No automatic download or model request. Linear mode makes no screen-reader compatibility claim.";
    let value = format!(
        "SPECIALIST REQUESTS / e\n g Goal; n New; r explicit same-agent/task Resume.\n 1-4 previews idle agent switch and portable context; grants reset.\n f toggles Plan/Implement locally; x selects JSON edit paths.\n Run always reviews executable/config effects and exact payload.\n PgUp/Dn or End reviews; Tab explicitly selects one-time consent.\n Codex selected-file dynamic tool writes only reviewed existing files.\n Claude edits are explicitly labelled SDK/external-policy controlled.\n ACP selected-file implementation is refused without confinement.\n Permissions default to Deny and preserve exact once-only options.\n Questions: arrows option; a literal answer; s reviews before Send.\n Esc/c cancel and wait; q/Ctrl+C neutral safe exit, not rollback.\n d reviews a complete selected decision in NextAction before saving.\n h retained provider-origin journal; p neutral purge; r storage retry.\n z manual JSON/Markdown handoff, imported claims stay unverified.\n Provider-added context/history/auth/billing remain provider-owned.\n No private reasoning, automatic replay or portable sessions/grants.\n\nAGENT PREFLIGHT / o\n1-4 selects profiles without executing; Enter reviews a version probe.\nEnd/PgDn review; Tab explicitly Trust; Enter confirms once.\nInstalled is not authenticated, negotiated, supported or sandboxed.\nSDK/ACP credentials/capabilities require separate e qualification.\n\n{value}"
    );
    scroll_text(
        frame,
        content,
        Paragraph::new(value)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app,
        ScrollSurface::Help,
    );
}

fn preview(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = centered(area, 82, 24);
    frame.render_widget(Clear, area);
    let block = panel(
        " Local handoff preview / nothing is sent ",
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let form = &FORMS[app.preview_form()];
    let value = format!(
        "GOAL\n{}\n\nROLE\n{} / {}\n\nEXPECTED ARTIFACT\n{}\n\nCONTEXT\n{} captured references loaded. Open v for exact envelope preview and source revalidation. Provider-added history or reads are not governed by this basket.\n\nEXECUTION / CONTROL\nThis local preview sends nothing. e opens separately reviewed provider planning. Command approval is separate in j.\nThis preview performs no operation and sends nothing.\n\nEsc returns to the invoking control.",
        if app.goal.is_empty() {
            "No task goal set"
        } else {
            &app.goal
        },
        form.name,
        form.role,
        form.evidence,
        app.sources.basket.len()
    );
    scroll_text(
        frame,
        content,
        Paragraph::new(value)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app,
        ScrollSurface::Preview,
    );
}

fn footer(frame: &mut Frame, area: Rect, app: &App, paint: &Paint) {
    let value = if app.quit_after_save {
        "Saving task and drafts before exit / Esc Cancel exit"
    } else if app.confirmation.is_some() {
        "Tab/arrows Choose / Enter Confirm / Esc Stay"
    } else if app.transitioning {
        "Applying task operation / mutation paused"
    } else if app.editing {
        "Enter Apply / Ctrl+S Draft / Esc Review / Ctrl+N Newline"
    } else {
        match app.overlay {
            Some(Overlay::Forms) => {
                "Arrows Preview / 1-9 Preview / Enter Select / p Details / Esc Back"
            }
            Some(Overlay::Palette) => "Arrows Choose / Enter Open / Esc Back",
            Some(Overlay::Repository) => "r Refresh / Up/Down Scroll / PgUp/Dn Page / Esc Back",
            Some(Overlay::Sources) => {
                if app.sources.editor.is_some() {
                    "Enter Apply / Esc Back"
                } else if app.sources.confirmation.is_some() {
                    "Tab/arrows Choose / Enter Confirm / Esc Stay / PgUp/Dn"
                } else {
                    match app.sources.mode {
                        crate::app::sources::Mode::Files => {
                            "/ Filter / r Range / Enter Inspect / b Basket / Esc Back"
                        }
                        crate::app::sources::Mode::Basket => {
                            "a Preview / d Revalidate / Del Remove / x Purge / b Files"
                        }
                        crate::app::sources::Mode::Preview => {
                            "p Pin / r Range / PgUp/Dn Scroll / Esc Files"
                        }
                        crate::app::sources::Mode::Payload => {
                            "n/p Pages / d Revalidate / PgUp/Dn Scroll / Esc Basket"
                        }
                    }
                }
            }
            Some(Overlay::Execution) => "",
            Some(Overlay::Agents) => "",
            Some(Overlay::Planning) => "",
            Some(Overlay::Handoff) => "",
            Some(Overlay::Tasks) => {
                "Enter Resume / h Archives / a Archive or r Restore / Del / Esc"
            }
            Some(Overlay::Criteria) => "Space Toggle / e Edit / Arrows Choose / PgUp/Dn / Esc Back",
            Some(_) => "Up/Down Scroll / Esc Back / Ctrl+C Exit",
            None => match app.effective_focus() {
                Pane::Companion => "Enter Evolution / Tab Focus / i Inspect / ? Help / q Quit",
                Pane::Inspector => "Enter Inspect / Tab Focus / t Tasks / ? Help / q Quit",
                Pane::Workspace => "g Edit / t Tasks / c Criteria / Ctrl+K Actions / ? Help",
            },
        }
    };
    if !app.notice.is_empty() {
        text(
            frame,
            row(area, 0, 1),
            &clipped(&app.notice, area.width as usize),
            paint.accent,
        );
        text(frame, row(area, 1, 1), value, paint.base);
    } else if app.scroll() > 0 && !app.editing {
        text(
            frame,
            area,
            &format!("{value}\nDetail line {}", app.scroll() + 1),
            paint.base,
        );
    } else {
        text(frame, area, value, paint.base);
    }
}

/// Wrap graphemes once, retaining the exact insertion boundary even when a wide
/// glyph wraps. The caret owns a cell; at end-of-line it moves onto a fresh row.
fn editor_rows(input: &str, cursor: usize, width: usize) -> (Vec<String>, (usize, usize)) {
    let width = width.max(1);
    let mut lines = vec![String::new()];
    let mut column = 0;
    let mut caret = (0, 0);
    for (index, grapheme) in input.grapheme_indices(true) {
        let size = Span::raw(grapheme).width();
        if grapheme != "\n" && column + size > width {
            lines.push(String::new());
            column = 0;
        }
        if index == cursor {
            caret = (lines.len() - 1, column);
        }
        if grapheme == "\n" {
            lines.push(String::new());
            column = 0;
        } else {
            lines.last_mut().expect("one row").push_str(grapheme);
            column += size;
        }
    }
    if column >= width {
        lines.push(String::new());
        column = 0;
    }
    if cursor == input.len() {
        caret = (lines.len() - 1, column);
    }
    (lines, caret)
}

fn editor(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let area = centered(area, 86, 16);
    frame.render_widget(Clear, area);
    let title = format!(" Edit {} ", app.editor_label());
    let block = panel(&title, config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        row(content, 0, 2),
        match app.editor_field {
            EditorField::Criteria => {
                "One criterion per line; 64 max, 256 bytes each. Changed criteria reset to unmet."
            }
            EditorField::Goal => {
                "Goal draft. Apply saves locally; no provider request or execution."
            }
            EditorField::NextAction => {
                "Next-action draft. Describes your plan; does not execute it."
            }
            EditorField::Command => {
                "JSON argv array or explicit shell object. Apply prepares review; never executes."
            }
        },
        paint.muted,
    );
    let input = row(content, 3, content.height.saturating_sub(6));
    let (lines, (caret_row, caret_column)) = editor_rows(
        &app.input,
        app.cursor,
        input.width.saturating_sub(1) as usize,
    );
    let top = caret_row.saturating_sub(input.height.saturating_sub(1) as usize);
    frame.render_widget(
        Paragraph::new(
            lines
                .iter()
                .skip(top)
                .map(|line| Line::from(line.as_str()))
                .collect::<Vec<_>>(),
        )
        .style(paint.surface),
        input,
    );
    if input.width > 0 && input.height > 0 {
        frame.set_cursor_position((
            input.x + caret_column.min(input.width.saturating_sub(1) as usize) as u16,
            input.y + (caret_row - top) as u16,
        ));
    }
    let warning = if app.input_truncated {
        " / LIMIT: paste truncated"
    } else {
        ""
    };
    text(
        frame,
        row(content, content.height.saturating_sub(2), 2),
        &format!(
            "{} / 4096 bytes{}\nEnter Apply / Ctrl+S Save draft/back / Esc Review",
            app.input.len(),
            warning
        ),
        paint.muted,
    );
}

fn scroll_text(
    frame: &mut Frame,
    area: Rect,
    paragraph: Paragraph<'_>,
    app: &mut App,
    surface: ScrollSurface,
) {
    let count = paragraph.line_count(area.width.max(1));
    let max = count
        .saturating_sub(area.height as usize)
        .min(u16::MAX as usize) as u16;
    app.set_scroll_max(surface, max);
    frame.render_widget(paragraph.scroll((app.scroll_for(surface), 0)), area);
}

fn confirmation(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let Some(confirm) = app.confirmation.as_ref() else {
        return;
    };
    if confirm.kind == ConfirmationKind::DeleteTasks {
        delete_confirmation(frame, area, app, config, paint);
        return;
    }
    let area = if area.width < 30 || area.height < 10 {
        area
    } else {
        centered(area, 62, 10)
    };
    frame.render_widget(Clear, area);
    let block = panel(
        match confirm.kind {
            ConfirmationKind::Complete => " Confirm task completion ",
            ConfirmationKind::Reopen(_) => " Reopen completed task ",
            ConfirmationKind::DiscardDraft => " Review draft discard ",
            ConfirmationKind::Quit => " Confirm exit ",
            ConfirmationKind::DeleteTasks => unreachable!("separate deletion preview"),
        },
        config,
        paint,
        true,
    );
    let content = if area.width < 8 || area.height < 5 {
        area
    } else {
        block.inner(area)
    };
    frame.render_widget(block, area);
    let (question, keep, discard) = match confirm.kind {
        ConfirmationKind::DiscardDraft => (
            "Discard this field's draft? Applied task data is kept.",
            "Keep editing",
            "Discard draft",
        ),
        ConfirmationKind::Complete => (
            "Mark Complete based on your checklist review? This is not verified command evidence.",
            "Stay",
            "Complete task",
        ),
        ConfirmationKind::Reopen(_) => (
            "Reopen this completed task before editing its goal or criteria?",
            "Stay",
            "Reopen and edit",
        ),
        ConfirmationKind::Quit if matches!(app.storage, crate::app::StorageState::Failed(_)) => (
            "Storage failed. Unsaved edits may be lost; accepted saves may finish. Stay, then r retries.",
            "Stay",
            "Exit despite save failure",
        ),
        ConfirmationKind::Quit if app.sources.store_failed => (
            "Context storage failed. The retained operation is not saved. Stay, fix storage, then v and s retry.",
            "Stay",
            "Exit despite unsaved context",
        ),
        ConfirmationKind::Quit => (
            "Exit? Save the applied task and retained drafts before closing.",
            "Stay",
            "Save task/drafts and quit",
        ),
        ConfirmationKind::DeleteTasks => unreachable!("separate deletion preview"),
    };
    text(frame, row(content, 0, 3), question, paint.surface);
    text(
        frame,
        row(content, 3, 1),
        keep,
        if !confirm.destructive {
            paint.selected
        } else {
            paint.surface
        },
    );
    text(
        frame,
        row(content, 4, 1),
        discard,
        if confirm.destructive {
            paint.selected
        } else {
            paint.surface
        },
    );
    text(
        frame,
        row(content, 6, 2),
        "Tab/arrows Choose / Enter Confirm / Esc Stay",
        paint.muted,
    );
}

fn delete_confirmation(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
) {
    let area = centered(area, 78, 26);
    frame.render_widget(Clear, area);
    let block = panel(" Review local task deletion ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let question = format!(
        "Delete {} task records/drafts and listed excerpts/runs? Sources, exports and provider sessions stay. Not secure erasure. Page {}/{} (n/p).",
        app.delete_targets.len(),
        app.delete_page + 1,
        app.deletion_pages()
    );
    text(frame, row(content, 0, 4), &question, paint.surface);
    let details = row(content, 4, content.height.saturating_sub(8));
    enum Item<'a> {
        Task(&'a crate::store::Snapshot),
        Excerpt(i64, &'a str),
        Run(&'a str, bool),
        Provider(&'a str, crate::provider::Agent),
    }
    let lines: Vec<_> = app
        .delete_preview
        .iter()
        .flat_map(|target| {
            std::iter::once(Item::Task(&target.task))
                .chain(
                    target
                        .excerpts
                        .iter()
                        .map(|(id, path)| Item::Excerpt(*id, path)),
                )
                .chain(
                    target
                        .runs
                        .iter()
                        .map(|(id, available)| Item::Run(id, *available)),
                )
                .chain(
                    target
                        .provider_records
                        .iter()
                        .map(|(id, agent)| Item::Provider(id, *agent)),
                )
        })
        .skip(app.delete_page * crate::app::DELETE_PREVIEW_PAGE)
        .take(crate::app::DELETE_PREVIEW_PAGE)
        .map(|item| {
            Line::from(match item {
                Item::Task(task) => format!(
                    "#{} {}",
                    task.id,
                    clipped(
                        task.goal.lines().next().unwrap_or("Untitled task"),
                        details.width.saturating_sub(24) as usize
                    )
                ),
                Item::Excerpt(id, path) => format!(
                    "  excerpt #{id} {}",
                    clipped(path, details.width.saturating_sub(32) as usize)
                ),
                Item::Run(id, available) => format!(
                    "  run {id} / output {}",
                    if available { "retained" } else { "unavailable" }
                ),
                Item::Provider(id, agent) => {
                    format!("  provider {} / observation {id}", agent.label())
                }
            })
        })
        .collect();
    let max = selection_details(
        frame,
        details,
        Paragraph::new(lines)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app.scroll_for(ScrollSurface::Preview),
    );
    app.set_scroll_max(ScrollSurface::Preview, max);
    let selected = app
        .confirmation
        .is_some_and(|confirmation| confirmation.destructive);
    text(
        frame,
        row(content, content.height.saturating_sub(4), 1),
        "Stay / keep all data",
        if !selected {
            paint.selected
        } else {
            paint.surface
        },
    );
    text(
        frame,
        row(content, content.height.saturating_sub(3), 1),
        "Delete the listed local tasks",
        if selected {
            paint.selected
        } else {
            paint.surface
        },
    );
    text(
        frame,
        row(content, content.height.saturating_sub(2), 2),
        "PgUp/Dn Review IDs / Tab Choose / Enter Confirm / Esc Stay",
        paint.muted,
    );
}

fn clipped(value: &str, columns: usize) -> Cow<'_, str> {
    if Span::raw(value).width() <= columns {
        return Cow::Borrowed(value);
    }
    let mut result = String::new();
    let mut used = 0;
    for grapheme in value.graphemes(true) {
        let next = Span::raw(grapheme).width();
        if used + next > columns.saturating_sub(1) {
            break;
        }
        result.push_str(grapheme);
        used += next;
    }
    result.push('…');
    Cow::Owned(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ImageMode, LayoutMode};

    fn luminance(color: Color) -> f64 {
        let Color::Rgb(r, g, b) = color else {
            panic!("true-color theme expected")
        };
        let linear = |value: u8| {
            let v = f64::from(value) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    fn contrast(foreground: Color, background: Color) -> f64 {
        let a = luminance(foreground);
        let b = luminance(background);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn all_true_color_forms_keep_readable_text_and_selection() {
        for theme in [Theme::Dark, Theme::Light] {
            let config = Config {
                ascii: false,
                theme,
                color: ColorMode::TrueColor,
                images: ImageMode::Off,
                assets_dir: "unused".into(),
                layout: LayoutMode::Sanctuary,
                layout_explicit: false,
                data_dir: "unused-data".into(),
                storage_policy: crate::store::Policy::default(),
                context_exclusions: Vec::new(),
                capture_capacity: 64 * 1024,
            };
            for (index, form) in FORMS.iter().enumerate() {
                let paint = Paint::new(&config, form, index);
                for style in [
                    paint.base,
                    paint.surface,
                    paint.muted,
                    paint.accent,
                    paint.selected,
                ] {
                    assert!(
                        contrast(style.fg.unwrap(), style.bg.unwrap()) >= 4.5,
                        "{} {theme:?}: {style:?}",
                        form.name
                    );
                }
                assert!(contrast(paint.accent.fg.unwrap(), paint.surface.bg.unwrap()) >= 3.0);
            }
        }
    }

    #[test]
    fn caret_tracks_wide_wrapped_and_combining_graphemes() {
        let input = "ab한e\u{301}\n끝";
        assert_eq!(editor_rows(input, 2, 3).1, (1, 0));
        assert_eq!(editor_rows(input, "ab한".len(), 3).1, (1, 2));
        assert_eq!(editor_rows(input, input.len(), 3).1, (2, 2));
        assert_eq!(editor_rows("한글", 6, 2).1, (2, 0));
    }
}
