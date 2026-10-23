//! Content/configuration revision index. No source bytes, provider retention or billing claims.
use super::*;
use crate::workspace::context::optimizer::{
    CacheRevision, MAX_ASSEMBLY_METADATA_BYTES, validate_assembly_metadata,
};
#[derive(Clone, Debug)]
pub struct Request {
    pub task_id: i64,
    pub operation: Operation,
}
#[derive(Clone, Debug)]
pub enum Operation {
    Load,
    Save(Vec<CacheRevision>),
    Purge,
}
#[derive(Clone, Debug)]
pub struct Response {
    pub rows: Vec<CacheRevision>,
}
pub fn migrate(connection: &rusqlite::Connection) -> Result<(), String> {
    connection.execute_batch("CREATE TABLE context_revisions(task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,source_key TEXT NOT NULL CHECK(length(CAST(source_key AS BLOB))<=65536),source_sha256 TEXT NOT NULL CHECK(length(source_sha256)=64),policy TEXT NOT NULL CHECK(length(policy)<=256),tokenizer TEXT NOT NULL CHECK(length(tokenizer)<=256),tokens INTEGER NOT NULL CHECK(tokens>=0),assembly TEXT CHECK(assembly IS NULL OR length(CAST(assembly AS BLOB))<=4194304),PRIMARY KEY(task_id,source_key));").map_err(|e|format!("Cannot migrate context revision index: {e}"))
}
pub fn inspect(connection: &rusqlite::Connection) -> Result<(), String> {
    let columns = {
        let mut statement = connection
            .prepare("PRAGMA table_info(context_revisions)")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    let expected = [
        ("task_id", "INTEGER", 1, 1),
        ("source_key", "TEXT", 1, 2),
        ("source_sha256", "TEXT", 1, 0),
        ("policy", "TEXT", 1, 0),
        ("tokenizer", "TEXT", 1, 0),
        ("tokens", "INTEGER", 1, 0),
        ("assembly", "TEXT", 0, 0),
    ];
    if columns.len() != expected.len()
        || columns.iter().zip(expected).any(|(actual, expected)| {
            actual.0 != expected.0
                || actual.1 != expected.1
                || actual.2 != expected.2
                || actual.3 != expected.3
        })
    {
        return Err("Unsupported context revision schema; refusing to open".into());
    }
    let orphan:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM context_revisions c LEFT JOIN tasks t ON t.id=c.task_id WHERE t.id IS NULL)",[],|row|row.get(0)).map_err(|error|error.to_string())?;
    if orphan {
        return Err("Orphan context revision owner; refusing to open".into());
    }
    let mut statement=connection.prepare("SELECT task_id,source_key,source_sha256,policy,tokenizer,tokens,assembly FROM context_revisions").map_err(|e|e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    let mut count = 0;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let task: i64 = row.get(0).map_err(|e| e.to_string())?;
        let entry = read_row(row, 1)?;
        validate(&entry)?;
        if task <= 0 {
            return Err("Invalid context revision owner".into());
        }
        count += 1;
        if count > 32768 {
            return Err("Context revision capacity exceeded".into());
        }
    }
    let over_quota: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM context_revisions GROUP BY task_id HAVING count(*)>4096 OR sum(length(CAST(source_key AS BLOB))+length(CAST(policy AS BLOB))+length(CAST(tokenizer AS BLOB))+length(CAST(source_sha256 AS BLOB))+COALESCE(length(CAST(assembly AS BLOB)),0)+16)>4194304)",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if over_quota {
        return Err(
            "Context revision task metadata or entry quota exceeded; refusing to open".into(),
        );
    }
    Ok(())
}
fn validate(row: &CacheRevision) -> Result<(), String> {
    if row.source_key.is_empty()
        || row.source_key.len() > 65536
        || row.source_sha256.len() != 64
        || !row.source_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || row.policy.is_empty()
        || row.policy.len() > 256
        || row.tokenizer.is_empty()
        || row.tokenizer.len() > 256
        || row.tokens > i64::MAX as usize
    {
        return Err("Invalid context revision identity/configuration".into());
    }
    if crate::workspace::privacy::redact(&row.source_key) != row.source_key {
        return Err("Context revision key contains recognizable credentials".into());
    }
    if let Some(assembly) = &row.assembly {
        let identity = row
            .source_key
            .strip_prefix("manifest:")
            .ok_or("Context assembly metadata requires a stable manifest identity")?;
        if identity.len() != 64 || !identity.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Invalid context assembly manifest identity".into());
        }
        if assembly.len() > MAX_ASSEMBLY_METADATA_BYTES {
            return Err("Context assembly metadata exceeds its 4 MiB limit".into());
        }
        validate_assembly_metadata(assembly)?;
    } else if row.source_key.starts_with("manifest:") {
        return Err("Context assembly manifest is missing metadata".into());
    }
    Ok(())
}
fn read_row(row: &rusqlite::Row<'_>, offset: usize) -> Result<CacheRevision, String> {
    let tokens: i64 = row.get(offset + 4).map_err(|e| e.to_string())?;
    Ok(CacheRevision {
        source_key: row.get(offset).map_err(|e| e.to_string())?,
        source_sha256: row.get(offset + 1).map_err(|e| e.to_string())?,
        policy: row.get(offset + 2).map_err(|e| e.to_string())?,
        tokenizer: row.get(offset + 3).map_err(|e| e.to_string())?,
        tokens: usize::try_from(tokens).map_err(|_| "Invalid token count")?,
        assembly: row.get(offset + 5).map_err(|e| e.to_string())?,
    })
}
impl State {
    pub(super) fn context_cache_request(&mut self, request: &Request) -> Result<Response, String> {
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let task = load_task(&transaction, &self.project, request.task_id)?
            .ok_or("Context cache belongs to a missing or foreign task")?;
        match &request.operation {
            Operation::Load => {}
            Operation::Purge => {
                transaction
                    .execute("DELETE FROM context_revisions WHERE task_id=?1", [task.id])
                    .map_err(|e| e.to_string())?;
            }
            Operation::Save(rows) => {
                if task.archived {
                    return Err("Restore task before changing its context cache".into());
                }
                if rows.len() > 4096 {
                    return Err("Context revision request exceeds 4096 entries".into());
                }
                let mut keys = std::collections::BTreeSet::new();
                for row in rows {
                    validate(row)?;
                    if !keys.insert(&row.source_key) {
                        return Err("Duplicate context revision key".into());
                    }
                }
                // Explicit complete index replacement makes changed sources/configurations invalidate old rows.
                transaction
                    .execute("DELETE FROM context_revisions WHERE task_id=?1", [task.id])
                    .map_err(|e| e.to_string())?;
                for row in rows {
                    transaction.execute("INSERT INTO context_revisions(task_id,source_key,source_sha256,policy,tokenizer,tokens,assembly) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![task.id,row.source_key,row.source_sha256,row.policy,row.tokenizer,row.tokens as i64,row.assembly]).map_err(|e|e.to_string())?;
                }
                let count: i64 = transaction
                    .query_row("SELECT count(*) FROM context_revisions", [], |row| {
                        row.get(0)
                    })
                    .map_err(|e| e.to_string())?;
                if count > 32768 {
                    return Err("Context revision quota reached; purge an index explicitly".into());
                }
                let bytes:i64=transaction.query_row("SELECT COALESCE(sum(length(CAST(source_key AS BLOB))+length(CAST(policy AS BLOB))+length(CAST(tokenizer AS BLOB))+length(CAST(source_sha256 AS BLOB))+COALESCE(length(CAST(assembly AS BLOB)),0)+16),0) FROM context_revisions WHERE task_id=?1",[request.task_id],|row|row.get(0)).map_err(|e|e.to_string())?;
                if bytes > 4 * 1024 * 1024 {
                    return Err("Context revision metadata exceeds its 4 MiB quota; purge an index explicitly".into());
                }
                enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
            }
        }
        let rows = {
            let mut statement=transaction.prepare("SELECT source_key,source_sha256,policy,tokenizer,tokens,assembly FROM context_revisions WHERE task_id=?1 ORDER BY source_key").map_err(|e|e.to_string())?;
            let mut cursor = statement.query([task.id]).map_err(|e| e.to_string())?;
            let mut rows = Vec::new();
            while let Some(row) = cursor.next().map_err(|e| e.to_string())? {
                rows.push(read_row(row, 0)?);
            }
            rows
        };
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(Response { rows })
    }
}
