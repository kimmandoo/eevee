use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{App, Overlay, StorageState};

#[derive(Clone)]
pub struct Choice {
    pub command: &'static str,
    pub label: String,
    pub reason: Option<String>,
}

impl Choice {
    pub fn new(command: &'static str, label: impl Into<String>) -> Self {
        Self {
            command,
            label: label.into(),
            reason: None,
        }
    }

    pub fn disabled(
        command: &'static str,
        label: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            command,
            label: label.into(),
            reason: Some(reason.into()),
        }
    }
}

pub struct Popup {
    pub query: String,
    pub selected: Option<usize>,
    pub choices: Vec<Choice>,
    pub scope: Option<Overlay>,
}

impl Popup {
    pub fn candidates(&self) -> impl Iterator<Item = &Choice> {
        let query = self
            .query
            .trim()
            .strip_prefix('/')
            .unwrap_or(self.query.trim());
        let exact = self
            .choices
            .iter()
            .any(|choice| choice.command.eq_ignore_ascii_case(query));
        self.choices.iter().filter(move |choice| {
            if exact {
                return choice.command.eq_ignore_ascii_case(query);
            }
            // Terms may match the command and label independently, in any order.
            // Keep catalogue order stable and avoid fuzzy scores or search copies.
            query.split_whitespace().all(|term| {
                [choice.command, choice.label.as_str()].iter().any(|text| {
                    text.as_bytes()
                        .windows(term.len())
                        .any(|part| part.eq_ignore_ascii_case(term.as_bytes()))
                })
            })
        })
    }
}

/// Slash commands that switch the lead partner's form without opening the
/// picker: the form name plus one element alias per form (`/water`…).
const FORM_COMMANDS: [(&str, &str); 9] = [
    ("eevee", "normal"),
    ("vaporeon", "water"),
    ("jolteon", "electric"),
    ("flareon", "fire"),
    ("espeon", "psychic"),
    ("umbreon", "dark"),
    ("leafeon", "grass"),
    ("glaceon", "ice"),
    ("sylveon", "fairy"),
];

fn form_command_index(name: &str) -> Option<usize> {
    FORM_COMMANDS
        .iter()
        .position(|(form, alias)| name == *form || name == *alias)
}

