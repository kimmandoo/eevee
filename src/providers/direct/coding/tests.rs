use super::*;
use crate::providers::{Agent, ModelSelection};
use std::io::{Read, Write};
use std::net::TcpListener;

struct Fixture {
    root: PathBuf,
    workspace: Workspace,
    input: Input,
    store: Option<crate::store::Worker>,
}
impl Fixture {
    fn new(paths: &[&str]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "ev-native-writes-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("project")).unwrap();
        let root = root.canonicalize().unwrap();
        let workspace = Workspace {
            project: root.join("project"),
            data_dir: root.join("data"),
            exclusions: Vec::new(),
        };
        let store = crate::store::Worker::start(
            workspace.data_dir.clone(),
            workspace.project.to_str().unwrap().into(),
            crate::store::Policy::default(),
        )
        .unwrap();
        let next = || {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(event) = store.try_event() {
                    break event;
                }
                assert!(Instant::now() < deadline, "Store acknowledgement timed out");
                thread::sleep(Duration::from_millis(2));
            }
        };
        let crate::store::Event::Loaded(project) = next() else {
            panic!("Expected loaded store")
        };
        let owner = crate::store::Attribution::of(&project.partners[0]);
        let mut task = crate::store::Snapshot::empty(workspace.project.to_str().unwrap().into());
        task.goal = "Create the exact selected files without command execution".into();
        task.lead_partner = owner.partner;
        task.form = owner.form;
        task.revision = 1;
        task.partner_drafts
            .entry(owner.partner)
            .or_default()
            .drafts
            .push(crate::store::Draft {
                field: crate::store::EditorField::Request,
                text: "Keep this original frozen instruction".into(),
                cursor: 37,
            });
        let mut save = Some(task);
        assert!(store.try_save(&mut save));
        let crate::store::Event::Saved(task) = next() else {
            panic!("Expected saved task")
        };
        let claim = crate::store::WorkClaim {
            task_id: task.id,
            owner,
            request_id: "native-write-fixture".into(),
            writing: true,
        };
        let mut acquire = Some(crate::store::WorkRequest::Acquire {
            claim: claim.clone(),
            task: Box::new(task.clone()),
        });
        assert!(store.try_work(&mut acquire));
        let crate::store::Event::Work { result, .. } = next() else {
            panic!("Expected work claim")
        };
        result.unwrap();
        let input = Input {
            task,
            work: Some(claim),
            instruction_override: None,
            excerpts: Vec::new(),
            report: None,
            resume: None,
            agent: Agent::Compatible,
            operation: Operation::Implement,
            edit_paths: paths.iter().map(PathBuf::from).collect(),
            handoff: None,
            inspect_sessions: false,
            inspect_models: false,
            selection: ModelSelection::default(),
            conversation: None,
            credential: None,
        };
        Self {
            root,
            workspace,
            input,
            store: Some(store),
        }
    }
    fn prepare(&self, protocol: Protocol, url: &str) -> (Prepared, Connection) {
        // Existing test-only connection construction does not add a production
        // ChatGPT endpoint override or claim authenticated conformance.
        let connection = config::test_connection(protocol, url, Some("fixture-model"));
        let mut input = self.input.clone();
        input.conversation = Some(
            completed_conversation(
                &input,
                &connection.conversation_scope(),
                "Earlier public answer".into(),
            )
            .unwrap(),
        );
        let writes = capture_writes(&self.workspace, &input).unwrap();
        let payload = payload(&self.workspace, &input, &connection, writes.as_ref()).unwrap();
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
}
impl Drop for Fixture {
    fn drop(&mut self) {
        drop(self.store.take());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn frame(value: Value) -> String {
    format!("data: {value}\n\n")
}
fn wire(protocol: Protocol, calls: &[(&str, &str, &str)], text: &str) -> String {
    let args = |path: &str, content: &str| json!({"path":path,"content":content});
    match protocol {
        Protocol::Codex | Protocol::Responses => {
            let mut output = vec![
                json!({"type":"reasoning","id":"real-reasoning-item","encrypted_content":"opaque-signed-state"}),
            ];
            if !text.is_empty() {
                output.push(json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}));
            }
            output.extend(calls.iter().map(|(id,path,content)| json!({"type":"function_call","call_id":id,"name":super::super::super::writes::TOOL_NAME,"arguments":args(path,content).to_string()})));
            frame(
                json!({"type":"response.completed","response":{"status":"completed","model":"fixture-model","output":output,"usage":{"input_tokens":11,"output_tokens":3}}}),
            )
        }
        Protocol::ChatCompletions => {
            let tool_calls = calls.iter().enumerate().map(|(i,(id,path,content))| json!({"index":i,"id":id,"type":"function","function":{"name":super::super::super::writes::TOOL_NAME,"arguments":args(path,content).to_string()}})).collect::<Vec<_>>();
            let mut delta = json!({"content":text});
            if !calls.is_empty() {
                delta["tool_calls"] = json!(tool_calls);
            }
            frame(
                json!({"model":"fixture-model","choices":[{"index":0,"delta":delta,"finish_reason":if calls.is_empty(){"stop"}else{"tool_calls"}}]}),
            ) + &frame(json!({"choices":[],"usage":{"prompt_tokens":11,"completion_tokens":3}}))
                + "data: [DONE]\n\n"
        }
        Protocol::Messages => {
            let mut out = frame(
                json!({"type":"message_start","message":{"model":"fixture-model","usage":{"input_tokens":11}}}),
            );
            let mut index = 0;
            if !text.is_empty() {
                out += &frame(
                    json!({"type":"content_block_start","index":index,"content_block":{"type":"text","text":text}}),
                );
                out += &frame(json!({"type":"content_block_stop","index":index}));
                index += 1;
            }
            for (id, path, content) in calls {
                out += &frame(
                    json!({"type":"content_block_start","index":index,"content_block":{"type":"tool_use","id":id,"name":super::super::super::writes::TOOL_NAME,"input":{}}}),
                );
                out += &frame(
                    json!({"type":"content_block_delta","index":index,"delta":{"type":"input_json_delta","partial_json":args(path,content).to_string()}}),
                );
                out += &frame(json!({"type":"content_block_stop","index":index}));
                index += 1;
            }
            out += &frame(
                json!({"type":"message_delta","delta":{"stop_reason":if calls.is_empty(){"end_turn"}else{"tool_use"}},"usage":{"output_tokens":3}}),
            );
            out + &frame(json!({"type":"message_stop"}))
        }
        Protocol::Gemini => {
            let mut parts = Vec::new();
            if !text.is_empty() {
                parts.push(json!({"text":text}));
            }
            parts.extend(calls.iter().map(|(id,path,content)| json!({"functionCall":{"id":id,"name":super::super::super::writes::TOOL_NAME,"args":args(path,content)},"thoughtSignature":"opaque-signed-state"})));
            frame(
                json!({"modelVersion":"fixture-model","candidates":[{"index":0,"content":{"role":"model","parts":parts},"finishReason":"STOP"}]}),
            ) + &frame(json!({"usageMetadata":{"promptTokenCount":11,"candidatesTokenCount":3}}))
        }
    }
}

