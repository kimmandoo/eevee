use super::*;
use crate::workspace::context::screenshot::Format;
use image::{ImageEncoder, codecs::png::PngEncoder};
use rusqlite::types::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn validate_selected(data_dir: &Path, attachment: &Attachment) -> Result<(), String> {
    validate_selection(data_dir, &attachment.capture.root, attachment)
}

struct Fixture {
    root: PathBuf,
    project: String,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let mut random = [0; 16];
        getrandom::getrandom(&mut random).unwrap();
        let path = std::env::temp_dir().join(format!(
            "ev-screenshots-{:032x}",
            u128::from_le_bytes(random)
        ));
        fs::create_dir(&path).unwrap();
        let path = fs::canonicalize(path).unwrap();
        let project = path.join("project");
        fs::create_dir(&project).unwrap();
        let data = project.join("local-state");
        fs::create_dir(&data).unwrap();
        Self {
            root: path,
            project: project.to_str().unwrap().into(),
            data,
        }
    }
    fn open(&self) -> State {
        self.open_data(&self.data)
    }
    fn open_data(&self, data: &Path) -> State {
        let mut state = State {
            data_dir: data.into(),
            project: self.project.clone(),
            policy: Policy::default(),
            connection: None,
            lock: None,
        };
        assert!(matches!(state.reopen(), Event::Loaded(_)));
        state
    }
    fn task(&self, state: &mut State) -> Snapshot {
        let mut task = Snapshot::empty(self.project.clone());
        task.goal = "Inspect the explicitly attached image".into();
        state.save(task).unwrap()
    }
    fn image(&self, task_id: i64, partner: i64, seed: u8, side: u32) -> Attachment {
        // Deliberately varied literal pixels exercise nontrivial quota growth.
        let mut n = u32::from(seed) + 1;
        let pixels: Vec<u8> = (0..side as usize * side as usize * 4)
            .map(|_| {
                n ^= n << 13;
                n ^= n >> 17;
                n ^= n << 5;
                n as u8
            })
            .collect();
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(&pixels, side, side, image::ExtendedColorType::Rgba8)
            .unwrap();
        let capture = Capture {
            task_id,
            partner,
            root: self.project.clone(),
            path: "screen.png".into(),
            sha256: Sha256::digest(&bytes).into(),
            format: Format::Png,
            width: side,
            height: side,
            bytes: bytes.len(),
        };
        Attachment {
            capture,
            bytes: bytes.into(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn request(task_id: i64, partner: i64, operation: Operation) -> Request {
    Request {
        task_id,
        partner,
        operation,
    }
}
fn apply(state: &mut State, image: &Attachment, operation: Operation) -> Response {
    state
        .screenshot_request(&request(
            image.capture.task_id,
            image.capture.partner,
            operation,
        ))
        .unwrap()
}
fn event(worker: &Worker) -> Event {
    worker
        .events
        .as_ref()
        .unwrap()
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
}
fn worker_apply(worker: &Worker, image: &Attachment, operation: Operation) -> Response {
    let mut pending = Some(request(
        image.capture.task_id,
        image.capture.partner,
        operation,
    ));
    assert!(worker.try_screenshot(&mut pending));
    assert!(pending.is_none());
    match event(worker) {
        Event::Screenshot { request, result } => {
            assert_eq!(
                (request.task_id, request.partner),
                (image.capture.task_id, image.capture.partner)
            );
            result.unwrap()
        }
        other => panic!("Expected screenshot acknowledgment, got {other:?}"),
    }
}

#[test]
fn worker_restart_restores_exact_selected_bytes_but_no_work_or_grants() {
    let fixture = Fixture::new();
    let worker = Worker::start(
        fixture.data.clone(),
        fixture.project.clone(),
        Policy::default(),
    )
    .unwrap();
    assert!(matches!(event(&worker), Event::Loaded(_)));
    let mut pending = Some(Snapshot::empty(fixture.project.clone()));
    assert!(worker.try_save(&mut pending));
    let task = match event(&worker) {
        Event::Saved(task) => task,
        other => panic!("{other:?}"),
    };
    let image = fixture.image(task.id, 1, 7, 8);
    assert_eq!(
        worker_apply(&worker, &image, Operation::Put(image.clone())).attachment,
        Some(image.clone())
    );
    validate_selected(&fixture.data, &image).unwrap();
    drop(worker);
    let worker = Worker::start(
        fixture.data.clone(),
        fixture.project.clone(),
        Policy::default(),
    )
    .unwrap();
    assert!(matches!(event(&worker), Event::Loaded(_)));
    let restored = worker_apply(&worker, &image, Operation::Load);
    assert_eq!(restored.attachment, Some(image.clone()));
    assert_eq!(restored.retained, Some(image.capture.clone()));
    let connection = open_readonly(&fixture.data.join(DATABASE)).unwrap();
    let active: i64 = connection
        .query_row("SELECT count(*) FROM active_work", [], |row| row.get(0))
        .unwrap();
    assert_eq!(active, 0);
    assert_eq!(
        connection
            .query_row::<Vec<u8>, _, _>("SELECT bytes FROM screenshots", [], |row| row.get(0))
            .unwrap(),
        image.bytes.as_ref()
    );
}

#[test]
fn removal_retains_restart_inspection_reselection_is_explicit_and_purge_removes_binary() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let image = fixture.image(task.id, 1, 8, 8);
    apply(&mut state, &image, Operation::Put(image.clone()));
    let removed = apply(&mut state, &image, Operation::Remove(image.capture.clone()));
    assert!(removed.attachment.is_none());
    assert_eq!(removed.retained, Some(image.capture.clone()));
    assert!(validate_selected(&fixture.data, &image).is_err());
    drop(state);
    let mut state = fixture.open();
    let removed = apply(&mut state, &image, Operation::Load);
    assert!(removed.attachment.is_none());
    assert_eq!(removed.retained, Some(image.capture.clone()));
    assert_eq!(
        apply(&mut state, &image, Operation::Put(image.clone())).attachment,
        Some(image.clone())
    );
    validate_selection(&fixture.data, &fixture.project, &image).unwrap();
    let purged = apply(&mut state, &image, Operation::Purge(image.capture.clone()));
    assert!(purged.attachment.is_none() && purged.retained.is_none());
    assert_eq!(
        state
            .connection
            .as_ref()
            .unwrap()
            .query_row::<i64, _, _>("SELECT count(*) FROM screenshots", [], |row| row.get(0))
            .unwrap(),
        0
    );
    assert!(validate_selected(&fixture.data, &image).is_err());
    drop(state);
    assert!(
        apply(&mut fixture.open(), &image, Operation::Load)
            .retained
            .is_none()
    );
}

#[test]
fn replacement_requires_exact_reviewed_purge_and_stale_mutations_preserve_new_image() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let first = fixture.image(task.id, 1, 1, 8);
    let second = fixture.image(task.id, 1, 2, 8);
    apply(&mut state, &first, Operation::Put(first.clone()));
    assert!(
        state
            .screenshot_request(&request(task.id, 1, Operation::Put(second.clone())))
            .is_err()
    );
    validate_selected(&fixture.data, &first).unwrap();
    apply(&mut state, &first, Operation::Remove(first.capture.clone()));
    assert!(
        state
            .screenshot_request(&request(task.id, 1, Operation::Put(second.clone())))
            .is_err()
    );
    apply(&mut state, &first, Operation::Purge(first.capture.clone()));
    apply(&mut state, &second, Operation::Put(second.clone()));
    let before = fs::read(fixture.data.join(DATABASE)).unwrap();
    for operation in [
        Operation::Remove(first.capture.clone()),
        Operation::Purge(first.capture.clone()),
    ] {
        assert!(
            state
                .screenshot_request(&request(task.id, 1, operation))
                .is_err()
        );
    }
    assert_eq!(fs::read(fixture.data.join(DATABASE)).unwrap(), before);
    let mut wrong_identity = second.capture.clone();
    wrong_identity.path = "same-bytes-different-source.png".into();
    for operation in [
        Operation::Remove(wrong_identity.clone()),
        Operation::Purge(wrong_identity),
    ] {
        assert!(
            state
                .screenshot_request(&request(task.id, 1, operation))
                .is_err()
        );
    }
    assert_eq!(fs::read(fixture.data.join(DATABASE)).unwrap(), before);
    assert!(validate_selected(&fixture.data, &first).is_err());
    validate_selected(&fixture.data, &second).unwrap();
    let connection = state.connection.as_ref().unwrap();
    let (count, bytes, metadata): (i64, Vec<u8>, String) = connection
        .query_row(
            "SELECT count(*),bytes,capture FROM screenshots",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(bytes, second.bytes.as_ref());
    let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(metadata["bytes"], second.bytes.len());
    assert!(!serde_json::to_string(&metadata).unwrap().contains("base64"));
    let mut changed = second.clone();
    changed.capture.path = "different.png".into();
    assert!(validate_selected(&fixture.data, &changed).is_err());
    assert!(validate_selection(&fixture.data, "/another-project", &second).is_err());
    changed = second.clone();
    changed.capture.partner = 999;
    assert!(validate_selected(&fixture.data, &changed).is_err());
    changed = second.clone();
    changed.bytes = first.bytes.clone();
    assert!(validate_selected(&fixture.data, &changed).is_err());
}

#[test]
fn quota_failure_preserves_existing_images_without_eviction_and_purge_frees_live_pages() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let partner = state.add_partner().unwrap().last().unwrap().id;
    let small = fixture.image(task.id, 1, 1, 4);
    let large = fixture.image(task.id, partner, 3, 128);
    apply(&mut state, &small, Operation::Put(small.clone()));
    let initial = evidence_usage(state.connection.as_ref().unwrap(), &fixture.data).unwrap();
    state.policy.quota_bytes = initial;
    assert!(
        state
            .screenshot_request(&request(task.id, partner, Operation::Put(large.clone())))
            .is_err()
    );
    assert!(
        apply(&mut state, &large, Operation::Load)
            .retained
            .is_none()
    );
    assert_eq!(
        apply(&mut state, &small, Operation::Load).attachment,
        Some(small.clone())
    );
    validate_selected(&fixture.data, &small).unwrap();
    state.policy = Policy::default();
    apply(&mut state, &large, Operation::Put(large.clone()));
    let retained = evidence_usage(state.connection.as_ref().unwrap(), &fixture.data).unwrap();
    assert!(retained > initial);
    apply(&mut state, &large, Operation::Remove(large.capture.clone()));
    assert!(evidence_usage(state.connection.as_ref().unwrap(), &fixture.data).unwrap() >= retained);
    // Purge remains possible even when policy is below current retained usage.
    state.policy.quota_bytes = 1;
    apply(&mut state, &large, Operation::Purge(large.capture.clone()));
    assert!(evidence_usage(state.connection.as_ref().unwrap(), &fixture.data).unwrap() < retained);
}

#[test]
fn project_application_storage_and_archival_boundaries_refuse_without_replacing() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let image = fixture.image(task.id, 1, 4, 4);
    apply(&mut state, &image, Operation::Put(image.clone()));
    let mut bad = image.clone();
    bad.capture.path = "local-state/screen.png".into();
    assert!(
        state
            .screenshot_request(&request(task.id, 1, Operation::Put(bad)))
            .is_err()
    );
    bad = image.clone();
    bad.capture.root = fixture.root.to_str().unwrap().into();
    assert!(
        state
            .screenshot_request(&request(task.id, 1, Operation::Put(bad)))
            .is_err()
    );
    assert!(
        state
            .screenshot_request(&request(task.id, 999, Operation::Load))
            .is_err()
    );
    state
        .manage_at(
            Management::Archive {
                id: task.id,
                archived: true,
            },
            timestamp().unwrap(),
        )
        .unwrap();
    assert!(validate_selected(&fixture.data, &image).is_err());
    assert!(
        state
            .screenshot_request(&request(task.id, 1, Operation::Put(image.clone())))
            .is_err()
    );
    // Inspection and explicit deletion are still available for archived tasks.
    assert_eq!(
        apply(&mut state, &image, Operation::Load).retained,
        Some(image.capture.clone())
    );
    apply(&mut state, &image, Operation::Purge(image.capture.clone()));
}

