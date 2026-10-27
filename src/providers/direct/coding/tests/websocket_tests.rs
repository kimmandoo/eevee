use super::*;
use std::net::TcpStream;
use tokio_tungstenite::tungstenite::{self, Message, WebSocket};

type Socket = WebSocket<TcpStream>;

struct Trace {
    packets: Vec<String>,
    path: String,
    authorization: Option<String>,
}

fn fixture(paths: &[&str]) -> Fixture {
    let mut fixture = Fixture::new(paths);
    fixture.input.native_transport = crate::providers::NativeTransport::WebSocket;
    fixture
}

fn accept_socket(listener: &TcpListener) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        match listener.accept() {
            Ok((socket, _)) => {
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                return socket;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "Native WebSocket did not connect"
                );
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("{error}"),
        }
    }
}

fn no_second_connection(listener: &TcpListener) {
    let deadline = Instant::now() + Duration::from_millis(150);
    loop {
        match listener.accept() {
            Ok(_) => panic!("Native WebSocket reconnected or attempted an HTTP fallback"),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("{error}"),
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn websocket_peer(
    script: impl FnOnce(&mut Socket) -> Vec<String> + Send + 'static,
) -> (String, thread::JoinHandle<Trace>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let socket = accept_socket(&listener);
        let mut path = String::new();
        let mut authorization = None;
        let mut socket = tungstenite::accept_hdr(
            socket,
            |request: &tungstenite::handshake::server::Request,
             response: tungstenite::handshake::server::Response| {
                path = request.uri().to_string();
                authorization = request
                    .headers()
                    .get("authorization")
                    .map(|value| value.to_str().unwrap().to_owned());
                Ok(response)
            },
        )
        .unwrap();
        let packets = script(&mut socket);
        drop(socket);
        no_second_connection(&listener);
        Trace {
            packets,
            path,
            authorization,
        }
    });
    (url, server)
}

fn packet(socket: &mut Socket) -> String {
    loop {
        match socket
            .read()
            .expect("Expected response.create on retained WebSocket")
        {
            Message::Text(text) => return text.to_string(),
            Message::Ping(_) => socket.flush().unwrap(),
            other => panic!("Unexpected client frame before response.create: {other:?}"),
        }
    }
}

fn closed_without_replay(socket: &mut Socket) {
    loop {
        match socket.read() {
            Ok(Message::Close(_)) => {
                let _ = socket.flush();
                return;
            }
            Ok(Message::Ping(_)) => socket.flush().unwrap(),
            Ok(Message::Pong(_)) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => return,
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionReset
                        | io::ErrorKind::BrokenPipe
                        | io::ErrorKind::UnexpectedEof
                ) =>
            {
                return;
            }
            Err(tungstenite::Error::Protocol(
                tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
            )) => return,
            other => panic!("Expected bounded retirement without another model packet: {other:?}"),
        }
    }
}

fn send(socket: &mut Socket, event: Value) {
    socket
        .send(Message::Text(event.to_string().into()))
        .unwrap();
}

fn created(id: &str) -> Value {
    json!({"type":"response.created","sequence_number":0,"response":{"id":id,"status":"in_progress","model":"fixture-model"}})
}

fn completed(id: &str, sequence: u64, calls: &[(&str, &str, &str)], text: &str) -> Value {
    let mut output = vec![
        json!({"type":"reasoning","id":format!("reason-{id}"),"encrypted_content":"private-native-websocket-state"}),
    ];
    if !text.is_empty() {
        output.push(json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}));
    }
    output.extend(calls.iter().map(|(call, path, content)| {
        json!({
            "type":"function_call","id":format!("item-{call}"),"call_id":call,
            "name":"eevee_write_file","arguments":json!({"path":path,"content":content}).to_string()
        })
    }));
    json!({"type":"response.completed","sequence_number":sequence,"response":{"id":id,"status":"completed","model":"fixture-model","output":output,"usage":{"input_tokens":11,"output_tokens":3}}})
}

fn round(socket: &mut Socket, id: &str, calls: &[(&str, &str, &str)], text: &str) {
    send(socket, created(id));
    send(socket, completed(id, 1, calls, text));
}

fn receipt_count(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, Event::Dispatched { .. }))
        .count()
}

fn no_public_completion(events: &[Event]) {
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::NativeCompleted { .. }))
    );
}

fn only_output(packet: &Value, anchor: &str, call: &str, status: &str, path: &str) {
    assert_eq!(packet["type"], "response.create");
    assert_eq!(packet["store"], false);
    assert_eq!(packet["previous_response_id"], anchor);
    assert!(packet.get("stream").is_none());
    assert!(packet.get("background").is_none());
    let input = packet["input"].as_array().unwrap();
    assert_eq!(
        input.len(),
        1,
        "Continuation must not replay public, private or earlier tool items"
    );
    assert_eq!(input[0]["type"], "function_call_output");
    assert_eq!(input[0]["call_id"], call);
    let output: Value = serde_json::from_str(input[0]["output"].as_str().unwrap()).unwrap();
    assert_eq!(output["status"], status);
    assert_eq!(output["path"], path);
    if status == "success" {
        assert_eq!(output["effect"], "applied");
    }
}

