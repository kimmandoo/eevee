//! Read-only children borrow one live lead lease; all transport and persistence use existing workers.
use super::*;
use crate::providers::{
    self,
    delegation::{Call, Context, MAX_CHILDREN, MAX_RESULT_BYTES, MAX_SECONDS},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
    },
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct State {
    parents: BTreeMap<String, Box<providers::Input>>,
    routes: BTreeMap<String, Route>,
    pub(super) children: BTreeMap<i64, Child>,
    batches: Vec<Batch>,
    next: i64,
}
pub(super) struct Child {
    pub state: planning::State,
    worker: Option<providers::Worker>,
    context: Context,
    pub(super) parent_partner: i64,
    pub(super) bench: Option<store::Attribution>,
    started: Instant,
    terminal: Option<(providers::Outcome, String)>,
    label: String,
    cancelled: Arc<AtomicBool>,
    route: Route,
}
struct Batch {
    parent: String,
    reply: SyncSender<Vec<Value>>,
    entries: Vec<Entry>,
}
#[derive(Clone, PartialEq, Eq)]
struct Route {
    scope: Option<providers::ComparisonScope>,
    destination: Option<String>,
    transport: Option<providers::NativeWire>,
}
struct Entry {
    call_id: String,
    child: Option<i64>,
    result: Option<Value>,
}
impl Child {
    pub(super) fn refuse_reservation(&mut self, error: String) {
        self.state.receipts.discard_refused_reservation();
        self.terminal = Some((providers::Outcome::Failed, error));
        self.state.cancel_owned();
    }
}

fn bounded_tool_result(mut result: Value) -> Value {
    // Delegation has no single filesystem target, but still uses the same
    // strict native result envelope as every other locally produced tool result.
    result["path"] = json!("");
    result["message"] = json!(
        "Read-only delegated investigation outcome; summary/error is unverified data, not execution evidence or permission"
    );
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    loop {
        let mut count = Count(0);
        serde_json::to_writer(&mut count, &result).expect("JSON value serialization");
        if count.0 <= MAX_RESULT_BYTES {
            return result;
        }
        let key = if result
            .get("summary")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty())
        {
            "summary"
        } else {
            "error"
        };
        let Some(Value::String(text)) = result.get_mut(key) else {
            return result;
        };
        let mut end = text.len() / 2;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        result["truncated"] = Value::Bool(true);
    }
}

fn refused(call: &Call, message: &str) -> Value {
    let call_id = (call.call_id.len() <= 256).then_some(call.call_id.as_str());
    bounded_tool_result(json!({"call_id":call_id,"status":"refused",
        "error":providers::delegation::bounded_result(message.to_owned())}))
}
fn available(ordinary: usize, children: usize) -> usize {
    MAX_CHILDREN
        .saturating_sub(ordinary.saturating_sub(1))
        .saturating_sub(children)
}

impl App {
    pub(super) fn remember_delegation_route(&mut self, partner: i64, preview: &providers::Preview) {
        let request = self
            .planning_state(partner)
            .filter(|state| state.accepts_preview(preview))
            .and_then(|state| state.work.as_ref())
            .map(|claim| claim.request_id.clone());
        if let Some(request) = request
            && let Some(input) = self.delegation.parents.get_mut(&request)
        {
            if let Ok(body) = serde_json::from_str::<Value>(&preview.payload)
                && let Some(model) = body.get("model").and_then(Value::as_str)
            {
                input.selection.model = Some(model.to_owned());
            }
            self.delegation.routes.insert(
                request,
                Route {
                    scope: preview.comparison_scope,
                    destination: preview.destination.clone(),
                    transport: preview.transport,
                },
            );
        }
    }
    pub(super) fn freeze_delegation_parent(&mut self, claim: &store::WorkClaim) {
        if let Some(providers::Request::Preview(input)) = &mut self.planning.pending {
            input.delegation_enabled = input.agent.is_direct()
                && input.delegation.is_none()
                && !input.local_only
                && claim.owner.partner == self.lead.id;
            if input.delegation_enabled {
                let mut frozen = input.clone();
                frozen.work = Some(claim.clone());
                self.delegation
                    .parents
                    .insert(claim.request_id.clone(), frozen);
            }
        }
    }

