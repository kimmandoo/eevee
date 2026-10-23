use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ev-store-{}-{stamp}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
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

fn worker(directory: &Directory, project: &str) -> Worker {
    Worker::start(directory.0.clone(), project.into(), Policy::default()).unwrap()
}

fn usage(directory: &Directory) -> u64 {
    live_bytes(&open_readonly(&directory.0.join(DATABASE)).unwrap()).unwrap()
}

/// Partner #1 in a store that initialized before any task existed: form 0 at
/// creation, adopted each saved form, derived met timestamp.
fn first_partner(form: usize) -> Partner {
    Partner {
        form,
        auto_form: false,
        ..Partner::default()
    }
}

fn empty_project(directory: &Directory) -> Project {
    // Partners are store-global: a task saved under a sibling project still
    // shows up in this project's roster.
    let connection = open_readonly(&directory.0.join(DATABASE)).unwrap();
    Project {
        tasks: Vec::new(),
        partners: partners(&connection).unwrap(),
        active_id: None,
        usage_bytes: live_bytes(&connection).unwrap(),
    }
}

fn event(worker: &Worker) -> Event {
    let event = worker
        .events
        .as_ref()
        .unwrap()
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    // Captured by the test harness and displayed on assertion failures.
    eprintln!("storage event: {event:?}");
    event
}

fn loaded(worker: &Worker) -> Project {
    match event(worker) {
        Event::Loaded(project) => project,
        other => panic!("expected loaded project: {other:?}"),
    }
}

fn saved(worker: &Worker) -> Snapshot {
    match event(worker) {
        Event::Saved(snapshot) => snapshot,
        other => panic!("expected committed snapshot: {other:?}"),
    }
}

fn enqueue(worker: &Worker, snapshot: Snapshot) {
    let mut pending = Some(snapshot);
    assert!(worker.try_save(&mut pending));
    assert!(pending.is_none());
}

fn create(worker: &Worker, snapshot: Snapshot) -> Snapshot {
    enqueue(worker, snapshot);
    saved(worker)
}

fn downgrade_workflows(connection: &Connection) {
    downgrade_partners(connection);
    connection
        .execute_batch(
            "DROP TABLE memories;
             ALTER TABLE tasks DROP COLUMN workflows;
             ALTER TABLE tasks DROP COLUMN memory_draft;
             ALTER TABLE tasks DROP COLUMN approved_memories;
             ALTER TABLE tasks DROP COLUMN workflow_draft;
             PRAGMA user_version=5;",
        )
        .unwrap();
}
#[test]
fn provider_journal_survives_restart_with_owner_origin_and_explicit_purge() {
    let directory = Directory::new();
    let worker = worker(&directory, "journal-project");
    loaded(&worker);
    let task = create(&worker, Snapshot::empty("journal-project".into()));
    let record = ProviderRecord {
        id: "process-nonce-generation-item".into(),
        agent: crate::providers::Agent::Codex,
        generation: 1,
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: "item".into(),
        origin: "Codex provider-native".into(),
        coverage: "Public observed result only; not a local check".into(),
        text: "Final public output".into(),
        complete: true,
        partner: None,
    };
    let mut pending = Some(JournalRequest {
        task_id: task.id,
        operation: JournalOperation::Put(Box::new(record.clone())),
    });
    assert!(worker.try_journal(&mut pending));
    assert!(
        matches!(event(&worker), Event::Journal { result: Ok(JournalResponse::Saved(saved)), .. } if saved == record)
    );
    drop(worker);
    let reopened = super::Worker::start(
        directory.0.clone(),
        "journal-project".into(),
        Policy::default(),
    )
    .unwrap();
    loaded(&reopened);
    let mut pending = Some(JournalRequest {
        task_id: task.id,
        operation: JournalOperation::List,
    });
    assert!(reopened.try_journal(&mut pending));
    assert!(
        matches!(event(&reopened), Event::Journal { result: Ok(JournalResponse::Records(records)), .. } if records == vec![record.clone()])
    );
    let mut changed = record.clone();
    changed.text = "A later process must not overwrite".into();
    let mut pending = Some(JournalRequest {
        task_id: task.id,
        operation: JournalOperation::Put(Box::new(changed)),
    });
    assert!(reopened.try_journal(&mut pending));
    assert!(matches!(
        event(&reopened),
        Event::Journal { result: Err(_), .. }
    ));
    let mut pending = Some(JournalRequest {
        task_id: task.id + 999,
        operation: JournalOperation::List,
    });
    assert!(reopened.try_journal(&mut pending));
    assert!(matches!(
        event(&reopened),
        Event::Journal { result: Err(_), .. }
    ));
    let mut pending = Some(JournalRequest {
        task_id: task.id,
        operation: JournalOperation::Purge,
    });
    assert!(reopened.try_journal(&mut pending));
    assert!(matches!(
        event(&reopened),
        Event::Journal {
            result: Ok(JournalResponse::Purged),
            ..
        }
    ));
    let mut pending = Some(JournalRequest {
        task_id: task.id,
        operation: JournalOperation::List,
    });
    assert!(reopened.try_journal(&mut pending));
    assert!(
        matches!(event(&reopened), Event::Journal { result: Ok(JournalResponse::Records(records)), .. } if records.is_empty())
    );
}
#[test]
fn provider_journal_migrates_v4_and_enforces_quota_without_partial_record() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialize(&mut connection).unwrap();
    downgrade_workflows(&connection);
    connection
        .execute_batch("DROP TABLE provider_journal; PRAGMA user_version=4;")
        .unwrap();
    assert!(matches!(inspect(&connection).unwrap(), Schema::Evidence));
    initialize(&mut connection).unwrap();
    assert!(matches!(inspect(&connection).unwrap(), Schema::Current));
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(Snapshot::empty("project".into())).unwrap();
    let record = ProviderRecord {
        id: "quota-session-item".into(),
        agent: crate::providers::Agent::Codex,
        generation: 1,
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: "item".into(),
        origin: "provider-native".into(),
        coverage: "observed only".into(),
        text: "bounded result".into(),
        complete: true,
        partner: None,
    };
    store.policy.quota_bytes = 0;
    assert!(
        store
            .journal_request(JournalRequest {
                task_id: task.id,
                operation: JournalOperation::Put(Box::new(record))
            })
            .is_err()
    );
    assert!(
        matches!(store.journal_request(JournalRequest { task_id: task.id, operation: JournalOperation::List }).unwrap(), JournalResponse::Records(records) if records.is_empty())
    );
    store.project = "different-project".into();
    assert!(
        store
            .journal_request(JournalRequest {
                task_id: task.id,
                operation: JournalOperation::List
            })
            .is_err()
    );
}

#[test]
fn provider_journal_masks_after_normalization_and_rejects_empty_provenance() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let record = ProviderRecord {
        id: "normalized-session-item".into(),
        agent: crate::providers::Agent::Codex,
        generation: 1,
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: "item".into(),
        origin: "provider-native".into(),
        coverage: "observed only".into(),
        text: "pass\u{0008}word=journal-private-value\nsafe".into(),
        complete: true,
        partner: None,
    };
    let response = store
        .journal_request(JournalRequest {
            task_id: task.id,
            operation: JournalOperation::Put(Box::new(record.clone())),
        })
        .unwrap();
    let JournalResponse::Saved(saved) = response else {
        panic!("expected committed provider record");
    };
    assert!(!saved.text.contains("journal-private-value"));
    assert!(saved.text.contains("[REDACTED]") && saved.text.contains("safe"));
    // Retrying the original observation is idempotent, including its normalization.
    assert!(matches!(
        store.journal_request(JournalRequest {
            task_id: task.id,
            operation: JournalOperation::Put(Box::new(record.clone())),
        }).unwrap(),
        JournalResponse::Saved(retried) if retried == saved
    ));
    for coverage in ["\u{0008}", "\u{202e}", " \n\t "] {
        let mut invalid = record.clone();
        invalid.id = "invalid-provenance".into();
        invalid.coverage = coverage.into();
        assert!(
            store
                .journal_request(JournalRequest {
                    task_id: task.id,
                    operation: JournalOperation::Put(Box::new(invalid)),
                })
                .is_err()
        );
    }
    let mut invalid = record;
    invalid.id = "misleading\u{202e}identity".into();
    assert!(
        store
            .journal_request(JournalRequest {
                task_id: task.id,
                operation: JournalOperation::Put(Box::new(invalid)),
            })
            .is_err()
    );
    drop(store);
    let mut reopened = state(&directory, "project", Policy::default());
    assert!(matches!(
        reopened.journal_request(JournalRequest {
            task_id: task.id,
            operation: JournalOperation::List,
        }).unwrap(),
        JournalResponse::Records(records) if records == vec![saved.clone()]
    ));
    let bytes = fs::read(directory.0.join(DATABASE)).unwrap();
    assert!(
        !bytes
            .windows(b"journal-private-value".len())
            .any(|window| window == b"journal-private-value")
    );
    // An observation written before the masking fix remains append-only on disk,
    // but loading it must apply the current privacy boundary.
    let mut legacy = saved.clone();
    legacy.text = "password=legacy-journal-value".into();
    let json = serde_json::to_string(&legacy).unwrap();
    reopened
        .connection
        .as_ref()
        .unwrap()
        .execute(
            "UPDATE provider_journal SET record=?1 WHERE id=?2",
            params![json, legacy.id],
        )
        .unwrap();
    let JournalResponse::Records(records) = reopened
        .journal_request(JournalRequest {
            task_id: task.id,
            operation: JournalOperation::List,
        })
        .unwrap()
    else {
        panic!("expected provider journal records");
    };
    assert!(!records[0].text.contains("legacy-journal-value"));
    let retained: String = reopened
        .connection
        .as_ref()
        .unwrap()
        .query_row(
            "SELECT record FROM provider_journal WHERE id=?1",
            [&legacy.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        retained, json,
        "a journal read must not rewrite retained evidence"
    );
}

#[test]
fn save_queue_retains_pending_on_full_disconnected_or_missing_sender() {
    let (commands, incoming) = mpsc::sync_channel(1);
    let mut worker = Worker {
        commands: Some(commands),
        events: None,
        thread: None,
    };
    let mut empty = None;
    assert!(!worker.try_save(&mut empty));
    let mut first = Some(snapshot("project", 1));
    assert!(worker.try_save(&mut first));
    assert!(first.is_none());

    let mut pending = Some(snapshot("project", 2));
    assert!(!worker.try_save(&mut pending));
    assert_eq!(pending.as_ref().unwrap().revision, 2);
    assert!(matches!(incoming.try_recv(), Ok(Command::Save(saved)) if saved.revision == 1));
    assert!(worker.try_save(&mut pending));
    assert!(pending.is_none());
    assert!(matches!(incoming.try_recv(), Ok(Command::Save(saved)) if saved.revision == 2));

    drop(incoming);
    let expected = snapshot("project", 3);
    let mut pending = Some(expected.clone());
    assert!(!worker.try_save(&mut pending));
    assert_eq!(pending.as_ref(), Some(&expected));
    worker.commands.take();
    assert!(!worker.try_save(&mut pending));
    assert_eq!(pending.as_ref(), Some(&expected));
}

fn snapshot(project: &str, revision: u64) -> Snapshot {
    Snapshot {
        goal: "保持 Unicode goal".into(),
        form: 3,
        auto_form: false,
        revision,
        ..Snapshot::empty(project.into())
    }
}

fn partner_drafts(drafts: Vec<Draft>) -> BTreeMap<i64, PartnerDrafts> {
    BTreeMap::from([(
        1,
        PartnerDrafts {
            drafts,
            ..PartnerDrafts::default()
        },
    )])
}

#[test]
fn saves_reopens_and_isolates_projects() {
    let directory = Directory::new();
    let mut committed = Vec::new();
    for project in ["/项目/one", "/项目/two"] {
        let worker = worker(&directory, project);
        assert_eq!(loaded(&worker), empty_project(&directory));
        let saved = create(&worker, snapshot(project, 1));
        assert!(saved.id > 0 && saved.updated_at > 0);
        committed.push(saved);
    }
    assert_ne!(committed[0].id, committed[1].id);
    for saved in committed {
        let worker = worker(&directory, &saved.project);
        assert_eq!(
            loaded(&worker),
            Project {
                usage_bytes: usage(&directory),
                partners: vec![first_partner(3)],
                active_id: Some(saved.id),
                tasks: vec![saved]
            }
        );
    }
}

#[test]
fn writer_conflict_can_retry_without_takeover() {
    let directory = Directory::new();
    let first = worker(&directory, "project");
    assert_eq!(loaded(&first), empty_project(&directory));
    let second = worker(&directory, "project");
    assert!(
        matches!(event(&second), Event::Failed { revision: None, message } if message.contains("writer lock"))
    );
    assert!(second.retry());
    assert!(matches!(
        event(&second),
        Event::Failed { revision: None, .. }
    ));
    drop(first);
    assert!(second.retry());
    assert_eq!(loaded(&second), empty_project(&directory));
    enqueue(&second, snapshot("project", 1));
    assert_eq!(saved(&second).revision, 1);
}

#[cfg(unix)]
#[test]
fn directory_aliases_share_writer_identity_and_allow_readonly_open() {
    use std::os::unix::fs::symlink;
    let directory = Directory::new();
    let aliases = Directory::new();
    let alias = aliases.0.join("store-alias");
    symlink(&directory.0, &alias).unwrap();
    let first = Worker::start(alias.clone(), "project".into(), Policy::default()).unwrap();
    assert_eq!(loaded(&first), empty_project(&directory));
    let saved = create(&first, snapshot("project", 1));
    let second = worker(&directory, "project");
    assert!(
        matches!(event(&second), Event::Failed { revision: None, message } if message.contains("writer lock"))
    );
    let readonly = open_readonly(&alias.join(DATABASE)).unwrap();
    let expected = Project {
        active_id: Some(saved.id),
        tasks: vec![saved],
        partners: vec![first_partner(3)],
        usage_bytes: usage(&directory),
    };
    assert_eq!(load(&readonly, "project").unwrap(), expected);
    drop(readonly);
    drop(first);
    assert!(second.retry());
    assert_eq!(loaded(&second), expected);
}

#[test]
fn migrates_documented_schema_zero_and_one_atomically() {
    for version in [0, 1] {
        let directory = Directory::new();
        let path = directory.0.join(DATABASE);
        let connection = Connection::open(&path).unwrap();
        legacy(&connection, version);
        drop(connection);
        for (project, goal, form, revision) in [
            (
                "project",
                "legacy goal",
                2,
                if version == 0 { 0 } else { 37 },
            ),
            ("other", "other goal", 8, if version == 0 { 0 } else { 91 }),
        ] {
            let store = worker(&directory, project);
            let actual = loaded(&store);
            assert_eq!(actual.tasks.len(), 1);
            let task = &actual.tasks[0];
            assert_eq!(actual.active_id, Some(task.id));
            assert!(task.id > 0);
            assert_eq!(task.goal, goal);
            assert_eq!(task.form, form);
            assert_eq!(task.revision, revision);
            assert_eq!(task.updated_at, 0);
            assert!(
                task.partner_drafts
                    .values()
                    .all(|state| state.drafts.is_empty())
                    && task.criteria.is_empty()
                    && !task.complete
            );
        }
        let connection = open_readonly(&path).unwrap();
        assert!(matches!(inspect(&connection).unwrap(), Schema::Current));
        assert!(connection.prepare("SELECT * FROM goals").is_err());
        assert!(connection.prepare("SELECT * FROM projects").is_err());
    }
}

