//! Owned run records, deletion scope and evidence quota.
use super::*;

pub(super) const MAX_RUN_BYTES: usize = 128 * 1024;
pub(super) const MAX_RUNS: usize = 256;
pub(super) const RUN_HEADROOM: u64 = 256 * 1024;
pub(super) const MAX_SPOOL_BYTES: u64 = 8 * 1024 * 1024;
pub(super) const RUN_COLUMNS: &str = "record,output_name,output_identity,capacity,reserved";

pub(super) struct StoredRun {
    pub(super) record: crate::workspace::runner::RunRecord,
    pub(super) output_name: String,
    pub(super) output_identity: String,
    pub(super) capacity: u64,
    pub(super) reserved: bool,
}

pub(super) fn active_state(state: &crate::workspace::runner::RunState) -> bool {
    use crate::workspace::runner::RunState;
    matches!(
        state,
        RunState::AwaitingDispatch | RunState::Running | RunState::Cancelling
    )
}

pub(crate) fn valid_run_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
pub(super) fn deletion_scope(
    ids: &[i64],
    expired_before: Option<i64>,
    policy: Policy,
    now: i64,
) -> Result<i64, String> {
    if ids.is_empty() || ids.len() > MAX_TASKS {
        return Err("Deletion requires a bounded, nonempty list of task IDs".into());
    }
    for (index, id) in ids.iter().enumerate() {
        if ids[..index].contains(id) {
            return Err("Deletion contains duplicate task IDs".into());
        }
    }
    let eligible_before = now.saturating_sub(i64::from(policy.retention_days) * 86_400_000);
    if expired_before.is_some_and(|cutoff| cutoff < 0 || cutoff > eligible_before) {
        return Err(
            "Retention preview is incompatible with the configured policy; review again".into(),
        );
    }
    Ok(eligible_before)
}

pub(super) fn deletion_task(
    connection: &Connection,
    project: &str,
    id: i64,
    expired_before: Option<i64>,
    eligible_before: i64,
) -> Result<Snapshot, String> {
    no_active(connection, Some(id))?;
    let task = load_task(connection, project, id)?
        .ok_or("Deletion target is missing or belongs to another project; review again")?;
    if expired_before.is_some_and(|cutoff| {
        !task.archived
            || task.archived_at <= 0
            || task.archived_at > cutoff
            || task.archived_at > eligible_before
    }) {
        return Err("Retention target changed or is not old enough; review again".into());
    }
    Ok(task)
}

pub(super) fn spool_name(run_id: &str) -> Result<String, String> {
    if !valid_run_id(run_id) {
        return Err("Invalid run identity".into());
    }
    Ok(format!("ev-run-{run_id}.output"))
}

pub(super) fn run_json(record: &crate::workspace::runner::RunRecord) -> Result<String, String> {
    use crate::workspace::runner::{RunState, TreeOutcome};
    let prepared = &record.prepared;
    if prepared
        .before
        .fingerprint
        .as_ref()
        .is_some_and(|value| value.len() > 256)
        || record
            .after
            .fingerprint
            .as_ref()
            .is_some_and(|value| value.len() > 256)
    {
        return Err("Source identity exceeds the journal bound".into());
    }
    if !valid_run_id(&prepared.run_id)
        || !valid_run_id(&prepared.request_id)
        || prepared.task_id <= 0
    {
        return Err("Invalid run/request/task correlation".into());
    }
    if prepared
        .rerun_of
        .as_ref()
        .is_some_and(|run| !valid_run_id(run) || *run == prepared.run_id)
    {
        return Err("Invalid or self-referencing rerun identity".into());
    }
    if let Some(partner) = &prepared.partner {
        partner.validate()?;
    }
    if matches!(record.state, RunState::Running | RunState::Cancelling)
        && record.started_at_ms.is_none()
        || active_state(&record.state) && record.ended_at_ms.is_some()
        || record.state == RunState::AwaitingDispatch && record.started_at_ms.is_some()
    {
        return Err("Run lifecycle timestamps do not match its observed state".into());
    }
    if record.state == RunState::Succeeded
        && (record.started_at_ms.is_none()
            || record.exit_code != Some(0)
            || record.tree != TreeOutcome::OwnedTreeExited
            || record.cancelled
            || record.signal.is_some())
    {
        return Err(
            "Successful command metadata lacks observed start, exit zero or owned-tree exit".into(),
        );
    }
    if record.state == RunState::Cancelling && !record.cancelled
        || record.state == RunState::Cancelled
            && (!record.cancelled
                || record.started_at_ms.is_none()
                || record.tree != TreeOutcome::OwnedTreeExited)
    {
        return Err(
            "Cancelled command metadata lacks an observed cancellation and owned-tree exit".into(),
        );
    }
    if record
        .ended_at_ms
        .is_some_and(|end| record.started_at_ms.is_some_and(|start| end < start))
    {
        return Err("Run end precedes its start".into());
    }
    let json = serde_json::to_string(record).map_err(|error| error.to_string())?;
    if json.len() > MAX_RUN_BYTES {
        return Err("Run metadata exceeds 128 KiB".into());
    }
    Ok(json)
}

