//! Eevee request/event contract; provider sessions and permissions are never portable.
pub mod acp;
pub mod agent;
pub mod auth;
pub mod claude;
pub mod codex;
pub mod direct;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc};

/// Process-wide producer order, independent of worker drain or partner-map order.
pub fn approval_arrival() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Agent {
    #[default]
    Codex,
    Claude,
    OpenCode,
    Gemini,
    Devin,
    ChatGPT,
    OpenAI,
    Anthropic,
    Google,
    Compatible,
}
impl Agent {
    pub const ALL: [Self; 10] = [
        Self::Codex,
        Self::Claude,
        Self::OpenCode,
        Self::Gemini,
        Self::Devin,
        Self::ChatGPT,
        Self::OpenAI,
        Self::Anthropic,
        Self::Google,
        Self::Compatible,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex · CLI agent",
            Self::Claude => "Claude Agent",
            Self::OpenCode => "OpenCode ACP",
            Self::Gemini => "Gemini ACP",
            Self::Devin => "Devin ACP",
            Self::ChatGPT => "Codex · native (no tools)",
            Self::OpenAI => "OpenAI API",
            Self::Anthropic => "Anthropic API",
            Self::Google => "Google Gemini API",
            Self::Compatible => "Custom / local API",
        }
    }
    /// Connection profiles are discovered in `ALL` order.
    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|agent| *agent == self)
            .unwrap_or(0)
    }
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
    pub fn command(self) -> &'static str {
        match self {
            Self::Codex => "provider-codex",
            Self::Claude => "provider-claude",
            Self::OpenCode => "provider-opencode",
            Self::Gemini => "provider-gemini",
            Self::Devin => "provider-devin",
            Self::ChatGPT => "provider-chatgpt",
            Self::OpenAI => "provider-openai",
            Self::Anthropic => "provider-anthropic",
            Self::Google => "provider-google",
            Self::Compatible => "provider-compatible",
        }
    }
    pub fn from_command(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|agent| agent.command() == name)
    }
    pub fn is_direct(self) -> bool {
        matches!(
            self,
            Self::ChatGPT | Self::OpenAI | Self::Anthropic | Self::Google | Self::Compatible
        )
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Operation {
    #[default]
    Plan,
    Implement,
    Review,
}

/// Explicit local selection; None means the provider's advertised default.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSelection {
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl ModelSelection {
    /// Compact "model · effort" line shared by the connection hub and pickers.
    pub fn summary(&self) -> String {
        format!(
            "model {} · effort {}",
            self.model.as_deref().unwrap_or("default"),
            self.effort.as_deref().unwrap_or("default")
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelOption {
    pub id: String,
    pub name: String,
    pub description: String,
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ModelCatalogue {
    pub models: Vec<ModelOption>,
    pub current: Option<String>,
    pub source: String,
    /// None when the official interface supplies no selectable reasoning control.
    pub effort_note: Option<String>,
}

impl ModelCatalogue {
    pub fn validate(&self) -> Result<(), String> {
        fn text(value: &str, limit: usize, empty: bool) -> bool {
            (empty || !value.is_empty())
                && value.len() <= limit
                && crate::workspace::privacy::redact(value) == value
                && !value.chars().any(|c| c.is_control()
                    || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        }
        if self.models.len() > 4096
            || !text(&self.source, 2048, false)
            || self.current.as_ref().is_some_and(|v| !text(v, 256, false))
            || self
                .effort_note
                .as_ref()
                .is_some_and(|v| !text(v, 2048, false))
        {
            return Err("Invalid or oversized provider model catalogue".into());
        }
        for (index, model) in self.models.iter().enumerate() {
            if !text(&model.id, 256, false)
                || !text(&model.name, 512, false)
                || !text(&model.description, 4096, true)
                || model.efforts.len() > 32
                || self.models[..index].iter().any(|v| v.id == model.id)
                || model.efforts.iter().enumerate().any(|(i, effort)| {
                    !text(effort, 256, false) || model.efforts[..i].contains(effort)
                })
                || model
                    .default_effort
                    .as_ref()
                    .is_some_and(|v| !model.efforts.contains(v))
            {
                return Err("Invalid, duplicate or oversized provider model option".into());
            }
        }
        Ok(())
    }
}
/// Native conversation data is process-local, never a resumable provider session.
/// Only complete public user/assistant messages belong here, not rendered Items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationRole {
    User,
    Assistant,
}

#[derive(Clone, Debug)]
pub struct ConversationMessage {
    pub role: ConversationRole,
    pub text: Arc<str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationOwner {
    pub task_id: i64,
    pub partner: i64,
    pub agent: Agent,
    pub operation: Operation,
    /// Opaque account/credential/endpoint identity; never serialized into prompts.
    pub connection_scope: String,
}

#[derive(Clone, Debug)]
pub struct NativeConversation {
    pub owner: ConversationOwner,
    pub messages: Arc<[ConversationMessage]>,
}

#[derive(Clone, Debug)]
pub struct Input {
    pub task: crate::store::Snapshot,
    /// Durable ownership of this immutable submitted request; never a tool permission.
    pub work: Option<crate::store::WorkClaim>,
    pub excerpts: Vec<crate::workspace::context::Excerpt>,
    pub report: Option<Arc<crate::workspace::repo::Report>>,
    pub resume: Option<String>,
    pub agent: Agent,
    pub operation: Operation,
    /// Deliberately reviewed root-relative edit paths. Not a sandbox by themselves.
    pub edit_paths: Vec<PathBuf>,
    /// Explicitly accepted portable handoff, never provider sessions or grants.
    pub handoff: Option<crate::workspace::handoff::Document>,
    /// ACP capability/session-list inspection only; never creates a session or sends a model prompt.
    pub inspect_sessions: bool,
    pub inspect_models: bool,
    pub selection: ModelSelection,
    /// Frozen canonical native history, independent of display/journal retention.
    pub conversation: Option<Arc<NativeConversation>>,
    /// Eevee-owned website login result, injected into the launched provider
    /// process environment at Run time. Memory only; never serialized or logged.
    pub credential: Option<crate::providers::auth::SessionCredential>,
}

/// Advanced native requests may target saved task facts without a Request draft.
pub(crate) fn native_request_text(task: &crate::store::Snapshot) -> &str {
    let request = task.request_text();
    if request.trim().is_empty() {
        &task.goal
    } else {
        request
    }
}

impl Input {
    /// Revalidates immutable request ownership and committed task/context facts.
    /// Unclaimed requests retain the ordinary exact-snapshot admission boundary.
    pub fn validate_committed(&self, data_dir: &std::path::Path) -> Result<(), String> {
        self.validate_work()?;
        match &self.work {
            Some(claim) => {
                crate::store::validate_work_input(data_dir, &self.task, &self.excerpts, claim)
            }
            None => crate::store::validate_planning_input(data_dir, &self.task, &self.excerpts),
        }
    }

    fn validate_work(&self) -> Result<(), String> {
        let Some(claim) = &self.work else {
            return Ok(());
        };
        if self.inspect_models || self.inspect_sessions {
            return Err("Provider inspection cannot carry a work claim".into());
        }
        claim.owner.validate()?;
        if claim.task_id != self.task.id
            || claim.owner.partner != self.task.lead_partner
            || claim.owner.form != self.task.form
        {
            return Err("Work claim task or immutable request owner does not match".into());
        }
        if claim.writing != (self.operation == Operation::Implement && !self.agent.is_direct()) {
            return Err("Work claim writing lease does not match the provider operation".into());
        }
        Ok(())
    }

    pub fn validate_selection(&self) -> Result<(), String> {
        self.validate_work()?;
        if self.conversation.is_some()
            && (!self.agent.is_direct() || self.inspect_models || self.inspect_sessions)
        {
            return Err("Canonical native chat history cannot accompany CLI routes or metadata inspection; use /chat-new to reset local context".into());
        }
        if self.inspect_models && (self.inspect_sessions || self.resume.is_some()) {
            return Err(
                "Model inspection cannot resume a session or inspect session history".into(),
            );
        }
        for value in [&self.selection.model, &self.selection.effort]
            .into_iter()
            .flatten()
        {
            if value.is_empty() || value.len() > 256
                || crate::workspace::privacy::redact(value) != *value
                || value.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
            {
                return Err("Invalid model or reasoning selection; no request dispatched".into());
            }
        }
        Ok(())
    }
    /// Review is a fresh, read-only operation, never a continuation or an edit grant.
    pub fn validate_review(&self) -> Result<(), String> {
        self.validate_selection()?;
        if self.operation != Operation::Review {
            return Ok(());
        }
        if self.resume.is_some() {
            return Err("Review requires a fresh provider session; Resume is unavailable".into());
        }
        if self.conversation.is_some() {
            return Err(
                "Independent Review requires fresh context without native conversation history"
                    .into(),
            );
        }
        if !self.edit_paths.is_empty() {
            return Err("Review cannot carry source-writing paths or grants".into());
        }
        if self.inspect_sessions {
            return Err("Review cannot be combined with provider session inspection".into());
        }
        if matches!(self.agent, Agent::OpenCode | Agent::Gemini | Agent::Devin) {
            return Err("Managed Review unavailable: this ACP peer cannot establish an enforceable no-source-write tool policy. Local review remains available.".into());
        }
        Ok(())
    }

    pub fn review_payload(
        &self,
        data_dir: &std::path::Path,
        exclusions: &[PathBuf],
    ) -> Result<Option<String>, String> {
        self.validate_review()?;
        if self.operation != Operation::Review {
            return Ok(None);
        }
        let text = crate::workspace::context::review_payload(
            data_dir,
            &self.task,
            self.report.as_ref(),
            &self.excerpts,
            exclusions,
            self.work.as_ref(),
        )?;
        if text.len() > 1024 * 1024
            || crate::workspace::privacy::redact(&text) != text
            || text.chars().any(|ch| {
                (ch.is_control() && !matches!(ch, '\n' | '\t'))
                    || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            })
        {
            return Err("Review payload exceeds the 1 MiB bound or contains credential/display-control text".into());
        }
        Ok(Some(text))
    }
}
#[derive(Debug)]
pub enum Request {
    Preview(Box<Input>),
    Run {
        generation: u64,
    },
    Answer {
        generation: u64,
        request_id: Value,
        answer: Value,
    },
    Cancel {
        generation: u64,
    },
}
#[derive(Clone, Debug)]
pub struct Preview {
    /// Assigned by the producing worker; retain when deferring or fronting approval.
    pub arrival: u64,
    pub generation: u64,
    pub task_id: i64,
    pub thread_id: Option<String>,
    /// Immutable HTTP destination; external agents retain their launch trust.
    pub destination: Option<String>,
    /// Project-supplied executables and unmanaged tool/helper effects still need consent.
    pub requires_connection_review: bool,
    pub trust: Vec<String>,
    pub payload: String,
    pub blockers: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Message,
    Plan,
    Tool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Failed,
    Cancelled,
    Unknown,
}
#[derive(Clone, Debug)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}
#[derive(Clone, Debug)]
pub struct Question {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub other: bool,
    pub secret: bool,
}
#[derive(Clone, Debug)]
pub struct PermissionOption {
    pub label: String,
    pub description: String,
    pub answer: Value,
}
#[derive(Clone, Debug)]
pub enum PromptKind {
    Questions(Vec<Question>),
    Approval {
        command: String,
        cwd: String,
        reason: String,
        allow_once: bool,
        deny_decision: String,
    },
    /// Options retain each provider's exact response. Deny is index zero; no persistent option.
    Permission {
        description: String,
        options: Vec<PermissionOption>,
    },
}
#[derive(Clone, Debug)]
pub struct Prompt {
    /// Assigned by the producing worker, before the request enters its event queue.
    pub arrival: u64,
    pub request_id: Value,
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub kind: PromptKind,
}
#[derive(Clone, Debug)]
pub enum Event {
    Preview(Preview),
    Models {
        generation: u64,
        task_id: i64,
        catalogue: ModelCatalogue,
    },
    Status {
        generation: u64,
        task_id: i64,
        message: String,
    },
    Inspection {
        generation: u64,
        task_id: i64,
        text: String,
    },
    Thread {
        generation: u64,
        task_id: i64,
        thread_id: String,
        model: String,
        account: String,
    },
    ModelObserved {
        generation: u64,
        task_id: i64,
        model: String,
    },
    /// Complete native parser output, emitted before any proposal/display formatting.
    /// Promoted by the owner only after the matching successful terminal event.
    NativeCompleted {
        generation: u64,
        task_id: i64,
        conversation: Arc<NativeConversation>,
    },
    Item {
        generation: u64,
        task_id: i64,
        thread_id: String,
        turn_id: String,
        item_id: String,
        kind: ItemKind,
        text: String,
        complete: bool,
    },
    Prompt {
        generation: u64,
        task_id: i64,
        request: Prompt,
    },
    PromptClosed {
        generation: u64,
        request_id: Value,
    },
    Finished {
        generation: u64,
        task_id: i64,
        outcome: Outcome,
        message: String,
    },
    Error {
        generation: Option<u64>,
        message: String,
    },
    Journal {
        generation: u64,
        task_id: i64,
        record: crate::store::ProviderRecord,
    },
}

/// One foreground provider route. Constructors do not launch any provider process.
pub struct Worker {
    codex: crate::providers::codex::Worker,
    claude: crate::providers::claude::Worker,
    acp: crate::providers::acp::Worker,
    direct: crate::providers::direct::Worker,
    selected: std::sync::atomic::AtomicUsize,
}
impl Worker {
    pub fn start(
        project: PathBuf,
        data_dir: PathBuf,
        exclusions: Vec<PathBuf>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            codex: crate::providers::codex::Worker::start(
                project.clone(),
                data_dir.clone(),
                exclusions.clone(),
            )?,
            claude: crate::providers::claude::Worker::start(
                project.clone(),
                data_dir.clone(),
                exclusions.clone(),
            )?,
            acp: crate::providers::acp::Worker::start(
                project.clone(),
                data_dir.clone(),
                exclusions.clone(),
            )?,
            direct: crate::providers::direct::Worker::start(project, data_dir, exclusions)?,
            selected: std::sync::atomic::AtomicUsize::new(0),
        })
    }
    pub fn try_request(&self, pending: &mut Option<Request>) -> bool {
        let selected = match pending.as_ref() {
            Some(Request::Preview(input)) => {
                if self.busy() {
                    return false;
                }
                input.agent as usize
            }
            Some(_) => self.selected.load(std::sync::atomic::Ordering::Acquire),
            None => return false,
        };
        let sent = match Agent::ALL[selected] {
            Agent::Codex => self.codex.try_request(pending),
            Agent::Claude => self.claude.try_request(pending),
            Agent::OpenCode | Agent::Gemini | Agent::Devin => self.acp.try_request(pending),
            Agent::ChatGPT
            | Agent::OpenAI
            | Agent::Anthropic
            | Agent::Google
            | Agent::Compatible => self.direct.try_request(pending),
        };
        if sent {
            self.selected
                .store(selected, std::sync::atomic::Ordering::Release);
        }
        sent
    }
    pub fn try_event(&self) -> Option<Event> {
        match Agent::ALL[self.selected.load(std::sync::atomic::Ordering::Acquire)] {
            Agent::Codex => self.codex.try_event(),
            Agent::Claude => self.claude.try_event(),
            Agent::OpenCode | Agent::Gemini | Agent::Devin => self.acp.try_event(),
            Agent::ChatGPT
            | Agent::OpenAI
            | Agent::Anthropic
            | Agent::Google
            | Agent::Compatible => self.direct.try_event(),
        }
    }
    pub fn busy(&self) -> bool {
        self.codex.busy() || self.claude.busy() || self.acp.busy() || self.direct.busy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review(agent: Agent) -> Input {
        Input {
            task: crate::store::Snapshot::empty("project".into()),
            work: None,
            excerpts: Vec::new(),
            report: None,
            resume: None,
            agent,
            operation: Operation::Review,
            edit_paths: Vec::new(),
            handoff: None,
            inspect_sessions: false,
            inspect_models: false,
            selection: ModelSelection::default(),
            conversation: None,
            credential: None,
        }
    }

    fn claimed(agent: Agent, operation: Operation, writing: bool) -> Input {
        let mut input = review(agent);
        input.task.id = 7;
        input.operation = operation;
        input.work = Some(crate::store::WorkClaim {
            task_id: input.task.id,
            owner: crate::store::Attribution {
                partner: input.task.lead_partner,
                name: "Immutable owner".into(),
                form: input.task.form,
            },
            request_id: "provider-admission-test".into(),
            writing,
        });
        input
    }

    #[test]
    fn work_writing_lease_matches_only_agent_implementation_not_native_proposals() {
        for agent in Agent::ALL {
            for operation in [Operation::Plan, Operation::Implement, Operation::Review] {
                let writing = operation == Operation::Implement && !agent.is_direct();
                let mut input = claimed(agent, operation, writing);
                assert!(input.validate_work().is_ok(), "{agent:?} {operation:?}");
                input.work.as_mut().unwrap().writing = !writing;
                assert!(
                    input
                        .validate_committed(std::path::Path::new("unopened-data"))
                        .unwrap_err()
                        .contains("writing lease"),
                    "{agent:?} {operation:?}"
                );
            }
        }
    }

    #[test]
    fn claims_cannot_move_to_another_task_owner_form_or_inspection() {
        let input = claimed(Agent::Codex, Operation::Implement, true);
        let mut changed = input.clone();
        changed.task.id += 1;
        assert!(changed.validate_work().unwrap_err().contains("owner"));
        let mut changed = input.clone();
        changed.task.lead_partner += 1;
        assert!(changed.validate_work().unwrap_err().contains("owner"));
        let mut changed = input.clone();
        changed.task.form += 1;
        assert!(changed.validate_work().unwrap_err().contains("owner"));
        let mut changed = input.clone();
        changed.work.as_mut().unwrap().owner.name.clear();
        assert!(changed.validate_work().is_err());
        for models in [false, true] {
            let mut changed = input.clone();
            changed.inspect_models = models;
            changed.inspect_sessions = !models;
            assert!(
                changed
                    .validate_selection()
                    .unwrap_err()
                    .contains("inspection")
            );
            assert!(
                changed
                    .validate_committed(std::path::Path::new("unopened-data"))
                    .unwrap_err()
                    .contains("inspection")
            );
        }
        assert_eq!(input.work.as_ref().unwrap().owner.name, "Immutable owner");
    }

    #[test]
    fn committed_admission_keeps_submitted_draft_and_requires_the_exact_live_claim() {
        use crate::store::{Event, WorkRequest};
        use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let root = std::env::temp_dir().join(format!(
            "ev-provider-work-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let cleanup = Cleanup(root.canonicalize().unwrap());
        let data = cleanup.0.join("data");
        let worker = crate::store::Worker::start(
            data.clone(),
            cleanup.0.to_str().unwrap().into(),
            crate::store::Policy::default(),
        )
        .unwrap();
        let next = || {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(event) = worker.try_event() {
                    return event;
                }
                assert!(Instant::now() < deadline, "Store acknowledgement timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let Event::Loaded(project) = next() else {
            panic!("Expected loaded store")
        };
        let owner = crate::store::Attribution::of(&project.partners[0]);
        let mut input = claimed(Agent::Codex, Operation::Plan, false);
        input.task = crate::store::Snapshot::empty(cleanup.0.to_str().unwrap().into());
        input.task.goal = "Keep immutable submitted work".into();
        input.task.revision = 1;
        input
            .task
            .partner_drafts
            .entry(owner.partner)
            .or_default()
            .drafts
            .push(crate::store::Draft {
                field: crate::store::EditorField::Request,
                text: "submitted request".into(),
                cursor: 17,
            });
        let mut pending = Some(input.task.clone());
        assert!(worker.try_save(&mut pending));
        let Event::Saved(saved) = next() else {
            panic!("Expected saved task")
        };
        input.task = saved;
        let claim = input.work.as_mut().unwrap();
        claim.task_id = input.task.id;
        claim.owner = owner;
        let claim = claim.clone();
        assert!(
            input.validate_committed(&data).is_err(),
            "An unacquired claim grants nothing"
        );
        let mut pending = Some(WorkRequest::Acquire {
            claim: claim.clone(),
            task: Box::new(input.task.clone()),
        });
        assert!(worker.try_work(&mut pending));
        let Event::Work { result, .. } = next() else {
            panic!("Expected acquired claim")
        };
        result.unwrap();
        assert!(input.validate_committed(&data).is_ok());
        let mut forged = input.clone();
        forged
            .work
            .as_mut()
            .unwrap()
            .request_id
            .push_str("-different");
        assert!(forged.validate_committed(&data).is_err());

        let mut future = input.task.clone();
        future.revision += 1;
        future
            .partner_drafts
            .get_mut(&claim.owner.partner)
            .unwrap()
            .drafts[0]
            .text = "future draft".into();
        future
            .partner_drafts
            .get_mut(&claim.owner.partner)
            .unwrap()
            .drafts[0]
            .cursor = 12;
        let mut pending = Some(future);
        assert!(worker.try_save(&mut pending));
        assert!(matches!(next(), Event::Saved(_)));
        assert!(input.validate_committed(&data).is_ok());
        assert_eq!(input.task.request_text(), "submitted request");
        let mut unclaimed = input.clone();
        unclaimed.work = None;
        assert!(
            unclaimed.validate_committed(&data).is_err(),
            "Unclaimed requests stay strict"
        );
        let mut pending = Some(WorkRequest::Release(claim));
        assert!(worker.try_work(&mut pending));
        let Event::Work { result, .. } = next() else {
            panic!("Expected released claim")
        };
        result.unwrap();
        assert!(
            input.validate_committed(&data).is_err(),
            "Released work cannot authorize callbacks"
        );
        drop(worker);
    }

    #[test]
    fn managed_review_admits_only_fresh_no_write_supported_provider_requests() {
        for agent in [Agent::Codex, Agent::Claude] {
            let mut input = review(agent);
            assert!(input.validate_review().is_ok());
            input.resume = Some("implementation-or-prior-review".into());
            assert!(input.validate_review().unwrap_err().contains("fresh"));
            input.resume = None;
            input.edit_paths.push("src/main.rs".into());
            assert!(
                input
                    .validate_review()
                    .unwrap_err()
                    .contains("source-writing")
            );
            input.edit_paths.clear();
            input.inspect_sessions = true;
            assert!(input.validate_review().unwrap_err().contains("inspection"));
        }
        for agent in [Agent::OpenCode, Agent::Gemini, Agent::Devin] {
            assert!(review(agent).validate_review().unwrap_err().contains("ACP"));
        }
    }

    #[test]
    fn non_review_operations_can_resume_and_select_write_paths() {
        let mut input = review(Agent::Codex);
        input.operation = Operation::Plan;
        input.resume = Some("existing-plan-session".into());
        assert!(input.validate_review().is_ok());
        input.operation = Operation::Implement;
        input.edit_paths.push("src/file.rs".into());
        assert!(input.validate_review().is_ok());
    }

    #[test]
    fn model_catalogues_reject_ambiguous_ids_and_unadvertised_default_efforts() {
        let model = ModelOption {
            id: "official-model".into(),
            name: "Official model".into(),
            description: String::new(),
            efforts: vec!["low".into(), "high".into()],
            default_effort: Some("low".into()),
        };
        let mut catalogue = ModelCatalogue {
            models: vec![model.clone()],
            current: Some(model.id.clone()),
            source: "Official control response".into(),
            effort_note: None,
        };
        assert!(catalogue.validate().is_ok());
        catalogue.models.push(model);
        assert!(catalogue.validate().is_err());
        catalogue.models.pop();
        catalogue.models[0].default_effort = Some("not-advertised".into());
        assert!(catalogue.validate().is_err());
        catalogue.models[0].default_effort = None;
        catalogue.models[0].name = "model\u{202e}spoof".into();
        assert!(catalogue.validate().is_err());
    }

    #[test]
    fn model_inspection_refuses_resume_and_secret_or_control_selections() {
        let mut input = review(Agent::Codex);
        input.operation = Operation::Plan;
        input.inspect_models = true;
        input.resume = Some("owned-session".into());
        assert!(input.validate_selection().is_err());
        input.resume = None;
        input.selection.model = Some("model\nanother-command".into());
        assert!(input.validate_selection().is_err());
        input.selection.model = Some(format!("sk-{}", "x".repeat(48)));
        assert!(input.validate_selection().is_err());
        input.selection.model = Some("provider-specific:version-1".into());
        assert!(input.validate_selection().is_ok());
        input.inspect_sessions = true;
        assert!(input.validate_selection().is_err());
    }
    #[test]
    fn native_history_is_not_a_cli_session_or_metadata_or_independent_review_input() {
        let mut input = review(Agent::Compatible);
        input.operation = Operation::Plan;
        input.conversation = Some(Arc::new(NativeConversation {
            owner: ConversationOwner {
                task_id: input.task.id,
                partner: input.task.lead_partner,
                agent: Agent::Compatible,
                operation: Operation::Plan,
                connection_scope: "opaque".into(),
            },
            messages: vec![
                ConversationMessage {
                    role: ConversationRole::User,
                    text: "raw request".into(),
                },
                ConversationMessage {
                    role: ConversationRole::Assistant,
                    text: "public answer".into(),
                },
            ]
            .into(),
        }));
        assert!(input.validate_selection().is_ok());
        for agent in [
            Agent::Codex,
            Agent::Claude,
            Agent::OpenCode,
            Agent::Gemini,
            Agent::Devin,
        ] {
            input.agent = agent;
            assert!(
                input
                    .validate_selection()
                    .unwrap_err()
                    .contains("/chat-new")
            );
        }
        input.agent = Agent::Compatible;
        input.inspect_models = true;
        assert!(input.validate_selection().is_err());
        input.inspect_models = false;
        input.operation = Operation::Review;
        assert!(
            input
                .validate_review()
                .unwrap_err()
                .contains("fresh context")
        );
    }
}
