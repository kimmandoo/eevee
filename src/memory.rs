//! Explicitly approved, scoped notes. Persistence belongs to the existing store worker.
use serde::{Deserialize, Serialize};

pub const MAX_CONTENT_BYTES: usize = 4096;
pub const MAX_REFERENCE_BYTES: usize = 1024;
pub const MAX_RECORDS: usize = 256;
pub const MAX_SELECTED: usize = 32;
pub const ERASURE_NOTICE: &str = "Deletion removes this memory from future Eevee context. Prior requests, exports, provider history, SQLite/WAL and backups are outside this local deletion; secure erasure is not promised. Deleting its source task preserves approved project/personal memories.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    Task,
    Project,
    Personal,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Task => "Task / only this task",
            Self::Project => "Project / all tasks in this project",
            Self::Personal => "Personal / all projects",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Task => Self::Project,
            Self::Project => Self::Personal,
            Self::Personal => Self::Task,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    UserAuthored,
    VerifiedNote,
}

impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::UserAuthored => "User-authored note / not execution evidence",
            Self::VerifiedNote => "Verified note / inspect its source reference",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub kind: SourceKind,
    pub reference: String,
    /// Historical provenance, not a cascading foreign key for shared memory.
    pub task_id: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub scope: Scope,
    pub content: String,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: i64,
    pub revision: u64,
    pub project: String,
    /// Only task-scoped records have an owning task. Shared records survive purge.
    pub task_id: Option<i64>,
    pub proposal: Proposal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mutation {
    Approve(Proposal),
    Edit {
        id: i64,
        expected_revision: u64,
        proposal: Proposal,
    },
    Delete {
        id: i64,
        expected_revision: u64,
    },
}

/// Reject, rather than offer a partly redacted credential as a habit. Redaction
/// detection remains best-effort; exclusions must also be checked at the source.
pub fn validate_text(text: &str, max: usize) -> Result<(), String> {
    if text.len() > max {
        return Err(format!(
            "Memory text exceeds {max} UTF-8 bytes; nothing was saved."
        ));
    }
    if text.chars().any(|ch| {
        (ch.is_control() && !matches!(ch, '\n' | '\t'))
            || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    }) {
        return Err("Memory cannot contain terminal or directional control characters.".into());
    }
    if crate::privacy::redact(text) != text || text.contains("[REDACTED]") {
        return Err(
            "Credential-bearing or redacted captured content cannot be proposed as memory.".into(),
        );
    }
    Ok(())
}

pub fn validate_proposal(proposal: &Proposal) -> Result<(), String> {
    validate_text(&proposal.content, MAX_CONTENT_BYTES)?;
    validate_text(&proposal.provenance.reference, MAX_REFERENCE_BYTES)?;
    if proposal.content.trim().is_empty()
        || proposal.provenance.reference.trim().is_empty()
        || proposal.provenance.task_id <= 0
    {
        return Err("A memory needs nonempty content and an identified user-authored or verified source note.".into());
    }
    Ok(())
}

pub fn visible(record: &Record, project: &str, task_id: i64) -> bool {
    match record.proposal.scope {
        Scope::Task => record.project == project && record.task_id == Some(task_id),
        Scope::Project => record.project == project && record.task_id.is_none(),
        Scope::Personal => record.task_id.is_none(),
    }
}

pub fn validate_record(record: &Record) -> Result<(), String> {
    validate_proposal(&record.proposal)?;
    if record.id <= 0 || record.revision == 0 {
        return Err("Memory is not a committed approved record.".into());
    }
    match record.proposal.scope {
        Scope::Task if record.task_id.is_none_or(|id| id <= 0) || record.project.is_empty() => {
            return Err("Task memory has no valid owner.".into());
        }
        Scope::Project if record.task_id.is_some() || record.project.is_empty() => {
            return Err("Project memory has an invalid owner.".into());
        }
        Scope::Personal if record.task_id.is_some() => {
            return Err("Personal memory cannot be owned by a task.".into());
        }
        _ => {}
    }
    Ok(())
}