#[test]
fn native_websocket_retains_one_handshake_with_frozen_initial_and_actual_anchored_tool_deltas() {
    let mut fixture = fixture(&["first.txt", "nested/日本語.txt"]);
    let (url, server) = websocket_peer(|socket| {
        let first = packet(socket);
        round(
            socket,
            "response-first",
            &[("call-first", "first.txt", "first bytes\n")],
            "First public step",
        );
        let second = packet(socket);
        round(
            socket,
            "response-second",
            &[("call-second", "nested/日本語.txt", "こんにちは\n")],
            "Second public step",
        );
        let third = packet(socket);
        round(socket, "response-final", &[], "Both actual writes settled");
        closed_without_replay(socket);
        vec![first, second, third]
    });
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let frozen = run.payload.clone();
    let prior = run.input.conversation.clone().unwrap();
    let partner = fixture.input.task.lead_partner;
    let draft = &mut fixture
        .input
        .task
        .partner_drafts
        .get_mut(&partner)
        .unwrap()
        .drafts[0];
    draft.text = "Changed only after preparation; must never be sent".into();
    draft.cursor = draft.text.len();
    let project = fixture.workspace.project.clone();
    let approvals = Arc::new(Mutex::new(Vec::new()));
    let observed = approvals.clone();
    let ((outcome, message), events) = run_with_handler(
        &fixture,
        &mut run,
        &connection,
        move |generation, request, sender| {
            let PromptKind::Permission { options, file, .. } = &request.kind else {
                panic!("Expected file permission")
            };
            let file = file.as_ref().unwrap();
            assert!(
                !project.join(&file.path).exists(),
                "Proposal must not write before fresh approval"
            );
            if file.path == "nested/日本語.txt" {
                assert_eq!(
                    std::fs::read_to_string(project.join("first.txt")).unwrap(),
                    "first bytes\n"
                );
            }
            observed.lock().push(request.request_id.clone());
            sender
                .send(Request::Answer {
                    generation,
                    request_id: request.request_id.clone(),
                    answer: options[1].answer.clone(),
                })
                .unwrap();
        },
    );
    assert_eq!(outcome, Outcome::Completed, "{message}");
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.project.join("first.txt")).unwrap(),
        "first bytes\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.project.join("nested/日本語.txt")).unwrap(),
        "こんにちは\n"
    );
    assert_eq!(run.payload, frozen);
    let trace = server.join().unwrap();
    assert_eq!(trace.path, "/v1/responses");
    assert!(trace.authorization.is_none());
    assert_eq!(trace.packets.len(), 3);
    assert_eq!(
        trace.packets[0], frozen,
        "Send exactly the reviewed serialized frame"
    );
    let initial: Value = serde_json::from_str(&trace.packets[0]).unwrap();
    assert_eq!(initial["type"], "response.create");
    assert_eq!(initial["store"], false);
    assert!(initial.get("stream").is_none());
    assert!(initial.get("background").is_none());
    assert!(initial.get("previous_response_id").is_none());
    let second: Value = serde_json::from_str(&trace.packets[1]).unwrap();
    let third: Value = serde_json::from_str(&trace.packets[2]).unwrap();
    only_output(
        &second,
        "response-first",
        "call-first",
        "success",
        "first.txt",
    );
    only_output(
        &third,
        "response-second",
        "call-second",
        "success",
        "nested/日本語.txt",
    );
    for (key, value) in initial.as_object().unwrap() {
        if key != "input" {
            assert_eq!(&second[key], value, "Frozen option {key} changed");
            assert_eq!(&third[key], value, "Frozen option {key} changed");
        }
    }
    assert_eq!(receipt_count(&events), 1);
    let approvals = approvals.lock();
    assert_eq!(approvals.len(), 2);
    assert_ne!(
        approvals[0], approvals[1],
        "Every effect needs a fresh request identity"
    );
    assert_eq!(events.iter().filter(|event| matches!(event, Event::Journal { record, .. } if record.text.contains("success"))).count(), 2);
    let canonical = events
        .iter()
        .find_map(|event| match event {
            Event::NativeCompleted { conversation, .. } => Some(conversation),
            _ => None,
        })
        .unwrap();
    assert_eq!(canonical.messages.len(), prior.messages.len() + 2);
    for (before, after) in prior.messages.iter().zip(canonical.messages.iter()) {
        assert!(Arc::ptr_eq(&before.text, &after.text));
    }
    assert_eq!(
        canonical.messages.last().unwrap().text.as_ref(),
        "First public step\n\nSecond public step\n\nBoth actual writes settled"
    );
    assert!(!canonical.messages.iter().any(|message| {
        message.text.contains("private-native-websocket-state")
            || message.text.contains("function_call")
            || message.text.contains("call-first")
            || message.text.contains("first bytes")
    }));
}

