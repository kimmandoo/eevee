use std::{fmt::Write, ops::Range, sync::Arc};

use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;

use super::{Item, ItemKind, State};
use crate::i18n::{Language, Message, Template};

#[derive(Clone, PartialEq, Eq)]
struct Identity(String, String, String);

struct Row {
    range: Range<usize>,
    header: bool,
}

/// A visible row borrows the cached header or original observation. File styling
/// follows the typed lifecycle, never provider-controlled text or code prefixes.
pub struct ResponseRow<'a> {
    pub text: &'a str,
    pub kind: ItemKind,
    pub header: bool,
}

struct Segment {
    identity: Arc<Identity>,
    valid: bool,
    header: String,
    header_valid: bool,
    kind: ItemKind,
    rows: Vec<Row>,
    start: usize,
}

struct Anchor {
    identity: Arc<Identity>,
    byte: usize,
    header: bool,
}

/// One bounded transcript; text rows borrow their original observations. Only
/// changed segments reflow, while anchors use identity/bytes rather than indices.
#[derive(Default)]
pub(super) struct Layout {
    segments: Vec<Segment>,
    width: u16,
    verbose: bool,
    attributed: bool,
    language: Option<Language>,
    anchor: Option<Anchor>,
    total: usize,
}

fn wrap(rows: &mut Vec<Row>, text: &str, width: usize, header: bool) {
    let mut base = 0;
    for line in text.split_terminator('\n') {
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
                rows.push(Row {
                    range: base + start..base + end,
                    header,
                });
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
        rows.push(Row {
            range: base + start..base + line.len(),
            header,
        });
        base += line.len() + 1;
    }
}

impl Layout {
    pub(super) fn invalidate(&mut self) {
        for segment in &mut self.segments {
            segment.valid = false;
            segment.header_valid = false;
        }
    }

    pub(super) fn item_changed(&mut self, index: usize) {
        if let Some(segment) = self.segments.get_mut(index) {
            segment.valid = false;
        }
    }

    pub(super) fn remove(&mut self, index: usize) {
        if index < self.segments.len() {
            self.segments.remove(index);
        }
    }

    pub(super) fn move_to_end(&mut self, index: usize) {
        if index < self.segments.len() {
            let segment = self.segments.remove(index);
            self.segments.push(segment);
        }
    }

