mod agents;
mod composer;
mod execution;
mod handoff;
mod memory;
mod planning;
mod sources;
mod workflows;

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
        let (canvas, foreground, muted, border) = match (config.color, light) {
            (ColorMode::TrueColor, false) => (
                Color::Rgb(18, 26, 25),
                Color::Rgb(235, 232, 208),
                Color::Rgb(171, 187, 160),
                Color::Rgb(101, 130, 98),
            ),
            (ColorMode::TrueColor, true) => (
                Color::Rgb(232, 229, 204),
                Color::Rgb(38, 55, 42),
                Color::Rgb(79, 99, 72),
                Color::Rgb(112, 131, 91),
            ),
            (_, false) => (
                Color::Indexed(233),
                Color::Indexed(230),
                Color::Indexed(144),
                Color::Indexed(65),
            ),
            (_, true) => (
                Color::Indexed(187),
                Color::Indexed(22),
                Color::Indexed(65),
                Color::Indexed(101),
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
        let surface_style = base.bg(image_background(config));
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
    // Cell images composite alpha against the same surface painted by panels.
    match (config.color, config.theme) {
        (ColorMode::Mono, _) => Color::Reset,
        (ColorMode::TrueColor, Theme::Dark) => Color::Rgb(27, 39, 34),
        (ColorMode::TrueColor, Theme::Light) => Color::Rgb(250, 247, 226),
        (ColorMode::Indexed, Theme::Dark) => Color::Indexed(234),
        (ColorMode::Indexed, Theme::Light) => Color::Indexed(230),
    }
}

struct Shell {
    header: Rect,
    footer: Rect,
    rail: Option<Rect>,
    workspace: Rect,
    composer: Rect,
    inspector: Option<Rect>,
}

fn shell(area: Rect, linear: bool, focus: Pane) -> Shell {
    let outer = area.inner(ratatui::layout::Margin::new(1, 0));
    let header = row(outer, 0, 2);
    let footer = row(outer, outer.height.saturating_sub(2), 2);
    let composer_height = if area.height >= 26 { 7 } else { 6 };
    let composer = row(
        outer,
        outer.height.saturating_sub(composer_height + 2),
        composer_height,
    );
    let body = row(outer, 3, composer.y.saturating_sub(outer.y + 4));
    // The task owns the screen. Secondary details are disclosed on focus,
    // rather than keeping three full-height dashboard columns on screen.
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

fn companion_portrait(area: Rect) -> Rect {
    Rect::new(
        area.x + 2,
        area.y + 3,
        area.width.saturating_sub(4),
        area.height.saturating_sub(6).min(10),
    )
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
        block.border_type(if focused {
            BorderType::Double
        } else {
            BorderType::Plain
        })
    }
}

fn inner(area: Rect) -> Rect {
    area.inner(ratatui::layout::Margin::new(2, 1))
}
fn portrait(area: Rect) -> Rect {
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
    let modal_editor = app.editing
        && matches!(
            app.editor_field,
            EditorField::Criteria | EditorField::Command
        );
    if area.width < 60
        || area.height < 18
        || modal_editor
        || app.confirmation.is_some()
        || app.composer_menu.is_some()
    {
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
            if let Some(rail) = shell(area, app.linear, app.effective_focus()).rail {
                targets.push((app.form, companion_portrait(rail)));
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
        } else if app.overlay == Some(Overlay::Memory) {
            memory::render_small(frame, area, &mut app.memory, &paint);
        } else if app.overlay == Some(Overlay::Workflows) {
            workflows::render(frame, area, app, config, &paint);
        }
        return;
    }
    let shell = shell(area, app.linear, app.effective_focus());
    header(frame, shell.header, app, form, &paint);
    if let Some(rail) = shell.rail {
        compact_companion(
            frame,
            rail,
            app,
            config,
            &paint,
            artwork,
            app.overlay.is_none()
                && app.composer_menu.is_none()
                && !(app.editing
                    && matches!(
                        app.editor_field,
                        EditorField::Criteria | EditorField::Command
                    )),
        );
    }
    workspace(frame, shell.workspace, app, form, config, &paint);
    composer_input(frame, shell.composer, app, config, &paint);
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
    if app.composer_menu.is_some() && app.overlay.is_none() && app.confirmation.is_none() {
        let height = shell
            .composer
            .y
            .saturating_sub(shell.header.bottom())
            .min(9);
        composer::render(
            frame,
            Rect::new(
                shell.composer.x,
                shell.composer.y.saturating_sub(height),
                shell.composer.width.min(84),
                height,
            ),
            app,
            config,
            &paint,
        );
    }
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
            Overlay::Workflows => workflows::render(frame, area, app, config, &paint),
            Overlay::Memory => memory::render(frame, area, &mut app.memory, config, &paint),
            Overlay::Images => images(frame, area, app, config, &paint, artwork, graphics_status),
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
    } else if app.editing
        && matches!(
            app.editor_field,
            EditorField::Criteria | EditorField::Command
        )
    {
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
    let project = std::path::Path::new(&app.cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&app.cwd);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" EEVEE ", paint.selected),
            Span::styled(format!("  {project}"), paint.base),
            Span::styled(format!("  /  {}", form.name), paint.muted),
        ])),
        row(area, 0, 1),
    );
    let activity = if app.planning.request.is_some() {
        "Provider needs input / e Review"
    } else if app.planning.cancelling {
        "Cancelling provider / e Status"
    } else if app.planning.preview.is_some() {
        "Provider preview / e Review"
    } else if app.planning.busy() {
        "Provider active / e Status"
    } else if app.planning.outcome_unknown() {
        "Provider outcome unknown / e Inspect"
    } else {
        app.execution.activity_label()
    };
    text(
        frame,
        row(area, 1, 1),
        &clipped(
            &format!("{activity}  /  {}", app.storage_label()),
            area.width as usize,
        ),
        paint.muted,
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
        " FIELD LOG ",
        config,
        paint,
        app.effective_focus() == Pane::Workspace && !app.editing && app.overlay.is_none(),
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let area = content;
    let met = app
        .criteria
        .iter()
        .filter(|criterion| criterion.met)
        .count();
    let mut lines = Vec::new();
    if app.goal.is_empty() {
        lines.push(Line::styled("Ready for a new task", paint.accent));
        lines.push(Line::from("Type a goal below; Enter saves locally."));
        lines.push(Line::styled("F2 Connect an agent", paint.muted));
    } else {
        lines.push(Line::styled("SAVED TASK", paint.accent));
        lines.extend(app.goal.lines().map(Line::from));
        lines.push(Line::styled(
            format!(
                "{} / {met}/{} criteria met  /  c Criteria",
                if app.complete {
                    "User-marked complete"
                } else {
                    "Open task"
                },
                app.criteria.len()
            ),
            paint.muted,
        ));
        if !app.next_action.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::styled("NEXT ACTION", paint.accent));
            lines.extend(app.next_action.lines().map(Line::from));
        }
        lines.push(Line::from(""));
        if app.planning.items.is_empty() {
            lines.push(Line::from("e Review a provider request"));
        } else {
            lines.push(Line::styled(
                format!("{} / PROVIDER ACTIVITY", app.planning.agent.label()),
                paint.accent,
            ));
            lines.push(Line::styled(
                "Unverified provider output / e Details",
                paint.muted,
            ));
            // A bounded recent activity view, not a fabricated chat transcript.
            // Full item provenance and decisions remain in the provider drawer.
            for item in app.planning.items.iter().rev().take(3).rev() {
                let kind = match item.kind {
                    crate::provider::ItemKind::Message => "Message",
                    crate::provider::ItemKind::Plan => "Plan",
                    crate::provider::ItemKind::Tool => "Tool output",
                };
                lines.push(Line::from(""));
                lines.push(Line::styled(
                    format!(
                        "{kind} / {}",
                        if item.complete {
                            "complete"
                        } else {
                            "streaming"
                        }
                    ),
                    paint.accent,
                ));
                lines.extend(item.text.lines().take(8).map(Line::from));
                if item.text.lines().nth(8).is_some() {
                    lines.push(Line::styled("... e Full item and provenance", paint.muted));
                }
            }
            lines.push(Line::styled(
                "e All items, provenance and provider controls",
                paint.muted,
            ));
        }
    }
    if !app.drafts.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "Retained draft available / g Resume goal editor",
            paint.accent,
        ));
    }
    if matches!(app.form, 5..=8) {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            if app.form == 8 {
                "Enter Handoff / m Approved memories"
            } else {
                "Enter Specialist desk / u Workflows"
            },
            paint.accent,
        ));
        lines.push(Line::styled(form.evidence, paint.muted));
    }
    let paragraph = Paragraph::new(lines)
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    let max = paragraph
        .line_count(area.width.max(1))
        .saturating_sub(area.height as usize)
        .min(u16::MAX as usize) as u16;
    frame.render_widget(
        paragraph.scroll((app.scroll_for(ScrollSurface::Workspace).min(max), 0)),
        area,
    );
    app.set_scroll_max(ScrollSurface::Workspace, max);
}