fn existing_rows(connection: &Connection) -> BTreeMap<String, Vec<Vec<Value>>> {
    let names = connection.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name!='screenshots' ORDER BY name").unwrap()
        .query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM {name} ORDER BY rowid"))
                .unwrap();
            let columns = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|i| row.get(i))
                        .collect::<rusqlite::Result<Vec<Value>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}

#[test]
fn genuine_schema_nineteen_migration_preserves_every_old_value_and_is_atomic() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let mut task = Snapshot::empty(fixture.project.clone());
    task.goal = "Unchanged historical task".into();
    let task = state.save(task).unwrap();
    drop(state);
    let mut connection = Connection::open(fixture.data.join(DATABASE)).unwrap();
    connection
        .execute_batch("DROP TABLE screenshots; PRAGMA user_version=19;")
        .unwrap();
    let before = existing_rows(&connection);
    assert!(matches!(
        super::super::schema::inspect(&connection).unwrap(),
        Schema::Current
    ));
    {
        let transaction = connection.transaction().unwrap();
        super::super::schema::migrate(&transaction, Schema::Current).unwrap();
        assert_eq!(
            transaction
                .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
                .unwrap(),
            20
        );
        assert_eq!(existing_rows(&transaction), before);
    }
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        19
    );
    assert!(connection.prepare("SELECT * FROM screenshots").is_err());
    connection.execute_batch("VACUUM;").unwrap();
    let pages: i64 = connection
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    connection
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    assert!(initialize(&mut connection).is_err());
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        19
    );
    assert_eq!(existing_rows(&connection), before);
    connection
        .pragma_update(None, "max_page_count", 1_000_000)
        .unwrap();
    initialize(&mut connection).unwrap();
    assert_eq!(existing_rows(&connection), before);
    assert_eq!(
        load_task(&connection, &fixture.project, task.id).unwrap(),
        Some(task)
    );
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        20
    );
    super::super::schema::inspect(&connection).unwrap();
}