fn peer(responses: Vec<String>) -> (String, thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "Expected native HTTP round did not arrive"
                        );
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let (start, length) = loop {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&chunk[..n]);
                assert!(bytes.len() <= BODY_LIMIT + 32768);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let header = std::str::from_utf8(&bytes[..end]).unwrap();
                    let length = header
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    break (end + 4, length);
                }
            };
            while bytes.len() < start + length {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            requests.push(serde_json::from_slice(&bytes[start..start + length]).unwrap());
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",response.len()).unwrap();
            // Exercise the real UTF-8/SSE reader across narrow transport writes.
            for chunk in response.as_bytes().chunks(7) {
                socket.write_all(chunk).unwrap();
            }
        }
        requests
    });
    (url, server)
}

fn run_with_answers(
    fixture: &Fixture,
    run: &mut Prepared,
    connection: &Connection,
    allow: bool,
) -> ((Outcome, String), Vec<Event>) {
    run_with_handler(
        fixture,
        run,
        connection,
        move |generation, request, sender| {
            let PromptKind::Permission { options, .. } = &request.kind else {
                panic!("Expected exact write permission")
            };
            sender
                .send(Request::Answer {
                    generation,
                    request_id: request.request_id.clone(),
                    answer: options[usize::from(allow)].answer.clone(),
                })
                .unwrap();
        },
    )
}