    fn update(&mut self, width: u16, items: &[Item], agent: &str, verbose: bool) {
        let attributed = items.len() > 1;
        let reflow =
            self.width != width || self.verbose != verbose || self.attributed != attributed;
        self.segments.truncate(items.len());
        self.total = 0;
        for (index, item) in items.iter().enumerate() {
            let matches = |segment: &Segment| {
                let Identity(thread, turn, id) = segment.identity.as_ref();
                thread == &item.thread_id && turn == &item.turn_id && id == &item.item_id
            };
            if self
                .segments
                .get(index)
                .is_none_or(|segment| !matches(segment))
            {
                let segment = Segment {
                    identity: Arc::new(Identity(
                        item.thread_id.clone(),
                        item.turn_id.clone(),
                        item.item_id.clone(),
                    )),
                    valid: false,
                    header: String::new(),
                    header_valid: false,
                    kind: item.kind,
                    rows: Vec::new(),
                    start: 0,
                };
                if index < self.segments.len() {
                    self.segments[index] = segment;
                } else {
                    self.segments.push(segment);
                }
            }
            let segment = &mut self.segments[index];
            segment.start = self.total;
            if reflow || !segment.valid {
                if !segment.header_valid
                    || segment.kind != item.kind
                    || self.verbose != verbose
                    || self.attributed != attributed
                {
                    segment.header.clear();
                    if let ItemKind::File(phase) = item.kind {
                        let language = self.language.unwrap_or_else(|| Language::from_locale("en"));
                        let _ = write!(
                            segment.header,
                            "{} {}",
                            language.text(Message::ProvidersPlanningFileLabel),
                            language.text(super::file_phase_message(phase)),
                        );
                    } else if attributed {
                        let language = self.language.unwrap_or_else(|| Language::from_locale("en"));
                        let kind = language.text(match item.kind {
                            ItemKind::Message => Message::ProvidersPlanningResponse,
                            ItemKind::Plan => Message::ProvidersPlanningPlan,
                            ItemKind::Tool => Message::ProvidersPlanningToolOutput,
                            ItemKind::File(phase) => super::file_phase_message(phase),
                        });
                        let owner = crate::store::Attribution::label(item.owner.as_ref());
                        // Labels are bounded display previews, never outgoing input.
                        let redacted = crate::workspace::privacy::redact(item.request.as_ref());
                        let request: String = redacted
                            .graphemes(true)
                            .take(96)
                            .flat_map(str::chars)
                            .map(|ch| {
                                if matches!(ch, '\n' | '\r' | '\t') {
                                    ' '
                                } else {
                                    ch
                                }
                            })
                            .collect();
                        let owner = crate::workspace::handoff::sanitize(&owner);
                        let id = crate::workspace::handoff::sanitize(&item.item_id);
                        segment.header = language
                            .format(
                                Template::ProvidersPlanningTranscriptSegment,
                                &[
                                    ("kind", kind),
                                    ("owner", &owner),
                                    ("item", &id),
                                    ("request", &request),
                                ],
                            )
                            .expect("validated transcript template");
                    }
                    if verbose && !matches!(item.kind, ItemKind::File(_)) {
                        let _ = write!(
                            segment.header,
                            "\nSource: {agent}\nThread: {}\nTurn: {}\nItem: {}\n",
                            item.thread_id, item.turn_id, item.item_id
                        );
                    }
                    segment.kind = item.kind;
                    segment.header_valid = true;
                }
                segment.rows.clear();
                // The file-only panel gutter is painted separately on every row,
                // including wrapped continuations and a viewport starting mid-code.
                let content_width = if matches!(item.kind, ItemKind::File(_)) {
                    width.saturating_sub(2).max(1)
                } else {
                    width.max(1)
                };
                if !segment.header.is_empty() {
                    wrap(
                        &mut segment.rows,
                        &segment.header,
                        usize::from(content_width),
                        true,
                    );
                }
                wrap(
                    &mut segment.rows,
                    &item.text,
                    usize::from(content_width),
                    false,
                );
                segment.valid = true;
            }
            self.total += segment.rows.len();
        }
        self.width = width;
        self.verbose = verbose;
        self.attributed = attributed;
    }

    fn segment_at(&self, row: usize) -> Option<usize> {
        (!self.segments.is_empty()).then(|| {
            self.segments
                .partition_point(|segment| segment.start <= row)
                .saturating_sub(1)
        })
    }

    fn anchor_segment(&self) -> Option<usize> {
        let anchor = self.anchor.as_ref()?;
        self.segments
            .iter()
            .position(|segment| segment.identity == anchor.identity)
    }

    fn anchor_row(&self) -> Option<usize> {
        let anchor = self.anchor.as_ref()?;
        let segment = self
            .segments
            .iter()
            .find(|segment| segment.identity == anchor.identity)?;
        let mut fallback = None;
        for (index, row) in segment.rows.iter().enumerate() {
            if row.header == anchor.header {
                fallback = Some(segment.start + index);
                if row.range.end > anchor.byte || row.range.start >= anchor.byte {
                    return fallback;
                }
            }
        }
        fallback.or(Some(segment.start))
    }

    fn pin(&mut self, row: usize) {
        let Some(index) = self.segment_at(row) else {
            return;
        };
        let segment = &self.segments[index];
        if let Some(row) = segment.rows.get(row.saturating_sub(segment.start)) {
            self.anchor = Some(Anchor {
                identity: Arc::clone(&segment.identity),
                byte: row.range.start,
                header: row.header,
            });
        }
    }