#[test]
fn native_websocket_denied_effect_sends_only_the_actual_denial() {
    let fixture = fixture(&[]);
    let (url, server) = websocket_peer(|socket| {
        let first = packet(socket);
        round(
            socket,
            "response-denied",
            &[("call-denied", "not-created.txt", "unapproved")],
            "",
        );
        let second = packet(socket);
        round(socket, "response-denial-final", &[], "The write was denied");
        closed_without_replay(socket);
        vec![first, second]
    });
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, false);
    assert_eq!(outcome, Outcome::Completed, "{message}");
    assert!(!fixture.workspace.project.join("not-created.txt").exists());
    let trace = server.join().unwrap();
    only_output(
        &serde_json::from_str(&trace.packets[1]).unwrap(),
        "response-denied",
        "call-denied",
        "denied",
        "not-created.txt",
    );
    assert_eq!(receipt_count(&events), 1);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::Prompt { .. }))
            .count(),
        1
    );
    assert_eq!(events.iter().filter(|event| matches!(event, Event::Journal { record, .. } if record.text.contains("denied"))).count(), 1);
}

#[test]
fn native_websocket_registered_item_lane_streams_real_tool_arguments() {
    let fixture = fixture(&[]);
    let (url, server) = websocket_peer(|socket| {
        let first = packet(socket);
        let terminal = completed(
            "response-streamed",
            6,
            &[("call-streamed", "streamed.txt", "streamed bytes")],
            "",
        );
        let arguments = terminal["response"]["output"][1]["arguments"]
            .as_str()
            .unwrap();
        let split = arguments.len() / 2;
        send(socket, created("response-streamed"));
        send(
            socket,
            json!({"type":"response.output_item.added","sequence_number":1,"response_id":"response-streamed","output_index":1,"item":{"id":"item-call-streamed","type":"function_call","call_id":"call-streamed","name":"eevee_write_file","arguments":""}}),
        );
        send(
            socket,
            json!({"type":"response.function_call_arguments.delta","sequence_number":2,"item_id":"item-call-streamed","output_index":1,"delta":&arguments[..split]}),
        );
        send(
            socket,
            json!({"type":"response.function_call_arguments.delta","sequence_number":3,"item_id":"item-call-streamed","output_index":1,"delta":&arguments[split..]}),
        );
        send(
            socket,
            json!({"type":"response.function_call_arguments.done","sequence_number":4,"item_id":"item-call-streamed","output_index":1,"arguments":arguments}),
        );
        send(
            socket,
            json!({"type":"response.output_item.done","sequence_number":5,"response_id":"response-streamed","output_index":1,"item":terminal["response"]["output"][1]}),
        );
        send(socket, terminal);
        let second = packet(socket);
        round(
            socket,
            "response-streamed-final",
            &[],
            "Actual streamed write settled",
        );
        closed_without_replay(socket);
        vec![first, second]
    });
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Completed, "{message}");
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.project.join("streamed.txt")).unwrap(),
        "streamed bytes"
    );
    let trace = server.join().unwrap();
    only_output(
        &serde_json::from_str(&trace.packets[1]).unwrap(),
        "response-streamed",
        "call-streamed",
        "success",
        "streamed.txt",
    );
    assert_eq!(receipt_count(&events), 1);
}

#[test]
fn native_websocket_permission_wait_services_ping_then_cancel_without_write_or_replay() {
    let fixture = fixture(&[]);
    let (permission, waiting) = mpsc::sync_channel(1);
    let (pong, received) = mpsc::sync_channel(1);
    let (url, server) = websocket_peer(move |socket| {
        let first = packet(socket);
        round(
            socket,
            "response-ping",
            &[("call-ping", "never-created.txt", "no consent")],
            "",
        );
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        socket
            .send(Message::Ping(b"permission-wait-control".to_vec().into()))
            .unwrap();
        let Message::Pong(payload) = socket
            .read()
            .expect("Ping must be serviced during synchronous permission wait")
        else {
            panic!("Expected pong")
        };
        assert_eq!(payload.as_ref(), b"permission-wait-control");
        pong.send(()).unwrap();
        closed_without_replay(socket);
        vec![first]
    });
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let project = fixture.workspace.project.clone();
    let ((outcome, _), events) = run_with_handler(
        &fixture,
        &mut run,
        &connection,
        move |generation, _, sender| {
            assert!(!project.join("never-created.txt").exists());
            permission.send(()).unwrap();
            received.recv_timeout(Duration::from_secs(5)).unwrap();
            sender.send(Request::Cancel { generation }).unwrap();
        },
    );
    assert_eq!(outcome, Outcome::Cancelled);
    assert_eq!(server.join().unwrap().packets.len(), 1);
    assert!(!fixture.workspace.project.join("never-created.txt").exists());
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::PromptClosed { .. }))
            .count(),
        1
    );
    assert_eq!(receipt_count(&events), 1);
    no_public_completion(&events);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::LocalEffectsUnknown { .. }))
    );
}

