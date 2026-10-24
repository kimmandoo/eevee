//! Project/partner/form configuration is local advice, never packet membership.
use std::{
    cell::{Cell, OnceCell},
    path::{Path, PathBuf},
};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{App, Overlay, StorageState, commands::Choice};
use crate::{
    providers::Input,
    store::{self, records::Operation},
    workspace::{
        context,
        held::{FileProof, Renewal, Target},
        memory,
    },
};

/// Cached local observation only; never submission or dispatch authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    NotConfigured,
    Available,
    Unavailable,
    Unknown,
}

pub struct Review {
    pub intent: Renewal,
    pub task: i64,
    pub old_content: Option<String>,
    pub new_content: String,
    pub scroll: u16,
    pub scroll_max: Cell<u16>,
    pub reviewed: Cell<bool>,
    pub geometry: Cell<(u16, u16)>,
    pub details: OnceCell<String>,
}

pub struct Inspection {
    pub owner: store::Attribution,
    pub target: Option<Target>,
    pub content: Option<String>,
    pub return_to: Option<Overlay>,
    pub scroll: u16,
    pub details: OnceCell<String>,
    pub readiness: Readiness,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Empty,
    Saved,
    Proposed,
    Included,
    Unavailable,
    Dismissed,
    Saving,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    ChooseFile,
    ChooseMemory,
    Saved,
    Waiting,
    Busy,
    Unavailable,
    ReviewFile,
    Accepted,
    Dismissed,
    NoProposal,
    QueueFull,
    TooSmall,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub task: i64,
    pub owner: store::Attribution,
    pub target: Option<Target>,
    pub included: bool,
}
#[derive(Default)]
pub struct State {
    pub target: Option<Target>,
    pub dismissed: bool,
    pub notice: Option<Notice>,
    pub receipt: Option<Receipt>,
    /// The source worker and admission boundary receive this same startup policy.
    pub exclusions: Vec<PathBuf>,
    visit: Option<(i64, i64, usize)>,
    pub renewal: Option<Review>,
    pub inspection: Option<Inspection>,
    proposed: bool,
    accepted_memory: Option<(i64, i64, u64)>,
    memory_observation: Option<(i64, String, bool)>,
}

pub const COMMANDS: &[(&str, &str)] = &[
    ("held", "Inspect held item and local request receipt"),
    ("held-file", "Choose an eligible file to hold"),
    (
        "held-set-file",
        "Save reviewed file preference without attaching",
    ),
    ("held-memory", "Choose an approved memory to hold"),
    (
        "held-set-memory",
        "Save highlighted exact memory preference without selecting",
    ),
    ("held-accept", "Explicitly review and accept held context"),
    ("held-dismiss", "Dismiss held proposal for this visit"),
    ("held-clear", "Clear saved held preference only"),
    (
        "held-renew",
        "Choose a replacement, then explicitly review renewal",
    ),
];

fn contains(target: &Target, input: &Input) -> bool {
    match target {
        Target::File { path } => input.excerpts.iter().any(|row| {
            row.available
                && row.task_id == input.task.id
                && row.root == input.task.project
                && &row.path == path
        }),
        Target::Memory { id, revision } => input.task.approved_memories.iter().any(|row| {
            row.id == *id
                && row.revision == *revision
                && memory::visible(row, &input.task.project, input.task.id)
        }),
    }
}

