use super::*;
use crate::providers::writes::FileTool;

pub(super) fn child_run(
    fixture: &Fixture,
    protocol: Protocol,
    url: &str,
    paths: &[&str],
) -> (Prepared, Connection) {
    let connection = config::test_connection(protocol, url, Some("fixture-model"));
    let mut input = fixture.input.clone();
    input.operation = Operation::Plan;
    input.edit_paths.clear();
    input.delegation_enabled = false;
    input.delegation = Some(crate::providers::delegation::Context {
        parent_request_id: input.work.as_ref().unwrap().request_id.clone(),
        child_request_id: "native-child-fixture".into(),
        role: "explore".into(),
        instruction: "Inspect the actual scoped source; do not execute any effects".into(),
        call_id: "delegate-child".into(),
        paths: paths.iter().map(PathBuf::from).collect(),
    });
    input.conversation = None;
    input.prior_plan = None;
    input.public_context = None;
    input.tool_catalog = None;
    input.mcp.clear();
    input.behavior_profile = None;
    let writes = capture_writes(&fixture.workspace, &input).unwrap();
    let payload = payload(&fixture.workspace, &input, &connection, writes.as_ref()).unwrap();
    (
        Prepared {
            generation: 7,
            input: Box::new(input),
            payload,
            fingerprint: connection.fingerprint(),
            writes,
        },
        connection,
    )
}

#[test]
fn delegated_child_refuses_effects_commands_todo_and_recursion_in_all_native_protocols() {
    for protocol in [
        Protocol::Responses,
        Protocol::Codex,
        Protocol::ChatCompletions,
        Protocol::Messages,
        Protocol::Gemini,
    ] {
        let fixture = Fixture::new(&[]);
        std::fs::write(
            fixture.workspace.project.join("source.txt"),
            b"actual read-only child source\n",
        )
        .unwrap();
        let calls = [
            (
                "read",
                "eevee_read_file",
                json!({"path":"source.txt","start_line":1,"end_line":0}),
            ),
            (
                "effect",
                "eevee_write_file",
                json!({"path":"must-not-exist.txt","content":"unauthorized"}),
            ),
            (
                "command",
                "eevee_run_command",
                json!({"cwd":"","command":{}}),
            ),
            (
                "todo",
                "todo",
                json!({"operation":"replace","label":"","steps":[]}),
            ),
            (
                "recursive",
                "eevee_delegate",
                json!({"role":"review","instruction":"recurse","paths":[""]}),
            ),
        ];
        let (url, peer) = peer(vec![
            wire_tools(protocol, &calls, ""),
            wire_tools(protocol, &[], "Child observed only read-only source"),
        ]);
        let (mut run, connection) = child_run(&fixture, protocol, &url, &[""]);
        let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
        assert_eq!(outcome, Outcome::Completed, "{protocol:?}: {message}");
        assert!(
            !fixture
                .workspace
                .project
                .join("must-not-exist.txt")
                .exists()
        );
        assert!(!events.iter().any(|event| matches!(
            event,
            Event::Prompt { .. }
                | Event::RunCommand { .. }
                | Event::Progress { .. }
                | Event::Delegate { .. }
        )));
        let artifacts = events
            .iter()
            .filter_map(|event| {
                if let Event::ToolArtifact { capture, .. } = event {
                    Some(capture)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(artifacts.len(), 5);
        for capture in &artifacts {
            assert_eq!(capture.request_id, "native-child-fixture");
            let result: Value = serde_json::from_str(&capture.text).unwrap();
            assert_eq!(
                result["status"],
                if capture.call_id == "read" {
                    "success"
                } else {
                    "refused"
                }
            );
        }
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Usage { .. }))
        );
        assert_eq!(peer.join().unwrap().len(), 2);
    }
}