fn compact_companion(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    config: &Config,
    paint: &Paint,
    artwork: &mut Artwork,
    show_image: bool,
) {
    let focused = app.effective_focus() == Pane::Companion;
    let block = panel(" PARTNER ", config, paint, focused);
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        row(content, 0, 1),
        FORMS[app.form].name,
        paint.accent,
    );
    text(
        frame,
        row(content, 1, 1),
        FORMS[app.form].element,
        paint.muted,
    );
    let image = companion_portrait(area);
    if show_image {
        artwork.render(app.form, image, frame.buffer_mut());
    }
    let status = if app.criteria.is_empty() {
        Cow::Borrowed("No criteria yet")
    } else {
        let met = app
            .criteria
            .iter()
            .filter(|criterion| criterion.met)
            .count();
        Cow::Owned(format!("Objectives {met}/{}", app.criteria.len()))
    };
    text(
        frame,
        row(content, content.height.saturating_sub(2), 1),
        &status,
        paint.muted,
    );
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        if focused {
            "> Enter Choose form"
        } else {
            "F3 Forms / b Details"
        },
        paint.accent,
    );
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
    frame.render_widget(
        Paragraph::new("F3 Forms / F4 Images")
            .style(paint.accent)
            .alignment(Alignment::Center),
        row(content, 1, 1),
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
        Line::from(form.role),
        Line::styled(artwork.status(app.form), paint.muted),
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
    let block = panel(" Choose evolution / PARTY ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        row(content, 0, 1),
        &format!(
            "Current: {} / selection starts no work",
            FORMS[app.form].name
        ),
        paint.muted,
    );
    text(
        frame,
        row(content, 1, 1),
        &format!(
            "Enter Select / Esc Back / F4 Images ({}/9)",
            artwork.available()
        ),
        paint.accent,
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
            "{} / {}\np Role details",
            FORMS[selected].name, FORMS[selected].role
        );
        text(frame, info, &value, paint.muted);
    }
}

