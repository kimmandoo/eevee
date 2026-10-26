use super::*;

fn write_content(call: &Call) -> &str {
    match &call.tool {
        FileTool::Write { content, .. } => content,
        _ => panic!("expected write"),
    }
}

fn args(path: &str, content: &str) -> String {
    json!({"path":path,"content":content}).to_string()
}

fn result(status: &str) -> Value {
    json!({"status":status,"message":"local outcome","path":"src/new.rs"})
}

fn response_call(id: &str, arguments: &str) -> Value {
    json!({"type":"function_call","id":format!("item_{id}"),"call_id":id,"name":TOOL_NAME,"arguments":arguments,"status":"completed"})
}

fn completed(calls: &mut Calls, output: Vec<Value>) -> Result<(), String> {
    calls.observe(
        "response.completed",
        &json!({"response":{"status":"completed","output":output}}),
    )
}

fn chat_fragment(calls: &mut Calls, fragments: Value, content: Option<&str>) -> Result<(), String> {
    calls.observe(
        "",
        &json!({"choices":[{"index":0,"delta":{"tool_calls":fragments,"content":content}}]}),
    )
}

fn chat_stop(calls: &mut Calls, reason: &str) -> Result<(), String> {
    calls.observe(
        "",
        &json!({"choices":[{"index":0,"delta":{},"finish_reason":reason}]}),
    )
}

fn message_block(calls: &mut Calls, index: usize, value: Value) -> Result<(), String> {
    calls.observe(
        "content_block_start",
        &json!({"index":index,"content_block":value}),
    )
}

fn message_delta(calls: &mut Calls, index: usize, value: Value) -> Result<(), String> {
    calls.observe("content_block_delta", &json!({"index":index,"delta":value}))
}

fn message_close(calls: &mut Calls, index: usize) {
    calls
        .observe("content_block_stop", &json!({"index":index}))
        .unwrap();
}

fn message_finish(calls: &mut Calls) {
    calls
        .observe(
            "message_delta",
            &json!({"delta":{"stop_reason":"tool_use"}}),
        )
        .unwrap();
    calls.observe("message_stop", &json!({})).unwrap();
}

fn gemini(calls: &mut Calls, parts: Vec<Value>, stop: bool) -> Result<(), String> {
    let mut candidate = json!({"index":0,"content":{"role":"model","parts":parts}});
    if stop {
        candidate["finishReason"] = json!("STOP");
    }
    calls.observe("", &json!({"candidates":[candidate]}))
}

#[test]
fn advertisements_are_exact_native_function_declarations() {
    for protocol in [
        Protocol::Responses,
        Protocol::Codex,
        Protocol::ChatCompletions,
        Protocol::Messages,
        Protocol::Gemini,
    ] {
        let mut body = json!({"policy":"immutable"});
        advertise_selected(&mut body, protocol, true, None);
        assert_eq!(body["policy"], "immutable");
        let declaration = match protocol {
            Protocol::Responses | Protocol::Codex | Protocol::Messages => &body["tools"][0],
            Protocol::ChatCompletions => &body["tools"][0]["function"],
            Protocol::Gemini => &body["tools"][0]["functionDeclarations"][0],
        };
        assert_eq!(declaration["name"], TOOL_NAME);
        let schema = match protocol {
            Protocol::Messages => &declaration["input_schema"],
            Protocol::Gemini => &declaration["parametersJsonSchema"],
            _ => &declaration["parameters"],
        };
        assert_eq!(schema["required"], json!(["path", "content"]));
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"].as_object().unwrap().len(), 2);
        if matches!(protocol, Protocol::Responses | Protocol::Codex) {
            assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        }
        assert_eq!(
            declaration
                .get("eager_input_streaming")
                .and_then(Value::as_bool),
            (protocol == Protocol::Messages).then_some(true)
        );
    }
}

#[test]
fn responses_fragments_use_authoritative_output_and_keep_opaque_reasoning() {
    for protocol in [Protocol::Responses, Protocol::Codex] {
        let mut calls = Calls::new(protocol);
        let arguments = args("src/new.rs", "fn main() {}\n");
        let mut started = response_call("call_one", "");
        started["status"] = json!("in_progress");
        calls
            .observe(
                "response.output_item.added",
                &json!({"output_index":1,"item":started}),
            )
            .unwrap();
        let split = arguments.len() / 2;
        for delta in [&arguments[..split], &arguments[split..]] {
            calls
                .observe(
                    "response.function_call_arguments.delta",
                    &json!({"output_index":1,"item_id":"item_call_one","delta":delta}),
                )
                .unwrap();
            assert!(calls.finish(true).is_err());
        }
        calls
            .observe(
                "response.function_call_arguments.done",
                &json!({"output_index":1,"item_id":"item_call_one","arguments":arguments}),
            )
            .unwrap();
        let done = response_call("call_one", &arguments);
        calls
            .observe(
                "response.output_item.done",
                &json!({"output_index":1,"item":done}),
            )
            .unwrap();
        let output = vec![
            json!({"type":"reasoning","id":"reasoning_1","summary":[],"encrypted_content":"opaque-signed-not-public"}),
            done,
            json!({"type":"message","id":"msg_1","role":"assistant","content":[{"type":"output_text","text":"Creating the file.","annotations":[]}],"status":"completed"}),
        ];
        completed(&mut calls, output.clone()).unwrap();
        let decoded = calls.finish(true).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].id.as_deref(), Some("call_one"));
        assert_eq!(decoded[0].tool.path(), PathBuf::from("src/new.rs"));
        assert_eq!(write_content(&decoded[0]), "fn main() {}\n");
        let original = json!({"instructions":"fixed policy","model":"fixed model","input":[{"role":"user","content":"original"}],"store":false});
        let mut body = original.clone();
        calls
            .append_results(&mut body, &[result("success")])
            .unwrap();
        assert_eq!(body["instructions"], original["instructions"]);
        assert_eq!(body["model"], original["model"]);
        assert_eq!(body["input"][0], original["input"][0]);
        assert_eq!(&body["input"].as_array().unwrap()[1..4], output.as_slice());
        assert_eq!(
            body["input"][4],
            json!({"type":"function_call_output","call_id":"call_one","output":result("success").to_string()})
        );
        assert!(calls.finish(false).is_err());
    }
}

