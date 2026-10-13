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
}
impl Agent {
    pub const ALL: [Self; 4] = [Self::Codex, Self::Claude, Self::OpenCode, Self::Gemini];
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Agent",
            Self::OpenCode => "OpenCode ACP",
            Self::Gemini => "Gemini ACP",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Operation {
    #[default]
    Plan,
    Implement,
    Review,
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
}

impl Input {
    /// Review is a fresh, read-only operation, never a continuation or an edit grant.
    pub fn validate_review(&self) -> Result<(), String> {
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
        if matches!(self.agent, Agent::OpenCode | Agent::Gemini) {
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
            Agent::OpenCode | Agent::Gemini => self.acp.try_request(pending),
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
            Agent::OpenCode | Agent::Gemini => self.acp.try_event(),
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
        for agent in [Agent::OpenCode, Agent::Gemini] {
            assert!(review(agent).validate_review().unwrap_err().contains("ACP"));
        }
    }

    #[test]
    fn review_operation_is_explicit_and_keeps_existing_operation_encoding() {
        assert_eq!(
            serde_json::to_string(&Operation::Review).unwrap(),
            "\"Review\""
        );
        assert_eq!(serde_json::to_string(&Operation::Plan).unwrap(), "\"Plan\"");
        assert_eq!(
            serde_json::to_string(&Operation::Implement).unwrap(),
            "\"Implement\""
        );
        let mut input = review(Agent::Codex);
        input.operation = Operation::Plan;
        input.resume = Some("existing-plan-session".into());
        assert!(input.validate_review().is_ok());
        input.operation = Operation::Implement;
        input.edit_paths.push("src/file.rs".into());
        assert!(input.validate_review().is_ok());
    }
}
