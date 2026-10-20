use super::*;
use crate::providers::{Agent, ModelSelection};

fn input() -> Input {
    Input {
        task: crate::store::Snapshot::empty("unopened-project".into()),
        work: None,
        excerpts: Vec::new(),
        report: None,
        resume: None,
        agent: Agent::Compatible,
        operation: Operation::Plan,
        edit_paths: Vec::new(),
        handoff: None,
        inspect_sessions: false,
        inspect_models: true,
        selection: ModelSelection::default(),
        credential: None,
    }
}
fn workspace() -> Workspace {
    Workspace {
        project: "unopened-project".into(),
        data_dir: "unopened-data".into(),
        exclusions: Vec::new(),
    }
}
fn prepared() -> Prepared {
    Prepared {
        generation: 7,
        input: Box::new(input()),
        payload: String::new(),
        fingerprint: String::new(),
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn control<'a>(stopped: &'a AtomicBool, cancelled: &'a AtomicU64) -> Control<'a> {
    Control {
        stopped,
        cancelled,
        generation: 7,
        deadline: Instant::now() + Duration::from_secs(10),
    }
}
fn frame(value: Value) -> String {
    format!("data: {value}\r\n\r\n")
}

#[test]
fn protocol_sse_chunks_publish_only_user_facing_text_and_confirmed_model() {
    let responses = [
        json!({"type":"response.created","response":{"model":"confirmed"}}),
        json!({"type":"response.reasoning_text.delta","delta":"private thought"}),
        json!({"type":"response.output_text.delta","delta":"Hello "}),
        json!({"type":"response.output_text.delta","delta":"日本語"}),
        json!({"type":"response.completed","response":{"model":"confirmed","status":"completed"}}),
    ]
    .into_iter()
    .map(frame)
    .collect::<String>();
    let chat = [
        json!({"model":"confirmed","choices":[{"index":0,"delta":{"reasoning_content":"private thought","content":"Hello "},"finish_reason":null}]}),
        json!({"model":"confirmed","choices":[{"index":0,"delta":{"content":"日本語"},"finish_reason":"stop"}]}),
    ].into_iter().map(frame).collect::<String>() + "data: [DONE]\n\n";
    let anthropic = [
        json!({"type":"message_start","message":{"model":"confirmed"}}),
        json!({"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"private thought"}}),
        json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello 日本語"}}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
        json!({"type":"message_stop"}),
    ].into_iter().map(frame).collect::<String>();
    let gemini = frame(
        json!({"modelVersion":"confirmed","candidates":[{"index":0,"content":{"parts":[{"text":"private thought","thought":true},{"text":"Hello 日本語"}]},"finishReason":"STOP"}]}),
    );
    for (protocol, wire) in [
        (Protocol::Codex, responses.clone()),
        (Protocol::Responses, responses),
        (Protocol::ChatCompletions, chat),
        (Protocol::Messages, anthropic),
        (Protocol::Gemini, gemini),
    ] {
        for chunk_size in [1, 3, 27, 4096] {
            let mut parser = Sse::default();
            let mut stream = Stream::new(protocol);
            let run = prepared();
            let events = Arc::new(Mutex::new(VecDeque::new()));
            stream.publish(&events, &run).unwrap();
            assert!(
                events.lock().is_empty(),
                "no acknowledgement masquerades as observed model"
            );
            for chunk in wire.as_bytes().chunks(chunk_size) {
                for (kind, data) in parser.feed(chunk).unwrap() {
                    stream.event(&kind, &data).unwrap();
                    stream.publish(&events, &run).unwrap();
                }
            }
            assert!(stream.complete);
            stream.publish_final(&events, &run).unwrap();
            let queue = events.lock();
            assert!(queue.iter().any(
                |event| matches!(event, Event::ModelObserved { model, .. } if model == "confirmed")
            ));
            assert!(
                !queue
                    .iter()
                    .any(|event| matches!(event, Event::Thread { .. }))
            );
            assert!(queue.iter().any(
                |e| matches!(e, Event::Item { text, complete: true, .. } if text == "Hello 日本語")
            ));
            assert!(!queue.iter().any(
                |e| matches!(e, Event::Item { text, .. } if text.contains("private thought"))
            ));
        }
    }
}

#[test]
fn native_responses_keep_distinct_cross_request_item_ownership() {
    let events = Arc::new(Mutex::new(VecDeque::new()));
    let first = prepared();
    let mut second = prepared();
    second.generation += 1;
    let mut stream = Stream::new(Protocol::Responses);
    stream.append("First answer").unwrap();
    stream.publish_final(&events, &first).unwrap();
    stream.text.clear();
    stream.append("Second answer").unwrap();
    stream.publish_final(&events, &second).unwrap();
    let items: Vec<_> = events
        .lock()
        .iter()
        .filter_map(|event| match event {
            Event::Item {
                thread_id,
                turn_id,
                item_id,
                text,
                ..
            } => Some((
                (thread_id.clone(), turn_id.clone(), item_id.clone()),
                text.clone(),
            )),
            _ => None,
        })
        .collect();
    assert_ne!(
        (&items[0].0.0, &items[0].0.1),
        (&items[1].0.0, &items[1].0.1)
    );
    assert_eq!(items[0].1, "First answer");
    assert_eq!(items[1].1, "Second answer");
}

#[test]
fn responses_refusals_and_authoritative_final_tails_are_visible_once() {
    let mut stream = Stream::new(Protocol::Responses);
    stream
        .event(
            "",
            &json!({"type":"response.refusal.delta","delta":"Cannot help "}).to_string(),
        )
        .unwrap();
    assert_eq!(stream.text, "Cannot help ");
    stream.event("", &json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","content":[{"type":"refusal","refusal":"Cannot help with that request."}]}]}}).to_string()).unwrap();
    assert!(stream.complete);
    assert_eq!(stream.text, "Cannot help with that request.");
    let mut final_only = Stream::new(Protocol::Responses);
    final_only.event("", &json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","content":[{"type":"refusal","refusal":"Request refused."}]}]}}).to_string()).unwrap();
    assert_eq!(final_only.text, "Request refused.");
}

#[test]
fn missing_observed_model_remains_missing_and_final_response_text_is_supported() {
    let mut stream = Stream::new(Protocol::Responses);
    stream
        .event(
            "",
            &json!({"type":"response.completed","response":{"status":"completed","output":[
                {"type":"reasoning","content":[{"type":"output_text","text":"private thought"}]},
                {"type":"message","content":[{"type":"output_text","text":"Final answer"}]}
            ]}})
            .to_string(),
        )
        .unwrap();
    let events = Arc::new(Mutex::new(VecDeque::new()));
    stream.publish_final(&events, &prepared()).unwrap();
    assert!(stream.model.is_none());
    assert!(
        !events
            .lock()
            .iter()
            .any(|e| matches!(e, Event::ModelObserved { .. } | Event::Thread { .. }))
    );
    assert!(
        events.lock().iter().any(
            |e| matches!(e, Event::Item { text, complete: true, .. } if text == "Final answer")
        )
    );
}

#[test]
fn protocol_errors_and_non_success_finishes_never_become_completed() {
    for (protocol, data) in [
        (
            Protocol::Responses,
            json!({"type":"response.failed","response":{"error":{"message":"private provider diagnostic"}}}),
        ),
        (Protocol::Codex, json!({"type":"response.incomplete"})),
        (
            Protocol::Messages,
            json!({"type":"error","error":{"message":"private provider diagnostic"}}),
        ),
        (
            Protocol::Messages,
            json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"}}),
        ),
        (
            Protocol::Gemini,
            json!({"candidates":[{"finishReason":"SAFETY"}]}),
        ),
        (
            Protocol::ChatCompletions,
            json!({"choices":[{"delta":{},"finish_reason":"length"}]}),
        ),
    ] {
        let mut stream = Stream::new(protocol);
        let error = stream.event("", &data.to_string()).unwrap_err();
        assert!(stream.failed);
        assert!(!stream.complete);
        assert!(!error.contains("private provider diagnostic"));
    }
    let mut stream = Stream::new(Protocol::ChatCompletions);
    assert!(stream.event("", "[DONE]").is_err());
    assert!(!stream.complete);
    assert!(stream.event("", "not JSON").is_err());
    stream.observe(Some("first")).unwrap();
    assert!(stream.observe(Some("second")).is_err());
}

#[test]
fn exact_protocol_bodies_preserve_custom_model_and_do_not_grant_tools() {
    for protocol in [
        Protocol::Codex,
        Protocol::Responses,
        Protocol::Messages,
        Protocol::Gemini,
        Protocol::ChatCompletions,
    ] {
        let body =
            inference_body(protocol, "custom/explicit-model", None, "Reviewed text").unwrap();
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
        assert!(body.get("reasoning").is_none());
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("thinking").is_none());
        if protocol != Protocol::Gemini {
            assert_eq!(body["model"], "custom/explicit-model");
        }
        match protocol {
            Protocol::Codex | Protocol::Responses => {
                assert_eq!(body["input"][0]["content"][0]["text"], "Reviewed text")
            }
            Protocol::Messages | Protocol::ChatCompletions => {
                assert_eq!(body["messages"][0]["content"], "Reviewed text")
            }
            Protocol::Gemini => {
                assert_eq!(body["contents"][0]["parts"][0]["text"], "Reviewed text")
            }
        }
    }
    assert_eq!(
        inference_body(Protocol::Codex, "custom", Some("max"), "x").unwrap()["reasoning"]["effort"],
        "max"
    );
    assert_eq!(
        inference_body(Protocol::ChatCompletions, "custom", Some("low"), "x").unwrap()["reasoning_effort"],
        "low"
    );
    for protocol in [Protocol::Messages, Protocol::Gemini] {
        assert!(inference_body(protocol, "custom", Some("high"), "x").is_err());
    }
}

#[test]
fn complete_catalogues_keep_configured_and_custom_models_without_generation_filters() {
    let stopped = AtomicBool::new(false);
    let cancelled = AtomicU64::new(0);
    let control = control(&stopped, &cancelled);
    let runtime = runtime();
    for protocol in [
        Protocol::Codex,
        Protocol::Responses,
        Protocol::ChatCompletions,
        Protocol::Messages,
        Protocol::Gemini,
    ] {
        let connection = config::test_connection(
            protocol,
            "https://example.invalid/v1",
            Some("configured-outside-list"),
        );
        let mut cursors = Vec::new();
        let mut pages = VecDeque::from(match protocol {
            Protocol::Messages => vec![
                json!({"data":[{"id":"older-model"}],"has_more":true,"last_id":"after-one"}),
                json!({"data":[{"id":"future-model","display_name":null}],"has_more":false}),
            ],
            Protocol::Gemini => vec![
                json!({"models":[{"name":"models/older-model"}],"nextPageToken":"after-one"}),
                json!({"models":[{"name":"models/future-model"}]}),
            ],
            Protocol::Codex => vec![
                json!({"models":[{"slug":"older-model"},{"slug":"future-model","description":null}]}),
            ],
            Protocol::Responses => vec![json!([{"id":"older-model"},{"id":"future-model"}])],
            Protocol::ChatCompletions => {
                vec![json!({"data":[{"id":"older-model"},{"id":"future-model"}]})]
            }
        });
        let catalogue = runtime
            .block_on(discover_pages(
                &connection,
                Some("deliberate-custom"),
                &control,
                |cursor| {
                    cursors.push(cursor);
                    std::future::ready(Ok(pages.pop_front().unwrap()))
                },
            ))
            .unwrap();
        assert_eq!(catalogue.models.len(), 4);
        assert_eq!(
            catalogue.current.as_deref(),
            Some("configured-outside-list")
        );
        assert!(catalogue.models.iter().any(|m| m.id == "deliberate-custom"));
        assert!(
            catalogue
                .models
                .iter()
                .all(|m| m.efforts.is_empty() && m.default_effort.is_none())
        );
        catalogue.validate().unwrap();
        if matches!(protocol, Protocol::Messages | Protocol::Gemini) {
            assert_eq!(cursors, vec![None, Some("after-one".into())]);
        }
    }
}

#[test]
fn catalogue_bounds_cycles_and_metadata_failures_are_errors_not_fallbacks() {
    let stopped = AtomicBool::new(false);
    let cancelled = AtomicU64::new(0);
    let control = control(&stopped, &cancelled);
    let connection = config::test_connection(Protocol::Gemini, "https://example.invalid", None);
    let runtime = runtime();
    let error = runtime
        .block_on(discover_pages(&connection, None, &control, |_| {
            std::future::ready(Ok(json!({"models":[],"nextPageToken":"cycle"})))
        }))
        .unwrap_err();
    assert!(error.contains("repeated"));
    let mut index = 0;
    let error = runtime
        .block_on(discover_pages(&connection, None, &control, |_| {
            index += 1;
            std::future::ready(Ok(
                json!({"models":[],"nextPageToken":format!("page-{index}")}),
            ))
        }))
        .unwrap_err();
    assert!(error.contains("pagination bound"));
    let connection = config::test_connection(Protocol::Responses, "https://example.invalid", None);
    let entries: Vec<Value> = (0..4096)
        .map(|i| json!({"id":format!("model-{i}")}))
        .collect();
    let catalogue = runtime
        .block_on(discover_pages(&connection, None, &control, |_| {
            std::future::ready(Ok(json!({"data":entries})))
        }))
        .unwrap();
    assert_eq!(catalogue.models.len(), 4096);
    let entries: Vec<Value> = (0..4097)
        .map(|i| json!({"id":format!("model-{i}")}))
        .collect();
    assert!(
        runtime
            .block_on(discover_pages(&connection, None, &control, |_| {
                std::future::ready(Ok(json!({"data":entries})))
            }))
            .is_err()
    );
    assert!(
        runtime
            .block_on(discover_pages(&connection, None, &control, |_| {
                std::future::ready(Err("HTTP 503 metadata unavailable".into()))
            }))
            .unwrap_err()
            .contains("503")
    );
    assert!(
        runtime
            .block_on(discover_pages(&connection, None, &control, |_| {
                std::future::ready(Ok(json!({"unrelated":[]})))
            }))
            .is_err()
    );
    assert!(catalogue_page(&json!({"data":[],"has_more":true}), Protocol::Messages).is_err());
}

#[test]
fn discovery_excludes_known_non_generating_models_without_losing_valid_rows() {
    let stopped = AtomicBool::new(false);
    let cancelled = AtomicU64::new(0);
    let control = control(&stopped, &cancelled);
    let connection =
        config::test_connection(Protocol::Gemini, "https://example.invalid/v1beta", None);
    let catalogue = runtime()
        .block_on(discover_pages(&connection, None, &control, |_| {
            std::future::ready(Ok(json!({"models":[
                {"name":"models/chat","supportedGenerationMethods":["generateContent"]},
                {"name":"models/embedding","supportedGenerationMethods":["embedContent"]},
                {"name":"models/image","input_modalities":["image"]},
                {"name":"models/hidden","hidden":true},
                {"displayName":"Missing request identity"}
            ]})))
        }))
        .unwrap();
    assert_eq!(
        catalogue
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["models/chat"]
    );
}

#[test]
fn optional_model_metadata_is_tolerant_but_identities_remain_exact() {
    let option = model_option(
        &json!({"slug":"custom-request","display_name":"","description":17}),
        Protocol::Codex,
    )
    .unwrap();
    assert_eq!(option.id, "custom-request");
    assert_eq!(option.name, "custom-request");
    assert!(option.efforts.is_empty());
    assert!(option.default_effort.is_none());
    let option = model_option(&json!({"slug":"reasoning","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}],"default_reasoning_level":"high"}), Protocol::Codex).unwrap();
    assert_eq!(option.efforts, ["low", "high"]);
    assert_eq!(option.default_effort.as_deref(), Some("high"));
    assert!(model_option(&json!({"id":"unsafe\u{001b}"}), Protocol::Responses).is_err());
}

#[test]
fn explicit_selection_and_dispatch_revalidation_never_guess_a_fallback() {
    let mut connection = config::test_connection(
        Protocol::Responses,
        "https://example.invalid/v1",
        Some("configured-custom"),
    );
    let mut input = input();
    assert_eq!(
        selected_model(&input, &connection).unwrap(),
        "configured-custom"
    );
    input.selection.model = Some("deliberate-unlisted".into());
    assert_eq!(
        selected_model(&input, &connection).unwrap(),
        "deliberate-unlisted"
    );
    let workspace = workspace();
    let body = payload(&workspace, &input, &connection).unwrap();
    assert!(!body.contains("approved_memories"));
    let mut run = Prepared {
        generation: 7,
        input: Box::new(input),
        payload: body,
        fingerprint: connection.fingerprint(),
    };
    validate_prepared(&workspace, &run, &connection).unwrap();
    run.input.selection.model = Some("changed-selection".into());
    assert!(
        validate_prepared(&workspace, &run, &connection)
            .unwrap_err()
            .contains("HTTP body changed")
    );
    run.input.selection.model = Some("deliberate-unlisted".into());
    connection.model = Some("changed-configured-model".into());
    assert!(
        validate_prepared(&workspace, &run, &connection)
            .unwrap_err()
            .contains("connection/authentication changed")
    );
    connection.model = None;
    run.input.selection.model = None;
    assert!(selected_model(&run.input, &connection).is_err());
    run.input.resume = Some("invented-session".into());
    assert!(validate_input(&workspace, &run.input).is_err());
}

#[test]
fn cumulative_output_is_redacted_escaped_and_event_coalescing_is_bounded() {
    let run = prepared();
    let events = Arc::new(Mutex::new(VecDeque::new()));
    let mut stream = Stream::new(Protocol::Responses);
    for text in [
        "Safe ",
        "password=",
        "opaque-value",
        " \u{001b}[31m",
        "visible ",
    ] {
        stream.append(text).unwrap();
        stream.publish(&events, &run).unwrap();
        for event in events.lock().iter() {
            if let Event::Item { text, .. } = event {
                assert!(!text.contains("opaque-value"));
                assert!(!text.contains('\u{001b}'));
            }
        }
    }
    for _ in 0..1000 {
        stream.append("word ").unwrap();
        stream.publish(&events, &run).unwrap();
    }
    assert_eq!(events.lock().len(), 1);
    stream.publish_final(&events, &run).unwrap();
    assert_eq!(events.lock().len(), 1);
    assert!(stream.append(&"x".repeat(OUTPUT_LIMIT)).is_err());
    let mut parser = Sse::default();
    assert!(parser.feed(&vec![b'x'; FRAME_LIMIT + 1]).is_err());
    assert!(Sse::default().feed(b"data: \xff\n\n").is_err());
}

#[test]
fn waiting_for_headers_or_chunks_is_cancellable_and_deadlines_are_not_retries() {
    let runtime = runtime();
    let stopped = AtomicBool::new(false);
    let cancelled = AtomicU64::new(0);
    let mut control = control(&stopped, &cancelled);
    control.deadline = Instant::now() + Duration::from_millis(35);
    let start = Instant::now();
    let result = runtime.block_on(wait_http(
        std::future::pending::<Result<(), reqwest::Error>>(),
        &control,
    ));
    assert!(result.unwrap_err().contains("deadline"));
    assert!(start.elapsed() < Duration::from_secs(1));
    cancelled.store(7, Ordering::Release);
    let control = Control {
        deadline: Instant::now() + Duration::from_secs(10),
        ..control
    };
    let result = runtime.block_on(wait_http(
        std::future::pending::<Result<(), reqwest::Error>>(),
        &control,
    ));
    assert!(result.unwrap_err().contains("cancelled locally"));
    assert!(http_status(401).contains("credentials"));
    assert!(http_status(429).contains("quota"));
    assert!(http_status(404).contains("exact model"));
    assert!(http_status(302).contains("redirect refused"));
}

fn event(worker: &Worker) -> Event {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(event) = worker.try_event() {
            return event;
        }
        assert!(Instant::now() < deadline, "worker event deadline");
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn worker_rejects_unreviewed_runs_and_cancels_only_current_preview() {
    let worker = Worker::start(
        "unopened-project".into(),
        "unopened-data".into(),
        Vec::new(),
    )
    .unwrap();
    let mut pending = Some(Request::Run { generation: 99 });
    assert!(worker.try_request(&mut pending));
    assert!(matches!(
        event(&worker),
        Event::Finished {
            generation: 99,
            outcome: Outcome::Failed,
            ..
        }
    ));
    let mut invalid = input();
    invalid.agent = Agent::Codex; // Rejected before resolving any credentials/configuration.
    let mut pending = Some(Request::Preview(Box::new(invalid)));
    assert!(worker.try_request(&mut pending));
    let generation = match event(&worker) {
        Event::Preview(preview) => {
            assert!(!preview.blockers.is_empty());
            preview.generation
        }
        event => panic!("unexpected event: {event:?}"),
    };
    let mut pending = Some(Request::Cancel {
        generation: generation + 1,
    });
    assert!(worker.try_request(&mut pending));
    assert_eq!(worker.cancelled.load(Ordering::Acquire), 0);
    let mut pending = Some(Request::Cancel { generation });
    assert!(worker.try_request(&mut pending));
    assert!(
        matches!(event(&worker), Event::Finished { generation: g, outcome: Outcome::Cancelled, .. } if g == generation)
    );
    let mut pending = Some(Request::Run { generation });
    assert!(worker.try_request(&mut pending));
    assert!(matches!(
        event(&worker),
        Event::Finished {
            outcome: Outcome::Failed,
            ..
        }
    ));
}

#[test]
fn implementation_items_are_explicitly_non_mutating_proposals() {
    let mut run = prepared();
    run.input.operation = Operation::Implement;
    let mut stream = Stream::new(Protocol::Responses);
    stream.append("Suggested patch").unwrap();
    let events = Arc::new(Mutex::new(VecDeque::new()));
    stream.publish_final(&events, &run).unwrap();
    assert!(
        events
            .lock()
            .iter()
            .any(|e| matches!(e, Event::Item { text, .. }
        if text.starts_with("IMPLEMENT PROPOSAL ONLY") && text.contains("no local files changed")))
    );
}

fn http_fixture(body: String) -> (String, JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/fixture", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "fixture connection deadline");
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("fixture accept failed: {error}"),
            }
        };
        // Accepted sockets inherit O_NONBLOCK on Darwin. Use bounded blocking
        // fixture I/O consistently across platforms, not scheduler timing.
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let count = socket.read(&mut buffer).unwrap();
            assert!(
                count > 0 && request.len() + count <= 16 * 1024,
                "fixture request headers incomplete or oversized"
            );
            request.extend_from_slice(&buffer[..count]);
        }
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    (url, server)
}

#[test]
fn real_response_reader_rejects_eof_without_completion_and_cancels_between_chunks() {
    let runtime = runtime();
    for cancel_after_output in [false, true] {
        let stopped = AtomicBool::new(false);
        let cancelled = AtomicU64::new(0);
        let control = control(&stopped, &cancelled);
        let (url, server) = http_fixture(frame(
            json!({"type":"response.output_text.delta","delta":"Partial text "}),
        ));
        let mut stream = Stream::new(Protocol::Responses);
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let run = prepared();
        let error = runtime.block_on(async {
            let client = http_client().unwrap();
            let mut response = wait_http(client.get(url).send(), &control).await.unwrap();
            stream
                .read(&mut response, &control, |stream| {
                    stream.publish(&events, &run)?;
                    if cancel_after_output {
                        cancelled.store(7, Ordering::Release);
                    }
                    Ok(())
                })
                .await
                .unwrap_err()
        });
        server.join().unwrap();
        if cancel_after_output {
            assert!(error.contains("cancelled locally"));
        } else {
            assert!(error.contains("without an authoritative completion"));
        }
        assert!(!stream.complete);
        assert!(events.lock().iter().any(
            |e| matches!(e, Event::Item { complete: false, text, .. } if text == "Partial text ")
        ));
        assert!(
            !events
                .lock()
                .iter()
                .any(|e| matches!(e, Event::Item { complete: true, .. }))
        );
    }
}

#[test]
fn configured_effort_never_leaks_into_a_different_custom_model() {
    let mut connection = config::test_connection(
        Protocol::Responses,
        "https://example.invalid/v1",
        Some("configured"),
    );
    connection.effort = Some("high".into());
    let mut input = input();
    assert_eq!(selected_effort(&input, &connection), Some("high"));
    input.selection.model = Some("configured".into());
    assert_eq!(selected_effort(&input, &connection), Some("high"));
    input.selection.model = Some("custom-other-model".into());
    assert_eq!(selected_effort(&input, &connection), None);
    let body = inference_body(
        connection.protocol,
        selected_model(&input, &connection).unwrap(),
        selected_effort(&input, &connection),
        "reviewed",
    )
    .unwrap();
    assert!(body.get("reasoning").is_none());
    input.selection.effort = Some("low".into());
    assert_eq!(selected_effort(&input, &connection), Some("low"));
}
