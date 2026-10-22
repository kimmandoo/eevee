//! One bounded local identity preference per project/partner/form; never context.
use super::*;
use crate::workspace::held::{FileProof, Item, Target};
use std::sync::atomic::AtomicBool;

const MAX_TARGET_BYTES: usize = 32 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const MAX_PROJECT_ITEMS: usize = MAX_PARTNERS * 9;

pub(super) fn migrate(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE held_items(
         project TEXT NOT NULL CHECK(length(CAST(project AS BLOB)) BETWEEN 1 AND 4096),
         partner INTEGER NOT NULL CHECK(partner>0),
         form INTEGER NOT NULL CHECK(form>=0 AND form<9),
         target TEXT NOT NULL CHECK(length(CAST(target AS BLOB))<=32768),
         PRIMARY KEY(project,partner,form));
         PRAGMA user_version=17;",
        )
        .map_err(|error| format!("Cannot migrate held preferences: {error}"))
}

fn target_identity(target: &Target) -> Result<(), String> {
    match target {
        Target::File { path } => {
            if path.len() > MAX_PATH_BYTES
                || path.chars().any(|ch| ch.is_control() || matches!(ch,
                    '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
                || crate::workspace::context::excluded(Path::new(path), &[])
                || crate::workspace::privacy::redact(path) != path.as_str()
                || Path::new(path).components().any(|component| {
                    let name = component.as_os_str().to_str().unwrap_or_default();
                    name.get(.."workbench.sqlite3".len()).is_some_and(|prefix| prefix.eq_ignore_ascii_case("workbench.sqlite3"))
                        || name.eq_ignore_ascii_case("workbench.lock")
                        || (name.get(.."ev-run-".len()).is_some_and(|prefix| prefix.eq_ignore_ascii_case("ev-run-"))
                            && name.get(name.len().saturating_sub(".output".len())..).is_some_and(|suffix| suffix.eq_ignore_ascii_case(".output")))
                })
            {
                return Err("Held file identity is excluded, protected or invalid".into());
            }
        }
        Target::Memory { id, revision } => {
            if *id <= 0 || *revision == 0 || *revision > i64::MAX as u64 {
                return Err("Held memory requires an exact positive committed revision".into());
            }
        }
    }
    Ok(())
}

fn file_proof(
    data_dir: &Path,
    project: &str,
    task: &Snapshot,
    path: &str,
    proof: &FileProof,
) -> Result<(), String> {
    let excerpt = &proof.excerpt;
    excerpt.validate()?;
    if excerpt.task_id != task.id
        || !excerpt.available
        || excerpt.path != path
        || proof.exclusions.len() > 32
        || proof.exclusions.iter().any(|prefix| {
            prefix.as_os_str().is_empty()
                || prefix.as_os_str().len() > 32 * 1024
                || !prefix
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
        })
        || crate::workspace::context::excluded(Path::new(path), &proof.exclusions)
    {
        return Err(
            "Held file requires its current task-owned, policy-approved source preview".into(),
        );
    }
    let root = Path::new(&excerpt.root);
    let project = Path::new(project);
    let source = root.join(path);
    if fs::canonicalize(project).map_err(|error| error.to_string())? != project
        || !project.starts_with(root)
        || !source.starts_with(project)
        || fs::canonicalize(root).map_err(|error| error.to_string())? != root
        || source.starts_with(fs::canonicalize(data_dir).map_err(|error| error.to_string())?)
    {
        return Err(
            "Held file is outside the canonical project or inside protected storage".into(),
        );
    }
    let stopped = AtomicBool::new(false);
    if !crate::workspace::repo::source_visible(root, Path::new(path), &stopped)? {
        return Err("Held file is ignored, missing or no longer visible".into());
    }
    let (bytes, digest) = crate::workspace::repo::read_source(
        root,
        Path::new(path),
        crate::workspace::context::MAX_SOURCE_BYTES,
        &stopped,
    )?;
    if digest != excerpt.source_sha256 || bytes.contains(&0) {
        return Err("Held file changed or became unavailable; inspect it again".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "Held file is not UTF-8")?;
    let masked = crate::workspace::privacy::redact(text);
    if crate::workspace::context::line_range(&masked, excerpt.start_line, excerpt.end_line)?
        != excerpt.text
    {
        return Err("Held file range or privacy projection changed; inspect it again".into());
    }
    Ok(())
}

pub(super) fn set(
    connection: &Connection,
    data_dir: &Path,
    task: &Snapshot,
    partner: i64,
    form: usize,
    target: Option<&Target>,
    proof: Option<&FileProof>,
) -> Result<(), String> {
    let project = &task.project;
    if task.id <= 0
        || task.archived
        || task.goal.trim().is_empty()
        || form >= 9
        || !partners(connection)?
            .iter()
            .any(|member| member.id == partner)
    {
        return Err(
            "Held preference requires a live partner, valid form and saved active project task"
                .into(),
        );
    }
    let selected: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM selections WHERE project=?1 AND active_id=?2)",
            params![project, task.id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !selected {
        return Err("Held preference task is no longer active".into());
    }
    super::partners::idle(connection, partner)?;
    no_active(connection, Some(task.id))?;
    if let Some(target) = target {
        target_identity(target)?;
        match target {
            Target::File { path } => file_proof(
                data_dir,
                project,
                task,
                path,
                proof.ok_or("Held file requires an actual reviewed source-worker preview")?,
            )?,
            Target::Memory { id, revision } => {
                if proof.is_some()
                    || !memories(connection, project, task.id)?
                        .iter()
                        .any(|record| record.id == *id && record.revision == *revision)
                {
                    return Err("Held memory is missing, edited or outside this task's approved visible scope".into());
                }
            }
        }
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM held_items WHERE project=?1 AND NOT(partner=?2 AND form=?3)",
                params![project, partner, form as i64],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if count >= MAX_PROJECT_ITEMS as i64 {
            return Err("Held preference capacity reached; nothing was evicted".into());
        }
        connection
            .execute(
                "INSERT INTO held_items(project,partner,form,target) VALUES(?1,?2,?3,?4)
             ON CONFLICT(project,partner,form) DO UPDATE SET target=excluded.target",
                params![
                    project,
                    partner,
                    form as i64,
                    bounded_json(target, MAX_TARGET_BYTES)?
                ],
            )
            .map_err(|error| format!("Cannot save held preference: {error}"))?;
    } else {
        if proof.is_some() {
            return Err("Clearing a held preference cannot contain source proof".into());
        }
        connection
            .execute(
                "DELETE FROM held_items WHERE project=?1 AND partner=?2 AND form=?3",
                params![project, partner, form as i64],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(super) fn items(connection: &Connection, project: &str) -> Result<Vec<Item>, String> {
    let mut statement = connection.prepare(
        "SELECT partner,form,target FROM held_items WHERE project=?1 ORDER BY partner,form LIMIT 55",
    ).map_err(|error| error.to_string())?;
    let items = statement
        .query_map([project], |row| {
            Ok(Item {
                partner: row.get(0)?,
                form: row.get(1)?,
                target: json_column(row, 2, MAX_TARGET_BYTES)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    if items.len() > MAX_PROJECT_ITEMS {
        return Err("Held preference capacity exceeded".into());
    }
    for item in &items {
        target_identity(&item.target)?;
    }
    Ok(items)
}

/// Remove references on any memory edit/delete rather than silently adopting its new bytes.
pub(super) fn purge_memory(connection: &Connection, id: i64) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM held_items WHERE json_extract(target,'$.memory.id')=?1",
            [id],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn purge_missing_memories(connection: &Connection) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM held_items WHERE json_type(target,'$.memory') IS NOT NULL
        AND NOT EXISTS(SELECT 1 FROM memories m WHERE m.id=json_extract(target,'$.memory.id')
        AND m.revision=json_extract(target,'$.memory.revision'))",
            [],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn inspect(connection: &Connection) -> Result<(), String> {
    inspect_columns(
        connection,
        "held_items",
        &[
            ("project", "TEXT", true, 1),
            ("partner", "INTEGER", true, 2),
            ("form", "INTEGER", true, 3),
            ("target", "TEXT", true, 0),
        ],
    )?;
    let invalid: bool = connection
        .query_row(
            "SELECT EXISTS(
        SELECT 1 FROM held_items h LEFT JOIN partners p ON p.id=h.partner
        WHERE p.id IS NULL OR h.form<0 OR h.form>=9
        OR NOT EXISTS(SELECT 1 FROM tasks t WHERE t.project=h.project))
        OR EXISTS(SELECT 1 FROM held_items GROUP BY project HAVING count(*)>54)
        OR EXISTS(SELECT 1 FROM held_items GROUP BY project,partner,form HAVING count(*)!=1)",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if invalid {
        return Err("Held preferences have invalid ownership, duplicate scopes or capacity".into());
    }
    let mut statement = connection
        .prepare("SELECT project,partner,form,target FROM held_items")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Item {
                    partner: row.get(1)?,
                    form: row.get(2)?,
                    target: json_column(row, 3, MAX_TARGET_BYTES)?,
                },
            ))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (project, item) = row.map_err(|error| error.to_string())?;
        reject_secret_identity(&project)?;
        if project.is_empty() || project.len() > 4096 || item.partner <= 0 || item.form >= 9 {
            return Err("Invalid held preference identity".into());
        }
        target_identity(&item.target)?;
        if let Target::Memory { id, revision } = item.target {
            let record = connection.query_row(
                "SELECT id,revision,project,task_id,proposal FROM memories WHERE id=?1 AND revision=?2",
                params![id, revision as i64], memory_row,
            ).optional().map_err(|error| error.to_string())?
                .ok_or("Held preference references an unavailable memory revision")?;
            validate_memory(&record)?;
            if record.proposal.scope != crate::workspace::memory::Scope::Personal
                && record.project != project
            {
                return Err("Held preference cannot broaden memory project scope".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "held_tests.rs"]
mod tests;