fn run_with_handler(
    fixture: &Fixture,
    run: &mut Prepared,
    connection: &Connection,
    mut answer: impl FnMut(u64, &Prompt, &SyncSender<Request>) + Send + 'static,
) -> ((Outcome, String), Vec<Event>) {
    let events = Arc::new(Mutex::new(VecDeque::new()));
    let (sender, incoming) = mpsc::sync_channel(8);
    let done = Arc::new(AtomicBool::new(false));
    let output = events.clone();
    let stop = done.clone();
    let responder = thread::spawn(move || {
        let mut retained = Vec::new();
        loop {
            let next = output.lock().pop_front();
            if let Some(event) = next {
                if let Event::Prompt {
                    generation,
                    request,
                    ..
                } = &event
                {
                    answer(*generation, request, &sender);
                }
                retained.push(event);
            } else if stop.load(Ordering::Acquire) {
                break;
            } else {
                thread::sleep(Duration::from_millis(1));
            }
        }
        retained
    });
    let stopped = AtomicBool::new(false);
    let cancelled = AtomicU64::new(0);
    // Deliberately expired original HTTP deadline: approvals and subsequent
    // rounds must use their own deadlines, not this dispatch-time timestamp.
    let control = Control {
        stopped: &stopped,
        cancelled: &cancelled,
        generation: 7,
        deadline: Instant::now() - Duration::from_secs(1),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(async {
        let http = http_client().unwrap();
        inference_loop(
            &fixture.workspace,
            run,
            &events,
            &control,
            &incoming,
            &http,
            connection,
            &|_| Ok(()),
        )
        .await
    });
    done.store(true, Ordering::Release);
    (result, responder.join().unwrap())
}

#[test]
fn all_native_dialects_create_siblings_with_one_receipt_frozen_context_and_public_history() {
    for protocol in [
        Protocol::Codex,
        Protocol::Responses,
        Protocol::ChatCompletions,
        Protocol::Messages,
        Protocol::Gemini,
    ] {
        let fixture = Fixture::new(&["nested/empty.txt", "nested/日本語.txt"]);
        let (url, server) = peer(vec![
            wire(
                protocol,
                &[
                    ("call-empty", "nested/empty.txt", ""),
                    ("call-unicode", "nested/日本語.txt", "こんにちは\n"),
                ],
                "Preparing selected files",
            ),
            wire(protocol, &[], "Both reviewed writes settled"),
        ]);
        let (mut run, connection) = fixture.prepare(protocol, &url);
        let frozen = run.payload.clone();
        let prior = run.input.conversation.clone().unwrap();
        let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
        assert_eq!(outcome, Outcome::Completed, "{protocol:?}: {message}");
        assert_eq!(
            std::fs::read(fixture.workspace.project.join("nested/empty.txt")).unwrap(),
            b""
        );
        assert_eq!(
            std::fs::read_to_string(fixture.workspace.project.join("nested/日本語.txt")).unwrap(),
            "こんにちは\n"
        );
        assert_eq!(
            run.payload, frozen,
            "Initial reviewed bytes must not be consumed or regenerated"
        );
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        let initial: Value = serde_json::from_str(&frozen).unwrap();
        assert_eq!(requests[0], initial);
        let transcript = match protocol {
            Protocol::Codex | Protocol::Responses => "input",
            Protocol::Gemini => "contents",
            _ => "messages",
        };
        for (key, value) in initial.as_object().unwrap() {
            if key != transcript {
                assert_eq!(&requests[1][key], value, "Frozen {key} changed");
            }
        }
        let old = initial[transcript].as_array().unwrap();
        assert_eq!(
            &requests[1][transcript].as_array().unwrap()[..old.len()],
            old
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::Dispatched { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::Prompt { .. }))
                .count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::Journal { .. }))
                .count(),
            2
        );
        let canonical = events
            .iter()
            .find_map(|e| {
                if let Event::NativeCompleted { conversation, .. } = e {
                    Some(conversation)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(canonical.messages.len(), prior.messages.len() + 2);
        for (before, after) in prior.messages.iter().zip(canonical.messages.iter()) {
            assert!(Arc::ptr_eq(&before.text, &after.text));
        }
        assert_eq!(
            canonical.messages.last().unwrap().text.as_ref(),
            "Preparing selected files\n\nBoth reviewed writes settled"
        );
        assert!(!canonical.messages.iter().any(|m|m.text.contains("opaque-signed-state")||m.text.contains("function_call")));
        let mut usage = crate::workspace::usage::Record::new(
            "native-write-fixture".into(),
            fixture.input.task.id,
            fixture.input.work.as_ref().unwrap().owner.clone(),
            Agent::Compatible,
        );
        for report in events.iter().filter_map(|e| {
            if let Event::Usage { report, .. } = e {
                Some(report)
            } else {
                None
            }
        }) {
            assert!(report.identity.response.is_none());
            usage.apply_retained(report.clone()).unwrap();
            usage.apply_retained(report.clone()).unwrap();
        }
        assert_eq!(usage.measurements.len(), 2);
        assert_eq!(
            usage.measurements[0].report.identity.turn.as_deref(),
            Some("eevee-http-round:1")
        );
        assert_eq!(
            usage.measurements[1].report.identity.turn.as_deref(),
            Some("eevee-http-round:2")
        );
        assert_eq!(
            usage
                .measurements
                .iter()
                .map(|m| m.attributable.input.unwrap())
                .sum::<u64>(),
            22
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::LocalEffectsUnknown { .. }))
        );
    }
}