#[test]
fn newer_foreign_and_corrupt_databases_are_untouched() {
    for fixture in [0, 1, 2] {
        let directory = Directory::new();
        let path = directory.0.join(DATABASE);
        if fixture == 2 {
            fs::write(&path, b"not a SQLite database\0do not overwrite").unwrap();
        } else {
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch(if fixture == 0 {
                    "CREATE TABLE projects(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL,revision INTEGER NOT NULL); PRAGMA user_version=999;"
                } else {
                    "CREATE TABLE unrelated(value TEXT); INSERT INTO unrelated VALUES ('keep');"
                }).unwrap();
        }
        let before = fs::read(&path).unwrap();
        let worker = worker(&directory, "project");
        assert!(matches!(
            event(&worker),
            Event::Failed { revision: None, .. }
        ));
        drop(worker);
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn failed_transaction_is_not_acknowledged_or_committed() {
    let directory = Directory::new();
    let worker = worker(&directory, "project");
    assert_eq!(loaded(&worker), empty_project(&directory));
    let connection = Connection::open(directory.0.join(DATABASE)).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_save BEFORE INSERT ON selections BEGIN SELECT RAISE(ABORT, 'injected transaction failure'); END;").unwrap();
    enqueue(&worker, snapshot("project", 1));
    assert!(
        matches!(event(&worker), Event::Failed { revision: Some(1), message } if message.contains("injected transaction failure"))
    );
    assert!(worker.try_event().is_none());
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM tasks", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn stale_revision_cannot_replace_committed_goal() {
    let directory = Directory::new();
    let worker = worker(&directory, "project");
    assert_eq!(loaded(&worker), empty_project(&directory));
    let latest = create(&worker, snapshot("project", 2));
    for revision in [1, 2] {
        let mut stale = latest.clone();
        stale.revision = revision;
        stale.goal = "conflict".into();
        enqueue(&worker, stale);
        assert!(
            matches!(event(&worker), Event::Failed { revision: Some(actual), message }
                if actual == revision && message.contains("stale"))
        );
    }
    assert!(worker.retry());
    assert_eq!(loaded(&worker).tasks, vec![latest]);
}

#[test]
fn drop_drains_accepted_saves_even_with_full_event_queue() {
    let directory = Directory::new();
    let store = worker(&directory, "project");
    assert_eq!(loaded(&store), empty_project(&directory));
    let initial = create(&store, snapshot("project", 1));
    // Fill the event queue with eight acknowledgements without receiving them.
    for revision in 2..=11 {
        let mut pending = initial.clone();
        pending.revision = revision;
        let mut pending = Some(pending);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !store.try_save(&mut pending) {
            assert!(
                std::time::Instant::now() < deadline,
                "save queue did not drain"
            );
            assert_eq!(pending.as_ref().unwrap().revision, revision);
            thread::yield_now();
        }
        assert!(pending.is_none());
    }
    drop(store);
    let reopened = worker(&directory, "project");
    let project = loaded(&reopened);
    assert_eq!(project.tasks.len(), 1);
    assert_eq!(project.tasks[0].revision, 11);
    assert_eq!(project.tasks[0].id, initial.id);
}

#[test]
fn export_is_read_only_and_never_overwrites() {
    for version in [0, 1, 2] {
        let directory = Directory::new();
        let output = directory.0.join("tasks.json");
        assert!(export_readable(&directory.0, &output).is_err());
        assert!(!directory.0.join(DATABASE).exists());
        let path = directory.0.join(DATABASE);
        let expected = if version == 2 {
            let store = worker(&directory, "project");
            loaded(&store);
            Some(create(
                &store,
                Snapshot {
                    partner_drafts: partner_drafts(vec![Draft {
                        field: EditorField::Goal,
                        text: "保留".into(),
                        cursor: 6,
                    }]),
                    criteria: vec![Criterion {
                        text: "reviewed".into(),
                        met: true,
                    }],
                    complete: true,
                    next_action: "Follow up".into(),
                    ..snapshot("project", 1)
                },
            ))
        } else {
            let connection = Connection::open(&path).unwrap();
            legacy(&connection, version);
            None
        };
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("PRAGMA user_version=42;").unwrap();
        drop(connection);
        let before = fs::read(&path).unwrap();
        export_readable(&directory.0, &output).unwrap();
        let exported = fs::read(&output).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&exported).unwrap();
        if let Some(expected) = expected {
            let tasks: Vec<Snapshot> = serde_json::from_value(json).unwrap();
            assert_eq!(tasks, vec![expected]);
        } else {
            assert_eq!(json[0]["project"], "other");
            assert_eq!(json[1]["goal"], "legacy goal");
        }
        assert!(export_readable(&directory.0, &output).is_err());
        assert_eq!(fs::read(&output).unwrap(), exported);
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[cfg(unix)]
#[test]
fn read_only_and_symlink_targets_are_refused() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = Directory::new();
    let first = worker(&directory, "project");
    assert_eq!(loaded(&first), empty_project(&directory));
    drop(first);
    let path = directory.0.join(DATABASE);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let readonly = worker(&directory, "project");
    assert!(
        matches!(event(&readonly), Event::Failed { revision: None, message } if message.contains("read-only"))
    );
    drop(readonly);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let other = Directory::new();
    symlink(&path, other.0.join(DATABASE)).unwrap();
    let linked = worker(&other, "project");
    assert!(
        matches!(event(&linked), Event::Failed { revision: None, message } if message.contains("regular"))
    );
    assert!(export_readable(&other.0, &other.0.join("export.json")).is_err());
}

#[test]
fn invalid_save_bounds_are_never_acknowledged() {
    let directory = Directory::new();
    let worker = worker(&directory, "project");
    assert_eq!(loaded(&worker), empty_project(&directory));
    let draft = Draft {
        field: EditorField::Goal,
        text: "目".into(),
        cursor: 1,
    };
    let duplicate = Draft {
        cursor: 3,
        ..draft.clone()
    };
    for invalid in [
        Snapshot {
            goal: "x".repeat(4097),
            ..snapshot("project", 1)
        },
        Snapshot {
            form: 9,
            ..snapshot("project", 1)
        },
        Snapshot {
            revision: i64::MAX as u64 + 1,
            ..snapshot("project", 1)
        },
        Snapshot {
            next_action: "目".repeat(1366),
            ..snapshot("project", 1)
        },
        Snapshot {
            id: -1,
            ..snapshot("project", 1)
        },
        Snapshot {
            criteria: vec![Criterion {
                text: "x".repeat(257),
                met: false,
            }],
            ..snapshot("project", 1)
        },
        Snapshot {
            criteria: vec![
                Criterion {
                    text: String::new(),
                    met: false
                };
                65
            ],
            ..snapshot("project", 1)
        },
        Snapshot {
            partner_drafts: partner_drafts(vec![draft]),
            ..snapshot("project", 1)
        },
        Snapshot {
            partner_drafts: partner_drafts(vec![duplicate.clone(), duplicate.clone()]),
            ..snapshot("project", 1)
        },
        Snapshot {
            partner_drafts: partner_drafts(vec![duplicate; 5]),
            ..snapshot("project", 1)
        },
        Snapshot {
            partner_drafts: partner_drafts(vec![Draft {
                field: EditorField::Goal,
                text: "x".repeat(4097),
                cursor: 0,
            }]),
            ..snapshot("project", 1)
        },
        Snapshot {
            partner_drafts: partner_drafts(vec![Draft {
                field: EditorField::Goal,
                text: "x".into(),
                cursor: 2,
            }]),
            ..snapshot("project", 1)
        },
        Snapshot {
            navigation: Navigation {
                focus: 3,
                ..Navigation::default()
            },
            ..snapshot("project", 1)
        },
        Snapshot {
            navigation: Navigation {
                form_selection: 9,
                ..Navigation::default()
            },
            ..snapshot("project", 1)
        },
        Snapshot {
            navigation: Navigation {
                criterion_selection: 64,
                ..Navigation::default()
            },
            ..snapshot("project", 1)
        },
        Snapshot {
            navigation: Navigation {
                overlay: Some(8),
                ..Navigation::default()
            },
            ..snapshot("project", 1)
        },
        snapshot("other-project", 1),
    ] {
        let expected_revision = invalid.revision;
        enqueue(&worker, invalid);
        assert!(
            matches!(event(&worker), Event::Failed { revision: Some(revision), .. } if revision == expected_revision)
        );
    }
    assert!(worker.retry());
    assert_eq!(loaded(&worker), empty_project(&directory));
}

#[test]
fn failed_revision_does_not_hide_a_later_accepted_save() {
    let directory = Directory::new();
    let worker = worker(&directory, "project");
    assert_eq!(loaded(&worker), empty_project(&directory));
    enqueue(
        &worker,
        Snapshot {
            form: 9,
            ..snapshot("project", 1)
        },
    );
    enqueue(&worker, snapshot("project", 2));
    assert!(matches!(
        event(&worker),
        Event::Failed {
            revision: Some(1),
            ..
        }
    ));
    let committed = saved(&worker);
    assert_eq!(committed.revision, 2);
    assert!(worker.retry());
    assert_eq!(loaded(&worker).tasks, vec![committed]);
}

#[test]
fn final_form_index_eight_round_trips() {
    let directory = Directory::new();
    let saved = Snapshot {
        form: 8,
        ..snapshot("project", 1)
    };
    let store = worker(&directory, "project");
    assert_eq!(loaded(&store), empty_project(&directory));
    let saved = create(&store, saved);
    drop(store);
    let reopened = worker(&directory, "project");
    assert_eq!(loaded(&reopened).tasks, vec![saved]);
}

#[test]
fn sqlite_full_rolls_back_save_without_success() {
    let directory = Directory::new();
    let mut state = State {
        data_dir: directory.0.clone(),
        project: "project".into(),
        policy: Policy::default(),
        connection: None,
        lock: None,
    };
    assert!(
        matches!(state.reopen(), Event::Loaded(project) if project == empty_project(&directory))
    );
    let connection = state.connection.as_ref().unwrap();
    // Schema table rebuilds can leave reusable pages. Compact before capping
    // growth so the overflow-page write deterministically reaches SQLITE_FULL.
    connection.execute_batch("VACUUM;").unwrap();
    let page_count: i64 = connection
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    connection
        .pragma_update(None, "max_page_count", page_count)
        .unwrap();
    let oversized = Snapshot {
        goal: "x".repeat(4096),
        ..snapshot("project", 1)
    };
    // The goal is within the application limit, but requires an overflow page.
    let error = state.save(oversized).unwrap_err();
    assert!(error.contains("database or disk is full"), "{error}");
    assert_eq!(
        load(state.connection.as_ref().unwrap(), "project").unwrap(),
        empty_project(&directory)
    );
    drop(state);
    let reopened = worker(&directory, "project");
    assert_eq!(loaded(&reopened), empty_project(&directory));
}

#[test]
fn sqlite_full_during_migration_preserves_committed_legacy_schema() {
    for version in [0, 1] {
        let directory = Directory::new();
        let path = directory.0.join(DATABASE);
        let mut connection = Connection::open(&path).unwrap();
        legacy(&connection, version);
        let page_count: i64 = connection
            .pragma_query_value(None, "page_count", |row| row.get(0))
            .unwrap();
        connection
            .pragma_update(None, "max_page_count", page_count)
            .unwrap();
        let error = initialize(&mut connection).unwrap_err();
        assert!(error.contains("database or disk is full"), "{error}");
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, u8>(0))
                .unwrap(),
            version
        );
        let table = if version == 0 { "goals" } else { "projects" };
        assert_eq!(
            connection
                .query_row(
                    &format!("SELECT goal FROM {table} WHERE project='project'"),
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "legacy goal"
        );
        assert!(connection.prepare("SELECT * FROM tasks").is_err());
        assert!(connection.prepare("SELECT * FROM selections").is_err());
        drop(connection);
        let reopened = worker(&directory, "project");
        assert_eq!(loaded(&reopened).tasks[0].goal, "legacy goal");
    }
}

#[test]
fn interrupted_migration_transaction_rolls_back_all_schema_changes() {
    for version in [0, 1] {
        let directory = Directory::new();
        let path = directory.0.join(DATABASE);
        let mut connection = Connection::open(&path).unwrap();
        legacy(&connection, version);
        {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
            // Drop the actual migration transaction before commit.
        }
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, u8>(0))
                .unwrap(),
            version
        );
        let table = if version == 0 { "goals" } else { "projects" };
        assert_eq!(
            connection
                .query_row(
                    &format!("SELECT goal FROM {table} WHERE project='project'"),
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "legacy goal"
        );
        assert!(connection.prepare("SELECT * FROM tasks").is_err());
        assert!(connection.prepare("SELECT * FROM selections").is_err());
        drop(connection);
        let reopened = worker(&directory, "project");
        assert_eq!(loaded(&reopened).tasks[0].goal, "legacy goal");
    }
}

#[test]
#[ignore = "subprocess-only fixture; invoked by hot-journal recovery tests"]
fn interrupted_migration_process_fixture() {
    let path = std::env::var_os("EV_STORE_CRASH_FIXTURE").expect("fixture database path");
    let kind = std::env::var("EV_STORE_CRASH_KIND").unwrap();
    let mut connection = Connection::open(PathBuf::from(path)).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode=DELETE;
             PRAGMA synchronous=FULL;
             PRAGMA cache_size=1;
             PRAGMA cache_spill=ON;",
        )
        .unwrap();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    match kind.as_str() {
        "current" => {
            transaction
                .execute(
                    "UPDATE tasks SET goal=printf('%04000d',id),revision=revision+1",
                    [],
                )
                .unwrap();
            transaction
                .execute("UPDATE selections SET active_id=999999", [])
                .unwrap();
        }
        "one" => migrate(&transaction, Schema::Projects).unwrap(),
        _ => migrate(&transaction, Schema::Legacy).unwrap(),
    }
    // Terminate the actual process without running SQLite or transaction
    // destructors. The spilled transaction leaves a hot rollback journal.
    std::process::exit(73);
}

fn crash_during_migration(directory: &Directory, kind: &str) {
    let path = directory.0.join(DATABASE);
    let mut connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE goals(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL);
             INSERT INTO goals VALUES ('project','committed before process interruption',8);
             WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<100)
             INSERT INTO goals SELECT 'project-' || n, printf('%02000d',n), 0 FROM numbers;",
        )
        .unwrap();
    match kind {
            "newer" => connection.execute_batch("PRAGMA user_version=16;").unwrap(),
            "foreign" => connection.execute_batch("CREATE TABLE unrelated(value TEXT); INSERT INTO unrelated VALUES ('preserve');").unwrap(),
            "one" => connection.execute_batch(
                "CREATE TABLE projects(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL,revision INTEGER NOT NULL);
                 INSERT INTO projects SELECT project,goal,form,17 FROM goals;
                 DROP TABLE goals; PRAGMA user_version=1;",
            ).unwrap(),
            "current" => initialize(&mut connection).unwrap(),
            "known" => {}
            _ => panic!("unknown fixture"),
        }
    drop(connection);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "store::tests::interrupted_migration_process_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("EV_STORE_CRASH_FIXTURE", &path)
        .env("EV_STORE_CRASH_KIND", kind)
        .status()
        .unwrap();
    assert_eq!(
        status.code(),
        Some(73),
        "fixture must exit without transaction cleanup"
    );
    let readonly = open_readonly(&path).unwrap();
    let error = readonly
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .unwrap_err();
    assert!(
        recovery_required(&error),
        "fixture must require real journal recovery: {error}"
    );
}

#[test]
fn process_interruption_recovers_legacy_and_current_committed_tasks() {
    for kind in ["known", "one", "current"] {
        let directory = Directory::new();
        crash_during_migration(&directory, kind);
        let store = worker(&directory, "project");
        let project = loaded(&store);
        assert_eq!(project.tasks.len(), 1);
        let task = &project.tasks[0];
        assert_eq!(task.goal, "committed before process interruption");
        assert_eq!(task.form, 8);
        assert_eq!(task.revision, if kind == "one" { 17 } else { 0 });
        assert_eq!(project.active_id, Some(task.id));
        drop(store);
        let connection = open_readonly(&directory.0.join(DATABASE)).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM tasks", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            101
        );
    }
}

#[test]
fn hot_journal_recovery_never_modifies_newer_or_foreign_originals() {
    for kind in ["newer", "foreign"] {
        let directory = Directory::new();
        crash_during_migration(&directory, kind);
        let path = directory.0.join(DATABASE);
        let journal = directory.0.join("workbench.sqlite3-journal");
        let database_before = fs::read(&path).unwrap();
        let journal_before = fs::read(&journal).unwrap();
        let worker = worker(&directory, "project");
        assert!(
            matches!(event(&worker), Event::Failed { revision: None, message } if message.contains("recovery was refused"))
        );
        drop(worker);
        assert_eq!(fs::read(path).unwrap(), database_before);
        assert_eq!(fs::read(journal).unwrap(), journal_before);
    }
}

