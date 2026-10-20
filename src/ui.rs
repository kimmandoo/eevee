mod agents;
pub mod artwork;
mod composer;
mod execution;
mod handoff;
mod memory;
mod planning;
mod sources;
mod workflows;

use std::{borrow::Cow, cell::Cell};

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
    app::{App, ConfirmationKind, Overlay, Pane, ScrollSurface},
    config::{ColorMode, Config, Theme},
    domain::{FORMS, Form},
    store,
    store::EditorField,
    ui::artwork::Artwork,
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

thread_local! {
    /// `/verbose` for the frame being drawn; set once at the start of `render`.
    static VERBOSE: Cell<bool> = const { Cell::new(false) };
}

/// Coarse age of a Unix-millisecond timestamp ("just now", "5m ago", "3d ago").
fn ago(unix_ms: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64);
    match now.saturating_sub(unix_ms).max(0) / 1000 {
        seconds if seconds < 60 => "just now".to_owned(),
        seconds if seconds < 3600 => format!("{}m ago", seconds / 60),
        seconds if seconds < 86_400 => format!("{}h ago", seconds / 3600),
        seconds => format!("{}d ago", seconds / 86_400),
    }
}

/// First sentence of an explanation plus any "Next:" step, marked "…" when more
/// is hidden. `/verbose` shows everything. Never use for consent reviews, which
/// must stay complete.
fn brief(value: &str) -> Cow<'_, str> {
    if VERBOSE.get() {
        return Cow::Borrowed(value);
    }
    let mut sentences = Vec::new();
    let mut start = 0;
    for (index, character) in value.char_indices() {
        let rest = &value[index + character.len_utf8()..];
        let end = match character {
            '.' | '!' | '?' if rest.starts_with([' ', '\n']) => index + 1,
            '\n' if rest.starts_with('\n') => index,
            _ => continue,
        };
        if end > start {
            sentences.push(value[start..end].trim());
        }
        start = end;
    }
    sentences.push(value[start..].trim());
    sentences.retain(|sentence| !sentence.is_empty());
    let kept: Vec<_> = sentences
        .iter()
        .enumerate()
        .filter(|(index, sentence)| *index == 0 || sentence.starts_with("Next:"))
        .map(|(_, sentence)| *sentence)
        .collect();
    if kept.len() == sentences.len() {
        Cow::Borrowed(value)
    } else {
        Cow::Owned(format!("{} …", kept.join(" ")))
    }
}

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
        area.y + (area.height - height) / 2,
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
        || app.nickname_editor.is_some()
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
        Some(Overlay::Companion) => targets.push((app.form(), portrait(drawer(area)))),
        Some(_) => {}
        None => {
            if let Some(rail) = shell(area, app.linear, app.effective_focus()).rail {
                targets.push((app.form(), companion_portrait(rail)));
            }
        }
    }
    targets.retain(|&(_, target)| !occluded(area, app, target));
}

/// Center choices while reserving the header and footer at supported sizes.
fn command_popup_area(area: Rect, _app: &App) -> Rect {
    centered(area, 100, area.height.saturating_sub(4).min(22))
}

fn composer_menu_area(area: Rect, app: &App) -> Option<Rect> {
    if app.composer_menu.is_none() || app.overlay.is_some() || app.confirmation.is_some() {
        return None;
    }
    Some(centered(area, 84, area.height.saturating_sub(4).min(12)))
}

/// Native images draw above cells, so only portraits under a popup are withheld.
fn occluded(area: Rect, app: &App, target: Rect) -> bool {
    if app.nickname_editor.is_some() {
        return true;
    }
    app.command_popup.is_some() && command_popup_area(area, app).intersects(target)
        || composer_menu_area(area, app).is_some_and(|menu| menu.intersects(target))
}

