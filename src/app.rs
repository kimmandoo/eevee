pub mod execution;
pub mod sources;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;

use crate::domain::FORMS;
use crate::repo;
use crate::store;

pub const MAX_GOAL_BYTES: usize = 4096;
pub const DELETE_PREVIEW_PAGE: usize = 256;

pub const PALETTE: [&str; 16] = [
    "Choose evolution",
    "Edit task goal",
    "Inspect task and storage",
    "Show companion",
    "Keyboard help",
    "Preview local handoff",
    "Toggle linear layout",
    "Quit Eevee",
    "Task cards",
    "New task",
    "Acceptance criteria",
    "Next action",
    "Complete task",
    "Repository baseline and diff",
    "Source map and context basket",
    "Command execution and evidence",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Companion,
    Workspace,
    Inspector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    Forms,
    Palette,
    Help,
    Inspector,
    Companion,
    Preview,
    Tasks,
    Criteria,
    Repository,
    Sources,
    Execution,
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
    pub form: usize,
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
    pub command: String,
    pub delete_preview: Vec<store::DeletionTarget>,
    pub delete_page: usize,
    deletion_preview_pending: bool,
    pending_command: Option<String>,
    edit_original: String,
    form_selection: usize,
    palette_selection: usize,
    overlay_history: Vec<OverlayFrame>,
    edit_return: Option<OverlayFrame>,
    preview_form: usize,
    preferred_column: Option<usize>,
    // Companion, workspace, inspector, help, and local preview respectively.
    surface_scroll: [u16; 5],
    surface_scroll_max: [u16; 5],
}

impl App {
    pub fn new(cwd: String, width: u16, height: u16) -> Self {
        Self {
            form: 0,
            selected: 0,
            focus: Pane::Workspace,
            overlay: None,
            goal: String::new(),
            input: String::new(),
            cursor: 0,
            input_truncated: false,
            confirmation: None,
            editing: false,
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
            command: String::new(),
            delete_preview: Vec::new(),
            delete_page: 0,
            deletion_preview_pending: false,
            pending_command: None,
            edit_original: String::new(),
            form_selection: 0,
            palette_selection: 0,
            overlay_history: Vec::new(),
            edit_return: None,
            preview_form: 0,
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
            StorageState::Failed(_) => "Storage: save failed / r retry / i details",
        }
    }

    pub fn repo_event(&mut self, event: repo::Event) {
        use std::fmt::Write;
        self.repo_loading = false;
        match event {
            repo::Event::Updated(report) => {
                self.repo_status = if report.initial {
                    "Repository: baseline captured"
                } else {
                    "Repository: refreshed / original baseline retained"
                }
                .into();
                self.repo_error.clear();
                let mut text = format!(
                    "ROOT\n{:?}\nGit metadata: {:?}\n\nBASELINE / CURRENT\nHEAD: {} / {}\nFiles: {} / {}\nCurrent coverage complete: {} / stable observation: {}\nIgnored untracked files and submodule contents are excluded.\n\nCHANGES\n",
                    report.repository.root,
                    report.repository.git_dir,
                    report
                        .baseline
                        .head
                        .as_deref()
                        .unwrap_or("unborn/unavailable"),
                    report
                        .current
                        .head
                        .as_deref()
                        .unwrap_or("unborn/unavailable"),
                    report.baseline.files.len(),
                    report.current.files.len(),
                    report.current.complete,
                    report.current.stable
                );
                if report.changes.is_empty() {
                    text.push_str("No observed changes in captured scope; incomplete coverage is not a clean verdict.\n");
                }
                for change in &report.changes {
                    if text.len() >= 64 * 1024 {
                        text.push_str("Change list display truncated; no full-coverage claim.\n");
                        break;
                    }
                    let origin = match change.origin {
                        repo::ChangeOrigin::PreExisting => "pre-existing",
                        repo::ChangeOrigin::Unattributed => "unattributed",
                        repo::ChangeOrigin::Unknown => "unknown",
                    };
                    let _ = writeln!(
                        text,
                        "{origin} / {:?} / initially dirty: {} / {:?}",
                        change.kind, change.was_dirty, change.path
                    );
                }
                text.push_str("\nREAD-ONLY RAW HEAD → WORKTREE DIFF\nNo filters/textconv; staged identities are tracked separately. No authorship is inferred.\n");
                if report.current.working_diff.truncated {
                    text.push_str("TRUNCATED: only the bounded captured diff follows.\n");
                }
                if let Some(error) = &report.current.working_diff.unavailable {
                    let _ = writeln!(text, "Diff unavailable/partial: {error}");
                }
                text.push_str(&report.current.working_diff.text);
                text.push_str("\nOBSERVATION WARNINGS\n");
                for warning in report.current.warnings.iter().take(16) {
                    let _ = writeln!(text, "{warning}");
                }
                text.retain(safe_character);
                self.repo_text = text;
                self.repository = Some(report);
            }
            repo::Event::Unsupported { path, reason } => {
                self.repo_status = "Repository: unsupported".into();
                self.repo_error = format!("{path:?}: {reason}. Task data is retained; r retries.");
                self.repo_error.retain(safe_character);
            }
            repo::Event::Failed { operation, message } => {
                self.repo_status = "Repository: inspection failed".into();
                self.repo_error = format!(
                    "{operation}: {message}. Previous evidence is retained; r retries the read-only inspection."
                );
                self.repo_error.retain(safe_character);
            }
        }
    }

    pub fn store_event(&mut self, event: store::Event) {
        match event {
            store::Event::Loaded(project) => {
                let local_changes = self.revision != self.saved_revision;
                self.usage_bytes = project.usage_bytes;
                self.tasks = project.tasks;
                let target = if self.task_id > 0 {
                    Some(self.task_id)
                } else {
                    project.active_id
                };
                if let Some(snapshot) = self
                    .tasks
                    .iter()
                    .find(|task| Some(task.id) == target)
                    .cloned()
                {
                    if !local_changes {
                        self.load_task(snapshot);
                    } else if local_changes && self.task_id == snapshot.id {
                        self.saved_revision = snapshot.revision;
                        self.revision = self.revision.max(snapshot.revision.saturating_add(1));
                    }
                }
                self.save_requested = self.revision != self.saved_revision;
                self.storage = if self.save_requested {
                    StorageState::Saving
                } else {
                    StorageState::Saved
                };
            }
            store::Event::Selected(snapshot) => {
                self.load_task(snapshot);
                self.transitioning = false;
                self.notice.clear();
            }
            store::Event::Saved(snapshot) => {
                if self.task_id == 0 {
                    self.task_id = snapshot.id;
                }
                if self.task_id == snapshot.id {
                    self.updated_at = snapshot.updated_at;
                    self.saved_revision = self.saved_revision.max(snapshot.revision);
                    if snapshot.revision == self.revision {
                        self.goal.clone_from(&snapshot.goal);
                        self.criteria.clone_from(&snapshot.criteria);
                        self.next_action.clone_from(&snapshot.next_action);
                        self.command.clone_from(&snapshot.command);
                        self.redacted = snapshot.redacted;
                        if snapshot.redacted {
                            self.notice = "Identifiable secrets were masked in local storage. Detection is incomplete; review exports before sharing.".into();
                        }
                        self.drafts.clone_from(&snapshot.drafts);
                        self.storage = StorageState::Saved;
                        if let Some(command) = self.pending_command.take() {
                            self.execution.prepare_command(self.task_id, &command);
                        }
                    }
                }
                if let Some(previous) = self.tasks.iter_mut().find(|task| task.id == snapshot.id) {
                    *previous = snapshot;
                } else {
                    self.tasks.push(snapshot);
                }
            }
            store::Event::Managed {
                project,
                affected,
                warning,
            } => {
                self.transitioning = false;
                self.usage_bytes = project.usage_bytes;
                let current_removed = self.task_id > 0
                    && affected.contains(&self.task_id)
                    && !project
                        .tasks
                        .iter()
                        .any(|task| task.id == self.task_id && !task.archived);
                self.tasks = project.tasks;
                if current_removed {
                    let next = self
                        .tasks
                        .iter()
                        .find(|task| Some(task.id) == project.active_id && !task.archived)
                        .cloned()
                        .unwrap_or_else(|| store::Snapshot::empty(self.cwd.clone()));
                    self.load_task(next);
                    self.open_overlay(Overlay::Tasks);
                }
                self.task_selection = self
                    .task_selection
                    .min(self.visible_tasks().count().saturating_sub(1));
                self.delete_targets.clear();
                self.delete_preview.clear();
                self.notice = format!(
                    "Updated {} selected local task records. Repository files and exports were not changed.",
                    affected.len()
                );
                if let Some(warning) = warning {
                    self.notice
                        .push_str(&format!(" Cleanup warning: {warning}"));
                }
            }
            store::Event::ManagementFailed(message) => {
                self.transitioning = false;
                self.deletion_preview_pending = false;
                self.notice = format!("Task operation failed: {message}. No automatic retry.");
            }
            store::Event::DeletePreview {
                targets,
                expired_before,
            } => {
                if self.deletion_preview_pending
                    && targets
                        .iter()
                        .map(|target| target.task.id)
                        .eq(self.delete_targets.iter().copied())
                {
                    self.deletion_preview_pending = false;
                    self.transitioning = false;
                    self.delete_preview = targets;
                    self.delete_page = 0;
                    self.delete_expired_before = expired_before;
                    self.surface_scroll[ScrollSurface::Preview.index()] = 0;
                    self.confirmation = Some(Confirmation {
                        kind: ConfirmationKind::DeleteTasks,
                        destructive: false,
                    });
                }
            }
            store::Event::Context { task_id, result } => {
                self.sources.stored(self.task_id, task_id, result);
            }
            store::Event::Run {
                task_id,
                request_id,
                run_id,
                result,
            } => {
                self.execution.store_event(
                    task_id,
                    request_id.as_deref(),
                    run_id.as_deref(),
                    result.map(|response| *response),
                );
            }
            store::Event::Failed { revision, message } => {
                self.transitioning = false;
                if revision.is_none_or(|revision| revision == self.revision) {
                    self.storage = StorageState::Failed(message);
                    self.quit_after_save = false;
                }
            }
        }
        self.settle_exit();
    }