#[test]
fn child_paths_are_enforced_at_executor_without_prefix_confusion() {
    let fixture = Fixture::new(&[]);
    let (run, _) = child_run(
        &fixture,
        Protocol::Responses,
        "http://127.0.0.1:1",
        &["src"],
    );
    let read = |path: &str| FileTool::Read {
        path: PathBuf::from(path),
        start_line: 1,
        end_line: 0,
    };
    assert!(delegation::allowed(&run.input, &read("src/lib.rs")));
    for path in ["src2/lib.rs", "Cargo.toml", "src/../secret", "/src/lib.rs"] {
        assert!(!delegation::allowed(&run.input, &read(path)), "{path}");
    }
    assert!(!delegation::allowed(
        &run.input,
        &FileTool::List {
            directory: PathBuf::new(),
            query: String::new(),
            offset: 0
        }
    ));
    assert!(!delegation::allowed(&run.input, &FileTool::ArtifactList {}));
    assert!(run.payload.contains("native-child-fixture"));
    assert!(run.payload.contains("native-write-fixture"));
}

fn requests() -> Vec<tools::Call> {
    (0..3)
        .map(|index| tools::Call {
            id: Some(format!("child-{index}")),
            tool: FileTool::Delegate {
                role: if index == 1 { "review" } else { "explore" }.into(),
                instruction: format!("Instruction {index}: inspect exact scoped source"),
                paths: vec![PathBuf::from(format!("src{index}"))],
            },
        })
        .collect()
}

#[test]
fn delegation_reviews_every_instruction_once_and_preserves_ordered_results() {
    let fixture = Fixture::new(&[]);
    let (mut run, _) = fixture.prepare(Protocol::Responses, "http://127.0.0.1:1");
    run.input.delegation_enabled = true;
    let events: Events = Arc::new(Mutex::new(VecDeque::new()));
    let output = events.clone();
    let (sender, incoming) = mpsc::sync_channel(8);
    let responder = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut reviews = 0;
        loop {
            assert!(Instant::now() < deadline);
            match output.lock().pop_front() {
                Some(Event::DelegateCapacity { reply, .. }) => reply.send(3).unwrap(),
                Some(Event::Prompt {
                    generation,
                    request,
                    ..
                }) => {
                    reviews += 1;
                    let PromptKind::Permission {
                        description,
                        options,
                        ..
                    } = &request.kind
                    else {
                        panic!("Expected delegation permission");
                    };
                    assert!(description.contains("3 planned children"));
                    for index in 0..3 {
                        assert!(description.contains(&format!(
                            "Instruction {index}: inspect exact scoped source"
                        )));
                        assert!(description.contains(&format!("src{index}")));
                    }
                    for bound in [
                        "64 model rounds",
                        "120 s",
                        "16 MiB",
                        "16 KiB",
                        "no commands, writes or recursive delegation",
                    ] {
                        assert!(description.contains(bound));
                    }
                    // A foreign permission cannot grant this round.
                    sender
                        .send(Request::Answer {
                            generation,
                            request_id: json!("foreign"),
                            answer: options[1].answer.clone(),
                        })
                        .unwrap();
                    sender
                        .send(Request::Answer {
                            generation,
                            request_id: request.request_id.clone(),
                            answer: options[1].answer.clone(),
                        })
                        .unwrap();
                }
                Some(Event::Delegate {
                    calls,
                    reply,
                    cancelled,
                    ..
                }) => {
                    assert_eq!(reviews, 1);
                    assert_eq!(
                        calls
                            .iter()
                            .map(|call| call.call_id.as_str())
                            .collect::<Vec<_>>(),
                        ["child-0", "child-1", "child-2"]
                    );
                    reply.send(calls.into_iter().map(|call|json!({"call_id":call.call_id,"status":"success","summary":"Observed child"})).collect()).unwrap();
                    break cancelled;
                }
                Some(_) | None => thread::sleep(Duration::from_millis(1)),
            }
        }
    });
    let stopped = AtomicBool::new(false);
    let cancelled = AtomicU64::new(0);
    let control = Control {
        stopped: &stopped,
        cancelled: &cancelled,
        generation: 7,
        deadline: Instant::now() + CALL_TIMEOUT,
        wire_alive: None,
        steering: None,
    };
    let results = delegation::execute_round(
        &run,
        &events,
        &control,
        &incoming,
        2,
        &requests(),
        || Ok(()),
    )
    .unwrap();
    assert_eq!(
        results
            .iter()
            .map(|value| value["call_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["child-0", "child-1", "child-2"]
    );
    assert!(responder.join().unwrap().load(Ordering::Acquire));
}

#[test]
fn cancelling_parent_wait_signals_owned_children() {
    let fixture = Fixture::new(&[]);
    let (mut run, _) = fixture.prepare(Protocol::Responses, "http://127.0.0.1:1");
    run.input.delegation_enabled = true;
    let events: Events = Arc::new(Mutex::new(VecDeque::new()));
    let output = events.clone();
    let (sender, incoming) = mpsc::sync_channel(8);
    let responder = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline);
            match output.lock().pop_front() {
                Some(Event::DelegateCapacity { reply, .. }) => reply.send(3).unwrap(),
                Some(Event::Prompt {
                    generation,
                    request,
                    ..
                }) => {
                    let PromptKind::Permission { options, .. } = &request.kind else {
                        panic!();
                    };
                    sender
                        .send(Request::Answer {
                            generation,
                            request_id: request.request_id.clone(),
                            answer: options[1].answer.clone(),
                        })
                        .unwrap();
                }
                Some(Event::Delegate {
                    cancelled, reply, ..
                }) => {
                    sender.send(Request::Cancel { generation: 7 }).unwrap();
                    while !cancelled.load(Ordering::Acquire) {
                        assert!(Instant::now() < deadline);
                        thread::sleep(Duration::from_millis(1));
                    }
                    drop(reply);
                    return;
                }
                Some(_) | None => thread::sleep(Duration::from_millis(1)),
            }
        }
    });
    let stopped = AtomicBool::new(false);
    let cancelled = AtomicU64::new(0);
    let control = Control {
        stopped: &stopped,
        cancelled: &cancelled,
        generation: 7,
        deadline: Instant::now() + CALL_TIMEOUT,
        wire_alive: None,
        steering: None,
    };
    assert!(
        delegation::execute_round(
            &run,
            &events,
            &control,
            &incoming,
            1,
            &requests(),
            || Ok(())
        )
        .unwrap_err()
        .contains("cancelled")
    );
    responder.join().unwrap();
}