    fn select(&mut self, index: usize, item: &Item) {
        let identity = self
            .segments
            .get(index)
            .filter(|segment| {
                let Identity(thread, turn, id) = segment.identity.as_ref();
                thread == &item.thread_id && turn == &item.turn_id && id == &item.item_id
            })
            .map(|segment| Arc::clone(&segment.identity))
            .unwrap_or_else(|| {
                Arc::new(Identity(
                    item.thread_id.clone(),
                    item.turn_id.clone(),
                    item.item_id.clone(),
                ))
            });
        self.anchor = Some(Anchor {
            identity,
            byte: 0,
            header: false,
        });
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

    /// Real file output takes viewport priority over decorative transition chrome.
    pub fn has_current_file_output(&self) -> bool {
        self.items.iter().any(|item| {
            matches!(item.kind, ItemKind::File(_))
                && std::sync::Arc::ptr_eq(&item.request, &self.submitted_request)
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

    /// Traverse the whole retained transcript without implicitly resuming follow.
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
        if let Some(index) = self.response_cache.segment_at(self.response_scroll) {
            self.selected = index;
        }
        self.scroll = self.response_scroll.min(usize::from(u16::MAX)) as u16;
        true
    }

    /// One-based visible display rows and total, for a compact position label.
    pub fn response_position(&self) -> (usize, usize, usize) {
        let total = self.response_cache.total;
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
        self.items.len().checked_sub(1)
    }

    pub(super) fn response_arrived(&mut self, _index: usize) {
        if self.following && self.response_surface() {
            if let Some(latest) = self.newest_response() {
                self.selected = latest;
            }
            self.new_output = false;
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
                self.selected = 0;
                self.response_cache.pin(0);
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
                if let Some(item) = self.items.get(self.selected) {
                    self.response_cache.select(self.selected, item);
                }
                self.response_scroll = self.response_cache.anchor_row().unwrap_or(0);
            }
            _ => return false,
        }
        self.scroll = self.response_scroll.min(usize::from(u16::MAX)) as u16;
        true
    }

    pub fn set_response_language(&mut self, language: Language) {
        if self.response_cache.language != Some(language) {
            self.response_cache.language = Some(language);
            self.response_cache.invalidate();
        }
    }

    /// Text-only test access to the same cached chronological viewport.
    #[cfg(test)]
    pub fn response_rows(
        &mut self,
        width: u16,
        height: u16,
        verbose: bool,
    ) -> impl Iterator<Item = &str> {
        self.response_display_rows(width, height, verbose)
            .map(|row| row.text)
    }

    /// Borrow visible ranges with structural metadata, without copying output.
    pub fn response_display_rows(
        &mut self,
        width: u16,
        height: u16,
        verbose: bool,
    ) -> impl Iterator<Item = ResponseRow<'_>> {
        self.response_height = height;
        self.response_cache
            .update(width, &self.items, self.agent.label(), verbose);
        self.response_max = self
            .response_cache
            .total
            .saturating_sub(usize::from(height));
        self.scroll_max = self.response_max.min(usize::from(u16::MAX)) as u16;
        if self.following && !self.new_output {
            self.response_scroll = self.response_max;
        } else {
            if self.response_cache.anchor_row().is_none()
                && let Some(item) = self.items.get(self.selected)
            {
                self.response_cache.select(self.selected, item);
            }
            self.response_scroll = self
                .response_cache
                .anchor_row()
                .unwrap_or(0)
                .min(self.response_max);
        }
        self.scroll = self.response_scroll.min(usize::from(u16::MAX)) as u16;
        if self.following && !self.new_output {
            self.response_cache.pin(self.response_scroll);
        }
        if !self.following
            && let Some(index) = self.response_cache.anchor_segment()
        {
            self.selected = index;
        }
        let start = self
            .response_cache
            .segment_at(self.response_scroll)
            .unwrap_or(0);
        let skip = self
            .response_cache
            .segments
            .get(start)
            .map_or(0, |segment| {
                self.response_scroll.saturating_sub(segment.start)
            });
        let items = &self.items;
        self.response_cache
            .segments
            .iter()
            .enumerate()
            .skip(start)
            .flat_map(move |(index, segment)| {
                segment.rows.iter().map(move |row| {
                    let text = if row.header {
                        &segment.header
                    } else {
                        &items[index].text
                    };
                    ResponseRow {
                        text: &text[row.range.clone()],
                        kind: segment.kind,
                        header: row.header,
                    }
                })
            })
            .skip(skip)
            .take(usize::from(height))
    }
}
