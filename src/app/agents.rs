//! Passive profile selection and explicit, generation-correlated executable trust.
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::{
    providers::Agent,
    providers::agent::{Event, Profile, Request},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionField {
    Credential,
    Endpoint,
    Model,
    Account,
    Protocol,
    Keyless,
    Storage,
    Save,
    LoadCli,
    Forget,
    Advanced,
}

pub(crate) struct ConnectionEditor {
    pub form: crate::providers::direct::config::Setup,
    pub field: usize,
    pub cursor: usize,
    /// Set only when the complete local effect/storage disclosure is on screen.
    pub visible: bool,
    pub advanced: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Providers,
    OpenAi,
    Account,
    Install,
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
            Self::ConnectionMethod => "Choose OpenAI connection method",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    Checking,
    Configured,
    Setup,
    MissingCli,
    Blocked,
    SignedIn,
    Session,
    SignedOut,
    Unknown,
}

#[derive(Clone)]
struct OpenAiIntent {
    method: crate::providers::auth::Method,
    generation: u64,
    persist: bool,
    approved: bool,
    owner: Option<(i64, i64, Agent)>,
}

pub fn openai_family(index: usize) -> bool {
    matches!(
        Agent::from_index(index),
        Some(Agent::Codex | Agent::ChatGPT | Agent::OpenAI | Agent::Compatible)
    )
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
    pub search: String,
    pub(crate) connection: Option<ConnectionEditor>,
    pub(crate) forget_preview: Option<crate::providers::direct::config::ForgetPlan>,
    connection_changes: std::collections::VecDeque<Agent>,
    deferred_connection: Option<Agent>,
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
    failures: [Option<crate::providers::diagnostics::Failure>; Agent::ALL.len()],
    failure_attention: [bool; Agent::ALL.len()],
    /// Only redacted host-preparation errors; provider/login response output never enters here.
    preparation_details: [Option<String>; Agent::ALL.len()],
    /// Live website-OAuth credential expiry (unix seconds) for Devin launches;
    /// `u64::MAX` when the token has no JWT expiry, `None` when detached.
    pub session_credential_until: Option<u64>,
    pub family_choice: usize,
    pub login_persist: bool,
    pub connection_owner: Option<(i64, i64, Agent)>,
    openai_intent: Option<OpenAiIntent>,
    deferred_family: bool,
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
            search: String::new(),
            connection: None,
            forget_preview: None,
            connection_changes: std::collections::VecDeque::new(),
            deferred_connection: None,
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
            failures: [None; Agent::ALL.len()],
            failure_attention: [false; Agent::ALL.len()],
            preparation_details: std::array::from_fn(|_| None),
            session_credential_until: None,
            family_choice: 0,
            login_persist: false,
            connection_owner: None,
            openai_intent: None,
            deferred_family: false,
        }
    }

    pub fn busy(&self) -> bool {
        self.discovering || self.inflight.is_some() || self.pending.is_some() || self.auth_busy()
    }

    pub fn auth_busy(&self) -> bool {
        self.auth_preparing.is_some() || self.pending_auth.is_some()
    }

    pub fn at_root(&self) -> bool {
        self.page == Page::Providers && self.connection.is_none() && self.search.is_empty()
    }

    /// Presentation grouping only; registry identities and selected routes stay exact.
    pub fn provider_indices(&self) -> impl Iterator<Item = usize> + Clone + use<> {
        let query = self.search.to_lowercase();
        let family = if openai_family(self.selected) {
            self.selected
        } else if openai_family(self.active) {
            self.active
        } else {
            Agent::ChatGPT.index()
        };
        std::iter::once(family)
            .chain(0..self.profiles.len())
            .filter(|index| *index < self.profiles.len())
            .enumerate()
            .filter(|(position, index)| *position == 0 || !openai_family(*index))
            .map(|(_, index)| index)
            .filter(|index| {
                if openai_family(*index) {
                    ["openai", "chatgpt", "codex", "compatible", "custom"]
                        .iter()
                        .any(|name| name.contains(&query))
                } else {
                    Agent::from_index(*index).is_some_and(|agent| {
                        agent.label().to_lowercase().contains(&query)
                            || agent.command().contains(&query)
                    })
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub fn begin_openai_family(&mut self) {
        if self.connection_editing()
            || self.preview
            || self.auth_preview.is_some()
            || self.auth_busy()
        {
            return;
        }
        if self.discovering {
            self.deferred_family = true;
            return;
        }
        if self.busy() {
            return;
        }
        let current = Agent::from_index(self.active).filter(|agent| openai_family(agent.index()));
        self.selected = crate::providers::direct::config::openai_family_route(current)
            .unwrap_or(Agent::ChatGPT)
            .index();
        self.page = Page::OpenAi;
        self.family_choice = 0;
        self.search.clear();
        self.notice.clear();
    }

    pub fn family_has_current(&self) -> bool {
        self.ready_for_task()
    }

    fn back_to_connection_context(&mut self) {
        self.page = if openai_family(self.selected) {
            if self.selected == Agent::ChatGPT.index() {
                Page::Account
            } else {
                Page::OpenAi
            }
        } else {
            Page::Providers
        };
        self.family_choice = 0;
    }

    fn select_openai_method(&mut self) {
        match self.family_choice {
            0 => {
                self.selected = Agent::ChatGPT.index();
                self.page = Page::Account;
                self.family_choice = 0;
            }
            1 => self.begin_connection(Agent::OpenAI),
            2 => self.begin_connection(Agent::Compatible),
            3 => self.begin_connection(Agent::Codex),
            _ if self.family_has_current() => {
                self.use_for_task = Some(self.selected);
                self.pick = Some("model");
            }
            _ => {}
        }
    }

    fn select_account_method(&mut self) {
        use crate::providers::auth::Method;
        match self.family_choice {
            0 | 1 => {
                let method = if self.family_choice == 0 {
                    Method::OpenAiBrowser
                } else {
                    Method::OpenAiDevice
                };
                self.prepare_auth_for(Agent::ChatGPT.index(), Some(method));
            }
            2 => self.login_persist = !self.login_persist,
            3 => match crate::providers::direct::config::setup(Agent::ChatGPT) {
                Ok(form) => {
                    self.connection = Some(ConnectionEditor {
                        form,
                        field: 0,
                        cursor: 0,
                        visible: false,
                        advanced: true,
                    });
                    self.notice.clear();
                }
                Err(reason) => self.notice = reason,
            },
            _ => {}
        }
    }

    pub fn openai_auth_generation(&self) -> Option<u64> {
        self.openai_intent
            .as_ref()
            .filter(|intent| intent.approved)
            .map(|intent| intent.generation)
    }

    pub fn openai_login_persist(&self) -> bool {
        self.openai_intent
            .as_ref()
            .map_or(self.login_persist, |intent| intent.persist)
    }

    /// Consume only the exact reviewed login; never infer a route from returned credentials.
    pub fn finish_openai_login(
        &mut self,
        plan: &crate::providers::auth::Plan,
        generation: Option<u64>,
        credential: Option<crate::providers::auth::OpenAiCredential>,
        outcome: crate::providers::auth::Outcome,
        owner: &(i64, i64, Agent),
        admitted: bool,
    ) {
        let Some(intent) = self.openai_intent.as_ref() else {
            return;
        };
        if generation != Some(intent.generation)
            || plan.profile != Agent::ChatGPT.index()
            || plan.method != intent.method
        {
            return;
        }
        let intent = self.openai_intent.take().expect("correlated intent");
        if !admitted || !intent.approved || intent.owner.as_ref() != Some(owner) {
            self.page = Page::Account;
            self.notice = "Login continuation expired; no connection settings changed.".into();
            return;
        }
        self.page = Page::Account;
        self.family_choice =
            usize::from(intent.method == crate::providers::auth::Method::OpenAiDevice);
        if outcome != crate::providers::auth::Outcome::Authenticated {
            self.auth_returned(plan.profile, outcome);
            self.notice = "Login did not complete. Retry the selected method.".into();
            return;
        }
        let Some(credential) = credential else {
            self.notice = "Login returned no usable credential; retry this method.".into();
            return;
        };
        match crate::providers::direct::config::apply_openai_login(credential, intent.persist) {
            Ok(applied) => {
                if applied.changed {
                    self.connection_changes.push_back(Agent::ChatGPT);
                    self.connection_succeeded(Agent::ChatGPT);
                }
                if let Some(profile) = self.profiles.get_mut(Agent::ChatGPT.index()) {
                    *profile = crate::providers::direct::config::profile(Agent::ChatGPT);
                }
                if applied.durable {
                    self.use_for_task = Some(Agent::ChatGPT.index());
                    self.pick = Some("model");
                    self.notice = "Login saved; choose model metadata only. No task sent.".into();
                } else {
                    self.notice = "Settings changed but disk durability is unconfirmed; inspect setup before continuing.".into();
                }
            }
            Err(reason) => self.notice = reason,
        }
    }

    pub fn action_label(&self, action: Action) -> &'static str {
        action.label()
    }

    pub fn actions(&self) -> Vec<Action> {
        let mut actions = vec![Action::Connect];
        if openai_family(self.selected) {
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
                self.readiness(self.selected),
                Readiness::Configured | Readiness::SignedIn | Readiness::Session
            )
    }

    pub fn enter_hint(&self) -> &'static str {
        if openai_family(self.selected) {
            return "Setup";
        }
        if !self.ready_for_task() {
            "Setup"
        } else {
            "Choose model"
        }
    }

    /// Most likely next step: connect, then use for task, then choose a model.
    fn default_action(&self) -> usize {
        let next = if !self.ready_for_task() {
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
        if self.discovering {
            self.selected = self.active;
            self.page = Page::Providers;
            self.search.clear();
            return;
        }
        if !self.connection_editing()
            && !self.busy()
            && self.auth_preview.is_none()
            && !self.preview
        {
            self.selected = self.active.min(self.profiles.len().saturating_sub(1));
            self.method = crate::providers::auth::default_method(self.selected);
            self.page = Page::Providers;
            self.search.clear();
            self.scroll = 0;
        }
    }

    /// Explicit setup entry: never changes the task provider or starts inference.
    pub fn begin_connection(&mut self, agent: Agent) {
        if self.connection_editing()
            || self.preview
            || self.auth_preview.is_some()
            || self.auth_busy()
        {
            self.notice = "Finish or cancel the current connection review first.".into();
            return;
        }
        if self.discovering {
            self.deferred_connection = Some(agent);
            self.selected = agent.index();
            return;
        }
        if self.busy() {
            return;
        }
        self.selected = agent.index();
        self.search.clear();
        self.method = crate::providers::auth::default_method(self.selected);
        self.action = 0;
        self.scroll = 0;
        self.activate(Action::Connect);
    }

    /// Opens a retained blocker without stealing an existing review/editor.
    pub fn show_failure(&mut self, agent: Agent) -> bool {
        if self.connection_editing() || self.preview || self.auth_preview.is_some() || self.busy() {
            return false;
        }
        self.selected = agent.index();
        self.search.clear();
        self.page = Page::Diagnostics;
        self.action = 0;
        self.scroll = 0;
        self.scroll_max = 0;
        true
    }

    pub fn connection_editing(&self) -> bool {
        self.connection.is_some() || self.forget_preview.is_some()
    }

    /// Only committed connection mutations need the app's saved-and-idle gate.
    /// All other keys remain owned by the private editor while task saves settle.
    pub fn connection_needs_idle(&self, key: &KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return false;
        }
        if self.forget_preview.is_some() {
            return self.reviewed
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && (key.code == KeyCode::Char('y')
                    || key.code == KeyCode::Enter && self.affirmative);
        }
        let Some(editor) = &self.connection else {
            return false;
        };
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
        {
            return matches!(key.code, KeyCode::Char('s' | 'S'));
        }
        !key.modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && key.code == KeyCode::Enter
            && self.connection_fields().get(editor.field) == Some(&ConnectionField::Save)
    }

    /// Consume all paste owned by this private form, including option/save rows.
    pub fn paste_connection(&mut self, text: &str) -> bool {
        if !self.connection_editing() {
            return false;
        }
        if self.forget_preview.is_some() {
            return true;
        }
        self.insert_connection(text.trim_matches(['\r', '\n']));
        true
    }

    pub fn take_connection_change(&mut self) -> Option<Agent> {
        self.connection_changes.pop_front()
    }

    pub fn connection_fields(&self) -> &'static [ConnectionField] {
        use ConnectionField::*;
        match Agent::from_index(self.selected) {
            Some(Agent::ChatGPT) => &[Credential, Model, Account, Storage, Save, LoadCli, Forget],
            Some(Agent::Compatible) => &[
                Credential, Endpoint, Model, Protocol, Keyless, Storage, Save, Forget,
            ],
            _ if self
                .connection
                .as_ref()
                .is_some_and(|editor| editor.advanced) =>
            {
                &[Credential, Storage, Save, Advanced, Endpoint, Model, Forget]
            }
            _ => &[Credential, Storage, Save, Advanced],
        }
    }

    fn connection_text(&mut self) -> Option<&mut String> {
        let field = *self
            .connection_fields()
            .get(self.connection.as_ref()?.field)?;
        let form = &mut self.connection.as_mut()?.form;
        match field {
            ConnectionField::Credential => Some(&mut form.credential),
            ConnectionField::Endpoint => Some(&mut form.base_url),
            ConnectionField::Model => Some(&mut form.model),
            ConnectionField::Account => Some(&mut form.account_id),
            _ => None,
        }
    }

    fn insert_connection(&mut self, value: &str) {
        let Some(editor) = self.connection.as_ref() else {
            return;
        };
        let cursor = editor.cursor;
        let field = self.connection_fields()[editor.field];
        let limit = match field {
            ConnectionField::Endpoint => 2048,
            ConnectionField::Model => 256,
            _ => 64 * 1024,
        };
        if value.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
            self.notice = "Private paste refused: use a single value without control characters.".into();
            return;
        }
        let Some(text) = self.connection_text() else {
            return;
        };
        if text.len().saturating_add(value.len()) > limit {
            self.notice = "Private field exceeds its bounded input size.".into();
            return;
        }
        let cursor = cursor.min(text.len());
        text.insert_str(cursor, value);
        if let Some(editor) = self.connection.as_mut() {
            editor.cursor = cursor + value.len();
            editor.visible = false;
        }
    }

    fn save_connection(&mut self, choose_model: bool) {
        if !self
            .connection
            .as_ref()
            .is_some_and(|editor| editor.visible)
        {
            self.notice = "Show the full endpoint/storage disclosure before saving.".into();
            return;
        }
        let Some(agent) = Agent::from_index(self.selected) else {
            return;
        };
        match crate::providers::direct::config::apply_setup(
            agent,
            &self.connection.as_ref().expect("private editor").form,
        ) {
            Ok(applied) => {
                self.failure_attention[agent.index()] = false;
                if applied.changed {
                    self.connection_changes.push_back(agent);
                    self.connection_succeeded(agent);
                }
                self.connection = None;
                self.page = Page::Providers;
                self.notice = if !applied.durable {
                    "Settings replaced but disk durability is unconfirmed. Inspect setup; no model metadata or inference sent."
                } else if choose_model {
                    "Native settings saved; choosing this route's model metadata only. No task inference or replay."
                } else {
                    "Native settings saved locally, not authenticated. No route switch, model metadata or inference."
                }.into();
                if applied.effort_reset {
                    self.notice
                        .push_str(" Configured effort explicitly reset for the changed model.");
                }
                if let Some(profile) = self.profiles.get_mut(self.selected) {
                    *profile = crate::providers::direct::config::profile(agent);
                }
                if choose_model && applied.durable {
                    self.use_for_task = Some(self.selected);
                    self.pick = Some("model");
                }
            }
            Err(reason) => self.notice = reason,
        }
    }

    fn forget_key(&mut self, key: KeyEvent, large_enough: bool) -> bool {
        if key.kind != KeyEventKind::Press {
            return true;
        }
        if key.code == KeyCode::Esc {
            self.forget_preview = None;
            self.connection = None;
            self.back_to_connection_context();
            self.reviewed = false;
            self.affirmative = false;
            self.notice =
                "Forget cancelled; private draft discarded. No connection changed.".into();
            return true;
        }
        if !large_enough
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            if !large_enough {
                self.reviewed = false;
                self.affirmative = false;
            }
            return true;
        }
        let approve = match key.code {
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right if self.reviewed => {
                self.affirmative = !self.affirmative;
                return true;
            }
            KeyCode::Char('n') => false,
            KeyCode::Enter if !self.affirmative => false,
            KeyCode::Char('y') | KeyCode::Enter if self.reviewed => true,
            _ => return true,
        };
        if !approve {
            self.forget_preview = None;
            self.reviewed = false;
            self.affirmative = false;
            if let Some(editor) = self.connection.as_mut() {
                editor.visible = false;
            }
            self.notice =
                "Forget cancelled. Existing app/environment/provider sources are unchanged.".into();
            return true;
        }
        if self.busy() {
            return true;
        }
        let plan = self
            .forget_preview
            .take()
            .expect("reviewed private deletion");
        let agent = plan.agent();
        self.reviewed = false;
        self.affirmative = false;
        match crate::providers::direct::config::forget_connection(plan) {
            Ok(applied) => {
                if applied.changed {
                    self.connection_changes.push_back(agent);
                    self.connection_succeeded(agent);
                }
                self.connection = None;
                self.use_for_task = None;
                self.pick = None;
                self.page = Page::Providers;
                self.notice = if applied.durable {
                    "App override forgotten for this route. Future requests may use existing env/provider sources; no network or remote revocation."
                } else {
                    "App override removed locally; disk durability unconfirmed. Future requests may use env/provider sources. No network or remote revocation."
                }.into();
                if let Some(profile) = self.profiles.get_mut(agent.index()) {
                    *profile = crate::providers::direct::config::profile(agent);
                }
            }
            Err(reason) => {
                if let Some(editor) = self.connection.as_mut() {
                    editor.visible = false;
                }
                self.notice = reason;
            }
        }
        true
    }

    fn connection_key(&mut self, key: KeyEvent, large_enough: bool) -> bool {
        if self.forget_preview.is_some() {
            return self.forget_key(key, large_enough);
        }
        if key.kind == KeyEventKind::Release {
            return true;
        }
        if key.code == KeyCode::Esc {
            self.connection = None;
            self.back_to_connection_context();
            self.notice = "Private setup cancelled; no connection settings changed.".into();
            return true;
        }
        if large_enough
            && key.kind == KeyEventKind::Press
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('a' | 'A' | 'e' | 'E' | 'u' | 'U'))
        {
            if let Some(text) = self.connection_text() {
                let cursor = match key.code {
                    KeyCode::Char('e' | 'E') => text.len(),
                    KeyCode::Char('u' | 'U') => {
                        text.clear();
                        0
                    }
                    _ => 0,
                };
                let editor = self.connection.as_mut().expect("private editor");
                editor.cursor = cursor;
                editor.visible = false;
            }
            self.scroll = 0;
            return true;
        }
        if large_enough
            && key.kind == KeyEventKind::Press
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('s' | 'S'))
        {
            self.scroll = 0;
            self.save_connection(false);
            return true;
        }
        if !large_enough
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return true;
        }
        if key.kind == KeyEventKind::Repeat
            && !matches!(
                key.code,
                KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Backspace
                    | KeyCode::Delete
                    | KeyCode::PageUp
                    | KeyCode::PageDown
            )
        {
            return true;
        }
        if !matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            self.scroll = 0;
        }
        let fields = self.connection_fields();
        let field = fields[self.connection.as_ref().expect("private editor").field];
        match key.code {
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                let backwards = matches!(key.code, KeyCode::BackTab | KeyCode::Up);
                let editor = self.connection.as_mut().expect("private editor");
                editor.field =
                    (editor.field + if backwards { fields.len() - 1 } else { 1 }) % fields.len();
                let cursor = self.connection_text().map_or(0, |text| text.len());
                self.connection.as_mut().expect("private editor").cursor = cursor;
            }
            KeyCode::Char(' ') | KeyCode::Enter
                if matches!(
                    field,
                    ConnectionField::Protocol | ConnectionField::Keyless | ConnectionField::Storage
                ) =>
            {
                let editor = self.connection.as_mut().expect("private editor");
                match field {
                    ConnectionField::Protocol => editor.form.responses = !editor.form.responses,
                    ConnectionField::Keyless => editor.form.keyless = !editor.form.keyless,
                    ConnectionField::Storage => editor.form.persist = !editor.form.persist,
                    _ => {}
                }
                editor.visible = false;
            }
            KeyCode::Enter if field == ConnectionField::Save => self.save_connection(true),
            KeyCode::Enter if field == ConnectionField::Advanced => {
                let editor = self.connection.as_mut().expect("private editor");
                editor.advanced = !editor.advanced;
                editor.visible = false;
            }
            KeyCode::Enter if field == ConnectionField::LoadCli => {
                let editor = self.connection.as_mut().expect("private editor");
                match crate::providers::direct::config::load_cli_credential(&mut editor.form) {
                    Ok(()) => {
                        editor.field = 0;
                        editor.cursor = editor.form.credential.len();
                        editor.visible = false;
                        self.notice = "CLI token loaded into masked draft only. Save explicitly selects it; no request or persistence yet.".into();
                    }
                    Err(reason) => self.notice = reason,
                }
            }
            KeyCode::Enter if field == ConnectionField::Forget => {
                let Some(agent) = Agent::from_index(self.selected) else {
                    return true;
                };
                match crate::providers::direct::config::prepare_forget(agent) {
                    Ok(plan) => {
                        self.forget_preview = Some(plan);
                        self.reviewed = false;
                        self.affirmative = false;
                        self.notice = "Review exact local deletion. Back is selected; nothing removed or sent.".into();
                    }
                    Err(reason) => self.notice = reason,
                }
            }
            KeyCode::Enter => {
                let editor = self.connection.as_mut().expect("private editor");
                editor.field = (editor.field + 1) % fields.len();
                let cursor = self.connection_text().map_or(0, |text| text.len());
                self.connection.as_mut().expect("private editor").cursor = cursor;
            }
            KeyCode::Char(c) => self.insert_connection(&c.to_string()),
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Backspace
            | KeyCode::Delete => {
                let cursor = self.connection.as_ref().expect("private editor").cursor;
                if let Some(text) = self.connection_text() {
                    let cursor = cursor.min(text.len());
                    let previous = text[..cursor]
                        .char_indices()
                        .next_back()
                        .map_or(0, |(index, _)| index);
                    let next = text[cursor..]
                        .chars()
                        .next()
                        .map_or(cursor, |c| cursor + c.len_utf8());
                    let position = match key.code {
                        KeyCode::Home => 0,
                        KeyCode::End => text.len(),
                        KeyCode::Left => previous,
                        KeyCode::Right => next,
                        KeyCode::Backspace => {
                            text.replace_range(previous..cursor, "");
                            previous
                        }
                        KeyCode::Delete => {
                            text.replace_range(cursor..next, "");
                            cursor
                        }
                        _ => cursor,
                    };
                    let editor = self.connection.as_mut().expect("private editor");
                    editor.cursor = position;
                    editor.visible = false;
                }
            }
            KeyCode::PageUp | KeyCode::PageDown => self.scroll_key(key.code),
            _ => {}
        }
        true
    }

    pub fn readiness(&self, index: usize) -> Readiness {
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
            return Readiness::Checking;
        }
        if self.failure(index).is_some()
            && self.failure_attention.get(index).copied().unwrap_or(false)
        {
            return Readiness::Blocked;
        }
        if let Some(agent) = Agent::from_index(index).filter(|agent| agent.is_direct()) {
            return if crate::providers::direct::config::configured(agent) {
                Readiness::Configured
            } else {
                Readiness::Setup
            };
        }
        if self.profiles.get(index).is_some() && !self.connectable(index) {
            return Readiness::MissingCli;
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
                Readiness::Session
            }
            Some(crate::providers::auth::Outcome::Authenticated) => Readiness::SignedIn,
            Some(crate::providers::auth::Outcome::NotAuthenticated) => Readiness::SignedOut,
            _ => Readiness::Unknown,
        }
    }

    /// Content-free, actual observations only. Parent validates request ownership first.
    pub fn connection_failed(
        &mut self,
        agent: Agent,
        failure: crate::providers::diagnostics::Failure,
    ) {
        self.failures[agent.index()] = Some(failure);
        self.failure_attention[agent.index()] = true;
        self.preparation_details[agent.index()] = None;
        if matches!(
            failure.kind(),
            crate::providers::diagnostics::Kind::Authentication
        ) && let Some(outcome) = self.outcomes.get_mut(agent.index())
        {
            *outcome = Some(crate::providers::auth::Outcome::NotAuthenticated);
        }
        if self.selected == agent.index() {
            self.scroll = 0;
        }
    }

    /// Only actual successful inference or a changed connection supersedes a failure;
    /// successful metadata discovery is deliberately not authentication evidence.
    pub fn connection_succeeded(&mut self, agent: Agent) {
        self.failures[agent.index()] = None;
        self.failure_attention[agent.index()] = false;
        self.preparation_details[agent.index()] = None;
    }

    pub fn failure(&self, index: usize) -> Option<crate::providers::diagnostics::Failure> {
        self.failures.get(index).copied().flatten()
    }

    pub fn preparation_detail(&self, index: usize) -> Option<&str> {
        self.preparation_details
            .get(index)
            .and_then(|detail| detail.as_deref())
    }

    pub fn auth_outcome(&self, index: usize) -> Option<crate::providers::auth::Outcome> {
        self.outcomes.get(index).copied().flatten()
    }

    pub fn missing_executable(&self, index: usize) -> bool {
        !Agent::from_index(index).is_some_and(Agent::is_direct)
            && index != Agent::Claude.index()
            && self
                .profiles
                .get(index)
                .is_some_and(|profile| profile.executable.is_none())
    }

    /// Opens existing local setup/review only. No resend, retry or grant is created.
    fn remediate(&mut self) {
        use crate::providers::diagnostics::{Failure, Kind};
        // A deliberate next step acknowledges this observation, not authentication.
        // Keep its content-free details; a later explicit send still requalifies.
        self.failure_attention[self.selected] = false;
        if self.missing_executable(self.selected)
            || self.failure(self.selected).is_some_and(|failure| {
                matches!(
                    failure.kind(),
                    Kind::Incompatible | Kind::VersionCheck | Kind::LoginRefused
                ) && !Agent::from_index(self.selected).is_some_and(Agent::is_direct)
            })
        {
            self.page = Page::Install;
            self.scroll = 0;
        } else if matches!(
            self.failure(self.selected),
            Some(Failure::Http(403 | 404 | 405 | 429 | 400 | 422))
        ) && self.selected == self.active
        {
            self.activate(Action::Model);
        } else {
            self.activate(Action::Connect);
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
        self.prepare_auth_for(self.selected, method);
    }

    fn prepare_auth_for(&mut self, profile: usize, method: Option<crate::providers::auth::Method>) {
        if let Some(method) = method {
            self.generation += 1;
            self.openai_intent = if matches!(
                method,
                crate::providers::auth::Method::OpenAiBrowser
                    | crate::providers::auth::Method::OpenAiDevice
            ) {
                Some(OpenAiIntent {
                    method,
                    generation: self.generation,
                    persist: self.login_persist,
                    approved: false,
                    owner: self.connection_owner,
                })
            } else {
                None
            };
            self.auth_preparing = Some((profile, self.generation));
            self.pending = Some(Request::PrepareAuth {
                profile,
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
                if self.selected == Agent::ChatGPT.index() {
                    self.page = Page::Account;
                    self.family_choice = 0;
                    self.notice.clear();
                    return;
                }
                if let Some(agent) =
                    Agent::from_index(self.selected).filter(|agent| agent.is_direct())
                {
                    match crate::providers::direct::config::setup(agent) {
                        Ok(form) => {
                            self.connection = Some(ConnectionEditor {
                                form,
                                field: 0,
                                cursor: 0,
                                visible: false,
                                advanced: false,
                            });
                            self.notice = self.connection.as_ref().and_then(|editor| editor.form.source_notice.clone()).unwrap_or_else(|| "Blank key keeps source. Ctrl+U clears a field; Ctrl+S saves locally without model metadata.".into());
                        }
                        Err(reason) => self.notice = reason,
                    }
                    return;
                }
                if !self.connectable(self.selected) {
                    self.page = Page::Install;
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
                self.page = Page::OpenAi;
                self.family_choice = 0;
            }
            Action::UseForTask => {
                self.use_for_task = Some(self.selected);
                self.pick = Some("model");
            }
            Action::Model => self.pick = Some("model"),
            Action::Effort => self.pick = Some("effort"),
        }
    }

    pub fn auth_returned(&mut self, profile: usize, outcome: crate::providers::auth::Outcome) {
        use crate::providers::diagnostics::Failure;
        let failure = match outcome {
            crate::providers::auth::Outcome::NotAuthenticated => Some(Failure::SignedOut),
            crate::providers::auth::Outcome::Cancelled => Some(Failure::LoginCancelled),
            crate::providers::auth::Outcome::Expired => Some(Failure::LoginExpired),
            crate::providers::auth::Outcome::Failed => Some(Failure::LoginFailed),
            crate::providers::auth::Outcome::Refused => Some(Failure::LoginRefused),
            _ => None,
        };
        if let Some(agent) = Agent::from_index(profile) {
            if let Some(failure) = failure {
                self.connection_failed(agent, failure);
            } else if outcome == crate::providers::auth::Outcome::Authenticated {
                self.connection_succeeded(agent);
            }
        }
        if failure.is_some() && profile == self.selected && profile != Agent::ChatGPT.index() {
            self.page = Page::Actions;
            self.action = self.default_action();
        }
        self.notice = outcome.label().into();
        if let Some(current) = self.outcomes.get_mut(profile) {
            *current = Some(outcome);
        }
        if let Some(profile) = self.profiles.get_mut(profile) {
            profile.authentication = outcome.label().into();
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
                if self.page == Page::Install && self.connectable(self.selected) {
                    self.page = Page::Methods;
                }
                self.notice =
                    "Enter chooses a ready provider; Right opens Actions and setup.".into();
                if self.deferred_family {
                    self.deferred_family = false;
                    self.begin_openai_family();
                } else if let Some(agent) = self.deferred_connection.take() {
                    self.begin_connection(agent);
                }
            }
            Event::Probed {
                profile,
                generation,
                result,
            } if self.inflight == Some((profile, generation)) => {
                self.inflight = None;
                if let Some(current) = self.profiles.get_mut(profile) {
                    if result.version.starts_with("Probe failed:")
                        && matches!(
                            self.failures[profile],
                            None | Some(crate::providers::diagnostics::Failure::VersionCheck)
                        )
                    {
                        self.failures[profile] =
                            Some(crate::providers::diagnostics::Failure::VersionCheck);
                        self.failure_attention[profile] = true;
                    } else if self.failures[profile]
                        == Some(crate::providers::diagnostics::Failure::VersionCheck)
                    {
                        self.failures[profile] = None;
                        self.failure_attention[profile] = false;
                    }
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
                    Ok(mut plan) => {
                        if self.openai_intent.as_ref().is_some_and(|intent| {
                            intent.generation != generation
                                || intent.method != plan.method
                                || profile != Agent::ChatGPT.index()
                        }) {
                            self.openai_intent = None;
                            self.notice =
                                "Connection review no longer matches its requested method.".into();
                            return;
                        }
                        if let Some(intent) = &self.openai_intent {
                            plan.lines.push(if intent.persist {
                                "Replace the saved ChatGPT app account with this login in private local plaintext, not encrypted or keychain."
                            } else {
                                "Use this login for this session only; shadow the saved ChatGPT app account until exit without changing its file."
                            }.into());
                            plan.lines.push("After success, select only ChatGPT and open model metadata selection through task/partner switch review. Never send or replay the Request. Reconnect on credential expiry.".into());
                        }
                        self.auth_preview = Some(*plan);
                        self.affirmative = false;
                        self.reviewed = false;
                        self.scroll = 0;
                        self.scroll_max = 0;
                        self.auth_details = false;
                        self.review_choice = 0;
                    }
                    Err(reason) => {
                        if let Some(agent) = Agent::from_index(profile) {
                            self.connection_failed(
                                agent,
                                crate::providers::diagnostics::Failure::LoginRefused,
                            );
                            let detail: String = crate::workspace::privacy::redact(&reason)
                                .chars()
                                .filter(|c| !c.is_control() && !matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
                                .take(2048)
                                .collect();
                            self.preparation_details[profile] = Some(detail);
                        }
                        self.notice = crate::workspace::privacy::redact(&reason).into_owned();
                        self.page = Page::Diagnostics;
                        self.scroll = 0;
                    }
                }
            }
            _ => {}
        }
    }

    pub fn cancel_preview(&mut self) {
        self.connection = None;
        self.forget_preview = None;
        self.preview = false;
        self.auth_preview = None;
        self.pending_auth = None;
        self.auth_preparing = None;
        self.openai_intent = None;
        self.deferred_family = false;
        self.deferred_connection = None;
        if matches!(self.pending, Some(Request::PrepareAuth { .. })) {
            self.pending = None;
        }
        self.affirmative = false;
        self.reviewed = false;
        self.scroll = 0;
        self.scroll_max = 0;
        self.auth_details = false;
        self.review_choice = 0;
        self.page = Page::Providers;
    }

    fn cancel_auth_review(&mut self) {
        let page = self.page;
        self.cancel_preview();
        self.page = page;
        self.notice = "Connection review cancelled; no provider program launched.".into();
    }

    pub fn command_choices(&self) -> Vec<super::commands::Choice> {
        use super::commands::Choice;
        let blocked = if self.connection_editing() {
            Some("Finish or cancel the private native setup first.")
        } else if self.preview || self.auth_preview.is_some() {
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
                "connection-setup",
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
                    (!openai_family(self.selected)).then_some(
                        "Select OpenAI to choose account, API key, custom, or advanced CLI.",
                    )
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
        if self.connection_editing() || self.preview || self.auth_preview.is_some() || self.busy() {
            return false;
        }
        match name {
            "connection-setup" => self.activate(Action::Connect),
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
            "connection-method" if openai_family(self.selected) => {
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
        if self.connection_editing() {
            return self.connection_key(key, large_enough);
        }
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
            if self.auth_preview.is_some() || self.auth_busy() {
                self.cancel_auth_review();
            } else if self.preview {
                self.cancel_preview();
            } else {
                self.page = match self.page {
                    Page::Account => Page::OpenAi,
                    Page::Methods | Page::Install | Page::Actions
                        if openai_family(self.selected) =>
                    {
                        Page::OpenAi
                    }
                    _ => Page::Providers,
                };
                self.search.clear();
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
                        if self.auth_preview.is_some() {
                            self.cancel_auth_review();
                        } else {
                            self.cancel_preview();
                        }
                    } else if self.reviewed && !self.busy() {
                        if self.auth_preview.is_some() {
                            if let Some(intent) = self.openai_intent.as_mut() {
                                intent.approved = self.auth_preview.as_ref().is_some_and(|plan| {
                                    plan.profile == Agent::ChatGPT.index()
                                        && plan.method == intent.method
                                });
                            }
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
            KeyCode::Char('d')
                if matches!(
                    self.page,
                    Page::Actions | Page::Install | Page::Account | Page::Methods
                ) =>
            {
                self.activate(Action::Diagnostics);
            }
            KeyCode::Char('n')
                if !self.busy() && matches!(self.page, Page::Actions | Page::Diagnostics) =>
            {
                self.remediate();
            }
            KeyCode::Enter if !self.busy() => match self.page {
                Page::Providers if !self.provider_indices().any(|index| index == self.selected) => {
                }
                Page::Providers => {
                    if openai_family(self.selected) {
                        self.page = Page::OpenAi;
                        self.family_choice = 0;
                        self.notice.clear();
                        return true;
                    }
                    if self.ready_for_task() {
                        self.activate(if self.selected == self.active {
                            Action::Model
                        } else {
                            Action::UseForTask
                        });
                    } else {
                        self.activate(Action::Connect);
                    }
                }
                Page::Actions => self.activate(self.actions()[self.action]),
                Page::Methods => self.prepare_auth(
                    crate::providers::auth::methods(self.selected)
                        .get(self.method)
                        .copied(),
                ),
                Page::Diagnostics => self.remediate(),
                Page::OpenAi => self.select_openai_method(),
                Page::Account => self.select_account_method(),
                Page::Install => self.activate(Action::Refresh),
            },
            KeyCode::Right if !matches!(self.page, Page::OpenAi | Page::Account) => {
                self.page = Page::Actions;
                self.action = self.default_action();
                self.scroll = 0;
            }
            KeyCode::Tab | KeyCode::BackTab
                if matches!(self.page, Page::OpenAi | Page::Account) =>
            {
                let count = if self.page == Page::Account {
                    4
                } else {
                    4 + usize::from(self.family_has_current())
                };
                self.family_choice = (self.family_choice
                    + if key.code == KeyCode::BackTab {
                        count - 1
                    } else {
                        1
                    })
                    % count;
            }
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left => {
                self.page = match self.page {
                    Page::Providers => Page::Actions,
                    Page::Account => Page::OpenAi,
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
                    Page::OpenAi => {
                        let count = 4 + usize::from(self.family_has_current());
                        (&mut self.family_choice, count)
                    }
                    Page::Account => (&mut self.family_choice, 4),
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
            KeyCode::Char(c)
                if self.page == Page::Providers && !c.is_control() && self.search.len() < 128 =>
            {
                self.search.push(c);
                if !self.provider_indices().any(|index| index == self.selected)
                    && let Some(index) = self.provider_indices().next()
                {
                    self.selected = index;
                    self.method = crate::providers::auth::default_method(index);
                }
            }
            KeyCode::Backspace if self.page == Page::Providers => {
                self.search.pop();
                if !self.provider_indices().any(|index| index == self.selected)
                    && let Some(index) = self.provider_indices().next()
                {
                    self.selected = index;
                    self.method = crate::providers::auth::default_method(index);
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
    fn family_enter_preserves_saved_cli_and_requires_explicit_current_choice() {
        let mut state = discovered();
        state.outcomes[0] = Some(crate::providers::auth::Outcome::Authenticated);
        state.active = Agent::Codex.index();
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.page, Page::OpenAi);
        assert_eq!(state.selected, Agent::Codex.index());
        assert!(state.use_for_task.is_none() && state.pick.is_none());
        state.family_choice = 4;
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.use_for_task, Some(Agent::Codex.index()));
        assert_eq!(state.pick, Some("model"));
        assert!(state.pending.is_none() && state.pending_auth.is_none());
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
    fn family_login_choice_is_native_and_does_not_select_a_task_route() {
        let mut state = discovered();
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.page, Page::OpenAi);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.page, Page::Account);
        assert_eq!(state.selected, Agent::ChatGPT.index());
        assert!(state.pending.is_none() && state.use_for_task.is_none());
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Enter);
        assert!(
            matches!(state.pending, Some(Request::PrepareAuth { profile, method: crate::providers::auth::Method::OpenAiDevice, .. }) if profile == Agent::ChatGPT.index())
        );
        assert!(state.pending_auth.is_none());
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.page, Page::Account);
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.page, Page::OpenAi);
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
        assert_eq!(state.readiness(0), Readiness::MissingCli);
        assert!(state.missing_executable(0));
        assert!(state.show_failure(Agent::Codex));
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.page, Page::Install);
        assert!(state.pending.is_none() && state.pending_auth.is_none());
        state.discovering = true;
        state.event(Event::Discovered(vec![profile()]));
        assert_eq!(state.readiness(0), Readiness::Unknown);
        assert!(
            state.pending_auth.is_none(),
            "install discovery never authenticates or launches login"
        );
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
    fn cancelled_auth_preparation_and_neutral_back_allow_fresh_worker_reviews() {
        use crate::app::{App, Overlay, StorageState};
        use crossterm::event::Event as InputEvent;

        let worker = crate::providers::agent::Worker::start(std::env::temp_dir()).unwrap();
        let receive = || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if let Some(event) = worker.try_event() {
                    break event;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "auth preparation did not finish"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        let mut app = App::new("/workspace".into(), 60, 18);
        app.storage = StorageState::Saved;
        app.overlay = Some(Overlay::Agents);
        app.input = "retained request".into();
        app.cursor = 8;
        app.agents = discovered();
        app.agents.selected = Agent::Devin.index();
        app.agents.method = 0;
        app.agents.page = Page::Methods;
        let enter = InputEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.update(enter.clone());
        let mut cancelled = app.agents.pending.take();
        assert!(matches!(
            cancelled,
            Some(Request::PrepareAuth { generation: 1, .. })
        ));
        app.update(InputEvent::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.agents.page, Page::Methods);
        app.update(enter.clone());
        assert!(matches!(
            app.agents.pending,
            Some(Request::PrepareAuth { generation: 2, .. })
        ));

        // Use real passive preparation, not a synthetic completion or a login.
        assert!(worker.try_request(&mut cancelled));
        let stale = receive();
        assert!(matches!(stale, Event::AuthPrepared { generation: 1, .. }));
        app.agent_event(stale);
        assert!(app.agents.auth_busy() && app.agents.auth_preview.is_none());
        assert!(worker.try_request(&mut app.agents.pending));
        let current = receive();
        assert!(matches!(current, Event::AuthPrepared { generation: 2, .. }));
        app.agent_event(current);
        let plan = app
            .agents
            .auth_preview
            .as_ref()
            .expect("second generation review");
        assert_eq!(
            (plan.profile, plan.method),
            (
                Agent::Devin.index(),
                crate::providers::auth::Method::DevinWeb
            )
        );
        assert!(app.agents.pending_auth.is_none());

        app.update(enter.clone());
        assert_eq!(app.agents.page, Page::Methods);
        assert!(!app.agents.auth_busy() && app.agents.auth_preview.is_none());
        assert!(app.agents.pending.is_none() && app.agents.pending_auth.is_none());
        app.update(enter.clone());
        assert!(matches!(
            app.agents.pending,
            Some(Request::PrepareAuth { generation: 3, .. })
        ));
        assert!(worker.try_request(&mut app.agents.pending));
        let next = receive();
        assert!(matches!(next, Event::AuthPrepared { generation: 3, .. }));
        app.agent_event(next);
        assert!(app.agents.auth_preview.is_some());
        assert!(app.agents.pending_auth.is_none());
        assert_eq!((&*app.input, app.cursor), ("retained request", 8));
        assert!(app.planning.pending.is_none());
        app.agents.reviewed = true;
        app.update(InputEvent::Key(KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        )));
        app.update(enter);
        let approved = app
            .agents
            .pending_auth
            .as_ref()
            .expect("explicit approval only");
        assert_eq!(
            (approved.profile, approved.method),
            (
                Agent::Devin.index(),
                crate::providers::auth::Method::DevinWeb
            )
        );
        assert!(app.agents.pending.is_none() && app.planning.pending.is_none());
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
        assert_eq!(state.readiness(0), Readiness::Unknown);
        state.auth_returned(0, crate::providers::auth::Outcome::Authenticated);
        assert_eq!(state.readiness(0), Readiness::SignedIn);
        state.auth_returned(0, crate::providers::auth::Outcome::Unknown);
        assert_eq!(state.readiness(0), Readiness::Unknown);
        state.auth_returned(0, crate::providers::auth::Outcome::NotAuthenticated);
        press(&mut state, KeyCode::Esc);
        press(&mut state, KeyCode::Esc);
        assert!(state.at_root());
        assert!(state.pending.is_none());
        state.command("status", true);
        assert_eq!(state.readiness(0), Readiness::Checking);
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
    fn ready_selection_switches_explicit_route_and_requests_model_picker_together() {
        let mut state = discovered();
        state.active = 0;
        state.selected = 1;
        state.outcomes[1] = Some(crate::providers::auth::Outcome::Authenticated);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.use_for_task.take(), Some(1));
        assert_eq!(state.pick.take(), Some("model"));
        assert_eq!(state.active, 0);
        assert!(state.pending.is_none() && state.pending_auth.is_none());
        state.active = 1;
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.pick.take(), Some("model"));
        assert!(state.use_for_task.is_none());
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
    fn one_family_groups_ten_unchanged_route_identities_without_switching() {
        let mut state = discovered();
        state.profiles = Agent::ALL.iter().map(|_| profile()).collect();
        assert_eq!(Agent::ALL.len(), 10);
        for (index, agent) in Agent::ALL.iter().enumerate() {
            assert_eq!(agent.index(), index);
            assert_eq!(Agent::from_index(index), Some(*agent));
        }
        assert_eq!(state.provider_indices().count(), 7);
        assert_eq!(
            state
                .provider_indices()
                .filter(|index| openai_family(*index))
                .count(),
            1
        );
        state.active = Agent::ChatGPT.index();
        state.focus_active();
        assert_eq!(state.selected, Agent::ChatGPT.index());
        for c in "openai".chars() {
            press(&mut state, KeyCode::Char(c));
        }
        assert_eq!(
            state.provider_indices().collect::<Vec<_>>(),
            vec![Agent::ChatGPT.index()]
        );
        assert_eq!(state.selected, Agent::ChatGPT.index());
        assert_eq!(state.active, Agent::ChatGPT.index());
        assert!(state.pending.is_none() && state.use_for_task.is_none());
        press(&mut state, KeyCode::Esc);
        assert!(state.search.is_empty());
    }

    #[test]
    fn missing_cli_never_silently_selects_native_or_launches() {
        let mut state = discovered();
        state.profiles = Agent::ALL.iter().map(|_| profile()).collect();
        state.profiles[Agent::Codex.index()].executable = None;
        state.active = Agent::Codex.index();
        state.begin_connection(Agent::Codex);
        assert_eq!(state.selected, Agent::Codex.index());
        assert_eq!(state.readiness(Agent::Codex.index()), Readiness::MissingCli);
        assert!(state.use_for_task.is_none() && state.pending.is_none());
        assert_eq!(state.active, Agent::Codex.index());
    }

    fn private_editor(agent: Agent) -> State {
        let mut state = discovered();
        state.profiles = Agent::ALL.iter().map(|_| profile()).collect();
        state.selected = agent.index();
        state.connection = Some(ConnectionEditor {
            form: crate::providers::direct::config::Setup {
                credential: String::new(),
                base_url: "https://api.example/v1".into(),
                model: String::new(),
                effort: None,
                effort_model: None,
                account_id: String::new(),
                responses: false,
                keyless: false,
                persist: false,
                source_notice: None,
            },
            field: 0,
            cursor: 0,
            visible: false,
            advanced: false,
        });
        state
    }

    #[test]
    fn private_paste_and_shortcuts_cannot_enter_request_or_commands() {
        let mut state = private_editor(Agent::Compatible);
        assert!(state.connection_editing());
        assert!(state.paste_connection("private-sentinel\r\n"));
        assert_eq!(
            state.connection.as_ref().unwrap().form.credential,
            "private-sentinel"
        );
        assert!(!state.notice.contains("private-sentinel"));
        state.key(
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
            true,
        );
        assert_eq!(
            state.connection.as_ref().unwrap().form.credential,
            "private-sentinel"
        );
        assert!(!state.command("version", true));
        assert!(state.pending.is_none());
        state.connection.as_mut().unwrap().field = 6;
        assert!(state.paste_connection("must-not-enter-another-editor"));
        press(&mut state, KeyCode::Esc);
        assert!(!state.connection_editing());
        assert!(!state.paste_connection("not-owned"));
        assert!(state.take_connection_change().is_none());
    }

    #[test]
    fn private_setup_bounds_and_resize_cannot_save_or_change_source() {
        let mut state = private_editor(Agent::Compatible);
        assert!(state.paste_connection("first\nsecond"));
        assert!(
            state
                .connection
                .as_ref()
                .unwrap()
                .form
                .credential
                .is_empty()
        );
        state.connection.as_mut().unwrap().field = 1;
        assert!(state.paste_connection(&"x".repeat(2049)));
        assert_eq!(
            state.connection.as_ref().unwrap().form.base_url,
            "https://api.example/v1"
        );
        state.connection.as_mut().unwrap().field = 6;
        press(&mut state, KeyCode::Enter);
        assert!(state.connection_editing());
        state.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), false);
        assert!(state.take_connection_change().is_none());
        assert!(state.pending.is_none() && state.pending_auth.is_none());
    }

    #[test]
    fn native_browser_requires_review_without_installed_cli() {
        let mut state = discovered();
        state.profiles = Agent::ALL
            .iter()
            .map(|_| Profile {
                executable: None,
                ..profile()
            })
            .collect();
        state.begin_connection(Agent::ChatGPT);
        assert_eq!(state.page, Page::Account);
        assert!(!state.connection_editing());
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.selected, Agent::ChatGPT.index());
        assert!(state.use_for_task.is_none() && state.pending_auth.is_none());
        assert!(matches!(state.pending, Some(Request::PrepareAuth {
            profile, method: crate::providers::auth::Method::OpenAiBrowser, ..
        }) if profile == Agent::ChatGPT.index()));
        let intent = state.openai_intent.as_ref().unwrap();
        assert!(!intent.persist && !intent.approved);
        assert_eq!(state.openai_auth_generation(), None);
    }

    #[test]
    fn status_return_is_not_a_configuration_mutation() {
        let mut state = discovered();
        state.auth_returned(0, crate::providers::auth::Outcome::Authenticated);
        assert!(state.take_connection_change().is_none());
    }

    #[test]
    fn shallow_form_edits_protocol_keyless_storage_and_model_without_network() {
        let mut state = private_editor(Agent::Compatible);
        state.connection.as_mut().unwrap().field = 2;
        assert!(state.paste_connection("explicit-model"));
        for field in [3, 4, 5] {
            state.connection.as_mut().unwrap().field = field;
            press(&mut state, KeyCode::Enter);
        }
        let form = &state.connection.as_ref().unwrap().form;
        assert_eq!(form.model, "explicit-model");
        assert!(form.responses && form.keyless && form.persist);
        assert!(state.pending.is_none() && state.pending_auth.is_none());
        assert!(state.take_connection_change().is_none());
        assert!(state.use_for_task.is_none() && state.pick.is_none());
    }

    #[test]
    fn pending_task_save_preserves_private_edits_but_blocks_connection_mutation() {
        use crate::app::{App, Overlay, StorageState};
        use crossterm::event::Event as InputEvent;

        for (width, height) in [(60, 18), (120, 36)] {
            let mut app = App::new("/workspace".into(), width, height);
            app.storage = StorageState::Saving;
            app.revision = 2;
            app.saved_revision = 1;
            app.save_requested = true;
            app.overlay = Some(Overlay::Agents);
            app.input = "untouched request".into();
            app.cursor = 4;
            app.agents = private_editor(Agent::Compatible);
            app.update(InputEvent::Paste("private-sentinel".into()));
            for code in [KeyCode::Left, KeyCode::Right, KeyCode::Tab] {
                app.update(InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)));
            }
            app.update(InputEvent::Key(KeyEvent::new(
                KeyCode::Char('u'),
                KeyModifiers::CONTROL,
            )));
            app.update(InputEvent::Paste("https://private.example/v1".into()));
            app.update(InputEvent::Key(KeyEvent::new(
                KeyCode::Tab,
                KeyModifiers::NONE,
            )));
            app.update(InputEvent::Paste("explicit-model".into()));
            for _ in 0..4 {
                app.update(InputEvent::Key(KeyEvent::new(
                    KeyCode::Tab,
                    KeyModifiers::NONE,
                )));
            }
            let editor = app.agents.connection.as_mut().unwrap();
            assert_eq!(editor.field, 6);
            assert_eq!(editor.form.credential, "private-sentinel");
            assert_eq!(editor.form.base_url, "https://private.example/v1");
            assert_eq!(editor.form.model, "explicit-model");
            editor.visible = true;
            for key in [
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            ] {
                assert!(app.agents.connection_needs_idle(&key));
                app.update(InputEvent::Key(key));
                assert!(app.agents.connection_editing());
                assert!(app.agents.take_connection_change().is_none());
                assert!(app.agents.use_for_task.is_none() && app.agents.pick.is_none());
            }
            assert_eq!((&*app.input, app.cursor), ("untouched request", 4));
            assert!(app.agents.search.is_empty() && app.command_popup.is_none());
            assert!(app.agents.pending.is_none() && app.agents.pending_auth.is_none());
            app.update(InputEvent::Key(KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::NONE,
            )));
            assert!(!app.agents.connection_editing());
            assert!(app.agents.take_connection_change().is_none());
        }
    }

    #[test]
    fn private_idle_admission_classifies_only_deliberate_save_keys() {
        let mut state = private_editor(Agent::Compatible);
        for (field, _) in state.connection_fields().iter().enumerate() {
            state.connection.as_mut().unwrap().field = field;
            for kind in [
                KeyEventKind::Press,
                KeyEventKind::Repeat,
                KeyEventKind::Release,
            ] {
                for code in [
                    KeyCode::Enter,
                    KeyCode::Tab,
                    KeyCode::Esc,
                    KeyCode::Char('x'),
                ] {
                    let key = KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind);
                    assert_eq!(
                        state.connection_needs_idle(&key),
                        field == 6 && kind == KeyEventKind::Press && code == KeyCode::Enter
                    );
                }
                let save = KeyEvent::new_with_kind(KeyCode::Char('s'), KeyModifiers::CONTROL, kind);
                assert_eq!(
                    state.connection_needs_idle(&save),
                    kind == KeyEventKind::Press
                );
            }
        }
        assert!(!state.connection_needs_idle(&KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL | KeyModifiers::ALT
        )));
        press(&mut state, KeyCode::Esc);
        assert!(
            !state.connection_needs_idle(&KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        );
    }

    #[test]
    fn save_only_shortcut_still_requires_visible_scope_and_never_repeats() {
        let mut state = private_editor(Agent::Compatible);
        for kind in [KeyEventKind::Press, KeyEventKind::Repeat] {
            state.key(
                KeyEvent::new_with_kind(KeyCode::Char('s'), KeyModifiers::CONTROL, kind),
                true,
            );
        }
        assert!(state.connection_editing());
        assert!(state.use_for_task.is_none() && state.pick.is_none());
        assert!(state.take_connection_change().is_none());
    }

    #[test]
    fn every_connection_page_owns_paste_without_changing_request_or_caret() {
        use crate::app::{App, Overlay};
        use crossterm::event::Event as InputEvent;
        let mut app = App::new("/unused".into(), 60, 18);
        app.overlay = Some(Overlay::Agents);
        app.editing = true;
        app.input = "retained 한글 request".into();
        app.cursor = 4;
        app.agents = discovered();
        for page in [
            Page::Providers,
            Page::OpenAi,
            Page::Account,
            Page::Methods,
            Page::Actions,
            Page::Install,
            Page::Diagnostics,
        ] {
            app.agents.page = page;
            app.update(InputEvent::Paste("secret-must-not-reach-request".into()));
            assert_eq!((&*app.input, app.cursor), ("retained 한글 request", 4));
            assert!(app.agents.search.is_empty());
            assert!(app.agents.pending.is_none() && app.agents.use_for_task.is_none());
        }
    }

    #[test]
    fn private_back_returns_to_family_before_root_without_touching_request() {
        use crate::app::{App, Overlay};
        use crossterm::event::Event as InputEvent;
        let mut app = App::new("/unused".into(), 60, 18);
        app.overlay = Some(Overlay::Agents);
        app.input = "retained request".into();
        app.cursor = 4;
        app.agents = private_editor(Agent::Compatible);
        let back = InputEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.update(back.clone());
        assert_eq!(app.overlay, Some(Overlay::Agents));
        assert_eq!(app.agents.page, Page::OpenAi);
        app.update(back);
        assert_eq!(app.agents.page, Page::Providers);
        assert_eq!((&*app.input, app.cursor), ("retained request", 4));
    }

    fn approved_native_login(persist: bool) -> (State, crate::providers::auth::Plan, u64) {
        let mut state = discovered();
        state.profiles = Agent::ALL.iter().map(|_| profile()).collect();
        state.connection_owner = Some((11, 22, Agent::Codex));
        state.begin_connection(Agent::ChatGPT);
        state.login_persist = persist;
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Enter);
        let Some(Request::PrepareAuth {
            profile,
            generation,
            method,
        }) = state.pending.take()
        else {
            panic!("native preparation request");
        };
        let plan =
            crate::providers::auth::Plan::prepare(profile, method, &std::env::temp_dir()).unwrap();
        state.event(Event::AuthPrepared {
            profile,
            generation,
            result: Ok(Box::new(plan)),
        });
        assert_eq!(state.openai_auth_generation(), None);
        state.reviewed = true;
        press(&mut state, KeyCode::Tab);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.openai_auth_generation(), Some(generation));
        let plan = state.pending_auth.take().unwrap();
        (state, plan, generation)
    }

    #[test]
    fn native_completion_is_bound_to_generation_owner_method_and_storage() {
        let (mut state, plan, generation) = approved_native_login(true);
        state.login_persist = false;
        assert!(
            state.openai_login_persist(),
            "reviewed storage cannot be re-derived"
        );
        let owner = (11, 22, Agent::Codex);
        let credential = || {
            Some(crate::providers::auth::test_openai_credential(
                "private-native-sentinel",
                "account",
                u64::MAX,
            ))
        };
        state.finish_openai_login(
            &plan,
            Some(generation - 1),
            credential(),
            crate::providers::auth::Outcome::Authenticated,
            &owner,
            true,
        );
        assert_eq!(state.openai_auth_generation(), Some(generation));
        let wrong_method = crate::providers::auth::Plan::prepare(
            Agent::ChatGPT.index(),
            crate::providers::auth::Method::OpenAiBrowser,
            &std::env::temp_dir(),
        )
        .unwrap();
        state.finish_openai_login(
            &wrong_method,
            Some(generation),
            credential(),
            crate::providers::auth::Outcome::Authenticated,
            &owner,
            true,
        );
        assert_eq!(state.openai_auth_generation(), Some(generation));
        let other_owner = (33, 22, Agent::Codex);
        state.finish_openai_login(
            &plan,
            Some(generation),
            credential(),
            crate::providers::auth::Outcome::Authenticated,
            &other_owner,
            true,
        );
        assert_eq!(state.openai_auth_generation(), None);
        assert!(state.take_connection_change().is_none());
        assert!(state.use_for_task.is_none() && state.pick.is_none());
        assert!(!state.notice.contains("private-native-sentinel"));
    }

    #[test]
    fn native_cancel_failure_and_resize_never_substitute_or_replay() {
        let owner = (11, 22, Agent::Codex);
        let (mut state, plan, generation) = approved_native_login(false);
        state.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), false);
        assert_eq!(
            state.openai_auth_generation(),
            Some(generation),
            "post-auth terminal resize does not erase an already reviewed intent"
        );
        state.finish_openai_login(
            &plan,
            Some(generation),
            None,
            crate::providers::auth::Outcome::Failed,
            &owner,
            true,
        );
        assert_eq!(state.page, Page::Account);
        assert_eq!(state.family_choice, 1);
        assert_eq!(state.active, Agent::Codex.index());
        assert!(state.use_for_task.is_none() && state.pick.is_none());
        assert!(state.take_connection_change().is_none());
        let (mut cancelled, plan, generation) = approved_native_login(false);
        cancelled.cancel_preview();
        cancelled.finish_openai_login(
            &plan,
            Some(generation),
            None,
            crate::providers::auth::Outcome::Authenticated,
            &owner,
            true,
        );
        assert!(cancelled.take_connection_change().is_none());
        assert!(cancelled.use_for_task.is_none() && cancelled.pick.is_none());
    }

    #[test]
    fn cancelled_deferred_family_does_not_reopen_after_discovery() {
        let mut state = State::new();
        state.pending.take();
        state.begin_openai_family();
        assert!(state.deferred_family);
        state.cancel_preview();
        state.event(Event::Discovered(
            Agent::ALL.iter().map(|_| profile()).collect(),
        ));
        assert_eq!(state.page, Page::Providers);
        assert!(!state.connection_editing());
        assert!(state.use_for_task.is_none() && state.pending_auth.is_none());
    }

    #[test]
    fn family_api_and_custom_choices_preserve_distinct_explicit_routes() {
        let mut state = discovered();
        state.profiles = Agent::ALL.iter().map(|_| profile()).collect();
        state.page = Page::OpenAi;
        state.family_choice = 0;
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.page, Page::Account);
        assert_eq!(state.selected, Agent::ChatGPT.index());
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.page, Page::OpenAi);
        assert!(state.use_for_task.is_none() && state.pending.is_none());
        // The field contract keeps advanced API overrides reachable, while
        // custom protocol/keyless decisions remain primary and explicit.
        let mut api = private_editor(Agent::OpenAI);
        assert_eq!(
            api.connection_fields(),
            &[
                ConnectionField::Credential,
                ConnectionField::Storage,
                ConnectionField::Save,
                ConnectionField::Advanced,
            ]
        );
        api.connection.as_mut().unwrap().field = 3;
        press(&mut api, KeyCode::Enter);
        assert!(api.connection_fields().contains(&ConnectionField::Endpoint));
        assert!(api.connection_fields().contains(&ConnectionField::Model));
        let custom = private_editor(Agent::Compatible);
        assert!(
            custom
                .connection_fields()
                .contains(&ConnectionField::Protocol)
        );
        assert!(
            custom
                .connection_fields()
                .contains(&ConnectionField::Keyless)
        );
        assert_eq!(api.active, Agent::Codex.index());
        assert!(api.use_for_task.is_none() && api.pending.is_none());
    }

    #[test]
    fn diagnostics_remediation_keeps_observation_without_dispatch_or_authentication_claim() {
        use crate::providers::diagnostics::Failure;
        let mut state = discovered();
        state.selected = Agent::OpenCode.index();
        for failure in [
            Failure::Http(401),
            Failure::Http(403),
            Failure::Transport,
            Failure::Incompatible,
            Failure::VersionCheck,
            Failure::LoginCancelled,
        ] {
            state.connection_failed(Agent::OpenCode, failure);
            assert_eq!(state.readiness(Agent::OpenCode.index()), Readiness::Blocked);
            assert!(state.show_failure(Agent::OpenCode));
            assert_eq!(state.page, Page::Diagnostics);
            press(&mut state, KeyCode::Char('n'));
            assert_eq!(state.failure(Agent::OpenCode.index()), Some(failure));
            assert!(!matches!(
                state.readiness(Agent::OpenCode.index()),
                Readiness::SignedIn
            ));
            assert!(state.pending_auth.is_none() && state.use_for_task.is_none());
            // Login preparation is only a local review; no launch/grant exists.
            if let Some(pending) = state.pending.take() {
                assert!(matches!(pending, Request::PrepareAuth { .. }));
            }
            state.cancel_preview();
            state.selected = Agent::OpenCode.index();
        }
    }

    #[test]
    fn discovery_and_version_success_never_erase_real_auth_refusal() {
        use crate::providers::diagnostics::Failure;
        let mut state = discovered();
        state.connection_failed(Agent::Codex, Failure::Http(401));
        state.discovering = true;
        state.event(Event::Discovered(vec![profile(); 5]));
        assert_eq!(state.failure(0), Some(Failure::Http(401)));
        assert_ne!(state.readiness(0), Readiness::SignedIn);
        state.inflight = Some((0, 11));
        let mut failed_probe = profile();
        failed_probe.version = "Probe failed: unsupported output withheld".into();
        state.event(Event::Probed {
            profile: 0,
            generation: 11,
            result: Box::new(failed_probe),
        });
        assert_eq!(state.failure(0), Some(Failure::Http(401)));
        state.inflight = Some((0, 12));
        state.event(Event::Probed {
            profile: 0,
            generation: 12,
            result: Box::new(profile()),
        });
        assert_eq!(state.failure(0), Some(Failure::Http(401)));
        assert_eq!(state.readiness(0), Readiness::Blocked);
    }

    #[test]
    fn cancelled_login_reports_cancel_and_neutral_navigation_never_launches() {
        use crate::providers::{auth::Outcome, diagnostics::Failure};
        let mut state = discovered();
        state.auth_returned(0, Outcome::Cancelled);
        assert_eq!(state.failure(0), Some(Failure::LoginCancelled));
        assert_eq!(state.page, Page::Actions);
        assert!(state.show_failure(Agent::Codex));
        for key in [
            KeyCode::Down,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::Tab,
            KeyCode::Esc,
        ] {
            press(&mut state, key);
            assert!(state.pending.is_none() && state.pending_auth.is_none());
            assert!(state.use_for_task.is_none() && state.pick.is_none());
        }
        state.preview = true;
        let selected = state.selected;
        assert!(!state.show_failure(Agent::OpenCode));
        assert_eq!(state.selected, selected);
    }

    #[test]
    fn prerequisite_failure_is_inspectable_redacted_and_never_launches_login() {
        let mut state = discovered();
        state.selected = Agent::OpenCode.index();
        state.auth_preparing = Some((state.selected, 21));
        state.event(Event::AuthPrepared {
            profile: state.selected,
            generation: 21,
            result: Err(
                "opencode missing; token=private-preflight-sentinel; refresh PATH\u{001b}[31m"
                    .into(),
            ),
        });
        assert_eq!(state.page, Page::Diagnostics);
        assert_eq!(
            state.failure(state.selected),
            Some(crate::providers::diagnostics::Failure::LoginRefused)
        );
        let detail = state.preparation_detail(state.selected).unwrap();
        assert!(!detail.contains("private-preflight-sentinel"));
        assert!(!detail.chars().any(char::is_control));
        assert!(state.pending.is_none() && state.pending_auth.is_none());
    }
}
