use crossterm::event::{KeyCode, KeyEvent};

use super::{App, Overlay};
use crate::store::EditorField;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Files,
}

#[derive(Clone, Copy)]
pub struct Menu {
    pub kind: Kind,
    pub start: usize,
    pub selected: usize,
}

fn subsequence(value: &str, query: &str) -> bool {
    let mut wanted = query.chars().flat_map(char::to_lowercase);
    let Some(mut next) = wanted.next() else {
        return true;
    };
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch == next {
            let Some(following) = wanted.next() else {
                return true;
            };
            next = following;
        }
    }
    false
}

fn file_query_matches(path: &str, query: &str) -> bool {
    query.split_whitespace().all(|term| {
        let term = term
            .strip_prefix("./")
            .or_else(|| term.strip_prefix(".\\"))
            .unwrap_or(term);
        if let Some((directory, name)) = term.rsplit_once(['/', '\\']) {
            let mut components = path.split(['/', '\\']);
            if !directory.split(['/', '\\']).all(|part| {
                components.next().is_some_and(|component| {
                    component
                        .chars()
                        .flat_map(char::to_lowercase)
                        .eq(part.chars().flat_map(char::to_lowercase))
                })
            }) || components.next().is_none()
            {
                return false;
            }
            subsequence(path.rsplit(['/', '\\']).next().unwrap_or(path), name)
        } else {
            subsequence(path, term)
        }
    })
}

impl App {
    pub fn composer_is_request(&self) -> bool {
        self.editing && self.editor_field == EditorField::Request
    }

    pub fn response_on_home(&self) -> bool {
        self.planning.response_visible()
    }

    /// Enter sends the exact committed draft; preparation and storage still gate dispatch.
    pub(super) fn submit_request(&mut self) -> bool {
        if self.width < 60 || self.height < 18 || self.input.trim().is_empty() {
            self.notice = "Type a request and enlarge the terminal before sending.".into();
            return true;
        }
        if self.pending_request.is_some() || self.planning.busy() {
            self.notice = "Finish or cancel this partner's request before sending another. Your draft is kept.".into();
            return true;
        }
        if let Some(reason) = self.work_capacity_reason(self.lead.id) {
            self.notice = reason;
            return true;
        }
        if self.archived {
            self.notice = "Restore this task before preparing a request.".into();
            return true;
        }
        self.planning.prepare_ordinary_operation(self.form());
        if self.goal.trim().is_empty() {
            self.goal.clone_from(&self.input);
        }
        self.changed_task();
        // Explicit source-worker freshness before preparation; the existing
        // committed-input gate waits for this check, and dispatch still revalidates.
        self.sources
            .observe_captures(self.task_id, self.repository.as_ref());
        // Controlled Codex tools belong to a fresh thread, never inherited
        // planning/resumed authority. Ordinary follow-ups must prepare New.
        self.pending_request = Some((
            self.revision,
            self.planning.thread_id.is_some()
                && self.planning.operation != crate::providers::Operation::Review
                && !(self.planning.agent == crate::providers::Agent::Codex
                    && self.planning.operation == crate::providers::Operation::Implement),
        ));
        self.pending_request_owner = Some(self.lead.id);
        self.flush_save();
        self.notice = "Saving request before sending…".into();
        true
    }