#[test]
fn denial_returns_real_tool_result_without_creating_reviewed_parents() {
    for protocol in [
        Protocol::Responses,
        Protocol::ChatCompletions,
        Protocol::Messages,
        Protocol::Gemini,
    ] {
        let fixture = Fixture::new(&["missing/denied.txt"]);
        let (url, server) = peer(vec![
            wire(protocol, &[("denied-call", "missing/denied.txt", "no")], ""),
            wire(protocol, &[], "The requested write was denied"),
        ]);
        let (mut run, connection) = fixture.prepare(protocol, &url);
        let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, false);
        assert_eq!(outcome, Outcome::Completed, "{message}");
        assert!(!fixture.workspace.project.join("missing").exists());
        let requests = server.join().unwrap();
        assert!(requests[1].to_string().contains("denied"));
        assert!(
            events
                .iter()
                .any(|e| matches!(e,Event::Journal{record,..} if record.text.contains("denied")))
        );
    }
}

#[test]
fn whole_call_set_and_late_terminal_failures_never_offer_partial_authority() {
    for protocol in [
        Protocol::Responses,
        Protocol::ChatCompletions,
        Protocol::Messages,
        Protocol::Gemini,
    ] {
        for late_error in [false, true] {
            let fixture = Fixture::new(&["selected.txt"]);
            let mut response = if late_error {
                wire(protocol, &[("valid", "selected.txt", "must not write")], "")
            } else {
                wire(
                    protocol,
                    &[
                        ("valid", "selected.txt", "must not write"),
                        ("foreign", "unselected.txt", "also refused"),
                    ],
                    "",
                )
            };
            if late_error {
                response += &frame(
                    json!({"error":{"message":"terminal failure after apparent completion"}}),
                );
            }
            let (url, server) = peer(vec![response]);
            let (mut run, connection) = fixture.prepare(protocol, &url);
            let ((outcome, _), events) = run_with_answers(&fixture, &mut run, &connection, true);
            assert_ne!(outcome, Outcome::Completed);
            assert_eq!(server.join().unwrap().len(), 1);
            assert!(!fixture.workspace.project.join("selected.txt").exists());
            assert!(!events.iter().any(|e| matches!(
                e,
                Event::Prompt { .. }
                    | Event::NativeCompleted { .. }
                    | Event::LocalEffectsUnknown { .. }
            )));
            assert!(events.iter().any(|e| matches!(e, Event::Usage { .. })));
        }
    }
}