#[test]
fn responses_reject_inconsistent_fragments_and_replayed_item_ids() {
    let mut calls = Calls::new(Protocol::Responses);
    calls
        .observe(
            "response.output_item.added",
            &json!({"output_index":0,"item":response_call("one", "")}),
        )
        .unwrap();
    assert!(
        calls
            .observe(
                "response.function_call_arguments.delta",
                &json!({"output_index":0,"item_id":"wrong","delta":"{}"})
            )
            .is_err()
    );

    let mut calls = Calls::new(Protocol::Responses);
    let first = response_call("one", &args("src/new.rs", "a"));
    let mut second = response_call("two", &args("src/new.rs", "b"));
    second["id"] = first["id"].clone();
    completed(&mut calls, vec![first, second]).unwrap();
    assert!(calls.finish(true).is_err());

    let mut calls = Calls::new(Protocol::Responses);
    calls
        .observe(
            "response.output_item.added",
            &json!({"output_index":0,"item":response_call("one", "{}")}),
        )
        .unwrap();
    assert!(
        completed(
            &mut calls,
            vec![response_call("one", &args("src/new.rs", "a"))]
        )
        .is_err()
    );
}

#[test]
fn chat_indexed_fragments_preserve_content_call_order_and_native_results() {
    let mut calls = Calls::new(Protocol::ChatCompletions);
    let first = args("src/new.rs", "first");
    let second = args("src/new.rs", "second");
    chat_fragment(&mut calls, json!([
        {"index":1,"id":"two","type":"function","function":{"name":TOOL_NAME,"arguments":second}},
        {"index":0,"id":"one","type":"function","function":{"name":"eevee_","arguments":&first[..8]}}
    ]), Some("I will write ")).unwrap();
    chat_fragment(
        &mut calls,
        json!([
            {"index":0,"id":"one","function":{"name":"write_file","arguments":&first[8..]}}
        ]),
        Some("the files."),
    )
    .unwrap();
    assert!(calls.finish(true).is_err());
    chat_stop(&mut calls, "tool_calls").unwrap();
    let decoded = calls.finish(true).unwrap();
    assert_eq!(decoded[0].id.as_deref(), Some("one"));
    assert_eq!(write_content(&decoded[0]), "first");
    assert_eq!(decoded[1].id.as_deref(), Some("two"));
    let prefix = json!({"role":"system","content":"immutable policy"});
    let mut body = json!({"messages":[prefix.clone()],"reasoning_effort":"high"});
    calls
        .append_results(&mut body, &[result("denied"), result("success")])
        .unwrap();
    assert_eq!(body["messages"][0], prefix);
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(
        body["messages"][1],
        json!({"role":"assistant","content":"I will write the files.","tool_calls":[
            {"id":"one","type":"function","function":{"name":TOOL_NAME,"arguments":first}},
            {"id":"two","type":"function","function":{"name":TOOL_NAME,"arguments":second}}
        ]})
    );
    assert_eq!(
        body["messages"][2],
        json!({"role":"tool","tool_call_id":"one","content":result("denied").to_string()})
    );
    assert_eq!(
        body["messages"][3],
        json!({"role":"tool","tool_call_id":"two","content":result("success").to_string()})
    );
}

#[test]
fn messages_indexed_text_tools_and_signed_thinking_are_request_local() {
    let mut calls = Calls::new(Protocol::Messages);
    message_block(
        &mut calls,
        0,
        json!({"type":"thinking","thinking":"private ","signature":""}),
    )
    .unwrap();
    message_delta(
        &mut calls,
        0,
        json!({"type":"thinking_delta","thinking":"reasoning"}),
    )
    .unwrap();
    message_delta(
        &mut calls,
        0,
        json!({"type":"signature_delta","signature":"signed-"}),
    )
    .unwrap();
    message_delta(
        &mut calls,
        0,
        json!({"type":"signature_delta","signature":"opaque"}),
    )
    .unwrap();
    message_close(&mut calls, 0);
    message_block(&mut calls, 1, json!({"type":"text","text":"Writing "})).unwrap();
    message_delta(&mut calls, 1, json!({"type":"text_delta","text":"now."})).unwrap();
    message_close(&mut calls, 1);
    message_block(
        &mut calls,
        2,
        json!({"type":"tool_use","id":"tool_1","name":TOOL_NAME,"input":{}}),
    )
    .unwrap();
    let arguments = args("src/new.rs", "only file contents");
    for fragment in [&arguments[..5], &arguments[5..]] {
        message_delta(
            &mut calls,
            2,
            json!({"type":"input_json_delta","partial_json":fragment}),
        )
        .unwrap();
    }
    assert!(calls.finish(true).is_err());
    message_close(&mut calls, 2);
    message_finish(&mut calls);
    let decoded = calls.finish(true).unwrap();
    assert_eq!(write_content(&decoded[0]), "only file contents");
    let prefix = json!({"role":"user","content":"original input"});
    let mut body = json!({"system":"fixed policy","messages":[prefix.clone()]});
    calls
        .append_results(&mut body, &[result("refused")])
        .unwrap();
    assert_eq!(body["system"], "fixed policy");
    assert_eq!(body["messages"][0], prefix);
    assert_eq!(
        body["messages"][1],
        json!({"role":"assistant","content":[
            {"type":"thinking","thinking":"private reasoning","signature":"signed-opaque"},
            {"type":"text","text":"Writing now."},
            {"type":"tool_use","id":"tool_1","name":TOOL_NAME,"input":{"path":"src/new.rs","content":"only file contents"}}
        ]})
    );
    assert_eq!(
        body["messages"][2],
        json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"tool_1","is_error":true,"content":result("refused").to_string()}]})
    );
}