fn legacy(connection: &Connection, version: u8) {
    if version == 0 {
        connection.execute_batch(
                "CREATE TABLE goals(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL);
                 INSERT INTO goals VALUES ('project','legacy goal',2),('other','other goal',8);",
            ).unwrap();
    } else {
        connection.execute_batch(
                "CREATE TABLE projects(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL,revision INTEGER NOT NULL);
                 INSERT INTO projects VALUES ('project','legacy goal',2,37),('other','other goal',8,91);
                 PRAGMA user_version=1;",
            ).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn closed_store_releases_writer_lock_despite_inherited_descriptor() {
    let directory = Directory::new();
    let store = state(&directory, "project", Policy::default());
    let inherited = store.lock.as_ref().unwrap().try_clone().unwrap();
    drop(store);
    let mut reopened = state(&directory, "project", Policy::default());
    let saved = reopened.save(snapshot("project", 1)).unwrap();
    assert_eq!(
        load(reopened.connection.as_ref().unwrap(), "project")
            .unwrap()
            .tasks,
        vec![saved]
    );
    drop(inherited);
}

#[test]
fn two_tasks_preserve_drafts_navigation_selection_and_timestamps_across_restart() {
    let directory = Directory::new();
    let store = worker(&directory, "project");
    assert_eq!(loaded(&store), empty_project(&directory));
    let first = create(
        &store,
        Snapshot {
            criteria: vec![Criterion {
                text: "用户检查".into(),
                met: true,
            }],
            next_action: "Review the result".into(),
            partner_drafts: BTreeMap::from([(
                1,
                PartnerDrafts {
                    drafts: vec![
                        Draft {
                            field: EditorField::Goal,
                            text: "新目标".into(),
                            cursor: 3,
                        },
                        Draft {
                            field: EditorField::Criteria,
                            text: "first\nsecond".into(),
                            cursor: 6,
                        },
                        Draft {
                            field: EditorField::NextAction,
                            text: "later".into(),
                            cursor: 5,
                        },
                    ],
                    editor_field: EditorField::NextAction,
                    editing: true,
                    previous_form: Some(1),
                },
            )]),
            navigation: Navigation {
                focus: 2,
                linear: true,
                form_selection: 8,
                criterion_selection: 63,
                scroll: [1, 2, 3, 4, u16::MAX],
                overlay: Some(5),
                previous_form: Some(1),
                dismissed_suggestion: None,
                editing: true,
                editor_field: EditorField::NextAction,
            },
            ..snapshot("project", 7)
        },
    );
    let second = create(
        &store,
        Snapshot {
            form: 8,
            ..snapshot("project", 1)
        },
    );
    assert_ne!(first.id, second.id);
    let mut changed = first.clone();
    changed.revision += 1;
    changed.next_action = "Updated without selecting".into();
    changed.updated_at = 0;
    let first = create(&store, changed);
    assert!(store.retry());
    assert_eq!(
        loaded(&store),
        Project {
            usage_bytes: usage(&directory),
            partners: vec![first_partner(3)],
            active_id: Some(second.id),
            tasks: vec![first.clone(), second.clone()],
        }
    );
    assert!(store.try_select(first.id));
    assert!(matches!(event(&store), Event::Selected(actual) if actual == first));
    drop(store);
    let reopened = worker(&directory, "project");
    assert_eq!(
        loaded(&reopened),
        Project {
            usage_bytes: usage(&directory),
            partners: vec![first_partner(3)],
            active_id: Some(first.id),
            tasks: vec![first, second],
        }
    );
}

#[test]
fn selection_rejects_missing_and_cross_project_ids_without_changing_active_task() {
    let directory = Directory::new();
    let other = worker(&directory, "other");
    loaded(&other);
    let foreign = create(&other, snapshot("other", 1));
    drop(other);
    let store = worker(&directory, "project");
    loaded(&store);
    let own = create(&store, snapshot("project", 1));
    for id in [0, -1, foreign.id, own.id + 100] {
        assert!(store.try_select(id));
        assert!(matches!(
            event(&store),
            Event::Failed { revision: None, .. }
        ));
    }
    let mut forged = foreign;
    forged.project = "project".into();
    forged.revision += 1;
    enqueue(&store, forged);
    assert!(matches!(
        event(&store),
        Event::Failed {
            revision: Some(2),
            ..
        }
    ));
    assert!(store.retry());
    assert_eq!(
        loaded(&store),
        Project {
            usage_bytes: usage(&directory),
            partners: vec![first_partner(3)],
            active_id: Some(own.id),
            tasks: vec![own]
        }
    );
}

#[test]
fn completion_requires_reviewed_criteria_and_explicit_reopen_for_edits() {
    let directory = Directory::new();
    let store = worker(&directory, "project");
    loaded(&store);
    for criteria in [
        vec![],
        vec![Criterion {
            text: "check".into(),
            met: false,
        }],
    ] {
        enqueue(
            &store,
            Snapshot {
                complete: true,
                criteria,
                ..snapshot("project", 1)
            },
        );
        assert!(matches!(
            event(&store),
            Event::Failed {
                revision: Some(1),
                ..
            }
        ));
    }
    let complete = create(
        &store,
        Snapshot {
            complete: true,
            criteria: vec![Criterion {
                text: "check".into(),
                met: true,
            }],
            ..snapshot("project", 2)
        },
    );
    for edit_criteria in [false, true] {
        let mut rejected = complete.clone();
        rejected.revision += 1;
        if edit_criteria {
            rejected.criteria[0].text = "different".into();
        } else {
            rejected.goal = "different".into();
        }
        enqueue(&store, rejected);
        assert!(
            matches!(event(&store), Event::Failed { revision: Some(3), message } if message.contains("Reopen"))
        );
    }
    let mut allowed = complete;
    allowed.revision += 1;
    allowed.next_action = "A completed task can retain a next action".into();
    let mut reopened = create(&store, allowed);
    reopened.revision += 1;
    reopened.complete = false;
    reopened.goal = "reopened".into();
    reopened.criteria[0].met = false;
    let reopened = create(&store, reopened);
    drop(store);
    let store = worker(&directory, "project");
    assert_eq!(loaded(&store).tasks, vec![reopened]);
}

#[test]
fn current_schema_reopens_without_rewriting_or_inventing_activity() {
    let directory = Directory::new();
    let store = worker(&directory, "project");
    loaded(&store);
    let saved = create(&store, snapshot("project", 4));
    drop(store);
    let path = directory.0.join(DATABASE);
    let before = fs::read(&path).unwrap();
    let store = worker(&directory, "project");
    assert_eq!(loaded(&store).tasks, vec![saved.clone()]);
    assert_eq!(create(&store, saved.clone()), saved);
    drop(store);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn failed_selection_preserves_previous_selection() {
    let directory = Directory::new();
    let store = worker(&directory, "project");
    loaded(&store);
    let first = create(&store, snapshot("project", 1));
    let second = create(&store, snapshot("project", 1));
    let connection = Connection::open(directory.0.join(DATABASE)).unwrap();
    connection.execute_batch(
            "CREATE TRIGGER reject_select BEFORE UPDATE ON selections BEGIN SELECT RAISE(ABORT, 'injected selection failure'); END;"
        ).unwrap();
    assert!(store.try_select(first.id));
    assert!(
        matches!(event(&store), Event::Failed { revision: None, message } if message.contains("injected selection failure"))
    );
    assert_eq!(
        load(&connection, "project").unwrap().active_id,
        Some(second.id)
    );
}

#[test]
fn capacity_is_explicit_and_never_evicts_existing_tasks() {
    let directory = Directory::new();
    let store = worker(&directory, "project");
    loaded(&store);
    let mut tasks = Vec::new();
    for _ in 0..MAX_TASKS {
        tasks.push(create(&store, snapshot("project", 1)));
    }
    enqueue(&store, snapshot("project", 2));
    assert!(
        matches!(event(&store), Event::Failed { revision: Some(2), message } if message.contains("capacity"))
    );
    let mut updated = tasks[0].clone();
    updated.revision += 1;
    updated.goal = "still editable at capacity".into();
    tasks[0] = create(&store, updated);
    assert!(store.retry());
    assert_eq!(
        loaded(&store),
        Project {
            usage_bytes: usage(&directory),
            partners: vec![first_partner(3)],
            active_id: Some(tasks[MAX_TASKS - 1].id),
            tasks
        }
    );
}

#[test]
fn malformed_current_schema_data_is_refused_without_rewriting() {
    for mutation in [
        "UPDATE tasks SET navigation='{}'",
        "UPDATE tasks SET criteria='not json'",
        "UPDATE selections SET active_id=99999",
        "UPDATE tasks SET complete=1",
        "UPDATE tasks SET drafts='[{\"field\":\"Goal\",\"text\":\"目\",\"cursor\":1}]'",
        "DELETE FROM partners",
        "UPDATE partners SET id=77",
        "UPDATE tasks SET partner_drafts='[0]'",
    ] {
        let directory = Directory::new();
        let store = worker(&directory, "project");
        loaded(&store);
        create(&store, snapshot("project", 1));
        drop(store);
        let path = directory.0.join(DATABASE);
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(mutation).unwrap();
        drop(connection);
        let before = fs::read(&path).unwrap();
        let store = worker(&directory, "project");
        assert!(matches!(
            event(&store),
            Event::Failed { revision: None, .. }
        ));
        drop(store);
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

fn state(directory: &Directory, project: &str, policy: Policy) -> State {
    let mut state = State {
        data_dir: directory.0.clone(),
        project: project.into(),
        policy,
        connection: None,
        lock: None,
    };
    let reopened = state.reopen();
    assert!(matches!(reopened, Event::Loaded(_)), "{reopened:?}");
    state
}

#[test]
fn management_queue_retains_ownership_on_backpressure_and_disconnect() {
    let (commands, incoming) = mpsc::sync_channel(1);
    let mut worker = Worker {
        commands: Some(commands),
        events: None,
        thread: None,
    };
    let request = Management::Delete {
        ids: vec![17],
        expired_before: None,
    };
    let mut pending = Some(request.clone());
    assert!(worker.try_manage(&mut pending));
    assert!(pending.is_none());
    assert!(!worker.try_manage(&mut pending));
    pending = Some(request.clone());
    assert!(!worker.try_manage(&mut pending));
    assert_eq!(pending, Some(request.clone()));
    drop(incoming);
    assert!(!worker.try_manage(&mut pending));
    assert_eq!(pending, Some(request.clone()));
    worker.commands.take();
    assert!(!worker.try_manage(&mut pending));
    assert_eq!(pending, Some(request));
}

#[test]
fn archive_restore_delete_are_scoped_atomic_and_survive_restart() {
    let directory = Directory::new();
    let mut other = state(&directory, "other", Policy::default());
    let foreign = other.save(snapshot("other", 1)).unwrap();
    drop(other);
    let mut store = state(&directory, "project", Policy::default());
    let first = store.save(snapshot("project", 1)).unwrap();
    let second = store.save(snapshot("project", 1)).unwrap();
    let Managed {
        project: archived,
        affected,
        warning,
    } = store
        .manage_at(
            Management::Archive {
                id: second.id,
                archived: true,
            },
            100,
        )
        .unwrap();
    assert!(warning.is_none());
    assert_eq!(affected, vec![second.id]);
    assert_eq!(archived.active_id, Some(first.id));
    assert!(archived.tasks[1].archived && !archived.tasks[1].complete);
    assert_eq!(archived.tasks[1].archived_at, 100);
    assert!(store.select(second.id).is_err());
    let export = directory.0.join("external-export.json");
    export_readable(&directory.0, &export).unwrap();
    let exported: Vec<Snapshot> = serde_json::from_slice(&fs::read(&export).unwrap()).unwrap();
    assert!(
        exported
            .iter()
            .any(|task| task.id == second.id && task.archived)
    );
    let Managed {
        project: restored, ..
    } = store
        .manage(Management::Archive {
            id: second.id,
            archived: false,
        })
        .unwrap();
    assert_eq!(restored.active_id, Some(first.id));
    assert!(!restored.tasks[1].archived);
    assert_eq!(restored.tasks[1].archived_at, 0);
    for ids in [
        vec![first.id, foreign.id],
        vec![first.id, 99999],
        vec![first.id, first.id],
    ] {
        assert!(
            store
                .manage(Management::Delete {
                    ids,
                    expired_before: None
                })
                .is_err()
        );
        assert_eq!(
            load(store.connection.as_ref().unwrap(), "project")
                .unwrap()
                .tasks
                .len(),
            2
        );
    }
    let Managed {
        project: remaining, ..
    } = store
        .manage(Management::Delete {
            ids: vec![first.id],
            expired_before: None,
        })
        .unwrap();
    assert_eq!(remaining.active_id, Some(second.id));
    assert_eq!(remaining.tasks.len(), 1);
    assert!(export.exists());
    assert_eq!(
        load(store.connection.as_ref().unwrap(), "other")
            .unwrap()
            .tasks,
        vec![foreign]
    );
    store
        .manage(Management::Archive {
            id: second.id,
            archived: true,
        })
        .unwrap();
    drop(store);
    let mut store = state(&directory, "project", Policy::default());
    assert_eq!(
        load(store.connection.as_ref().unwrap(), "project")
            .unwrap()
            .active_id,
        None
    );
    let Managed {
        project: restored, ..
    } = store
        .manage(Management::Archive {
            id: second.id,
            archived: false,
        })
        .unwrap();
    assert_eq!(restored.active_id, None);
    store
        .manage(Management::Delete {
            ids: vec![second.id],
            expired_before: None,
        })
        .unwrap();
    let third = store.save(snapshot("project", 1)).unwrap();
    assert!(
        third.id > second.id,
        "deleted IDs must not be reused by a stale deletion preview"
    );
    assert!(
        store
            .manage(Management::Delete {
                ids: vec![second.id],
                expired_before: None
            })
            .is_err()
    );
}

#[test]
fn retention_revalidates_archive_age_policy_and_every_target() {
    let directory = Directory::new();
    let policy = Policy {
        retention_days: 1,
        ..Policy::default()
    };
    let mut store = state(&directory, "project", policy);
    let first = store.save(snapshot("project", 1)).unwrap();
    let second = store.save(snapshot("project", 1)).unwrap();
    store
        .manage_at(
            Management::Archive {
                id: first.id,
                archived: true,
            },
            1_000,
        )
        .unwrap();
    store
        .manage_at(
            Management::Archive {
                id: second.id,
                archived: true,
            },
            2_000,
        )
        .unwrap();
    let now = 86_400_000 + 3_000;
    let request = Management::Delete {
        ids: vec![first.id, second.id],
        expired_before: Some(3_000),
    };
    store
        .manage_at(
            Management::Archive {
                id: second.id,
                archived: false,
            },
            now,
        )
        .unwrap();
    assert!(store.manage_at(request.clone(), now).is_err());
    store
        .manage_at(
            Management::Archive {
                id: second.id,
                archived: true,
            },
            now,
        )
        .unwrap();
    assert!(store.manage_at(request, now).is_err());
    assert!(
        store
            .manage_at(
                Management::Delete {
                    ids: vec![first.id],
                    expired_before: Some(3_001)
                },
                now
            )
            .is_err()
    );
    assert_eq!(
        load(store.connection.as_ref().unwrap(), "project")
            .unwrap()
            .tasks
            .len(),
        2
    );
    let Managed {
        project: remaining,
        affected,
        warning,
    } = store
        .manage_at(
            Management::Delete {
                ids: vec![first.id],
                expired_before: Some(3_000),
            },
            now,
        )
        .unwrap();
    assert!(warning.is_none());
    assert_eq!(affected, vec![first.id]);
    assert_eq!(remaining.tasks.len(), 1);
    assert_eq!(remaining.tasks[0].id, second.id);
}

#[test]
fn secrets_are_redacted_before_sqlite_and_export_and_history_is_sticky() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let raw = "前 password=super-secret 後";
    let task = Snapshot {
        goal: "sk-secret123456".into(),
        next_action: "Bearer bearer-value".into(),
        criteria: vec![Criterion {
            text: "https://user:pass@example.test".into(),
            met: false,
        }],
        partner_drafts: partner_drafts(vec![Draft {
            field: EditorField::Goal,
            text: raw.into(),
            cursor: raw.len() - 3,
        }]),
        ..snapshot("project", 1)
    };
    let mut saved = store.save(task).unwrap();
    assert!(saved.redacted);
    assert!(
        saved.partner_drafts[&1].drafts[0]
            .text
            .is_char_boundary(saved.partner_drafts[&1].drafts[0].cursor)
    );
    let connection = store.connection.as_ref().unwrap();
    let raw: String = connection
        .query_row(
            "SELECT goal||criteria||next_action||partner_drafts FROM tasks",
            [],
            |row| row.get(0),
        )
        .unwrap();
    for secret in ["secret123456", "bearer-value", "user:pass", "super-secret"] {
        assert!(!raw.contains(secret));
    }
    saved.revision += 1;
    saved.goal = "ordinary goal".into();
    saved.redacted = false;
    assert!(store.save(saved).unwrap().redacted);
    let output = directory.0.join("export.json");
    export_readable(&directory.0, &output).unwrap();
    let text = fs::read_to_string(output).unwrap();
    assert!(!text.contains("super-secret"));
    assert!(text.contains("\"redacted\": true"));
}

#[test]
fn migrations_redact_all_legacy_payloads_and_exports_without_rewriting_identities() {
    for version in [0, 1, 2] {
        let directory = Directory::new();
        let path = directory.0.join(DATABASE);
        let connection = Connection::open(&path).unwrap();
        if version < 2 {
            legacy(&connection, version);
            let table = if version == 0 { "goals" } else { "projects" };
            connection
                .execute(
                    &format!("UPDATE {table} SET goal='password=legacy-secret'"),
                    [],
                )
                .unwrap();
        } else {
            connection.execute_batch(
                    "CREATE TABLE tasks(id INTEGER PRIMARY KEY,project TEXT NOT NULL,goal TEXT NOT NULL,form INTEGER NOT NULL,revision INTEGER NOT NULL,criteria TEXT NOT NULL,next_action TEXT NOT NULL,complete INTEGER NOT NULL,drafts TEXT NOT NULL,navigation TEXT NOT NULL,updated_at INTEGER NOT NULL);
                     CREATE TABLE selections(project TEXT PRIMARY KEY,active_id INTEGER NOT NULL);
                     PRAGMA user_version=2;"
                ).unwrap();
            connection.execute("INSERT INTO tasks VALUES (7,'project','password=legacy-secret',0,1,?1,'Bearer legacy-bearer',0,?2,?3,0)",
                    params![r#"[{"text":"token=legacy-token","met":false}]"#, r#"[{"field":"Goal","text":"ghp_legacysecret","cursor":15}]"#, serde_json::to_string(&Navigation::default()).unwrap()]).unwrap();
            connection
                .execute("INSERT INTO selections VALUES ('project',7)", [])
                .unwrap();
        }
        drop(connection);
        let before = fs::read(&path).unwrap();
        let output = directory.0.join("legacy-export.json");
        export_readable(&directory.0, &output).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(
            !fs::read_to_string(output)
                .unwrap()
                .contains("legacy-secret")
        );
        {
            let mut connection = Connection::open(&path).unwrap();
            {
                let transaction = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .unwrap();
                migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
                // Dropping the transaction models interruption before commit.
            }
            assert_eq!(
                connection
                    .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                    .unwrap(),
                i64::from(version)
            );
            let pages: i64 = connection
                .pragma_query_value(None, "page_count", |row| row.get(0))
                .unwrap();
            connection
                .pragma_update(None, "max_page_count", pages)
                .unwrap();
            assert!(initialize(&mut connection).is_err());
            assert_eq!(
                connection
                    .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                    .unwrap(),
                i64::from(version)
            );
        }
        let store = state(&directory, "project", Policy::default());
        let connection = store.connection.as_ref().unwrap();
        let raw: String = connection
            .query_row(
                "SELECT goal||criteria||next_action||drafts FROM tasks WHERE project='project'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        for secret in [
            "legacy-secret",
            "legacy-bearer",
            "legacy-token",
            "legacysecret",
        ] {
            assert!(!raw.contains(secret));
        }
        assert!(load(connection, "project").unwrap().tasks[0].redacted);
    }
    let directory = Directory::new();
    let secret = "/project/ghp_secret123456";
    let worker = worker(&directory, secret);
    assert!(
        matches!(event(&worker), Event::Failed { message, .. } if !message.contains(secret) && message.contains("identity"))
    );
    assert!(!directory.0.join(DATABASE).exists());
}

#[test]
fn secret_bearing_legacy_identity_is_refused_without_rewriting_or_merging() {
    let directory = Directory::new();
    let path = directory.0.join(DATABASE);
    let connection = Connection::open(&path).unwrap();
    legacy(&connection, 1);
    connection
        .execute(
            "UPDATE projects SET project='/projects/ghp_identitysecret' WHERE project='project'",
            [],
        )
        .unwrap();
    drop(connection);
    let before = fs::read(&path).unwrap();
    let store = worker(&directory, "ordinary-project");
    assert!(
        matches!(event(&store), Event::Failed { message, .. } if message.contains("identity") && !message.contains("ghp_identitysecret"))
    );
    drop(store);
    assert_eq!(fs::read(&path).unwrap(), before);
    let output = directory.0.join("refused.json");
    assert!(export_readable(&directory.0, &output).is_err());
    assert!(!output.exists());
}

#[test]
fn management_events_follow_commit_and_failures_preserve_records() {
    let directory = Directory::new();
    let store = worker(&directory, "project");
    loaded(&store);
    let task = create(&store, snapshot("project", 1));
    let mut request = Some(Management::Archive {
        id: task.id,
        archived: true,
    });
    assert!(store.try_manage(&mut request));
    assert!(request.is_none());
    let managed = match event(&store) {
        Event::Managed {
            project,
            affected,
            warning,
        } => {
            assert!(warning.is_none());
            assert_eq!(affected, vec![task.id]);
            project
        }
        other => panic!("expected committed management: {other:?}"),
    };
    let readonly = open_readonly(&directory.0.join(DATABASE)).unwrap();
    assert_eq!(load(&readonly, "project").unwrap(), managed);
    let mut request = Some(Management::Delete {
        ids: vec![task.id, 99999],
        expired_before: None,
    });
    assert!(store.try_manage(&mut request));
    assert!(matches!(event(&store), Event::ManagementFailed(_)));
    assert_eq!(load(&readonly, "project").unwrap(), managed);
}

#[test]
fn real_quota_rolls_back_and_deletion_recovers_without_eviction() {
    let directory = Directory::new();
    let initial = state(&directory, "project", Policy::default());
    let connection = initial.connection.as_ref().unwrap();
    let schema_bytes = live_bytes(connection).unwrap();
    let page_bytes: u64 = connection
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .unwrap();
    drop(initial);
    let policy = Policy {
        quota_bytes: schema_bytes + 4 * page_bytes,
        ..Policy::default()
    };
    let mut store = state(&directory, "project", policy);
    let mut committed = Vec::new();
    let large = Snapshot {
        goal: "q".repeat(4096),
        ..snapshot("project", 1)
    };
    loop {
        match store.save(large.clone()) {
            Ok(saved) => committed.push(saved),
            Err(error) => {
                assert!(error.contains("full") || error.contains("quota"), "{error}");
                break;
            }
        }
        assert!(
            committed.len() < 64,
            "a bounded SQLite quota must stop actual payload growth"
        );
    }
    assert!(!committed.is_empty());
    let connection = store.connection.as_ref().unwrap();
    assert_eq!(load(connection, "project").unwrap().tasks, committed);
    assert!(live_bytes(connection).unwrap() <= policy.quota_bytes);
    let max: u64 = connection
        .pragma_query_value(None, "max_page_count", |row| row.get(0))
        .unwrap();
    let size: u64 = connection
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .unwrap();
    assert!(max * size <= policy.quota_bytes);
    let deleted = committed.remove(0);
    store
        .manage(Management::Delete {
            ids: vec![deleted.id],
            expired_before: None,
        })
        .unwrap();
    let retry = store.save(large).unwrap();
    committed.push(retry);
    assert_eq!(
        load(store.connection.as_ref().unwrap(), "project")
            .unwrap()
            .tasks,
        committed
    );
    drop(store);
    let small = Policy {
        quota_bytes: schema_bytes + page_bytes,
        ..policy
    };
    let mut store = state(&directory, "project", small);
    assert_eq!(
        load(store.connection.as_ref().unwrap(), "project")
            .unwrap()
            .tasks,
        committed
    );
    assert!(
        store.save(snapshot("project", 1)).is_err(),
        "freelist reuse must still obey the smaller live-page quota"
    );
    store
        .manage(Management::Delete {
            ids: committed.iter().map(|task| task.id).collect(),
            expired_before: None,
        })
        .unwrap();
    assert!(
        store.save(snapshot("project", 1)).is_ok(),
        "deletion must recover capacity even without shrinking the physical SQLite file"
    );
}

#[test]
fn maximum_valid_text_and_json_escaping_round_trip() {
    let directory = Directory::new();
    let store = worker(&directory, "project");
    loaded(&store);
    let text = "\0".repeat(4096);
    let expected = create(
        &store,
        Snapshot {
            goal: text.clone(),
            next_action: text.clone(),
            command: text.clone(),
            criteria: vec![
                Criterion {
                    text: "\0".repeat(256),
                    met: true
                };
                64
            ],
            complete: true,
            partner_drafts: partner_drafts(
                [
                    EditorField::Goal,
                    EditorField::Criteria,
                    EditorField::NextAction,
                    EditorField::Command,
                ]
                .into_iter()
                .map(|field| Draft {
                    field,
                    text: text.clone(),
                    cursor: 4096,
                })
                .collect(),
            ),
            ..snapshot("project", 1)
        },
    );
    drop(store);
    let store = worker(&directory, "project");
    assert_eq!(loaded(&store).tasks, vec![expected]);
}
fn captured_excerpt(task_id: i64, source: &Path, text: &str) -> crate::workspace::context::Excerpt {
    use sha2::{Digest, Sha256};
    crate::workspace::context::Excerpt {
        origin_run: None,
        id: 0,
        task_id,
        root: source.parent().unwrap().to_str().unwrap().into(),
        path: source.file_name().unwrap().to_str().unwrap().into(),
        start_line: 1,
        end_line: 1,
        source_sha256: Sha256::digest(text.as_bytes()).into(),
        text: text.into(),
        available: true,
    }
}

fn prepared_run(task_id: i64, root: &Path, suffix: &str) -> crate::workspace::runner::Prepared {
    crate::workspace::runner::Prepared {
        approval_arrival: 0,
        request_id: format!("request-{suffix}"),
        run_id: format!("run-{suffix}"),
        task_id,
        spec: crate::workspace::runner::CommandSpec::Direct {
            program: "echo".into(),
            args: vec!["password=journal-value".into()],
        },
        cwd: root.to_path_buf(),
        cwd_identity: "fixture-directory".into(),
        executable: crate::workspace::runner::ExecutableIdentity {
            path: root.join("fixture-executable"),
            sha256: "0".repeat(64),
            bytes: 1,
            modified_ns: 1,
            file_id: "fixture-file".into(),
        },
        prepared_at_ms: timestamp().unwrap() as u64,
        expires_at_ms: timestamp().unwrap() as u64 + 60_000,
        before: crate::workspace::runner::InputSnapshot::default(),
        partner: None,
        rerun_of: None,
    }
}

#[test]
fn ordinary_pin_refuses_application_storage_without_mutating_retained_data() {
    let directory = Directory::new();
    let sources = Directory::new();
    let project = sources.0.canonicalize().unwrap();
    let project = project.to_str().unwrap();
    let mut store = state(&directory, project, Policy::default());
    let task = store.save(snapshot(project, 1)).unwrap();
    let private = directory.0.join("ev-run-retained.output");
    fs::write(&private, b"other task private output\n").unwrap();
    let request = |path: &Path, text: &str| ContextRequest {
        producer: None,
        task_id: task.id,
        operation: ContextOperation::Pin(captured_excerpt(task.id, path, text)),
    };
    assert!(
        store
            .context(request(&private, "other task private output\n"))
            .is_err()
    );
    assert_eq!(fs::read(&private).unwrap(), b"other task private output\n");
    let public = Path::new(project).join("source.rs");
    fs::write(&public, b"public source\n").unwrap();
    let basket = store.context(request(&public, "public source\n")).unwrap();
    assert_eq!(basket.len(), 1);
    assert_eq!(basket[0].path, "source.rs");
    assert_eq!(basket[0].text, "public source\n");
}

#[test]
fn context_is_independent_masked_task_owned_and_purge_preserves_unavailable_metadata() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let first = store.save(snapshot("project", 1)).unwrap();
    let second = store.save(snapshot("project", 1)).unwrap();
    let sources = Directory::new();
    let source = sources.0.join("source.rs");
    fs::write(&source, "password=source-value\n").unwrap();
    let pin = |task_id| ContextRequest {
        producer: None,
        task_id,
        operation: ContextOperation::Pin(captured_excerpt(
            task_id,
            &source,
            "password=source-value\n",
        )),
    };
    let one = store.context(pin(first.id)).unwrap();
    let two = store.context(pin(second.id)).unwrap();
    assert_eq!(one[0].text, "password=[REDACTED]\n");
    let connection = store.connection.as_ref().unwrap();
    let raw: String = connection
        .query_row(
            "SELECT record FROM excerpts WHERE task_id=?1",
            [first.id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!raw.contains("source-value"));
    let mut updated = first.clone();
    updated.revision += 1;
    updated.goal = "changed without touching source payload".into();
    store.save(updated).unwrap();
    assert_eq!(
        store
            .context(ContextRequest {
                task_id: first.id,
                producer: None,
                operation: ContextOperation::Load
            })
            .unwrap(),
        one
    );
    assert!(
        store
            .context(ContextRequest {
                task_id: second.id,
                producer: None,
                operation: ContextOperation::Remove(one[0].id)
            })
            .is_err()
    );
    let purged = store
        .context(ContextRequest {
            producer: None,
            task_id: first.id,
            operation: ContextOperation::Purge(vec![one[0].id]),
        })
        .unwrap();
    assert!(!purged[0].available && purged[0].text.is_empty());
    assert!(
        store
            .context(ContextRequest {
                task_id: first.id,
                producer: None,
                operation: ContextOperation::Validate(one)
            })
            .is_err()
    );
    assert_eq!(
        store
            .context(ContextRequest {
                task_id: second.id,
                producer: None,
                operation: ContextOperation::Load
            })
            .unwrap(),
        two
    );
    assert_eq!(
        fs::read_to_string(&source).unwrap(),
        "password=source-value\n"
    );
    store
        .manage(Management::Delete {
            ids: vec![first.id],
            expired_before: None,
        })
        .unwrap();
    assert_eq!(
        store
            .context(ContextRequest {
                task_id: second.id,
                producer: None,
                operation: ContextOperation::Load
            })
            .unwrap(),
        two
    );
}

#[test]
fn reservations_commit_before_permit_mask_journal_and_refuse_active_deletion_and_purge() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let prepared = prepared_run(task.id, &store.data_dir, "active");
    let mut permit = store.reserve_run(task.id, prepared.clone(), 4096).unwrap();
    let connection = open_readonly(&store.data_dir.join(DATABASE)).unwrap();
    let stored = stored_run(&connection, task.id, &prepared.run_id).unwrap();
    assert!(stored.reserved);
    assert_eq!(
        stored.record.state,
        crate::workspace::runner::RunState::AwaitingDispatch
    );
    assert!(stored.record.started_at_ms.is_none());
    assert!(
        !serde_json::to_string(&stored.record)
            .unwrap()
            .contains("journal-value")
    );
    assert_eq!(permit.capacity, 4096);
    assert!(
        store
            .manage(Management::Delete {
                ids: vec![task.id],
                expired_before: None
            })
            .is_err()
    );
    assert!(
        store
            .manage(Management::Archive {
                id: task.id,
                archived: true
            })
            .is_err()
    );
    assert!(store.purge_output(task.id, &prepared.run_id).is_err());
    assert!(
        store
            .reserve_run(
                task.id,
                prepared_run(task.id, &store.data_dir, "second"),
                1024
            )
            .is_err()
    );
    permit.file.write_all(b"partial output\n").unwrap();
    permit.file.sync_all().unwrap();
    let mut record = crate::workspace::runner::RunRecord::awaiting(&prepared);
    record.state = crate::workspace::runner::RunState::Running;
    record.started_at_ms = Some(timestamp().unwrap() as u64);
    store.update_run(task.id, record.clone()).unwrap();
    record.state = crate::workspace::runner::RunState::Succeeded;
    record.ended_at_ms = Some(timestamp().unwrap() as u64);
    record.exit_code = Some(0);
    record.tree = crate::workspace::runner::TreeOutcome::OwnedTreeExited;
    record.captured_bytes = 15;
    drop(permit);
    store.update_run(task.id, record.clone()).unwrap();
    assert!(
        store.update_run(task.id, record).is_err(),
        "late duplicate finish must not rewrite history"
    );
    let output = store
        .read_output(task.id, &prepared.run_id, 0, 4096)
        .unwrap();
    assert_eq!(output.text, "partial output\n");
    assert!(!output.unavailable);
    let (purged, warning) = store.purge_output(task.id, &prepared.run_id).unwrap();
    assert!(warning.is_none());
    assert!(purged.output_unavailable);
    assert!(
        store
            .read_output(task.id, &prepared.run_id, 0, 4096)
            .unwrap()
            .unavailable
    );
}

#[test]
fn committed_purge_and_deletion_return_metadata_when_artifact_cleanup_fails() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let survivor = store.save(snapshot("project", 1)).unwrap();
    store.select(task.id).unwrap();
    let prepared = prepared_run(task.id, &store.data_dir, "cleanup-failure");
    let mut permit = store.reserve_run(task.id, prepared.clone(), 1024).unwrap();
    permit.file.write_all(b"retained\n").unwrap();
    permit.file.sync_all().unwrap();
    drop(permit);
    let mut record = crate::workspace::runner::RunRecord::awaiting(&prepared);
    record.state = crate::workspace::runner::RunState::Running;
    record.started_at_ms = Some(timestamp().unwrap() as u64);
    store.update_run(task.id, record.clone()).unwrap();
    record.state = crate::workspace::runner::RunState::Succeeded;
    record.ended_at_ms = Some(timestamp().unwrap() as u64);
    record.exit_code = Some(0);
    record.tree = crate::workspace::runner::TreeOutcome::OwnedTreeExited;
    record.captured_bytes = 9;
    let record = store.update_run(task.id, record).unwrap();
    assert!(!record.output_unavailable);

    let data_dir = store.data_dir.clone();
    let output = data_dir.join(spool_name(&prepared.run_id).unwrap());
    // Keep SQLite available but make artifact-directory access fail deterministically,
    // without relying on filesystem permissions or the test runner's privileges.
    let blocked_directory = data_dir.join("not-a-directory");
    fs::write(&blocked_directory, b"preserve").unwrap();
    store.data_dir = blocked_directory.clone();
    let response = store
        .run_request(RunRequest {
            task_id: task.id,
            operation: RunOperation::PurgeOutput {
                run_id: prepared.run_id.clone(),
            },
        })
        .unwrap();
    let RunResponse::Purged {
        record: purged,
        warning,
    } = response
    else {
        panic!("expected committed purge response");
    };
    assert!(warning.is_some());
    assert!(purged.output_unavailable);
    assert_eq!(purged.state, crate::workspace::runner::RunState::Succeeded);
    assert_eq!(purged.prepared.run_id, prepared.run_id);
    let readonly = open_readonly(&data_dir.join(DATABASE)).unwrap();
    assert!(
        stored_run(&readonly, task.id, &prepared.run_id)
            .unwrap()
            .record
            .output_unavailable
    );
    assert_eq!(fs::read(&output).unwrap(), b"retained\n");
    store.data_dir = data_dir.clone();
    assert!(
        store
            .read_output(task.id, &prepared.run_id, 0, 64)
            .unwrap()
            .unavailable
    );

    store.data_dir = blocked_directory;
    let Managed {
        project,
        affected,
        warning,
    } = store
        .manage(Management::Delete {
            ids: vec![task.id],
            expired_before: None,
        })
        .unwrap();
    assert!(warning.is_some());
    assert_eq!(affected, vec![task.id]);
    assert_eq!(project.tasks, vec![survivor.clone()]);
    assert_eq!(project.active_id, Some(survivor.id));
    assert_eq!(load(&readonly, "project").unwrap(), project);
    assert!(stored_run(&readonly, task.id, &prepared.run_id).is_err());
    assert_eq!(fs::read(&output).unwrap(), b"retained\n");
    drop(readonly);
    store.data_dir = data_dir;
    drop(store);
    let store = state(&directory, "project", Policy::default());
    let reopened = load(store.connection.as_ref().unwrap(), "project").unwrap();
    assert_eq!(reopened.tasks, vec![survivor]);
    assert_eq!(reopened.active_id, project.active_id);
}

#[test]
fn quota_accounts_live_sqlite_retained_spools_and_reserved_capacity_without_eviction() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let initial = live_bytes(store.connection.as_ref().unwrap()).unwrap();
    store.policy.quota_bytes = initial + RUN_HEADROOM + 4096;
    let before = fs::read(store.data_dir.join(DATABASE)).unwrap();
    assert!(
        store
            .reserve_run(
                task.id,
                prepared_run(task.id, &store.data_dir, "quota"),
                8192
            )
            .is_err()
    );
    assert!(store.load_runs(task.id).unwrap().is_empty());
    assert_eq!(
        load_task(store.connection.as_ref().unwrap(), "project", task.id).unwrap(),
        Some(task.clone())
    );
    assert_eq!(fs::read(store.data_dir.join(DATABASE)).unwrap(), before);
    store.policy = Policy::default();
    let permit = store
        .reserve_run(
            task.id,
            prepared_run(task.id, &store.data_dir, "reserved"),
            8192,
        )
        .unwrap();
    let used = evidence_usage(store.connection.as_ref().unwrap(), &store.data_dir).unwrap();
    assert!(used >= initial + 8192 + RUN_HEADROOM);
    store.policy.quota_bytes = used;
    let large = Snapshot {
        goal: "x".repeat(4096),
        ..snapshot("project", 1)
    };
    assert!(store.save(large).is_err());
    assert!(
        store.load_runs(task.id).unwrap()[0].state
            == crate::workspace::runner::RunState::AwaitingDispatch
    );
    drop(permit);
}

#[test]
fn restart_interrupts_reservations_and_retains_partial_bytes_without_claiming_running() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let prepared = prepared_run(task.id, &store.data_dir, "restart");
    let mut permit = store.reserve_run(task.id, prepared.clone(), 4096).unwrap();
    let mut record = crate::workspace::runner::RunRecord::awaiting(&prepared);
    record.state = crate::workspace::runner::RunState::Running;
    record.started_at_ms = Some(timestamp().unwrap() as u64);
    store.update_run(task.id, record).unwrap();
    permit.file.write_all(b"retained\n").unwrap();
    permit.file.sync_all().unwrap();
    assert!(
        matches!(store.reopen(), Event::Failed { .. }),
        "retry cannot abandon live ownership"
    );
    drop(permit);
    drop(store);
    let mut store = state(&directory, "project", Policy::default());
    let records = store.load_runs(task.id).unwrap();
    assert_eq!(
        records[0].state,
        crate::workspace::runner::RunState::Interrupted
    );
    assert_eq!(
        records[0].tree,
        crate::workspace::runner::TreeOutcome::OwnershipUnknown
    );
    assert!(records[0].truncated);
    assert_eq!(
        store
            .read_output(task.id, &prepared.run_id, 0, 4096)
            .unwrap()
            .text,
        "retained\n"
    );
    assert!(
        store
            .reserve_run(
                task.id,
                prepared_run(task.id, &store.data_dir, "after-restart"),
                1024
            )
            .is_ok()
    );
}

#[cfg(unix)]
#[test]
fn replaced_output_links_are_unavailable_and_purge_never_deletes_external_files() {
    use std::os::unix::fs::symlink;
    let directory = Directory::new();
    let external = Directory::new();
    let outside = external.0.join("preserve.txt");
    fs::write(&outside, "external bytes").unwrap();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let prepared = prepared_run(task.id, &store.data_dir, "swap");
    let permit = store.reserve_run(task.id, prepared.clone(), 1024).unwrap();
    drop(permit);
    store
        .run_request(RunRequest {
            task_id: task.id,
            operation: RunOperation::Abandon {
                request_id: prepared.request_id.clone(),
                run_id: prepared.run_id.clone(),
            },
        })
        .unwrap();
    let output = store.data_dir.join(spool_name(&prepared.run_id).unwrap());
    fs::remove_file(&output).unwrap();
    symlink(&outside, &output).unwrap();
    assert!(
        store
            .read_output(task.id, &prepared.run_id, 0, 64)
            .unwrap()
            .unavailable
    );
    store.purge_output(task.id, &prepared.run_id).unwrap();
    assert_eq!(fs::read_to_string(&outside).unwrap(), "external bytes");
    store
        .manage(Management::Delete {
            ids: vec![task.id],
            expired_before: None,
        })
        .unwrap();
    assert_eq!(fs::read_to_string(&outside).unwrap(), "external bytes");
}

#[test]
fn schema_three_migration_is_transactional_and_preserves_archives_and_four_command_drafts() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let first = store.save(snapshot("project", 1)).unwrap();
    let second = store.save(snapshot("project", 1)).unwrap();
    store
        .manage(Management::Archive {
            id: first.id,
            archived: true,
        })
        .unwrap();
    drop(store);
    let path = directory.0.join(DATABASE);
    let mut connection = Connection::open(&path).unwrap();
    downgrade_workflows(&connection);
    connection.execute_batch("DROP TABLE provider_journal; DROP TABLE excerpts; DROP TABLE executions; ALTER TABLE tasks DROP COLUMN command; PRAGMA user_version=3;").unwrap();
    {
        let transaction = connection.transaction().unwrap();
        migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
        // No commit: the complete schema-three task/selection remains usable.
    }
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert!(matches!(inspect(&connection).unwrap(), Schema::Lifecycle));
    drop(connection);
    let mut store = state(&directory, "project", Policy::default());
    let project = load(store.connection.as_ref().unwrap(), "project").unwrap();
    assert!(project.tasks[0].archived && project.tasks[0].command.is_empty());
    assert_eq!(project.active_id, Some(second.id));
    let mut task = project.tasks[1].clone();
    task.revision += 1;
    task.command = "echo password=command-value".into();
    task.partner_drafts = partner_drafts(
        [
            EditorField::Goal,
            EditorField::Criteria,
            EditorField::NextAction,
            EditorField::Command,
        ]
        .into_iter()
        .map(|field| Draft {
            field,
            text: "token=draft-value".into(),
            cursor: 17,
        })
        .collect(),
    );
    task.partner_drafts.get_mut(&1).unwrap().editor_field = EditorField::Command;
    task.navigation.editor_field = EditorField::Command;
    let saved = store.save(task).unwrap();
    assert_eq!(saved.command, "echo password=[REDACTED]");
    assert_eq!(saved.partner_drafts[&1].drafts.len(), 4);
    assert!(
        saved.partner_drafts[&1]
            .drafts
            .iter()
            .all(|draft| !draft.text.contains("draft-value"))
    );
    drop(store);
    let store = state(&directory, "project", Policy::default());
    assert_eq!(
        load_task(store.connection.as_ref().unwrap(), "project", second.id).unwrap(),
        Some(saved)
    );
}

#[test]
fn excerpt_capacity_and_lowered_quota_never_evict_and_still_allow_explicit_reclamation() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let sources = Directory::new();
    let source = sources.0.join("source.rs");
    fs::write(&source, "visible\n").unwrap();
    for _ in 0..crate::workspace::context::MAX_EXCERPTS {
        store
            .context(ContextRequest {
                producer: None,
                task_id: task.id,
                operation: ContextOperation::Pin(captured_excerpt(task.id, &source, "visible\n")),
            })
            .unwrap();
    }
    assert!(
        store
            .context(ContextRequest {
                task_id: task.id,
                producer: None,
                operation: ContextOperation::Pin(captured_excerpt(task.id, &source, "visible\n"))
            })
            .is_err()
    );
    let basket = store
        .context(ContextRequest {
            producer: None,
            task_id: task.id,
            operation: ContextOperation::Load,
        })
        .unwrap();
    assert_eq!(basket.len(), 16);
    store.policy.quota_bytes = 1;
    let reduced = store
        .context(ContextRequest {
            producer: None,
            task_id: task.id,
            operation: ContextOperation::Remove(basket[0].id),
        })
        .unwrap();
    assert_eq!(reduced.len(), 15);
    let purged = store
        .context(ContextRequest {
            producer: None,
            task_id: task.id,
            operation: ContextOperation::Purge(vec![basket[1].id]),
        })
        .unwrap();
    assert!(!purged[0].available);
    assert_eq!(fs::read_to_string(source).unwrap(), "visible\n");
}