    pub(super) fn drive_request(&mut self) -> bool {
        let Some((revision, resume)) = self.pending_request else {
            return false;
        };
        if self
            .pending_request_owner
            .is_some_and(|owner| owner != self.lead.id)
        {
            return false;
        }
        if revision != self.revision {
            self.pending_request = None;
            self.notice =
                "Draft changed while saving. Press Enter to send the current request.".into();
            return true;
        }
        if self.planning.preference_restore_error().is_some()
            || self.planning.preferences_loading()
            || !self.planning_input_ready()
            || self.agents.busy()
        {
            return false;
        }
        self.pending_request = None;
        let status = self.agents.connection_status(self.planning.agent.index());
        if matches!(status, "Not installed" | "Not connected") {
            self.agents.selected = self.planning.agent.index();
            self.agents.page = super::agents::Page::Actions;
            self.agents.action = 0;
            self.agents.notice = format!(
                "{status}. Connect or install this provider, then return to your retained request and press Enter."
            );
            self.composer_return = Some(EditorField::Request);
            self.editing = false;
            self.input.clear();
            self.open_overlay(Overlay::Agents);
            return true;
        }
        // Only the committed snapshot supplies provider input, including redaction.
        self.editing = false;
        self.input.clear();
        self.composer_return = Some(EditorField::Request);
        self.open_overlay(Overlay::Planning);
        self.planning_command(if resume { "resume" } else { "new" });
        if self.planning.owns_work() {
            self.planning.send_after_prepare = true;
            self.request_home(false);
            self.notice = "Preparing request… Enter will not send it twice.".into();
        }
        true
    }

    pub(super) fn request_home(&mut self, consumed: bool) {
        if self.composer_is_request() {
            self.retain_editor();
        }
        if consumed {
            self.drafts.retain(|draft| {
                draft.field != EditorField::Request
                    || draft.text != self.planning.submitted_request.as_ref()
            });
        }
        self.overlay = None;
        self.overlay_history.clear();
        self.composer_return = None;
        self.edit_return = None;
        self.open_editor(EditorField::Request);
        if consumed {
            self.changed_task();
        }
        self.refresh_approvals();
    }

    pub fn composer_query(&self) -> &str {
        self.composer_menu
            .and_then(|menu| self.input.get(menu.start + 1..self.cursor))
            .unwrap_or("")
    }

    pub fn composer_match(&self, index: usize) -> bool {
        let Some(menu) = self.composer_menu else {
            return false;
        };
        let query = self.composer_query();
        match menu.kind {
            Kind::Files => self.sources.files.get(index).is_some_and(|source| {
                source
                    .path
                    .to_str()
                    .is_some_and(|path| file_query_matches(path, query))
            }),
        }
    }

