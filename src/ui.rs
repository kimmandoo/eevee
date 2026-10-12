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
        text(
            frame,
            area,
            "Terminal too small\nNeed 60 columns x 18 rows\nResize keeps your draft. Ctrl+C opens safe exit.",
            paint.accent,
        );
        if app.confirmation.is_some() {
            confirmation(frame, area, app, config, &paint);
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
        format!("{} / local draft", form.name)
    } else {
        "Task: local draft / session only".to_owned()
    };
    text(frame, row(area, 1, 1), &state, paint.accent);
    text(
        frame,
        row(area, 2, 1),
        "Agent: not connected / no work running",
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
    text(frame, row(content, 0, 1), "NEXT LOCAL ACTION", paint.muted);
    text(
        frame,
        row(content, 1, 2),
        if app.goal.is_empty() {
            "g / Enter  Edit session goal"
        } else {
            "f  Choose a specialist / p  Preview handoff"
        },
        paint.accent,
    );
    let artifact = row(content, 4, content.height.saturating_sub(4));
    let offset = app.scroll_for(ScrollSurface::Workspace);
    let mut lines = vec![Line::styled("SESSION GOAL", paint.muted), Line::from("")];
    if app.goal.is_empty() {
        lines.push(Line::from("Give your companion one bounded goal."));
    } else {
        lines.extend(app.goal.lines().map(Line::from));
    }
    lines.push(Line::from(""));
    lines.push(Line::styled("EXPECTED ARTIFACT", paint.muted));
    lines.push(Line::from(form.evidence));
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "Specialist work: not implemented. Preview sends nothing.",
        paint.muted,
    ));
    if app.linear {
        lines.push(Line::from(""));
        lines.push(Line::from("i Inspector / b Companion / l Sanctuary layout"));
    }
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
        " Session inspector ",
        config,
        paint,
        drawer || app.effective_focus() == Pane::Inspector,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let path = config.assets_dir.to_string_lossy();
    let safe_path: String = path.chars().filter(|c| !c.is_control()).collect();
    let value = format!(
        "PROJECT DIRECTORY\n{}\n\nCONTROL BOUNDARY\nNo grants or pending approvals\n\nEVIDENCE\nRepository not inspected\nNo results recorded\n\nSTORAGE\nSession only; discarded on exit\n\nARTWORK\n{}\n{}\n{}\n\nUp/Down scroll / Esc back",
        app.cwd,
        artwork.status(app.form),
        graphics_status,
        safe_path
    );
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
            "{} / {}\nLocal selection: ready / work unavailable\np Inspect role and expected artifact",
            FORMS[selected].name, FORMS[selected].role
        );
        text(frame, info, &value, paint.muted);
    }
}