#[test]
fn completed_output_is_bounded_missing_is_unavailable_and_task_purge_is_isolated() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let first = store.save(snapshot("project", 1)).unwrap();
    let second = store.save(snapshot("project", 1)).unwrap();
    let mut runs = Vec::new();
    for (task, suffix) in [(first.id, "first"), (second.id, "second")] {
        let prepared = prepared_run(task, &store.data_dir, suffix);
        let mut permit = store.reserve_run(task, prepared.clone(), 1024).unwrap();
        permit.file.write_all("前\n".as_bytes()).unwrap();
        permit.file.sync_all().unwrap();
        let mut record = crate::workspace::runner::RunRecord::awaiting(&prepared);
        record.state = crate::workspace::runner::RunState::Running;
        record.started_at_ms = Some(timestamp().unwrap() as u64);
        store.update_run(task, record.clone()).unwrap();
        record.state = crate::workspace::runner::RunState::Succeeded;
        record.ended_at_ms = Some(timestamp().unwrap() as u64);
        record.exit_code = Some(0);
        record.tree = crate::workspace::runner::TreeOutcome::OwnedTreeExited;
        record.captured_bytes = 4;
        drop(permit);
        store.update_run(task, record).unwrap();
        runs.push(prepared.run_id);
    }
    assert!(store.read_output(first.id, &runs[0], 0, 2).is_err());
    assert_eq!(
        store.read_output(first.id, &runs[0], 0, 3).unwrap().text,
        "前"
    );
    assert_eq!(
        store.read_output(first.id, &runs[0], 3, 1).unwrap().text,
        "\n"
    );
    assert!(store.read_output(first.id, &runs[0], 0, 65_537).is_err());
    assert!(store.read_output(second.id, &runs[0], 0, 64).is_err());
    fs::remove_file(store.data_dir.join(spool_name(&runs[0]).unwrap())).unwrap();
    let records = store.load_runs(first.id).unwrap();
    assert_eq!(
        records[0].state,
        crate::workspace::runner::RunState::Succeeded
    );
    assert!(records[0].output_unavailable);
    assert!(
        store
            .read_output(first.id, &runs[0], 0, 64)
            .unwrap()
            .unavailable
    );
    store.purge_output(first.id, &runs[0]).unwrap();
    store
        .manage(Management::Delete {
            ids: vec![first.id],
            expired_before: None,
        })
        .unwrap();
    assert_eq!(
        store.read_output(second.id, &runs[1], 0, 64).unwrap().text,
        "前\n"
    );
}