impl App {
    /// Runs on actual ownership/record transitions, not as a rendering side effect.
    pub fn reconcile_held(&mut self) -> bool {
        if !self.records.loaded {
            return false;
        }
        let visit = (self.task_id, self.lead().id, self.form());
        let target = self
            .records
            .snapshot
            .held_items
            .iter()
            .find(|item| item.partner == visit.1 && item.form == visit.2)
            .map(|item| item.target.clone());
        let entered = self.held.visit != Some(visit);
        let changed = entered || self.held.target != target;
        if entered {
            self.held.visit = Some(visit);
            self.held.dismissed = false;
            self.held.proposed = target.is_some();
            self.held.notice = None;
        } else if self.held.target != target {
            // A committed configuration change is saved, not an implicit proposal
            // acceptance. A refresh must never resurrect a dismissed visit.
            self.held.proposed = false;
        }
        self.held.target = target;
        if let Some((task, id, revision)) = self.held.accepted_memory {
            if task != self.task_id || !self.memory.selected_ids.contains(&id) {
                self.held.accepted_memory = None;
            } else if !self.memory.records.iter().any(|row| {
                row.id == id && row.revision == revision && memory::visible(row, &self.cwd, task)
            }) && self.memory.selected_ids.contains(&id)
                && !self.memory_loading
            {
                // Do not silently adopt an edited revision through a formerly
                // accepted held reference. Existing frozen requests stay untouched.
                self.memory.selected_ids.retain(|selected| *selected != id);
                self.held.accepted_memory = None;
                self.held.notice = Some(Notice::Unavailable);
                self.changed_task();
                return true;
            }
        }
        changed
    }

    pub(super) fn held_request_owned(&self, request: &store::records::Request) -> bool {
        request.task_id == self.task_id
            && match &request.operation {
                Operation::SetHeld { partner, form, .. } => {
                    *partner == self.lead().id && *form == self.form()
                }
                Operation::RenewHeld { review } => {
                    review.owner.partner == self.lead().id && review.owner.form == self.form()
                }
                _ => false,
            }
    }

    pub fn held_status(&self) -> Status {
        if self
            .records
            .failed
            .as_ref()
            .is_some_and(|request| self.held_request_owned(request))
        {
            return Status::Failed;
        }
        if self
            .records
            .pending
            .iter()
            .chain(self.records.inflight.iter())
            .chain(self.records.queued.iter())
            .any(|request| self.held_request_owned(request))
        {
            return Status::Saving;
        }
        let Some(target) = &self.held.target else {
            return Status::Empty;
        };
        if !self.held_available(target) {
            return Status::Unavailable;
        }
        let included = match target {
            Target::File { path } => self
                .sources
                .basket
                .iter()
                .any(|row| row.available && &row.path == path),
            Target::Memory { id, revision } => {
                self.memory.selected_ids.contains(id)
                    && self
                        .memory
                        .records
                        .iter()
                        .any(|row| row.id == *id && row.revision == *revision)
            }
        };
        if included {
            return Status::Included;
        }
        if self.held.dismissed {
            Status::Dismissed
        } else if self.held.proposed {
            Status::Proposed
        } else {
            Status::Saved
        }
    }

    pub fn held_target_label(&self) -> String {
        match &self.held.target {
            Some(Target::File { path }) => path.clone(),
            Some(Target::Memory { id, revision }) => format!("#{id} @{revision}"),
            None => String::new(),
        }
    }

    fn held_available(&self, target: &Target) -> bool {
        match target {
            Target::File { path } => {
                !context::excluded(Path::new(path), &self.held.exclusions)
                    && self.repository.as_ref().is_some_and(|report| {
                        !report.repository.excludes_source(Path::new(path))
                            && report
                                .current
                                .files
                                .get(Path::new(path))
                                .is_some_and(|file| {
                                    matches!(
                                        &file.state,
                                        crate::workspace::repo::FileState::Regular {
                                            binary: false,
                                            ..
                                        }
                                    )
                                })
                    })
            }
            Target::Memory { id, revision } => {
                !self.memory_loading
                    && self.memory.records.iter().any(|row| {
                        row.id == *id
                            && row.revision == *revision
                            && memory::validate_record(row).is_ok()
                            && memory::visible(row, &self.cwd, self.task_id)
                    })
            }
        }
    }

    pub fn held_memory_result(&mut self, result: &Result<Vec<memory::Record>, String>) {
        let valid = result.as_ref().is_ok_and(|records| {
            records.len() <= memory::MAX_RECORDS
                && records
                    .iter()
                    .all(|record| memory::validate_record(record).is_ok())
        });
        self.held.memory_observation = Some((self.task_id, self.cwd.clone(), valid));
    }