#[test]
fn gemini_parts_preserve_thought_signatures_and_optional_ids_stay_absent() {
    for id in [None, Some("gemini_one")] {
        let mut calls = Calls::new(Protocol::Gemini);
        let thought =
            json!({"text":"private thought","thought":true,"thoughtSignature":"signed-thought"});
        gemini(&mut calls, vec![thought.clone()], false).unwrap();
        let mut function =
            json!({"name":TOOL_NAME,"args":{"path":"src/new.rs","content":"contents"}});
        if let Some(id) = id {
            function["id"] = json!(id);
        }
        let call_part = json!({"functionCall":function,"thoughtSignature":"signed-call"});
        gemini(
            &mut calls,
            vec![json!({"text":"Writing."}), call_part.clone()],
            true,
        )
        .unwrap();
        // A usage-only trailer is not another assistant message.
        calls
            .observe("", &json!({"usageMetadata":{"totalTokenCount":20}}))
            .unwrap();
        let decoded = calls.finish(true).unwrap();
        assert_eq!(decoded[0].id.as_deref(), id);
        assert_eq!(write_content(&decoded[0]), "contents");
        let prefix = json!({"role":"user","parts":[{"text":"original"}]});
        let mut body = json!({"systemInstruction":{"parts":[{"text":"fixed policy"}]},"contents":[prefix.clone()]});
        calls
            .append_results(&mut body, &[result("denied")])
            .unwrap();
        assert_eq!(body["contents"][0], prefix);
        assert_eq!(
            body["systemInstruction"],
            json!({"parts":[{"text":"fixed policy"}]})
        );
        assert_eq!(
            body["contents"][1],
            json!({"role":"model","parts":[thought,{"text":"Writing."},call_part]})
        );
        let response = &body["contents"][2]["parts"][0]["functionResponse"];
        assert_eq!(response["name"], TOOL_NAME);
        assert_eq!(response["response"], json!({"error":result("denied")}));
        assert_eq!(response.get("id").and_then(Value::as_str), id);
        if id.is_none() {
            assert!(response.get("id").is_none());
        }
    }
}

#[test]
fn every_dialect_retains_schema_refusals_for_correlated_zero_effect_results() {
    for protocol in [
        Protocol::Responses,
        Protocol::Codex,
        Protocol::ChatCompletions,
        Protocol::Messages,
        Protocol::Gemini,
    ] {
        let mut calls = Calls::new(protocol);
        match protocol {
            Protocol::Responses | Protocol::Codex => completed(&mut calls, vec![response_call("one", &args("src/new.rs", "valid")), response_call("two", "{\"path\":\"src/new.rs\"}")]).unwrap(),
            Protocol::ChatCompletions => {
                chat_fragment(&mut calls, json!([
                    {"index":0,"id":"one","type":"function","function":{"name":TOOL_NAME,"arguments":args("src/new.rs","valid")}},
                    {"index":1,"id":"two","type":"function","function":{"name":TOOL_NAME,"arguments":"{\"path\":\"src/new.rs\"}"}}
                ]), None).unwrap();
                chat_stop(&mut calls, "tool_calls").unwrap();
            }
            Protocol::Messages => {
                for index in 0..2 {
                    let input = if index == 0 { json!({"path":"src/new.rs","content":"valid"}) } else { json!({"path":"src/new.rs"}) };
                    message_block(&mut calls, index, json!({"type":"tool_use","id":format!("tool_{index}"),"name":TOOL_NAME,"input":input})).unwrap();
                    message_close(&mut calls, index);
                }
                message_finish(&mut calls);
            }
            Protocol::Gemini => gemini(&mut calls, vec![
                json!({"functionCall":{"name":TOOL_NAME,"args":{"path":"src/new.rs","content":"valid"}}}),
                json!({"functionCall":{"name":TOOL_NAME,"args":{"path":"src/new.rs"}}})
            ], true).unwrap(),
        }
        let decoded = calls.finish(true).unwrap();
        assert_eq!(decoded.len(), 2);
        assert!(!decoded[0].tool.invalid());
        assert!(decoded[1].tool.invalid());
        let mut body = json!({"input":[],"messages":[],"contents":[],"policy":"unchanged"});
        calls
            .append_results(&mut body, &[result("refused"), result("refused")])
            .unwrap();
        assert_eq!(body["policy"], "unchanged");
    }
}

