use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{App, Overlay, StorageState};

#[derive(Clone, Copy)]
struct RoleSend {
    action: &'static str,
    target: i64,
}

#[derive(Clone)]
pub struct Choice {
    pub command: &'static str,
    pub label: String,
    pub reason: Option<String>,
    role_send: Option<RoleSend>,
}

impl Choice {
    pub fn new(command: &'static str, label: impl Into<String>) -> Self {
        Self {
            command,
            label: label.into(),
            reason: None,
            role_send: None,
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
            role_send: None,
        }
    }
    fn send_role(action: &'static str, target: i64, label: String) -> Self {
        Self {
            command: "send-role",
            label,
            reason: None,
            role_send: Some(RoleSend { action, target }),
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
    pub fn argument(&self) -> Option<(&str, &str)> {
        let query = self
            .query
            .trim()
            .strip_prefix('/')
            .unwrap_or(self.query.trim());
        let (name, argument) = query.split_once(char::is_whitespace)?;
        let argument = argument.trim();
        (matches!(name, "lead" | "model" | "effort") && !argument.is_empty())
            .then_some((name, argument))
    }

    pub fn candidates(&self) -> impl Iterator<Item = &Choice> {
        let query = self
            .query
            .trim()
            .strip_prefix('/')
            .unwrap_or(self.query.trim());
        let argument = self.argument().map(|(name, _)| name);
        let exact = self
            .choices
            .iter()
            .any(|choice| choice.command.eq_ignore_ascii_case(query));
        self.choices.iter().filter(move |choice| {
            if let Some(name) = argument {
                return choice.command == name;
            }
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
            ("reset", "Fresh local task; keep old task and drafts"),
            ("init", "Draft project guide request; review before sending"),
            ("model", "Choose model / optional exact ID"),
            ("effort", "Reasoning level / optional value"),
            ("role", "Choose Plan / Implement / Review role"),
            ("forms", "Choose evolution"),
            ("back-form", "Return to the previous form (Backspace)"),
            (
                "suggestion-dismiss",
                "Dismiss this task's current suggestion; no work",
            ),
            ("carry-over", "Reopen the last form carry-over; no work"),
            ("files", "Attach files / inspect source context"),
            ("tasks", "Saved tasks"),
            ("party", "Party roster"),
            ("nickname", "Edit a local-only partner nickname"),
            (
                "lead",
                "Front a partner: /lead <name> or pick in the roster",
            ),
            ("run", "Local commands / execution evidence / cancellation"),
            ("rerun", "Rerun with Jolteon (new approval)"),
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
                    || matches!(
                        choice.command,
                        "sessions" | "paths" | "models-refresh" | "decision"
                    )
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
        if self.transitioning
            || self.quit_after_save
            || self.reset_after_save
            || self.confirmation.is_some()
        {
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
        if name == "suggestion-dismiss" && self.visible_suggestion().is_none() {
            return Some("No visible suggestion to dismiss.".into());
        }
        if name == "carry-over" && self.last_carry_over().is_none() {
            return Some("No form carry-over yet; switch forms first.".into());
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
                "cancel"
                    | "quit"
                    | "provider"
                    | "answer"
                    | "answers-review"
                    | "verbose"
                    | "reset"
                    | "lead"
                    | "party"
                    | "eevee"
                    | "normal"
            )
        {
            return Some("Finish or cancel the active provider request first.".into());
        }
        if name == "init" && self.execution.busy() {
            return Some(
                "Finish or cancel the local command before preparing an initialization request."
                    .into(),
            );
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
        if matches!(name, "new" | "resume" | "sessions") && self.task_id <= 0 {
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
        self.role_owner(name).map(|owner| {
            let form = crate::domain::FORMS[owner].name;
            format!(
                "{form} {} Choose Switch to {form}; nothing was started.",
                if owner == 2 {
                    "runs commands."
                } else {
                    "edits source."
                }
            )
        })
    }

    /// The form that owns `name` when the active form may not start it (EV03B):
    /// commands belong to Jolteon, source edits to Flareon (Leafeon also edits
    /// its refactor scope). Descriptive routing only; grants are unchanged.
    fn role_owner(&self, name: &str) -> Option<usize> {
        match name {
            "command-edit" if self.form() != 2 => Some(2),
            "paths" | "role-implement" if !matches!(self.form(), 3 | 6) => Some(3),
            _ => None,
        }
    }

    /// An out-of-role action offers a non-replaying switch and reference-only
    /// relays to idle partners already in that owning form (PT10).
    fn offer_owner(&mut self, name: &str) {
        if let Some(owner) = self.role_owner(name)
            && self
                .command_disabled_reason(FORM_COMMANDS[owner].0)
                .is_none()
        {
            let action = match name {
                "command-edit" => "command-edit",
                "paths" => "paths",
                "role-implement" => "role-implement",
                _ => return,
            };
            let mut choices = vec![Choice::new(
                FORM_COMMANDS[owner].0,
                format!("Switch to {}", crate::domain::FORMS[owner].name),
            )];
            for partner in self.partners() {
                if partner.id == self.lead().id
                    || partner.form != owner
                    || self.partner_busy(partner.id)
                {
                    continue;
                }
                let mut choice = Choice::send_role(
                    action,
                    partner.id,
                    format!(
                        "Send to {} ({})",
                        Self::partner_label(&partner),
                        crate::domain::FORMS[owner].name
                    ),
                );
                if self.task_id <= 0 || self.complete {
                    choice.reason =
                        Some("Save an open task before sending role references.".into());
                }
                choices.push(choice);
            }
            self.command_popup = Some(Popup {
                query: String::new(),
                selected: Some(0),
                choices,
                scope: self.overlay,
            });
        }
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
                if popup.argument().is_some() {
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
                    self.offer_owner(choice.command);
                    return true;
                }
                if let Some(send) = choice.role_send {
                    self.command_popup = None;
                    if let Err(error) = self.stage_role_action(send.action, send.target) {
                        self.notice = error;
                    }
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

    /// Literal local arguments share the same selection and permission gates
    /// as their corresponding interactive controls.
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
        if matches!(name, "model" | "effort") && !argument.is_empty() {
            if let Some(reason) = self.command_disabled_reason(name) {
                self.notice = reason;
                return true;
            }
            self.command_popup = None;
            if self.editing {
                self.pending_request = None;
                let field = self.editor_field;
                self.retain_editor();
                self.composer_return = Some(field);
            }
            self.open_overlay(Overlay::Planning);
            if self.overlay != Some(Overlay::Planning) {
                return true;
            }
            let accepted = if name == "model" {
                self.planning.select_model_id(argument)
            } else {
                self.planning.select_effort_value(argument)
            };
            if !accepted
                || (self.planning.picker.is_none() && self.planning.selection_change.is_none())
            {
                self.close_overlay();
            }
            self.notice.clone_from(&self.planning.notice);
            return true;
        }
        false
    }

    pub fn command(&mut self, name: &str) -> bool {
        if let Some(reason) = self.command_disabled_reason(name) {
            self.notice = reason;
            self.offer_owner(name);
            return true;
        }
        self.command_popup = None;
        if name == "carry-over" {
            // A display-only action must not retain/hide the Request editor.
            self.reopen_carry_over();
            return true;
        }
        if name == "suggestion-dismiss" {
            self.dismiss_suggestion();
            return true;
        }
        if name == "nickname" {
            let partner = if self.overlay == Some(Overlay::Party) {
                self.partners()
                    .get(self.selected)
                    .map_or(self.lead().id, |p| p.id)
            } else {
                self.lead().id
            };
            self.open_nickname(partner);
            return true;
        }
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
        if name == "role" {
            // This is a local chooser, not a provider workspace or request.
            // Keep the invoking editor alive so Back needs no extra drawer.
            self.command_popup = Some(Popup {
                query: String::new(),
                selected: None,
                choices: self
                    .planning
                    .command_choices()
                    .into_iter()
                    .filter(|choice| {
                        matches!(
                            choice.command,
                            "role-plan" | "role-implement" | "role-review"
                        )
                    })
                    .collect(),
                scope: self.overlay,
            });
            return true;
        }
        if name == "reset" {
            self.confirmation_reviewed = false;
            self.confirmation = Some(super::Confirmation {
                kind: super::ConfirmationKind::ResetTask,
                destructive: false,
            });
            return true;
        }
        if name == "init" {
            self.prepare_init();
            return true;
        }
        if matches!(name, "eevee" | "normal") && self.planning.busy() {
            self.request_return_to_eevee();
            return true;
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
            if index == 0 {
                self.request_return_to_eevee();
            } else {
                self.request_navigation(super::execution::Action::SelectForm(index));
            }
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
        if matches!(name, "role-plan" | "role-implement" | "role-review") {
            let changed = self.planning_command(name);
            if self.overlay.is_none() {
                self.close_overlay();
            }
            return changed;
        }
        if name.starts_with("provider-") {
            self.open_overlay(Overlay::Planning);
            if self.overlay == Some(Overlay::Planning) {
                let changed = self.planning_command(name);
                if !self.planning.busy() {
                    self.close_overlay();
                    self.notice.clone_from(&self.planning.notice);
                }
                return changed;
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
            | "models-refresh" | "decision" => {
                self.open_overlay(Overlay::Planning);
                if self.overlay == Some(Overlay::Planning) {
                    return self.planning_command(if name == "history" {
                        "provider-journal"
                    } else {
                        name
                    });
                }
            }
            "forms" => self.open_overlay(Overlay::Forms),
            "back-form" => self.previous_form_toggle(),
            "files" => self.open_overlay(Overlay::Sources),
            "run" => self.open_overlay(Overlay::Execution),
            "rerun" => self.rerun_stale_check(),
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
    fn direct_model_and_effort_arguments_preserve_drafts_and_do_not_send() {
        for (width, height) in [(120, 36), (60, 18)] {
            let mut app = App::new("/workspace".into(), width, height);
            app.update(Event::Paste("before suffix".into()));
            app.cursor = 6;
            app.planning.catalogues.insert(
                app.planning.agent,
                crate::providers::ModelCatalogue {
                    models: vec![crate::providers::ModelOption {
                        id: "vendor/Exact:v1".into(),
                        name: "Version 1".into(),
                        description: String::new(),
                        efforts: vec!["low".into(), "high".into()],
                        default_effort: None,
                    }],
                    current: None,
                    source: "local metadata".into(),
                    effort_note: None,
                },
            );
            key(&mut app, KeyCode::Char('/'));
            query(&mut app, "model vendor/Exact:v1");
            key(&mut app, KeyCode::Enter);
            assert!(matches!(
                app.planning.picker,
                Some(super::super::planning::Picker::Effort)
            ));
            assert!(app.planning.selection.model.is_none());
            key(&mut app, KeyCode::Down);
            key(&mut app, KeyCode::Enter);
            assert_eq!(
                app.planning.selection.model.as_deref(),
                Some("vendor/Exact:v1")
            );
            assert_eq!(app.planning.selection.effort.as_deref(), Some("low"));
            assert!(app.editing && app.overlay.is_none());
            assert_eq!((&*app.input, app.cursor), ("before suffix", 6));

            key(&mut app, KeyCode::Char('/'));
            query(&mut app, "effort high");
            key(&mut app, KeyCode::Enter);
            assert_eq!(app.planning.selection.effort.as_deref(), Some("high"));
            assert!(app.editing && app.overlay.is_none());
            assert_eq!((&*app.input, app.cursor), ("before suffix", 6));

            key(&mut app, KeyCode::Char('/'));
            query(&mut app, "effort unsupported");
            key(&mut app, KeyCode::Enter);
            assert_eq!(app.planning.selection.effort.as_deref(), Some("high"));
            assert!(app.editing && app.overlay.is_none());

            key(&mut app, KeyCode::Char('/'));
            query(&mut app, "model provider");
            key(&mut app, KeyCode::Enter);
            assert_eq!(app.planning.selection.model.as_deref(), Some("provider"));
            assert!(app.planning.selection.effort.is_none());
            assert_eq!((&*app.input, app.cursor), ("before suffix", 6));
            assert!(app.editing && app.overlay.is_none());
            assert!(app.planning.pending.is_none());
            assert!(app.pending_request.is_none());
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
        assert!(app.editing);
        assert!(app.overlay.is_none());
        assert_eq!(app.input, "draft stays 漢字");
        assert_eq!(app.cursor, 6);
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
        assert!(
            app.overlay.is_none(),
            "preparation must not open a request popup"
        );
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
    fn model_then_reasoning_returns_to_original_draft_and_caret_without_sending() {
        for (width, height) in [(120, 36), (60, 18)] {
            let mut app = App::new("/workspace".into(), width, height);
            app.update(Event::Paste("before suffix".into()));
            app.cursor = 6;
            app.planning.catalogues.insert(
                crate::providers::Agent::Codex,
                crate::providers::ModelCatalogue {
                    models: vec![crate::providers::ModelOption {
                        id: "exact-model".into(),
                        name: "exact-model".into(),
                        description: String::new(),
                        efforts: vec!["high".into()],
                        default_effort: Some("high".into()),
                    }],
                    current: Some("exact-model".into()),
                    source: "Test metadata".into(),
                    effort_note: None,
                },
            );
            app.command("model");
            key(&mut app, KeyCode::Down);
            key(&mut app, KeyCode::Enter);
            assert!(app.planning.picker == Some(super::super::planning::Picker::Effort));
            assert_eq!(app.overlay, Some(Overlay::Planning));
            assert!(app.planning.selection.model.is_none());
            key(&mut app, KeyCode::Down);
            key(&mut app, KeyCode::Enter);
            assert!(app.overlay.is_none() && app.editing);
            assert_eq!(app.input, "before suffix");
            assert_eq!(app.cursor, 6);
            assert_eq!(app.planning.selection.model.as_deref(), Some("exact-model"));
            assert_eq!(app.planning.selection.effort.as_deref(), Some("high"));
            assert!(app.planning.pending.is_none());
            assert!(app.pending_request.is_none());
            app.command("effort");
            key(&mut app, KeyCode::Home);
            key(&mut app, KeyCode::Esc);
            assert!(app.overlay.is_none() && app.editing);
            assert_eq!(app.planning.selection.effort.as_deref(), Some("high"));
            assert_eq!(app.input, "before suffix");
            assert_eq!(app.cursor, 6);
        }
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
    #[test]
    fn role_chooser_and_selection_return_directly_to_unchanged_composer() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.update(Event::Paste("before suffix".into()));
        app.cursor = 6;
        app.command("role");
        assert!(app.overlay.is_none());
        assert!(app.editing);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.input, "before suffix");
        assert_eq!(app.cursor, 6);
        app.command("role");
        query(&mut app, "role-review");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.planning.operation, crate::providers::Operation::Review);
        assert_eq!(app.form(), 7);
        assert!(app.overlay.is_none());
        assert!(app.command_popup.is_none());
        assert!(app.editing);
        assert_eq!(app.input, "before suffix");
        assert_eq!(app.cursor, 6);
        assert!(app.planning.pending.is_none());
    }

    #[test]
    fn local_picker_returns_to_its_invoker_without_an_empty_planning_drawer() {
        for from_hub in [false, true] {
            for (command, finish) in [
                ("model", KeyCode::Esc),
                ("model", KeyCode::Enter),
                ("effort", KeyCode::Esc),
                ("effort", KeyCode::Enter),
            ] {
                let mut app = App::new("/workspace".into(), 120, 36);
                app.update(Event::Paste("before suffix".into()));
                app.cursor = 6;
                app.planning.catalogues.insert(
                    crate::providers::Agent::Codex,
                    crate::providers::ModelCatalogue {
                        models: Vec::new(),
                        current: None,
                        source: "Inspected metadata".into(),
                        effort_note: None,
                    },
                );
                if from_hub {
                    app.command("provider");
                }
                app.command(command);
                assert!(app.planning.picker.is_some());
                key(&mut app, finish);
                assert!(app.planning.picker.is_none());
                assert!(app.planning.pending.is_none());
                if from_hub {
                    assert_eq!(app.overlay, Some(Overlay::Agents));
                    key(&mut app, KeyCode::Esc);
                }
                assert!(app.overlay.is_none());
                assert!(app.editing);
                assert_eq!(app.input, "before suffix");
                assert_eq!(app.cursor, 6);
            }
        }
    }

    #[test]
    fn fresh_manual_model_refresh_prepares_review_without_saving_request_text() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.update(Event::Paste("private unsaved request".into()));
        assert!(app.command("models-refresh"));
        assert!(matches!(
            app.planning.pending.as_ref(),
            Some(crate::providers::Request::Preview(input))
                if input.inspect_models && input.task.request_text().is_empty()
        ));
        assert_eq!(app.overlay, Some(Overlay::Planning));
        assert!(!app.planning.reviewed);
    }
    fn role_relay_app(width: u16, height: u16, sender_form: usize) -> App {
        let mut app = App::new("/workspace".into(), width, height);
        let partners = vec![
            crate::store::Partner {
                id: 1,
                form: sender_form,
                ..Default::default()
            },
            crate::store::Partner {
                id: 2,
                form: 2,
                nickname: Some("Bolt".into()),
                ..Default::default()
            },
            crate::store::Partner {
                id: 3,
                form: 3,
                nickname: Some("Ember".into()),
                ..Default::default()
            },
            crate::store::Partner {
                id: 4,
                form: 2,
                nickname: Some("Scout".into()),
                ..Default::default()
            },
        ];
        let mut task = crate::store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.goal = "Keep task references and permissions separate".into();
        task.form = sender_form;
        task.partner_drafts.insert(
            1,
            crate::store::PartnerDrafts {
                drafts: vec![crate::store::Draft {
                    field: crate::store::EditorField::Request,
                    text: "sender 漢字 suffix".into(),
                    cursor: 7,
                }],
                editor_field: crate::store::EditorField::Request,
                editing: true,
                previous_form: None,
            },
        );
        app.store_event(crate::store::Event::Loaded(crate::store::Project {
            tasks: vec![task],
            partners,
            active_id: Some(7),
            usage_bytes: 0,
        }));
        app
    }

    fn choose_role_recipient(app: &mut App, action: &str, target: i64) {
        assert!(app.command(action));
        let popup = app.command_popup.as_ref().expect("owning-form choices");
        assert!(
            popup.choices[0].role_send.is_none(),
            "switch remains first and non-replaying"
        );
        let index = popup
            .choices
            .iter()
            .position(|choice| {
                choice
                    .role_send
                    .is_some_and(|send| send.target == target && send.action == action)
            })
            .expect("idle matching recipient choice");
        app.command_popup.as_mut().unwrap().selected = Some(index);
        assert!(app.command_popup_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
    }

    fn recipient_request(app: &App, partner: i64) -> Option<&crate::store::Draft> {
        app.partner_drafts
            .get(&partner)?
            .drafts
            .iter()
            .find(|draft| draft.field == crate::store::EditorField::Request)
    }

    #[test]
    fn owning_role_send_stages_only_reviewed_references_at_both_sizes() {
        for (width, height) in [(120, 36), (60, 18)] {
            for (action, sender_form, target) in [
                ("command-edit", 4, 2),
                ("paths", 7, 3),
                ("role-implement", 7, 3),
            ] {
                let mut app = role_relay_app(width, height, sender_form);
                let before = (
                    app.input.clone(),
                    app.cursor,
                    app.drafts.clone(),
                    app.goal.clone(),
                    app.form(),
                );
                let operation = app.planning.operation;
                let paths = app.planning.edit_paths.clone();
                choose_role_recipient(&mut app, action, target);
                assert!(
                    app.workflow
                        .relay
                        .as_ref()
                        .is_some_and(|relay| relay.confirm && !relay.reviewed)
                );
                assert!(recipient_request(&app, target).is_none());
                app.relay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                assert!(
                    recipient_request(&app, target).is_none(),
                    "neutral Enter never stages"
                );
                app.relay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                assert!(app.workflow.relay.is_none());
                assert_eq!(
                    (
                        app.input.clone(),
                        app.cursor,
                        app.drafts.clone(),
                        app.goal.clone(),
                        app.form()
                    ),
                    before
                );
                choose_role_recipient(&mut app, action, target);
                app.workflow.relay.as_mut().unwrap().reviewed = true;
                app.relay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
                let draft =
                    recipient_request(&app, target).expect("references staged after review");
                assert!(draft.text.contains(action) && draft.text.contains('7'));
                assert_eq!(
                    (
                        app.input.clone(),
                        app.cursor,
                        app.drafts.clone(),
                        app.goal.clone(),
                        app.form()
                    ),
                    before
                );
                assert_eq!(app.lead().id, 1);
                assert_eq!(app.planning.operation, operation);
                assert_eq!(app.planning.edit_paths, paths);
                assert!(app.planning.pending.is_none());
                assert!(app.execution.prepared.is_none() && app.execution.active.is_none());
                assert!(app.command.is_empty());
                let relay = app
                    .workflows
                    .draft_relays
                    .iter()
                    .find(|relay| relay.partner == target)
                    .unwrap();
                assert!(relay.paths.is_empty());
                // Fronting a reference-only recipient never replays role/scope selection.
                app.request_lead(target);
                assert_eq!(app.planning.operation, operation);
                assert_eq!(app.planning.edit_paths, paths);
                assert!(app.planning.pending.is_none() && app.execution.prepared.is_none());
            }
        }
    }

    #[test]
    fn owning_role_send_excludes_busy_or_wrong_form_recipients() {
        let mut app = role_relay_app(120, 36, 4);
        assert!(app.execution.prepare_command_for(
            7,
            Some(crate::store::Attribution::of(&app.party[1])),
            r#"["echo","explicitly prepared but never dispatched"]"#,
        ));
        app.command("command-edit");
        let choices = &app.command_popup.as_ref().unwrap().choices;
        assert!(
            !choices
                .iter()
                .any(|choice| choice.role_send.is_some_and(|send| send.target == 2))
        );
        assert!(
            choices
                .iter()
                .any(|choice| choice.role_send.is_some_and(|send| send.target == 4))
        );
        assert!(
            !choices
                .iter()
                .any(|choice| choice.role_send.is_some_and(|send| send.target == 3))
        );
        assert!(app.planning.pending.is_none() && app.execution.active.is_none());
    }

    #[test]
    fn owning_role_send_rechecks_nonempty_target_and_changed_task() {
        let mut app = role_relay_app(120, 36, 7);
        choose_role_recipient(&mut app, "paths", 3);
        app.partner_drafts
            .entry(3)
            .or_default()
            .drafts
            .push(crate::store::Draft {
                field: crate::store::EditorField::Request,
                text: "keep recipient draft".into(),
                cursor: 4,
            });
        app.workflow.relay.as_mut().unwrap().reviewed = true;
        app.relay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.workflow.notice.contains("nonempty"));
        assert_eq!(
            recipient_request(&app, 3).unwrap().text,
            "keep recipient draft"
        );
        app.relay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.partner_drafts.remove(&3);
        choose_role_recipient(&mut app, "paths", 3);
        app.goal.push_str(" changed after review");
        app.workflow.relay.as_mut().unwrap().reviewed = true;
        app.relay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(recipient_request(&app, 3).is_none());
        assert!(app.workflow.notice.contains("changed"));
        assert!(app.execution.prepared.is_none() && app.planning.pending.is_none());
    }
    #[test]
    fn owning_role_switch_remains_nonreplaying_when_send_choices_exist() {
        for (width, height) in [(120, 36), (60, 18)] {
            let mut app = role_relay_app(width, height, 4);
            let before = (app.input.clone(), app.cursor);
            app.command("command-edit");
            assert_eq!(app.command_popup.as_ref().unwrap().selected, Some(0));
            app.command_popup_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            assert_eq!(app.form(), 2);
            assert_eq!((app.input.clone(), app.cursor), before);
            assert!(app.command.is_empty());
            assert!(app.workflow.relay.is_none());
            assert!(app.execution.prepared.is_none() && app.execution.active.is_none());
            assert!(app.planning.pending.is_none() && app.planning.paths_editor.is_none());
        }
    }
}