    pub fn editor_label(&self) -> &str {
        match self.editor_field {
            store::EditorField::Goal => "Goal",
            store::EditorField::Criteria => "Acceptance criteria",
            store::EditorField::NextAction => "Next action",
            store::EditorField::Command => "Command / explicit JSON argv or shell",
        }
    }

    fn applied_text(&self, field: store::EditorField) -> String {
        match field {
            store::EditorField::Goal => self.goal.clone(),
            store::EditorField::Criteria => self
                .criteria
                .iter()
                .map(|criterion| criterion.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            store::EditorField::NextAction => self.next_action.clone(),
            store::EditorField::Command => self.command.clone(),
        }
    }

    fn navigation(&self) -> store::Navigation {
        store::Navigation {
            focus: match self.focus {
                Pane::Companion => 0,
                Pane::Workspace => 1,
                Pane::Inspector => 2,
            },
            linear: self.linear,
            form_selection: self.form_selection,
            palette_selection: self.palette_selection,
            criterion_selection: self.criterion_selection,
            scroll: self.surface_scroll,
            overlay: match self.overlay {
                Some(Overlay::Forms) => Some(0),
                Some(Overlay::Palette) => Some(1),
                Some(Overlay::Help) => Some(2),
                Some(Overlay::Inspector) => Some(3),
                Some(Overlay::Companion) => Some(4),
                Some(Overlay::Preview) => Some(5),
                _ => None,
            },
            editing: self.editing,
            editor_field: self.editor_field,
        }
    }

    pub fn snapshot(&self, project: &str) -> store::Snapshot {
        let mut drafts = self.drafts.clone();
        if self.editing {
            drafts.retain(|draft| draft.field != self.editor_field);
            if self.draft_dirty() {
                drafts.push(store::Draft {
                    field: self.editor_field,
                    text: self.input.clone(),
                    cursor: self.cursor,
                });
            }
        }
        store::Snapshot {
            project: project.to_owned(),
            goal: self.goal.clone(),
            form: self.form,
            revision: self.revision,
            id: self.task_id,
            criteria: self.criteria.clone(),
            next_action: self.next_action.clone(),
            command: self.command.clone(),
            complete: self.complete,
            drafts,
            navigation: self.navigation(),
            updated_at: self.updated_at,
            archived: self.archived,
            archived_at: self.archived_at,
            redacted: self.redacted,
        }
    }

    fn load_task(&mut self, snapshot: store::Snapshot) {
        self.task_id = snapshot.id;
        self.pending_command = None;
        self.goal = snapshot.goal;
        self.form = snapshot.form;
        self.preview_form = self.form;
        self.revision = snapshot.revision;
        self.saved_revision = snapshot.revision;
        self.criteria = snapshot.criteria;
        self.next_action = snapshot.next_action;
        self.command = snapshot.command;
        self.complete = snapshot.complete;
        self.updated_at = snapshot.updated_at;
        self.archived = snapshot.archived;
        self.archived_at = snapshot.archived_at;
        self.redacted = snapshot.redacted;
        self.drafts = snapshot.drafts;
        self.input.clear();
        self.edit_original.clear();
        self.confirmation = None;
        self.overlay_history.clear();
        self.edit_return = None;
        self.editing = false;
        self.cursor = 0;
        let navigation = snapshot.navigation;
        self.focus = match navigation.focus {
            0 => Pane::Companion,
            2 => Pane::Inspector,
            _ => Pane::Workspace,
        };
        self.linear = navigation.linear;
        self.form_selection = navigation.form_selection;
        self.palette_selection = navigation.palette_selection.min(PALETTE.len() - 1);
        self.surface_scroll = navigation.scroll;
        self.surface_scroll_max = [u16::MAX; 5];
        self.overlay = match navigation.overlay {
            Some(0) => Some(Overlay::Forms),
            Some(1) => Some(Overlay::Palette),
            Some(2) => Some(Overlay::Help),
            Some(3) => Some(Overlay::Inspector),
            Some(4) => Some(Overlay::Companion),
            Some(5) => Some(Overlay::Preview),
            _ => None,
        };
        self.restore_selection();
        self.editor_field = navigation.editor_field;
        if navigation.editing {
            self.open_editor(navigation.editor_field);
        }
        self.storage = StorageState::Saved;
        self.show_archived = false;
        self.save_requested = false;
        let selection = self
            .visible_tasks()
            .position(|task| task.id == self.task_id)
            .unwrap_or(0);
        self.task_selection = selection;
        self.criterion_selection = navigation
            .criterion_selection
            .min(self.criteria.len().saturating_sub(1));
    }

    fn new_task(&mut self) {
        self.request_navigation(execution::Action::NewTask);
    }

    fn finish_new_task(&mut self) {
        if !matches!(self.storage, StorageState::Saved)
            || self.save_requested
            || self.sources.persistence_pending()
            || self.sources.store_failed
        {
            self.notice = "Finish or retry the current save before creating another task.".into();
            return;
        }
        let mut task = store::Snapshot::empty(self.cwd.clone());
        task.navigation.linear = self.linear;
        self.load_task(task);
        self.overlay = None;
        self.notice.clear();
        self.open_editor(store::EditorField::Goal);
    }

    fn select_task(&mut self) {
        if self.show_archived {
            self.notice = "Restore this archived task before resuming it.".into();
            return;
        }
        let selected = self
            .visible_tasks()
            .nth(self.task_selection)
            .map(|task| task.id);
        if let Some(id) = selected {
            self.request_navigation(execution::Action::SwitchTask(id));
        }
    }

    fn request_navigation(&mut self, action: execution::Action) {
        if let Some(action) = self.execution.request_navigation(action) {
            self.apply_execution_action(action);
        } else if self.execution.visible() {
            self.open_overlay(Overlay::Execution);
        }
    }

    pub fn apply_execution_action(&mut self, action: execution::Action) {
        match action {
            execution::Action::NewTask => self.finish_new_task(),
            execution::Action::Quit => self.finish_request_quit(),
            execution::Action::SelectForm(index) => {
                if self.form != index {
                    self.form = index;
                    self.changed_task();
                }
                if self.overlay == Some(Overlay::Execution) {
                    self.close_overlay();
                }
                if self.overlay == Some(Overlay::Forms) {
                    self.close_overlay();
                }
            }
            execution::Action::EditCommand => {
                self.execution.invalidate();
                self.pending_command = None;
                self.begin_edit_field(store::EditorField::Command);
            }
            execution::Action::SwitchTask(id) => {
                if !matches!(self.storage, StorageState::Saved)
                    || self.save_requested
                    || self.sources.persistence_pending()
                    || self.sources.store_failed
                {
                    self.notice = "Finish or retry task/context saves before switching; the current task was retained.".into();
                } else {
                    self.switch_task = Some(id);
                    self.transitioning = true;
                    self.storage = StorageState::Loading;
                }
            }
        }
    }

    pub fn settle_exit(&mut self) {
        if self.quit_after_save
            && matches!(self.storage, StorageState::Saved)
            && !self.sources.persistence_pending()
            && !self.execution.busy()
        {
            if self.sources.store_failed {
                self.quit_after_save = false;
                self.confirmation = Some(Confirmation {
                    kind: ConfirmationKind::Quit,
                    destructive: false,
                });
            } else {
                self.quit = true;
            }
        }
    }

    fn source_key(&mut self, key: KeyEvent) -> bool {
        match self
            .sources
            .key(key, self.task_id, self.repository.as_ref())
        {
            sources::Action::Close => self.close_overlay(),
            sources::Action::Quit => self.request_quit(),
            sources::Action::RefreshRepository => {
                self.repo_refresh = true;
                self.repo_loading = true;
            }
            sources::Action::Scroll(code) => return self.scroll_key(code),
            sources::Action::Changed => self.surface_scroll[ScrollSurface::Preview.index()] = 0,
        }
        true
    }

    fn execution_key(&mut self, key: KeyEvent) -> bool {
        if (self.width < 60 || self.height < 18)
            && key.code == KeyCode::Enter
            && matches!(
                self.execution.confirmation,
                Some(execution::Confirmation::Approval)
            )
        {
            return false;
        }
        if key.modifiers == KeyModifiers::CONTROL
            && key.code == KeyCode::Char('k')
            && !self.execution.searching
            && self.execution.confirmation.is_none()
        {
            self.open_overlay(Overlay::Palette);
            return true;
        }
        if let Some(action) = self.execution.key(key) {
            self.apply_execution_action(action);
        }
        if !self.execution.visible() && self.overlay == Some(Overlay::Execution) {
            self.close_overlay();
        }
        true
    }

    pub fn visible_tasks(&self) -> impl Iterator<Item = &store::Snapshot> {
        self.tasks
            .iter()
            .filter(|task| task.archived == self.show_archived)
    }

    fn archive_selected(&mut self, archived: bool) {
        let selected = self
            .visible_tasks()
            .nth(self.task_selection)
            .map(|task| task.id);
        let Some(id) = selected else { return };
        if archived
            && id == self.task_id
            && (!matches!(self.storage, StorageState::Saved) || self.save_requested)
        {
            self.notice = "Finish or retry this task's save before archiving it.".into();
            return;
        }
        self.management = Some(store::Management::Archive { id, archived });
        self.transitioning = true;
    }

    fn preview_delete(&mut self, expired: bool) {
        self.delete_targets.clear();
        self.delete_expired_before = None;
        let selected = self
            .visible_tasks()
            .nth(self.task_selection)
            .map(|task| task.id);
        if expired {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let age = u128::from(self.policy.retention_days) * 86_400_000;
            let cutoff = now.saturating_sub(age).min(i64::MAX as u128) as i64;
            self.delete_targets.extend(
                self.tasks
                    .iter()
                    .filter(|task| {
                        task.archived && task.archived_at > 0 && task.archived_at <= cutoff
                    })
                    .map(|task| task.id),
            );
            self.delete_expired_before = Some(cutoff);
        } else if let Some(id) = selected {
            self.delete_targets.push(id);
        }
        if self.delete_targets.is_empty() {
            self.notice = "No eligible tasks selected; nothing was deleted.".into();
            return;
        }
        self.delete_preview.clear();
        self.deletion_preview_pending = true;
        self.transitioning = true;
        self.management = Some(store::Management::PreviewDelete {
            ids: self.delete_targets.clone(),
            expired_before: self.delete_expired_before,
        });
    }

    fn request_complete(&mut self) {
        if self.complete {
            self.notice =
                "Already complete. Editing goal or criteria requires explicit reopening.".into();
        } else if self.drafts.iter().any(|draft| {
            matches!(
                draft.field,
                store::EditorField::Goal | store::EditorField::Criteria
            )
        }) {
            self.notice =
                "Apply or discard retained goal/criteria drafts before completing the task.".into();
        } else if self.criteria.is_empty() || self.criteria.iter().any(|criterion| !criterion.met) {
            self.notice =
                "Complete requires at least one criterion and every criterion checked.".into();
        } else {
            self.confirmation = Some(Confirmation {
                kind: ConfirmationKind::Complete,
                destructive: false,
            });
        }
    }

    fn changed_task(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.save_requested = true;
        if !matches!(
            self.storage,
            StorageState::Failed(_) | StorageState::Loading
        ) {
            self.storage = StorageState::Saving;
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
            self.form
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

    /// Returns whether visible state changed. The event loop otherwise stays asleep.
    pub fn update(&mut self, event: Event) -> bool {
        let resized = matches!(event, Event::Resize(_, _));
        let revision = self.revision;
        let navigation = self.navigation();
        let input_state = (self.input.len(), self.cursor);
        let changed = match event {
            Event::Resize(width, height) => {
                if (width, height) == (self.width, self.height) {
                    return false;
                }
                self.width = width;
                self.height = height;
                true
            }
            Event::Paste(text)
                if self.overlay == Some(Overlay::Sources)
                    && self.sources.editor.is_some()
                    && self.confirmation.is_none()
                    && !self.transitioning =>
            {
                self.sources.paste(&text)
            }
            Event::Paste(text)
                if self.editing
                    && self.confirmation.is_none()
                    && !self.quit_after_save
                    && !self.transitioning =>
            {
                self.insert_input(&text)
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            _ => false,
        };
        if changed
            && !resized
            && !self.transitioning
            && !self.quit
            && !self.quit_after_save
            && self.revision == revision
            && (navigation != self.navigation() || input_state != (self.input.len(), self.cursor))
            && (self.task_id > 0 || !self.goal.is_empty() || self.draft_dirty())
        {
            self.changed_task();
        }
        changed
    }

    fn key(&mut self, key: KeyEvent) -> bool {
        if self.transitioning {
            if self.deletion_preview_pending && key.code == KeyCode::Esc {
                self.deletion_preview_pending = false;
                self.transitioning = false;
                self.management = None;
                return true;
            }
            return false;
        }
        if self.quit_after_save {
            if key.code == KeyCode::Esc {
                self.quit_after_save = false;
                return true;
            }
            return false;
        }
        if self.confirmation.is_some() {
            return self.confirmation_key(key);
        }
        if self.overlay == Some(Overlay::Sources)
            && (self.sources.editor.is_some() || self.sources.confirmation.is_some())
        {
            return self.source_key(key);
        }
        if self.overlay == Some(Overlay::Execution) {
            return self.execution_key(key);
        }
        // Auto-repeat must never apply a draft or approve/open a destructive action.
        if key.kind == KeyEventKind::Repeat
            && (key.code == KeyCode::Enter
                || (!self.editing && key.code == KeyCode::Char('q'))
                || (key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('c')))
        {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.request_quit();
            return true;
        }
        if self.editing {
            return self.edit_key(key);
        }
        if key.code == KeyCode::Char('q')
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.request_quit();
            return true;
        }
        // Back and exit remain available even when the terminal cannot draw a panel.
        if key.code == KeyCode::Esc
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && self.overlay.is_some()
            && self.overlay != Some(Overlay::Sources)
        {
            self.close_overlay();
            return true;
        }
        if self.width < 60 || self.height < 18 {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('k') {
            if self.overlay == Some(Overlay::Palette) {
                return false;
            }
            self.open_overlay(Overlay::Palette);
            return true;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        if key.code == KeyCode::Char('l') {
            self.linear = !self.linear;
            return true;
        }
        if key.code == KeyCode::Char('r')
            && !matches!(self.overlay, Some(Overlay::Repository | Overlay::Sources))
            && matches!(self.storage, StorageState::Failed(_))
        {
            self.retry_storage = true;
            self.storage = StorageState::Loading;
            return true;
        }
        if self.overlay == Some(Overlay::Sources) {
            return self.source_key(key);
        }
        if let Some(overlay) = self.overlay {
            return self.overlay_key(overlay, key.code);
        }
        match key.code {
            KeyCode::Char('f') => self.open_overlay(Overlay::Forms),
            KeyCode::Char('g') => self.begin_edit(),
            KeyCode::Char('i') => self.open_overlay(Overlay::Inspector),
            KeyCode::Char('b') => self.open_overlay(Overlay::Companion),
            KeyCode::Char('?') => self.open_overlay(Overlay::Help),
            KeyCode::Char('p') => self.open_overlay(Overlay::Preview),
            KeyCode::Char('t') => self.open_overlay(Overlay::Tasks),
            KeyCode::Char('n') => self.new_task(),
            KeyCode::Char('c') => self.open_overlay(Overlay::Criteria),
            KeyCode::Char('a') => self.begin_edit_field(store::EditorField::NextAction),
            KeyCode::Char('d') => self.request_complete(),
            KeyCode::Char('w') => self.open_overlay(Overlay::Repository),
            KeyCode::Char('v') => self.open_overlay(Overlay::Sources),
            KeyCode::Char('j') => self.open_overlay(Overlay::Execution),
            KeyCode::Tab | KeyCode::BackTab => {
                return self.cycle_focus(key.code == KeyCode::BackTab);
            }
            KeyCode::Enter => match self.effective_focus() {
                Pane::Companion => self.open_overlay(Overlay::Forms),
                Pane::Workspace => self.begin_edit(),
                Pane::Inspector => self.open_overlay(Overlay::Inspector),
            },
            code => return self.scroll_key(code),
        }
        true
    }

    fn edit_key(&mut self, mut key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('s') && key.modifiers == KeyModifiers::CONTROL {
            self.drafts.retain(|draft| draft.field != self.editor_field);
            if self.draft_dirty() {
                self.drafts.push(store::Draft {
                    field: self.editor_field,
                    text: self.input.clone(),
                    cursor: self.cursor,
                });
            }
            self.input.clear();
            self.finish_edit();
            self.changed_task();
            return true;
        }
        // Native VT/ConPTY terminals may encode Backspace as BS (Ctrl+H),
        // while others use DEL. Both delete one complete grapheme.
        if key.code == KeyCode::Char('h') && key.modifiers == KeyModifiers::CONTROL {
            key.code = KeyCode::Backspace;
            key.modifiers = KeyModifiers::NONE;
        }
        // A portable newline key for terminals that cannot distinguish
        // Shift+Enter from Enter. Never bind a pasted newline to local apply.
        if key.code == KeyCode::Char('n') && key.modifiers == KeyModifiers::CONTROL {
            return self.insert_input("\n");
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        self.normalize_cursor();
        match key.code {
            KeyCode::Esc => {
                if self.draft_dirty() {
                    self.confirmation = Some(Confirmation {
                        kind: ConfirmationKind::DiscardDraft,
                        destructive: false,
                    });
                } else {
                    self.drafts.retain(|draft| draft.field != self.editor_field);
                    self.input.clear();
                    self.finish_edit();
                }
                true
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.insert_input("\n")
            }
            KeyCode::Enter => self.apply_editor(),
            KeyCode::Left => self.move_cursor(self.previous_boundary()),
            KeyCode::Right => self.move_cursor(self.next_boundary()),
            KeyCode::Home => self.move_cursor(self.line_start()),
            KeyCode::End => self.move_cursor(self.line_end()),
            KeyCode::Up => self.move_vertical(false),
            KeyCode::Down => self.move_vertical(true),
            KeyCode::Backspace => {
                let start = self.previous_boundary();
                if start == self.cursor {
                    return false;
                }
                self.input.replace_range(start..self.cursor, "");
                self.cursor = start;
                self.normalize_cursor();
                self.preferred_column = None;
                true
            }
            KeyCode::Delete => {
                let end = self.next_boundary();
                if end == self.cursor {
                    return false;
                }
                self.input.replace_range(self.cursor..end, "");
                self.normalize_cursor();
                self.preferred_column = None;
                true
            }
            KeyCode::Char(character) if safe_character(character) && character != '\n' => {
                let mut bytes = [0; 4];
                self.insert_input(character.encode_utf8(&mut bytes))
            }
            _ => false,
        }
    }

    fn request_quit(&mut self) {
        self.request_navigation(execution::Action::Quit);
    }

    fn finish_request_quit(&mut self) {
        if !self.goal.is_empty()
            || self.draft_dirty()
            || self.revision != self.saved_revision
            || self.sources.persistence_pending()
            || self.sources.store_failed
        {
            self.confirmation = Some(Confirmation {
                kind: ConfirmationKind::Quit,
                destructive: false,
            });
        } else {
            self.quit = true;
        }
    }

    pub fn deletion_pages(&self) -> usize {
        self.delete_preview
            .iter()
            .map(|target| 1 + target.excerpts.len() + target.runs.len())
            .sum::<usize>()
            .div_ceil(DELETE_PREVIEW_PAGE)
            .max(1)
    }

    fn confirmation_key(&mut self, key: KeyEvent) -> bool {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            || key.kind != KeyEventKind::Press
        {
            return false;
        }
        if self
            .confirmation
            .is_some_and(|confirmation| confirmation.kind == ConfirmationKind::DeleteTasks)
            && matches!(key.code, KeyCode::PageUp | KeyCode::PageDown)
        {
            return self.scroll_key(key.code);
        }
        if self
            .confirmation
            .is_some_and(|c| c.kind == ConfirmationKind::DeleteTasks)
            && matches!(key.code, KeyCode::Char('n' | 'p'))
        {
            self.delete_page = if key.code == KeyCode::Char('n') {
                (self.delete_page + 1).min(self.deletion_pages() - 1)
            } else {
                self.delete_page.saturating_sub(1)
            };
            self.surface_scroll[ScrollSurface::Preview.index()] = 0;
            self.confirmation
                .as_mut()
                .expect("active deletion review")
                .destructive = false;
            return true;
        }
        let confirmation = self.confirmation.as_mut().expect("active confirmation");
        match key.code {
            KeyCode::Esc => {
                self.confirmation = None;
                true
            }
            KeyCode::Tab | KeyCode::BackTab => {
                confirmation.destructive = !confirmation.destructive;
                true
            }
            KeyCode::Left | KeyCode::Up | KeyCode::Home => {
                let changed = confirmation.destructive;
                confirmation.destructive = false;
                changed
            }
            KeyCode::Right | KeyCode::Down | KeyCode::End => {
                let changed = !confirmation.destructive;
                confirmation.destructive = true;
                changed
            }
            KeyCode::Enter => {
                let confirmation = self.confirmation.take().expect("active confirmation");
                if confirmation.destructive {
                    match confirmation.kind {
                        ConfirmationKind::DiscardDraft => {
                            self.drafts.retain(|draft| draft.field != self.editor_field);
                            self.input.clear();
                            self.finish_edit();
                            self.changed_task();
                        }
                        ConfirmationKind::Quit => {
                            if matches!(self.storage, StorageState::Saving | StorageState::Loading)
                                || self.sources.persistence_pending()
                                || self.execution.busy()
                            {
                                self.quit_after_save = true;
                            } else {
                                self.quit = true;
                            }
                        }
                        ConfirmationKind::Complete => {
                            if !self.criteria.is_empty()
                                && self.criteria.iter().all(|criterion| criterion.met)
                            {
                                self.complete = true;
                                self.changed_task();
                                self.notice = "Task completed by your criterion review; no command evidence is implied.".into();
                            }
                        }
                        ConfirmationKind::Reopen(field) => {
                            self.complete = false;
                            self.changed_task();
                            self.open_editor(field);
                        }
                        ConfirmationKind::DeleteTasks => {
                            self.management = Some(store::Management::Delete {
                                ids: std::mem::take(&mut self.delete_targets),
                                expired_before: self.delete_expired_before.take(),
                            });
                            self.transitioning = true;
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }
    fn insert_input(&mut self, text: &str) -> bool {
        self.normalize_cursor();
        let capacity = MAX_GOAL_BYTES.saturating_sub(self.input.len());
        let mut inserted = String::with_capacity(text.len().min(capacity));
        let was_truncated = self.input_truncated;
        let mut chars = text.chars().peekable();
        while let Some(mut character) = chars.next() {
            if character == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                character = '\n';
            }
            if !safe_character(character) {
                continue;
            }
            if inserted.len() + character.len_utf8() > capacity {
                self.input_truncated = true;
                break;
            }
            inserted.push(character);
        }
        if inserted.is_empty() {
            return self.input_truncated != was_truncated;
        }
        self.input.insert_str(self.cursor, &inserted);
        self.cursor += inserted.len();
        // A combining insertion can merge with the following grapheme.
        self.cursor = self
            .input
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|&index| index >= self.cursor)
            .unwrap_or(self.input.len());
        self.preferred_column = None;
        true
    }

    fn normalize_cursor(&mut self) {
        self.cursor = self.cursor.min(self.input.len());
        if self.cursor != self.input.len() {
            self.cursor = self
                .input
                .grapheme_indices(true)
                .map(|(index, _)| index)
                .take_while(|&index| index <= self.cursor)
                .last()
                .unwrap_or(0);
        }
    }

    fn previous_boundary(&self) -> usize {
        self.input
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .take_while(|&index| index < self.cursor)
            .last()
            .unwrap_or(0)
    }

    fn next_boundary(&self) -> usize {
        self.input
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|&index| index > self.cursor)
            .unwrap_or(self.input.len())
    }

    fn line_start(&self) -> usize {
        self.input[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1)
    }

    fn line_end(&self) -> usize {
        self.input[self.cursor..]
            .find('\n')
            .map_or(self.input.len(), |index| self.cursor + index)
    }

    fn move_cursor(&mut self, cursor: usize) -> bool {
        self.preferred_column = None;
        let changed = self.cursor != cursor;
        self.cursor = cursor;
        changed
    }

    fn move_vertical(&mut self, down: bool) -> bool {
        let start = self.line_start();
        let end = self.line_end();
        let column = self
            .preferred_column
            .unwrap_or_else(|| Span::raw(&self.input[start..self.cursor]).width());
        let (target_start, target_end) = if down {
            if end == self.input.len() {
                return false;
            }
            let next = end + 1;
            let next_end = self.input[next..]
                .find('\n')
                .map_or(self.input.len(), |index| next + index);
            (next, next_end)
        } else {
            if start == 0 {
                return false;
            }
            let previous_end = start - 1;
            let previous = self.input[..previous_end]
                .rfind('\n')
                .map_or(0, |index| index + 1);
            (previous, previous_end)
        };
        let mut cursor = target_start;
        let mut cells = 0;
        for (index, grapheme) in self.input[target_start..target_end].grapheme_indices(true) {
            let width = Span::raw(grapheme).width();
            if cells + width > column {
                break;
            }
            cells += width;
            cursor = target_start + index + grapheme.len();
        }
        self.preferred_column = Some(column);
        let changed = cursor != self.cursor;
        self.cursor = cursor;
        changed
    }

    fn begin_edit(&mut self) {
        self.begin_edit_field(store::EditorField::Goal);
    }

    fn begin_edit_field(&mut self, field: store::EditorField) {
        if self.complete
            && matches!(
                field,
                store::EditorField::Goal | store::EditorField::Criteria
            )
        {
            self.confirmation = Some(Confirmation {
                kind: ConfirmationKind::Reopen(field),
                destructive: false,
            });
            return;
        }
        self.open_editor(field);
    }

    fn open_editor(&mut self, field: store::EditorField) {
        if field == store::EditorField::Command {
            self.execution.invalidate();
            self.pending_command = None;
        }
        self.save_selection();
        self.edit_return = self.overlay_frame();
        self.overlay = None;
        self.editor_field = field;
        self.edit_original = self.applied_text(field);
        if let Some(draft) = self.drafts.iter().find(|draft| draft.field == field) {
            self.input.clone_from(&draft.text);
            self.cursor = draft.cursor;
        } else {
            self.input.clone_from(&self.edit_original);
            self.cursor = self.input.len();
        }
        self.normalize_cursor();
        self.input_truncated = false;
        self.preferred_column = None;
        self.editing = true;
        self.notice.clear();
    }

    fn apply_editor(&mut self) -> bool {
        match self.editor_field {
            store::EditorField::Goal => self.goal = std::mem::take(&mut self.input),
            store::EditorField::NextAction => self.next_action = std::mem::take(&mut self.input),
            store::EditorField::Command => {
                self.command = std::mem::take(&mut self.input);
                self.pending_command = Some(self.command.clone());
            }
            store::EditorField::Criteria => {
                let lines: Vec<&str> = self
                    .input
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .collect();
                if lines.len() > 64 || lines.iter().any(|line| line.len() > 256) {
                    self.notice =
                        "At most 64 criteria, each at most 256 UTF-8 bytes. Draft kept.".into();
                    return true;
                }
                self.criteria = lines
                    .into_iter()
                    .enumerate()
                    .map(|(index, text)| store::Criterion {
                        met: self
                            .criteria
                            .get(index)
                            .is_some_and(|old| old.text == text && old.met),
                        text: text.to_owned(),
                    })
                    .collect();
                self.input.clear();
                self.criterion_selection = self
                    .criterion_selection
                    .min(self.criteria.len().saturating_sub(1));
            }
        }
        self.drafts.retain(|draft| draft.field != self.editor_field);
        self.finish_edit();
        self.changed_task();
        true
    }

    fn finish_edit(&mut self) {
        self.editing = false;
        self.cursor = 0;
        self.input_truncated = false;
        self.preferred_column = None;
        let frame = self.edit_return.take();
        self.restore_frame(frame);
    }

    fn save_selection(&mut self) {
        match self.overlay {
            Some(Overlay::Forms) => self.form_selection = self.selected,
            Some(Overlay::Palette) => self.palette_selection = self.selected,
            _ => {}
        }
    }

    fn restore_selection(&mut self) {
        match self.overlay {
            Some(Overlay::Forms) => self.selected = self.form_selection,
            Some(Overlay::Palette) => self.selected = self.palette_selection,
            _ => {}
        }
    }

    fn overlay_frame(&self) -> Option<OverlayFrame> {
        self.overlay.map(|overlay| OverlayFrame {
            overlay,
            selected: self.selected,
            focus: self.focus,
            preview_form: self.preview_form,
        })
    }

    fn restore_frame(&mut self, frame: Option<OverlayFrame>) {
        self.overlay = frame.map(|frame| frame.overlay);
        if let Some(frame) = frame {
            self.selected = frame.selected;
            self.focus = frame.focus;
            self.preview_form = frame.preview_form;
        }
    }

    fn open_overlay(&mut self, overlay: Overlay) {
        self.save_selection();
        if let Some(previous) = self.overlay_frame() {
            if previous.overlay == overlay {
                return;
            }
            self.overlay_history.push(previous);
        }
        if overlay == Overlay::Preview {
            self.preview_form = if self.overlay == Some(Overlay::Forms) {
                self.selected
            } else {
                self.form
            };
        }
        if matches!(
            overlay,
            Overlay::Tasks | Overlay::Criteria | Overlay::Repository | Overlay::Sources
        ) {
            self.surface_scroll[ScrollSurface::Preview.index()] = 0;
        }
        self.overlay = Some(overlay);
        self.restore_selection();
        if overlay == Overlay::Sources {
            self.sources
                .catalogue(self.task_id, self.repository.as_ref());
        } else if overlay == Overlay::Execution {
            self.execution.open(self.task_id);
        }
    }

    fn close_overlay(&mut self) {
        self.save_selection();
        let frame = self.overlay_history.pop();
        self.restore_frame(frame);
        if self.overlay == Some(Overlay::Execution) {
            self.execution.open(self.task_id);
        }
    }

    fn task_overlay_key(&mut self, overlay: Overlay, key: KeyCode) -> bool {
        if matches!(key, KeyCode::PageUp | KeyCode::PageDown) {
            return self.scroll_key(key);
        }
        match (overlay, key) {
            (Overlay::Tasks, KeyCode::Char('t')) | (Overlay::Criteria, KeyCode::Char('c')) => {
                self.close_overlay();
                return true;
            }
            (Overlay::Tasks, KeyCode::Char('n')) => {
                self.new_task();
                return true;
            }
            (Overlay::Tasks, KeyCode::Char('h')) => {
                self.show_archived = !self.show_archived;
                self.task_selection = 0;
                return true;
            }
            (Overlay::Tasks, KeyCode::Char('a')) if !self.show_archived => {
                self.archive_selected(true);
                return true;
            }
            (Overlay::Tasks, KeyCode::Char('r')) if self.show_archived => {
                self.archive_selected(false);
                return true;
            }
            (Overlay::Tasks, KeyCode::Delete) => {
                self.preview_delete(false);
                return true;
            }
            (Overlay::Tasks, KeyCode::Char('x')) => {
                self.preview_delete(true);
                return true;
            }
            (Overlay::Tasks, KeyCode::Enter) => {
                self.select_task();
                return true;
            }
            (Overlay::Criteria, KeyCode::Char('e')) => {
                self.begin_edit_field(store::EditorField::Criteria);
                return true;
            }
            (Overlay::Criteria, KeyCode::Char('d')) => {
                self.request_complete();
                return true;
            }
            (Overlay::Criteria, KeyCode::Char(' ')) => {
                if self.complete {
                    self.confirmation = Some(Confirmation {
                        kind: ConfirmationKind::Reopen(store::EditorField::Criteria),
                        destructive: false,
                    });
                } else if let Some(criterion) = self.criteria.get_mut(self.criterion_selection) {
                    criterion.met = !criterion.met;
                    self.changed_task();
                }
                return true;
            }
            _ => {}
        }
        let task_count = self.visible_tasks().count();
        let (selection, count) = if overlay == Overlay::Tasks {
            (&mut self.task_selection, task_count)
        } else {
            (&mut self.criterion_selection, self.criteria.len())
        };
        if count == 0 {
            return false;
        }
        let previous = *selection;
        *selection = match key {
            KeyCode::Up => previous.saturating_sub(1),
            KeyCode::Down => (previous + 1).min(count - 1),
            KeyCode::Home => 0,
            KeyCode::End => count - 1,
            _ => return false,
        };
        let changed = previous != *selection;
        if changed {
            self.surface_scroll[ScrollSurface::Preview.index()] = 0;
        }
        changed
    }

    fn overlay_key(&mut self, overlay: Overlay, key: KeyCode) -> bool {
        if matches!(overlay, Overlay::Tasks | Overlay::Criteria) {
            return self.task_overlay_key(overlay, key);
        }
        match overlay {
            Overlay::Help if key == KeyCode::Char('?') => {
                self.close_overlay();
                return true;
            }
            Overlay::Inspector if key == KeyCode::Char('i') => {
                self.close_overlay();
                return true;
            }
            Overlay::Companion => {
                if matches!(key, KeyCode::Enter | KeyCode::Char('f')) {
                    self.open_overlay(Overlay::Forms);
                    return true;
                }
                if key == KeyCode::Char('b') {
                    self.close_overlay();
                    return true;
                }
            }
            Overlay::Preview if key == KeyCode::Char('p') => {
                self.close_overlay();
                return true;
            }
            Overlay::Repository if key == KeyCode::Char('w') => {
                self.close_overlay();
                return true;
            }
            Overlay::Repository if key == KeyCode::Char('r') => {
                if !self.repo_loading {
                    self.repo_refresh = true;
                    self.repo_loading = true;
                    self.repo_status = "Repository: refreshing / original baseline retained".into();
                }
                return true;
            }
            _ => {}
        }
        let count = match overlay {
            Overlay::Forms => FORMS.len(),
            Overlay::Palette => PALETTE.len(),
            _ => return self.scroll_key(key),
        };
        let previous = self.selected;
        let grid =
            overlay == Overlay::Forms && self.width >= 90 && self.height >= 30 && !self.linear;
        let vertical_step = if grid { 3 } else { 1 };
        match key {
            KeyCode::Up => self.selected = (self.selected + count - vertical_step) % count,
            KeyCode::Down => self.selected = (self.selected + vertical_step) % count,
            KeyCode::Left if overlay == Overlay::Forms => {
                self.selected = (self.selected + count - 1) % count
            }
            KeyCode::Right if overlay == Overlay::Forms => {
                self.selected = (self.selected + 1) % count
            }
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = count - 1,
            KeyCode::Char(character)
                if overlay == Overlay::Forms && ('1'..='9').contains(&character) =>
            {
                self.selected = (character as u8 - b'1') as usize;
            }
            KeyCode::Char('p') if overlay == Overlay::Forms => {
                self.open_overlay(Overlay::Preview);
                return true;
            }
            KeyCode::Enter => {
                let selection = self.selected;
                if overlay == Overlay::Palette && selection == 7 {
                    self.request_quit();
                    return true;
                }
                if overlay == Overlay::Forms {
                    self.request_navigation(execution::Action::SelectForm(selection));
                } else {
                    self.close_overlay();
                    match selection {
                        0 => self.open_overlay(Overlay::Forms),
                        1 => self.begin_edit(),
                        2 => self.open_overlay(Overlay::Inspector),
                        3 => self.open_overlay(Overlay::Companion),
                        4 => self.open_overlay(Overlay::Help),
                        5 => self.open_overlay(Overlay::Preview),
                        6 => self.linear = !self.linear,
                        7 => unreachable!("quit handled without leaving the palette"),
                        8 => self.open_overlay(Overlay::Tasks),
                        9 => self.new_task(),
                        10 => self.open_overlay(Overlay::Criteria),
                        11 => self.begin_edit_field(store::EditorField::NextAction),
                        12 => self.request_complete(),
                        13 => self.open_overlay(Overlay::Repository),
                        14 => self.open_overlay(Overlay::Sources),
                        15 => self.open_overlay(Overlay::Execution),
                        _ => unreachable!("palette selection is bounded"),
                    }
                }
                return true;
            }
            _ => return false,
        }
        self.save_selection();
        previous != self.selected
    }

    fn scroll_surface(&self) -> ScrollSurface {
        match self.overlay {
            Some(Overlay::Companion) => ScrollSurface::Companion,
            Some(Overlay::Inspector) => ScrollSurface::Inspector,
            Some(Overlay::Help) => ScrollSurface::Help,
            Some(
                Overlay::Preview
                | Overlay::Tasks
                | Overlay::Criteria
                | Overlay::Repository
                | Overlay::Sources,
            ) => ScrollSurface::Preview,
            _ => match self.effective_focus() {
                Pane::Companion => ScrollSurface::Companion,
                Pane::Workspace => ScrollSurface::Workspace,
                Pane::Inspector => ScrollSurface::Inspector,
            },
        }
    }

    fn scroll_key(&mut self, key: KeyCode) -> bool {
        let page = self.height.saturating_sub(8).max(1);
        let surface = self.scroll_surface().index();
        let previous = self.surface_scroll[surface];
        self.surface_scroll[surface] = match key {
            KeyCode::Up => previous.saturating_sub(1),
            KeyCode::Down => previous.saturating_add(1),
            KeyCode::PageUp => previous.saturating_sub(page),
            KeyCode::PageDown => previous.saturating_add(page),
            KeyCode::Home => 0,
            KeyCode::End => self.surface_scroll_max[surface],
            _ => return false,
        }
        .min(self.surface_scroll_max[surface]);
        previous != self.surface_scroll[surface]
    }

    fn cycle_focus(&mut self, backwards: bool) -> bool {
        if self.linear || self.width < 90 {
            return false;
        }
        self.focus = if self.width < 120 {
            match self.effective_focus() {
                Pane::Companion => Pane::Workspace,
                _ => Pane::Companion,
            }
        } else {
            match (self.effective_focus(), backwards) {
                (Pane::Companion, false) | (Pane::Inspector, true) => Pane::Workspace,
                (Pane::Workspace, false) | (Pane::Companion, true) => Pane::Inspector,
                (Pane::Inspector, false) | (Pane::Workspace, true) => Pane::Companion,
            }
        };
        true
    }
}

fn safe_character(character: char) -> bool {
    (character == '\n' || !character.is_control())
        && !matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.store_event(store::Event::Loaded(store::Project {
            tasks: Vec::new(),
            active_id: None,
            usage_bytes: 0,
        }));
        app
    }

    fn committed(app: &App) -> store::Snapshot {
        let mut snapshot = app.snapshot("/workspace");
        snapshot.id = app.task_id.max(1);
        snapshot
    }

    fn key(app: &mut App, code: KeyCode) -> bool {
        modified_key(app, code, KeyModifiers::NONE)
    }

    fn modified_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> bool {
        app.update(Event::Key(KeyEvent::new(code, modifiers)))
    }

    #[test]
    fn all_forms_preview_confirm_and_cancel_in_grid_list_and_linear_layouts() {
        for (width, height, linear) in [
            (120, 36, false),
            (90, 30, false),
            (90, 29, false),
            (70, 24, false),
            (120, 36, true),
        ] {
            let mut app = App::new("/workspace".into(), width, height);
            app.linear = linear;
            app.goal = "Keep existing work\n한글".into();
            app.focus = Pane::Inspector;
            for selection in 0..FORMS.len() {
                let original = app.form;
                key(&mut app, KeyCode::Char('f'));
                key(&mut app, KeyCode::Char((b'1' + selection as u8) as char));
                assert_eq!(app.selected, selection);
                assert_eq!(app.form, original);
                key(&mut app, KeyCode::Esc);
                assert_eq!(app.form, original);
                key(&mut app, KeyCode::Char('f'));
                assert_eq!(app.selected, selection);
                key(&mut app, KeyCode::Enter);
                assert_eq!(app.form, selection);
                assert_eq!(app.goal, "Keep existing work\n한글");
                assert_eq!(app.focus, Pane::Inspector);
            }
        }
    }

    #[test]
    fn grid_arrows_wrap_by_column_and_row_while_list_moves_one_item() {
        let mut app = app();
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Up);
        assert_eq!(app.selected, 6);
        key(&mut app, KeyCode::Down);
        assert_eq!(app.selected, 0);
        key(&mut app, KeyCode::Left);
        assert_eq!(app.selected, 8);
        key(&mut app, KeyCode::Right);
        assert_eq!(app.selected, 0);
        app.update(Event::Resize(89, 30));
        key(&mut app, KeyCode::Up);
        assert_eq!(app.selected, 8);
        app.update(Event::Resize(120, 36));
        key(&mut app, KeyCode::Char('l'));
        key(&mut app, KeyCode::Down);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn editing_is_modal_and_cancel_keeps_the_committed_goal() {
        let mut app = app();
        app.goal = "original".into();
        key(&mut app, KeyCode::Char('g'));
        key(&mut app, KeyCode::Char('q'));
        assert!(!app.quit);
        assert!(!key(&mut app, KeyCode::Tab));
        for code in [
            KeyCode::Char('k'),
            KeyCode::Left,
            KeyCode::Enter,
            KeyCode::Esc,
        ] {
            assert!(!modified_key(&mut app, code, KeyModifiers::CONTROL));
            assert!(!modified_key(&mut app, code, KeyModifiers::ALT));
        }
        assert_eq!(app.overlay, None);
        assert!(app.editing);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.goal, "original");
        assert_eq!(
            app.confirmation,
            Some(Confirmation {
                kind: ConfirmationKind::DiscardDraft,
                destructive: false
            })
        );
        assert_eq!(app.input, "originalq");
        key(&mut app, KeyCode::Esc);
        assert!(app.editing);
        assert_eq!(app.input, "originalq");
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Enter);
        assert!(!app.editing);
        assert_eq!(app.input, "");
        key(&mut app, KeyCode::Char('g'));
        key(&mut app, KeyCode::Char('!'));
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.goal, "original!");
    }

    #[test]
    fn cursor_insertion_and_deletion_preserve_whole_unicode_graphemes() {
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("한e\u{301}👩‍💻界".into()));
        key(&mut app, KeyCode::Left);
        assert_eq!(&app.input[app.cursor..], "界");
        key(&mut app, KeyCode::Backspace);
        assert_eq!(app.input, "한e\u{301}界");
        key(&mut app, KeyCode::Left);
        assert_eq!(&app.input[app.cursor..], "e\u{301}界");
        key(&mut app, KeyCode::Delete);
        assert_eq!(app.input, "한界");
        key(&mut app, KeyCode::Char('中'));
        assert_eq!(app.input, "한中界");
        key(&mut app, KeyCode::Home);
        assert_eq!(app.cursor, 0);
        assert!(!key(&mut app, KeyCode::Backspace));
        key(&mut app, KeyCode::End);
        assert_eq!(app.cursor, app.input.len());
        assert!(!key(&mut app, KeyCode::Delete));
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.goal, "한中界");
    }

    #[test]
    fn vt_backspace_deletes_a_combining_grapheme_without_leaving_its_mark() {
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("cafe\u{301}".into()));
        assert!(modified_key(
            &mut app,
            KeyCode::Char('h'),
            KeyModifiers::CONTROL
        ));
        assert_eq!(app.input, "caf");
        assert_eq!(app.cursor, 3);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.goal, "caf");
    }

