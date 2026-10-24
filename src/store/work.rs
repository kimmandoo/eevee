//! Transactional request ownership. Claims grant no command or filesystem authority.
use super::*;

fn validate_claim(claim: &WorkClaim) -> Result<(), String> {
    claim.owner.validate()?;
    if claim.task_id <= 0
        || claim.request_id.is_empty()
        || claim.request_id.len() > 128
        || !claim
            .request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err("Invalid work request identity".into());
    }
    Ok(())
}

fn rows(connection: &Connection) -> Result<Vec<(WorkClaim, String)>, String> {
    let mut statement = connection.prepare("SELECT task_id,partner,request_id,owner,writing,kind FROM active_work ORDER BY task_id,partner")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let owner: Attribution = json_column(row, 3, 2048)?;
            let partner: i64 = row.get(1)?;
            if owner.partner != partner {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok((
                WorkClaim {
                    task_id: row.get(0)?,
                    owner,
                    request_id: row.get(2)?,
                    writing: row.get(4)?,
                },
                row.get(5)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    for (claim, kind) in &rows {
        validate_claim(claim)?;
        if kind != "provider" && kind != "command" || kind == "command" && claim.writing {
            return Err("Invalid stored work kind".into());
        }
    }
    Ok(rows)
}

pub(super) fn claims(connection: &Connection) -> Result<Vec<WorkClaim>, String> {
    Ok(rows(connection)?
        .into_iter()
        .map(|(claim, _)| claim)
        .collect())
}

pub(super) fn validate_live(connection: &Connection, claim: &WorkClaim) -> Result<(), String> {
    validate_claim(claim)?;
    if !rows(connection)?
        .iter()
        .any(|(stored, kind)| stored == claim && kind == "provider")
    {
        return Err("Request no longer owns its exact durable work claim; preview again".into());
    }
    Ok(())
}

fn acquire(
    connection: &Connection,
    task: &Snapshot,
    claim: &WorkClaim,
    kind: &str,
) -> Result<(), String> {
    validate_claim(claim)?;
    if task.id != claim.task_id || task.archived || task.goal.trim().is_empty() {
        return Err("Work requires its saved active task with a goal".into());
    }
    let current = rows(connection)?;
    if let Some((existing, existing_kind)) = current
        .iter()
        .find(|(existing, _)| existing.request_id == claim.request_id)
    {
        return if existing == claim && existing_kind == kind {
            Ok(())
        } else {
            Err("Work request identity is already owned by a different claim".into())
        };
    }
    let roster = partners(connection)?;
    let member = roster
        .iter()
        .find(|partner| partner.id == claim.owner.partner)
        .ok_or("Work owner is no longer in the party")?;
    if Attribution::of(member) != claim.owner {
        return Err("Work owner identity changed; refresh and preview again".into());
    }
    if current
        .iter()
        .any(|(existing, _)| existing.owner.partner == claim.owner.partner)
    {
        return Err(
            "This partner already owns active work; wait for its exact request to settle".into(),
        );
    }
    if current
        .iter()
        .filter(|(existing, _)| existing.task_id == task.id)
        .count()
        >= 3
    {
        return Err("This task already has three working partners".into());
    }
    if claim.writing
        && current
            .iter()
            .any(|(existing, _)| existing.task_id == task.id && existing.writing)
    {
        return Err("Another partner is editing this task; wait for its write lease".into());
    }
    if kind == "provider" {
        super::receipts::reserve_capacity(connection, task.id)?;
    }
    connection.execute("INSERT INTO active_work(task_id,partner,request_id,owner,writing,kind) VALUES (?1,?2,?3,?4,?5,?6)",
        params![claim.task_id, claim.owner.partner, claim.request_id, serde_json::to_string(&claim.owner).map_err(|error| error.to_string())?, claim.writing, kind])
        .map_err(|error| error.to_string())?;
    if claim.writing {
        // The overlap begins here, even if a command reserved before the writer.
        let active = {
            let mut statement = connection
                .prepare(&format!(
                    "SELECT {RUN_COLUMNS} FROM executions WHERE task_id=?1 AND reserved=1"
                ))
                .map_err(|error| error.to_string())?;
            statement
                .query_map([task.id], stored_run_row)
                .map_err(|error| error.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?
        };
        for mut run in active {
            run.record.revision_moving = true;
            write_run(connection, &run)?;
        }
    }
    Ok(())
}

fn release(connection: &Connection, claim: &WorkClaim, kind: &str) -> Result<(), String> {
    validate_claim(claim)?;
    let current = rows(connection)?;
    // Absent exact requests are idempotent. A stale request never deletes its successor.
    if let Some((stored, stored_kind)) = current
        .iter()
        .find(|(stored, _)| stored.request_id == claim.request_id)
    {
        if stored != claim || stored_kind != kind {
            return Err("Refusing to release another work owner's claim".into());
        }
        connection.execute("DELETE FROM active_work WHERE task_id=?1 AND partner=?2 AND request_id=?3 AND kind=?4",
            params![claim.task_id, claim.owner.partner, claim.request_id, kind]).map_err(|error| error.to_string())?;
    }
    Ok(())
}

impl State {
    pub(super) fn work_request(&mut self, request: &WorkRequest) -> Result<Vec<WorkClaim>, String> {
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        match request {
            WorkRequest::Acquire { claim, task } => {
                validate(task)?;
                if task.project != self.project {
                    return Err("Work task belongs to another project".into());
                }
                if rows(&transaction)?
                    .iter()
                    .any(|(stored, kind)| stored == claim && kind == "provider")
                {
                    admission::validate_work_task(&transaction, task, claim)?;
                } else {
                    admission::validate_committed_task(&transaction, task)?;
                }
                acquire(&transaction, task, claim, "provider")?;
            }
            WorkRequest::Release(claim) => {
                if rows(&transaction)?
                    .iter()
                    .any(|(stored, _)| stored.request_id == claim.request_id)
                {
                    load_task(&transaction, &self.project, claim.task_id)?
                        .ok_or("Work task is missing or belongs to another project")?;
                }
                // A provider's cancellation/EOF cannot release its writer while
                // a borrowed command still owns a live reservation or journal.
                let mut statement = transaction
                    .prepare("SELECT record FROM executions WHERE task_id=?1 AND reserved=1")
                    .map_err(|error| error.to_string())?;
                let nested = statement
                    .query_map([claim.task_id], |row| {
                        json_column::<crate::workspace::runner::RunRecord>(
                            row,
                            0,
                            super::runs::MAX_RUN_BYTES,
                        )
                    })
                    .map_err(|error| error.to_string())?;
                for record in nested {
                    if record
                        .map_err(|error| error.to_string())?
                        .prepared
                        .partner
                        .as_ref()
                        == Some(&claim.owner)
                    {
                        return Err("Provider writer must remain owned until its nested command and critical journal settle".into());
                    }
                }
                drop(statement);
                release(&transaction, claim, "provider")?;
            }
        }
        let result = claims(&transaction)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(result)
    }
}

/// A nested callback borrows ONLY its exact live writer, never another owner or a batch grant.
pub(super) fn validate_nested_command(
    connection: &Connection,
    claim: &WorkClaim,
    prepared: &crate::workspace::runner::Prepared,
) -> Result<(), String> {
    validate_live(connection, claim)?;
    if !claim.writing
        || claim.task_id != prepared.task_id
        || prepared.partner.as_ref() != Some(&claim.owner)
    {
        return Err("Command callback does not own its exact Implement writer".into());
    }
    let mut statement = connection
        .prepare("SELECT record FROM executions WHERE task_id=?1 AND reserved=1")
        .map_err(|e| e.to_string())?;
    let active = statement
        .query_map([claim.task_id], |row| {
            json_column::<crate::workspace::runner::RunRecord>(row, 0, super::runs::MAX_RUN_BYTES)
        })
        .map_err(|e| e.to_string())?;
    for record in active {
        let record = record.map_err(|e| e.to_string())?;
        if record.prepared.partner.as_ref() == Some(&claim.owner)
            && record.prepared.run_id != prepared.run_id
        {
            return Err("This exact writer already owns a nested command".into());
        }
    }
    Ok(())
}

pub(super) fn acquire_command(
    connection: &Connection,
    task: &Snapshot,
    prepared: &crate::workspace::runner::Prepared,
) -> Result<bool, String> {
    let owner = match &prepared.partner {
        Some(owner) => owner.clone(),
        None => Attribution::of(
            partners(connection)?
                .iter()
                .find(|member| member.id == task.lead_partner)
                .ok_or("Command owner is missing")?,
        ),
    };
    let claim = WorkClaim {
        task_id: task.id,
        owner,
        request_id: prepared.request_id.clone(),
        writing: false,
    };
    acquire(connection, task, &claim, "command")?;
    Ok(claims(connection)?
        .iter()
        .any(|claim| claim.task_id == task.id && claim.writing))
}

pub(super) fn release_command(
    connection: &Connection,
    prepared: &crate::workspace::runner::Prepared,
) -> Result<(), String> {
    if let Some((claim, kind)) = rows(connection)?
        .into_iter()
        .find(|(claim, _)| claim.request_id == prepared.request_id)
    {
        if claim.task_id != prepared.task_id
            || kind != "command"
            || claim.writing
            || prepared
                .partner
                .as_ref()
                .is_some_and(|owner| owner != &claim.owner)
        {
            return Err("Command cannot release another request's ownership".into());
        }
        release(connection, &claim, "command")?;
    }
    Ok(())
}

pub(super) fn inspect_work(connection: &Connection) -> Result<(), String> {
    inspect_columns(
        connection,
        "active_work",
        &[
            ("task_id", "INTEGER", true, 1),
            ("partner", "INTEGER", true, 2),
            ("request_id", "TEXT", true, 0),
            ("owner", "TEXT", true, 0),
            ("writing", "INTEGER", true, 0),
            ("kind", "TEXT", true, 0),
        ],
    )?;
    let invalid: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM active_work w LEFT JOIN tasks t ON t.id=w.task_id LEFT JOIN partners p ON p.id=w.partner WHERE t.id IS NULL OR p.id IS NULL OR t.archived=1)
         OR EXISTS(SELECT 1 FROM active_work GROUP BY task_id HAVING count(*)>3 OR sum(writing)>1)
         OR EXISTS(SELECT 1 FROM active_work GROUP BY partner HAVING count(*)>1)", [], |row| row.get(0)).map_err(|error| error.to_string())?;
    if invalid {
        return Err("Invalid durable work ownership or write lease capacity".into());
    }
    rows(connection)?;
    Ok(())
}
