//! Task-correlated artifact inspection. No input here approves or executes a tool.
use crate::{messages::Message, store, workspace::artifacts::*};
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
    cursor: Option<(Handle, View)>,
    // Only the latest explicit UI request may present an inspection. Provider
    // replies and durable capture/purge acknowledgments have separate ownership.
    inspection: Option<u64>,
    details_return: Option<(View, usize)>,
    lookups: BTreeMap<u64, SyncSender<Result<Response, String>>>,
    captures: BTreeMap<u64, SyncSender<Result<Record, String>>>,
    capture_owners: BTreeMap<u64, String>,
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
    fn inspect(&mut self, task: i64, operation: Operation) {
        self.enqueue(task, operation);
        self.inspection = Some(self.serial);
    }
    fn clear_inspection(&mut self) {
        self.inspection = None;
        self.cursor = None;
        self.history.clear();
        self.slices.clear();
        self.locations.clear();
        self.next_offset = None;
        self.offset = 0;
        self.scroll = 0;
        self.details_return = None;
    }
    fn cursor_matches(&self, view: View) -> bool {
        self.cursor.as_ref().is_some_and(|(handle, mode)| {
            *mode == view
                && self
                    .selected_record()
                    .is_some_and(|record| &record.handle == handle && !record.unavailable)
        })
    }
    pub fn purge_record(&self) -> Option<&Record> {
        let handle = self.purge_review.as_ref()?;
        self.records.iter().find(|record| {
            &record.handle == handle && Some(record.task_id) == self.task_id && !record.unavailable
        })
    }
    pub fn validate_purge_review(&mut self) {
        if self.purge_review.is_some() && self.purge_record().is_none() {
            self.purge_review = None;
            self.review_visible = false;
            self.scroll = 0;
        }
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
        let owner = capture.request_id.clone();
        self.retain(task, capture);
        self.captures.insert(self.serial, reply);
        self.capture_owners.insert(self.serial, owner);
    }
    pub(super) fn retention_pending(&self, request_id: &str) -> bool {
        self.capture_owners.values().any(|owner| owner == request_id)
            || self.failed.as_ref().is_some_and(|request| {
                matches!(&request.operation, Operation::RetainTool(capture) if capture.request_id == request_id)
            })
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
        }
        self.clear_inspection();
        self.open = true;
        self.view = View::List;
        self.purge_review = None;
        self.discard_review = false;
        self.editor = None;
        self.review_visible = false;
        self.scroll = 0;
        self.inspect(task_id, Operation::Load);
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
        self.capture_owners.remove(&request.request_id);
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
        if matches!(
            &request.operation,
            Operation::Load
                | Operation::Read { .. }
                | Operation::Search { .. }
                | Operation::Locations { .. }
        ) {
            if !self.open
                || self.task_id != Some(request.task_id)
                || self.inspection != Some(request.request_id)
            {
                return;
            }
            self.inspection = None;
            let handle = match &request.operation {
                Operation::Read { handle, .. }
                | Operation::Search { handle, .. }
                | Operation::Locations { handle, .. } => Some(handle),
                _ => None,
            };
            if handle.is_some_and(|handle| {
                self.selected_record()
                    .is_none_or(|record| &record.handle != handle || record.unavailable)
            }) {
                return;
            }
            if let Ok(response) = &result {
                let matches = match (&request.operation, response) {
                    (Operation::Load, Response::Records(_)) => true,
                    (Operation::Read { handle, offset, .. }, Response::Page(slice)) => {
                        &slice.handle == handle && slice.offset == *offset
                    }
                    (Operation::Search { handle, query, .. }, Response::Search(result)) => {
                        &result.handle == handle
                            && &result.query == query
                            && result.slices.iter().all(|slice| &slice.handle == handle)
                    }
                    (Operation::Locations { handle, .. }, Response::Locations { slices, .. }) => {
                        slices.iter().all(|located| &located.slice.handle == handle)
                    }
                    _ => false,
                };
                if !matches {
                    return;
                }
            }
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
                let inspected = self.handle();
                self.records = records;
                if let Some(handle) = inspected {
                    if let Some(index) = self
                        .records
                        .iter()
                        .position(|record| record.handle == handle)
                    {
                        self.selected = index;
                        if self.records[index].unavailable {
                            self.clear_inspection();
                            self.view = View::List;
                        }
                    } else {
                        self.clear_inspection();
                        self.view = View::List;
                    }
                }
                self.selected = self.selected.min(self.records.len().saturating_sub(1));
                self.validate_purge_review();
            }
            Response::Retained(record) => {
                let inspected = self.handle();
                if let Some(existing) = self.records.iter_mut().find(|r| r.handle == record.handle)
                {
                    *existing = record;
                } else {
                    self.records.insert(0, record);
                }
                if let Some(index) = inspected.and_then(|handle| {
                    self.records
                        .iter()
                        .position(|record| record.handle == handle)
                }) {
                    self.selected = index;
                }
                self.validate_purge_review();
                if self
                    .selected_record()
                    .is_some_and(|record| record.unavailable)
                {
                    self.clear_inspection();
                    self.view = View::List;
                }
            }
            Response::Page(slice) => {
                if self.view != View::Page || !self.cursor_matches(View::Page) {
                    self.history.clear();
                }
                self.cursor = Some((slice.handle.clone(), View::Page));
                self.locations.clear();
                self.details_return = None;
                self.offset = slice.offset;
                self.next_offset = slice.next_offset;
                self.slices = vec![slice];
                self.view = View::Page;
                self.scroll = 0;
            }
            Response::Search(result) => {
                self.history.clear();
                self.cursor = Some((result.handle.clone(), View::Search));
                self.locations.clear();
                self.details_return = None;
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
                self.history.clear();
                self.cursor = self.handle().map(|handle| (handle, View::Locations));
                self.slices.clear();
                self.details_return = None;
                self.locations = slices;
                self.next_offset = next_offset;
                self.view = View::Locations;
                self.scroll = 0;
            }
            Response::Purged { handle, warning } => {
                if let Some(record) = self.records.iter_mut().find(|r| r.handle == handle) {
                    record.unavailable = true;
                }
                if self.handle().as_ref() == Some(&handle) {
                    self.clear_inspection();
                    self.view = View::List;
                }
                self.validate_purge_review();
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
            self.inspect(
                task,
                Operation::Read {
                    handle,
                    offset,
                    limit: PREVIEW_BYTES,
                },
            );
        }
    }
    fn request_search(&mut self, offset: u64, query: String) {
        if offset == 0 {
            // A new query must not inherit the previous query's continuation,
            // including when the new search fails before producing a result.
            self.cursor = None;
            self.next_offset = None;
            self.history.clear();
        }
        if let (Some(task), Some(handle)) = (self.task_id, self.handle()) {
            self.inspect(
                task,
                Operation::Search {
                    handle,
                    query,
                    offset,
                    limit: MAX_MATCHES,
                },
            );
        }
    }
    fn request_locations(&mut self, offset: u64) {
        if let (Some(task), Some(handle)) = (self.task_id, self.handle()) {
            self.inspect(task, Operation::Locations { handle, offset });
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
        self.validate_purge_review();
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
                            let query = std::mem::take(&mut self.input);
                            self.editor = None;
                            self.request_search(0, query);
                        }
                        Editor::Offset => match self.input.parse::<u64>() {
                            Ok(offset) => {
                                self.editor = None;
                                if self.view == View::Page && self.cursor_matches(View::Page) {
                                    self.history.push(self.offset);
                                }
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
                self.inspection = None;
                if self.view == View::Details {
                    let (view, scroll) = self.details_return.take().unwrap_or((View::List, 0));
                    self.view = view;
                    self.scroll = scroll;
                } else if self.view != View::List {
                    self.clear_inspection();
                    self.view = View::List;
                    self.scroll = 0;
                } else {
                    self.clear_inspection();
                    self.open = false;
                }
            }
            KeyCode::Char('r') => {
                if let Some(request) = self.failed.take() {
                    self.pending = Some(request);
                    self.error = None;
                } else if let Some(task) = self.task_id {
                    self.inspect(task, Operation::Load);
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
                self.clear_inspection();
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Down if self.view == View::List => {
                self.clear_inspection();
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
                self.request_locations(
                    if self.view == View::Page && self.cursor_matches(View::Page) {
                        self.offset
                    } else {
                        0
                    },
                );
            }
            KeyCode::Char('d') => {
                self.purge_review = self.handle();
                self.scroll = 0;
                self.review_visible = false;
            }
            KeyCode::Char('b') if self.view == View::Page && self.cursor_matches(View::Page) => {
                if let Some(offset) = self.history.pop() {
                    self.request_page(offset);
                }
            }
            KeyCode::Char('n') if self.cursor_matches(self.view) => {
                if let Some(offset) = self.next_offset {
                    match self.view {
                        View::Search => self.request_search(offset, self.query.clone()),
                        View::Locations => self.request_locations(offset),
                        View::Page => {
                            self.history.push(self.offset);
                            self.request_page(offset);
                        }
                        _ => {}
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
    #[test]
    fn child_capture_ownership_waits_for_queued_inflight_and_failed_retention() {
        let mut state = State::default();
        let (reply, received) = std::sync::mpsc::sync_channel(1);
        state.retain_with_reply(
            1,
            ToolCapture {
                request_id: "child-request".into(),
                call_id: "read-1".into(),
                tool: "eevee_read_file".into(),
                partner: None,
                text: "actual observation".into(),
                failed: false,
            },
            reply,
        );
        assert!(state.retention_pending("child-request"));
        assert!(!state.retention_pending("another-request"));
        let request = state.queued.pop_front().unwrap();
        state.inflight = Some((request.request_id, request.task_id));
        assert!(state.retention_pending("child-request"));
        state.store_event(request, Err("fixture storage failure".into()));
        assert!(received.try_recv().unwrap().is_err());
        assert!(
            state.retention_pending("child-request"),
            "transport cancellation cannot abandon a failed durable write"
        );
        let retry = state.failed.take().unwrap();
        state.inflight = Some((retry.request_id, retry.task_id));
        state.store_event(retry, Ok(Response::Retained(record("child-artifact"))));
        assert!(!state.retention_pending("child-request"));
    }
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }
    fn record(name: &str) -> Record {
        Record {
            handle: Handle {
                id: name.into(),
                revision: format!("{name}-revision"),
            },
            task_id: 1,
            request_id: format!("{name}-request"),
            partner: None,
            origin: Origin::OwnedRun {
                run_id: name.into(),
            },
            created_at_ms: 0,
            exit: ExitSummary {
                status: "completed".into(),
                exit_code: Some(0),
                signal: None,
                error: None,
            },
            total_bytes: 16384,
            captured_bytes: 16384,
            omitted_bytes: 0,
            sha256: digest(name.as_bytes()),
            observation_sha256: None,
            unavailable: false,
        }
    }
    fn dispatch(state: &mut State) -> Request {
        let request = state.queued.pop_front().expect("expected UI request");
        state.inflight = Some((request.request_id, request.task_id));
        request
    }
    fn respond(state: &mut State, response: Response) {
        let request = dispatch(state);
        state.store_event(request, Ok(response));
    }
    fn loaded() -> State {
        let mut state = State::default();
        state.command("artifacts", 1);
        respond(
            &mut state,
            Response::Records(vec![record("A"), record("B")]),
        );
        state
    }
    fn page(state: &mut State, expected: &str, start: u64, next: Option<u64>) {
        let request = dispatch(state);
        let Operation::Read { handle, offset, .. } = &request.operation else {
            panic!("expected page request");
        };
        assert_eq!(handle.id, expected);
        assert_eq!(*offset, start);
        let slice = Slice {
            handle: handle.clone(),
            offset: start,
            text: format!("{expected} bytes at {start}"),
            next_offset: next,
        };
        state.store_event(request, Ok(Response::Page(slice)));
        assert!(state.view == View::Page);
        assert_eq!(state.slices[0].text, format!("{expected} bytes at {start}"));
    }
    #[test]
    fn artifact_pagination_never_crosses_list_selection_details_or_reopen() {
        let mut state = loaded();
        state.key(key(KeyCode::Enter));
        page(&mut state, "A", 0, Some(4096));
        state.key(key(KeyCode::Char('n')));
        page(&mut state, "A", 4096, Some(8192));
        state.key(key(KeyCode::Char('b')));
        page(&mut state, "A", 0, Some(4096));
        state.key(key(KeyCode::Char('n')));
        page(&mut state, "A", 4096, Some(8192));
        state.key(key(KeyCode::Char('i')));
        state.key(key(KeyCode::Char('n')));
        state.key(key(KeyCode::Char('b')));
        assert!(state.queued.is_empty());
        state.key(key(KeyCode::Esc));
        assert!(state.view == View::Page);
        assert_eq!(state.slices[0].text, "A bytes at 4096");
        state.key(key(KeyCode::Esc));
        state.key(key(KeyCode::Down));
        assert_eq!(state.selected_record().unwrap().handle.id, "B");
        for details in [false, true] {
            if details {
                state.key(key(KeyCode::Char('i')));
            }
            state.key(key(KeyCode::Char('n')));
            state.key(key(KeyCode::Char('b')));
            assert!(state.queued.is_empty());
        }
        state.key(key(KeyCode::Enter));
        page(&mut state, "B", 0, Some(4096));
        state.key(key(KeyCode::Char('n')));
        page(&mut state, "B", 4096, Some(8192));
        state.command("artifacts", 1);
        respond(
            &mut state,
            Response::Records(vec![record("B"), record("A")]),
        );
        assert_eq!(state.selected_record().unwrap().handle.id, "B");
        state.key(key(KeyCode::Char('n')));
        state.key(key(KeyCode::Char('b')));
        assert!(state.queued.is_empty());
        state.key(key(KeyCode::Enter));
        page(&mut state, "B", 0, Some(4096));
    }
    #[test]
    fn artifact_search_locations_and_pages_use_only_their_own_cursor() {
        let mut state = loaded();
        state.key(key(KeyCode::Enter));
        page(&mut state, "A", 0, Some(4096));
        state.key(key(KeyCode::Char('n')));
        page(&mut state, "A", 4096, Some(8192));
        state.key(key(KeyCode::Char('/')));
        state.paste("needle");
        state.key(key(KeyCode::Enter));
        respond(
            &mut state,
            Response::Search(SearchResult {
                handle: record("A").handle,
                query: "needle".into(),
                slices: vec![],
                next_offset: Some(7000),
                scanned_bytes: 7000,
                omitted_bytes: 0,
            }),
        );
        state.key(key(KeyCode::Char('b')));
        assert!(state.queued.is_empty());
        state.key(key(KeyCode::Char('n')));
        let request = dispatch(&mut state);
        assert!(
            matches!(&request.operation, Operation::Search { handle, query, offset: 7000, .. }
            if handle.id == "A" && query == "needle")
        );
        state.store_event(
            request,
            Ok(Response::Search(SearchResult {
                handle: record("A").handle,
                query: "needle".into(),
                slices: vec![],
                next_offset: None,
                scanned_bytes: 9384,
                omitted_bytes: 0,
            })),
        );
        state.key(key(KeyCode::Char('l')));
        let request = dispatch(&mut state);
        assert!(matches!(
            &request.operation,
            Operation::Locations { offset: 0, .. }
        ));
        state.store_event(
            request,
            Ok(Response::Locations {
                slices: vec![],
                next_offset: Some(9000),
            }),
        );
        state.key(key(KeyCode::Char('b')));
        assert!(state.queued.is_empty());
        state.key(key(KeyCode::Char('n')));
        let request = dispatch(&mut state);
        assert!(
            matches!(&request.operation, Operation::Locations { handle, offset: 9000 }
            if handle.id == "A")
        );
        state.store_event(
            request,
            Ok(Response::Locations {
                slices: vec![],
                next_offset: None,
            }),
        );
        state.key(key(KeyCode::Char('g')));
        state.paste("123");
        state.key(key(KeyCode::Enter));
        page(&mut state, "A", 123, Some(4219));
        state.key(key(KeyCode::Char('b')));
        assert!(state.queued.is_empty());
        state.key(key(KeyCode::Char('l')));
        let request = dispatch(&mut state);
        assert!(matches!(
            &request.operation,
            Operation::Locations { offset: 123, .. }
        ));
    }
    #[test]
    fn artifact_cancelled_inspection_cannot_reopen_from_late_success_or_error() {
        for mode in [View::Page, View::Search, View::Locations] {
            for reopen in [false, true] {
                let mut state = loaded();
                match mode {
                    View::Page => state.request_page(0),
                    View::Search => state.request_search(0, String::new()),
                    View::Locations => state.request_locations(0),
                    _ => unreachable!(),
                }
                let request = dispatch(&mut state);
                state.key(key(KeyCode::Esc));
                assert!(!state.open);
                if reopen {
                    state.command("artifacts", 1);
                }
                let response = match mode {
                    View::Page => Response::Page(Slice {
                        handle: record("A").handle,
                        offset: 0,
                        text: "cancelled bytes".into(),
                        next_offset: Some(4096),
                    }),
                    View::Search => Response::Search(SearchResult {
                        handle: record("A").handle,
                        query: String::new(),
                        slices: vec![],
                        next_offset: Some(7000),
                        scanned_bytes: 7000,
                        omitted_bytes: 0,
                    }),
                    View::Locations => Response::Locations {
                        slices: vec![],
                        next_offset: Some(9000),
                    },
                    _ => unreachable!(),
                };
                state.store_event(request, Ok(response));
                assert!(state.view == View::List);
                assert!(state.slices.is_empty() && state.next_offset.is_none());
                assert_eq!(state.open, reopen);
                if reopen {
                    respond(
                        &mut state,
                        Response::Records(vec![record("A"), record("B")]),
                    );
                }
            }
        }
        let mut state = loaded();
        state.key(key(KeyCode::Enter));
        let request = dispatch(&mut state);
        state.key(key(KeyCode::Esc));
        state.command("artifacts", 1);
        state.store_event(request, Err("cancelled read failed".into()));
        assert!(state.error.is_none());
        respond(
            &mut state,
            Response::Records(vec![record("A"), record("B")]),
        );
    }
    #[test]
    fn artifact_refresh_and_capture_keep_selection_and_exact_purge_scope() {
        let mut state = loaded();
        state.key(key(KeyCode::Char('r')));
        state.key(key(KeyCode::Char('d'))); // Busy refresh cannot open a review.
        respond(
            &mut state,
            Response::Records(vec![record("B"), record("A")]),
        );
        assert_eq!(state.selected_record().unwrap().handle.id, "A");
        state.key(key(KeyCode::Char('d')));
        state.review_visible = true;
        state.retain(
            1,
            ToolCapture {
                request_id: "C-request".into(),
                call_id: "capture-C".into(),
                tool: "read".into(),
                partner: None,
                text: "C bytes".into(),
                failed: false,
            },
        );
        respond(&mut state, Response::Retained(record("C")));
        assert_eq!(state.selected_record().unwrap().handle.id, "A");
        assert_eq!(state.purge_record().unwrap().handle.id, "A");
        state.key(key(KeyCode::Char('y')));
        let request = dispatch(&mut state);
        assert!(matches!(&request.operation, Operation::Purge { handle } if handle.id == "A"));
        state.store_event(
            request,
            Ok(Response::Purged {
                handle: record("A").handle,
                warning: None,
            }),
        );
        assert!(
            state
                .records
                .iter()
                .find(|record| record.handle.id == "A")
                .unwrap()
                .unavailable
        );
        assert!(
            !state
                .records
                .iter()
                .find(|record| record.handle.id == "C")
                .unwrap()
                .unavailable
        );

        let mut state = loaded();
        state.key(key(KeyCode::Char('r')));
        let request = dispatch(&mut state);
        state.purge_review = Some(record("A").handle);
        state.review_visible = true;
        state.store_event(request, Ok(Response::Records(vec![record("B")])));
        assert!(state.purge_review.is_none() && !state.review_visible);
        state.key(key(KeyCode::Char('y')));
        assert!(state.queued.is_empty());
        assert_eq!(state.selected_record().unwrap().handle.id, "B");
    }
    #[test]
    fn artifact_refresh_preserves_available_page_but_invalidates_missing_or_purged_scope() {
        for unavailable in [false, true] {
            let mut state = loaded();
            state.key(key(KeyCode::Enter));
            page(&mut state, "A", 0, Some(4096));
            state.key(key(KeyCode::Char('n')));
            page(&mut state, "A", 4096, Some(8192));
            state.key(key(KeyCode::Char('r')));
            respond(
                &mut state,
                Response::Records(vec![record("B"), record("A")]),
            );
            assert_eq!(state.selected_record().unwrap().handle.id, "A");
            assert_eq!(state.slices[0].text, "A bytes at 4096");
            state.key(key(KeyCode::Char('b')));
            page(&mut state, "A", 0, Some(4096));
            state.key(key(KeyCode::Char('r')));
            let mut records = vec![record("B")];
            if unavailable {
                let mut purged = record("A");
                purged.unavailable = true;
                records.push(purged);
            }
            respond(&mut state, Response::Records(records));
            assert!(state.view == View::List && state.slices.is_empty());
            assert_eq!(
                state.selected_record().unwrap().handle.id,
                if unavailable { "A" } else { "B" }
            );
            state.key(key(KeyCode::Char('n')));
            state.key(key(KeyCode::Char('b')));
            assert!(state.queued.is_empty());
        }
    }
    #[test]
    fn artifact_failed_new_search_does_not_resume_at_the_old_query_offset() {
        let mut state = loaded();
        state.key(key(KeyCode::Char('/')));
        state.paste("old");
        state.key(key(KeyCode::Enter));
        respond(
            &mut state,
            Response::Search(SearchResult {
                handle: record("A").handle,
                query: "old".into(),
                slices: vec![],
                next_offset: Some(7000),
                scanned_bytes: 7000,
                omitted_bytes: 0,
            }),
        );
        state.key(key(KeyCode::Char('/')));
        state.paste("new");
        state.key(key(KeyCode::Enter));
        let request = dispatch(&mut state);
        state.store_event(request, Err("search failed".into()));
        state.key(key(KeyCode::Char('n')));
        assert!(state.queued.is_empty());
        assert_eq!(state.error.as_deref(), Some("search failed"));
        assert_eq!(
            state.query, "old",
            "failed query must not relabel prior results"
        );
    }
    #[test]
    fn artifact_response_must_match_the_live_selected_handle_and_requested_mode() {
        let mut state = loaded();
        state.key(key(KeyCode::Enter));
        let request = dispatch(&mut state);
        // A page carrying another artifact's identity must never be presented.
        state.store_event(
            request,
            Ok(Response::Page(Slice {
                handle: record("B").handle,
                offset: 0,
                text: "wrong artifact".into(),
                next_offset: Some(4096),
            })),
        );
        assert!(state.view == View::List && state.slices.is_empty());
        state.key(key(KeyCode::Enter));
        let request = dispatch(&mut state);
        state.store_event(
            request,
            Ok(Response::Locations {
                slices: vec![],
                next_offset: Some(9000),
            }),
        );
        assert!(state.view == View::List && state.next_offset.is_none());
        state.key(key(KeyCode::Enter));
        let request = dispatch(&mut state);
        state.selected = 1;
        state.store_event(
            request,
            Ok(Response::Page(Slice {
                handle: record("A").handle,
                offset: 0,
                text: "old selection".into(),
                next_offset: Some(4096),
            })),
        );
        assert!(state.view == View::List && state.slices.is_empty());
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