#[test]
fn duplicate_ids_unknown_tools_and_unsupported_tool_types_are_rejected() {
    for protocol in [Protocol::Responses, Protocol::Codex] {
        let mut calls = Calls::new(protocol);
        completed(
            &mut calls,
            vec![response_call("same", &args("a", "a")), {
                let mut call = response_call("same", &args("b", "b"));
                call["id"] = json!("different_item");
                call
            }],
        )
        .unwrap();
        assert!(calls.finish(true).is_err());
        let mut calls = Calls::new(protocol);
        let mut unknown = response_call("one", &args("a", "a"));
        unknown["name"] = json!("shell");
        assert!(completed(&mut calls, vec![unknown]).is_err());
        assert!(
            completed(
                &mut Calls::new(protocol),
                vec![json!({"type":"web_search_call"})]
            )
            .is_err()
        );
    }
    let mut calls = Calls::new(Protocol::ChatCompletions);
    chat_fragment(&mut calls, json!([
        {"index":0,"id":"same","type":"function","function":{"name":TOOL_NAME,"arguments":args("a","a")}},
        {"index":1,"id":"same","type":"function","function":{"name":TOOL_NAME,"arguments":args("b","b")}}
    ]), None).unwrap();
    chat_stop(&mut calls, "tool_calls").unwrap();
    assert!(calls.finish(true).is_err());
    assert!(
        chat_fragment(
            &mut Calls::new(Protocol::ChatCompletions),
            json!([{"index":0,"type":"custom"}]),
            None
        )
        .is_err()
    );
    assert!(
        message_block(
            &mut Calls::new(Protocol::Messages),
            0,
            json!({"type":"server_tool_use","name":"shell"})
        )
        .is_err()
    );
    let mut calls = Calls::new(Protocol::Messages);
    for index in 0..2 {
        message_block(&mut calls, index, json!({"type":"tool_use","id":"same","name":TOOL_NAME,"input":{"path":"a","content":"a"}})).unwrap();
        message_close(&mut calls, index);
    }
    message_finish(&mut calls);
    assert!(calls.finish(true).is_err());
    let mut calls = Calls::new(Protocol::Gemini);
    gemini(&mut calls, vec![
        json!({"functionCall":{"id":"same","name":TOOL_NAME,"args":{"path":"a","content":"a"}}}),
        json!({"functionCall":{"id":"same","name":TOOL_NAME,"args":{"path":"b","content":"b"}}})
    ], true).unwrap();
    assert!(calls.finish(true).is_err());
    assert!(
        gemini(
            &mut Calls::new(Protocol::Gemini),
            vec![json!({"executableCode":{"code":"write()"}})],
            true
        )
        .is_err()
    );
}

#[test]
fn malformed_json_is_terminal_but_known_schema_errors_remain_actionable() {
    for invalid in [
        "[]",
        "[\"a\",\"b\"]",
        "null",
        "{\"path\":\"a\",\"path\":\"b\",\"content\":\"c\"}",
        "{\"path\":\"a\",\"content\":\"b\"} trailing",
    ] {
        let mut calls = Calls::new(Protocol::Responses);
        completed(&mut calls, vec![response_call("one", invalid)]).unwrap();
        assert!(calls.finish(true).is_err(), "accepted {invalid}");
    }
    for invalid in [
        "{}",
        "{\"path\":\"a\"}",
        "{\"path\":\"a\",\"content\":null}",
        "{\"path\":\"a\",\"content\":\"b\",\"extra\":true}",
    ] {
        let mut calls = Calls::new(Protocol::Responses);
        completed(&mut calls, vec![response_call("one", invalid)]).unwrap();
        assert!(calls.finish(true).unwrap()[0].tool.invalid());
    }
    for (path, content) in [
        ("".to_owned(), "a".to_owned()),
        ("a".repeat(PATH_LIMIT + 1), "b".into()),
        ("a".into(), "é".repeat(MAX_BYTES / 2 + 1)),
    ] {
        let mut calls = Calls::new(Protocol::Responses);
        completed(
            &mut calls,
            vec![response_call("one", &args(&path, &content))],
        )
        .unwrap();
        // Complete structurally valid arguments reach whole-batch semantic admission.
        assert!(calls.finish(true).is_ok());
    }
    let mut calls = Calls::new(Protocol::Responses);
    assert!(
        completed(
            &mut calls,
            vec![response_call(&"x".repeat(ID_LIMIT + 1), &args("a", "b"))]
        )
        .is_err()
    );
    let mut calls = Calls::new(Protocol::ChatCompletions);
    assert!(
        chat_fragment(
            &mut calls,
            json!([{"index":0,"type":"function","function":{"name":"x".repeat(NAME_LIMIT + 1)}}]),
            None
        )
        .is_err()
    );
    assert!(chat_fragment(&mut Calls::new(Protocol::ChatCompletions), json!([{"index":0,"type":"function","function":{"arguments":"x".repeat(ARGUMENT_LIMIT + 1)}}]), None).is_err());
}

#[test]
fn transcript_and_call_limits_refuse_without_mutating_original_body() {
    let mut calls = Calls::new(Protocol::Responses);
    let output: Vec<_> = (0..=CALL_LIMIT)
        .map(|i| response_call(&format!("call_{i}"), &args("a", "a")))
        .collect();
    completed(&mut calls, output).unwrap();
    assert!(calls.finish(true).is_err());
    let mut calls = Calls::new(Protocol::Gemini);
    assert!(
        gemini(
            &mut calls,
            vec![json!({"text":"x".repeat(BODY_LIMIT + 1),"thought":true})],
            false
        )
        .is_err()
    );
    let mut calls = Calls::new(Protocol::Responses);
    completed(&mut calls, vec![response_call("one", &args("a", "b"))]).unwrap();
    let mut body = json!({"input":[],"instructions":"x".repeat(BODY_LIMIT - 50)});
    let before = body.clone();
    assert!(
        calls
            .append_results(&mut body, &[result("success")])
            .is_err()
    );
    assert_eq!(body, before);
    assert!(calls.append_results(&mut json!({"input":[]}), &[]).is_err());
}