#[test]
fn strict_schema_and_corrupt_capture_admission_refuse_without_changing_database_bytes() {
    for corruption in [
        "PRAGMA user_version=21;",
        "PRAGMA user_version=19;",
        "ALTER TABLE screenshots ADD COLUMN future_grant TEXT;",
        "UPDATE screenshots SET capture=json_set(capture,'$.future_grant',1);",
        "UPDATE screenshots SET capture=json_set(capture,'$.task_id',999);",
        "UPDATE screenshots SET partner=999;",
        "UPDATE screenshots SET capture=json_set(capture,'$.width',2);",
        "UPDATE screenshots SET bytes=zeroblob(length(bytes));",
        "PRAGMA ignore_check_constraints=ON; UPDATE screenshots SET selected=2;",
    ] {
        let fixture = Fixture::new();
        let mut state = fixture.open();
        let task = fixture.task(&mut state);
        let image = fixture.image(task.id, 1, 5, 4);
        apply(&mut state, &image, Operation::Put(image.clone()));
        drop(state);
        let connection = Connection::open(fixture.data.join(DATABASE)).unwrap();
        // Deliberately forge orphaned rows; production admission must reject
        // them even when an external writer disabled relational constraints.
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .unwrap();
        connection.execute_batch(corruption).unwrap();
        drop(connection);
        let before = fs::read(fixture.data.join(DATABASE)).unwrap();
        assert!(
            validate_selected(&fixture.data, &image).is_err(),
            "{corruption}"
        );
        assert!(
            preflight(&fixture.data.join(DATABASE)).is_err(),
            "{corruption}"
        );
        assert!(
            inspect_lifecycle_data(&fixture.data).is_err(),
            "{corruption}"
        );
        assert_eq!(
            fs::read(fixture.data.join(DATABASE)).unwrap(),
            before,
            "{corruption}"
        );
    }
}

