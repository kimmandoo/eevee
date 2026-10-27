//! Ephemeral two-stage consent bound to one exact Request draft and caret.
use super::*;
use crate::platform::external_editor::Plan;

#[derive(Default)]
pub struct State {
    pub scope: String,
    pub scroll: u16,
    pub scroll_max: u16,
    pub page_rows: u16,
    pub seen_rows: usize,
    pub scope_rows: usize,
    pub scope_width: u16,
    pub visible: bool,
    pub choice: bool,
    pub waiting: bool,
    launch: bool,
    owner: (i64, i64),
    revision: u64,
    original: String,
    cursor: usize,
    plan: Option<Plan>,
    candidate: Option<String>,
}

impl State {
    pub fn invalidate(&mut self) {
        self.visible = false;
        self.choice = false;
        self.launch = false;
        self.seen_rows = 0;
        self.scroll = 0;
    }
    pub fn leave(&mut self) {
        self.invalidate();
        self.waiting = false;
        self.plan = None;
        self.candidate = None;
    }
    fn show(&mut self, scope: String) {
        self.invalidate();
        self.scope = scope;
        self.scope_rows = 0;
        self.scope_width = 0;
    }
    pub fn actionable(&self) -> bool {
        self.plan.is_some() || self.candidate.is_some()
    }
}

impl App {
    fn external_draft(&self) -> (&str, usize) {
        if self.composer_is_request() {
            return (&self.input, self.cursor);
        }
        self.drafts
            .iter()
            .find(|draft| draft.field == store::EditorField::Request)
            .map_or(("", 0), |draft| (draft.text.as_str(), draft.cursor))
    }

    fn external_owner_matches(&self) -> bool {
        let state = &self.external_editor;
        let (text, cursor) = self.external_draft();
        state.owner == (self.task_id, self.lead.id)
            && state.revision == self.revision
            && state.original == text
            && state.cursor == cursor
    }

    pub(super) fn external_editor_command(&mut self) -> bool {
        let revision = self.revision;
        if self.editing {
            self.retain_action_editor();
        }
        self.command_popup = None;
        self.open_overlay(Overlay::ExternalEditor);
        // Navigation itself is durable. Bind the review after acquiring that
        // revision, rather than before update_at persists the changed surface.
        if self.revision == revision {
            self.changed_implicitly(std::time::Instant::now());
        }
        let (text, cursor) = self.external_draft();
        self.external_editor = State {
            waiting: true,
            owner: (self.task_id, self.lead.id),
            revision: self.revision,
            original: text.to_owned(), cursor,
            scope: "Waiting for committed global idle state. No editor has started; original Request is retained. Esc cancels.".into(),
            ..State::default()
        };
        true
    }

    /// Called only after the sole main loop observes all worker acknowledgments.
    pub fn prepare_external_editor(&mut self) -> bool {
        if !self.external_editor.waiting
            || self.overlay != Some(Overlay::ExternalEditor)
            || !self.lifecycle_ready()
            || self.width < 60
            || self.height < 18
        {
            return false;
        }
        if self.external_editor.owner == (0, self.lead.id) {
            self.external_editor.owner.0 = self.task_id;
        }
        self.external_editor.waiting = false;
        if !self.external_owner_matches() {
            self.external_editor.show("Owner or draft changed while waiting; original Request kept. Reopen /external-edit.".into());
            return true;
        }
        match Plan::prepare() {
            Ok(plan) => {
                let scope = format!(
                    "External Request editor — launch review\nRequest: {} UTF-8 bytes; only this exact owning draft.\n\n{}",
                    self.external_editor.original.len(),
                    plan.review()
                );
                self.external_editor.show(scope);
                self.external_editor.plan = Some(plan);
            }
            Err(error) => self.external_editor.show(error),
        }
        true
    }

    pub fn take_external_editor(&mut self) -> Option<(Plan, String)> {
        if !std::mem::take(&mut self.external_editor.launch) {
            return None;
        }
        if !self.external_review_ready() {
            self.external_editor.leave();
            self.external_editor.show(
                "External editing expired; original Request kept. Reopen /external-edit.".into(),
            );
            return None;
        }
        self.external_editor.invalidate();
        self.external_editor
            .plan
            .take()
            .map(|plan| (plan, self.external_editor.original.clone()))
    }