#[test]
fn exact_permission_rejects_foreign_replayed_or_augmented_answers_and_closes_once() {
    let fixture = Fixture::new(&["new/file.txt"]);
    let (run, _) = fixture.prepare(Protocol::Responses, "https://example.invalid/v1");
    let proposal = run
        .writes
        .as_ref()
        .unwrap()
        .propose("new/file.txt".into(), "content".into())
        .unwrap();
    let events = Arc::new(Mutex::new(VecDeque::new()));
    let (sender, incoming) = mpsc::sync_channel(8);
    let output = events.clone();
    let responder = thread::spawn(move || {
        let request = loop {
            if let Some(Event::Prompt { request, .. }) = output.lock().pop_front() {
                break request;
            }
            thread::sleep(Duration::from_millis(1));
        };
        let PromptKind::Permission {
            options,
            description,
        } = &request.kind
        else {
            panic!("Permission expected")
        };
        assert!(description.starts_with("Create file\nPath: new/file.txt\n"));
        let allow = options[1].answer.clone();
        sender
            .send(Request::Answer {
                generation: 8,
                request_id: request.request_id.clone(),
                answer: allow.clone(),
            })
            .unwrap();
        sender
            .send(Request::Answer {
                generation: 7,
                request_id: json!("foreign"),
                answer: allow.clone(),
            })
            .unwrap();
        let mut augmented = allow.clone();
        augmented["extra"] = json!(true);
        sender
            .send(Request::Answer {
                generation: 7,
                request_id: request.request_id.clone(),
                answer: augmented,
            })
            .unwrap();
        let mut replay = allow;
        replay["permission"] = json!(0);
        sender
            .send(Request::Answer {
                generation: 7,
                request_id: request.request_id.clone(),
                answer: replay,
            })
            .unwrap();
        sender
            .send(Request::Answer {
                generation: 7,
                request_id: request.request_id.clone(),
                answer: options[0].answer.clone(),
            })
            .unwrap();
    });
    let stopped = AtomicBool::new(false);
    let cancelled = AtomicU64::new(0);
    let control = Control {
        stopped: &stopped,
        cancelled: &cancelled,
        generation: 7,
        deadline: Instant::now() - Duration::from_secs(1),
    };
    assert!(
        !permission(
            &run,
            &events,
            &control,
            &incoming,
            &proposal,
            1,
            0,
            Some("call"),
            Duration::from_secs(5)
        )
        .unwrap()
    );
    responder.join().unwrap();
    assert_eq!(
        events
            .lock()
            .iter()
            .filter(|e| matches!(e, Event::PromptClosed { .. }))
            .count(),
        1
    );
    assert!(!fixture.workspace.project.join("new").exists());
}

#[test]
fn permission_expiry_and_cancellation_never_create_missing_parents() {
    for cancel in [false, true] {
        let fixture = Fixture::new(&["missing/file.txt"]);
        let (run, _) = fixture.prepare(Protocol::Responses, "https://example.invalid/v1");
        let proposal = run
            .writes
            .as_ref()
            .unwrap()
            .propose("missing/file.txt".into(), "content".into())
            .unwrap();
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let (_sender, incoming) = mpsc::sync_channel(8);
        let stopped = AtomicBool::new(false);
        let cancelled = AtomicU64::new(if cancel { 7 } else { 0 });
        let control = Control {
            stopped: &stopped,
            cancelled: &cancelled,
            generation: 7,
            deadline: Instant::now() + CALL_TIMEOUT,
        };
        assert!(
            permission(
                &run,
                &events,
                &control,
                &incoming,
                &proposal,
                1,
                0,
                None,
                Duration::ZERO
            )
            .is_err()
        );
        assert_eq!(
            events
                .lock()
                .iter()
                .filter(|e| matches!(e, Event::PromptClosed { .. }))
                .count(),
            1
        );
        assert!(!fixture.workspace.project.join("missing").exists());
    }
}

#[test]
fn approved_create_rechecks_appeared_leaf_and_parent_before_any_effect() {
    for appeared_parent in [false, true] {
        let fixture = Fixture::new(&["new/file.txt"]);
        let (url, server) = peer(vec![wire(
            Protocol::Responses,
            &[("call", "new/file.txt", "replacement")],
            "",
        )]);
        let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
        let project = fixture.workspace.project.clone();
        let ((outcome, _), events) = run_with_handler(
            &fixture,
            &mut run,
            &connection,
            move |generation, request, sender| {
                std::fs::create_dir(project.join("new")).unwrap();
                if !appeared_parent {
                    std::fs::write(project.join("new/file.txt"), "external contents").unwrap();
                }
                let PromptKind::Permission { options, .. } = &request.kind else {
                    panic!("Permission expected")
                };
                sender
                    .send(Request::Answer {
                        generation,
                        request_id: request.request_id.clone(),
                        answer: options[1].answer.clone(),
                    })
                    .unwrap();
            },
        );
        assert_eq!(outcome, Outcome::Failed);
        assert_eq!(server.join().unwrap().len(), 1);
        if appeared_parent {
            assert!(!fixture.workspace.project.join("new/file.txt").exists());
        } else {
            assert_eq!(
                std::fs::read_to_string(fixture.workspace.project.join("new/file.txt")).unwrap(),
                "external contents"
            );
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e,Event::Journal{record,..} if record.text.contains("refused")))
        );
        assert!(!events.iter().any(|e| matches!(
            e,
            Event::LocalEffectsUnknown { .. } | Event::NativeCompleted { .. }
        )));
    }
}