    #[test]
    fn multiline_movement_retains_column_across_short_rows_and_shift_enter() {
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("한e\u{301}界\nx\n👩‍💻中文".into()));
        key(&mut app, KeyCode::Left);
        key(&mut app, KeyCode::Up);
        assert_eq!(&app.input[..app.cursor], "한e\u{301}界\nx");
        key(&mut app, KeyCode::Up);
        assert_eq!(&app.input[..app.cursor], "한e\u{301}");
        key(&mut app, KeyCode::Home);
        assert_eq!(app.cursor, 0);
        key(&mut app, KeyCode::Down);
        assert_eq!(&app.input[app.cursor..], "x\n👩‍💻中文");
        key(&mut app, KeyCode::End);
        modified_key(&mut app, KeyCode::Enter, KeyModifiers::SHIFT);
        assert!(app.editing);
        assert!(app.input.contains("x\n\n"));
        key(&mut app, KeyCode::Backspace);
        key(&mut app, KeyCode::Delete);
        assert!(app.input.contains("x👩‍💻"));
    }

    #[test]
    fn portable_newline_key_keeps_the_draft_local_until_explicit_apply() {
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("first".into()));
        modified_key(&mut app, KeyCode::Char('n'), KeyModifiers::CONTROL);
        app.update(Event::Paste("second".into()));
        assert!(app.editing);
        assert_eq!(app.goal, "");
        assert_eq!(app.input, "first\nsecond");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.goal, "first\nsecond");
    }

    #[test]
    fn pasted_newlines_and_commands_are_bounded_local_text_not_actions() {
        let mut app = app();
        assert!(!app.update(Event::Paste("q\n/run".into())));
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("q\r\n/run\r한\x1b\x07\u{202e}\t".into()));
        assert_eq!(app.input, "q\n/run\n한");
        assert!(!app.quit);
        assert!(app.editing);
        assert_eq!(app.goal, "");
        app.update(Event::Paste("界".repeat(MAX_GOAL_BYTES)));
        assert!(app.input.len() <= MAX_GOAL_BYTES);
        assert!(app.input.len() > MAX_GOAL_BYTES - 3);
        assert!(app.input_truncated);
        assert!(!app.update(Event::Paste("界".into())));
        key(&mut app, KeyCode::Enter);
        assert!(app.goal.starts_with("q\n/run\n한"));
        assert!(!app.quit);
    }

    #[test]
    fn resizing_preserves_preferred_focus_overlay_selection_and_unicode_draft() {
        let mut app = app();
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Pane::Inspector);
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('9'));
        app.update(Event::Resize(70, 24));
        assert_eq!(app.focus, Pane::Inspector);
        assert_eq!(app.effective_focus(), Pane::Workspace);
        assert_eq!(app.selected, 8);
        key(&mut app, KeyCode::Esc);
        assert!(!key(&mut app, KeyCode::Tab));
        app.update(Event::Resize(120, 36));
        assert_eq!(app.effective_focus(), Pane::Inspector);
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("draft\n한e\u{301}".into()));
        key(&mut app, KeyCode::Left);
        let cursor = app.cursor;
        app.update(Event::Resize(10, 3));
        assert_eq!(app.input, "draft\n한e\u{301}");
        assert_eq!(app.cursor, cursor);
        key(&mut app, KeyCode::Esc);
        assert!(app.editing);
        assert_eq!(
            app.confirmation.unwrap().kind,
            ConfirmationKind::DiscardDraft
        );
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.cursor, cursor);
        assert_eq!(app.input, "draft\n한e\u{301}");
        app.update(Event::Resize(120, 36));
        assert_eq!(app.effective_focus(), Pane::Inspector);
    }

    #[test]
    fn overlays_restore_invoking_surface_and_keep_independent_selections() {
        let mut app = app();
        app.focus = Pane::Companion;
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('8'));
        modified_key(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!(app.selected, 0);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.overlay, Some(Overlay::Forms));
        assert_eq!(app.selected, 7);
        modified_key(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!(app.selected, 1);
        key(&mut app, KeyCode::Enter);
        assert!(app.editing);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.overlay, Some(Overlay::Forms));
        assert_eq!(app.selected, 7);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.effective_focus(), Pane::Companion);
    }

    #[test]
    fn each_nonselecting_surface_retains_its_own_scroll() {
        let mut app = app();
        for surface in [
            ScrollSurface::Companion,
            ScrollSurface::Workspace,
            ScrollSurface::Inspector,
            ScrollSurface::Help,
            ScrollSurface::Preview,
        ] {
            app.set_scroll_max(surface, 100);
        }
        key(&mut app, KeyCode::Down);
        assert_eq!(app.scroll(), 1);
        key(&mut app, KeyCode::Char('?'));
        key(&mut app, KeyCode::PageDown);
        let help_scroll = app.scroll();
        assert!(help_scroll > 1);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.scroll(), 1);
        key(&mut app, KeyCode::Char('i'));
        assert_eq!(app.scroll(), 0);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('?'));
        assert_eq!(app.scroll(), help_scroll);
        key(&mut app, KeyCode::PageUp);
        assert_eq!(app.scroll(), 0);
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('i'));
        assert_eq!(app.scroll(), 2);
        key(&mut app, KeyCode::Home);
        assert_eq!(app.scroll(), 0);
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('b'));
        key(&mut app, KeyCode::Down);
        assert_eq!(app.scroll(), 1);
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(app.scroll(), 0);
    }

    #[test]
    fn preview_is_navigation_only_and_preserves_picker_preview() {
        let mut app = app();
        app.goal = "local goal".into();
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('9'));
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(app.overlay, Some(Overlay::Preview));
        assert_eq!(app.selected, 8);
        assert_eq!(app.preview_form(), 8);
        assert!(!key(&mut app, KeyCode::Enter));
        assert!(!key(&mut app, KeyCode::Char('r')));
        assert_eq!(app.form, 0);
        assert_eq!(app.goal, "local goal");
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.overlay, Some(Overlay::Forms));
        assert_eq!(app.selected, 8);
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(app.overlay, Some(Overlay::Preview));
        assert_eq!(app.preview_form(), 0);
        key(&mut app, KeyCode::Esc);
        modified_key(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
        for _ in 0..5 {
            key(&mut app, KeyCode::Down);
        }
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.overlay, Some(Overlay::Preview));
        assert_eq!(app.preview_form(), 0);
    }

    #[test]
    fn linear_toggle_and_palette_actions_remain_available() {
        let mut app = app();
        key(&mut app, KeyCode::Char('l'));
        assert!(app.linear);
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("linear goal".into()));
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('3'));
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.form, 2);
        key(&mut app, KeyCode::Char('p'));
        key(&mut app, KeyCode::Esc);
        modified_key(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
        for _ in 0..6 {
            key(&mut app, KeyCode::Down);
        }
        key(&mut app, KeyCode::Enter);
        assert!(!app.linear);
        assert_eq!(app.goal, "linear goal");
        app.store_event(store::Event::Saved(committed(&app)));
        key(&mut app, KeyCode::Char('q'));
        assert!(!app.quit);
        assert_eq!(app.confirmation.unwrap().kind, ConfirmationKind::Quit);
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Enter);
        assert!(app.quit);
    }

    #[test]
    fn tiny_terminal_preserves_overlay_back_and_control_c_exit() {
        let mut app = app();
        key(&mut app, KeyCode::Char('?'));
        app.update(Event::Resize(10, 3));
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.overlay, None);
        app.update(Event::Resize(120, 36));
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Resize(10, 3));
        modified_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(app.quit);
    }

    #[test]
    fn dirty_draft_confirmation_restores_invoking_overlay_and_focus() {
        let mut app = app();
        app.goal = "committed".into();
        app.focus = Pane::Inspector;
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('8'));
        modified_key(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        app.update(Event::Paste("\r\n한글".into()));
        key(&mut app, KeyCode::Esc);
        assert_eq!(
            app.confirmation.unwrap().kind,
            ConfirmationKind::DiscardDraft
        );
        assert!(!app.update(Event::Paste("must not enter modal draft".into())));
        key(&mut app, KeyCode::Enter); // Neutral Keep.
        assert_eq!(app.input, "committed\n한글");
        assert!(app.editing);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.goal, "committed\n한글");
        assert_eq!(app.overlay, Some(Overlay::Forms));
        assert_eq!(app.selected, 7);
        assert_eq!(app.focus, Pane::Inspector);
        modified_key(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!(app.selected, 1);
        key(&mut app, KeyCode::Enter);
        app.update(Event::Paste("discard only this".into()));
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Enter);
        assert!(!app.editing);
        assert_eq!(app.input, "");
        assert_eq!(app.goal, "committed\n한글");
        assert_eq!(app.overlay, Some(Overlay::Forms));
        assert_eq!(app.selected, 7);
        assert_eq!(app.focus, Pane::Inspector);
    }

    #[test]
    fn repeat_enter_cannot_approve_or_apply_a_draft() {
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("keep".into()));
        let repeat =
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat);
        assert!(!app.update(Event::Key(repeat)));
        assert_eq!(app.goal, "");
        key(&mut app, KeyCode::Esc);
        assert!(!app.update(Event::Key(repeat)));
        assert!(!app.confirmation.unwrap().destructive);
        key(&mut app, KeyCode::Right);
        assert!(!app.update(Event::Key(repeat)));
        assert_eq!(app.input, "keep");
        key(&mut app, KeyCode::Enter);
        assert!(!app.editing);
        assert_eq!(app.input, "");
    }

    #[test]
    fn quit_protects_goal_and_dirty_draft_even_below_minimum_size() {
        for editing in [false, true] {
            let mut app = app();
            app.goal = "session data".into();
            if editing {
                key(&mut app, KeyCode::Char('g'));
                app.update(Event::Paste("\n未保存".into()));
            }
            let input = app.input.clone();
            let cursor = app.cursor;
            app.update(Event::Resize(1, 1));
            if editing {
                modified_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
            } else {
                key(&mut app, KeyCode::Char('q'));
            }
            assert!(!app.quit);
            assert_eq!(
                app.confirmation,
                Some(Confirmation {
                    kind: ConfirmationKind::Quit,
                    destructive: false
                })
            );
            key(&mut app, KeyCode::Enter);
            assert!(!app.quit);
            assert_eq!(app.input, input);
            assert_eq!(app.cursor, cursor);
            modified_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
            key(&mut app, KeyCode::Esc);
            assert_eq!(app.input, input);
            app.update(Event::Resize(120, 36));
            assert_eq!(app.editing, editing);
            modified_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
            key(&mut app, KeyCode::Right);
            key(&mut app, KeyCode::Enter);
            if editing {
                assert!(app.quit_after_save);
                assert!(!app.quit);
                app.store_event(store::Event::Saved(committed(&app)));
            }
            assert!(app.quit);
        }
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("first unsaved goal".into()));
        modified_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(app.confirmation.unwrap().kind, ConfirmationKind::Quit);
        assert!(!app.quit);
    }

    #[test]
    fn palette_quit_stay_restores_exact_nested_selection() {
        let mut app = app();
        app.goal = "retained goal".into();
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('6'));
        modified_key(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
        key(&mut app, KeyCode::End);
        let palette_selection = app.selected;
        modified_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(app.confirmation.unwrap().kind, ConfirmationKind::Quit);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.overlay, Some(Overlay::Palette));
        assert_eq!(app.selected, palette_selection);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.overlay, Some(Overlay::Forms));
        assert_eq!(app.selected, 5);
        assert!(!app.quit);
    }

    #[test]
    fn vertical_cursor_uses_terminal_cells_and_keeps_column_across_short_lines() {
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("abcd\n界界\nx\nabcd".into()));
        key(&mut app, KeyCode::Home);
        key(&mut app, KeyCode::Up);
        key(&mut app, KeyCode::Up);
        key(&mut app, KeyCode::Right);
        assert_eq!(&app.input[..app.cursor], "abcd\n界");
        key(&mut app, KeyCode::Up);
        assert_eq!(&app.input[..app.cursor], "ab");
        key(&mut app, KeyCode::Down);
        assert_eq!(&app.input[..app.cursor], "abcd\n界");
        key(&mut app, KeyCode::Down);
        assert_eq!(&app.input[..app.cursor], "abcd\n界界\nx");
        key(&mut app, KeyCode::Down);
        assert_eq!(&app.input[app.cursor..], "cd");
    }

    #[test]
    fn paste_limit_warning_is_visible_without_text_change_and_survives_reflow() {
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("a".repeat(MAX_GOAL_BYTES)));
        assert!(!app.input_truncated);
        assert!(app.update(Event::Paste("界\r\nrest".into())));
        assert!(app.input_truncated);
        assert_eq!(app.input, "a".repeat(MAX_GOAL_BYTES));
        app.update(Event::Resize(60, 18));
        assert!(app.input_truncated);
        key(&mut app, KeyCode::Left);
        key(&mut app, KeyCode::Backspace);
        assert!(app.input_truncated);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.goal.len(), MAX_GOAL_BYTES - 1);
        assert!(!app.input_truncated);
    }

    #[test]
    fn rendered_scroll_bounds_prevent_overscroll_and_clamp_after_reflow() {
        let mut app = app();
        assert!(!key(&mut app, KeyCode::Down));
        app.set_scroll_max(ScrollSurface::Workspace, 2);
        key(&mut app, KeyCode::PageDown);
        assert_eq!(app.scroll(), 2);
        assert!(!key(&mut app, KeyCode::Down));
        app.set_scroll_max(ScrollSurface::Workspace, 1);
        assert_eq!(app.scroll(), 1);
        app.set_scroll_max(ScrollSurface::Help, 3);
        key(&mut app, KeyCode::Char('?'));
        key(&mut app, KeyCode::End);
        assert_eq!(app.scroll(), 3);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.scroll(), 1);
        app.set_scroll_max(ScrollSurface::Workspace, 0);
        assert_eq!(app.scroll(), 0);
    }

    #[test]
    fn nested_previews_restore_role_selection_and_hidden_focus_exactly() {
        let mut app = app();
        app.focus = Pane::Inspector;
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('9'));
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(app.preview_form(), 8);
        modified_key(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
        key(&mut app, KeyCode::Enter); // Forms over the first preview.
        key(&mut app, KeyCode::Char('2'));
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(app.preview_form(), 1);
        app.update(Event::Resize(10, 3));
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.overlay, Some(Overlay::Forms));
        assert_eq!(app.selected, 1);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.overlay, Some(Overlay::Preview));
        assert_eq!(app.preview_form(), 8);
        assert_eq!(app.selected, 8);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.overlay, Some(Overlay::Forms));
        assert_eq!(app.selected, 8);
        key(&mut app, KeyCode::Esc);
        app.update(Event::Resize(120, 36));
        assert_eq!(app.effective_focus(), Pane::Inspector);
        assert_eq!(app.form, 0);
    }

    #[test]
    fn repeated_printable_characters_remain_text_while_editing() {
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        for character in ['q', 'c', '界'] {
            let repeat = KeyEvent::new_with_kind(
                KeyCode::Char(character),
                KeyModifiers::NONE,
                KeyEventKind::Repeat,
            );
            assert!(app.update(Event::Key(repeat)));
        }
        assert_eq!(app.input, "qc界");
        assert!(app.confirmation.is_none());
        assert!(!app.quit);
    }

    #[test]
    fn key_release_events_never_edit_or_navigate() {
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        assert!(!app.update(Event::Key(release)));
        assert_eq!(app.input, "");
    }

    #[test]
    fn late_acknowledgement_cannot_save_newer_edit_or_quit_early() {
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("first".into()));
        key(&mut app, KeyCode::Enter);
        let first = app.revision;
        let first_snapshot = committed(&app);
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste(" newer".into()));
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('q'));
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Enter);
        assert!(app.quit_after_save);
        app.store_event(store::Event::Failed {
            revision: Some(first),
            message: "older failure".into(),
        });
        assert_eq!(app.storage, StorageState::Saving);
        assert!(app.quit_after_save);
        app.store_event(store::Event::Saved(first_snapshot));
        assert_eq!(app.storage, StorageState::Saving);
        assert!(!app.quit);
        app.store_event(store::Event::Saved(committed(&app)));
        assert!(app.quit);
        assert_eq!(app.goal, "first newer");
    }

    #[test]
    fn failed_save_retains_goal_and_requires_explicit_unsaved_exit() {
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("recover me".into()));
        key(&mut app, KeyCode::Enter);
        app.store_event(store::Event::Failed {
            revision: Some(app.revision),
            message: "disk full".into(),
        });
        key(&mut app, KeyCode::Char('q'));
        key(&mut app, KeyCode::Enter);
        assert!(!app.quit);
        assert_eq!(app.goal, "recover me");
        key(&mut app, KeyCode::Char('r'));
        assert!(app.retry_storage);
        app.store_event(store::Event::Loaded(store::Project {
            tasks: Vec::new(),
            active_id: None,
            usage_bytes: 0,
        }));
        assert!(app.save_requested);
        assert_eq!(app.goal, "recover me");
    }

    #[test]
    fn startup_load_retains_new_edit_without_overwriting_existing_task() {
        let mut app = App::new("/workspace".into(), 120, 36);
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("new edit".into()));
        key(&mut app, KeyCode::Enter);
        let mut existing = store::Snapshot::empty("/workspace".into());
        existing.id = 7;
        existing.goal = "old goal".into();
        existing.form = 8;
        existing.revision = 40;
        app.store_event(store::Event::Loaded(store::Project {
            tasks: vec![existing],
            active_id: Some(7),
            usage_bytes: 0,
        }));
        assert_eq!(app.goal, "new edit");
        assert_eq!(app.task_id, 0);
        assert!(app.save_requested);
        assert_eq!(app.tasks[0].goal, "old goal");
    }

    #[test]
    fn deleting_another_task_to_free_quota_keeps_the_failed_current_draft() {
        let mut app = app();
        let mut old = store::Snapshot::empty("/workspace".into());
        old.id = 7;
        old.goal = "eligible old task".into();
        app.tasks.push(old);
        app.goal = "new unsaved goal".into();
        app.storage = StorageState::Failed("quota reached".into());
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste(" retained draft".into()));
        modified_key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Delete);
        assert!(matches!(
            app.management.take(),
            Some(store::Management::PreviewDelete { .. })
        ));
        app.store_event(store::Event::DeletePreview {
            targets: vec![store::DeletionTarget {
                task: app.tasks[0].clone(),
                excerpts: Vec::new(),
                runs: Vec::new(),
            }],
            expired_before: None,
        });
        key(&mut app, KeyCode::Enter);
        assert!(app.management.is_none());
        assert_eq!(app.tasks[0].id, 7);
        key(&mut app, KeyCode::Delete);
        assert!(matches!(
            app.management.take(),
            Some(store::Management::PreviewDelete { .. })
        ));
        app.store_event(store::Event::DeletePreview {
            targets: vec![store::DeletionTarget {
                task: app.tasks[0].clone(),
                excerpts: Vec::new(),
                runs: Vec::new(),
            }],
            expired_before: None,
        });
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Enter);
        assert!(
            matches!(app.management.take(), Some(store::Management::Delete { ids, expired_before: None }) if ids == [7])
        );
        app.store_event(store::Event::Managed {
            project: store::Project {
                tasks: Vec::new(),
                active_id: None,
                usage_bytes: 8192,
            },
            affected: vec![7],
            warning: None,
        });
        assert_eq!(app.task_id, 0);
        assert_eq!(app.goal, "new unsaved goal");
        assert_eq!(app.drafts[0].text, "new unsaved goal retained draft");
        assert!(matches!(app.storage, StorageState::Failed(_)));
        assert!(app.save_requested && !app.transitioning);
        key(&mut app, KeyCode::Char('r'));
        assert!(app.retry_storage);
    }

    #[test]
    fn archived_view_restores_instead_of_resuming_hidden_tasks() {
        let mut app = app();
        let mut archived = store::Snapshot::empty("/workspace".into());
        archived.id = 8;
        archived.archived = true;
        archived.archived_at = 1;
        app.tasks.push(archived);
        key(&mut app, KeyCode::Char('t'));
        assert_eq!(app.visible_tasks().count(), 0);
        key(&mut app, KeyCode::Char('h'));
        key(&mut app, KeyCode::Enter);
        assert!(app.switch_task.is_none());
        key(&mut app, KeyCode::Char('r'));
        assert!(matches!(
            app.management,
            Some(store::Management::Archive {
                id: 8,
                archived: false
            })
        ));
        assert_eq!(app.task_id, 0);
    }
}
