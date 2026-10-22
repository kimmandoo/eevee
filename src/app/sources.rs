use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

use crate::{store, workspace::context, workspace::repo};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Files,
    Basket,
    Preview,
    Payload,
}
#[derive(Clone, Copy)]
pub enum EditKind {
    Search,
    Range,
}
pub struct Editor {
    pub kind: EditKind,
    pub text: String,
    pub cursor: usize,
}
#[derive(Clone, Copy)]
pub struct Confirmation {
    pub id: i64,
    pub purge: bool,
    pub destructive: bool,
    pub reviewed: bool,
}
pub enum Action {
    Changed,
    Close,
    RefreshRepository,
    Quit,
    Scroll(KeyCode),
}
#[derive(Clone, Copy)]
enum Mutation {
    Pin,
    Remove(i64),
    Purge(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RankReason {
    Failure,
    Changed,
    Tests,
    ChangedTests,
}

#[derive(Default)]
struct Rankings {
    changed: std::collections::BTreeSet<std::path::PathBuf>,
    failures: std::collections::BTreeSet<std::path::PathBuf>,
    orders: [Vec<usize>; 9],
    reasons: [Vec<Option<RankReason>>; 9],
}

#[derive(Default)]
pub struct State {
    pub mode: Mode,
    pub files: Vec<context::Source>,
    pub file_selection: usize,
    pub basket: Vec<context::Excerpt>,
    pub basket_selection: usize,
    pub preview: Option<context::Excerpt>,
    pub payload: Option<context::Payload>,
    pub detail: String,
    pub query: String,
    pub status: String,
    pub error: String,
    pub editor: Option<Editor>,
    pub confirmation: Option<Confirmation>,
    pub pending: Option<context::Request>,
    pub pending_store: Option<store::ContextRequest>,
    pub source_busy: bool,
    pub store_busy: bool,
    pub store_failed: bool,
    pub await_refresh: bool,
    pub preview_reviewed: bool,
    pub return_after_attach: bool,
    return_to_composer: bool,
    pending_payload: Option<context::Payload>,
    store_mutation: bool,
    last_mutation: Option<Mutation>,
    payload_page: usize,
    pending_origin_run: Option<String>,
    producer: Option<store::Attribution>,
    capture_producer: Option<store::Attribution>,
    pub chip_focus: Option<i64>,
    pub observations: Vec<context::Observation>,
    pub wire: Option<context::WireReceipt>,
    rankings: Rankings,
}

impl State {
    /// Updates only the next capture's producer, never an in-flight capture.
    pub fn set_producer(&mut self, producer: Option<store::Attribution>) {
        self.producer = producer;
    }

    /// Shared initiation hook for move/workflow captures. Freezes attribution
    /// before the request can leave the UI, including backpressure and retry.
    pub fn begin_capture(
        &mut self,
        task_id: i64,
        operation: context::Operation,
    ) -> Result<(), String> {
        if self.busy() || self.store_failed {
            return Err("Finish or retry the pending context operation first.".into());
        }
        if !matches!(
            &operation,
            context::Operation::Inspect { .. } | context::Operation::InspectFile { .. }
        ) {
            return Err("Capture initiation requires an exact source inspection.".into());
        }
        self.queue(task_id, operation);
        Ok(())
    }

    pub fn set_rank_facts(
        &mut self,
        changed: std::collections::BTreeSet<std::path::PathBuf>,
        failures: std::collections::BTreeSet<std::path::PathBuf>,
    ) {
        self.rankings.changed = changed;
        self.rankings.failures = failures;
        self.rebuild_rankings();
    }

    pub fn set_failure_paths(&mut self, failures: impl IntoIterator<Item = std::path::PathBuf>) {
        self.rankings.failures = failures.into_iter().take(10_000).collect();
        self.rebuild_rankings();
    }

    fn rebuild_rankings(&mut self) {
        for reasons in &mut self.rankings.reasons {
            reasons.clear();
            reasons.resize(self.files.len(), None);
        }
        let changed_stems: std::collections::BTreeSet<_> = self
            .rankings
            .changed
            .iter()
            .filter_map(|path| source_stem(path))
            .collect();
        for (index, file) in self.files.iter().enumerate() {
            let changed = self.rankings.changed.contains(&file.path);
            let failed = self.rankings.failures.contains(&file.path);
            let tests = test_path(&file.path)
                && source_stem(&file.path).is_some_and(|stem| changed_stems.contains(stem));
            if failed {
                self.rankings.reasons[5][index] = Some(RankReason::Failure);
            }
            if changed {
                self.rankings.reasons[3][index] = Some(RankReason::Changed);
                self.rankings.reasons[6][index] = Some(RankReason::Changed);
            }
            self.rankings.reasons[7][index] = match (changed, tests) {
                (true, true) => Some(RankReason::ChangedTests),
                (true, false) => Some(RankReason::Changed),
                (false, true) => Some(RankReason::Tests),
                _ => None,
            };
        }
        for form in 0..9 {
            let order = &mut self.rankings.orders[form];
            order.clear();
            order.extend(0..self.files.len());
            order.sort_unstable_by_key(|index| {
                (
                    match self.rankings.reasons[form][*index] {
                        Some(RankReason::ChangedTests) => 0,
                        Some(RankReason::Failure | RankReason::Changed) => 1,
                        Some(RankReason::Tests) => 2,
                        None => 3,
                    },
                    *index,
                )
            });
        }
    }

    pub fn ranked_files(&self, form: usize) -> impl Iterator<Item = usize> + '_ {
        let order = &self.rankings.orders[form.min(8)];
        // Test fixtures and a newly hydrated catalogue can precede rank facts.
        (0..self.files.len()).map(move |position| order.get(position).copied().unwrap_or(position))
    }

    pub fn rank_reason(&self, form: usize, index: usize) -> Option<RankReason> {
        self.rankings.reasons[form.min(8)]
            .get(index)
            .copied()
            .flatten()
    }

    pub fn observe_captures(&mut self, task_id: i64, report: Option<&Arc<repo::Report>>) {
        if self.busy() || self.store_failed || self.basket.is_empty() {
            return;
        }
        self.observations.clear();
        if let Some(report) = report {
            self.queue(
                task_id,
                context::Operation::Check {
                    report: Arc::clone(report),
                    excerpts: self.basket.clone(),
                },
            );
        } else {
            self.observations = self
                .basket
                .iter()
                .map(|capture| context::Observation {
                    id: capture.id,
                    freshness: context::Freshness::Unavailable(
                        "Repository evidence unavailable".into(),
                    ),
                })
                .collect();
        }
    }

    /// Call only with the worker-produced immutable request preview.
    pub fn set_wire_request(&mut self, payload: &str) -> Result<(), String> {
        self.wire = None;
        self.wire = Some(context::wire_receipt(payload)?);
        Ok(())
    }

    pub fn clear_wire_request(&mut self) {
        self.wire = None;
    }

    pub fn focus_chip(&mut self, reverse: bool) -> bool {
        if self.basket.is_empty() {
            self.chip_focus = None;
            return false;
        }
        let current = self
            .chip_focus
            .and_then(|id| self.basket.iter().position(|row| row.id == id));
        let index = match current {
            Some(index) if reverse => (index + self.basket.len() - 1) % self.basket.len(),
            Some(index) => (index + 1) % self.basket.len(),
            None if reverse => self.basket.len() - 1,
            None => 0,
        };
        self.chip_focus = Some(self.basket[index].id);
        true
    }

    /// Exact capture lookup: distinct ranges in one file are independent chips.
    pub fn inspect_chip(&mut self, id: i64, remove: bool, task_id: i64) -> bool {
        if self.busy() || self.store_failed {
            return false;
        }
        let Some(index) = self.basket.iter().position(|capture| capture.id == id) else {
            return false;
        };
        self.basket_selection = index;
        self.mode = Mode::Basket;
        self.basket_detail();
        if remove {
            self.command("unpin", task_id, None);
        }
        true
    }

    pub fn show_basket(&mut self) {
        self.mode = Mode::Basket;
        self.basket_detail();
    }

    pub fn reset(&mut self, task_id: i64) {
        let producer = self.producer.take();
        *self = Self {
            producer,
            ..Self::default()
        };
        if task_id > 0 {
            self.queue_store(task_id, store::ContextOperation::Load);
        }
    }
    pub fn invalidate_review(&mut self) {
        self.preview_reviewed = false;
        if let Some(confirmation) = &mut self.confirmation {
            confirmation.reviewed = false;
            confirmation.destructive = false;
        }
    }
    pub fn busy(&self) -> bool {
        self.source_busy || self.store_busy || self.await_refresh
    }
    pub fn persistence_pending(&self) -> bool {
        self.store_busy && self.store_mutation
    }
    pub fn take_return_to_composer(&mut self) -> bool {
        std::mem::take(&mut self.return_to_composer)
    }
    pub fn file_attachment(&self, index: usize) -> Option<usize> {
        let source = self.files.get(index)?;
        self.basket
            .iter()
            .position(|excerpt| std::path::Path::new(&excerpt.path) == source.path)
    }
    pub fn catalogue(
        &mut self,
        task_id: i64,
        report: Option<&Arc<repo::Report>>,
        repository_pending: bool,
    ) {
        if self.store_failed {
            self.error =
                "Context save failed. Open /files and press Enter to retry the retained operation."
                    .into();
            return;
        }
        if task_id <= 0 {
            self.error = "Create and save a task before selecting sources.".into();
            return;
        }
        let Some(report) = report else {
            // Missing evidence is not itself an outstanding inspection. Only
            // wait when the root actually owns a queued or loading refresh.
            self.await_refresh = repository_pending;
            if repository_pending {
                self.error.clear();
                self.status =
                    "Waiting for repository evidence; /repo shows its error layer.".into();
            } else {
                self.status =
                    "Repository evidence unavailable; /repo shows its error layer.".into();
                self.error =
                    "Repository inspection unavailable. /repo shows details; /refresh retries."
                        .into();
            }
            return;
        };
        if self.source_busy || self.persistence_pending() {
            return;
        }
        self.preview = None;
        self.preview_reviewed = false;
        self.return_after_attach = false;
        self.return_to_composer = false;
        self.payload = None;
        self.detail.clear();
        self.mode = Mode::Files;
        self.queue(
            task_id,
            context::Operation::Catalogue {
                report: Arc::clone(report),
                query: self.query.clone(),
            },
        );
    }
    fn queue(&mut self, task_id: i64, operation: context::Operation) {
        if matches!(
            &operation,
            context::Operation::Inspect { .. } | context::Operation::InspectFile { .. }
        ) {
            self.capture_producer.clone_from(&self.producer);
        }
        self.pending = Some(context::Request { task_id, operation });
        self.source_busy = true;
        self.error.clear();
        self.status = "Reading and validating selected source evidence…".into();
    }
    fn queue_store(&mut self, task_id: i64, operation: store::ContextOperation) {
        self.store_mutation = matches!(
            &operation,
            store::ContextOperation::Pin(_)
                | store::ContextOperation::Remove(_)
                | store::ContextOperation::Purge(_)
        );
        if self.store_mutation {
            self.last_mutation = match &operation {
                store::ContextOperation::Pin(_) => Some(Mutation::Pin),
                store::ContextOperation::Remove(id) => Some(Mutation::Remove(*id)),
                store::ContextOperation::Purge(ids) => ids.first().copied().map(Mutation::Purge),
                _ => None,
            };
        }
        let producer = matches!(&operation, store::ContextOperation::Pin(_))
            .then(|| self.capture_producer.clone())
            .flatten();
        self.pending_store = Some(store::ContextRequest {
            task_id,
            producer,
            operation,
        });
        self.store_busy = true;
        self.error.clear();
    }
    pub fn observed_repository(&mut self, task_id: i64, report: Option<&Arc<repo::Report>>) {
        let changed = report
            .map(|report| {
                report
                    .current
                    .files
                    .iter()
                    .filter(|(_, identity)| identity.dirty == Some(true))
                    .map(|(path, _)| path.clone())
                    .chain(report.changes.iter().map(|change| change.path.clone()))
                    .take(10_000)
                    .collect()
            })
            .unwrap_or_default();
        let failures = std::mem::take(&mut self.rankings.failures);
        self.set_rank_facts(changed, failures);
        self.observations.clear();
        self.wire = None;
        if self.await_refresh {
            self.await_refresh = false;
            if report.is_some() {
                self.catalogue(task_id, report, false);
            } else {
                self.error =
                    "Repository inspection unavailable. /repo shows details; /refresh retries."
                        .into();
            }
        } else {
            self.observe_captures(task_id, report);
        }
    }
    pub fn event(&mut self, task_id: i64, event: context::Event) {
        if event.task_id != task_id {
            return;
        }
        self.source_busy = false;
        match event.result {
            Ok(context::Response::Catalogue(files)) => {
                self.files = files;
                self.rebuild_rankings();
                self.file_selection = self.file_selection.min(self.files.len().saturating_sub(1));
                self.status = format!(
                    "{} eligible files / ignored, excluded, binary and linked paths omitted",
                    self.files.len()
                );
                self.file_detail();
            }
            Ok(context::Response::Inspected(mut excerpt)) => {
                excerpt.origin_run = self.pending_origin_run.take();
                self.preview_reviewed = false;
                self.detail = excerpt_detail(&excerpt);
                self.preview = Some(excerpt);
                self.mode = Mode::Preview;
                self.status =
                    "Review local capture / Enter attaches / nothing sent to provider".into();
            }
            Ok(context::Response::Checked(observations)) => {
                self.observations = observations;
                self.status =
                    "Explicit source-worker check complete; sources may change afterwards.".into();
                if self.mode == Mode::Basket {
                    self.basket_detail();
                }
            }
            Ok(context::Response::Assembled(payload)) => {
                self.pending_payload = Some(payload);
                self.queue_store(
                    task_id,
                    store::ContextOperation::Validate(self.basket.clone()),
                );
                self.status =
                    "Source identities checked; validating persisted basket references…".into();
            }
            Err(error) => {
                self.pending_origin_run = None;
                self.error = safe_error(error);
                self.pending_payload = None;
                self.payload = None;
                self.status = "Context unavailable or stale / no assembly or dispatch".into();
            }
        }
    }
    pub fn stored(
        &mut self,
        task_id: i64,
        owner: i64,
        result: Result<Vec<context::Excerpt>, String>,
    ) {
        if task_id != owner {
            return;
        }
        self.store_busy = false;
        match result {
            Ok(rows) => {
                self.basket = rows;
                self.wire = None;
                self.observations.clear();
                if self
                    .chip_focus
                    .is_some_and(|id| !self.basket.iter().any(|row| row.id == id))
                {
                    self.chip_focus = None;
                }
                self.basket_selection = self
                    .basket_selection
                    .min(self.basket.len().saturating_sub(1));
                if self.store_mutation {
                    if matches!(self.last_mutation, Some(Mutation::Pin)) {
                        self.mode = Mode::Basket;
                    }
                    if matches!(self.last_mutation, Some(Mutation::Pin)) && self.return_after_attach
                    {
                        self.return_to_composer = true;
                        self.return_after_attach = false;
                    }
                    self.store_failed = false;
                    self.last_mutation = None;
                }
                self.error.clear();
                if let Some(payload) = self.pending_payload.take() {
                    self.status = if payload.dispatch {
                        "Dispatch boundary revalidated / no provider send"
                    } else {
                        "Source and stored references validated / preview only"
                    }
                    .into();
                    self.payload = Some(payload);
                    self.payload_page = 0;
                    self.mode = Mode::Payload;
                    self.payload_detail();
                } else {
                    self.status = format!(
                        "{} task-owned basket references / stored",
                        self.basket.len()
                    );
                    if self.mode == Mode::Basket {
                        self.basket_detail();
                    }
                }
            }
            Err(error) => {
                self.error = safe_error(error);
                self.pending_payload = None;
                self.payload = None;
                self.status = "Context storage/validation failed /journal-retry retries failed mutation; no automatic retry".into();
                self.store_failed |= self.store_mutation;
            }
        }
    }
    /// Reuses the normal containment/exclusion/content guard and capture review.
    pub fn inspect_location(
        &mut self,
        task_id: i64,
        report: &Arc<repo::Report>,
        location: &crate::workspace::locations::Location,
        run_cwd: &std::path::Path,
        run: String,
    ) -> Result<(), String> {
        if self.busy() || self.store_failed {
            return Err("Finish or retry the pending context operation first.".into());
        }
        let path = std::path::Path::new(&location.path);
        let absolute;
        let path = if path.is_absolute() {
            path
        } else {
            if !run_cwd.is_absolute() || !run_cwd.starts_with(&report.repository.root) {
                return Err("The diagnostic run directory is outside the current project.".into());
            }
            absolute = run_cwd.join(path);
            &absolute
        };
        let relative = path
            .strip_prefix(&report.repository.root)
            .map_err(|_| "Location is outside the current project root.")?;
        if location.line == 0 || location.path.len() > 32 * 1024 {
            return Err("Location must name a bounded contained file and positive line.".into());
        }
        let mut normalized = std::path::PathBuf::new();
        for component in relative.components() {
            match component {
                std::path::Component::Normal(name) => normalized.push(name),
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir if normalized.pop() => {}
                _ => return Err("Location escapes the current project root.".into()),
            }
        }
        if normalized.as_os_str().is_empty() {
            return Err("Location must name a file inside the current project.".into());
        }
        self.preview = None;
        self.preview_reviewed = false;
        self.return_after_attach = false;
        self.pending_origin_run = Some(run);
        self.queue(
            task_id,
            context::Operation::Inspect {
                report: Arc::clone(report),
                path: normalized,
                start_line: location.line.saturating_sub(3).max(1),
                end_line: location.line,
            },
        );
        Ok(())
    }
    pub fn inspect_file(&mut self, task_id: i64, report: Option<&Arc<repo::Report>>) {
        if self.busy() || self.store_failed {
            self.error =
                "Finish or retry the pending context operation before selecting another file."
                    .into();
            return;
        }
        let Some(report) = report else {
            self.error = "Repository evidence unavailable; /refresh obtains fresh evidence.".into();
            return;
        };
        let Some(source) = self.files.get(self.file_selection) else {
            self.error = "No eligible source selected.".into();
            return;
        };
        self.preview = None;
        self.preview_reviewed = false;
        self.queue(
            task_id,
            context::Operation::InspectFile {
                report: Arc::clone(report),
                path: source.path.clone(),
            },
        );
    }

    fn inspect(
        &mut self,
        task_id: i64,
        report: Option<&Arc<repo::Report>>,
        start: usize,
        end: usize,
    ) {
        let Some(report) = report else {
            self.error = "Repository evidence unavailable; /refresh obtains fresh evidence.".into();
            return;
        };
        let Some(source) = self.files.get(self.file_selection) else {
            self.error = "No eligible source selected.".into();
            return;
        };
        self.preview = None;
        self.preview_reviewed = false;
        self.queue(
            task_id,
            context::Operation::Inspect {
                report: Arc::clone(report),
                path: source.path.clone(),
                start_line: start,
                end_line: end,
            },
        );
    }
    fn assemble(&mut self, task_id: i64, report: Option<&Arc<repo::Report>>, dispatch: bool) {
        let Some(report) = report else {
            self.error = "Repository evidence unavailable; /refresh obtains fresh evidence.".into();
            return;
        };
        self.payload = None;
        self.pending_payload = None;
        self.detail.clear();
        self.queue(
            task_id,
            context::Operation::Assemble {
                report: Arc::clone(report),
                excerpts: self.basket.clone(),
                dispatch,
            },
        );
    }
    pub fn paste(&mut self, input: &str) -> bool {
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        let limit: usize = match editor.kind {
            EditKind::Search => 1024,
            EditKind::Range => 64,
        };
        let capacity = limit.saturating_sub(editor.text.len());
        let mut text = String::with_capacity(input.len().min(capacity));
        for ch in input
            .chars()
            .filter(|ch| super::safe_character(*ch) && *ch != '\n')
        {
            if text.len() + ch.len_utf8() > capacity {
                self.error = format!("Input limited to {limit} UTF-8 bytes; paste was truncated.");
                break;
            }
            text.push(ch);
        }
        if text.is_empty() {
            return false;
        }
        editor.text.insert_str(editor.cursor, &text);
        editor.cursor += text.len();
        editor.cursor = editor
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i >= editor.cursor)
            .unwrap_or(editor.text.len());
        true
    }
    pub fn command_choices(&self) -> Vec<super::commands::Choice> {
        use super::commands::Choice;
        let blocked = if self.editor.is_some() {
            Some("Finish or cancel the literal editor first.")
        } else if self.confirmation.is_some() {
            Some("Finish or cancel the current removal review first.")
        } else if self.busy() {
            Some("A source or storage operation is pending.")
        } else if self.store_failed {
            Some("Retry the retained storage operation first.")
        } else {
            None
        };
        let choice = |name, label, reason: Option<&str>| match reason {
            Some(reason) => Choice::disabled(name, label, reason),
            None => Choice::new(name, label),
        };
        vec![
            choice("search", "Edit literal filename filter", blocked),
            choice("refresh", "Refresh repository source evidence", blocked),
            choice("basket", "Switch between files and pinned basket", blocked),
            choice(
                "range",
                "Inspect an exact source line range",
                blocked.or_else(|| {
                    (self.mode == Mode::Basket || self.files.get(self.file_selection).is_none())
                        .then_some("Select an eligible file first.")
                }),
            ),
            choice(
                "inspect",
                "Inspect selected whole file before attaching",
                blocked.or_else(|| {
                    (self.mode != Mode::Files || self.files.get(self.file_selection).is_none())
                        .then_some("Select an eligible file first.")
                }),
            ),
            choice(
                "attach",
                "Attach reviewed local capture to this task",
                blocked.or_else(|| {
                    if self.mode != Mode::Preview || self.preview.is_none() {
                        Some("Inspect an eligible source first.")
                    } else {
                        (!self.preview_reviewed)
                            .then_some("Review the visible capture before attaching.")
                    }
                }),
            ),
            choice(
                "assemble",
                "Inspect locally assembled context payload",
                blocked.or_else(|| {
                    self.basket
                        .is_empty()
                        .then_some("Pin source excerpts first.")
                }),
            ),
            choice(
                "dispatch-preview",
                "Validate context for provider dispatch",
                blocked.or_else(|| {
                    self.basket
                        .is_empty()
                        .then_some("Pin source excerpts first.")
                }),
            ),
            choice(
                "unpin",
                "Review removing selected basket excerpt",
                blocked.or_else(|| {
                    (self.mode != Mode::Basket || self.basket.get(self.basket_selection).is_none())
                        .then_some("Select a pinned basket excerpt first.")
                }),
            ),
            choice(
                "purge",
                "Review purging selected captured bytes",
                blocked.or_else(|| {
                    (self.mode != Mode::Basket || self.basket.get(self.basket_selection).is_none())
                        .then_some("Select a pinned basket excerpt first.")
                }),
            ),
            choice(
                "journal-retry",
                "Retry retained context storage operation",
                if self.editor.is_some() || self.confirmation.is_some() || self.busy() {
                    blocked
                } else {
                    (!self.store_failed).then_some("No failed context storage operation to retry.")
                },
            ),
            choice(
                "page-next",
                "Next assembled payload page",
                blocked.or_else(|| {
                    (self.mode != Mode::Payload).then_some("Assemble a context payload first.")
                }),
            ),
            choice(
                "page-previous",
                "Previous assembled payload page",
                blocked.or_else(|| {
                    (self.mode != Mode::Payload).then_some("Assemble a context payload first.")
                }),
            ),
            choice(
                "close",
                "Close sources, retaining captures",
                if self.editor.is_some() || self.confirmation.is_some() {
                    blocked
                } else {
                    None
                },
            ),
        ]
    }

