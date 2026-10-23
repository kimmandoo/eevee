//! Move actions enter the existing execution, source and provider state machines.
//! Display labels never enter outgoing content; frozen evidence owns every use.
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::{App, Overlay, StorageState, sources};
use crate::{
    providers, store,
    workspace::{
        context,
        moves::{self, Outcome, Pin, Proposal, Provenance, Record, Scope, Steps, Usage},
        runner,
    },
};
use store::moves::{Operation, Request};

#[derive(Clone)]
pub(super) struct Authored {
    pub claim: store::WorkClaim,
    pub instruction: String,
    pub pins: Vec<Pin>,
}
struct PendingRun {
    record: Record,
    command: String,
    cwd: PathBuf,
    revision: u64,
}
struct PinFlow {
    record: Record,
    task: i64,
    partner: i64,
    remaining: VecDeque<Pin>,
    current: Option<Pin>,
    request: Option<String>,
}
#[derive(Default)]
pub(super) struct State {
    owner: Option<(i64, i64, usize)>,
    picker_moves: BTreeMap<(i64, usize), Result<Vec<String>, String>>,
    validation: Option<(i64, u64, Record, super::moves::Mode)>,
    prepared_requests: BTreeMap<i64, Authored>,
    last_requests: BTreeMap<(i64, i64, usize), Authored>,
    pending_run: Option<PendingRun>,
    staged_runs: BTreeMap<i64, Record>,
    run_uses: BTreeMap<String, Record>,
    provider_uses: BTreeMap<String, (Record, bool)>,
    pending_request: Option<Record>,
    pin_flow: Option<PinFlow>,
    task_save: Option<(i64, u64, Record)>,
    observed_runs: HashSet<String>,
}

