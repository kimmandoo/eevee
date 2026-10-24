//! Task state operations, snapshot validation and row loading.
use super::*;

impl State {
    pub(super) fn reopen(&mut self) -> Event {
        self.reopen_with_baseline(false)
    }

    pub(super) fn reopen_startup(&mut self) -> Event {
        self.reopen_with_baseline(true)
    }

    fn reopen_with_baseline(&mut self, startup: bool) -> Event {
        if let Some(connection) = &self.connection
            && let Err(message) = no_active(connection, None)
        {
            return Event::Failed {
                revision: None,
                message,
            };
        }
        self.connection.take();
        let opened = if startup {
            self.open_and_load()
        } else {
            self.open_and_load_with_baseline(false)
        };
        match opened {
            Ok(snapshot) => Event::Loaded(snapshot),
            Err(error) => Event::Failed {
                revision: None,
                message: actionable(error),
            },
        }
    }

    pub(super) fn open_and_load(&mut self) -> Result<Project, String> {
        self.open_and_load_with_baseline(true)
    }

    fn open_and_load_with_baseline(&mut self, startup: bool) -> Result<Project, String> {
        if fs::symlink_metadata(self.data_dir.join(super::snapshot::INCOMPLETE)).is_ok() {
            return Err(
                "This data directory is an incomplete restore; it cannot be opened or migrated"
                    .into(),
            );
        }
        if self.project.is_empty() {
            return Err("The canonical Unicode project identity is empty".into());
        }
        reject_secret_identity(&self.project)?;
        if self.data_dir.exists() {
            ensure_writable(&self.data_dir)?;
        }
        fs::create_dir_all(&self.data_dir)
            .map_err(|error| format!("Cannot create the data directory: {error}"))?;
        // Resolve directory aliases (including macOS /var -> /private/var)
        // before SQLite's NOFOLLOW flag checks every path component. Never
        // canonicalize the final DB/lock basename: target links remain refused.
        self.data_dir = fs::canonicalize(&self.data_dir)
            .map_err(|error| format!("Cannot resolve the data directory: {error}"))?;
        ensure_writable(&self.data_dir)?;
        if self.lock.is_none() {
            let path = self.data_dir.join(LOCK);
            regular_target(&path, true)?;
            let mut options = OpenOptions::new();
            options.read(true).write(true).create(true);
            no_follow(&mut options);
            let lock = options
                .open(&path)
                .map_err(|error| format!("Cannot open the writer lock: {error}"))?;
            ensure_regular_metadata(&lock.metadata().map_err(|error| error.to_string())?)?;
            lock.try_lock().map_err(|error| {
                format!("Cannot acquire the writer lock (another ev may own this store): {error}")
            })?;
            self.lock = Some(lock);
        }

        let path = self.data_dir.join(DATABASE);
        if regular_target(&path, true)? {
            ensure_writable(&path)?;
            // Inspect before any writable SQLite open. Unknown, corrupt, and newer
            // databases must not be changed by initialization or journal recovery.
            preflight(&path)?;
        }
        let flags =
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | sqlite_no_follow();
        let mut connection = Connection::open_with_flags(&path, flags)
            .map_err(|error| format!("Cannot open the database for writing: {error}"))?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|error| error.to_string())?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|error| error.to_string())?;
        initialize(&mut connection)?;
        reconcile_runs(&mut connection, &self.data_dir)?;
        set_quota(&connection, self.policy)?;
        super::records::baseline(&mut connection, &self.data_dir, &self.project, startup)?;
        let snapshot = load(&connection, &self.project)?;
        self.connection = Some(connection);
        Ok(snapshot)
    }

    pub(super) fn save(&mut self, mut snapshot: Snapshot) -> Result<Snapshot, String> {
        let local_records = std::mem::take(&mut snapshot.local_records);
        validate(&snapshot)?;
        if snapshot.project != self.project {
            return Err(
                "Save project does not match this worker's canonical project identity".into(),
            );
        }
        redact_snapshot(&mut snapshot);
        validate(&snapshot)?;
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it before saving")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot begin save transaction: {error}"))?;
        if snapshot.id != 0 {
            let previous = load_task(&transaction, &self.project, snapshot.id)?
                .ok_or("Task does not exist in this project")?;
            // A caller may still have the timestamp from an earlier acknowledgement.
            // Timestamps are storage-owned and never participate in revision conflicts.
            snapshot.updated_at = previous.updated_at;
            if previous.archived {
                return Err("Restore the archived task before saving edits".into());
            }
            snapshot.archived = previous.archived;
            snapshot.archived_at = previous.archived_at;
            snapshot.redacted |= previous.redacted;
            if snapshot.revision < previous.revision
                || (snapshot.revision == previous.revision && snapshot != previous)
            {
                return Err("Refusing a stale or conflicting revision; reload the committed task before saving".into());
            }
            if snapshot == previous && local_records.is_empty() {
                transaction
                    .commit()
                    .map_err(|error| format!("Cannot commit save transaction: {error}"))?;
                return Ok(previous);
            }
            if previous.complete
                && snapshot.complete
                && (previous.goal != snapshot.goal || previous.criteria != snapshot.criteria)
            {
                return Err(
                    "Reopen the completed task before changing its goal or criteria".into(),
                );
            }
        } else {
            if snapshot.archived || snapshot.archived_at != 0 {
                return Err("New tasks cannot be archived through save".into());
            }
            let count: i64 = transaction
                .query_row(
                    "SELECT count(*) FROM tasks WHERE project=?1",
                    [&self.project],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if count >= MAX_TASKS as i64 {
                return Err("Project capacity reached (256 tasks); no tasks were removed".into());
            }
        }
        let roster = partners(&transaction)?;
        if snapshot
            .partner_drafts
            .keys()
            .any(|id| !roster.iter().any(|member| member.id == *id))
        {
            return Err("Draft owner is no longer in the party; reload the committed task".into());
        }
        let criteria =
            serde_json::to_string(&snapshot.criteria).map_err(|error| error.to_string())?;
        let partner_drafts = bounded_json(&snapshot.partner_drafts, MAX_PARTNER_DRAFTS_BYTES)?;
        let navigation =
            serde_json::to_string(&snapshot.navigation).map_err(|error| error.to_string())?;
        let workflows = bounded_json(&snapshot.workflows, MAX_WORKFLOW_BYTES)?;
        let memory_draft = bounded_json(&snapshot.memory_draft, MAX_MEMORY_BYTES)?;
        let approved_memories = bounded_json(&snapshot.approved_memories, MAX_WORKFLOW_BYTES)?;
        let workflow_draft = bounded_json(&snapshot.workflow_draft, MAX_WORKFLOW_BYTES)?;
        let updated_at: i64 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("Cannot timestamp the save: {error}"))?
            .as_millis()
            .try_into()
            .map_err(|_| "Save timestamp exceeds SQLite's integer range")?;
        let id = if snapshot.id == 0 {
            transaction.execute(
                "INSERT INTO tasks(project,goal,form,revision,criteria,next_action,complete,drafts,navigation,updated_at,redacted,command,workflows,memory_draft,approved_memories,workflow_draft,lead_partner,partner_drafts)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,'[]',?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
                params![snapshot.project, snapshot.goal, snapshot.form as i64, snapshot.revision as i64,
                    criteria, snapshot.next_action, snapshot.complete, navigation, updated_at, snapshot.redacted, snapshot.command,
                    workflows, memory_draft, approved_memories, workflow_draft, snapshot.lead_partner, partner_drafts],
            ).map_err(|error| format!("Cannot insert the task: {error}"))?;
            let id = transaction.last_insert_rowid();
            select_task(&transaction, &self.project, id)?;
            id
        } else {
            transaction.execute(
                "UPDATE tasks SET goal=?1,form=?2,revision=?3,criteria=?4,next_action=?5,complete=?6,
                 drafts='[]',navigation=?7,updated_at=?8,redacted=?11,command=?12,
                 workflows=?13,memory_draft=?14,approved_memories=?15,workflow_draft=?16,lead_partner=?17,partner_drafts=?18 WHERE id=?9 AND project=?10",
                params![snapshot.goal, snapshot.form as i64, snapshot.revision as i64, criteria,
                    snapshot.next_action, snapshot.complete, navigation, updated_at,
                    snapshot.id, snapshot.project, snapshot.redacted, snapshot.command,
                    workflows, memory_draft, approved_memories, workflow_draft, snapshot.lead_partner, partner_drafts],
            ).map_err(|error| format!("Cannot write the task: {error}"))?;
            snapshot.id
        };
        set_partner_form(
            &transaction,
            snapshot.lead_partner,
            snapshot.form,
            snapshot.auto_form,
        )?;
        snapshot.id = id;
        super::records::save_intents(&transaction, &self.data_dir, &snapshot, &local_records)?;
        enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit the task: {error}"))?;
        snapshot.id = id;
        snapshot.updated_at = updated_at;
        snapshot.local_records = local_records;
        Ok(snapshot)
    }

    pub(super) fn select(&mut self, id: i64) -> Result<Snapshot, String> {
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it before selecting a task")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot begin selection transaction: {error}"))?;
        let saved = load_task(&transaction, &self.project, id)?
            .ok_or("Task does not exist in this project")?;
        if saved.archived {
            return Err("Archived tasks must be restored before selection".into());
        }
        select_task(&transaction, &self.project, id)?;
        enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit task selection: {error}"))?;
        Ok(saved)
    }

    pub(super) fn manage(&mut self, request: Management) -> Result<Managed, String> {
        self.manage_at(request, timestamp()?)
    }

    pub(super) fn preview_delete(
        &self,
        ids: &[i64],
        expired_before: Option<i64>,
    ) -> Result<Vec<DeletionTarget>, String> {
        let connection = self
            .connection
            .as_ref()
            .ok_or("Storage is unavailable; retry opening it")?;
        let eligible_before = deletion_scope(ids, expired_before, self.policy, timestamp()?)?;
        let directory = SpoolDirectory::open(&self.data_dir);
        let mut targets = Vec::with_capacity(ids.len());
        for id in ids {
            let task = deletion_task(
                connection,
                &self.project,
                *id,
                expired_before,
                eligible_before,
            )?;
            let mut statement = connection.prepare("SELECT id,json_extract(record,'$.path') FROM excerpts WHERE task_id=?1 ORDER BY id LIMIT 17")
                .map_err(|error| error.to_string())?;
            let excerpts = statement
                .query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|error| error.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?;
            if excerpts.len() > crate::workspace::context::MAX_EXCERPTS {
                return Err("Excerpt preview exceeds its bounded capacity".into());
            }
            let mut statement = connection.prepare("SELECT run_id,output_name,output_identity FROM executions WHERE task_id=?1 ORDER BY rowid LIMIT 257")
                .map_err(|error| error.to_string())?;
            let references = statement
                .query_map([id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|error| error.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?;
            if references.len() > MAX_RUNS {
                return Err("Run preview exceeds its bounded capacity".into());
            }
            let runs = references
                .into_iter()
                .map(|(run_id, name, identity)| {
                    let has_output = directory
                        .as_ref()
                        .ok()
                        .is_some_and(|directory| directory.read(&name, &identity).is_ok());
                    (run_id, has_output)
                })
                .collect();
            let mut statement = connection
                .prepare(
                    "SELECT record FROM provider_journal WHERE task_id=?1 ORDER BY rowid LIMIT 257",
                )
                .map_err(|error| error.to_string())?;
            let records = statement
                .query_map([id], |row| json_column::<ProviderRecord>(row, 0, 65536))
                .map_err(|error| error.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?;
            if records.len() > 256 {
                return Err("Provider journal deletion preview exceeds its bound".into());
            }
            let provider_records = records
                .into_iter()
                .map(|record| (record.id, record.agent))
                .collect();
            let mut statement = connection
                .prepare(
                    "SELECT id,revision,project,task_id,proposal FROM memories
                 WHERE task_id=?1 AND scope='task' ORDER BY id LIMIT 257",
                )
                .map_err(|error| error.to_string())?;
            let memories = statement
                .query_map([id], memory_row)
                .map_err(|error| error.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?;
            if memories.len() > MAX_MEMORIES {
                return Err("Memory deletion preview exceeds its bounded capacity".into());
            }
            for record in &memories {
                validate_memory(record)?;
                if !crate::workspace::memory::visible(record, &self.project, *id)
                    || record.proposal.scope != crate::workspace::memory::Scope::Task
                {
                    return Err("Memory deletion preview has conflicting task ownership".into());
                }
            }
            targets.push(DeletionTarget {
                task,
                excerpts,
                runs,
                provider_records,
                memories,
            });
        }
        Ok(targets)
    }

    pub(super) fn manage_at(&mut self, request: Management, now: i64) -> Result<Managed, String> {
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot begin task management: {error}"))?;
        let mut deleted_outputs = Vec::new();
        let affected = match request {
            Management::PreviewDelete { .. } => {
                return Err(
                    "Deletion preview is read-only; no management mutation was performed".into(),
                );
            }
            Management::Archive { id, archived } => {
                no_active(&transaction, Some(id))?;
                let task = load_task(&transaction, &self.project, id)?
                    .ok_or("Task is missing or belongs to another project")?;
                if task.archived == archived {
                    return Err("Task archive state changed; review the task again".into());
                }
                transaction
                    .execute(
                        "UPDATE tasks SET archived=?1,archived_at=?2 WHERE id=?3 AND project=?4",
                        params![archived, if archived { now } else { 0 }, id, self.project],
                    )
                    .map_err(|error| format!("Cannot change archive state: {error}"))?;
                vec![id]
            }
            Management::Delete {
                ids,
                expired_before,
            } => {
                let eligible_before = deletion_scope(&ids, expired_before, self.policy, now)?;
                for id in &ids {
                    deletion_task(
                        &transaction,
                        &self.project,
                        *id,
                        expired_before,
                        eligible_before,
                    )?;
                }
                for id in &ids {
                    deleted_outputs.extend(super::artifacts::deletion_outputs(&transaction, *id)?);
                    let mut outputs = transaction
                        .prepare(
                            "SELECT output_name,output_identity FROM executions WHERE task_id=?1",
                        )
                        .map_err(|error| error.to_string())?;
                    deleted_outputs.extend(
                        outputs
                            .query_map([id], |row| {
                                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                            })
                            .map_err(|error| error.to_string())?
                            .collect::<rusqlite::Result<Vec<_>>>()
                            .map_err(|error| error.to_string())?,
                    );
                    transaction
                        .execute("DELETE FROM excerpts WHERE task_id=?1", [id])
                        .map_err(|error| error.to_string())?;
                    transaction
                        .execute("DELETE FROM executions WHERE task_id=?1", [id])
                        .map_err(|error| error.to_string())?;
                    transaction
                        .execute("DELETE FROM provider_journal WHERE task_id=?1", [id])
                        .map_err(|error| error.to_string())?;
                    transaction
                        .execute("DELETE FROM outgoing_receipts WHERE task_id=?1", [id])
                        .map_err(|error| error.to_string())?;
                    transaction
                        .execute("DELETE FROM tool_artifacts WHERE task_id=?1", [id])
                        .map_err(|error| error.to_string())?;
                    transaction
                        .execute("DELETE FROM context_revisions WHERE task_id=?1", [id])
                        .map_err(|error| error.to_string())?;
                    transaction
                        .execute("DELETE FROM memories WHERE task_id=?1", [id])
                        .map_err(|error| error.to_string())?;
                    super::held::purge_missing_memories(&transaction)?;
                    super::moves::purge_task(&transaction, *id)?;
                    super::records::purge_task(&transaction, &self.project, *id)?;
                    transaction
                        .execute(
                            "DELETE FROM tasks WHERE project=?1 AND id=?2",
                            params![self.project, id],
                        )
                        .map_err(|error| format!("Cannot delete task: {error}"))?;
                }
                let remaining: i64 = transaction
                    .query_row(
                        "SELECT count(*) FROM tasks WHERE project=?1",
                        [&self.project],
                        |row| row.get(0),
                    )
                    .map_err(|error| error.to_string())?;
                if remaining == 0 {
                    super::moves::purge_project(&transaction, &self.project)?;
                    super::records::purge_project(&transaction, &self.project)?;
                }
                ids
            }
        };
        // A restore never creates an active selection. Repair only an invalidated one.
        let active: Option<i64> = transaction
            .query_row(
                "SELECT active_id FROM selections WHERE project=?1",
                [&self.project],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(active) = active {
            let valid =
                load_task(&transaction, &self.project, active)?.is_some_and(|task| !task.archived);
            if !valid {
                transaction
                    .execute("DELETE FROM selections WHERE project=?1", [&self.project])
                    .map_err(|error| error.to_string())?;
                let replacement: Option<i64> = transaction
                    .query_row(
                        "SELECT id FROM tasks WHERE project=?1 AND archived=0 ORDER BY id LIMIT 1",
                        [&self.project],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|error| error.to_string())?;
                if let Some(id) = replacement {
                    select_task(&transaction, &self.project, id)?;
                }
            }
        }
        let mut project = load(&transaction, &self.project)?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit task management: {error}"))?;
        // From this point the UI must receive the committed task list. Cleanup and
        // accounting failures are warnings, never rollback-shaped errors.
        let warning = if deleted_outputs.is_empty() {
            None
        } else {
            (|| -> Result<(), String> {
                let directory = SpoolDirectory::open(&self.data_dir)
                    .map_err(|error| format!("Task deletion committed, but artifact cleanup failed: {error}"))?;
                for (name, identity) in deleted_outputs {
                    directory.remove(&name, &identity)
                        .map_err(|error| format!("Task deletion committed, but artifact cleanup failed: {error}"))?;
                }
                project.usage_bytes = evidence_usage(connection, &self.data_dir)
                    .map_err(|error| format!("Task deletion committed, but updated quota accounting is unavailable: {error}"))?;
                Ok(())
            })().err()
        };
        Ok(Managed {
            project,
            affected,
            warning,
        })
    }
}

pub(super) fn check_json_size<T: Serialize>(value: &T, max: usize) -> Result<(), String> {
    struct Capacity(usize);
    impl Write for Capacity {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self.0.checked_sub(bytes.len()).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Stored JSON exceeds its bounded size",
                )
            })?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Capacity(max), value).map_err(|error| error.to_string())
}

pub(super) fn validate_draft_shape(value: &serde_json::Value, depth: usize) -> Result<(), String> {
    if depth > 32 {
        return Err("Stored drawer draft exceeds its nesting limit".into());
    }
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                validate_draft_shape(value, depth + 1)?;
            }
        }
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                if crate::workspace::privacy::redact(key) != key.as_str() {
                    return Err("Stored draft field names must not contain credentials".into());
                }
                validate_draft_shape(value, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn bounded_json<T: Serialize>(value: &T, max: usize) -> Result<String, String> {
    let json = serde_json::to_string(value).map_err(|error| error.to_string())?;
    if json.len() > max {
        return Err(format!(
            "Stored workflow or memory JSON exceeds {max} bytes"
        ));
    }
    Ok(json)
}

pub(super) fn redact_json(value: &mut serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => match crate::workspace::privacy::redact(text) {
            std::borrow::Cow::Owned(clean) if clean != *text => {
                *text = clean;
                true
            }
            _ => false,
        },
        serde_json::Value::Array(values) => values
            .iter_mut()
            .fold(false, |changed, value| redact_json(value) | changed),
        serde_json::Value::Object(values) => values
            .values_mut()
            .fold(false, |changed, value| redact_json(value) | changed),
        _ => false,
    }
}