/// Freeze the exact, explicitly selected approved records in the request preview.
/// No default selection and no implicit inclusion of unrelated project history.
pub fn snapshot(
    records: &[Record],
    selected_ids: &[i64],
    project: &str,
    task_id: i64,
) -> Result<Vec<Record>, String> {
    if selected_ids.len() > MAX_SELECTED {
        return Err(format!(
            "Select at most {MAX_SELECTED} approved memories per request."
        ));
    }
    let mut selected = Vec::with_capacity(selected_ids.len());
    for id in selected_ids {
        if selected.iter().any(|record: &Record| record.id == *id) {
            return Err("Duplicate memory selection; reopen the memory drawer.".into());
        }
        let record = records
            .iter()
            .find(|record| record.id == *id)
            .ok_or("A selected memory was deleted; review context again.")?;
        validate_record(record)?;
        if !visible(record, project, task_id) {
            return Err("A selected memory belongs to a different task or project.".into());
        }
        selected.push(record.clone());
    }
    Ok(selected)
}

/// Store worker calls this against a fresh database read at dispatch admission.
/// An edit/deletion invalidates approval; never replace preview bytes silently.
pub fn revalidate(
    planned: &[Record],
    current: &[Record],
    project: &str,
    task_id: i64,
) -> Result<(), String> {
    if planned.len() > MAX_SELECTED {
        return Err("Memory request selection exceeds its bound.".into());
    }
    for (index, record) in planned.iter().enumerate() {
        validate_record(record)?;
        if planned[..index].iter().any(|other| other.id == record.id)
            || !visible(record, project, task_id)
            || !current
                .iter()
                .any(|now| now == record && visible(now, project, task_id))
        {
            return Err("Approved memory changed, was deleted, or left this scope. Review a new request before dispatch.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(scope: Scope) -> Record {
        Record {
            id: 1,
            revision: 1,
            project: "/project/a".into(),
            task_id: (scope == Scope::Task).then_some(7),
            proposal: Proposal {
                scope,
                content: "Keep focused verification commands in the handoff.".into(),
                provenance: Provenance {
                    kind: SourceKind::UserAuthored,
                    reference: "task:7/next-action".into(),
                    task_id: 7,
                },
            },
        }
    }

    #[test]
    fn scope_isolation_and_shared_ownership() {
        assert!(visible(&record(Scope::Task), "/project/a", 7));
        assert!(!visible(&record(Scope::Task), "/project/a", 8));
        assert!(visible(&record(Scope::Project), "/project/a", 8));
        assert!(!visible(&record(Scope::Project), "/project/b", 7));
        assert!(visible(&record(Scope::Personal), "/project/b", 8));
        assert_eq!(record(Scope::Project).task_id, None);
        assert_eq!(record(Scope::Personal).task_id, None);
    }

    #[test]
    fn exact_context_revalidation_refuses_edits_and_deletions() {
        let original = record(Scope::Project);
        let planned = snapshot(std::slice::from_ref(&original), &[1], "/project/a", 7).unwrap();
        assert!(revalidate(&planned, std::slice::from_ref(&original), "/project/a", 7).is_ok());
        let mut edited = original;
        edited.proposal.content = "Changed".into();
        assert!(revalidate(&planned, &[edited], "/project/a", 7).is_err());
        assert!(revalidate(&planned, &[], "/project/a", 7).is_err());
        assert!(snapshot(&planned, &[], "/project/a", 7).unwrap().is_empty());
        assert!(snapshot(&planned, &[1], "/project/b", 7).is_err());
    }

    #[test]
    fn secret_and_unsafe_proposals_are_not_offered() {
        for content in [
            "token=private-value",
            "[REDACTED]",
            "\u{1b}[31m",
            "unsafe\u{202e}text",
        ] {
            let mut proposal = record(Scope::Task).proposal;
            proposal.content = content.into();
            assert!(validate_proposal(&proposal).is_err());
        }
        assert!(validate_text(&"a".repeat(MAX_CONTENT_BYTES + 1), MAX_CONTENT_BYTES).is_err());
        assert!(validate_proposal(&record(Scope::Task).proposal).is_ok());
    }
}