#[test]
fn foreign_output_reference_is_refused_before_writable_open() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let prepared = prepared_run(task.id, &store.data_dir, "foreign");
    let permit = store.reserve_run(task.id, prepared.clone(), 1024).unwrap();
    drop(permit);
    store
        .run_request(RunRequest {
            task_id: task.id,
            operation: RunOperation::Abandon {
                request_id: prepared.request_id,
                run_id: prepared.run_id,
            },
        })
        .unwrap();
    store
        .connection
        .as_ref()
        .unwrap()
        .execute("UPDATE executions SET output_name='../outside'", [])
        .unwrap();
    drop(store);
    let path = directory.0.join(DATABASE);
    let before = fs::read(&path).unwrap();
    let worker = worker(&directory, "project");
    assert!(matches!(event(&worker), Event::Failed { .. }));
    drop(worker);
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn deletion_preview_lists_exact_owned_artifacts_without_mutation_and_rechecks_active_work() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let sources = Directory::new();
    let source = sources.0.join("source.rs");
    fs::write(&source, "visible\n").unwrap();
    let basket = store
        .context(ContextRequest {
            producer: None,
            task_id: task.id,
            operation: ContextOperation::Pin(captured_excerpt(task.id, &source, "visible\n")),
        })
        .unwrap();
    let prepared = prepared_run(task.id, &store.data_dir, "preview");
    let permit = store.reserve_run(task.id, prepared.clone(), 1024).unwrap();
    assert!(store.preview_delete(&[task.id], None).is_err());
    assert!(
        store
            .context(ContextRequest {
                task_id: task.id,
                producer: None,
                operation: ContextOperation::Purge(vec![basket[0].id])
            })
            .is_err()
    );
    drop(permit);
    store
        .run_request(RunRequest {
            task_id: task.id,
            operation: RunOperation::Abandon {
                request_id: prepared.request_id,
                run_id: prepared.run_id.clone(),
            },
        })
        .unwrap();
    let before = fs::read(store.data_dir.join(DATABASE)).unwrap();
    let preview = store.preview_delete(&[task.id], None).unwrap();
    assert_eq!(preview.len(), 1);
    assert_eq!(preview[0].task.id, task.id);
    assert_eq!(
        preview[0].excerpts,
        vec![(basket[0].id, "source.rs".into())]
    );
    assert_eq!(preview[0].runs, vec![(prepared.run_id, true)]);
    assert_eq!(fs::read(store.data_dir.join(DATABASE)).unwrap(), before);
    assert_eq!(fs::read_to_string(&source).unwrap(), "visible\n");
    let permit = store
        .reserve_run(
            task.id,
            prepared_run(task.id, &store.data_dir, "after-preview"),
            1024,
        )
        .unwrap();
    assert!(
        store
            .manage(Management::Delete {
                ids: vec![task.id],
                expired_before: None
            })
            .is_err()
    );
    assert!(
        load_task(store.connection.as_ref().unwrap(), "project", task.id)
            .unwrap()
            .is_some()
    );
    drop(permit);
}
#[test]
fn planning_admission_rejects_changed_task_and_purged_context() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let saved = store.save(snapshot("project", 1)).unwrap();
    validate_planning_input(&store.data_dir, &saved, &[]).unwrap();
    let sources = Directory::new();
    let source = sources.0.join("source.rs");
    fs::write(&source, "visible\n").unwrap();
    let basket = store
        .context(ContextRequest {
            producer: None,
            task_id: saved.id,
            operation: ContextOperation::Pin(captured_excerpt(saved.id, &source, "visible\n")),
        })
        .unwrap();
    validate_planning_input(&store.data_dir, &saved, &basket).unwrap();
    store
        .context(ContextRequest {
            producer: None,
            task_id: saved.id,
            operation: ContextOperation::Purge(vec![basket[0].id]),
        })
        .unwrap();
    assert!(validate_planning_input(&store.data_dir, &saved, &basket).is_err());
    let mut changed = saved.clone();
    changed.revision += 1;
    changed.goal = "A different decision needs a new preview".into();
    let current = store.save(changed).unwrap();
    assert!(validate_planning_input(&store.data_dir, &saved, &[]).is_err());
    validate_planning_input(&store.data_dir, &current, &[]).unwrap();
}

#[test]
fn terminal_run_observations_recover_after_failed_start_commit() {
    use crate::workspace::runner::{RunRecord, RunState, TreeOutcome};
    for terminal in [RunState::Succeeded, RunState::Cancelled] {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        let prepared = prepared_run(task.id, &store.data_dir, "failed-start");
        let permit = store.reserve_run(task.id, prepared.clone(), 4096).unwrap();
        let mut record = RunRecord::awaiting(&prepared);
        record.state = RunState::Running;
        record.started_at_ms = Some(timestamp().unwrap() as u64);
        store
            .connection
            .as_ref()
            .unwrap()
            .pragma_update(None, "query_only", true)
            .unwrap();
        assert!(store.update_run(task.id, record.clone()).is_err());
        store
            .connection
            .as_ref()
            .unwrap()
            .pragma_update(None, "query_only", false)
            .unwrap();
        assert_eq!(
            stored_run(
                store.connection.as_ref().unwrap(),
                task.id,
                &prepared.run_id
            )
            .unwrap()
            .record
            .state,
            RunState::AwaitingDispatch
        );
        record.state = terminal;
        record.ended_at_ms = record.started_at_ms;
        record.exit_code = Some(0);
        record.tree = TreeOutcome::OwnedTreeExited;
        record.cancelled = terminal == RunState::Cancelled;
        drop(permit);
        store.update_run(task.id, record).unwrap();
        no_active(store.connection.as_ref().unwrap(), Some(task.id)).unwrap();
        drop(store);
        let reopened = state(&directory, "project", Policy::default());
        assert_eq!(reopened.load_runs(task.id).unwrap()[0].state, terminal);
    }
}

#[test]
fn stale_run_observation_cannot_erase_committed_capture_evidence() {
    use crate::workspace::runner::{RunRecord, RunState};
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let prepared = prepared_run(task.id, &store.data_dir, "monotonic");
    let permit = store.reserve_run(task.id, prepared.clone(), 4096).unwrap();
    let mut record = RunRecord::awaiting(&prepared);
    record.state = RunState::Running;
    record.started_at_ms = Some(timestamp().unwrap() as u64);
    record.captured_bytes = 10;
    record.discarded_bytes = 20;
    record.dropped_live_bytes = 5;
    record.truncated = true;
    record.output_unavailable = true;
    let committed = store.update_run(task.id, record.clone()).unwrap();
    for mutation in 0..5 {
        let mut stale = record.clone();
        match mutation {
            0 => stale.captured_bytes = 0,
            1 => stale.discarded_bytes = 0,
            2 => stale.dropped_live_bytes = 0,
            3 => stale.truncated = false,
            _ => stale.output_unavailable = false,
        }
        assert!(store.update_run(task.id, stale).is_err());
        assert_eq!(
            stored_run(
                store.connection.as_ref().unwrap(),
                task.id,
                &prepared.run_id
            )
            .unwrap()
            .record,
            committed
        );
    }
    drop(permit);
}

#[test]
fn lifecycle_migration_never_reuses_deleted_task_identities() {
    for keep_survivor in [false, true] {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let first = store.save(snapshot("project", 1)).unwrap();
        let deleted = store.save(snapshot("project", 1)).unwrap();
        let ids = if keep_survivor {
            vec![deleted.id]
        } else {
            vec![first.id, deleted.id]
        };
        store
            .manage(Management::Delete {
                ids,
                expired_before: None,
            })
            .unwrap();
        drop(store);
        let mut connection = Connection::open(directory.0.join(DATABASE)).unwrap();
        downgrade_workflows(&connection);
        connection
            .execute_batch(
                "DROP TABLE provider_journal; DROP TABLE excerpts; DROP TABLE executions;
                 ALTER TABLE tasks DROP COLUMN command; PRAGMA user_version=3;",
            )
            .unwrap();
        {
            let transaction = connection.transaction().unwrap();
            migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
            // Aborting the migration must preserve both schema and the high-water mark.
        }
        assert!(matches!(inspect(&connection).unwrap(), Schema::Lifecycle));
        let sequence: i64 = connection
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name='tasks'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sequence, deleted.id);
        drop(connection);
        let mut reopened = state(&directory, "project", Policy::default());
        assert!(reopened.save(snapshot("project", 1)).unwrap().id > deleted.id);
    }
}
fn memory_proposal(
    task_id: i64,
    scope: crate::workspace::memory::Scope,
) -> crate::workspace::memory::Proposal {
    crate::workspace::memory::Proposal {
        scope,
        content: "Prefer explicit, bounded changes".into(),
        provenance: crate::workspace::memory::Provenance {
            kind: crate::workspace::memory::SourceKind::UserAuthored,
            reference: "User-authored task note".into(),
            task_id,
        },
    }
}

#[test]
fn schema_six_migration_is_atomic_and_defaults_preserve_old_snapshots() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    drop(store);
    let mut connection = Connection::open(directory.0.join(DATABASE)).unwrap();
    downgrade_workflows(&connection);
    {
        let transaction = connection.transaction().unwrap();
        migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
        assert!(matches!(inspect(&transaction).unwrap(), Schema::Current));
    }
    assert!(matches!(
        inspect(&connection).unwrap(),
        Schema::ProviderJournal
    ));
    initialize(&mut connection).unwrap();
    assert_eq!(
        load_task(&connection, "project", task.id).unwrap().unwrap(),
        task
    );
    assert!(
        memories(&connection, "project", task.id)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(inspect(&connection).unwrap(), Schema::Current));
}

fn downgrade_moves(connection: &Connection) {
    super::records::remove_schema(connection);
    connection
        .execute_batch(
            "DROP TABLE moves; DROP TABLE move_candidates; DROP TABLE move_sources;
         DROP TABLE move_usages; DROP TABLE move_dispatches; PRAGMA user_version=13;",
        )
        .unwrap();
}

fn downgrade_partners(connection: &Connection) {
    downgrade_moves(connection);
    connection
        .execute_batch(
            "DROP TABLE provider_preferences;
             DROP TABLE active_work;
             DROP TABLE partners;
             ALTER TABLE tasks DROP COLUMN lead_partner;
             ALTER TABLE tasks DROP COLUMN partner_drafts;
             PRAGMA user_version=6;",
        )
        .unwrap();
}

#[test]
fn schema_seven_capacity_migration_preserves_escaped_drafts_evidence_and_deleted_ids() {
    for version in [6, 7] {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        drop(store);
        let mut connection = Connection::open(directory.0.join(DATABASE)).unwrap();
        downgrade_partners(&connection);
        if version == 7 {
            migrate_partners(&connection).unwrap();
        }
        let text = "\0".repeat(4096);
        let drafts: Vec<_> = [
            EditorField::Goal,
            EditorField::Criteria,
            EditorField::NextAction,
            EditorField::Command,
        ]
        .into_iter()
        .map(|field| Draft {
            field,
            text: text.clone(),
            cursor: text.len(),
        })
        .collect();
        let legacy = serde_json::to_string(&drafts).unwrap();
        assert!(legacy.len() > 65536);
        connection
            .execute("UPDATE tasks SET drafts=?1", [&legacy])
            .unwrap();
        let mut excerpt = captured_excerpt(
            task.id,
            &directory.0.join("source.txt"),
            "preserved evidence",
        );
        excerpt.id = 1;
        let evidence = serde_json::to_string(&excerpt).unwrap();
        connection
            .execute(
                "INSERT INTO excerpts(id,task_id,record) VALUES (1,?1,?2)",
                params![task.id, evidence],
            )
            .unwrap();
        connection
            .execute_batch(
                "UPDATE sqlite_sequence SET seq=900 WHERE name='tasks';
             UPDATE sqlite_sequence SET seq=700 WHERE name='excerpts';",
            )
            .unwrap();
        let schema = inspect(&connection).unwrap();
        assert!(matches!(
            (version, schema),
            (6, Schema::Workflows) | (7, Schema::Partners)
        ));
        {
            let transaction = connection.transaction().unwrap();
            migrate(&transaction, schema).unwrap();
            assert!(matches!(inspect(&transaction).unwrap(), Schema::Current));
            // Dropping the transaction must restore the original schema and rows.
        }
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            version
        );
        assert_eq!(
            connection
                .query_row("SELECT drafts FROM tasks", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            legacy
        );
        initialize(&mut connection).unwrap();
        assert!(matches!(inspect(&connection).unwrap(), Schema::Current));
        connection
            .execute_batch(
                "INSERT INTO partners(id,nickname,form,met_at,met_derived,created_project)
             SELECT 2,NULL,0,0,1,'project' UNION ALL SELECT 3,NULL,0,0,1,'project'
             UNION ALL SELECT 4,NULL,0,0,1,'project' UNION ALL SELECT 5,NULL,0,0,1,'project'
             UNION ALL SELECT 6,NULL,0,0,1,'project';",
            )
            .unwrap();
        let loaded = load(&connection, "project").unwrap();
        assert_eq!(loaded.tasks[0].partner_drafts[&1].drafts, drafts);
        assert_eq!(
            connection
                .query_row("SELECT record FROM excerpts", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            evidence
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT seq FROM sqlite_sequence WHERE name='tasks'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            900
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT seq FROM sqlite_sequence WHERE name='excerpts'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            700
        );
        let states: BTreeMap<_, _> = (1..=6)
            .map(|id| {
                (
                    id,
                    PartnerDrafts {
                        drafts: drafts.clone(),
                        ..PartnerDrafts::default()
                    },
                )
            })
            .collect();
        let encoded = serde_json::to_string(&states).unwrap();
        assert!(encoded.len() > 512 * 1024);
        assert!(encoded.len() < MAX_PARTNER_DRAFTS_BYTES);
        connection
            .execute("UPDATE tasks SET drafts='[]',partner_drafts=?1", [&encoded])
            .unwrap();
        assert_eq!(
            load(&connection, "project").unwrap().tasks[0].partner_drafts,
            states
        );
        let oversized = "x".repeat(MAX_PARTNER_DRAFTS_BYTES + 1);
        assert!(
            connection
                .execute("UPDATE tasks SET partner_drafts=?1", [&oversized])
                .is_err()
        );
        connection.execute(&format!(
            "INSERT INTO tasks({TASK_COLUMNS}) SELECT project,goal,form,revision,NULL,criteria,next_action,complete,drafts,navigation,updated_at,archived,archived_at,redacted,command,workflows,memory_draft,approved_memories,workflow_draft,lead_partner,partner_drafts FROM tasks"
        ), []).unwrap();
        assert_eq!(connection.last_insert_rowid(), 901);
    }
}

#[test]
fn schema_six_store_migrates_partner_one_and_resumes_tasks_unchanged() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let first = store
        .save(Snapshot {
            form: 2,
            ..snapshot("project", 1)
        })
        .unwrap();
    let second = store.save(snapshot("project", 1)).unwrap();
    drop(store);
    let path = directory.0.join(DATABASE);
    let mut connection = Connection::open(&path).unwrap();
    downgrade_partners(&connection);
    // Deterministic history: a foreign project's earliest task wins partner #1.
    connection
        .execute_batch(&format!(
            "INSERT INTO tasks(project,goal,form,revision,criteria,next_action,complete,drafts,
                 navigation,updated_at,archived,archived_at,redacted,command,workflows,
                 memory_draft,approved_memories,workflow_draft)
             SELECT 'other',goal,5,revision,criteria,next_action,complete,drafts,navigation,
                 200000,archived,archived_at,redacted,command,workflows,memory_draft,
                 approved_memories,workflow_draft FROM tasks WHERE id={};
             UPDATE tasks SET updated_at=300000 WHERE id={};
             UPDATE tasks SET updated_at=400000 WHERE id={};",
            first.id, first.id, second.id
        ))
        .unwrap();
    assert!(matches!(inspect(&connection).unwrap(), Schema::Workflows));
    {
        let transaction = connection.transaction().unwrap();
        migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
        transaction.commit().unwrap();
    }
    assert!(matches!(inspect(&connection).unwrap(), Schema::Current));
    // Partner #1: the oldest task's project and form, earliest met date,
    // derived flag set, no nickname, and every task leads partner #1.
    let partner: (i64, Option<String>, i64, i64, i64, String) = connection
        .query_row(
            "SELECT id,nickname,form,met_at,met_derived,created_project FROM partners",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(partner, (1, None, 5, 200000, 1, "other".to_string()));
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM partners WHERE id != 1", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM tasks WHERE lead_partner != 1 OR partner_drafts != '{}'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    drop(connection);
    // Every task resumes unchanged: snapshots equal the pre-migration rows.
    let reopened = worker(&directory, "project");
    let project = loaded(&reopened);
    assert_eq!(
        project.partners,
        vec![Partner {
            id: 1,
            nickname: None,
            form: 5,
            auto_form: false,
            met_at: 200000,
            met_derived: true,
            created_project: "other".into(),
        }]
    );
    assert_eq!(
        project.tasks,
        vec![
            Snapshot {
                lead_partner: 1,
                updated_at: 300000,
                ..first
            },
            Snapshot {
                lead_partner: 1,
                updated_at: 400000,
                ..second
            },
        ]
    );
    drop(reopened);
    let other = worker(&directory, "other");
    let other_project = loaded(&other);
    assert_eq!(other_project.tasks.len(), 1);
    assert_eq!(other_project.tasks[0].form, 5);
    assert_eq!(other_project.tasks[0].lead_partner, 1);
    drop(other);
}

