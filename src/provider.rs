//! Eevee request/event contract; provider sessions and permissions are never portable.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Agent {
    #[default]
    Codex,
    Claude,
    OpenCode,
    Gemini,
    Devin,
}
impl Agent {
    pub const ALL: [Self; 5] = [
        Self::Codex,
        Self::Claude,
        Self::OpenCode,
        Self::Gemini,
        Self::Devin,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Agent",
            Self::OpenCode => "OpenCode ACP",
            Self::Gemini => "Gemini ACP",
            Self::Devin => "Devin ACP",
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
        }
    }
    pub fn from_command(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|agent| agent.command() == name)
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
                && crate::privacy::redact(value) == value
                && !value.chars().any(|c| c.is_control()
                    || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        }
        if self.models.len() > 256
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
#[derive(Clone, Debug)]
pub struct Input {
    pub task: crate::store::Snapshot,
    pub excerpts: Vec<crate::context::Excerpt>,
    pub report: Option<Arc<crate::repo::Report>>,
    pub resume: Option<String>,
    pub agent: Agent,
    pub operation: Operation,
    /// Deliberately reviewed root-relative edit paths. Not a sandbox by themselves.
    pub edit_paths: Vec<PathBuf>,
    /// Explicitly accepted portable handoff, never provider sessions or grants.
    pub handoff: Option<crate::handoff::Document>,
    /// ACP capability/session-list inspection only; never creates a session or sends a model prompt.
    pub inspect_sessions: bool,
    pub inspect_models: bool,
    pub selection: ModelSelection,
    /// Eevee-owned website login result, injected into the launched provider
    /// process environment at Run time. Memory only; never serialized or logged.
    pub credential: Option<crate::auth::SessionCredential>,
}

impl Input {
    pub fn validate_selection(&self) -> Result<(), String> {
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
                || crate::privacy::redact(value) != *value
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
        let text = crate::context::review_payload(
            data_dir,
            &self.task,
            self.report.as_ref(),
            &self.excerpts,
            exclusions,
        )?;
        if text.len() > 1024 * 1024
            || crate::privacy::redact(&text) != text
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
    pub generation: u64,
    pub task_id: i64,
    pub thread_id: Option<String>,
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
    codex: crate::codex::Worker,
    claude: crate::claude::Worker,
    acp: crate::acp::Worker,
    selected: std::sync::atomic::AtomicUsize,
}
impl Worker {
    pub fn start(
        project: PathBuf,
        data_dir: PathBuf,
        exclusions: Vec<PathBuf>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            codex: crate::codex::Worker::start(
                project.clone(),
                data_dir.clone(),
                exclusions.clone(),
            )?,
            claude: crate::claude::Worker::start(
                project.clone(),
                data_dir.clone(),
                exclusions.clone(),
            )?,
            acp: crate::acp::Worker::start(project, data_dir, exclusions)?,
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
        }
    }
    pub fn busy(&self) -> bool {
        self.codex.busy() || self.claude.busy() || self.acp.busy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review(agent: Agent) -> Input {
        Input {
            task: crate::store::Snapshot::empty("project".into()),
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
            credential: None,
        }
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
}
