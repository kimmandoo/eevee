//! Read-only admission checks for reviewed provider requests and reviews.
use super::*;

/// Read-only, transaction-consistent admission check for a reviewed provider request.
/// This grants no command/file permission and never repairs or migrates storage.
pub fn validate_planning_input(
    data_dir: &Path,
    expected: &Snapshot,
    selected: &[crate::workspace::context::Excerpt],
) -> Result<(), String> {
    validate(expected)?;
    if expected.id <= 0 || expected.archived || expected.goal.trim().is_empty() {
        return Err("Planning requires a saved, active task with a goal".into());
    }
    if selected.len() > crate::workspace::context::MAX_EXCERPTS {
        return Err("Planning context exceeds the excerpt limit".into());
    }
    let mut connection = open_readonly(&data_dir.join(DATABASE))?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    validate_committed_task(&transaction, expected)?;
    let retained = excerpts(&transaction, expected.id)?;
    for (index, excerpt) in selected.iter().enumerate() {
        excerpt.validate()?;
        if !excerpt.available
            || excerpt.task_id != expected.id
            || excerpt.id <= 0
            || selected[..index].iter().any(|other| other.id == excerpt.id)
            || !retained.iter().any(|stored| stored == excerpt)
        {
            return Err("Selected context changed or was purged; reload and preview again".into());
        }
    }
    Ok(())
}

pub(super) fn validate_committed_task(
    connection: &Connection,
    expected: &Snapshot,
) -> Result<(), String> {
    let current = load_task(connection, &expected.project, expected.id)?
        .ok_or("The previewed task no longer exists in this project")?;
    if current.revision != expected.revision
        || current.goal != expected.goal
        || current.criteria != expected.criteria
        || current.next_action != expected.next_action
        || current.command != expected.command
        || current.form != expected.form
        || current.complete != expected.complete
        || current.archived != expected.archived
        || current.lead_partner != expected.lead_partner
        || current.partner_drafts != expected.partner_drafts
        || current.workflows != expected.workflows
        || current.approved_memories != expected.approved_memories
        || current.memory_draft != expected.memory_draft
        || current.workflow_draft != expected.workflow_draft
    {
        return Err("Task changed since preview; save and review a new request".into());
    }
    if expected.approved_memories.is_empty() {
        return Ok(());
    }
    crate::workspace::memory::revalidate(
        &expected.approved_memories,
        &memories(connection, &expected.project, expected.id)?,
        &expected.project,
        expected.id,
    )
}

/// Reads only explicitly linked local observations, not provider claims or spool content.
/// Missing/purged references remain absent so the review reports them as unavailable.
pub fn review_run_observations(
    data_dir: &Path,
    task: &Snapshot,
) -> Result<Vec<crate::workspace::runner::RunRecord>, String> {
    validate(task)?;
    if task.id <= 0 || task.archived {
        return Err("Review observations require a committed active task".into());
    }
    let mut connection = open_readonly(&data_dir.join(DATABASE))?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    validate_committed_task(&transaction, task)?;
    let references = task
        .workflows
        .review
        .iter()
        .flat_map(|review| {
            review
                .criteria
                .iter()
                .flat_map(|criterion| &criterion.evidence)
        })
        .chain(
            task.workflows
                .findings
                .iter()
                .flat_map(|finding| &finding.evidence),
        );
    let mut ids = Vec::new();
    for reference in references {
        if let crate::workspace::workflows::EvidenceRef::Run { task_id, run_id } = reference {
            if *task_id != task.id {
                return Err("Review run reference belongs to another task".into());
            }
            if !ids.contains(&run_id.as_str()) {
                if ids.len() == 32 {
                    return Err("Review may include at most 32 linked local runs".into());
                }
                ids.push(run_id.as_str());
            }
        }
    }
    let mut records = Vec::with_capacity(ids.len());
    let mut statement = transaction
        .prepare("SELECT record FROM executions WHERE task_id=?1 AND run_id=?2")
        .map_err(|error| error.to_string())?;
    for id in ids {
        let record = statement
            .query_row(params![task.id, id], |row| {
                json_column::<crate::workspace::runner::RunRecord>(row, 0, MAX_RUN_BYTES)
            })
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(record) = record {
            run_json(&record)?;
            if record.prepared.task_id != task.id || record.prepared.run_id != id {
                return Err("Stored review observation has conflicting ownership".into());
            }
            let mut sanitized = record.clone();
            sanitized.redact_for_storage();
            if record != sanitized {
                return Err("Stored review observation violates before-storage masking".into());
            }
            records.push(record);
        }
    }
    Ok(records)
}
