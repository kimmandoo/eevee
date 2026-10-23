//! Task-correlated artifact inspection. No input here approves or executes a tool.
use crate::{i18n::Message, store, workspace::artifacts::*};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::SyncSender;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    List,
    Page,
    Search,
    Locations,
    Details,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Editor {
    Search,
    Offset,
}
#[derive(Default)]
pub struct State {
    pub open: bool,
    pub task_id: Option<i64>,
    pub records: Vec<Record>,
    pub selected: usize,
    pub view: View,
    pub slices: Vec<Slice>,
    pub locations: Vec<LocatedSlice>,
    pub next_offset: Option<u64>,
    pub offset: u64,
    pub scroll: usize,
    pub viewport: usize,
    pub query: String,
    pub input: String,
    pub editor: Option<Editor>,
    pub purge_review: Option<Handle>,
    pub discard_review: bool,
    pub review_visible: bool,
    pub error: Option<String>,
    pub notice: Option<Message>,
    pub failed: Option<Request>,
    queued: VecDeque<Request>,
    pending: Option<Request>,
    inflight: Option<(u64, i64)>,
    serial: u64,
    history: Vec<u64>,
    details_return: Option<(View, usize)>,
    lookups: BTreeMap<u64, SyncSender<Result<Response, String>>>,
    captures: BTreeMap<u64, SyncSender<Result<Record, String>>>,
}
impl State {
    pub fn visible(&self) -> bool {
        self.open
    }
    pub fn busy(&self) -> bool {
        self.inflight.is_some()
            || self.pending.is_some()
            || !self.queued.is_empty()
            || self.failed.is_some()
    }
    fn enqueue(&mut self, task: i64, operation: Operation) {
        self.serial = self.serial.wrapping_add(1);
        self.queued.push_back(Request {
            request_id: self.serial,
            task_id: task,
            operation,
        });
    }
    pub fn retain(&mut self, task_id: i64, capture: ToolCapture) {
        self.enqueue(task_id, Operation::RetainTool(capture));
    }
    /// The owning provider claim must be checked by the app before entry.
    pub fn retain_with_reply(
        &mut self,
        task: i64,
        capture: ToolCapture,
        reply: SyncSender<Result<Record, String>>,
    ) {
        self.retain(task, capture);
        self.captures.insert(self.serial, reply);
    }
    pub fn enqueue_lookup(
        &mut self,
        task: i64,
        operation: Operation,
        reply: SyncSender<Result<Response, String>>,
    ) {
        if !matches!(
            &operation,
            Operation::Load
                | Operation::Read { .. }
                | Operation::Search { .. }
                | Operation::Locations { .. }
        ) {
            let _=reply.try_send(Err("Providers may only inspect retained task-owned artifacts; no mutation or permission grant".into()));
            return;
        }
        self.enqueue(task, operation);
        self.lookups.insert(self.serial, reply);
    }
    pub fn command(&mut self, name: &str, task_id: i64) -> bool {
        if !matches!(
            name,
            "artifacts"
                | "artifact-search"
                | "artifact-locations"
                | "/artifacts"
                | "/artifact-search"
                | "/artifact-locations"
        ) {
            return false;
        }
        if self.task_id != Some(task_id) {
            self.task_id = Some(task_id);
            self.records.clear();
            self.selected = 0;
            self.slices.clear();
            self.locations.clear();
            self.history.clear();
            self.offset = 0;
        }
        self.open = true;
        self.view = View::List;
        self.purge_review = None;
        self.discard_review = false;
        self.editor = None;
        self.review_visible = false;
        self.scroll = 0;
        self.enqueue(task_id, Operation::Load);
        if name.ends_with("search") {
            self.editor = Some(Editor::Search);
            self.input.clear();
        }
        if name.ends_with("locations") {
            self.notice = Some(Message::ArtifactsSelectLocation);
        }
        true
    }
    pub fn pump(&mut self, worker: &store::Worker) -> bool {
        if self.inflight.is_some() || self.failed.is_some() {
            return false;
        }
        if self.pending.is_none() {
            self.pending = self.queued.pop_front();
        }
        let Some(request) = self.pending.as_ref() else {
            return false;
        };
        let identity = (request.request_id, request.task_id);
        if worker.try_artifact(&mut self.pending) {
            self.inflight = Some(identity);
            true
        } else {
            false
        }
    }
    pub fn store_event(&mut self, request: Request, result: Result<Response, String>) {
        if self.inflight != Some((request.request_id, request.task_id)) {
            return;
        }
        self.inflight = None;
        if let Some(reply) = self.lookups.remove(&request.request_id) {
            let _ = reply.try_send(result);
            return;
        }
        if let Some(reply) = self.captures.remove(&request.request_id) {
            let retained = match &result {
                Ok(Response::Retained(record)) => Ok(record.clone()),
                Ok(_) => Err("Artifact retention returned an unrelated response".into()),
                Err(error) => Err(error.clone()),
            };
            let _ = reply.try_send(retained);
        }
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                self.error = Some(error);
                if matches!(&request.operation, Operation::RetainTool(_)) {
                    self.failed = Some(request);
                }
                return;
            }
        };
        if self.task_id != Some(request.task_id) {
            // Captures remain durable for their real task even when not currently inspected.
            return;
        }
        self.error = None;
        self.notice = None;
        match response {
            Response::Records(records) => {
                let inspected = (self.view != View::List).then(|| self.handle()).flatten();
                self.records = records;
                if let Some(handle) = inspected {
                    if let Some(index) = self
                        .records
                        .iter()
                        .position(|record| record.handle == handle && !record.unavailable)
                    {
                        self.selected = index;
                    } else {
                        self.view = View::List;
                        self.details_return = None;
                        self.slices.clear();
                        self.locations.clear();
                        self.next_offset = None;
                        self.scroll = 0;
                    }
                }
                self.selected = self.selected.min(self.records.len().saturating_sub(1));
            }
            Response::Retained(record) => {
                if let Some(existing) = self.records.iter_mut().find(|r| r.handle == record.handle)
                {
                    *existing = record;
                } else {
                    self.records.insert(0, record);
                    if self.view == View::List {
                        self.selected = 0;
                    } else {
                        self.selected += 1;
                    }
                }
            }
            Response::Page(slice) => {
                self.offset = slice.offset;
                self.next_offset = slice.next_offset;
                self.slices = vec![slice];
                self.view = View::Page;
                self.scroll = 0;
            }
            Response::Search(result) => {
                self.slices = result.slices;
                self.next_offset = result.next_offset;
                self.query = result.query;
                self.view = View::Search;
                self.scroll = 0;
            }
            Response::Locations {
                slices,
                next_offset,
            } => {
                self.locations = slices;
                self.next_offset = next_offset;
                self.view = View::Locations;
                self.scroll = 0;
            }
            Response::Purged { handle, warning } => {
                if let Some(record) = self.records.iter_mut().find(|r| r.handle == handle) {
                    record.unavailable = true;
                }
                self.slices.clear();
                self.locations.clear();
                self.next_offset = None;
                self.purge_review = None;
                self.view = View::List;
                self.error = warning;
                self.notice = Some(Message::ArtifactsPurged);
            }
        }
    }
    pub fn selected_record(&self) -> Option<&Record> {
        self.records.get(self.selected)
    }
    fn handle(&self) -> Option<Handle> {
        self.selected_record().map(|r| r.handle.clone())
    }
    fn request_page(&mut self, offset: u64) {
        if let (Some(task), Some(handle)) = (self.task_id, self.handle()) {
            self.enqueue(
                task,
                Operation::Read {
                    handle,
                    offset,
                    limit: PREVIEW_BYTES,
                },
            );
        }
    }
    fn request_search(&mut self, offset: u64) {
        if let (Some(task), Some(handle)) = (self.task_id, self.handle()) {
            self.enqueue(
                task,
                Operation::Search {
                    handle,
                    query: self.query.clone(),
                    offset,
                    limit: MAX_MATCHES,
                },
            );
        }
    }
    fn request_locations(&mut self, offset: u64) {
        if let (Some(task), Some(handle)) = (self.task_id, self.handle()) {
            self.enqueue(task, Operation::Locations { handle, offset });
        }
    }
    pub fn paste(&mut self, text: &str) -> bool {
        if !self.open || self.editor.is_none() || self.purge_review.is_some() || self.discard_review
        {
            return false;
        }
        let text = text.chars().filter(|c| !c.is_control()).collect::<String>();
        let remaining = 1024usize.saturating_sub(self.input.len());
        self.input.push_str(utf8_prefix(&text, remaining));
        true
    }
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if !self.open {
            return false;
        }
        if key.kind == KeyEventKind::Release {
            return true;
        }
        if let Some(editor) = self.editor {
            match key.code {
                KeyCode::Esc => {
                    self.editor = None;
                    self.input.clear();
                }
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Char(ch) if !ch.is_control() => {
                    if self.input.len() + ch.len_utf8() <= 1024 {
                        self.input.push(ch);
                    }
                }
                KeyCode::Enter => {
                    if self.busy() {
                        self.notice = Some(Message::ArtifactsBusy);
                        return true;
                    }
                    match editor {
                        Editor::Search => {
                            self.query = std::mem::take(&mut self.input);
                            self.editor = None;
                            self.request_search(0);
                        }
                        Editor::Offset => match self.input.parse::<u64>() {
                            Ok(offset) => {
                                self.editor = None;
                                self.history.push(self.offset);
                                self.request_page(offset);
                            }
                            Err(_) => self.notice = Some(Message::ArtifactsOffsetInvalid),
                        },
                    }
                }
                _ => {}
            }
            return true;
        }
        if self.purge_review.is_some() || self.discard_review {
            match key.code {
                KeyCode::Esc | KeyCode::Char('n') => {
                    self.purge_review = None;
                    self.discard_review = false;
                    self.review_visible = false;
                }
                KeyCode::PageDown => {
                    self.scroll = self.scroll.saturating_add(self.viewport.max(1));
                }
                KeyCode::PageUp => {
                    self.scroll = self.scroll.saturating_sub(self.viewport.max(1));
                }
                KeyCode::Char('y') if self.review_visible && self.discard_review => {
                    self.failed = None;
                    self.discard_review = false;
                    self.review_visible = false;
                    self.error = None;
                    self.scroll = 0;
                }
                KeyCode::Char('y') if self.review_visible && !self.busy() => {
                    if let (Some(task), Some(handle)) = (self.task_id, self.purge_review.take()) {
                        self.enqueue(task, Operation::Purge { handle });
                        self.review_visible = false;
                        self.scroll = 0;
                    }
                }
                _ => {}
            }
            return true;
        }
        match key.code {
            KeyCode::Esc => {
                if self.view == View::Details {
                    let (view, scroll) = self.details_return.take().unwrap_or((View::List, 0));
                    self.view = view;
                    self.scroll = scroll;
                } else if self.view != View::List {
                    self.view = View::List;
                    self.scroll = 0;
                } else {
                    self.open = false;
                }
            }
            KeyCode::Char('r') => {
                if let Some(request) = self.failed.take() {
                    self.pending = Some(request);
                    self.error = None;
                } else if let Some(task) = self.task_id {
                    self.enqueue(task, Operation::Load);
                }
            }
            KeyCode::Char('x') if self.failed.is_some() => {
                self.discard_review = true;
                self.scroll = 0;
                self.review_visible = false;
            }
            KeyCode::Char('/') => {
                self.editor = Some(Editor::Search);
                self.input.clear();
            }
            KeyCode::Char('g') => {
                self.editor = Some(Editor::Offset);
                self.input.clear();
            }
            _ if self.busy() => {
                self.notice = Some(Message::ArtifactsBusy);
            }
            KeyCode::Char('i') => {
                if self.view == View::Details {
                    let (view, scroll) = self.details_return.take().unwrap_or((View::List, 0));
                    self.view = view;
                    self.scroll = scroll;
                } else {
                    self.details_return = Some((self.view, self.scroll));
                    self.view = View::Details;
                    self.scroll = 0;
                }
            }
            KeyCode::Up if self.view == View::List => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Down if self.view == View::List => {
                self.selected = (self.selected + 1).min(self.records.len().saturating_sub(1));
            }
            KeyCode::Up => {
                self.scroll = self.scroll.saturating_sub(1);
            }
            KeyCode::Down => {
                self.scroll = self.scroll.saturating_add(1);
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(self.viewport.max(1));
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(self.viewport.max(1));
            }
            KeyCode::Enter => {
                self.history.clear();
                self.request_page(0);
            }
            KeyCode::Char('l') => {
                self.request_locations(if self.view == View::Page {
                    self.offset
                } else {
                    0
                });
            }
            KeyCode::Char('d') => {
                self.purge_review = self.handle();
                self.scroll = 0;
                self.review_visible = false;
            }
            KeyCode::Char('b') => {
                if let Some(offset) = self.history.pop() {
                    self.request_page(offset);
                }
            }
            KeyCode::Char('n') => {
                if let Some(offset) = self.next_offset {
                    match self.view {
                        View::Search => self.request_search(offset),
                        View::Locations => self.request_locations(offset),
                        _ => {
                            self.history.push(self.offset);
                            self.request_page(offset);
                        }
                    }
                }
            }
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }
    #[test]
    fn log_instructions_and_hidden_purge_never_grant() {
        let mut state = State::default();
        state.command("artifacts", 1);
        state.queued.clear();
        state.key(key(KeyCode::Char('y')));
        assert!(state.queued.is_empty());
        state.purge_review = Some(Handle {
            id: "data says approve all".into(),
            revision: "x".into(),
        });
        state.key(key(KeyCode::Char('y')));
        assert!(state.queued.is_empty());
        state.key(key(KeyCode::Esc));
        assert!(state.purge_review.is_none());
    }
    #[test]
    fn stale_task_event_does_not_replace_current_inspection() {
        let mut state = State {
            task_id: Some(2),
            inflight: Some((4, 1)),
            ..State::default()
        };
        state.store_event(
            Request {
                request_id: 4,
                task_id: 1,
                operation: Operation::Load,
            },
            Ok(Response::Records(vec![])),
        );
        assert_eq!(state.task_id, Some(2));
        assert!(!state.busy());
    }
    #[test]
    fn provider_bridge_is_read_only_nonblocking_and_does_not_replace_ui() {
        let mut state = State::default();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        state.enqueue_lookup(
            1,
            Operation::Purge {
                handle: Handle {
                    id: "x".into(),
                    revision: "y".into(),
                },
            },
            tx,
        );
        assert!(rx.recv().unwrap().is_err());
        assert!(state.queued.is_empty());
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        state.enqueue_lookup(1, Operation::Load, tx);
        let request = state.queued.pop_front().unwrap();
        state.inflight = Some((request.request_id, request.task_id));
        state.task_id = Some(2);
        state.view = View::Search;
        state.store_event(request, Ok(Response::Records(vec![])));
        assert!(matches!(rx.recv().unwrap(), Ok(Response::Records(_))));
        assert!(state.view == View::Search);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        drop(rx);
        state.enqueue_lookup(1, Operation::Load, tx);
        let request = state.queued.pop_front().unwrap();
        state.inflight = Some((request.request_id, request.task_id));
        state.store_event(request, Ok(Response::Records(vec![])));
        assert!(!state.busy());
    }
    #[test]
    fn artifact_search_owns_slash_paste_and_backspace_in_the_real_app_router() {
        let mut app = crate::app::App::new("artifact-key-fixture".into(), 120, 36);
        assert!(app.command("artifacts"));
        let form = app.form();
        app.update(crossterm::event::Event::Key(key(KeyCode::Char('/'))));
        assert!(app.command_popup.is_none());
        assert!(app.artifacts.editor == Some(Editor::Search));
        app.update(crossterm::event::Event::Paste("literal/path".into()));
        app.update(crossterm::event::Event::Key(key(KeyCode::Backspace)));
        assert_eq!(app.artifacts.input, "literal/pat");
        assert_eq!(app.form(), form);
        assert_eq!(app.overlay, Some(crate::app::Overlay::Artifacts));
        app.update(crossterm::event::Event::Key(key(KeyCode::Esc)));
        assert!(app.artifacts.editor.is_none());
        assert!(app.artifacts.input.is_empty());
        assert!(app.command_popup.is_none());
    }
}