fn form_details(frame: &mut Frame, area: Rect, index: usize, config: &Config, artwork: &Artwork) {
    let form = &FORMS[index];
    let paint = Paint::new(config, form, index);
    let value = format!(
        "{} / {}\n{}\n\n{}\n\nLocal selection: available\nSpecialist work: not implemented\n{}\n\nArrows / 1-9 preview\nEnter select / Esc keep current",
        form.name,
        form.element,
        form.role,
        form.evidence,
        artwork.status(index)
    );
    frame.render_widget(
        Paragraph::new(value)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn palette(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let area = centered(area, 66, 15);
    frame.render_widget(Clear, area);
    let block = panel(" Local command palette ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    for (index, command) in PALETTE.iter().enumerate() {
        text(
            frame,
            row(content, index as u16 + 1, 1),
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
        row(content, 10, 2),
        "Agent execution unavailable: connection and runtime not implemented.",
        paint.muted,
    );
}

fn help(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = centered(area, 78, 24);
    frame.render_widget(Clear, area);
    let block = panel(" Keyboard / local actions ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let value = "f          Evolution picker: arrows/1-9 preview\ng          Edit local goal; Enter applies, Esc goes back\np          Preview selected handoff; Run unavailable\ni / b      Inspector / companion drawer\nl          Toggle linear / sanctuary layout\nCtrl+K     Local command palette\nTab        Cycle visible panes\nUp/Down    Select or scroll the focused surface\nPageUp/Down Scroll a page\nHome/End   First/last selection or cursor\nEnter      Open or explicitly confirm\nEsc        Close overlay and restore focus\n?          Show/close this help\nq          Quit outside editor; types q in editor\nCtrl+C     Safe exit; confirm session data loss\n\nGoal editor: Left/Right move by grapheme.\nBackspace/Delete remove one whole grapheme.\nUp/Down move between lines; Ctrl+N inserts newline.\nShift+Enter also inserts newline when the terminal distinguishes it.\nPaste preserves newlines but never applies or executes.\nCtrl+K and form shortcuts are inactive while editing.\nChanged drafts require deliberate discard.\nNeutral Enter keeps your draft; Tab/Right selects discard.\n\nArtwork: use --images cells|off for compatibility.\nNo automatic download, model request, or permissions.\nLinear mode is text-first; no assistive-tool claim yet.";
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
        "GOAL\n{}\n\nROLE\n{} / {}\n\nEXPECTED ARTIFACT\n{}\n\nCONTEXT\nNo selected source excerpts. Project not inspected.\n\nEXECUTION / CONTROL\nAgent: not connected. No grants.\nRun unavailable: provider, storage, and runtime not implemented.\nThis preview performs no operation.\n\nEsc returns to the invoking control.",
        if app.goal.is_empty() {
            "No session goal set"
        } else {
            &app.goal
        },
        form.name,
        form.role,
        form.evidence
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
    let value = if app.editing {
        "Enter Apply locally / Esc Back / Arrows Move / Ctrl+N Newline / Ctrl+C Safe exit"
    } else {
        match app.overlay {
            Some(Overlay::Forms) => {
                "Arrows Preview / 1-9 Preview / Enter Select / p Details / Esc Back"
            }
            Some(Overlay::Palette) => "Up/Down Choose / Enter Open / Esc Back / Ctrl+C Exit",
            Some(_) => "Up/Down Scroll / Esc Back / Ctrl+C Exit",
            None => match app.effective_focus() {
                Pane::Companion => "Enter Evolution / Tab Focus / i Inspect / ? Help / q Quit",
                Pane::Inspector => "Enter Inspect / Tab Focus / f Evolution / ? Help / q Quit",
                Pane::Workspace => "g Edit goal / f Evolution / Ctrl+K Actions / ? Help / q Quit",
            },
        }
    };
    if app.scroll() > 0 && !app.editing {
        text(
            frame,
            area,
            &format!("{value} / line {}", app.scroll() + 1),
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
    let block = panel(
        " Edit session goal / local apply only ",
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        row(content, 0, 2),
        "Not saved or sent. Paste is text, never an action.",
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
            "{} / 4096 bytes{}\nEnter Apply / Esc Back / Arrows Move / Ctrl+N Newline",
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

fn confirmation(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let Some(confirm) = app.confirmation.as_ref() else {
        return;
    };
    let area = if area.width < 30 || area.height < 10 {
        area
    } else {
        centered(area, 62, 10)
    };
    frame.render_widget(Clear, area);
    let block = panel(" Confirm local data loss ", config, paint, true);
    let content = if area.width < 8 || area.height < 5 {
        area
    } else {
        block.inner(area)
    };
    frame.render_widget(block, area);
    let (question, keep, discard) = match confirm.kind {
        ConfirmationKind::DiscardDraft => (
            "Discard this changed draft? Applied goal is kept.",
            "Keep editing",
            "Discard draft",
        ),
        ConfirmationKind::Quit => (
            "Exit? Session goal and draft will be lost.",
            "Stay",
            "Discard and quit",
        ),
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