#[test]
fn full_child_capacity_refuses_before_permission_review_or_dispatch() {
    let fixture = Fixture::new(&[]);
    let (mut run, _) = fixture.prepare(Protocol::Responses, "http://127.0.0.1:1");
    run.input.delegation_enabled = true;
    let events: Events = Arc::new(Mutex::new(VecDeque::new()));
    let output = events.clone();
    let (_sender, incoming) = mpsc::sync_channel(8);
    let responder = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline);
            if let Some(event) = output.lock().pop_front() {
                let Event::DelegateCapacity { reply, .. } = event else {
                    panic!("Full capacity must not present a permission or start children");
                };
                reply.send(0).unwrap();
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
    });
    let stopped = AtomicBool::new(false);
    let cancelled = AtomicU64::new(0);
    let control = Control {
        stopped: &stopped,
        cancelled: &cancelled,
        generation: 7,
        deadline: Instant::now() + CALL_TIMEOUT,
        wire_alive: None,
        steering: None,
    };
    let results = delegation::execute_round(
        &run,
        &events,
        &control,
        &incoming,
        1,
        &requests(),
        || Ok(()),
    )
    .unwrap();
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|result| result["status"] == "refused"));
    responder.join().unwrap();
    assert!(events.lock().is_empty());
}

#[test]
fn nonlead_delegation_has_no_review_dispatch_or_capacity_side_channel() {
    let fixture = Fixture::new(&[]);
    let (mut run, _) = fixture.prepare(Protocol::Responses, "http://127.0.0.1:1");
    run.input.delegation_enabled = false;
    let events: Events = Arc::new(Mutex::new(VecDeque::new()));
    let (_sender, incoming) = mpsc::sync_channel(8);
    let stopped = AtomicBool::new(false);
    let cancelled = AtomicU64::new(0);
    let control = Control {
        stopped: &stopped,
        cancelled: &cancelled,
        generation: 7,
        deadline: Instant::now() + CALL_TIMEOUT,
        wire_alive: None,
        steering: None,
    };
    let results =
        delegation::execute_round(&run, &events, &control, &incoming, 1, &requests(), || {
            panic!("Unadvertised delegation must not reach admission")
        })
        .unwrap();
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|result| result["status"] == "refused"));
    assert!(events.lock().is_empty());
}