    pub(super) fn delegation_active(&self) -> usize {
        self.delegation.children.len()
    }
    pub(super) fn delegation_capacity(&self, partner: i64, generation: u64, task_id: i64) -> usize {
        if partner != self.lead.id
            || task_id != self.task_id
            || self.cancelling_owned_work()
            || self.planning_any_unknown_effects()
        {
            return 0;
        }
        let owned = self.planning_state(partner).is_some_and(|state| {
            state.running()
                && state.matches(generation, task_id)
                && state.work_acquired
                && !state.cancelling
                && state.work.as_ref().is_some_and(|claim| {
                    self.delegation
                        .parents
                        .get(&claim.request_id)
                        .is_some_and(|input| {
                            input.delegation_enabled
                                && input.delegation.is_none()
                                && input.work.as_ref() == Some(claim)
                        })
                        && self.delegation.routes.contains_key(&claim.request_id)
                })
        });
        if owned {
            available(self.ordinary_work_count(), self.delegation_active())
        } else {
            0
        }
    }

    fn ordinary_work_count(&self) -> usize {
        let mut ids = std::collections::BTreeSet::new();
        for state in std::iter::once(&self.planning).chain(self.cooperation.parked.values()) {
            if state.owns_work()
                && let Some(owner) = state.owner().or(state.front.as_ref())
            {
                ids.insert(owner.partner);
            }
        }
        for state in std::iter::once(&self.execution).chain(self.cooperation.executions.values()) {
            if (state.busy() || state.outcome_unknown())
                && let Some(owner) = &state.owner
            {
                ids.insert(owner.partner);
            }
        }
        ids.extend(self.cooperation.command_saves.keys().copied());
        ids.len()
    }

    pub(super) fn provider_delegate(
        &mut self,
        partner: i64,
        (generation, task_id): (u64, i64),
        round: usize,
        calls: Vec<Call>,
        reply: SyncSender<Vec<Value>>,
        cancelled: Arc<AtomicBool>,
    ) -> bool {
        let parent = self
            .planning_state(partner)
            .filter(|state| {
                partner == self.lead.id
                    && state.running()
                    && state.matches(generation, task_id)
                    && state.work_acquired
                    && !state.cancelling
            })
            .and_then(|state| state.work.as_ref())
            .and_then(|claim| {
                self.delegation
                    .parents
                    .get(&claim.request_id)
                    .filter(|input| {
                        input.delegation_enabled
                            && input.delegation.is_none()
                            && input.work.as_ref() == Some(claim)
                            && self.delegation.routes.contains_key(&claim.request_id)
                    })
                    .cloned()
            });
        let invalid = if cancelled.load(Ordering::Acquire) {
            Some("Parent request was cancelled")
        } else if calls.is_empty() || calls.len() > MAX_CHILDREN {
            Some("A delegated round admits at most three children")
        } else if parent.is_none() {
            Some("Only the exact active lead request may delegate; children cannot recurse")
        } else if self.delegation_capacity(partner, generation, task_id) == 0 {
            Some(
                "Read-only child capacity is full or this request lost admission; no work was queued",
            )
        } else if round >= 1025 {
            Some("Delegation round is outside the parent request bounds")
        } else {
            None
        };
        if let Some(error) = invalid {
            let _ = reply.try_send(calls.iter().map(|call| refused(call, error)).collect());
            return true;
        }
        let parent = parent.expect("validated parent");
        let parent_id = parent
            .work
            .as_ref()
            .expect("parent lease")
            .request_id
            .clone();
        let mut slots = available(self.ordinary_work_count(), self.delegation_active());
        let mut batch = Batch {
            parent: parent_id.clone(),
            reply,
            entries: Vec::with_capacity(calls.len()),
        };
        for (index, call) in calls.into_iter().enumerate() {
            let error = call.validate().err().or_else(|| {
                if slots == 0 {
                    Some("Read-only child capacity is full; no work was queued".into())
                } else if batch
                    .entries
                    .iter()
                    .any(|entry| entry.call_id == call.call_id)
                {
                    Some("Duplicate delegation call identity".into())
                } else {
                    None
                }
            });
            if let Some(error) = error {
                batch.entries.push(Entry {
                    call_id: call.call_id.clone(),
                    child: None,
                    result: Some(refused(&call, &error)),
                });
                continue;
            }
            self.delegation.next += 1;
            let key = -self.delegation.next;
            let context = Context {
                parent_request_id: parent_id.clone(),
                child_request_id: format!(
                    "child-{}-{:020}",
                    std::process::id(),
                    providers::approval_arrival()
                ),
                call_id: call.call_id.clone(),
                role: call.role.clone(),
                instruction: call.instruction,
                paths: call.paths,
            };
            let mut input = parent.clone();
            input.delegation_enabled = false;
            input.delegation = Some(context.clone());
            input.behavior_profile = None;
            input.operation = providers::Operation::Plan;
            input.instruction_override = None;
            input.edit_paths.clear();
            input.conversation = None;
            input.prior_plan = None;
            input.public_context = None;
            input.resume = None;
            input.handoff = None;
            input.screenshot = None;
            input.tool_catalog = None;
            input.excerpts.clear();
            input.context_plan = None;
            input.mcp.clear();
            input.credential = None;
            input.report = None;
            let mut state = planning::State::new();
            state.prepare_delegated(input);
            let bench = self
                .party
                .iter()
                .find(|member| member.id != partner && !self.partner_busy(member.id))
                .map(store::Attribution::of);
            let helper_name = bench.as_ref().map_or_else(
                || format!("Helper {}", index + 1),
                |owner| owner.name.clone(),
            );
            self.delegation.children.insert(
                key,
                Child {
                    state,
                    worker: None,
                    context,
                    parent_partner: partner,
                    bench,
                    started: Instant::now(),
                    terminal: None,
                    label: format!(
                        "{} · {} for {} · round {}",
                        helper_name,
                        if call.role == "review" {
                            "reviewing"
                        } else {
                            "exploring"
                        },
                        parent.work.as_ref().expect("lease").owner.name,
                        round
                    ),
                    cancelled: cancelled.clone(),
                    route: self
                        .delegation
                        .routes
                        .get(&parent_id)
                        .cloned()
                        .expect("reviewed parent route"),
                },
            );
            batch.entries.push(Entry {
                call_id: call.call_id,
                child: Some(key),
                result: None,
            });
            slots -= 1;
        }
        self.delegation.batches.push(batch);
        true
    }

