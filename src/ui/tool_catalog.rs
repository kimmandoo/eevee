//! Fixed neutral review controls; imported prose is displayed only as literal data.
use super::*;
use crate::{
    app::tool_catalog::Mode,
    messages::{Message, Template},
    providers::Agent,
};

pub(super) fn render(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    frame.render_widget(Clear, area);
    let state = &app.tool_catalog;
    if app.width < 60 || app.height < 18 || area.width < 56 || area.height < 14 {
        state.invalidate();
        text(
            frame,
            area,
            paint.hint(Message::ToolCatalogTooSmall),
            paint.accent,
        );
        return;
    }
    let area = centered(area, 112, area.height.min(30));
    let block = panel(paint.hint(Message::ToolCatalogTitle), config, paint, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let footer = inner.bottom().saturating_sub(1);
    let content_height = footer.saturating_sub(inner.y);
    let mut lines: Vec<Line<'_>> = Vec::new();
    let mut complete_review = false;
    let mut review_scroll = 0u16;
    match state.mode {
        Mode::Overview => {
            let agent = app.planning.agent;
            let writing = app.planning.operation == crate::providers::Operation::Implement;
            lines.push(Line::styled(
                format!(
                    "{} · {:?} · next Send",
                    agent.label(),
                    app.planning.operation
                ),
                paint.accent,
            ));
            lines.push(Line::from("Built-in tools need no catalog import."));
            if agent.is_direct() || agent == Agent::Codex {
                lines.push(Line::from(
                    "Source: eevee_list_files · eevee_read_file · eevee_structural_search (Rust)",
                ));
                lines.push(Line::from(
                    "Progress: todo · retained output: eevee_artifact_list/read/search/locations",
                ));
                if writing {
                    lines.push(Line::from(
                        "Files: exact create / write / edit / delete / rename",
                    ));
                    lines.push(Line::from(
                        "Build / test / run / download: eevee_run_command",
                    ));
                }
            } else if agent == Agent::Claude {
                lines.push(Line::from(
                    "Source: host discovery · Read / Glob / Grep; progress: TodoWrite",
                ));
                if writing {
                    lines.push(Line::from(
                        "Files: Write / Edit · run / download: mcp__eevee_command__run_command",
                    ));
                }
            } else {
                lines.push(Line::from("Source: ACP fs/read_text_file · provider plan and tools depend on the connected agent."));
                if writing {
                    lines.push(Line::from("Files: ACP fs/write_text_file · run / download: terminal/create → wait_for_exit/output"));
                }
            }
            if writing {
                lines.push(Line::from("Downloads use an installed downloader/runtime through the same reviewed command tool. Downloading does not authorize running fetched code."));
            }
            if !writing {
                lines.push(Line::styled(
                    "Builds and file changes are disabled in explicit Plan/Review. /role-implement restores ordinary coding on the next Send.",
                    paint.accent,
                ));
            }
            lines.push(Line::from("File changes and commands still need exact approval or your explicit approval rule. /approvals shows waiting calls."));
            lines.push(Line::from(""));
            if agent.is_direct() {
                let connected = app
                    .mcp
                    .tools
                    .iter()
                    .filter(|tool| tool.project == app.cwd)
                    .count();
                let selected = app
                    .mcp
                    .tools
                    .iter()
                    .filter(|tool| {
                        tool.project == app.cwd
                            && app.mcp.selected.contains(&tool.wire_name())
                            && (writing || tool.user_read)
                    })
                    .count();
                lines.push(Line::from(format!(
                    "MCP: {connected} connected tools · {selected} eligible selected tools."
                )));
                lines.push(Line::from("/mcp connect <server> reviews launch; /mcp select <server> <tool> exposes its schema. Selection never grants a call."));
            } else {
                lines.push(Line::from("User-registered MCP tools are native-route only; this route's built-ins are independent of /mcp."));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(format!(
                "Optional catalog aliases: {} stored · {} selected. c opens the catalog; Tab searches it.",
                state.document.as_ref().map_or(0, |doc| doc.manifest.tools.len()),
                state.selected.len(),
            )));
            lines.push(Line::from("Availability is host configuration, not proof that the model has used a tool. Calls and real results appear in /response; /verbose retains details."));
            let total = Paragraph::new(lines.clone())
                .wrap(Wrap { trim: false })
                .line_count(inner.width);
            review_scroll = state.scroll.min(
                total
                    .saturating_sub(usize::from(content_height))
                    .min(u16::MAX as usize) as u16,
            );
        }
        Mode::Path => {
            lines.push(Line::from(paint.hint(Message::ToolCatalogPath)));
            lines.push(Line::from(
                super::display_identity(&format!(
                    "{}|{}",
                    &state.path[..state.cursor],
                    &state.path[state.cursor..]
                ))
                .into_owned(),
            ));
            lines.push(Line::from(paint.hint(Message::ToolCatalogPolicy)));
        }
        Mode::SetupReview => {
            if let Some(document) = &state.candidate {
                let count = document.manifest.tools.len().to_string();
                let summary = paint.format(Template::ToolCatalogReview, &[("count", &count)]);
                let origin = paint.format(
                    Template::ToolCatalogOrigin,
                    &[(
                        "path",
                        &super::display_identity(&document.source_path.to_string_lossy()),
                    )],
                );
                lines.push(Line::from(summary));
                lines.push(Line::from(origin));
                lines.push(Line::from(paint.hint(Message::ToolCatalogEffectsRead)));
                if document
                    .manifest
                    .tools
                    .iter()
                    .any(|entry| entry.route.mutating())
                {
                    lines.push(Line::from(paint.hint(Message::ToolCatalogEffectsWrite)));
                }
                lines.push(Line::from(paint.hint(Message::ToolCatalogPolicy)));
                let paragraph = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
                let total = paragraph.line_count(inner.width);
                review_scroll = state.scroll.min(
                    total
                        .saturating_sub(usize::from(content_height))
                        .min(u16::MAX as usize) as u16,
                );
                complete_review = state.observe_review(
                    usize::from(review_scroll),
                    total,
                    usize::from(content_height),
                    inner.width,
                );
            }
        }
        Mode::Details => {
            frame.render_widget(
                Paragraph::new(state.details.as_str())
                    .wrap(Wrap { trim: false })
                    .scroll((state.scroll, 0))
                    .style(paint.base),
                Rect::new(inner.x, inner.y, inner.width, content_height),
            );
        }
        Mode::Browse | Mode::Search => {
            if app.planning.agent != Agent::Compatible {
                lines.push(Line::from(paint.hint(Message::ToolCatalogUnavailable)));
            }
            if let Some(document) = &state.document {
                lines.push(Line::from(paint.format(
                    Template::ToolCatalogCount,
                    &[
                        ("found", &state.ranked.len().to_string()),
                        ("total", &document.manifest.tools.len().to_string()),
                        ("selected", &state.selected.len().to_string()),
                    ],
                )));
                let query = if state.mode == Mode::Search {
                    format!(
                        "{}|{}",
                        &state.query[..state.cursor],
                        &state.query[state.cursor..]
                    )
                } else {
                    state.query.clone()
                };
                lines.push(Line::from(
                    paint.format(Template::ToolCatalogQuery, &[("query", &query)]),
                ));
                if !state.selected.is_empty()
                    && let Some(comparison) = &state.schema_counts
                {
                    lines.push(Line::from(paint.format(
                        Template::ToolCatalogTokens,
                        &[
                            ("selected", &comparison.selected.to_string()),
                            ("full", &comparison.full.to_string()),
                            ("tokenizer", comparison.tokenizer),
                        ],
                    )));
                }
                if document.connection_scope.is_empty() {
                    lines.push(Line::from(paint.hint(Message::ToolCatalogDisconnected)));
                }
                let used = Paragraph::new(lines.clone())
                    .wrap(Wrap { trim: false })
                    .line_count(inner.width);
                let visible = usize::from(content_height).saturating_sub(used + 2).max(1);
                let start = state.focused.saturating_sub(visible.saturating_sub(1));
                for (position, index) in state.ranked.iter().enumerate().skip(start).take(visible) {
                    let entry = &document.manifest.tools[*index];
                    lines.push(Line::styled(
                        format!(
                            "{} [{}] {}",
                            if position == state.focused { ">" } else { " " },
                            if state.selected.contains(index) {
                                "x"
                            } else {
                                " "
                            },
                            entry.identity()
                        ),
                        if position == state.focused {
                            paint.accent
                        } else {
                            paint.base
                        },
                    ));
                }
            } else {
                lines.push(Line::from(paint.hint(Message::ToolCatalogEmpty)));
            }
        }
    }
    if state.busy() {
        lines.push(Line::from(paint.hint(Message::ToolCatalogWaiting)));
    }
    if let Some(notice) = state.notice {
        lines.push(Line::from(paint.hint(notice)));
    }
    if !state.error.is_empty() {
        lines.push(Line::from(super::display_identity(&state.error)));
    }
    if state.failed.is_some() {
        lines.push(Line::from(paint.hint(Message::ToolCatalogRetry)));
    }
    if state.mode != Mode::Details {
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((review_scroll, 0))
                .style(paint.base),
            Rect::new(inner.x, inner.y, inner.width, content_height),
        );
    }
    let hint = match state.mode {
        Mode::Overview => Message::ToolCatalogOverviewHint,
        Mode::Path => Message::ToolCatalogPathHint,
        Mode::Search => Message::ToolCatalogSearchHint,
        Mode::Browse => Message::ToolCatalogBrowseHint,
        Mode::SetupReview => Message::ToolCatalogReviewHint,
        Mode::Details => Message::ToolCatalogDetailsHint,
    };
    text(
        frame,
        Rect::new(inner.x, footer, inner.width, 1),
        paint.hint(hint),
        paint.accent,
    );
    state
        .reviewed
        .set(complete_review && !state.busy() && state.failed.is_none());
}