pub(super) fn actionable(error: String) -> String {
    let error = crate::workspace::privacy::redact(&error);
    format!(
        "{error}. Retry after resolving the problem, select an isolated data directory, or export readable tasks with --export-data FILE; the existing store is never reset."
    )
}

pub(super) fn timestamp() -> Result<i64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System clock predates Unix epoch".to_string())?
        .as_millis()
        .try_into()
        .map_err(|_| "Timestamp exceeds SQLite range".into())
}

pub(super) fn reject_secret_identity(project: &str) -> Result<(), String> {
    if crate::workspace::privacy::redact(project).as_ref() != project {
        return Err("Canonical project identity contains recognizable credentials; choose a credential-free path (identity was not rewritten)".into());
    }
    Ok(())
}

pub(super) fn redact_snapshot(snapshot: &mut Snapshot) {
    pub(super) fn redact_field(text: &mut String) -> bool {
        match crate::workspace::privacy::redact(text) {
            std::borrow::Cow::Owned(clean) if clean != *text => {
                *text = clean;
                true
            }
            _ => false,
        }
    }
    snapshot.redacted |= redact_field(&mut snapshot.goal);
    snapshot.redacted |= redact_field(&mut snapshot.next_action);
    snapshot.redacted |= redact_field(&mut snapshot.command);
    snapshot.redacted |= redact_json(&mut snapshot.memory_draft);
    snapshot.redacted |= redact_json(&mut snapshot.workflow_draft);
    for criterion in &mut snapshot.criteria {
        snapshot.redacted |= redact_field(&mut criterion.text);
    }
    for state in snapshot.partner_drafts.values_mut() {
        for draft in &mut state.drafts {
            if redact_field(&mut draft.text) {
                snapshot.redacted = true;
                draft.cursor = draft.cursor.min(draft.text.len());
                while !draft.text.is_char_boundary(draft.cursor) {
                    draft.cursor -= 1;
                }
            }
        }
    }
}