#[test]
fn future_schema_store_is_refused_without_rewriting() {
    let directory = Directory::new();
    let store = worker(&directory, "project");
    loaded(&store);
    drop(store);
    let path = directory.0.join(DATABASE);
    let unsupported_version = {
        let connection = Connection::open(&path).unwrap();
        let version = connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap()
            + 1;
        connection
            .pragma_update(None, "user_version", version)
            .unwrap();
        version
    };
    let before = fs::read(&path).unwrap();
    let refused = worker(&directory, "project");
    assert!(matches!(
        event(&refused),
        Event::Failed { revision: None, message } if message.contains(&format!("schema version {unsupported_version}"))
    ));
    drop(refused);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn memories_are_scoped_cas_guarded_and_survive_only_shared_task_purge() {
    use crate::workspace::memory::{Mutation, Scope};
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let other = store.save(snapshot("project", 1)).unwrap();
    for scope in [Scope::Task, Scope::Project, Scope::Personal] {
        store
            .memory_request(MemoryRequest {
                task_id: task.id,
                mutation: Some(Mutation::Approve(memory_proposal(task.id, scope))),
            })
            .unwrap();
    }
    let records = store
        .memory_request(MemoryRequest {
            task_id: task.id,
            mutation: None,
        })
        .unwrap();
    assert_eq!(records.len(), 3);
    let shared = store
        .memory_request(MemoryRequest {
            task_id: other.id,
            mutation: None,
        })
        .unwrap();
    assert_eq!(shared.len(), 2);
    assert_eq!(
        memories(store.connection.as_ref().unwrap(), "foreign", 777)
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .memory_request(MemoryRequest {
                task_id: other.id,
                mutation: Some(Mutation::Delete {
                    id: records[0].id,
                    expected_revision: 1
                })
            })
            .is_err()
    );
    let mut proposal = records[1].proposal.clone();
    proposal.content = "Updated explicit note".into();
    let changed = store
        .memory_request(MemoryRequest {
            task_id: task.id,
            mutation: Some(Mutation::Edit {
                id: records[1].id,
                expected_revision: 1,
                proposal: proposal.clone(),
            }),
        })
        .unwrap();
    assert_eq!(changed[1].revision, 2);
    assert!(
        store
            .memory_request(MemoryRequest {
                task_id: task.id,
                mutation: Some(Mutation::Edit {
                    id: records[1].id,
                    expected_revision: 1,
                    proposal
                })
            })
            .is_err()
    );
    assert!(
        store
            .memory_request(MemoryRequest {
                task_id: task.id,
                mutation: Some(Mutation::Delete {
                    id: records[1].id,
                    expected_revision: 1
                })
            })
            .is_err()
    );
    let preview = store.preview_delete(&[task.id], None).unwrap();
    assert_eq!(preview[0].memories, vec![records[0].clone()]);
    store
        .manage(Management::Delete {
            ids: vec![task.id],
            expired_before: None,
        })
        .unwrap();
    let retained = store
        .memory_request(MemoryRequest {
            task_id: other.id,
            mutation: None,
        })
        .unwrap();
    assert_eq!(retained.len(), 2);
    assert!(retained.iter().all(|record| record.task_id.is_none()));
    drop(store);
    let mut store = state(&directory, "project", Policy::default());
    assert_eq!(
        store
            .memory_request(MemoryRequest {
                task_id: other.id,
                mutation: None
            })
            .unwrap(),
        retained
    );
}

#[test]
fn explicit_memory_snapshot_is_stable_and_dispatch_rejects_changed_approval() {
    use crate::workspace::memory::{Mutation, Scope};
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let mut task = store.save(snapshot("project", 1)).unwrap();
    let records = store
        .memory_request(MemoryRequest {
            task_id: task.id,
            mutation: Some(Mutation::Approve(memory_proposal(task.id, Scope::Project))),
        })
        .unwrap();
    task.memory_draft =
        serde_json::json!({"selected_ids":[records[0].id],"text":"unfinished note"});
    task.workflow_draft = serde_json::json!({"text":"password=private-draft"});
    task.approved_memories =
        crate::workspace::memory::snapshot(&records, &[records[0].id], "project", task.id).unwrap();
    task.revision += 1;
    let committed = store.save(task).unwrap();
    assert!(committed.redacted);
    assert!(
        !committed
            .workflow_draft
            .to_string()
            .contains("private-draft")
    );
    assert_eq!(committed.approved_memories, records);
    validate_planning_input(&directory.0, &committed, &[]).unwrap();
    let mut proposal = records[0].proposal.clone();
    proposal.content = "Changed after preview".into();
    store
        .memory_request(MemoryRequest {
            task_id: committed.id,
            mutation: Some(Mutation::Edit {
                id: records[0].id,
                expected_revision: 1,
                proposal,
            }),
        })
        .unwrap();
    assert!(validate_planning_input(&directory.0, &committed, &[]).is_err());
    assert_eq!(
        load_task(store.connection.as_ref().unwrap(), "project", committed.id)
            .unwrap()
            .unwrap(),
        committed
    );
    assert_eq!(store.save(committed.clone()).unwrap(), committed);
    drop(store);
    let mut store = state(&directory, "project", Policy::default());
    assert_eq!(store.select(committed.id).unwrap(), committed);
    store
        .memory_request(MemoryRequest {
            task_id: committed.id,
            mutation: Some(Mutation::Delete {
                id: records[0].id,
                expected_revision: 2,
            }),
        })
        .unwrap();
    assert!(validate_planning_input(&directory.0, &committed, &[]).is_err());
}

#[test]
fn memory_worker_events_and_backpressure_retain_owned_requests() {
    let directory = Directory::new();
    let store = worker(&directory, "project");
    loaded(&store);
    let task = create(&store, snapshot("project", 1));
    let mut pending = Some(MemoryRequest {
        task_id: task.id,
        mutation: None,
    });
    assert!(store.try_memory(&mut pending));
    assert!(pending.is_none());
    assert!(
        matches!(event(&store), Event::Memory { task_id, result: Ok(records) }
            if task_id == task.id && records.is_empty())
    );
    drop(store);
    let (commands, incoming) = mpsc::sync_channel(1);
    let store = Worker {
        commands: Some(commands),
        events: None,
        thread: None,
    };
    let mut pending = Some(MemoryRequest {
        task_id: 1,
        mutation: None,
    });
    assert!(store.try_memory(&mut pending));
    pending = Some(MemoryRequest {
        task_id: 2,
        mutation: None,
    });
    assert!(!store.try_memory(&mut pending));
    assert_eq!(pending.as_ref().unwrap().task_id, 2);
    drop(incoming);
    assert!(!store.try_memory(&mut pending));
    assert_eq!(pending.as_ref().unwrap().task_id, 2);
}

#[test]
fn memory_secrets_and_quota_fail_without_partial_approval() {
    use crate::workspace::memory::{Mutation, Scope};
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let mut proposal = memory_proposal(task.id, Scope::Task);
    proposal.content = "password=memory-secret".into();
    assert!(
        store
            .memory_request(MemoryRequest {
                task_id: task.id,
                mutation: Some(Mutation::Approve(proposal))
            })
            .is_err()
    );
    store.policy.quota_bytes = 1;
    assert!(
        store
            .memory_request(MemoryRequest {
                task_id: task.id,
                mutation: Some(Mutation::Approve(memory_proposal(task.id, Scope::Task)))
            })
            .is_err()
    );
    assert!(
        store
            .memory_request(MemoryRequest {
                task_id: task.id,
                mutation: None
            })
            .unwrap()
            .is_empty()
    );
}
#[test]
fn review_reads_only_linked_owned_observations_and_rejects_unsaved_selection() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let mut task = store.save(snapshot("project", 1)).unwrap();
    let prepared = prepared_run(task.id, &store.data_dir, "review");
    let _permit = store.reserve_run(task.id, prepared.clone(), 4096).unwrap();
    assert!(
        review_run_observations(&directory.0, &task, None)
            .unwrap()
            .is_empty()
    );
    let reference = crate::workspace::workflows::EvidenceRef::Run {
        task_id: task.id,
        run_id: prepared.run_id.clone(),
    };
    task.workflows.review = Some(crate::workspace::workflows::Review {
        requested: crate::workspace::workflows::Verdict::Inconclusive,
        criteria: vec![
            crate::workspace::workflows::CriterionEvidence {
                criterion: "Observed result".into(),
                evidence: vec![reference.clone()],
            },
            crate::workspace::workflows::CriterionEvidence {
                criterion: "Explicit duplicate link".into(),
                evidence: vec![reference],
            },
        ],
    });
    assert!(review_run_observations(&directory.0, &task, None).is_err());
    task.revision += 1;
    task = store.save(task).unwrap();
    let records = review_run_observations(&directory.0, &task, None).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].prepared.run_id, prepared.run_id);
    assert_eq!(records[0].prepared.task_id, task.id);
    assert!(
        !serde_json::to_string(&records)
            .unwrap()
            .contains("journal-value")
    );
    let mut foreign = task.clone();
    foreign.workflows.review.as_mut().unwrap().criteria[0].evidence =
        vec![crate::workspace::workflows::EvidenceRef::Run {
            task_id: task.id + 1,
            run_id: prepared.run_id,
        }];
    assert!(review_run_observations(&directory.0, &foreign, None).is_err());
}

#[test]
fn drawer_drafts_reject_excess_bytes_depth_and_secret_field_names() {
    let mut task = snapshot("project", 1);
    task.memory_draft = serde_json::Value::String("x".repeat(MAX_MEMORY_BYTES));
    assert!(validate(&task).is_err());
    task.memory_draft = serde_json::Value::Null;
    for _ in 0..34 {
        task.memory_draft = serde_json::Value::Array(vec![task.memory_draft]);
    }
    assert!(validate(&task).is_err());
    task.memory_draft = serde_json::json!({"password=private-key":"draft"});
    assert!(validate(&task).is_err());
}

#[test]
fn old_palette_navigation_preserves_remaining_navigation() {
    let stored = serde_json::json!({
        "focus":2, "linear":true, "form_selection":8,
        "palette_selection":21, "criterion_selection":4,
        "scroll":[1,2,3,4,5], "overlay":5,
        "editing":true, "editor_field":"NextAction"
    });
    let navigation: Navigation = serde_json::from_value(stored).unwrap();
    assert_eq!(navigation.focus, 2);
    assert_eq!(navigation.form_selection, 8);
    assert_eq!(navigation.criterion_selection, 4);
    assert_eq!(navigation.scroll, [1, 2, 3, 4, 5]);
    assert_eq!(navigation.editor_field, EditorField::NextAction);
    // Stores written before PT03B carry no previous form.
    assert_eq!(navigation.previous_form, None);
    assert_eq!(navigation.dismissed_suggestion, None);
    assert_eq!(Navigation::default().dismissed_suggestion, None);
    assert!(
        serde_json::to_value(navigation)
            .unwrap()
            .get("dismissed_suggestion")
            .is_none()
    );
}

#[test]
fn add_partner_seats_eevee_with_today_and_refuses_the_seventh() {
    let directory = Directory::new();
    let store = worker(&directory, "project");
    loaded(&store);
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    for _ in 0..5 {
        let mut pending = Some(PartyRequest::Add);
        assert!(store.try_party(&mut pending));
        match event(&store) {
            Event::Party(Ok(_)) => {}
            other => panic!("expected added roster: {other:?}"),
        }
    }
    let mut pending = Some(PartyRequest::Add);
    assert!(store.try_party(&mut pending));
    let refusal = event(&store);
    let Event::Party(Err(message)) = refusal else {
        panic!("expected capacity refusal: {refusal:?}");
    };
    assert!(message.contains("at most 6"), "wrong refusal: {message}");
    let roster = return_full_check(&directory);
    assert_eq!(roster.len(), MAX_PARTNERS);
    for (seat, partner) in roster.iter().enumerate().skip(1) {
        assert_eq!(partner.id, seat as i64 + 1);
        assert_eq!(partner.form, 0, "a new partner starts in Eevee form");
        assert!(
            partner.met_at >= before,
            "met date is the real clock, not derived"
        );
        assert!(!partner.met_derived);
        assert_eq!(partner.created_project, "project");
        assert!(partner.nickname.is_none());
    }
}

fn return_full_check(directory: &Directory) -> Vec<Partner> {
    let connection = open_readonly(&directory.0.join(DATABASE)).unwrap();
    partners(&connection).unwrap()
}

#[test]
fn added_partner_ids_are_never_reused_after_a_release() {
    let directory = Directory::new();
    {
        let store = worker(&directory, "project");
        loaded(&store);
        let mut pending = Some(PartyRequest::Add);
        assert!(store.try_party(&mut pending));
        assert!(matches!(event(&store), Event::Party(Ok(_))));
    }
    // Simulate a future release: delete partner #2 with a raw connection while
    // no worker holds the store lock.
    let path = directory.0.join(DATABASE);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute("DELETE FROM partners WHERE id=2", [])
        .unwrap();
    drop(connection);
    let store = worker(&directory, "project");
    loaded(&store);
    let mut pending = Some(PartyRequest::Add);
    assert!(store.try_party(&mut pending));
    match event(&store) {
        Event::Party(Ok(roster)) => {
            assert_eq!(roster.len(), 2);
            assert_eq!(
                roster[1].id, 3,
                "AUTOINCREMENT must not reuse released partner id 2"
            );
        }
        other => panic!("expected added roster: {other:?}"),
    }
}

fn party_mutation(worker: &Worker, request: PartyRequest) -> Result<Vec<Partner>, String> {
    let mut pending = Some(request);
    assert!(worker.try_party(&mut pending));
    assert!(pending.is_none());
    match event(worker) {
        Event::Party(result) => result,
        other => panic!("expected party mutation result: {other:?}"),
    }
}

#[test]
fn nickname_validation_counts_graphemes_and_refuses_unsafe_or_oversized_text() {
    for unit in ["e\u{301}", "👩‍💻", "界"] {
        assert!(validate_nickname(&unit.repeat(12)).is_ok());
        assert!(validate_nickname(&unit.repeat(13)).is_err());
    }
    // Combining marks can exceed the byte bound inside a single grapheme.
    let at_byte_limit = format!("ab{}", "\u{301}".repeat(255));
    assert_eq!(at_byte_limit.len(), 512);
    assert!(validate_nickname(&at_byte_limit).is_ok());
    assert!(validate_nickname(&format!("{at_byte_limit}\u{301}")).is_err());
    for invalid in [
        "",
        "a\nb",
        "a\tb",
        "a\rb",
        "a\0b",
        "a\x1bb",
        "a\u{7f}b",
        "a\u{85}b",
        "a\u{061c}b",
        "a\u{200e}b",
        "a\u{200f}b",
        "a\u{202a}b",
        "a\u{202e}b",
        "a\u{2066}b",
        "a\u{2069}b",
    ] {
        assert!(validate_nickname(invalid).is_err(), "{invalid:?}");
    }
}

#[test]
fn nicknames_commit_refuse_duplicates_clear_and_reopen_without_changing_task_or_evidence() {
    let directory = Directory::new();
    let store = worker(&directory, "nickname-project");
    loaded(&store);
    let task = create(&store, snapshot("nickname-project", 7));
    let record = ProviderRecord {
        id: "nickname-evidence".into(),
        agent: crate::providers::Agent::Codex,
        generation: 1,
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: "item".into(),
        origin: "Codex provider-native".into(),
        coverage: "Public observed result only".into(),
        text: "Original evidence".into(),
        complete: true,
        partner: Some(attribution(1, "Eevee #1", task.form)),
    };
    let mut pending = Some(JournalRequest {
        task_id: task.id,
        operation: JournalOperation::Put(Box::new(record.clone())),
    });
    assert!(store.try_journal(&mut pending));
    assert!(
        matches!(event(&store), Event::Journal { result: Ok(JournalResponse::Saved(back)), .. } if back == record)
    );
    party_mutation(&store, PartyRequest::Add).unwrap();
    let rename = |partner, nickname: Option<&str>| PartyRequest::Nickname {
        partner,
        nickname: nickname.map(str::to_owned),
    };
    let mut expected = return_full_check(&directory);
    expected[0].nickname = Some("Éclair".into());
    assert_eq!(
        party_mutation(&store, rename(1, Some("Éclair"))).unwrap(),
        expected
    );
    expected[1].nickname = Some("Mizu".into());
    assert_eq!(
        party_mutation(&store, rename(2, Some("Mizu"))).unwrap(),
        expected
    );
    // Unicode case comparison applies across partners, not to one's own name.
    assert!(
        party_mutation(&store, rename(2, Some("éCLAIR")))
            .unwrap_err()
            .contains("already uses")
    );
    assert_eq!(return_full_check(&directory), expected);
    for invalid in [Some("thirteenchars"), Some("Bad\nname"), Some("")] {
        assert!(party_mutation(&store, rename(1, invalid)).is_err());
        assert_eq!(return_full_check(&directory), expected);
    }
    for missing in [0, -1, 999] {
        for nickname in [None, Some("Other")] {
            assert!(
                party_mutation(&store, rename(missing, nickname))
                    .unwrap_err()
                    .contains("does not exist")
            );
            assert_eq!(return_full_check(&directory), expected);
        }
    }
    expected[0].nickname = Some("éCLAIR".into());
    assert_eq!(
        party_mutation(&store, rename(1, Some("éCLAIR"))).unwrap(),
        expected
    );
    drop(store);
    let reopened = worker(&directory, "nickname-project");
    let project = loaded(&reopened);
    assert_eq!(project.partners, expected);
    assert_eq!(project.tasks, vec![task.clone()]);
    expected[0].nickname = None;
    assert_eq!(
        party_mutation(&reopened, rename(1, None)).unwrap(),
        expected
    );
    drop(reopened);
    let reopened = worker(&directory, "nickname-project");
    let project = loaded(&reopened);
    assert_eq!(project.partners, expected);
    assert_eq!(project.tasks, vec![task.clone()]);
    let mut pending = Some(JournalRequest {
        task_id: task.id,
        operation: JournalOperation::List,
    });
    assert!(reopened.try_journal(&mut pending));
    assert!(
        matches!(event(&reopened), Event::Journal { result: Ok(JournalResponse::Records(records)), .. } if records == vec![record])
    );
}

#[test]
fn reserved_long_nicknames_remain_readable_and_can_be_cleared() {
    let directory = Directory::new();
    drop(state(&directory, "project", Policy::default()));
    let legacy = "Reserved nickname predating PK01";
    {
        let connection = Connection::open(directory.0.join(DATABASE)).unwrap();
        connection
            .execute("UPDATE partners SET nickname=?1 WHERE id=1", [legacy])
            .unwrap();
    }
    let store = worker(&directory, "project");
    assert_eq!(loaded(&store).partners[0].nickname.as_deref(), Some(legacy));
    assert!(
        party_mutation(
            &store,
            PartyRequest::Nickname {
                partner: 1,
                nickname: None,
            },
        )
        .unwrap()[0]
            .nickname
            .is_none()
    );
}

#[test]
fn nickname_queue_retains_owned_text_on_backpressure_and_disconnect() {
    let (commands, incoming) = mpsc::sync_channel(1);
    let mut worker = Worker {
        commands: Some(commands),
        events: None,
        thread: None,
    };
    let request = PartyRequest::Nickname {
        partner: 1,
        nickname: Some("Eevee".into()),
    };
    let mut pending = Some(request.clone());
    assert!(worker.try_party(&mut pending));
    assert!(pending.is_none());
    assert!(!worker.try_party(&mut pending));
    pending = Some(request.clone());
    assert!(!worker.try_party(&mut pending));
    assert_eq!(pending, Some(request.clone()));
    drop(incoming);
    assert!(!worker.try_party(&mut pending));
    assert_eq!(pending, Some(request.clone()));
    worker.commands.take();
    assert!(!worker.try_party(&mut pending));
    assert_eq!(pending, Some(request));
}

#[test]
fn nickname_store_enforces_unicode_write_boundaries_without_losing_the_original() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let accepted = "👩‍💻e\u{301}".repeat(6);
    let roster = store.nickname_partner(1, Some(accepted.clone())).unwrap();
    assert_eq!(roster[0].nickname.as_deref(), Some(accepted.as_str()));
    for refused in [
        format!("{accepted}界"),
        format!("a{}", "\u{301}".repeat(256)),
        "Unsafe\u{202e}".into(),
    ] {
        assert!(store.nickname_partner(1, Some(refused)).is_err());
        assert_eq!(return_full_check(&directory), roster);
    }
}

#[test]
fn schema_eight_migrates_request_drafts_without_changing_task_facts() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialize(&mut connection).unwrap();
    downgrade_moves(&connection);
    connection
        .execute_batch(
            "DROP TABLE active_work; DROP TABLE provider_preferences; PRAGMA user_version=8;",
        )
        .unwrap();
    initialize(&mut connection).unwrap();
    let mut task = Snapshot::empty("/workspace".into());
    task.goal = "Stable task identity".into();
    task.next_action = "Existing task fact".into();
    task.partner_drafts = partner_drafts(vec![Draft {
        field: EditorField::Request,
        text: "Follow-up question".into(),
        cursor: 18,
    }]);
    validate(&task).unwrap();
    let restored: Snapshot = serde_json::from_str(&serde_json::to_string(&task).unwrap()).unwrap();
    assert_eq!(restored.request_text(), "Follow-up question");
    assert_eq!(restored.goal, "Stable task identity");
    assert_eq!(restored.next_action, "Existing task fact");
}