    pub fn held_reference_readiness(&self, partner: i64, form: usize) -> Readiness {
        if !self.records.current_for(self.task_id, &self.cwd) {
            return Readiness::Unknown;
        }
        let Some(item) = self
            .records
            .snapshot
            .held_items
            .iter()
            .find(|item| item.partner == partner && item.form == form)
        else {
            return Readiness::NotConfigured;
        };
        match &item.target {
            Target::Memory { .. }
                if self.memory_loading
                    || self.memory.task_id != self.task_id
                    || self.memory.project != self.cwd
                    || !self.held.memory_observation.as_ref().is_some_and(
                        |(task, project, valid)| {
                            *task == self.task_id && project == &self.cwd && *valid
                        },
                    ) =>
            {
                Readiness::Unknown
            }
            Target::File { path } if context::excluded(Path::new(path), &self.held.exclusions) => {
                Readiness::Unavailable
            }
            Target::File { .. }
                if self.repo_loading
                    || self.repo_refresh
                    || !self.repo_error.is_empty()
                    || self.repository.as_ref().is_none_or(|report| {
                        !report.current.complete || !report.current.stable
                    }) =>
            {
                Readiness::Unknown
            }
            target if self.held_available(target) => Readiness::Available,
            _ => Readiness::Unavailable,
        }
    }

    fn held_retained_content(&self, target: &Target) -> Option<String> {
        match target {
            Target::Memory { id, revision } => self
                .memory
                .records
                .iter()
                .find(|record| {
                    record.id == *id
                        && record.revision == *revision
                        && memory::visible(record, &self.cwd, self.task_id)
                        && memory::validate_record(record).is_ok()
                })
                .map(|record| record.proposal.content.clone()),
            // A retained capture is inspectable, not claimed as saved-file bytes.
            Target::File { path } => self
                .sources
                .basket
                .iter()
                .find(|capture| {
                    capture.available
                        && capture.task_id == self.task_id
                        && capture.root == self.cwd
                        && &capture.path == path
                })
                .map(|capture| capture.text.clone()),
        }
    }

    pub fn held_inspection_active(&self) -> bool {
        self.overlay == Some(Overlay::Held) && self.held.inspection.is_some()
    }

    pub fn held_inspection_key(&mut self, key: KeyEvent) -> bool {
        if !self.held_inspection_active() {
            return false;
        }
        self.held_key(key);
        true
    }

    pub fn inspect_held_reference(&mut self, partner: i64, form: usize) -> bool {
        if form >= 9 {
            return false;
        }
        let Some(member) = std::iter::once(&self.lead)
            .chain(self.party.iter())
            .find(|member| member.id == partner)
        else {
            return false;
        };
        let owner = store::Attribution {
            form,
            ..store::Attribution::of(member)
        };
        let target = self
            .records
            .current_for(self.task_id, &self.cwd)
            .then(|| {
                self.records
                    .snapshot
                    .held_items
                    .iter()
                    .find(|item| item.partner == partner && item.form == form)
                    .map(|item| item.target.clone())
            })
            .flatten();
        let content = target
            .as_ref()
            .and_then(|target| self.held_retained_content(target));
        let return_to = self.overlay;
        let readiness = self.held_reference_readiness(partner, form);
        self.open_overlay(Overlay::Held);
        self.held.renewal = None;
        self.held.inspection = Some(Inspection {
            owner,
            target,
            content,
            return_to,
            scroll: 0,
            details: OnceCell::new(),
            readiness,
        });
        true
    }

    pub fn invalidate_held_review(&mut self) {
        self.held.renewal = None;
        self.held.inspection = None;
    }

    fn review_held_renewal(&mut self, proposed: Target, file_proof: Option<FileProof>) {
        let Some(expected) = self.held.target.clone() else {
            self.held_queue(Some(proposed), file_proof);
            return;
        };
        if expected == proposed {
            self.held_queue(Some(proposed), file_proof);
            return;
        }
        let old_content = self.held_retained_content(&expected);
        let new_content = match &file_proof {
            Some(_) => String::new(),
            None => self.held_retained_content(&proposed).unwrap_or_default(),
        };
        let navigation = self.navigation();
        self.open_overlay(Overlay::Held);
        if navigation != self.navigation() {
            // Own this staging event's persisted navigation before freezing the
            // intent. update_at must not count it a second time after dispatch.
            self.changed_task();
        }
        let review = Review {
            task: self.task_id,
            intent: Renewal {
                expected,
                proposed,
                owner: self.attribution(),
                task_revision: self.revision,
                exclusions: self.held.exclusions.clone(),
                file_proof,
            },
            old_content,
            new_content,
            scroll: 0,
            scroll_max: Cell::new(0),
            reviewed: Cell::new(false),
            geometry: Cell::new((0, 0)),
            details: OnceCell::new(),
        };
        // The frozen revision includes only the staging transition above.
        self.held.inspection = None;
        self.held.renewal = Some(review);
    }