pub fn render(
    frame: &mut Frame,
    app: &mut App,
    config: &Config,
    artwork: &mut Artwork,
    graphics_status: &str,
) {
    VERBOSE.set(app.verbose);
    let area = frame.area();
    app.confirmation_reviewed = false;
    if let Some(confirmation) = &mut app.sources.confirmation {
        confirmation.reviewed = false;
    }
    let form = &FORMS[app.form().min(8)];
    let paint = Paint::new(config, form, app.form().min(8));
    frame.render_widget(Block::default().style(paint.base), area);
    if area.width < 60 || area.height < 18 {
        app.agents.affirmative = false;
        app.agents.reviewed = false;
        app.images_reviewed = false;
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
    header(frame, shell.header, app, config, &paint);
    // A provider review owns its modal; do not wrap hidden response output.
    if app.overlay != Some(Overlay::Planning) {
        if let Some(rail) = shell.rail {
            compact_companion(
                frame,
                rail,
                app,
                config,
                &paint,
                artwork,
                app.overlay.is_none()
                    && !occluded(area, app, companion_portrait(rail))
                    && !(app.editing
                        && matches!(
                            app.editor_field,
                            EditorField::Criteria | EditorField::Command
                        )),
            );
        }
        let workspace_area = if matches!(app.overlay, Some(Overlay::Tasks | Overlay::Forms)) {
            shell.workspace
        } else {
            suggestion_strip(frame, shell.workspace, app, &paint)
        };
        if app.response_on_home() {
            planning::render_workspace(frame, workspace_area, app, config, &paint);
        } else {
            workspace(frame, workspace_area, app, config, &paint);
        }
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
    }
    footer(frame, shell.footer, app, &paint);
    if let Some(menu) = composer_menu_area(area, app) {
        composer::render(frame, menu, app, config, &paint);
    }
    if let Some(overlay) = app.overlay {
        match overlay {
            Overlay::Forms => forms(frame, area, app, config, &paint, artwork),
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
            Overlay::Party => party(frame, area, app, config, &paint),
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
    if app.nickname_editor.is_some() {
        nickname(frame, area, app, config, &paint);
    }
    if app.command_popup.is_some() {
        composer::commands(frame, area, app, config, &paint);
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

fn selection_line(app: &App) -> String {
    selection_text(&app.planning)
}

/// Provider, model and effort at a glance; `/verbose` adds observation caveats.
fn selection_text(state: &crate::app::planning::State) -> String {
    if VERBOSE.get() {
        state.selection_label()
    } else {
        format!("{} · {}", state.agent.label(), state.selection.summary())
    }
}

fn activity_marker(frame: u8, config: &Config) -> &'static str {
    if config.no_animation {
        "*"
    } else if config.ascii {
        ["|", "/", "-", "\\"][usize::from(frame) % 4]
    } else {
        ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][usize::from(frame) % 10]
    }
}

fn header(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let project = std::path::Path::new(&app.cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&app.cwd);
    if area.width < 90 {
        use crate::providers::Agent;
        let route = match app.planning.agent {
            Agent::Codex => "Codex/CLI",
            Agent::ChatGPT => "Codex/native",
            Agent::Claude => "Claude/SDK",
            Agent::OpenCode => "OpenCode/ACP",
            Agent::Gemini => "Gemini/ACP",
            Agent::Devin => "Devin/ACP",
            Agent::OpenAI => "OpenAI",
            Agent::Anthropic => "Anthropic",
            Agent::Google => "Gemini/API",
            Agent::Compatible => "Custom",
        };
        let effort = Span::styled(
            format!(
                "/{}",
                clipped(
                    app.planning
                        .selection
                        .effort
                        .as_deref()
                        .unwrap_or("default"),
                    8
                )
            ),
            paint.muted,
        );
        let model_name = app.planning.selection.model.as_deref().unwrap_or("default");
        let model_hint_width = Span::raw(model_name).width().min(20);
        let identity_budget = usize::from(area.width)
            .saturating_sub(Span::raw(route).width() + effort.width() + model_hint_width + 6);
        let suffix = format!(" ({})", FORMS[app.form()].name);
        let name = App::partner_label(app.lead());
        let identity = format!(
            "{}{suffix}",
            clipped(
                &name,
                identity_budget.saturating_sub(Span::raw(suffix.as_str()).width())
            )
        );
        let spare = usize::from(area.width).saturating_sub(
            Span::raw(identity.as_str()).width()
                + Span::raw(route).width()
                + effort.width()
                + model_hint_width
                + 6,
        );
        let context = if spare >= 7 {
            format!("{identity} · {} · {route} · ", clipped(project, spare - 3))
        } else {
            format!("{identity} · {route} · ")
        };
        let context = Span::styled(context, paint.muted);
        let model_width = usize::from(area.width).saturating_sub(context.width() + effort.width());
        let model = Span::styled(
            clipped(
                app.planning.selection.model.as_deref().unwrap_or("default"),
                model_width,
            ),
            paint.accent,
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![context, model, effort])),
            row(area, 0, 1),
        );
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" EEVEE ", paint.selected),
                Span::styled(
                    format!("  {} / {}", app.lead_label(), clipped(project, 16)),
                    paint.base,
                ),
                Span::styled(format!("  /  {}", selection_line(app)), paint.muted),
            ])),
            row(area, 0, 1),
        );
    }
    let activity = if app.planning.request.is_some() {
        "Approval needed / /provider Review"
    } else if app.planning.cancelling {
        "Cancelling provider / /provider Status"
    } else if app.planning.preview.is_some() {
        "Request prepared / /provider Details"
    } else if app.planning.preference_error().is_some() {
        "Model preference not saved / /storage-retry"
    } else if let Some(activity) = app.planning.activity_label() {
        activity
    } else if app.planning.outcome_unknown() {
        "Provider outcome unknown / /provider Inspect"
    } else {
        app.execution.activity_label()
    };
    let status = format!(
        "{} {activity}  /  {}",
        if app.planning.activity_label().is_some() {
            activity_marker(app.activity_frame, config)
        } else {
            ""
        },
        app.storage_label()
    );
    let line = row(area, 1, 1);
    text(
        frame,
        line,
        &clipped(&status, area.width as usize),
        paint.muted,
    );
    // Capability chip (EV03A): lowest header priority, so it appears only in
    // space the activity/storage text leaves free and never clips it.
    let chip = format!("[{}] ", crate::domain::CAPABILITIES[app.form().min(8)]);
    let (chip_width, used) = (
        Span::raw(chip.as_str()).width() as u16,
        Span::raw(status.as_str()).width() as u16,
    );
    if used + 2 + chip_width <= line.width {
        text(
            frame,
            Rect::new(line.right() - chip_width, line.y, chip_width, 1),
            &chip,
            paint.accent,
        );
    }
}