#[test]
fn provider_preferences_survive_restart_and_preserve_exact_route_ids_and_task_data() {
    use crate::providers::{Agent, ModelSelection};
    for density in [TextDensity::Compact, TextDensity::Comfortable] {
        let directory = Directory::new();
        let worker = worker(&directory, "project");
        loaded(&worker);
        let task = create(&worker, snapshot("project", 1));
        let preferences = ProviderPreferences {
            provider: Agent::ChatGPT,
            selections: [
                (
                    Agent::Codex,
                    ModelSelection {
                        model: Some("cli/EXACT-2026".into()),
                        effort: Some("high".into()),
                    },
                ),
                (
                    Agent::ChatGPT,
                    ModelSelection {
                        model: Some("native/EXACT-2026".into()),
                        effort: Some("medium".into()),
                    },
                ),
            ]
            .into_iter()
            .collect(),
            reduced_motion: true,
            text_density: density,
        };
        let mut pending = Some(PreferenceRequest::Save(preferences.clone()));
        assert!(worker.try_preferences(&mut pending));
        assert!(
            matches!(event(&worker), Event::Preferences(Ok(Some(saved))) if saved == preferences)
        );
        drop(worker);
        let restarted =
            super::Worker::start(directory.0.clone(), "project".into(), Policy::default()).unwrap();
        assert_eq!(loaded(&restarted).tasks, vec![task]);
        let mut pending = Some(PreferenceRequest::Load);
        assert!(restarted.try_preferences(&mut pending));
        assert!(
            matches!(event(&restarted), Event::Preferences(Ok(Some(saved))) if saved == preferences)
        );
        let connection = open_readonly(&directory.0.join(DATABASE)).unwrap();
        let record: String = connection
            .query_row("SELECT record FROM provider_preferences", [], |row| {
                row.get(0)
            })
            .unwrap();
        let json: serde_json::Value = serde_json::from_str(&record).unwrap();
        assert_eq!(
            json.as_object().unwrap().len(),
            if density.is_compact() { 3 } else { 4 }
        );
        assert_eq!(
            json.get("text_density").and_then(serde_json::Value::as_str),
            if density.is_compact() {
                None
            } else {
                Some("comfortable")
            },
        );
        assert_eq!(json["reduced_motion"], true);
        for selection in json["selections"].as_object().unwrap().values() {
            assert_eq!(selection.as_object().unwrap().len(), 2);
            assert!(selection.get("model").is_some() && selection.get("effort").is_some());
        }
    }
}

#[test]
fn failed_preference_save_preserves_committed_preference_and_task_draft() {
    use crate::providers::{Agent, ModelSelection};
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let preferences = ProviderPreferences {
        provider: Agent::Codex,
        selections: [(
            Agent::Codex,
            ModelSelection {
                model: Some("exact".into()),
                effort: Some("high".into()),
            },
        )]
        .into_iter()
        .collect(),
        reduced_motion: false,
        text_density: TextDensity::Compact,
    };
    store
        .preference_request(PreferenceRequest::Save(preferences.clone()))
        .unwrap();
    let mut invalid = preferences.clone();
    invalid.selections.get_mut(&Agent::Codex).unwrap().model = Some("token=private-secret".into());
    assert!(
        store
            .preference_request(PreferenceRequest::Save(invalid))
            .is_err()
    );
    assert_eq!(
        store.preference_request(PreferenceRequest::Load).unwrap(),
        Some(preferences)
    );
    assert_eq!(
        load_task(store.connection.as_ref().unwrap(), "project", task.id).unwrap(),
        Some(task)
    );
}

#[test]
fn schema_nine_preference_migration_is_atomic_and_preserves_existing_data() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    drop(store);
    let mut connection = Connection::open(directory.0.join(DATABASE)).unwrap();
    downgrade_moves(&connection);
    connection
        .execute_batch(
            "DROP TABLE active_work; DROP TABLE provider_preferences; PRAGMA user_version=9;",
        )
        .unwrap();
    {
        let transaction = connection.transaction().unwrap();
        migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
        assert_eq!(
            load_task(&transaction, "project", task.id).unwrap(),
            Some(task.clone())
        );
        assert!(
            super::preferences::load_preferences(&transaction)
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        9
    );
    assert!(
        connection
            .prepare("SELECT * FROM provider_preferences")
            .is_err()
    );
    initialize(&mut connection).unwrap();
    assert_eq!(
        load_task(&connection, "project", task.id).unwrap(),
        Some(task)
    );
    assert!(
        super::preferences::load_preferences(&connection)
            .unwrap()
            .is_none()
    );
}

fn stored_table_rows(connection: &Connection, table: &str) -> Vec<Vec<rusqlite::types::Value>> {
    let mut statement = connection
        .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
        .unwrap();
    let columns = statement.column_count();
    statement
        .query_map([], |row| {
            (0..columns)
                .map(|index| row.get(index))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

#[test]
fn schema_ten_dismissal_migration_preserves_rows_and_rolls_back_atomically() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let mut original = snapshot("project", 1);
    original.auto_form = false;
    let task = store.save(original).unwrap();
    store.select(task.id).unwrap();
    let sources = Directory::new();
    store
        .context(ContextRequest {
            producer: None,
            task_id: task.id,
            operation: ContextOperation::Pin(captured_excerpt(
                task.id,
                &sources.0.join("source.txt"),
                "unchanged source evidence",
            )),
        })
        .unwrap();
    let prepared = prepared_run(task.id, &store.data_dir, "schema-ten");
    drop(store.reserve_run(task.id, prepared.clone(), 1024).unwrap());
    let mut record = crate::workspace::runner::RunRecord::awaiting(&prepared);
    record.state = crate::workspace::runner::RunState::Failed;
    record.tree = crate::workspace::runner::TreeOutcome::NotStarted;
    record.ended_at_ms = Some(timestamp().unwrap() as u64);
    store.update_run(task.id, record).unwrap();
    store
        .journal_request(JournalRequest {
            task_id: task.id,
            operation: JournalOperation::Put(Box::new(ProviderRecord {
                id: "schema-ten-provider-item".into(),
                agent: crate::providers::Agent::Codex,
                generation: 1,
                thread_id: "thread".into(),
                turn_id: "turn".into(),
                item_id: "item".into(),
                origin: "Codex provider-native".into(),
                coverage: "Public observed result only; not a local check".into(),
                text: "Unchanged provider evidence".into(),
                complete: true,
                partner: None,
            })),
        })
        .unwrap();
    let preferences = ProviderPreferences {
        provider: crate::providers::Agent::Codex,
        selections: [(
            crate::providers::Agent::Codex,
            crate::providers::ModelSelection {
                model: Some("exact/schema-ten".into()),
                effort: Some("high".into()),
            },
        )]
        .into_iter()
        .collect(),
        reduced_motion: false,
        text_density: TextDensity::Compact,
    };
    store
        .preference_request(PreferenceRequest::Save(preferences.clone()))
        .unwrap();
    drop(store);

    let mut connection = Connection::open(directory.0.join(DATABASE)).unwrap();
    // Whitespace and the obsolete palette field must survive without JSON reserialization.
    let navigation = r#"{ "focus":1, "linear":false, "form_selection":0,
        "palette_selection":21, "criterion_selection":0, "scroll":[0,0,0,0,0],
        "overlay":null, "editing":false, "editor_field":"Goal" }"#;
    connection
        .execute("UPDATE tasks SET navigation=?1", [navigation])
        .unwrap();
    downgrade_moves(&connection);
    connection
        .execute_batch("DROP TABLE active_work; PRAGMA user_version=10;")
        .unwrap();
    let tables = [
        "tasks",
        "excerpts",
        "executions",
        "provider_journal",
        "provider_preferences",
        "selections",
        "partners",
        "memories",
        "sqlite_sequence",
    ];
    // SQLite values retain each original TEXT/BLOB byte, not a parsed JSON representation.
    let before: Vec<_> = tables
        .iter()
        .map(|table| stored_table_rows(&connection, table))
        .collect();
    assert!(matches!(inspect(&connection).unwrap(), Schema::Current));
    {
        let transaction = connection.transaction().unwrap();
        migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
        assert!(matches!(inspect(&transaction).unwrap(), Schema::Current));
        for (table, rows) in tables.iter().zip(&before) {
            let actual = stored_table_rows(&transaction, table);
            assert_eq!(actual.len(), rows.len(), "{table}");
            for (actual, expected) in actual.iter().zip(rows) {
                assert_eq!(
                    actual.get(..expected.len()),
                    Some(expected.as_slice()),
                    "{table}"
                );
            }
        }
        // Aborting this actual migration transaction also rolls back the version fence.
    }
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        10
    );
    for (table, rows) in tables.iter().zip(&before) {
        assert_eq!(stored_table_rows(&connection, table), *rows, "{table}");
    }
    initialize(&mut connection).unwrap();
    for (table, rows) in tables.iter().zip(&before) {
        let actual = stored_table_rows(&connection, table);
        assert_eq!(actual.len(), rows.len(), "{table}");
        for (actual, expected) in actual.iter().zip(rows) {
            assert_eq!(
                actual.get(..expected.len()),
                Some(expected.as_slice()),
                "{table}"
            );
        }
    }
    let restored = load_task(&connection, "project", task.id).unwrap().unwrap();
    assert_eq!(restored, task);
    assert_eq!(restored.navigation.dismissed_suggestion, None);
    assert_eq!(
        super::preferences::load_preferences(&connection).unwrap(),
        Some(preferences)
    );
}

#[test]
fn task_local_dismissal_persists_and_clears_through_ordinary_save() {
    let directory = Directory::new();
    let worker = worker(&directory, "project");
    loaded(&worker);
    let mut task = create(&worker, snapshot("project", 1));
    let other = create(&worker, snapshot("project", 1));
    let trigger = format!("task:{}/latest-run-failed:{}", task.id, "a".repeat(64));
    task.navigation.dismissed_suggestion = Some(trigger.clone());
    task.revision += 1;
    let task = create(&worker, task);
    drop(worker);
    let reopened =
        super::Worker::start(directory.0.clone(), "project".into(), Policy::default()).unwrap();
    let project = loaded(&reopened);
    assert_eq!(project.tasks, vec![task.clone(), other]);
    assert_eq!(
        task.navigation.dismissed_suggestion.as_deref(),
        Some(trigger.as_str())
    );
    let mut cleared = task;
    cleared.revision += 1;
    cleared.navigation.dismissed_suggestion = None;
    let cleared = create(&reopened, cleared);
    drop(reopened);
    let restarted =
        super::Worker::start(directory.0.clone(), "project".into(), Policy::default()).unwrap();
    assert_eq!(loaded(&restarted).tasks[0], cleared);
    let connection = open_readonly(&directory.0.join(DATABASE)).unwrap();
    let navigation: String = connection
        .query_row(
            "SELECT navigation FROM tasks WHERE id=?1",
            [cleared.id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!navigation.contains("dismissed_suggestion"));
}

#[test]
fn malformed_foreign_and_oversized_dismissals_refuse_save_and_open_without_mutation() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let mut task = store.save(snapshot("project", 1)).unwrap();
    let prefix = format!("task:{}/", task.id);
    let hash = "a".repeat(64);
    let longest_rule = "r".repeat(160 - prefix.len() - 1 - hash.len());
    let longest = format!("{prefix}{longest_rule}:{hash}");
    assert_eq!(longest.len(), 160);
    task.revision += 1;
    task.navigation.dismissed_suggestion = Some(longest.clone());
    task = store.save(task).unwrap();
    let invalid = [
        String::new(),
        format!("task:{}/latest-run-failed:{hash}", task.id + 1),
        format!("task:0{}/latest-run-failed:{hash}", task.id),
        format!("task:+{}/latest-run-failed:{hash}", task.id),
        format!("task:0/latest-run-failed:{hash}"),
        format!("{prefix}:{hash}"),
        format!("{prefix}Latest-run-failed:{hash}"),
        format!("{prefix}latest_run_failed:{hash}"),
        format!("{prefix}-latest-run-failed:{hash}"),
        format!("{prefix}latest--run-failed:{hash}"),
        format!("{prefix}latest-run-failed-:{hash}"),
        format!("{prefix}latest/run-failed:{hash}"),
        format!("{prefix}règle:{hash}"),
        format!("{prefix}rule:\n{hash}"),
        format!("{prefix}rule:{}", "A".repeat(64)),
        format!("{prefix}rule:{}", "g".repeat(64)),
        format!("{prefix}rule:{}", "a".repeat(63)),
        format!("{prefix}rule:{}", "a".repeat(65)),
        format!("{prefix}{longest_rule}r:{hash}"),
    ];
    let before = stored_table_rows(store.connection.as_ref().unwrap(), "tasks");
    for trigger in &invalid {
        let mut changed = task.clone();
        changed.revision += 1;
        changed.navigation.dismissed_suggestion = Some(trigger.clone());
        assert!(store.save(changed).is_err(), "{trigger:?}");
        assert_eq!(
            stored_table_rows(store.connection.as_ref().unwrap(), "tasks"),
            before
        );
    }
    let mut unsaved = Snapshot::empty("project".into());
    unsaved.navigation.dismissed_suggestion = Some(format!("task:0/rule:{hash}"));
    assert!(store.save(unsaved).is_err());
    drop(store);

    let path = directory.0.join(DATABASE);
    for trigger in invalid {
        let connection = Connection::open(&path).unwrap();
        let mut navigation = task.navigation.clone();
        navigation.dismissed_suggestion = Some(trigger.clone());
        connection
            .execute(
                "UPDATE tasks SET navigation=?1 WHERE id=?2",
                params![serde_json::to_string(&navigation).unwrap(), task.id],
            )
            .unwrap();
        drop(connection);
        let before = fs::read(&path).unwrap();
        let refused = worker(&directory, "project");
        assert!(
            matches!(event(&refused), Event::Failed { revision: None, message } if message.contains("suggestion dismissal")),
            "{trigger:?}"
        );
        drop(refused);
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

fn attribution(partner: i64, name: &str, form: usize) -> Attribution {
    Attribution {
        partner,
        name: name.into(),
        form,
    }
}

#[test]
fn runs_name_their_partner_and_older_records_stay_partner_unknown() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let mut initial = snapshot("project", 1);
    initial.form = 2;
    let task = store.save(initial).unwrap();
    store.add_partner().unwrap();
    store.nickname_partner(1, Some("Bolt".into())).unwrap();
    let mut routed = task.clone();
    routed.revision += 1;
    routed.lead_partner = 2;
    routed.form = 1;
    store.save(routed).unwrap();
    store.select(task.id).unwrap();
    let mut finish = |suffix: &str, partner: Option<Attribution>| {
        let mut prepared = prepared_run(task.id, &store.data_dir, suffix);
        prepared.partner = partner;
        let permit = store.reserve_run(task.id, prepared.clone(), 1024).unwrap();
        drop(permit);
        let mut record = crate::workspace::runner::RunRecord::awaiting(&prepared);
        record.state = crate::workspace::runner::RunState::Failed;
        record.tree = crate::workspace::runner::TreeOutcome::NotStarted;
        record.ended_at_ms = Some(timestamp().unwrap() as u64);
        store.update_run(task.id, record).unwrap();
    };
    finish("old", None);
    finish("bolt", Some(attribution(1, "Bolt", 2)));
    finish("mizu", Some(attribution(2, "Eevee #2", 1)));
    let connection = store.connection.as_ref().unwrap();
    let raw: String = connection
        .query_row(
            "SELECT record FROM executions WHERE run_id='run-old'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !raw.contains("partner"),
        "older records keep their exact shape"
    );
    let labels: Vec<_> = store
        .load_runs(task.id)
        .unwrap()
        .iter()
        .map(|run| Attribution::label(run.prepared.partner.as_ref()))
        .collect();
    assert_eq!(
        labels,
        ["Eevee #2 (Vaporeon)", "Bolt (Jolteon)", "partner unknown"]
    );
    let mut invalid = prepared_run(task.id, &store.data_dir, "invalid");
    invalid.partner = Some(attribution(0, "", 99));
    assert!(store.reserve_run(task.id, invalid, 1024).is_err());
}

#[test]
fn provider_records_carry_a_partner_snapshot_and_fixture_records_read_unknown() {
    let directory = Directory::new();
    let worker = worker(&directory, "journal-partner");
    loaded(&worker);
    let task = create(&worker, Snapshot::empty("journal-partner".into()));
    let record = |id: &str, partner: Option<Attribution>| ProviderRecord {
        id: id.into(),
        agent: crate::providers::Agent::Codex,
        generation: 1,
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: id.into(),
        origin: "Codex provider-native".into(),
        coverage: "Public observed result only".into(),
        text: "output".into(),
        complete: true,
        partner,
    };
    let old = record("fixture-old", None);
    {
        let connection = Connection::open(directory.0.join(DATABASE)).unwrap();
        connection
            .execute(
                "INSERT INTO provider_journal(id,task_id,record) VALUES(?1,?2,?3)",
                params![old.id, task.id, serde_json::to_string(&old).unwrap()],
            )
            .unwrap();
    }
    for saved in [
        record("bolt", Some(attribution(1, "Bolt", 2))),
        record("mizu", Some(attribution(2, "Mizu", 1))),
    ] {
        let mut pending = Some(JournalRequest {
            task_id: task.id,
            operation: JournalOperation::Put(Box::new(saved.clone())),
        });
        assert!(worker.try_journal(&mut pending));
        assert!(
            matches!(event(&worker), Event::Journal { result: Ok(JournalResponse::Saved(back)), .. } if back == saved)
        );
    }
    let mut pending = Some(JournalRequest {
        task_id: task.id,
        operation: JournalOperation::List,
    });
    assert!(worker.try_journal(&mut pending));
    let Event::Journal {
        result: Ok(JournalResponse::Records(records)),
        ..
    } = event(&worker)
    else {
        panic!("journal list failed");
    };
    let labels: Vec<_> = records
        .iter()
        .map(|record| Attribution::label(record.partner.as_ref()))
        .collect();
    assert_eq!(
        labels,
        ["partner unknown", "Bolt (Jolteon)", "Mizu (Vaporeon)"]
    );
    let mut pending = Some(JournalRequest {
        task_id: task.id,
        operation: JournalOperation::Put(Box::new(record(
            "control",
            Some(attribution(1, "Bo\u{7}lt", 2)),
        ))),
    });
    assert!(worker.try_journal(&mut pending));
    assert!(matches!(
        event(&worker),
        Event::Journal { result: Err(_), .. }
    ));
}

#[test]
fn location_pin_requires_a_task_owned_run_and_retains_its_origin_after_reopen() {
    let directory = Directory::new();
    let mut storage = state(&directory, "project", Policy::default());
    let first = storage.save(snapshot("project", 1)).unwrap();
    let second = storage.save(snapshot("project", 1)).unwrap();
    let prepared = prepared_run(first.id, &storage.data_dir, "pin-origin");
    let permit = storage
        .reserve_run(first.id, prepared.clone(), 1024)
        .unwrap();
    drop(permit);
    storage
        .run_request(RunRequest {
            task_id: first.id,
            operation: RunOperation::Abandon {
                request_id: prepared.request_id.clone(),
                run_id: prepared.run_id.clone(),
            },
        })
        .unwrap();
    let sources = Directory::new();
    let source = sources.0.join("source.rs");
    let mut excerpt = captured_excerpt(first.id, &source, "source\n");
    excerpt.origin_run = Some(prepared.run_id.clone());
    let captured = storage
        .context(ContextRequest {
            producer: None,
            task_id: first.id,
            operation: ContextOperation::Pin(excerpt.clone()),
        })
        .unwrap();
    excerpt.task_id = second.id;
    assert!(
        storage
            .context(ContextRequest {
                task_id: second.id,
                producer: None,
                operation: ContextOperation::Pin(excerpt),
            })
            .is_err()
    );
    drop(storage);
    let mut reopened = state(&directory, "project", Policy::default());
    assert_eq!(
        reopened
            .context(ContextRequest {
                task_id: first.id,
                producer: None,
                operation: ContextOperation::Load,
            })
            .unwrap(),
        captured
    );
    assert_eq!(
        captured[0].origin_run.as_deref(),
        Some(prepared.run_id.as_str())
    );
}

fn work_claim(
    store: &State,
    task: &Snapshot,
    partner: i64,
    request: &str,
    writing: bool,
) -> WorkClaim {
    let roster = partners(store.connection.as_ref().unwrap()).unwrap();
    WorkClaim {
        task_id: task.id,
        owner: Attribution::of(roster.iter().find(|member| member.id == partner).unwrap()),
        request_id: request.into(),
        writing,
    }
}

fn claim_work(
    store: &mut State,
    task: &Snapshot,
    claim: &WorkClaim,
) -> Result<Vec<WorkClaim>, String> {
    store.work_request(&WorkRequest::Acquire {
        claim: claim.clone(),
        task: Box::new(task.clone()),
    })
}

#[test]
fn work_ownership_bounds_exact_idempotence_and_stale_release() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    for _ in 0..3 {
        store.add_partner().unwrap();
    }
    let first = work_claim(&store, &task, 1, "writer-one", true);
    let second = work_claim(&store, &task, 2, "writer-two", true);
    assert_eq!(
        claim_work(&mut store, &task, &first).unwrap(),
        vec![first.clone()]
    );
    assert_eq!(
        claim_work(&mut store, &task, &first).unwrap(),
        vec![first.clone()]
    );
    assert!(claim_work(&mut store, &task, &second).is_err());
    let second = WorkClaim {
        writing: false,
        ..second
    };
    let third = work_claim(&store, &task, 3, "reader-three", false);
    let fourth = work_claim(&store, &task, 4, "reader-four", false);
    claim_work(&mut store, &task, &second).unwrap();
    assert_eq!(claim_work(&mut store, &task, &third).unwrap().len(), 3);
    assert!(claim_work(&mut store, &task, &fourth).is_err());
    let mut forged = first.clone();
    forged.owner.name = "not its owner".into();
    assert!(store.work_request(&WorkRequest::Release(forged)).is_err());
    store
        .work_request(&WorkRequest::Release(first.clone()))
        .unwrap();
    store
        .work_request(&WorkRequest::Release(first.clone()))
        .unwrap();
    let successor = WorkClaim {
        request_id: "writer-successor".into(),
        ..first.clone()
    };
    claim_work(&mut store, &task, &successor).unwrap();
    let active = store.work_request(&WorkRequest::Release(first)).unwrap();
    assert!(active.contains(&successor));
    assert_eq!(active.len(), 3);
}

