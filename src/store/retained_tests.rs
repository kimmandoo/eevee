use super::*;
use crate::workspace::{memory, workflows};
fn state(root: &Path, project: &str) -> State {
    let mut value = State {
        data_dir: root.into(),
        project: project.into(),
        policy: Policy::default(),
        connection: None,
        lock: None,
    };
    value.open_and_load().unwrap();
    value
}
fn task(state: &mut State, goal: &str) -> Snapshot {
    let mut app = crate::app::App::new(state.project.clone(), 80, 24);
    app.goal = goal.into();
    app.revision = 1;
    state.save(app.snapshot(&state.project)).unwrap()
}
fn request(operation: Operation) -> Request {
    Request {
        generation: 1,
        cancel: Arc::new(AtomicBool::new(false)),
        operation,
    }
}
fn search(state: &mut State, literal: &str, archived: bool) -> Vec<Hit> {
    let Response::Hits { values, .. } = state
        .retained_request(&request(Operation::Search {
            literal: literal.into(),
            archived,
        }))
        .unwrap()
    else {
        panic!("Expected search")
    };
    values
}
fn journal(state: &mut State, task: i64, id: &str, text: &str) {
    state
        .journal_request(JournalRequest {
            task_id: task,
            operation: JournalOperation::Put(Box::new(ProviderRecord {
                id: id.into(),
                agent: crate::providers::Agent::Compatible,
                generation: 1,
                thread_id: "original-thread".into(),
                turn_id: "original-request".into(),
                item_id: "original-response".into(),
                origin: "retained public response".into(),
                coverage: "bounded observed text".into(),
                text: text.into(),
                complete: true,
                partner: Some(Attribution {
                    partner: 1,
                    name: "Original partner".into(),
                    form: 0,
                }),
            })),
        })
        .unwrap();
}
#[test]
fn retained_search_project_archive_restart_purge_and_exact_navigation() {
    let root = std::env::temp_dir().join(format!(
        "ev-retained-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut current = state(&root, "/local-project");
    let first = task(&mut current, "needle first Unicode 설명");
    let mut second = task(&mut current, "needle second");
    second.revision += 1;
    second.criteria.push(Criterion {
        text: "needle criterion".into(),
        met: false,
    });
    second.workflows.chosen_decision = Some(workflows::ChosenDecision {
        id: "decision".into(),
        text: "needle decision".into(),
    });
    current.save(second.clone()).unwrap();
    current
        .memory_request(MemoryRequest {
            task_id: first.id,
            mutation: Some(memory::Mutation::Approve(memory::Proposal {
                scope: memory::Scope::Task,
                content: "needle approved note".into(),
                provenance: memory::Provenance {
                    kind: memory::SourceKind::UserAuthored,
                    reference: "explicit author".into(),
                    task_id: first.id,
                },
            })),
        })
        .unwrap();
    journal(
        &mut current,
        first.id,
        "retained-first",
        "needle explanation\nsecond line Unicode 설명",
    );
    let local = search(&mut current, "needle", false);
    assert_eq!(local.len(), 6);
    assert!(
        local
            .iter()
            .all(|hit| hit.task == first.id || hit.task == second.id)
    );
    let hit = local
        .iter()
        .find(|hit| matches!(hit.target, Target::Journal(_)))
        .unwrap()
        .clone();
    let label = Landmark {
        label: "Explanation 설명".into(),
        journal_id: "retained-first".into(),
        position: Some(8),
    };
    let mut saved = load_task(
        current.connection.as_ref().unwrap(),
        "/local-project",
        first.id,
    )
    .unwrap()
    .unwrap();
    saved.revision += 1;
    saved.workflows.landmarks.push(label.clone());
    current.save(saved).unwrap();
    current
        .manage(Management::Archive {
            id: second.id,
            archived: true,
        })
        .unwrap();
    assert_eq!(search(&mut current, "needle", false).len(), 3);
    drop(current);
    let mut foreign = state(&root, "/foreign-project");
    let foreign_task = task(&mut foreign, "needle FOREIGN PRIVATE");
    journal(
        &mut foreign,
        foreign_task.id,
        "retained-foreign",
        "needle FOREIGN PRIVATE BODY",
    );
    drop(foreign);
    let mut current = state(&root, "/local-project");
    let all = search(&mut current, "needle", true);
    assert_eq!(all.len(), 6);
    assert!(all.iter().all(|h| !h.excerpt.contains("FOREIGN")));
    let Response::Opened { text, position, .. } = current
        .retained_request(&request(Operation::Jump {
            task: first.id,
            landmark: label.clone(),
        }))
        .unwrap()
    else {
        panic!("Expected landmark")
    };
    assert_eq!(position, 8);
    assert!(text.contains("Unicode 설명"));
    assert!(
        load_task(
            current.connection.as_ref().unwrap(),
            "/local-project",
            first.id
        )
        .unwrap()
        .unwrap()
        .workflows
        .landmarks
        .contains(&label)
    );
    current
        .journal_request(JournalRequest {
            task_id: first.id,
            operation: JournalOperation::Purge,
        })
        .unwrap();
    assert!(
        current
            .retained_request(&request(Operation::Open { hit }))
            .is_err()
    );
    assert!(
        current
            .retained_request(&request(Operation::Jump {
                task: first.id,
                landmark: label
            }))
            .is_err()
    );
    assert_eq!(search(&mut current, "needle", true).len(), 5);
    let cancelled = request(Operation::Search {
        literal: "needle".into(),
        archived: true,
    });
    cancelled.cancel.store(true, Ordering::Relaxed);
    assert!(current.retained_request(&cancelled).is_err());
    drop(current);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn landmark_metadata_strict_bounded_and_does_not_duplicate_bodies() {
    let value = Landmark {
        label: "Unicode 설명".into(),
        journal_id: "immutable-journal".into(),
        position: Some(MAX_POSITION),
    };
    assert!(validate_landmarks(std::slice::from_ref(&value)).is_ok());
    assert!(validate_landmarks(&[value.clone(), value.clone()]).is_err());
    let mut invalid = value.clone();
    invalid.position = Some(MAX_POSITION + 1);
    assert!(validate_landmarks(&[invalid]).is_err());
    let encoded = serde_json::to_value(&value).unwrap();
    assert_eq!(encoded.as_object().unwrap().len(), 3);
    assert!(
        serde_json::from_value::<Landmark>(
            serde_json::json!({"label":"x","journal_id":"id","text":"not allowed"})
        )
        .is_err()
    );
    let mut metadata = workflows::State::default();
    metadata.landmarks.push(value);
    assert!(metadata.validate(1).is_ok());
}

#[test]
fn project_notes_survive_provenance_task_deletion_without_duplicate_or_foreign_counts() {
    let root = std::env::temp_dir().join(format!(
        "ev-retained-notes-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut current = state(&root, "/local-project");
    let first = task(&mut current, "original note provenance");
    task(&mut current, "remaining project task");
    current
        .memory_request(MemoryRequest {
            task_id: first.id,
            mutation: Some(memory::Mutation::Approve(memory::Proposal {
                scope: memory::Scope::Project,
                content: "orphan-note 説明".into(),
                provenance: memory::Provenance {
                    kind: memory::SourceKind::UserAuthored,
                    reference: "explicit shared approval".into(),
                    task_id: first.id,
                },
            })),
        })
        .unwrap();
    assert_eq!(search(&mut current, "orphan-note", true).len(), 1);
    current
        .manage(Management::Delete {
            ids: vec![first.id],
            expired_before: None,
        })
        .unwrap();
    drop(current);
    let mut current = state(&root, "/local-project");
    let hits = search(&mut current, "orphan-note", false);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].when, When::Unknown);
    assert!(hits[0].attribution.contains("not retained"));
    let Response::Opened { text, .. } = current
        .retained_request(&request(Operation::Open {
            hit: hits[0].clone(),
        }))
        .unwrap()
    else {
        panic!("Expected original approved note")
    };
    assert_eq!(text, "orphan-note 説明");
    drop(current);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retained_match_limits_and_unknown_times_are_honest() {
    let request = request(Operation::Search {
        literal: "needle".into(),
        archived: true,
    });
    let mut scan = Scan {
        request: &request,
        literal: "needle",
        values: Vec::new(),
        bytes: 0,
        scanned: 0,
        incomplete: false,
        bounded: false,
    };
    for id in 1..=129 {
        scan.add(
            (id, false),
            "goal",
            When::TaskModified(10),
            Target::Goal,
            "Unicode 설명 needle",
            String::new(),
        );
    }
    assert_eq!(scan.values.len(), 128);
    assert!(scan.incomplete && scan.stopped());
    assert!(When::Unknown.label().contains("unknown"));
    assert!(When::TaskModified(100).label().contains("Unix ms"));
}
