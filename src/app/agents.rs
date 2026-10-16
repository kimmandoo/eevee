//! Passive profile selection and explicit, generation-correlated executable trust.
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::{
    providers::Agent,
    providers::agent::{Event, Profile, Request},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Providers,
    Actions,
    Methods,
    Diagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Connect,
    Status,
    Version,
    Diagnostics,
    Refresh,
    UseForTask,
    Model,
    Effort,
    ConnectionMethod,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connect => "Connect / credential setup",
            Self::Status => "Check connection status",
            Self::Version => "Check installed version",
            Self::Diagnostics => "Advanced diagnostics",
            Self::Refresh => "Refresh installed providers",
            Self::UseForTask => "Use for task",
            Self::Model => "Choose model",
            Self::Effort => "Choose reasoning effort",
            Self::ConnectionMethod => "Change Codex connection method",
        }
    }
}

pub struct State {
    pub pending: Option<Request>,
    pub use_for_task: Option<usize>,
    /// Local picker the app should open for the task provider ("model" or "effort").
    pub pick: Option<&'static str>,
    /// Profile index of the provider the task currently uses; synced by the app.
    pub active: usize,
    pub profiles: Vec<Profile>,
    pub selected: usize,
    pub preview: bool,
    pub affirmative: bool,
    pub reviewed: bool,
    pub scroll: u16,
    pub scroll_max: u16,
    pub notice: String,
    discovering: bool,
    generation: u64,
    inflight: Option<(usize, u64)>,
    pub auth_preview: Option<crate::providers::auth::Plan>,
    pub pending_auth: Option<crate::providers::auth::Plan>,
    auth_preparing: Option<(usize, u64)>,
    pub method: usize,
    pub page: Page,
    pub action: usize,
    pub auth_details: bool,
    pub review_choice: usize,
    outcomes: Vec<Option<crate::providers::auth::Outcome>>,
    /// Live website-OAuth credential expiry (unix seconds) for Devin launches;
    /// `u64::MAX` when the token has no JWT expiry, `None` when detached.
    pub session_credential_until: Option<u64>,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            pending: Some(Request::Discover),
            use_for_task: None,
            pick: None,
            active: 0,
            profiles: Vec::new(),
            selected: 0,
            preview: false,
            affirmative: false,
            reviewed: false,
            scroll: 0,
            scroll_max: 0,
            notice: "Finding installed providers...".into(),
            discovering: true,
            generation: 0,
            inflight: None,
            auth_preview: None,
            pending_auth: None,
            auth_preparing: None,
            method: crate::providers::auth::default_method(0),
            page: Page::Providers,
            action: 0,
            auth_details: false,
            review_choice: 0,
            outcomes: Vec::new(),
            session_credential_until: None,
        }
    }

    pub fn busy(&self) -> bool {
        self.discovering || self.inflight.is_some() || self.pending.is_some() || self.auth_busy()
    }

    pub fn auth_busy(&self) -> bool {
        self.auth_preparing.is_some() || self.pending_auth.is_some()
    }

    pub fn at_root(&self) -> bool {
        self.page == Page::Providers
    }
    /// Public provider rows; Codex's two routes retain their distinct profile indices.
    pub fn provider_indices(&self) -> impl Iterator<Item = usize> + Clone + use<> {
        let codex = if Agent::from_index(self.selected).is_some_and(Agent::is_codex) {
            self.selected
        } else if Agent::from_index(self.active).is_some_and(Agent::is_codex) {
            self.active
        } else {
            Agent::Codex.index()
        };
        (0..self.profiles.len())
            .filter(|index| *index != Agent::ChatGPT.index())
            .map(move |index| {
                if index == Agent::Codex.index() {
                    codex
                } else {
                    index
                }
            })
    }

    pub fn action_label(&self, action: Action) -> &'static str {
        if action == Action::ConnectionMethod {
            if self.selected == Agent::Codex.index() {
                "Use native responses (no tools; no CLI)"
            } else {
                "Use CLI agent (tools / edits)"
            }
        } else {
            action.label()
        }
    }

    fn alternate_codex(&self) -> Option<usize> {
        let alternate = match Agent::from_index(self.selected)? {
            Agent::Codex => Agent::ChatGPT,
            Agent::ChatGPT => Agent::Codex,
            _ => return None,
        };
        self.profiles
            .get(alternate.index())
            .map(|_| alternate.index())
    }

    pub fn actions(&self) -> Vec<Action> {
        let mut actions = vec![Action::Connect];
        if self.alternate_codex().is_some() {
            actions.push(Action::ConnectionMethod);
        }
        if crate::providers::auth::status_method(self.selected).is_some() {
            actions.push(Action::Status);
        }
        if self.selected == self.active {
            actions.extend([Action::Model, Action::Effort]);
        } else {
            actions.push(Action::UseForTask);
        }
        if !Agent::from_index(self.selected).is_some_and(Agent::is_direct) {
            actions.push(Action::Version);
        }
        actions.extend([Action::Diagnostics, Action::Refresh]);
        actions
    }

    fn ready_for_task(&self) -> bool {
        self.profiles.get(self.selected).is_some()
            && matches!(
                self.connection_status(self.selected),
                "Connected" | "Connected (session credential)" | "Configured (unchecked)"
            )
    }

    pub fn enter_hint(&self) -> &'static str {
        if !self.ready_for_task() {
            "Setup"
        } else if self.selected == self.active {
            "Choose model"
        } else {
            "Use provider"
        }
    }

    /// Most likely next step: connect, then use for task, then choose a model.
    fn default_action(&self) -> usize {
        let next = if self.selected == Agent::Codex.index()
            && !self.connectable(self.selected)
            && self.alternate_codex().is_some()
        {
            Action::ConnectionMethod
        } else if !self.ready_for_task() {
            Action::Connect
        } else if self.selected == self.active {
            Action::Model
        } else {
            Action::UseForTask
        };
        self.actions()
            .iter()
            .position(|action| *action == next)
            .unwrap_or(0)
    }

    /// Claude connects through the separate ant CLI, so its executable row is not required.
    fn connectable(&self, index: usize) -> bool {
        Agent::from_index(index).is_some_and(Agent::is_direct)
            || index == Agent::Claude.index()
            || self
                .profiles
                .get(index)
                .is_some_and(|p| p.executable.is_some())
    }

    /// Opens the hub on the task provider, as `/provider` does.
    pub fn focus_active(&mut self) {
        if !self.busy() && self.auth_preview.is_none() && !self.preview {
            self.selected = self.active.min(self.profiles.len().saturating_sub(1));
            self.method = crate::providers::auth::default_method(self.selected);
            self.page = Page::Providers;
            self.scroll = 0;
        }
    }

    pub fn connection_status(&self, index: usize) -> &'static str {
        if self.discovering
            || self.inflight.is_some_and(|(profile, _)| profile == index)
            || self
                .auth_preparing
                .is_some_and(|(profile, _)| profile == index)
            || self
                .pending_auth
                .as_ref()
                .is_some_and(|plan| plan.profile == index)
        {
            return "Checking";
        }
        if let Some(agent) = Agent::from_index(index).filter(|agent| agent.is_direct()) {
            return if crate::providers::direct::config::configured(agent) {
                "Configured (unchecked)"
            } else {
                "Not configured"
            };
        }
        if self.profiles.get(index).is_some() && !self.connectable(index) {
            return "Not installed";
        }
        match self.outcomes.get(index).copied().flatten() {
            _ if index == crate::providers::agent::DEVIN
                && self.session_credential_until.is_some_and(|until| {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|now| now.as_secs() < until)
                        .unwrap_or(false)
                }) =>
            {
                "Connected (session credential)"
            }
            Some(crate::providers::auth::Outcome::Authenticated) => "Connected",
            Some(crate::providers::auth::Outcome::NotAuthenticated) => "Not connected",
            _ => "Unknown",
        }
    }

    pub fn setup_hint(&self) -> String {
        if let Some(agent) = Agent::from_index(self.selected).filter(|agent| agent.is_direct()) {
            return crate::providers::direct::config::guidance(agent);
        }
        match self.selected {
            0 => "Update the same Codex installation shown under Executable, not a second copy elsewhere on PATH. Homebrew: brew upgrade --cask codex. npm: npm install -g @openai/codex@latest. Then Refresh installed providers, check the observed path/version, and refresh /model. Eevee never upgrades it automatically.".into(),
            1 => crate::providers::auth::CLAUDE_POLICY.into(),
            2 => "Install/update latest stable OpenCode on PATH: npm install -g opencode-ai@latest. Then Refresh installed providers; ACP capabilities are negotiated before use.".into(),
            3 => "Install/update latest stable Gemini on PATH: npm install -g @google/gemini-cli@latest. Then Refresh installed providers; current Gemini uses --acp.".into(),
            crate::providers::agent::DEVIN => "Install latest stable Devin from https://docs.devin.ai/cli/; update an existing installation with devin update. Then Refresh installed providers. Eevee does not run an installer automatically.".into(),
            _ => "Select a supported provider before installation or connection setup.".into(),
        }
    }

    fn prepare_auth(&mut self, method: Option<crate::providers::auth::Method>) {
        if let Some(method) = method {
            self.generation += 1;
            self.auth_preparing = Some((self.selected, self.generation));
            self.pending = Some(Request::PrepareAuth {
                profile: self.selected,
                generation: self.generation,
                method,
            });
            self.notice = "Preparing connection review; no provider program launched.".into();
        } else {
            self.notice = if self.selected == Agent::Claude.index() {
                crate::providers::auth::CLAUDE_POLICY.into()
            } else {
                "No read-only status command is available. Native exit leaves connection status unknown.".into()
            };
        }
    }

    fn activate(&mut self, action: Action) {
        match action {
            Action::Connect => {
                if let Some(agent) =
                    Agent::from_index(self.selected).filter(|agent| agent.is_direct())
                {
                    self.notice = crate::providers::direct::config::guidance(agent);
                    self.page = Page::Diagnostics;
                    self.scroll = 0;
                    return;
                }
                if !self.connectable(self.selected) {
                    self.notice = self.setup_hint();
                } else if crate::providers::auth::methods(self.selected).len() > 1 {
                    self.page = Page::Methods;
                } else {
                    self.prepare_auth(
                        crate::providers::auth::methods(self.selected)
                            .first()
                            .copied(),
                    );
                }
            }
            Action::Status => {
                self.prepare_auth(crate::providers::auth::status_method(self.selected))
            }
            Action::Version => {
                if self
                    .profiles
                    .get(self.selected)
                    .is_some_and(|p| p.probe_allowed)
                {
                    self.preview = true;
                    self.affirmative = false;
                    self.reviewed = false;
                    self.scroll = 0;
                    self.scroll_max = 0;
                } else {
                    self.notice = self.setup_hint();
                }
            }
            Action::Diagnostics => {
                self.page = Page::Diagnostics;
                self.scroll = 0;
            }
            Action::Refresh => {
                self.discovering = true;
                self.pending = Some(Request::Discover);
                self.notice =
                    "Refreshing provider configuration; no program or network request launched."
                        .into();
            }
            Action::ConnectionMethod => {
                if let Some(alternate) = self.alternate_codex() {
                    self.selected = alternate;
                    self.method = crate::providers::auth::default_method(alternate);
                    // The app's normal explicit provider switch owns persistence
                    // and any session/handoff boundary; never transfer state here.
                    self.use_for_task = Some(alternate);
                    self.action = self.default_action();
                }
            }
            Action::UseForTask => self.use_for_task = Some(self.selected),
            Action::Model => self.pick = Some("model"),
            Action::Effort => self.pick = Some("effort"),
        }
    }

    pub fn auth_returned(&mut self, profile: usize, outcome: crate::providers::auth::Outcome) {
        self.notice = outcome.label().into();
        if let Some(current) = self.outcomes.get_mut(profile) {
            *current = Some(outcome);
        }
        if let Some(profile) = self.profiles.get_mut(profile) {
            profile.authentication = self.notice.clone();
        }
        if outcome == crate::providers::auth::Outcome::Authenticated && profile == self.selected {
            self.page = Page::Actions;
            self.action = self.default_action();
            self.notice.push_str(if profile == self.active {
                " Next: Choose model."
            } else {
                " Next: Use for task."
            });
        }
        if profile == 0 && outcome != crate::providers::auth::Outcome::Authenticated {
            self.notice.push_str(" Next: if the browser showed localhost connection refused (WSL/SSH), choose Connect > device authorization. On WSL, Windows port reservations can block Codex ports 1455/1457 even while its Linux listener is running. Choose Connect > device authorization to log in without changing Windows networking. Enable device-code authorization in ChatGPT Settings > Security (workspace policy may require an admin). Keep the official login open while entering its code in your browser. Never share codes or callback URLs; cancelled links cannot be reused.");
        }
        self.scroll = 0;
    }

    pub fn event(&mut self, event: Event) {
        match event {
            Event::Discovered(profiles) if self.discovering => {
                self.profiles = profiles;
                // Re-discovery must not erase in-session auth outcomes: a
                // Refresh is not a logout. The profiles vec is registry-order
                // stable, so index-aligned outcomes carry over.
                self.outcomes.resize(self.profiles.len(), None);
                self.selected = self.selected.min(self.profiles.len().saturating_sub(1));
                self.method = crate::providers::auth::default_method(self.selected);
                self.discovering = false;
                self.notice =
                    "Enter chooses a ready provider; Right opens Actions and setup.".into();
            }
            Event::Probed {
                profile,
                generation,
                result,
            } if self.inflight == Some((profile, generation)) => {
                self.inflight = None;
                if let Some(current) = self.profiles.get_mut(profile) {
                    *current = *result;
                }
                self.notice = "Version checked; authentication remains separate.".into();
            }
            Event::AuthPrepared {
                profile,
                generation,
                result,
            } if self.auth_preparing == Some((profile, generation)) => {
                self.auth_preparing = None;
                match result {
                    Ok(plan) => {
                        self.auth_preview = Some(*plan);
                        self.affirmative = false;
                        self.reviewed = false;
                        self.scroll = 0;
                        self.scroll_max = 0;
                        self.auth_details = false;
                        self.review_choice = 0;
                    }
                    Err(reason) => self.notice = reason,
                }
            }
            _ => {}
        }
    }

    pub fn cancel_preview(&mut self) {
        self.preview = false;
        self.auth_preview = None;
        self.pending_auth = None;
        self.auth_preparing = None;
        if matches!(self.pending, Some(Request::PrepareAuth { .. })) {
            self.pending = None;
        }
        self.affirmative = false;
        self.reviewed = false;
        self.scroll = 0;
        self.scroll_max = 0;
        self.auth_details = false;
        self.review_choice = 0;
    }

    pub fn command_choices(&self) -> Vec<super::commands::Choice> {
        use super::commands::Choice;
        let blocked = if self.preview || self.auth_preview.is_some() {
            Some("Finish or cancel the current consent review first.")
        } else if self.busy() {
            Some("A provider operation is pending.")
        } else {
            None
        };
        let profile = self.profiles.get(self.selected);
        let missing = profile
            .is_none()
            .then_some("Wait for installed provider discovery.");
        let connect = if profile.is_some() && !self.connectable(self.selected) {
            Some("This provider is not installed; inspect diagnostics for setup.")
        } else if crate::providers::auth::methods(self.selected).is_empty() {
            Some("This provider has no supported connection method.")
        } else {
            None
        };
        let choice = |name, label, reason: Option<&str>| match reason {
            Some(reason) => Choice::disabled(name, label, reason),
            None => Choice::new(name, label),
        };
        vec![
            choice(
                "connect",
                "Set up selected connection",
                blocked.or(missing).or_else(|| {
                    if Agent::from_index(self.selected).is_some_and(Agent::is_direct) {
                        None
                    } else {
                        connect
                    }
                }),
            ),
            choice(
                "connect-method",
                "Review selected connection method",
                blocked.or(missing).or(connect),
            ),
            choice(
                "method-next",
                "Select next connection method",
                blocked.or(missing).or(connect),
            ),
            choice(
                "status",
                "Review provider connection status check",
                blocked.or(missing).or_else(|| {
                    crate::providers::auth::status_method(self.selected)
                        .is_none()
                        .then_some("This provider has no supported status check.")
                }),
            ),
            choice(
                "version",
                "Review installed provider version check",
                blocked.or(missing).or_else(|| {
                    profile
                        .is_none_or(|p| !p.probe_allowed)
                        .then_some("No installed executable is eligible for this version check.")
                }),
            ),
            choice(
                "diagnostics",
                "Inspect provider diagnostics",
                blocked.or(missing),
            ),
            choice(
                "refresh",
                "Refresh installed providers without launching",
                blocked,
            ),
            choice(
                "connection-method",
                self.action_label(Action::ConnectionMethod),
                blocked.or(missing).or_else(|| {
                    self.alternate_codex()
                        .is_none()
                        .then_some("Select Codex to choose native responses or the CLI agent.")
                }),
            ),
            choice(
                "provider-use",
                "Use selected provider for task",
                blocked.or(missing),
            ),
            choice(
                "details",
                "Toggle full connection review details",
                self.auth_preview
                    .is_none()
                    .then_some("No connection consent review is open."),
            ),
        ]
    }

    pub fn command(&mut self, name: &str, large_enough: bool) -> bool {
        if !large_enough {
            self.affirmative = false;
            self.reviewed = false;
            return false;
        }
        if name == "details" && (self.auth_preview.is_some() || self.preview) {
            self.auth_details = !self.auth_details;
            self.affirmative = false;
            self.reviewed = false;
            self.review_choice = 0;
            self.scroll = 0;
            return true;
        }
        if self.preview || self.auth_preview.is_some() || self.busy() {
            return false;
        }
        match name {
            "connect" => self.activate(Action::Connect),
            "connect-method" => self.prepare_auth(
                crate::providers::auth::methods(self.selected)
                    .get(self.method)
                    .copied(),
            ),
            "method-next" => {
                let count = crate::providers::auth::methods(self.selected).len();
                if count > 0 {
                    self.method = (self.method + 1) % count;
                }
            }
            "status" => self.activate(Action::Status),
            "connection-method" if self.alternate_codex().is_some() => {
                self.activate(Action::ConnectionMethod)
            }
            "version" => self.activate(Action::Version),
            "diagnostics" => self.activate(Action::Diagnostics),
            "refresh" => self.activate(Action::Refresh),
            "provider-use" if self.profiles.get(self.selected).is_some() => {
                self.activate(Action::UseForTask)
            }
            _ => return false,
        }
        true
    }

    pub fn key(&mut self, mut key: KeyEvent, large_enough: bool) -> bool {
        if key.kind == KeyEventKind::Release
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        if key.kind == KeyEventKind::Repeat
            && !matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Home
                    | KeyCode::End
            )
        {
            return false;
        }
        if key.code == KeyCode::Esc {
            if self.preview || self.auth_preview.is_some() || self.auth_busy() {
                self.cancel_preview();
            } else {
                self.page = match self.page {
                    Page::Methods | Page::Diagnostics => Page::Actions,
                    _ => Page::Providers,
                };
            }
            self.scroll = 0;
            return true;
        }
        if !large_enough {
            self.affirmative = false;
            self.reviewed = false;
            return false;
        }
        if self.preview || self.auth_preview.is_some() {
            if key.kind == KeyEventKind::Press
                && key.modifiers.is_empty()
                && self.reviewed
                && !self.auth_details
                && matches!(key.code, KeyCode::Char('y' | 'n'))
            {
                self.affirmative = key.code == KeyCode::Char('y');
                self.review_choice = usize::from(self.affirmative);
                key.code = KeyCode::Enter;
            }
            match key.code {
                KeyCode::Char('d') => {
                    self.auth_details = !self.auth_details;
                    self.affirmative = false;
                    self.reviewed = false;
                    self.review_choice = 0;
                    self.scroll = 0;
                }
                KeyCode::Enter => {
                    if self.auth_details || (self.auth_preview.is_some() && self.review_choice == 2)
                    {
                        self.auth_details = !self.auth_details;
                        self.affirmative = false;
                        self.reviewed = false;
                        self.review_choice = 0;
                        self.scroll = 0;
                    } else if !self.affirmative {
                        self.cancel_preview();
                    } else if self.reviewed && !self.busy() {
                        if self.auth_preview.is_some() {
                            self.pending_auth = self.auth_preview.take();
                            self.affirmative = false;
                            self.reviewed = false;
                            self.notice =
                                "Connection approved once; checking launch prerequisites.".into();
                        } else if self
                            .profiles
                            .get(self.selected)
                            .is_some_and(|p| p.probe_allowed)
                        {
                            self.generation += 1;
                            self.inflight = Some((self.selected, self.generation));
                            self.pending = Some(Request::Probe {
                                profile: self.selected,
                                generation: self.generation,
                            });
                            self.cancel_preview();
                            self.notice = "Checking version only. Closing this drawer does not cancel the bounded check.".into();
                        }
                    }
                }
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    if self.auth_preview.is_some() && !self.auth_details {
                        let backwards = matches!(key.code, KeyCode::BackTab | KeyCode::Left);
                        self.review_choice =
                            (self.review_choice + if backwards { 2 } else { 1 }) % 3;
                        if !self.reviewed && self.review_choice == 1 {
                            self.review_choice = if backwards { 0 } else { 2 };
                        }
                        self.affirmative = self.review_choice == 1 && self.reviewed;
                    } else if self.reviewed && !self.auth_details {
                        self.affirmative = !self.affirmative;
                    }
                }
                KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Home
                | KeyCode::End => self.scroll_key(key.code),
                _ => return false,
            }
            return true;
        }
        if self.auth_busy() {
            return false;
        }
        match key.code {
            KeyCode::Enter if !self.busy() => match self.page {
                Page::Providers => {
                    if self.ready_for_task() {
                        self.activate(if self.selected == self.active {
                            Action::Model
                        } else {
                            Action::UseForTask
                        });
                    } else {
                        self.page = Page::Actions;
                        self.action = self.default_action();
                    }
                }
                Page::Actions => self.activate(self.actions()[self.action]),
                Page::Methods => self.prepare_auth(
                    crate::providers::auth::methods(self.selected)
                        .get(self.method)
                        .copied(),
                ),
                Page::Diagnostics => self.page = Page::Actions,
            },
            KeyCode::Right => {
                self.page = Page::Actions;
                self.scroll = 0;
            }
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left => {
                self.page = match self.page {
                    Page::Providers => Page::Actions,
                    _ => Page::Providers,
                };
                self.scroll = 0;
            }
            KeyCode::Up | KeyCode::Down if self.page == Page::Providers => {
                let indices = self.provider_indices();
                let Some(current) = indices.clone().position(|index| index == self.selected) else {
                    return false;
                };
                let count = indices.clone().count();
                let next = if key.code == KeyCode::Up {
                    (current + count - 1) % count
                } else {
                    (current + 1) % count
                };
                self.selected = indices.clone().nth(next).expect("visible provider index");
                self.method = crate::providers::auth::default_method(self.selected);
                self.action = 0;
                self.scroll = 0;
            }
            KeyCode::Up | KeyCode::Down if self.page != Page::Diagnostics => {
                let (selected, count) = match self.page {
                    Page::Providers => unreachable!("provider navigation handled above"),
                    Page::Methods => (
                        &mut self.method,
                        crate::providers::auth::methods(self.selected).len(),
                    ),
                    _ => {
                        let count = self.actions().len();
                        (&mut self.action, count)
                    }
                };
                if count == 0 {
                    return false;
                }
                *selected = if key.code == KeyCode::Up {
                    (*selected + count - 1) % count
                } else {
                    (*selected + 1) % count
                };
                self.scroll = 0;
            }
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End => self.scroll_key(key.code),
            _ => return false,
        }
        true
    }

    fn scroll_key(&mut self, code: KeyCode) {
        self.scroll = match code {
            KeyCode::Up => self.scroll.saturating_sub(1),
            KeyCode::Down => self.scroll.saturating_add(1).min(self.scroll_max),
            KeyCode::PageUp => self.scroll.saturating_sub(5),
            KeyCode::PageDown => self.scroll.saturating_add(5).min(self.scroll_max),
            KeyCode::Home => 0,
            KeyCode::End => self.scroll_max,
            _ => self.scroll,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile {
            name: "Test profile".into(),
            executable: Some("/trusted/test-agent".into()),
            argv: vec!["--version".into()],
            installed: "present".into(),
            version: "unknown".into(),
            authentication: "unknown".into(),
            configuration: "no configuration read".into(),
            os: "test".into(),
            adapter: "absent".into(),
            action: "disabled".into(),
            probe_allowed: true,
            trust_preview: vec!["Executes trusted test binary".into()],
            revision: 1,
            architecture: "test".into(),
            billing: "unknown".into(),
            blockers: vec!["No adapter".into()],
            capabilities: Vec::new(),
        }
    }

    fn discovered() -> State {
        let mut state = State::new();
        assert!(matches!(state.pending.take(), Some(Request::Discover)));
        state.event(Event::Discovered(vec![
            profile(),
            profile(),
            profile(),
            profile(),
            profile(),
        ]));
        // Keep navigation tests independent of the host recommendation.
        state.method = 0;
        state
    }

    fn press(state: &mut State, code: KeyCode) {
        state.key(KeyEvent::new(code, KeyModifiers::NONE), true);
    }

    #[test]
    fn connected_enter_uses_existing_outputs_without_auth() {
        let mut state = discovered();
        state.outcomes[0] = Some(crate::providers::auth::Outcome::Authenticated);
        state.active = 1;
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.use_for_task.take(), Some(0));
        assert_eq!(state.page, Page::Providers);
        assert!(state.pending.is_none());
        assert!(state.pending_auth.is_none());
        state.active = 0;
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.pick.take(), Some("model"));
        assert!(state.pending.is_none());
    }

    #[test]
    fn right_exposes_actions_without_switching_or_auth() {
        for outcome in [
            None,
            Some(crate::providers::auth::Outcome::Authenticated),
            Some(crate::providers::auth::Outcome::NotAuthenticated),
        ] {
            let mut state = discovered();
            state.outcomes[0] = outcome;
            press(&mut state, KeyCode::Right);
            assert_eq!(state.page, Page::Actions);
            press(&mut state, KeyCode::Right);
            assert_eq!(state.page, Page::Actions);
            assert!(state.use_for_task.is_none());
            assert!(state.pick.is_none());
            assert!(state.pending.is_none());
        }
    }

    #[test]
    fn ready_provider_repeat_and_navigation_are_passive() {
        let mut state = discovered();
        state.outcomes[0] = Some(crate::providers::auth::Outcome::Authenticated);
        state.key(
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat),
            true,
        );
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Up);
        assert_eq!(state.page, Page::Providers);
        assert!(state.use_for_task.is_none());
        assert!(state.pick.is_none());
        assert!(state.pending.is_none());
    }

    #[test]
    fn discovery_and_provider_return_select_recommendation_without_launching() {
        let state = State::new();
        assert_eq!(state.method, crate::providers::auth::default_method(0));
        let mut state = discovered();
        state.method = 1;
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Down);
        assert_eq!(state.method, 0);
        press(&mut state, KeyCode::Up);
        press(&mut state, KeyCode::Up);
        assert_eq!(state.method, crate::providers::auth::default_method(0));
        assert!(state.pending.is_none());
        assert!(state.pending_auth.is_none());
    }

    #[test]
    fn login_failure_offers_recovery_without_substituting_method_or_launching() {
        let mut state = discovered();
        state.method = 0; // Deliberately selected browser, even on WSL/SSH.
        state.auth_returned(0, crate::providers::auth::Outcome::Failed);
        assert_eq!(state.method, 0);
        assert!(state.notice.contains("localhost connection refused"));
        assert!(state.notice.contains("device authorization"));
        assert!(state.pending.is_none());
        assert!(state.pending_auth.is_none());
        state.command("connect-method", true);
        assert!(matches!(
            state.pending,
            Some(Request::PrepareAuth {
                method: crate::providers::auth::Method::CodexBrowser,
                ..
            })
        ));
    }

    #[test]
    fn provider_and_method_navigation_is_passive() {
        let mut state = discovered();
        press(&mut state, KeyCode::Down);
        assert_eq!(state.selected, 1);
        press(&mut state, KeyCode::Up);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.page, Page::Actions);
        assert!(state.pending.is_none());
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.page, Page::Methods);
        assert!(state.pending.is_none());
        press(&mut state, KeyCode::Down);
        let expected = crate::providers::auth::methods(0)[state.method];
        press(&mut state, KeyCode::Enter);
        assert!(matches!(
            state.pending,
            Some(Request::PrepareAuth {
                profile: 0,
                method,
                ..
            }) if method == expected
        ));
        assert!(state.pending_auth.is_none());
        press(&mut state, KeyCode::Esc);
        assert!(!state.auth_busy());
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.page, Page::Actions);
        press(&mut state, KeyCode::Esc);
        assert!(state.at_root());
    }

    #[test]
    fn reviewed_deliberate_consent_is_generation_correlated() {
        let mut state = discovered();
        state.command("version", true);
        state.reviewed = true;
        press(&mut state, KeyCode::Tab);
        press(&mut state, KeyCode::Enter);
        assert!(matches!(
            state.pending.take(),
            Some(Request::Probe {
                profile: 0,
                generation: 1
            })
        ));
        assert!(state.busy());
        press(&mut state, KeyCode::Down);
        state.event(Event::Probed {
            profile: 0,
            generation: 2,
            result: Box::new(profile()),
        });
        assert!(
            state.busy(),
            "stale result must not clear the in-flight request"
        );
        let mut result = profile();
        result.version = "observed".into();
        state.event(Event::Probed {
            profile: 0,
            generation: 1,
            result: Box::new(result),
        });
        assert!(!state.busy());
        assert_eq!(state.profiles[0].version, "observed");
        assert_eq!(state.profiles[1].version, "unknown");
    }

    #[test]
    fn undersize_repeat_and_cancel_cannot_approve() {
        let mut state = discovered();
        state.command("version", true);
        state.reviewed = true;
        press(&mut state, KeyCode::Tab);
        let mut repeated = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        repeated.kind = KeyEventKind::Repeat;
        state.key(repeated, true);
        assert!(state.pending.is_none());
        state.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), false);
        assert!(!state.affirmative);
        assert!(!state.reviewed);
        assert!(state.pending.is_none());
        press(&mut state, KeyCode::Esc);
        assert!(!state.preview);
    }

    #[test]
    fn missing_profile_cannot_open_trust_and_refresh_is_passive() {
        let mut state = discovered();
        state.profiles[0].probe_allowed = false;
        state.command("version", true);
        assert!(!state.preview);
        assert!(state.pending.is_none());
        state.command("refresh", true);
        assert!(matches!(state.pending, Some(Request::Discover)));
        assert!(state.busy());
        state.pending.take();
        state.event(Event::Discovered(vec![Profile {
            executable: None,
            probe_allowed: false,
            ..profile()
        }]));
        press(&mut state, KeyCode::Enter);
        press(&mut state, KeyCode::Enter);
        assert!(state.pending.is_none());
        assert!(state.auth_preview.is_none());
        assert_eq!(state.connection_status(0), "Not installed");
    }

    #[test]
    fn login_review_is_passive_and_cancel_discards_late_generation() {
        let mut state = discovered();
        state.command("method-next", true);
        state.command("connect-method", true);
        assert!(matches!(
            state.pending.take(),
            Some(Request::PrepareAuth {
                profile: 0,
                generation: 1,
                method: crate::providers::auth::Method::CodexDevice
            })
        ));
        assert!(state.auth_busy());
        assert!(state.pending_auth.is_none());
        press(&mut state, KeyCode::Esc);
        assert!(!state.auth_busy());
        let notice = state.notice.clone();
        state.event(Event::AuthPrepared {
            profile: 0,
            generation: 1,
            result: Err("late result".into()),
        });
        assert_eq!(state.notice, notice);
        assert!(state.auth_preview.is_none());
        state.command("connect-method", true);
        assert!(matches!(
            state.pending.take(),
            Some(Request::PrepareAuth { generation: 2, .. })
        ));
        state.event(Event::AuthPrepared {
            profile: 0,
            generation: 1,
            result: Err("stale".into()),
        });
        assert!(state.auth_busy());
    }

    #[test]
    fn undersized_commands_and_repeated_navigation_never_prepare_or_change_method() {
        let mut state = discovered();
        for name in ["connect-method", "status", "method-next"] {
            state.command(name, false);
        }
        assert!(state.pending.is_none());
        assert!(state.pending_auth.is_none());
        assert!(!state.auth_busy());
        assert_eq!(state.method, 0);
        for code in [KeyCode::Enter, KeyCode::Tab] {
            let mut key = KeyEvent::new(code, KeyModifiers::NONE);
            key.kind = KeyEventKind::Repeat;
            state.key(key, true);
            state.key(KeyEvent::new(code, KeyModifiers::NONE), false);
        }
        assert_eq!(state.page, Page::Providers);
        assert!(state.pending.is_none());
    }

    #[test]
    fn provider_switch_resets_method_and_claude_reviews_only_console_oauth() {
        let mut state = discovered();
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Down);
        state.command("method-next", true);
        assert_eq!(state.method, 0);
        press(&mut state, KeyCode::Up);
        assert_eq!(state.method, crate::providers::auth::default_method(1));
        press(&mut state, KeyCode::Enter);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.page, Page::Methods);
        assert!(state.pending.is_none());
        state.command("connect-method", true);
        assert!(state.auth_busy());
        assert!(matches!(
            state.pending,
            Some(Request::PrepareAuth {
                profile: 1,
                method: crate::providers::auth::Method::ClaudeConsole
                    | crate::providers::auth::Method::ClaudeConsoleCode,
                ..
            })
        ));
    }

    #[test]
    fn native_exit_does_not_claim_connected_and_status_checks_are_explicit() {
        let mut state = discovered();
        assert_eq!(state.connection_status(0), "Unknown");
        state.auth_returned(0, crate::providers::auth::Outcome::Authenticated);
        assert_eq!(state.connection_status(0), "Connected");
        state.auth_returned(0, crate::providers::auth::Outcome::Unknown);
        assert_eq!(state.connection_status(0), "Unknown");
        state.auth_returned(0, crate::providers::auth::Outcome::NotAuthenticated);
        assert_eq!(state.connection_status(0), "Not connected");
        press(&mut state, KeyCode::Esc);
        assert!(state.at_root());
        press(&mut state, KeyCode::Enter);
        press(&mut state, KeyCode::Down);
        assert!(state.pending.is_none());
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.connection_status(0), "Checking");
        assert!(matches!(
            state.pending,
            Some(Request::PrepareAuth {
                method: crate::providers::auth::Method::CodexStatus,
                ..
            })
        ));
        assert!(state.pending_auth.is_none());
        press(&mut state, KeyCode::Esc);
        assert!(!state.auth_busy());
    }

    #[test]
    fn hub_suggests_next_step_and_requests_local_pickers_only() {
        let mut state = discovered();
        state.active = 0;
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.actions()[state.action], Action::Connect);
        assert!(!state.actions().contains(&Action::Model));
        state.auth_returned(1, crate::providers::auth::Outcome::Authenticated);
        assert_eq!(state.page, Page::Actions);
        assert_eq!(state.actions()[state.action], Action::UseForTask);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.use_for_task, Some(1));
        assert!(state.pending.is_none() && state.pending_auth.is_none());

        state.active = 1;
        state.use_for_task = None;
        press(&mut state, KeyCode::Esc);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.actions()[state.action], Action::Model);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.pick, Some("model"));
        assert!(state.pending.is_none() && state.pending_auth.is_none());
    }

    #[test]
    fn focus_active_selects_task_provider_without_launching() {
        let mut state = discovered();
        state.active = Agent::Devin.index();
        state.focus_active();
        assert_eq!(state.selected, Agent::Devin.index());
        assert!(state.at_root());
        assert!(state.pending.is_none());
    }

    #[test]
    fn sole_native_method_skips_method_picker_without_launching() {
        for (index, method) in [
            (2, crate::providers::auth::Method::OpenCode),
            (3, crate::providers::auth::Method::GeminiNative),
        ] {
            let mut state = discovered();
            state.selected = index;
            press(&mut state, KeyCode::Enter);
            press(&mut state, KeyCode::Enter);
            assert!(matches!(state.pending, Some(Request::PrepareAuth {
                profile, method: selected_method, ..
            }) if profile == index && selected_method == method));
            assert!(state.pending_auth.is_none());
        }
    }

    #[test]
    fn login_result_keeps_only_classification_and_never_replays_task() {
        let mut state = discovered();
        state.auth_returned(0, crate::providers::auth::Outcome::Unknown);
        assert_eq!(
            state.profiles[0].authentication,
            crate::providers::auth::Outcome::Unknown.label()
        );
        assert_eq!(state.profiles[1].authentication, "unknown");
        assert!(state.pending.is_none());
        assert!(state.pending_auth.is_none());
    }

    #[test]
    fn codex_has_one_public_row_and_native_route_is_not_a_second_provider() {
        let mut state = discovered();
        state.profiles = Agent::ALL.iter().map(|_| profile()).collect();
        assert_eq!(state.provider_indices().count(), Agent::ALL.len() - 1);
        assert!(
            !state
                .provider_indices()
                .any(|index| index == Agent::ChatGPT.index())
        );
        state.active = Agent::ChatGPT.index();
        state.focus_active();
        assert_eq!(state.selected, Agent::ChatGPT.index());
        assert_eq!(
            state.provider_indices().next(),
            Some(Agent::ChatGPT.index())
        );
        press(&mut state, KeyCode::Down);
        assert_eq!(state.selected, Agent::Claude.index());
        press(&mut state, KeyCode::Up);
        assert_eq!(state.selected, Agent::ChatGPT.index());
        assert!(state.pending.is_none() && state.use_for_task.is_none());
    }

    #[test]
    fn native_method_without_cli_uses_explicit_provider_switch_hook_only() {
        let mut state = discovered();
        state.profiles = Agent::ALL.iter().map(|_| profile()).collect();
        state.profiles[Agent::Codex.index()].executable = None;
        state.active = Agent::Codex.index();
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.actions()[state.action], Action::ConnectionMethod);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.selected, Agent::ChatGPT.index());
        assert_eq!(state.use_for_task.take(), Some(Agent::ChatGPT.index()));
        assert_eq!(
            state.active,
            Agent::Codex.index(),
            "only app may change task ownership"
        );
        assert!(state.connectable(Agent::ChatGPT.index()));
        assert!(!state.actions().contains(&Action::Version));
        assert!(!state.actions().contains(&Action::Status));
        assert!(state.pending.is_none() && state.pending_auth.is_none());
        assert!(state.command("connection-method", true));
        assert_eq!(state.use_for_task, Some(Agent::Codex.index()));
        assert!(state.pending.is_none() && state.pending_auth.is_none());
        assert_eq!(Agent::from_command("provider-codex"), Some(Agent::Codex));
        assert_eq!(
            Agent::from_command("provider-chatgpt"),
            Some(Agent::ChatGPT)
        );
    }
}
