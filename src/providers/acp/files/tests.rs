use super::*;

#[test]
fn structured_diff_schema_never_guesses_raw_input_or_authority() {
    let tool = json!({"rawInput":{"file_path":"guessed.txt","content":"not a schema"}});
    assert!(external_file_review(&tool).is_none());
    let tool = json!({"kind":"edit","content":[{"type":"diff","path":"/project/new.txt","oldText":null,"newText":"actual reported body\n"}]});
    let review = external_file_review(&tool).unwrap();
    assert!(review.external);
    assert_eq!(review.bytes, "actual reported body\n".len());
    assert_eq!(diffs(&tool["content"])[0].1, "actual reported body\n");
    assert!(external_file_review(&json!({"content":[{"type":"diff","path":"/p","newText":"one"},{"type":"content","content":{"type":"text","text":"other unknown effects"}}]})).is_none());
    assert!(diffs(&json!([{"type":"diff","path":"/p","newText":42}])).is_empty());
    assert!(external_file_review(&json!({"kind":"edit","content":[{"type":"diff","path":"/p","newText":"unknown original"}]})).is_none());
    assert!(external_file_review(&json!({"kind":"execute","content":tool["content"]})).is_none());
}

#[test]
fn file_capabilities_are_operation_bound_not_profile_guesses() {
    for agent in [Agent::OpenCode, Agent::Gemini, Agent::Devin] {
        for operation in [Operation::Plan, Operation::Implement, Operation::Review] {
            let mut input = crate::providers::tests::claimed(
                agent,
                operation,
                operation == Operation::Implement,
            );
            assert_eq!(
                client_capabilities(&input)["fs"]["writeTextFile"],
                operation == Operation::Implement
            );
            assert_eq!(
                client_capabilities(&input)["fs"]["readTextFile"],
                operation != Operation::Review
            );
            input.inspect_models = true;
            assert_eq!(
                client_capabilities(&input)["fs"],
                json!({"readTextFile":false,"writeTextFile":false})
            );
        }
    }
}

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "eevee-acp-files-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        Self {
            root: root.canonicalize().unwrap(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn acp_reads_requested_ranges_beyond_mutation_bound_without_silent_output_truncation() {
    let fixture = Fixture::new();
    let path = "중앙-cafe\u{301}.rs";
    let source: String = (1..=3000)
        .map(|line| format!("// ACP actual source {line:04}: café 界\r\n"))
        .collect();
    assert!(source.len() > crate::providers::writes::MAX_BYTES);
    std::fs::write(fixture.root.join(path), &source).unwrap();
    let mut scope =
        WriteSet::capture_workspace(&fixture.root, &fixture.root.join("private-data"), &[])
            .unwrap();
    let params = json!({"path":fixture.root.join(path),"line":2500,"limit":2});
    let selected: String = source.split_inclusive('\n').skip(2499).take(2).collect();
    assert_eq!(
        read_content(&mut scope, path.into(), &params).unwrap(),
        selected
    );
    let tail = json!({"path":fixture.root.join(path),"line":3000,"limit":50});
    assert_eq!(
        read_content(&mut scope, path.into(), &tail).unwrap(),
        source.split_inclusive('\n').next_back().unwrap()
    );
    let unbounded = json!({"path":fixture.root.join(path)});
    assert!(
        read_content(&mut scope, path.into(), &unbounded)
            .unwrap_err()
            .contains("narrower line/limit")
    );
    assert!(scope.admit_paths(&[path.into()]).is_err());
    assert_eq!(
        std::fs::read(fixture.root.join(path)).unwrap(),
        source.as_bytes()
    );
}

#[test]
fn shared_callbacks_read_create_replace_and_refuse_unsafe_scope_on_every_host() {
    for agent in [Agent::OpenCode, Agent::Gemini, Agent::Devin] {
        let fixture = Fixture::new();
        let workspace = Workspace {
            project: fixture.root.clone(),
            data_dir: fixture.root.join("private-data"),
            exclusions: vec!["excluded.txt".into()],
        };
        std::fs::create_dir(&workspace.data_dir).unwrap();
        std::fs::write(
            fixture.root.join("existing.txt"),
            b"original\r\npreserved\r\n",
        )
        .unwrap();
        let mut input = crate::providers::tests::claimed(agent, Operation::Implement, true);
        let mut files = capture_files(&workspace, &input).unwrap().unwrap();
        let params = json!({"path":fixture.root.join("existing.txt"),"line":2,"limit":50});
        let path = relative_path(&fixture.root, &params).unwrap();
        assert_eq!(
            read_content(&mut files, path, &params).unwrap(),
            "preserved\r\n"
        );
        assert!(
            relative_path(
                &fixture.root,
                &json!({"path":fixture.root.parent().unwrap().join("outside.txt")})
            )
            .is_err()
        );
        for path in ["private-data/blocked.txt", "excluded.txt", "../outside.txt"] {
            assert!(files.admit_paths(&[path.into()]).is_err());
        }
        files.admit_paths(&["nested/new.txt".into()]).unwrap();
        let proposal = files
            .propose("nested/new.txt".into(), "created\r\n".into())
            .unwrap();
        assert!(!fixture.root.join("nested").exists());
        let PromptKind::Permission {
            file: Some(review), ..
        } = file_permission(&proposal)
        else {
            panic!("Expected exact file review")
        };
        assert!(!review.external);
        files.apply(&proposal).unwrap();
        assert_eq!(
            std::fs::read(fixture.root.join("nested/new.txt")).unwrap(),
            b"created\r\n"
        );
        files.admit_paths(&["existing.txt".into()]).unwrap();
        let proposal = files
            .propose("existing.txt".into(), "edited\r\npreserved\r\n".into())
            .unwrap();
        files.apply(&proposal).unwrap();
        assert_eq!(
            std::fs::read(fixture.root.join("existing.txt")).unwrap(),
            b"edited\r\npreserved\r\n"
        );
        let stale = files
            .propose("existing.txt".into(), "stale replacement".into())
            .unwrap();
        std::fs::write(fixture.root.join("existing.txt"), b"external dirty bytes").unwrap();
        assert!(!files.apply(&stale).unwrap_err().uncertain);
        input.work.as_mut().unwrap().writing = false;
        assert!(capture_files(&workspace, &input).is_err());
    }
}

#[cfg(unix)]
fn store_event(store: &crate::store::Worker) -> crate::store::Event {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(event) = store.try_event() {
            return event;
        }
        assert!(Instant::now() < deadline, "Store acknowledgment timed out");
        thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(unix)]
fn owned_input(workspace: &Workspace, agent: Agent, mode: &str) -> (crate::store::Worker, Input) {
    let store = crate::store::Worker::start(
        workspace.data_dir.clone(),
        workspace.project.to_str().unwrap().into(),
        crate::store::Policy::default(),
    )
    .unwrap();
    let crate::store::Event::Loaded(project) = store_event(&store) else {
        panic!("Expected project")
    };
    let owner = crate::store::Attribution::of(&project.partners[0]);
    let mut task = crate::store::Snapshot::empty(workspace.project.to_str().unwrap().into());
    task.goal = "Exercise actual current-project file callbacks".into();
    task.lead_partner = owner.partner;
    task.form = owner.form;
    task.revision = 1;
    task.partner_drafts
        .entry(task.lead_partner)
        .or_default()
        .drafts
        .push(crate::store::Draft {
            field: crate::store::EditorField::Request,
            text: mode.into(),
            cursor: mode.len(),
        });
    let mut save = Some(task);
    assert!(store.try_save(&mut save));
    let crate::store::Event::Saved(task) = store_event(&store) else {
        panic!("Expected saved task")
    };
    let claim = crate::store::WorkClaim {
        task_id: task.id,
        owner,
        request_id: "acp-file-peer-fixture".into(),
        writing: true,
    };
    let mut acquire = Some(crate::store::WorkRequest::Acquire {
        claim: claim.clone(),
        task: Box::new(task.clone()),
    });
    assert!(store.try_work(&mut acquire));
    let crate::store::Event::Work { result, .. } = store_event(&store) else {
        panic!("Expected acquired claim")
    };
    result.unwrap();
    let mut input = crate::providers::tests::claimed(agent, Operation::Implement, true);
    input.task = task;
    input.work = Some(claim);
    (store, input)
}

#[cfg(unix)]
fn peer_case(agent: Agent, mode: &str) {
    let fixture = Fixture::new();
    let workspace = Workspace {
        project: fixture.root.clone(),
        data_dir: fixture.root.join(".eevee-data"),
        exclusions: vec!["excluded.txt".into()],
    };
    std::fs::write(
        fixture.root.join("existing.txt"),
        b"original\r\nkeep dirty bytes\r\n",
    )
    .unwrap();
    std::fs::write(fixture.root.join("excluded.txt"), b"protected exclusion\n").unwrap();
    std::os::unix::fs::symlink(
        fixture.root.join("existing.txt"),
        fixture.root.join("linked.txt"),
    )
    .unwrap();
    let (store, input) = owned_input(&workspace, agent, mode);
    let (index, launch) = profile(agent).unwrap();
    let files = capture_files(&workspace, &input).unwrap();
    let run = Reviewed {
        generation: 1,
        payload: payload(&workspace, &input, files.as_ref()).unwrap(),
        files,
        trust: crate::providers::agent::external_trust(index, &workspace.project, launch).unwrap(),
        input: Box::new(input),
    };
    let envelope: Value = serde_json::from_str(&run.payload).unwrap();
    assert_eq!(
        envelope["project_context"]["root"],
        json!(workspace.project)
    );
    assert_eq!(
        envelope["project_context"]["filenames"]["paths"],
        json!(["existing.txt"])
    );
    assert_eq!(envelope["selected_sources"], json!([]));
    assert!(
        !envelope["project_context"]
            .to_string()
            .contains("keep dirty bytes")
    );
    let events: Events = Arc::new(Mutex::new(VecDeque::new()));
    let stop = AtomicBool::new(false);
    let cancelled = AtomicU64::new(0);
    let (sender, incoming) = mpsc::sync_channel(8);
    let mut retained = Vec::new();
    let mut approvals = 0;
    let outcome = thread::scope(|scope| {
        let (workspace_ref, run_ref, events_ref, stop_ref, cancelled_ref) =
            (&workspace, &run, &events, &stop, &cancelled);
        let path = fixture.root.join("existing.txt");
        let worker = scope.spawn(move || {
            if mode == "unknown" {
                crate::providers::writes::test_after_leaf_write(move || {
                    std::fs::write(path, "concurrent bytes after leaf write\n").unwrap();
                });
            }
            transport(
                workspace_ref,
                run_ref,
                &incoming,
                events_ref,
                stop_ref,
                cancelled_ref,
                &mut HashMap::new(),
            )
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        while !worker.is_finished() {
            let event = events.lock().pop_front();
            if let Some(event) = event {
                if let Event::Prompt { request, .. } = &event {
                    approvals += 1;
                    let PromptKind::Permission {
                        file: Some(file),
                        options,
                        ..
                    } = &request.kind
                    else {
                        panic!("Expected exact typed file review")
                    };
                    assert!(!file.external);
                    assert_eq!(options.len(), 2);
                    if approvals == 1 {
                        assert!(
                            !fixture.root.join("nested").exists(),
                            "No parents before consent"
                        );
                        assert_eq!(
                            std::fs::read(fixture.root.join("existing.txt")).unwrap(),
                            b"original\r\nkeep dirty bytes\r\n"
                        );
                        match mode {
                            "stale" => std::fs::write(
                                fixture.root.join("existing.txt"),
                                "user changed before consent\n",
                            )
                            .unwrap(),
                            "parents" => std::fs::create_dir(fixture.root.join("nested")).unwrap(),
                            "lease" => {
                                let mut release = Some(crate::store::WorkRequest::Release(
                                    run.input.work.clone().unwrap(),
                                ));
                                assert!(store.try_work(&mut release));
                                let crate::store::Event::Work { result, .. } = store_event(&store)
                                else {
                                    panic!("Expected released claim")
                                };
                                result.unwrap();
                            }
                            "cancel" => {
                                cancelled.store(1, Ordering::Release);
                            }
                            _ => {}
                        }
                    }
                    sender.send(Request::Answer { generation: 1, request_id: request.request_id.clone(), answer: json!({"decision":if mode == "deny" { "decline" } else { "accept" }}) }).unwrap();
                }
                retained.push(event);
            } else {
                thread::sleep(Duration::from_millis(2));
            }
            if Instant::now() >= deadline {
                stop.store(true, Ordering::Release);
                panic!("ACP deterministic peer timed out: {agent:?} {mode}");
            }
        }
        worker.join().unwrap()
    });
    retained.extend(events.lock().drain(..));
    let applied = retained
        .iter()
        .filter(|event| {
            matches!(
                event,
                Event::Item {
                    kind: ItemKind::File(FilePhase::Applied),
                    ..
                }
            )
        })
        .count();
    assert!(
        !retained.iter().any(|event| matches!(
            event,
            Event::Item {
                kind: ItemKind::File(FilePhase::Generating),
                ..
            }
        )),
        "ACP does not provide partial arguments"
    );
    let reported = if mode == "reported-failed" {
        FilePhase::ReportedFailed
    } else {
        FilePhase::Reported
    };
    assert!(retained.iter().any(|event| matches!(event, Event::Item { kind: ItemKind::File(phase), text, .. } if *phase == reported && text.contains("observed_only"))), "Structured external report must be typed, not Applied");
    assert!(
        !fixture.root.join("reported.rs").exists(),
        "An observed provider diff is not a host write"
    );
    assert!(retained.iter().any(|event| matches!(event, Event::Journal { record, .. } if record.origin.contains("session/update") && record.item_id == "acp-tool-external-edit" && record.text.contains("External diff observation"))), "Original tool journal remains correlated");
    match mode {
        "happy" | "reported-failed" => {
            assert_eq!(outcome.0, Outcome::Completed, "{}", outcome.1);
            assert_eq!(approvals, 2);
            assert_eq!(applied, 2);
            assert_eq!(
                std::fs::read(fixture.root.join("nested/new.txt")).unwrap(),
                b"actual new file\n"
            );
            assert_eq!(
                std::fs::read(fixture.root.join("existing.txt")).unwrap(),
                b"edited\r\nkeep dirty bytes\r\n"
            );
            assert!(retained.iter().filter(|event| matches!(event, Event::Journal { record, .. } if record.origin.contains("mediated") && record.text.contains("source_sha256"))).count() == 2);
        }
        "duplicate" | "disconnect" => {
            assert_eq!(outcome.0, Outcome::Unknown);
            assert_eq!(applied, 1);
            assert_eq!(
                std::fs::read(fixture.root.join("nested/new.txt")).unwrap(),
                b"actual new file\n"
            );
        }
        "unknown" => {
            assert_eq!(outcome.0, Outcome::Unknown);
            assert_eq!(applied, 0);
            assert!(
                retained
                    .iter()
                    .any(|event| matches!(event, Event::LocalEffectsUnknown { .. }))
            );
            assert!(retained.iter().any(|event| matches!(
                event,
                Event::Item {
                    kind: ItemKind::File(FilePhase::Unknown),
                    ..
                }
            )));
            assert_eq!(
                std::fs::read_to_string(fixture.root.join("existing.txt")).unwrap(),
                "concurrent bytes after leaf write\n"
            );
        }
        _ => {
            assert_eq!(applied, 0, "{agent:?} {mode}: {}", outcome.1);
            assert!(!fixture.root.join("nested/new.txt").exists());
            assert!(!workspace.data_dir.join("blocked.txt").exists());
            if mode != "stale" {
                assert_eq!(
                    std::fs::read(fixture.root.join("existing.txt")).unwrap(),
                    b"original\r\nkeep dirty bytes\r\n"
                );
            }
            if mode == "cancel" {
                assert_eq!(outcome.0, Outcome::Cancelled, "{}", outcome.1);
            } else {
                assert_eq!(outcome.0, Outcome::Completed, "{}", outcome.1);
            }
            let phase = match mode {
                "deny" => Some(FilePhase::Denied),
                "stale" | "parents" | "lease" => Some(FilePhase::Refused),
                "cancel" => Some(FilePhase::Interrupted),
                _ => None,
            };
            if let Some(phase) = phase {
                assert!(retained.iter().any(|event| matches!(event, Event::Item { kind: ItemKind::File(actual), .. } if *actual == phase)), "Missing exact refusal/cancellation phase: {mode}");
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn actual_acp_peer_file_callbacks_across_all_three_routes() {
    use std::os::unix::fs::PermissionsExt;
    if std::env::var("EEVEE_ACP_FILE_TEST_CHILD").as_deref() == Ok("1") {
        for agent in [Agent::OpenCode, Agent::Gemini, Agent::Devin] {
            for mode in [
                "happy",
                "deny",
                "stale",
                "outside",
                "protected",
                "symlink",
                "excluded",
                "parents",
                "lease",
                "cancel",
                "unknown",
                "reported-failed",
                "duplicate",
                "disconnect",
            ] {
                peer_case(agent, mode);
            }
        }
        return;
    }
    // Child-only environment changes; never replace installed providers or mutate
    // the parent test runner's process-global PATH/HOME/authentication state.
    let fixture = Fixture::new();
    let runtime = fixture.root.join("runtime");
    let home = fixture.root.join("home");
    std::fs::create_dir(&runtime).unwrap();
    std::fs::create_dir(&home).unwrap();
    for name in ["opencode", "gemini", "devin"] {
        let path = runtime.join(name);
        std::fs::write(
            &path,
            include_str!("../../../../scripts/acp_file_peer_fixture.py"),
        )
        .unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut paths = vec![runtime];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let current = thread::current();
    let name = current.name().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("EEVEE_ACP_FILE_TEST_CHILD", "1")
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Isolated ACP peer acceptance failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
