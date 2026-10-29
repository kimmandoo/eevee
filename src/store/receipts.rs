//! Durable outgoing receipts through the existing single SQLite writer.
use super::*;
use crate::workspace::receipt::{Durable, Outgoing};
use sha2::{Digest, Sha256};

pub const MAX_RECORDS: usize = 256;
const MAX_RECORD_BYTES: usize = 65536;
const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;

/// Admission and insertion share the existing immediate work transaction.
/// Unrecorded live provider claims reserve a slot; once their receipt commits
/// the same identity is counted exactly once, until work release.
pub(super) fn reserve_capacity(connection: &Connection, task_id: i64) -> Result<(), String> {
    let occupied: i64 = connection
        .query_row(
            "SELECT (SELECT count(*) FROM outgoing_receipts WHERE task_id=?1)
         + (SELECT count(*) FROM active_work w WHERE w.task_id=?1 AND w.kind='provider'
            AND NOT EXISTS(SELECT 1 FROM outgoing_receipts r WHERE r.request_id=w.request_id))",
            [task_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if occupied >= MAX_RECORDS as i64 {
        return Err("Outgoing receipt capacity reached; no provider request was admitted. Delete this task's records explicitly before sending again".into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub task_id: i64,
    pub partner: i64,
    pub operation: Operation,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Put(Box<Outgoing>),
    List,
    ReadSource { request_id: String },
    PurgeSources { request_id: Option<String> },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub record: Durable,
    pub source_available: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    Saved(Box<View>),
    Records(Vec<View>),
    Source { request_id: String, text: String },
    Purged { request_id: Option<String> },
}

pub(super) fn migrate(connection: &Connection) -> Result<(), String> {
    connection.execute_batch(
        "CREATE TABLE outgoing_receipts(
         request_id TEXT PRIMARY KEY NOT NULL CHECK(length(CAST(request_id AS BLOB)) BETWEEN 1 AND 128),
         task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
         partner INTEGER NOT NULL CHECK(partner>0),
         record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=65536),
         source TEXT CHECK(source IS NULL OR length(CAST(source AS BLOB))<=8388608));"
    ).map_err(|error| format!("Cannot migrate outgoing receipts: {error}"))
}

fn validate_source(record: &Durable, source: Option<&str>) -> Result<(), String> {
    record.validate()?;
    if record.image.is_some() && source.is_some() {
        return Err("Screenshot bytes must not be duplicated in outgoing source history".into());
    }
    if let Some(source) = source {
        if source.len() > MAX_SOURCE_BYTES
            || source.len() != record.bytes
            || <[u8; 32]>::from(Sha256::digest(source.as_bytes())) != record.sha256
            || crate::workspace::privacy::redact(source) != source
        {
            return Err(
                "Outgoing source does not match its immutable receipt or privacy boundary".into(),
            );
        }
        let measured = crate::workspace::receipt::receipt(source);
        if measured.bytes != record.bytes
            || measured.tokens != record.tokens
            || measured.envelope_bytes != record.envelope_bytes
            || measured.deduplicated != record.deduplicated
            || measured.image != record.image
            || measured.sections.len() != record.sections.len()
            || measured
                .sections
                .iter()
                .zip(&record.sections)
                .any(|(a, b)| {
                    a.name != b.name
                        || a.keys != b.keys
                        || a.bytes != b.bytes
                        || a.tokens != b.tokens
                })
        {
            return Err("Outgoing source section counts do not match the receipt".into());
        }
        let wire = crate::workspace::context::wire_receipt(source)?;
        if wire.captures.len() != record.captures.len()
            || wire
                .captures
                .iter()
                .zip(&record.captures)
                .any(|(a, b)| a.id != b.id || a.bytes != b.bytes || a.estimated_tokens != b.tokens)
        {
            return Err("Outgoing source capture references do not match the receipt".into());
        }
    }
    Ok(())
}

/// Strict row inspection participates in startup/migration admission. The parent
/// schema inspector also checks the table's exact columns and constraints.
pub(super) fn inspect(connection: &Connection) -> Result<(), String> {
    inspect_columns(
        connection,
        "outgoing_receipts",
        &[
            ("request_id", "TEXT", true, 1),
            ("task_id", "INTEGER", true, 0),
            ("partner", "INTEGER", true, 0),
            ("record", "TEXT", true, 0),
            ("source", "TEXT", false, 0),
        ],
    )?;
    let mut statement = connection.prepare("SELECT r.request_id,r.task_id,r.partner,r.record,r.source,t.id FROM outgoing_receipts r LEFT JOIN tasks t ON t.id=r.task_id ORDER BY r.rowid").map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    let mut counts = std::collections::BTreeMap::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let id: String = row.get(0).map_err(|e| e.to_string())?;
        let task: i64 = row.get(1).map_err(|e| e.to_string())?;
        let partner: i64 = row.get(2).map_err(|e| e.to_string())?;
        let record: Durable = json_column(row, 3, MAX_RECORD_BYTES).map_err(|e| e.to_string())?;
        let source: Option<String> = row.get(4).map_err(|e| e.to_string())?;
        let owner_task: Option<i64> = row.get(5).map_err(|e| e.to_string())?;
        if id != record.request_id
            || task != record.task_id
            || partner != record.owner.partner
            || owner_task != Some(task)
        {
            return Err("Outgoing receipt ownership or task reference is invalid".into());
        }
        let count = counts.entry(task).or_insert(0usize);
        *count += 1;
        if *count > MAX_RECORDS {
            return Err("Outgoing receipt task capacity exceeded".into());
        }
        validate_source(&record, source.as_deref())?;
    }
    Ok(())
}

fn records(connection: &Connection, task_id: i64, partner: i64) -> Result<Vec<View>, String> {
    let mut statement = connection.prepare("SELECT record,source IS NOT NULL FROM outgoing_receipts WHERE task_id=?1 AND partner=?2 ORDER BY rowid DESC LIMIT 257").map_err(|e| e.to_string())?;
    let records = statement
        .query_map(params![task_id, partner], |row| {
            Ok(View {
                record: json_column(row, 0, MAX_RECORD_BYTES)?,
                source_available: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if records.len() > MAX_RECORDS {
        return Err("Outgoing receipt task capacity exceeded".into());
    }
    for view in &records {
        view.record.validate()?;
        if view.record.task_id != task_id || view.record.owner.partner != partner {
            return Err("Foreign outgoing receipt refused".into());
        }
    }
    Ok(records)
}

/// Source purge deliberately removes every outgoing copy for the task. Capture
/// aliases, native string escaping, memories and tool references can overlap;
/// conservatively clearing the whole source is safer than retaining a hidden copy.
/// Identity, source IDs, hashes and exact historical counts remain unchanged.
pub(super) fn purge_sources(
    connection: &Connection,
    task_id: i64,
    _capture_ids: Option<&[i64]>,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE outgoing_receipts SET source=NULL WHERE task_id=?1",
            [task_id],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

impl State {
    pub(super) fn receipts_request(&mut self, request: &Request) -> Result<Response, String> {
        if request.task_id <= 0 || request.partner <= 0 {
            return Err("Outgoing receipt requires an exact task and partner".into());
        }
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage unavailable; outgoing receipt was not saved")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let task = load_task(&transaction, &self.project, request.task_id)?
            .ok_or("Outgoing receipt task is missing or belongs to another project")?;
        let response = match &request.operation {
            Operation::Put(outgoing) => {
                let Outgoing { record, source } = outgoing.as_ref();
                validate_source(record, source.as_deref())?;
                if record.task_id != request.task_id || record.owner.partner != request.partner {
                    return Err("Wrong-owner outgoing receipt refused".into());
                }
                let json = serde_json::to_string(&record).map_err(|e| e.to_string())?;
                if json.len() > MAX_RECORD_BYTES {
                    return Err("Outgoing receipt metadata exceeds 64 KiB".into());
                }
                let existing: Option<(i64,i64,String,bool)> = transaction.query_row("SELECT task_id,partner,record,source IS NOT NULL FROM outgoing_receipts WHERE request_id=?1",[&record.request_id],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(|e| e.to_string())?;
                let available = if let Some((owner, partner, saved, available)) = existing {
                    let mut saved_record: Durable =
                        serde_json::from_str(&saved).map_err(|e| e.to_string())?;
                    let newly_submitted = !saved_record.submitted && record.submitted;
                    if newly_submitted {
                        saved_record.set_submitted(true);
                    }
                    saved_record
                        .child_request_ids
                        .clone_from(&record.child_request_ids);
                    if owner != request.task_id
                        || partner != request.partner
                        || saved_record != *record
                    {
                        return Err(
                            "Outgoing request identity cannot overwrite different owner or content"
                                .into(),
                        );
                    }
                    if newly_submitted {
                        transaction
                            .execute(
                                "UPDATE outgoing_receipts SET record=?1 WHERE request_id=?2",
                                params![json, record.request_id],
                            )
                            .map_err(|e| e.to_string())?;
                    }
                    // Idempotent delayed retry is permitted, but NEVER resurrects source bytes.
                    available
                } else {
                    if !record.child_request_ids.is_empty() {
                        return Err(
                            "Only actual dispatched child receipts may create parent links".into(),
                        );
                    }
                    if task.archived {
                        return Err("Archived task cannot accept outgoing receipts".into());
                    }
                    if let Some(parent_id) = &record.parent_request_id {
                        let parent: Durable = transaction.query_row(
                            "SELECT record FROM outgoing_receipts WHERE request_id=?1 AND task_id=?2 AND partner=?3",
                            params![parent_id, record.task_id, record.owner.partner],
                            |row| json_column(row, 0, MAX_RECORD_BYTES),
                        ).map_err(|e| format!("Delegation parent receipt is unavailable: {e}"))?;
                        if parent.parent_request_id.is_some()
                            || parent.owner != record.owner
                            || parent.agent != record.agent
                            || record.operation != crate::providers::Operation::Plan
                        {
                            return Err(
                                "Delegation child cannot borrow this receipt's ownership".into()
                            );
                        }
                        super::work::validate_live(&transaction, &parent.claim())?;
                    } else {
                        super::work::validate_live(&transaction, &record.claim())?;
                    }
                    let count: i64 = transaction
                        .query_row(
                            "SELECT (SELECT count(*) FROM outgoing_receipts WHERE task_id=?1)
                             + (SELECT count(*) FROM active_work w WHERE w.task_id=?1 AND w.kind='provider'
                                AND w.request_id!=?2
                                AND NOT EXISTS(SELECT 1 FROM outgoing_receipts r WHERE r.request_id=w.request_id))",
                            params![request.task_id, record.request_id],
                            |r| r.get(0),
                        )
                        .map_err(|e| e.to_string())?;
                    if count >= MAX_RECORDS as i64 {
                        return Err("Outgoing receipt capacity reached; delete task records explicitly before capturing more".into());
                    }
                    let available = source.is_some();
                    transaction.execute("INSERT INTO outgoing_receipts(request_id,task_id,partner,record,source) VALUES(?1,?2,?3,?4,?5)",params![record.request_id,record.task_id,record.owner.partner,json,source]).map_err(|e| format!("Outgoing receipt was not saved: {e}"))?;
                    available
                };
                if record.submitted
                    && let Some(parent_id) = &record.parent_request_id
                {
                    let mut parent: Durable = transaction
                        .query_row(
                            "SELECT record FROM outgoing_receipts WHERE request_id=?1",
                            [parent_id],
                            |row| json_column(row, 0, MAX_RECORD_BYTES),
                        )
                        .map_err(|e| e.to_string())?;
                    if !parent.child_request_ids.contains(&record.request_id) {
                        super::work::validate_live(&transaction, &parent.claim())?;
                        parent.child_request_ids.push(record.request_id.clone());
                        parent.child_request_ids.sort();
                        parent.validate()?;
                        let linked = serde_json::to_string(&parent).map_err(|e| e.to_string())?;
                        if linked.len() > MAX_RECORD_BYTES {
                            return Err("Delegation parent receipt metadata exceeds 64 KiB".into());
                        }
                        transaction
                            .execute(
                                "UPDATE outgoing_receipts SET record=?1 WHERE request_id=?2",
                                params![linked, parent_id],
                            )
                            .map_err(|e| e.to_string())?;
                    }
                }
                enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
                Response::Saved(Box::new(View {
                    record: record.clone(),
                    source_available: available,
                }))
            }
            Operation::List => {
                Response::Records(records(&transaction, request.task_id, request.partner)?)
            }
            Operation::ReadSource { request_id } => {
                let (owner,partner,record,source): (i64,i64,String,Option<String>) = transaction.query_row("SELECT task_id,partner,record,source FROM outgoing_receipts WHERE request_id=?1",[request_id],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(|e| e.to_string())?.ok_or("Outgoing receipt is missing")?;
                if owner != request.task_id || partner != request.partner {
                    return Err("Wrong-owner outgoing source refused".into());
                }
                let record: Durable = serde_json::from_str(&record).map_err(|e| e.to_string())?;
                let text = source.ok_or("Outgoing source bytes are unavailable (purged or privacy-omitted); counts remain")?;
                validate_source(&record, Some(&text))?;
                Response::Source {
                    request_id: request_id.clone(),
                    text,
                }
            }
            Operation::PurgeSources { request_id } => {
                no_active(&transaction, Some(task.id))?;
                if let Some(id) = request_id {
                    let owner: Option<(i64, i64)> = transaction
                        .query_row(
                            "SELECT task_id,partner FROM outgoing_receipts WHERE request_id=?1",
                            [id],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()
                        .map_err(|e| e.to_string())?;
                    if owner != Some((request.task_id, request.partner)) {
                        return Err("Wrong-owner or missing outgoing receipt purge refused".into());
                    }
                    transaction
                        .execute(
                            "UPDATE outgoing_receipts SET source=NULL WHERE request_id=?1",
                            [id],
                        )
                        .map_err(|e| e.to_string())?;
                } else {
                    transaction.execute("UPDATE outgoing_receipts SET source=NULL WHERE task_id=?1 AND partner=?2",params![request.task_id,request.partner]).map_err(|e| e.to_string())?;
                }
                Response::Purged {
                    request_id: request_id.clone(),
                }
            }
        };
        transaction.commit().map_err(|error| {
            format!("Outgoing receipt commit failed; persistence is unconfirmed: {error}")
        })?;
        Ok(response)
    }
}

#[cfg(test)]
#[path = "receipts_tests.rs"]
mod tests;