pub(super) fn no_active(connection: &Connection, task_id: Option<i64>) -> Result<(), String> {
    let active: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM executions WHERE reserved=1 AND (?1 IS NULL OR task_id=?1)) OR EXISTS(SELECT 1 FROM active_work WHERE ?1 IS NULL OR task_id=?1)",
        [task_id], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if active {
        Err("Task owns reserved or active work; cancel and wait before removing, purging or archiving evidence".into())
    } else {
        Ok(())
    }
}

pub(super) fn excerpts(
    connection: &Connection,
    task_id: i64,
) -> Result<Vec<crate::workspace::context::Excerpt>, String> {
    let mut statement = connection
        .prepare("SELECT record FROM excerpts WHERE task_id=?1 ORDER BY id LIMIT 17")
        .map_err(|error| error.to_string())?;
    let result = statement
        .query_map([task_id], |row| json_column(row, 0, 512 * 1024))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<crate::workspace::context::Excerpt>>>()
        .map_err(|error| error.to_string())?;
    if result.len() > crate::workspace::context::MAX_EXCERPTS {
        return Err("Stored basket exceeds its bound".into());
    }
    for excerpt in &result {
        excerpt.validate()?;
    }
    Ok(result)
}

pub(super) fn stored_run(
    connection: &Connection,
    task_id: i64,
    run_id: &str,
) -> Result<StoredRun, String> {
    connection
        .query_row(
            &format!("SELECT {RUN_COLUMNS} FROM executions WHERE task_id=?1 AND run_id=?2"),
            params![task_id, run_id],
            stored_run_row,
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Run is missing or belongs to another task".into())
}

pub(super) fn stored_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRun> {
    Ok(StoredRun {
        record: json_column(row, 0, MAX_RUN_BYTES)?,
        output_name: row.get(1)?,
        output_identity: row.get(2)?,
        capacity: row.get(3)?,
        reserved: row.get(4)?,
    })
}

pub(super) fn write_run(connection: &Connection, stored: &StoredRun) -> Result<(), String> {
    connection
        .execute(
            "UPDATE executions SET record=?1,reserved=?2 WHERE run_id=?3 AND task_id=?4",
            params![
                run_json(&stored.record)?,
                stored.reserved,
                stored.record.prepared.run_id,
                stored.record.prepared.task_id
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

impl State {
    pub(super) fn context(
        &mut self,
        request: ContextRequest,
    ) -> Result<Vec<crate::workspace::context::Excerpt>, String> {
        let growing = matches!(&request.operation, ContextOperation::Pin(_));
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let task = load_task(&transaction, &self.project, request.task_id)?
            .ok_or("Task is missing or belongs to another project")?;
        let current = excerpts(&transaction, task.id)?;
        match request.operation {
            ContextOperation::Load => return Ok(current),
            ContextOperation::Validate(expected) => {
                if expected.is_empty() || expected.len() > crate::workspace::context::MAX_EXCERPTS {
                    return Err("A dispatch basket requires 1–16 committed excerpts".into());
                }
                for (index, excerpt) in expected.iter().enumerate() {
                    if excerpt.task_id != task.id
                        || !excerpt.available
                        || expected[..index].iter().any(|old| old.id == excerpt.id)
                        || !current.iter().any(|stored| stored == excerpt)
                    {
                        return Err(
                            "Basket changed, was removed or purged; load and review it again"
                                .into(),
                        );
                    }
                }
                return Ok(expected);
            }
            ContextOperation::Pin(mut excerpt) => {
                no_active(&transaction, Some(task.id))?;
                if task.archived {
                    return Err("Restore the task before pinning context".into());
                }
                if current.len() >= crate::workspace::context::MAX_EXCERPTS {
                    return Err("Basket capacity reached (16 excerpts); nothing was evicted".into());
                }
                if excerpt.task_id != task.id || excerpt.id != 0 || !excerpt.available {
                    return Err("Pin requires a new available capture for this task".into());
                }
                excerpt.text = crate::workspace::privacy::redact(&excerpt.text).into_owned();
                excerpt.validate()?;
                if let Some(run) = &excerpt.origin_run {
                    stored_run(&transaction, task.id, run)?;
                }
                // The row identity belongs to storage, not to a user-supplied reference.
                transaction
                    .execute(
                        "INSERT INTO excerpts(task_id,record) VALUES (?1,'')",
                        [task.id],
                    )
                    .map_err(|error| error.to_string())?;
                excerpt.id = transaction.last_insert_rowid();
                transaction
                    .execute(
                        "UPDATE excerpts SET record=?1 WHERE id=?2",
                        params![
                            serde_json::to_string(&excerpt).map_err(|error| error.to_string())?,
                            excerpt.id
                        ],
                    )
                    .map_err(|error| error.to_string())?;
                super::records::capture(
                    &transaction,
                    &self.project,
                    &excerpt,
                    request.producer.as_ref(),
                )?;
            }
            ContextOperation::Remove(id) => {
                no_active(&transaction, Some(task.id))?;
                super::records::purge_capture(&transaction, &self.project, task.id, id)?;
                if !current.iter().any(|excerpt| excerpt.id == id) {
                    return Err("Excerpt is missing or belongs to another task".into());
                }
                transaction
                    .execute(
                        "DELETE FROM excerpts WHERE task_id=?1 AND id=?2",
                        params![task.id, id],
                    )
                    .map_err(|error| error.to_string())?;
            }
            ContextOperation::Purge(ids) => {
                no_active(&transaction, Some(task.id))?;
                if ids.is_empty() || ids.len() > crate::workspace::context::MAX_EXCERPTS {
                    return Err("Purge requires a bounded nonempty excerpt selection".into());
                }
                for (index, id) in ids.iter().enumerate() {
                    if ids[..index].contains(id) {
                        return Err("Duplicate purge reference".into());
                    }
                    let mut excerpt = current
                        .iter()
                        .find(|excerpt| excerpt.id == *id)
                        .cloned()
                        .ok_or("Purge reference is missing or belongs to another task")?;
                    super::records::purge_capture(&transaction, &self.project, task.id, *id)?;
                    excerpt.text.clear();
                    excerpt.available = false;
                    transaction
                        .execute(
                            "UPDATE excerpts SET record=?1 WHERE task_id=?2 AND id=?3",
                            params![
                                serde_json::to_string(&excerpt)
                                    .map_err(|error| error.to_string())?,
                                task.id,
                                id
                            ],
                        )
                        .map_err(|error| error.to_string())?;
                }
            }
        }
        if growing {
            enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
        }
        let result = excerpts(&transaction, task.id)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(result)
    }

    pub(super) fn run_request(&mut self, request: RunRequest) -> Result<RunResponse, String> {
        let task_id = request.task_id;
        let connection = self
            .connection
            .as_ref()
            .ok_or("Storage is unavailable; retry opening it")?;
        load_task(connection, &self.project, task_id)?
            .ok_or("Task is missing or belongs to another project")?;
        match request.operation {
            RunOperation::Reserve { prepared, capacity } => self
                .reserve_run(task_id, prepared, capacity)
                .map(RunResponse::Reserved),
            RunOperation::Update(record) => {
                self.update_run(task_id, record).map(RunResponse::Updated)
            }
            RunOperation::Abandon { request_id, run_id } => {
                let mut stored = stored_run(connection, task_id, &run_id)?;
                if stored.record.prepared.request_id != request_id
                    || stored.record.state != crate::workspace::runner::RunState::AwaitingDispatch
                {
                    return Err(
                        "Only the correlated, undispatched reservation can be abandoned".into(),
                    );
                }
                stored.record.state = crate::workspace::runner::RunState::Interrupted;
                stored.record.ended_at_ms = Some(timestamp()? as u64);
                stored.record.error = Some(
                    "Approval was invalidated or dispatch was refused; command was not started"
                        .into(),
                );
                stored.record.output_unavailable = true;
                self.update_run(task_id, stored.record)
                    .map(RunResponse::Updated)
            }
            RunOperation::Load => self.load_runs(task_id).map(RunResponse::Runs),
            RunOperation::ReadOutput {
                run_id,
                offset,
                limit,
            } => self
                .read_output(task_id, &run_id, offset, limit)
                .map(RunResponse::Output),
            RunOperation::PurgeOutput { run_id } => self
                .purge_output(task_id, &run_id)
                .map(|(record, warning)| RunResponse::Purged { record, warning }),
        }
    }

    pub(super) fn reserve_run(
        &mut self,
        task_id: i64,
        prepared: crate::workspace::runner::Prepared,
        capacity: u64,
    ) -> Result<CommittedRunPermit, String> {
        if prepared.task_id != task_id || !(1024..=MAX_SPOOL_BYTES).contains(&capacity) {
            return Err("Run owner or capture reservation bound is invalid (1 KiB–8 MiB)".into());
        }
        if timestamp()? as u64 >= prepared.expires_at_ms {
            return Err("Prepared request expired before quota reservation; review again".into());
        }
        let name = spool_name(&prepared.run_id)?;
        let connection = self.connection.as_mut().ok_or("Storage is unavailable")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let task = load_task(&transaction, &self.project, task_id)?.ok_or("Task is missing")?;
        if task.archived {
            return Err("Restore the task before execution".into());
        }
        if let Some(origin) = &prepared.rerun_of {
            let prior = stored_run(&transaction, task_id, origin)?;
            if prior.record.prepared.spec != prepared.spec
                || prior.record.prepared.cwd != prepared.cwd
            {
                return Err("Rerun argv/cwd differs from the referenced task-owned run".into());
            }
        }
        super::records::reserve_capacity(&transaction, &self.project)?;
        let revision_moving = super::work::acquire_command(&transaction, &task, &prepared)?;
        let mut record = crate::workspace::runner::RunRecord::awaiting(&prepared);
        record.revision_moving = revision_moving;
        record.redact_for_storage();
        let json = run_json(&record)?;
        let count: usize = transaction
            .query_row(
                "SELECT count(*) FROM executions WHERE task_id=?1",
                [task_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if count >= MAX_RUNS {
            return Err("Run journal reached 256 retained records; no evidence was evicted".into());
        }
        let directory = SpoolDirectory::open(&self.data_dir)?;
        let file = directory.create(&name)?;
        let identity = file_identity(&file)?;
        let result = (|| {
            file.sync_all().map_err(|error| error.to_string())?;
            directory.sync()?;
            transaction.execute("INSERT INTO executions(run_id,task_id,request_id,record,output_name,output_identity,capacity,reserved) VALUES (?1,?2,?3,?4,?5,?6,?7,1)",
                params![prepared.run_id, task_id, prepared.request_id, json, name, identity, capacity]).map_err(|error| error.to_string())?;
            enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
            transaction.commit().map_err(|error| error.to_string())
        })();
        if let Err(error) = result {
            drop(file);
            let _ = directory.remove(&name, &identity);
            return Err(error);
        }
        Ok(CommittedRunPermit {
            request_id: prepared.request_id,
            task_id,
            run_id: prepared.run_id,
            file,
            capacity,
            rerun_of: prepared.rerun_of,
            revision_moving,
            _committed: (),
        })
    }

    pub(super) fn update_run(
        &mut self,
        task_id: i64,
        mut record: crate::workspace::runner::RunRecord,
    ) -> Result<crate::workspace::runner::RunRecord, String> {
        use crate::workspace::runner::RunState;
        if record.prepared.task_id != task_id {
            return Err("Run event belongs to another task".into());
        }
        record.redact_for_storage();
        run_json(&record)?;
        let connection = self.connection.as_mut().ok_or("Storage is unavailable")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let mut stored = stored_run(&transaction, task_id, &record.prepared.run_id)?;
        if stored.record.prepared != record.prepared {
            return Err("Run event correlation or approved inputs changed".into());
        }
        record.revision_moving |= stored.record.revision_moving;
        if !stored.reserved || !active_state(&stored.record.state) {
            return Err("Late run event cannot rewrite terminal journal evidence".into());
        }
        let transition = match stored.record.state {
            // Each notification carries a complete observation. A failed intermediate
            // commit must not strand the reservation when a later observed state arrives.
            RunState::AwaitingDispatch => record.state != RunState::AwaitingDispatch,
            RunState::Running => record.state != RunState::AwaitingDispatch,
            RunState::Cancelling => {
                !matches!(record.state, RunState::AwaitingDispatch | RunState::Running)
            }
            _ => false,
        };
        if !transition || record.captured_bytes > stored.capacity {
            return Err("Invalid run transition or capture exceeds its reservation".into());
        }
        if stored.record.started_at_ms.is_some()
            && stored.record.started_at_ms != record.started_at_ms
        {
            return Err("Run start metadata is immutable".into());
        }
        if record.captured_bytes < stored.record.captured_bytes
            || record.discarded_bytes < stored.record.discarded_bytes
            || record.dropped_live_bytes < stored.record.dropped_live_bytes
            || stored.record.cancelled && !record.cancelled
            || stored.record.truncated && !record.truncated
            || stored.record.output_unavailable && !record.output_unavailable
        {
            return Err("Run observation cannot erase previously committed capture or cancellation evidence".into());
        }
        stored.reserved = active_state(&record.state);
        if !stored.reserved {
            record.ended_at_ms.get_or_insert(timestamp()? as u64);
            super::work::release_command(&transaction, &stored.record.prepared)?;
            match SpoolDirectory::open(&self.data_dir)
                .and_then(|directory| directory.read(&stored.output_name, &stored.output_identity))
            {
                Ok(file) => {
                    if file.metadata().map_err(|error| error.to_string())?.len()
                        != record.captured_bytes
                    {
                        record.output_unavailable = true;
                    }
                }
                Err(_) => record.output_unavailable = true,
            }
        }
        stored.record = record;
        write_run(&transaction, &stored)?;
        if !stored.reserved && stored.record.started_at_ms.is_some() {
            super::records::record_run(&transaction, &self.project, &stored)?;
        }
        // Reservation includes bounded terminal metadata headroom. Do not evict evidence
        // or reject an honest terminal transition solely because the quota was lowered.
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(stored.record)
    }

    pub(super) fn load_runs(
        &self,
        task_id: i64,
    ) -> Result<Vec<crate::workspace::runner::RunRecord>, String> {
        let connection = self.connection.as_ref().ok_or("Storage is unavailable")?;
        let mut statement = connection.prepare(&format!("SELECT {RUN_COLUMNS} FROM executions WHERE task_id=?1 ORDER BY rowid DESC LIMIT 257")).map_err(|error| error.to_string())?;
        let stored = statement
            .query_map([task_id], stored_run_row)
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        if stored.len() > MAX_RUNS {
            return Err("Journal exceeds its bounded record capacity".into());
        }
        let directory = SpoolDirectory::open(&self.data_dir);
        Ok(stored
            .into_iter()
            .map(|mut stored| {
                if directory
                    .as_ref()
                    .ok()
                    .and_then(|directory| {
                        directory
                            .read(&stored.output_name, &stored.output_identity)
                            .ok()
                    })
                    .and_then(|file| file.metadata().ok())
                    .is_none_or(|metadata| {
                        !stored.reserved && metadata.len() != stored.record.captured_bytes
                    })
                {
                    stored.record.output_unavailable = true;
                }
                stored.record
            })
            .collect())
    }

    pub(super) fn read_output(
        &self,
        task_id: i64,
        run_id: &str,
        offset: u64,
        limit: usize,
    ) -> Result<OutputPage, String> {
        use std::io::{Read, Seek, SeekFrom};
        if limit == 0 || limit > 64 * 1024 {
            return Err("Output reads require a 1–65536-byte bound".into());
        }
        let connection = self.connection.as_ref().ok_or("Storage is unavailable")?;
        let stored = stored_run(connection, task_id, run_id)?;
        let unavailable = || OutputPage {
            offset,
            text: String::new(),
            total_bytes: stored.record.captured_bytes,
            unavailable: true,
            truncated: stored.record.truncated,
        };
        if stored.record.output_unavailable {
            return Ok(unavailable());
        }
        let Ok(mut file) = SpoolDirectory::open(&self.data_dir)
            .and_then(|directory| directory.read(&stored.output_name, &stored.output_identity))
        else {
            return Ok(unavailable());
        };
        let total = file.metadata().map_err(|error| error.to_string())?.len();
        if total > stored.capacity || (!stored.reserved && total != stored.record.captured_bytes) {
            return Ok(unavailable());
        }
        if offset > total {
            return Err("Output offset is beyond retained bytes".into());
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        let mut bytes = vec![0; limit.min((total - offset) as usize)];
        file.read_exact(&mut bytes)
            .map_err(|error| error.to_string())?;
        match std::str::from_utf8(&bytes) {
            Ok(_) => {}
            Err(error) if error.error_len().is_none() => bytes.truncate(error.valid_up_to()),
            Err(_) => {
                return Err("Output offset is not a UTF-8 boundary or capture is invalid".into());
            }
        }
        if bytes.is_empty() && offset < total {
            return Err("Output read bound is too small for the next UTF-8 character".into());
        }
        let text = String::from_utf8(bytes).map_err(|error| error.to_string())?;
        Ok(OutputPage {
            offset,
            text,
            total_bytes: total,
            unavailable: false,
            truncated: stored.record.truncated,
        })
    }

    pub(super) fn purge_output(
        &mut self,
        task_id: i64,
        run_id: &str,
    ) -> Result<(crate::workspace::runner::RunRecord, Option<String>), String> {
        let connection = self.connection.as_mut().ok_or("Storage is unavailable")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        no_active(&transaction, Some(task_id))?;
        let mut stored = stored_run(&transaction, task_id, run_id)?;
        stored.record.output_unavailable = true;
        write_run(&transaction, &stored)?;
        super::records::purge_run(&transaction, &self.project, task_id, run_id)?;
        transaction.commit().map_err(|error| error.to_string())?;
        // Metadata becomes unavailable before physical cleanup. A replaced link is
        // never followed, and a cleanup failure cannot resurrect successful evidence.
        let warning = SpoolDirectory::open(&self.data_dir)
            .and_then(|directory| directory.remove(&stored.output_name, &stored.output_identity))
            .err()
            .map(|error| {
                format!(
                    "Output metadata is committed unavailable, but physical cleanup failed: {error}"
                )
            });
        Ok((stored.record, warning))
    }
}

pub(super) fn reconcile_runs(connection: &mut Connection, data_dir: &Path) -> Result<(), String> {
    let needs_recovery: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM executions WHERE reserved=1) OR EXISTS(SELECT 1 FROM active_work)",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !needs_recovery {
        return Ok(());
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let mut records = {
        let mut statement = transaction
            .prepare(&format!(
                "SELECT {RUN_COLUMNS} FROM executions WHERE reserved=1"
            ))
            .map_err(|error| error.to_string())?;
        statement
            .query_map([], stored_run_row)
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?
    };
    for stored in &mut records {
        stored.reserved = false;
        stored.record.state = crate::workspace::runner::RunState::Interrupted;
        stored.record.tree = crate::workspace::runner::TreeOutcome::OwnershipUnknown;
        stored.record.ended_at_ms = Some(timestamp()? as u64);
        stored.record.error = Some("Eevee restarted without re-established process ownership; outcome is unknown and nothing was re-executed".into());
        stored.record.output_unavailable = true;
        stored.record.truncated = true;
        if let Ok(file) = SpoolDirectory::open(data_dir)
            .and_then(|directory| directory.read(&stored.output_name, &stored.output_identity))
        {
            let length = file.metadata().map_err(|error| error.to_string())?.len();
            if length <= stored.capacity {
                stored.record.captured_bytes = length;
                stored.record.output_unavailable = false;
            }
        }
        write_run(&transaction, stored)?;
    }
    // Recovery runs only under the exclusive store lock. Interrupt evidence and
    // discard stale ownership atomically; neither commands nor providers resume.
    transaction
        .execute("DELETE FROM active_work", [])
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

pub(super) fn evidence_usage(connection: &Connection, data_dir: &Path) -> Result<u64, String> {
    let mut total = live_bytes(connection)?;
    let mut sizes = std::collections::BTreeMap::new();
    for entry in fs::read_dir(data_dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with("ev-run-") || !name.ends_with(".output") {
            continue;
        }
        if sizes.len() >= 65_536 {
            return Err("Spool inventory exceeds the bounded store capacity".into());
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        // Account all owned-named retained files, including an interrupted pre-commit
        // orphan. Links are never followed to measure external target bytes.
        let size = if ensure_regular_metadata(&metadata).is_ok() {
            metadata.len()
        } else {
            0
        };
        total = total.saturating_add(size);
        sizes.insert(name.to_owned(), size);
    }
    let mut statement = connection
        .prepare("SELECT output_name,capacity FROM executions WHERE reserved=1")
        .map_err(|error| error.to_string())?;
    for row in statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })
        .map_err(|error| error.to_string())?
    {
        let (name, capacity) = row.map_err(|error| error.to_string())?;
        total = total
            .saturating_add(capacity.saturating_sub(sizes.get(&name).copied().unwrap_or(0)))
            .saturating_add(RUN_HEADROOM);
    }
    Ok(total)
}

pub(super) fn enforce_evidence_quota(
    connection: &Connection,
    data_dir: &Path,
    policy: Policy,
) -> Result<(), String> {
    if evidence_usage(connection, data_dir)? > policy.quota_bytes {
        Err("Local storage quota exceeded (live SQLite + retained output + committed reservations); nothing was evicted and no command was started".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod moving_tests {
    use super::*;
    use crate::workspace::runner::{
        CommandSpec, ExecutableIdentity, InputSnapshot, Prepared, RunRecord, RunState, TreeOutcome,
    };

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "eevee-moving-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            Self(fs::canonicalize(root).unwrap())
        }

        fn open(&self) -> State {
            let mut state = State {
                data_dir: self.0.clone(),
                project: "moving".into(),
                policy: Policy::default(),
                connection: None,
                lock: None,
            };
            assert!(matches!(state.reopen(), Event::Loaded(_)));
            state
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn saved_task(store: &mut State) -> Snapshot {
        let mut task = Snapshot::empty("moving".into());
        task.goal = "Preserve truthful overlap evidence".into();
        store.save(task).unwrap()
    }

    fn prepared(task: &Snapshot, root: &Path, id: &str, owner: Option<Attribution>) -> Prepared {
        let now = timestamp().unwrap() as u64;
        Prepared {
            request_id: format!("request-{id}"),
            run_id: format!("run-{id}"),
            task_id: task.id,
            spec: CommandSpec::Direct {
                program: "check".into(),
                args: vec!["exact-argument".into()],
            },
            cwd: root.to_path_buf(),
            cwd_identity: "fixture-directory".into(),
            executable: ExecutableIdentity {
                path: root.join("check"),
                sha256: "0".repeat(64),
                bytes: 1,
                modified_ns: 1,
                file_id: "fixture-file".into(),
            },
            prepared_at_ms: now,
            expires_at_ms: now + 60_000,
            approval_arrival: 0,
            before: InputSnapshot::default(),
            partner: owner,
            rerun_of: None,
        }
    }

    fn writer(task: &Snapshot, owner: Attribution) -> WorkClaim {
        WorkClaim {
            task_id: task.id,
            owner,
            request_id: "writer-request".into(),
            writing: true,
        }
    }

    fn acquire_writer(store: &mut State, task: &Snapshot, claim: &WorkClaim) {
        store
            .work_request(&WorkRequest::Acquire {
                claim: claim.clone(),
                task: Box::new(task.clone()),
            })
            .unwrap();
    }

    #[test]
    fn writer_before_command_marks_permit_and_stale_terminal_update_cannot_clear_it() {
        let fixture = Fixture::new();
        let mut store = fixture.open();
        let task = saved_task(&mut store);
        let roster = store.add_partner().unwrap();
        let claim = writer(&task, Attribution::of(&roster[1]));
        acquire_writer(&mut store, &task, &claim);
        assert!(no_active(store.connection.as_ref().unwrap(), Some(task.id)).is_err());
        let prepared = prepared(
            &task,
            &fixture.0,
            "during-write",
            Some(Attribution::of(&roster[0])),
        );
        let permit = store.reserve_run(task.id, prepared.clone(), 1024).unwrap();
        assert!(permit.revision_moving);
        let mut stale = RunRecord::awaiting(&prepared);
        stale.state = RunState::Running;
        stale.started_at_ms = Some(timestamp().unwrap() as u64);
        assert!(
            store
                .update_run(task.id, stale.clone())
                .unwrap()
                .revision_moving
        );
        stale.state = RunState::Succeeded;
        stale.ended_at_ms = Some(timestamp().unwrap() as u64);
        stale.exit_code = Some(0);
        stale.tree = TreeOutcome::OwnedTreeExited;
        let committed = store.update_run(task.id, stale).unwrap();
        assert!(committed.revision_moving);
        assert_eq!(committed.prepared, prepared);
        assert_eq!(
            super::super::work::claims(store.connection.as_ref().unwrap()).unwrap(),
            vec![claim.clone()]
        );
        store.work_request(&WorkRequest::Release(claim)).unwrap();
        drop(permit);
        drop(store);
        let reopened = fixture.open();
        assert!(reopened.load_runs(task.id).unwrap()[0].revision_moving);
    }

    #[test]
    fn writer_after_command_taints_stored_run_and_restart_preserves_it_and_clears_claims() {
        let fixture = Fixture::new();
        let mut store = fixture.open();
        let task = saved_task(&mut store);
        let roster = store.add_partner().unwrap();
        // Legacy unattributed reservations still receive a lead-owned claim.
        let prepared = prepared(&task, &fixture.0, "before-write", None);
        let permit = store.reserve_run(task.id, prepared.clone(), 1024).unwrap();
        assert!(!permit.revision_moving);
        let claim = writer(&task, Attribution::of(&roster[1]));
        acquire_writer(&mut store, &task, &claim);
        let mut stale = RunRecord::awaiting(&prepared);
        stale.state = RunState::Running;
        stale.started_at_ms = Some(timestamp().unwrap() as u64);
        assert!(store.update_run(task.id, stale).unwrap().revision_moving);
        drop(permit);
        drop(store);
        let reopened = fixture.open();
        let records = reopened.load_runs(task.id).unwrap();
        assert_eq!(records[0].state, RunState::Interrupted);
        assert!(records[0].revision_moving);
        assert!(
            super::super::work::claims(reopened.connection.as_ref().unwrap())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn distinct_commands_share_task_and_abandon_releases_only_correlated_owner() {
        let fixture = Fixture::new();
        let mut store = fixture.open();
        let task = saved_task(&mut store);
        let other_task = saved_task(&mut store);
        let roster = store.add_partner().unwrap();
        let first = prepared(&task, &fixture.0, "first", None);
        let second = prepared(
            &task,
            &fixture.0,
            "second",
            Some(Attribution::of(&roster[1])),
        );
        let first_permit = store.reserve_run(task.id, first.clone(), 1024).unwrap();
        let second_permit = store.reserve_run(task.id, second.clone(), 1024).unwrap();
        assert_eq!(
            super::super::work::claims(store.connection.as_ref().unwrap())
                .unwrap()
                .len(),
            2
        );
        let duplicate = prepared(
            &other_task,
            &fixture.0,
            "duplicate",
            Some(Attribution::of(&roster[0])),
        );
        assert!(store.reserve_run(other_task.id, duplicate, 1024).is_err());
        assert!(
            store
                .run_request(RunRequest {
                    task_id: task.id,
                    operation: RunOperation::Abandon {
                        request_id: second.request_id.clone(),
                        run_id: first.run_id.clone()
                    },
                })
                .is_err()
        );
        store
            .run_request(RunRequest {
                task_id: task.id,
                operation: RunOperation::Abandon {
                    request_id: first.request_id,
                    run_id: first.run_id,
                },
            })
            .unwrap();
        let claims = super::super::work::claims(store.connection.as_ref().unwrap()).unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].request_id, second.request_id);
        drop((first_permit, second_permit));
    }
}