#[test]
fn text_only_rounds_and_legacy_text_events_return_no_calls() {
    let mut calls = Calls::new(Protocol::Responses);
    completed(&mut calls, vec![json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]})]).unwrap();
    completed(&mut calls, vec![json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]})]).unwrap();
    assert!(calls.finish(false).unwrap().is_empty());
    let mut calls = Calls::new(Protocol::ChatCompletions);
    calls
        .observe("", &json!({"choices":[{"delta":{"content":"answer"}}]}))
        .unwrap();
    chat_stop(&mut calls, "stop").unwrap();
    chat_stop(&mut calls, "stop").unwrap();
    assert!(calls.finish(false).unwrap().is_empty());
    let mut calls = Calls::new(Protocol::Messages);
    calls
        .observe(
            "content_block_delta",
            &json!({"delta":{"type":"text_delta","text":"answer"}}),
        )
        .unwrap();
    calls
        .observe(
            "content_block_delta",
            &json!({"delta":{"type":"thinking_delta","thinking":"private"}}),
        )
        .unwrap();
    calls.observe("message_stop", &json!({})).unwrap();
    calls.observe("message_stop", &json!({})).unwrap();
    assert!(calls.finish(false).unwrap().is_empty());
    let mut calls = Calls::new(Protocol::Gemini);
    gemini(&mut calls, vec![json!({"text":"answer"})], true).unwrap();
    calls
        .observe(
            "",
            &json!({"candidates":[{"finishReason":"STOP"}],"usageMetadata":{}}),
        )
        .unwrap();
    assert!(calls.finish(false).unwrap().is_empty());
}

#[test]
fn lifecycle_inconsistencies_cannot_be_promoted_to_effects() {
    let mut calls = Calls::new(Protocol::ChatCompletions);
    chat_fragment(&mut calls, json!([{"index":0,"id":"one","type":"function","function":{"name":TOOL_NAME,"arguments":""}}]), None).unwrap();
    assert!(chat_fragment(&mut calls, json!([{"index":0,"id":"changed"}]), None).is_err());
    let mut calls = Calls::new(Protocol::Messages);
    calls
        .observe(
            "content_block_delta",
            &json!({"delta":{"type":"text_delta","text":"legacy"}}),
        )
        .unwrap();
    assert!(
        message_block(
            &mut calls,
            0,
            json!({"type":"tool_use","id":"one","name":TOOL_NAME,"input":{}})
        )
        .is_err()
    );
    let mut calls = Calls::new(Protocol::Messages);
    message_block(
        &mut calls,
        0,
        json!({"type":"thinking","thinking":"","signature":""}),
    )
    .unwrap();
    assert!(
        message_delta(
            &mut calls,
            0,
            json!({"type":"input_json_delta","partial_json":"{}"})
        )
        .is_err()
    );
    assert!(calls.observe("message_stop", &json!({})).is_err());
    let mut calls = Calls::new(Protocol::Gemini);
    gemini(
        &mut calls,
        vec![json!({"functionCall":{"name":TOOL_NAME,"args":{"path":"a","content":"b"}}})],
        true,
    )
    .unwrap();
    assert!(
        calls
            .observe("", &json!({"candidates":[{"finishReason":"STOP"}]}))
            .is_err()
    );
}

#[test]
fn messages_tool_rounds_require_authoritative_successful_stop_reasons() {
    for reason in [
        None,
        Some("end_turn"),
        Some("stop_sequence"),
        Some("tool_use"),
    ] {
        let mut calls = Calls::new(Protocol::Messages);
        message_block(&mut calls, 0, json!({"type":"tool_use","id":"one","name":TOOL_NAME,"input":{"path":"src/new.rs","content":"new"}})).unwrap();
        message_close(&mut calls, 0);
        if let Some(reason) = reason {
            calls
                .observe("message_delta", &json!({"delta":{"stop_reason":reason}}))
                .unwrap();
        }
        calls.observe("message_stop", &json!({})).unwrap();
        assert_eq!(calls.finish(true).is_ok(), reason.is_some());
    }
    let mut calls = Calls::new(Protocol::Messages);
    calls
        .observe(
            "message_delta",
            &json!({"delta":{"stop_reason":"tool_use"}}),
        )
        .unwrap();
    calls.observe("message_stop", &json!({})).unwrap();
    assert!(calls.finish(true).is_err());
}

#[test]
fn terminal_content_and_legacy_thinking_cannot_hide_later_calls() {
    let mut calls = Calls::new(Protocol::Messages);
    calls
        .observe(
            "content_block_delta",
            &json!({"delta":{"type":"thinking_delta","thinking":"private"}}),
        )
        .unwrap();
    assert!(
        message_block(
            &mut calls,
            0,
            json!({"type":"tool_use","id":"one","name":TOOL_NAME,"input":{}})
        )
        .is_err()
    );
    let mut calls = Calls::new(Protocol::Messages);
    calls.observe("message_stop", &json!({})).unwrap();
    assert!(
        calls
            .observe(
                "content_block_delta",
                &json!({"delta":{"type":"text_delta","text":"late"}})
            )
            .is_err()
    );
    assert!(
        calls
            .observe("message_start", &json!({"message":{"content":[]}}))
            .is_err()
    );
    assert!(
        calls
            .observe(
                "message_delta",
                &json!({"delta":{"stop_reason":"tool_use"}})
            )
            .is_err()
    );
    for protocol in [Protocol::Responses, Protocol::Codex] {
        let mut calls = Calls::new(protocol);
        completed(&mut calls, vec![]).unwrap();
        for kind in ["response.output_text.delta", "response.refusal.delta"] {
            assert!(calls.observe(kind, &json!({"delta":"late"})).is_err());
        }
    }
}