    /// Local configuration UI readiness, never submission/dispatch authority.
    pub fn held_renewal_can_confirm(&self) -> bool {
        !self.held_blocked()
    }

    fn confirm_held_renewal(&mut self) {
        let Some(review) = self.held.renewal.as_ref() else {
            return;
        };
        let current = self.records.snapshot.held_items.iter().find(|item| {
            item.partner == review.intent.owner.partner && item.form == review.intent.owner.form
        });
        if !review.reviewed.get() || self.width < 60 || self.height < 18 {
            return;
        }
        if review.task != self.task_id
            || review.intent.owner != self.attribution()
            || review.intent.task_revision != self.revision
            || review.intent.exclusions != self.held.exclusions
            || current.map(|item| &item.target) != Some(&review.intent.expected)
            || !self.held_available(&review.intent.proposed)
        {
            self.held.renewal = None;
            self.held.notice = Some(Notice::Unavailable);
            return;
        }
        if self.held_blocked() {
            self.held.notice = Some(Notice::Busy);
            return;
        }
        let intent = self.held.renewal.take().unwrap().intent;
        let queued =
            self.queue_record_operation(self.task_id, Operation::RenewHeld { review: intent });
        self.held.notice = Some(if queued {
            Notice::Waiting
        } else {
            Notice::QueueFull
        });
    }

    fn held_blocked(&self) -> bool {
        self.task_id <= 0
            || self.archived
            || !self.records.loaded
            || self.records.persistence_pending()
            || self.records.mutation_pending()
            || self.sources.busy()
            || self.sources.store_failed
            || self.memory_loading
            || self.memory.busy()
            || self.pending_request.is_some()
            || self.planning.owns_work()
            || self.partner_busy(self.lead().id)
            || self.partner_approval_pending(self.lead().id)
            || self.execution_any_pending()
            || self.transitioning
            || self.quit_after_save
            || self.reset_after_save
            || self.confirmation.is_some()
            || !matches!(self.storage, StorageState::Saved)
            || self.saved_revision != self.revision
            || self.save_requested
    }

    pub fn held_command_choices(&self) -> Vec<Choice> {
        COMMANDS
            .iter()
            .map(|(command, label)| Choice::new(command, *label))
            .collect()
    }

    fn held_queue(&mut self, target: Option<Target>, file_proof: Option<FileProof>) {
        let queued = self.queue_record_operation(
            self.task_id,
            Operation::SetHeld {
                partner: self.lead().id,
                form: self.form(),
                target,
                file_proof,
            },
        );
        self.held.notice = Some(if queued {
            Notice::Waiting
        } else {
            Notice::QueueFull
        });
        self.open_overlay(Overlay::Held);
    }