#[test]
fn native_websocket_permission_wait_services_close_and_retires_unapproved_effect() {
    let fixture = fixture(&[]);
    let (permission, waiting) = mpsc::sync_channel(1);
    let (closed, observed) = mpsc::sync_channel(1);
    let (url, server) = websocket_peer(move |socket| {
        let first = packet(socket);
        round(
            socket,
            "response-close",
            &[("call-close", "closed.txt", "no consent")],
            "",
        );
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        socket.close(None).unwrap();
        assert!(matches!(socket.read(), Ok(Message::Close(_))));
        closed.send(()).unwrap();
        vec![first]
    });
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, _), events) =
        run_with_handler(&fixture, &mut run, &connection, move |_, _, _| {
            permission.send(()).unwrap();
            observed.recv_timeout(Duration::from_secs(5)).unwrap();
        });
    assert_eq!(outcome, Outcome::Failed);
    assert_eq!(server.join().unwrap().packets.len(), 1);
    assert!(!fixture.workspace.project.join("closed.txt").exists());
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::PromptClosed { .. }))
            .count(),
        1
    );
    no_public_completion(&events);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::LocalEffectsUnknown { .. }))
    );
}

#[test]
fn native_websocket_lane_refuses_unattributed_interleaved_and_regressed_events() {
    let item = json!({"type":"response.output_item.added","sequence_number":1,"response_id":"response-lane","output_index":0,"item":{"id":"known-item","type":"message","role":"assistant","content":[]}});
    let sparse_item = json!({"type":"response.output_item.added","sequence_number":1,"response_id":"response-lane","output_index":1,"item":{"id":"known-item","type":"message","role":"assistant","content":[]}});
    let cases = vec![
        (
            "completion before creation",
            vec![completed("response-lane", 1, &[], "not attributable")],
            0,
        ),
        (
            "missing sequence",
            vec![json!({"type":"response.created","response":{"id":"response-lane"}})],
            0,
        ),
        (
            "negative sequence",
            vec![
                json!({"type":"response.created","sequence_number":-1,"response":{"id":"response-lane"}}),
            ],
            0,
        ),
        (
            "named stream",
            vec![
                json!({"type":"response.created","sequence_number":0,"stream_id":"other-lane","response":{"id":"response-lane"}}),
            ],
            0,
        ),
        (
            "duplicate sequence",
            vec![
                created("response-lane"),
                json!({"type":"response.output_text.delta","sequence_number":0,"response_id":"response-lane","delta":"unowned"}),
            ],
            1,
        ),
        (
            "regressed sequence",
            vec![
                created("response-lane"),
                json!({"type":"response.in_progress","sequence_number":3,"response_id":"response-lane"}),
                completed("response-lane", 2, &[], "unowned"),
            ],
            1,
        ),
        (
            "mismatched response",
            vec![
                created("response-lane"),
                completed("different-response", 1, &[], "unowned"),
            ],
            1,
        ),
        (
            "second creation",
            vec![
                created("response-lane"),
                json!({"type":"response.created","sequence_number":1,"response":{"id":"other-response"}}),
            ],
            1,
        ),
        (
            "unknown item",
            vec![
                created("response-lane"),
                json!({"type":"response.output_text.delta","sequence_number":1,"item_id":"unknown-item","output_index":0,"delta":"unowned"}),
            ],
            1,
        ),
        (
            "missing attribution",
            vec![
                created("response-lane"),
                json!({"type":"response.output_text.delta","sequence_number":1,"delta":"unowned"}),
            ],
            1,
        ),
        (
            "changed item index",
            vec![
                created("response-lane"),
                item.clone(),
                json!({"type":"response.output_text.delta","sequence_number":2,"item_id":"known-item","output_index":1,"delta":"unowned"}),
            ],
            1,
        ),
        (
            "duplicate item index",
            vec![
                created("response-lane"),
                item,
                json!({"type":"response.output_item.added","sequence_number":2,"response_id":"response-lane","output_index":0,"item":{"id":"different-item","type":"message","role":"assistant","content":[]}}),
            ],
            1,
        ),
        (
            "sparse duplicate item identity",
            vec![
                created("response-lane"),
                sparse_item.clone(),
                json!({"type":"response.output_item.added","sequence_number":2,"response_id":"response-lane","output_index":2,"item":{"id":"known-item","type":"message","role":"assistant","content":[]}}),
            ],
            1,
        ),
        (
            "sparse regressed item index",
            vec![
                created("response-lane"),
                sparse_item.clone(),
                json!({"type":"response.output_item.added","sequence_number":2,"response_id":"response-lane","output_index":0,"item":{"id":"earlier-item","type":"message","role":"assistant","content":[]}}),
            ],
            1,
        ),
        (
            "snapshot changed registered item identity",
            vec![
                created("response-lane"),
                sparse_item.clone(),
                json!({"type":"response.completed","sequence_number":2,"response":{"id":"response-lane","status":"completed","output":[{"type":"reasoning","id":"reason-response-lane"},{"type":"message","id":"different-item","role":"assistant","content":[]}]}}),
            ],
            1,
        ),
        (
            "repeated completed item",
            vec![
                created("response-lane"),
                sparse_item,
                json!({"type":"response.output_item.done","sequence_number":2,"response_id":"response-lane","output_index":1,"item":{"id":"known-item","type":"message","role":"assistant","content":[]}}),
                json!({"type":"response.output_item.done","sequence_number":3,"response_id":"response-lane","output_index":1,"item":{"id":"known-item","type":"message","role":"assistant","content":[]}}),
            ],
            1,
        ),
    ];
    for (name, frames, receipts) in cases {
        let fixture = fixture(&[]);
        let (url, server) = websocket_peer(move |socket| {
            let first = packet(socket);
            for event in frames {
                send(socket, event);
            }
            closed_without_replay(socket);
            vec![first]
        });
        let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
        let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
        assert_eq!(outcome, Outcome::Unknown, "{name}: {message}");
        assert_eq!(receipt_count(&events), receipts, "{name}");
        no_public_completion(&events);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::Prompt { .. })),
            "{name}"
        );
        assert_eq!(server.join().unwrap().packets.len(), 1, "{name}");
    }
}

