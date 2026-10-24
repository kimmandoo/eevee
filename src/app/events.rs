//! Background worker events: agent preflight, repository and store results.
use super::*;

impl App {
    fn apply_released_project(&mut self, project: store::Project) {
        let committed = project
            .tasks
            .iter()
            .find(|task| task.id == self.task_id)
            .cloned();
        self.party = project.partners;
        self.tasks = project.tasks;
        self.usage_bytes = project.usage_bytes;
        let ids: Vec<_> = self.party.iter().map(|partner| partner.id).collect();
        let front_removed = !ids.contains(&self.lead.id);

        // These are personal recipient intents, not evidence or task requirements.
        // Reconcile them in live state as well as the store's returned task rows.
        self.workflows
            .command_relays
            .retain(|relay| ids.contains(&relay.partner));
        self.workflows
            .draft_relays
            .retain(|relay| ids.contains(&relay.partner));
        if front_removed {
            self.workflow = workflows::State::default();
            self.memory.clear_draft();
        } else if self.workflow.relay.as_ref().is_some_and(|relay| {
            relay.target.is_some_and(|target| !ids.contains(&target))
                || matches!(&relay.request.source, relays::RelaySource::RoleAction { target, .. } if !ids.contains(target))
        }) {
            self.workflow.relay = None;
        }

        // A newly fronted partner creates a local navigation/draft mutation.
        // Start above the committed release revision, never reuse its revision.
        if let Some(task) = &committed {
            self.saved_revision = self.saved_revision.max(task.revision);
            self.revision = self.revision.max(self.saved_revision);
            self.updated_at = task.updated_at;
        }
        if front_removed {
            let next = committed
                .as_ref()
                .and_then(|task| {
                    self.party
                        .iter()
                        .find(|partner| partner.id == task.lead_partner)
                })
                .or_else(|| self.party.first())
                .cloned();
            if let Some(next) = next {
                self.request_lead(next.id);
            }
        } else if let Some(partner) = self.party.iter().find(|partner| partner.id == self.lead.id) {
            self.lead.nickname.clone_from(&partner.nickname);
            self.sync_front();
        }
        self.partner_drafts
            .retain(|partner, _| ids.contains(partner));
        self.cooperation
            .execution_actions
            .retain(|partner, _| ids.contains(partner));
        self.cooperation
            .command_saves
            .retain(|partner, _| ids.contains(partner));
        if self
            .pending_request_owner
            .is_some_and(|partner| !ids.contains(&partner))
        {
            self.pending_request = None;
            self.pending_request_owner = None;
        }
        if self
            .pending_command
            .as_ref()
            .is_some_and(|(_, owner)| !ids.contains(&owner.partner))
        {
            self.pending_command = None;
        }
        self.retain_slots();
        if let Some(task) = committed {
            if self.snapshot(&task.project) != task {
                self.revision = self.revision.max(self.saved_revision.saturating_add(1));
                self.save_requested = true;
            } else {
                self.save_requested = false;
                self.autosave = None;
            }
        }
        self.refresh_save_state();
        self.notice = "Partner released; tasks and attributed evidence are retained.".into();
    }

    pub fn agent_event(&mut self, event: crate::providers::agent::Event) -> bool {
        self.agents.event(event);
        // Discovery/probe results belong to the connections drawer. In
        // particular, delayed startup discovery must not repaint an idle home.
        self.overlay == Some(Overlay::Agents)
    }