    pub fn held_command(&mut self, command: &str) -> bool {
        let command = command.trim_start_matches('/');
        if !COMMANDS.iter().any(|(name, _)| *name == command) {
            return false;
        }
        self.reconcile_held();
        if self.held.inspection.is_some() && command != "held" {
            return true;
        }
        if command != "held" {
            self.held.renewal = None;
        }
        if command == "held" {
            if self.editing {
                self.retain_action_editor();
            }
            self.open_overlay(Overlay::Held);
            return true;
        }
        if command == "held-dismiss" {
            // Visit-only chrome is not a context mutation or permission action.
            self.held.dismissed = true;
            self.held.proposed = false;
            self.held.notice = Some(Notice::Dismissed);
            return true;
        }
        if self.width < 60 || self.height < 18 {
            self.held.notice = Some(Notice::TooSmall);
            return true;
        }
        if self.held_blocked() {
            self.held.notice = Some(Notice::Busy);
            self.open_overlay(Overlay::Held);
            return true;
        }
        match command {
            "held-renew" => {
                if matches!(self.held.target, Some(Target::Memory { .. })) {
                    self.held.notice = Some(Notice::ChooseMemory);
                    self.open_overlay(Overlay::Memory);
                } else {
                    self.held.notice = Some(Notice::ChooseFile);
                    self.open_overlay(Overlay::Sources);
                }
            }
            "held-file" => {
                if self.editing {
                    self.retain_action_editor();
                }
                self.held.notice = Some(Notice::ChooseFile);
                self.open_overlay(Overlay::Sources);
            }
            "held-memory" => {
                if self.editing {
                    self.retain_action_editor();
                }
                self.held.notice = Some(Notice::ChooseMemory);
                self.open_overlay(Overlay::Memory);
            }
            "held-set-file" => {
                let preview = self.sources.preview.as_ref().filter(|preview| {
                    self.overlay == Some(Overlay::Sources)
                        && self.sources.mode == super::sources::Mode::Preview
                        && self.sources.preview_reviewed
                        && self.sources.editor.is_none()
                        && self.sources.confirmation.is_none()
                        && preview.task_id == self.task_id
                        && preview.available
                        && preview.validate().is_ok()
                        && !context::excluded(Path::new(&preview.path), &self.held.exclusions)
                        && self.repository.as_ref().is_some_and(|report| {
                            Path::new(&preview.root) == report.repository.root
                                && !report.repository.excludes_source(Path::new(&preview.path))
                        })
                });
                if let Some(preview) = preview {
                    let target = Target::File {
                        path: preview.path.clone(),
                    };
                    let proof = FileProof::new(preview.clone(), self.held.exclusions.clone());
                    self.review_held_renewal(target, Some(proof));
                } else {
                    self.held.notice = Some(Notice::ReviewFile);
                    self.open_overlay(Overlay::Held);
                }
            }
            "held-set-memory" => {
                let record = self.memory.selected_record().filter(|row| {
                    self.overlay == Some(Overlay::Memory)
                        && memory::validate_record(row).is_ok()
                        && memory::visible(row, &self.cwd, self.task_id)
                });
                if let Some(record) = record {
                    self.review_held_renewal(
                        Target::Memory {
                            id: record.id,
                            revision: record.revision,
                        },
                        None,
                    );
                } else {
                    self.held.notice = Some(Notice::ChooseMemory);
                    self.open_overlay(Overlay::Held);
                }
            }
            "held-accept" => {
                let Some(target) = self.held.target.clone() else {
                    self.held.notice = Some(Notice::NoProposal);
                    return true;
                };
                if !self.held_available(&target) {
                    self.held.notice = Some(Notice::Unavailable);
                    return true;
                }
                if self.editing {
                    self.retain_action_editor();
                }
                match target {
                    Target::File { path } => {
                        let Some(report) = self.repository.clone() else {
                            self.held.notice = Some(Notice::Unavailable);
                            return true;
                        };
                        self.sources.preview = None;
                        self.sources.preview_reviewed = false;
                        if self
                            .sources
                            .begin_capture(
                                self.task_id,
                                context::Operation::InspectFile {
                                    report,
                                    path: PathBuf::from(path),
                                },
                            )
                            .is_err()
                        {
                            self.held.notice = Some(Notice::Busy);
                            return true;
                        }
                        // Opening Sources sees the already-owned inspection and
                        // cannot replace it with a catalogue request.
                        self.open_overlay(Overlay::Sources);
                        self.held.notice = Some(Notice::ReviewFile);
                    }
                    Target::Memory { id, revision } => {
                        let Some(index) = self
                            .memory
                            .records
                            .iter()
                            .position(|row| row.id == id && row.revision == revision)
                        else {
                            self.held.notice = Some(Notice::Unavailable);
                            return true;
                        };
                        self.memory.selected = index;
                        if !self.memory.selected_ids.contains(&id) {
                            self.memory_command("select-memory");
                        }
                        if self.memory.selected_ids.contains(&id) {
                            self.held.accepted_memory = Some((self.task_id, id, revision));
                            self.held.dismissed = false;
                            self.held.notice = Some(Notice::Accepted);
                        } else {
                            self.held.notice = Some(Notice::Unavailable);
                        }
                    }
                }
            }
            "held-clear" => self.held_queue(None, None),
            _ => {}
        }
        true
    }

