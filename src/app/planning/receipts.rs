//! Restart-safe outgoing receipt inspector; persistence never resubmits a request.
use crate::{
    messages::{Message, Template},
    providers, store,
    workspace::receipt::Outgoing,
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::collections::VecDeque;
use store::receipts::{Operation, Request, Response, View};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Discard {
    record: crate::workspace::receipt::Durable,
    error: String,
}

#[derive(Default)]
pub struct State {
    staged: Option<(u64, Outgoing)>,
    queue: VecDeque<Request>,
    inflight: Option<Request>,
    failed: VecDeque<Request>,
    rollback: std::collections::BTreeMap<String, String>,
    terminal: Option<(store::WorkClaim, u64)>,
    discard: Option<Discard>,
    discard_seen: usize,
    gap: Option<crate::workspace::receipt::Durable>,
    gap_count: usize,
    pub error: Option<String>,
    task: i64,
    partner: i64,
    records: Vec<View>,
    selected: usize,
    source: Option<(String, String)>,
    confirm: Option<String>,
    affirmative: bool,
    rendered: bool,
    review_size: Option<(u16, u16)>,
    scroll: usize,
    page: usize,
    max_scroll: usize,
    notice: Option<Message>,
    display: String,
    lines: Vec<(usize, usize)>,
    width: u16,
    dirty: bool,
}

#[derive(Default)]
pub(super) struct Metadata<'a> {
    pub selection: Option<&'a providers::ModelSelection>,
    pub effort: Option<&'a providers::effort::Resolution>,
    pub progress: Option<&'a str>,
}

impl State {
    /// Freezes only local prepared bytes. No durable receipt exists until the
    /// exact generation/request receives the transport submission event.
    pub(super) fn stage(
        &mut self,
        preview: &providers::Preview,
        claim: &store::WorkClaim,
        agent: providers::Agent,
        operation: providers::Operation,
        metadata: Metadata<'_>,
    ) -> Result<(), String> {
        self.staged = None;
        self.terminal = None;
        if preview.task_id != claim.task_id {
            return Err("Receipt preview belongs to a different task".into());
        }
        if !preview.blockers.is_empty() {
            return Ok(());
        }
        let mut outgoing = Outgoing::frozen(
            &preview.payload,
            claim,
            preview.generation,
            agent,
            operation,
            0,
        )?;
        outgoing.record.selection = metadata.selection.cloned();
        outgoing.record.effort = metadata.effort.cloned();
        if metadata.effort.is_some()
            && let Some(selection) = &mut outgoing.record.selection
        {
            selection.effort = Some("auto".into());
        }
        outgoing.record.progress = metadata
            .progress
            .map(|progress| crate::workspace::privacy::redact(progress).into_owned());
        outgoing.record.validate()?;
        self.staged = Some((preview.generation, outgoing));
        Ok(())
    }