#[test]
fn full_directory_backup_restores_retained_blob_task_export_does_not_duplicate_it() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let image = fixture.image(task.id, 1, 6, 16);
    apply(&mut state, &image, Operation::Put(image.clone()));
    apply(&mut state, &image, Operation::Remove(image.capture.clone()));
    let export = fixture.root.join("tasks.json");
    export_readable(&fixture.data, &export).unwrap();
    let exported = fs::read_to_string(export).unwrap();
    assert!(!exported.contains("base64"));
    assert!(!exported.contains("screen.png"));
    drop(state);
    let backup = fixture.root.join("backup");
    let restored = fixture.root.join("restored");
    super::super::snapshot::create(&fixture.data, &backup).unwrap();
    super::super::snapshot::restore(&backup, &restored).unwrap();
    let mut state = fixture.open_data(&restored);
    let response = apply(&mut state, &image, Operation::Load);
    assert!(response.attachment.is_none());
    assert_eq!(response.retained, Some(image.capture.clone()));
    assert_eq!(
        state
            .connection
            .as_ref()
            .unwrap()
            .query_row::<Vec<u8>, _, _>("SELECT bytes FROM screenshots", [], |row| row.get(0))
            .unwrap(),
        image.bytes.as_ref()
    );
}

#[test]
fn task_deletion_and_partner_release_remove_their_owned_screenshot_blobs_only() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let other = fixture.task(&mut state);
    let partner = state.add_partner().unwrap().last().unwrap().id;
    let first = fixture.image(task.id, 1, 1, 8);
    let second = fixture.image(other.id, partner, 2, 8);
    apply(&mut state, &first, Operation::Put(first.clone()));
    apply(&mut state, &second, Operation::Put(second.clone()));
    state
        .manage_at(
            Management::Delete {
                ids: vec![task.id],
                expired_before: None,
            },
            timestamp().unwrap(),
        )
        .unwrap();
    assert!(validate_selected(&fixture.data, &first).is_err());
    validate_selected(&fixture.data, &second).unwrap();
    assert_eq!(
        apply(&mut state, &second, Operation::Load).attachment,
        Some(second.clone())
    );
    state.release_partner(partner).unwrap();
    assert_eq!(
        state
            .connection
            .as_ref()
            .unwrap()
            .query_row::<i64, _, _>("SELECT count(*) FROM screenshots", [], |row| row.get(0))
            .unwrap(),
        0
    );
    super::super::schema::inspect(state.connection.as_ref().unwrap()).unwrap();
}