    pub(super) fn drive_delegation(
        &mut self,
        store: &store::Worker,
        project: &Path,
        data_dir: &Path,
        exclusions: &[PathBuf],
        storage_available: bool,
    ) -> usize {
        let mut sent = 0;
        let parent_ready: std::collections::BTreeSet<_> = self
            .delegation
            .children
            .values()
            .filter(|child| {
                self.planning_state(child.parent_partner)
                    .is_some_and(|state| !state.receipts.pending())
            })
            .map(|child| child.parent_partner)
            .collect();
        for (&key, child) in &mut self.delegation.children {
            if child.cancelled.load(Ordering::Acquire)
                || child.started.elapsed() >= Duration::from_secs(MAX_SECONDS)
            {
                child.state.cancel_owned();
            }
            if storage_available && sent == 0 && child.state.receipts.try_store(store) {
                sent += 1;
            }
            if storage_available
                && sent == 0
                && !child.state.receipts.pending()
                && child.state.try_usage(store)
            {
                sent += 1;
            }
            if storage_available
                && sent == 0
                && let Some(request) = child.state.pending_journal.front().cloned()
            {
                let mut request = Some(request);
                if store.try_journal(&mut request) {
                    child.state.journal_started();
                    self.cooperation.journal_owners.push_back(key);
                    sent += 1;
                }
            }
            if !parent_ready.contains(&child.parent_partner) {
                continue;
            }
            if child.state.preview.is_some()
                && !child.state.receipts.pending()
                && child.state.receipts.error.is_none()
            {
                child.state.reviewed = true;
                child.state.affirmative = true;
                child.state.dispatch_prepared();
            }
            if child.worker.is_none() && child.state.pending.is_some() {
                match providers::Worker::start(
                    project.to_path_buf(),
                    data_dir.to_path_buf(),
                    exclusions.to_vec(),
                ) {
                    Ok(worker) => child.worker = Some(worker),
                    Err(error) => {
                        child.terminal = Some((
                            providers::Outcome::Failed,
                            format!("Child worker could not start: {error}"),
                        ));
                        child.state.event(providers::Event::Error {
                            generation: None,
                            message: error.to_string(),
                        });
                    }
                }
            }
            if let Some(worker) = &child.worker {
                worker.try_request(&mut child.state.pending);
            }
            child.state.settle_work();
        }
        sent
    }