    pub fn dispatched(&mut self, generation: u64, task_id: i64, request_id: &str) {
        let matching = self.staged.as_ref().is_some_and(|(g, o)| {
            *g == generation && o.record.task_id == task_id && o.record.request_id == request_id
        });
        if !matching {
            return;
        }
        let (_, mut outgoing) = self.staged.take().expect("matched staged receipt");
        outgoing.record.recorded_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis().min(i64::MAX as u128) as i64);
        self.queue.push_back(Request {
            task_id,
            partner: outgoing.record.owner.partner,
            operation: Operation::Put(Box::new(outgoing)),
        });
    }

    pub fn pending(&self) -> bool {
        !self.queue.is_empty()
            || self.inflight.is_some()
            || self
                .failed
                .iter()
                .any(|r| matches!(r.operation, Operation::Put(_)))
    }

    pub fn try_store(&mut self, worker: &store::Worker) -> bool {
        if self.inflight.is_some()
            || self
                .failed
                .iter()
                .any(|r| matches!(r.operation, Operation::Put(_)))
        {
            return false;
        }
        let Some(request) = self.queue.pop_front() else {
            return false;
        };
        let expected = request_identity(&request);
        let mut pending = Some(request);
        if worker.try_receipts(&mut pending) {
            self.inflight = Some(expected);
            true
        } else {
            self.queue
                .push_front(pending.expect("unaccepted receipt retained"));
            false
        }
    }

    pub fn event(&mut self, request: Request, result: Result<Response, String>) {
        if self.inflight.as_ref() != Some(&request_identity(&request)) {
            return;
        }
        self.inflight = None;
        if let Operation::Put(outgoing) = &request.operation {
            self.rollback.remove(&outgoing.record.request_id);
            if let Err(error) = &result
                && (error.starts_with("Local storage quota exceeded (live SQLite + retained output + committed reservations);")
                    || error.starts_with("Outgoing receipt was not saved:"))
            {
                self.rollback.insert(outgoing.record.request_id.clone(), error.clone());
            }
        }
        let correlated = match (&request.operation, &result) {
            (Operation::Put(expected), Ok(Response::Saved(saved))) => {
                expected.record == saved.record
            }
            (Operation::List, Ok(Response::Records(views))) => views.iter().all(|v| {
                v.record.task_id == request.task_id && v.record.owner.partner == request.partner
            }),
            (
                Operation::ReadSource { request_id: a },
                Ok(Response::Source { request_id: b, .. }),
            ) => a == b,
            (Operation::PurgeSources { request_id: a }, Ok(Response::Purged { request_id: b })) => {
                a == b
            }
            (_, Err(_)) => true,
            _ => false,
        };
        if !correlated {
            self.error = Some("Uncorrelated outgoing receipt acknowledgement refused".into());
            self.failed.push_back(request);
            self.dirty = true;
            return;
        }
        match result {
            Err(error) => {
                self.error = Some(error);
                self.failed.push_back(request);
            }
            Ok(response) => {
                self.error = None;
                let current = request.task_id == self.task && request.partner == self.partner;
                match response {
                    Response::Saved(view) if current => {
                        let view = *view;
                        if let Some(old) = self
                            .records
                            .iter_mut()
                            .find(|v| v.record.request_id == view.record.request_id)
                        {
                            *old = view;
                        } else {
                            self.records.insert(0, view);
                        }
                        self.notice = Some(Message::ReceiptsSaved);
                    }
                    Response::Records(views) if current => {
                        self.records = views;
                        self.selected = self.selected.min(self.records.len().saturating_sub(1));
                        self.scroll = 0;
                        self.source = None;
                        self.notice = None;
                    }
                    Response::Source { request_id, text } if current => {
                        self.source = Some((request_id, text));
                        self.scroll = 0;
                    }
                    Response::Purged { request_id } if current => {
                        self.source = None;
                        self.display.clear();
                        self.lines.clear();
                        for view in &mut self.records {
                            if request_id
                                .as_ref()
                                .is_none_or(|id| *id == view.record.request_id)
                            {
                                view.source_available = false;
                            }
                        }
                        self.notice = Some(Message::ReceiptsPurged);
                    }
                    _ => {}
                }
            }
        }
        self.dirty = true;
    }

    pub fn retry(&mut self) {
        self.queue.extend(self.failed.drain(..));
        self.error = None;
        self.dirty = true;
    }

    pub(super) fn finished(&mut self, terminal: Option<&(store::WorkClaim, u64)>) {
        self.terminal = terminal.cloned();
    }

    fn discard_candidate(&self) -> Option<Discard> {
        if self.inflight.is_some()
            || self
                .queue
                .iter()
                .any(|r| matches!(r.operation, Operation::Put(_)))
        {
            return None;
        }
        let (claim, generation) = self.terminal.as_ref()?;
        let mut candidates = self.failed.iter().filter_map(|request| {
            let Operation::Put(outgoing) = &request.operation else {
                return None;
            };
            let record = &outgoing.record;
            (request.task_id == self.task
                && request.partner == self.partner
                && record.task_id == claim.task_id
                && record.owner == claim.owner
                && record.request_id == claim.request_id
                && record.writing == claim.writing
                && record.generation == *generation)
                .then_some(record)
        });
        let record = candidates.next()?;
        if candidates.next().is_some() {
            return None;
        }
        Some(Discard {
            record: record.clone(),
            error: self.rollback.get(&record.request_id)?.clone(),
        })
    }

    fn discard_confirmed(&mut self) {
        let Some(review) = self.discard.take() else {
            return;
        };
        if self.discard_candidate().as_ref() != Some(&review) {
            return;
        }
        self.failed.retain(|request| {
            !matches!(&request.operation,
            Operation::Put(outgoing) if outgoing.record == review.record)
        });
        self.rollback.remove(&review.record.request_id);
        self.gap = Some(review.record);
        self.gap_count = self.gap_count.saturating_add(1);
        self.error = None;
        self.source = None;
        self.display.clear();
        self.lines.clear();
        self.notice = None;
    }

    pub fn open(&mut self, task_id: i64, partner: i64) {
        if self.task != task_id || self.partner != partner {
            self.records.clear();
            self.selected = 0;
        }
        self.task = task_id;
        self.partner = partner;
        self.source = None;
        self.confirm = None;
        self.discard = None;
        self.affirmative = false;
        self.scroll = 0;
        self.notice = Some(Message::ReceiptsLoading);
        self.dirty = true;
        if !self.queue.iter().chain(self.inflight.iter()).any(|r| {
            r.task_id == task_id && r.partner == partner && matches!(r.operation, Operation::List)
        }) {
            self.queue.push_back(Request {
                task_id,
                partner,
                operation: Operation::List,
            });
        }
    }

    /// Called for capture/run/provider source purges committed elsewhere.
    pub fn source_purged(&mut self, task_id: i64) {
        if self.task == task_id {
            self.source = None;
            self.display.clear();
            self.lines.clear();
            for v in &mut self.records {
                v.source_available = false;
            }
            self.dirty = true;
        }
        if self
            .staged
            .as_ref()
            .is_some_and(|(_, o)| o.record.task_id == task_id)
        {
            self.staged = None;
        }
    }

    /// false closes only the inspector; ordinary drafts and their carets are untouched.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return true;
        }
        if key.code == KeyCode::Esc {
            if self.discard.take().is_some() {
                self.affirmative = false;
                self.dirty = true;
                return true;
            }
            if self.confirm.take().is_some() {
                self.affirmative = false;
                self.dirty = true;
                return true;
            }
            if self.source.take().is_some() {
                self.scroll = 0;
                self.dirty = true;
                return true;
            }
            return false;
        }
        if self.discard.is_some() {
            match key.code {
                KeyCode::PageDown => {
                    self.scroll = self
                        .scroll
                        .saturating_add(self.page.max(1))
                        .min(self.max_scroll)
                }
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(self.page.max(1)),
                KeyCode::Tab | KeyCode::Left | KeyCode::Right if self.rendered => {
                    self.affirmative = !self.affirmative
                }
                KeyCode::Enter if self.rendered && key.modifiers.is_empty() => {
                    if self.affirmative {
                        self.discard_confirmed();
                    }
                    self.discard = None;
                    self.affirmative = false;
                    self.scroll = 0;
                }
                _ => {}
            }
            self.dirty = true;
            return true;
        }
        if self.confirm.is_some() {
            match key.code {
                KeyCode::Tab | KeyCode::Left | KeyCode::Right if self.rendered => {
                    self.affirmative = !self.affirmative
                }
                KeyCode::Enter if self.rendered && key.modifiers.is_empty() => {
                    if self.affirmative {
                        let id = self.confirm.take().expect("purge review");
                        self.queue.push_back(Request {
                            task_id: self.task,
                            partner: self.partner,
                            operation: Operation::PurgeSources {
                                request_id: Some(id),
                            },
                        });
                    }
                    self.confirm = None;
                    self.affirmative = false;
                }
                _ => {}
            }
            self.dirty = true;
            return true;
        }
        match key.code {
            KeyCode::Char('r') if key.modifiers.is_empty() => self.retry(),
            KeyCode::Char('d') if key.modifiers.is_empty() => {
                self.discard = self.discard_candidate();
                self.notice = self
                    .discard
                    .is_none()
                    .then_some(Message::ReceiptsDiscardBlocked);
                self.affirmative = false;
                self.rendered = false;
                self.review_size = None;
                self.discard_seen = 0;
                self.scroll = 0;
                self.dirty = true;
            }
            KeyCode::Char('s') if key.modifiers.is_empty() => {
                if let Some(view) = self.records.get(self.selected) {
                    if view.source_available {
                        self.queue.push_back(Request {
                            task_id: self.task,
                            partner: self.partner,
                            operation: Operation::ReadSource {
                                request_id: view.record.request_id.clone(),
                            },
                        });
                    } else {
                        self.notice = Some(Message::ReceiptsUnavailable);
                        self.dirty = true;
                    }
                }
            }
            KeyCode::Char('p') if key.modifiers.is_empty() => {
                if let Some(view) = self
                    .records
                    .get(self.selected)
                    .filter(|v| v.source_available)
                {
                    self.confirm = Some(view.record.request_id.clone());
                    self.affirmative = false;
                    self.rendered = false;
                    self.review_size = None;
                }
            }
            KeyCode::Up if key.modifiers == KeyModifiers::NONE && self.source.is_none() => {
                self.selected = self.selected.saturating_sub(1);
                self.scroll = 0;
                self.dirty = true;
            }
            KeyCode::Down if key.modifiers == KeyModifiers::NONE && self.source.is_none() => {
                self.selected = (self.selected + 1).min(self.records.len().saturating_sub(1));
                self.scroll = 0;
                self.dirty = true;
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(self.page.max(1)),
            KeyCode::PageDown => {
                self.scroll = self
                    .scroll
                    .saturating_add(self.page.max(1))
                    .min(self.max_scroll)
            }
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.max_scroll,
            _ => {}
        }
        true
    }

    fn body(&self) -> String {
        if let Some((_, source)) = &self.source {
            return crate::workspace::handoff::sanitize(source);
        }
        let mut lines = Vec::new();
        if let Some(gap) = self
            .gap
            .as_ref()
            .filter(|gap| gap.task_id == self.task && gap.owner.partner == self.partner)
        {
            lines.push((Message::ReceiptsGapNotice).text().into());
            lines.push(
                Template::ReceiptsGap
                    .format(&[
                        ("count", &self.gap_count.to_string()),
                        ("request", &gap.request_id),
                    ])
                    .expect("receipt gap catalog arguments"),
            );
            lines.push(
                Template::ReceiptsTotal
                    .format(&[
                        ("bytes", &gap.bytes.to_string()),
                        ("tokens", &gap.tokens.to_string()),
                        ("deduplicated", &gap.deduplicated.to_string()),
                    ])
                    .expect("receipt gap count catalog arguments"),
            );
        }
        if let Some(view) = self.records.get(self.selected) {
            let r = &view.record;
            let format = |template: Template, args: &[(&str, &str)]| {
                template.format(args).expect("receipt catalog arguments")
            };
            lines.push(format!("{}/{}", self.selected + 1, self.records.len()));
            lines.push(format(
                Template::ReceiptsScope,
                &[
                    ("task", &r.task_id.to_string()),
                    ("partner", &r.owner.partner.to_string()),
                    ("name", &r.owner.name),
                ],
            ));
            lines.push(format(
                Template::ReceiptsRequest,
                &[
                    ("request", &r.request_id),
                    ("agent", &format!("{:?}", r.agent)),
                    ("operation", &format!("{:?}", r.operation)),
                    ("generation", &r.generation.to_string()),
                ],
            ));
            lines.push(format!("Route: {}", r.agent.label()));
            lines.push(format!(
                "Model: {} / requested; observed unknown in outgoing receipt",
                r.selection
                    .as_ref()
                    .and_then(|selection| selection.model.as_deref())
                    .unwrap_or("provider default")
            ));
            lines.push(r.effort.as_ref().map_or_else(
                || {
                    format!(
                        "Effort: {} / requested; not acknowledged",
                        r.selection
                            .as_ref()
                            .and_then(|selection| selection.effort.as_deref())
                            .unwrap_or("provider default")
                    )
                },
                |effort| effort.line(),
            ));
            lines.push(format!(
                "Todo: {}",
                r.progress
                    .as_deref()
                    .unwrap_or("no step list from this route")
            ));
            lines.push(
                "Usage: unknown in outgoing receipt; inspect provider-reported usage separately"
                    .into(),
            );
            lines.push("Outcome: unknown at dispatch; provider claims are not verification".into());
            lines.push(
                (if view.source_available {
                    Message::ReceiptsAvailable
                } else {
                    Message::ReceiptsUnavailable
                })
                .text()
                .into(),
            );
            lines.push(format(
                Template::ReceiptsTotal,
                &[
                    ("bytes", &r.bytes.to_string()),
                    ("tokens", &r.tokens.to_string()),
                    ("deduplicated", &r.deduplicated.to_string()),
                ],
            ));
            for section in &r.sections {
                let name = match section.name.as_str() {
                    "Instructions" => Message::ReceiptsInstructions,
                    "Task facts" => Message::ReceiptsTaskFacts,
                    "Attachments" => Message::ReceiptsAttachments,
                    "Handoff" => Message::ReceiptsHandoff,
                    "Approved memories" => Message::ReceiptsApprovedMemories,
                    _ => Message::ReceiptsOtherFields,
                };
                lines.push(format(
                    Template::ReceiptsCounts,
                    &[
                        ("section", (name).text()),
                        ("bytes", &section.bytes.to_string()),
                        ("tokens", &section.tokens.to_string()),
                    ],
                ));
                lines.push(format!("  [{}]", section.keys.join(", ")));
            }
            lines.push(format(
                Template::ReceiptsEnvelope,
                &[("bytes", &r.envelope_bytes.to_string())],
            ));
            lines.push(format(
                Template::ReceiptsTime,
                &[("time", &r.recorded_at.to_string())],
            ));
            lines.push(format!(
                "SHA-256: {}",
                r.sha256
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            ));
            for capture in &r.captures {
                lines.push(format(
                    Template::ReceiptsCapture,
                    &[
                        ("id", &capture.id.to_string()),
                        ("bytes", &capture.bytes.to_string()),
                        ("tokens", &capture.tokens.to_string()),
                    ],
                ));
            }
            for reference in &r.references {
                lines.push(format(
                    Template::ReceiptsReference,
                    &[("kind", &reference.kind), ("id", &reference.id)],
                ));
            }
            lines.push((Message::ReceiptsEstimator).text().into());
            lines.push((Message::ReceiptsBoundary).text().into());
        } else {
            lines.push((Message::ReceiptsEmpty).text().into());
        }
        if let Some(notice) = self.notice {
            lines.push((notice).text().into());
        }
        if let Some(error) = &self.error {
            lines.push((Message::ReceiptsError).text().into());
            lines.push(crate::workspace::handoff::sanitize(error));
        }
        crate::workspace::handoff::sanitize(&lines.join("\n"))
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Clear, area);
        if self.confirm.is_some() || self.discard.is_some() {
            let size = (area.width, area.height);
            if self.review_size != Some(size) {
                self.rendered = false;
                self.discard_seen = 0;
                if self.discard.is_some() {
                    self.scroll = 0;
                }
                self.affirmative = false;
                self.review_size = Some(size);
            }
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .title((Message::ReceiptsTitle).text());
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if area.width < 60 || area.height < 18 {
            self.rendered = false;
            frame.render_widget(Paragraph::new((Message::ReceiptsSmall).text()), inner);
            return;
        }
        let mut content = inner;
        content.height = content.height.saturating_sub(2);
        if let Some(discard) = &self.discard {
            let scope = Template::ReceiptsScope
                .format(&[
                    ("task", &discard.record.task_id.to_string()),
                    ("partner", &discard.record.owner.partner.to_string()),
                    ("name", &discard.record.owner.name),
                ])
                .expect("receipt discard scope arguments");
            let text = crate::workspace::handoff::sanitize(&format!(
                "{}\n{}\n{}\n{}",
                Message::ReceiptsDiscardReview.text(),
                scope,
                discard.record.request_id,
                discard.error
            ));
            let ranges = wrap_ranges(&text, usize::from(content.width));
            let body = Rect {
                height: content.height.saturating_sub(2),
                ..content
            };
            self.page = usize::from(body.height);
            self.max_scroll = ranges.len().saturating_sub(self.page);
            self.scroll = self.scroll.min(self.max_scroll);
            if self.scroll <= self.discard_seen {
                self.discard_seen = self
                    .discard_seen
                    .max((self.scroll + self.page).min(ranges.len()));
            }
            self.rendered = self.discard_seen >= ranges.len();
            let visible: Vec<_> = ranges
                .iter()
                .skip(self.scroll)
                .take(self.page)
                .map(|(start, end)| Line::raw(&text[*start..*end]))
                .collect();
            frame.render_widget(Paragraph::new(visible), body);
            let choices = format!(
                "{} {}     {} {}",
                if self.affirmative { "[ ]" } else { "[x]" },
                Message::ReceiptsStay.text(),
                if self.affirmative { "[x]" } else { "[ ]" },
                Message::ReceiptsDiscardAction.text()
            );
            frame.render_widget(
                Paragraph::new(choices).wrap(ratatui::widgets::Wrap { trim: false }),
                Rect::new(content.x, body.bottom(), content.width, 2),
            );
        } else if self.confirm.is_some() {
            let text = format!(
                "{}\n\n{}\n\n{} {}     {} {}",
                Message::ReceiptsConfirm.text(),
                self.confirm.as_deref().unwrap_or_default(),
                if self.affirmative { "[ ]" } else { "[x]" },
                Message::ReceiptsStay.text(),
                if self.affirmative { "[x]" } else { "[ ]" },
                Message::ReceiptsPurge.text()
            );
            frame.render_widget(
                Paragraph::new(text).wrap(ratatui::widgets::Wrap { trim: false }),
                content,
            );
            self.rendered = true;
        } else {
            if self.dirty || self.width != content.width {
                self.display = self.body();
                self.lines = wrap_ranges(&self.display, usize::from(content.width));
                self.width = content.width;
                self.dirty = false;
            }
            self.page = usize::from(content.height);
            self.max_scroll = self.lines.len().saturating_sub(self.page);
            self.scroll = self.scroll.min(self.max_scroll);
            let visible: Vec<Line<'_>> = self
                .lines
                .iter()
                .skip(self.scroll)
                .take(self.page)
                .map(|(start, end)| Line::raw(&self.display[*start..*end]))
                .collect();
            frame.render_widget(Paragraph::new(visible), content);
        }
        let footer = Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 2);
        let hint = if self.source.is_some() {
            (Message::ReceiptsSource).text()
        } else {
            (Message::ReceiptsHelp).text()
        };
        frame.render_widget(
            Paragraph::new(hint).wrap(ratatui::widgets::Wrap { trim: false }),
            footer,
        );
    }
}

// Do not copy an 8 MiB payload merely to await its acknowledgement. The worker
// returns the original owned request on both success and failure.
fn request_identity(request: &Request) -> Request {
    Request {
        task_id: request.task_id,
        partner: request.partner,
        operation: match &request.operation {
            Operation::Put(outgoing) => Operation::Put(Box::new(Outgoing {
                record: outgoing.record.clone(),
                source: None,
            })),
            other => other.clone(),
        },
    }
}

fn wrap_ranges(text: &str, width: usize) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut used = 0;
    for (offset, g) in text.grapheme_indices(true) {
        if g == "\n" || g == "\r\n" {
            ranges.push((start, offset));
            start = offset + g.len();
            used = 0;
            continue;
        }
        let cells = Line::raw(g).width();
        if used > 0 && used + cells > width.max(1) {
            ranges.push((start, offset));
            start = offset;
            used = 0;
        }
        used += cells;
    }
    ranges.push((start, text.len()));
    ranges
}

#[cfg(test)]
#[path = "receipts_tests.rs"]
mod tests;