    pub fn command(
        &mut self,
        name: &str,
        task_id: i64,
        report: Option<&Arc<repo::Report>>,
    ) -> Action {
        if self.editor.is_some() || self.confirmation.is_some() {
            return Action::Changed;
        }
        if name == "close" {
            self.return_after_attach = false;
            self.return_to_composer = false;
            return Action::Close;
        }
        if self.busy() {
            self.error = "A context operation is pending; PgUp/Dn and Esc remain available.".into();
            return Action::Changed;
        }
        if self.store_failed {
            if name == "journal-retry" {
                let operation = match self.last_mutation {
                    Some(Mutation::Pin) => self.preview.clone().map(store::ContextOperation::Pin),
                    Some(Mutation::Remove(id)) => Some(store::ContextOperation::Remove(id)),
                    Some(Mutation::Purge(id)) => Some(store::ContextOperation::Purge(vec![id])),
                    None => None,
                };
                if let Some(operation) = operation {
                    self.queue_store(task_id, operation);
                }
            } else {
                self.error = "Resolve storage, then /journal-retry retries the retained operation. Esc keeps it; quitting requires explicit loss review.".into();
            }
            return Action::Changed;
        }
        match name {
            "search" => {
                self.editor = Some(Editor {
                    kind: EditKind::Search,
                    text: self.query.clone(),
                    cursor: self.query.len(),
                });
            }
            "refresh" => {
                self.await_refresh = true;
                self.payload = None;
                return Action::RefreshRepository;
            }
            "basket" => {
                self.mode = if self.mode == Mode::Basket {
                    Mode::Files
                } else {
                    Mode::Basket
                };
                if self.mode == Mode::Files {
                    self.file_detail();
                } else {
                    self.basket_detail();
                }
            }
            "range" if self.mode != Mode::Basket => {
                self.editor = Some(Editor {
                    kind: EditKind::Range,
                    text: "1:1".into(),
                    cursor: 3,
                });
            }
            "inspect" if self.mode == Mode::Files => self.inspect_file(task_id, report),
            "attach" if self.mode == Mode::Preview => {
                if !self.preview_reviewed {
                    self.error =
                        "Review the visible capture before attaching; nothing saved or sent."
                            .into();
                } else if let Some(excerpt) = self.preview.as_ref() {
                    if let Some(index) = self.basket.iter().position(|stored| {
                        stored.task_id == excerpt.task_id
                            && stored.root == excerpt.root
                            && stored.path == excerpt.path
                            && stored.start_line == excerpt.start_line
                            && stored.end_line == excerpt.end_line
                            && stored.source_sha256 == excerpt.source_sha256
                            && stored.text == excerpt.text
                            && stored.available
                    }) {
                        self.payload = None;
                        self.basket_selection = index;
                        self.mode = Mode::Basket;
                        self.return_to_composer = true;
                        self.basket_detail();
                        return Action::Changed;
                    }
                    let excerpt = excerpt.clone();
                    self.payload = None;
                    self.return_after_attach = true;
                    self.queue_store(task_id, store::ContextOperation::Pin(excerpt));
                    self.status =
                        "Saving local attachment; not attached until storage confirms…".into();
                }
            }
            "assemble" => self.assemble(task_id, report, false),
            "dispatch-preview" => self.assemble(task_id, report, true),
            "unpin" | "purge" if self.mode == Mode::Basket => {
                if let Some(excerpt) = self.basket.get(self.basket_selection) {
                    self.confirmation = Some(Confirmation {
                        id: excerpt.id,
                        purge: name == "purge",
                        destructive: false,
                        reviewed: false,
                    });
                }
            }
            "page-next" if self.mode == Mode::Payload => {
                if let Some(payload) = &self.payload {
                    self.payload_page =
                        (self.payload_page + 1).min(payload.text.len().saturating_sub(1) / 8192);
                }
                self.payload_detail();
            }
            "page-previous" if self.mode == Mode::Payload => {
                self.payload_page = self.payload_page.saturating_sub(1);
                self.payload_detail();
            }
            _ => {}
        }
        Action::Changed
    }

