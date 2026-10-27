//! Read-only admission checks for reviewed provider requests and reviews.
use super::*;

fn current_schema(connection: &Connection) -> Result<(), String> {
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version != 20 {
        return Err("Read-only request admission requires the current schema; reopen storage to migrate before reviewing".into());
    }
    inspect(connection).map(|_| ())
}

/// Full schema/row inspection and request validation share one SQLite snapshot.
/// Acquire the worker's store gate first: an exhausted cursor does not release
/// its enclosing transaction's SHARED lock, and a writer waiting to commit can
/// otherwise time out or prevent a later admission from acquiring its snapshot.
pub(super) fn read_snapshot<T>(
    data_dir: &Path,
    read: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    let path = canonical_parent(&data_dir.join(DATABASE))?;
    let access = store_access(
        path.parent()
            .ok_or("The database has no parent directory")?,
    );
    let _access = access.lock();
    let mut connection = open_readonly_canonical(&path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    current_schema(&transaction)?;
    read(&transaction)
}
/// Recheck exact borrowed provider authority immediately before runner spawn.
pub fn validate_command_callback(
    data_dir: &Path,
    claim: &WorkClaim,
    prepared: &crate::workspace::runner::Prepared,
) -> Result<(), String> {
    read_snapshot(data_dir, |connection| {
        super::work::validate_nested_command(connection, claim, prepared)
    })
}

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
    read_snapshot(data_dir, |transaction| {
        validate_committed_task(transaction, expected)?;
        let retained = excerpts(transaction, expected.id)?;
        for (index, excerpt) in selected.iter().enumerate() {
            excerpt.validate()?;
            if !excerpt.available
                || excerpt.task_id != expected.id
                || excerpt.id <= 0
                || selected[..index].iter().any(|other| other.id == excerpt.id)
                || !retained.iter().any(|stored| stored == excerpt)
            {
                return Err(
                    "Selected context changed or was purged; reload and preview again".into(),
                );
            }
        }
        Ok(())
    })
}

/// Claimed admission keeps shared facts and selected captures immutable while the
/// UI may save routing, drafts, navigation and a future command independently.
pub fn validate_work_input(
    data_dir: &Path,
    expected: &Snapshot,
    selected: &[crate::workspace::context::Excerpt],
    claim: &WorkClaim,
) -> Result<(), String> {
    validate(expected)?;
    if selected.len() > crate::workspace::context::MAX_EXCERPTS {
        return Err("Planning context exceeds the excerpt limit".into());
    }
    read_snapshot(data_dir, |transaction| {
        validate_work_task(transaction, expected, claim)?;
        let retained = excerpts(transaction, expected.id)?;
        for (index, excerpt) in selected.iter().enumerate() {
            excerpt.validate()?;
            if !excerpt.available
                || excerpt.task_id != expected.id
                || excerpt.id <= 0
                || selected[..index].iter().any(|other| other.id == excerpt.id)
                || !retained.iter().any(|stored| stored == excerpt)
            {
                return Err(
                    "Selected context changed or was purged; reload and preview again".into(),
                );
            }
        }
        Ok(())
    })
}

pub(super) fn shared_facts_equal(current: &Snapshot, expected: &Snapshot) -> bool {
    current.id == expected.id
        && current.project == expected.project
        && current.goal == expected.goal
        && current.criteria == expected.criteria
        && current.next_action == expected.next_action
        && current.complete == expected.complete
        && current.archived == expected.archived
        && current.archived_at == expected.archived_at
        && current.workflows.hypotheses == expected.workflows.hypotheses
        && current.workflows.refactor == expected.workflows.refactor
        && current.workflows.findings == expected.workflows.findings
        && current.workflows.review == expected.workflows.review
        && current.workflows.recipe == expected.workflows.recipe
        && current.workflows.chosen_decision == expected.workflows.chosen_decision
        && current.workflows.criterion_origins == expected.workflows.criterion_origins
        && current.approved_memories == expected.approved_memories
}