pub(super) fn live_bytes(connection: &Connection) -> Result<u64, String> {
    let pages: u64 = connection
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let free: u64 = connection
        .pragma_query_value(None, "freelist_count", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let size: u64 = connection
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    Ok(pages.saturating_sub(free).saturating_mul(size))
}

pub(super) fn set_quota(connection: &Connection, policy: Policy) -> Result<(), String> {
    let size: u64 = connection
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    // SQLite cannot shrink an existing file here. The live-page check also limits
    // freelist reuse when a store was previously larger than its current quota.
    connection
        .pragma_update(None, "max_page_count", (policy.quota_bytes / size).max(1))
        .map_err(|error| format!("Cannot apply task storage quota: {error}"))
}

pub(super) fn validate(snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.project.is_empty() {
        return Err("A stored project identity is empty".into());
    }
    reject_secret_identity(&snapshot.project)?;
    if snapshot.archived_at < 0 || (!snapshot.archived && snapshot.archived_at != 0) {
        return Err("Invalid archive timestamp".into());
    }
    if snapshot.goal.len() > MAX_GOAL_BYTES {
        return Err("A goal exceeds the 4096-byte storage limit".into());
    }
    if snapshot.form >= 9 {
        return Err("A stored form is outside 0..=8".into());
    }
    // AUTOINCREMENT ids are monotonic, so a party that released members can
    // hold ids above the six-seat capacity; only positivity is structural.
    if snapshot.lead_partner < 1 {
        return Err("A lead partner must be a positive AUTOINCREMENT id".into());
    }
    if snapshot.revision > i64::MAX as u64 {
        return Err("A revision exceeds SQLite's signed integer range".into());
    }
    if snapshot.id < 0 || snapshot.updated_at < 0 {
        return Err("Task IDs and timestamps must not be negative".into());
    }
    if snapshot.next_action.len() > MAX_GOAL_BYTES {
        return Err("A next action exceeds the 4096-byte storage limit".into());
    }
    if snapshot.command.len() > MAX_GOAL_BYTES {
        return Err("A command exceeds the 4096-byte storage limit".into());
    }
    snapshot.workflows.validate(snapshot.id)?;
    if snapshot
        .workflows
        .tool_plans
        .iter()
        .chain(snapshot.workflows.tool_plan_history.iter())
        .any(|plan| plan.project != snapshot.project)
    {
        return Err("A tool plan belongs to another project".into());
    }
    validate_draft_shape(&snapshot.memory_draft, 0)?;
    validate_draft_shape(&snapshot.workflow_draft, 0)?;
    check_json_size(&snapshot.workflows, MAX_WORKFLOW_BYTES)?;
    check_json_size(&snapshot.memory_draft, MAX_MEMORY_BYTES)?;
    check_json_size(&snapshot.workflow_draft, MAX_WORKFLOW_BYTES)?;
    check_json_size(&snapshot.approved_memories, MAX_WORKFLOW_BYTES)?;
    if snapshot.approved_memories.len() > crate::workspace::memory::MAX_SELECTED {
        return Err("Selected memories exceed their bounded capacity".into());
    }
    for (index, record) in snapshot.approved_memories.iter().enumerate() {
        validate_memory(record)?;
        if !crate::workspace::memory::visible(record, &snapshot.project, snapshot.id)
            || snapshot.approved_memories[..index]
                .iter()
                .any(|other| other.id == record.id)
        {
            return Err("Selected memory has duplicate or foreign ownership".into());
        }
    }
    if snapshot.criteria.len() > 64 || snapshot.criteria.iter().any(|item| item.text.len() > 256) {
        return Err("Acceptance criteria exceed 64 entries or 256 bytes per entry".into());
    }
    if snapshot.complete
        && (snapshot.criteria.is_empty() || snapshot.criteria.iter().any(|item| !item.met))
    {
        return Err("Completing a task requires nonempty, all-met acceptance criteria".into());
    }
    check_json_size(&snapshot.partner_drafts, MAX_PARTNER_DRAFTS_BYTES)?;
    if snapshot.partner_drafts.len() > MAX_PARTNERS {
        return Err("A task may retain drafts for at most six partners".into());
    }
    for (partner, state) in &snapshot.partner_drafts {
        if *partner < 1 || state.previous_form.is_some_and(|form| form >= 9) {
            return Err("Invalid partner draft ownership or previous form".into());
        }
        if state.drafts.len() > 5 {
            return Err("A partner may retain at most five drafts per task".into());
        }
        for (index, draft) in state.drafts.iter().enumerate() {
            if draft.text.len() > MAX_GOAL_BYTES || !draft.text.is_char_boundary(draft.cursor) {
                return Err(
                    "A draft exceeds 4096 bytes or has an invalid UTF-8 cursor boundary".into(),
                );
            }
            if state.drafts[..index]
                .iter()
                .any(|other| other.field == draft.field)
            {
                return Err("A partner may retain only one draft per editor field".into());
            }
        }
    }
    let nav = &snapshot.navigation;
    if nav.focus > 2
        || nav.form_selection >= 9
        || nav.previous_form.is_some_and(|form| form >= 9)
        || nav.criterion_selection >= 64
        || nav.overlay.is_some_and(|overlay| overlay > 7)
    {
        return Err("Stored navigation indices are out of bounds".into());
    }
    if let Some(trigger) = &nav.dismissed_suggestion {
        let valid = trigger.len() <= 160
            && snapshot.id > 0
            && trigger
                .strip_prefix("task:")
                .and_then(|identity| identity.split_once('/'))
                .is_some_and(|(task, identity)| {
                    task.parse::<i64>().ok() == Some(snapshot.id)
                        && !task.starts_with('0')
                        && task.bytes().all(|byte| byte.is_ascii_digit())
                        && identity.split_once(':').is_some_and(|(rule, hash)| {
                            rule.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                                && rule.split('-').all(|part| {
                                    !part.is_empty()
                                        && part.bytes().all(|byte| {
                                            byte.is_ascii_lowercase() || byte.is_ascii_digit()
                                        })
                                })
                                && hash.len() == 64
                                && hash.bytes().all(|byte| {
                                    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                                })
                        })
                });
        if !valid {
            return Err("Stored suggestion dismissal has invalid task ownership or trigger shape (maximum 160 bytes)".into());
        }
    }
    Ok(())
}

pub(super) fn select_task(connection: &Connection, project: &str, id: i64) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO selections(project,active_id) VALUES (?1,?2)
         ON CONFLICT(project) DO UPDATE SET active_id=excluded.active_id",
            params![project, id],
        )
        .map_err(|error| format!("Cannot persist task selection: {error}"))?;
    Ok(())
}