    pub fn external_editor_returned(&mut self, result: Result<String, String>) {
        if !self.external_owner_matches()
            || self.overlay != Some(Overlay::ExternalEditor)
            || !self.lifecycle_ready()
        {
            self.external_editor.leave();
            self.external_editor.show(
                "Editor returned to a stale owner/draft; candidate refused. Original Request kept."
                    .into(),
            );
            return;
        }
        match result {
            Ok(candidate) => {
                // JSON escaping exposes every byte distinction: CRLF, trailing newlines,
                // tabs and Unicode remain unchanged in the actual candidate.
                let exact = serde_json::to_string(&candidate).expect("String serialization");
                self.external_editor.show(format!("External Request editor — returned candidate\n{} UTF-8 bytes. Replace only this owning draft; nothing sends or selects context.\n\nExact JSON string (\\r\\n = CRLF; \\n = LF; quotes are not imported):\n{}\n\nOriginal: {} UTF-8 bytes, caret {}. Esc/n keeps original text and caret. Temporary file removed; editor-owned copies may remain.", candidate.len(), exact, self.external_editor.original.len(), self.external_editor.cursor));
                self.external_editor.candidate = Some(candidate);
            }
            Err(error) => self.external_editor.show(format!(
                "External editing failed; original Request and caret kept.\n\n{error}"
            )),
        }
    }

    fn external_review_ready(&self) -> bool {
        self.overlay == Some(Overlay::ExternalEditor)
            && self.command_popup.is_none()
            && self.confirmation.is_none()
            && self.width >= 60
            && self.height >= 18
            && self.external_editor.visible
            && self.external_owner_matches()
            && self.lifecycle_ready()
    }

