//! Factual, project-local records. Rendering and input never perform repository IO.
use super::{App, Overlay};
use crate::{
    i18n::Message,
    store::{
        self,
        records::{Operation, Request, Response},
    },
    workspace::{
        records::{self, ArtifactKind, Freshness, LocalIntent},
        runner::InputSnapshot,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::collections::{BTreeMap, HashSet, VecDeque};

const CAPACITY: usize = 8192;
const LOCAL_CAPACITY: usize = 256;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Summary,
    Routes,
    Dex,
    Resolve,
    Habits,
    Pairings,
    Ribbons,
    Discard,
}
#[derive(Clone, Debug, Default)]
pub struct FormMarker {
    pub count: usize,
    pub stale: bool,
    pub unknown: usize,
    pub producers: Vec<String>,
    pub fact: String,
}

pub struct RouteVisit {
    pub form: usize,
    pub produced: bool,
    pub discontinuity: bool,
    start: u64,
    end: Option<u64>,
}
pub struct RouteTrace {
    pub task: i64,
    pub partner: i64,
    pub name: String,
    pub visits: Vec<RouteVisit>,
    pub receipts: Vec<usize>,
}

pub struct State {
    pub snapshot: records::Snapshot,
    pub habits: super::habits::State,
    pub pairings: super::pairings::State,
    pub local: Vec<LocalIntent>,
    pub restoring: bool,
    pub mode: Mode,
    pub selected: usize,
    pub scroll: u16,
    pub scroll_max: u16,
    pub page: u16,
    pub reviewed: bool,
    pub affirmative: bool,
    pub notice: String,
    pub hint: Option<Message>,
    pub pending: Option<Request>,
    pub inflight: Option<Request>,
    pub queued: VecDeque<Request>,
    pub failed: Option<Request>,
    pub failed_error: String,
    pub loaded: bool,
    pub markers: [FormMarker; 9],
    pub traces: Vec<RouteTrace>,
    pub(super) quit_waiting: bool,
    task: i64,
    project: String,
    current: InputSnapshot,
    invalidated: bool,
    load_failed: bool,
    delivered: HashSet<String>,
    lines: VecDeque<records::Activity>,
    return_mode: Mode,
    pub resolution_signature: Option<String>,
    submission_receipt: Option<Request>,
    pairing_context: Option<(i64, i64, usize)>,
    restored_pairing: Option<(i64, i64, usize, Option<records::Pairing>)>,
    notified_habits: HashSet<String>,
    habit_hint: bool,
    current_activity: Option<records::Activity>,
    quit_discard: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            snapshot: records::Snapshot {
                artifacts: vec![],
                routes: vec![],
                activities: vec![],
                signatures: vec![],
                resolutions: vec![],
                resolution_choices: vec![],
                failure_comparisons: vec![],
                habits: vec![],
                pairings: vec![],
                usages: vec![],
                held_items: vec![],
                summaries: vec![],
                ribbons: vec![],
                flavor_enabled: true,
            },
            habits: Default::default(),
            pairings: Default::default(),
            local: vec![],
            restoring: false,
            mode: Mode::Summary,
            selected: 0,
            scroll: 0,
            scroll_max: 0,
            page: 1,
            reviewed: false,
            affirmative: false,
            notice: String::new(),
            hint: None,
            pending: None,
            inflight: None,
            queued: VecDeque::new(),
            failed: None,
            failed_error: String::new(),
            loaded: false,
            markers: Default::default(),
            traces: vec![],
            quit_waiting: false,
            task: 0,
            project: String::new(),
            current: Default::default(),
            invalidated: true,
            load_failed: false,
            delivered: HashSet::new(),
            lines: VecDeque::new(),
            return_mode: Mode::Summary,
            resolution_signature: None,
            submission_receipt: None,
            pairing_context: None,
            restored_pairing: None,
            notified_habits: HashSet::new(),
            habit_hint: false,
            current_activity: None,
            quit_discard: false,
        }
    }
}
fn mutation(request: &Request) -> bool {
    !matches!(request.operation, Operation::Load { .. })
}
impl State {
    pub(super) fn current_for(&self, task: i64, project: &str) -> bool {
        self.loaded && !self.load_failed && self.task == task && self.project == project
    }
    pub fn invalidate_review(&mut self) {
        self.reviewed = false;
        self.affirmative = false;
    }
    pub fn enter(&mut self, mode: Mode) {
        self.mode = mode;
        self.selected = 0;
        self.scroll = 0;
        self.invalidate_review();
    }
    pub fn mutation_pending(&self) -> bool {
        self.pending
            .iter()
            .chain(self.inflight.iter())
            .chain(self.failed.iter())
            .chain(self.queued.iter())
            .any(mutation)
            || !self.local.is_empty()
    }
    pub fn persistence_pending(&self) -> bool {
        self.pending.is_some()
            || self.inflight.is_some()
            || self.failed.is_some()
            || !self.queued.is_empty()
    }
    fn count(&self) -> usize {
        self.pending.iter().count()
            + self.inflight.iter().count()
            + self.failed.iter().count()
            + self.queued.len()
    }
    fn enqueue(&mut self, request: Request) -> bool {
        if self
            .pending
            .iter()
            .chain(self.inflight.iter())
            .chain(self.failed.iter())
            .chain(self.queued.iter())
            .any(|old| old == &request)
        {
            return true;
        }
        if self.count() >= CAPACITY {
            self.hint = Some(Message::RecordsQueueFull);
            return false;
        }
        self.queued.push_back(request);
        true
    }
    fn rebuild_markers(&mut self) {
        self.markers = Default::default();
        let mut latest = std::collections::HashMap::new();
        for view in self
            .snapshot
            .artifacts
            .iter()
            .filter(|view| view.artifact.task_id == self.task)
        {
            let key = (view.artifact.kind as u8, view.artifact.reference.as_str());
            let saved = latest.entry(key).or_insert(view);
            if view.artifact.ordinal > saved.artifact.ordinal {
                *saved = view;
            }
        }
        let mut views: Vec<_> = latest.into_values().collect();
        views.sort_by_key(|view| view.artifact.ordinal);
        let mut current = [false; 9];
        let mut representative = [None; 9];
        let mut explanations: [Vec<&str>; 9] = Default::default();
        for view in views {
            let artifact = &view.artifact;
            let Some(marker) = self.markers.get_mut(artifact.role_form) else {
                continue;
            };
            let form = artifact.role_form;
            marker.count += 1;
            marker.stale |= view.freshness == Freshness::Stale;
            if view.freshness == Freshness::Current || !current[form] {
                representative[form] = Some(view);
            }
            current[form] |= view.freshness == Freshness::Current;
            if matches!(
                view.freshness,
                Freshness::Unavailable | Freshness::Ambiguous
            ) && !view.explanation.is_empty()
                && !explanations[form].contains(&view.explanation.as_str())
            {
                explanations[form].push(&view.explanation);
            }
            if let Some(owner) = &artifact.producer {
                if !marker.producers.contains(&owner.name) {
                    marker.producers.push(owner.name.clone());
                }
            } else {
                marker.unknown += 1;
            }
        }
        for (form, marker) in self.markers.iter_mut().enumerate() {
            marker.stale &= !current[form];
            if let Some(view) = representative[form] {
                marker.fact = view.artifact.detail.clone();
                if !view.explanation.is_empty() {
                    marker.fact.push_str(" · ");
                    marker.fact.push_str(&view.explanation);
                }
                for explanation in &explanations[form] {
                    if *explanation != view.explanation {
                        marker.fact.push_str(" · ");
                        marker.fact.push_str(explanation);
                    }
                }
            }
        }
    }
    fn rebuild_routes(&mut self) {
        let mut outputs: BTreeMap<_, Vec<u64>> = BTreeMap::new();
        for view in &self.snapshot.artifacts {
            if let Some(owner) = &view.artifact.producer {
                outputs
                    .entry((view.artifact.task_id, owner.partner, owner.form))
                    .or_default()
                    .push(view.artifact.ordinal);
            }
        }
        for ordinals in outputs.values_mut() {
            ordinals.sort_unstable();
        }
        let mut grouped: BTreeMap<_, Vec<usize>> = BTreeMap::new();
        for (index, route) in self.snapshot.routes.iter().enumerate() {
            grouped
                .entry((route.task_id, route.owner.partner))
                .or_default()
                .push(index);
        }
        self.traces.clear();
        for ((task, partner), mut receipts) in grouped {
            receipts.sort_unstable_by_key(|index| self.snapshot.routes[*index].ordinal);
            let mut visits: Vec<RouteVisit> = Vec::with_capacity(receipts.len() + 1);
            for index in &receipts {
                let route = &self.snapshot.routes[*index];
                if let Some(previous) = visits.last_mut() {
                    previous.end = Some(route.ordinal);
                    if previous.form != route.from {
                        let start = previous.start;
                        visits.push(RouteVisit {
                            form: route.from,
                            produced: false,
                            discontinuity: true,
                            start,
                            end: Some(route.ordinal),
                        });
                    }
                } else {
                    visits.push(RouteVisit {
                        form: route.from,
                        produced: false,
                        discontinuity: false,
                        start: 0,
                        end: Some(route.ordinal),
                    });
                }
                visits.push(RouteVisit {
                    form: route.to,
                    produced: false,
                    discontinuity: false,
                    start: route.ordinal,
                    end: None,
                });
            }
            for visit in &mut visits {
                visit.produced = outputs
                    .get(&(task, partner, visit.form))
                    .and_then(|ordinals| {
                        ordinals.get(ordinals.partition_point(|ordinal| *ordinal <= visit.start))
                    })
                    .is_some_and(|ordinal| visit.end.is_none_or(|end| *ordinal < end));
            }
            let name = self.snapshot.routes[*receipts.last().expect("nonempty route group")]
                .owner
                .name
                .clone();
            self.traces.push(RouteTrace {
                task,
                partner,
                name,
                visits,
                receipts,
            });
        }
    }
    pub fn routes_for_task(&self, task: i64) -> &[RouteTrace] {
        let start = self.traces.partition_point(|trace| trace.task < task);
        let end = self.traces.partition_point(|trace| trace.task <= task);
        &self.traces[start..end]
    }
    fn acknowledge(&mut self, acknowledged: &mut Vec<LocalIntent>) {
        self.local
            .retain(|intent| !acknowledged.iter().any(|saved| saved == intent));
        acknowledged.clear();
        self.invalidated = true;
    }
    pub fn selected_habit(&self) -> Option<&records::Habit> {
        self.habits.selected(&self.snapshot.habits)
    }
    pub fn resolution_choices(&self) -> impl Iterator<Item = &records::ResolutionChoice> {
        self.snapshot.resolution_choices.iter().filter(|choice| {
            self.resolution_signature
                .as_ref()
                .is_some_and(|signature| signature == &choice.signature)
        })
    }
    pub fn retry(&mut self) {
        if let Some(request) = self.failed.take() {
            self.pending = Some(request);
            self.failed_error.clear();
        } else {
            self.load_failed = false;
            self.invalidated = true;
        }
        self.invalidate_review();
    }
}
fn id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "local-{}-{epoch}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