#[test]
fn successful_messages_and_gemini_results_are_not_error_envelopes() {
    let mut calls = Calls::new(Protocol::Messages);
    message_block(&mut calls, 0, json!({"type":"tool_use","id":"one","name":TOOL_NAME,"input":{"path":"src/new.rs","content":"new"}})).unwrap();
    message_close(&mut calls, 0);
    message_finish(&mut calls);
    let mut body = json!({"messages":[]});
    calls
        .append_results(&mut body, &[result("success")])
        .unwrap();
    assert_eq!(body["messages"][1]["content"][0]["is_error"], false);
    let mut calls = Calls::new(Protocol::Gemini);
    gemini(
        &mut calls,
        vec![
            json!({"functionCall":{"name":TOOL_NAME,"args":{"path":"src/new.rs","content":"new"}}}),
        ],
        true,
    )
    .unwrap();
    let mut body = json!({"contents":[]});
    calls
        .append_results(&mut body, &[result("success")])
        .unwrap();
    assert_eq!(
        body["contents"][1]["parts"][0]["functionResponse"]["response"],
        json!({"output":result("success")})
    );
}

#[test]
fn every_dialect_refuses_unknown_function_names() {
    let mut calls = Calls::new(Protocol::ChatCompletions);
    chat_fragment(&mut calls, json!([{"index":0,"id":"one","type":"function","function":{"name":"shell","arguments":args("a","b")}}]), None).unwrap();
    chat_stop(&mut calls, "tool_calls").unwrap();
    assert!(calls.finish(true).is_err());
    assert!(
        message_block(
            &mut Calls::new(Protocol::Messages),
            0,
            json!({"type":"tool_use","id":"one","name":"shell","input":{"path":"a","content":"b"}})
        )
        .is_err()
    );
    assert!(
        gemini(
            &mut Calls::new(Protocol::Gemini),
            vec![json!({"functionCall":{"name":"shell","args":{"path":"a","content":"b"}}})],
            true
        )
        .is_err()
    );
}

#[test]
fn responses_authoritative_call_status_must_be_complete_when_present() {
    for protocol in [Protocol::Responses, Protocol::Codex] {
        for status in [
            None,
            Some("completed"),
            Some("in_progress"),
            Some("incomplete"),
            Some("failed"),
        ] {
            let mut calls = Calls::new(protocol);
            let mut call = response_call("one", &args("src/new.rs", "new"));
            if let Some(status) = status {
                call["status"] = json!(status);
            } else {
                call.as_object_mut().unwrap().remove("status");
            }
            completed(&mut calls, vec![call]).unwrap();
            assert_eq!(
                calls.finish(true).is_ok(),
                status.is_none() || status == Some("completed")
            );
        }
    }
}

#[test]
fn complete_fragmented_chat_calls_accept_stop_but_never_truncated_json() {
    for complete in [false, true] {
        let mut calls = Calls::new(Protocol::ChatCompletions);
        chat_fragment(&mut calls, json!([{"index":0,"id":"one","type":"function","function":{"name":TOOL_NAME,"arguments":"{\"path\":\"new.txt\","}}]), None).unwrap();
        chat_fragment(&mut calls, json!([{"index":0,"function":{"arguments":if complete { "\"content\":\"complete\"}" } else { "\"content\":\"incomplete" }}}]), None).unwrap();
        chat_stop(&mut calls, "stop").unwrap();
        assert_eq!(calls.finish(true).is_ok(), complete);
        let mut body = json!({"messages":[]});
        assert_eq!(
            calls
                .append_results(&mut body, &[result("refused")])
                .is_ok(),
            complete
        );
        if !complete {
            assert_eq!(body, json!({"messages":[]}));
        }
    }
}

#[test]
fn successful_messages_stop_labels_cannot_promote_truncated_arguments() {
    for reason in ["tool_use", "end_turn", "stop_sequence"] {
        let mut calls = Calls::new(Protocol::Messages);
        message_block(
            &mut calls,
            0,
            json!({"type":"tool_use","id":"one","name":TOOL_NAME,"input":{}}),
        )
        .unwrap();
        message_delta(&mut calls, 0, json!({"type":"input_json_delta","partial_json":"{\"path\":\"new.txt\",\"content\":\"unfinished"})).unwrap();
        message_close(&mut calls, 0);
        calls
            .observe("message_delta", &json!({"delta":{"stop_reason":reason}}))
            .unwrap();
        calls.observe("message_stop", &json!({})).unwrap();
        assert!(calls.finish(true).is_err());
        let mut body = json!({"messages":[]});
        assert!(
            calls
                .append_results(&mut body, &[result("refused")])
                .is_err()
        );
        assert_eq!(body, json!({"messages":[]}));
    }
}