    fn child_event(&mut self, key: i64, event: providers::Event) {
        let Some(child) = self.delegation.children.get_mut(&key) else {
            return;
        };
        match event {
            providers::Event::ToolArtifact {
                generation,
                task_id,
                capture,
                reply,
            } => {
                if child.state.matches(generation, task_id)
                    && capture.request_id == child.context.child_request_id
                    && capture.partner.as_ref() == child.state.front.as_ref()
                {
                    self.artifacts.retain_with_reply(task_id, capture, reply);
                } else {
                    let _ =
                        reply.try_send(Err("Child tool artifact lost its exact ownership".into()));
                }
            }
            providers::Event::ArtifactLookup {
                generation,
                task_id,
                operation,
                reply,
            } => {
                if child.state.matches(generation, task_id) {
                    self.artifacts.enqueue_lookup(task_id, operation, reply);
                } else {
                    let _ = reply
                        .try_send(Err("Child artifact lookup lost its exact ownership".into()));
                }
            }
            providers::Event::RunCommand { reply, .. } => {
                let _ = reply.try_send(Err("Delegated children cannot execute commands".into()));
            }
            providers::Event::Delegate { calls, reply, .. } => {
                let _ = reply.try_send(
                    calls
                        .iter()
                        .map(|call| refused(call, "Delegated children cannot recurse"))
                        .collect(),
                );
            }
            providers::Event::DelegateCapacity { reply, .. } => {
                let _ = reply.try_send(0);
            }
            providers::Event::Prompt { .. } => {
                child.state.cancel_owned();
            }
            providers::Event::Progress { .. } => {} // Provider todo claims cannot mutate the parent's task.
            providers::Event::Preview(mut preview) => {
                if child.route
                    != (Route {
                        scope: preview.comparison_scope,
                        destination: preview.destination.clone(),
                        transport: preview.transport,
                    })
                {
                    preview.blockers.push("Parent connection, destination or transport changed; child refused without dispatch".into());
                }
                child.state.event(providers::Event::Preview(preview));
                let blockers = child
                    .state
                    .preview
                    .as_ref()
                    .map_or_else(Vec::new, |preview| preview.blockers.clone());
                if blockers.is_empty() {
                    if child.state.preview.is_some() {
                        child.state.receipts.reserve_delegated();
                    }
                } else {
                    child.terminal = Some((providers::Outcome::Failed, blockers.join("; ")));
                    child.state.cancel_owned();
                }
            }
            providers::Event::Finished {
                generation,
                task_id,
                outcome,
                message,
            } => {
                if child.state.matches(generation, task_id) && child.terminal.is_none() {
                    child.terminal = Some((outcome, message.clone()));
                }
                child.state.event(providers::Event::Finished {
                    generation,
                    task_id,
                    outcome,
                    message,
                });
            }
            providers::Event::Error {
                generation,
                message,
            } => {
                child.terminal = Some((providers::Outcome::Failed, message.clone()));
                child.state.event(providers::Event::Error {
                    generation,
                    message,
                });
            }
            other => child.state.event(other),
        }
    }

