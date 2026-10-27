//! One exact task/partner-owned compressed image. Selection is not send authority.
//! Full-directory backups retain these private bytes; task-row exports do not.
use super::*;
use crate::workspace::context::screenshot::{Attachment, Capture, MAX_COMPRESSED_BYTES};
use std::sync::Arc;

const MAX_CAPTURE_METADATA_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug)]
pub struct Request {
    pub task_id: i64,
    pub partner: i64,
    pub operation: Operation,
}

#[derive(Clone, Debug)]
pub enum Operation {
    Load,
    Put(Attachment),
    Remove(Capture),
    Purge(Capture),
}

#[derive(Clone, Debug)]
pub struct Response {
    /// Only a selected, available image; never a restored approval or grant.
    pub attachment: Option<Attachment>,
    /// Retained bytes remain inspectable and explicitly purgeable after removal.
    pub retained: Option<Capture>,
}

pub(super) fn migrate(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE screenshots(
            task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE CHECK(task_id>0),
            partner INTEGER NOT NULL REFERENCES partners(id) CHECK(partner>0),
            capture TEXT NOT NULL CHECK(length(CAST(capture AS BLOB)) BETWEEN 1 AND 32768),
            bytes BLOB NOT NULL CHECK(typeof(bytes)='blob' AND length(bytes) BETWEEN 1 AND 2097152),
            selected INTEGER NOT NULL CHECK(selected IN (0,1)),
            PRIMARY KEY(task_id,partner));",
        )
        .map_err(|error| format!("Cannot migrate screenshot storage: {error}"))
}

struct Stored {
    attachment: Attachment,
    selected: bool,
}

fn read_row(row: &rusqlite::Row<'_>) -> Result<Stored, String> {
    let task_id: i64 = row.get(0).map_err(|error| error.to_string())?;
    let partner: i64 = row.get(1).map_err(|error| error.to_string())?;
    let capture: Capture =
        json_column(row, 2, MAX_CAPTURE_METADATA_BYTES).map_err(|error| error.to_string())?;
    let bytes = row
        .get_ref(3)
        .map_err(|error| error.to_string())?
        .as_blob()
        .map_err(|error| error.to_string())?;
    if bytes.is_empty() || bytes.len() > MAX_COMPRESSED_BYTES {
        return Err("Stored screenshot exceeds its compressed-byte bounds".into());
    }
    let selected: i64 = row.get(4).map_err(|error| error.to_string())?;
    if !matches!(selected, 0 | 1) || capture.task_id != task_id || capture.partner != partner {
        return Err("Stored screenshot identity or selection is invalid".into());
    }
    let attachment = Attachment {
        capture,
        bytes: Arc::from(bytes),
    };
    attachment.validate()?;
    Ok(Stored {
        attachment,
        selected: selected == 1,
    })
}

fn load(connection: &Connection, task_id: i64, partner: i64) -> Result<Option<Stored>, String> {
    let mut statement = connection.prepare(
        "SELECT task_id,partner,capture,bytes,selected FROM screenshots WHERE task_id=?1 AND partner=?2"
    ).map_err(|error| error.to_string())?;
    let mut rows = statement
        .query(params![task_id, partner])
        .map_err(|error| error.to_string())?;
    rows.next()
        .map_err(|error| error.to_string())?
        .map(read_row)
        .transpose()
}

pub(super) fn deletion_captures(
    connection: &Connection,
    task_id: i64,
) -> Result<Vec<(Capture, bool)>, String> {
    let mut statement = connection.prepare(
        "SELECT partner,capture,length(bytes) FROM screenshots WHERE task_id=?1 ORDER BY partner"
    ).map_err(|error| error.to_string())?;
    let mut rows = statement
        .query([task_id])
        .map_err(|error| error.to_string())?;
    let mut captures = Vec::new();
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let partner: i64 = row.get(0).map_err(|error| error.to_string())?;
        let capture: Capture =
            json_column(row, 1, MAX_CAPTURE_METADATA_BYTES).map_err(|error| error.to_string())?;
        let bytes: usize = row.get(2).map_err(|error| error.to_string())?;
        capture.validate()?;
        if capture.task_id != task_id || capture.partner != partner || capture.bytes != bytes {
            return Err("Screenshot deletion scope changed; inspect the task again".into());
        }
        captures.push((capture, true));
        if captures.len() > MAX_PARTNERS {
            return Err("Screenshot deletion scope exceeds the task partner capacity".into());
        }
    }
    Ok(captures)
}

fn validate_reviewed(
    connection: &Connection,
    task_id: i64,
    partner: i64,
    expected: &Capture,
) -> Result<(), String> {
    expected.validate()?;
    match load(connection, task_id, partner)? {
        Some(stored) if stored.attachment.capture == *expected => Ok(()),
        _ => Err(
            "Reviewed screenshot changed or was purged; inspect it again before removal or purge"
                .into(),
        ),
    }
}

pub(super) fn inspect(connection: &Connection) -> Result<(), String> {
    inspect_columns(
        connection,
        "screenshots",
        &[
            ("task_id", "INTEGER", true, 1),
            ("partner", "INTEGER", true, 2),
            ("capture", "TEXT", true, 0),
            ("bytes", "BLOB", true, 0),
            ("selected", "INTEGER", true, 0),
        ],
    )?;
    let mut statement = connection
        .prepare(
            "SELECT s.task_id,s.partner,s.capture,s.bytes,s.selected,t.project,p.id
         FROM screenshots s LEFT JOIN tasks t ON t.id=s.task_id
         LEFT JOIN partners p ON p.id=s.partner",
        )
        .map_err(|error| error.to_string())?;
    let mut rows = statement.query([]).map_err(|error| error.to_string())?;
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let stored = read_row(row)?;
        let project: Option<String> = row.get(5).map_err(|error| error.to_string())?;
        let partner: Option<i64> = row.get(6).map_err(|error| error.to_string())?;
        if project.as_deref() != Some(stored.attachment.capture.root.as_str())
            || partner != Some(stored.attachment.capture.partner)
        {
            return Err("Stored screenshot has a missing or foreign task/partner owner".into());
        }
    }
    Ok(())
}