#[test]
fn cancel_during_permission_closes_prompt_without_write_or_continuation() {
    let fixture = Fixture::new(&["new/file.txt"]);
    let (url, server) = peer(vec![wire(
        Protocol::Responses,
        &[("call", "new/file.txt", "content")],
        "",
    )]);
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, _), events) =
        run_with_handler(&fixture, &mut run, &connection, |generation, _, sender| {
            sender.send(Request::Cancel { generation }).unwrap();
        });
    assert_eq!(outcome, Outcome::Cancelled);
    assert_eq!(server.join().unwrap().len(), 1);
    assert!(!fixture.workspace.project.join("new").exists());
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::PromptClosed { .. }))
            .count(),
        1
    );
    assert!(!events.iter().any(|e| matches!(
        e,
        Event::LocalEffectsUnknown { .. } | Event::NativeCompleted { .. }
    )));
}

#[test]
fn remote_interruption_after_settled_write_does_not_invent_unknown_local_effects() {
    let fixture = Fixture::new(&["settled.txt"]);
    let incomplete =
        frame(json!({"type":"response.output_text.delta","delta":"Interrupted public output"}));
    let (url, server) = peer(vec![
        wire(
            Protocol::Responses,
            &[("call", "settled.txt", "known bytes")],
            "",
        ),
        incomplete,
    ]);
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, _), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Unknown);
    assert_eq!(server.join().unwrap().len(), 2);
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.project.join("settled.txt")).unwrap(),
        "known bytes"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e,Event::Journal{record,..} if record.text.contains("success")))
    );
    assert!(!events.iter().any(|e| matches!(
        e,
        Event::LocalEffectsUnknown { .. } | Event::NativeCompleted { .. }
    )));
}

#[test]
fn replayed_provider_call_cannot_authorize_another_write_in_later_round() {
    let fixture = Fixture::new(&["selected.txt"]);
    let (url, server) = peer(vec![
        wire(
            Protocol::Responses,
            &[("same-call", "selected.txt", "first")],
            "",
        ),
        wire(
            Protocol::Responses,
            &[("same-call", "selected.txt", "replayed")],
            "",
        ),
    ]);
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, _), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Failed);
    assert_eq!(server.join().unwrap().len(), 2);
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.project.join("selected.txt")).unwrap(),
        "first"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Prompt { .. }))
            .count(),
        1
    );
}

#[test]
fn eight_round_limit_refuses_the_terminal_call_set_without_executing_it() {
    let fixture = Fixture::new(&["selected.txt"]);
    let responses = (1..=ROUND_LIMIT)
        .map(|round| {
            wire(
                Protocol::Responses,
                &[(
                    format!("call-{round}").as_str(),
                    "selected.txt",
                    format!("round-{round}").as_str(),
                )],
                "",
            )
        })
        .collect();
    let (url, server) = peer(responses);
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, _), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Failed);
    assert_eq!(server.join().unwrap().len(), ROUND_LIMIT);
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.project.join("selected.txt")).unwrap(),
        "round-7"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Prompt { .. }))
            .count(),
        7
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Dispatched { .. }))
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::NativeCompleted { .. }))
    );
}

#[test]
fn sixteen_call_limit_is_aggregate_across_rounds_not_per_response() {
    let names = (0..8)
        .map(|index| format!("files/{index}.txt"))
        .collect::<Vec<_>>();
    let fixture = Fixture::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
    let first = (0..8)
        .map(|index| {
            (
                format!("first-{index}"),
                names[index].clone(),
                "initial".to_string(),
            )
        })
        .collect::<Vec<_>>();
    let second = (0..8)
        .map(|index| {
            (
                format!("second-{index}"),
                names[index].clone(),
                "replacement".to_string(),
            )
        })
        .collect::<Vec<_>>();
    fn refs(calls: &[(String, String, String)]) -> Vec<(&str, &str, &str)> {
        calls
            .iter()
            .map(|(id, path, content)| (id.as_str(), path.as_str(), content.as_str()))
            .collect()
    }
    let (url, server) = peer(vec![
        wire(Protocol::Responses, &refs(&first), ""),
        wire(Protocol::Responses, &refs(&second), ""),
        wire(
            Protocol::Responses,
            &[("seventeenth", &names[0], "must not apply")],
            "",
        ),
    ]);
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, _), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Failed);
    assert_eq!(server.join().unwrap().len(), 3);
    for name in names {
        assert_eq!(
            std::fs::read_to_string(fixture.workspace.project.join(name)).unwrap(),
            "replacement"
        );
    }
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Prompt { .. }))
            .count(),
        TOOL_CALL_LIMIT
    );
}