pub fn command_help(command: &str) -> Option<Message> {
    Some(match command.trim_start_matches('/') {
        "partner-summary" => Message::RecordsHelpSummary,
        "ribbons" => Message::RibbonsHelp,
        "ribbon-hide" => Message::RibbonsHelpHide,
        "ribbon-show" => Message::RibbonsHelpShow,
        "ribbon-inspect" => Message::RibbonsHelpInspect,
        "routes" => Message::RecordsHelpRoutes,
        "dex" => Message::RecordsHelpDex,
        "dex-resolve" => Message::RecordsHelpResolve,
        "habits" => Message::RecordsHelpHabits,
        "habit-approve" => Message::RecordsHelpHabitApprove,
        "habit-dismiss" => Message::RecordsHelpHabitDismiss,
        "pairings" => Message::RecordsHelpPairings,
        "save-pairing" => Message::RecordsHelpPairingSave,
        "remove-pairing" => Message::RecordsHelpPairingRemove,
        "flavor-off" => Message::RecordsHelpFlavorOff,
        "flavor-on" => Message::RecordsHelpFlavorOn,
        "records-retry" => Message::RecordsHelpRetry,
        "records-discard" => Message::RecordsHelpDiscard,
        _ => return None,
    })
}

impl App {
    pub fn records_invalidate(&mut self) {
        self.records.invalidated = true;
        self.records.load_failed = false;
    }
    pub fn queue_record_operation(&mut self, task: i64, operation: Operation) -> bool {
        self.records.enqueue(Request {
            task_id: task,
            operation,
        })
    }
    pub fn records_can_switch(&self) -> bool {
        self.records.restoring || self.records.local.len() < LOCAL_CAPACITY
    }
    pub fn record_form_marker(&self, form: usize) -> &FormMarker {
        &self.records.markers[form.min(8)]
    }
    pub fn record_form_switch(&mut self, from: usize, to: usize) {
        self.record_switch(from, to, None);
    }
    pub fn record_suggested_form_switch(&mut self, from: usize, to: usize, trigger: String) {
        self.record_switch(from, to, Some(trigger));
    }
    fn record_switch(&mut self, from: usize, to: usize, accepted_suggestion: Option<String>) {
        if self.records.restoring || from == to || from >= 9 || to >= 9 {
            return;
        }
        let mut owner = self.attribution();
        owner.form = to;
        self.records.pairing_context = Some((self.task_id, owner.partner, to));
        self.records.restored_pairing = None;
        self.records.pairings.offer = None;
        self.records.local.push(LocalIntent::FormRoute {
            id: id(),
            owner,
            from,
            to,
            accepted_suggestion,
        });
    }
    pub fn record_artifact(&mut self, kind: ArtifactKind, reference: impl Into<String>) {
        if self.records.restoring {
            return;
        }
        let reference = reference.into();
        let owner = self.attribution();
        let current = Some(self.workflow_fingerprint());
        if self.records.local.iter().any(|intent| matches!(intent, LocalIntent::Artifact { owner: old, kind: old_kind, reference: old_ref, .. } if old == &owner && old_kind == &kind && old_ref == &reference)) { return; }
        self.records.local.push(LocalIntent::Artifact {
            id: id(),
            owner,
            kind,
            reference,
            current,
        });
    }
    pub fn records_task_saved(&mut self, saved: &mut store::Snapshot) {
        self.records.acknowledge(&mut saved.local_records);
    }
    pub fn record_request_dispatched(&mut self, partner: i64, request_id: &str) -> bool {
        if self.records.pending.iter().chain(self.records.inflight.iter()).chain(self.records.failed.iter()).chain(self.records.queued.iter()).any(|request| matches!(&request.operation, Operation::ObserveInstruction { request_id: old, .. } if old == request_id)) { return true; }
        let Some(state) = self.planning_state(partner) else {
            return false;
        };
        let Some(work) = state
            .work
            .as_ref()
            .filter(|work| work.request_id == request_id)
        else {
            return false;
        };
        let Some(task) = state.immutable_task() else {
            return false;
        };
        let instruction = task.request_text().to_string();
        // Privacy rejection is not a write and must never hold an active claim.
        if records::instruction_key(&instruction).is_err() {
            return true;
        }
        let task_id = work.task_id;
        let owner = work.owner.clone();
        if self.records.count() >= CAPACITY {
            self.records.hint = Some(Message::RecordsQueueFull);
            return false;
        }
        if !self
            .planning_state_mut(partner)
            .is_some_and(|state| state.hold_local_record(request_id))
        {
            return false;
        }
        self.queue_record_operation(
            task_id,
            Operation::ObserveInstruction {
                request_id: request_id.into(),
                owner,
                instruction,
            },
        )
    }
    fn release_record_owner(&mut self, request: &Request) {
        if let Operation::ObserveInstruction {
            request_id, owner, ..
        } = &request.operation
            && let Some(state) = self.planning_state_mut(owner.partner)
        {
            state.release_local_record(request_id);
        }
    }
    pub fn record_handoff_ready(&mut self) {
        if !matches!(self.handoff.mode, super::handoff::Mode::Export(_))
            || !self.records_can_switch()
        {
            return;
        }
        use sha2::{Digest, Sha256};
        let Some(document) = self.handoff.candidate.as_ref() else {
            return;
        };
        if document.task.id != self.task_id || document.task.revision != self.revision {
            return;
        }
        let Ok(bytes) = serde_json::to_vec(document) else {
            return;
        };
        let digest = format!("{:x}", Sha256::digest(bytes));
        let receipt_id = format!(
            "handoff-{}-{}-{digest}",
            self.task_id,
            self.attribution().partner
        );
        if self
            .records
            .local
            .iter()
            .any(|intent| intent.id() == receipt_id)
            || self
                .records
                .snapshot
                .artifacts
                .iter()
                .any(|view| view.artifact.id == receipt_id)
        {
            return;
        }
        let evidence = document
            .sources
            .iter()
            .map(|source| crate::workspace::workflows::EvidenceRef::Source {
                task_id: document.task.id,
                excerpt_id: source.excerpt_id,
                path: source.path.clone(),
                start_line: source.start_line,
                end_line: source.end_line,
                source_sha256: source.source_sha256,
            })
            .collect();
        self.records.local.push(LocalIntent::HandoffReady {
            id: receipt_id,
            owner: self.attribution(),
            revision: document.task.revision,
            digest,
            evidence,
        });
        // A factual receipt does not change the document's committed revision.
        self.save_requested = true;
        self.flush_save();
    }
    pub fn drive_records(&mut self) -> bool {
        let mut changed = false;
        let visible = matches!(
            self.overlay,
            Some(Overlay::Records | Overlay::Forms | Overlay::Formations)
        ) || self.formation_inspector_active();
        let current_changed = {
            let unknown = InputSnapshot::default();
            let current = if self.repo_loading || self.repo_refresh || !self.repo_error.is_empty() {
                &unknown
            } else {
                &self.suggestion_cache.current
            };
            if &self.records.current == current {
                false
            } else {
                self.records.current = current.clone();
                true
            }
        };
        if self.records.task != self.task_id || self.records.project != self.cwd {
            self.records.pairing_context = None;
            self.records.restored_pairing = None;
            self.records.task = self.task_id;
            self.records.project = self.cwd.clone();
            self.records.loaded = false;
            self.records.load_failed = false;
            self.records.invalidated = true;
            self.records.snapshot.artifacts.clear();
            self.records.snapshot.routes.clear();
            self.records.snapshot.held_items.clear();
            self.records.snapshot.ribbons.clear();
            self.records.markers = Default::default();
            self.records.invalidate_review();
            changed |= visible;
        }
        if current_changed {
            self.records.invalidated = true;
            self.records.load_failed = false;
            self.records.invalidate_review();
            changed |= visible;
        }
        changed |= self.queue_memory_binding();
        if self.records.invalidated
            && !self.records.load_failed
            && !self.records.persistence_pending()
        {
            self.records.invalidated = false;
            self.queue_record_operation(
                self.task_id,
                Operation::Load {
                    current: Some(self.records.current.clone()),
                },
            );
            changed |= visible;
        }
        if !self.records.snapshot.flavor_enabled {
            self.records.lines.clear();
            self.records.current_activity = None;
        }
        // The root paints this separate factual slot only when no safety/error
        // surface needs it. Never overwrite an existing ordinary or safety notice.
        if !self.planning_any_busy()
            && !self.planning_storage_failed()
            && !matches!(self.storage, super::StorageState::Failed(_))
            && self.records.failed.is_none()
            && let Some(activity) = self.records.lines.pop_front()
        {
            self.records.current_activity = Some(activity);
            changed = true;
        }
        changed |= self.reconcile_held();
        changed
    }
    pub fn submit_record_store(&mut self, worker: &store::Worker) -> usize {
        if self.records.inflight.is_some() || self.records.failed.is_some() {
            return 0;
        }
        if self.records.pending.is_none() {
            self.records.pending = self.records.queued.pop_front();
        }
        if self.records.pending.is_none() {
            return 0;
        }
        // Retain exact correlation once, not on every full-queue retry.
        if self.records.submission_receipt.is_none() {
            self.records.submission_receipt = self.records.pending.clone();
        }
        if worker.try_records(&mut self.records.pending) {
            self.records.inflight = self.records.submission_receipt.take();
            1
        } else {
            0
        }
    }
    /// Apply every correlated result, but repaint only its visible consequences.
    pub fn records_event(
        &mut self,
        request: Request,
        result: Result<Box<Response>, String>,
    ) -> bool {
        if self.records.inflight.as_ref() != Some(&request) {
            return false;
        }
        self.records.inflight = None;
        let mut changed = !matches!(request.operation, Operation::Load { .. })
            || matches!(
                self.overlay,
                Some(
                    Overlay::Records
                        | Overlay::Forms
                        | Overlay::Held
                        | Overlay::Sources
                        | Overlay::Formations
                )
            )
            || self.formation_inspector_active();
        match result {
            Err(error) => {
                self.records.notice = error.clone();
                if self.held_request_owned(&request) {
                    self.held.notice = None;
                }
                if mutation(&request) {
                    self.records.failed = Some(request);
                    self.records.failed_error = error;
                } else {
                    self.records.load_failed = true;
                    self.records.hint = Some(Message::RecordsLoadFailed);
                }
            }
            Ok(response) => {
                self.release_record_owner(&request);
                if let Response::Snapshot(mut snapshot) = *response {
                    self.records.snapshot.flavor_enabled = snapshot.flavor_enabled;
                    if !snapshot.flavor_enabled {
                        changed |= self.records.current_activity.is_some();
                        self.records.lines.clear();
                        self.records.current_activity = None;
                    }
                    for activity in snapshot.activities.drain(..) {
                        if !self.records.delivered.contains(&activity.id) {
                            self.records.delivered.insert(activity.id.clone());
                            if snapshot.flavor_enabled && self.records.lines.len() < CAPACITY {
                                self.records.lines.push_back(activity);
                            }
                        }
                    }
                    self.records.pairings.committed(&snapshot.pairings);
                    if let Operation::SetPairing {
                        partner,
                        form,
                        selection: None,
                    } = &request.operation
                        && self
                            .records
                            .pairings
                            .offer
                            .as_ref()
                            .is_some_and(|offer| offer.partner == *partner && offer.form == *form)
                    {
                        self.records.pairings.offer = None;
                    }
                    if request.task_id == self.task_id {
                        self.sources
                            .set_failure_paths(snapshot.signatures.iter().flat_map(|signature| {
                                signature.paths.iter().map(std::path::PathBuf::from)
                            }));
                        self.records.snapshot = *snapshot;
                        if self.held_request_owned(&request) {
                            self.held.notice = Some(super::held::Notice::Saved);
                        }
                        self.records.loaded = true;
                        self.records.load_failed = false;
                        self.records.notice.clear();
                        self.records.hint = None;
                        for habit in self.records.snapshot.habits.iter().filter(|habit| {
                            super::habits::eligible(habit) && habit.memory.is_none()
                        }) {
                            if !self.records.notified_habits.contains(&habit.key) {
                                changed |= !self.records.habit_hint;
                                self.records.notified_habits.insert(habit.key.clone());
                                self.records.habit_hint = true;
                            }
                        }
                        if self.records.habit_hint {
                            self.records.hint = Some(Message::RecordsHabitAvailable);
                        }
                        self.records.rebuild_markers();
                        self.records.rebuild_routes();
                        self.records
                            .habits
                            .select(self.records.habits.selected, &self.records.snapshot.habits);
                        self.records.invalidate_review();
                        if let Operation::Load { current } = &request.operation
                            && current.as_ref() != Some(&self.records.current)
                        {
                            self.records.invalidated = true;
                        }
                    } else {
                        self.records.invalidated = true;
                    }
                }
                changed |= self.records_restore_pairing();
                if mutation(&request) {
                    self.records.invalidated = true;
                }
            }
        }
        let held_visible = self.held.target.is_some()
            || self.held.notice.is_some()
            || self
                .records
                .snapshot
                .held_items
                .iter()
                .any(|item| item.partner == self.lead().id && item.form == self.form());
        let revision = self.revision;
        changed |= self.reconcile_held() && (held_visible || self.revision != revision);
        changed
    }
    pub fn records_pairing_selected(&mut self) {
        if !self.planning.take_pairing_offer() {
            return;
        }
        let owner = self.attribution();
        let context = (self.task_id, owner.partner, owner.form);
        if self.records.pairing_context != Some(context) {
            self.records.restored_pairing = None;
        }
        self.records.pairing_context = Some(context);
        // A real new choice supersedes the old cue; later factual events win again.
        self.records.current_activity = None;
        self.records.pairings.offer_selection(
            &owner,
            self.planning.agent,
            &self.planning.selection,
        );
    }
    /// Restore an idle route once per data identity; report a visible change, not readiness.
    pub fn records_restore_pairing(&mut self) -> bool {
        if !self.records.loaded {
            return false;
        }
        // Automatic selection preserves the actual provider/model/effort, even
        // when a committed pairing exists for the newly selected specialist.
        if self.lead().auto_form {
            return false;
        }
        let owner = self.attribution();
        let context = (self.task_id, owner.partner, owner.form);
        if self.records.pairing_context != Some(context) {
            self.records.pairing_context = Some(context);
            self.records.restored_pairing = None;
            self.records.pairings.offer = None;
        }
        self.records_pairing_selected();
        if self
            .records
            .pairings
            .offer
            .as_ref()
            .is_some_and(|offer| offer.partner == owner.partner && offer.form == owner.form)
        {
            return false;
        }
        let pairing = super::pairings::remembered(&self.records.snapshot.pairings, &owner);
        if self
            .records
            .restored_pairing
            .as_ref()
            .is_some_and(|(task, partner, form, saved)| {
                (*task, *partner, *form) == context && saved.as_ref() == pairing
            })
        {
            return false;
        }
        let changed = if let Some(pairing) = pairing {
            self.planning.agent != pairing.agent
                || self.planning.selection != pairing.selection
                || self.planning.remembered_pairing() != Some((pairing.partner, pairing.form))
        } else {
            self.planning.remembered_pairing().is_some()
        };
        let applied = if let Some(pairing) = pairing {
            self.planning.preselect_pairing(pairing)
        } else {
            self.planning.clear_pairing()
        };
        if applied {
            self.records.restored_pairing =
                Some((context.0, context.1, context.2, pairing.cloned()));
        }
        applied && changed
    }
    pub fn has_pairing_offer(&self) -> bool {
        self.records.pairings.offer.as_ref().is_some_and(|offer| {
            offer.partner == self.lead.id
                && offer.form == self.lead.form
                && offer.agent == self.planning.agent
                && offer.selection == self.planning.selection
        })
    }
    pub fn record_habit_hint(&self) -> Option<Message> {
        self.records
            .habit_hint
            .then_some(Message::RecordsHabitAvailable)
    }
    pub fn record_activity(&self) -> Option<&records::Activity> {
        self.records
            .snapshot
            .flavor_enabled
            .then_some(self.records.current_activity.as_ref())
            .flatten()
    }
    pub fn records_pairing_badge(&self, known_available: bool) -> Option<Message> {
        let available = known_available
            && self
                .planning
                .catalogues
                .get(&self.planning.agent)
                .is_some_and(|catalogue| {
                    self.planning
                        .selection
                        .model
                        .as_ref()
                        .or(catalogue.current.as_ref())
                        .and_then(|id| catalogue.models.iter().find(|model| &model.id == id))
                        .is_some_and(|model| {
                            self.planning
                                .selection
                                .effort
                                .as_ref()
                                .is_none_or(|effort| model.efforts.contains(effort))
                        })
                });
        self.planning.remembered_pairing().map(|_| {
            if available {
                Message::RecordsRemembered
            } else {
                Message::RecordsRememberedUnavailable
            }
        })
    }
    fn queue_memory_binding(&mut self) -> bool {
        if self.records.count() >= CAPACITY {
            return false;
        }
        let Some((key, memory, revision)) = self.memory.take_habit_binding() else {
            return false;
        };
        self.queue_record_operation(
            self.task_id,
            Operation::BindInstructionMemory {
                key,
                memory,
                revision,
            },
        )
    }
    pub fn records_memory_committed(&mut self) {
        self.queue_memory_binding();
        self.records_invalidate();
    }
    pub fn record_instruction_replacement(&self) -> Option<records::InstructionReplacement> {
        let Some(crate::providers::Request::Preview(input)) = &self.planning.pending else {
            return None;
        };
        super::habits::replacement(
            &self.records.snapshot.habits,
            &input.task.approved_memories,
            &input.task.project,
            input.task.request_text(),
        )
    }
    pub fn open_records(&mut self, mode: Mode) {
        if self.editing {
            self.retain_action_editor();
        }
        self.records.enter(mode);
        self.open_overlay(Overlay::Records);
        self.records_invalidate();
    }
    pub fn record_command(&mut self, command: &str) -> bool {
        let command = command.trim().trim_start_matches('/');
        let mode = match command {
            "partner-summary" => Mode::Summary,
            "ribbons" | "ribbon-hide" | "ribbon-show" | "ribbon-inspect" => Mode::Ribbons,
            "routes" => Mode::Routes,
            "dex" | "dex-resolve" => Mode::Dex,
            "habits" | "habit-approve" | "habit-dismiss" => Mode::Habits,
            "pairings" | "save-pairing" | "remove-pairing" => Mode::Pairings,
            "flavor-on" | "flavor-off" | "records-retry" | "records-discard" => self.records.mode,
            _ => return false,
        };
        let already = self.overlay == Some(Overlay::Records) && self.records.mode == mode;
        let direct_pairing =
            command == "save-pairing" && self.has_pairing_offer() && self.records.failed.is_none();
        if !already && !direct_pairing {
            self.open_records(mode);
        }
        if !already && matches!(command, "ribbon-hide" | "ribbon-show" | "ribbon-inspect") {
            return true;
        }
        if mode == Mode::Habits {
            self.records.habit_hint = false;
            if self.records.hint == Some(Message::RecordsHabitAvailable) {
                self.records.hint = None;
            }
        }
        if command == "records-retry" {
            self.records.retry();
            return true;
        }
        if command == "records-discard" {
            if self.records.failed.is_some() {
                self.records.return_mode = self.records.mode;
                self.records.enter(Mode::Discard);
            }
            return true;
        }
        if self.records.failed.is_some() {
            return true;
        }
        match command {
            "ribbon-hide" | "ribbon-show" => {
                self.record_ribbon_visibility(command == "ribbon-hide")
            }
            "ribbon-inspect" => self.record_ribbon_inspect(),
            "flavor-on" | "flavor-off" => {
                self.queue_record_operation(
                    0,
                    Operation::SetFlavor {
                        enabled: command == "flavor-on",
                    },
                );
            }
            "habit-approve" if self.records.loaded => {
                match self
                    .records
                    .habits
                    .propose(&self.records.snapshot.habits, &mut self.memory)
                {
                    Ok(()) => self.open_overlay(Overlay::Memory),
                    Err(error) => self.records.notice = error,
                }
            }
            "habit-dismiss" if self.records.loaded => {
                match self.records.habits.dismiss(&self.records.snapshot.habits) {
                    Ok(operation) => {
                        self.queue_record_operation(self.task_id, operation);
                    }
                    Err(error) => self.records.notice = error,
                }
            }
            "save-pairing" => {
                let owner = self.attribution();
                match self.records.pairings.save(
                    &owner,
                    self.planning.agent,
                    &self.planning.selection,
                ) {
                    Ok(operation) => {
                        self.queue_record_operation(self.task_id, operation);
                    }
                    Err(error) => self.records.notice = error,
                }
            }
            "remove-pairing" => {
                let owner = self.attribution();
                self.queue_record_operation(self.task_id, super::pairings::State::remove(&owner));
            }
            "dex-resolve" if self.records.loaded => {
                self.records.resolution_signature = self
                    .records
                    .snapshot
                    .signatures
                    .get(self.records.selected)
                    .map(|signature| signature.key.clone());
                self.records.return_mode = Mode::Dex;
                self.records.enter(Mode::Resolve);
            }
            _ => {}
        }
        true
    }
    pub fn record_command_choices(&self) -> Vec<super::commands::Choice> {
        use super::commands::Choice;
        let mut commands = vec![
            Choice::new("partner-summary", "Factual partner summary"),
            Choice::new("ribbons", "Inspect this project's historical milestones"),
            Choice::new("routes", "Local non-evidence form routes"),
            Choice::new("dex", "Project failure signatures"),
            Choice::new("habits", "Review repeated authored instructions"),
            Choice::new("pairings", "Project partner provider pairings"),
            Choice::new("flavor-off", "Disable factual activity lines"),
            Choice::new("flavor-on", "Enable factual activity lines"),
        ];
        if self.records.failed.is_some() || self.records.load_failed {
            commands.push(Choice::new(
                "records-retry",
                "Retry retained records operation",
            ));
        }
        if self.records.failed.is_some() {
            commands.push(Choice::new(
                "records-discard",
                "Review discarding failed records operation",
            ));
        }
        if self.has_pairing_offer() {
            commands.push(Choice::new(
                "save-pairing",
                "Save the explicitly chosen provider/model",
            ));
        }
        match self.records.mode {
            Mode::Ribbons => {
                commands.push(Choice::new(
                    "ribbon-inspect",
                    "Inspect the selected local evidence",
                ));
                commands.push(Choice::new(
                    "ribbon-hide",
                    "Hide the selected historical line",
                ));
                commands.push(Choice::new(
                    "ribbon-show",
                    "Show the selected historical line",
                ));
            }
            Mode::Dex => commands.push(Choice::new(
                "dex-resolve",
                "Review historical diagnosis and passing-run relation",
            )),
            Mode::Habits => {
                commands.push(Choice::new(
                    "habit-approve",
                    "Open explicit project memory approval",
                ));
                commands.push(Choice::new(
                    "habit-dismiss",
                    "Dismiss this exact repeated instruction",
                ));
            }
            Mode::Pairings => {
                commands.push(Choice::new(
                    "remove-pairing",
                    "Remove this form pairing and restore global default",
                ));
            }
            _ => {}
        }
        commands
    }
    pub fn records_allow_quit(&mut self, quit: bool) -> bool {
        self.records.quit_waiting |= quit;
        if !self.records.quit_waiting {
            return false;
        }
        if !self.records.mutation_pending() {
            return true;
        }
        if self.records.failed.is_some() {
            self.records.quit_discard = true;
            if self.overlay != Some(Overlay::Records) || self.records.mode != Mode::Discard {
                self.open_records(Mode::Discard);
                self.records.return_mode = Mode::Summary;
            }
        } else {
            self.records.hint = Some(Message::RecordsWaiting);
        }
        false
    }
    fn record_stay(&mut self) {
        if self.records.quit_discard {
            self.records.quit_discard = false;
            self.cancel_record_quit();
        }
    }
    pub fn record_paste(&mut self, _value: &str) -> bool {
        self.overlay == Some(Overlay::Records)
    }
    pub fn record_key(&mut self, key: KeyEvent) -> bool {
        if self.overlay != Some(Overlay::Records) {
            return false;
        }
        if key.kind != KeyEventKind::Press {
            return true;
        }
        if key.code == KeyCode::Esc {
            if self.records.mode == Mode::Discard {
                self.record_stay();
            }
            if matches!(self.records.mode, Mode::Resolve | Mode::Discard) {
                let mode = self.records.return_mode;
                self.records.enter(mode);
            } else {
                self.close_overlay();
            }
            return true;
        }
        if self.width < 60 || self.height < 18 {
            self.records.invalidate_review();
            return true;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return true;
        }
        match key.code {
            KeyCode::PageDown => {
                self.records.scroll = self
                    .records
                    .scroll
                    .saturating_add(self.records.page)
                    .min(self.records.scroll_max);
                self.records.affirmative = false;
            }
            KeyCode::PageUp => {
                self.records.scroll = self.records.scroll.saturating_sub(self.records.page);
                self.records.affirmative = false;
            }
            KeyCode::Home => {
                self.records.scroll = 0;
                self.records.affirmative = false;
            }
            KeyCode::End => {
                self.records.scroll = self.records.scroll_max;
                self.records.affirmative = false;
            }
            KeyCode::Char('n')
                if key.modifiers.is_empty()
                    && matches!(self.records.mode, Mode::Resolve | Mode::Discard) =>
            {
                self.records.affirmative = false;
            }
            KeyCode::Char('y')
                if key.modifiers.is_empty()
                    && self.records.reviewed
                    && matches!(self.records.mode, Mode::Resolve | Mode::Discard) =>
            {
                self.records.affirmative = true;
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Tab
                if matches!(self.records.mode, Mode::Resolve | Mode::Discard) =>
            {
                if self.records.reviewed {
                    self.records.affirmative = !self.records.affirmative;
                }
            }
            KeyCode::Up | KeyCode::Down if !matches!(self.records.mode, Mode::Discard) => {
                let count = self.record_selection_count();
                self.records.selected = if key.code == KeyCode::Down {
                    (self.records.selected + 1).min(count.saturating_sub(1))
                } else {
                    self.records.selected.saturating_sub(1)
                };
                if self.records.mode == Mode::Habits {
                    self.records
                        .habits
                        .select(self.records.selected, &self.records.snapshot.habits);
                }
                self.records.scroll = 0;
                self.records.invalidate_review();
            }
            KeyCode::Char('r') if self.records.mode == Mode::Dex => self.record_dex_relay(),
            KeyCode::Enter if self.records.mode == Mode::Ribbons => self.record_ribbon_inspect(),
            KeyCode::Char('h') if self.records.mode == Mode::Ribbons => {
                if let Some(view) = self.records.snapshot.ribbons.get(self.records.selected) {
                    self.record_ribbon_visibility(!view.ribbon.hidden);
                }
            }
            KeyCode::Enter if self.records.mode == Mode::Discard && !self.records.affirmative => {
                self.record_stay();
                let mode = self.records.return_mode;
                self.records.enter(mode);
            }
            KeyCode::Enter
                if self.records.mode == Mode::Discard
                    && self.records.reviewed
                    && self.records.affirmative =>
            {
                if let Some(request) = self.records.failed.take() {
                    self.release_record_owner(&request);
                }
                self.records.quit_discard = false;
                self.records.failed_error.clear();
                let mode = self.records.return_mode;
                self.records.enter(mode);
                self.records_invalidate();
            }
            KeyCode::Enter
                if self.records.mode == Mode::Resolve
                    && self.records.reviewed
                    && self.records.affirmative
                    && self.records.failed.is_none() =>
            {
                self.record_resolve_selected()
            }
            _ => {}
        }
        true
    }
    pub fn record_selection_count(&self) -> usize {
        match self.records.mode {
            Mode::Summary => self.records.snapshot.summaries.len(),
            Mode::Dex => self.records.snapshot.signatures.len(),
            Mode::Habits => self
                .records
                .snapshot
                .habits
                .iter()
                .filter(|habit| super::habits::eligible(habit) && habit.memory.is_none())
                .count(),
            Mode::Pairings => self.records.snapshot.pairings.len(),
            Mode::Routes => self.records.snapshot.summaries.len(),
            Mode::Resolve => self.records.resolution_choices().count(),
            Mode::Ribbons => self.records.snapshot.ribbons.len(),
            Mode::Discard => 1,
        }
    }
    fn record_ribbon_visibility(&mut self, hidden: bool) {
        if !self.records.loaded || self.records.persistence_pending() {
            return;
        }
        let Some(view) = self.records.snapshot.ribbons.get(self.records.selected) else {
            return;
        };
        if view.ribbon.hidden != hidden {
            self.queue_record_operation(
                self.task_id,
                Operation::SetRibbonHidden {
                    kind: view.ribbon.kind,
                    hidden,
                },
            );
        }
    }
    fn record_ribbon_inspect(&mut self) {
        use crate::workspace::ribbons::Evidence;
        if !self.records.loaded {
            return;
        }
        let Some(view) = self.records.snapshot.ribbons.get(self.records.selected) else {
            return;
        };
        if matches!(
            view.freshness,
            Freshness::Unavailable | Freshness::Ambiguous
        ) {
            self.records.hint = Some(Message::RibbonsSourceUnavailable);
            return;
        }
        let ribbon = &view.ribbon;
        // Never resume another task, front an attributed partner, or switch form.
        // The ordinary task picker retains its normal explicit navigation guards.
        if ribbon.task_id != self.task_id {
            let task = ribbon.task_id;
            if let Some(source) = self.tasks.iter().find(|source| source.id == task) {
                self.show_archived = source.archived;
                let selected = self
                    .visible_tasks()
                    .position(|source| source.id == task)
                    .unwrap_or(0);
                self.task_selection = selected;
            }
            self.command("tasks");
            return;
        }
        match &ribbon.evidence {
            Evidence::Artifact { .. } => {
                let command = match ribbon.kind {
                    crate::workspace::ribbons::Kind::AcceptedReview => "workflow-review",
                    _ => "handoff",
                };
                self.command(command);
            }
            Evidence::Move { id, .. } => {
                let (id, partner, form) = (*id, ribbon.owner.partner, ribbon.owner.form);
                if partner != self.lead.id {
                    self.command("party");
                } else if form != self.form() {
                    self.command("forms");
                } else {
                    self.open_moves();
                    if self.overlay == Some(Overlay::Moves)
                        && self.moves.records.iter().any(|record| record.id == id)
                    {
                        self.moves.detail = Some(id);
                        self.moves.mode = super::moves::Mode::Detail;
                    }
                }
            }
        }
    }
    fn record_dex_relay(&mut self) {
        if !self.records.loaded || self.records.persistence_pending() {
            return;
        }
        let Some(signature) = self.records.snapshot.signatures.get(self.records.selected) else {
            return;
        };
        let Some(comparison) = self
            .records
            .snapshot
            .failure_comparisons
            .iter()
            .rev()
            .find(|comparison| comparison.signature == signature.key)
        else {
            self.records.hint = Some(Message::DexComparisonUnavailable);
            return;
        };
        let history = comparison.historical.iter().map(|support|
            format!("Historical support {} / passing run {} / diagnosis {:?} / pass {:?} / source {:?}",
                support.support_artifact, support.passing_run, support.diagnosis,
                support.pass, support.source)).collect::<Vec<_>>().join("\n");
        let text = format!(
            "Inspect task {} failure reference {} / signature {}.\nCommand: {}\nCwd: {}\nObservation: {:?}; source {:?}.\n{}\nHistorical/stale observations are investigation leads only, never current verification or inferred causality. Review retained evidence; no automatic fix, send, execution or permission is requested.",
            self.task_id,
            comparison.failure_run,
            comparison.signature,
            super::execution_relays::command_draft(&comparison.command),
            comparison.cwd.display(),
            comparison.disposition,
            comparison.source,
            history
        );
        let request = super::relays::RelayRequest {
            target_form: 5,
            text,
            paths: Vec::new(),
            source: super::relays::RelaySource::DexEvidence {
                signature: signature.key.clone(),
                run: comparison.failure_run.clone(),
            },
            command: None,
        };
        if let Err(error) = self.stage_relay(request) {
            self.records.notice = error;
        }
    }
    fn record_resolve_selected(&mut self) {
        let Some(choice) = self.records.resolution_choices().nth(self.records.selected) else {
            return;
        };
        let operation = Operation::ResolveFailure {
            signature: choice.signature.clone(),
            support_artifact: choice.support_artifact.clone(),
            passing_run: choice.passing_run.clone(),
        };
        if self.queue_record_operation(self.task_id, operation) {
            self.records.enter(Mode::Dex);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner() -> store::Attribution {
        store::Attribution {
            partner: 1,
            name: "Bolt".into(),
            form: 0,
        }
    }
    #[test]
    fn quit_intent_survives_metadata_ack_and_stay_cancels_it() {
        let mut app = App::new("/project".into(), 120, 36);
        assert!(!app.records_allow_quit(false));
        app.apply_execution_action(super::super::execution::Action::SelectForm(1));
        assert!(!app.records.local.is_empty());
        assert!(!app.records_allow_quit(true));
        let mut saved = app.snapshot("/project");
        app.records_task_saved(&mut saved);
        assert!(app.records_allow_quit(false));
        app.cancel_record_quit();
        assert!(!app.records_allow_quit(false));
        assert!(!app.quit_after_save);
    }

    #[test]
    fn acknowledged_local_intents_remove_only_exact_saved_receipts() {
        let first = LocalIntent::FormRoute {
            id: "one".into(),
            owner: owner(),
            from: 0,
            to: 1,
            accepted_suggestion: None,
        };
        let second = LocalIntent::FormRoute {
            id: "two".into(),
            owner: owner(),
            from: 1,
            to: 2,
            accepted_suggestion: None,
        };
        let mut state = State {
            local: vec![first.clone(), second.clone()],
            ..State::default()
        };
        let mut wrong = first.clone();
        if let LocalIntent::FormRoute { to, .. } = &mut wrong {
            *to = 8;
        }
        state.acknowledge(&mut vec![wrong]);
        assert_eq!(state.local.len(), 2);
        state.acknowledge(&mut vec![first]);
        assert_eq!(state.local, vec![second]);
    }
    #[test]
    fn failed_write_keeps_exact_owner_instruction_and_blocks_later_submission() {
        let mut state = State::default();
        let request = Request {
            task_id: 7,
            operation: Operation::ObserveInstruction {
                request_id: "request-1".into(),
                owner: owner(),
                instruction: "Keep  two spaces\n".into(),
            },
        };
        state.failed = Some(request.clone());
        assert!(state.mutation_pending());
        assert!(state.enqueue(Request {
            task_id: 9,
            operation: Operation::SetFlavor { enabled: false }
        }));
        state.retry();
        assert_eq!(state.pending, Some(request));
        assert_eq!(state.queued.len(), 1);
    }
    #[test]
    fn read_failure_never_traps_quit_and_retries_only_explicitly() {
        let mut state = State {
            load_failed: true,
            ..State::default()
        };
        assert!(!state.mutation_pending());
        state.retry();
        assert!(!state.load_failed);
        assert!(state.invalidated);
    }
    #[test]
    fn neutral_enter_repeat_and_paste_cannot_commit_a_resolution() {
        let mut app = App::new("/project".into(), 60, 18);
        app.overlay = Some(Overlay::Records);
        app.records.enter(Mode::Resolve);
        app.records.reviewed = true;
        app.record_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let mut repeated = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        repeated.kind = KeyEventKind::Repeat;
        app.record_key(repeated);
        assert!(!app.records.affirmative);
        assert!(app.record_paste("yes\n"));
        assert!(!app.records.persistence_pending());
    }
    #[test]
    fn fresh_activity_is_deduplicated_and_off_consumes_without_replay() {
        let mut app = App::new("/project".into(), 60, 18);
        let request = Request {
            task_id: 0,
            operation: Operation::Load { current: None },
        };
        let activity = records::Activity {
            id: "committed-run".into(),
            task_id: 7,
            owner: Some(owner()),
            at: 1,
            detail: "terminal fact".into(),
        };
        let mut snapshot = app.records.snapshot.clone();
        snapshot.activities = vec![activity.clone()];
        app.records.inflight = Some(request.clone());
        app.records_event(
            request.clone(),
            Ok(Box::new(Response::Snapshot(Box::new(snapshot.clone())))),
        );
        assert_eq!(app.records.lines.len(), 1);
        app.records.inflight = Some(request.clone());
        app.records_event(
            request.clone(),
            Ok(Box::new(Response::Snapshot(Box::new(snapshot.clone())))),
        );
        assert_eq!(app.records.lines.len(), 1);
        snapshot.flavor_enabled = false;
        snapshot.activities[0].id = "while-off".into();
        app.records.inflight = Some(request.clone());
        app.records_event(
            request.clone(),
            Ok(Box::new(Response::Snapshot(Box::new(snapshot.clone())))),
        );
        assert!(app.records.lines.is_empty());
        snapshot.flavor_enabled = true;
        app.records.inflight = Some(request.clone());
        app.records_event(
            request,
            Ok(Box::new(Response::Snapshot(Box::new(snapshot)))),
        );
        assert!(app.records.lines.is_empty());
    }
    #[test]
    fn bounded_queue_retains_all_accepted_requests_when_next_is_refused() {
        let mut state = State::default();
        for index in 0..CAPACITY {
            assert!(state.enqueue(Request {
                task_id: 7,
                operation: Operation::ObserveRun {
                    run_id: index.to_string()
                }
            }));
        }
        assert!(!state.enqueue(Request {
            task_id: 7,
            operation: Operation::ObserveRun {
                run_id: "overflow".into()
            }
        }));
        assert_eq!(state.queued.len(), CAPACITY);
        assert_eq!(state.queued.front().unwrap().task_id, 7);
        assert!(
            matches!(&state.queued.back().unwrap().operation, Operation::ObserveRun { run_id } if run_id == &(CAPACITY - 1).to_string())
        );
    }
    #[test]
    fn intermediate_switches_survive_and_restore_never_creates_a_route() {
        let mut app = App::new("/project".into(), 60, 18);
        app.record_form_switch(0, 1);
        app.record_form_switch(1, 3);
        app.records.restoring = true;
        app.record_form_switch(3, 7);
        assert_eq!(app.records.local.len(), 2);
        assert!(matches!(
            &app.records.local[0],
            LocalIntent::FormRoute {
                from: 0,
                to: 1,
                accepted_suggestion: None,
                ..
            }
        ));
        assert!(matches!(
            &app.records.local[1],
            LocalIntent::FormRoute {
                from: 1,
                to: 3,
                accepted_suggestion: None,
                ..
            }
        ));
    }
    #[test]
    fn marker_counts_deduplicate_logical_artifact_and_preserve_unknown_producer() {
        let mut state = State {
            task: 7,
            ..State::default()
        };
        let artifact = records::Artifact {
            id: "one".into(),
            task_id: 7,
            kind: ArtifactKind::Run,
            role_form: 3,
            ordinal: 1,
            supported_diagnosis: false,
            handoff_revision: None,
            observation: None,
            producer: None,
            reference: "same-run".into(),
            created_at: 1,
            evidence: vec![],
            detail: "exit 101".into(),
        };
        let view = records::ArtifactView {
            artifact,
            freshness: Freshness::Stale,
            explanation: String::new(),
        };
        state.snapshot.artifacts = vec![view.clone(), view];
        state.rebuild_markers();
        assert_eq!(state.markers[3].count, 1);
        assert_eq!(state.markers[3].unknown, 1);
        assert!(state.markers[3].producers.is_empty());
        assert!(state.markers[3].stale);
        assert_eq!(state.markers[0].count, 0);
    }
    #[test]
    fn marker_uses_latest_logical_version_and_current_support_beats_old_stale() {
        let mut state = State {
            task: 7,
            ..State::default()
        };
        let artifact = records::Artifact {
            id: "old".into(),
            task_id: 7,
            kind: ArtifactKind::Run,
            role_form: 3,
            ordinal: 1,
            supported_diagnosis: false,
            handoff_revision: None,
            observation: None,
            producer: Some(owner()),
            reference: "same-run".into(),
            created_at: 500,
            evidence: vec![],
            detail: "old fact".into(),
        };
        let old = records::ArtifactView {
            artifact,
            freshness: Freshness::Stale,
            explanation: "changed source".into(),
        };
        let mut current = old.clone();
        current.artifact.id = "new".into();
        current.artifact.ordinal = 2;
        current.artifact.created_at = 100;
        current.artifact.detail = "new fact".into();
        current.freshness = Freshness::Current;
        current.explanation = "current support".into();
        let mut unknown = old.clone();
        unknown.artifact.reference = "other-run".into();
        unknown.artifact.ordinal = 3;
        unknown.freshness = Freshness::Unavailable;
        unknown.explanation = "retained spool unavailable".into();
        state.snapshot.artifacts = vec![current, unknown, old];
        state.rebuild_markers();
        assert_eq!(state.markers[3].count, 2);
        assert!(!state.markers[3].stale);
        assert!(state.markers[3].fact.contains("new fact"));
        assert!(state.markers[3].fact.contains("retained spool unavailable"));
        assert!(!state.markers[3].fact.contains("old fact"));
    }
    #[test]
    fn repeated_snapshot_restoration_never_overwrites_explicit_newer_choice() {
        let mut app = App::new("/project".into(), 60, 18);
        app.lead.auto_form = false; // This test exercises deliberate manual pairings.
        app.records.loaded = true;
        app.planning.preference_event(Ok(None));
        let owner = app.attribution();
        let saved = records::Pairing {
            partner: owner.partner,
            form: owner.form,
            agent: crate::providers::Agent::Codex,
            selection: crate::providers::ModelSelection {
                model: Some("saved/model".into()),
                effort: None,
            },
        };
        app.records.snapshot.pairings.push(saved.clone());
        app.records_restore_pairing();
        assert_eq!(app.planning.selection, saved.selection);
        let newer = crate::providers::ModelSelection {
            model: Some("explicit/new".into()),
            effort: None,
        };
        assert!(app.planning.select_model_id("explicit/new"));
        app.records_restore_pairing();
        app.records_restore_pairing();
        assert_eq!(app.planning.selection, newer);
        assert!(app.records.pairings.offer.is_some());
        app.lead.form = 1;
        app.record_form_switch(0, 1);
        app.records_restore_pairing();
        app.lead.form = 0;
        app.record_form_switch(1, 0);
        app.records_restore_pairing();
        assert_eq!(app.planning.selection, saved.selection);
    }
    #[test]
    fn quit_discard_review_survives_repeated_gate_and_stay_retains_failed_owner() {
        let mut app = App::new("/project".into(), 60, 18);
        app.records.failed = Some(Request {
            task_id: 7,
            operation: Operation::SetFlavor { enabled: false },
        });
        app.quit = true;
        assert!(!app.records_allow_quit(true));
        app.records.reviewed = true;
        app.records.scroll = 3;
        assert!(!app.records_allow_quit(true));
        assert!(app.records.reviewed);
        assert_eq!(app.records.scroll, 3);
        app.record_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.quit);
        assert!(!app.quit_after_save);
        assert!(app.records.failed.is_some());
        app.quit = true;
        assert!(!app.records_allow_quit(true));
        app.record_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.quit);
        assert!(!app.quit_after_save);
        assert!(app.records.failed.is_some());
    }
    #[test]
    fn route_visits_use_frozen_owner_and_exclusive_intervals_without_inventing_edges() {
        let route = |ordinal, from, to| records::Route {
            id: format!("route-{ordinal}"),
            task_id: 7,
            ordinal,
            owner: crate::store::Attribution {
                partner: 1,
                name: "Bolt".into(),
                form: to,
            },
            from,
            to,
            at: 1,
            accepted_suggestion: None,
        };
        let artifact = |task_id, partner, form, ordinal| records::ArtifactView {
            artifact: records::Artifact {
                id: format!("artifact-{task_id}-{partner}-{ordinal}"),
                task_id,
                kind: ArtifactKind::Handoff,
                role_form: 7,
                ordinal,
                supported_diagnosis: false,
                handoff_revision: None,
                observation: None,
                producer: (partner != 0).then(|| crate::store::Attribution {
                    partner,
                    name: "Owner".into(),
                    form,
                }),
                reference: format!("handoff-{ordinal}"),
                created_at: 1,
                evidence: vec![],
                detail: "handoff ready".into(),
            },
            freshness: Freshness::Stale,
            explanation: String::new(),
        };
        let mut state = State::default();
        state.snapshot.routes = vec![route(80, 0, 2), route(20, 0, 1), route(40, 1, 3)];
        state.snapshot.artifacts = vec![
            artifact(7, 1, 0, 20),
            artifact(7, 1, 1, 25),
            artifact(7, 1, 3, 40),
            artifact(7, 2, 3, 70),
            artifact(8, 1, 3, 60),
            artifact(7, 0, 3, 61),
            artifact(7, 1, 0, 79),
            artifact(7, 1, 2, 81),
        ];
        state.rebuild_routes();
        let visits = &state.routes_for_task(7)[0].visits;
        assert_eq!(
            visits
                .iter()
                .map(|visit| (visit.form, visit.produced, visit.discontinuity))
                .collect::<Vec<_>>(),
            vec![
                (0, false, false),
                (1, true, false),
                (3, false, false),
                (0, true, true),
                (2, true, false),
            ]
        );
        assert!(state.routes_for_task(8).is_empty());
        state.snapshot.artifacts.push(artifact(7, 1, 3, 41));
        state.rebuild_routes();
        assert!(state.routes_for_task(7)[0].visits[2].produced);
    }

    fn ribbon(
        kind: crate::workspace::ribbons::Kind,
        hidden: bool,
    ) -> crate::workspace::ribbons::View {
        crate::workspace::ribbons::View {
            ribbon: crate::workspace::ribbons::Ribbon {
                kind,
                task_id: 7,
                owner: owner(),
                at: 1,
                evidence: crate::workspace::ribbons::Evidence::Move {
                    id: 41,
                    revision: 2,
                },
                hidden,
            },
            freshness: Freshness::Current,
            explanation: String::new(),
        }
    }

    #[test]
    fn ribbon_visibility_keeps_selected_kind_and_failed_intent_until_explicit_retry() {
        use crate::workspace::ribbons::Kind;
        let mut app = App::new("/project".into(), 60, 18);
        app.task_id = 7;
        app.records.loaded = true;
        app.records.snapshot.ribbons = vec![
            ribbon(Kind::AcceptedReview, false),
            ribbon(Kind::LearnedMove, false),
        ];
        app.open_records(Mode::Ribbons);
        app.record_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.record_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        let request = app.records.queued.pop_front().unwrap();
        assert_eq!(request.task_id, 7);
        assert!(matches!(
            request.operation,
            Operation::SetRibbonHidden {
                kind: Kind::LearnedMove,
                hidden: true
            }
        ));
        assert!(
            !app.records.snapshot.ribbons[1].ribbon.hidden,
            "uncommitted visibility must not be painted as durable"
        );
        app.records.inflight = Some(request.clone());
        app.records_event(request.clone(), Err("disk full".into()));
        assert_eq!(app.records.failed.as_ref(), Some(&request));
        assert!(!app.records_allow_quit(true));
        assert_eq!(app.records.mode, Mode::Discard);
        app.records.retry();
        assert_eq!(app.records.pending.as_ref(), Some(&request));
        assert!(app.planning.pending.is_none());
        assert!(!app.execution.busy());
    }

    #[test]
    fn ribbon_navigation_never_switches_foreign_task_partner_or_form() {
        use crate::workspace::ribbons::Kind;
        let mut app = App::new("/project".into(), 120, 36);
        app.task_id = 9;
        app.input = "literal 한글 draft".into();
        app.cursor = 8;
        app.editing = true;
        app.records.loaded = true;
        app.records.snapshot.ribbons = vec![ribbon(Kind::LearnedMove, false)];
        let original = (app.task_id, app.lead.id, app.form());
        app.command("ribbons");
        app.record_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.overlay, Some(Overlay::Tasks));
        assert_eq!((app.task_id, app.lead.id, app.form()), original);
        assert!(!app.execution.busy());
        assert!(app.planning.pending.is_none());
        assert!(app.handoff.pending.is_none());
        app.close_overlay();
        assert_eq!(app.overlay, Some(Overlay::Records));
        app.record_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.editing);
        assert_eq!(app.input, "literal 한글 draft");
        assert_eq!(app.cursor, 8);
    }

    #[test]
    fn direct_ribbon_actions_require_a_visible_selection_and_unavailable_sources_do_not_navigate() {
        use crate::workspace::ribbons::Kind;
        let mut app = App::new("/project".into(), 120, 36);
        app.records.loaded = true;
        app.records.snapshot.ribbons = vec![ribbon(Kind::LearnedMove, true)];
        app.command("ribbon-show");
        assert_eq!(app.overlay, Some(Overlay::Records));
        assert!(app.records.queued.is_empty());
        app.records.snapshot.ribbons[0].freshness = Freshness::Unavailable;
        app.record_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.overlay, Some(Overlay::Records));
        assert!(app.moves.pending.is_none());
        app.command("ribbon-show");
        assert!(matches!(
            app.records.queued.front().map(|request| &request.operation),
            Some(Operation::SetRibbonHidden { hidden: false, .. })
        ));
    }

    #[test]
    fn ribbon_move_inspection_offers_owner_navigation_without_fronting_or_using() {
        use crate::workspace::ribbons::Kind;
        let mut app = App::new("/project".into(), 120, 36);
        app.task_id = 7;
        app.records.loaded = true;
        let mut view = ribbon(Kind::LearnedMove, false);
        view.ribbon.owner.partner = app.lead.id + 100;
        app.records.snapshot.ribbons = vec![view];
        let original = (app.lead.id, app.form());
        app.open_records(Mode::Ribbons);
        app.record_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.overlay, Some(Overlay::Party));
        assert_eq!((app.lead.id, app.form()), original);
        assert!(app.moves.pending.is_none());
        assert!(app.planning.pending.is_none());
        assert!(!app.execution.busy());
    }

    #[test]
    fn changing_project_removes_old_ribbon_view_without_dropping_pending_visibility() {
        use crate::workspace::ribbons::Kind;
        let mut app = App::new("/next-project".into(), 120, 36);
        app.records.project = "/previous-project".into();
        app.records.snapshot.ribbons = vec![ribbon(Kind::LearnedMove, false)];
        let request = Request {
            task_id: 7,
            operation: Operation::SetRibbonHidden {
                kind: Kind::LearnedMove,
                hidden: true,
            },
        };
        app.records.pending = Some(request.clone());
        app.drive_records();
        assert!(app.records.snapshot.ribbons.is_empty());
        assert!(!app.records.loaded);
        assert_eq!(app.records.pending.as_ref(), Some(&request));
    }
}
