use super::*;
use crate::workspace::memory::{Mutation, Proposal, Provenance, Record, Scope, SourceKind};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    project: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "ev-held-{}-{}-{}",
            std::process::id(),
            timestamp().unwrap(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("project")).unwrap();
        fs::create_dir_all(root.join("other")).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let project = root.join("project");
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(&project)
                .status()
                .unwrap()
                .success()
        );
        fs::write(
            project.join("source.rs"),
            "first source line\nsecond source line\n",
        )
        .unwrap();
        Self {
            project: project.to_str().unwrap().into(),
            root,
        }
    }
    fn open(&self, project: &str) -> State {
        let mut state = State {
            data_dir: self.root.join("data"),
            project: project.into(),
            policy: Policy::default(),
            connection: None,
            lock: None,
        };
        state.open_and_load().unwrap();
        state
    }
    fn task(&self, state: &mut State) -> Snapshot {
        let mut task = Snapshot::empty(state.project.clone());
        task.goal = "Keep explicit local choices separate from request context".into();
        state.save(task).unwrap()
    }
    fn proof(&self, task: i64, path: &str, exclusions: Vec<PathBuf>) -> FileProof {
        use sha2::{Digest, Sha256};
        let bytes = fs::read(Path::new(&self.project).join(path)).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        let text = crate::workspace::privacy::redact(&text).into_owned();
        FileProof::new(
            crate::workspace::context::Excerpt {
                id: 0,
                task_id: task,
                root: self.project.clone(),
                path: path.into(),
                start_line: 1,
                end_line: text.split_inclusive('\n').count(),
                source_sha256: Sha256::digest(&bytes).into(),
                text,
                available: true,
                origin_run: None,
            },
            exclusions,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn call(
    state: &mut State,
    task: i64,
    partner: i64,
    form: usize,
    target: Option<Target>,
    file_proof: Option<FileProof>,
) -> Result<crate::workspace::records::Snapshot, String> {
    match state.records(&super::super::records::Request {
        task_id: task,
        operation: super::super::records::Operation::SetHeld {
            partner,
            form,
            target,
            file_proof,
        },
    })? {
        super::super::records::Response::Snapshot(snapshot) => Ok(*snapshot),
        _ => panic!("Held mutation must return its committed snapshot"),
    }
}
fn approve(state: &mut State, task: i64, scope: Scope, content: &str) -> Record {
    state
        .memory_request(MemoryRequest {
            task_id: task,
            mutation: Some(Mutation::Approve(Proposal {
                scope,
                content: content.into(),
                provenance: Provenance {
                    kind: SourceKind::UserAuthored,
                    reference: "Explicit user note".into(),
                    task_id: task,
                },
            })),
        })
        .unwrap()
        .pop()
        .unwrap()
}
fn memory(record: &Record) -> Target {
    Target::Memory {
        id: record.id,
        revision: record.revision,
    }
}
fn stored(state: &State) -> Vec<Item> {
    items(state.connection.as_ref().unwrap(), &state.project).unwrap()
}
fn retained(connection: &Connection) -> BTreeMap<String, Vec<Vec<rusqlite::types::Value>>> {
    let names: Vec<String> = connection.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT GLOB 'sqlite_*' AND name!='held_items' ORDER BY name").unwrap()
        .query_map([], |row| row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
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
                        .map(|index| row.get(index))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}

#[test]
fn choices_replace_one_exact_slot_and_survive_restart_without_selecting_context() {
    let fixture = Fixture::new();
    let mut state = fixture.open(&fixture.project);
    let task = fixture.task(&mut state);
    let second = state.add_partner().unwrap().last().unwrap().id;
    let first = approve(
        &mut state,
        task.id,
        Scope::Project,
        "First committed choice",
    );
    let replacement = approve(
        &mut state,
        task.id,
        Scope::Project,
        "Replacement committed choice",
    );
    call(&mut state, task.id, 1, 0, Some(memory(&first)), None).unwrap();
    call(&mut state, task.id, 1, 1, Some(memory(&first)), None).unwrap();
    call(&mut state, task.id, second, 0, Some(memory(&first)), None).unwrap();
    let result = call(&mut state, task.id, 1, 0, Some(memory(&replacement)), None).unwrap();
    assert_eq!(result.held_items.len(), 3);
    assert_eq!(result.held_items[0].target, memory(&replacement));
    let saved = load_task(
        state.connection.as_ref().unwrap(),
        &fixture.project,
        task.id,
    )
    .unwrap()
    .unwrap();
    assert!(saved.approved_memories.is_empty());
    assert!(
        excerpts(state.connection.as_ref().unwrap(), task.id)
            .unwrap()
            .is_empty()
    );
    let expected = stored(&state);
    drop(state);
    let mut state = fixture.open(&fixture.project);
    assert_eq!(stored(&state), expected);
    call(&mut state, task.id, 1, 0, None, None).unwrap();
    assert_eq!(stored(&state).len(), 2);
    drop(state);
    let other_project = fixture.root.join("other").to_str().unwrap().to_owned();
    let mut other = fixture.open(&other_project);
    let other_task = fixture.task(&mut other);
    assert!(stored(&other).is_empty());
    assert!(call(&mut other, other_task.id, 1, 0, Some(memory(&first)), None).is_err());
    let personal = approve(
        &mut other,
        other_task.id,
        Scope::Personal,
        "Personal explicitly approved note",
    );
    call(
        &mut other,
        other_task.id,
        1,
        0,
        Some(memory(&personal)),
        None,
    )
    .unwrap();
    drop(other);
    let original = fixture.open(&fixture.project);
    assert_eq!(stored(&original).len(), 2);
    inspect(original.connection.as_ref().unwrap()).unwrap();
}

#[test]
fn unavailable_revision_task_scope_and_invalid_owner_cannot_replace_saved_choice() {
    let fixture = Fixture::new();
    let mut state = fixture.open(&fixture.project);
    let first_task = fixture.task(&mut state);
    let private = approve(
        &mut state,
        first_task.id,
        Scope::Task,
        "Only the first task may use this",
    );
    call(
        &mut state,
        first_task.id,
        1,
        0,
        Some(memory(&private)),
        None,
    )
    .unwrap();
    let second_task = fixture.task(&mut state);
    let shared = approve(
        &mut state,
        second_task.id,
        Scope::Project,
        "Visible from both saved tasks",
    );
    let old = stored(&state);
    for (task, partner, form, target) in [
        (second_task.id, 1, 0, memory(&private)),
        (
            second_task.id,
            1,
            0,
            Target::Memory {
                id: private.id,
                revision: private.revision + 1,
            },
        ),
        (
            second_task.id,
            1,
            0,
            Target::Memory {
                id: shared.id + 1000,
                revision: 1,
            },
        ),
        (second_task.id, 1, 0, Target::Memory { id: 0, revision: 0 }),
        (second_task.id, 999, 0, memory(&shared)),
        (second_task.id, 1, 9, memory(&shared)),
        (first_task.id, 1, 0, memory(&shared)),
        (0, 1, 0, memory(&shared)),
    ] {
        assert!(call(&mut state, task, partner, form, Some(target), None).is_err());
        assert_eq!(stored(&state), old);
    }
    let owner = Attribution::of(&partners(state.connection.as_ref().unwrap()).unwrap()[0]);
    let claim = WorkClaim {
        task_id: second_task.id,
        owner,
        request_id: "held-busy".into(),
        writing: false,
    };
    state
        .work_request(&WorkRequest::Acquire {
            claim: claim.clone(),
            task: Box::new(second_task.clone()),
        })
        .unwrap();
    assert!(
        call(
            &mut state,
            second_task.id,
            1,
            0,
            Some(memory(&shared)),
            None
        )
        .is_err()
    );
    state.work_request(&WorkRequest::Release(claim)).unwrap();
    assert_eq!(stored(&state), old);
    assert!(
        memories(
            state.connection.as_ref().unwrap(),
            &fixture.project,
            second_task.id
        )
        .unwrap()
        .iter()
        .all(|record| record.id != private.id)
    );
    state
        .manage_at(
            Management::Archive {
                id: second_task.id,
                archived: true,
            },
            timestamp().unwrap(),
        )
        .unwrap();
    assert!(
        call(
            &mut state,
            second_task.id,
            1,
            0,
            Some(memory(&shared)),
            None
        )
        .is_err()
    );
    assert_eq!(stored(&state), old);
}

#[test]
fn file_preferences_require_current_review_policy_and_do_not_pin() {
    let fixture = Fixture::new();
    let mut state = fixture.open(&fixture.project);
    let task = fixture.task(&mut state);
    let target = Target::File {
        path: "source.rs".into(),
    };
    assert!(call(&mut state, task.id, 1, 0, Some(target.clone()), None).is_err());
    let proof = fixture.proof(task.id, "source.rs", Vec::new());
    let result = call(
        &mut state,
        task.id,
        1,
        0,
        Some(target.clone()),
        Some(proof.clone()),
    )
    .unwrap();
    assert_eq!(result.held_items[0].target, target);
    assert!(
        excerpts(state.connection.as_ref().unwrap(), task.id)
            .unwrap()
            .is_empty()
    );
    let old = stored(&state);
    let excluded = fixture.proof(task.id, "source.rs", vec!["source.rs".into()]);
    assert!(
        call(
            &mut state,
            task.id,
            1,
            0,
            Some(target.clone()),
            Some(excluded)
        )
        .is_err()
    );
    for path in [
        ".env",
        ".git/config",
        "../outside.rs",
        "/absolute.rs",
        "workbench.sqlite3",
        "key.pem",
        "oauth_creds.json",
        "",
    ] {
        assert!(
            call(
                &mut state,
                task.id,
                1,
                0,
                Some(Target::File { path: path.into() }),
                Some(proof.clone())
            )
            .is_err()
        );
    }
    fs::write(
        Path::new(&fixture.project).join(".gitignore"),
        "source.rs\n",
    )
    .unwrap();
    assert!(
        call(
            &mut state,
            task.id,
            1,
            0,
            Some(target.clone()),
            Some(proof.clone())
        )
        .is_err()
    );
    fs::remove_file(Path::new(&fixture.project).join(".gitignore")).unwrap();
    fs::create_dir_all(Path::new(&fixture.project).join("local-data")).unwrap();
    fs::write(
        Path::new(&fixture.project).join("local-data/note.txt"),
        "Protected data\n",
    )
    .unwrap();
    let protected = fixture.proof(task.id, "local-data/note.txt", Vec::new());
    assert!(
        file_proof(
            &Path::new(&fixture.project).join("local-data"),
            &fixture.project,
            &task,
            "local-data/note.txt",
            &protected
        )
        .is_err()
    );
    let mut foreign = proof.clone();
    foreign.excerpt.root = fixture.root.join("other").to_str().unwrap().into();
    assert!(
        call(
            &mut state,
            task.id,
            1,
            0,
            Some(target.clone()),
            Some(foreign)
        )
        .is_err()
    );
    fs::write(
        Path::new(&fixture.project).join("source.rs"),
        "edited after review\n",
    )
    .unwrap();
    assert!(
        call(
            &mut state,
            task.id,
            1,
            0,
            Some(target.clone()),
            Some(proof.clone())
        )
        .is_err()
    );
    fs::remove_file(Path::new(&fixture.project).join("source.rs")).unwrap();
    assert!(
        call(
            &mut state,
            task.id,
            1,
            0,
            Some(target.clone()),
            Some(proof.clone())
        )
        .is_err()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            "../outside.rs",
            Path::new(&fixture.project).join("source.rs"),
        )
        .unwrap();
        fs::write(
            fixture.root.join("outside.rs"),
            "first source line\nsecond source line\n",
        )
        .unwrap();
        assert!(
            call(
                &mut state,
                task.id,
                1,
                0,
                Some(target.clone()),
                Some(proof.clone())
            )
            .is_err()
        );
    }
    assert_eq!(stored(&state), old);
    drop(state);
    let state = fixture.open(&fixture.project);
    assert_eq!(stored(&state), old); // Unavailable files stay identity-only preferences, not authority.
}

#[test]
fn quota_failure_rolls_back_replacement_and_retains_all_other_records() {
    let fixture = Fixture::new();
    let mut state = fixture.open(&fixture.project);
    let task = fixture.task(&mut state);
    let first = approve(&mut state, task.id, Scope::Project, "Retain this choice");
    let next = approve(
        &mut state,
        task.id,
        Scope::Project,
        "Do not commit without quota",
    );
    call(&mut state, task.id, 1, 0, Some(memory(&first)), None).unwrap();
    let old = stored(&state);
    let others = retained(state.connection.as_ref().unwrap());
    state.policy.quota_bytes = 0;
    assert!(
        call(&mut state, task.id, 1, 0, Some(memory(&next)), None)
            .unwrap_err()
            .contains("quota")
    );
    assert_eq!(stored(&state), old);
    assert_eq!(retained(state.connection.as_ref().unwrap()), others);
    state.policy = Policy::default();
    call(&mut state, task.id, 1, 0, Some(memory(&next)), None).unwrap();
    assert_eq!(stored(&state)[0].target, memory(&next));
}

#[test]
fn memory_edits_deletes_partner_release_and_task_project_purges_clean_exact_references() {
    let fixture = Fixture::new();
    let mut state = fixture.open(&fixture.project);
    let first_task = fixture.task(&mut state);
    let private = approve(
        &mut state,
        first_task.id,
        Scope::Task,
        "First task private note",
    );
    let shared = approve(
        &mut state,
        first_task.id,
        Scope::Project,
        "Shared note survives source task deletion",
    );
    let personal = approve(
        &mut state,
        first_task.id,
        Scope::Personal,
        "Shared personal note",
    );
    let second = state.add_partner().unwrap().last().unwrap().id;
    call(
        &mut state,
        first_task.id,
        1,
        0,
        Some(memory(&private)),
        None,
    )
    .unwrap();
    call(&mut state, first_task.id, 1, 1, Some(memory(&shared)), None).unwrap();
    call(
        &mut state,
        first_task.id,
        second,
        0,
        Some(memory(&personal)),
        None,
    )
    .unwrap();
    let second_task = fixture.task(&mut state);
    state
        .manage_at(
            Management::Delete {
                ids: vec![first_task.id],
                expired_before: None,
            },
            timestamp().unwrap(),
        )
        .unwrap();
    assert_eq!(stored(&state).len(), 2);
    assert_eq!(stored(&state)[0].target, memory(&shared));
    state.release_partner(second).unwrap();
    assert_eq!(stored(&state).len(), 1);
    let mut proposal = shared.proposal.clone();
    proposal.content = "An explicitly edited revision is a different choice".into();
    let edited = state
        .memory_request(MemoryRequest {
            task_id: second_task.id,
            mutation: Some(Mutation::Edit {
                id: shared.id,
                expected_revision: shared.revision,
                proposal,
            }),
        })
        .unwrap()
        .into_iter()
        .find(|record| record.id == shared.id)
        .unwrap();
    assert!(stored(&state).is_empty());
    assert!(
        call(
            &mut state,
            second_task.id,
            1,
            1,
            Some(memory(&shared)),
            None
        )
        .is_err()
    );
    call(
        &mut state,
        second_task.id,
        1,
        1,
        Some(memory(&edited)),
        None,
    )
    .unwrap();
    state
        .memory_request(MemoryRequest {
            task_id: second_task.id,
            mutation: Some(Mutation::Delete {
                id: edited.id,
                expected_revision: edited.revision,
            }),
        })
        .unwrap();
    assert!(stored(&state).is_empty());
    call(
        &mut state,
        second_task.id,
        1,
        0,
        Some(memory(&personal)),
        None,
    )
    .unwrap();
    state
        .manage_at(
            Management::Delete {
                ids: vec![second_task.id],
                expired_before: None,
            },
            timestamp().unwrap(),
        )
        .unwrap();
    assert!(stored(&state).is_empty());
    assert!(
        memories(state.connection.as_ref().unwrap(), &fixture.project, 0)
            .unwrap()
            .iter()
            .any(|record| record.id == personal.id)
    );
    inspect(state.connection.as_ref().unwrap()).unwrap();
}

#[test]
fn schema16_upgrade_is_atomic_preserves_every_old_table_and_modes() {
    let fixture = Fixture::new();
    let mut state = fixture.open(&fixture.project);
    let task = fixture.task(&mut state);
    approve(
        &mut state,
        task.id,
        Scope::Project,
        "Retained pre-upgrade memory",
    );
    state.add_partner().unwrap();
    state
        .connection
        .as_ref()
        .unwrap()
        .execute("UPDATE partners SET auto_form=0 WHERE id=1", [])
        .unwrap();
    let source = fixture.proof(task.id, "source.rs", Vec::new()).excerpt;
    state
        .context(ContextRequest {
            task_id: task.id,
            producer: None,
            operation: ContextOperation::Pin(source),
        })
        .unwrap();
    state
        .preference_request(PreferenceRequest::Save(ProviderPreferences {
            provider: crate::providers::Agent::Codex,
            selections: std::collections::HashMap::new(),
        }))
        .unwrap();
    let mut connection = state.connection.take().unwrap();
    connection
        .execute_batch("DROP TABLE held_items; PRAGMA user_version=16;")
        .unwrap();
    let before = retained(&connection);
    {
        let transaction = connection.transaction().unwrap();
        super::super::schema::migrate(
            &transaction,
            super::super::schema::inspect(&transaction).unwrap(),
        )
        .unwrap();
        inspect(&transaction).unwrap();
        assert_eq!(retained(&transaction), before);
        assert!(items(&transaction, &fixture.project).unwrap().is_empty());
    }
    assert!(connection.prepare("SELECT * FROM held_items").is_err());
    assert_eq!(retained(&connection), before);
    connection.execute_batch("VACUUM;").unwrap();
    let pages: i64 = connection
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    connection
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    assert!(initialize(&mut connection).is_err());
    assert!(connection.prepare("SELECT * FROM held_items").is_err());
    assert_eq!(retained(&connection), before);
    connection
        .pragma_update(None, "max_page_count", 1_000_000)
        .unwrap();
    initialize(&mut connection).unwrap();
    assert!(matches!(
        super::super::schema::inspect(&connection).unwrap(),
        Schema::Current
    ));
    assert_eq!(retained(&connection), before);
    state.connection = Some(connection);
    validate_planning_input(&state.data_dir, &task, &[]).unwrap();
}

#[test]
fn corrupt_held_rows_or_schema_and_downgrade_marker_are_refused_without_repair() {
    for target in [
        r#"{"file":{"path":".env"}}"#,
        r#"{"file":{"path":"source.rs","content":"never store this"}}"#,
        r#"{"memory":{"id":999,"revision":1}}"#,
        r#"{"memory":{"id":1,"revision":0}}"#,
        r#"{"file":{"path":"../outside.rs"}}"#,
    ] {
        let fixture = Fixture::new();
        let mut state = fixture.open(&fixture.project);
        fixture.task(&mut state);
        let connection = state.connection.as_ref().unwrap();
        connection
            .execute(
                "INSERT INTO held_items(project,partner,form,target) VALUES(?1,1,0,?2)",
                params![fixture.project, target],
            )
            .unwrap();
        assert!(super::super::schema::inspect(connection).is_err());
        drop(state);
        let before = fs::read(fixture.root.join("data").join(DATABASE)).unwrap();
        let mut refused = State {
            data_dir: fixture.root.join("data"),
            project: fixture.project.clone(),
            policy: Policy::default(),
            connection: None,
            lock: None,
        };
        assert!(refused.open_and_load().is_err());
        assert_eq!(
            fs::read(fixture.root.join("data").join(DATABASE)).unwrap(),
            before
        );
    }
    let fixture = Fixture::new();
    let mut state = fixture.open(&fixture.project);
    let task = fixture.task(&mut state);
    let current: i64 = state
        .connection
        .as_ref()
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    for version in [current - 1, current + 1] {
        state
            .connection
            .as_ref()
            .unwrap()
            .pragma_update(None, "user_version", version)
            .unwrap();
        let before = fs::read(state.data_dir.join(DATABASE)).unwrap();
        assert!(validate_planning_input(&state.data_dir, &task, &[]).is_err());
        assert_eq!(fs::read(state.data_dir.join(DATABASE)).unwrap(), before);
    }
    let connection = state.connection.as_ref().unwrap();
    connection
        .pragma_update(None, "user_version", current)
        .unwrap();
    connection
        .execute_batch("ALTER TABLE held_items ADD COLUMN copied_content TEXT;")
        .unwrap();
    assert!(super::super::schema::inspect(connection).is_err());
}

#[test]
fn all_live_owner_form_slots_are_bounded_and_full_capacity_still_allows_replacement() {
    let fixture = Fixture::new();
    let mut state = fixture.open(&fixture.project);
    let task = fixture.task(&mut state);
    for _ in 1..MAX_PARTNERS {
        state.add_partner().unwrap();
    }
    let first = approve(
        &mut state,
        task.id,
        Scope::Project,
        "A bounded preference identity",
    );
    let next = approve(
        &mut state,
        task.id,
        Scope::Project,
        "A replacement is not an extra slot",
    );
    let roster = partners(state.connection.as_ref().unwrap()).unwrap();
    for member in &roster {
        for form in 0..9 {
            call(
                &mut state,
                task.id,
                member.id,
                form,
                Some(memory(&first)),
                None,
            )
            .unwrap();
        }
    }
    assert_eq!(stored(&state).len(), MAX_PROJECT_ITEMS);
    call(
        &mut state,
        task.id,
        roster[0].id,
        0,
        Some(memory(&next)),
        None,
    )
    .unwrap();
    assert_eq!(stored(&state).len(), MAX_PROJECT_ITEMS);
    assert_eq!(stored(&state)[0].target, memory(&next));
    assert!(
        call(
            &mut state,
            task.id,
            roster.last().unwrap().id + 1,
            0,
            Some(memory(&next)),
            None
        )
        .is_err()
    );
    assert_eq!(stored(&state).len(), MAX_PROJECT_ITEMS);
    call(&mut state, task.id, roster[0].id, 0, None, None).unwrap();
    assert_eq!(stored(&state).len(), MAX_PROJECT_ITEMS - 1);
    assert!(matches!(
        super::super::schema::inspect(state.connection.as_ref().unwrap()).unwrap(),
        Schema::Current
    ));
}

#[test]
fn personal_memory_invalidation_cleans_all_projects_without_touching_other_slots() {
    let fixture = Fixture::new();
    let mut state = fixture.open(&fixture.project);
    let first_task = fixture.task(&mut state);
    let personal = approve(
        &mut state,
        first_task.id,
        Scope::Personal,
        "Explicitly shared personal preference",
    );
    let project = approve(
        &mut state,
        first_task.id,
        Scope::Project,
        "Unrelated project preference",
    );
    call(
        &mut state,
        first_task.id,
        1,
        0,
        Some(memory(&personal)),
        None,
    )
    .unwrap();
    call(
        &mut state,
        first_task.id,
        1,
        1,
        Some(memory(&project)),
        None,
    )
    .unwrap();
    drop(state);
    let other_project = fixture.root.join("other").to_str().unwrap().to_owned();
    let mut other = fixture.open(&other_project);
    let other_task = fixture.task(&mut other);
    call(
        &mut other,
        other_task.id,
        1,
        0,
        Some(memory(&personal)),
        None,
    )
    .unwrap();
    other
        .memory_request(MemoryRequest {
            task_id: other_task.id,
            mutation: Some(Mutation::Delete {
                id: personal.id,
                expected_revision: personal.revision,
            }),
        })
        .unwrap();
    assert!(stored(&other).is_empty());
    drop(other);
    let state = fixture.open(&fixture.project);
    assert_eq!(
        stored(&state),
        vec![Item {
            partner: 1,
            form: 1,
            target: memory(&project)
        }]
    );
}
