//! Local reference relays. No provider, permission, session or execution transfer.
use super::{App, Overlay};
use crate::{
    store,
    workspace::{
        context, repo, runner,
        workflows::{self, EvidenceRef, EvidenceView},
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelaySource {
    Hypothesis(u64),
    Finding(u64),
    Rerun(String),
    Decision(String),
    Review,
    RoleAction { action: &'static str, target: i64 },
}
#[derive(Clone, Debug)]
pub struct RelayRequest {
    pub target_form: usize,
    pub text: String,
    pub paths: Vec<PathBuf>,
    pub source: RelaySource,
    pub command: Option<runner::CommandSpec>,
}
pub struct Pending {
    pub request: RelayRequest,
    task: i64,
    sender: i64,
    source_identity: String,
    pub selected: usize,
    pub confirm: bool,
    pub reviewed: bool,
    pub target: Option<i64>,
}
impl App {
    fn relay_source_identity(&self, source: &RelaySource) -> Result<String, String> {
        let current = if matches!(source, RelaySource::RoleAction { .. }) {
            runner::InputSnapshot::default()
        } else {
            self.workflow_fingerprint()
        };
        let view = EvidenceView {
            task_id: self.task_id,
            excerpts: &self.sources.basket,
            runs: &self.execution.history,
            current: &current,
        };
        let refs: Vec<&EvidenceRef> = match source {
            RelaySource::Hypothesis(id) => {
                if self.workflows.hypothesis_status(*id, &view)
                    != workflows::HypothesisStatus::Supported
                {
                    return Err("Fix disabled: hypothesis must be supported by current source and run evidence (not unresolved, refuted or conflicting).".into());
                }
                self.workflows
                    .hypotheses
                    .iter()
                    .find(|h| h.id == *id)
                    .ok_or("Hypothesis missing")?
                    .evidence
                    .iter()
                    .map(|e| &e.reference)
                    .collect()
            }
            RelaySource::Finding(id) => {
                if !self
                    .workflows
                    .review
                    .as_ref()
                    .is_some_and(|r| r.requested == workflows::Verdict::RequestChanges)
                {
                    return Err("Fix disabled: record a Request changes verdict first.".into());
                }
                let finding = self
                    .workflows
                    .findings
                    .iter()
                    .find(|f| f.id == *id)
                    .ok_or("Finding missing")?;
                if finding.resolution.is_some() {
                    return Err("Finding is resolved; choose an unresolved finding.".into());
                }
                finding.evidence.iter().collect()
            }
            RelaySource::Rerun(id) => {
                let run = view.run(id).ok_or("Retained run unavailable")?;
                super::execution_relays::retained_rerun_spec(run).map_err(str::to_owned)?;
                if run.prepared.cwd != Path::new(&self.cwd) {
                    return Err(
                        "Run cwd differs from this project; no command draft was changed.".into(),
                    );
                }
                if runner::native::directory_identity(Path::new(&self.cwd))
                    .map_err(|e| e.to_string())?
                    != run.prepared.cwd_identity
                {
                    return Err("Run cwd identity changed; drafts unchanged.".into());
                }
                return serde_json::to_string(run).map_err(|e| e.to_string());
            }
            RelaySource::Decision(id) => {
                let decision = self
                    .workflows
                    .chosen_decision
                    .as_ref()
                    .filter(|d| d.id == *id)
                    .ok_or("Choose and save an Espeon /decision first")?;
                return serde_json::to_string(decision).map_err(|e| e.to_string());
            }
            RelaySource::Review => return self.workflows.review_observations(),
            RelaySource::RoleAction { action, target } => {
                let owner = workflows::role_action_form(action).ok_or("Unsupported role action")?;
                if self.form() == owner || (owner == 3 && self.form() == 6) {
                    return Err(
                        "The lead already owns this action; use the local action deliberately."
                            .into(),
                    );
                }
                let partner = self
                    .partners()
                    .into_iter()
                    .find(|partner| partner.id == *target)
                    .ok_or("Role-action recipient is no longer in the party")?;
                if *target == self.lead.id || partner.form != owner || self.partner_busy(*target) {
                    return Err("Role-action recipient must remain idle in the owning form.".into());
                }
                let pins = self
                    .sources
                    .basket
                    .iter()
                    .map(|pin| {
                        (
                            pin.task_id,
                            pin.id,
                            &pin.root,
                            &pin.path,
                            pin.start_line,
                            pin.end_line,
                            &pin.source_sha256,
                            pin.available,
                        )
                    })
                    .collect::<Vec<_>>();
                return serde_json::to_string(&(
                    self.task_id,
                    self.lead.id,
                    self.form(),
                    action,
                    target,
                    &self.goal,
                    &self.criteria,
                    &self.next_action,
                    pins,
                ))
                .map_err(|error| error.to_string());
            }
        };
        if refs.is_empty()
            || refs
                .iter()
                .any(|r| view.status(r) != workflows::EvidenceStatus::Current)
        {
            return Err("Relay evidence is missing, stale or unavailable; refresh and select current evidence.".into());
        }
        serde_json::to_string(&self.workflows).map_err(|e| e.to_string())
    }
    fn relay_paths(&self, request: &RelayRequest) -> Result<Vec<workflows::RangeBaseline>, String> {
        if matches!(request.source, RelaySource::RoleAction { .. }) {
            if !request.paths.is_empty() || request.command.is_some() {
                return Err("Role-action relay transfers references only; no command or paths may be proposed.".into());
            }
            return Ok(Vec::new());
        }
        if request.target_form != 3 && request.target_form != 6 {
            return Ok(Vec::new());
        }
        if !cfg!(any(unix, windows)) {
            return Err("Controlled writes require a supported native filesystem backend.".into());
        }
        if request.paths.is_empty() || request.paths.len() > crate::providers::writes::MAX_FILES {
            return Err(
                "Controlled-write scope requires 1–32 files; limit hit, nothing trimmed.".into(),
            );
        }
        let mut ranges = Vec::new();
        let mut seen = Vec::new();
        for path in &request.paths {
            if seen.contains(path) {
                return Err("Duplicate proposed editable path.".into());
            }
            seen.push(path.clone());
            if context::excluded(path, &[]) {
                return Err("Proposed editable path is excluded.".into());
            }
            let report = self
                .repository
                .as_deref()
                .ok_or("Refresh repository evidence first")?;
            let (content, _) = repo::read_source(
                &report.repository.root,
                path,
                crate::providers::writes::MAX_BYTES,
                &AtomicBool::new(false),
            )?;
            let text = String::from_utf8(content)
                .map_err(|_| "Controlled-write scope must be UTF-8 text")?;
            if text.contains('\0') {
                return Err("Controlled-write scope cannot contain binary files.".into());
            }
            if crate::workspace::privacy::redact(&text) != text {
                return Err("Proposed source contains recognizable credentials; no raw baseline or relay metadata was retained. Source and evidence are unchanged.".into());
            }
            if let RelaySource::Finding(id) = request.source
                && let Some(finding) = self.workflows.findings.iter().find(|f| f.id == id)
            {
                for reference in &finding.evidence {
                    if let EvidenceRef::Source {
                        path: evidence_path,
                        start_line,
                        end_line,
                        ..
                    } = reference
                        && path == Path::new(evidence_path)
                    {
                        ranges.push(workflows::RangeBaseline {
                            path: evidence_path.clone(),
                            start: *start_line,
                            end: *end_line,
                            text: text.clone(),
                        });
                    }
                }
            }
        }
        Ok(ranges)
    }
    pub fn stage_relay(&mut self, request: RelayRequest) -> Result<(), String> {
        if self.task_id <= 0 || self.complete {
            return Err("Save an open task before staging a relay.".into());
        }
        if request.target_form >= crate::domain::FORMS.len() || request.text.len() > 4096 {
            return Err("Relay exceeds the 4096-byte draft bound or has an invalid target.".into());
        }
        if let RelaySource::RoleAction { action, .. } = &request.source
            && workflows::role_action_form(action) != Some(request.target_form)
        {
            return Err("Role-action target form changed; no draft was staged.".into());
        }
        let source_identity = self.relay_source_identity(&request.source)?;
        if let RelaySource::Rerun(id) = &request.source {
            let run = self
                .execution
                .history
                .iter()
                .find(|r| r.prepared.run_id == *id && r.prepared.task_id == self.task_id)
                .ok_or("Run unavailable")?;
            if request.command.as_ref() != Some(&run.prepared.spec) {
                return Err(
                    "Relay command does not match the retained argv/shell identity.".into(),
                );
            }
        }
        self.relay_paths(&request)?;
        self.workflow.relay = Some(Pending {
            request,
            task: self.task_id,
            sender: self.lead.id,
            source_identity,
            selected: 0,
            confirm: false,
            reviewed: false,
            target: None,
        });
        self.open_overlay(Overlay::Workflows);
        self.workflow.scroll = 0;
        self.workflow.notice =
            "Choose a target, then review. Stay is neutral; nothing has been transferred.".into();
        Ok(())
    }
    /// The offer already identifies one owner. This only opens a neutral local
    /// reference review; it never replays the action or opens an effects editor.
    pub fn stage_role_action(&mut self, action: &'static str, target: i64) -> Result<(), String> {
        if self.width < 60 || self.height < 18 {
            return Err("Resize to at least 60 x 18 before reviewing a role-action relay.".into());
        }
        let target_form = workflows::role_action_form(action).ok_or("Unsupported role action")?;
        let boundary = match action {
            "command-edit" => {
                "Choose an explicit command locally; no argv, shell or cwd was prepared."
            }
            "paths" => "Choose and review editable paths locally; none were proposed or granted.",
            "role-implement" => {
                "Choose the implementation role locally if desired; this relay does not change it."
            }
            _ => return Err("Unsupported role action".into()),
        };
        self.stage_relay(RelayRequest {
            target_form,
            text: format!("Task {} reference: requested /{action}.\nReview this task's existing requirements and pinned evidence before choosing that action.\n{boundary}\nReferences only: no provider call, run, permission or session was transferred.", self.task_id),
            paths: Vec::new(),
            source: RelaySource::RoleAction { action, target },
            command: None,
        })?;
        let selected = self
            .relay_targets()
            .iter()
            .position(|(id, _)| *id == target);
        let Some(selected) = selected else {
            self.workflow.relay = None;
            return Err(
                "Chosen role-action recipient is no longer eligible; drafts are unchanged.".into(),
            );
        };
        if let Some(pending) = &mut self.workflow.relay {
            pending.selected = selected;
            pending.target = Some(target);
            pending.confirm = true;
            pending.reviewed = false;
        }
        self.workflow.notice =
            "Review a reference-only send. Stay is neutral; the original action is not replayed."
                .into();
        Ok(())
    }

    pub fn relay_targets(&self) -> Vec<(i64, String)> {
        let Some(pending) = &self.workflow.relay else {
            return Vec::new();
        };
        let mut result = Vec::new();
        for partner in self.partners() {
            if self.partner_busy(partner.id) {
                continue;
            }
            if let RelaySource::RoleAction { target, .. } = &pending.request.source
                && (partner.id != *target || partner.form != pending.request.target_form)
            {
                continue;
            }
            if partner.id == pending.sender || partner.form == pending.request.target_form {
                let same = pending.request.target_form == 7 && self.relay_same_partner(partner.id);
                let verb = if partner.id == pending.sender {
                    "Switch own form to"
                } else {
                    "Send to"
                };
                result.push((
                    partner.id,
                    format!(
                        "{verb} {} ({}){}",
                        Self::partner_label(&partner),
                        crate::domain::FORMS[pending.request.target_form].name,
                        if same { " · same partner" } else { "" }
                    ),
                ));
            }
        }
        if pending.request.target_form == 7 {
            result.sort_by_key(|(id, _)| self.relay_same_partner(*id));
        }
        result
    }
    pub fn relay_key(&mut self, key: KeyEvent) -> bool {
        if self.workflow.relay.is_none() {
            return false;
        }
        if key.kind != KeyEventKind::Press {
            return true;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.request_quit();
            return true;
        }
        if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            let page = self.workflow.scroll_page.max(1);
            self.workflow.scroll = if key.code == KeyCode::PageUp {
                self.workflow.scroll.saturating_sub(page)
            } else {
                self.workflow
                    .scroll
                    .saturating_add(page)
                    .min(self.workflow.scroll_max)
            };
            return true;
        }
        if key.code == KeyCode::Esc || (key.code == KeyCode::Char('n') && key.modifiers.is_empty())
        {
            self.workflow.relay = None;
            self.workflow.notice = "Relay cancelled; sender and recipient drafts unchanged.".into();
            return true;
        }
        if self.width < 60 || self.height < 18 {
            return true;
        }
        let targets = self.relay_targets();
        let count = targets.len();
        let pending = self.workflow.relay.as_mut().unwrap();
        match key.code {
            KeyCode::Up if !pending.confirm => {
                pending.selected = pending.selected.saturating_sub(1)
            }
            KeyCode::Down if !pending.confirm => {
                pending.selected = (pending.selected + 1).min(count.saturating_sub(1))
            }
            KeyCode::Enter if !pending.confirm => {
                pending.target = targets.get(pending.selected).map(|t| t.0);
                pending.confirm = pending.target.is_some();
                pending.reviewed = false;
                self.workflow.scroll = 0;
                if pending.target == Some(pending.sender)
                    && matches!(pending.request.source, RelaySource::Rerun(_))
                {
                    let result = self.commit_relay();
                    if let Err(error) = result {
                        self.workflow.notice = error;
                    }
                }
            }
            KeyCode::Enter => {
                pending.confirm = false;
                pending.reviewed = false;
                pending.target = None;
                self.workflow.scroll = 0;
            }
            KeyCode::Char('y')
                if key.modifiers == KeyModifiers::NONE && pending.confirm && pending.reviewed =>
            {
                let result = self.commit_relay();
                if let Err(error) = result {
                    self.workflow.notice = error;
                }
            }
            _ => {}
        }
        true
    }
    fn commit_relay(&mut self) -> Result<(), String> {
        let pending = self.workflow.relay.as_ref().ok_or("No relay")?;
        let target = pending.target.filter(|id| self.relay_targets().iter().any(|t| t.0 == *id)).ok_or("Chosen target is no longer idle/eligible; cancel and choose again. Drafts unchanged.")?;
        if pending.task != self.task_id
            || pending.sender != self.lead.id
            || self.relay_source_identity(&pending.request.source)? != pending.source_identity
        {
            return Err(
                "Relay source changed; cancel and stage a fresh relay. Drafts unchanged.".into(),
            );
        }
        let ranges = self.relay_paths(&pending.request)?;
        let request = pending.request.clone();
        let field = if request.command.is_some() {
            store::EditorField::Command
        } else {
            store::EditorField::Request
        };
        let mut draft = if target == self.lead.id {
            self.partner_draft()
        } else {
            self.partner_drafts
                .get(&target)
                .cloned()
                .unwrap_or_default()
        };
        if field != store::EditorField::Command
            && (draft
                .drafts
                .iter()
                .any(|d| d.field == field && !d.text.is_empty())
                || (target == self.lead.id
                    && self.editing
                    && self.editor_field == field
                    && !self.input.is_empty()))
        {
            return Err("Target draft is nonempty. Replacement refused; retain or explicitly clear it before relaying.".into());
        }
        if target == self.lead.id
            && let RelaySource::Rerun(id) = &request.source
        {
            let id = id.clone();
            self.execution_rerun(&id)?;
            self.workflow.relay = None;
            return Ok(());
        }
        let text = request.command.as_ref().map_or_else(
            || request.text.clone(),
            super::execution_relays::command_draft,
        );
        if text.len() > 4096 {
            return Err("Relay command exceeds draft limit; nothing trimmed.".into());
        }
        draft.drafts.retain(|d| d.field != field);
        draft.drafts.push(store::Draft {
            field,
            cursor: text.len(),
            text,
        });
        let mut state = self.workflows.clone();
        if let RelaySource::Rerun(id) = &request.source {
            state.command_relays.retain(|r| r.partner != target);
            state.command_relays.push(workflows::CommandRelay {
                partner: target,
                run_id: id.clone(),
            });
        }
        if let RelaySource::Finding(id) = request.source {
            if ranges.is_empty() {
                return Err("Finding needs source ranges before implementation relay.".into());
            }
            state.relayed_findings.retain(|r| r.finding != id);
            state.relayed_findings.push(workflows::RelayedFinding {
                finding: id,
                author: target,
                ranges,
                addressed: false,
                review_generation: state.review_generation,
            });
        }
        if field == store::EditorField::Request {
            state.draft_relays.retain(|r| r.partner != target);
            let decision = if matches!(&request.source, RelaySource::Decision(_)) {
                state.chosen_decision.clone()
            } else {
                None
            };
            state.draft_relays.push(workflows::DraftRelay {
                partner: target,
                form: request.target_form,
                text: request.text,
                paths: request.paths,
                decision,
                role_action: match &request.source {
                    RelaySource::RoleAction { action, .. } => Some((*action).into()),
                    _ => None,
                },
            });
        }
        state.validate(self.task_id)?;
        self.workflow.addressed_fingerprint = None;
        self.workflows = state;
        if target == self.lead.id {
            self.drafts = draft.drafts;
            self.set_form(request.target_form);
            self.apply_relay_draft_role();
        } else {
            self.partner_drafts.insert(target, draft);
        }
        self.changed_task();
        self.workflow.relay = None;
        self.workflow.notice = "References staged in target draft only. No provider call, command, approval or session transferred. Paths still require ordinary review/approval.".into();
        Ok(())
    }
    /// Restore only the deliberately staged local role/scope for this exact
    /// recipient draft. Sessions and permissions are never copied.
    pub fn apply_relay_draft_role(&mut self) {
        let Some(relay) = self
            .workflows
            .draft_relays
            .iter()
            .find(|r| r.partner == self.lead.id && r.form == self.form())
            .cloned()
        else {
            return;
        };
        let draft_matches = self
            .drafts
            .iter()
            .any(|d| d.field == store::EditorField::Request && d.text == relay.text)
            || (self.editor_field == store::EditorField::Request && self.input == relay.text);
        if !draft_matches || self.partner_busy(self.lead.id) {
            return;
        }
        if relay.role_action.is_some() {
            return;
        }
        if relay.form == 7 {
            self.planning
                .set_operation(crate::providers::Operation::Review);
            self.planning.edit_paths.clear();
        } else if matches!(relay.form, 3 | 6) {
            self.planning
                .set_operation(crate::providers::Operation::Implement);
            self.planning.edit_paths = relay.paths;
        } else if relay.form == 0 {
            self.workflow.criteria_text = relay
                .decision
                .as_ref()
                .map_or_else(String::new, |decision| decision.text.clone());
            self.workflow.criteria_decision = relay.decision;
            self.workflow.preview = false;
            self.workflow.criteria_preview = false;
            self.workflow.affirmative = false;
            self.workflow.reviewed = false;
        }
    }

    pub fn relay_same_partner(&self, partner: i64) -> bool {
        if self.repo_loading || self.repo_refresh {
            return false;
        }
        let Some(report) = &self.repository else {
            return false;
        };
        if !report.current.complete || !report.current.stable {
            return false;
        }
        self.workflows.patch_authors.iter().any(|author| author.partner == partner
            && report.current.files.get(Path::new(&author.path)).is_some_and(|file|
                matches!(&file.state, repo::FileState::Regular { sha256, .. } if *sha256 == author.source_sha256)))
    }

    /// Consume a committed task-owned Eevee write acknowledgement. A native
    /// proposal or external filesystem change is never author evidence.
    pub fn workflow_observe_patch(&mut self, task_id: i64, record: &store::ProviderRecord) {
        if task_id != self.task_id {
            return;
        }
        let Some(author) = workflows::controlled_patch_author(record) else {
            return;
        };
        if self
            .workflows
            .patch_authors
            .iter()
            .any(|old| old == &author)
        {
            return;
        }
        let mut state = self.workflows.clone();
        state.patch_authors.retain(|old| old.path != author.path);
        state.patch_authors.push(author);
        if let Err(error) = state.validate(task_id) {
            self.workflow.notice = format!("Controlled patch attribution not retained: {error}");
            return;
        }
        self.workflows = state;
        self.changed_task();
    }
}