#[test]
fn existing_work_ownership_blocks_selection_mutation_and_restart_never_replays_it() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let image = fixture.image(task.id, 1, 9, 4);
    apply(&mut state, &image, Operation::Put(image.clone()));
    let owner = Attribution::of(&partners(state.connection.as_ref().unwrap()).unwrap()[0]);
    let claim = WorkClaim {
        task_id: task.id,
        owner,
        request_id: "screenshot-work".into(),
        writing: false,
    };
    state
        .work_request(&WorkRequest::Acquire {
            claim,
            task: Box::new(task),
        })
        .unwrap();
    for operation in [
        Operation::Remove(image.capture.clone()),
        Operation::Purge(image.capture.clone()),
        Operation::Put(image.clone()),
    ] {
        assert!(
            state
                .screenshot_request(&request(image.capture.task_id, 1, operation))
                .is_err()
        );
    }
    assert_eq!(
        apply(&mut state, &image, Operation::Load).attachment,
        Some(image.clone())
    );
    validate_selected(&fixture.data, &image).unwrap();
    drop(state);
    let mut state = fixture.open();
    assert_eq!(
        state
            .connection
            .as_ref()
            .unwrap()
            .query_row::<i64, _, _>("SELECT count(*) FROM active_work", [], |row| row.get(0))
            .unwrap(),
        0
    );
    assert_eq!(
        apply(&mut state, &image, Operation::Load).attachment,
        Some(image.clone())
    );
    apply(&mut state, &image, Operation::Remove(image.capture.clone()));
}

#[test]
fn deletion_preview_lists_exact_selected_and_removed_images_and_excludes_purged() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = fixture.task(&mut state);
    let partner = state.add_partner().unwrap().last().unwrap().id;
    let first = fixture.image(task.id, 1, 1, 4);
    let second = fixture.image(task.id, partner, 2, 4);
    apply(&mut state, &first, Operation::Put(first.clone()));
    apply(&mut state, &second, Operation::Put(second.clone()));
    apply(
        &mut state,
        &second,
        Operation::Remove(second.capture.clone()),
    );
    let targets = state.preview_delete(&[task.id], None).unwrap();
    assert_eq!(
        targets[0].screenshots,
        vec![
            (first.capture.clone(), true),
            (second.capture.clone(), true)
        ]
    );
    apply(&mut state, &first, Operation::Purge(first.capture.clone()));
    let targets = state.preview_delete(&[task.id], None).unwrap();
    assert_eq!(targets[0].screenshots, vec![(second.capture.clone(), true)]);
}
