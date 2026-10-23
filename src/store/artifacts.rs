//! Existing single-writer artifact retention; no execution, discovery or permission path.
use super::*;
use crate::workspace::artifacts::{self as model, *};
pub use crate::workspace::artifacts::{Request, Response};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};

const COLUMNS: &str = "record,output_name,output_identity,run_id";
const RECORD_BYTES: usize = 16 * 1024;
struct Stored {
    record: Record,
    name: String,
    identity: String,
    run: Option<String>,
}
fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Stored> {
    Ok(Stored {
        record: json_column(row, 0, RECORD_BYTES)?,
        name: row.get(1)?,
        identity: row.get(2)?,
        run: row.get(3)?,
    })
}
pub(super) fn reserve_capacity(connection: &Connection, task: i64) -> Result<(), String> {
    let count:usize=connection.query_row("SELECT (SELECT count(*) FROM tool_artifacts WHERE task_id=?1)+(SELECT count(*) FROM executions WHERE task_id=?1 AND reserved=1)",[task],|r|r.get(0)).map_err(|e|e.to_string())?;
    if count >= MAX_ARTIFACTS {
        return Err("Artifact capacity reached; no command started and nothing evicted".into());
    }
    Ok(())
}
pub(super) fn migrate(connection: &Connection) -> Result<(), String> {
    let unexpected: bool = connection
        .query_row(
            r#"SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE tbl_name='executions' COLLATE NOCASE
               AND (type='trigger' OR (type='index' AND sql IS NOT NULL)))
           OR EXISTS(SELECT 1 FROM sqlite_master AS owner
               JOIN pragma_foreign_key_list(owner.name) AS dependency
               WHERE owner.type='table' AND dependency."table"='executions' COLLATE NOCASE)"#,
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if unexpected {
        return Err("Execution capacity migration refuses user-defined indexes, triggers or foreign-key dependents; no evidence was changed".into());
    }
    connection
        .execute_batch(
            "CREATE TABLE executions_v19(
        run_id TEXT PRIMARY KEY,task_id INTEGER NOT NULL,request_id TEXT NOT NULL UNIQUE,
        record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=131072),
        output_name TEXT NOT NULL UNIQUE,output_identity TEXT NOT NULL,
        capacity INTEGER NOT NULL CHECK(capacity>=1024 AND capacity<=16777216),
        reserved INTEGER NOT NULL CHECK(reserved IN (0,1)));
        INSERT INTO executions_v19(rowid,run_id,task_id,request_id,record,output_name,output_identity,capacity,reserved)
          SELECT rowid,run_id,task_id,request_id,record,output_name,output_identity,capacity,reserved FROM executions;
        DROP TABLE executions;
        ALTER TABLE executions_v19 RENAME TO executions;
        CREATE TABLE tool_artifacts(
        task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
        id TEXT PRIMARY KEY NOT NULL CHECK(length(id)=64),
        record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=16384),
        output_name TEXT NOT NULL,
        output_identity TEXT NOT NULL,
        run_id TEXT);",
        )
        .map_err(|e| e.to_string())
}
fn get(connection: &Connection, task: i64, handle: &Handle) -> Result<Stored, String> {
    let stored = connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM tool_artifacts WHERE task_id=?1 AND id=?2"),
            params![task, handle.id],
            row,
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or("Artifact is missing or belongs to another task")?;
    if stored.record.handle != *handle {
        return Err("Stale artifact handle; reload retained identities".into());
    }
    if stored.record.unavailable {
        return Err("Artifact was purged or its bytes are unavailable".into());
    }
    Ok(stored)
}
fn insert(connection: &Connection, stored: &Stored) -> Result<(), String> {
    let record = serde_json::to_string(&stored.record).map_err(|e| e.to_string())?;
    if record.len() > RECORD_BYTES {
        return Err("Artifact metadata exceeds its bound".into());
    }
    let count: usize = connection
        .query_row(
            "SELECT count(*) FROM tool_artifacts WHERE task_id=?1",
            [stored.record.task_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count >= MAX_ARTIFACTS {
        return Err("Artifact journal is full; nothing was evicted".into());
    }
    connection.execute("INSERT INTO tool_artifacts(task_id,id,record,output_name,output_identity,run_id) VALUES(?1,?2,?3,?4,?5,?6)", params![stored.record.task_id,stored.record.handle.id,record,stored.name,stored.identity,stored.run]).map_err(|e| e.to_string())?;
    Ok(())
}
fn file_hash(file: &mut File) -> Result<String, String> {
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn verified_file(directory: &SpoolDirectory, stored: &Stored) -> Result<File, String> {
    let mut file = directory.read(&stored.name, &stored.identity)?;
    if file.metadata().map_err(|e| e.to_string())?.len() != stored.record.captured_bytes
        || file_hash(&mut file)? != stored.record.sha256
    {
        return Err("Artifact bytes changed; stale retained evidence is refused".into());
    }
    Ok(file)
}
fn page(file: &mut File, record: &Record, offset: u64, limit: usize) -> Result<Slice, String> {
    if limit == 0 || limit > PAGE_BYTES || offset > record.captured_bytes {
        return Err("Artifact range requires an existing byte offset and 1–65536 bytes".into());
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let mut bytes = vec![0; limit.min((record.captured_bytes - offset) as usize)];
    file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    match std::str::from_utf8(&bytes) {
        Ok(_) => {}
        Err(error) if error.error_len().is_none() => bytes.truncate(error.valid_up_to()),
        Err(_) => return Err("Artifact offset is not a UTF-8 boundary".into()),
    }
    if bytes.is_empty() && offset < record.captured_bytes {
        return Err("Artifact bound cannot fit the next UTF-8 codepoint".into());
    }
    let end = offset + bytes.len() as u64;
    Ok(Slice {
        handle: record.handle.clone(),
        offset,
        text: String::from_utf8(bytes).map_err(|e| e.to_string())?,
        next_offset: (end < record.captured_bytes).then_some(end),
    })
}
fn search(
    file: &mut File,
    record: &Record,
    query: String,
    offset: u64,
    limit: usize,
) -> Result<SearchResult, String> {
    if query.is_empty() || query.len() > 1024 || limit == 0 || limit > MAX_MATCHES {
        return Err("Literal artifact search requires 1–1024 query bytes and 1–32 slices".into());
    }
    if offset > record.captured_bytes {
        return Err("Artifact search offset exceeds captured bytes".into());
    }
    let mut cursor = offset;
    let mut slices = Vec::new();
    let mut scanned = 0;
    while cursor < record.captured_bytes {
        let window = page(file, record, cursor, PAGE_BYTES)?;
        let overlap = query.len().saturating_sub(1);
        let mut advance = window.text.len();
        if window.next_offset.is_some() {
            advance = advance.saturating_sub(overlap);
            while !window.text.is_char_boundary(advance) {
                advance -= 1;
            }
        }
        for (index, _) in window.text.match_indices(&query) {
            // The overlapped tail is searched only with the next complete window.
            if window.next_offset.is_some() && index >= advance {
                break;
            }
            let mut start = index.saturating_sub(160);
            while !window.text.is_char_boundary(start) {
                start += 1;
            }
            let mut end = (index + query.len() + 240).min(window.text.len());
            while !window.text.is_char_boundary(end) {
                end -= 1;
            }
            let slice_offset = cursor + start as u64;
            let slice_end = cursor + end as u64;
            slices.push(Slice {
                handle: record.handle.clone(),
                offset: slice_offset,
                text: window.text[start..end].into(),
                next_offset: (slice_end < record.captured_bytes).then_some(slice_end),
            });
            if slices.len() == limit {
                let next = cursor + index as u64 + query.len() as u64;
                return Ok(SearchResult {
                    handle: record.handle.clone(),
                    query,
                    slices,
                    next_offset: (next < record.captured_bytes).then_some(next),
                    scanned_bytes: next.saturating_sub(offset),
                    omitted_bytes: record.omitted_bytes,
                });
            }
        }
        cursor += advance as u64;
        scanned += advance as u64;
    }
    Ok(SearchResult {
        handle: record.handle.clone(),
        query,
        slices,
        next_offset: None,
        scanned_bytes: scanned,
        omitted_bytes: record.omitted_bytes,
    })
}

impl State {
    pub(super) fn artifact_request(&mut self, request: &Request) -> Result<Response, String> {
        let connection = self.connection.as_ref().ok_or("Storage is unavailable")?;
        let task = load_task(connection, &self.project, request.task_id)?
            .ok_or("Artifact task is missing or belongs to another project")?;
        if task.redacted {
            return Err("Artifact task was redacted".into());
        }
        match &request.operation {
            Operation::Load => {
                let mut statement = connection.prepare(&format!("SELECT {COLUMNS} FROM tool_artifacts WHERE task_id=?1 ORDER BY rowid DESC LIMIT 257")).map_err(|e| e.to_string())?;
                let stored = statement
                    .query_map([task.id], row)
                    .map_err(|e| e.to_string())?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|e| e.to_string())?;
                if stored.len() > MAX_ARTIFACTS {
                    return Err("Artifact journal exceeds capacity".into());
                }
                Ok(Response::Records(
                    stored.into_iter().map(|s| s.record).collect(),
                ))
            }
            Operation::RetainTool(capture) => {
                self.retain_tool(task.id, capture).map(Response::Retained)
            }
            Operation::Purge { handle } => {
                no_active(connection, Some(task.id))?;
                let mut stored = get(connection, task.id, handle)?;
                if let Some(run) = &stored.run {
                    let (_, warning) = self.purge_output(task.id, run)?;
                    return Ok(Response::Purged {
                        handle: handle.clone(),
                        warning,
                    });
                }
                let transaction = self
                    .connection
                    .as_mut()
                    .ok_or("Storage is unavailable")?
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|e| e.to_string())?;
                stored.record.unavailable = true;
                transaction
                    .execute(
                        "UPDATE tool_artifacts SET record=?1 WHERE id=?2 AND task_id=?3",
                        params![
                            serde_json::to_string(&stored.record).map_err(|e| e.to_string())?,
                            handle.id,
                            task.id
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                super::receipts::purge_sources(&transaction, task.id, None)?;
                transaction.commit().map_err(|e| e.to_string())?;
                let warning = SpoolDirectory::open(&self.data_dir)
                    .and_then(|d| d.remove(&stored.name, &stored.identity))
                    .err()
                    .map(|e| format!("Artifact purge committed; physical cleanup failed: {e}"));
                Ok(Response::Purged {
                    handle: handle.clone(),
                    warning,
                })
            }
            operation => {
                let handle = match &operation {
                    Operation::Read { handle, .. }
                    | Operation::Search { handle, .. }
                    | Operation::Locations { handle, .. } => handle,
                    _ => unreachable!(),
                };
                let stored = get(connection, task.id, handle)?;
                let directory = SpoolDirectory::open(&self.data_dir)?;
                let mut file = verified_file(&directory, &stored)?;
                match operation {
                    Operation::Read { offset, limit, .. } => {
                        page(&mut file, &stored.record, *offset, *limit).map(Response::Page)
                    }
                    Operation::Search {
                        query,
                        offset,
                        limit,
                        ..
                    } => search(&mut file, &stored.record, query.clone(), *offset, *limit)
                        .map(Response::Search),
                    Operation::Locations { offset, .. } => {
                        let slice = page(&mut file, &stored.record, *offset, PAGE_BYTES)?;
                        let mut located = Vec::new();
                        let mut at = slice.offset;
                        for line in slice.text.split_inclusive('\n') {
                            for location in crate::workspace::locations::parse([line]) {
                                located.push(LocatedSlice {
                                    location,
                                    slice: Slice {
                                        handle: slice.handle.clone(),
                                        offset: at,
                                        text: line.into(),
                                        next_offset: Some(at + line.len() as u64)
                                            .filter(|end| *end < stored.record.captured_bytes),
                                    },
                                });
                                if located.len() == crate::workspace::locations::MAX_LOCATIONS {
                                    break;
                                }
                            }
                            at += line.len() as u64;
                            if located.len() == crate::workspace::locations::MAX_LOCATIONS {
                                break;
                            }
                        }
                        // Continue at the last complete line so a boundary-spanning location is not lost.
                        let next_offset =
                            if located.len() == crate::workspace::locations::MAX_LOCATIONS {
                                (at < stored.record.captured_bytes).then_some(at)
                            } else {
                                slice.next_offset.map(|next| {
                                    slice
                                        .text
                                        .rfind('\n')
                                        .map_or(next, |i| slice.offset + i as u64 + 1)
                                })
                            };
                        Ok(Response::Locations {
                            slices: located,
                            next_offset,
                        })
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
    fn retain_tool(&mut self, task: i64, capture: &ToolCapture) -> Result<Record, String> {
        for value in [&capture.request_id, &capture.call_id, &capture.tool] {
            if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
                return Err("Invalid tool artifact producer identity".into());
            }
        }
        if let Some(partner) = &capture.partner {
            partner.validate()?;
        }
        let origin = Origin::OwnedTool {
            call_id: capture.call_id.clone(),
            tool: capture.tool.clone(),
        };
        let id = model::identity(task, &capture.request_id, &origin);
        let text = crate::workspace::privacy::redact(&capture.text);
        let observation_sha256 = model::digest(text.as_bytes());
        let connection = self.connection.as_mut().ok_or("Storage is unavailable")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let old: Option<Stored> = transaction
            .query_row(
                &format!("SELECT {COLUMNS} FROM tool_artifacts WHERE id=?1 AND task_id=?2"),
                params![id, task],
                row,
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(old) = old {
            if old.record.unavailable {
                return Err("Purged tool identity cannot be republished".into());
            }
            if old.record.total_bytes != text.len() as u64
                || old
                    .record
                    .observation_sha256
                    .as_ref()
                    .unwrap_or(&old.record.sha256)
                    != &observation_sha256
                || old.record.partner != capture.partner
                || (old.record.exit.status == "failed") != capture.failed
            {
                return Err("Immutable tool artifact identity cannot be rewritten".into());
            }
            return Ok(old.record);
        }
        reserve_capacity(&transaction, task)?;
        let usage = evidence_usage(&transaction, &self.data_dir)?;
        let available = self
            .policy
            .quota_bytes
            .saturating_sub(usage)
            .saturating_sub(RUN_HEADROOM)
            .min(MAX_CAPTURE_BYTES as u64) as usize;
        if usage.saturating_add(RUN_HEADROOM) > self.policy.quota_bytes {
            return Err("Artifact metadata quota exceeded; no bytes retained or evicted".into());
        }
        let retained = model::utf8_prefix(&text, available);
        let (sha256, observation_sha256) = if retained.len() == text.len() {
            (observation_sha256, None)
        } else {
            (model::digest(retained.as_bytes()), Some(observation_sha256))
        };
        let name = format!("ev-run-artifact-{id}.output");
        let directory = SpoolDirectory::open(&self.data_dir)?;
        let mut file = directory.create(&name)?;
        let identity = file_identity(&file)?;
        let mut record = Record {
            handle: Handle {
                id,
                revision: String::new(),
            },
            task_id: task,
            request_id: capture.request_id.clone(),
            partner: capture.partner.clone(),
            origin,
            created_at_ms: timestamp()? as u64,
            exit: ExitSummary {
                status: if capture.failed {
                    "failed"
                } else {
                    "completed"
                }
                .into(),
                exit_code: None,
                signal: None,
                error: None,
            },
            total_bytes: text.len() as u64,
            captured_bytes: retained.len() as u64,
            omitted_bytes: (text.len() - retained.len()) as u64,
            sha256,
            observation_sha256,
            unavailable: false,
        };
        model::seal(&mut record);
        let result = (|| {
            file.write_all(retained.as_bytes())
                .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            directory.sync()?;
            insert(
                &transaction,
                &Stored {
                    record: record.clone(),
                    name: name.clone(),
                    identity: identity.clone(),
                    run: None,
                },
            )?;
            enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
            transaction.commit().map_err(|e| e.to_string())
        })();
        if let Err(error) = result {
            let _ = directory.remove(&name, &identity);
            return Err(error);
        }
        Ok(record)
    }
}

/// Publish once in the terminal run journal transaction; retain the existing spool, not a copy.
pub(super) fn publish_run(
    connection: &Connection,
    data_dir: &Path,
    stored: &super::runs::StoredRun,
) -> Result<(), String> {
    let run = &stored.record;
    let origin = Origin::OwnedRun {
        run_id: run.prepared.run_id.clone(),
    };
    let id = model::identity(run.prepared.task_id, &run.prepared.request_id, &origin);
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM tool_artifacts WHERE id=?1)",
            [&id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists {
        return Ok(());
    }
    let hash = if run.output_unavailable {
        None
    } else {
        SpoolDirectory::open(data_dir)
            .and_then(|directory| directory.read(&stored.output_name, &stored.output_identity))
            .and_then(|mut file| file_hash(&mut file))
            .ok()
    };
    // A spool disappearing during terminal settlement must not suppress the actual exit journal.
    let unavailable = hash.is_none();
    let sha256 = hash.unwrap_or_default();
    let mut record = Record {
        handle: Handle {
            id,
            revision: String::new(),
        },
        task_id: run.prepared.task_id,
        request_id: run.prepared.request_id.clone(),
        partner: run.prepared.partner.clone(),
        origin,
        created_at_ms: run.ended_at_ms.unwrap_or(0),
        exit: ExitSummary {
            status: format!("{:?}", run.state),
            exit_code: run.exit_code,
            signal: run.signal,
            error: run.error.clone(),
        },
        total_bytes: run.captured_bytes.saturating_add(run.discarded_bytes),
        captured_bytes: run.captured_bytes,
        omitted_bytes: run.discarded_bytes,
        sha256,
        observation_sha256: None,
        unavailable,
    };
    model::seal(&mut record);
    insert(
        connection,
        &Stored {
            record,
            name: stored.output_name.clone(),
            identity: stored.output_identity.clone(),
            run: Some(run.prepared.run_id.clone()),
        },
    )
}
pub(super) fn purge_run(connection: &Connection, task: i64, run: &str) -> Result<(), String> {
    connection.execute("UPDATE tool_artifacts SET record=json_set(record,'$.unavailable',json('true')) WHERE task_id=?1 AND run_id=?2",params![task,run]).map_err(|e|e.to_string())?;
    Ok(())
}
pub(super) fn deletion_outputs(
    connection: &Connection,
    task: i64,
) -> Result<Vec<(String, String)>, String> {
    let mut statement=connection.prepare("SELECT output_name,output_identity FROM tool_artifacts WHERE task_id=?1 AND run_id IS NULL").map_err(|e|e.to_string())?;
    statement
        .query_map([task], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}
pub(super) fn inspect(connection: &Connection) -> Result<(), String> {
    inspect_columns(
        connection,
        "tool_artifacts",
        &[
            ("task_id", "INTEGER", true, 0),
            ("id", "TEXT", true, 1),
            ("record", "TEXT", true, 0),
            ("output_name", "TEXT", true, 0),
            ("output_identity", "TEXT", true, 0),
            ("run_id", "TEXT", false, 0),
        ],
    )?;
    let orphan:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM tool_artifacts a LEFT JOIN tasks t ON t.id=a.task_id WHERE t.id IS NULL)",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if orphan {
        return Err("Artifact owner task is missing".into());
    }
    let mut statement = connection
        .prepare(&format!("SELECT task_id,id,{COLUMNS} FROM tool_artifacts"))
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                Stored {
                    record: json_column(r, 2, RECORD_BYTES)?,
                    name: r.get(3)?,
                    identity: r.get(4)?,
                    run: r.get(5)?,
                },
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut counts = std::collections::BTreeMap::new();
    for entry in rows {
        let (task, id, stored) = entry.map_err(|e| e.to_string())?;
        let record = &stored.record;
        validate_spool_name(&stored.name)?;
        if record.task_id != task
            || record.handle.id != id
            || id != model::identity(task, &record.request_id, &record.origin)
            || record.captured_bytes > MAX_CAPTURE_BYTES as u64
            || record.total_bytes != record.captured_bytes.saturating_add(record.omitted_bytes)
            || !(record.sha256.len() == 64 || record.unavailable && record.sha256.is_empty())
            || record.handle.revision.len() != 64
        {
            return Err("Invalid immutable tool artifact metadata".into());
        }
        let expected_name = match &record.origin {
            Origin::OwnedRun { run_id } if stored.run.as_ref() == Some(run_id) => {
                spool_name(run_id)?
            }
            Origin::OwnedTool { call_id, tool }
                if stored.run.is_none()
                    && !call_id.is_empty()
                    && call_id.len() <= 512
                    && !tool.is_empty()
                    && tool.len() <= 512 =>
            {
                format!("ev-run-artifact-{id}.output")
            }
            _ => return Err("Artifact origin and retained source disagree".into()),
        };
        if stored.name != expected_name
            || stored.identity.is_empty()
            || stored.identity.len() > 256
            || record.request_id.is_empty()
            || record.request_id.len() > 512
            || !record.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("Invalid artifact source identity".into());
        }
        if let Some(digest) = &record.observation_sha256
            && (digest.len() != 64
                || !digest.bytes().all(|b| b.is_ascii_hexdigit())
                || !matches!(record.origin, Origin::OwnedTool { .. })
                || record.omitted_bytes == 0)
        {
            return Err("Invalid full tool observation digest".into());
        }
        let mut sealed = record.clone();
        model::seal(&mut sealed);
        if sealed.handle.revision != record.handle.revision {
            if !record.unavailable {
                return Err("Artifact revision does not seal its metadata".into());
            }
            sealed = record.clone();
            sealed.unavailable = false;
            model::seal(&mut sealed);
            if sealed.handle.revision != record.handle.revision {
                return Err("Purged artifact revision is invalid".into());
            }
        }
        let count = counts.entry(task).or_insert(0usize);
        *count += 1;
        if *count > MAX_ARTIFACTS {
            return Err("Artifact task exceeds capacity".into());
        }
        if let Some(partner) = &record.partner {
            partner.validate()?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "artifacts_tests.rs"]
mod tests;