#[test]
fn work_acquire_requires_exact_committed_task_and_live_owner() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let first = work_claim(&store, &task, 1, "exact-preview", false);
    let mut unsaved = task.clone();
    unsaved.command = "not committed".into();
    assert!(claim_work(&mut store, &unsaved, &first).is_err());
    let mut stale_owner = first.clone();
    stale_owner.owner.name = "old nickname".into();
    assert!(claim_work(&mut store, &task, &stale_owner).is_err());
    claim_work(&mut store, &task, &first).unwrap();
    let other = store.save(snapshot("project", 1)).unwrap();
    let other_claim = work_claim(&store, &other, 1, "other-task", false);
    assert!(claim_work(&mut store, &other, &other_claim).is_err());
    assert!(validate_work_input(&directory.0, &other, &[], &first).is_err());
}

#[test]
fn claimed_admission_allows_ui_and_audit_saves_but_not_shared_fact_changes() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    store.add_partner().unwrap();
    let claim = work_claim(&store, &task, 1, "immutable-input", true);
    claim_work(&mut store, &task, &claim).unwrap();
    let mut live = task.clone();
    live.revision += 1;
    live.lead_partner = 2;
    live.form = 0;
    live.command = "future command".into();
    live.navigation.focus = 2;
    live.partner_drafts.insert(
        2,
        PartnerDrafts {
            drafts: vec![Draft {
                field: EditorField::Request,
                text: "follow-up".into(),
                cursor: 9,
            }],
            ..PartnerDrafts::default()
        },
    );
    live.workflows.review_generation += 1;
    live.workflows.review_stale = true;
    live.workflows
        .patch_authors
        .push(crate::workspace::workflows::PatchAuthor {
            partner: 1,
            path: "src/lib.rs".into(),
            source_sha256: [1; 32],
            record_id: "patch-record".into(),
        });
    let live = store.save(live).unwrap();
    validate_work_input(&directory.0, &task, &[], &claim).unwrap();
    assert!(validate_planning_input(&directory.0, &task, &[]).is_err());
    let mut changed = live.clone();
    changed.revision += 1;
    changed.goal = "changed canonical goal".into();
    store.save(changed).unwrap();
    assert!(validate_work_input(&directory.0, &task, &[], &claim).is_err());
    store
        .work_request(&WorkRequest::Release(claim.clone()))
        .unwrap();
    assert!(validate_work_input(&directory.0, &task, &[], &claim).is_err());
}

#[test]
fn claims_guard_selected_capture_identity_and_destructive_boundaries() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let sources = Directory::new();
    let source = sources.0.join("source.rs");
    fs::write(&source, "visible\n").unwrap();
    let basket = store
        .context(ContextRequest {
            producer: None,
            task_id: task.id,
            operation: ContextOperation::Pin(captured_excerpt(task.id, &source, "visible\n")),
        })
        .unwrap();
    let claim = work_claim(&store, &task, 1, "capture-owner", false);
    claim_work(&mut store, &task, &claim).unwrap();
    validate_work_input(&directory.0, &task, &basket, &claim).unwrap();
    let mut changed = basket.clone();
    changed[0].text = "substituted".into();
    assert!(validate_work_input(&directory.0, &task, &changed, &claim).is_err());
    assert!(
        store
            .manage(Management::Archive {
                id: task.id,
                archived: true
            })
            .is_err()
    );
    assert!(
        store
            .manage(Management::Delete {
                ids: vec![task.id],
                expired_before: None
            })
            .is_err()
    );
    assert!(
        store
            .context(ContextRequest {
                task_id: task.id,
                producer: None,
                operation: ContextOperation::Remove(basket[0].id)
            })
            .is_err()
    );
    store
        .work_request(&WorkRequest::Release(claim.clone()))
        .unwrap();
    store
        .context(ContextRequest {
            producer: None,
            task_id: task.id,
            operation: ContextOperation::Remove(basket[0].id),
        })
        .unwrap();
    assert!(validate_work_input(&directory.0, &task, &basket, &claim).is_err());
}

#[test]
fn startup_recovers_claims_only_after_exclusive_writer_lock() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let task = store.save(snapshot("project", 1)).unwrap();
    let claim = work_claim(&store, &task, 1, "crashed-writer", true);
    claim_work(&mut store, &task, &claim).unwrap();
    let mut refused = State {
        data_dir: directory.0.clone(),
        project: "project".into(),
        policy: Policy::default(),
        connection: None,
        lock: None,
    };
    assert!(refused.open_and_load().is_err());
    validate_work_input(&directory.0, &task, &[], &claim).unwrap();
    drop(refused);
    drop(store);
    let mut reopened = state(&directory, "project", Policy::default());
    assert!(
        work::claims(reopened.connection.as_ref().unwrap())
            .unwrap()
            .is_empty()
    );
    assert!(validate_work_input(&directory.0, &task, &[], &claim).is_err());
    let replacement = work_claim(&reopened, &task, 1, "new-writer", true);
    claim_work(&mut reopened, &task, &replacement).unwrap();
}

#[test]
fn schema_twelve_work_migration_preserves_old_rows_and_transactional_fence() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    store.save(snapshot("project", 1)).unwrap();
    drop(store);
    let mut connection = Connection::open(directory.0.join(DATABASE)).unwrap();
    downgrade_moves(&connection);
    connection
        .execute_batch("DROP TABLE active_work; PRAGMA user_version=12;")
        .unwrap();
    let tables = [
        "tasks",
        "executions",
        "excerpts",
        "provider_journal",
        "partners",
        "memories",
        "selections",
        "provider_preferences",
        "sqlite_sequence",
    ];
    let before: Vec<_> = tables
        .iter()
        .map(|table| stored_table_rows(&connection, table))
        .collect();
    {
        let transaction = connection.transaction().unwrap();
        migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
        for (table, rows) in tables.iter().zip(&before) {
            let actual = stored_table_rows(&transaction, table);
            assert_eq!(actual.len(), rows.len(), "{table}");
            for (actual, expected) in actual.iter().zip(rows) {
                assert_eq!(
                    actual.get(..expected.len()),
                    Some(expected.as_slice()),
                    "{table}"
                );
            }
        }
    }
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        12
    );
    initialize(&mut connection).unwrap();
    inspect(&connection).unwrap();
    for (table, rows) in tables.iter().zip(&before) {
        let actual = stored_table_rows(&connection, table);
        assert_eq!(actual.len(), rows.len(), "{table}");
        for (actual, expected) in actual.iter().zip(rows) {
            assert_eq!(
                actual.get(..expected.len()),
                Some(expected.as_slice()),
                "{table}"
            );
        }
    }
}

#[test]
fn schema15_auto_form_migration_is_conservative_atomic_and_preserves_all_other_rows() {
    fn retained_rows(connection: &Connection) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
        let mut tables = connection
            .prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name!='partners' ORDER BY name")
            .unwrap();
        let names = tables
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        names
            .into_iter()
            .map(|name| {
                let mut statement = connection
                    .prepare(&format!("SELECT * FROM \"{name}\" ORDER BY rowid"))
                    .unwrap();
                let count = statement.column_count();
                let rows = statement
                    .query_map([], |row| {
                        (0..count)
                            .map(|index| row.get(index))
                            .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                (name, rows)
            })
            .collect()
    }

    fn assert_retained(
        connection: &Connection,
        before: &[(String, Vec<Vec<rusqlite::types::Value>>)],
    ) {
        let after = retained_rows(connection);
        for retained in before {
            assert_eq!(
                after.iter().find(|(name, _)| name == &retained.0),
                Some(retained),
                "{}",
                retained.0,
            );
        }
    }

    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let second = store.add_partner().unwrap().last().unwrap().id;
    let mut eevee = Snapshot::empty("project".into());
    eevee.goal = "Preserve shared task facts".into();
    eevee.partner_drafts = partner_drafts(vec![Draft {
        field: EditorField::Request,
        text: "Retain this exact outgoing draft".into(),
        cursor: 7,
    }]);
    let eevee = store.save(eevee).unwrap();
    let mut specialist = snapshot("project", 1);
    specialist.lead_partner = second;
    let specialist = store.save(specialist).unwrap();
    let mut source = captured_excerpt(
        eevee.id,
        &directory.0.join("source.rs"),
        "Retained source evidence",
    );
    source.id = 17;
    let record = ProviderRecord {
        id: "retained-provider-record".into(),
        agent: crate::providers::Agent::Codex,
        generation: 1,
        thread_id: "retained-thread".into(),
        turn_id: "retained-turn".into(),
        item_id: "retained-item".into(),
        origin: "Observed fixture output".into(),
        coverage: "Retained observation only".into(),
        text: "Retain journal bytes".into(),
        complete: true,
        partner: None,
    };
    store
        .journal_request(JournalRequest {
            task_id: eevee.id,
            operation: JournalOperation::Put(Box::new(record)),
        })
        .unwrap();
    let mut connection = store.connection.take().unwrap();
    connection
        .execute(
            "INSERT INTO excerpts(id,task_id,record) VALUES (?1,?2,?3)",
            params![
                source.id,
                source.task_id,
                serde_json::to_string(&source).unwrap()
            ],
        )
        .unwrap();
    connection
        .execute_batch(
            "UPDATE sqlite_sequence SET seq=700 WHERE name='partners';
             ALTER TABLE partners DROP COLUMN auto_form;
             DROP TABLE held_items;
             DROP TABLE ribbons;
             PRAGMA user_version=15;",
        )
        .unwrap();
    let before = retained_rows(&connection);
    let partner_rows = |connection: &Connection| {
        let mut statement = connection
            .prepare("SELECT id,nickname,form,met_at,met_derived,created_project FROM partners ORDER BY id")
            .unwrap();
        statement
            .query_map([], |row| {
                (0..6)
                    .map(|index| row.get(index))
                    .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let before_partners = partner_rows(&connection);
    {
        let transaction = connection.transaction().unwrap();
        migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
        assert_retained(&transaction, &before);
        assert_eq!(partner_rows(&transaction), before_partners);
        let roster = partners(&transaction).unwrap();
        assert!(
            roster
                .iter()
                .find(|partner| partner.id == 1)
                .unwrap()
                .auto_form
        );
        assert!(
            !roster
                .iter()
                .find(|partner| partner.id == second)
                .unwrap()
                .auto_form
        );
        inspect(&transaction).unwrap();
        // Dropping the transaction restores the entire old layout and contents.
    }
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        15
    );
    assert!(
        connection
            .prepare("SELECT auto_form FROM partners")
            .is_err()
    );
    assert_retained(&connection, &before);
    initialize(&mut connection).unwrap();
    assert_retained(&connection, &before);
    assert_eq!(partner_rows(&connection), before_partners);
    assert_eq!(
        load_task(&connection, "project", eevee.id).unwrap(),
        Some(eevee)
    );
    assert_eq!(
        load_task(&connection, "project", specialist.id).unwrap(),
        Some(specialist)
    );
    store.connection = Some(connection);
    let added = store.add_partner().unwrap();
    let new_partner = added.last().unwrap();
    assert!(new_partner.id > 700 && new_partner.auto_form && new_partner.form == 0);
}

#[test]
fn automatic_and_manual_modes_are_global_durable_and_independent_of_current_form() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let second = store.add_partner().unwrap().last().unwrap().id;
    let mut manual = snapshot("project", 1);
    manual.partner_drafts = partner_drafts(vec![Draft {
        field: EditorField::Request,
        text: "Keep partner-owned draft".into(),
        cursor: 4,
    }]);
    let manual = store.save(manual).unwrap();
    let mut automatic = Snapshot::empty("project".into());
    automatic.lead_partner = second;
    automatic.form = 6;
    automatic.goal = "Preserve this second task".into();
    let automatic = store.save(automatic).unwrap();
    let facts = manual.clone();
    let mut changed_lead = manual.clone();
    changed_lead.lead_partner = second;
    changed_lead.form = 6;
    changed_lead.auto_form = true;
    changed_lead.revision += 1;
    let mut changed_lead = store.save(changed_lead).unwrap();
    assert_eq!(changed_lead.goal, facts.goal);
    assert_eq!(changed_lead.criteria, facts.criteria);
    assert_eq!(changed_lead.partner_drafts, facts.partner_drafts);
    // Manual Eevee and automatic specialist are distinct, valid stored modes.
    changed_lead.form = 0;
    changed_lead.auto_form = false;
    changed_lead.revision += 1;
    changed_lead = store.save(changed_lead).unwrap();
    assert!(
        !load_task(store.connection.as_ref().unwrap(), "project", automatic.id)
            .unwrap()
            .unwrap()
            .auto_form
    );
    changed_lead.form = 8;
    changed_lead.auto_form = true;
    changed_lead.revision += 1;
    let changed_lead = store.save(changed_lead).unwrap();
    drop(store);
    let mut reopened = state(&directory, "project", Policy::default());
    let project = load(reopened.connection.as_ref().unwrap(), "project").unwrap();
    assert_eq!(
        project.tasks.iter().find(|task| task.id == changed_lead.id),
        Some(&changed_lead)
    );
    let partner = project
        .partners
        .iter()
        .find(|partner| partner.id == second)
        .unwrap();
    assert_eq!((partner.form, partner.auto_form), (8, true));
    let original = project
        .partners
        .iter()
        .find(|partner| partner.id == 1)
        .unwrap();
    assert_eq!((original.form, original.auto_form), (manual.form, false));
    assert!(
        project
            .tasks
            .iter()
            .find(|task| task.id == automatic.id)
            .unwrap()
            .auto_form
    );
    let other_project = load(reopened.connection.as_ref().unwrap(), "other-project").unwrap();
    assert_eq!(other_project.partners, project.partners);
    reopened.select(automatic.id).unwrap();
    drop(reopened);
    let reopened = state(&directory, "project", Policy::default());
    let restored = load(reopened.connection.as_ref().unwrap(), "project").unwrap();
    assert_eq!(restored.active_id, Some(automatic.id));
    assert_eq!(restored.partners, project.partners);
    assert_eq!(restored.tasks, project.tasks);
}

#[test]
fn failed_task_save_rolls_back_partner_form_and_mode_together() {
    let directory = Directory::new();
    let mut store = state(&directory, "project", Policy::default());
    let original = store.save(snapshot("project", 1)).unwrap();
    let roster = partners(store.connection.as_ref().unwrap()).unwrap();
    store.policy.quota_bytes = 1;
    let mut changed = original.clone();
    changed.form = 7;
    changed.auto_form = true;
    changed.goal = "Must not partially commit".into();
    changed.revision += 1;
    assert!(store.save(changed).is_err());
    assert_eq!(
        partners(store.connection.as_ref().unwrap()).unwrap(),
        roster
    );
    assert_eq!(
        load_task(store.connection.as_ref().unwrap(), "project", original.id).unwrap(),
        Some(original)
    );
}

#[test]
fn invalid_auto_form_and_future_schema_are_refused_before_mutable_open() {
    for invalid in [Some(-1), Some(2), None] {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        store.save(snapshot("project", 1)).unwrap();
        let connection = store.connection.as_ref().unwrap();
        assert!(
            connection
                .execute("UPDATE partners SET auto_form=2 WHERE id=1", [])
                .is_err()
        );
        if let Some(value) = invalid {
            connection
                .execute_batch("PRAGMA ignore_check_constraints=ON;")
                .unwrap();
            connection
                .execute("UPDATE partners SET auto_form=?1 WHERE id=1", [value])
                .unwrap();
        } else {
            let current: i64 = connection
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            connection
                .pragma_update(None, "user_version", current + 1)
                .unwrap();
        }
        drop(store);
        let path = directory.0.join(DATABASE);
        let before = fs::read(&path).unwrap();
        assert!(preflight(&path).is_err());
        let reopened = worker(&directory, "project");
        assert!(matches!(
            event(&reopened),
            Event::Failed { revision: None, .. }
        ));
        drop(reopened);
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn partner_release_resumes_the_retained_partners_mode_without_changing_facts() {
    for retained_auto in [false, true] {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let second = store.add_partner().unwrap().last().unwrap().id;
        let mut retained = snapshot("project", 1);
        retained.auto_form = retained_auto;
        let retained = store.save(retained).unwrap();
        let mut released = snapshot("project", 1);
        released.lead_partner = second;
        released.form = 7;
        released.auto_form = !retained_auto;
        released.goal = "Facts shared independently of personal mode".into();
        let released = store.save(released).unwrap();
        let project = store.release_partner(second).unwrap();
        let repaired = project
            .tasks
            .iter()
            .find(|task| task.id == released.id)
            .unwrap();
        assert_eq!(repaired.lead_partner, retained.lead_partner);
        assert_eq!(
            (repaired.form, repaired.auto_form),
            (retained.form, retained_auto)
        );
        assert_eq!(repaired.goal, released.goal);
        assert_eq!(repaired.criteria, released.criteria);
        assert_eq!(repaired.next_action, released.next_action);
        let expected = repaired.clone();
        drop(store);
        let reopened = state(&directory, "project", Policy::default());
        assert_eq!(
            load_task(
                reopened.connection.as_ref().unwrap(),
                "project",
                released.id
            )
            .unwrap(),
            Some(expected)
        );
    }
}