fn suggestion_line<'a>(app: &'a App, paint: &Paint) -> Option<Line<'a>> {
    app.visible_suggestion().map(|suggestion| {
        let mut spans = vec![
            Span::raw("Suggested: "),
            Span::raw(FORMS[suggestion.form.min(FORMS.len() - 1)].name),
            Span::raw(" — "),
            Span::raw(suggestion.reason.as_str()),
        ];
        if let Some(partner) = app.suggested_partner() {
            spans.push(Span::raw(format!(
                " · {} ({}) is free",
                App::partner_label(partner),
                FORMS[partner.form].name
            )));
        }
        Line::from(spans).style(paint.accent)
    })
}

/// Descriptive local evidence only; rendering never evaluates or selects a form.
fn suggestion_strip(frame: &mut Frame, area: Rect, app: &App, paint: &Paint) -> Rect {
    if let Some(carry_over) = app.carry_over() {
        // The precomputed ASCII summary fits the minimum workspace width.
        // Give this transient record the existing strip slot, not a second row.
        frame.render_widget(
            Paragraph::new(carry_over.summary.as_str()).style(paint.accent),
            row(area, 0, 1),
        );
        return row(area, 1, area.height.saturating_sub(1));
    }
    let Some(line) = suggestion_line(app, paint) else {
        return area;
    };
    let paragraph = Paragraph::new(line).wrap(Wrap { trim: false });
    let height = paragraph
        .line_count(area.width.max(1))
        .min(usize::from(area.height.saturating_sub(5))) as u16;
    frame.render_widget(paragraph, row(area, 0, height));
    row(area, height, area.height.saturating_sub(height))
}