#[test]
fn fully_escaped_sixty_four_kib_content_fits_arguments_without_truncation() {
    let escaped = format!(
        "{{\"path\":\"new.txt\",\"content\":\"{}\"}}",
        "\\u0078".repeat(MAX_BYTES)
    );
    let mut calls = Calls::new(Protocol::ChatCompletions);
    chat_fragment(&mut calls, json!([{"index":0,"id":"one","type":"function","function":{"name":TOOL_NAME,"arguments":escaped}}]), None).unwrap();
    chat_stop(&mut calls, "stop").unwrap();
    let decoded = calls.finish(true).unwrap();
    assert_eq!(write_content(&decoded[0]), "x".repeat(MAX_BYTES));
}

#[test]
fn wire_json_preserves_unique_keys_in_whole_arguments_and_terminal_metadata() {
    for invalid in [
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_reason":"max_tokens"}}"#,
        r#"{"content_block":{"type":"tool_use","id":"one","name":"eevee_write_file","input":{"path":"one.txt","path":"two.txt","content":"new"}}}"#,
        r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"eevee_write_file","args":{"path":"one.txt","content":"first","content":"second"}}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"stop","finish_reason":"length"}]}"#,
        r#"{"complete":true} trailing"#,
    ] {
        assert!(frame(invalid).is_err());
    }
    let valid = r#"{"text":"こんにちは","usage":{"input_tokens":1,"output_tokens":2},"parts":[null,true,0,1.5]}"#;
    assert_eq!(
        frame(valid).unwrap(),
        serde_json::from_str::<Value>(valid).unwrap()
    );
}

#[test]
fn mixed_file_tool_identities_survive_every_protocol_continuation() {
    let typed = [
        (
            "eevee_list_files",
            json!({"directory":"","query":"src","offset":0}),
            ToolKind::List,
        ),
        (
            "eevee_read_file",
            json!({"path":"src/lib.rs","start_line":1,"end_line":0}),
            ToolKind::Read,
        ),
        (
            "eevee_edit_file",
            json!({"path":"src/lib.rs","sha256":"a".repeat(64),"replacements":[{"old_text":"old","new_text":"new"}]}),
            ToolKind::Edit,
        ),
        (
            TOOL_NAME,
            json!({"path":"new.rs","content":"generated"}),
            ToolKind::Write,
        ),
        (
            "eevee_structural_search",
            json!({"directory":"src","language":"rust","pattern":"call($X,$X)","offset":0,"snapshot":""}),
            ToolKind::StructuralSearch,
        ),
    ];
    for protocol in [
        Protocol::Responses,
        Protocol::Codex,
        Protocol::ChatCompletions,
        Protocol::Messages,
        Protocol::Gemini,
    ] {
        let mut calls = Calls::new(protocol);
        match protocol {
            Protocol::Responses | Protocol::Codex => completed(&mut calls, typed.iter().enumerate().map(|(index,(name,args,_))| {
                json!({"type":"function_call","call_id":format!("call-{index}"),"name":name,"arguments":args.to_string(),"status":"completed"})
            }).collect()).unwrap(),
            Protocol::ChatCompletions => {
                chat_fragment(&mut calls, Value::Array(typed.iter().enumerate().map(|(index,(name,args,_))| {
                    json!({"index":index,"id":format!("call-{index}"),"type":"function","function":{"name":name,"arguments":args.to_string()}})
                }).collect()), None).unwrap();
                chat_stop(&mut calls, "tool_calls").unwrap();
            }
            Protocol::Messages => {
                for (index,(name,args,_)) in typed.iter().enumerate() {
                    message_block(&mut calls, index, json!({"type":"tool_use","id":format!("call-{index}"),"name":name,"input":args})).unwrap();
                    message_close(&mut calls, index);
                }
                message_finish(&mut calls);
            }
            Protocol::Gemini => gemini(&mut calls, typed.iter().enumerate().map(|(index,(name,args,_))| {
                json!({"functionCall":{"id":format!("call-{index}"),"name":name,"args":args},"thoughtSignature":format!("signature-{index}")})
            }).collect(), true).unwrap(),
        }
        let decoded = calls.finish(true).unwrap();
        for (index, call) in decoded.iter().enumerate() {
            assert_eq!(call.tool.kind(), typed[index].2);
            assert_eq!(call.id, Some(format!("call-{index}")));
        }
        let mut body = json!({"input":[],"messages":[],"contents":[]});
        let results: Vec<_> = (0..typed.len())
            .map(|index| {
                json!({
                    "status":if index == 2 { "refused" } else { "success" },
                    "path":decoded[index].tool.path(),"message":format!("actual outcome {index}")
                })
            })
            .collect();
        calls.append_results(&mut body, &results).unwrap();
        match protocol {
            Protocol::Responses | Protocol::Codex => {
                for index in 0..typed.len() {
                    assert_eq!(
                        body["input"][index + typed.len()]["call_id"],
                        format!("call-{index}")
                    );
                }
            }
            Protocol::ChatCompletions => {
                for index in 0..typed.len() {
                    assert_eq!(
                        body["messages"][index + 1]["tool_call_id"],
                        format!("call-{index}")
                    );
                }
            }
            Protocol::Messages => {
                for index in 0..typed.len() {
                    assert_eq!(
                        body["messages"][1]["content"][index]["tool_use_id"],
                        format!("call-{index}")
                    );
                }
            }
            Protocol::Gemini => {
                for (index, (name, _, _)) in typed.iter().enumerate() {
                    let response = &body["contents"][1]["parts"][index]["functionResponse"];
                    assert_eq!(response["name"], *name);
                    assert_eq!(response["id"], format!("call-{index}"));
                    assert_eq!(
                        body["contents"][0]["parts"][index]["thoughtSignature"],
                        format!("signature-{index}")
                    );
                }
            }
        }
    }
}