pub(super) fn load(connection: &Connection, project: &str) -> Result<Project, String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {TASK_COLUMNS},{TASK_AUTO_FORM} FROM tasks WHERE project=?1 ORDER BY id"
        ))
        .map_err(|error| error.to_string())?;
    let tasks = statement
        .query_map([project], snapshot_row)
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Cannot load saved tasks: {error}"))?;
    let active_id = connection
        .query_row(
            "SELECT active_id FROM selections WHERE project=?1",
            [project],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(Project {
        tasks,
        partners: partners(connection)?,
        active_id,
        usage_bytes: match connection.path().and_then(|path| Path::new(path).parent()) {
            Some(directory) => evidence_usage(connection, directory)?,
            None => live_bytes(connection)?,
        },
    })
}

pub(super) fn load_task(
    connection: &Connection,
    project: &str,
    id: i64,
) -> Result<Option<Snapshot>, String> {
    connection
        .query_row(
            &format!(
                "SELECT {TASK_COLUMNS},{TASK_AUTO_FORM} FROM tasks WHERE project=?1 AND id=?2"
            ),
            params![project, id],
            snapshot_row,
        )
        .optional()
        .map_err(|error| format!("Cannot load the saved task: {error}"))
}

pub(super) fn json_column<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    index: usize,
    max: usize,
) -> rusqlite::Result<T> {
    let text: String = row.get(index)?;
    if text.len() > max {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(io::Error::new(
                io::ErrorKind::InvalidData,
                "Stored JSON exceeds its bounded size",
            )),
        ));
    }
    serde_json::from_str(&text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

pub(super) fn snapshot_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Snapshot> {
    let complete: i64 = row.get(7)?;
    if !(0..=1).contains(&complete) {
        return Err(rusqlite::Error::IntegralValueOutOfRange(7, complete));
    }
    let lead_partner = row.get(19)?;
    let navigation: Navigation = json_column(row, 9, 1024)?;
    let legacy_drafts: Vec<Draft> = json_column(row, 8, 110_000)?;
    let mut partner_drafts: std::collections::BTreeMap<i64, PartnerDrafts> =
        if row.as_ref().column_count() > 20 {
            json_column(row, 20, MAX_PARTNER_DRAFTS_BYTES)?
        } else {
            std::collections::BTreeMap::new()
        };
    // Schema 7 reserved this column before partner drafts shipped. Transfer
    // the legacy task draft only when its lead has no partner-owned state.
    if !legacy_drafts.is_empty() || navigation.editing || navigation.previous_form.is_some() {
        partner_drafts
            .entry(lead_partner)
            .or_insert_with(|| PartnerDrafts {
                drafts: legacy_drafts,
                editor_field: navigation.editor_field,
                editing: navigation.editing,
                previous_form: navigation.previous_form,
            });
    }
    let auto_form = if row.as_ref().column_count() > 21 {
        let value: i64 = row.get(21)?;
        if !(0..=1).contains(&value) {
            return Err(rusqlite::Error::IntegralValueOutOfRange(21, value));
        }
        value != 0
    } else {
        // Historical layouts predate durable mode and are read during migration.
        row.get::<_, usize>(2)? == 0
    };
    Ok(Snapshot {
        project: row.get(0)?,
        goal: row.get(1)?,
        form: row.get(2)?,
        auto_form,
        lead_partner,
        revision: row.get(3)?,
        id: row.get(4)?,
        criteria: json_column(row, 5, 110_000)?,
        next_action: row.get(6)?,
        complete: complete != 0,
        partner_drafts,
        navigation,
        updated_at: row.get(10)?,
        archived: row.get(11)?,
        archived_at: row.get(12)?,
        redacted: row.get(13)?,
        command: row.get(14)?,
        workflows: json_column(row, 15, MAX_WORKFLOW_BYTES)?,
        memory_draft: json_column(row, 16, MAX_MEMORY_BYTES)?,
        approved_memories: json_column(row, 17, MAX_WORKFLOW_BYTES)?,
        workflow_draft: json_column(row, 18, MAX_WORKFLOW_BYTES)?,
        local_records: Vec::new(),
    })
}

pub(super) fn legacy_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Snapshot> {
    Ok(Snapshot {
        goal: row.get(1)?,
        form: row.get(2)?,
        revision: row.get(3)?,
        ..Snapshot::empty(row.get(0)?)
    })
}