fn identity() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "move-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
fn pins(excerpts: &[context::Excerpt]) -> Result<Vec<Pin>, String> {
    excerpts.iter().map(Pin::from_excerpt).collect()
}
fn usage(record: &Record, task: i64, outcome: Outcome, revision: Option<String>) -> Usage {
    Usage {
        id: identity(),
        move_id: record.id,
        task_id: task,
        partner: record.proposal.partner,
        run_id: None,
        request_id: None,
        outcome,
        revision,
    }
}
impl App {
    fn queue_move_operation(&mut self, task: i64, partner: i64, form: usize, operation: Operation) {
        self.moves.queue(Request {
            task_id: task,
            partner,
            form,
            operation,
        });
    }
    fn queue_move_use(&mut self, record: &Record, value: Usage) {
        self.queue_move_operation(
            value.task_id,
            record.proposal.partner,
            record.proposal.form,
            Operation::Use { usage: value },
        );
    }
    pub(super) fn move_teaching_proposal(&self) -> Result<Proposal, String> {
        if self.task_id <= 0
            || self.archived
            || !matches!(self.storage, StorageState::Saved)
            || self.saved_revision != self.revision
        {
            return Err("Save an active task before teaching a move.".into());
        }
        let mut provenance = Provenance {
            task_id: self.task_id,
            sources: Vec::new(),
            taught_by: None,
            teacher_name: None,
        };
        let steps = match self.form() {
            0 => {
                provenance
                    .sources
                    .push(format!("task:{}@{}", self.task_id, self.saved_revision));
                Steps::Task {
                    goal: self.goal.clone(),
                    criteria: self.criteria.iter().map(|c| c.text.clone()).collect(),
                }
            }
            1 => {
                provenance.sources.extend(
                    self.sources
                        .basket
                        .iter()
                        .map(|e| format!("source:{}", e.id)),
                );
                moves::pin_steps(&self.sources.basket)?
            }
            2 | 5 | 6 => {
                let record = self
                    .execution
                    .selected_record()
                    .filter(|r| {
                        r.prepared.task_id == self.task_id
                            && r.prepared
                                .partner
                                .as_ref()
                                .is_some_and(|p| p.partner == self.lead.id)
                    })
                    .ok_or(
                        "Select this partner's approved local run in /runs before teaching it.",
                    )?;
                provenance.sources.push(record.prepared.run_id.clone());
                moves::journal_steps(Path::new(&self.cwd), record)?
            }
            _ => {
                let sent=self.move_runtime.last_requests.get(&(self.task_id,self.lead.id,self.form()))
                    .ok_or("Send a user-authored request from this form first. Unsent drafts and provider output cannot be taught.")?;
                provenance.sources.push(sent.claim.request_id.clone());
                Steps::Request {
                    instruction: sent.instruction.clone(),
                    pins: sent.pins.clone(),
                }
            }
        };
        let used = self
            .moves
            .records
            .iter()
            .map(|r| r.proposal.name.clone())
            .collect::<Vec<_>>();
        let proposal = Proposal {
            partner: self.lead.id,
            form: self.form(),
            name: moves::suggested_name(self.form(), &steps, &used),
            scope: Scope::Project,
            steps,
            provenance,
        };
        moves::validate_proposal(&proposal)?;
        Ok(proposal)
    }
    pub(super) fn stage_move_use(&mut self, record: &Record) -> Result<(), String> {
        if self.move_runtime.validation.is_some() || self.moves.persistence_pending() {
            return Err("Wait for the pending move validation or local save.".into());
        }
        moves::validate_proposal(&record.proposal)?;
        // Opening the menu may have just retained its invoking draft/caret.
        // Commit that exact revision before the queued validation callback.
        if self.revision != self.saved_revision {
            self.flush_save();
        }
        self.move_runtime.validation =
            Some((self.task_id, self.revision, record.clone(), self.moves.mode));
        self.queue_move_operation(
            self.task_id,
            self.lead.id,
            self.form(),
            Operation::Validate {
                id: record.id,
                revision: record.revision,
            },
        );
        Ok(())
    }
    pub(super) fn move_validation_finished(
        &mut self,
        task: i64,
        partner: i64,
        form: usize,
        accepted: bool,
    ) {
        let Some((expected_task, revision, record, mode)) = self.move_runtime.validation.take()
        else {
            return;
        };
        if !accepted {
            return;
        }
        if expected_task != task
            || self.task_id != task
            || self.revision != revision
            || self.lead.id != partner
            || self.form() != form
            || self.overlay != Some(Overlay::Moves)
            || self.moves.mode != mode
            || self.quit
            || self.quit_after_save
        {
            self.notice =
                "Move validation finished after its view or task changed; nothing staged.".into();
            return;
        }
        if let Err(error) = self.stage_validated_move(&record) {
            self.moves.notice = error;
        }
    }
    pub(super) fn cancel_move_validation(&mut self) {
        self.move_runtime.validation = None;
    }
    fn stage_validated_move(&mut self, record: &Record) -> Result<(), String> {
        if record.forgotten
            || record.proposal.partner != self.lead.id
            || record.proposal.form != self.form()
        {
            return Err("Choose an active move belonging to this partner and form.".into());
        }
        let shared_mutation = matches!(
            &record.proposal.steps,
            Steps::Task { .. } | Steps::Pins { .. }
        ) || matches!(&record.proposal.steps,Steps::Request { pins,.. } if !pins.is_empty());
        if self.task_id <= 0
            || self.archived
            || !self.planning_input_ready()
            || self.execution.busy()
            || shared_mutation && !self.authentication_ready()
            || self.pending_request.is_some()
            || self.move_runtime.pending_run.is_some()
            || self.move_runtime.pin_flow.is_some()
            || self.move_runtime.pending_request.is_some()
        {
            return Err("Finish or retry saves and owned work before using a move. Nothing was executed or sent.".into());
        }
        if record.proposal.scope == Scope::Project && record.project != self.cwd {
            return Err("This move belongs to another project.".into());
        }
        moves::validate_proposal(&record.proposal)?;
        match &record.proposal.steps {
            Steps::Run { argv, cwd } => {
                if let Some(reason) = self.work_capacity_reason(self.lead.id) {
                    return Err(reason);
                }
                let command =
                    serde_json::to_string(argv).map_err(|_| "Cannot stage literal move argv")?;
                self.command.clone_from(&command);
                self.drafts
                    .retain(|d| d.field != store::EditorField::Command);
                self.changed_task();
                self.flush_save();
                self.move_runtime.pending_run = Some(PendingRun {
                    record: record.clone(),
                    command,
                    cwd: PathBuf::from(cwd),
                    revision: self.revision,
                });
                self.open_overlay(Overlay::Execution);
                self.notice="Saving move command before its normal one-shot permission review; nothing executed.".into();
            }
            Steps::Pins { pins } => self.begin_move_pins(record, pins, None)?,
            Steps::Request { instruction, pins } => {
                let existing = if self.composer_is_request() {
                    self.input.as_str()
                } else {
                    self.drafts
                        .iter()
                        .find(|d| d.field == store::EditorField::Request)
                        .map_or("", |d| d.text.as_str())
                };
                if !existing.is_empty() && existing != instruction {
                    return Err("Request draft retained. Save/send or clear it before using a different request template.".into());
                }
                if pins.is_empty() {
                    self.stage_move_request(record, instruction)?;
                } else {
                    self.begin_move_pins(record, pins, Some(instruction.clone()))?;
                }
            }
            Steps::Task { goal, criteria } => {
                if self.complete {
                    return Err("Reopen the completed task before applying a task template.".into());
                }
                self.goal.clone_from(goal);
                self.criteria = criteria
                    .iter()
                    .map(|text| store::Criterion {
                        text: text.clone(),
                        met: false,
                    })
                    .collect();
                self.workflows.criterion_origins.clear();
                self.changed_task();
                self.flush_save();
                self.move_runtime.task_save = Some((self.task_id, self.revision, record.clone()));
                self.close_overlay();
                self.notice="Task template applied locally; criteria remain unverified. Waiting for saved acknowledgement.".into();
            }
        }
        Ok(())
    }
    fn begin_move_pins(
        &mut self,
        record: &Record,
        selected: &[Pin],
        request: Option<String>,
    ) -> Result<(), String> {
        if self.repository.is_none() {
            return Err(
                "Repository observation unavailable; /refresh before resolving move pins.".into(),
            );
        }
        self.open_overlay(Overlay::Sources);
        self.move_runtime.pin_flow = Some(PinFlow {
            record: record.clone(),
            task: self.task_id,
            partner: self.lead.id,
            remaining: selected.iter().cloned().collect(),
            current: None,
            request,
        });
        self.notice="Resolve and review each move pin against current files; nothing attached or sent automatically.".into();
        Ok(())
    }
    fn stage_move_request(&mut self, record: &Record, instruction: &str) -> Result<(), String> {
        if instruction.len() > super::MAX_GOAL_BYTES {
            return Err("Move instruction exceeds the Request editor limit.".into());
        }
        self.request_home(false);
        self.input = instruction.to_owned();
        self.cursor = self.input.len();
        self.changed_task();
        self.flush_save();
        self.move_runtime.pending_request = Some(record.clone());
        self.notice="Saving request template before the full receipt preview. Nothing sent; Run still requires review.".into();
        Ok(())
    }
    pub(super) fn remember_move_request(&mut self, claim: &store::WorkClaim) {
        let Some(providers::Request::Preview(input)) = &mut self.planning.pending else {
            return;
        };
        if let Some(record) = self.move_runtime.pending_request.as_ref()
            && let Steps::Request { pins, .. } = &record.proposal.steps
        {
            input
                .excerpts
                .retain(|e| pins.iter().any(|p| p.matches_excerpt(e)));
            let mut seen = HashSet::new();
            input.excerpts.reverse();
            input
                .excerpts
                .retain(|e| seen.insert((e.path.clone(), e.start_line, e.end_line)));
            input.excerpts.reverse();
        }
        let selected = match pins(&input.excerpts) {
            Ok(selected) => selected,
            Err(error) => {
                self.notice = format!("Move learning unavailable for these selectors: {error}");
                return;
            }
        };
        let authored = Authored {
            claim: claim.clone(),
            instruction: input.task.request_text().into(),
            pins: selected,
        };
        self.move_runtime
            .prepared_requests
            .insert(claim.owner.partner, authored);
        if let Some(record) = self.move_runtime.pending_request.take()
            && record.proposal.partner == claim.owner.partner
        {
            self.move_runtime
                .provider_uses
                .insert(claim.request_id.clone(), (record, false));
        }
    }
    pub(super) fn move_request_dispatched(&mut self, partner: i64, request: &str) {
        if self
            .move_runtime
            .prepared_requests
            .get(&partner)
            .is_none_or(|sent| sent.claim.request_id != request)
        {
            return;
        }
        let Some(sent) = self.move_runtime.prepared_requests.remove(&partner) else {
            return;
        };
        self.queue_move_operation(
            sent.claim.task_id,
            partner,
            sent.claim.owner.form,
            Operation::DispatchPins {
                request_id: sent.claim.request_id.clone(),
                pins: sent.pins.clone(),
            },
        );
        if let Some((record, dispatched)) = self
            .move_runtime
            .provider_uses
            .get_mut(&sent.claim.request_id)
        {
            *dispatched = true;
            let record = record.clone();
            let mut value = usage(
                &record,
                sent.claim.task_id,
                Outcome::Pending,
                self.execution.latest.fingerprint.clone(),
            );
            value.id = format!("move-{}", sent.claim.request_id);
            value.request_id = Some(sent.claim.request_id.clone());
            self.queue_move_use(&record, value);
        }
        self.move_runtime
            .last_requests
            .insert((sent.claim.task_id, partner, sent.claim.owner.form), sent);
    }
    pub(super) fn move_dispatch_pending(&self, partner: i64) -> bool {
        self.moves
            .pending
            .iter()
            .chain(self.moves.inflight.iter())
            .chain(self.moves.queued.iter())
            .chain(self.moves.failed.iter())
            .any(|r| r.partner == partner && matches!(r.operation, Operation::DispatchPins { .. }))
    }
    pub(super) fn move_provider_finished(
        &mut self,
        claim: &store::WorkClaim,
        outcome: providers::Outcome,
    ) {
        self.move_runtime
            .prepared_requests
            .remove(&claim.owner.partner);
        let Some((record, dispatched)) = self.move_runtime.provider_uses.remove(&claim.request_id)
        else {
            return;
        };
        let outcome = match outcome {
            providers::Outcome::Completed => Outcome::Completed,
            providers::Outcome::Failed => Outcome::Failed,
            providers::Outcome::Cancelled if !dispatched => Outcome::Denied,
            providers::Outcome::Cancelled => Outcome::Cancelled,
            providers::Outcome::Unknown => Outcome::Unknown,
        };
        let mut value = usage(
            &record,
            claim.task_id,
            outcome,
            self.execution.latest.fingerprint.clone(),
        );
        value.id = format!("move-{}", claim.request_id);
        if dispatched {
            value.request_id = Some(claim.request_id.clone());
        }
        self.queue_move_use(&record, value);
    }
    pub(super) fn move_run_prepared(&mut self, partner: i64) {
        let Some(prepared) = self
            .execution_state(partner)
            .and_then(|s| s.prepared.as_ref())
        else {
            return;
        };
        let Some(record) = self.move_runtime.staged_runs.get(&partner) else {
            return;
        };
        let Steps::Run { argv, cwd } = &record.proposal.steps else {
            return;
        };
        let expected = runner::CommandSpec::Direct {
            program: argv[0].clone(),
            args: argv[1..].to_vec(),
        };
        let expected_cwd = if cwd == "." {
            PathBuf::from(&self.cwd)
        } else {
            Path::new(&self.cwd).join(cwd)
        };
        if self
            .execution_state(partner)
            .and_then(|state| state.move_origin)
            != Some((record.id, record.revision))
            || prepared.spec != expected
            || prepared.cwd != expected_cwd
        {
            return;
        }
        let request = prepared.request_id.clone();
        let record = self
            .move_runtime
            .staged_runs
            .remove(&partner)
            .expect("matched staged move");
        self.move_runtime.run_uses.insert(request, record);
        if partner == self.lead.id && self.overlay == Some(Overlay::Execution) {
            self.execution.command("execute");
        }
    }
    pub(super) fn move_run_denied(&mut self) {
        let Some(prepared) = self.execution.prepared.as_ref() else {
            return;
        };
        let request = prepared.request_id.clone();
        let task = prepared.task_id;
        if let Some(record) = self.move_runtime.run_uses.remove(&request) {
            let mut value = usage(
                &record,
                task,
                Outcome::Denied,
                prepared.before.fingerprint.clone(),
            );
            value.id = format!("move-{request}");
            value.request_id = Some(request);
            self.queue_move_use(&record, value);
        }
    }
    pub(super) fn move_run_committed(&mut self, run: &runner::RunRecord) {
        let terminal = !matches!(
            run.state,
            runner::RunState::AwaitingDispatch
                | runner::RunState::Running
                | runner::RunState::Cancelling
        );
        if terminal
            && run.started_at_ms.is_some()
            && self
                .move_runtime
                .observed_runs
                .insert(run.prepared.run_id.clone())
            && let Some(owner) = &run.prepared.partner
            && moves::journal_steps(Path::new(&self.cwd), run).is_ok()
        {
            self.queue_move_operation(
                run.prepared.task_id,
                owner.partner,
                owner.form,
                Operation::ObserveRun {
                    run_id: run.prepared.run_id.clone(),
                },
            );
        }
        if let Some(record) = self
            .move_runtime
            .run_uses
            .get(&run.prepared.request_id)
            .cloned()
        {
            let outcome = match run.state {
                runner::RunState::Succeeded => Outcome::Succeeded,
                runner::RunState::Failed => Outcome::Failed,
                runner::RunState::Cancelled => Outcome::Cancelled,
                runner::RunState::Interrupted | runner::RunState::OutcomeUnknown => {
                    Outcome::Unknown
                }
                _ => Outcome::Pending,
            };
            let mut value = usage(
                &record,
                run.prepared.task_id,
                outcome,
                run.after
                    .fingerprint
                    .clone()
                    .or_else(|| run.prepared.before.fingerprint.clone()),
            );
            value.id = format!("move-{}", run.prepared.request_id);
            value.run_id = Some(run.prepared.run_id.clone());
            self.queue_move_use(&record, value);
            if terminal {
                self.move_runtime.run_uses.remove(&run.prepared.request_id);
            }
        }
    }
    pub(super) fn move_capture_stored(&mut self, task: i64, success: bool) -> bool {
        let Some(flow) = self.move_runtime.pin_flow.as_mut() else {
            return false;
        };
        if flow.task != task || flow.partner != self.lead.id {
            return false;
        }
        if let Some(pin) = flow.current.as_ref()
            && success
            && self.sources.mode == sources::Mode::Basket
            && self
                .sources
                .basket
                .iter()
                .any(|e| pin.matches_excerpt(e) && e.available)
        {
            flow.current = None;
        }
        self.sources.take_return_to_composer();
        true
    }
    pub(super) fn cancel_move_pins(&mut self) {
        if let Some(flow) = self.move_runtime.pin_flow.take() {
            let value = usage(
                &flow.record,
                flow.task,
                Outcome::Cancelled,
                self.execution.latest.fingerprint.clone(),
            );
            self.queue_move_use(&flow.record, value);
        }
    }
    fn drive_move_pins(&mut self) -> bool {
        let Some(flow) = self.move_runtime.pin_flow.as_ref() else {
            return false;
        };
        if flow.task != self.task_id
            || flow.partner != self.lead.id
            || self.overlay != Some(Overlay::Sources)
        {
            self.cancel_move_pins();
            return true;
        }
        if self.sources.busy() || flow.current.is_some() || !self.sources.error.is_empty() {
            return false;
        }
        let flow = self.move_runtime.pin_flow.as_mut().expect("owned pin flow");
        if let Some(pin) = flow.remaining.pop_front() {
            let Some(report) = self.repository.as_ref() else {
                return false;
            };
            self.sources.preview = None;
            self.sources.preview_reviewed = false;
            if let Err(error) = self.sources.begin_capture(
                self.task_id,
                context::Operation::Inspect {
                    report: report.clone(),
                    path: PathBuf::from(&pin.path),
                    start_line: pin.start_line,
                    end_line: pin.end_line,
                },
            ) {
                flow.remaining.push_front(pin);
                self.sources.error = error;
                return true;
            }
            self.sources.source_busy = true;
            self.sources.mode = sources::Mode::Preview;
            self.sources.status =
                "Resolving move pin against current files; review before attaching.".into();
            flow.current = Some(pin);
            return true;
        }
        let flow = self
            .move_runtime
            .pin_flow
            .take()
            .expect("completed pin flow");
        if let Some(instruction) = flow.request {
            if let Err(error) = self.stage_move_request(&flow.record, &instruction) {
                self.notice = error;
            }
        } else {
            let value = usage(
                &flow.record,
                flow.task,
                Outcome::Completed,
                self.execution.latest.fingerprint.clone(),
            );
            self.queue_move_use(&flow.record, value);
            self.close_overlay();
            self.notice = "Move pins reviewed and saved locally; nothing sent.".into();
        }
        true
    }
    pub(super) fn move_task_saved(&mut self, snapshot: &store::Snapshot) {
        let Some((task, revision, record)) = self.move_runtime.task_save.as_ref() else {
            return;
        };
        if *task != snapshot.id || snapshot.revision < *revision {
            return;
        }
        let record = record.clone();
        self.move_runtime.task_save = None;
        let matches = matches!(&record.proposal.steps,Steps::Task { goal,criteria }
            if snapshot.goal==*goal && snapshot.criteria.iter().map(|c| &c.text).eq(criteria.iter()));
        let value = usage(
            &record,
            snapshot.id,
            if matches {
                Outcome::Completed
            } else {
                Outcome::Cancelled
            },
            None,
        );
        self.queue_move_use(&record, value);
    }
    pub(super) fn cancel_pending_moves(&mut self) {
        self.move_runtime.validation = None;
        if let Some(pending) = self.move_runtime.pending_run.take() {
            let value = usage(&pending.record, self.task_id, Outcome::Cancelled, None);
            self.queue_move_use(&pending.record, value);
        }
        if let Some(record) = self.move_runtime.pending_request.take() {
            let value = usage(&record, self.task_id, Outcome::Cancelled, None);
            self.queue_move_use(&record, value);
        }
        self.cancel_move_pins();
    }
    fn settle_unstarted_move_runs(&mut self) -> bool {
        let cancelled = self
            .move_runtime
            .run_uses
            .iter()
            .filter_map(|(request, record)| {
                let state = self.execution_state(record.proposal.partner)?;
                (!state.busy()
                    && !state.storage_pending()
                    && state
                        .prepared
                        .as_ref()
                        .is_none_or(|p| &p.request_id != request)
                    && state
                        .active
                        .as_ref()
                        .is_none_or(|r| &r.prepared.request_id != request))
                .then(|| request.clone())
            })
            .collect::<Vec<_>>();
        let changed = !cancelled.is_empty();
        for request in cancelled {
            if let Some(record) = self.move_runtime.run_uses.remove(&request) {
                let mut value = usage(&record, self.task_id, Outcome::Cancelled, None);
                value.id = format!("move-{request}");
                value.request_id = Some(request);
                self.queue_move_use(&record, value);
            }
        }
        changed
    }
    pub(super) fn cache_picker_moves(
        &mut self,
        partner: i64,
        form: usize,
        result: &Result<store::moves::Response, String>,
    ) {
        let names = result
            .as_ref()
            .map(|response| {
                response
                    .records
                    .iter()
                    .filter(|record| {
                        !record.forgotten
                            && record.proposal.partner == partner
                            && record.proposal.form == form
                    })
                    .map(|record| record.proposal.name.clone())
                    .collect()
            })
            .map_err(Clone::clone);
        self.move_runtime
            .picker_moves
            .insert((partner, form), names);
    }
    pub fn picker_move_names(&self, form: usize) -> Option<&Result<Vec<String>, String>> {
        self.move_runtime.picker_moves.get(&(self.lead.id, form))
    }
    pub fn drive_moves(&mut self) -> bool {
        let mut changed = false;
        let owner = (self.task_id, self.lead.id, self.form());
        if self.move_runtime.owner != Some(owner) {
            let previous = self.move_runtime.owner.replace(owner);
            if previous.is_some_and(|(task, _, _)| task != self.task_id) {
                self.move_runtime.last_requests.clear();
                self.move_runtime.observed_runs.clear();
                self.move_runtime.picker_moves.clear();
            }
            self.moves.reset_view(owner.0, owner.1, owner.2);
            if self.task_id > 0 {
                self.queue_move_operation(owner.0, owner.1, owner.2, Operation::Load);
            }
            changed = true;
        }
        if self.overlay == Some(Overlay::Forms) && self.task_id > 0 {
            let form = self.selected.min(8);
            if !self
                .move_runtime
                .picker_moves
                .contains_key(&(self.lead.id, form))
                && !self
                    .moves
                    .inflight
                    .iter()
                    .chain(self.moves.queued.iter())
                    .chain(self.moves.failed.iter())
                    .any(|r| {
                        r.task_id == self.task_id
                            && r.partner == self.lead.id
                            && r.form == form
                            && matches!(r.operation, Operation::Load)
                    })
            {
                self.queue_move_operation(self.task_id, self.lead.id, form, Operation::Load);
                changed = true;
            }
        }
        changed |= self.drive_move_pins();
        changed |= self.settle_unstarted_move_runs();
        if self.quit
            || self.quit_after_save
            || self.cooperation.cancel_review.is_some()
            || self
                .confirmation
                .is_some_and(|c| c.kind == super::ConfirmationKind::Quit)
        {
            return changed;
        }
        if let Some(pending) = self.move_runtime.pending_run.as_ref() {
            if pending.record.proposal.partner != self.lead.id
                || pending.record.proposal.form != self.form()
            {
                self.move_runtime.pending_run = None;
                self.notice = "Move preparation cancelled: its owner changed.".into();
                changed = true;
            } else if self.saved_revision >= pending.revision
                && matches!(self.storage, StorageState::Saved)
                && !self.save_requested
            {
                let pending = self
                    .move_runtime
                    .pending_run
                    .take()
                    .expect("saved move command");
                let owner = self.attribution();
                if self.execution.prepare_move_for(
                    self.task_id,
                    owner,
                    &pending.command,
                    pending.cwd,
                    (pending.record.id, pending.record.revision),
                ) {
                    self.move_runtime
                        .staged_runs
                        .insert(self.lead.id, pending.record);
                } else {
                    self.notice.clone_from(&self.execution.notice);
                }
                changed = true;
            }
        }
        if let Some(record) = self.move_runtime.pending_request.as_ref() {
            let exact = matches!(&record.proposal.steps,Steps::Request { instruction,.. } if self.composer_is_request() && self.input==*instruction);
            if record.proposal.partner != self.lead.id
                || record.proposal.form != self.form()
                || !exact
                || self.overlay.is_some()
            {
                self.move_runtime.pending_request = None;
                self.notice="Request template preparation cancelled after its draft or owner changed; nothing sent.".into();
                changed = true;
            } else if self.planning_input_ready() && !self.execution.busy() {
                self.planning_command("new");
                if self.planning.work.is_some() {
                    self.open_overlay(Overlay::Planning);
                } else {
                    self.move_runtime.pending_request = None;
                }
                changed = true;
            }
        }
        let idle = self.task_id > 0
            && self.authentication_ready()
            && !self.moves.persistence_pending()
            && self.overlay.is_none()
            && self.confirmation.is_none()
            && (!self.editing || self.composer_is_request() && self.input.is_empty())
            && !self.quit_after_save
            && !self.quit
            && !self.transitioning
            && self.pending_request.is_none()
            && self.pending_command.is_none()
            && self.move_runtime.pending_run.is_none()
            && self.move_runtime.pending_request.is_none()
            && !self.memory.busy()
            && !self.party_inflight
            && self.party_request.is_none()
            && !self.planning_any_unknown_effects()
            && !self.execution_any_unknown();
        if idle {
            changed |= self.move_idle_offer();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(form: usize) -> App {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.task_id = 1;
        app.goal = "Existing task".into();
        app.lead.form = form;
        app.revision = 8;
        app.saved_revision = 8;
        app.storage = StorageState::Saved;
        app.save_requested = false;
        app.execution.task_id = 1;
        app.planning.preference_event(Ok(None));
        app.moves.reset_view(1, 1, form);
        app.moves.loaded = true;
        app.overlay = Some(Overlay::Moves);
        app.editing = false;
        app
    }
    fn record(form: usize, steps: Steps) -> Record {
        Record {
            id: 7,
            revision: 1,
            project: "/workspace".into(),
            proposal: Proposal {
                partner: 1,
                form,
                name: "Local label".into(),
                scope: Scope::Project,
                steps,
                provenance: Provenance {
                    task_id: 1,
                    sources: vec!["task:1@8".into()],
                    taught_by: None,
                    teacher_name: None,
                },
            },
            forgotten: false,
            created: 1,
        }
    }
    fn has_use(app: &App) -> bool {
        app.moves
            .inflight
            .iter()
            .chain(app.moves.queued.iter())
            .any(|r| matches!(r.operation, Operation::Use { .. }))
    }
    #[test]
    fn cancelled_validation_never_applies_template_after_its_view_closed() {
        let mut app = app(0);
        let record = record(
            0,
            Steps::Task {
                goal: "Replacement".into(),
                criteria: vec!["Observed check".into()],
            },
        );
        app.stage_move_use(&record).unwrap();
        app.moves.pending = None;
        app.moves.inflight = None;
        app.close_overlay();
        app.open_moves();
        app.move_validation_finished(1, 1, 0, true);
        assert_eq!(app.goal, "Existing task");
        assert!(!app.save_requested);
        assert!(!has_use(&app));
    }
    #[test]
    fn template_results_wait_for_the_matching_saved_facts() {
        let mut app = app(0);
        app.criteria = vec![store::Criterion {
            text: "Old criterion".into(),
            met: true,
        }];
        let record = record(
            0,
            Steps::Task {
                goal: "Replacement".into(),
                criteria: vec!["Observed check".into()],
            },
        );
        app.stage_validated_move(&record).unwrap();
        assert_eq!(app.goal, "Replacement");
        assert_eq!(app.criteria[0].text, "Observed check");
        assert!(!app.criteria[0].met);
        assert!(!has_use(&app));
        let snapshot = app.snapshot("/workspace");
        let mut foreign = snapshot.clone();
        foreign.id = 2;
        app.move_task_saved(&foreign);
        assert!(!has_use(&app));
        app.move_task_saved(&snapshot);
        assert!(app.moves.inflight.iter().any(|r| matches!(
            &r.operation,
            Operation::Use {
                usage: Usage {
                    outcome: Outcome::Completed,
                    task_id: 1,
                    ..
                }
            }
        )));
    }
    #[test]
    fn request_move_refuses_to_replace_a_different_unsent_draft() {
        let mut app = app(4);
        app.editing = true;
        app.editor_field = store::EditorField::Request;
        app.input = "Retain this unsent request 東京".into();
        app.cursor = 3;
        let before = (app.input.clone(), app.cursor);
        let record = record(
            4,
            Steps::Request {
                instruction: "Template instruction".into(),
                pins: vec![],
            },
        );
        assert!(app.stage_validated_move(&record).is_err());
        assert_eq!((app.input, app.cursor), before);
        assert!(app.move_runtime.pending_request.is_none());
        assert!(app.planning.pending.is_none());
    }
    #[test]
    fn teaching_uses_only_frozen_submitted_user_text_for_its_own_form() {
        let mut app = app(4);
        app.input = "Unsent follow-up must not be taught".into();
        let claim = store::WorkClaim {
            task_id: 1,
            owner: app.attribution(),
            request_id: "provider-actual".into(),
            writing: false,
        };
        app.move_runtime.last_requests.insert(
            (1, 1, 4),
            Authored {
                claim,
                instruction: "Actual user-authored submitted instruction".into(),
                pins: vec![Pin {
                    path: "src/lib.rs".into(),
                    start_line: 2,
                    end_line: 4,
                }],
            },
        );
        let proposal = app.move_teaching_proposal().unwrap();
        assert!(matches!(proposal.steps,Steps::Request { instruction,pins }
            if instruction=="Actual user-authored submitted instruction"
                && pins==vec![Pin { path:"src/lib.rs".into(),start_line:2,end_line:4 }]));
        app.lead.form = 7;
        assert!(app.move_teaching_proposal().is_err());
    }
}