#[test]
fn compatible_chat_variants_keep_calls_and_continuation_metadata() {
    // Unindexed, typeless, null-padded and repeated-name fragments from
    // OpenAI-compatible services still assemble one exact call per ID.
    let first = args("src/a.rs", "a");
    let second = args("src/b.rs", "b");
    let signature = json!({"google":{"thought_signature":"signed"}});
    let mut calls = Calls::new(Protocol::ChatCompletions);
    calls
        .observe("", &json!({"choices":[{"index":0,"delta":{"role":"assistant","content":null,"reasoning_content":"private ","function_call":null,"tool_calls":null}}]}))
        .unwrap();
    chat_fragment(&mut calls, json!([
        {"id":"one","function":{"name":TOOL_NAME,"arguments":&first[..6]},"extra_content":signature}
    ]), None).unwrap();
    chat_fragment(
        &mut calls,
        json!([
            {"id":"","type":null,"function":{"name":TOOL_NAME,"arguments":&first[6..]}}
        ]),
        None,
    )
    .unwrap();
    calls
        .observe(
            "",
            &json!({"choices":[{"index":0,"delta":{"reasoning_content":"plan"}}]}),
        )
        .unwrap();
    chat_fragment(
        &mut calls,
        json!([
            {"id":"two","type":"function","function":{"name":TOOL_NAME,"arguments":second}}
        ]),
        None,
    )
    .unwrap();
    chat_stop(&mut calls, "tool_calls").unwrap();
    let decoded = calls.finish(true).unwrap();
    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].id.as_deref(), Some("one"));
    assert_eq!(write_content(&decoded[0]), "a");
    assert_eq!(decoded[1].id.as_deref(), Some("two"));
    let mut body = json!({"messages":[]});
    calls
        .append_results(&mut body, &[result("denied"), result("denied")])
        .unwrap();
    assert_eq!(
        body["messages"][0],
        json!({"role":"assistant","content":null,"reasoning_content":"private plan","tool_calls":[
            {"id":"one","type":"function","function":{"name":TOOL_NAME,"arguments":first},"extra_content":signature},
            {"id":"two","type":"function","function":{"name":TOOL_NAME,"arguments":second}}
        ]})
    );

    // Services that omit call IDs receive position-derived correlation IDs.
    let mut calls = Calls::new(Protocol::ChatCompletions);
    chat_fragment(&mut calls, json!([
        {"index":0,"type":"function","function":{"name":TOOL_NAME,"arguments":args("src/a.rs","a")}}
    ]), None).unwrap();
    chat_stop(&mut calls, "stop").unwrap();
    assert_eq!(calls.finish(true).unwrap()[0].id.as_deref(), Some("call_0"));
    let mut body = json!({"messages":[]});
    calls
        .append_results(&mut body, &[result("denied")])
        .unwrap();
    assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "call_0");
    assert!(body["messages"][0].get("reasoning_content").is_none());
    assert_eq!(body["messages"][1]["tool_call_id"], "call_0");

    // Present but non-function types and real legacy calls still refuse.
    assert!(
        chat_fragment(
            &mut Calls::new(Protocol::ChatCompletions),
            json!([{"type":"custom"}]),
            None
        )
        .is_err()
    );
    assert!(
        Calls::new(Protocol::ChatCompletions)
            .observe(
                "",
                &json!({"choices":[{"index":0,"delta":{"function_call":{"name":TOOL_NAME}}}]})
            )
            .is_err()
    );
}

#[test]
fn gemini_partless_finish_and_argumentless_calls_are_tolerated() {
    let mut calls = Calls::new(Protocol::Gemini);
    calls
        .observe(
            "",
            &json!({"candidates":[{"index":0,"content":{"role":"model"},"finishReason":"STOP"}]}),
        )
        .unwrap();
    assert!(calls.finish(true).unwrap().is_empty());
    let mut calls = Calls::new(Protocol::Gemini);
    gemini(
        &mut calls,
        vec![json!({"functionCall":{"name":"eevee_artifact_list"}})],
        true,
    )
    .unwrap();
    let decoded = calls.finish(true).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].tool.kind().name(), "eevee_artifact_list");
}

#[test]
fn unfinished_file_lines_advance_each_frame_without_waiting_for_a_newline() {
    let mut preview = FilePreview::new();
    let mut wire = String::from(r#"{"path":"src/main.rs","content":"fn main() { "#);
    preview.feed(TOOL_NAME, &wire, false);
    let first = preview.snapshot().unwrap().1;
    assert!(first.contains("fn main() {"));
    // A same-frame token is coalesced rather than rendered immediately.
    wire.push_str("let ");
    preview.feed(TOOL_NAME, &wire, false);
    assert!(preview.snapshot().is_none());
    // Once a frame has elapsed, the unfinished line moves without a newline.
    preview.published_at = Some(Instant::now() - PREVIEW_INTERVAL);
    wire.push_str("value ");
    preview.feed(TOOL_NAME, &wire, false);
    let (phase, text) = preview.snapshot().unwrap();
    assert_eq!(phase, FilePhase::Generating);
    assert!(text.contains("fn main() { let value"));
    // No new bytes means no repeated frame.
    preview.published_at = Some(Instant::now() - PREVIEW_INTERVAL);
    preview.feed(TOOL_NAME, &wire, false);
    assert!(preview.snapshot().is_none());
}