impl App {
    pub fn command_choices(&self) -> Vec<Choice> {
        let mut choices = match self.overlay {
            Some(Overlay::Planning) => self.planning.command_choices(),
            Some(Overlay::Execution) => self.execution.command_choices(),
            Some(Overlay::Sources) => self.sources.command_choices(),
            Some(Overlay::Agents) => self.agents.command_choices(),
            Some(Overlay::Handoff) => self.handoff.command_choices(),
            Some(Overlay::Workflows) => self.workflow_command_choices(),
            Some(Overlay::Memory) => self.memory_command_choices(),
            Some(Overlay::Tasks) => vec![
                Choice::new("archive", "Archive selected task"),
                Choice::new("restore", "Restore selected task"),
                Choice::new("archived", "Show/hide archived tasks"),
                Choice::new("delete", "Review selected task deletion"),
                Choice::new("prune", "Review expired archived task deletion"),
            ],
            Some(Overlay::Criteria) => vec![
                Choice::new("criteria-edit", "Edit acceptance criteria"),
                Choice::new("criterion-toggle", "Check/uncheck selected criterion"),
            ],
            Some(Overlay::Party) => self.party_command_choices(),
            None if self.planning.busy() => self.planning.command_choices(),
            _ => Vec::new(),
        };
        for (name, label) in [
            (
                "provider",
                "Providers: connect, model, effort / active request / cancellation",
            ),
            ("connect", "Provider connections; nothing sent"),
            ("new", "New provider session"),
            ("resume", "Resume owned provider session"),
            ("task-new", "Create local task"),
            ("model", "Choose provider model"),
            ("effort", "Choose reasoning effort"),
            ("role", "Choose Plan / Implement / Review role"),
            ("forms", "Choose evolution"),
            ("back-form", "Return to the previous form (Backspace)"),
            ("files", "Attach files / inspect source context"),
            ("tasks", "Saved tasks"),
            ("party", "Party roster"),
            (
                "lead",
                "Front a partner: /lead <name> or pick in the roster",
            ),
            ("run", "Local commands / execution evidence / cancellation"),
            ("history", "Provider history / journal evidence"),
            ("handoff", "Portable task handoff"),
            ("goal", "Edit goal"),
            ("criteria", "Acceptance criteria"),
            ("criteria-edit", "Edit acceptance criteria"),
            ("next", "Edit next action"),
            ("complete", "Review task completion"),
            ("diff", "Repository diff / change evidence"),
            ("inspect", "Task and storage details"),
            ("companion", "Show companion"),
            ("preview", "Local role preview"),
            ("layout", "Toggle linear layout"),
            ("images", "Evolution artwork setup"),
            ("workflows", "Specialist workflows"),
            ("memory", "Approved memories"),
            ("refresh", "Refresh repository inspection"),
            ("storage-retry", "Retry failed storage"),
            ("help", "Navigation help"),
            ("verbose", "Toggle brief / full explanations"),
            ("quit", "Leave Eevee"),
        ] {
            if !choices.iter().any(|choice| choice.command == name) {
                choices.push(Choice::new(name, label));
            }
        }
        // Named form switches keep the picker's persisted SelectForm path.
        // Element aliases (`/water`…) are their own commands so typed input
        // resolves exactly like a form name.
        for (index, (name, alias)) in FORM_COMMANDS.iter().enumerate() {
            if !choices.iter().any(|choice| choice.command == *name) {
                choices.push(Choice::new(
                    name,
                    format!("Switch to {} form", crate::domain::FORMS[index].name),
                ));
            }
            if !choices.iter().any(|choice| choice.command == *alias) {
                choices.push(Choice::new(
                    alias,
                    format!("{} alias for /{name}", crate::domain::FORMS[index].element),
                ));
            }
        }
        for choice in self
            .planning
            .command_choices()
            .into_iter()
            .filter(|choice| {
                choice.command.starts_with("provider-")
                    || choice.command.starts_with("role-")
                    || matches!(choice.command, "sessions" | "paths" | "models-refresh")
            })
        {
            if !choices
                .iter()
                .any(|existing| existing.command == choice.command)
            {
                choices.push(choice);
            }
        }
        for choice in self
            .workflow_command_choices()
            .into_iter()
            .filter(|choice| choice.command.starts_with("workflow-"))
        {
            if !choices
                .iter()
                .any(|existing| existing.command == choice.command)
            {
                choices.push(choice);
            }
        }
        if self.overlay == Some(Overlay::Forms) {
            for (index, name) in [
                "role-eevee",
                "role-vaporeon",
                "role-jolteon",
                "role-flareon",
                "role-espeon",
                "role-umbreon",
                "role-leafeon",
                "role-glaceon",
                "role-sylveon",
            ]
            .iter()
            .enumerate()
            {
                choices.push(Choice::new(
                    name,
                    format!("Choose {}", crate::domain::FORMS[index].name),
                ));
            }
        }
        choices
    }

