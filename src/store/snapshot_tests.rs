use super::*;
use crate::workspace::runner::{
    CommandSpec, ExecutableIdentity, InputSnapshot, Prepared, RunRecord, RunState, TreeOutcome,
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).unwrap();
        let path = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("ev-snapshot-{:032x}", u128::from_le_bytes(random)));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn source(&self) -> PathBuf {
        self.0.join("source")
    }
    fn backup(&self) -> PathBuf {
        self.0.join("backup")
    }
    fn restored(&self) -> PathBuf {
        self.0.join("restored")
    }
    fn state_at(&self, directory: PathBuf) -> State {
        let mut state = State {
            data_dir: directory,
            project: self.0.to_str().unwrap().into(),
            policy: Policy::default(),
            connection: None,
            lock: None,
        };
        let event = state.reopen();
        assert!(matches!(event, Event::Loaded(_)), "{event:?}");
        state
    }
    fn populated(&self) -> (Snapshot, String) {
        let mut state = self.state_at(self.source());
        let mut task = Snapshot::empty(state.project.clone());
        task.goal = "Original snapshot task".into();
        task.revision = 1;
        task.partner_drafts.insert(
            1,
            PartnerDrafts {
                drafts: vec![Draft {
                    field: EditorField::Request,
                    text: "Unsent exact draft 東京".into(),
                    cursor: 4,
                }],
                ..PartnerDrafts::default()
            },
        );
        let task = state.save(task).unwrap();
        let archived = state
            .save({
                let mut t = Snapshot::empty(state.project.clone());
                t.goal = "Archived history".into();
                t.revision = 1;
                t
            })
            .unwrap();
        state
            .manage_at(
                Management::Archive {
                    id: archived.id,
                    archived: true,
                },
                123,
            )
            .unwrap();
        let owner = Attribution::of(&partners(state.connection.as_ref().unwrap()).unwrap()[0]);
        let claim = WorkClaim {
            task_id: task.id,
            owner,
            request_id: "snapshot-request".into(),
            writing: false,
        };
        state
            .work_request(&WorkRequest::Acquire {
                claim: claim.clone(),
                task: Box::new(task.clone()),
            })
            .unwrap();
        let outgoing = crate::workspace::receipt::Outgoing::frozen(
            "{\"purpose\":\"retained source\"}",
            &claim,
            1,
            crate::providers::Agent::Codex,
            crate::providers::Operation::Plan,
            1234,
        )
        .unwrap();
        state
            .receipts_request(&crate::store::receipts::Request {
                task_id: task.id,
                partner: 1,
                operation: crate::store::receipts::Operation::Put(Box::new(outgoing)),
            })
            .unwrap();
        state.work_request(&WorkRequest::Release(claim)).unwrap();
        let prepared_at_ms = u64::try_from(timestamp().unwrap()).unwrap();
        let prepared = Prepared {
            approval_arrival: 0,
            request_id: "snapshot-run-request".into(),
            run_id: "snapshot-run".into(),
            task_id: task.id,
            spec: CommandSpec::Direct {
                program: "echo".into(),
                args: vec!["retained".into()],
            },
            cwd: self.0.clone(),
            cwd_identity: "fixture-directory".into(),
            executable: ExecutableIdentity {
                path: self.0.join("fixture"),
                sha256: "0".repeat(64),
                bytes: 1,
                modified_ns: 1,
                file_id: "fixture".into(),
            },
            prepared_at_ms,
            expires_at_ms: prepared_at_ms + 60_000,
            before: InputSnapshot::default(),
            partner: None,
            rerun_of: None,
        };
        let mut permit = state.reserve_run(task.id, prepared.clone(), 4096).unwrap();
        permit
            .file
            .write_all(b"retained artifact output\n")
            .unwrap();
        permit.file.sync_all().unwrap();
        drop(permit);
        let mut record = RunRecord::awaiting(&prepared);
        record.state = RunState::Succeeded;
        record.started_at_ms = Some(10);
        record.ended_at_ms = Some(20);
        record.exit_code = Some(0);
        record.tree = TreeOutcome::OwnedTreeExited;
        record.captured_bytes = 25;
        state.update_run(task.id, record).unwrap();
        let name = spool_name(&prepared.run_id).unwrap();
        drop(state);
        fs::create_dir(self.source().join("saved-config")).unwrap();
        fs::write(
            self.source().join("saved-config/private.json"),
            b"app-owned private configuration",
        )
        .unwrap();
        (task, name)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn source_bytes(path: &Path) -> Vec<Member> {
    source_bytes_owned(path, None)
}

fn source_bytes_owned(path: &Path, owned_lock: Option<&File>) -> Vec<Member> {
    let mut members = Vec::new();
    inventory(
        &Directory::open(path).unwrap(),
        "",
        &mut members,
        None,
        owned_lock,
    )
    .unwrap();
    members
}

#[test]
fn complete_snapshot_restore_preserves_task_draft_receipt_run_artifact_and_archives_without_dispatch()
 {
    let fixture = Fixture::new();
    let (task, name) = fixture.populated();
    let before = source_bytes(&fixture.source());
    let manifest = create(&fixture.source(), &fixture.backup()).unwrap();
    assert_eq!(verify(&fixture.backup()).unwrap().members, manifest.members);
    assert_eq!(source_bytes(&fixture.source()), before);
    let backup_before = source_bytes(&fixture.backup());
    restore(&fixture.backup(), &fixture.restored()).unwrap();
    assert_eq!(source_bytes(&fixture.backup()), backup_before);
    assert_eq!(source_bytes(&fixture.source()), before);
    let mut restored = fixture.state_at(fixture.restored());
    let retained = load_task(
        restored.connection.as_ref().unwrap(),
        &task.project,
        task.id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(retained, task);
    assert_eq!(retained.request_text(), "Unsent exact draft 東京");
    assert!(
        work::claims(restored.connection.as_ref().unwrap())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        restored
            .read_output(task.id, "snapshot-run", 0, 4096)
            .unwrap()
            .text,
        "retained artifact output\n"
    );
    let runs = restored.load_runs(task.id).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].state, RunState::Succeeded);
    let connection = restored.connection.as_ref().unwrap();
    assert_eq!(
        connection
            .query_row::<i64, _, _>("SELECT count(*) FROM outgoing_receipts", [], |r| r.get(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row::<i64, _, _>("SELECT count(*) FROM tool_artifacts", [], |r| r.get(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row::<i64, _, _>("SELECT count(*) FROM tasks WHERE archived=1", [], |r| r
                .get(0))
            .unwrap(),
        1
    );
    assert_eq!(
        fs::read(fixture.restored().join(name)).unwrap(),
        b"retained artifact output\n"
    );
    assert_eq!(
        fs::read(fixture.restored().join("saved-config/private.json")).unwrap(),
        b"app-owned private configuration"
    );
    let artifact = match restored
        .artifact_request(&crate::workspace::artifacts::Request {
            request_id: 1,
            task_id: task.id,
            operation: crate::workspace::artifacts::Operation::Load,
        })
        .unwrap()
    {
        crate::workspace::artifacts::Response::Records(records) => records[0].clone(),
        other => panic!("Unexpected artifact response: {other:?}"),
    };
    assert!(
        matches!(restored.artifact_request(&crate::workspace::artifacts::Request {
        request_id: 2, task_id: task.id, operation: crate::workspace::artifacts::Operation::Read {
            handle: artifact.handle, offset: 0, limit: 4096,
        },
    }).unwrap(), crate::workspace::artifacts::Response::Page(page) if page.text == "retained artifact output\n")
    );
    assert!(
        matches!(restored.receipts_request(&crate::store::receipts::Request {
        task_id: task.id, partner: 1, operation: crate::store::receipts::Operation::ReadSource { request_id: "snapshot-request".into() },
    }).unwrap(), crate::store::receipts::Response::Source { text, .. } if text == "{\"purpose\":\"retained source\"}")
    );
}

#[test]
fn snapshot_preserves_every_owned_writer_lock_byte_and_restores_a_usable_store() {
    let fixture = Fixture::new();
    let (task, _) = fixture.populated();
    // Cover nonempty bytes across multiple copy buffers, not just the usual
    // empty lock file. No locked region may be skipped, zeroed or rewritten.
    let lock_bytes: Vec<u8> = (0u8..=255).cycle().take(128 * 1024 + 17).collect();
    fs::write(fixture.source().join(LOCK), &lock_bytes).unwrap();
    let before = source_bytes(&fixture.source());
    let manifest = create(&fixture.source(), &fixture.backup()).unwrap();
    assert_eq!(manifest.members, before);
    assert_eq!(verify(&fixture.backup()).unwrap(), manifest);
    let index = manifest
        .members
        .iter()
        .position(|m| m.path == LOCK)
        .unwrap();
    assert_eq!(
        fs::read(fixture.backup().join(payload(index))).unwrap(),
        lock_bytes
    );
    assert_eq!(source_bytes(&fixture.source()), before);
    let backup_before = source_bytes(&fixture.backup());
    restore(&fixture.backup(), &fixture.restored()).unwrap();
    assert_eq!(fs::read(fixture.restored().join(LOCK)).unwrap(), lock_bytes);
    assert_eq!(source_bytes(&fixture.source()), before);
    assert_eq!(source_bytes(&fixture.backup()), backup_before);
    let restored = fixture.state_at(fixture.restored());
    let retained = load_task(
        restored.connection.as_ref().unwrap(),
        &task.project,
        task.id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(retained, task);
}

#[test]
fn backup_precedes_migration_and_newer_reader_refusal_never_touches_source_or_backup() {
    let fixture = Fixture::new();
    fixture.populated();
    let connection = Connection::open(fixture.source().join(DATABASE)).unwrap();
    connection.pragma_update(None, "user_version", 999).unwrap();
    drop(connection);
    let before = source_bytes(&fixture.source());
    create(&fixture.source(), &fixture.backup()).unwrap();
    let backup = source_bytes(&fixture.backup());
    assert!(restore(&fixture.backup(), &fixture.restored()).is_err());
    assert_eq!(source_bytes(&fixture.source()), before);
    assert_eq!(source_bytes(&fixture.backup()), backup);
    assert!(fixture.restored().join(INCOMPLETE).exists());
    let mut state = State {
        data_dir: fixture.restored(),
        project: "project".into(),
        policy: Policy::default(),
        lock: None,
        connection: None,
    };
    assert!(
        matches!(state.reopen(), Event::Failed { message, .. } if message.contains("incomplete restore"))
    );
}

#[test]
fn older_backup_keeps_original_schema_until_restored_reader_migrates_fresh_copy() {
    let fixture = Fixture::new();
    drop(fixture.state_at(fixture.source()));
    let connection = Connection::open(fixture.source().join(DATABASE)).unwrap();
    remove_context_schema(&connection);
    connection.pragma_update(None, "user_version", 18).unwrap();
    drop(connection);
    let before = source_bytes(&fixture.source());
    create(&fixture.source(), &fixture.backup()).unwrap();
    restore(&fixture.backup(), &fixture.restored()).unwrap();
    assert_eq!(source_bytes(&fixture.source()), before);
    assert_eq!(
        open_readonly(&fixture.source().join(DATABASE))
            .unwrap()
            .pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        18
    );
    assert_eq!(
        open_readonly(&fixture.restored().join(DATABASE))
            .unwrap()
            .pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        19
    );
}

#[test]
fn live_owner_and_destination_collisions_are_create_only() {
    let fixture = Fixture::new();
    let state = fixture.state_at(fixture.source());
    // Observe live ownership through its actual handle: Windows mandatory locks
    // intentionally reject reads from a second handle, including in this process.
    let before = source_bytes_owned(&fixture.source(), state.lock.as_ref());
    assert!(
        create(&fixture.source(), &fixture.backup())
            .unwrap_err()
            .contains("Live owner")
    );
    assert!(!fixture.backup().exists());
    assert_eq!(
        source_bytes_owned(&fixture.source(), state.lock.as_ref()),
        before
    );
    drop(state);
    create(&fixture.source(), &fixture.backup()).unwrap();
    let backup = source_bytes(&fixture.backup());
    assert!(create(&fixture.source(), &fixture.backup()).is_err());
    fs::create_dir(fixture.restored()).unwrap();
    fs::write(fixture.restored().join("current-work"), b"keep").unwrap();
    assert!(restore(&fixture.backup(), &fixture.restored()).is_err());
    assert_eq!(
        fs::read(fixture.restored().join("current-work")).unwrap(),
        b"keep"
    );
    assert_eq!(source_bytes(&fixture.source()), before);
    assert_eq!(source_bytes(&fixture.backup()), backup);
}

#[test]
fn missing_owner_lock_is_not_created_by_snapshot() {
    let fixture = Fixture::new();
    drop(fixture.state_at(fixture.source()));
    fs::remove_file(fixture.source().join(LOCK)).unwrap();
    let before = source_bytes(&fixture.source());
    assert!(
        create(&fixture.source(), &fixture.backup())
            .unwrap_err()
            .contains("existing writer lock")
    );
    assert_eq!(source_bytes(&fixture.source()), before);
    assert!(!fixture.backup().exists());
}

#[test]
fn corruption_missing_truncation_unknown_version_traversal_and_unexpected_members_refuse() {
    for alteration in [
        "changed",
        "missing",
        "truncated",
        "version",
        "traversal",
        "absolute",
        "windows",
        "unexpected",
    ] {
        let fixture = Fixture::new();
        fixture.populated();
        let manifest = create(&fixture.source(), &fixture.backup()).unwrap();
        let index = manifest
            .members
            .iter()
            .position(|m| m.path == DATABASE)
            .unwrap();
        let member = fixture.backup().join(payload(index));
        match alteration {
            "changed" => {
                let mut bytes = fs::read(&member).unwrap();
                bytes[0] ^= 1;
                fs::write(&member, bytes).unwrap();
            }
            "missing" => fs::remove_file(&member).unwrap(),
            "truncated" => {
                OpenOptions::new()
                    .write(true)
                    .open(&member)
                    .unwrap()
                    .set_len(3)
                    .unwrap();
            }
            "unexpected" => fs::write(fixture.backup().join("extra"), b"unmanifested").unwrap(),
            _ => {
                let mut value = serde_json::to_value(manifest).unwrap();
                if alteration == "version" {
                    value["format"] = serde_json::json!(999);
                } else {
                    value["members"][index]["path"] = serde_json::json!(match alteration {
                        "traversal" => "../escaped",
                        "absolute" => "/escaped",
                        _ => "C:\\escaped",
                    });
                }
                fs::write(
                    fixture.backup().join(MANIFEST),
                    serde_json::to_vec(&value).unwrap(),
                )
                .unwrap();
            }
        }
        let before = source_bytes(&fixture.source());
        assert!(verify(&fixture.backup()).is_err(), "{alteration}");
        assert!(
            restore(&fixture.backup(), &fixture.restored()).is_err(),
            "{alteration}"
        );
        assert!(!fixture.restored().exists(), "{alteration}");
        assert_eq!(source_bytes(&fixture.source()), before);
    }
}

#[test]
fn incomplete_copies_are_never_backups_or_usable_app_data_and_failed_writes_are_errors() {
    let fixture = Fixture::new();
    fixture.populated();
    let root = fresh(&fixture.backup(), &fixture.source()).unwrap();
    let mut output = root.anchor.create_member(&payload(0)).unwrap();
    output.write_all(b"partial").unwrap();
    output.sync_all().unwrap();
    assert!(
        verify(&fixture.backup())
            .unwrap_err()
            .contains("incomplete")
    );
    assert!(restore(&fixture.backup(), &fixture.restored()).is_err());
    let read_only = File::open(fixture.source().join(DATABASE)).unwrap();
    assert!(
        digest(
            &File::open(fixture.source().join(DATABASE)).unwrap(),
            Some(read_only)
        )
        .is_err()
    );
    assert!(!fixture.backup().join(MANIFEST).exists());
}

#[test]
fn historical_claims_restore_as_unknown_and_all_active_ownership_is_retired() {
    let fixture = Fixture::new();
    let (task, _) = fixture.populated();
    let connection = Connection::open(fixture.source().join(DATABASE)).unwrap();
    let mut record: RunRecord = serde_json::from_str(
        &connection
            .query_row::<String, _, _>("SELECT record FROM executions", [], |r| r.get(0))
            .unwrap(),
    )
    .unwrap();
    record.state = RunState::Running;
    record.ended_at_ms = None;
    record.exit_code = None;
    record.tree = TreeOutcome::OwnershipUnknown;
    connection
        .execute(
            "UPDATE executions SET record=?1,reserved=1",
            [run_json(&record).unwrap()],
        )
        .unwrap();
    let owner = Attribution::of(&partners(&connection).unwrap()[0]);
    connection.execute("INSERT INTO active_work(task_id,partner,request_id,owner,writing,kind) VALUES(?1,1,'historical-provider',?2,1,'provider')",
        params![task.id, serde_json::to_string(&owner).unwrap()]).unwrap();
    drop(connection);
    create(&fixture.source(), &fixture.backup()).unwrap();
    restore(&fixture.backup(), &fixture.restored()).unwrap();
    let state = fixture.state_at(fixture.restored());
    let records = state.load_runs(task.id).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state, RunState::Interrupted);
    assert_eq!(records[0].tree, TreeOutcome::OwnershipUnknown);
    assert_eq!(records[0].exit_code, None);
    assert!(
        work::claims(state.connection.as_ref().unwrap())
            .unwrap()
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn symlinks_hardlinks_and_directory_links_refuse_without_following_targets() {
    use std::os::unix::fs::symlink;
    for link in ["file", "directory", "hard"] {
        let fixture = Fixture::new();
        fixture.populated();
        fs::write(fixture.0.join("outside"), b"outside-secret").unwrap();
        let target = fixture.source().join("unsafe");
        match link {
            "file" => symlink(fixture.0.join("outside"), &target).unwrap(),
            "directory" => symlink(&fixture.0, &target).unwrap(),
            _ => fs::hard_link(fixture.0.join("outside"), &target).unwrap(),
        }
        assert!(create(&fixture.source(), &fixture.backup()).is_err());
        assert!(verify(&fixture.backup()).is_err());
        assert_eq!(
            fs::read(fixture.0.join("outside")).unwrap(),
            b"outside-secret"
        );
    }
    let fixture = Fixture::new();
    fixture.populated();
    let manifest = create(&fixture.source(), &fixture.backup()).unwrap();
    let index = manifest
        .members
        .iter()
        .position(|m| m.path == DATABASE)
        .unwrap();
    let member = fixture.backup().join(payload(index));
    fs::remove_file(&member).unwrap();
    symlink(fixture.source().join(DATABASE), &member).unwrap();
    assert!(verify(&fixture.backup()).is_err());
    assert!(restore(&fixture.backup(), &fixture.restored()).is_err());
}

#[test]
fn disk_full_and_mid_copy_source_changes_cannot_finish_integrity_copy() {
    struct DiskFull;
    impl Write for DiskFull {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "deterministic disk-full boundary",
            ))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    struct TruncateSource(File);
    impl Write for TruncateSource {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.set_len(1)?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let fixture = Fixture::new();
    let path = fixture.0.join("member");
    fs::write(&path, vec![b'x'; 128 * 1024]).unwrap();
    assert!(
        digest_to(&File::open(&path).unwrap(), Some(&mut DiskFull))
            .unwrap_err()
            .contains("disk-full")
    );
    assert_eq!(fs::metadata(&path).unwrap().len(), 128 * 1024);
    let writable = OpenOptions::new().write(true).open(&path).unwrap();
    assert!(
        digest_to(
            &File::open(&path).unwrap(),
            Some(&mut TruncateSource(writable))
        )
        .unwrap_err()
        .contains("changed")
    );
}

#[test]
fn incompatible_serialized_reader_refuses_even_when_sql_schema_is_current() {
    let fixture = Fixture::new();
    drop(fixture.state_at(fixture.source()));
    let connection = Connection::open(fixture.source().join(DATABASE)).unwrap();
    connection
        .execute(
            "INSERT INTO provider_preferences(id,record) VALUES(1,?1)",
            [r#"{"provider":"Codex","selections":{},"text_density":"future-density"}"#],
        )
        .unwrap();
    drop(connection);
    let before = source_bytes(&fixture.source());
    create(&fixture.source(), &fixture.backup()).unwrap();
    assert!(
        verify(&fixture.backup()).is_ok(),
        "raw integrity must not claim reader compatibility"
    );
    let backup_before = source_bytes(&fixture.backup());
    assert!(restore(&fixture.backup(), &fixture.restored()).is_err());
    assert_eq!(source_bytes(&fixture.source()), before);
    assert_eq!(source_bytes(&fixture.backup()), backup_before);
    assert!(fixture.restored().join(INCOMPLETE).exists());
}

#[test]
fn snapshot_and_restored_files_are_private_and_stale_original_spools_are_not_rebound() {
    let fixture = Fixture::new();
    let (task, name) = fixture.populated();
    // Replace an original inode while retaining the bytes: stale evidence must remain unavailable.
    fs::rename(
        fixture.source().join(&name),
        fixture.0.join("original-spool"),
    )
    .unwrap();
    fs::write(fixture.source().join(&name), b"retained artifact output\n").unwrap();
    create(&fixture.source(), &fixture.backup()).unwrap();
    restore(&fixture.backup(), &fixture.restored()).unwrap();
    let state = fixture.state_at(fixture.restored());
    assert!(
        state
            .read_output(task.id, "snapshot-run", 0, 4096)
            .unwrap()
            .unavailable
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(fixture.backup()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(fixture.restored())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        for path in [
            fixture.backup().join(MANIFEST),
            fixture.backup().join(payload(0)),
            fixture.restored().join(DATABASE),
        ] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

#[test]
fn complete_wal_and_shm_bytes_restore_without_opening_the_source_with_sqlite() {
    let fixture = Fixture::new();
    drop(fixture.state_at(fixture.source()));
    let connection = Connection::open(fixture.source().join(DATABASE)).unwrap();
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    connection
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    connection
        .execute(
            "INSERT INTO provider_preferences(id,record) VALUES(1,?1)",
            [r#"{"provider":"Codex","selections":{},"text_density":"comfortable"}"#],
        )
        .unwrap();
    let before = source_bytes(&fixture.source());
    let manifest = create(&fixture.source(), &fixture.backup()).unwrap();
    assert!(
        manifest
            .members
            .iter()
            .any(|m| m.path == "workbench.sqlite3-wal")
    );
    assert!(
        manifest
            .members
            .iter()
            .any(|m| m.path == "workbench.sqlite3-shm")
    );
    let backup_before = source_bytes(&fixture.backup());
    restore(&fixture.backup(), &fixture.restored()).unwrap();
    let restored = open_readonly(&fixture.restored().join(DATABASE)).unwrap();
    let preference: String = restored
        .query_row("SELECT record FROM provider_preferences", [], |r| r.get(0))
        .unwrap();
    assert!(preference.contains("comfortable"));
    assert_eq!(source_bytes(&fixture.source()), before);
    assert_eq!(source_bytes(&fixture.backup()), backup_before);
    drop(connection);
}

#[test]
fn manifest_path_validation_is_portable_and_refuses_devices_streams_and_ambiguous_names() {
    for name in [
        "../outside",
        "/outside",
        "C:/outside",
        "dir\\\\outside",
        "CON",
        "nul.txt",
        "COM¹",
        "LPT3.log",
        "file:stream",
        "trailing.",
        "trailing ",
        "dir//name",
    ] {
        assert!(member_path(name).is_err(), "{name}");
    }
    for name in [
        "private/config.json",
        "東京/draft.txt",
        "workbench.sqlite3",
        "ev-run-owned.output",
    ] {
        assert!(member_path(name).is_ok(), "{name}");
    }
}
