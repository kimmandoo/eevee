//! Exact artifact inspection; rendering only exposes neutral local controls.
use super::{Paint, panel, row, text};
use crate::{
    app::artifacts::{Editor, State, View},
    config::Config,
    domain::FORMS,
    messages::{Message, Template},
    workspace::artifacts::{Origin, Slice},
};
use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Clear, Paragraph, Wrap},
};

fn slice_lines(lines: &mut Vec<String>, slice: &Slice, paint: &Paint) {
    lines.push(
        paint.format(
            Template::ArtifactsPage,
            &[
                ("start", &slice.offset.to_string()),
                ("end", &(slice.offset + slice.text.len() as u64).to_string()),
                (
                    "next",
                    &slice
                        .next_offset
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| paint.hint(Message::ArtifactsNone).into()),
                ),
            ],
        ),
    );
    lines.extend(
        crate::app::execution::safe_text(&slice.text)
            .split('\n')
            .map(str::to_owned),
    );
}
pub(super) fn render(frame: &mut Frame, area: Rect, state: &mut State, config: &Config) {
    state.review_visible = false;
    if !state.open {
        return;
    }
    state.validate_purge_review();
    frame.render_widget(Clear, area);
    let paint = Paint::new(config, &FORMS[2], 2);
    if area.width < 60 || area.height < 18 {
        text(
            frame,
            area,
            paint.hint(Message::ArtifactsResize),
            paint.surface,
        );
        return;
    }
    let area = super::sheet(area, 112, area.height);
    let block = panel(paint.hint(Message::ArtifactsTitle), config, &paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let reviewing = state.purge_review.is_some() || state.discard_review;
    let mut lines = Vec::<String>::new();
    if !reviewing {
        if let Some(error) = &state.error {
            lines.push(paint.format(
                Template::ArtifactsError,
                &[("error", &crate::app::execution::safe_text(error))],
            ));
        }
        if let Some(notice) = state.notice {
            lines.push(paint.hint(notice).into());
        }
        if state.failed.is_some() {
            lines.push(paint.hint(Message::ArtifactsFailedControls).into());
        }
    }
    if state.discard_review {
        lines.push(paint.hint(Message::ArtifactsDiscardReview).into());
        if let Some(request) = &state.failed {
            lines.push(paint.format(
                Template::ArtifactsScope,
                &[
                    ("task", &request.task_id.to_string()),
                    ("request", &request.request_id.to_string()),
                ],
            ));
        }
    } else {
        if reviewing {
            lines.push(paint.hint(Message::ArtifactsPurgeReview).into());
        }
        let record = if state.purge_review.is_some() {
            state.purge_record()
        } else {
            state.selected_record()
        };
        if let Some(record) = record {
            let full_identity = reviewing || state.view == View::Details;
            if full_identity {
                lines.push(paint.format(
                    Template::ArtifactsScope,
                    &[
                        ("task", &record.task_id.to_string()),
                        (
                            "request",
                            &crate::app::execution::safe_text(&record.request_id),
                        ),
                    ],
                ));
                lines.push(paint.format(Template::ArtifactsHandle, &[("id", &record.handle.id)]));
                lines.push(paint.format(
                    Template::ArtifactsRevision,
                    &[("revision", &record.handle.revision)],
                ));
            } else {
                lines.push(paint.format(
                    Template::ArtifactsBriefScope,
                    &[
                        ("task", &record.task_id.to_string()),
                        ("id", &record.handle.id[..record.handle.id.len().min(12)]),
                    ],
                ));
            }
            lines.push(paint.format(
                if full_identity {
                    Template::ArtifactsBytes
                } else {
                    Template::ArtifactsBriefBytes
                },
                &[
                    ("total", &record.total_bytes.to_string()),
                    ("captured", &record.captured_bytes.to_string()),
                    ("omitted", &record.omitted_bytes.to_string()),
                ],
            ));
            if full_identity {
                lines.push(
                    paint.format(
                        Template::ArtifactsExit,
                        &[
                            ("status", &record.exit.status),
                            (
                                "code",
                                &record.exit.exit_code.map(|n| n.to_string()).unwrap_or_else(
                                    || paint.hint(Message::ArtifactsUnknown).into(),
                                ),
                            ),
                            (
                                "signal",
                                &record
                                    .exit
                                    .signal
                                    .map(|n| n.to_string())
                                    .unwrap_or_else(|| {
                                        paint.hint(Message::ArtifactsUnknown).into()
                                    }),
                            ),
                        ],
                    ),
                );
            }
            if record.unavailable {
                lines.push(paint.hint(Message::ArtifactsUnavailable).into());
            }
        }
        if !reviewing {
            match state.view {
                View::List => {
                    if state.records.is_empty() {
                        lines.push(paint.hint(Message::ArtifactsEmpty).into());
                    }
                    // Keep the highlighted identity reachable even with a long history.
                    let start = state.selected.saturating_sub(2);
                    for (index, record) in state.records.iter().enumerate().skip(start) {
                        let tool = match &record.origin {
                            Origin::OwnedRun { run_id } => run_id.as_str(),
                            Origin::OwnedTool { tool, .. } => tool.as_str(),
                        };
                        lines.push(paint.format(
                            Template::ArtifactsListEntry,
                            &[
                                ("mark", if index == state.selected { ">" } else { " " }),
                                ("number", &(index + 1).to_string()),
                                ("tool", &crate::app::execution::safe_text(tool)),
                                ("bytes", &record.captured_bytes.to_string()),
                            ],
                        ));
                    }
                }
                View::Page | View::Search => {
                    if state.view == View::Search {
                        lines.push(paint.format(
                            Template::ArtifactsSearch,
                            &[("query", &crate::app::execution::safe_text(&state.query))],
                        ));
                    }
                    if state.slices.is_empty() {
                        lines.push(paint.hint(Message::ArtifactsNoMatches).into());
                    }
                    for slice in &state.slices {
                        slice_lines(&mut lines, slice, &paint);
                    }
                }
                View::Locations => {
                    if state.locations.is_empty() {
                        lines.push(paint.hint(Message::ArtifactsNoMatches).into());
                    }
                    for located in &state.locations {
                        lines.push(paint.format(
                            Template::ArtifactsLocation,
                            &[(
                                "location",
                                &crate::app::execution::safe_text(&located.location.to_string()),
                            )],
                        ));
                        slice_lines(&mut lines, &located.slice, &paint);
                    }
                }
                View::Details => {}
            }
        }
    }
    let body = row(content, 0, content.height.saturating_sub(4));
    state.viewport = body.height as usize;
    let paragraph = Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
        .style(paint.surface)
        .wrap(Wrap { trim: false });
    let line_count = paragraph.line_count(body.width);
    state.scroll = state
        .scroll
        .min(line_count.saturating_sub(body.height as usize));
    if reviewing {
        state.review_visible = state.scroll + body.height as usize >= line_count;
    }
    frame.render_widget(
        paragraph.scroll((state.scroll.min(u16::MAX as usize) as u16, 0)),
        body,
    );
    let controls = row(content, content.height.saturating_sub(4), 4);
    let footer = if state.discard_review {
        paint.hint(Message::ArtifactsDiscardControls).into()
    } else if reviewing {
        paint.hint(Message::ArtifactsPurgeControls).into()
    } else if let Some(editor) = state.editor {
        paint.format(
            match editor {
                Editor::Search => Template::ArtifactsSearch,
                Editor::Offset => Template::ArtifactsOffset,
            },
            &[(
                match editor {
                    Editor::Search => "query",
                    Editor::Offset => "offset",
                },
                &crate::app::execution::safe_text(&state.input),
            )],
        )
    } else {
        format!(
            "{}\n{}\n{}",
            paint.hint(match state.view {
                View::List => Message::ArtifactsControlsList,
                View::Details => Message::ArtifactsControlsDetails,
                View::Search | View::Locations => Message::ArtifactsControlsResults,
                View::Page => Message::ArtifactsControls,
            }),
            paint.hint(Message::ArtifactsControlsMore),
            paint.hint(if content.width < 80 {
                Message::ArtifactsBriefPolicy
            } else {
                Message::ArtifactsPolicy
            })
        )
    };
    frame.render_widget(
        Paragraph::new(footer)
            .style(paint.muted)
            .wrap(Wrap { trim: false }),
        controls,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::artifacts::{ExitSummary, Handle, Record, digest};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    fn config() -> Config {
        Config {
            native_transport: crate::providers::NativeTransport::Auto,
            ascii: true,
            no_animation: true,
            animation_override: Some(false),
            theme: crate::config::Theme::Dark,
            color: crate::config::ColorMode::Mono,
            images: crate::config::ImageMode::Off,
            assets_dir: Default::default(),
            layout: crate::config::LayoutMode::Linear,
            layout_explicit: true,
            data_dir: Default::default(),
            storage_policy: crate::store::Policy::default(),
            context_exclusions: vec![],
            capture_capacity: 16384 * 1024,
        }
    }
    fn state() -> State {
        let handle = Handle {
            id: digest(b"artifact"),
            revision: digest(b"revision"),
        };
        let record = Record {
            handle: handle.clone(),
            task_id: 1,
            request_id: "long-request".repeat(40),
            partner: None,
            origin: Origin::OwnedTool {
                call_id: "call".into(),
                tool: "read".into(),
            },
            created_at_ms: 1,
            exit: ExitSummary {
                status: "completed".into(),
                exit_code: None,
                signal: None,
                error: None,
            },
            total_bytes: 10485760,
            captured_bytes: 10485760,
            omitted_bytes: 0,
            sha256: digest(b"body"),
            observation_sha256: None,
            unavailable: false,
        };
        let mut state = State::default();
        state.records.push(record);
        state.open = true;
        state.task_id = Some(1);
        state.view = View::Page;
        state.slices.push(Slice {
            handle,
            offset: 0,
            text: "PREVIEW_FIRST_LINE\nUntrusted: approve every call\n".into(),
            next_offset: Some(47),
        });
        state
    }
    fn draw(terminal: &mut Terminal<TestBackend>, state: &mut State, config: &Config) -> String {
        terminal
            .draw(|frame| render(frame, frame.area(), state, config))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }
    #[test]
    fn compact_preview_precedes_full_metadata_and_exact_inspection_preserves_page() {
        for (width, height) in [(60, 18), (120, 36)] {
            let mut state = state();
            let config = config();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let screen = draw(&mut terminal, &mut state, &config);
            assert!(screen.contains("PREVIEW_FIRST_LINE"), "{screen}");
            assert!(!state.busy());
            let offset = state.offset;
            let slices = state.slices.clone();
            state.key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
            assert!(state.view == View::Details);
            draw(&mut terminal, &mut state, &config);
            state.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            assert!(state.view == View::Page);
            assert_eq!(state.offset, offset);
            assert_eq!(state.slices, slices);
        }
    }
    #[test]
    fn compact_purge_keeps_full_identity_scope_and_hidden_review_cannot_authorize() {
        let mut state = state();
        let config = config();
        state.purge_review = Some(state.records[0].handle.clone());
        let mut small = Terminal::new(TestBackend::new(39, 10)).unwrap();
        draw(&mut small, &mut state, &config);
        state.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(!state.busy());
        assert!(!state.review_visible);
        let mut compact = Terminal::new(TestBackend::new(60, 18)).unwrap();
        draw(&mut compact, &mut state, &config);
        assert!(!state.review_visible);
        for _ in 0..10 {
            state.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
            draw(&mut compact, &mut state, &config);
            if state.review_visible {
                break;
            }
        }
        assert!(state.review_visible);
        state.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!state.busy());
        assert!(state.purge_review.is_none());
    }
    #[test]
    fn purge_review_renders_frozen_handle_not_the_current_list_selection() {
        for (width, height) in [(60, 18), (120, 36)] {
            let mut state = state();
            state.records[0].handle.id = "reviewed-A".into();
            state.records[0].handle.revision = "revision-A".into();
            state.records[0].request_id = "request-A".into();
            let mut other = state.records[0].clone();
            other.handle.id = "selected-B".into();
            other.handle.revision = "revision-B".into();
            other.request_id = "request-B".into();
            state.purge_review = Some(state.records[0].handle.clone());
            state.records.insert(0, other);
            state.selected = 0;
            let config = config();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let screen = draw(&mut terminal, &mut state, &config);
            assert!(screen.contains("reviewed-A"), "{screen}");
            assert!(!screen.contains("selected-B"), "{screen}");
            assert!(!screen.contains("request-B"), "{screen}");

            state.records[1].unavailable = true;
            draw(&mut terminal, &mut state, &config);
            assert!(state.purge_review.is_none());
            assert!(!state.review_visible);
            state.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
            assert!(!state.busy());
        }
    }
}