    pub(super) fn poll_delegation(&mut self) -> (bool, bool) {
        let keys: Vec<_> = self.delegation.children.keys().copied().collect();
        let mut changed = false;
        let mut more = false;
        for key in keys {
            let cancel = self.delegation.children.get(&key).is_some_and(|child| {
                child.cancelled.load(Ordering::Acquire)
                    || child.started.elapsed() >= Duration::from_secs(MAX_SECONDS)
                    || !self
                        .planning_state(child.parent_partner)
                        .is_some_and(|state| {
                            state.work.as_ref().is_some_and(|claim| {
                                claim.request_id == child.context.parent_request_id
                            }) && !state.cancelling
                        })
            });
            if cancel && let Some(child) = self.delegation.children.get_mut(&key) {
                child.state.cancel_owned();
            }
            for index in 0..16 {
                let event = self
                    .delegation
                    .children
                    .get(&key)
                    .and_then(|child| child.worker.as_ref())
                    .and_then(providers::Worker::try_event);
                let Some(event) = event else {
                    break;
                };
                self.child_event(key, event);
                changed = true;
                more |= index == 15;
            }
        }
        changed |= self.observe_usage();
        for batch in &mut self.delegation.batches {
            for entry in &mut batch.entries {
                let Some(key) = entry.child else {
                    continue;
                };
                let Some(child) = self.delegation.children.get_mut(&key) else {
                    continue;
                };
                child.state.settle_work();
                if child.state.owns_work()
                    || child.worker.as_ref().is_some_and(providers::Worker::busy)
                    || self
                        .artifacts
                        .retention_pending(&child.context.child_request_id)
                {
                    continue;
                }
                let (outcome, message) = child.terminal.clone().unwrap_or((
                    providers::Outcome::Failed,
                    "Child ended without a terminal provider result".into(),
                ));
                let mut summary = String::new();
                for item in child.state.items.iter().filter(|item| {
                    item.complete
                        && matches!(
                            item.kind,
                            providers::ItemKind::Message | providers::ItemKind::Plan
                        )
                }) {
                    let remaining = (MAX_RESULT_BYTES - 64).saturating_sub(summary.len());
                    let mut end = remaining.min(item.text.len());
                    while !item.text.is_char_boundary(end) {
                        end -= 1;
                    }
                    summary.push_str(&item.text[..end]);
                    if end < item.text.len() {
                        summary.push_str("\n[Delegated summary truncated]");
                        break;
                    }
                    summary.push('\n');
                }
                let mut result = json!({"call_id":entry.call_id,
                    "status":if outcome == providers::Outcome::Completed { "success" } else { "error" },
                    "outcome":match outcome {
                        providers::Outcome::Completed=>"completed", providers::Outcome::Cancelled=>"cancelled",
                        providers::Outcome::Failed=>"failed", providers::Outcome::Unknown=>"unknown"
                    }, "summary":summary,"child_request_id":child.context.child_request_id,
                    "parent_request_id":batch.parent});
                if outcome != providers::Outcome::Completed {
                    result["error"] = Value::String(providers::delegation::bounded_result(message));
                }
                entry.result = Some(bounded_tool_result(result));
                entry.child = None;
                self.delegation.children.remove(&key);
                changed = true;
            }
        }
        self.delegation.batches.retain_mut(|batch| {
            if batch.entries.iter().any(|entry| entry.result.is_none()) {
                return true;
            }
            let results = batch
                .entries
                .iter_mut()
                .map(|entry| entry.result.take().expect("settled child"))
                .collect();
            let _ = batch.reply.try_send(results);
            false
        });
        let active: std::collections::BTreeSet<_> = std::iter::once(&self.planning)
            .chain(self.cooperation.parked.values())
            .filter_map(|state| state.work.as_ref())
            .map(|claim| claim.request_id.clone())
            .collect();
        self.delegation
            .parents
            .retain(|request, _| active.contains(request));
        self.delegation
            .routes
            .retain(|request, _| active.contains(request));
        (changed, more)
    }

