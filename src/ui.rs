mod agents;
mod artifacts;
pub mod artwork;
mod bag;
mod chips;
mod chrome;
mod composer;
mod cooperation;
mod execution;
mod external_editor;
mod formations;
mod handoff;
mod held;
mod layout;
mod mcp;
mod memory;
mod meter;
mod moves;
mod parked_drafts;
mod planning;
mod records;
mod retained;
mod rules;
mod sources;
mod tool_catalog;
mod tool_plans;
mod updates;
mod workflows;
mod workspace;

use chrome::*;
use layout::*;
use std::{borrow::Cow, cell::Cell};
use workspace::*;

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    app::{App, ConfirmationKind, Overlay, Pane, ScrollSurface},
    config::{ColorMode, Config, Theme},
    domain::{FORMS, Form},
    messages::{Message, Template},
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
    /// Live-output effects for this frame: `Some((ascii, frame))` while real
    /// provider work streams with motion enabled, otherwise `None`.
    static LIVE_FX: Cell<Option<(bool, u8)>> = const { Cell::new(None) };
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
    color: ColorMode,
    ascii: bool,
    base: Style,
    surface: Style,
    muted: Style,
    border: Style,
    accent: Style,
    selected: Style,
}

impl Paint {
    fn new(config: &Config, form: &Form, _index: usize) -> Self {
        if config.color == ColorMode::Mono {
            let base = Style::default().fg(Color::White).bg(Color::Black);
            return Self {
                color: config.color,
                ascii: config.ascii,
                base,
                surface: base,
                muted: base,
                border: base,
                accent: base.add_modifier(Modifier::BOLD),
                selected: base.add_modifier(Modifier::REVERSED | Modifier::BOLD),
            };
        }
        // Both theme preferences share the sprite's black canvas. Keep the
        // light preference's warmer text without darkening its foregrounds.
        let (foreground, muted, border) = match (config.color, config.theme) {
            (ColorMode::TrueColor, Theme::Dark) => (
                Color::Rgb(232, 239, 226),
                Color::Rgb(171, 187, 160),
                Color::Rgb(101, 130, 98),
            ),
            (ColorMode::TrueColor, Theme::Light) => (
                Color::Rgb(240, 237, 216),
                Color::Rgb(190, 198, 166),
                Color::Rgb(125, 149, 106),
            ),
            _ => (Color::Indexed(252), Color::Indexed(144), Color::Indexed(65)),
        };
        let accent = match config.color {
            ColorMode::TrueColor => {
                let (r, g, b) = form.rgb;
                Color::Rgb(r, g, b)
            }
            _ => Color::Indexed(form.indexed),
        };
        let base = Style::default().fg(foreground).bg(image_background(config));
        Self {
            color: config.color,
            ascii: config.ascii,
            base,
            surface: base,
            muted: base.fg(muted),
            border: base.fg(border),
            accent: base.fg(accent).add_modifier(Modifier::BOLD),
            selected: Style::default()
                .fg(image_background(config))
                .bg(accent)
                .add_modifier(Modifier::BOLD),
        }
    }
    fn output_style(&self, kind: crate::output_style::Kind, base: Style) -> Style {
        use crate::output_style::Kind;
        if self.color == ColorMode::Mono {
            return match kind {
                Kind::Error => base.add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                Kind::Warning | Kind::Keyword => base.add_modifier(Modifier::BOLD),
                _ => base,
            };
        }
        let ((r, g, b), indexed) = match kind {
            Kind::Keyword => ((203, 166, 247), 183),
            Kind::String => ((166, 218, 149), 150),
            Kind::Number => ((245, 169, 127), 216),
            Kind::Type => ((125, 207, 255), 117),
            Kind::Function => ((238, 212, 159), 223),
            Kind::Error => ((255, 117, 127), 210),
            Kind::Warning => ((240, 198, 116), 222),
            Kind::Comment | Kind::Fence => return base.patch(self.muted),
        };
        let style = base.fg(if self.color == ColorMode::TrueColor {
            Color::Rgb(r, g, b)
        } else {
            Color::Indexed(indexed)
        });
        if matches!(kind, Kind::Error | Kind::Warning) {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        }
    }