    pub fn composer_candidates(&self) -> impl Iterator<Item = usize> + '_ {
        self.sources
            .ranked_files(self.form())
            .filter(|index| self.composer_match(*index))
    }

    pub fn composer_rank_reason(&self, index: usize) -> Option<super::sources::RankReason> {
        self.sources.rank_reason(self.form(), index)
    }

    /// Called after task-owned state is reset in the main loop. Initial '@'
    /// participates in ordinary draft autosave, then awaits its durable task ID.
    pub fn prepare_composer_sources(&mut self) {
        if !self.composer_catalogue || self.task_id <= 0 || self.sources.busy() {
            return;
        }
        self.composer_catalogue = false;
        if self.repository.is_none() && !self.repo_error.is_empty() {
            self.sources.error =
                "Repository unavailable. Esc, then /files and /refresh; /diff shows details."
                    .into();
            return;
        }
        // Fetch once for this chooser; typed filtering stays local so it cannot
        // repeatedly hide ready rows behind source validation.
        self.sources.query.clear();
        self.sources.catalogue(
            self.task_id,
            self.repository.as_ref(),
            self.repo_loading || self.repo_refresh,
        );
    }

    pub(super) fn composer_key(&mut self, key: KeyEvent) -> Option<bool> {
        if !matches!(
            self.editor_field,
            EditorField::Request | EditorField::Goal | EditorField::NextAction
        ) {
            return None;
        }
        if self.composer_menu.is_none() {
            let focused = self.sources.chip_focus;
            match key.code {
                KeyCode::Tab | KeyCode::BackTab if !self.sources.basket.is_empty() => {
                    self.sources.focus_chip(key.code == KeyCode::BackTab);
                    return Some(true);
                }
                KeyCode::Left | KeyCode::Right if focused.is_some() => {
                    self.sources.focus_chip(key.code == KeyCode::Left);
                    return Some(true);
                }
                KeyCode::Esc if focused.is_some() => {
                    self.sources.chip_focus = None;
                    return Some(true);
                }
                KeyCode::Enter | KeyCode::Delete if focused.is_some() => {
                    let id = focused.expect("focused chip");
                    if !self
                        .sources
                        .inspect_chip(id, key.code == KeyCode::Delete, self.task_id)
                    {
                        return Some(false);
                    }
                    self.retain_editor();
                    self.composer_return = Some(self.editor_field);
                    self.save_selection();
                    if let Some(previous) = self.overlay_frame() {
                        self.overlay_history.push(previous);
                    }
                    self.overlay = Some(Overlay::Sources);
                    self.sources
                        .observe_captures(self.task_id, self.repository.as_ref());
                    return Some(true);
                }
                _ if focused.is_some() => {
                    // Ordinary editing resumes without consuming the character.
                    self.sources.chip_focus = None;
                }
                _ => {}
            }
        }
        if let Some(menu) = self.composer_menu {
            if self.cursor <= menu.start || self.input.get(menu.start + 1..self.cursor).is_none() {
                self.composer_menu = None;
                return None;
            }
            match key.code {
                KeyCode::Esc => {
                    self.composer_menu = None;
                    self.composer_catalogue = false;
                    return Some(true);
                }
                KeyCode::Up | KeyCode::BackTab | KeyCode::Down | KeyCode::Tab => {
                    let count = self.composer_candidates().count();
                    let selected = if matches!(key.code, KeyCode::Up | KeyCode::BackTab) {
                        menu.selected.saturating_sub(1)
                    } else {
                        menu.selected.saturating_add(1).min(count.saturating_sub(1))
                    };
                    self.composer_menu.as_mut().expect("open menu").selected = selected;
                    return Some(true);
                }
                KeyCode::Enter | KeyCode::Delete => {
                    if self.width < 60 || self.height < 18 {
                        return Some(false);
                    }
                    let selected = self.composer_candidates().nth(menu.selected);
                    if let Some(index) = selected {
                        if menu.kind == Kind::Files
                            && (self.sources.busy()
                                || self.composer_catalogue
                                || !self.sources.error.is_empty())
                        {
                            return Some(false);
                        }
                        if key.code == KeyCode::Delete
                            && self.sources.file_attachment(index).is_none()
                        {
                            return Some(false);
                        }
                        self.choose_composer_item(index, key.code == KeyCode::Delete);
                    }
                    return Some(true);
                }
                KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => {
                    self.composer_menu = None;
                    self.composer_catalogue = false;
                }
                KeyCode::Backspace if self.cursor == menu.start + 1 => {
                    self.composer_menu = None;
                    self.composer_catalogue = false;
                }
                KeyCode::Char(_) | KeyCode::Backspace => {
                    self.composer_menu.as_mut().expect("open menu").selected = 0;
                }
                _ => {}
            }
            return None;
        }
        let kind = match key.code {
            KeyCode::Char('@')
                if self.cursor == 0 || self.input[..self.cursor].ends_with(char::is_whitespace) =>
            {
                Kind::Files
            }
            _ => return None,
        };
        Some(self.start_composer_menu(kind))
    }

    pub(super) fn start_composer_menu(&mut self, kind: Kind) -> bool {
        let start = self.cursor;
        if !self.insert_input("@") {
            return false;
        }
        self.composer_menu = Some(Menu {
            kind,
            start,
            selected: 0,
        });
        self.composer_catalogue = kind == Kind::Files;
        true
    }

    fn choose_composer_item(&mut self, index: usize, remove: bool) {
        self.composer_menu.take().expect("open menu");
        self.composer_catalogue = false;
        let field = self.editor_field;
        // Picking context must not rewrite the user's draft or move its caret.
        self.retain_editor();
        self.composer_return = Some(field);
        // Reuse the validated catalogue and capture/persistence boundary.
        self.save_selection();
        if let Some(previous) = self.overlay_frame() {
            self.overlay_history.push(previous);
        }
        self.overlay = Some(Overlay::Sources);
        self.sources.mode = super::sources::Mode::Files;
        self.sources.file_selection = index;
        if remove {
            if let Some(attached) = self.sources.file_attachment(index) {
                let multiple =
                    self.sources
                        .basket
                        .iter()
                        .enumerate()
                        .any(|(candidate, capture)| {
                            candidate != attached
                                && std::path::Path::new(&capture.path)
                                    == self.sources.files[index].path
                        });
                if multiple {
                    self.sources.mode = super::sources::Mode::Basket;
                    self.sources.basket_selection = attached;
                    self.sources.show_basket();
                } else {
                    let id = self.sources.basket[attached].id;
                    self.sources.inspect_chip(id, true, self.task_id);
                }
            }
        } else {
            self.sources
                .inspect_file(self.task_id, self.repository.as_ref());
        }
        self.notice = if remove {
            "Review the exact capture ID/range before removing it. Your draft is retained."
        } else {
            "Review local capture, then Enter attaches and returns to your draft. r chooses lines; Esc cancels. Nothing sent."
        }.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::Event;
    use crossterm::event::KeyModifiers;

    fn key(app: &mut App, code: KeyCode) {
        app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    #[test]
    fn file_search_matches_case_abbreviations_spaces_and_directory_scope() {
        for (path, query, expected) in [
            ("src/app/composer.rs", "SRC/APP/CMP", true),
            ("src/other/composer.rs", "src/app/cmp", false),
            ("src/app", "src/app/", false),
            ("src/app/parser.rs", "./src/app/prs", true),
            ("src\\app\\changes.rs", "src/app/chg", true),
            ("docs/Design Notes.md", "design notes", true),
            ("docs/Architecture Draft.md", "architecture missing", false),
            ("docs/ÉTUDE.md", "étude", true),
            ("docs/Notes.md", "notse", false),
        ] {
            assert_eq!(file_query_matches(path, query), expected, "{path}: {query}");
        }
    }

    #[test]
    fn slash_action_preserves_draft_and_escape_restores_editor() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.update(Event::Paste("draft\n".into()));
        key(&mut app, KeyCode::Char('/'));
        for ch in "help".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        assert_eq!(app.command_popup.as_ref().unwrap().candidates().count(), 1);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.overlay, Some(Overlay::Help));
        assert_eq!(app.drafts[0].text, "draft\n");
        key(&mut app, KeyCode::Esc);
        assert!(app.editing);
        assert_eq!(app.input, "draft\n");
        assert!(app.planning.pending.is_none());
    }

    #[test]
    fn paste_does_not_execute_commands_and_menu_escape_keeps_literal_text() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.update(Event::Paste("/run echo unsafe\n@private".into()));
        assert!(app.composer_menu.is_none());
        assert!(app.overlay.is_none());
        key(&mut app, KeyCode::Char(' '));
        key(&mut app, KeyCode::Char('@'));
        assert!(app.composer_menu.is_some());
        assert!(app.save_requested);
        key(&mut app, KeyCode::Esc);
        assert!(app.editing);
        assert!(app.input.ends_with(" @"));
        assert!(app.composer_menu.is_none());
    }

    #[test]
    fn run_command_opens_local_execution_without_running() {
        let mut app = App::new("/workspace".into(), 120, 36);
        key(&mut app, KeyCode::Char('/'));
        for ch in "run".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.overlay, Some(Overlay::Execution));
        assert!(!app.editing);
        assert!(app.command.is_empty());
        assert!(app.planning.pending.is_none());
    }

    #[test]
    fn first_attachment_autosaves_draft_and_waits_for_its_task_owner() {
        let mut app = App::new("/workspace".into(), 120, 36);
        key(&mut app, KeyCode::Char('@'));
        assert_eq!(app.task_id, 0);
        assert!(app.save_requested);
        app.prepare_composer_sources();
        assert!(app.sources.pending.is_none());
        assert!(app.composer_catalogue);
        assert_eq!(app.input, "@");
        assert!(app.goal.is_empty());
        app.task_id = 7;
        app.repo_error = "not a repository".into();
        app.prepare_composer_sources();
        assert!(!app.composer_catalogue);
        assert!(!app.sources.error.is_empty());
        assert!(!app.sources.await_refresh);
        assert_eq!(app.input, "@");
    }

    #[test]
    fn attachment_filter_and_return_preserve_the_original_draft() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.task_id = 7;
        app.update(Event::Paste("Explain ".into()));
        key(&mut app, KeyCode::Char('@'));
        app.sources.files = vec![
            crate::workspace::context::Source {
                path: "src/main.rs".into(),
                bytes: 12,
                sha256: [0; 32],
            },
            crate::workspace::context::Source {
                path: "README.md".into(),
                bytes: 20,
                sha256: [0; 32],
            },
        ];
        app.composer_catalogue = false;
        for ch in "README".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        assert_eq!(app.composer_candidates().collect::<Vec<_>>(), vec![1]);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.overlay, Some(Overlay::Sources));
        assert_eq!(app.sources.file_selection, 1);
        assert_eq!(app.drafts[0].text, "Explain @README");
        assert!(app.sources.basket.is_empty());
        key(&mut app, KeyCode::Esc);
        assert!(app.editing);
        assert_eq!(app.input, "Explain @README");
        assert!(app.planning.pending.is_none());
    }

    #[test]
    fn attachment_path_search_owns_slashes_and_spaces_without_losing_the_suffix() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.task_id = 7;
        app.update(Event::Paste("Explain suffix".into()));
        app.cursor = "Explain ".len();
        key(&mut app, KeyCode::Char('@'));
        app.sources.files = vec![
            crate::workspace::context::Source {
                path: "src/other/Composer Notes.rs".into(),
                bytes: 12,
                sha256: [0; 32],
            },
            crate::workspace::context::Source {
                path: "src/app/Composer Notes.rs".into(),
                bytes: 20,
                sha256: [0; 32],
            },
        ];
        app.composer_catalogue = false;
        for ch in "SRC/APP/CMP NTS".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        assert!(app.command_popup.is_none());
        assert_eq!(app.composer_query(), "SRC/APP/CMP NTS");
        assert_eq!(app.composer_candidates().collect::<Vec<_>>(), vec![1]);
        let draft = app.input.clone();
        let caret = app.cursor;
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.overlay, Some(Overlay::Sources));
        assert_eq!(app.sources.file_selection, 1);
        key(&mut app, KeyCode::Esc);
        assert_eq!((app.input, app.cursor), (draft, caret));
        assert!(app.planning.pending.is_none());
    }

    #[test]
    fn acknowledged_attachment_returns_to_exact_draft_and_caret_without_sending() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.task_id = 7;
        app.update(Event::Paste("Explain @sample suffix".into()));
        app.cursor = "Explain @sample".len();
        let draft = app.input.clone();
        let cursor = app.cursor;
        app.sources.files.push(crate::workspace::context::Source {
            path: "sample.rs".into(),
            bytes: 6,
            sha256: [7; 32],
        });
        app.composer_menu = Some(Menu {
            kind: Kind::Files,
            start: 8,
            selected: 0,
        });
        key(&mut app, KeyCode::Enter);
        let mut capture = crate::workspace::context::Excerpt {
            id: 0,
            task_id: 7,
            root: "/workspace".into(),
            path: "sample.rs".into(),
            start_line: 1,
            end_line: 1,
            source_sha256: [7; 32],
            text: "first\n".into(),
            available: true,
            origin_run: None,
        };
        app.sources.event(
            7,
            crate::workspace::context::Event {
                task_id: 7,
                result: Ok(crate::workspace::context::Response::Inspected(
                    capture.clone(),
                )),
            },
        );
        app.sources.preview_reviewed = true;
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.overlay, Some(Overlay::Sources));
        assert!(app.sources.basket.is_empty());
        app.store_event(crate::store::Event::Context {
            task_id: 7,
            result: Err("disk full".into()),
        });
        assert_eq!(app.overlay, Some(Overlay::Sources));
        app.sources.command("journal-retry", 7, None);
        capture.id = 91;
        app.store_event(crate::store::Event::Context {
            task_id: 7,
            result: Ok(vec![capture]),
        });
        assert!(app.overlay.is_none());
        assert!(app.editing);
        assert_eq!(app.input, draft);
        assert_eq!(app.cursor, cursor);
        assert_eq!(app.sources.basket.len(), 1);
        assert!(app.planning.pending.is_none());
        assert!(app.pending_request.is_none());
    }

    #[test]
    fn saved_task_navigation_opens_commands_without_changing_the_goal() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.goal = "Existing task".into();
        app.editing = false;
        let focus = app.focus;
        key(&mut app, KeyCode::Char('/'));
        for ch in "help".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.overlay, Some(Overlay::Help));
        assert_eq!(app.goal, "Existing task");
        key(&mut app, KeyCode::Esc);
        assert!(!app.editing);
        assert_eq!(app.focus, focus);
        assert!(app.input.is_empty());
        assert_eq!(app.goal, "Existing task");
    }

    #[test]
    fn focused_chip_keyboard_inspects_exact_id_and_preserves_draft_caret() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.task_id = 7;
        app.update(Event::Paste("before suffix".into()));
        app.cursor = 6;
        let draft = app.input.clone();
        let caret = app.cursor;
        app.sources.basket = [41, 42]
            .into_iter()
            .map(|id| crate::workspace::context::Excerpt {
                id,
                task_id: 7,
                root: "/workspace".into(),
                path: "same.rs".into(),
                start_line: id as usize,
                end_line: id as usize,
                source_sha256: [7; 32],
                text: format!("capture {id}\n"),
                available: true,
                origin_run: None,
            })
            .collect();
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Right);
        assert_eq!(app.sources.chip_focus, Some(42));
        assert_eq!((&app.input, app.cursor), (&draft, caret));
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.overlay, Some(Overlay::Sources));
        assert_eq!(app.sources.basket_selection, 1);
        assert!(app.sources.detail.contains("#42"));
        assert!(app.planning.pending.is_none());
        key(&mut app, KeyCode::Esc);
        assert_eq!((app.input, app.cursor), (draft, caret));
        assert!(app.editing);
    }

    #[test]
    fn typed_matchsets_are_identical_across_cached_form_rankings() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.sources.files = [
            "src/failure.rs",
            "src/changed.rs",
            "tests/review.rs",
            "docs/readme",
        ]
        .into_iter()
        .map(|path| crate::workspace::context::Source {
            path: path.into(),
            bytes: 1,
            sha256: [0; 32],
        })
        .collect();
        app.sources.set_rank_facts(
            [std::path::PathBuf::from("src/changed.rs")]
                .into_iter()
                .collect(),
            [
                std::path::PathBuf::from("src/failure.rs"),
                std::path::PathBuf::from(".env"),
            ]
            .into_iter()
            .collect(),
        );
        for query in ["", "src/", "srs", "missing", "tests/rv"] {
            app.input = format!("@{query}");
            app.cursor = app.input.len();
            app.composer_menu = Some(Menu {
                kind: Kind::Files,
                start: 0,
                selected: 0,
            });
            let mut expected = None;
            for form in 0..9 {
                app.lead.form = form;
                let mut set: Vec<_> = app.composer_candidates().collect();
                set.sort_unstable();
                if let Some(expected) = &expected {
                    assert_eq!(&set, expected, "query={query}, form={form}");
                } else {
                    expected = Some(set);
                }
            }
        }
        app.input = "@".into();
        app.cursor = 1;
        app.lead.form = 5;
        assert_eq!(app.composer_candidates().next(), Some(0));
        app.lead.form = 3;
        assert_eq!(app.composer_candidates().next(), Some(1));
    }
}
