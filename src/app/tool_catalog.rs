//! Local catalog search/setup owns its own input and never submits a request.
use crate::{
    i18n::Message,
    providers::{
        self, Agent,
        catalog::{Document, Selection},
    },
    store::tool_catalog::{self as storage, Operation, Request, Response},
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::{cell::Cell, collections::BTreeSet, path::PathBuf};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Browse,
    Search,
    Path,
    SetupReview,
    Details,
}
pub enum Action {
    Changed,
    Close,
}
#[derive(Default)]
pub struct State {
    pub document: Option<Document>,
    pub candidate: Option<Document>,
    pub mode: Mode,
    pub query: String,
    pub path: String,
    pub cursor: usize,
    pub ranked: Vec<usize>,
    pub focused: usize,
    pub selected: BTreeSet<usize>,
    pub schema_counts: Option<providers::catalog::TokenComparison>,
    pub pending: Option<Request>,
    pub inflight: bool,
    pub failed: Option<Request>,
    pub error: String,
    pub notice: Option<Message>,
    pub reviewed: Cell<bool>,
    review_extent: Cell<usize>,
    review_width: Cell<u16>,
    pub scroll: u16,
    pub details: String,
    project: String,
    agent: Agent,
    owner: (i64, i64),
    return_mode: Mode,
    after_load: Option<Mode>,
    disconnect_after_settle: Option<Agent>,
}
impl State {
    pub fn command(
        &mut self,
        name: &str,
        argument: &str,
        project: &str,
        agent: Agent,
        task_id: i64,
        partner: i64,
    ) -> bool {
        if !matches!(name, "tools" | "tool-setup" | "tool-clear") {
            return false;
        }
        if self.inflight || self.pending.is_some() {
            self.notice = Some(Message::ToolCatalogWaiting);
            return true;
        }
        if self.project != project || self.owner != (task_id, partner) || self.agent != agent {
            self.selected.clear();
            self.schema_counts = None;
        }
        let changed_project = self.project != project;
        self.project = project.into();
        self.owner = (task_id, partner);
        self.agent = agent;
        self.error.clear();
        self.notice = None;
        self.candidate = None;
        self.invalidate();
        self.scroll = 0;
        if name == "tool-clear" {
            self.selected.clear();
            self.schema_counts = None;
            self.notice = Some(Message::ToolCatalogCleared);
            self.mode = Mode::Browse;
            return true;
        }
        if name == "tool-setup" {
            self.mode = Mode::Path;
            self.path = argument.into();
            self.cursor = self.path.len();
            // Hydrate current setup before staging a compare-and-swap replacement.
            if changed_project || self.document.is_none() {
                self.after_load = Some(Mode::Path);
                self.queue(Operation::Load);
            } else if !argument.is_empty() {
                self.import();
            }
        } else {
            self.mode = if argument.is_empty() {
                Mode::Browse
            } else {
                Mode::Search
            };
            self.query = argument.into();
            self.cursor = self.query.len();
            self.focused = 0;
            if changed_project || self.document.is_none() {
                self.after_load = Some(self.mode);
                self.queue(Operation::Load);
            } else {
                self.rank();
            }
        }
        true
    }
    pub fn invalidate(&self) {
        self.reviewed.set(false);
        self.review_extent.set(0);
        self.review_width.set(0);
    }
    pub fn observe_review(&self, offset: usize, total: usize, height: usize, width: u16) -> bool {
        if self.review_width.get() != width {
            self.review_extent.set(0);
            self.review_width.set(width);
        }
        if offset <= self.review_extent.get() {
            self.review_extent.set(
                self.review_extent
                    .get()
                    .max(offset.saturating_add(height).min(total)),
            );
        }
        self.review_extent.get() >= total
    }
    pub fn busy(&self) -> bool {
        self.inflight || self.pending.is_some()
    }
    fn queue(&mut self, operation: Operation) {
        self.pending = Some(Request {
            project: self.project.clone(),
            operation,
        });
    }
    fn rank(&mut self) {
        self.ranked = self
            .document
            .as_ref()
            .map_or_else(Vec::new, |doc| doc.manifest.search(&self.query));
        self.focused = self.focused.min(self.ranked.len().saturating_sub(1));
    }
    fn refresh_counts(&mut self) {
        self.schema_counts = self.document.as_ref().and_then(|doc| {
            providers::catalog::token_comparison(
                &doc.manifest,
                &self.selected.iter().copied().collect::<Vec<_>>(),
            )
            .ok()
        });
    }
    fn import(&mut self) {
        if self.path.is_empty() || self.path.len() > 4096 || self.path.chars().any(char::is_control)
        {
            self.notice = Some(Message::ToolCatalogChoosePath);
            return;
        }
        match providers::direct::catalog_connection_scope(self.agent) {
            Ok(connection_scope) => self.queue(Operation::Import {
                path: PathBuf::from(&self.project).join(&self.path),
                agent: self.agent,
                connection_scope,
            }),
            Err(error) => self.error = error,
        }
    }
    pub fn response(&mut self, request: &Request, result: Result<Response, String>) {
        self.inflight = false;
        if request.project != self.project {
            return;
        }
        match result {
            Err(error) => {
                self.failed = Some(request.clone());
                self.error = error;
                self.reviewed.set(false);
            }
            Ok(response) => {
                self.failed = None;
                self.error.clear();
                match response {
                    Response::Loaded(document) => {
                        self.document = document;
                        self.rank();
                        if let Some(mode) = self.after_load.take() {
                            self.mode = mode;
                        }
                        if self.mode == Mode::Path && !self.path.is_empty() {
                            self.import();
                        }
                    }
                    Response::Imported(document) => {
                        self.details = serde_json::to_string_pretty(&document).unwrap_or_default();
                        self.candidate = Some(document);
                        self.mode = Mode::SetupReview;
                        self.invalidate();
                        self.scroll = 0;
                    }
                    Response::Saved(document) => {
                        self.document = Some(document);
                        self.candidate = None;
                        self.selected.clear();
                        self.schema_counts = None;
                        self.mode = Mode::Browse;
                        self.rank();
                        self.notice = Some(Message::ToolCatalogSaved);
                    }
                    Response::Disconnected(document) => {
                        self.document = document;
                        self.selected.clear();
                        self.schema_counts = None;
                        self.notice = Some(Message::ToolCatalogDisconnected);
                    }
                }
            }
        }
        if let Some(agent) = self.disconnect_after_settle.take() {
            self.disconnected(agent);
        }
    }
    /// Called only for a correlated current-owner native connection failure.
    pub fn disconnected(&mut self, agent: Agent) {
        if self
            .document
            .as_ref()
            .is_none_or(|document| document.agent != agent)
        {
            return;
        }
        self.selected.clear();
        self.schema_counts = None;
        self.reviewed.set(false);
        if let Some(document) = &self.document {
            match storage::document_digest(document) {
                Ok(expected) if !document.connection_scope.is_empty() => {
                    let request = Request {
                        project: self.project.clone(),
                        operation: Operation::Disconnect { expected },
                    };
                    // Never drop an accepted save. Parent pumps disconnect after its
                    // in-flight setup has settled; local selection is already revoked.
                    if self.busy() {
                        self.disconnect_after_settle = Some(agent);
                    } else {
                        self.pending = Some(request);
                    }
                }
                Err(error) => self.error = error,
                _ => {}
            }
        }
    }
    pub fn selection(
        &self,
        agent: Agent,
        task_id: i64,
        partner: i64,
    ) -> Result<Option<Selection>, String> {
        if self.selected.is_empty() {
            return Ok(None);
        }
        if self.busy()
            || self.failed.is_some()
            || self.owner != (task_id, partner)
            || self.agent != agent
        {
            return Err(
                "Finish catalog setup for this exact task/partner/adapter before dispatch".into(),
            );
        }
        let document = self
            .document
            .as_ref()
            .ok_or("Reviewed tool setup is unavailable")?;
        document
            .select(
                &self.selected.iter().copied().collect::<Vec<_>>(),
                task_id,
                partner,
            )
            .map(Some)
    }
    pub fn paste(&mut self, value: &str) {
        if self.busy()
            || !matches!(self.mode, Mode::Search | Mode::Path)
            || value.chars().any(char::is_control)
        {
            return;
        }
        let target = if self.mode == Mode::Path {
            &mut self.path
        } else {
            &mut self.query
        };
        if target.len().saturating_add(value.len()) > 4096 {
            return;
        }
        target.insert_str(self.cursor, value);
        self.cursor += value.len();
        if self.mode == Mode::Search {
            self.rank();
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Action {
        if key.kind != KeyEventKind::Press {
            return Action::Changed;
        }
        if key.code == KeyCode::Esc {
            self.reviewed.set(false);
            return Action::Close;
        }
        if self.busy() {
            return Action::Changed;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return Action::Changed;
        }
        if matches!(self.mode, Mode::Search | Mode::Path) {
            let text = if self.mode == Mode::Path {
                &mut self.path
            } else {
                &mut self.query
            };
            match key.code {
                KeyCode::Char(ch) if !ch.is_control() => {
                    if text.len() + ch.len_utf8() <= 4096 {
                        text.insert(self.cursor, ch);
                        self.cursor += ch.len_utf8();
                    }
                }
                KeyCode::Backspace if self.cursor > 0 => {
                    let previous = text[..self.cursor]
                        .grapheme_indices(true)
                        .next_back()
                        .map_or(0, |(i, _)| i);
                    text.replace_range(previous..self.cursor, "");
                    self.cursor = previous;
                }
                KeyCode::Delete if self.cursor < text.len() => {
                    let end = text[self.cursor..]
                        .grapheme_indices(true)
                        .nth(1)
                        .map_or(text.len(), |(i, _)| self.cursor + i);
                    text.replace_range(self.cursor..end, "");
                }
                KeyCode::Left => {
                    self.cursor = text[..self.cursor]
                        .grapheme_indices(true)
                        .next_back()
                        .map_or(0, |(i, _)| i)
                }
                KeyCode::Right => {
                    self.cursor = text[self.cursor..]
                        .grapheme_indices(true)
                        .nth(1)
                        .map_or(text.len(), |(i, _)| self.cursor + i)
                }
                KeyCode::Home => self.cursor = 0,
                KeyCode::End => self.cursor = text.len(),
                KeyCode::Enter if self.mode == Mode::Path => self.import(),
                KeyCode::Enter | KeyCode::Tab if self.mode == Mode::Search => {
                    self.mode = Mode::Browse
                }
                _ => {}
            }
            if self.mode == Mode::Search {
                self.rank();
            }
            return Action::Changed;
        }
        match key.code {
            KeyCode::Char('r') if self.failed.is_some() => {
                self.pending = self.failed.take();
            }
            KeyCode::Char('y')
                if self.mode == Mode::SetupReview
                    && self.reviewed.get()
                    && key.modifiers.is_empty() =>
            {
                if let Some(document) = self.candidate.clone() {
                    match self
                        .document
                        .as_ref()
                        .map(storage::document_digest)
                        .transpose()
                    {
                        Ok(expected) => {
                            self.queue(Operation::Save { document, expected });
                            self.reviewed.set(false);
                        }
                        Err(error) => self.error = error,
                    }
                }
            }
            KeyCode::Char('n') if self.mode == Mode::SetupReview => {
                self.candidate = None;
                self.reviewed.set(false);
                return Action::Close;
            }
            KeyCode::Char('d') if self.mode == Mode::Details => {
                self.mode = self.return_mode;
                self.scroll = 0;
            }
            KeyCode::Char('d') | KeyCode::Enter => {
                self.return_mode = self.mode;
                if self.mode == Mode::Browse
                    && let Some(entry) = self
                        .ranked
                        .get(self.focused)
                        .and_then(|i| self.document.as_ref()?.manifest.tools.get(*i))
                {
                    self.details = serde_json::to_string_pretty(entry).unwrap_or_default();
                }
                self.mode = Mode::Details;
                self.scroll = 0;
                self.reviewed.set(false);
            }
            KeyCode::Tab | KeyCode::Char('/') if self.mode == Mode::Browse => {
                self.mode = Mode::Search;
                self.cursor = self.query.len();
            }
            KeyCode::Char(' ') if self.mode == Mode::Browse => {
                if let Some(index) = self.ranked.get(self.focused).copied()
                    && !self.selected.remove(&index)
                {
                    if self.selected.len() < 16 {
                        self.selected.insert(index);
                    } else {
                        self.notice = Some(Message::ToolCatalogLimit);
                    }
                }
                self.refresh_counts();
            }
            KeyCode::Up if self.mode == Mode::Browse => {
                self.focused = self.focused.saturating_sub(1)
            }
            KeyCode::Down if self.mode == Mode::Browse => {
                self.focused = (self.focused + 1).min(self.ranked.len().saturating_sub(1))
            }
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(6),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(6),
            _ => {}
        }
        Action::Changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        providers::catalog::{EXECUTOR_VERSION, Effect, Entry, Manifest, Route},
        store,
    };
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn next(worker: &store::Worker) -> store::Event {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(event) = worker.try_event() {
                return event;
            }
            assert!(
                Instant::now() < deadline,
                "Catalog writer acknowledgment timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn unrelated_provider_failure_preserves_catalog_but_matching_failure_revokes_durably() {
        let root = std::env::temp_dir().join(format!(
            "ev-catalog-disconnect-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("project")).unwrap();
        let root = root.canonicalize().unwrap();
        let project = root.join("project").to_str().unwrap().to_owned();
        let path = root.join("tools.json");
        let manifest = Manifest {
            schema_version: 1,
            tools: vec![Entry {
                namespace: "fixture".into(),
                name: "inspect".into(),
                version: EXECUTOR_VERSION.into(),
                description: "Inert source inspection metadata".into(),
                effects: vec![Effect::ReadProject],
                route: Route::ReadFile,
                parameters: Route::ReadFile.schema(),
            }],
        };
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let document = Document::import(&path, Agent::Compatible, "a".repeat(64)).unwrap();
        let worker =
            store::Worker::start(root.join("data"), project.clone(), store::Policy::default())
                .unwrap();
        assert!(matches!(next(&worker), store::Event::Loaded(_)));
        let mut save = Some(Request {
            project: project.clone(),
            operation: Operation::Save {
                document: document.clone(),
                expected: None,
            },
        });
        assert!(worker.try_tool_catalog(&mut save));
        let store::Event::ToolCatalog { result, .. } = next(&worker) else {
            panic!("Expected actual catalog save");
        };
        assert!(matches!(result.unwrap(), Response::Saved(_)));
        let pending = Request {
            project: project.clone(),
            operation: Operation::Load,
        };
        let mut state = State {
            document: Some(document.clone()),
            project: project.clone(),
            agent: Agent::Compatible,
            owner: (7, 1),
            selected: BTreeSet::from([0]),
            pending: Some(pending.clone()),
            ..State::default()
        };
        state.reviewed.set(true);
        let digest = storage::document_digest(&document).unwrap();
        state.disconnected(Agent::Anthropic);
        assert_eq!(state.document, Some(document.clone()));
        assert_eq!(
            storage::document_digest(state.document.as_ref().unwrap()).unwrap(),
            digest
        );
        assert_eq!(
            state.document.as_ref().unwrap().connection_scope,
            "a".repeat(64)
        );
        assert_eq!(state.selected, BTreeSet::from([0]));
        assert_eq!(state.pending, Some(pending));
        assert!(state.reviewed.get());
        assert!(state.disconnect_after_settle.is_none());

        state.pending = None;
        state.disconnected(Agent::Compatible);
        assert!(state.selected.is_empty());
        assert!(!state.reviewed.get());
        assert!(
            matches!(state.pending.as_ref().map(|request| &request.operation), Some(Operation::Disconnect { expected }) if *expected == digest)
        );
        assert!(worker.try_tool_catalog(&mut state.pending));
        state.inflight = true;
        let store::Event::ToolCatalog { request, result } = next(&worker) else {
            panic!("Expected actual catalog disconnect");
        };
        state.response(&request, result);
        assert!(state.document.as_ref().unwrap().connection_scope.is_empty());
        let mut load = Some(Request {
            project,
            operation: Operation::Load,
        });
        assert!(worker.try_tool_catalog(&mut load));
        let store::Event::ToolCatalog { result, .. } = next(&worker) else {
            panic!("Expected persisted catalog read");
        };
        let Response::Loaded(Some(retained)) = result.unwrap() else {
            panic!("Expected retained reviewed metadata");
        };
        assert!(retained.connection_scope.is_empty());
        assert_eq!(retained.manifest, document.manifest);
        drop(worker);
        std::fs::remove_dir_all(root).unwrap();
    }
}