fn workspace(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let block = panel(
        " RESPONSE ",
        config,
        paint,
        app.effective_focus() == Pane::Workspace && !app.editing && app.overlay.is_none(),
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let heading = if app.goal.trim().is_empty() {
        Line::styled("What would you like to do?", paint.accent)
    } else {
        Line::from(vec![
            Span::styled("Task: ", paint.accent),
            Span::styled(
                clipped(
                    app.goal.lines().next().unwrap_or(""),
                    content.width.saturating_sub(6) as usize,
                ),
                paint.accent,
            ),
        ])
    };
    let (signature, action, label) = crate::domain::FIRST_ACTIONS[app.form().min(8)];
    let mut lines = vec![
        heading,
        Line::from(format!(
            "{} / {}: {signature}",
            crate::domain::FORMS[app.form().min(8)].name,
            crate::domain::FORMS[app.form().min(8)].element
        )),
        Line::styled(
            match action {
                Some(command) => format!("/{command} {label}"),
                None => label.to_owned(),
            },
            paint.accent,
        ),
        Line::from(""),
        Line::styled("@ Attach context / /connect Choose provider", paint.muted),
        Line::styled("/tasks Resume / /goal Optional task facts", paint.muted),
    ];
    if app
        .drafts
        .iter()
        .any(|draft| draft.field != EditorField::Request)
    {
        lines.push(Line::styled(
            "Task-fact draft retained / /goal / /next",
            paint.muted,
        ));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        content,
    );
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
    text(frame, row(content, 0, 1), &app.lead_label(), paint.accent);
    text(
        frame,
        row(content, 1, 1),
        FORMS[app.form()].element,
        paint.muted,
    );
    let image = companion_portrait(area);
    if show_image && app.nickname_editor.is_none() {
        artwork.render(app.form(), image, frame.buffer_mut());
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
        if focused && app.last_carry_over().is_some() {
            "c Carry-over / Enter"
        } else if focused {
            "> Enter Form / n Nickname"
        } else {
            "/forms / /companion"
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
    let form = &FORMS[app.form()];
    frame.render_widget(
        Paragraph::new(app.lead_label())
            .style(paint.accent)
            .alignment(Alignment::Center),
        row(content, 0, 1),
    );
    frame.render_widget(
        Paragraph::new(if app.last_carry_over().is_some() {
            "/forms /images  n Nickname  c Carry-over"
        } else {
            "/forms / /images / n Nickname"
        })
        .style(paint.accent)
        .alignment(Alignment::Center),
        row(content, 1, 1),
    );
    let image = portrait(area);
    if show_image && app.nickname_editor.is_none() {
        artwork.render(app.form(), image, frame.buffer_mut());
    }
    let details = row(
        content,
        2 + image.height,
        content.height.saturating_sub(2 + image.height),
    );
    let mut lines = Vec::new();
    if app.last_carry_over().is_some() {
        lines.extend([
            Line::styled("C=criteria met/total P=committed pins", paint.muted),
            Line::styled("H=open hypotheses F=open findings", paint.muted),
        ]);
    }
    lines.extend([
        Line::styled(form.element, paint.muted),
        Line::from(form.role),
        Line::styled(artwork.status(app.form()), paint.muted),
        Line::styled("/party Roster / /lead Front a partner", paint.accent),
    ]);
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
            ago(app.updated_at)
        } else {
            "Not committed yet".to_owned()
        };
        let quota = app.policy.quota_bytes;
        let sampled = app.usage_bytes;
        let retention = app.policy.retention_days;
        format!(
            "TASK\n{task} / {}\nUpdated: {updated}\nCriteria: {}/{} met\nRetained drafts: {}\nComplete means user-reviewed criteria, not verified command evidence.\n\nPROJECT DIRECTORY\n{}\n\nSTORAGE\n{}\nRevision {} / committed {}\nDirectory: {}\n{}\nRecovery: /storage-retry; --export-data FILE exports readable local records, not provider history.\nQuota: {quota} bytes. Last sample: {sampled} live SQLite, retained output and committed reservations; not physical database size.\nRetention: archived at least {retention} days; explicit purge only, no automatic eviction.\nSecret detection is best-effort. Volatile editor text may remain unmasked.\nArchive is reversible. Deletion affects selected local records and owned artifacts, not repository files, exports or provider sessions. Not forensic secure erasure.\n\nEXECUTION\n/provider opens reviewed planning. /run opens local command approval/evidence. /handoff opens portable handoff. Cancellation does not roll back files; approval is not a sandbox.\n\nARTWORK\n{}\n{}\n{}\n\nUp/Down scroll / Esc back",
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
            artwork.status(app.form()),
            graphics_status,
            safe_path
        )
    } else {
        format!(
            "PROJECT\n{project}\n\nTASK\n{task} / {}\nCriteria: {}/{} met\nRetained drafts: {}\n\nSTORAGE\n{}\n{}\n\n/provider Requests / /run Local checks / /handoff Handoff\nUp/Down scroll",
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
    let current = FORMS[app.form()].name;
    let heading = match (app.visible_suggestion(), grid) {
        (Some(suggestion), false) => format!(
            "Current: {current} / Suggested: {} / d Dismiss",
            FORMS[suggestion.form].name
        ),
        (Some(_), true) => format!("Current: {current} / d Dismiss suggestion"),
        (None, _) => format!("Current: {current}"),
    };
    text(frame, row(content, 0, 1), &heading, paint.muted);
    text(
        frame,
        row(content, 1, 1),
        if app.form_return_selected {
            "> Return to Eevee / intake"
        } else {
            "  Return to Eevee / intake"
        },
        if app.form_return_selected {
            paint.selected
        } else {
            paint.accent
        },
    );
    let selected = app.selected.min(8);
    let form_selected = !app.form_return_selected;
    let suggested = app.visible_suggestion().map(|suggestion| suggestion.form);
    if grid {
        for (index, form) in FORMS.iter().enumerate() {
            let cell = grid_cell(content, index);
            let cell_paint = Paint::new(config, form, index);
            let block = panel(
                form.name,
                config,
                &cell_paint,
                form_selected && index == selected,
            );
            let inside = block.inner(cell);
            frame.render_widget(block, cell);
            text(
                frame,
                row(inside, 0, 1),
                &format!(
                    "{} {}{}{}",
                    if form_selected && index == selected {
                        ">"
                    } else {
                        " "
                    },
                    index + 1,
                    if index == app.form() { " *" } else { "" },
                    if suggested == Some(index) {
                        " suggested"
                    } else {
                        ""
                    }
                ),
                if form_selected && index == selected {
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
                "{} {} {:<9}{}{}",
                if form_selected && index == selected {
                    ">"
                } else {
                    " "
                },
                index + 1,
                form.name,
                if index == app.form() { " *" } else { "" },
                if suggested == Some(index) {
                    " suggested"
                } else {
                    ""
                }
            );
            text(
                frame,
                item,
                &label,
                if form_selected && index == selected {
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
            "{} / {}\n/preview Role details",
            FORMS[selected].name, FORMS[selected].role
        );
        text(frame, info, &value, paint.muted);
    }
}

fn form_details(frame: &mut Frame, area: Rect, index: usize, config: &Config, artwork: &Artwork) {
    let form = &FORMS[index];
    let paint = Paint::new(config, form, index);
    let value = format!(
        "{} / {}\n{}\n\n{}\n\n{}\n{}\n\nArrows Preview\n/preview Role details",
        form.name,
        form.element,
        form.role,
        form.evidence,
        match index {
            0 => "Task cards and resumption: available",
            1 => "Source ranges/context basket: /files",
            2 => "Approved local commands/evidence: /run",
            3 => "Reviewed selected-file implementation: /provider / /role",
            4 => "Reviewed multi-agent planning: /provider",
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
            "No task cards in this view.\n\n/archived Active/archived / /task-new New / Esc Back\nArchive is reversible; deletion requires review.",
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
            "{} cards / /archived View / {} / /delete / /prune Retention",
            count,
            if app.show_archived {
                "/restore"
            } else {
                "/archive"
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
        Line::from(format!(
            "Updated {} / {}/{} criteria met{}{}{}",
            if task.updated_at > 0 {
                ago(task.updated_at)
            } else {
                "-".to_owned()
            },
            task.criteria
                .iter()
                .filter(|criterion| criterion.met)
                .count(),
            task.criteria.len(),
            match task
                .partner_drafts
                .values()
                .map(|state| state.drafts.len())
                .sum::<usize>()
            {
                0 => String::new(),
                count => format!(" / {count} drafts"),
            },
            if task.archived { " / archived" } else { "" },
            if task.redacted {
                " / secrets masked"
            } else {
                ""
            },
        )),
        Line::styled("GOAL", paint.muted),
    ];
    if task.id == app.task_id
        && let Some(line) = suggestion_line(app, paint)
    {
        lines.insert(1, line);
    }
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
            "{} / {}/{} met",
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
            "No criteria yet. /criteria-edit adds them, one per line.",
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
            "Enter toggles your review. /criteria-edit edits; /complete reviews completion."
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

fn nickname(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let Some(editor) = &app.nickname_editor else {
        return;
    };
    let area = centered(area, 58, 13);
    frame.render_widget(Clear, area);
    let block = panel(" Nickname / local only ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        row(content, 0, 2),
        "Local only: provider / handoff nickname bytes 0 B.\n12 graphemes; empty clears. No send, run or grant.",
        paint.muted,
    );
    let input = row(content, 3, 2);
    let (lines, (caret_row, caret_column)) = editor_rows(
        &editor.text,
        editor.cursor,
        input.width.saturating_sub(1) as usize,
    );
    frame.render_widget(Paragraph::new(lines.join("\n")).style(paint.surface), input);
    if caret_row < input.height as usize && caret_column < input.width as usize {
        frame.set_cursor_position((input.x + caret_column as u16, input.y + caret_row as u16));
    }
    frame.render_widget(
        Paragraph::new(editor.error.as_str())
            .style(paint.accent)
            .wrap(Wrap { trim: false }),
        row(content, 6, 2),
    );
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        if app.party_inflight || app.party_request.is_some() {
            "Saving locally / Esc Back"
        } else {
            "Enter Save locally / Esc Cancel"
        },
        paint.muted,
    );
}

/// Party roster (PT04A/PT05A): every partner with its form, a derived status
/// and the task it is engaged with. Status is recomputed from live
/// request/approval state each frame; Enter/`l` fronts the highlighted partner.
fn party(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = centered(area, 88, 24);
    frame.render_widget(Clear, area);
    let block = panel(" Party roster ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let partners = app.partners();
    text(
        frame,
        row(content, 0, 1),
        &format!(
            "{}/{} partners / status is derived, never stored",
            partners.len(),
            store::MAX_PARTNERS
        ),
        paint.muted,
    );
    for (index, partner) in partners.iter().enumerate() {
        let label = format!(
            "{}{} {} · {} · {} · {}",
            if index == app.selected { ">" } else { " " },
            if partner.id == app.lead().id {
                "*"
            } else {
                " "
            },
            App::partner_label(partner),
            FORMS[partner.form.min(FORMS.len() - 1)].name,
            app.partner_status(partner).label(),
            app.partner_task(partner),
        );
        text(
            frame,
            row(content, index as u16 + 2, 1),
            &clipped(&label, content.width as usize),
            if index == app.selected || partner.id == app.lead().id {
                paint.accent
            } else {
                paint.surface
            },
        );
    }
    text(
        frame,
        row(content, partners.len() as u16 + 3, 1),
        if partners.len() >= store::MAX_PARTNERS {
            "Arrows Select / Enter Lead / n Nickname / Esc Back"
        } else {
            "Arrows Select / Enter Lead / n Nickname / a Add"
        },
        paint.muted,
    );
    app.set_scroll_max(ScrollSurface::Preview, 0);
}

fn selection_details(frame: &mut Frame, area: Rect, paragraph: Paragraph<'_>, offset: u16) -> u16 {
    let max = paragraph
        .line_count(area.width.max(1))
        .saturating_sub(area.height as usize)
        .min(u16::MAX as usize) as u16;
    frame.render_widget(paragraph.scroll((offset.min(max), 0)), area);
    max
}

fn help(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = centered(area, 78, 24);
    frame.render_widget(Clear, area);
    let block = panel(" Help / PgUp/Dn Scroll / Esc Back ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let value = "KEYS\n / Ctrl+K   Commands (type to search)\n Enter      Send request / select / confirm\n Esc        Back (drafts are kept)\n Tab        Switch pane\n PgUp/PgDn  Scroll response or details\n Shift+Enter, Ctrl+N  Newline\n Ctrl+S     Keep draft locally\n @          Attach files\n Backspace  Previous form (outside text)\n Ctrl+C     Exit\n\nCOMMANDS\n /connect /provider  Provider and connection method\n /model /effort      Model and reasoning effort\n /new /resume        Advanced session request review\n /task-new /tasks    Local tasks\n /reset              Fresh task; old task/draft in /tasks\n /init               Draft project guide request; not sent\n /goal /criteria     Optional task facts\n /party /lead NAME   Partners and the lead\n /forms /role        Evolution and role\n /files /run         Source context, local commands\n /complete           Mark acceptance checklist complete\n /verbose            Brief / full explanations\n\nSAFETY\n Enter sends the saved request to the selected provider.\n Commands, file changes and privilege grants require approval.\n Project-supplied runtimes and unmanaged tool routes still need connection review.\n Full technical details remain inspectable.\n Pasted text never sends a request or runs commands.\n Provider output and handoffs are unverified context.";
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
        "GOAL\n{}\n\nROLE\n{} / {}\n\nEXPECTED ARTIFACT\n{}\n\nCONTEXT\n{} captured references loaded. /files opens envelope preview and source revalidation. Provider-added history or reads are not governed by this basket.\n\nEXECUTION / CONTROL\nThis local preview sends nothing. /provider opens separately reviewed planning. Command approval is separate in /run.\nThis preview performs no operation and sends nothing.\n\nEsc returns to the invoking control.",
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
        "y Confirm / n Stay / Tab Choose / Enter Select"
    } else if app.reset_after_save {
        "Saving old task and drafts before reset / Esc Stay"
    } else if app.transitioning {
        "Applying task operation / mutation paused"
    } else if app.command_popup.is_some() {
        "Type / arrows Choose / Enter Open / Esc Back"
    } else if app.composer_menu.is_some() && app.overlay.is_none() {
        "Arrows Choose file / Enter Inspect / Esc Keep text"
    } else if app.editing && app.overlay.is_none() {
        if app.composer_is_request() && app.planning.busy() {
            "Draft only / PgUp/Dn Response / /provider / /cancel"
        } else if app.composer_is_request() && app.response_on_home() {
            "Enter Send / PgUp/Dn Response / / Commands"
        } else if app.composer_is_request() {
            "Enter Send / Ctrl+N Newline / @ Files / / Commands"
        } else {
            "Enter Save fact / Ctrl+N Newline / Esc Draft"
        }
    } else {
        match app.overlay {
            Some(Overlay::Forms) => "Arrows Preview / Home Intake / Enter Select / Esc Back",
            Some(Overlay::Criteria) => {
                "Enter Toggle / /criteria-edit / /complete / Arrows Choose / Esc Back"
            }
            Some(Overlay::Tasks) => {
                "Enter Resume / /archived / /archive / /restore / /delete / Esc Back"
            }
            Some(Overlay::Planning) if app.planning.activity_label().is_some() => {
                "/ Commands / Esc Cancel request / Ctrl+C Safe exit"
            }
            Some(
                Overlay::Execution
                | Overlay::Agents
                | Overlay::Planning
                | Overlay::Handoff
                | Overlay::Workflows
                | Overlay::Memory,
            ) => "/ Commands / Esc Back / Ctrl+C Exit",
            Some(_) => "/ Commands / Up/Down Scroll / Esc Back / Ctrl+C Exit",
            None => "Type Request / Tab Focus / / Commands / Ctrl+C Exit",
        }
    };
    if !app.notice.is_empty() {
        text(
            frame,
            row(area, 0, 1),
            &clipped(&brief(&app.notice), area.width as usize),
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
        // Hints sit on the bottom row so the notice row above stays free.
        let lines = value.lines().count().max(1) as u16;
        let top = area.height.saturating_sub(lines);
        text(frame, row(area, top, area.height - top), value, paint.base);
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
    let path = crate::workspace::privacy::redact(&display_path);
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
    let summary = "HTTPS: raw.githubusercontent.com / PokeAPI sprites.\nWrites image cache only; existing files kept.\nNo repository edits. Art is not a rights license.";
    let summary_rows = Paragraph::new(summary)
        .wrap(Wrap { trim: false })
        .line_count(content.width.max(1)) as u16;
    text(frame, row(content, 0, summary_rows), summary, paint.surface);
    let details = row(
        content,
        summary_rows,
        content.height.saturating_sub(summary_rows + 4),
    );
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
    app.images_reviewed = content.height >= summary_rows + 4;
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
        "Tab Choose / Enter Select / PgUp/Dn Details / Esc Back",
        paint.muted,
    );
}

fn composer_input(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let editing = app.editing
        && matches!(
            app.editor_field,
            EditorField::Request | EditorField::Goal | EditorField::NextAction
        );
    let title = if app.composer_is_request() || !editing {
        " Request "
    } else if editing && app.editor_field == EditorField::NextAction {
        " Next action / local "
    } else {
        " Task fact / local "
    };
    let attachment_status = if app.sources.store_failed {
        " · save failed"
    } else if app.sources.persistence_pending() {
        " · saving"
    } else {
        ""
    };
    let title = format!(
        "{title}/ {} / {} attached{attachment_status} ",
        app.lead_label(),
        app.sources.basket.len()
    );
    let mut block = panel(&title, config, paint, editing && app.overlay.is_none());
    if editing {
        block = block.title_bottom(Line::styled(
            " Ctrl+N Newline / Ctrl+S Retain draft ",
            paint.muted,
        ));
    }
    let content = block.inner(area);
    frame.render_widget(block, area);
    let hint_rows = if editing && !app.input_truncated {
        0
    } else {
        1
    };
    let attachment_rows = u16::from(!app.sources.basket.is_empty() && content.height >= 3);
    if attachment_rows != 0 {
        composer::attachments(frame, row(content, 0, 1), app, paint);
    }
    let input = row(
        content,
        attachment_rows,
        content.height.saturating_sub(hint_rows + attachment_rows),
    );
    let value = if editing { app.input.as_str() } else { "" };
    let cursor = if editing { app.cursor } else { 0 };
    let (lines, (caret_row, caret_column)) =
        editor_rows(value, cursor, input.width.saturating_sub(1) as usize);
    let top = caret_row.saturating_sub(input.height.saturating_sub(1) as usize);
    if value.is_empty() {
        text(
            frame,
            input,
            if app.composer_is_request() || !editing {
                "Ask a question or describe what you want to do..."
            } else if app.editor_field == EditorField::NextAction {
                "Describe the next action..."
            } else {
                "Describe the optional task goal..."
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
    if editing
        && app.overlay.is_none()
        && app.command_popup.is_none()
        && app.composer_menu.is_none()
        && app.confirmation.is_none()
        && input.width > 0
        && input.height > 0
    {
        frame.set_cursor_position((
            input.x + caret_column.min(input.width.saturating_sub(1) as usize) as u16,
            input.y + (caret_row - top) as u16,
        ));
    }
    // While editing, the footer carries the composer keys; only the input
    // limit warning stays inside the panel.
    let hint = if app.input_truncated && editing {
        "Input limit reached; draft retained."
    } else if editing {
        ""
    } else {
        "Type a request / / Commands / @ Files"
    };
    text(
        frame,
        row(content, content.height.saturating_sub(hint_rows), hint_rows),
        hint,
        paint.muted,
    );
}

fn editor(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let area = centered(area, 86, 16);
    frame.render_widget(Clear, area);
    let title = format!(
        " Edit {} / {} ",
        app.editor_label(),
        App::partner_label(app.lead())
    );
    let block = panel(&title, config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        row(content, 0, 2),
        match app.editor_field {
            EditorField::Request => {
                "Enter sends this request. Commands and file changes need separate approval."
            }
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
            ConfirmationKind::ResetTask => " Reset local task ",
            ConfirmationKind::ReplaceWithInit => " Prepare /init request ",
            ConfirmationKind::ReturnToEevee => " Return to Eevee ",
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
        ConfirmationKind::ResetTask => (
            "Fresh local task? Save old task + draft in /tasks. No files, history or credentials erased. Active work must finish/cancel first.",
            "Stay",
            "Save and reset",
        ),
        ConfirmationKind::ReplaceWithInit => (
            "Replace Request draft with /init proposal? Other drafts stay. Select Espeon / Plan, clear write paths. No send/write; review then Run.",
            "Keep draft",
            "Replace with /init",
        ),
        ConfirmationKind::ReturnToEevee => (
            "Return to intake? Cancel provider work and wait for it to finish first. Drafts and evidence stay; cancellation does not roll back effects.",
            "Stay",
            "Cancel and wait",
        ),
        ConfirmationKind::DeleteTasks => unreachable!("separate deletion preview"),
    };
    text(frame, row(content, 0, 3), question, paint.surface);
    text(
        frame,
        row(content, 3, 1),
        &format!("[n] {keep}"),
        if !confirm.destructive {
            paint.selected
        } else {
            paint.surface
        },
    );
    text(
        frame,
        row(content, 4, 1),
        &format!("[y] {discard}"),
        if confirm.destructive {
            paint.selected
        } else {
            paint.surface
        },
    );
    text(
        frame,
        row(content, 6, 2),
        "y Confirm / n Stay / Tab Choose / Enter Select",
        paint.muted,
    );
    app.confirmation_reviewed = content.height >= 8
        && content.width >= 44
        && Paragraph::new(question)
            .wrap(Wrap { trim: false })
            .line_count(content.width.max(1))
            <= 3;
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
        "Delete {} tasks/drafts and listed evidence/task memories? Shared memories, sources, exports and provider sessions stay. Not secure erasure. Page {}/{} (Home/End).",
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
        Provider(&'a str, crate::providers::Agent),
        Memory(&'a crate::workspace::memory::Record),
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
        "[n] Stay / keep all data",
        if !selected {
            paint.selected
        } else {
            paint.surface
        },
    );
    text(
        frame,
        row(content, content.height.saturating_sub(3), 1),
        "[y] Delete the listed local tasks",
        if selected {
            paint.selected
        } else {
            paint.surface
        },
    );
    text(
        frame,
        row(content, content.height.saturating_sub(2), 2),
        "y Delete / n Keep / Tab Choose / Enter Select\nPgUp/Dn IDs / Home/End Pages / Esc Stay",
        paint.muted,
    );
    app.confirmation_reviewed = content.height >= 8
        && content.width >= 44
        && Paragraph::new(question)
            .wrap(Wrap { trim: false })
            .line_count(content.width.max(1))
            <= 4;
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
mod tests;