pub(super) fn validate_work_task(
    connection: &Connection,
    expected: &Snapshot,
    claim: &WorkClaim,
) -> Result<(), String> {
    super::work::validate_live(connection, claim)?;
    if expected.id != claim.task_id
        || expected.id <= 0
        || expected.archived
        || expected.goal.trim().is_empty()
    {
        return Err("Claimed request does not match its saved active task".into());
    }
    let current = load_task(connection, &expected.project, expected.id)?
        .ok_or("The previewed task no longer exists in this project")?;
    if !shared_facts_equal(&current, expected) {
        return Err(
            "Shared task facts changed since preview; save and review a new request".into(),
        );
    }
    crate::workspace::memory::revalidate(
        &expected.approved_memories,
        &memories(connection, &expected.project, expected.id)?,
        &expected.project,
        expected.id,
    )
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
    work: Option<&WorkClaim>,
) -> Result<Vec<crate::workspace::runner::RunRecord>, String> {
    validate(task)?;
    if task.id <= 0 || task.archived {
        return Err("Review observations require a committed active task".into());
    }
    read_snapshot(data_dir, |transaction| {
        if let Some(claim) = work {
            validate_work_task(transaction, task, claim)?;
        } else {
            validate_committed_task(transaction, task)?;
        }
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::RecvTimeoutError;

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "ev-admission-{}-{stamp}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn next(worker: &Worker) -> Event {
        worker
            .events
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
    }

    fn fixture(directory: &Directory) -> (Worker, Snapshot, WorkClaim) {
        let worker =
            Worker::start(directory.0.clone(), "project".into(), Policy::default()).unwrap();
        assert!(matches!(next(&worker), Event::Loaded(_)));
        let mut pending = Some(Snapshot {
            goal: "Generate files from the retained plan".into(),
            revision: 1,
            ..Snapshot::empty("project".into())
        });
        assert!(worker.try_save(&mut pending));
        let Event::Saved(task) = next(&worker) else {
            panic!("expected a saved request");
        };
        let claim = WorkClaim {
            task_id: task.id,
            owner: Attribution::of(&Partner::default()),
            request_id: "admission-lock-regression".into(),
            writing: true,
        };
        (worker, task, claim)
    }

    fn queue_claim(worker: &Worker, task: &Snapshot, claim: &WorkClaim) {
        assert!(worker.try_work(&mut Some(WorkRequest::Acquire {
            task: Box::new(task.clone()),
            claim: claim.clone(),
        })));
    }

    fn while_snapshot_held(directory: &Directory, worker: &Worker, enqueue: impl FnOnce()) {
        let (ready, entered) = mpsc::channel();
        let (release, released) = mpsc::channel();
        thread::scope(|scope| {
            let reader = scope.spawn(move || {
                read_snapshot(&directory.0, |connection| {
                    // The cursor is exhausted; the enclosing transaction still
                    // owns SHARED while strict Rust-side validation continues.
                    connection
                        .query_row("SELECT count(*) FROM tasks", [], |row| row.get::<_, i64>(0))
                        .map_err(|error| error.to_string())?;
                    ready.send(()).unwrap();
                    released.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok(())
                })
            });
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
            enqueue();
            assert!(
                matches!(
                    worker
                        .events
                        .as_ref()
                        .unwrap()
                        .recv_timeout(BUSY_TIMEOUT * 2),
                    Err(RecvTimeoutError::Timeout)
                ),
                "the writer must wait before BEGIN, not expire its SQLite commit timeout"
            );
            release.send(()).unwrap();
            reader.join().unwrap().unwrap();
        });
    }

    #[test]
    fn long_admission_snapshot_orders_actual_claim_and_ui_save() {
        let directory = Directory::new();
        let (worker, task, claim) = fixture(&directory);
        while_snapshot_held(&directory, &worker, || queue_claim(&worker, &task, &claim));
        let Event::Work { result, .. } = next(&worker) else {
            panic!("expected the exact claim acknowledgement");
        };
        assert_eq!(result.unwrap(), vec![claim.clone()]);

        let mut future = task.clone();
        future.revision += 1;
        future.navigation.focus = 2;
        while_snapshot_held(&directory, &worker, || {
            assert!(worker.try_save(&mut Some(future)));
        });
        assert!(matches!(next(&worker), Event::Saved(_)));
        validate_work_input(&directory.0, &task, &[], &claim).unwrap();
        assert!(validate_planning_input(&directory.0, &task, &[]).is_err());
    }

    #[test]
    fn all_admission_paths_wait_before_opening_their_sqlite_snapshot() {
        let directory = Directory::new();
        let (worker, task, claim) = fixture(&directory);
        queue_claim(&worker, &task, &claim);
        assert!(matches!(next(&worker), Event::Work { result: Ok(_), .. }));
        let access = store_access(&fs::canonicalize(&directory.0).unwrap());
        let writer = access.lock();
        let mut connection = Connection::open(directory.0.join(DATABASE)).unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Exclusive)
            .unwrap();
        transaction
            .execute("UPDATE tasks SET updated_at=updated_at", [])
            .unwrap();
        let (ready, entered) = mpsc::channel();
        let (done, completed) = mpsc::channel();
        thread::scope(|scope| {
            for kind in 0..3 {
                let ready = ready.clone();
                let done = done.clone();
                let directory = &directory.0;
                let task = &task;
                let claim = &claim;
                scope.spawn(move || {
                    ready.send(()).unwrap();
                    let result = match kind {
                        0 => validate_planning_input(directory, task, &[]),
                        1 => validate_work_input(directory, task, &[], claim),
                        _ => review_run_observations(directory, task, Some(claim)).map(|_| ()),
                    };
                    done.send(result).unwrap();
                });
            }
            for _ in 0..3 {
                entered.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            assert!(
                matches!(
                    completed.recv_timeout(BUSY_TIMEOUT * 2),
                    Err(RecvTimeoutError::Timeout)
                ),
                "admissions must wait on the in-process gate, not fail SQLITE_BUSY"
            );
            transaction.commit().unwrap();
            drop(writer);
            for _ in 0..3 {
                completed
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
            }
        });
        assert!(worker.try_work(&mut Some(WorkRequest::Release(claim.clone()))));
        assert!(matches!(next(&worker), Event::Work { result: Ok(_), .. }));
        assert!(validate_work_input(&directory.0, &task, &[], &claim).is_err());
        assert!(review_run_observations(&directory.0, &task, Some(&claim)).is_err());
    }

    #[test]
    fn foreign_reader_preserves_bounded_busy_failure_and_claim_rollback() {
        let directory = Directory::new();
        let (worker, task, claim) = fixture(&directory);
        // An uncoordinated connection stands for a reader in another process.
        let mut connection = open_readonly(&directory.0.join(DATABASE)).unwrap();
        let reader = connection.transaction().unwrap();
        reader
            .query_row("SELECT count(*) FROM tasks", [], |row| row.get::<_, i64>(0))
            .unwrap();
        queue_claim(&worker, &task, &claim);
        let Event::Work { result, .. } = next(&worker) else {
            panic!("expected an external contention failure");
        };
        assert!(result.unwrap_err().contains("database is locked"));
        reader.commit().unwrap();
        assert!(super::super::work::claims(&connection).unwrap().is_empty());
        assert!(validate_work_input(&directory.0, &task, &[], &claim).is_err());
        queue_claim(&worker, &task, &claim);
        assert!(matches!(next(&worker), Event::Work { result: Ok(_), .. }));
    }

    #[test]
    fn store_gates_share_only_the_same_canonical_directory() {
        let first = Directory::new();
        let second = Directory::new();
        let first_path = fs::canonicalize(&first.0).unwrap();
        let access = store_access(&first_path);
        assert!(std::sync::Arc::ptr_eq(&access, &store_access(&first_path)));
        let _held = access.lock();
        let independent = store_access(&fs::canonicalize(&second.0).unwrap());
        assert!(independent.try_lock().is_some());
    }
}