#[test]
fn metadata_plan_review_and_unscoped_implement_never_advertise_or_execute_tools() {
    let fixture = Fixture::new(&[]);
    for (operation, metadata) in [
        (Operation::Plan, false),
        (Operation::Implement, false),
        (Operation::Review, false),
        (Operation::Implement, true),
    ] {
        let mut input = fixture.input.clone();
        input.operation = operation;
        input.inspect_models = metadata;
        input.work = None;
        if metadata {
            let connection = config::test_connection(
                Protocol::Responses,
                "https://example.invalid/v1",
                Some("fixture-model"),
            );
            assert!(
                capture_writes(&fixture.workspace, &input)
                    .unwrap()
                    .is_none()
            );
            let body: Value = serde_json::from_str(
                &payload(&fixture.workspace, &input, &connection, None).unwrap(),
            )
            .unwrap();
            assert!(body.get("tools").is_none());
            continue;
        }
        // These inputs deliberately use no claimed write authority. The
        // serialization primitive and parser independently deny tool capability.
        assert!(!native_writing(&input));
        for protocol in [
            Protocol::Codex,
            Protocol::Responses,
            Protocol::ChatCompletions,
            Protocol::Messages,
            Protocol::Gemini,
        ] {
            let body = serde_json::to_value(
                inference_body(protocol, "fixture-model", None, "No tools", None, "request")
                    .unwrap(),
            )
            .unwrap();
            assert!(body.get("tools").is_none());
            let mut stream = Stream::new(protocol);
            let mut framing = Sse::default();
            for (event, data) in framing
                .feed(wire(protocol, &[("foreign", "selected.txt", "no")], "").as_bytes())
                .unwrap()
            {
                stream.event(&event, &data).unwrap();
            }
            assert!(stream.complete);
            assert!(stream.calls.finish(false).is_err());
        }
    }
}

#[test]
fn aggregate_http_body_budget_refuses_continuation_without_truncating_frozen_history() {
    let fixture = Fixture::new(&["selected.txt"]);
    let (url, server) = peer(vec![wire(
        Protocol::Responses,
        &[("call", "selected.txt", "known write")],
        "",
    )]);
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let owner = run.input.conversation.as_ref().unwrap().owner.clone();
    let messages = vec![
        ConversationMessage {
            role: ConversationRole::User,
            text: "earlier one".into(),
        },
        ConversationMessage {
            role: ConversationRole::Assistant,
            text: "x".repeat(300_000).into(),
        },
        ConversationMessage {
            role: ConversationRole::User,
            text: "earlier two".into(),
        },
        ConversationMessage {
            role: ConversationRole::Assistant,
            text: "y".repeat(300_000).into(),
        },
    ];
    run.input.conversation = Some(Arc::new(NativeConversation {
        owner,
        messages: messages.into(),
    }));
    run.payload = payload(
        &fixture.workspace,
        &run.input,
        &connection,
        run.writes.as_ref(),
    )
    .unwrap();
    let frozen = run.payload.clone();
    let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Failed);
    assert!(message.contains("aggregate HTTP bodies"));
    assert_eq!(server.join().unwrap().len(), 1);
    assert_eq!(run.payload, frozen);
    assert_eq!(run.input.conversation.as_ref().unwrap().messages.len(), 4);
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.project.join("selected.txt")).unwrap(),
        "known write"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Dispatched { .. }))
            .count(),
        1
    );
    assert!(!events.iter().any(|e| matches!(
        e,
        Event::LocalEffectsUnknown { .. } | Event::NativeCompleted { .. }
    )));
}

