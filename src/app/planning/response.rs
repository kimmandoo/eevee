use std::{fmt::Write, ops::Range};

use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;

use super::{Item, ItemKind, State};

/// Byte ranges borrow the selected item at render time. Reflow only on an
/// observed item update, selection change or resize, never on spinner ticks.
#[derive(Default)]
pub(super) struct Layout {
    valid: bool,
    selected: usize,
    width: u16,
    rows: Vec<Range<usize>>,
    verbose: bool,
    annotated: String,
}

impl Layout {
    pub(super) fn invalidate(&mut self) {
        self.valid = false;
    }

    pub(super) fn item_changed(&mut self, index: usize) {
        if self.selected == index {
            self.invalidate();
        }
    }

    fn update(&mut self, selected: usize, width: u16, item: &Item, agent: &str, verbose: bool) {
        if self.valid && self.selected == selected && self.width == width && self.verbose == verbose
        {
            return;
        }
        self.annotated.clear();
        if verbose {
            let _ = write!(
                self.annotated,
                "Source: {agent}\nThread: {}\nTurn: {}\nItem: {}\n\n{}",
                item.thread_id, item.turn_id, item.item_id, item.text
            );
        }
        let text = if verbose {
            self.annotated.as_str()
        } else {
            item.text.as_str()
        };
        self.verbose = verbose;
        self.rows.clear();
        let width = usize::from(width.max(1));
        let mut base = 0;
        for line in text.split('\n') {
            let mut start = 0;
            let mut used = 0;
            let mut word_break = None;
            let mut cursor = 0;
            while cursor < line.len() {
                let grapheme = line[cursor..]
                    .graphemes(true)
                    .next()
                    .expect("remaining grapheme");
                let size = Span::raw(grapheme).width();
                if used + size > width && cursor > start {
                    let end = word_break.filter(|end| *end > start).unwrap_or(cursor);
                    self.rows.push(base + start..base + end);
                    start = end;
                    cursor = end;
                    used = 0;
                    word_break = None;
                    continue;
                }
                cursor += grapheme.len();
                used += size;
                if grapheme.chars().all(char::is_whitespace) {
                    word_break = Some(cursor);
                }
            }
            self.rows.push(base + start..base + line.len());
            base += line.len() + 1;
        }
        self.selected = selected;
        self.width = width as u16;
        self.valid = true;
    }
}

impl State {
    /// Genuine in-flight work, excluding every local decision/input surface.
    pub fn activity_label(&self) -> Option<&'static str> {
        if self.request.is_some()
            || self.preview.is_some()
            || self.picker.is_some()
            || self.quit_review
            || self.journal_mode
            || self.switch_to.is_some()
            || self.paths_editor.is_some()
            || self.selection_change.is_some()
        {
            return None;
        }
        if self.cancelling {
            Some("Cancelling")
        } else if self.awaiting_preview {
            Some("Preparing request")
        } else if self.active {
            Some(if self.inspect_models {
                "Loading models"
            } else if self.inspect_sessions {
                "Inspecting sessions"
            } else {
                self.activity
            })
        } else {
            None
        }
    }

    pub(super) fn response_surface(&self) -> bool {
        self.preview.is_none()
            && self.request.is_none()
            && !self.awaiting_preview
            && !self.quit_review
            && !self.journal_mode
            && self.switch_to.is_none()
            && self.paths_editor.is_none()
            && self.picker.is_none()
            && self.selection_change.is_none()
            && self.inspection.is_none()
            && !self.inspect_models
            && !self.inspect_sessions
    }

    fn newest_response(&self) -> Option<usize> {
        self.items
            .iter()
            .rposition(|item| matches!(item.kind, ItemKind::Message | ItemKind::Plan))
            .or_else(|| self.items.len().checked_sub(1))
    }

    pub(super) fn response_arrived(&mut self, index: usize) {
        if self.following && self.response_surface() {
            if let Some(latest) = self.newest_response() {
                self.selected = latest;
            }
            self.new_output = index != self.selected;
        } else {
            self.new_output = true;
        }
    }

    pub(super) fn resume_follow(&mut self) {
        self.following = true;
        self.new_output = false;
        if let Some(latest) = self.newest_response() {
            self.selected = latest;
        }
    }

    pub(super) fn response_key(&mut self, code: super::KeyCode) -> bool {
        use super::KeyCode;
        match code {
            KeyCode::End => self.resume_follow(),
            KeyCode::Home => {
                self.following = false;
                self.response_scroll = 0;
            }
            KeyCode::PageUp => {
                self.following = false;
                self.response_scroll = self.response_scroll.saturating_sub(5);
            }
            KeyCode::PageDown => {
                self.following = false;
                self.response_scroll = self
                    .response_scroll
                    .saturating_add(5)
                    .min(self.response_max);
            }
            KeyCode::Up | KeyCode::Down => {
                self.following = false;
                self.selected = if code == KeyCode::Up {
                    self.selected.saturating_sub(1)
                } else {
                    (self.selected + 1).min(self.items.len().saturating_sub(1))
                };
                self.response_scroll = 0;
                self.response_max = 0;
            }
            _ => return false,
        }
        self.scroll = self.response_scroll.min(usize::from(u16::MAX)) as u16;
        true
    }

    /// Reflow once per changed response and borrow only visible rows. Scrolling
    /// remains in display rows, matching the review surfaces' existing keys.
    pub fn response_rows(
        &mut self,
        width: u16,
        height: u16,
        verbose: bool,
    ) -> impl Iterator<Item = &str> {
        let item = self.items.get(self.selected);
        if let Some(item) = item {
            self.response_cache
                .update(self.selected, width, item, self.agent.label(), verbose);
        } else {
            self.response_cache.rows.clear();
        }
        let text = if verbose {
            self.response_cache.annotated.as_str()
        } else {
            item.map_or("", |item| item.text.as_str())
        };
        self.response_max = self
            .response_cache
            .rows
            .len()
            .saturating_sub(usize::from(height));
        self.scroll_max = self.response_max.min(usize::from(u16::MAX)) as u16;
        if self.following && !self.new_output {
            self.response_scroll = self.response_max;
        } else {
            self.response_scroll = self.response_scroll.min(self.response_max);
        }
        self.scroll = self.response_scroll.min(usize::from(u16::MAX)) as u16;
        self.response_cache
            .rows
            .iter()
            .skip(self.response_scroll)
            .take(usize::from(height))
            .map(move |range| &text[range.clone()])
    }
}