#[test]
fn native_websocket_stale_response_id_cannot_seed_a_second_tool_round() {
    let fixture = fixture(&[]);
    let (url, server) = websocket_peer(|socket| {
        let first = packet(socket);
        round(
            socket,
            "response-retired",
            &[("call-first", "settled.txt", "settled once")],
            "",
        );
        let second = packet(socket);
        send(socket, created("response-retired"));
        send(
            socket,
            completed(
                "response-retired",
                1,
                &[("call-second", "replayed.txt", "must not write")],
                "",
            ),
        );
        closed_without_replay(socket);
        vec![first, second]
    });
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Unknown, "{message}");
    assert!(message.contains("stale"));
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.project.join("settled.txt")).unwrap(),
        "settled once"
    );
    assert!(!fixture.workspace.project.join("replayed.txt").exists());
    assert_eq!(server.join().unwrap().packets.len(), 2);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::Prompt { .. }))
            .count(),
        1
    );
    assert_eq!(receipt_count(&events), 1);
    no_public_completion(&events);
}

#[test]
fn native_websocket_replayed_call_id_does_not_reapprove_or_repeat_a_settled_write() {
    let fixture = fixture(&[]);
    let (url, server) = websocket_peer(|socket| {
        let first = packet(socket);
        round(
            socket,
            "response-call-first",
            &[("same-call", "once.txt", "first exact bytes")],
            "",
        );
        let second = packet(socket);
        round(
            socket,
            "response-call-replay",
            &[("same-call", "once.txt", "replacement forbidden")],
            "",
        );
        closed_without_replay(socket);
        vec![first, second]
    });
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Failed, "{message}");
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.project.join("once.txt")).unwrap(),
        "first exact bytes"
    );
    assert_eq!(server.join().unwrap().packets.len(), 2);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::Prompt { .. }))
            .count(),
        1
    );
    assert_eq!(events.iter().filter(|event| matches!(event, Event::Journal { record, .. } if record.text.contains("success"))).count(), 1);
    assert_eq!(receipt_count(&events), 1);
    no_public_completion(&events);
}

#[test]
fn native_websocket_attributable_provider_failure_is_failed_without_public_history_or_retry() {
    let fixture = fixture(&[]);
    let (url, server) = websocket_peer(|socket| {
        let first = packet(socket);
        send(socket, created("response-failed"));
        send(
            socket,
            json!({"type":"response.failed","sequence_number":1,"response":{"id":"response-failed","status":"failed","error":{"message":"private provider failure marker"},"usage":{"input_tokens":7,"output_tokens":1}}}),
        );
        closed_without_replay(socket);
        vec![first]
    });
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Failed, "{message}");
    assert!(!message.contains("private provider failure marker"));
    assert_eq!(receipt_count(&events), 1);
    no_public_completion(&events);
    assert_eq!(server.join().unwrap().packets.len(), 1);
}

#[test]
fn native_websocket_upgrade_without_model_receipt_remains_unknown_and_never_replays() {
    let fixture = fixture(&[]);
    let (url, server) = websocket_peer(|socket| {
        let first = packet(socket);
        socket.close(None).unwrap();
        closed_without_replay(socket);
        vec![first]
    });
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Unknown, "{message}");
    assert_eq!(
        receipt_count(&events),
        0,
        "HTTP 101 is not model submission evidence"
    );
    no_public_completion(&events);
    assert_eq!(server.join().unwrap().packets.len(), 1);
}