    pub fn repo_event(&mut self, event: repo::Event) {
        use std::fmt::Write;
        self.repo_loading = false;
        match event {
            repo::Event::Updated(report) => {
                self.repo_status = if report.repository.git_dir.is_none() {
                    "Project: filesystem catalogue / Git evidence unavailable"
                } else if report.initial {
                    "Repository: baseline captured"
                } else {
                    "Repository: refreshed / original baseline retained"
                }
                .into();
                self.repo_error.clear();
                let mut text = format!(
                    "PROJECT ROOT\n{:?}\nEnclosing Git root: {}\nGit metadata: {}\n\nBASELINE / CURRENT\nHEAD: {} / {}\nFiles: {} / {}\nCurrent coverage complete: {} / stable observation: {}\nOnly project-relative paths are in scope. Application storage is excluded. Ignored untracked files and submodule contents are excluded in Git projects.\n\nCHANGES\n",
                    report.repository.root,
                    report
                        .repository
                        .git_root
                        .as_ref()
                        .map(|path| format!("{path:?}"))
                        .unwrap_or_else(|| "unavailable".into()),
                    report
                        .repository
                        .git_dir
                        .as_ref()
                        .map(|path| format!("{path:?}"))
                        .unwrap_or_else(|| "unavailable".into()),
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
                self.repo_error =
                    format!("{path:?}: {reason}. Task data is retained; /refresh retries.");
                self.repo_error.retain(safe_character);
            }
            repo::Event::Failed { operation, message } => {
                self.repo_status = "Repository: inspection failed".into();
                self.repo_error = format!(
                    "{operation}: {message}. Previous evidence is retained; /refresh retries the read-only inspection."
                );
                self.repo_error.retain(safe_character);
            }
        }
        if self.overlay == Some(Overlay::Workflows) {
            self.workflow_refresh();
        }
    }

    pub fn store_event(&mut self, event: store::Event) -> bool {
        let mut changed = true;
        match &event {
            store::Event::Journal {
                task_id,
                result: Ok(store::JournalResponse::Purged),
            } => {
                self.receipt_sources_purged(*task_id);
            }
            store::Event::Run {
                task_id,
                result: Ok(response),
                ..
            } if matches!(&**response, store::RunResponse::Purged { .. }) => {
                self.receipt_sources_purged(*task_id);
            }
            store::Event::Context {
                task_id,
                result: Ok(excerpts),
            } if excerpts.iter().any(|excerpt| !excerpt.available) => {
                self.receipt_sources_purged(*task_id);
            }
            store::Event::Artifact {
                request,
                result: Ok(crate::workspace::artifacts::Response::Purged { .. }),
            } => {
                self.receipt_sources_purged(request.task_id);
            }
            _ => {}
        }
        match event {
            store::Event::Receipts { request, result } => self.receipt_store_event(request, result),
            store::Event::Artifact { request, result } => {
                self.artifacts.store_event(request, result)
            }
            store::Event::ToolCatalog { request, result } => {
                self.tool_catalog.response(&request, result)
            }
            store::Event::ContextCache { request, result } => {
                self.sources
                    .context_cache_event(self.task_id, &request, result)
            }
            store::Event::Formation(result) => self.formation_event(result),
            store::Event::Records { request, result } => {
                if matches!(&request.operation, store::records::Operation::Usage { .. }) {
                    self.cooperation_usage_event(*request, result);
                } else {
                    changed = self.records_event(*request, result);
                }
            }
            store::Event::Moves {
                task_id,
                partner,
                form,
                result,
            } => {
                let validation = self.moves.inflight.as_ref().is_some_and(|request| {
                    (request.task_id, request.partner, request.form) == (task_id, partner, form)
                        && matches!(request.operation, store::moves::Operation::Validate { .. })
                });
                let accepted = result.is_ok();
                if task_id == self.task_id {
                    self.cache_picker_moves(partner, form, &result);
                }
                self.move_store_event(task_id, partner, form, result);
                if validation {
                    self.move_validation_finished(task_id, partner, form, accepted);
                }
            }
            store::Event::Work { request, result } => self.cooperation_work_event(request, result),
            store::Event::PartyReleased(result) => {
                self.party_inflight = false;
                self.release_partner_finished(&result);
                match result {
                    Ok(project) => self.apply_released_project(project),
                    Err(error) => self.notice = error,
                }
            }
            store::Event::Preferences(result) => {
                self.cooperation_preference_event(result);
                self.agents.active = self.planning.agent.index();
                self.agents.selected = self.agents.active;
                if self.planning.preference_error().is_some() {
                    self.notice.clone_from(&self.planning.notice);
                }
            }
            store::Event::Memory { task_id, result } => {
                if task_id == self.task_id {
                    let before = self.memory.draft_snapshot();
                    let saving = self.memory.saving;
                    let loaded = result.is_ok();
                    self.memory_loading = false;
                    self.memory.finish(result);
                    self.records_memory_committed();
                    if loaded {
                        self.records_invalidate();
                    }
                    if saving || before != self.memory.draft_snapshot() {
                        self.changed_task();
                    }
                    if let Some((task, partner, action)) = self.pending_memory_action.take()
                        && loaded
                        && task == self.task_id
                        && partner == self.lead.id
                        && self.overlay == Some(Overlay::Memory)
                        && self.confirmation.is_none()
                        && !self.has_focused_approval()
                        && self.command_disabled_reason(action).is_none()
                    {
                        self.memory_command(action);
                    }
                }
            }
            store::Event::Journal { task_id, result } => {
                match &result {
                    Ok(store::JournalResponse::Saved(record)) => {
                        self.workflow_observe_patch(task_id, record)
                    }
                    Ok(store::JournalResponse::Records(records)) => {
                        for record in records {
                            self.workflow_observe_patch(task_id, record);
                        }
                    }
                    _ => {}
                }
                self.cooperation_journal_event(task_id, result);
            }
            store::Event::Party(result) => {
                self.party_inflight = false;
                let renamed = self.nickname_pending.take();
                match result {
                    Ok(partners) => {
                        if renamed.is_some() {
                            if let Some(committed) = partners.iter().find(|p| p.id == self.lead.id)
                            {
                                self.lead.nickname.clone_from(&committed.nickname);
                            }
                            self.sync_front();
                            if self
                                .nickname_editor
                                .as_ref()
                                .is_some_and(|editor| Some(editor.partner) == renamed)
                            {
                                self.nickname_editor = None;
                            }
                            self.notice = "Partner nickname saved locally; nothing sent.".into();
                        } else if let Some(partner) = partners.last() {
                            self.notice = format!(
                                "{} joined the party in Eevee form ({} of {}).",
                                Self::partner_label(partner),
                                partners.len(),
                                store::MAX_PARTNERS
                            );
                        }
                        self.party = partners;
                        self.retain_slots();
                    }
                    Err(message) => {
                        if let Some(editor) = &mut self.nickname_editor
                            && Some(editor.partner) == renamed
                        {
                            editor.error.clone_from(&message);
                        }
                        self.notice = message;
                    }
                }
            }
            store::Event::Loaded(project) => {
                let local_changes = self.revision != self.saved_revision;
                self.usage_bytes = project.usage_bytes;
                self.tasks = project.tasks;
                self.party = project.partners.clone();
                self.retain_slots();
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
                    // The task's lead partner fronts this session; its stored
                    // form is what the task resumes into. With local changes,
                    // keep the live lead (unsaved lead/form switches) and only
                    // refresh its identity fields from the committed roster.
                    if !local_changes {
                        self.load_task(snapshot);
                    } else if self.task_id == snapshot.id {
                        if let Some(partner) = project
                            .partners
                            .iter()
                            .find(|partner| partner.id == self.lead.id)
                        {
                            let form = self.lead.form;
                            let auto_form = self.lead.auto_form;
                            self.lead = partner.clone();
                            self.lead.form = form;
                            self.lead.auto_form = auto_form;
                            self.sync_front();
                        }
                        self.saved_revision = snapshot.revision;
                        self.revision = self.revision.max(snapshot.revision.saturating_add(1));
                    }
                } else if let Some(partner) = project
                    .partners
                    .iter()
                    .find(|partner| partner.id == self.lead.id)
                    .or_else(|| project.partners.first())
                {
                    let live_form = self.lead.form;
                    let live_auto_form = self.lead.auto_form;
                    self.lead = partner.clone();
                    if local_changes {
                        self.lead.form = live_form;
                        self.lead.auto_form = live_auto_form;
                    } else {
                        self.form_selection = self.lead.form;
                    }
                    self.sync_front();
                }
                self.save_requested = self.revision != self.saved_revision;
                self.save_inflight = None;
                self.storage = StorageState::Saved;
                if !self.save_requested {
                    self.autosave = None;
                }
                self.refresh_save_state();
            }
            store::Event::Selected(snapshot) => {
                self.load_task(snapshot);
                self.transitioning = false;
                self.notice.clear();
            }
            store::Event::Saved(mut snapshot) => {
                self.records_task_saved(&mut snapshot);
                self.move_task_saved(&snapshot);
                if self.task_id == 0 {
                    // The first save assigns an identity to this same local task.
                    if let Some(owner) = &mut self.temporary_editor_return
                        && owner.task == 0
                    {
                        owner.task = snapshot.id;
                    }
                    if let Some((task, _)) = &mut self.composer_return_owner
                        && *task == 0
                    {
                        *task = snapshot.id;
                    }
                    if let Some((task, _, _)) = &mut self.direct_sheet_return
                        && *task == 0
                    {
                        *task = snapshot.id;
                    }
                    if let Some((task, _, _)) = &mut self.pending_source_action
                        && *task == 0
                    {
                        *task = snapshot.id;
                    }
                    if let Some((task, _, _, _, _)) = &mut self.pending_provider_picker
                        && *task == 0
                    {
                        *task = snapshot.id;
                    }
                    self.task_id = snapshot.id;
                    self.planning.adopt_initial_task(snapshot.id);
                    for state in self.cooperation.parked.values_mut() {
                        state.adopt_initial_task(snapshot.id);
                    }
                    self.memory.reset(snapshot.id, snapshot.project.clone());
                    self.memory_loading = true;
                    self.memory_request = Some(store::MemoryRequest {
                        task_id: snapshot.id,
                        mutation: None,
                    });
                }
                if self.task_id == snapshot.id {
                    if self
                        .save_inflight
                        .is_some_and(|revision| revision <= snapshot.revision)
                    {
                        self.save_inflight = None;
                    }
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
                        self.partner_drafts.clone_from(&snapshot.partner_drafts);
                        if let Some(draft) = snapshot.partner_drafts.get(&self.lead.id) {
                            self.drafts.clone_from(&draft.drafts);
                        }
                        self.storage = StorageState::Saved;
                        self.save_requested = !self.records.local.is_empty();
                        self.autosave = None;
                    }
                    self.refresh_save_state();
                }
                self.commit_pending_commands(&snapshot);
                if let Some(previous) = self.tasks.iter_mut().find(|task| task.id == snapshot.id) {
                    if previous.revision <= snapshot.revision {
                        *previous = snapshot;
                    }
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
                self.party = project.partners;
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
                let captured = result.is_ok();
                self.sources.stored(self.task_id, task_id, result);
                if captured {
                    self.records_invalidate();
                }
                let move_capture = self.move_capture_stored(task_id, captured);
                if !move_capture
                    && self.sources.take_return_to_composer()
                    && self.overlay == Some(Overlay::Sources)
                    && self.composer_return.is_some()
                {
                    self.close_overlay();
                    self.notice =
                        "Attachment saved to this task locally. Nothing sent to a provider.".into();
                }
                if self.overlay == Some(Overlay::Workflows) {
                    self.workflow_refresh();
                }
            }
            store::Event::Run {
                task_id,
                request_id,
                run_id,
                result,
            } => {
                self.cooperation_run_event(
                    task_id,
                    request_id.as_deref(),
                    run_id.as_deref(),
                    result.map(|response| *response),
                );
                if self.overlay == Some(Overlay::Workflows) {
                    self.workflow_refresh();
                }
            }
            store::Event::Failed { revision, message } => {
                self.transitioning = false;
                if self
                    .save_inflight
                    .is_some_and(|inflight| revision.is_none_or(|revision| revision == inflight))
                {
                    self.save_inflight = None;
                }
                if revision.is_none_or(|revision| revision == self.revision) {
                    self.storage = StorageState::Failed(message);
                    self.save_requested |= self.revision != self.saved_revision;
                    self.quit_after_save = false;
                }
                self.refresh_save_state();
            }
        }
        self.refresh_approvals();
        self.settle_cooperation();
        self.settle_exit();
        self.settle_reset();
        changed
    }
}
