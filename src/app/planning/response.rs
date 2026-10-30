use std::{fmt::Write, ops::Range, sync::Arc};

use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;

use super::{Item, ItemKind, State};
use crate::messages::{Message, Template};

#[derive(Clone, PartialEq, Eq)]
struct Identity(String, String, String);

struct Row {
    range: Range<usize>,
    header: bool,
    spacer: bool,
}

/// A visible row borrows the cached header or original observation. File styling
/// follows the typed lifecycle, never provider-controlled text or code prefixes.
pub struct ResponseRow<'a> {
    pub text: &'a str,
    pub kind: ItemKind,
    pub header: bool,
    /// A cached projection of a qualified todo, never a text-prefix guess.
    pub todo: bool,
    /// Status of a readable tool projection; `None` for raw or non-tool rows.
    pub tool: Option<super::tool_view::Status>,
    /// Last visible source fragment of this actual file snapshot, not a timer.
    pub writing_edge: bool,
    /// Part of the file lines that arrived in the latest snapshot, within
    /// [`FRESH`]. Lets the UI flash real arrivals; content is never delayed.
    pub fresh: bool,
}

/// How long newly received file lines stay highlighted.
pub const FRESH: std::time::Duration = std::time::Duration::from_millis(480);

struct Segment {
    identity: Arc<Identity>,
    valid: bool,
    header: String,
    header_valid: bool,
    kind: ItemKind,
    rows: Vec<Row>,
    start: usize,
    observation_valid: bool,
    todo: Option<super::todo::Observation>,
    todo_updates: usize,
    /// Readable tool projection; the original text remains the verbose view.
    tool: Option<super::tool_view::View>,
    tool_shown: bool,
    /// Same owner and request as the previous segment: attribution is not
    /// repeated outside `/verbose`.
    continued: bool,
    /// Leading blank row before this segment.
    gap: bool,
    display: Option<String>,
    suppressed: bool,
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
    comfortable: bool,
    /// Tool rows live in the side panel instead of the transcript.
    dock_tools: bool,
    anchor: Option<Anchor>,
    total: usize,
    grouping_dirty: bool,
}