fn oversized_message(fragmented: bool) {
    use tungstenite::protocol::frame::{
        Frame,
        coding::{Data, OpCode},
    };
    let fixture = fixture(&[]);
    let (url, server) = websocket_peer(move |socket| {
        let first = packet(socket);
        let mut event = created("oversized-created");
        event["padding"] = json!("x".repeat(FRAME_LIMIT));
        let text = event.to_string();
        assert!(text.len() > FRAME_LIMIT);
        if fragmented {
            let split = text.len() / 2;
            socket
                .send(Message::Frame(Frame::message(
                    text.as_bytes()[..split].to_vec(),
                    OpCode::Data(Data::Text),
                    false,
                )))
                .unwrap();
            let _ = socket.send(Message::Frame(Frame::message(
                text.as_bytes()[split..].to_vec(),
                OpCode::Data(Data::Continue),
                true,
            )));
        } else {
            let _ = socket.send(Message::Text(text.into()));
        }
        closed_without_replay(socket);
        vec![first]
    });
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Unknown, "{message}");
    assert_eq!(
        receipt_count(&events),
        0,
        "Oversized assembled JSON must not be admitted as a receipt"
    );
    no_public_completion(&events);
    assert_eq!(server.join().unwrap().packets.len(), 1);
}

#[test]
fn native_websocket_frame_limit_refuses_oversized_receipt_before_json_admission() {
    oversized_message(false);
}

#[test]
fn native_websocket_fragment_limit_refuses_oversized_assembled_receipt_before_json_admission() {
    oversized_message(true);
}

#[test]
fn native_websocket_event_queue_overflow_retires_the_actual_consumer_without_continuation() {
    let fixture = fixture(&[]);
    let events = Arc::new(Mutex::new(VecDeque::new()));
    let (block, requested) = mpsc::sync_channel(1);
    let (locked, ready) = mpsc::sync_channel(1);
    let (release, released) = mpsc::sync_channel(1);
    let held_events = events.clone();
    let blocker = thread::spawn(move || {
        requested.recv_timeout(Duration::from_secs(8)).unwrap();
        let _guard = held_events.lock();
        locked.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(8)).unwrap();
    });
    let (url, server) = websocket_peer(move |socket| {
        let first = packet(socket);
        block.send(()).unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        // Hold only the actual UI event sink. The IO owner must still enforce its
        // own queue bound while the consumer blocks publishing the first receipt.
        send(socket, created("response-overflow"));
        for sequence in 1..=(QUEUE_LIMIT as u64 + 8) {
            if socket.send(Message::Text(json!({"type":"response.in_progress","sequence_number":sequence,"response_id":"response-overflow"}).to_string().into())).is_err() {
                break;
            }
        }
        closed_without_replay(socket);
        release.send(()).unwrap();
        vec![first]
    });
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let stopped = AtomicBool::new(false);
    let cancelled = AtomicU64::new(0);
    let control = Control {
        wire_alive: None,
        stopped: &stopped,
        cancelled: &cancelled,
        generation: 7,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let (_sender, incoming) = mpsc::sync_channel(1);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (outcome, message) = runtime.block_on(async {
        let http = http_client().unwrap();
        inference_loop(
            &fixture.workspace,
            &mut run,
            &events,
            &control,
            &incoming,
            &http,
            &connection,
            &|_| Ok(()),
        )
        .await
    });
    blocker.join().unwrap();
    assert_eq!(outcome, Outcome::Unknown, "{message}");
    assert!(message.contains("queue overflow"), "{message}");
    let events: Vec<_> = events.lock().drain(..).collect();
    assert!(receipt_count(&events) <= 1);
    no_public_completion(&events);
    assert_eq!(server.join().unwrap().packets.len(), 1);
}

fn prepare_authenticated(fixture: &Fixture, url: &str) -> (Prepared, Connection) {
    let (mut run, _) = fixture.prepare(Protocol::Responses, url);
    let connection = config::test_authenticated_connection(
        Protocol::Responses,
        url,
        Some("fixture-model"),
        "synthetic-native-websocket-bearer",
    );
    run.fingerprint = connection.fingerprint();
    run.input.conversation = None;
    run.payload = payload(
        &fixture.workspace,
        &run.input,
        &connection,
        run.writes.as_ref(),
    )
    .unwrap();
    (run, connection)
}

#[test]
fn native_websocket_scoped_bearer_reaches_only_the_actual_reviewed_upgrade() {
    let fixture = fixture(&[]);
    let (url, server) = websocket_peer(|socket| {
        let first = packet(socket);
        round(
            socket,
            "response-authenticated",
            &[],
            "Scoped synthetic connection completed",
        );
        closed_without_replay(socket);
        vec![first]
    });
    let (mut run, connection) = prepare_authenticated(&fixture, &url);
    let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Completed, "{message}");
    let trace = server.join().unwrap();
    assert_eq!(trace.path, "/v1/responses");
    assert_eq!(
        trace.authorization.as_deref(),
        Some("Bearer synthetic-native-websocket-bearer")
    );
    assert!(!trace.packets[0].contains("synthetic-native-websocket-bearer"));
    assert_eq!(receipt_count(&events), 1);
}