    pub(crate) fn command_disabled_reason(&self, name: &str) -> Option<String> {
        if self.transitioning || self.quit_after_save || self.confirmation.is_some() {
            return Some("Finish the current confirmation or storage operation first.".into());
        }
        if (self.sources.confirmation.is_some()
            || self.execution.confirmation.is_some()
            || self.agents.preview
            || self.agents.auth_preview.is_some())
            && !matches!(name, "close" | "cancel" | "deny" | "quit" | "verbose")
        {
            return Some(
                "Resolve the visible neutral review before opening another action.".into(),
            );
        }
        if (self.width < 60 || self.height < 18)
            && !matches!(
                name,
                "quit" | "cancel" | "close" | "storage-retry" | "verbose"
            )
        {
            return Some("Enlarge the terminal to review this action.".into());
        }
        if name == "storage-retry" && !matches!(self.storage, StorageState::Failed(_)) {
            return Some("No failed task save to retry.".into());
        }
        if name == "archive" && self.show_archived {
            return Some("This task is already archived; use /restore.".into());
        }
        if name == "restore" && !self.show_archived {
            return Some("Choose the archived task view before restoring a task.".into());
        }
        if name == "complete"
            && (self.complete
                || self.criteria.is_empty()
                || self.criteria.iter().any(|criterion| !criterion.met))
        {
            return Some("Completion requires an open task with every nonempty acceptance criterion checked.".into());
        }
        if matches!(
            name,
            "workflow-diagnosis" | "workflow-refactor" | "workflow-review" | "workflow-recipe"
        ) && !self
            .workflow_command_choices()
            .iter()
            .any(|choice| choice.command == name)
        {
            return Some(
                "Finish or close the current specialist editor, evidence chooser, or review first."
                    .into(),
            );
        }
        let local_preview_choice = self.planning.preview.is_some()
            && (matches!(name, "model" | "effort" | "role")
                || name.starts_with("role-")
                || name.starts_with("provider-"));
        let local_picker_refresh = name == "models-refresh"
            && self.planning.picker.is_some()
            && !self.planning.busy_except_picker();
        if self.planning.busy()
            && !local_preview_choice
            && !local_picker_refresh
            && !matches!(
                name,
                "cancel" | "quit" | "provider" | "answer" | "answers-review" | "verbose"
            )
        {
            return Some("Finish or cancel the active provider request first.".into());
        }
        if matches!(name, "new" | "resume") && self.overlay.is_none() {
            return None;
        }
        if matches!(name, "new" | "resume" | "task-new" | "archive" | "restore")
            && (!matches!(self.storage, StorageState::Saved)
                || self.save_requested
                || self.sources.persistence_pending()
                || self.sources.store_failed)
        {
            return Some("Wait for saved task/context acknowledgement, or retry storage.".into());
        }
        if self.execution.busy()
            && matches!(
                name,
                "task-new" | "archive" | "restore" | "delete" | "prune"
            )
        {
            return Some(
                "Finish or cancel the active local command before changing task ownership.".into(),
            );
        }
        if (matches!(
            name,
            "model" | "effort" | "provider" | "history" | "models-refresh"
        ) || name.starts_with("provider-")
            || matches!(name, "role-plan" | "role-implement" | "role-review"))
            && self.execution.busy()
        {
            return Some("Finish or cancel the active local command first.".into());
        }
        if name == "resume"
            && (self.planning.operation == crate::providers::Operation::Review
                || self.planning.thread_id.is_none())
        {
            return Some("This provider/role has no owned resumable session; choose /new.".into());
        }
        if matches!(name, "new" | "resume" | "sessions" | "models-refresh") && self.task_id <= 0 {
            return Some("Create /task-new or save a local task before inspecting or submitting provider work.".into());
        }
        if matches!(name, "new" | "resume") && self.goal.trim().is_empty() {
            return Some("Save a nonempty goal before preparing a provider prompt.".into());
        }
        if self.planning.needs_saved_command(name)
            && !matches!(name, "role-plan" | "role-implement" | "role-review")
            && !self.planning_input_ready()
        {
            return Some("Save task/context first and wait for storage acknowledgement; no request was prepared.".into());
        }
        None
    }