fn validate_boundary(data_dir: &Path, capture: &Capture) -> Result<(), String> {
    let storage = fs::canonicalize(data_dir)
        .map_err(|error| format!("Cannot resolve protected application storage: {error}"))?;
    let root = fs::canonicalize(&capture.root)
        .map_err(|error| format!("Cannot resolve screenshot project root: {error}"))?;
    if root != Path::new(&capture.root) {
        return Err("Screenshot project root is not its canonical source identity".into());
    }
    if root.join(&capture.path).starts_with(storage) {
        return Err("Application storage cannot be attached as a screenshot".into());
    }
    Ok(())
}

/// The explicit project binding used by native request admission.
pub fn validate_selection(
    data_dir: &Path,
    project: &str,
    attachment: &Attachment,
) -> Result<(), String> {
    attachment.validate()?;
    validate_boundary(data_dir, &attachment.capture)?;
    if attachment.capture.root != project {
        return Err("Screenshot belongs to another project".into());
    }
    super::admission::read_snapshot(data_dir, |connection| {
        let task = load_task(connection, project, attachment.capture.task_id)?
            .ok_or("Screenshot task is missing or belongs to another project")?;
        if task.archived {
            return Err("Restore the task before sending its screenshot".into());
        }
        match load(connection, task.id, attachment.capture.partner)? {
            Some(stored) if stored.selected && stored.attachment == *attachment => Ok(()),
            _ => Err(
                "Selected screenshot changed, was removed or purged; reload and preview again"
                    .into(),
            ),
        }
    })
}

impl State {
    pub(super) fn screenshot_request(&mut self, request: &Request) -> Result<Response, String> {
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let task = load_task(&transaction, &self.project, request.task_id)?
            .ok_or("Screenshot belongs to a missing or foreign task")?;
        let partner_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM partners WHERE id=?1)",
                [request.partner],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !partner_exists || request.partner <= 0 {
            return Err("Screenshot partner no longer exists".into());
        }
        match &request.operation {
            Operation::Load => {}
            Operation::Put(attachment) => {
                no_active(&transaction, Some(task.id))?;
                if task.archived {
                    return Err("Restore the task before selecting a screenshot".into());
                }
                attachment.validate()?;
                validate_boundary(&self.data_dir, &attachment.capture)?;
                if attachment.capture.task_id != task.id
                    || attachment.capture.partner != request.partner
                    || attachment.capture.root != task.project
                {
                    return Err("Screenshot does not match its task, partner and project".into());
                }
                let capture = serde_json::to_string(&attachment.capture)
                    .map_err(|error| error.to_string())?;
                if capture.len() > MAX_CAPTURE_METADATA_BYTES {
                    return Err("Screenshot identity metadata exceeds its storage bound".into());
                }
                // Reselect the exact retained image without rewriting its binary.
                // Replacing it requires a separate exact-capture purge review.
                match load(&transaction, task.id, request.partner)? {
                    Some(stored) => {
                        if stored.attachment != *attachment {
                            return Err("A different screenshot is retained; explicitly review and purge it before attaching another".into());
                        }
                        transaction
                            .execute(
                                "UPDATE screenshots SET selected=1 WHERE task_id=?1 AND partner=?2",
                                params![task.id, request.partner],
                            )
                            .map_err(|error| error.to_string())?;
                    }
                    None => {
                        transaction.execute(
                            "INSERT INTO screenshots(task_id,partner,capture,bytes,selected) VALUES(?1,?2,?3,?4,1)",
                            params![task.id, request.partner, capture, attachment.bytes.as_ref()],
                        ).map_err(|error| error.to_string())?;
                    }
                }
                enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
            }
            Operation::Remove(expected) => {
                no_active(&transaction, Some(task.id))?;
                validate_reviewed(&transaction, task.id, request.partner, expected)?;
                transaction
                    .execute(
                        "UPDATE screenshots SET selected=0 WHERE task_id=?1 AND partner=?2",
                        params![task.id, request.partner],
                    )
                    .map_err(|error| error.to_string())?;
            }
            Operation::Purge(expected) => {
                no_active(&transaction, Some(task.id))?;
                validate_reviewed(&transaction, task.id, request.partner, expected)?;
                // Logical deletion is not secure erasure, backup erasure or provider deletion.
                transaction
                    .execute(
                        "DELETE FROM screenshots WHERE task_id=?1 AND partner=?2",
                        params![task.id, request.partner],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        let stored = load(&transaction, task.id, request.partner)?;
        let response = match stored {
            Some(stored) if stored.selected => Response {
                retained: Some(stored.attachment.capture.clone()),
                attachment: Some(stored.attachment),
            },
            Some(stored) => Response {
                retained: Some(stored.attachment.capture),
                attachment: None,
            },
            None => Response {
                attachment: None,
                retained: None,
            },
        };
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(response)
    }
}

#[cfg(test)]
#[path = "screenshots_tests.rs"]
mod tests;