fn rejected_upgrade(
    status: &'static str,
    location: Option<String>,
) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let mut socket = accept_socket(&listener);
        let mut bytes = Vec::new();
        while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            let mut chunk = [0; 2048];
            let length = socket.read(&mut chunk).unwrap();
            assert_ne!(length, 0);
            bytes.extend_from_slice(&chunk[..length]);
            assert!(bytes.len() <= 32768);
        }
        let request = String::from_utf8(bytes).unwrap();
        let location = location.map_or_else(String::new, |url| format!("Location: {url}\r\n"));
        socket
            .write_all(
                format!(
                    "HTTP/1.1 {status}\r\n{location}Content-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .unwrap();
        drop(socket);
        no_second_connection(&listener);
        request
    });
    (url, server)
}

#[test]
fn native_websocket_auth_rejection_does_not_submit_fallback_or_report_a_receipt() {
    let fixture = fixture(&[]);
    let (url, server) = rejected_upgrade("401 Unauthorized", None);
    let (mut run, connection) = prepare_authenticated(&fixture, &url);
    let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Failed, "{message}");
    assert_eq!(receipt_count(&events), 0);
    no_public_completion(&events);
    let request = server.join().unwrap();
    assert!(request.starts_with("GET /v1/responses HTTP/1.1\r\n"));
    assert!(request.lines().any(|line| {
        line.eq_ignore_ascii_case("authorization: Bearer synthetic-native-websocket-bearer")
    }));
    assert!(!request.contains("response.create"));
}

#[test]
fn native_websocket_redirect_does_not_forward_credentials_change_path_or_fallback() {
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let destination = format!(
        "http://{}/different/response/path",
        target.local_addr().unwrap()
    );
    let fixture = fixture(&[]);
    let (url, server) = rejected_upgrade("307 Temporary Redirect", Some(destination));
    let (mut run, connection) = prepare_authenticated(&fixture, &url);
    let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Failed, "{message}");
    assert_eq!(receipt_count(&events), 0);
    no_public_completion(&events);
    let request = server.join().unwrap();
    assert!(request.starts_with("GET /v1/responses HTTP/1.1\r\n"));
    assert!(request.lines().any(|line| {
        line.eq_ignore_ascii_case("authorization: Bearer synthetic-native-websocket-bearer")
    }));
    no_second_connection(&target);
}

#[test]
fn native_websocket_new_explicit_send_uses_fresh_connection_and_only_completed_public_history() {
    let fixture = fixture(&[]);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let mut first_socket = tungstenite::accept(accept_socket(&listener)).unwrap();
        let first = packet(&mut first_socket);
        round(
            &mut first_socket,
            "response-old-private",
            &[("call-old-private", "old.txt", "old actual bytes")],
            "First completed public step",
        );
        let continuation = packet(&mut first_socket);
        round(
            &mut first_socket,
            "response-old-final",
            &[],
            "First completed public answer",
        );
        closed_without_replay(&mut first_socket);
        drop(first_socket);
        let mut second_socket = tungstenite::accept(accept_socket(&listener)).unwrap();
        let next_send = packet(&mut second_socket);
        round(
            &mut second_socket,
            "response-new-send",
            &[],
            "Fresh explicit public answer",
        );
        closed_without_replay(&mut second_socket);
        drop(second_socket);
        no_second_connection(&listener);
        vec![first, continuation, next_send]
    });
    let (mut first_run, first_connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, message), first_events) =
        run_with_answers(&fixture, &mut first_run, &first_connection, true);
    assert_eq!(outcome, Outcome::Completed, "{message}");
    let canonical = first_events
        .iter()
        .find_map(|event| match event {
            Event::NativeCompleted { conversation, .. } => Some(conversation.clone()),
            _ => None,
        })
        .unwrap();
    let (mut second_run, second_connection) = fixture.prepare(Protocol::Responses, &url);
    second_run.input.conversation = Some(canonical);
    let partner = second_run.input.task.lead_partner;
    let draft = &mut second_run
        .input
        .task
        .partner_drafts
        .get_mut(&partner)
        .unwrap()
        .drafts[0];
    draft.text = "A distinct deliberately submitted request".into();
    draft.cursor = draft.text.len();
    second_run.payload = payload(
        &fixture.workspace,
        &second_run.input,
        &second_connection,
        second_run.writes.as_ref(),
    )
    .unwrap();
    let ((outcome, message), second_events) =
        run_with_answers(&fixture, &mut second_run, &second_connection, true);
    assert_eq!(outcome, Outcome::Completed, "{message}");
    assert_eq!(receipt_count(&first_events), 1);
    assert_eq!(receipt_count(&second_events), 1);
    assert!(
        !second_events
            .iter()
            .any(|event| matches!(event, Event::Prompt { .. } | Event::Journal { .. }))
    );
    let packets = server.join().unwrap();
    let fresh: Value = serde_json::from_str(&packets[2]).unwrap();
    assert!(fresh.get("previous_response_id").is_none());
    assert_eq!(fresh["type"], "response.create");
    assert!(packets[2].contains("First completed public answer"));
    assert!(packets[2].contains("A distinct deliberately submitted request"));
    for private in [
        "response-old-private",
        "response-old-final",
        "call-old-private",
        "private-native-websocket-state",
        "old actual bytes",
        "function_call_output",
    ] {
        assert!(
            !packets[2].contains(private),
            "New explicit Send retained private chain item {private}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.project.join("old.txt")).unwrap(),
        "old actual bytes"
    );
}

