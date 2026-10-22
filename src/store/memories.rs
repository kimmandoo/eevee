//! Curated memory records.
use super::*;

impl State {
    pub(super) fn memory_request(
        &mut self,
        request: MemoryRequest,
    ) -> Result<Vec<crate::workspace::memory::Record>, String> {
        use crate::workspace::memory::{Mutation, Scope};
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot begin memory transaction: {error}"))?;
        let task = load_task(&transaction, &self.project, request.task_id)?
            .ok_or("Memory task is missing or belongs to another project")?;
        let mut records = memories(&transaction, &self.project, request.task_id)?;
        if let Some(mutation) = request.mutation {
            if task.archived {
                return Err("Restore the archived task before changing memories".into());
            }
            let (previous, proposal) = match mutation {
                Mutation::Approve(proposal) => (None, Some(proposal)),
                Mutation::Edit {
                    id,
                    expected_revision,
                    proposal,
                } => {
                    let previous = records
                        .iter()
                        .find(|record| record.id == id)
                        .ok_or("Memory is missing or outside this task's visible scope")?;
                    if previous.revision != expected_revision {
                        return Err("Memory changed; reload before editing".into());
                    }
                    (Some(previous), Some(proposal))
                }
                Mutation::Delete {
                    id,
                    expected_revision,
                } => {
                    let previous = records
                        .iter()
                        .find(|record| record.id == id)
                        .ok_or("Memory is missing or outside this task's visible scope")?;
                    if previous.revision != expected_revision {
                        return Err("Memory changed; reload before deleting".into());
                    }
                    (Some(previous), None)
                }
            };
            if let Some(proposal) = proposal {
                crate::workspace::memory::validate_proposal(&proposal)?;
                // Existing shared provenance is historical and may survive source-task purge.
                if proposal.provenance.task_id != request.task_id
                    && !previous
                        .is_some_and(|record| record.proposal.provenance == proposal.provenance)
                {
                    return Err("Memory provenance must identify this task or its unchanged approved source".into());
                }
                let task_id = (proposal.scope == Scope::Task).then_some(request.task_id);
                let scope = match proposal.scope {
                    Scope::Task => "task",
                    Scope::Project => "project",
                    Scope::Personal => "personal",
                };
                let json = bounded_json(&proposal, MAX_MEMORY_BYTES)?;
                if let Some(previous) = previous {
                    let revision = previous
                        .revision
                        .checked_add(1)
                        .filter(|revision| *revision <= i64::MAX as u64)
                        .ok_or("Memory revision exhausted")?;
                    transaction.execute(
                        "UPDATE memories SET revision=?1,project=?2,task_id=?3,scope=?4,proposal=?5 WHERE id=?6 AND revision=?7",
                        params![revision as i64, self.project, task_id, scope, json, previous.id, previous.revision as i64],
                    ).map_err(|error| format!("Cannot edit memory: {error}"))?;
                    super::held::purge_memory(&transaction, previous.id)?;
                } else {
                    let count: i64 = transaction
                        .query_row("SELECT count(*) FROM memories", [], |row| row.get(0))
                        .map_err(|error| error.to_string())?;
                    if count >= MAX_MEMORIES as i64 {
                        return Err(
                            "Memory capacity reached (256 records); nothing was evicted".into()
                        );
                    }
                    transaction.execute(
                        "INSERT INTO memories(revision,project,task_id,scope,proposal) VALUES(1,?1,?2,?3,?4)",
                        params![self.project, task_id, scope, json],
                    ).map_err(|error| format!("Cannot approve memory: {error}"))?;
                }
                enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
            } else if let Some(previous) = previous {
                transaction
                    .execute(
                        "DELETE FROM memories WHERE id=?1 AND revision=?2",
                        params![previous.id, previous.revision as i64],
                    )
                    .map_err(|error| format!("Cannot delete memory: {error}"))?;
                super::held::purge_memory(&transaction, previous.id)?;
            }
            records = memories(&transaction, &self.project, request.task_id)?;
        }
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit memory transaction: {error}"))?;
        Ok(records)
    }
}

pub(super) fn validate_memory(record: &crate::workspace::memory::Record) -> Result<(), String> {
    crate::workspace::memory::validate_record(record)?;
    reject_secret_identity(&record.project)?;
    if record.revision > i64::MAX as u64 || record.project.len() > 4096 {
        return Err("Memory identity exceeds its bounded storage range".into());
    }
    Ok(())
}

pub(super) fn memory_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<crate::workspace::memory::Record> {
    Ok(crate::workspace::memory::Record {
        id: row.get(0)?,
        revision: row.get(1)?,
        project: row.get(2)?,
        task_id: row.get(3)?,
        proposal: json_column(row, 4, MAX_MEMORY_BYTES)?,
    })
}

pub(super) fn memories(
    connection: &Connection,
    project: &str,
    task_id: i64,
) -> Result<Vec<crate::workspace::memory::Record>, String> {
    let mut statement = connection.prepare(
        "SELECT id,revision,project,task_id,proposal FROM memories WHERE scope='personal'
         OR (project=?1 AND (scope='project' OR (scope='task' AND task_id=?2))) ORDER BY id LIMIT 257",
    ).map_err(|error| error.to_string())?;
    let records = statement
        .query_map(params![project, task_id], memory_row)
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    if records.len() > MAX_MEMORIES {
        return Err("Memory capacity exceeds its bounded limit".into());
    }
    for record in &records {
        validate_memory(record)?;
        if !crate::workspace::memory::visible(record, project, task_id) {
            return Err("Stored memory scope differs from its ownership".into());
        }
    }
    Ok(records)
}

pub(super) fn inspect_memories(connection: &Connection) -> Result<(), String> {
    inspect_columns(
        connection,
        "memories",
        &[
            ("id", "INTEGER", false, 1),
            ("revision", "INTEGER", true, 0),
            ("project", "TEXT", true, 0),
            ("task_id", "INTEGER", false, 0),
            ("scope", "TEXT", true, 0),
            ("proposal", "TEXT", true, 0),
        ],
    )?;
    let invalid: bool = connection
        .query_row(
            "SELECT (SELECT count(*) FROM memories)>256 OR EXISTS(
         SELECT 1 FROM memories m LEFT JOIN tasks t ON t.id=m.task_id
         WHERE (m.scope='task' AND (t.id IS NULL OR t.project!=m.project))
         OR (m.scope!='task' AND m.task_id IS NOT NULL))",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if invalid {
        return Err("Stored memories have invalid ownership or capacity".into());
    }
    let mut statement = connection
        .prepare("SELECT id,revision,project,task_id,proposal,scope FROM memories")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| Ok((memory_row(row)?, row.get::<_, String>(5)?)))
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (record, scope) = row.map_err(|error| error.to_string())?;
        validate_memory(&record)?;
        let expected = match record.proposal.scope {
            crate::workspace::memory::Scope::Task => "task",
            crate::workspace::memory::Scope::Project => "project",
            crate::workspace::memory::Scope::Personal => "personal",
        };
        if scope != expected {
            return Err("Stored memory scope differs from its proposal".into());
        }
    }
    Ok(())
}
