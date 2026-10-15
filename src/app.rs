pub mod agents;
pub mod commands;
pub mod composer;
pub mod execution;
pub mod handoff;
pub mod memory;
pub mod party;
pub mod planning;
pub mod sources;
pub mod workflows;

mod editing;
mod events;
mod keys;
mod overlays;
mod tasks;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;

use crate::domain::FORMS;
use crate::store;
use crate::workspace::repo;

pub const MAX_GOAL_BYTES: usize = 4096;
pub const DELETE_PREVIEW_PAGE: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Companion,
    Workspace,
    Inspector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    Forms,
    Help,
    Inspector,
    Companion,
    Preview,
    Tasks,
    Criteria,
    Repository,
    Sources,
    Execution,
    Agents,
    Planning,
    Handoff,
    Images,
    Workflows,
    Party,
    Memory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmationKind {
    DiscardDraft,
    Quit,
    Complete,
    Reopen(store::EditorField),
    DeleteTasks,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Confirmation {
    pub kind: ConfirmationKind,
    /// False is the neutral Keep/Stay choice; only navigation selects destruction.
    pub destructive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollSurface {
    Companion,
    Workspace,
    Inspector,
    Help,
    Preview,
}

impl ScrollSurface {
    fn index(self) -> usize {
        match self {
            Self::Companion => 0,
            Self::Workspace => 1,
            Self::Inspector => 2,
            Self::Help => 3,
            Self::Preview => 4,
        }
    }
}

#[derive(Clone, Copy)]
struct OverlayFrame {
    overlay: Overlay,
    selected: usize,
    focus: Pane,
    preview_form: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum StorageState {
    Loading,
    Saved,
    Saving,
    Failed(String),
}

pub struct App {
    /// The active task's lead partner; its form is the active form.
    lead: store::Partner,
    /// The committed party roster; `lead` is the live copy of the task's
    /// lead partner, so `App::partners()` overlays it onto these rows.
    party: Vec<store::Partner>,
    /// Queued party mutation for the store worker (`/party` Add).
    pub party_request: Option<store::PartyRequest>,
    /// A party mutation was handed to the store and awaits its committed roster.
    pub party_inflight: bool,
    pub selected: usize,
    /// Preferred focus, retained even when its pane is temporarily hidden.
    pub focus: Pane,
    pub overlay: Option<Overlay>,
    pub goal: String,
    pub input: String,
    /// UTF-8 byte offset at a grapheme boundary in the local goal draft.
    pub cursor: usize,
    pub input_truncated: bool,
    pub confirmation: Option<Confirmation>,
    pub editing: bool,
    pub composer_menu: Option<composer::Menu>,
    /// Full explanations instead of first-sentence summaries (`/verbose`).
    pub verbose: bool,
    pub command_popup: Option<commands::Popup>,
    composer_catalogue: bool,
    composer_return: Option<store::EditorField>,
    pub images_pending: Option<crate::ui::artwork::Request>,
    pub images_busy: bool,
    pub images_cancel: bool,
    pub images_selected: usize,
    pub images_notice: String,
    pub images_enabled: bool,
    pub images_reviewed: bool,
    pub linear: bool,
    pub cwd: String,
    pub width: u16,
    pub height: u16,
    pub quit: bool,
    pub storage: StorageState,
    pub revision: u64,
    pub saved_revision: u64,
    pub save_requested: bool,
    pub retry_storage: bool,
    pub quit_after_save: bool,
    pub task_id: i64,
    pub tasks: Vec<store::Snapshot>,
    pub criteria: Vec<store::Criterion>,
    pub next_action: String,
    pub complete: bool,
    pub updated_at: i64,
    pub editor_field: store::EditorField,
    pub drafts: Vec<store::Draft>,
    pub task_selection: usize,
    pub criterion_selection: usize,
    pub notice: String,
    pub transitioning: bool,
    pub switch_task: Option<i64>,
    pub policy: store::Policy,
    pub usage_bytes: u64,
    pub archived: bool,
    pub archived_at: i64,
    pub redacted: bool,
    pub show_archived: bool,
    pub management: Option<store::Management>,
    pub delete_targets: Vec<i64>,
    pub delete_expired_before: Option<i64>,
    pub repository: Option<std::sync::Arc<repo::Report>>,
    pub repo_text: String,
    pub repo_status: String,
    pub repo_error: String,
    pub repo_loading: bool,
    pub repo_refresh: bool,
    pub sources: sources::State,
    pub execution: execution::State,
    pub agents: agents::State,
    pub planning: planning::State,
    pub handoff: handoff::State,
    pub workflows: crate::workspace::workflows::State,
    pub workflow: workflows::State,
    pub memory: memory::State,
    pub memory_request: Option<store::MemoryRequest>,
    pub memory_loading: bool,
    pub command: String,
    pub delete_preview: Vec<store::DeletionTarget>,
    pub delete_page: usize,
    deletion_preview_pending: bool,
    pending_command: Option<String>,
    edit_original: String,
    form_selection: usize,
    overlay_history: Vec<OverlayFrame>,
    edit_return: Option<OverlayFrame>,
    preview_form: usize,
    /// Form the lead partner was in before the latest switch (PT03B);
    /// persisted in the task's navigation JSON.
    previous_form: Option<usize>,
    preferred_column: Option<usize>,
    // Companion, workspace, inspector, help, and local preview respectively.
    surface_scroll: [u16; 5],
    surface_scroll_max: [u16; 5],
}

impl App {
    pub fn new(cwd: String, width: u16, height: u16) -> Self {
        Self {
            lead: store::Partner::default(),
            party: Vec::new(),
            party_request: None,
            party_inflight: false,
            selected: 0,
            focus: Pane::Workspace,
            overlay: None,
            goal: String::new(),
            input: String::new(),
            cursor: 0,
            input_truncated: false,
            confirmation: None,
            editing: true,
            composer_menu: None,
            verbose: false,
            command_popup: None,
            composer_catalogue: false,
            composer_return: None,
            images_pending: None,
            images_busy: false,
            images_cancel: false,
            images_selected: 0,
            images_notice: String::new(),
            images_enabled: true,
            images_reviewed: false,
            linear: false,
            cwd,
            width,
            height,
            quit: false,
            storage: StorageState::Loading,
            revision: 0,
            saved_revision: 0,
            save_requested: false,
            retry_storage: false,
            quit_after_save: false,
            task_id: 0,
            tasks: Vec::new(),
            criteria: Vec::new(),
            next_action: String::new(),
            complete: false,
            updated_at: 0,
            editor_field: store::EditorField::Goal,
            drafts: Vec::new(),
            task_selection: 0,
            criterion_selection: 0,
            notice: String::new(),
            transitioning: false,
            switch_task: None,
            policy: store::Policy::default(),
            usage_bytes: 0,
            archived: false,
            archived_at: 0,
            redacted: false,
            show_archived: false,
            management: None,
            delete_targets: Vec::new(),
            delete_expired_before: None,
            repository: None,
            repo_text: String::new(),
            repo_status: "Repository: loading baseline".into(),
            repo_error: String::new(),
            repo_loading: true,
            repo_refresh: false,
            sources: sources::State::default(),
            execution: execution::State::default(),
            agents: agents::State::new(),
            planning: planning::State::new(),
            handoff: handoff::State::new(),
            workflows: crate::workspace::workflows::State::default(),
            workflow: workflows::State::default(),
            memory: memory::State::default(),
            memory_request: None,
            memory_loading: false,
            command: String::new(),
            delete_preview: Vec::new(),
            delete_page: 0,
            deletion_preview_pending: false,
            pending_command: None,
            edit_original: String::new(),
            form_selection: 0,
            overlay_history: Vec::new(),
            edit_return: None,
            preview_form: 0,
            previous_form: None,
            preferred_column: None,
            surface_scroll: [0; 5],
            surface_scroll_max: [0; 5],
        }
    }

    pub fn storage_label(&self) -> &str {
        match &self.storage {
            StorageState::Loading => "Storage: loading",
            StorageState::Saved if self.draft_dirty() || !self.drafts.is_empty() => {
                "Storage: saved / retained draft"
            }
            StorageState::Saved => "Storage: saved",
            StorageState::Saving if self.quit_after_save => {
                "Storage: saving before exit / Esc stay"
            }
            StorageState::Saving => "Storage: saving",
            StorageState::Failed(_) => "Storage: save failed / /storage-retry / /inspect",
        }
    }

    pub fn effective_focus(&self) -> Pane {
        if self.linear
            || self.width < 90
            || self.height < 18
            || (self.width < 120 && self.focus == Pane::Inspector)
        {
            Pane::Workspace
        } else {
            self.focus
        }
    }

    pub fn draft_dirty(&self) -> bool {
        self.editing && self.input != self.edit_original
    }

    /// The picker preview is independent of the committed role.
    pub fn preview_form(&self) -> usize {
        if self.overlay == Some(Overlay::Preview) {
            self.preview_form
        } else {
            self.form()
        }
    }

    /// The active form is the lead partner's form; persisted per task as
    /// `Snapshot.form` / `tasks.lead_partner`.
    pub fn form(&self) -> usize {
        self.lead.form
    }

    /// The lead partner record loaded from the store.
    pub fn lead(&self) -> &store::Partner {
        &self.lead
    }

    /// Header identity: nickname when set (PK01), else `Eevee #<id>`.
    pub fn lead_label(&self) -> String {
        format!(
            "{} · {}",
            Self::partner_label(&self.lead),
            crate::domain::FORMS[self.form().min(8)].name
        )
    }

    /// The only writer of the active form. Callers invoke `changed_task()`
    /// when the switch should persist. Every change records the prior form so
    /// Backspace / `/back-form` can return to it (PT03B), and the picker's
    /// selection follows so it opens on the form that is actually active.
    pub(super) fn set_form(&mut self, form: usize) {
        if self.lead.form != form {
            self.previous_form = Some(self.lead.form);
            self.form_selection = form;
        }
        self.lead.form = form;
        if let Some(partner) = self
            .party
            .iter_mut()
            .find(|partner| partner.id == self.lead.id)
        {
            partner.form = form;
        }
    }

    /// Flip the lead partner back to the form it left (Backspace / `/back-form`).
    /// The toggle routes through the same persisted navigation path as the
    /// picker; no previous form leaves a notice instead of switching.
    pub(super) fn previous_form_toggle(&mut self) {
        match self.previous_form {
            Some(previous) if previous != self.form() => {
                self.request_navigation(execution::Action::SelectForm(previous));
            }
            _ => self.notice = "No previous form yet; switch once before toggling back.".into(),
        }
    }

    /// Saved line offset for the currently visible, non-selecting surface.
    pub fn scroll(&self) -> u16 {
        self.scroll_for(self.scroll_surface())
    }

    pub fn scroll_for(&self, surface: ScrollSurface) -> u16 {
        self.surface_scroll[surface.index()]
    }

    /// Supply the last useful rendered line offset, not the total line count.
    /// Called by layout after wrapping/viewport measurement, with no I/O here.
    pub fn set_scroll_max(&mut self, surface: ScrollSurface, maximum: u16) {
        let index = surface.index();
        self.surface_scroll_max[index] = maximum;
        self.surface_scroll[index] = self.surface_scroll[index].min(maximum);
    }
}

fn safe_character(character: char) -> bool {
    (character == '\n' || !character.is_control())
        && !matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

#[cfg(test)]
mod tests;
