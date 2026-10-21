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
    identity: Option<(String, String, String)>,
    /// Byte position in the rendered text, independent of display-row reflow.
    anchor: Option<usize>,
    prefix_bytes: usize,
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
        let same_item = self.identity.as_ref().is_some_and(|(thread, turn, id)| {
            thread == &item.thread_id && turn == &item.turn_id && id == &item.item_id
        });
        if self.valid && same_item && self.width == width && self.verbose == verbose {
            self.selected = selected;
            return;
        }
        if !same_item {
            let (thread, turn, id) = self
                .identity
                .get_or_insert_with(|| (String::new(), String::new(), String::new()));
            thread.clone_from(&item.thread_id);
            turn.clone_from(&item.turn_id);
            id.clone_from(&item.item_id);
            self.anchor = None;
        }
        self.annotated.clear();
        if verbose {
            let _ = write!(
                self.annotated,
                "Source: {agent}\nThread: {}\nTurn: {}\nItem: {}\n\n{}",
                item.thread_id, item.turn_id, item.item_id, item.text
            );
        }
        let prefix_bytes = if verbose {
            self.annotated.len().saturating_sub(item.text.len())
        } else {
            0
        };
        if self.verbose != verbose {
            self.anchor = self.anchor.map(|byte| {
                if byte < self.prefix_bytes {
                    0
                } else {
                    byte - self.prefix_bytes + prefix_bytes
                }
            });
        }
        self.prefix_bytes = prefix_bytes;
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

    fn anchor_row(&self) -> Option<usize> {
        let byte = self.anchor?;
        Some(
            self.rows
                .partition_point(|range| range.start <= byte)
                .saturating_sub(1),
        )
    }

    fn pin(&mut self, row: usize) {
        self.anchor = self.rows.get(row).map(|range| range.start);
    }
}

impl State {
    /// The selected observation owns its original request even while a newer
    /// request is connecting or has failed without producing any output.
    pub fn response_request(&self) -> (&str, bool) {
        self.items
            .get(self.selected)
            .map_or((self.submitted_request.as_ref(), false), |item| {
                (
                    item.request.as_ref(),
                    !std::sync::Arc::ptr_eq(&item.request, &self.submitted_request),
                )
            })
    }

    /// Unclaimed strict admission needs a save barrier. Claimed requests retain
    /// their own immutable input and validate semantic committed facts.
    pub fn holds_committed_input(&self) -> bool {
        self.work.is_none() && self.active && !self.inspect_models && !self.inspect_sessions
    }

    pub fn response_visible(&self) -> bool {
        !self.items.is_empty()
            || !self.submitted_request.is_empty()
            || self.activity_label().is_some()
            || self.request.is_some()
    }

    pub fn home_response_key(&mut self, code: super::KeyCode) -> bool {
        self.response_key(code)
    }

    /// Called with the actual rendered body, excluding metadata and controls.
    /// The root renderer clears this before drawing so hidden views cannot hit.
    pub fn set_response_body(&mut self, body: ratatui::layout::Rect) {
        self.response_body = body;
    }

    pub fn response_wheel(&mut self, column: u16, row: u16, lines: i32) -> bool {
        let body = self.response_body;
        if column < body.x || column >= body.right() || row < body.y || row >= body.bottom() {
            return false;
        }
        self.scroll_response_lines(lines)
    }

    /// Scroll only the selected observation; reaching its end never resumes
    /// following or changes selection. No response text is copied here.
    pub fn scroll_response_lines(&mut self, delta: i32) -> bool {
        if self.items.get(self.selected).is_none() || self.response_height == 0 {
            return false;
        }
        self.following = false;
        self.response_scroll = if delta < 0 {
            self.response_scroll
                .saturating_sub(delta.unsigned_abs() as usize)
        } else {
            self.response_scroll
                .saturating_add(delta as usize)
                .min(self.response_max)
        };
        self.response_cache.pin(self.response_scroll);
        self.scroll = self.response_scroll.min(usize::from(u16::MAX)) as u16;
        true
    }

    /// One-based visible display rows and total, for a compact position label.
    pub fn response_position(&self) -> (usize, usize, usize) {
        let total = self.response_cache.rows.len();
        let start = if total == 0 || self.response_height == 0 {
            0
        } else {
            self.response_scroll + 1
        };
        (
            start,
            self.response_scroll
                .saturating_add(usize::from(self.response_height))
                .min(total),
            total,
        )
    }

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
                self.response_cache.anchor = Some(0);
            }
            KeyCode::PageUp | KeyCode::PageDown => {
                let page = i32::from(self.response_height.saturating_sub(1).max(1));
                return self.scroll_response_lines(if code == KeyCode::PageUp {
                    -page
                } else {
                    page
                });
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
                self.response_cache.anchor = None;
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
        self.response_height = height;
        if let Some(item) = item {
            self.response_cache
                .update(self.selected, width, item, self.agent.label(), verbose);
        } else {
            self.response_cache.rows.clear();
            self.response_cache.anchor = None;
        }
        self.response_max = self
            .response_cache
            .rows
            .len()
            .saturating_sub(usize::from(height));
        self.scroll_max = self.response_max.min(usize::from(u16::MAX)) as u16;
        if self.following && !self.new_output {
            self.response_scroll = self.response_max;
        } else {
            self.response_scroll = self
                .response_cache
                .anchor_row()
                .unwrap_or(self.response_scroll)
                .min(self.response_max);
        }
        self.scroll = self.response_scroll.min(usize::from(u16::MAX)) as u16;
        if self.following && !self.new_output {
            self.response_cache.pin(self.response_scroll);
        }
        let text = if verbose {
            self.response_cache.annotated.as_str()
        } else {
            item.map_or("", |item| item.text.as_str())
        };
        self.response_cache
            .rows
            .iter()
            .skip(self.response_scroll)
            .take(usize::from(height))
            .map(move |range| &text[range.clone()])
    }
}
