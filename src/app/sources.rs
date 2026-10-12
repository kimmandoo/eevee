use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

use crate::{context, repo, store};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Files,
    Basket,
    Preview,
    Payload,
}
#[derive(Clone, Copy)]
pub enum EditKind {
    Search,
    Range,
}
pub struct Editor {
    pub kind: EditKind,
    pub text: String,
    pub cursor: usize,
}
#[derive(Clone, Copy)]
pub struct Confirmation {
    pub id: i64,
    pub purge: bool,
    pub destructive: bool,
}
pub enum Action {
    Changed,
    Close,
    RefreshRepository,
    Quit,
    Scroll(KeyCode),
}
#[derive(Clone, Copy)]
enum Mutation {
    Pin,
    Remove(i64),
    Purge(i64),
}

#[derive(Default)]
pub struct State {
    pub mode: Mode,
    pub files: Vec<context::Source>,
    pub file_selection: usize,
    pub basket: Vec<context::Excerpt>,
    pub basket_selection: usize,
    pub preview: Option<context::Excerpt>,
    pub payload: Option<context::Payload>,
    pub detail: String,
    pub query: String,
    pub status: String,
    pub error: String,
    pub editor: Option<Editor>,
    pub confirmation: Option<Confirmation>,
    pub pending: Option<context::Request>,
    pub pending_store: Option<store::ContextRequest>,
    pub source_busy: bool,
    pub store_busy: bool,
    pub store_failed: bool,
    pub await_refresh: bool,
    pending_payload: Option<context::Payload>,
    store_mutation: bool,
    last_mutation: Option<Mutation>,
    payload_page: usize,
}