    pub(super) fn external_editor_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press || !key.modifiers.is_empty() {
            return false;
        }
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('n')) {
            self.external_editor.leave();
            self.close_overlay();
            return true;
        }
        if self.width < 60 || self.height < 18 {
            self.external_editor.invalidate();
            return false;
        }
        match key.code {
            KeyCode::Up => {
                self.external_editor.scroll = self.external_editor.scroll.saturating_sub(1)
            }
            KeyCode::Down => {
                self.external_editor.scroll = self.external_editor.scroll.saturating_add(1)
            }
            KeyCode::PageDown => {
                self.external_editor.scroll = self
                    .external_editor
                    .scroll
                    .saturating_add(self.external_editor.page_rows.max(1))
            }
            KeyCode::PageUp => {
                self.external_editor.scroll = self
                    .external_editor
                    .scroll
                    .saturating_sub(self.external_editor.page_rows.max(1))
            }
            KeyCode::Home => self.external_editor.scroll = 0,
            KeyCode::End => self.external_editor.scroll = self.external_editor.scroll_max,
            KeyCode::Left => self.external_editor.choice = false,
            KeyCode::Right | KeyCode::Tab if self.external_review_ready() => {
                self.external_editor.choice = !self.external_editor.choice
            }
            KeyCode::Char('y') | KeyCode::Enter
                if self.external_review_ready()
                    && (key.code == KeyCode::Char('y') || self.external_editor.choice) =>
            {
                if let Some(candidate) = self.external_editor.candidate.take() {
                    self.drafts
                        .retain(|draft| draft.field != store::EditorField::Request);
                    let cursor = candidate.len();
                    self.drafts.push(store::Draft {
                        field: store::EditorField::Request,
                        text: candidate,
                        cursor,
                    });
                    self.sources.clear_wire_request();
                    self.external_editor.leave();
                    self.close_overlay();
                    self.request_home(false);
                    self.changed_task();
                    self.notice = "External candidate staged exactly; nothing sent or selected. Ctrl+S saves locally; ordinary Enter sends.".into();
                } else if self.external_editor.plan.is_some() {
                    self.external_editor.launch = true;
                }
            }
            KeyCode::Enter => {
                self.external_editor.leave();
                self.close_overlay();
            }
            _ => return false,
        }
        self.external_editor.scroll = self
            .external_editor
            .scroll
            .min(self.external_editor.scroll_max);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prepared() -> App {
        let mut app = App::new("/tmp".into(), 80, 24);
        app.store_event(store::Event::Loaded(store::Project {
            active_id: None,
            tasks: Vec::new(),
            partners: vec![store::Partner::default()],
            usage_bytes: 0,
        }));
        let baseline = std::sync::Arc::new(repo::Snapshot {
            head: None,
            files: Default::default(),
            working_diff: repo::WorkingDiff::default(),
            complete: true,
            stable: true,
            warnings: Vec::new(),
        });
        app.repo_event(repo::Event::Updated(std::sync::Arc::new(repo::Report {
            repository: repo::RepositoryIdentity {
                root: "/tmp".into(),
                git_root: None,
                git_dir: None,
                storage_root: None,
            },
            baseline: std::sync::Arc::clone(&baseline),
            current: baseline,
            changes: Vec::new(),
            initial: true,
        })));
        app.input = "original 한글\r\n\n".into();
        app.cursor = 3;
        app.changed_task();
        let mut snapshot = app.snapshot("/tmp");
        snapshot.id = 7;
        app.save_submitted();
        app.store_event(store::Event::Saved(snapshot));
        let memory = app.memory_request.take().expect("first save loads memory");
        app.store_event(store::Event::Memory {
            task_id: memory.task_id,
            result: Ok(Vec::new()),
        });
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        )));
        for character in "external-edit".chars() {
            app.update(Event::Key(KeyEvent::new(
                KeyCode::Char(character),
                KeyModifiers::NONE,
            )));
        }
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let snapshot = app.snapshot("/tmp");
        app.save_submitted();
        app.store_event(store::Event::Saved(snapshot));
        // Taking the worker request is not the discovery acknowledgement.
        assert!(app.agents.pending.take().is_some());
        assert!(app.agents.busy());
        assert!(!app.lifecycle_ready());
        app.agent_event(crate::providers::agent::Event::Discovered(Vec::new()));
        assert!(app.lifecycle_ready());
        assert_eq!(app.external_draft(), ("original 한글\r\n\n", 3));
        // Supply the editor return directly without preparing a host editor.
        app.external_editor.waiting = false;
        app
    }
    #[test]
    fn cancellation_and_stale_owner_never_replace_owned_draft() {
        let mut app = prepared();
        let before = app.external_draft().0.to_owned();
        app.external_editor.candidate = Some("candidate".into());
        app.external_editor.visible = true;
        app.external_editor_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.external_draft(), (before.as_str(), 3));
        app.external_editor_command();
        app.revision += 1;
        app.external_editor_returned(Ok("candidate".into()));
        assert!(app.external_editor.candidate.is_none());
        assert_eq!(app.external_draft(), (before.as_str(), 3));
        assert!(app.pending_request.is_none());
    }
    #[test]
    fn exact_candidate_stages_without_send_or_context_selection() {
        let mut app = prepared();
        let candidate = "returned 한글😀\r\nlast\r\n\n";
        assert!(app.lifecycle_ready());
        app.external_editor_returned(Ok(candidate.into()));
        assert_eq!(app.external_editor.candidate.as_deref(), Some(candidate));
        assert_eq!(app.external_draft(), ("original 한글\r\n\n", 3));
        app.external_editor.visible = true;
        app.external_editor_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(app.input.as_bytes(), candidate.as_bytes());
        assert_eq!(app.cursor, candidate.len());
        assert!(app.pending_request.is_none());
        assert!(app.pending_command.is_none());
        assert!(app.planning.pending.is_none());
        assert!(app.sources.basket.is_empty());
        assert!(app.sources.pending.is_none());
        assert!(app.memory.selected_ids.is_empty());
        assert!(!app.execution.busy());
    }
    #[test]
    fn hidden_or_small_review_cannot_launch() {
        let mut app = prepared();
        app.external_editor_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(!app.external_editor.launch);
        app.external_editor.visible = true;
        app.width = 59;
        app.external_editor_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(!app.external_editor.launch);
    }
}