    pub fn key(
        &mut self,
        mut key: KeyEvent,
        task_id: i64,
        report: Option<&Arc<repo::Report>>,
    ) -> Action {
        if key.kind != KeyEventKind::Press {
            return Action::Changed;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.confirmation = None;
            return Action::Quit;
        }
        if let Some(confirmation) = self.confirmation.as_mut() {
            if key.modifiers.is_empty() {
                match key.code {
                    KeyCode::Char('y') if confirmation.reviewed => {
                        confirmation.destructive = true;
                        key.code = KeyCode::Enter;
                    }
                    KeyCode::Char('n') => key.code = KeyCode::Esc,
                    _ => {}
                }
            }
            if key.code == KeyCode::Enter && confirmation.destructive && !confirmation.reviewed {
                self.error = "Review the visible excerpt scope before confirming.".into();
                return Action::Changed;
            }
            match key.code {
                KeyCode::Esc => self.confirmation = None,
                KeyCode::Tab | KeyCode::BackTab => {
                    confirmation.destructive = !confirmation.destructive
                }
                KeyCode::Left | KeyCode::Up | KeyCode::Home => confirmation.destructive = false,
                KeyCode::Right | KeyCode::Down | KeyCode::End => confirmation.destructive = true,
                KeyCode::PageUp | KeyCode::PageDown => return Action::Scroll(key.code),
                KeyCode::Enter => {
                    let confirmation = self
                        .confirmation
                        .take()
                        .expect("active context confirmation");
                    if confirmation.destructive && !self.busy() {
                        self.payload = None;
                        let operation = if confirmation.purge {
                            store::ContextOperation::Purge(vec![confirmation.id])
                        } else {
                            store::ContextOperation::Remove(confirmation.id)
                        };
                        self.queue_store(task_id, operation);
                    }
                }
                _ => {}
            }
            return Action::Changed;
        }
        if self.editor.is_some() {
            return self.editor_key(key, task_id, report);
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Action::Changed;
        }
        if key.code == KeyCode::Esc {
            self.return_after_attach = false;
            self.return_to_composer = false;
            return Action::Close;
        }
        if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            return Action::Scroll(key.code);
        }
        if self.busy() {
            self.error = "A context operation is pending; PgUp/Dn and Esc remain available.".into();
            return Action::Changed;
        }
        if self.store_failed {
            if key.code == KeyCode::Enter {
                return self.command("journal-retry", task_id, report);
            }
            return Action::Changed;
        }
        match key.code {
            KeyCode::Enter if self.mode == Mode::Files => self.inspect_file(task_id, report),
            KeyCode::Enter if self.mode == Mode::Preview => {
                return self.command("attach", task_id, report);
            }
            KeyCode::Tab | KeyCode::BackTab => return self.command("basket", task_id, report),
            KeyCode::Char('s') => return self.command("search", task_id, report),
            KeyCode::Char('r') if self.mode != Mode::Basket => {
                return self.command("range", task_id, report);
            }
            KeyCode::Char('i') if self.mode == Mode::Files => {
                self.inspect(task_id, report, 1, 1);
            }
            KeyCode::Delete | KeyCode::Backspace => {
                if self.mode == Mode::Files
                    && let Some(index) = self.file_attachment(self.file_selection)
                {
                    self.basket_selection = index;
                    self.mode = Mode::Basket;
                    self.basket_detail();
                }
                if self.mode == Mode::Basket {
                    return self.command("unpin", task_id, report);
                }
            }
            code if matches!(self.mode, Mode::Files | Mode::Basket) => {
                let (selected, count) = if self.mode == Mode::Files {
                    (&mut self.file_selection, self.files.len())
                } else {
                    (&mut self.basket_selection, self.basket.len())
                };
                *selected = match code {
                    KeyCode::Up => selected.saturating_sub(1),
                    KeyCode::Down => selected.saturating_add(1).min(count.saturating_sub(1)),
                    KeyCode::Home => 0,
                    KeyCode::End => count.saturating_sub(1),
                    _ => *selected,
                };
                if self.mode == Mode::Basket {
                    self.basket_detail();
                } else {
                    self.file_detail();
                }
            }
            code => return Action::Scroll(code),
        }
        Action::Changed
    }
    fn editor_key(
        &mut self,
        key: KeyEvent,
        task_id: i64,
        report: Option<&Arc<repo::Report>>,
    ) -> Action {
        if key.code == KeyCode::Esc {
            self.editor = None;
            return Action::Changed;
        }
        if key.code == KeyCode::Enter && key.modifiers.is_empty() {
            if self.busy() {
                self.error = "Wait for the pending context operation; input is retained.".into();
                return Action::Changed;
            }
            let editor = self.editor.take().expect("active source editor");
            match editor.kind {
                EditKind::Search => {
                    self.query = editor.text;
                    self.file_selection = 0;
                    self.catalogue(task_id, report, self.await_refresh);
                }
                EditKind::Range => {
                    let range = editor.text.split_once(':').and_then(|(a, b)| {
                        Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
                    });
                    if let Some((start, end)) =
                        range.filter(|(start, end)| *start > 0 && end >= start)
                    {
                        self.inspect(task_id, report, start, end);
                    } else {
                        self.error = "Use a one-based inclusive range, e.g. 2:5.".into();
                        self.editor = Some(editor);
                    }
                }
            }
            return Action::Changed;
        }
        if let KeyCode::Char(ch) = key.code
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.paste(ch.encode_utf8(&mut [0; 4]));
            return Action::Changed;
        }
        let editor = self.editor.as_mut().expect("active source editor");
        let before = editor.text[..editor.cursor]
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .next_back()
            .unwrap_or(0);
        let after = editor
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i > editor.cursor)
            .unwrap_or(editor.text.len());
        match key.code {
            KeyCode::Left => editor.cursor = before,
            KeyCode::Right => editor.cursor = after,
            KeyCode::Home => editor.cursor = 0,
            KeyCode::End => editor.cursor = editor.text.len(),
            KeyCode::Backspace => {
                editor.text.replace_range(before..editor.cursor, "");
                editor.cursor = before;
            }
            KeyCode::Delete => {
                editor.text.replace_range(editor.cursor..after, "");
            }
            KeyCode::Char('h') if key.modifiers == KeyModifiers::CONTROL => {
                editor.text.replace_range(before..editor.cursor, "");
                editor.cursor = before;
            }
            _ => {}
        }
        if editor.cursor < editor.text.len() {
            editor.cursor = editor
                .text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .take_while(|i| *i <= editor.cursor)
                .last()
                .unwrap_or(0);
        }
        Action::Changed
    }
    fn file_detail(&mut self) {
        self.detail = self.files.get(self.file_selection).map(|source|
            format!("Project-relative path: {:?}\n{} bytes / SHA-256 bytes: {:02x?}\nEnter reviews the whole file before attaching; r chooses bounded lines; i inspects line 1. Binary, oversized, credential and excluded paths remain ineligible.", source.path, source.bytes, source.sha256))
            .unwrap_or_else(|| "No eligible files match this filename filter. s edits the filter; /refresh refreshes project source evidence.".into());
    }
    fn basket_detail(&mut self) {
        self.detail = self
            .basket
            .get(self.basket_selection)
            .map(excerpt_detail)
            .unwrap_or_else(|| {
                "No saved attachments. Tab opens files; Enter reviews a file before attaching."
                    .into()
            });
        if let Some(capture) = self.basket.get(self.basket_selection) {
            let freshness = self
                .observations
                .iter()
                .find(|row| row.id == capture.id)
                .map(|row| match &row.freshness {
                    context::Freshness::Current => {
                        "CURRENT at last explicit worker check; dispatch revalidates".into()
                    }
                    context::Freshness::Stale(reason) => format!("STALE: {reason}"),
                    context::Freshness::Unavailable(reason) => format!("UNAVAILABLE: {reason}"),
                })
                .unwrap_or_else(|| {
                    "NOT CHECKED: captured bytes are not a live-watch assertion".into()
                });
            let wire = self.wire.as_ref().and_then(|receipt| receipt.captures.iter()
                .find(|row| row.id == capture.id).map(|row| format!(
                    "{} serialized entry bytes / ~{} tokens ({}) / includes native outer escaping\n{} B all capture entries + {} B shared envelope/other fields = {} B actual frozen request\nProvider-added context/history/reads are not counted or constrained.",
                    row.bytes, row.estimated_tokens, crate::workspace::receipt::ESTIMATOR,
                    receipt.total_bytes - receipt.other_bytes, receipt.other_bytes, receipt.total_bytes,
                ))).unwrap_or_else(|| "Serialized request contribution unavailable until an immutable provider preview is prepared; raw text bytes are not wire bytes".into());
            self.detail.insert_str(0, &format!("{freshness}\n{wire}\n"));
        }
    }
    fn payload_detail(&mut self) {
        let Some(payload) = &self.payload else {
            return;
        };
        let boundary = |mut n: usize| {
            n = n.min(payload.text.len());
            while !payload.text.is_char_boundary(n) {
                n -= 1;
            }
            n
        };
        let start = boundary(self.payload_page * 8192);
        let end = boundary((self.payload_page + 1) * 8192);
        let dedup = if payload.deduplicated_bytes > 0 {
            format!(
                " Local deduplication saved {} B: overlapping captures cite identical bytes via same_bytes_as instead of resending them.",
                payload.deduplicated_bytes
            )
        } else {
            String::new()
        };
        self.detail = format!(
            "EXACT BASKET ENVELOPE / escaped display\n{} UTF-8 bytes / ~{} tokens ({})\nProvider history/config/agent-owned reads may add context; this basket does not constrain them. No provider is connected and nothing is sent.{}\nPage {} / bytes {}..{} / /page-next, /page-previous\n\n{:?}",
            payload.bytes,
            payload.estimated_tokens,
            crate::workspace::receipt::ESTIMATOR,
            dedup,
            self.payload_page + 1,
            start,
            end,
            &payload.text[start..end]
        );
    }
}