    pub fn held_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return false;
        }
        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
            self.request_quit();
            return true;
        }
        if (self.held.inspection.is_some() || self.held.renewal.is_some())
            && !key.modifiers.is_empty()
        {
            return true;
        }
        if self.held.inspection.is_some() {
            match key.code {
                KeyCode::Esc => {
                    let return_to = self
                        .held
                        .inspection
                        .take()
                        .and_then(|scope| scope.return_to);
                    if let Some(overlay) = return_to {
                        self.open_overlay(overlay);
                    } else {
                        self.close_overlay();
                    }
                }
                KeyCode::Down | KeyCode::PageDown => {
                    let scope = self.held.inspection.as_mut().unwrap();
                    scope.scroll =
                        scope
                            .scroll
                            .saturating_add(if key.code == KeyCode::Down { 1 } else { 8 });
                }
                KeyCode::Up | KeyCode::PageUp => {
                    let scope = self.held.inspection.as_mut().unwrap();
                    scope.scroll =
                        scope
                            .scroll
                            .saturating_sub(if key.code == KeyCode::Up { 1 } else { 8 });
                }
                _ => {}
            }
            return true;
        }
        if self.held.renewal.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Enter => {
                    self.held.renewal = None;
                }
                KeyCode::Char('y') => self.confirm_held_renewal(),
                KeyCode::Down | KeyCode::PageDown => {
                    let review = self.held.renewal.as_mut().unwrap();
                    review.scroll = review
                        .scroll
                        .saturating_add(if key.code == KeyCode::Down { 1 } else { 8 })
                        .min(review.scroll_max.get());
                }
                KeyCode::Up | KeyCode::PageUp => {
                    let review = self.held.renewal.as_mut().unwrap();
                    review.scroll =
                        review
                            .scroll
                            .saturating_sub(if key.code == KeyCode::Up { 1 } else { 8 });
                }
                _ => {}
            }
            return true;
        }
        if key.modifiers.is_empty() {
            match key.code {
                KeyCode::Esc => self.close_overlay(),
                KeyCode::Char('f') => {
                    self.held_command("held-file");
                }
                KeyCode::Char('m') => {
                    self.held_command("held-memory");
                }
                KeyCode::Char('a') | KeyCode::Enter => {
                    self.held_command("held-accept");
                }
                KeyCode::Char('d') | KeyCode::Delete => {
                    self.held_command("held-dismiss");
                }
                KeyCode::Char('c') => {
                    self.held_command("held-clear");
                }
                _ => return false,
            }
            return true;
        }
        false
    }

    /// Immutable, process-local attribution to the actual frozen ordinary input.
    /// No held labels, configuration or receipts enter provider/handoff data.
    pub fn freeze_held_receipt(&mut self, input: &Input) {
        if input.inspect_models || input.inspect_sessions {
            return;
        }
        let owner = input
            .work
            .as_ref()
            .map(|claim| claim.owner.clone())
            .unwrap_or_else(|| {
                let partner = if self.lead.id == input.task.lead_partner {
                    Some(&self.lead)
                } else {
                    self.party
                        .iter()
                        .find(|partner| partner.id == input.task.lead_partner)
                };
                let mut owner =
                    partner
                        .map(store::Attribution::of)
                        .unwrap_or_else(|| store::Attribution {
                            partner: input.task.lead_partner,
                            name: format!("Eevee #{}", input.task.lead_partner),
                            form: input.task.form,
                        });
                owner.form = input.task.form;
                owner
            });
        let target = self
            .records
            .snapshot
            .held_items
            .iter()
            .find(|item| item.partner == owner.partner && item.form == owner.form)
            .map(|item| item.target.clone());
        let included = target
            .as_ref()
            .is_some_and(|target| contains(target, input));
        self.held.receipt = Some(Receipt {
            task: input.task.id,
            owner,
            target,
            included,
        });
    }
}

#[cfg(test)]
mod tests;