#[test]
fn native_websocket_inflight_cancel_retires_connection_without_replay_or_public_history() {
    let fixture = fixture(&[]);
    let (url, server) = websocket_peer(|socket| {
        let first = packet(socket);
        send(socket, created("response-cancel-active"));
        send(
            socket,
            json!({"type":"response.output_text.delta","sequence_number":1,"response_id":"response-cancel-active","delta":"Public partial before cancellation\nsk-secret123456"}),
        );
        closed_without_replay(socket);
        vec![first]
    });
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let events = Arc::new(Mutex::new(VecDeque::new()));
    let cancelled = Arc::new(AtomicU64::new(0));
    let observed = events.clone();
    let cancellation = cancelled.clone();
    let cancel = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if observed.lock().iter().any(|event| matches!(event, Event::Item { text, .. } if text.contains("Public partial before cancellation"))) {
                cancellation.store(7, Ordering::Release);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Actual provider partial never reached the consumer"
            );
            thread::sleep(Duration::from_millis(2));
        }
    });
    let stopped = AtomicBool::new(false);
    let control = Control {
        wire_alive: None,
        stopped: &stopped,
        cancelled: &cancelled,
        generation: 7,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let (_sender, incoming) = mpsc::sync_channel(1);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (outcome, message) = runtime.block_on(async {
        let http = http_client().unwrap();
        inference_loop(
            &fixture.workspace,
            &mut run,
            &events,
            &control,
            &incoming,
            &http,
            &connection,
            &|_| Ok(()),
        )
        .await
    });
    cancel.join().unwrap();
    assert_eq!(outcome, Outcome::Unknown, "{message}");
    let events: Vec<_> = events.lock().drain(..).collect();
    assert_eq!(receipt_count(&events), 1);
    assert!(!events.iter().any(
        |event| matches!(event, Event::Item { text, .. } if text.contains("sk-secret123456"))
    ));
    no_public_completion(&events);
    assert!(!events.iter().any(|event| matches!(
        event,
        Event::LocalEffectsUnknown { .. } | Event::Journal { .. }
    )));
    assert_eq!(server.join().unwrap().packets.len(), 1);
}

#[test]
fn native_websocket_abrupt_transport_loss_preserves_partial_display_but_not_public_history() {
    let fixture = fixture(&[]);
    let (url, server) = websocket_peer(|socket| {
        let first = packet(socket);
        send(socket, created("response-interrupted"));
        send(
            socket,
            json!({"type":"response.output_text.delta","sequence_number":1,"response_id":"response-interrupted","delta":"Interrupted actual public output\nsk-secret123456"}),
        );
        socket.get_mut().shutdown(std::net::Shutdown::Both).unwrap();
        vec![first]
    });
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Unknown, "{message}");
    assert_eq!(receipt_count(&events), 1);
    assert!(events.iter().any(|event| matches!(event, Event::Item { text, .. } if text.contains("Interrupted actual public output"))));
    assert!(!events.iter().any(
        |event| matches!(event, Event::Item { text, .. } if text.contains("sk-secret123456"))
    ));
    no_public_completion(&events);
    assert_eq!(server.join().unwrap().packets.len(), 1);
}

#[test]
fn native_websocket_public_output_limit_spans_tool_rounds_without_truncating_into_history() {
    let fixture = fixture(&[]);
    let text = "public bounded text ".repeat(16_000);
    assert!(text.len() < OUTPUT_LIMIT && text.len() * 2 > OUTPUT_LIMIT);
    let (url, server) = websocket_peer(move |socket| {
        let first = packet(socket);
        round(
            socket,
            "response-output-first",
            &[(
                "call-output",
                "settled-before-limit.txt",
                "settled exact bytes",
            )],
            &text,
        );
        let second = packet(socket);
        round(socket, "response-output-limit", &[], &text);
        closed_without_replay(socket);
        vec![first, second]
    });
    let (mut run, connection) = fixture.prepare(Protocol::Responses, &url);
    let ((outcome, message), events) = run_with_answers(&fixture, &mut run, &connection, true);
    assert_eq!(outcome, Outcome::Unknown, "{message}");
    assert!(message.contains("output capacity"), "{message}");
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.project.join("settled-before-limit.txt"))
            .unwrap(),
        "settled exact bytes"
    );
    assert_eq!(receipt_count(&events), 1);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::Prompt { .. }))
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::LocalEffectsUnknown { .. }))
    );
    no_public_completion(&events);
    assert_eq!(server.join().unwrap().packets.len(), 2);
}