fn source_stem(path: &std::path::Path) -> Option<&str> {
    let name = path.file_stem()?.to_str()?;
    let name = name.strip_prefix("test_").unwrap_or(name);
    Some(
        name.strip_suffix("_test")
            .or_else(|| name.strip_suffix(".test"))
            .or_else(|| name.strip_suffix(".spec"))
            .unwrap_or(name),
    )
}

fn test_path(path: &std::path::Path) -> bool {
    path.components().any(|component| {
        let Some(name) = component.as_os_str().to_str() else {
            return false;
        };
        let bytes = name.as_bytes();
        ["test", "tests", "__tests__", "spec", "specs"]
            .iter()
            .any(|candidate| name.eq_ignore_ascii_case(candidate))
            || bytes
                .get(..5)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"test_"))
            || bytes
                .get(bytes.len().saturating_sub(8)..)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(b"_test.rs"))
            || bytes.windows(6).any(|window| {
                window.eq_ignore_ascii_case(b".test.") || window.eq_ignore_ascii_case(b".spec.")
            })
    })
}

fn excerpt_detail(excerpt: &context::Excerpt) -> String {
    let mut value = format!(
        "#{} / {:?}\nTask {} / root {:?}\nLines {}:{} inclusive / {} captured UTF-8 bytes\n{}\n\nExact captured bytes / escaped display (not reread from the current file):\n{:?}",
        excerpt.id,
        excerpt.path,
        excerpt.task_id,
        excerpt.root,
        excerpt.start_line,
        excerpt.end_line,
        excerpt.text.len(),
        if excerpt.available {
            "Captured source / best-effort secret masking"
        } else {
            "UNAVAILABLE: retained identity only; assembly must refuse"
        },
        excerpt.text
    );
    if let Some(run) = &excerpt.origin_run {
        value.insert_str(0, &format!("From run {run} (reference only)\n"));
    }
    value.retain(super::safe_character);
    value
}
fn safe_error(mut error: String) -> String {
    error.retain(super::safe_character);
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capture() -> context::Excerpt {
        context::Excerpt {
            origin_run: None,
            id: 0,
            task_id: 7,
            root: "/workspace".into(),
            path: "sample.rs".into(),
            start_line: 1,
            end_line: 2,
            source_sha256: [7; 32],
            text: "first\npassword=[REDACTED]\n".into(),
            available: true,
        }
    }

    #[test]
    fn direct_attachment_requires_capture_review_and_acknowledged_owner() {
        let mut state = State::default();
        let excerpt = capture();
        state.event(
            7,
            context::Event {
                task_id: 7,
                result: Ok(context::Response::Inspected(excerpt.clone())),
            },
        );
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        state.key(enter, 7, None);
        assert!(state.pending_store.is_none());
        state.preview_reviewed = true;
        state.key(enter, 7, None);
        assert!(matches!(
            &state.pending_store,
            Some(store::ContextRequest { task_id: 7, operation: store::ContextOperation::Pin(saved), .. })
                if saved == &excerpt
        ));
        assert!(state.basket.is_empty());
        assert!(state.persistence_pending());
        state.stored(7, 8, Ok(vec![excerpt.clone()]));
        assert!(state.basket.is_empty());
        assert!(!state.take_return_to_composer());
        state.stored(7, 7, Err("disk full".into()));
        assert!(state.store_failed);
        assert_eq!(state.preview.as_ref(), Some(&excerpt));
        assert!(!state.take_return_to_composer());
        state.command("journal-retry", 7, None);
        assert!(matches!(&state.pending_store,
            Some(store::ContextRequest { operation: store::ContextOperation::Pin(saved), .. })
                if saved == &excerpt));
        let mut stored = excerpt;
        stored.id = 91;
        state.stored(7, 7, Ok(vec![stored.clone()]));
        assert_eq!(state.basket, vec![stored]);
        assert!(state.take_return_to_composer());
        assert!(!state.take_return_to_composer());
    }

    #[test]
    fn escape_during_attachment_save_retains_mutation_without_late_navigation() {
        let mut state = State {
            mode: Mode::Preview,
            preview: Some(capture()),
            preview_reviewed: true,
            ..State::default()
        };
        state.command("attach", 7, None);
        assert!(matches!(
            state.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), 7, None),
            Action::Close
        ));
        assert!(state.pending_store.is_some());
        state.stored(7, 7, Ok(vec![capture()]));
        assert_eq!(state.basket.len(), 1);
        assert!(!state.take_return_to_composer());
    }

    #[test]
    fn missing_repository_waits_only_for_an_owned_inspection() {
        for repository_pending in [false, true] {
            let mut state = State::default();
            state.catalogue(7, None, repository_pending);
            assert_eq!(state.await_refresh, repository_pending);
            assert_eq!(state.busy(), repository_pending);
            assert!(state.pending.is_none() && state.pending_store.is_none());
            assert!(!state.source_busy && !state.store_busy);
            if repository_pending {
                assert!(state.error.is_empty());
                state.observed_repository(7, None);
                assert!(!state.busy());
            }
            assert!(state.error.contains("Repository inspection unavailable"));
            state.catalogue(7, None, false);
            assert!(!state.busy());
            assert!(state.status.contains("unavailable"));
            state.command("search", 7, None);
            state.paste("literal-filter");
            state.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 7, None);
            assert_eq!(state.query, "literal-filter");
            assert!(!state.busy());
            assert!(state.error.contains("Repository inspection unavailable"));
        }
    }

    #[test]
    fn unavailable_repository_does_not_release_real_source_or_store_work() {
        for (source_busy, store_busy) in [(true, false), (false, true)] {
            let mut state = State {
                source_busy,
                store_busy,
                ..State::default()
            };
            state.catalogue(7, None, false);
            assert!(state.busy());
            assert_eq!(state.source_busy, source_busy);
            assert_eq!(state.store_busy, store_busy);
        }
    }

    #[test]
    fn source_commands_preserve_literal_search_and_cancel_without_work() {
        let mut state = State::default();
        state.command("search", 1, None);
        assert!(matches!(
            state.editor.as_ref().map(|editor| editor.kind),
            Some(EditKind::Search)
        ));
        for ch in "/range 1:3".chars() {
            state.key(
                KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                1,
                None,
            );
        }
        assert_eq!(state.editor.as_ref().unwrap().text, "/range 1:3");
        assert!(state.pending.is_none() && state.pending_store.is_none());
        state.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), 1, None);
        assert!(state.editor.is_none());
        assert!(state.query.is_empty());
    }

    #[test]
    fn source_commands_do_not_bypass_busy_or_neutral_removal_review() {
        let mut state = State {
            source_busy: true,
            ..State::default()
        };
        state.command("refresh", 1, None);
        assert!(!state.await_refresh);
        assert!(state.pending.is_none() && state.pending_store.is_none());
        state.source_busy = false;
        state.confirmation = Some(Confirmation {
            id: 7,
            purge: true,
            destructive: false,
            reviewed: false,
        });
        state.command("purge", 1, None);
        assert!(!state.confirmation.unwrap().destructive);
        state.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 1, None);
        assert!(state.confirmation.is_none());
        assert!(state.pending_store.is_none());
    }

    #[test]
    fn exact_capture_focus_and_removal_never_confuse_two_ranges_of_one_file() {
        let mut first = capture();
        first.id = 41;
        let mut second = first.clone();
        second.id = 42;
        second.start_line = 4;
        second.end_line = 5;
        second.text = "four\nfive\n".into();
        let mut state = State {
            basket: vec![first.clone(), second.clone()],
            ..State::default()
        };
        assert!(state.focus_chip(false));
        assert_eq!(state.chip_focus, Some(41));
        assert!(state.focus_chip(false));
        assert_eq!(state.chip_focus, Some(42));
        assert!(state.inspect_chip(42, true, 7));
        assert_eq!(state.confirmation.as_ref().unwrap().id, 42);
        assert_eq!(state.basket_selection, 1);
        assert!(state.detail.contains(&format!("{:?}", second.text)));
        assert!(state.pending_store.is_none());
        state.confirmation = None;
        state.stored(7, 7, Ok(vec![first.clone()]));
        assert_eq!(state.basket, vec![first]);
        assert_eq!(state.chip_focus, None);
        assert!(!state.inspect_chip(42, true, 7));
    }

    #[test]
    fn ranking_changes_order_and_reasons_but_never_catalogue_membership() {
        let mut state = State {
            files: [
                "a/failure.rs",
                "b/changed.rs",
                "tests/changed.rs",
                "tests/unrelated.rs",
            ]
            .into_iter()
            .map(|path| context::Source {
                path: path.into(),
                bytes: 1,
                sha256: [0; 32],
            })
            .collect(),
            ..State::default()
        };
        state.rankings.changed = [std::path::PathBuf::from("b/changed.rs")]
            .into_iter()
            .collect();
        state.set_failure_paths([
            std::path::PathBuf::from("a/failure.rs"),
            std::path::PathBuf::from(".env"),
        ]);
        assert_eq!(state.ranked_files(5).next(), Some(0));
        assert_eq!(state.rank_reason(5, 0), Some(RankReason::Failure));
        assert_eq!(state.ranked_files(3).next(), Some(1));
        assert_eq!(state.ranked_files(6).next(), Some(1));
        assert_eq!(state.rank_reason(7, 2), Some(RankReason::Tests));
        assert_eq!(state.rank_reason(7, 3), None);
        for form in 0..9 {
            let mut members: Vec<_> = state.ranked_files(form).collect();
            members.sort_unstable();
            assert_eq!(members, vec![0, 1, 2, 3]);
        }
    }

    #[test]
    fn capture_producer_survives_lead_change_and_failed_pin_retry() {
        let owner = store::Attribution {
            partner: 1,
            name: "First".into(),
            form: 5,
        };
        let next = store::Attribution {
            partner: 2,
            name: "Second".into(),
            form: 3,
        };
        let snapshot = Arc::new(repo::Snapshot {
            head: None,
            files: Default::default(),
            working_diff: Default::default(),
            complete: true,
            stable: true,
            warnings: Vec::new(),
        });
        let report = Arc::new(repo::Report {
            repository: repo::RepositoryIdentity {
                root: "/workspace".into(),
                git_root: Some("/workspace".into()),
                git_dir: Some("/workspace/.git".into()),
                storage_root: None,
            },
            baseline: Arc::clone(&snapshot),
            current: snapshot,
            changes: Vec::new(),
            initial: true,
        });
        let mut state = State::default();
        state.set_producer(Some(owner.clone()));
        state.queue(
            7,
            context::Operation::InspectFile {
                report,
                path: "sample.rs".into(),
            },
        );
        state.set_producer(Some(next));
        state.event(
            7,
            context::Event {
                task_id: 7,
                result: Ok(context::Response::Inspected(capture())),
            },
        );
        state.preview_reviewed = true;
        state.command("attach", 7, None);
        assert_eq!(
            state.pending_store.as_ref().unwrap().producer.as_ref(),
            Some(&owner)
        );
        state.stored(7, 7, Err("disk full".into()));
        state.command("journal-retry", 7, None);
        assert_eq!(
            state.pending_store.as_ref().unwrap().producer.as_ref(),
            Some(&owner)
        );
        assert!(matches!(
            &state.pending_store.as_ref().unwrap().operation,
            store::ContextOperation::Pin(_)
        ));
    }

    #[test]
    fn diagnostics_resolve_against_actual_run_directory_with_project_containment() {
        let enclosing = std::env::temp_dir().join("ev-diagnostic-scope");
        let project = enclosing.join("project");
        let snapshot = Arc::new(repo::Snapshot {
            head: None,
            files: Default::default(),
            working_diff: Default::default(),
            complete: false,
            stable: false,
            warnings: Vec::new(),
        });
        let report = Arc::new(repo::Report {
            repository: repo::RepositoryIdentity {
                root: project.clone(),
                git_root: Some(enclosing.clone()),
                git_dir: Some(enclosing.join(".git")),
                storage_root: None,
            },
            baseline: Arc::clone(&snapshot),
            current: snapshot,
            changes: Vec::new(),
            initial: true,
        });
        let location = crate::workspace::locations::Location {
            path: "src/current.rs".into(),
            line: 4,
            column: Some(1),
        };
        let mut state = State::default();
        state
            .inspect_location(
                7,
                &report,
                &location,
                &project.join("subdir"),
                "r-test-run".into(),
            )
            .unwrap();
        let context::Operation::Inspect {
            path,
            start_line,
            end_line,
            ..
        } = &state.pending.as_ref().unwrap().operation
        else {
            panic!("expected diagnostic capture");
        };
        assert_eq!(path, std::path::Path::new("subdir/src/current.rs"));
        assert_eq!((*start_line, *end_line), (1, 4));
        let mut state = State::default();
        let parent_relative = crate::workspace::locations::Location {
            path: "../src/main.c".into(),
            line: 12,
            column: Some(3),
        };
        state
            .inspect_location(
                7,
                &report,
                &parent_relative,
                &project.join("build"),
                "r-test-run".into(),
            )
            .unwrap();
        let context::Operation::Inspect { path, .. } = &state.pending.as_ref().unwrap().operation
        else {
            panic!("expected contained parent-relative diagnostic capture");
        };
        assert_eq!(path, std::path::Path::new("src/main.c"));
        let mut state = State::default();
        let escaping = crate::workspace::locations::Location {
            path: "../../sibling/main.c".into(),
            ..parent_relative
        };
        assert!(
            state
                .inspect_location(
                    7,
                    &report,
                    &escaping,
                    &project.join("build"),
                    "r-test-run".into()
                )
                .is_err()
        );
        assert!(state.pending.is_none());
        let mut state = State::default();
        assert!(
            state
                .inspect_location(
                    7,
                    &report,
                    &location,
                    &enclosing.join("sibling"),
                    "r-test-run".into()
                )
                .is_err()
        );
        let outside = crate::workspace::locations::Location {
            path: enclosing.join("sibling/source.rs").to_str().unwrap().into(),
            ..location
        };
        assert!(
            state
                .inspect_location(7, &report, &outside, &project, "r-test-run".into())
                .is_err()
        );
        assert!(state.pending.is_none());
    }
}