    pub(super) fn open_commands(&mut self) {
        self.command_popup = Some(Popup {
            query: String::new(),
            selected: None,
            choices: self.command_choices(),
            scope: self.overlay,
        });
    }
    pub(super) fn command_popup_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return false;
        }
        if key.code == KeyCode::Char('k') && key.modifiers == KeyModifiers::CONTROL {
            self.open_commands();
            return true;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        let popup = self.command_popup.as_mut().expect("open command popup");
        match key.code {
            KeyCode::Esc => {
                self.command_popup = None;
            }
            KeyCode::Up | KeyCode::BackTab => {
                popup.selected = Some(
                    popup
                        .selected
                        .unwrap_or(popup.candidates().count())
                        .saturating_sub(1),
                );
            }
            KeyCode::Down | KeyCode::Tab => {
                let count = popup.candidates().count();
                popup.selected = (count > 0)
                    .then(|| popup.selected.map_or(0, |index| (index + 1).min(count - 1)));
            }
            KeyCode::Home => popup.selected = (popup.candidates().count() > 0).then_some(0),
            KeyCode::End => popup.selected = popup.candidates().count().checked_sub(1),
            KeyCode::Backspace => {
                popup.query.pop();
                popup.selected =
                    (!popup.query.is_empty() && popup.candidates().count() > 0).then_some(0);
            }
            KeyCode::Char(character) if super::safe_character(character) => {
                popup.query.push(character);
                popup.selected = (popup.candidates().count() > 0).then_some(0);
            }
            KeyCode::Enter => {
                if popup.scope != self.overlay {
                    popup.selected = None;
                    self.notice = "The workspace changed while commands were open. Reopen / commands before choosing an action.".into();
                    return true;
                }
                // A literal argument owns dispatch even if its words also match
                // a catalogue label (for example, a partner named "partner").
                let query = popup
                    .query
                    .trim()
                    .strip_prefix('/')
                    .unwrap_or(popup.query.trim());
                if query
                    .split_once(char::is_whitespace)
                    .is_some_and(|(name, argument)| name == "lead" && !argument.trim().is_empty())
                {
                    let query = query.to_owned();
                    return self.command_with_argument(&query);
                }
                let choice = popup
                    .selected
                    .and_then(|index| popup.candidates().nth(index))
                    .cloned();
                let Some(choice) = choice else {
                    let query = popup.query.clone();
                    return self.command_with_argument(&query);
                };
                if let Some(reason) = choice
                    .reason
                    .or_else(|| self.command_disabled_reason(choice.command))
                {
                    self.notice = reason;
                    return true;
                }
                let name = choice.command;
                self.command_popup = None;
                self.command(name);
            }
            _ => return false,
        }
        true
    }

    /// Commands that take a literal typed argument rather than a catalogue
    /// selection. Only `lead` accepts one today: `/lead <partner>`.
    pub(super) fn command_with_argument(&mut self, query: &str) -> bool {
        let query = query.trim().strip_prefix('/').unwrap_or(query.trim());
        let (name, argument) = match query.split_once(char::is_whitespace) {
            Some((name, argument)) => (name, argument.trim()),
            None => (query, ""),
        };
        if name == "lead" {
            self.command_popup = None;
            return self.lead_command(argument);
        }
        false
    }

    pub fn command(&mut self, name: &str) -> bool {
        if let Some(reason) = self.command_disabled_reason(name) {
            self.notice = reason;
            return true;
        }
        self.command_popup = None;
        if matches!(name, "new" | "resume") && self.overlay.is_none() {
            if !self.composer_is_request() {
                if self.editing {
                    self.retain_editor();
                }
                self.open_editor(crate::store::EditorField::Request);
            }
            let changed = self.submit_request();
            if name == "new"
                && let Some((_, resume)) = &mut self.pending_request
            {
                *resume = false;
            }
            return changed;
        }
        if self.editing && name != "quit" {
            self.pending_request = None;
            // Opening an action retains the actual draft without applying it.
            let field = self.editor_field;
            self.retain_editor();
            self.composer_return = Some(field);
        }
        let contextual = match self.overlay {
            Some(Overlay::Planning) => self.planning.command_choices(),
            Some(Overlay::Execution) => self.execution.command_choices(),
            Some(Overlay::Sources) => self.sources.command_choices(),
            Some(Overlay::Agents) => self.agents.command_choices(),
            Some(Overlay::Handoff) => self.handoff.command_choices(),
            Some(Overlay::Workflows) => self.workflow_command_choices(),
            Some(Overlay::Memory) => self.memory_command_choices(),
            Some(Overlay::Party) => self.party_command_choices(),
            _ => Vec::new(),
        };
        if let Some(choice) = contextual.iter().find(|choice| choice.command == name) {
            if let Some(reason) = &choice.reason {
                self.notice.clone_from(reason);
                return true;
            }
            return match self.overlay {
                Some(Overlay::Planning) => self.planning_command(name),
                Some(Overlay::Execution) => {
                    if let Some(action) = self.execution.command(name) {
                        self.apply_execution_action(action);
                    }
                    if !self.execution.visible() && self.overlay == Some(Overlay::Execution) {
                        self.close_overlay();
                    }
                    true
                }
                Some(Overlay::Sources) => {
                    let action = self
                        .sources
                        .command(name, self.task_id, self.repository.as_ref());
                    self.apply_source_action(action)
                }
                Some(Overlay::Agents) => {
                    self.agents.active = self.planning.agent.index();
                    let changed = self
                        .agents
                        .command(name, self.width >= 60 && self.height >= 18);
                    self.use_selected_provider();
                    changed
                }
                Some(Overlay::Handoff) => self.handoff_command(name),
                Some(Overlay::Workflows) => self.workflow_command(name),
                Some(Overlay::Memory) => self.memory_command(name),
                Some(Overlay::Party) => self.party_command(name),
                _ => false,
            };
        }
        if let Some(index) = form_command_index(name) {
            self.request_navigation(super::execution::Action::SelectForm(index));
            return true;
        }
        if let Some(index) = [
            "role-eevee",
            "role-vaporeon",
            "role-jolteon",
            "role-flareon",
            "role-espeon",
            "role-umbreon",
            "role-leafeon",
            "role-glaceon",
            "role-sylveon",
        ]
        .iter()
        .position(|command| *command == name)
        {
            self.request_navigation(super::execution::Action::SelectForm(index));
            return true;
        }
        if name.starts_with("provider-")
            || matches!(name, "role-plan" | "role-implement" | "role-review")
        {
            self.open_overlay(Overlay::Planning);
            if self.overlay == Some(Overlay::Planning) {
                return self.planning_command(name);
            }
            return false;
        }
        match name {
            "cancel" | "deny" | "answer" | "answers-review" if self.planning.busy() => {
                self.open_overlay(Overlay::Planning);
                return self.planning_command(name);
            }
            // Idle /provider opens the shared hub; an active request keeps its drawer.
            "provider" if self.planning.busy() => self.open_overlay(Overlay::Planning),
            "provider" => {
                self.open_overlay(Overlay::Agents);
                self.agents.active = self.planning.agent.index();
                self.agents.focus_active();
            }
            "connect" => {
                self.open_overlay(Overlay::Agents);
                // Same active-provider sync as /provider so the hub's
                // Model/Effort vs Use-for-task choice reflects the task's provider.
                self.agents.active = self.planning.agent.index();
            }
            "task-new" => self.new_task(),
            "tasks" => self.open_overlay(Overlay::Tasks),
            "party" => self.open_overlay(Overlay::Party),
            "lead" => {
                self.lead_command("");
            }
            "new" | "resume" | "model" | "effort" | "history" | "sessions" | "paths"
            | "models-refresh" => {
                self.open_overlay(Overlay::Planning);
                if self.overlay == Some(Overlay::Planning) {
                    return self.planning_command(if name == "history" {
                        "provider-journal"
                    } else {
                        name
                    });
                }
            }
            "role" => {
                self.open_overlay(Overlay::Planning);
                if self.overlay == Some(Overlay::Planning) {
                    let choices = self
                        .planning
                        .command_choices()
                        .into_iter()
                        .filter(|choice| {
                            matches!(
                                choice.command,
                                "role-plan" | "role-implement" | "role-review"
                            )
                        })
                        .collect();
                    self.command_popup = Some(Popup {
                        query: String::new(),
                        selected: None,
                        choices,
                        scope: self.overlay,
                    });
                }
            }
            "forms" => self.open_overlay(Overlay::Forms),
            "back-form" => self.previous_form_toggle(),
            "files" => self.open_overlay(Overlay::Sources),
            "run" => self.open_overlay(Overlay::Execution),
            "handoff" => self.open_overlay(Overlay::Handoff),
            "goal" => self.begin_edit(),
            "criteria" => self.open_overlay(Overlay::Criteria),
            "criteria-edit" => self.begin_edit_field(crate::store::EditorField::Criteria),
            "criterion-toggle" => self.toggle_criterion(),
            "next" => self.begin_edit_field(crate::store::EditorField::NextAction),
            "complete" => self.request_complete(),
            "diff" => self.open_overlay(Overlay::Repository),
            "inspect" => self.open_overlay(Overlay::Inspector),
            "companion" => self.open_overlay(Overlay::Companion),
            "preview" => self.open_overlay(Overlay::Preview),
            "layout" => self.linear = !self.linear,
            "images" => self.open_overlay(Overlay::Images),
            "workflows" => self.open_overlay(Overlay::Workflows),
            "workflow-diagnosis" | "workflow-refactor" | "workflow-review" | "workflow-recipe" => {
                self.open_overlay(Overlay::Workflows);
                if self.overlay == Some(Overlay::Workflows) {
                    return self.workflow_command(name);
                }
            }
            "memory" => self.open_overlay(Overlay::Memory),
            "refresh" => {
                self.repo_refresh = true;
                self.repo_loading = true;
            }
            "storage-retry" => {
                self.retry_storage = true;
                self.storage = StorageState::Loading;
            }
            "help" => self.open_overlay(Overlay::Help),
            "verbose" => {
                self.verbose = !self.verbose;
                self.notice = if self.verbose {
                    "Full explanations on. /verbose again for brief.".into()
                } else {
                    "Brief explanations on. /verbose shows full text.".into()
                };
            }
            "quit" => self.request_quit(),
            "archive" => self.archive_selected(true),
            "restore" => self.archive_selected(false),
            "archived" => {
                self.show_archived = !self.show_archived;
                self.task_selection = 0;
            }
            "delete" => self.preview_delete(false),
            "prune" => self.preview_delete(true),
            _ => {
                self.notice =
                    format!("/{name} is unavailable on this surface; open its workspace first.");
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::Event;

    fn key(app: &mut App, code: KeyCode) -> bool {
        app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    fn query(app: &mut App, name: &str) {
        for character in name.chars() {
            key(app, KeyCode::Char(character));
        }
    }

    #[test]
    fn command_search_is_isolated_from_draft_caret_and_suffix() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.update(Event::Paste("before suffix".into()));
        app.cursor = 6;
        let revision = app.revision;
        key(&mut app, KeyCode::Char('/'));
        query(&mut app, "help");
        assert_eq!(app.input, "before suffix");
        assert_eq!(app.cursor, 6);
        assert_eq!(app.revision, revision);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.input, "before suffix");
        assert_eq!(app.cursor, 6);
        assert!(app.editing);
        assert!(app.planning.pending.is_none());
    }

    #[test]
    fn literal_slash_paste_and_unselected_enter_never_dispatch() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.update(Event::Paste("/new\n/run unsafe".into()));
        assert!(app.command_popup.is_none());
        assert_eq!(app.input, "/new\n/run unsafe");
        key(&mut app, KeyCode::Char('/'));
        assert!(!key(&mut app, KeyCode::Enter));
        assert!(app.execution.active.is_none());
        assert!(app.planning.pending.is_none());
        assert!(app.editing);
        app.update(Event::Paste("\n/jolteon stays literal".into()));
        assert_eq!(app.form(), 0);
        app.update(Event::Paste("\n/quit pasted literally".into()));
        assert!(app.command_popup.is_none());
        assert_eq!(
            app.input,
            "/new\n/run unsafe\n/jolteon stays literal\n/quit pasted literally"
        );
        assert!(!app.quit);
        assert_eq!(app.form(), 0);
    }

    #[test]
    fn typed_named_form_command_switches_and_retains_draft_and_caret() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.update(Event::Paste("draft stays 漢字".into()));
        app.cursor = 6;
        key(&mut app, KeyCode::Char('/'));
        query(&mut app, "jolteon");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.form(), 2);
        assert!(app.command_popup.is_none());
        assert_eq!(app.goal, "");
        assert_eq!(app.drafts.len(), 1);
        assert_eq!(app.drafts[0].text, "draft stays 漢字");
        assert_eq!(app.drafts[0].cursor, 6);
        assert!(app.planning.pending.is_none());
        assert!(app.execution.active.is_none());
    }

    #[test]
    fn named_form_commands_and_element_aliases_resolve_to_the_same_form() {
        for (index, (name, alias)) in FORM_COMMANDS.iter().enumerate() {
            for command in [name, alias] {
                let mut app = App::new("/workspace".into(), 120, 36);
                app.editing = false;
                assert!(app.command(command));
                assert_eq!(app.form(), index, "/{command} switched the wrong form");
            }
        }
        let mut app = App::new("/workspace".into(), 120, 36);
        app.editing = false;
        assert!(app.command("electric"));
        assert_eq!(app.form(), 2);
        assert!(app.command("electric"));
        assert_eq!(app.form(), 2);
    }

    #[test]
    fn command_and_alias_namespace_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for (name, alias) in FORM_COMMANDS {
            assert!(seen.insert(name), "form command /{name} repeats");
            assert!(seen.insert(alias), "element alias /{alias} repeats");
        }
        let mut app = App::new("/workspace".into(), 120, 36);
        app.editing = false;
        for overlay in [
            None,
            Some(Overlay::Forms),
            Some(Overlay::Help),
            Some(Overlay::Inspector),
            Some(Overlay::Companion),
            Some(Overlay::Preview),
            Some(Overlay::Tasks),
            Some(Overlay::Criteria),
            Some(Overlay::Repository),
            Some(Overlay::Sources),
            Some(Overlay::Execution),
            Some(Overlay::Agents),
            Some(Overlay::Planning),
            Some(Overlay::Handoff),
            Some(Overlay::Images),
            Some(Overlay::Workflows),
            Some(Overlay::Memory),
            Some(Overlay::Party),
        ] {
            app.overlay = overlay;
            let choices = app.command_choices();
            let mut names = std::collections::HashSet::new();
            for choice in &choices {
                assert!(
                    names.insert(choice.command),
                    "duplicate command /{} on {overlay:?}",
                    choice.command
                );
            }
            for name in &seen {
                assert!(
                    names.contains(name),
                    "form command /{name} missing on {overlay:?}"
                );
            }
        }
    }

    #[test]
    fn popup_context_does_not_change_underlying_surface_or_selection() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.editing = false;
        app.overlay = Some(Overlay::Forms);
        app.selected = 7;
        key(&mut app, KeyCode::Char('/'));
        query(&mut app, "role-glaceon");
        assert_eq!(app.command_popup.as_ref().unwrap().candidates().count(), 1);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.overlay, Some(Overlay::Forms));
        assert_eq!(app.selected, 7);
        app.overlay = Some(Overlay::Help);
        key(&mut app, KeyCode::Char('/'));
        query(&mut app, "role-glaceon");
        assert_eq!(app.command_popup.as_ref().unwrap().candidates().count(), 0);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.form(), 0);
    }

    #[test]
    fn request_enter_prepares_once_after_matching_save_without_a_chooser() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.agents.pending = None;
        app.agent_event(crate::providers::agent::Event::Discovered(Vec::new()));
        app.update(Event::Paste("Saved prompt".into()));
        key(&mut app, KeyCode::Enter);
        let revision = app.revision;
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.revision, revision);
        assert!(app.command_popup.is_none());
        assert!(app.planning.pending.is_none());
        assert!(!app.settle_planning());
        let mut snapshot = app.snapshot("/workspace");
        snapshot.id = 7;
        assert_eq!(snapshot.request_text(), "Saved prompt");
        app.save_submitted();
        app.store_event(crate::store::Event::Saved(snapshot.clone()));
        assert!(app.settle_planning());
        assert_eq!(app.overlay, Some(Overlay::Planning));
        assert!(matches!(app.planning.pending.as_ref(),
            Some(crate::providers::Request::Preview(input))
                if input.task.request_text() == "Saved prompt" && !input.inspect_models));
        assert!(app.pending_request.is_none());
        app.store_event(crate::store::Event::Saved(snapshot));
        app.settle_planning();
        assert!(app.pending_request.is_none());
        assert!(!app.planning.affirmative);
    }

    #[test]
    fn resize_neutralizes_popup_and_blocks_stale_affirmation() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.editing = false;
        key(&mut app, KeyCode::Char('/'));
        query(&mut app, "images");
        app.update(Event::Resize(39, 10));
        assert!(app.command_popup.as_ref().unwrap().selected.is_none());
        assert!(!key(&mut app, KeyCode::Enter));
        query(&mut app, "x");
        app.command("images");
        assert!(app.images_pending.is_none());
        assert!(app.overlay.is_none());
        assert!(app.notice.contains("Enlarge"));
        key(&mut app, KeyCode::Esc);
        assert!(app.command_popup.is_none());
    }

    #[test]
    fn busy_request_rechecks_global_prefix_commands_without_dispatch() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.editing = false;
        app.planning.quit_review = true;
        assert!(app.planning.busy());
        for name in [
            "provider-codex",
            "role-plan",
            "model",
            "effort",
            "models-refresh",
            "task-new",
        ] {
            app.command(name);
            assert!(app.planning.pending.is_none());
            assert_eq!(app.task_id, 0);
            assert!(app.notice.contains("active provider"));
        }
    }

    #[test]
    fn ordinary_home_typing_resumes_editor_not_action_dispatch() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.editing = false;
        key(&mut app, KeyCode::Char('n'));
        key(&mut app, KeyCode::Char('e'));
        key(&mut app, KeyCode::Char('w'));
        assert_eq!(app.input, "new");
        assert!(app.editing);
        assert!(app.planning.pending.is_none());
        assert!(app.overlay.is_none());
    }

    #[test]
    fn local_model_picker_can_prepare_inspection_but_pending_work_cannot_repeat_it() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.update(Event::Paste("Inspect available models".into()));
        key(&mut app, KeyCode::Enter);
        app.command_popup = None;
        app.command("model");
        let mut snapshot = app.snapshot("/workspace");
        snapshot.id = 7;
        app.save_requested = false;
        app.store_event(crate::store::Event::Saved(snapshot));
        assert!(app.planning.picker.is_some());
        assert!(!app.authentication_ready());
        app.command("models-refresh");
        assert!(matches!(app.planning.pending.as_ref(),
            Some(crate::providers::Request::Preview(input)) if input.inspect_models && input.resume.is_none()));
        app.command("models-refresh");
        assert!(app.notice.contains("active provider"));
    }

    #[test]
    fn failed_request_save_keeps_draft_without_opening_provider_work() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.update(Event::Paste("Uncommitted goal".into()));
        key(&mut app, KeyCode::Enter);
        assert!(app.command_popup.is_none());
        app.store_event(crate::store::Event::Failed {
            revision: Some(app.revision),
            message: "disk full".into(),
        });
        assert!(app.command_popup.is_none());
        assert!(matches!(app.storage, StorageState::Failed(_)));
        assert!(!app.settle_planning());
        assert_eq!(app.input, "Uncommitted goal");
        assert!(app.planning.pending.is_none());
    }

    #[test]
    fn slash_layout_selection_requests_persistence_without_changing_task_facts() {
        let mut app = App::new("/workspace".into(), 120, 36);
        let mut task = crate::store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.goal = "Saved task".into();
        app.store_event(crate::store::Event::Loaded(crate::store::Project {
            tasks: vec![task],
            partners: vec![crate::store::Partner::default()],
            active_id: Some(7),
            usage_bytes: 0,
        }));
        let revision = app.revision;
        key(&mut app, KeyCode::Char('/'));
        query(&mut app, "layout");
        key(&mut app, KeyCode::Enter);
        assert!(app.linear && app.save_requested && app.revision > revision);
        assert_eq!(app.goal, "Saved task");
        assert!(app.planning.pending.is_none());
    }
}