    /// The same tokens drawn directly on the canvas, for chrome outside panels
    /// (header and status rows) that should not paint a surface band.
    fn on_canvas(&self) -> Self {
        let canvas = |style: Style| match self.base.bg {
            Some(bg) => style.bg(bg),
            None => style,
        };
        Self {
            color: self.color,
            ascii: self.ascii,
            base: self.base,
            surface: canvas(self.surface),
            muted: canvas(self.muted),
            border: canvas(self.border),
            accent: canvas(self.accent),
            selected: self.selected,
        }
    }

    fn hint(&self, message: Message) -> &'static str {
        message.text()
    }

    fn format(&self, template: Template, args: &[(&str, &str)]) -> String {
        template
            .format(args)
            .unwrap_or_else(|error| format!("Message error: {error}"))
    }

    fn format_values(
        &self,
        template: Template,
        args: &[(&str, crate::messages::Argument<'_>)],
    ) -> String {
        template
            .format_values(args)
            .unwrap_or_else(|error| format!("Message error: {error}"))
    }
}

pub fn image_background(config: &Config) -> Color {
    // Cell alpha and opaque native canvases composite onto the same surface.
    // Kitty retains source alpha and reveals this canvas beneath it.
    match config.color {
        ColorMode::TrueColor => Color::Rgb(0, 0, 0),
        ColorMode::Indexed => Color::Indexed(16),
        ColorMode::Mono => Color::Black,
    }
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
            BorderType::Rounded
        } else {
            BorderType::Plain
        })
    }
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
        None | Some(Overlay::Planning) => {
            if app.overlay.is_none() || planning::file_prompt_area(area, app).is_some() {
                let layout = shell(area, app.linear, app.effective_focus());
                if let Some(rail) = layout.rail {
                    targets.push((app.form(), companion_portrait(rail)));
                }
            }
        }
        Some(_) => {}
    }
    targets.retain(|&(_, target)| !occluded(area, app, target));
}

pub fn render(
    frame: &mut Frame,
    app: &mut App,
    config: &Config,
    artwork: &mut Artwork,
    graphics_status: &str,
) {
    render_content(frame, app, config, artwork, graphics_status);
    // Clear resets cells rather than inheriting the canvas. Resolve only those
    // defaults after every overlay (including early-return/small-screen paths),
    // preserving explicit selection fills and the sprite's own colors.
    let base = Paint::new(config, &FORMS[app.form().min(8)], app.form().min(8)).base;
    for cell in &mut frame.buffer_mut().content {
        if cell.bg == Color::Reset {
            cell.bg = base.bg.unwrap_or(Color::Black);
        }
        if cell.fg == Color::Reset {
            cell.fg = base.fg.unwrap_or(Color::White);
        }
    }
}