    pub(super) fn delegation_work_rows(&self, rows: &mut Vec<cooperation::WorkRow>) {
        for (&key, child) in &self.delegation.children {
            let Some(mut owner) = child.bench.as_ref().or(child.state.front.as_ref()).cloned()
            else {
                continue;
            };
            owner.partner = key;
            owner.name.clone_from(&child.label);
            let storage = child
                .state
                .receipts
                .error
                .as_ref()
                .or(child.state.usage.error.as_ref())
                .or_else(|| {
                    self.artifacts
                        .retention_pending(&child.context.child_request_id)
                        .then_some(self.artifacts.error.as_ref())
                        .flatten()
                });
            rows.push(cooperation::WorkRow { owner, status: if storage.is_some() { "child storage failed" }
                else if child.state.cancelling { "cancelling" } else { "read-only child" },
                detail: storage.map_or_else(|| child.state.notice.clone(), |error| {
                    format!("{error}. /storage-retry retries storage only; the child is never resent.")
                }), local: false });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn child_slots_share_other_ordinary_partners_without_changing_ordinary_cap() {
        assert_eq!(available(1, 0), 3);
        assert_eq!(available(3, 0), 1);
        assert_eq!(available(2, 1), 1);
        assert_eq!(available(3, 1), 0);
        assert_eq!(available(1, 3), 0);
    }
    #[test]
    fn bounded_results_preserve_utf8() {
        let result = providers::delegation::bounded_result("🦊".repeat(20_000));
        assert!(result.len() <= MAX_RESULT_BYTES);
        assert!(result.ends_with("[Delegated summary truncated at 16 KiB]"));
    }

    fn preview(generation: u64) -> providers::Preview {
        providers::Preview {
            arrival: providers::approval_arrival(),
            generation,
            task_id: 7,
            transport: Some(providers::NativeWire::Http),
            comparison_scope: None,
            thread_id: None,
            destination: Some("http://127.0.0.1/fixture".into()),
            requires_connection_review: false,
            trust: Vec::new(),
            payload: "{\"model\":\"fixture-model\",\"input\":\"read\"}".into(),
            blockers: Vec::new(),
        }
    }

    fn parent() -> App {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.task_id = 7;
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.lead_partner = app.lead.id;
        task.form = app.lead.form;
        task.goal = "Investigate the project without effects".into();
        app.planning.agent = providers::Agent::Compatible;
        app.planning.front = Some(app.attribution());
        app.planning.command("new", true, &task, &[], None);
        let claim = store::WorkClaim {
            task_id: 7,
            owner: app.attribution(),
            request_id: "lead-request".into(),
            writing: false,
        };
        app.freeze_delegation_parent(&claim);
        app.planning.stage_work(claim.clone());
        let acquire = app.planning.work_pending.take().expect("work acquire");
        app.planning.work_event(acquire, Ok(vec![claim]));
        app.planning.pending.take();
        let prepared = preview(11);
        app.remember_delegation_route(app.lead.id, &prepared);
        app.planning.event(providers::Event::Preview(prepared));
        assert!(app.planning.dispatch_prepared());
        app.planning.pending.take();
        app
    }

    fn calls() -> Vec<Call> {
        (0..3)
            .map(|index| Call {
                call_id: format!("call-{index}"),
                role: if index == 1 { "review" } else { "explore" }.into(),
                instruction: format!("Investigate source question {index}"),
                paths: vec![PathBuf::new()],
            })
            .collect()
    }

    #[test]
    fn three_children_admit_together_share_one_parent_lease_and_refuse_more_immediately() {
        let mut app = parent();
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        app.provider_delegate(
            app.lead.id,
            (11, 7),
            1,
            calls(),
            reply,
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(app.delegation.children.len(), 3);
        assert_eq!(app.work_rows().len(), 4);
        assert!(app.work_capacity_reason(app.lead.id + 1).is_some());
        let mut ids = std::collections::BTreeSet::new();
        for child in app.delegation.children.values() {
            let Some(providers::Request::Preview(input)) = &child.state.pending else {
                panic!("child preparation");
            };
            assert_eq!(input.work, app.planning.work);
            assert!(!input.delegation_enabled);
            assert_eq!(input.operation, providers::Operation::Plan);
            assert!(
                input.behavior_profile.is_none()
                    && input.mcp.is_empty()
                    && input.conversation.is_none()
            );
            ids.insert(child.context.child_request_id.clone());
        }
        assert_eq!(ids.len(), 3);
        assert!(receiver.try_recv().is_err());
        let (reply, refused) = std::sync::mpsc::sync_channel(1);
        app.provider_delegate(
            app.lead.id,
            (11, 7),
            2,
            calls(),
            reply,
            Arc::new(AtomicBool::new(false)),
        );
        app.poll_delegation();
        assert!(
            refused
                .try_recv()
                .unwrap()
                .iter()
                .all(|result| result["status"] == "refused")
        );
        assert!(
            receiver.try_recv().is_err(),
            "active children never return unfinished summaries"
        );
        let keys: Vec<_> = app.delegation.children.keys().copied().collect();
        for key in keys {
            app.child_event(
                key,
                providers::Event::Error {
                    generation: None,
                    message: "Actual fixture child failure".into(),
                },
            );
            app.poll_delegation();
        }
        let results = receiver.try_recv().unwrap();
        assert_eq!(
            results
                .iter()
                .map(|result| result["call_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["call-0", "call-1", "call-2"]
        );
        assert!(results.iter().all(|result| result["status"] == "error"));
        assert!(
            app.planning.work.is_some(),
            "children never release their borrowed parent claim"
        );
    }

    #[test]
    fn parent_cancel_stops_every_child_before_any_run_request() {
        let mut app = parent();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (reply, results) = std::sync::mpsc::sync_channel(1);
        app.provider_delegate(app.lead.id, (11, 7), 1, calls(), reply, cancelled.clone());
        cancelled.store(true, Ordering::Release);
        app.poll_delegation();
        let keys: Vec<_> = app.delegation.children.keys().copied().collect();
        for key in keys {
            app.delegation
                .children
                .get_mut(&key)
                .unwrap()
                .state
                .pending
                .take();
            app.child_event(key, providers::Event::Preview(preview(21)));
            let child = app.delegation.children.get_mut(&key).unwrap();
            assert!(matches!(
                child.state.pending.take(),
                Some(providers::Request::Cancel { generation: 21 })
            ));
            assert!(
                !child.state.receipts.pending(),
                "cancelled preparation is not dispatched or reserved"
            );
            app.child_event(
                key,
                providers::Event::Finished {
                    generation: 21,
                    task_id: 7,
                    outcome: providers::Outcome::Cancelled,
                    message: "Cancelled without dispatch".into(),
                },
            );
        }
        app.poll_delegation();
        assert!(
            results
                .try_recv()
                .unwrap()
                .iter()
                .all(|result| result["outcome"] == "cancelled")
        );
        assert_eq!(app.delegation_active(), 0);
        assert!(
            app.planning.usage.record.is_none(),
            "preparation/cancellation never invents provider usage"
        );
    }

    #[test]
    fn native_child_unknown_transport_settles_without_inventing_local_effects() {
        let mut app = parent();
        let (reply, results) = std::sync::mpsc::sync_channel(1);
        app.provider_delegate(
            app.lead.id,
            (11, 7),
            1,
            calls(),
            reply,
            Arc::new(AtomicBool::new(false)),
        );
        let keys: Vec<_> = app.delegation.children.keys().copied().collect();
        for key in keys {
            let state = &mut app.delegation.children.get_mut(&key).unwrap().state;
            state.pending.take();
            state.event(providers::Event::Preview(preview(21)));
            app.child_event(
                key,
                providers::Event::Finished {
                    generation: 21,
                    task_id: 7,
                    outcome: providers::Outcome::Unknown,
                    message: "Local reader settled; remote completion unknown".into(),
                },
            );
            assert!(!app.delegation.children[&key].state.unknown_effects());
        }
        app.poll_delegation();
        let outcomes = results.try_recv().unwrap();
        assert_eq!(outcomes.len(), 3);
        assert!(
            outcomes
                .iter()
                .all(|result| result["outcome"] == "unknown" && result["status"] == "error")
        );
        assert_eq!(app.delegation_active(), 0);
        assert!(
            app.planning.work.is_some(),
            "children never release the parent's durable claim"
        );
    }

    #[test]
    fn full_result_bound_accounts_for_json_escaping_and_error_metadata() {
        let result = bounded_tool_result(json!({ "call_id":"a".repeat(256), "status":"error",
            "summary":"\"\\\n🦊".repeat(20_000), "error":"\"\\\n".repeat(10_000) }));
        assert!(serde_json::to_vec(&result).unwrap().len() <= MAX_RESULT_BYTES);
        assert_eq!(result["truncated"], true);
    }

    #[test]
    fn every_delegated_outcome_uses_the_strict_native_result_envelope() {
        for outcome in ["completed", "failed", "cancelled", "unknown"] {
            let status = if outcome == "completed" {
                "success"
            } else {
                "error"
            };
            let result = bounded_tool_result(json!({
                "status":status,"outcome":outcome,"call_id":"call-0",
                "child_request_id":"child-0","parent_request_id":"lead",
                "summary":"Literal child observation","error":"Actual failure detail"
            }));
            assert_eq!(result["status"], status);
            assert_eq!(result["outcome"], outcome);
            assert_eq!(result["path"], "");
            assert!(
                result["message"]
                    .as_str()
                    .is_some_and(|message| !message.is_empty())
            );
            assert_eq!(result["call_id"], "call-0");
            assert_eq!(result["child_request_id"], "child-0");
            assert_eq!(result["parent_request_id"], "lead");
            assert_eq!(result["summary"], "Literal child observation");
            assert_eq!(result["error"], "Actual failure detail");
        }
        let refusal = refused(&calls()[0], "Child capacity is full");
        assert_eq!(refusal["status"], "refused");
        assert_eq!(refusal["path"], "");
        assert!(refusal["message"].is_string());
        assert_eq!(refusal["call_id"], "call-0");
        assert_eq!(refusal["error"], "Child capacity is full");
    }

    #[test]
    fn two_other_ordinary_owners_leave_one_child_and_capacity_query_never_waits() {
        let mut app = parent();
        for partner in [2, 3] {
            let owner = store::Attribution {
                partner,
                name: format!("User owner {partner}"),
                form: 0,
            };
            app.cooperation
                .command_saves
                .insert(partner, ("saved-command".into(), owner, 1));
        }
        let (reply, capacity) = std::sync::mpsc::sync_channel(1);
        app.provider_event(
            app.lead.id,
            providers::Event::DelegateCapacity {
                generation: 11,
                task_id: 7,
                reply,
            },
        );
        assert_eq!(capacity.try_recv().unwrap(), 1);
        let (reply, _results) = std::sync::mpsc::sync_channel(1);
        app.provider_delegate(
            app.lead.id,
            (11, 7),
            1,
            calls(),
            reply,
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(app.delegation_active(), 1);
        assert_eq!(
            app.delegation.batches[0]
                .entries
                .iter()
                .filter(|entry| entry.result.is_some())
                .count(),
            2
        );
        assert_eq!(app.delegation_capacity(app.lead.id, 11, 7), 0);
        assert_eq!(app.delegation_capacity(app.lead.id + 1, 11, 7), 0);
        assert_eq!(app.delegation_capacity(app.lead.id, 12, 7), 0);
    }

    #[test]
    fn child_reports_aggregate_once_without_changing_raw_parent_usage() {
        use crate::workspace::usage::{Counters, Identity, Kind, Record, Report, Scope, Semantics};
        let mut app = parent();
        let (reply, _results) = std::sync::mpsc::sync_channel(1);
        app.provider_delegate(
            app.lead.id,
            (11, 7),
            1,
            calls(),
            reply,
            Arc::new(AtomicBool::new(false)),
        );
        let owner = app.attribution();
        let reported = |id: &str, amount: u64| {
            let mut record = Record::new(id.into(), 7, owner.clone(), providers::Agent::Compatible);
            let report = Report {
                identity: Identity {
                    response: Some(id.into()),
                    ..Default::default()
                },
                scope: Scope::Response,
                kind: Kind::Snapshot,
                counters: Counters {
                    input: Some(amount),
                    output: Some(1),
                    ..Default::default()
                },
                baseline: None,
                model: Some("fixture-model".into()),
                tokenizer: None,
                semantics: Semantics::default(),
                complete: true,
            };
            assert!(record.apply_retained(report.clone()).unwrap());
            assert!(
                !record.apply_retained(report).unwrap(),
                "repeated provider snapshots deduplicate"
            );
            record
        };
        let parent = reported("lead-request", 5);
        app.planning.usage.record = Some(parent.clone());
        for (index, child) in app.delegation.children.values_mut().enumerate() {
            child.state.usage.record = Some(reported(
                &child.context.child_request_id,
                (index as u64 + 1) * 10,
            ));
        }
        assert!(app.observe_usage());
        assert!(!app.observe_usage());
        app.records.snapshot.usages = app
            .delegation
            .children
            .values()
            .filter_map(|child| child.state.usage.record.clone())
            .collect();
        let totals = app.meter_summary();
        assert_eq!(totals.launch.input, 65);
        assert_eq!(totals.task.input, 65, "saved copies do not count again");
        assert_eq!(totals.request.as_ref().unwrap().input, 5);
        assert_eq!(app.planning.usage.record, Some(parent));
    }
}