fn form_details(frame: &mut Frame, area: Rect, index: usize, config: &Config, artwork: &Artwork) {
    let form = &FORMS[index];
    let paint = Paint::new(config, form, index);
    let value = format!(
        "{} / {}\n{}\n\n{}\n\n{}\n{}\n\nArrows / 1-9 Preview\np Role details",
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
    let block = panel(" Actions ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let visible = content.height.saturating_sub(1) as usize;
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
        row(content, content.height.saturating_sub(1), 1),
        "Arrows Choose / Enter Open / Esc Back",
        paint.muted,
    );
}

fn help(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = centered(area, 78, 24);
    frame.render_widget(Clear, area);
    let block = panel(" Help / PgUp/Dn Scroll / Esc Back ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let value = "TASKS\n g / Enter  Edit goal; Enter in workspace resumes\n t          Task cards; Enter resumes selection\n n          New task after saves and owned work settle\n c          Criteria; Space toggles, e edits lines\n a          Next action (descriptive, never execution)\n d          Review Complete; requires all criteria met\n w          Repository baseline; r refreshes current evidence\nExternal changes are unattributed, not agent-authored.\nTask cards: h active/archived; a archive; r restore.\nDelete reviews a task; x reviews eligible retention purge.\nDeletion: PgUp/Dn scroll; n/p artifact pages; neutral Stay.\nPurge leaves repository files, exports and provider sessions alone. Not forensic secure erasure.\nComplete is user review, not verified execution.\nGoal/criteria changes require explicit Reopen.\n\nSOURCES / v\n / filters filenames; r chooses inclusive start:end.\n Enter inspects line 1; p pins an inspected range.\n b toggles files/basket; Delete removes; x purges capture.\n a previews exact escaped envelope; d revalidates without sending.\n u refreshes repository; n/p pages a large envelope.\n s retries a failed retained storage operation explicitly.\nIgnore/exclusion, containment and source identity are rechecked. Provider history/config/independent reads are not constrained.\n\nCOMMANDS / j\n e edits JSON argv or explicit shell object; Apply prepares only.\n v opens neutral approval; closing it grants nothing.\n c reviews cancellation; h history; r fresh evidence.\n / searches output; n/N next/previous; f follows.\n Arrows/PgUp/Dn/Home/End scroll without following.\n [/] pages retained output; p reviews purge; d error details.\n s retries journal persistence, never command execution.\nApproval is not a sandbox. Cancellation does not roll back edits.\nTask/form changes and quit wait for owned work or explicit unknown-outcome review. Restart never replays a command.\n\nEDITOR\nEnter applies locally; Ctrl+S retains draft without applying.\nEsc reviews changed draft; unchanged drafts close directly.\nLeft/Right and deletion operate on graphemes.\nUp/Down move lines; Home/End line boundaries.\nCtrl+N newline; Shift+Enter only if terminal distinguishes it.\nPaste never applies or approves. Drafts survive restart.\nSecret masking is best-effort; volatile text may be unmasked.\n\nNAVIGATION\n f Evolution: arrows/1-9 preview, Enter activates\n p Local handoff preview; sends nothing\n i / b Inspector / companion\n r Retry failed task storage outside editor\n l Linear/sanctuary; Ctrl+K actions; Tab pane focus\n Esc closes overlay and restores focus; ? help\n q / Ctrl+C safe exit with neutral Stay\nDuring save-and-exit, Esc cancels exit.\nFailed persistence requires informed explicit exit.\nArtwork: --images cells|off. No automatic download or model request. Linear mode makes no screen-reader compatibility claim.";
    let value = format!(
        "QUICK KEYS\n Enter       Save goal locally; never sends or runs\n @ / /       Files / commands in the composer\n F2          Connections\n F3 / F4     Forms / images\n Ctrl+K      All actions\n Esc         Back; review a changed draft\n Ctrl+C      Safe exit\n PgUp/Dn     Scroll focused details\n\nLAYOUT\n b Companion / i Inspector / Tab Focus\n Narrow layouts keep the composer full-width.\n Global shortcuts retain unfinished drafts.\n Provider output is unverified; e opens full items.\n\nCONNECTIONS / F2\n Arrows select; Enter opens actions or a review.\n Connect requires explicit approval; Back is default.\n Native login owns browser/credentials and may open chat.\n No task is sent and no token is copied into Eevee.\n Installed, authenticated and model eligibility differ.\n Claude: API key, Console OAuth (ant) or cloud; no subscription OAuth.\n Diagnostics keeps executable, capability and blocker details.\n\nIMAGES / F4\n Download missing / Retry repairs the sprite cache.\n Reload local is offline; Cancel keeps completed images.\n Startup and form changes never download images.\n Auto uses Sixel/Kitty or falls back to text, not a mosaic.\n --images cells explicitly enables character art.\n\nMORE KEYS (Esc leaves the input first)\n{value}\n\nSPECIALIST REQUESTS / e\n n New session / r Reviewed same-task resume\n f Plan or Implement / x Select edit paths\n Review payload and effects, then authorize once.\n Permissions start at Deny; cancellation is not rollback.\n d Review selected decision as a next action / h Journal\n Provider history/configuration/billing remain external.\n No automatic replay, private reasoning or portable permissions."
    );
    let value = format!(
        "{value}\n\nSPECIALIST DESK / u\n Left/Right chooses Diagnosis, Refactor, Review or Recipe.\n Up/Down chooses a field or action; Enter opens it.\n Text fields: Ctrl+S applies locally; Enter adds a line.\n Source/run evidence retains task ownership and freshness.\n Refactor and diagnosis handoffs open normal approvals.\n Managed review starts fresh and has no source-writing tools.\n Recipes preview before Apply; Apply stages only, never runs.\n\nAPPROVED MEMORIES / m\n r Remember the saved next-action note; n author a note.\n Scope and contents require explicit preview/approval.\n Space chooses memories for future provider context.\n Editing/deleting invalidates old request snapshots.\n Prior exports and provider history are outside local erasure."
    );
    let value = format!(
        "{value}\n\nCOMPOSER / FILES AND COMMANDS\n @ at a word boundary filters eligible repository files.\n Arrows/Tab chooses; Enter inspects the complete bounded file.\n r chooses a smaller line range; p pins reviewed context.\n Esc returns to the original draft; the title counts attachments.\n New-task drafts save locally before gathering context.\n / at the start of a line filters real actions, e.g. /help.\n /run opens the command editor, never executes a shell string.\n Save prepares the ordinary explicit command review/approval.\n Esc dismisses suggestions and keeps literal input.\n Paste never opens a picker or invokes a command."
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
    } else if app.editing && app.overlay.is_none() {
        "F1 Help / F2 Connect / F3 Forms / F4 Images\nCtrl+K Actions / Ctrl+C Exit"
    } else {
        match app.overlay {
            Some(Overlay::Forms) => "Arrows/1-9 Preview / Enter Select / p Details / Esc Back",
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
            Some(Overlay::Workflows | Overlay::Memory) => "",
            Some(Overlay::Tasks) => "Enter Resume / h Archives / a Archive / r Restore / Del",
            Some(Overlay::Criteria) => "Space Toggle / e Edit / Arrows Choose / PgUp/Dn / Esc Back",
            Some(_) => "Up/Down Scroll / Esc Back / Ctrl+C Exit",
            None => match app.effective_focus() {
                Pane::Companion => "Enter Evolution / Tab Focus / i Inspect / ? Help / q Quit",
                Pane::Inspector => "Enter Inspect / Tab Focus / t Tasks / ? Help / q Quit",
                Pane::Workspace if matches!(app.form, 5..=7) => {
                    "Enter Desk / m Memories / Ctrl+K Actions / F1 Help"
                }
                Pane::Workspace if app.form == 8 => {
                    "Enter Handoff / m Memories / Ctrl+K Actions / F1 Help"
                }
                Pane::Workspace => {
                    "F1 Help / F2 Connect / F3 Forms / F4 Images\nCtrl+K Actions / Ctrl+C Exit"
                }
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

fn images(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
    artwork: &Artwork,
    graphics_status: &str,
) {
    let area = centered(area, 84, 24.min(area.height.saturating_sub(2)));
    frame.render_widget(Clear, area);
    let block = panel(" Images / Eevee and evolutions ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let display_path = format!("{:?}", config.assets_dir);
    let path = crate::privacy::redact(&display_path);
    let body = format!(
        "{}/9 sprites ready / {}\n\nDownload: pinned PokeAPI/sprites on raw.githubusercontent.com (HTTPS)\nSave to: {}\nMatching files are reused; conflicting files are preserved.\nPokemon belongs to its rights holders; download is not a rights license.\n\n{}\n\nStartup stays offline. Auto uses Sixel or Kitty; --images cells enables character art.",
        artwork.available(),
        graphics_status,
        path,
        if config.images == crate::config::ImageMode::Off {
            "Images disabled: restart with --images auto or --images cells."
        } else if app.images_notice.is_empty() {
            "Choose Download missing / Retry, or Reload local."
        } else {
            &app.images_notice
        }
    );
    let details = row(content, 0, content.height.saturating_sub(4));
    let paragraph = Paragraph::new(body)
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    let maximum = paragraph
        .line_count(details.width.max(1))
        .saturating_sub(details.height as usize)
        .min(u16::MAX as usize) as u16;
    let offset = app.scroll_for(ScrollSurface::Preview).min(maximum);
    frame.render_widget(paragraph.scroll((offset, 0)), details);
    app.set_scroll_max(ScrollSurface::Preview, maximum);
    app.images_reviewed = offset >= maximum;
    let choices = [
        "Back",
        if app.images_busy {
            "Cancel download"
        } else {
            "Download missing / Retry"
        },
        "Reload local",
    ];
    for (index, label) in choices.iter().enumerate() {
        text(
            frame,
            row(content, content.height.saturating_sub(4) + index as u16, 1),
            &format!(
                "{} {label}",
                if index == app.images_selected {
                    ">"
                } else {
                    " "
                }
            ),
            if index == app.images_selected {
                paint.accent
            } else {
                paint.muted
            },
        );
    }
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        if maximum > offset {
            "PgDown Review details / Esc Back"
        } else {
            "Arrows/Tab Choose / Enter Select / Esc Back"
        },
        paint.muted,
    );
}

fn composer_input(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let editing = app.editing
        && matches!(
            app.editor_field,
            EditorField::Goal | EditorField::NextAction
        );
    let title = if editing && app.editor_field == EditorField::NextAction {
        " Next action / local "
    } else {
        " Task composer / local "
    };
    let title = format!("{title}/ {} attached ", app.sources.basket.len());
    let block = panel(&title, config, paint, editing && app.overlay.is_none());
    let content = block.inner(area);
    frame.render_widget(block, area);
    let input = row(content, 0, content.height.saturating_sub(2));
    let value = if editing { app.input.as_str() } else { "" };
    let cursor = if editing { app.cursor } else { 0 };
    let (lines, (caret_row, caret_column)) =
        editor_rows(value, cursor, input.width.saturating_sub(1) as usize);
    let top = caret_row.saturating_sub(input.height.saturating_sub(1) as usize);
    if value.is_empty() {
        text(
            frame,
            input,
            if editing {
                "Describe your task..."
            } else if app.goal.is_empty() {
                "Press g to describe a task."
            } else {
                "g Edit goal"
            },
            paint.muted,
        );
    } else {
        frame.render_widget(
            Paragraph::new(
                lines
                    .into_iter()
                    .skip(top)
                    .map(Line::from)
                    .collect::<Vec<_>>(),
            )
            .style(paint.surface),
            input,
        );
    }
    if editing && app.overlay.is_none() && input.width > 0 && input.height > 0 {
        frame.set_cursor_position((
            input.x + caret_column.min(input.width.saturating_sub(1) as usize) as u16,
            input.y + (caret_row - top) as u16,
        ));
    }
    let hint = if app.input_truncated && editing {
        "Input limit reached; draft retained.\nEnter Save locally / Esc Review"
    } else if editing {
        "Enter Save locally / Ctrl+N Newline\n@ Files / / Commands / Esc Review draft"
    } else if app.effective_focus() != Pane::Workspace {
        "g Edit goal / Tab Focus\nEnter opens focused details"
    } else if matches!(app.form, 5..=7) {
        "Enter Specialist desk / g Edit goal"
    } else if app.form == 8 {
        "Enter Prepare handoff / g Edit goal"
    } else {
        "Enter Edit goal / e Review provider request"
    };
    text(
        frame,
        row(content, content.height.saturating_sub(2), 2),
        hint,
        paint.muted,
    );
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
        "Delete {} tasks/drafts and listed evidence/task memories? Shared memories, sources, exports and provider sessions stay. Not secure erasure. Page {}/{} (n/p).",
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
        Memory(&'a crate::memory::Record),
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
                .chain(target.memories.iter().map(Item::Memory))
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
                Item::Memory(record) => format!(
                    "  task memory #{} {}",
                    record.id,
                    clipped(
                        &record.proposal.content,
                        details.width.saturating_sub(24) as usize
                    )
                ),
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
                budget: crate::budget::Limits::default(),
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

    #[test]
    fn task_layout_keeps_composer_full_width_and_details_progressive() {
        for (width, height) in [(120, 36), (100, 30), (80, 24), (70, 24), (60, 18)] {
            let area = Rect::new(0, 0, width, height);
            let layout = shell(area, false, Pane::Workspace);
            assert_eq!(layout.composer.width, width - 2);
            assert!(
                inner(layout.composer).height > 2,
                "input remains above hints"
            );
            assert!(layout.workspace.height > 0);
            assert!(layout.workspace.bottom() < layout.composer.y);
            assert_eq!(layout.composer.bottom(), layout.footer.y);
            assert!(layout.inspector.is_none());
            assert_eq!(layout.rail.is_some(), width >= 90);
            if let Some(rail) = layout.rail {
                assert!(rail.bottom() < layout.composer.y);
                assert!(layout.workspace.right() < rail.x);
                let image = companion_portrait(rail);
                assert!(image.right() <= rail.right());
                assert!(image.bottom() < rail.bottom());
            }
            let linear = shell(area, true, Pane::Inspector);
            assert!(linear.rail.is_none());
            assert!(linear.inspector.is_none());
            assert_eq!(linear.workspace.width, linear.composer.width);
        }
        let focused = shell(Rect::new(0, 0, 120, 36), false, Pane::Inspector);
        assert!(focused.inspector.is_some());
        assert!(focused.rail.is_none());
    }
}