fn render_content(
    frame: &mut Frame,
    app: &mut App,
    config: &Config,
    artwork: &mut Artwork,
    graphics_status: &str,
) {
    VERBOSE.set(app.verbose);
    LIVE_FX.set(
        (app.motion_enabled() && !config.no_animation && app.planning.activity_label().is_some())
            .then_some((config.ascii, app.activity_frame)),
    );
    app.planning.set_response_body(Rect::default());
    let area = frame.area();
    app.confirmation_reviewed = false;
    let consent_obscured = app.command_popup.is_some()
        || app.nickname_editor.is_some()
        || app.confirmation.is_some()
        || app.composer_menu.is_some()
        || app.cooperation_ui.release.is_some()
        || app.cooperation.cancel_review.is_some()
        || app.cooperation.journal_discard.is_some();
    if app.overlay != Some(Overlay::Execution) || consent_obscured {
        app.execution.invalidate_review_exposure();
    }
    if app.overlay != Some(Overlay::Sources) || consent_obscured {
        app.sources.invalidate_screenshot_exposure();
    }
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
    let file_prompt = planning::file_prompt_area(area, app);
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
    // Compact file decisions stay attached to the draft and live response.
    if app.overlay != Some(Overlay::Planning) || file_prompt.is_some() {
        if let Some(rail) = shell.rail {
            compact_companion(
                frame,
                rail,
                app,
                &paint,
                artwork,
                (app.overlay.is_none() || file_prompt.is_some())
                    && !occluded(area, app, companion_portrait(rail))
                    && !(app.editing
                        && matches!(
                            app.editor_field,
                            EditorField::Criteria | EditorField::Command
                        )),
            );
        }
        let workspace_area = cooperation::header(frame, shell.workspace, app, &paint);
        let workspace_area = if let Some(prompt) = file_prompt {
            Rect {
                height: workspace_area
                    .height
                    .min(prompt.y.saturating_sub(workspace_area.y)),
                ..workspace_area
            }
        } else {
            workspace_area
        };
        let workspace_area = cooperation::strip(frame, workspace_area, app, &paint);
        let workspace_area = if matches!(app.overlay, Some(Overlay::Tasks | Overlay::Forms)) {
            workspace_area
        } else {
            suggestion_strip(frame, workspace_area, app, &paint)
        };
        let workspace_area = held::strip(frame, workspace_area, app, &paint);
        let review_scroll = file_prompt.map(|_| (app.planning.scroll, app.planning.scroll_max));
        // Roomy layouts list tool calls beside the transcript; `/verbose`
        // keeps every raw observation inline.
        let tools = shell
            .tools
            .filter(|_| app.response_on_home() && !app.verbose);
        app.planning.set_tools_docked(tools.is_some());
        if app.response_on_home() {
            planning::render_workspace(frame, workspace_area, app, &paint);
        } else {
            workspace(frame, workspace_area, app, &paint);
        }
        if let Some(tools) = tools {
            tool_panel(frame, tools, app, config, &paint);
        }
        if let Some((scroll, scroll_max)) = review_scroll {
            // The transcript backdrop must not move the file-review viewport.
            app.planning.scroll = scroll;
            app.planning.scroll_max = scroll_max;
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
    } else if let Some(popup) = bag::notice_area(area, app) {
        bag::notice(frame, popup, app, config, &paint, artwork);
    }
    if let Some(menu) = composer_menu_area(area, app) {
        composer::render(frame, menu, app, config, &paint);
    }
    if let Some(overlay) = app.overlay {
        match overlay {
            Overlay::Forms => forms(frame, area, app, config, &paint, artwork),
            Overlay::Help => help(frame, area, app, config, &paint),
            Overlay::EvolutionPreview => evolution_preview(frame, area, app, config, &paint),
            Overlay::Journey => journey(frame, area, app, config, &paint),
            Overlay::Bag => bag::render(frame, area, app, config, &paint, artwork),
            Overlay::Usage => meter::render(frame, area, app, config, &paint),
            Overlay::Rules => rules::render(frame, area, app, config, &paint),
            Overlay::Mcp => mcp::render(frame, area, app, config, &paint),
            Overlay::Preview => preview(frame, area, app, config, &paint),
            Overlay::Tasks => tasks(frame, area, app, config, &paint),
            Overlay::Criteria => criteria(frame, area, app, config, &paint),
            Overlay::Repository => repository(frame, area, app, config, &paint),
            Overlay::Sources => {
                if !consent_obscured {
                    sources::render(frame, area, app, config, &paint);
                }
            }
            Overlay::Execution => {
                let queue = app.approval_position().map(|(position, total)| {
                    format!(
                        " {} ",
                        paint.format(
                            Template::ProvidersPlanningPositionOfTotal,
                            &[
                                ("position", &position.to_string()),
                                ("total", &total.to_string()),
                                ("owner", &cooperation::approval_owner(app)),
                            ],
                        )
                    )
                });
                let editing = app.write_owner().map(|claim| {
                    paint.format(
                        Template::ProvidersExecutionOwnerEditing,
                        &[("owner", &store::Attribution::label(Some(&claim.owner)))],
                    )
                });
                let allow_all = app.current_callback_command_deniable();
                if !consent_obscured || app.execution.confirmation.is_none() {
                    execution::render(
                        frame,
                        area,
                        &mut app.execution,
                        config,
                        queue.as_deref(),
                        editing.as_deref(),
                        allow_all,
                    );
                }
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
            Overlay::ExternalEditor => external_editor::render(frame, area, app, config, &paint),
            Overlay::Artifacts => artifacts::render(frame, area, &mut app.artifacts, config),
            Overlay::ToolCatalog => tool_catalog::render(frame, area, app, config, &paint),
            Overlay::ToolPlans => tool_plans::render(frame, area, app, config, &paint),
            Overlay::ParkedDrafts => parked_drafts::render(frame, area, app, config, &paint),
            Overlay::Retained => retained::render(frame, area, app, config, &paint),
            Overlay::Receipts => app.planning.receipts.render(frame, area),
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
    if consent_obscured {
        app.planning.invalidate_review();
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
    // The mode decides who picks the form at Send; say it and how to change it.
    let mode = if app.lead().auto_form {
        "Auto (Eevee picks at Send)".to_owned()
    } else {
        format!("Pinned: {current} · a Auto")
    };
    let heading = match (app.visible_suggestion(), grid) {
        (Some(suggestion), false) => format!(
            "Current: {current} · {mode} · Suggested: {} · d Dismiss",
            FORMS[suggestion.form].name
        ),
        (Some(_), true) => format!("Current: {current} · {mode} · d Dismiss suggestion"),
        (None, _) => format!("Current: {current} · {mode}"),
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
                    picker_marker(app, index, &cell_paint),
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
        // Show as many forms as fit above the selected form's details.
        let visible = usize::from(content.height.saturating_sub(10)).clamp(3, 9);
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
                picker_marker(app, index, paint),
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
        // One blank row separates the list from the selected form's details
        // when the sheet has room; compact sheets keep it for saved moves.
        let top = 2 + visible as u16 + u16::from(content.height >= 20);
        let info = row(content, top, content.height.saturating_sub(top));
        // Saved moves keep their rows; the description wraps only when the
        // compact list leaves room for it.
        let form = &FORMS[selected];
        let width = info.width as usize;
        let summary = format!("{} · {}", form.name, form.summary);
        let mut value = if info.height >= 10 {
            format!("{summary}\nGood for: {}\n", form.good_for)
        } else {
            format!("{}\n", clipped(&summary, width))
        };
        let marker = picker_marker(app, selected, paint);
        if !marker.is_empty() {
            value.push_str(&clipped(&marker, width));
            value.push('\n');
        }
        value.push_str(&picker_moves(app, selected, info.width, paint));
        frame.render_widget(
            Paragraph::new(value)
                .style(paint.muted)
                .wrap(Wrap { trim: false }),
            info,
        );
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
    // Plain-language summary first; the expected-evidence contract and
    // workflow availability are technical details shown with /verbose.
    let technical = if VERBOSE.get() {
        format!(
            "\n{}\n{}\n",
            form.evidence,
            match index {
                0 => "Task cards and resumption: available",
                1 => "Source ranges/context basket: /files",
                2 => "Approved local commands/evidence: /run",
                3 => "Reviewed selected-file implementation: /provider | /role",
                4 => "Reviewed multi-agent planning: /provider",
                _ => "Provider specialist workflow: unavailable",
            }
        )
    } else {
        String::new()
    };
    let value = format!(
        "{} / {} · {}\n\n{}\nGood for: {}\n\n{}\n{}\n{technical}\n{}\n{}",
        form.name,
        form.element,
        form.role,
        form.summary,
        form.good_for,
        picker_moves(app, index, area.width, &paint),
        if app.record_form_marker(index).count == 0 {
            String::new()
        } else {
            records::marker(app.record_form_marker(index), &paint)
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

/// A form with no saved records shows nothing rather than a bare "none".
fn picker_marker(app: &App, form: usize, paint: &Paint) -> String {
    let marker = app.record_form_marker(form);
    if marker.count == 0 {
        String::new()
    } else {
        records::compact_marker(marker, paint)
    }
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

fn tasks(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    // An empty list needs only its short guidance, not a full-height sheet.
    let height = if app.visible_tasks().next().is_none() {
        8
    } else {
        28
    };
    let area = sheet(area, 88, height);
    frame.render_widget(Clear, area);
    let block = panel(
        if app.show_archived {
            " Archived task cards "
        } else {
            " Saved sessions / committed tasks "
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
            "{} cards | /archived View / {} | /delete | /prune Retention",
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
    // Border, heading, gap, one row per partner, gap and two control rows.
    let rows = u16::try_from(app.partners().len()).unwrap_or(u16::MAX);
    let area = sheet(area, 88, rows.saturating_add(7).min(24));
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

fn journey(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = sheet(area, 82, 30);
    frame.render_widget(Clear, area);
    let block = panel(
        " Request journey / all routes and models ",
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let mut lines: Vec<String> = crate::providers::capabilities::JOURNEY
        .iter()
        .enumerate()
        .map(|(index, (state, keys))| format!("{:02} {state}: {keys}", index + 1))
        .collect();
    lines.push(String::new());
    lines.extend(crate::providers::capabilities::disclosures(
        app.planning.agent,
        app.planning.operation,
        app.planning.catalogues.get(&app.planning.agent),
        &app.planning.selection,
    ));
    lines.push(format!(
        "Task progress: {}",
        app.workflows.progress.as_ref().map_or_else(
            || "no step list from this route".into(),
            |progress| progress.summary(),
        )
    ));
    lines.push(String::new());
    lines.push("Esc back · Up/Down/PgUp/PgDn scroll · /commands unchanged. Inspection never sends, retries, adopts progress or grants authority.".into());
    scroll_text(
        frame,
        content,
        Paragraph::new(lines.join("\n"))
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app,
        ScrollSurface::Preview,
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
            format!("What it does: {}", FORMS[preview.selection.form].summary),
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
            preview.effort.as_ref().map_or_else(
                || format!("Effort: {} / {}", app.planning.selection.effort.as_deref().unwrap_or("provider default"), crate::providers::capabilities::route(app.planning.agent).effort),
                |effort| effort.line(),
            ),
            format!("Todo step: {}", preview.todo_step.as_deref().filter(|step| *step != "no step list from this route").unwrap_or("none yet; the agent creates one when work starts")),
            if VERBOSE.get() {
                "Context: current task and reviewed selections; exact outgoing bytes reviewed before dispatch".into()
            } else {
                "Context: current task and reviewed selections".into()
            },
            format!("Model capabilities: {}", crate::providers::capabilities::model_disclosure(app.planning.catalogues.get(&app.planning.agent), &app.planning.selection)),
            String::new(),
            // The guard repeats what every effect review enforces; /verbose shows it.
            if VERBOSE.get() {
                format!("{}\n", paint.hint(Message::CoreEvolutionPreviewGuard))
            } else {
                String::new()
            },
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
    let screenshot = app.sources.screenshot.as_ref().filter(|attachment| {
        attachment.capture.task_id == app.task_id && attachment.capture.partner == app.lead().id
    });
    let image_title;
    let title = if let Some(attachment) = screenshot {
        image_title = format!(
            "{} + {} image ({} B)",
            title,
            attachment.capture.format.label(),
            attachment.capture.bytes,
        );
        image_title.as_str()
    } else {
        title
    };
    let attachment_status = if app.sources.store_failed || app.sources.screenshot_storage_failed() {
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
            (
                "count",
                &(app.sources.basket.len() + usize::from(screenshot.is_some())).to_string(),
            ),
            ("status", attachment_status),
            (
                "pairing",
                remembered.map(|message| paint.hint(message)).unwrap_or(""),
            ),
        ],
    );
    let title = match composer_todo(app, frame.area().width) {
        Some(todo) => format!("{} · {todo} ", title.trim_end()),
        None => title,
    };
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
    let profile_rows = u16::from(app.composer_is_request() && content.height > hint_rows + 1);
    if profile_rows != 0 {
        text(
            frame,
            row(content, 0, profile_rows),
            &app.profile_chip(),
            paint.muted,
        );
    }
    let content = row(
        content,
        profile_rows,
        content.height.saturating_sub(profile_rows),
    );
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