impl State {
    pub fn reset(&mut self, task_id: i64) {
        *self = Self::default();
        if task_id > 0 {
            self.queue_store(task_id, store::ContextOperation::Load);
        }
    }
    pub fn busy(&self) -> bool {
        self.source_busy || self.store_busy || self.await_refresh
    }
    pub fn persistence_pending(&self) -> bool {
        self.store_busy && self.store_mutation
    }
    pub fn catalogue(&mut self, task_id: i64, report: Option<&Arc<repo::Report>>) {
        if task_id <= 0 {
            self.error = "Create and save a task before selecting sources.".into();
            return;
        }
        let Some(report) = report else {
            self.await_refresh = true;
            self.status = "Waiting for repository evidence; w shows its error layer.".into();
            return;
        };
        if self.source_busy {
            return;
        }
        self.preview = None;
        self.payload = None;
        self.detail.clear();
        self.mode = Mode::Files;
        self.queue(
            task_id,
            context::Operation::Catalogue {
                report: Arc::clone(report),
                query: self.query.clone(),
            },
        );
    }
    fn queue(&mut self, task_id: i64, operation: context::Operation) {
        self.pending = Some(context::Request { task_id, operation });
        self.source_busy = true;
        self.error.clear();
        self.status = "Reading and validating selected source evidence…".into();
    }
    fn queue_store(&mut self, task_id: i64, operation: store::ContextOperation) {
        self.store_mutation = matches!(
            &operation,
            store::ContextOperation::Pin(_)
                | store::ContextOperation::Remove(_)
                | store::ContextOperation::Purge(_)
        );
        if self.store_mutation {
            self.last_mutation = match &operation {
                store::ContextOperation::Pin(_) => Some(Mutation::Pin),
                store::ContextOperation::Remove(id) => Some(Mutation::Remove(*id)),
                store::ContextOperation::Purge(ids) => ids.first().copied().map(Mutation::Purge),
                _ => None,
            };
        }
        self.pending_store = Some(store::ContextRequest { task_id, operation });
        self.store_busy = true;
        self.error.clear();
    }
    pub fn observed_repository(&mut self, task_id: i64, report: Option<&Arc<repo::Report>>) {
        if self.await_refresh {
            self.await_refresh = false;
            if report.is_some() {
                self.catalogue(task_id, report);
            } else {
                self.error =
                    "Repository inspection unavailable. w shows details; u retries.".into();
            }
        }
    }
    pub fn event(&mut self, task_id: i64, event: context::Event) {
        if event.task_id != task_id {
            return;
        }
        self.source_busy = false;
        match event.result {
            Ok(context::Response::Catalogue(files)) => {
                self.files = files;
                self.file_selection = self.file_selection.min(self.files.len().saturating_sub(1));
                self.status = format!(
                    "{} eligible files / ignored, excluded, binary and linked paths omitted",
                    self.files.len()
                );
                self.file_detail();
            }
            Ok(context::Response::Inspected(excerpt)) => {
                self.detail = excerpt_detail(&excerpt);
                self.preview = Some(excerpt);
                self.mode = Mode::Preview;
                self.status = "Inspected capture / p pins after persistence validation".into();
            }
            Ok(context::Response::Assembled(payload)) => {
                self.pending_payload = Some(payload);
                self.queue_store(
                    task_id,
                    store::ContextOperation::Validate(self.basket.clone()),
                );
                self.status =
                    "Source identities checked; validating persisted basket references…".into();
            }
            Err(error) => {
                self.error = safe_error(error);
                self.pending_payload = None;
                self.payload = None;
                self.status = "Context unavailable or stale / no assembly or dispatch".into();
            }
        }
    }
    pub fn stored(
        &mut self,
        task_id: i64,
        owner: i64,
        result: Result<Vec<context::Excerpt>, String>,
    ) {
        if task_id != owner {
            return;
        }
        self.store_busy = false;
        match result {
            Ok(rows) => {
                self.basket = rows;
                self.basket_selection = self
                    .basket_selection
                    .min(self.basket.len().saturating_sub(1));
                if self.store_mutation {
                    self.store_failed = false;
                    self.last_mutation = None;
                }
                self.error.clear();
                if let Some(payload) = self.pending_payload.take() {
                    self.status = if payload.dispatch {
                        "Dispatch boundary revalidated / no provider send"
                    } else {
                        "Source and stored references validated / preview only"
                    }
                    .into();
                    self.payload = Some(payload);
                    self.payload_page = 0;
                    self.mode = Mode::Payload;
                    self.payload_detail();
                } else {
                    self.status = format!(
                        "{} task-owned basket references / stored",
                        self.basket.len()
                    );
                    if self.mode == Mode::Basket {
                        self.basket_detail();
                    }
                }
            }
            Err(error) => {
                self.error = safe_error(error);
                self.pending_payload = None;
                self.payload = None;
                self.status = "Context storage/validation failed / s retries failed mutation; no automatic retry".into();
                self.store_failed |= self.store_mutation;
            }
        }
    }
    fn inspect(
        &mut self,
        task_id: i64,
        report: Option<&Arc<repo::Report>>,
        start: usize,
        end: usize,
    ) {
        let Some(report) = report else {
            self.error = "Repository evidence unavailable; u refreshes.".into();
            return;
        };
        let Some(source) = self.files.get(self.file_selection) else {
            self.error = "No eligible source selected.".into();
            return;
        };
        self.queue(
            task_id,
            context::Operation::Inspect {
                report: Arc::clone(report),
                path: source.path.clone(),
                start_line: start,
                end_line: end,
            },
        );
    }
    fn assemble(&mut self, task_id: i64, report: Option<&Arc<repo::Report>>, dispatch: bool) {
        let Some(report) = report else {
            self.error = "Repository evidence unavailable; u refreshes.".into();
            return;
        };
        self.payload = None;
        self.pending_payload = None;
        self.detail.clear();
        self.queue(
            task_id,
            context::Operation::Assemble {
                report: Arc::clone(report),
                excerpts: self.basket.clone(),
                dispatch,
            },
        );
    }
    pub fn paste(&mut self, input: &str) -> bool {
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        let limit: usize = match editor.kind {
            EditKind::Search => 1024,
            EditKind::Range => 64,
        };
        let capacity = limit.saturating_sub(editor.text.len());
        let mut text = String::with_capacity(input.len().min(capacity));
        for ch in input
            .chars()
            .filter(|ch| super::safe_character(*ch) && *ch != '\n')
        {
            if text.len() + ch.len_utf8() > capacity {
                self.error = format!("Input limited to {limit} UTF-8 bytes; paste was truncated.");
                break;
            }
            text.push(ch);
        }
        if text.is_empty() {
            return false;
        }
        editor.text.insert_str(editor.cursor, &text);
        editor.cursor += text.len();
        editor.cursor = editor
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i >= editor.cursor)
            .unwrap_or(editor.text.len());
        true
    }
    pub fn key(
        &mut self,
        key: KeyEvent,
        task_id: i64,
        report: Option<&Arc<repo::Report>>,
    ) -> Action {
        if key.kind != KeyEventKind::Press {
            return Action::Changed;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.confirmation = None;
            return Action::Quit;
        }
        if let Some(confirmation) = self.confirmation.as_mut() {
            match key.code {
                KeyCode::Esc => self.confirmation = None,
                KeyCode::Tab | KeyCode::BackTab => {
                    confirmation.destructive = !confirmation.destructive
                }
                KeyCode::Left | KeyCode::Up | KeyCode::Home => confirmation.destructive = false,
                KeyCode::Right | KeyCode::Down | KeyCode::End => confirmation.destructive = true,
                KeyCode::PageUp | KeyCode::PageDown => return Action::Scroll(key.code),
                KeyCode::Enter => {
                    let confirmation = self
                        .confirmation
                        .take()
                        .expect("active context confirmation");
                    if confirmation.destructive && !self.busy() {
                        self.payload = None;
                        let operation = if confirmation.purge {
                            store::ContextOperation::Purge(vec![confirmation.id])
                        } else {
                            store::ContextOperation::Remove(confirmation.id)
                        };
                        self.queue_store(task_id, operation);
                    }
                }
                _ => {}
            }
            return Action::Changed;
        }
        if self.editor.is_some() {
            return self.editor_key(key, task_id, report);
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Action::Changed;
        }
        if key.code == KeyCode::Esc {
            match self.mode {
                Mode::Preview => {
                    self.mode = Mode::Files;
                    self.file_detail();
                }
                Mode::Payload => {
                    self.mode = Mode::Basket;
                    self.basket_detail();
                }
                _ => return Action::Close,
            }
            return Action::Changed;
        }
        if key.code == KeyCode::Char('v') {
            return Action::Close;
        }
        if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            return Action::Scroll(key.code);
        }
        if self.busy() {
            self.error = "A context operation is pending; PgUp/Dn and Esc remain available.".into();
            return Action::Changed;
        }
        if self.store_failed {
            if key.code == KeyCode::Char('s') {
                let operation = match self.last_mutation {
                    Some(Mutation::Pin) => self.preview.clone().map(store::ContextOperation::Pin),
                    Some(Mutation::Remove(id)) => Some(store::ContextOperation::Remove(id)),
                    Some(Mutation::Purge(id)) => Some(store::ContextOperation::Purge(vec![id])),
                    None => None,
                };
                if let Some(operation) = operation {
                    self.queue_store(task_id, operation);
                }
            } else {
                self.error = "Resolve the storage problem, then s retries the retained operation. Esc keeps it; quitting requires explicit loss review.".into();
            }
            return Action::Changed;
        }
        match key.code {
            KeyCode::Char('/') => {
                self.editor = Some(Editor {
                    kind: EditKind::Search,
                    text: self.query.clone(),
                    cursor: self.query.len(),
                })
            }
            KeyCode::Char('u') => {
                self.await_refresh = true;
                self.payload = None;
                return Action::RefreshRepository;
            }
            KeyCode::Char('b') => {
                self.mode = if self.mode == Mode::Basket {
                    Mode::Files
                } else {
                    Mode::Basket
                };
                if self.mode == Mode::Files {
                    self.file_detail();
                } else {
                    self.basket_detail();
                }
            }
            KeyCode::Char('r') if self.mode != Mode::Basket => {
                self.editor = Some(Editor {
                    kind: EditKind::Range,
                    text: "1:1".into(),
                    cursor: 3,
                })
            }
            KeyCode::Enter if self.mode == Mode::Files => self.inspect(task_id, report, 1, 1),
            KeyCode::Char('p') if self.mode == Mode::Preview => {
                if let Some(excerpt) = self.preview.clone() {
                    self.mode = Mode::Basket;
                    self.payload = None;
                    self.queue_store(task_id, store::ContextOperation::Pin(excerpt));
                }
            }
            KeyCode::Char('a') => self.assemble(task_id, report, false),
            KeyCode::Char('d') => self.assemble(task_id, report, true),
            KeyCode::Delete | KeyCode::Char('x') if self.mode == Mode::Basket => {
                if let Some(excerpt) = self.basket.get(self.basket_selection) {
                    self.confirmation = Some(Confirmation {
                        id: excerpt.id,
                        purge: key.code == KeyCode::Char('x'),
                        destructive: false,
                    });
                }
            }
            KeyCode::Char('n') if self.mode == Mode::Payload => {
                if let Some(payload) = &self.payload {
                    self.payload_page =
                        (self.payload_page + 1).min(payload.text.len().saturating_sub(1) / 8192);
                }
                self.payload_detail();
            }
            KeyCode::Char('p') if self.mode == Mode::Payload => {
                self.payload_page = self.payload_page.saturating_sub(1);
                self.payload_detail();
            }
            code if matches!(self.mode, Mode::Files | Mode::Basket) => {
                let (selected, count) = if self.mode == Mode::Files {
                    (&mut self.file_selection, self.files.len())
                } else {
                    (&mut self.basket_selection, self.basket.len())
                };
                *selected = match code {
                    KeyCode::Up => selected.saturating_sub(1),
                    KeyCode::Down => selected.saturating_add(1).min(count.saturating_sub(1)),
                    KeyCode::Home => 0,
                    KeyCode::End => count.saturating_sub(1),
                    _ => *selected,
                };
                if self.mode == Mode::Basket {
                    self.basket_detail();
                } else {
                    self.file_detail();
                }
            }
            code => return Action::Scroll(code),
        }
        Action::Changed
    }
    fn editor_key(
        &mut self,
        key: KeyEvent,
        task_id: i64,
        report: Option<&Arc<repo::Report>>,
    ) -> Action {
        if key.code == KeyCode::Esc {
            self.editor = None;
            return Action::Changed;
        }
        if key.code == KeyCode::Enter && key.modifiers.is_empty() {
            if self.busy() {
                self.error = "Wait for the pending context operation; input is retained.".into();
                return Action::Changed;
            }
            let editor = self.editor.take().expect("active source editor");
            match editor.kind {
                EditKind::Search => {
                    self.query = editor.text;
                    self.file_selection = 0;
                    self.catalogue(task_id, report);
                }
                EditKind::Range => {
                    let range = editor.text.split_once(':').and_then(|(a, b)| {
                        Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
                    });
                    if let Some((start, end)) =
                        range.filter(|(start, end)| *start > 0 && end >= start)
                    {
                        self.inspect(task_id, report, start, end);
                    } else {
                        self.error = "Use a one-based inclusive range, e.g. 2:5.".into();
                        self.editor = Some(editor);
                    }
                }
            }
            return Action::Changed;
        }
        if let KeyCode::Char(ch) = key.code
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.paste(ch.encode_utf8(&mut [0; 4]));
            return Action::Changed;
        }
        let editor = self.editor.as_mut().expect("active source editor");
        let before = editor.text[..editor.cursor]
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .next_back()
            .unwrap_or(0);
        let after = editor
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i > editor.cursor)
            .unwrap_or(editor.text.len());
        match key.code {
            KeyCode::Left => editor.cursor = before,
            KeyCode::Right => editor.cursor = after,
            KeyCode::Home => editor.cursor = 0,
            KeyCode::End => editor.cursor = editor.text.len(),
            KeyCode::Backspace => {
                editor.text.replace_range(before..editor.cursor, "");
                editor.cursor = before;
            }
            KeyCode::Delete => {
                editor.text.replace_range(editor.cursor..after, "");
            }
            KeyCode::Char('h') if key.modifiers == KeyModifiers::CONTROL => {
                editor.text.replace_range(before..editor.cursor, "");
                editor.cursor = before;
            }
            _ => {}
        }
        if editor.cursor < editor.text.len() {
            editor.cursor = editor
                .text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .take_while(|i| *i <= editor.cursor)
                .last()
                .unwrap_or(0);
        }
        Action::Changed
    }
    fn file_detail(&mut self) {
        self.detail = self.files.get(self.file_selection).map(|source|
            format!("Repository-relative path: {:?}\n{} bytes / SHA-256 bytes: {:02x?}\nEnter inspects line 1; r chooses a range. Binary, oversized, credential and excluded paths are not eligible.", source.path, source.bytes, source.sha256))
            .unwrap_or_else(|| "No eligible files match this filename filter. / edits the filter; u refreshes repository evidence.".into());
    }
    fn basket_detail(&mut self) {
        self.detail = self
            .basket
            .get(self.basket_selection)
            .map(excerpt_detail)
            .unwrap_or_else(|| {
                "No pinned source excerpts. Inspect a file/range, then p pins it.".into()
            });
    }
    fn payload_detail(&mut self) {
        let Some(payload) = &self.payload else {
            return;
        };
        let boundary = |mut n: usize| {
            n = n.min(payload.text.len());
            while !payload.text.is_char_boundary(n) {
                n -= 1;
            }
            n
        };
        let start = boundary(self.payload_page * 8192);
        let end = boundary((self.payload_page + 1) * 8192);
        self.detail = format!(
            "EXACT BASKET ENVELOPE / escaped display\n{} UTF-8 bytes / ~{} tokens (bytes/4 estimate, not tokenizer usage)\nProvider history/config/agent-owned reads may add context; this basket does not constrain them. No provider is connected and nothing is sent.\nPage {} / bytes {}..{} / n next, p previous\n\n{:?}",
            payload.bytes,
            payload.estimated_tokens,
            self.payload_page + 1,
            start,
            end,
            &payload.text[start..end]
        );
    }
}

fn excerpt_detail(excerpt: &context::Excerpt) -> String {
    let mut value = format!(
        "#{} / {:?}\nRoot: {:?}\nLines {}:{} inclusive / {} bytes\nSHA-256 bytes: {:02x?}\n{}\n\n{}",
        excerpt.id,
        excerpt.path,
        excerpt.root,
        excerpt.start_line,
        excerpt.end_line,
        excerpt.text.len(),
        excerpt.source_sha256,
        if excerpt.available {
            "Captured source / best-effort masked / display strips controls"
        } else {
            "UNAVAILABLE: retained identity only; assembly must refuse"
        },
        excerpt.text
    );
    value.retain(super::safe_character);
    value
}
fn safe_error(mut error: String) -> String {
    error.retain(super::safe_character);
    error
}