fn wrap(rows: &mut Vec<Row>, text: &str, width: usize, header: bool, spaced: bool) {
    let mut push = |range: Range<usize>| {
        if spaced
            && !range.is_empty()
            && rows
                .last()
                .is_some_and(|row| row.header == header && !row.range.is_empty())
        {
            rows.push(Row {
                range: range.start..range.start,
                header,
                spacer: true,
            });
        }
        rows.push(Row {
            range,
            header,
            spacer: false,
        });
    };
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
                push(base + start..base + end);
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
        push(base + start..base + line.len());
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
        self.grouping_dirty = true;
        if let Some(segment) = self.segments.get_mut(index) {
            segment.valid = false;
            segment.observation_valid = false;
        }
    }

    pub(super) fn settlement_changed(&mut self, index: usize) {
        self.grouping_dirty = true;
        if let Some(segment) = self.segments.get_mut(index) {
            segment.valid = false;
            segment.header_valid = false;
            segment.todo_updates = 0;
        }
    }

    pub(super) fn remove(&mut self, index: usize) {
        self.grouping_dirty = true;
        if index < self.segments.len() {
            self.segments.remove(index);
        }
    }

    pub(super) fn move_to_end(&mut self, index: usize) {
        self.grouping_dirty = true;
        if index < self.segments.len() {
            let segment = self.segments.remove(index);
            self.segments.push(segment);
        }
    }

    fn update(
        &mut self,
        width: u16,
        items: &[Item],
        agent: &str,
        verbose: bool,
        comfortable: bool,
        dock_tools: bool,
    ) {
        let mut regroup = self.grouping_dirty
            || self.verbose != verbose
            || self.dock_tools != dock_tools
            || self.segments.len() != items.len();
        let attributed = items.len() > 1;
        let reflow = self.width != width
            || self.verbose != verbose
            || self.attributed != attributed
            || self.comfortable != comfortable;
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
                    observation_valid: false,
                    todo: None,
                    todo_updates: 0,
                    tool: None,
                    tool_shown: false,
                    continued: false,
                    gap: false,
                    display: None,
                    suppressed: false,
                };
                if index < self.segments.len() {
                    self.segments[index] = segment;
                } else {
                    self.segments.push(segment);
                }
            }
            let segment = &mut self.segments[index];
            if !segment.observation_valid {
                regroup = true;
                segment.todo = super::todo::Observation::read(item);
                segment.tool = segment
                    .todo
                    .is_none()
                    .then(|| super::tool_view::View::read(item))
                    .flatten();
                segment.observation_valid = true;
                segment.display = None;
                segment.header_valid = false;
                segment.todo_updates = 0;
                segment.tool_shown = false;
            }
        }
        // Collapse only qualified successful todo snapshots within one immutable
        // request/owner/worker. Errors, malformed data and ordinary tools remain.
        let mut previous: Option<usize> = None;
        for (index, item) in items.iter().enumerate() {
            if regroup {
                let todo = self.segments[index].todo.as_ref();
                let suppressed = !verbose
                    && todo.is_some_and(|todo| {
                        self.segments[index + 1..]
                            .iter()
                            .zip(&items[index + 1..])
                            .any(|(next, other)| {
                                next.todo
                                    .as_ref()
                                    .is_some_and(|next| next.source == todo.source)
                                    && super::todo::same_request(item, other)
                            })
                    });
                let updates = if !verbose && !suppressed {
                    todo.map_or(0, |todo| {
                        self.segments[..=index]
                            .iter()
                            .zip(&items[..=index])
                            .filter(|(previous, other)| {
                                previous
                                    .todo
                                    .as_ref()
                                    .is_some_and(|previous| previous.source == todo.source)
                                    && super::todo::same_request(item, other)
                            })
                            .count()
                    })
                } else {
                    0
                };
                let segment = &mut self.segments[index];
                let settled = item
                    .status
                    .as_ref()
                    .is_some_and(|status| status.outcome.is_some());
                let tool_shown = !verbose && segment.tool.is_some();
                // A docked tool is listed in the side panel; a request outcome
                // attached to it always stays in the transcript.
                let suppressed = suppressed || tool_shown && dock_tools && !settled;
                if segment.suppressed != suppressed
                    || segment.todo_updates != updates
                    || segment.tool_shown != tool_shown
                {
                    segment.valid = false;
                    segment.header_valid = false;
                    segment.suppressed = suppressed;
                    segment.todo_updates = updates;
                    segment.tool_shown = tool_shown;
                    segment.display = if updates > 0 {
                        segment
                            .todo
                            .as_ref()
                            .map(|todo| todo.display(updates, item))
                    } else if tool_shown {
                        segment.tool.as_ref().map(super::tool_view::View::display)
                    } else {
                        None
                    };
                }
            }
            // Compare with the previous visible segment: docked tools and
            // collapsed todo snapshots do not break a run of one request.
            // The first segment is introduced by the response's own Request
            // and status rows above the transcript.
            let continued = !verbose
                && previous.is_none_or(|before| {
                    items[before].owner == item.owner && items[before].request == item.request
                });
            // One blank row separates attributed blocks and a change between
            // tool rows and prose, so segments do not run together.
            let gap = previous.is_some_and(|before| {
                !continued || (self.segments[before].tool_shown != self.segments[index].tool_shown)
            });
            let segment = &mut self.segments[index];
            if segment.continued != continued || segment.gap != gap {
                segment.continued = continued;
                segment.gap = gap;
                segment.valid = false;
                segment.header_valid = false;
            }
            segment.start = self.total;
            if segment.suppressed {
                segment.rows.clear();
                continue;
            }
            previous = Some(index);
            if reflow || !segment.valid {
                if !segment.header_valid
                    || segment.kind != item.kind
                    || self.verbose != verbose
                    || self.attributed != attributed
                {
                    segment.header.clear();
                    let settled = item
                        .status
                        .as_ref()
                        .is_some_and(|status| status.outcome.is_some());
                    if segment.display.is_some() && !(segment.tool_shown && settled) {
                        // The structured summary already names source and owner;
                        // a readable tool row belongs to the surrounding response.
                    } else if let ItemKind::File(phase) = item.kind {
                        let _ = write!(
                            segment.header,
                            "{} {}",
                            Message::ProvidersPlanningFileLabel.text(),
                            super::file_phase_message(phase).text(),
                        );
                    } else if attributed && !segment.continued
                        || item
                            .status
                            .as_ref()
                            .is_some_and(|status| status.outcome.is_some())
                    {
                        let kind = if item.text.is_empty()
                            && item
                                .status
                                .as_ref()
                                .is_some_and(|status| status.outcome.is_some())
                        {
                            "Request outcome"
                        } else {
                            (match item.kind {
                                ItemKind::Message => Message::ProvidersPlanningResponse,
                                ItemKind::Plan => Message::ProvidersPlanningPlan,
                                ItemKind::Tool => Message::ProvidersPlanningToolOutput,
                                ItemKind::File(phase) => super::file_phase_message(phase),
                            })
                            .text()
                        };
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
                        let id = if verbose {
                            format!(" · {}", crate::workspace::handoff::sanitize(&item.item_id))
                        } else {
                            String::new()
                        };
                        segment.header = Template::ProvidersPlanningTranscriptSegment
                            .format(&[
                                ("kind", kind),
                                ("owner", &owner),
                                ("item", &id),
                                ("request", &request),
                            ])
                            .expect("validated transcript template");
                    }
                    if (segment.display.is_none() || segment.tool_shown)
                        && let Some(status) = &item.status
                        && let Some(outcome) = status.outcome_label()
                    {
                        let context = match status.public_context {
                            Some(true) => {
                                "Public context: admitted at settlement (bounded completed pairs)"
                            }
                            Some(false) => "Public context: excluded from follow-up",
                            None => "Provider context: opaque; retention unknown",
                        };
                        let _ = write!(
                            segment.header,
                            "\nRequest outcome: {outcome}\nReason: {}\n{context}\n",
                            status.reason,
                        );
                        // Worker/transport identities help debugging, not reading
                        // the answer; /verbose shows them. The original model
                        // stays visible so an old answer is never relabelled.
                        if !verbose {
                            let _ = writeln!(
                                segment.header,
                                "Model: {}",
                                status
                                    .model
                                    .as_deref()
                                    .unwrap_or("provider default / unknown"),
                            );
                        } else {
                            let _ = writeln!(
                                segment.header,
                                "Origin: {} {:?} / task {} / worker generation {} / model {}",
                                status.agent.label(),
                                status.operation,
                                status.task_id,
                                status.generation,
                                status
                                    .model
                                    .as_deref()
                                    .unwrap_or("provider default / unknown"),
                            );
                        }
                        if let Some(wire) = status.transport.filter(|_| verbose) {
                            let wire = match wire {
                                crate::providers::NativeWire::Http => "HTTP",
                                crate::providers::NativeWire::WebSocket => "WebSocket",
                            };
                            let _ = writeln!(
                                segment.header,
                                "Prepared native transport: {wire}; connection/retention not inferred; no provider thread",
                            );
                        }
                    }
                    if verbose && !item.text.is_empty() && !matches!(item.kind, ItemKind::File(_)) {
                        let _ = write!(
                            segment.header,
                            "\nSource: {}\nThread: {}\nTurn: {}\nItem: {}\n",
                            item.status
                                .as_ref()
                                .map_or(agent, |status| status.agent.label()),
                            item.thread_id,
                            item.turn_id,
                            item.item_id
                        );
                    }
                    segment.kind = item.kind;
                    segment.header_valid = true;
                }
                let content = segment.display.as_deref().unwrap_or(&item.text);
                segment.rows.clear();
                // File cards paint a gutter on every row, so they never get a gap.
                if segment.gap && !matches!(item.kind, ItemKind::File(_)) {
                    segment.rows.push(Row {
                        range: 0..0,
                        header: false,
                        spacer: true,
                    });
                }
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
                        false,
                    );
                }
                if comfortable
                    && matches!(item.kind, ItemKind::Message | ItemKind::Plan)
                    && !segment.header.is_empty()
                    && !content.is_empty()
                    && segment.display.is_none()
                {
                    segment.rows.push(Row {
                        range: 0..0,
                        header: false,
                        spacer: true,
                    });
                }
                wrap(
                    &mut segment.rows,
                    content,
                    usize::from(content_width),
                    false,
                    comfortable
                        && segment.display.is_none()
                        && matches!(item.kind, ItemKind::Message | ItemKind::Plan),
                );
                segment.valid = true;
            }
            self.total += segment.rows.len();
        }
        self.grouping_dirty = false;
        self.width = width;
        self.verbose = verbose;
        self.attributed = attributed;
        self.comfortable = comfortable;
        self.dock_tools = dock_tools;
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
            if !row.spacer && row.header == anchor.header {
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
        if let Some(row) = segment
            .rows
            .iter()
            .skip(row.saturating_sub(segment.start))
            .find(|row| !row.spacer)
        {
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
    pub(super) fn request_status(
        &self,
        generation: u64,
        task_id: i64,
    ) -> Arc<super::RequestStatus> {
        self.items
            .iter()
            .filter(|item| Arc::ptr_eq(&item.request, &self.submitted_request))
            .filter_map(|item| item.status.as_ref())
            .find(|status| status.generation == generation && status.task_id == task_id)
            .cloned()
            .unwrap_or_else(|| {
                Arc::new(super::RequestStatus {
                    generation,
                    task_id,
                    agent: self.conversation_flow.agent.unwrap_or(self.agent),
                    operation: self.conversation_flow.operation.unwrap_or(self.operation),
                    model: self.request_selection.model.clone(),
                    transport: self.conversation_flow.transport,
                    outcome: None,
                    reason: String::new(),
                    public_context: None,
                })
            })
    }

    pub(super) fn retain_terminal(
        &mut self,
        generation: u64,
        task_id: i64,
        outcome: super::Outcome,
        message: &str,
        public_admitted: bool,
    ) {
        let origin = self.request_status(generation, task_id);
        let mut status = origin.as_ref().clone();
        status.outcome = Some(outcome);
        status.model = self.observed_model.clone().or(status.model);
        status.reason = crate::workspace::handoff::sanitize(message);
        if self.unknown_effects() {
            status.reason.push_str(" Local effects remain unknown; inspect the owning journal before starting more work.");
        }
        const REASON_BYTES: usize = 1024;
        if status.reason.len() > REASON_BYTES {
            let mut boundary = REASON_BYTES - " [truncated]".len();
            while !status.reason.is_char_boundary(boundary) {
                boundary -= 1;
            }
            status.reason.truncate(boundary);
            status.reason.push_str(" [truncated]");
        }
        status.public_context = (status.agent.is_direct()
            || (status.agent == super::Agent::Codex
                && status.operation == super::Operation::Implement))
            .then_some(public_admitted);
        let status = Arc::new(status);
        let mut found = false;
        for (index, item) in self.items.iter_mut().enumerate() {
            if Arc::ptr_eq(&item.request, &self.submitted_request)
                && item.status.as_ref().is_some_and(|origin| {
                    origin.generation == generation && origin.task_id == task_id
                })
            {
                item.status = Some(Arc::clone(&status));
                self.response_cache.settlement_changed(index);
                found = true;
            }
        }
        if !found {
            // A local terminal-only card preserves failures before the first
            // provider item. Its empty text is never a synthetic assistant turn.
            if self.items.len() == super::ITEM_LIMIT {
                let removed = usize::from(
                    self.selected == 0 && (!self.following || !self.response_surface()),
                );
                self.items.remove(removed);
                if self.selected > removed {
                    self.selected -= 1;
                }
                self.response_cache.remove(removed);
                self.omitted += 1;
            }
            self.items.push(Item {
                thread_id: "local-request-outcome".into(),
                turn_id: generation.to_string(),
                item_id: "request-outcome".into(),
                kind: ItemKind::Message,
                text: String::new(),
                complete: false,
                request: Arc::clone(&self.submitted_request),
                owner: self.owner().cloned(),
                status: Some(status),
            });
            self.response_arrived(self.items.len() - 1);
        }
    }

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
    /// Whether the side Tools panel lists tool rows instead of the transcript.
    pub fn set_tools_docked(&mut self, docked: bool) {
        self.tools_docked = docked;
    }

    /// Readable tool calls, oldest first, from the cached transcript layout.
    /// Todo snapshots stay in the transcript's progress projection.
    pub fn tool_activity(&self) -> Vec<&super::tool_view::View> {
        self.response_cache
            .segments
            .iter()
            .filter_map(|segment| segment.tool.as_ref())
            .collect()
    }

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
        // Tight viewports keep the same content budget for safety/status and editing.
        let comfortable = self.text_density() == crate::store::TextDensity::Comfortable
            && width >= 20
            && height >= 4;
        // Output that arrived behind a review while the user never scrolled
        // away is caught up as soon as the transcript is drawn again, so live
        // code keeps following instead of stopping at the review's arrival.
        if self.following && self.new_output {
            self.new_output = false;
            if let Some(latest) = self.newest_response() {
                self.selected = latest;
            }
        }
        let dock_tools = self.tools_docked;
        self.response_cache.update(
            width,
            &self.items,
            self.agent.label(),
            verbose,
            comfortable,
            dock_tools,
        );
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
        let growth = &self.file_growth;
        let now = std::time::Instant::now();
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
                        segment.display.as_deref().unwrap_or(&items[index].text)
                    };
                    ResponseRow {
                        text: &text[row.range.clone()],
                        kind: segment.kind,
                        header: row.header,
                        todo: segment.display.is_some() && !segment.tool_shown,
                        tool: segment
                            .tool
                            .as_ref()
                            .filter(|_| segment.tool_shown)
                            .map(|tool| tool.status),
                        writing_edge: segment.kind
                            == ItemKind::File(crate::providers::FilePhase::Generating)
                            && !row.header
                            && text.contains('\n')
                            && row.range.end == text.trim_end_matches('\n').len(),
                        fresh: matches!(segment.kind, ItemKind::File(_))
                            && !row.header
                            && growth.get(&items[index].item_id).is_some_and(|(from, at)| {
                                row.range.end > *from && now.duration_since(*at) < FRESH
                            }),
                    }
                })
            })
            .skip(skip)
            .take(usize::from(height))
    }
}
