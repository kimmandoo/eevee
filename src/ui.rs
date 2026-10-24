mod agents;
mod artifacts;
pub mod artwork;
mod chips;
mod composer;
mod cooperation;
mod execution;
mod formations;
mod handoff;
mod held;
mod memory;
mod moves;
mod planning;
mod records;
mod sources;
mod tool_catalog;
mod tool_plans;
mod updates;
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
    i18n::{Language, Message, Template},
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
    language: Language,
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
                language: config.language,
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
            language: config.language,
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

    fn hint(&self, message: Message) -> &'static str {
        self.language.text(message)
    }

    fn format(&self, template: Template, args: &[(&str, &str)]) -> String {
        self.language
            .format(template, args)
            .unwrap_or_else(|error| format!("Localization error: {error}"))
    }

    fn format_values(
        &self,
        template: Template,
        args: &[(&str, crate::i18n::Argument<'_>)],
    ) -> String {
        self.language
            .format_values(template, args)
            .unwrap_or_else(|error| format!("Localization error: {error}"))
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

/// Temporary sheets leave the real status header and safe-return footer visible.
fn sheet(area: Rect, width: u16, height: u16) -> Rect {
    centered(area, width, height.min(area.height.saturating_sub(4)))
}

fn drawer(area: Rect) -> Rect {
    let width = 46.min(area.width.saturating_sub(2));
    Rect::new(
        area.right().saturating_sub(width + 1),
        area.y + 2,
        width,
        area.height.saturating_sub(4),
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
        sheet(
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
        || app.cooperation_ui.release.is_some()
        || app.cooperation.cancel_review.is_some()
        || app.cooperation.journal_discard.is_some()
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
            let layout = shell(area, app.linear, app.effective_focus());
            if let Some(rail) = layout.rail {
                targets.push((app.form(), companion_portrait(rail)));
            }
            cooperation::sprite_targets(app, layout.workspace, targets);
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

fn evolution_popup_area(area: Rect, app: &App) -> Option<Rect> {
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
fn occluded(area: Rect, app: &App, target: Rect) -> bool {
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

pub fn render(
    frame: &mut Frame,
    app: &mut App,
    config: &Config,
    artwork: &mut Artwork,
    graphics_status: &str,
) {
    VERBOSE.set(app.verbose);
    app.planning.set_response_body(Rect::default());
    let area = frame.area();
    app.confirmation_reviewed = false;
    if let Some(review) = &mut app.cooperation_ui.release {
        review.reviewed = false;
    }
    if let Some(review) = &mut app.cooperation.cancel_review {
        review.reviewed = false;
    }
    if let Some(review) = &mut app.cooperation.journal_discard {
        review.reviewed = false;
    }
    if let Some(confirmation) = &mut app.sources.confirmation {
        confirmation.reviewed = false;
    }
    let form = &FORMS[app.form().min(8)];
    let paint = Paint::new(config, form, app.form().min(8));
    frame.render_widget(Block::default().style(paint.base), area);
    if area.width < 60 || area.height < 18 {
        let review_area = row(area, 0, area.height.saturating_sub(1));
        app.agents.affirmative = false;
        app.agents.reviewed = false;
        app.images_reviewed = false;
        text(
            frame,
            area,
            paint.hint(Message::CoreTerminalTooSmall),
            paint.accent,
        );
        if app.confirmation.is_some() {
            confirmation(frame, review_area, app, config, &paint);
        } else if app.overlay == Some(Overlay::Execution) && app.execution.confirmation.is_some() {
            execution::render_small_confirmation(frame, review_area, &mut app.execution, config);
        } else if app.overlay == Some(Overlay::Planning) {
            planning::render_small(frame, review_area, &mut app.planning, &paint);
        } else if app.overlay == Some(Overlay::Handoff) {
            handoff::render_small(frame, review_area, &mut app.handoff, &paint);
        } else if app.overlay == Some(Overlay::Memory) {
            memory::render_small(frame, review_area, &mut app.memory, &paint);
        } else if app.overlay == Some(Overlay::Workflows) {
            workflows::render(frame, review_area, app, config, &paint);
        } else if app.overlay == Some(Overlay::Moves) {
            moves::render(frame, review_area, app, config, &paint);
        } else if app.overlay == Some(Overlay::Records) {
            records::render_small(frame, review_area, app, &paint);
        } else if app.overlay == Some(Overlay::PartyBoard) {
            cooperation::board(frame, review_area, app, config, &paint);
        } else if app.overlay == Some(Overlay::Held) {
            held::render(frame, review_area, app, config, &paint);
        }
        if app.cooperation_ui.release.is_some() {
            cooperation::release(frame, area, app, config, &paint);
        }
        if app.cooperation.cancel_review.is_some() {
            cooperation::cancellation(frame, area, app, config, &paint);
        }
        if app.cooperation.journal_discard.is_some() {
            cooperation::journal_discard(frame, area, app, config, &paint);
        }
        text(
            frame,
            row(area, area.height.saturating_sub(1), 1),
            app.storage_label(),
            paint.muted,
        );
        return;
    }
    let shell = shell(area, app.linear, app.effective_focus());
    header(frame, shell.header, app, config, &paint);
    if app.cooperation.journal_discard.is_some() {
        cooperation::journal_discard(frame, area, app, config, &paint);
        return;
    }
    if app.cooperation.cancel_review.is_some() {
        cooperation::cancellation(frame, area, app, config, &paint);
        return;
    }
    if app.cooperation_ui.release.is_some() {
        cooperation::release(frame, area, app, config, &paint);
        return;
    }
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
        let workspace_area = cooperation::header(frame, shell.workspace, app, &paint);
        let workspace_area = cooperation::strip(frame, workspace_area, app, &paint, artwork);
        let workspace_area = if matches!(app.overlay, Some(Overlay::Tasks | Overlay::Forms)) {
            workspace_area
        } else {
            suggestion_strip(frame, workspace_area, app, &paint)
        };
        let workspace_area = held::strip(frame, workspace_area, app, &paint);
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
    if let Some(popup) = evolution_popup_area(area, app) {
        cooperation::evolution_popup(frame, popup, app, config, &paint);
    }
    if let Some(menu) = composer_menu_area(area, app) {
        composer::render(frame, menu, app, config, &paint);
    }
    if let Some(overlay) = app.overlay {
        match overlay {
            Overlay::Forms => forms(frame, area, app, config, &paint, artwork),
            Overlay::Help => help(frame, area, app, config, &paint),
            Overlay::EvolutionPreview => evolution_preview(frame, area, app, config, &paint),
            Overlay::Preview => preview(frame, area, app, config, &paint),
            Overlay::Tasks => tasks(frame, area, app, config, &paint),
            Overlay::Criteria => criteria(frame, area, app, config, &paint),
            Overlay::Repository => repository(frame, area, app, config, &paint),
            Overlay::Sources => sources::render(frame, area, app, config, &paint),
            Overlay::Execution => {
                let queue = app.approval_position().map(|(position, total)| {
                    format!(
                        " {position} of {total} / {} ",
                        cooperation::approval_owner(app)
                    )
                });
                let editing = app.write_owner().map(|claim| {
                    format!(
                        "{} is editing",
                        store::Attribution::label(Some(&claim.owner))
                    )
                });
                execution::render(
                    frame,
                    area,
                    &mut app.execution,
                    config,
                    queue.as_deref(),
                    editing.as_deref(),
                );
            }
            Overlay::Agents => agents::render(frame, area, app, config, &paint),
            Overlay::Planning => planning::render(frame, area, app, config, &paint),
            Overlay::Handoff => handoff::render(frame, area, app, config, &paint),
            Overlay::Workflows => workflows::render(frame, area, app, config, &paint),
            Overlay::Memory => memory::render(frame, area, &mut app.memory, config, &paint),
            Overlay::Moves => moves::render(frame, area, app, config, &paint),
            Overlay::Records => records::render(frame, area, app, config, &paint),
            Overlay::Held => held::render(frame, area, app, config, &paint),
            Overlay::Formations => formations::render(frame, area, app, config, &paint),
            Overlay::Updates => updates::render(frame, area, app, config, &paint),
            Overlay::Artifacts => artifacts::render(frame, area, &mut app.artifacts, config),
            Overlay::ToolCatalog => tool_catalog::render(frame, area, app, config, &paint),
            Overlay::ToolPlans => tool_plans::render(frame, area, app, config, &paint),
            Overlay::Receipts => app.planning.receipts.render(frame, area, config.language),
            Overlay::Party => party(frame, area, app, config, &paint),
            Overlay::PartyBoard => cooperation::board(frame, area, app, config, &paint),
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

/// Provider, model and effort at a glance; `/verbose` adds observation caveats.
fn selection_text(state: &crate::app::planning::State) -> String {
    if VERBOSE.get() {
        state.selection_label()
    } else {
        format!("{} · {}", state.agent.label(), state.selection.summary())
    }
}

fn activity_marker(frame: u8, config: &Config, motion_enabled: bool) -> &'static str {
    if !motion_enabled || config.no_animation {
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
    let project = display_identity(project);
    let mode = paint.hint(if app.lead().auto_form {
        Message::CoreEvolutionAuto
    } else {
        Message::CoreEvolutionManual
    });
    let brand = if area.width >= 90 { " EEVEE " } else { "" };
    let usable_width = usize::from(area.width).saturating_sub(brand.len());
    {
        use crate::providers::Agent;
        let route = match app.planning.agent {
            Agent::Codex => "Codex/CLI",
            Agent::ChatGPT => "OpenAI/OAuth",
            Agent::Claude => "Claude/SDK",
            Agent::OpenCode => "OpenCode/ACP",
            Agent::Gemini => "Gemini/ACP",
            Agent::Devin => "Devin/ACP",
            Agent::OpenAI => "OpenAI/API",
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
        let identity_budget = usable_width
            .saturating_sub(Span::raw(route).width() + effort.width() + model_hint_width + 6);
        let suffix = format!(" ({}) [{mode}]", FORMS[app.form()].name);
        let name = App::partner_label(app.lead());
        let identity = format!(
            "{}{suffix}",
            clipped(
                &name,
                identity_budget.saturating_sub(Span::raw(suffix.as_str()).width())
            )
        );
        let spare = usable_width.saturating_sub(
            Span::raw(identity.as_str()).width()
                + Span::raw(route).width()
                + effort.width()
                + model_hint_width
                + 6,
        );
        let context = if spare >= 7 {
            format!("{identity} · {} · {route} · ", clipped(&project, spare - 3))
        } else {
            format!("{identity} · {route} · ")
        };
        let context = Span::styled(context, paint.muted);
        let model_width = usable_width.saturating_sub(context.width() + effort.width());
        let model = Span::styled(
            clipped(
                app.planning.selection.model.as_deref().unwrap_or("default"),
                model_width,
            ),
            paint.accent,
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(brand, paint.selected),
                context,
                model,
                effort,
            ])),
            row(area, 0, 1),
        );
    }
    let waiting = app.approval_count() > 0;
    let activity = if waiting {
        paint.hint(Message::CoreWaitingForUser)
    } else if app.planning.cancelling {
        "Cancelling provider / /response Status"
    } else if let Some(preview) = app.planning.preview.as_ref() {
        if preview.blockers.is_empty() {
            "Request prepared / /approvals Details"
        } else {
            "Request blocked / /approvals Details"
        }
    } else if app.planning.preference_error().is_some() {
        "Model preference not saved / /storage-retry"
    } else if let Some(activity) = app.planning.activity_label() {
        activity
    } else if app.planning.outcome_unknown() {
        if area.width < 90 {
            "Outcome unknown / /response"
        } else {
            "Provider outcome unknown / /response Inspect"
        }
    } else {
        app.execution.activity_label()
    };
    let policy = match app.planning.explicit_operation() {
        Some(crate::providers::Operation::Plan) => paint.hint(Message::CorePolicyReadonly),
        Some(crate::providers::Operation::Review) => paint.hint(Message::CorePolicyReview),
        _ => "",
    };
    let status = format!(
        "{policy}{}{}  /  {} {activity}",
        if policy.is_empty() { "" } else { " · " },
        app.storage_label(),
        if !waiting && app.planning.activity_label().is_some() {
            activity_marker(app.activity_frame, config, app.motion_enabled())
        } else {
            ""
        }
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
    let operation = if app.planning.busy() {
        app.planning.operation
    } else {
        app.planning
            .explicit_operation()
            .unwrap_or(crate::providers::Operation::Implement)
    };
    let chip = format!(
        "[{}] ",
        paint.hint(match operation {
            crate::providers::Operation::Plan | crate::providers::Operation::Review =>
                Message::CoreCapabilityReadonly,
            crate::providers::Operation::Implement => Message::CoreCapabilityReviewedCoding,
        })
    );
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
    if app.planning.request.is_some() || (app.lead().auto_form && app.form() != 0) {
        return None;
    }
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
    if app.planning.has_current_file_output() && !app.carry_over_explicit() {
        return area;
    }
    if let Some(carry_over) = app.carry_over() {
        // The precomputed ASCII summary fits the minimum workspace width.
        // Give this transient record the existing strip slot, not a second row.
        frame.render_widget(
            Paragraph::new(carry_over.summary.as_str()).style(paint.accent),
            row(area, 0, 1),
        );
        return row(area, 1, area.height.saturating_sub(1));
    }
    // A deliberate saved item takes the advisory slot; carry-over stays inspectable.
    if app.held.target.is_some() && !app.held.dismissed {
        return area;
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
    let mut lines = Vec::with_capacity(8);
    lines.push(heading);
    if app.text_density() == store::TextDensity::Comfortable && content.height >= 9 {
        lines.push(Line::from(""));
    }
    lines.extend([
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
    ]);
    if app
        .drafts
        .iter()
        .any(|draft| draft.field != EditorField::Request)
    {
        lines.push(Line::styled(
            paint.hint(Message::CoreRetainedTaskFact),
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
            paint.hint(Message::CoreCarryOverControl)
        } else if focused {
            paint.hint(Message::CoreFormControl)
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
    let project = display_identity(project);
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
            "TASK\n{task} / {}\nUpdated: {updated}\nCriteria: {}/{} met\nRetained drafts: {}\nComplete means user-reviewed criteria, not verified command evidence.\n\nPROJECT DIRECTORY\n{}\n\nSTORAGE\n{}\nRevision {} / committed {}\nDirectory: {}\n{}\nRecovery: /storage-retry; --export-data FILE exports readable local records, not provider history.\nQuota: {quota} bytes. Last sample: {sampled} live SQLite, retained output and committed reservations; not physical database size.\nRetention: archived at least {retention} days; explicit purge only, no automatic eviction.\nSecret detection is best-effort. Volatile editor text may remain unmasked.\nArchive is reversible. Deletion affects selected local records and owned artifacts, not repository files, exports or provider sessions. Not forensic secure erasure.\n\nEXECUTION\n/provider opens connection selection. /run inspects local command evidence; /execute reviews a prepared command. /handoff opens portable handoff. Cancellation does not roll back files; approval is not a sandbox.\n\nARTWORK\n{}\n{}\n{}\n\nUp/Down scroll / Esc back",
            if app.complete { "Complete" } else { "Open" },
            app.criteria
                .iter()
                .filter(|criterion| criterion.met)
                .count(),
            app.criteria.len(),
            app.drafts.len(),
            app.cwd.escape_debug(),
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
    let block = panel(
        paint.hint(Message::CoreFormPickerTitle),
        config,
        paint,
        true,
    );
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
            paint.hint(Message::CoreReturnIntakeSelected)
        } else {
            paint.hint(Message::CoreReturnIntake)
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
                    "{} {}{}{} {}",
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
                    },
                    records::compact_marker(app.record_form_marker(index), &cell_paint),
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
        form_details(frame, detail, selected, app, config, artwork);
    } else {
        let list_width = content.width * 2 / 3;
        let visible = if content.height < 20 { 3 } else { 9 };
        let start = selected.saturating_sub(visible / 2).min(9 - visible);
        for (row_index, index) in (start..start + visible).enumerate() {
            let form = &FORMS[index];
            let item = Rect::new(content.x, content.y + 2 + row_index as u16, list_width, 1);
            let label = format!(
                "{} {} {:<9}{}{} {}",
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
                },
                records::compact_marker(app.record_form_marker(index), paint),
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
            2 + visible as u16,
            content.height.saturating_sub(2 + visible as u16),
        );
        let role = format!("{} / {}", FORMS[selected].name, FORMS[selected].role);
        let value = format!(
            "{}\n{}\n{}",
            clipped(&role, info.width as usize),
            clipped(
                &records::compact_marker(app.record_form_marker(selected), paint),
                info.width as usize,
            ),
            picker_moves(app, selected, info.width, paint)
        );
        text(frame, info, &value, paint.muted);
    }
}

fn form_details(
    frame: &mut Frame,
    area: Rect,
    index: usize,
    app: &App,
    config: &Config,
    artwork: &Artwork,
) {
    let form = &FORMS[index];
    let paint = Paint::new(config, form, index);
    let value = format!(
        "{} / {}\n{}\n{}\n{}\n\n{}\n\n{}\n{}\n{}",
        form.name,
        form.element,
        form.role,
        picker_moves(app, index, area.width, &paint),
        records::marker(app.record_form_marker(index), &paint),
        form.evidence,
        match index {
            0 => "Task cards and resumption: available",
            1 => "Source ranges/context basket: /files",
            2 => "Approved local commands/evidence: /run",
            3 => "Reviewed selected-file implementation: /provider / /role",
            4 => "Reviewed multi-agent planning: /provider",
            _ => "Provider specialist workflow: unavailable",
        },
        artwork.status(index),
        paint.hint(Message::CoreRolePreviewControl)
    );
    frame.render_widget(
        Paragraph::new(value)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn picker_moves(app: &App, form: usize, width: u16, paint: &Paint) -> String {
    if app.task_id <= 0 {
        return paint.hint(Message::CoreMovesUnsaved).into();
    }
    match app.picker_move_names(form) {
        None => paint.hint(Message::CoreMovesLoading).into(),
        Some(Err(_)) => paint.hint(Message::CoreMovesUnavailable).into(),
        Some(Ok(names)) if names.is_empty() => paint.hint(Message::CoreMovesEmpty).into(),
        Some(Ok(names)) => {
            let mut value = paint.hint(Message::CoreMovesHeading).to_owned();
            for name in names {
                value.push_str("- ");
                value.push_str(&clipped(name, width.saturating_sub(2) as usize));
                value.push('\n');
            }
            value
        }
    }
}

fn repository(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = sheet(area, 106, area.height);
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
    app.set_scroll_viewport(ScrollSurface::Preview, max, details.height);
}

fn tasks(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = sheet(area, 88, 28);
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
            paint.hint(Message::CoreTasksEmpty),
            paint.surface,
        );
        app.set_scroll_viewport(ScrollSurface::Preview, 0, 0);
        return;
    }
    let selected = app.task_selection.min(count - 1);
    let stride = if app.text_density() == store::TextDensity::Comfortable && content.height >= 10 {
        2
    } else {
        1
    };
    let visible = count.min(((content.height / 3) / stride).clamp(1, 6) as usize);
    let list_rows = (visible as u16 * stride).saturating_sub(stride - 1);
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
            row(content, (index - start) as u16 * stride, 1),
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
        row(content, list_rows, 1),
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
        list_rows + 2,
        content.height.saturating_sub(list_rows + 2),
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
    ];
    if task.id == app.task_id
        && let Some(line) = suggestion_line(app, paint)
    {
        lines.insert(1, line);
    }
    lines.push(Line::styled(
        paint.hint(Message::RecordsRoutes),
        paint.muted,
    ));
    records::task_routes(&app.records, task.id, paint, &mut lines);
    lines.push(Line::styled("GOAL", paint.muted));
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
    app.set_scroll_viewport(ScrollSurface::Preview, max, details.height);
}

fn criteria(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = sheet(area, 88, 26);
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
        app.set_scroll_viewport(ScrollSurface::Preview, 0, 0);
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
        paint.hint(Message::CoreCriterionDetailControl),
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
            paint.hint(Message::CoreCriterionCompleted)
        } else {
            paint.hint(Message::CoreCriterionReviewControl)
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
    app.set_scroll_viewport(ScrollSurface::Preview, max, details.height);
}

fn nickname(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let Some(editor) = &app.nickname_editor else {
        return;
    };
    let area = sheet(area, 58, 13);
    frame.render_widget(Clear, area);
    let block = panel(paint.hint(Message::CoreNicknameTitle), config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        row(content, 0, 2),
        paint.hint(Message::CoreNicknamePrivacy),
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
            paint.hint(Message::CoreNicknameSaving)
        } else {
            paint.hint(Message::CoreNicknameSaveControl)
        },
        paint.muted,
    );
}

/// Party roster (PT04A/PT05A): every partner with its form, a derived status
/// and the task it is engaged with. Status is recomputed from live
/// request/approval state each frame; Enter/`l` fronts the highlighted partner.
fn party(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = sheet(area, 88, 24);
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
            paint.hint(Message::CorePartyControlFull)
        } else {
            paint.hint(Message::CorePartyControlAdd)
        },
        paint.muted,
    );
    text(
        frame,
        row(content, partners.len() as u16 + 4, 1),
        paint.hint(Message::CoreReleasePartnerControl),
        paint.muted,
    );
    app.set_scroll_viewport(ScrollSurface::Preview, 0, 0);
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
    let area = sheet(area, 78, 24);
    frame.render_widget(Clear, area);
    let block = panel(paint.hint(Message::CoreHelpTitle), config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let value = paint.hint(Message::CoreHelpContent);
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

fn evolution_preview(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = sheet(area, 78, 20);
    frame.render_widget(Clear, area);
    let block = panel(
        paint.hint(Message::CoreEvolutionPreviewTitle),
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let operation = |operation| match operation {
        crate::providers::Operation::Plan => "Plan",
        crate::providers::Operation::Implement => "Implement",
        crate::providers::Operation::Review => "Review",
    };
    let value = if let Some(preview) = app.evolution_preview() {
        [
            paint.format(
                Template::CoreEvolutionPreviewForm,
                &[
                    ("form", FORMS[preview.selection.form].name),
                    (
                        "mode",
                        paint.hint(if preview.automatic {
                            Message::CoreEvolutionAuto
                        } else {
                            Message::CoreEvolutionManual
                        }),
                    ),
                ],
            ),
            paint.format(
                Template::CoreEvolutionPreviewOperation,
                &[("operation", operation(preview.selection.operation))],
            ),
            paint.format(
                Template::CoreEvolutionPreviewReason,
                &[("reason", paint.hint(preview.selection.reason))],
            ),
            paint.format(
                Template::CoreEvolutionPreviewPolicy,
                &[(
                    "policy",
                    preview
                        .restriction
                        .map(operation)
                        .unwrap_or_else(|| paint.hint(Message::CoreEvolutionPreviewNone)),
                )],
            ),
            String::new(),
            paint.hint(Message::CoreEvolutionPreviewGuard).into(),
            String::new(),
            paint.hint(Message::CoreEvolutionPreviewControls).into(),
        ]
        .join("\n")
    } else {
        paint.hint(Message::CoreEvolutionPreviewStale).into()
    };
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

fn preview(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = sheet(area, 82, 24);
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
        "GOAL\n{}\n\nROLE\n{} / {}\n\nEXPECTED ARTIFACT\n{}\n\nCONTEXT\n{} captured references loaded. /files opens envelope preview and source revalidation. Provider-added history or reads are not governed by this basket.\n\nEXECUTION / CONTROL\nThis local preview sends nothing. /provider selects a connection; /session-new opens exact session review. /run inspects local command evidence; /execute reviews a prepared command.\nThis preview performs no operation.\n\nEsc returns to the invoking control.",
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
        paint.hint(Message::CoreSavingExit)
    } else if app.confirmation.is_some() {
        paint.hint(Message::CoreConfirmationControl)
    } else if app.reset_after_save {
        paint.hint(Message::CoreSavingReset)
    } else if app.transitioning {
        paint.hint(Message::CoreApplyingTask)
    } else if app.command_popup.is_some() {
        paint.hint(Message::CoreCommandPickerControl)
    } else if app.composer_menu.is_some() && app.overlay.is_none() {
        paint.hint(Message::CoreFilePickerControl)
    } else if app.approval_count() > 0 && app.approval_shortcut_available() {
        if app.own_approval_pending()
            && (app.composer_is_request() || app.effective_focus() == Pane::Workspace)
        {
            paint.hint(Message::CorePendingRequestControl)
        } else {
            paint.hint(Message::CorePendingReviewControl)
        }
    } else if app.editing && app.overlay.is_none() {
        if app.composer_is_request() && app.planning.busy() {
            paint.hint(Message::CoreRequestBusyControl)
        } else if app.composer_is_request() && app.response_on_home() {
            paint.hint(Message::CoreResponseRequestControl)
        } else if app.composer_is_request() {
            paint.hint(Message::CoreRequestControl)
        } else {
            paint.hint(Message::CoreTaskFactControl)
        }
    } else {
        match app.overlay {
            Some(Overlay::Forms) => paint.hint(Message::CoreFormsControl),
            Some(Overlay::Criteria) if area.width < 70 => {
                paint.hint(Message::CoreCriteriaControlCompact)
            }
            Some(Overlay::Criteria) => paint.hint(Message::CoreCriteriaControl),
            Some(Overlay::Tasks) if area.width < 70 => paint.hint(Message::CoreTasksControlCompact),
            Some(Overlay::Tasks) => paint.hint(Message::CoreTasksControl),
            Some(Overlay::Planning)
                if app.planning.request.is_some() || app.planning.preview.is_some() =>
            {
                paint.hint(Message::CoreApprovalsControl)
            }
            Some(Overlay::Planning) if app.planning.activity_label().is_some() => {
                paint.hint(Message::CoreRequestActiveControl)
            }
            Some(Overlay::Sources) if app.sources.editor.is_some() => {
                paint.hint(Message::CoreSourceFilterControl)
            }
            Some(
                Overlay::Execution
                | Overlay::Agents
                | Overlay::Planning
                | Overlay::Handoff
                | Overlay::Workflows
                | Overlay::Memory
                | Overlay::Held
                | Overlay::Updates
                | Overlay::PartyBoard,
            ) => paint.hint(Message::CoreOverlayControl),
            Some(_) => paint.hint(Message::CoreOverlayScrollControl),
            None => paint.hint(Message::CoreHomeControl),
        }
    };
    let queued_notice = app
        .approval_response_queued()
        .then(|| paint.hint(Message::CoreResponseQueued));
    let notice = if !app.notice.is_empty() {
        Some(
            app.motion_notice()
                .or_else(|| app.density_notice())
                .or_else(|| app.formation_notice())
                .map_or_else(
                    || brief(&app.notice),
                    |message| Cow::Borrowed(paint.hint(message)),
                ),
        )
    } else {
        queued_notice.map(Cow::Borrowed)
    };
    if let Some(notice) = notice {
        text(
            frame,
            row(area, 0, 1),
            &clipped(&notice, area.width as usize),
            paint.accent,
        );
        text(frame, row(area, 1, 1), value, paint.base);
    } else if app.scroll() > 0 && !app.editing {
        text(
            frame,
            area,
            &paint.format(
                Template::CoreDetailPosition,
                &[("value", value), ("line", &(app.scroll() + 1).to_string())],
            ),
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
        let newline = matches!(grapheme, "\n" | "\r\n");
        let size = if newline {
            0
        } else {
            Span::raw(grapheme).width()
        };
        if !newline && column + size > width {
            lines.push(String::new());
            column = 0;
        }
        if index == cursor {
            caret = (lines.len() - 1, column);
        }
        if newline {
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
    let area = sheet(area, 84, 24);
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
    app.set_scroll_viewport(ScrollSurface::Preview, maximum, details.height);
    app.images_reviewed = content.height >= summary_rows + 4;
    let choices = [
        paint.hint(Message::CoreBack),
        if app.images_busy {
            paint.hint(Message::CoreCancelDownload)
        } else {
            paint.hint(Message::CoreDownloadMissing)
        },
        paint.hint(Message::CoreReloadLocal),
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
        paint.hint(Message::CoreImagesControl),
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
        paint.hint(Message::CoreRequestTitle)
    } else if editing && app.editor_field == EditorField::NextAction {
        paint.hint(Message::CoreNextActionTitle)
    } else {
        paint.hint(Message::CoreTaskFactTitle)
    };
    let attachment_status = if app.sources.store_failed {
        paint.hint(Message::CoreAttachmentSaveFailed)
    } else if app.sources.persistence_pending() {
        paint.hint(Message::CoreAttachmentSaving)
    } else {
        ""
    };
    let remembered = app.records_pairing_badge(
        app.agents
            .profiles
            .get(app.planning.agent.index())
            .is_some_and(|profile| profile.blockers.is_empty()),
    );
    let partner = app.lead_label();
    let title = paint.format(
        Template::CoreComposerTitle,
        &[
            ("title", title),
            ("partner", &partner),
            ("count", &app.sources.basket.len().to_string()),
            ("status", attachment_status),
            (
                "pairing",
                remembered.map(|message| paint.hint(message)).unwrap_or(""),
            ),
        ],
    );
    let mut block = panel(&title, config, paint, editing && app.overlay.is_none());
    let pairing_offer;
    if app.overlay.is_none() && app.records.failed.is_some() {
        block = block.title_bottom(Line::styled(
            paint.hint(Message::RecordsWriteFailed),
            paint.accent,
        ));
    } else if app.overlay.is_none()
        && app.confirmation.is_none()
        && !app.planning_any_busy()
        && !app.execution_any_busy()
        && let Some(activity) = app.record_activity()
    {
        block = block.title_bottom(Line::styled(
            clipped(&activity.detail, area.width.saturating_sub(4) as usize),
            paint.muted,
        ));
    } else if app.overlay.is_none()
        && app.confirmation.is_none()
        && !app.planning_any_busy()
        && !app.execution_any_busy()
        && app.has_pairing_offer()
    {
        pairing_offer = paint.format(Template::RecordsPairingUseOffer, &[("partner", &partner)]);
        block = block.title_bottom(Line::styled(
            clipped(&pairing_offer, area.width.saturating_sub(4) as usize),
            paint.accent,
        ));
    } else if editing {
        block = block.title_bottom(Line::styled(
            paint.hint(Message::CoreComposerDraftControl),
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
    let attachment_rows = if app.sources.basket.is_empty() {
        0
    } else {
        content.height.saturating_sub(hint_rows + 1).min(2)
    };
    if attachment_rows != 0 {
        chips::render(frame, row(content, 0, attachment_rows), app, paint);
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
                paint.hint(Message::CoreRequestPlaceholder)
            } else if app.editor_field == EditorField::NextAction {
                paint.hint(Message::CoreNextActionPlaceholder)
            } else {
                paint.hint(Message::CoreGoalPlaceholder)
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
    if value.is_empty()
        && input.height > 1
        && let Some(hint) = app.record_habit_hint()
    {
        text(
            frame,
            row(input, input.height - 1, 1),
            paint.hint(hint),
            paint.muted,
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
        paint.hint(Message::CoreInputLimit)
    } else if editing {
        ""
    } else {
        paint.hint(Message::CoreComposerIdleControl)
    };
    text(
        frame,
        row(content, content.height.saturating_sub(hint_rows), hint_rows),
        hint,
        paint.muted,
    );
}

fn editor(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    // A modal must not erase task/storage status or the refusal/exit footer.
    let area = row(area, 2, area.height.saturating_sub(4));
    let area = centered(area, 86, 16);
    frame.render_widget(Clear, area);
    let title = paint.format(
        Template::CoreEditorTitle,
        &[
            ("field", app.editor_label()),
            ("partner", &App::partner_label(app.lead())),
        ],
    );
    let block = panel(&title, config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(
        frame,
        row(content, 0, 2),
        match app.editor_field {
            EditorField::Request => paint.hint(Message::CoreEditorRequestHelp),
            EditorField::Criteria => paint.hint(Message::CoreEditorCriteriaHelp),
            EditorField::Goal => paint.hint(Message::CoreEditorGoalHelp),
            EditorField::NextAction => paint.hint(Message::CoreEditorNextActionHelp),
            EditorField::Command => paint.hint(Message::CoreEditorCommandHelp),
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
        paint.hint(Message::CorePasteLimit)
    } else {
        ""
    };
    text(
        frame,
        row(content, content.height.saturating_sub(2), 2),
        &paint.format(
            Template::CoreEditorStatus,
            &[
                ("count", &app.input.len().to_string()),
                ("warning", warning),
                ("controls", paint.hint(Message::CoreEditorApplyControl)),
            ],
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
    app.set_scroll_viewport(surface, max, area.height);
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
        sheet(area, 62, 10)
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
        ConfirmationKind::Quit if app.moves.learning.is_some() => (
            "Exit and discard the unsaved move name/learning choice? Task drafts are saved; only committed moves survive.",
            "Keep editing",
            "Discard move draft and quit",
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
        paint.hint(Message::CoreConfirmationControl),
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
    let area = sheet(area, 78, 26);
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
    app.set_scroll_viewport(ScrollSurface::Preview, max, details.height);
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
        paint.hint(Message::CoreDeleteControl),
        paint.muted,
    );
    app.confirmation_reviewed = content.height >= 8
        && content.width >= 44
        && Paragraph::new(question)
            .wrap(Wrap { trim: false })
            .line_count(content.width.max(1))
            <= 4;
}

/// Escape only display-unsafe identities; ordinary Unicode paths stay borrowed.
fn display_identity(value: &str) -> Cow<'_, str> {
    if value.chars().any(|ch| {
        ch.is_control()
            || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    }) {
        Cow::Owned(value.escape_debug().to_string())
    } else {
        Cow::Borrowed(value)
    }
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