#[test]
fn aggregate_public_output_budget_spans_tool_rounds_and_never_seeds_partial_history() {
    let fixture = Fixture::new(&["selected.txt"]);
    let text = "bounded public text ".repeat(16_000);
    let (url, server) = peer(vec![
        wire(
            Protocol::Responses,
            &[("call", "selected.txt", "known write")],
            &text,
        ),
        wire(Protocol::Responses, &[], &text),
    ]);
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Unknown);
    assert!(message.contains("512 KiB"));
    assert_eq!(server.join().unwrap().len(), 2);
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.project.join("selected.txt")).unwrap(),
        "known write"
    );
    assert!(!events.iter().any(|e| matches!(
        e,
        Event::LocalEffectsUnknown { .. } | Event::NativeCompleted { .. }
    )));
}

#[test]
fn filesystem_outcome_consumer_distinguishes_refusal_from_explicit_uncertainty() {
    let fixture = Fixture::new(&["selected.txt"]);
    let (mut run, _) = fixture.prepare(Protocol::Responses, "https://example.invalid/v1");
    let proposal = run
        .writes
        .as_ref()
        .unwrap()
        .propose("selected.txt".into(), "new bytes".into())
        .unwrap();
    std::fs::write(
        fixture.workspace.project.join("selected.txt"),
        "appeared external bytes",
    )
    .unwrap();
    let failure = run.writes.as_mut().unwrap().apply(&proposal).unwrap_err();
    assert!(!failure.uncertain);
    let events = Arc::new(Mutex::new(VecDeque::new()));
    let result = failed_write(&run, &events, 1, 0, &proposal, failure).unwrap();
    assert_eq!(result["status"], "refused");
    assert!(
        !events
            .lock()
            .iter()
            .any(|event| matches!(event, Event::LocalEffectsUnknown { .. }))
    );
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.project.join("selected.txt")).unwrap(),
        "appeared external bytes"
    );

    // A typed filesystem-boundary fixture exercises event/journal ordering;
    // this is not a simulated successful write or a command-verification claim.
    let failure = crate::providers::writes::WriteFailure {
        message: "Write began but resulting identity could not be established".into(),
        uncertain: true,
    };
    assert!(failed_write(&run, &events, 1, 0, &proposal, failure).is_err());
    finish(
        &events,
        run.generation,
        run.input.task.id,
        Outcome::Failed,
        "Filesystem outcome unresolved",
    );
    let queue = events.lock();
    let unknown=queue.iter().position(|event|matches!(event,Event::LocalEffectsUnknown{generation:7,task_id,..} if *task_id==run.input.task.id)).unwrap();
    let journal = queue
        .iter()
        .position(|event| matches!(event, Event::Journal { .. }))
        .unwrap();
    let terminal = queue
        .iter()
        .position(|event| matches!(event, Event::Finished { .. }))
        .unwrap();
    assert!(unknown < journal && journal < terminal);
}

#[test]
fn saturated_ordinary_event_capacity_cannot_hide_local_uncertainty_or_terminal_settlement() {
    let fixture = Fixture::new(&["selected.txt"]);
    let (run, _) = fixture.prepare(Protocol::Responses, "https://example.invalid/v1");
    let proposal = run
        .writes
        .as_ref()
        .unwrap()
        .propose("selected.txt".into(), "bytes".into())
        .unwrap();
    let events = Arc::new(Mutex::new(VecDeque::new()));
    for _ in 0..QUEUE_LIMIT - 4 {
        emit(
            &events,
            Event::Error {
                generation: Some(7),
                message: "Retained ordinary event".into(),
            },
        )
        .unwrap();
    }
    assert!(
        failed_write(
            &run,
            &events,
            1,
            0,
            &proposal,
            crate::providers::writes::WriteFailure {
                message: "Explicit unresolved local filesystem outcome".into(),
                uncertain: true,
            }
        )
        .is_err()
    );
    finish(
        &events,
        7,
        run.input.task.id,
        Outcome::Failed,
        "Local effects unresolved",
    );
    let queue = events.lock();
    assert_eq!(
        queue
            .iter()
            .filter(|event| matches!(event, Event::LocalEffectsUnknown { .. }))
            .count(),
        1
    );
    assert_eq!(
        queue
            .iter()
            .filter(|event| matches!(event, Event::Journal { .. }))
            .count(),
        1
    );
    assert!(matches!(queue.back(), Some(Event::Finished { .. })));
}
