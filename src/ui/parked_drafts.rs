//! Complete escaped text inspection and neutral exact replacement review.
use super::*;

pub(super) fn render(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    frame.render_widget(Clear, area);
    let state = &app.parked_drafts;
    if app.width < 60 || app.height < 18 || area.width < 56 || area.height < 14 {
        state.complete.set(false);
        state.viewed.set(0);
        text(
            frame,
            area,
            "Parked drafts: resize to at least 60x18. Escape keeps text.",
            paint.accent,
        );
        return;
    }
    let area = centered(area, 110, area.height.min(36));
    let block = panel("Parked unsent drafts", config, paint, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = inner.height.saturating_sub(3);
    let mut lines = vec![Line::from(format!(
        "Task {} / partner {} · TEXT ONLY · never scheduled",
        app.task_id,
        app.lead().id
    ))];
    if let Some(review) = &state.review {
        lines.push(Line::from(if review.discard {
            "Discard this exact parked entry? Local deletion is not secure erasure."
        } else {
            "Replace the exact current Request? No context, session or grants move."
        }));
        lines.push(Line::from(format!(
            "Entry {} · {} UTF-8 bytes",
            review.entry.id,
            review.entry.text.len()
        )));
        lines.push(Line::from(format!(
            "Parked text (JSON escaped, exact): {}",
            serde_json::to_string(&review.entry.text).expect("text serializes")
        )));
        lines.push(Line::from(format!(
            "Current Request kept on cancel ({} bytes, JSON escaped): {}",
            review.original.len(),
            serde_json::to_string(&review.original).expect("text serializes")
        )));
    } else if state.inspect {
        if let Some(entry) = app.parked_entries().nth(state.selected) {
            lines.push(Line::from(format!(
                "Entry {} · {} UTF-8 bytes",
                entry.id,
                entry.text.len()
            )));
            lines.push(Line::from(
                "Exact text follows as JSON: escapes preserve CR/LF/tab and trailing newlines.",
            ));
            lines.push(Line::from(
                serde_json::to_string(&entry.text).expect("text serializes"),
            ));
        } else {
            lines.push(Line::from("No selected parked draft."));
        }
    } else {
        lines.push(Line::from(
            "Limits: 8 entries / 16 KiB per partner; 48 / 64 KiB per task; 4096 B each.",
        ));
        lines.push(Line::from(
            "Restore copies text only. Entries remain until explicitly discarded.",
        ));
        let mut count = 0;
        for (index, entry) in app.parked_entries().enumerate() {
            count += 1;
            let preview: String = entry.text.chars().take(52).collect();
            lines.push(Line::from(format!(
                "{} Entry {} · {} B · {}",
                if index == state.selected { ">" } else { " " },
                entry.id,
                entry.text.len(),
                serde_json::to_string(&preview).expect("text serializes")
            )));
        }
        if count == 0 {
            lines.push(Line::from(
                "No parked drafts for this task and partner. Use /park-draft from Request.",
            ));
        }
    }
    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .style(paint.base);
    let total = paragraph.line_count(inner.width);
    let scroll = usize::from(state.scroll).min(total.saturating_sub(usize::from(rows)));
    if app.command_popup.is_some() || app.confirmation.is_some() {
        state.complete.set(false);
        state.viewed.set(0);
    } else if state.review.is_some() {
        let seen = state.viewed.get();
        if scroll <= seen {
            state
                .viewed
                .set(seen.max(scroll + usize::from(rows)).min(total));
        }
        state.complete.set(state.viewed.get() >= total);
    }
    frame.render_widget(
        paragraph.scroll((scroll as u16, 0)),
        Rect::new(inner.x, inner.y, inner.width, rows),
    );
    let controls = if let Some(review) = &state.review {
        format!(
            "Up/Down or PgUp/PgDn: read all · Left/Right: choose\n{} / {} · Enter: confirm choice · Esc: keep both",
            if state.affirmative { "Stay" } else { "[Stay]" },
            match (review.discard, state.affirmative) {
                (true, true) => "[Discard]",
                (true, false) => "Discard",
                (false, true) => "[Replace]",
                (false, false) => "Replace",
            }
        )
    } else {
        "Up/Down: select/read · Enter: inspect · r: restore copy\nd: review discard · Esc: back · /park-draft: park Request".into()
    };
    frame.render_widget(
        Paragraph::new(choice_text(&controls, paint.muted, paint.selected))
            .wrap(Wrap { trim: false }),
        Rect::new(inner.x, inner.bottom().saturating_sub(3), inner.width, 3),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn exact_replacement_requires_sequential_full_scope_and_resize_revokes_it() {
        for (width, height) in [(120, 36), (60, 18)] {
            let config = Config {
                native_transport: crate::providers::NativeTransport::Auto,
                retry: Default::default(),
                launch_session: Default::default(),
                ascii: true,
                no_animation: true,
                animation_override: Some(false),
                theme: crate::config::Theme::Dark,
                color: crate::config::ColorMode::Mono,
                images: crate::config::ImageMode::Off,
                assets_dir: "unused".into(),
                layout: crate::config::LayoutMode::Sanctuary,
                layout_explicit: false,
                data_dir: "unused".into(),
                storage_policy: crate::store::Policy::default(),
                context_exclusions: Vec::new(),
                capture_capacity: 64 * 1024,
            };
            let paint = Paint::new(&config, &FORMS[0], 0);
            let mut app = App::new("/project".into(), width, height);
            app.storage = crate::app::StorageState::Saved;
            app.task_id = 7;
            app.input = "current exact draft".into();
            app.cursor = 3;
            app.workflows
                .parked_drafts
                .push(crate::store::parked_drafts::Entry {
                    id: 1,
                    partner: 1,
                    text: "\t".repeat(4096),
                });
            app.command("drafts");
            app.update(Event::Key(KeyEvent::new(
                KeyCode::Char('r'),
                KeyModifiers::NONE,
            )));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| render(frame, frame.area(), &app, &config, &paint))
                .unwrap();
            assert!(!app.parked_drafts.complete.get());
            app.parked_drafts.scroll = u16::MAX;
            terminal
                .draw(|frame| render(frame, frame.area(), &app, &config, &paint))
                .unwrap();
            assert!(
                !app.parked_drafts.complete.get(),
                "skipping hidden text grants no review"
            );
            app.parked_drafts.scroll = 0;
            for scroll in 0..500 {
                app.parked_drafts.scroll = scroll;
                terminal
                    .draw(|frame| render(frame, frame.area(), &app, &config, &paint))
                    .unwrap();
                if app.parked_drafts.complete.get() {
                    break;
                }
            }
            assert!(app.parked_drafts.complete.get());
            app.update(Event::Key(KeyEvent::new(
                KeyCode::Char('k'),
                KeyModifiers::CONTROL,
            )));
            assert!(
                !app.parked_drafts.complete.get(),
                "opening another surface revokes the replacement review"
            );
            terminal
                .draw(|frame| render(frame, frame.area(), &app, &config, &paint))
                .unwrap();
            assert!(
                !app.parked_drafts.complete.get(),
                "an obscured review cannot reacquire consent"
            );
            app.update(Event::Resize(59, 17));
            assert!(!app.parked_drafts.complete.get());
            app.update(Event::Key(KeyEvent::new(
                KeyCode::Right,
                KeyModifiers::NONE,
            )));
            app.update(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )));
            assert_eq!((app.input.as_str(), app.cursor), ("current exact draft", 3));
            assert_eq!(app.workflows.parked_drafts.len(), 1);
        }
    }
}
