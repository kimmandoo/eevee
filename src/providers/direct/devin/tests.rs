use super::*;

fn call(id: &str, name: &str, arguments: &str) -> Encode {
    let mut call = Encode::default();
    call.text(1, id);
    call.text(2, name);
    call.text(3, arguments);
    call
}
fn delta(call: &Encode) -> Encode {
    let mut message = Encode::default();
    message.bytes(6, &call.0);
    message
}
fn ended(state: &mut Response, stream: &mut Stream) -> Result<(), String> {
    trailer(b"{}")?;
    state.ended = true;
    state.finish(stream)
}
#[test]
fn connect_split_headers_truncation_flags_and_lengths_are_bounded() {
    let encoded = frame(b"actual protobuf bytes");
    for cut in 0..=encoded.len() {
        let mut decoder = Frames::default();
        let mut frames = decoder.feed(&encoded[..cut]).unwrap();
        frames.extend(decoder.feed(&encoded[cut..]).unwrap());
        decoder.finish().unwrap();
        assert_eq!(frames, vec![(0, b"actual protobuf bytes".to_vec())]);
    }
    let mut decoder = Frames::default();
    decoder.feed(&encoded[..encoded.len() - 1]).unwrap();
    assert!(decoder.finish().is_err());
    assert!(Frames::default().feed(&[4, 0, 0, 0, 0]).is_err());
    assert!(Frames::default().feed(&[0, 1, 0, 0, 1]).is_err());
}
#[test]
fn protobuf_rejects_overflow_truncation_invalid_utf8_and_duplicate_scalar() {
    assert!(Message(&[0x80; 11]).text(1).is_err());
    assert!(Message(&[10, 2, 1]).text(1).is_err());
    assert!(Message(&[10, 1, 255]).text(1).is_err());
    assert!(Message(&[10, 1, b'a', 10, 1, b'b']).text(1).is_err());
    assert!(Message(&[0]).text(1).is_err());
    assert!(Message(&[8, 1]).text(1).is_err());
}
#[test]
fn unary_accepts_plain_message_frames_and_rejects_late_errors_or_multiple_messages() {
    let mut encoded = frame(&[10, 1, b'x']);
    let mut end = frame(b"{}");
    end[0] = 2;
    encoded.extend(end);
    assert_eq!(decode_unary(&encoded).unwrap(), [10, 1, b'x']);
    let mut double = frame(b"one");
    double.extend(frame(b"two"));
    assert!(decode_unary(&double).is_err());
    let mut error = frame(b"message");
    let mut end = frame(br#"{"error":{"code":"permission_denied","message":"untrusted secret"}}"#);
    end[0] = 2;
    error.extend(end);
    let error = decode_unary(&error).unwrap_err();
    assert!(!error.contains("untrusted secret"));
    assert!(trailer(b"not-json").is_err());
}
#[test]
fn exact_catalog_models_filter_only_disabled_internal_and_unservable_pairings() {
    let mut response = Encode::default();
    for (id, display, disabled, router) in [
        ("exact-high-wire", 8, 0, 0),
        ("exact-low-wire", 0, 0, 0),
        ("adaptive", 3, 0, 1),
        ("hidden", 6, 0, 0),
        ("review", 4, 0, 0),
        ("disabled", 8, 1, 0),
    ] {
        let mut config = Encode::default();
        config.text(22, id);
        config.text(1, id);
        config.number(4, disabled);
        let mut info = Encode::default();
        info.number(22, display);
        info.number(25, router);
        config.bytes(23, &info.0);
        response.bytes(1, &config.0);
    }
    let models = models(&response.0).unwrap();
    assert_eq!(
        models
            .iter()
            .map(|model| model.option.id.as_str())
            .collect::<Vec<_>>(),
        ["exact-high-wire", "exact-low-wire", "adaptive"]
    );
    assert!(models.iter().all(|model| model.option.efforts.is_empty()));
    assert!(models[2].router);
}
#[test]
fn cumulative_and_incremental_tool_fragments_retain_actual_text_thinking_and_signature() {
    let mut state = Response::default();
    let mut stream = Stream::new(Protocol::Devin);
    let mut first = delta(&call(
        "native_call",
        "eevee_write_file",
        r#"{"path":"actual.txt","content":"#,
    ));
    first.text(3, "Public answer");
    first.text(9, "private reasoning");
    first.text(10, "opaque-signature");
    state.message(&first.0, &mut stream).unwrap();
    state
        .message(
            &delta(&call("", "", r#"{"path":"actual.txt","content":"actual"}"#)).0,
            &mut stream,
        )
        .unwrap();
    ended(&mut state, &mut stream).unwrap();
    let calls = stream.calls.finish(true).unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id.as_deref(), Some("native_call"));
    let mut body = json!({"messages":[]});
    stream
        .calls
        .append_results(
            &mut body,
            &[json!({"status":"success","message":"Actual local result","path":"actual.txt"})],
        )
        .unwrap();
    let assistant = &body["messages"][0];
    assert_eq!(assistant["reasoning_content"], "private reasoning");
    assert_eq!(assistant["devin_signature"], "opaque-signature");
    let translated = prompt(assistant, "fixture-cascade", 0).unwrap();
    assert_eq!(Message(&translated.0).text(12).unwrap(), "opaque-signature");
    assert_eq!(stream.text, "Public answer");
}
#[test]
fn malformed_or_duplicate_key_arguments_never_reach_host_effects() {
    for args in [
        r#"{"path":"a","content":"truncated"#,
        r#"{"path":"a","path":"b","content":"x"}"#,
    ] {
        let mut state = Response::default();
        let mut stream = Stream::new(Protocol::Devin);
        state
            .message(
                &delta(&call("call_one", "eevee_write_file", args)).0,
                &mut stream,
            )
            .unwrap();
        ended(&mut state, &mut stream).unwrap();
        assert!(stream.calls.finish(true).is_err());
    }
    let mut state = Response::default();
    let mut stream = Stream::new(Protocol::Devin);
    assert!(
        state
            .message(&delta(&call("", "", "{}")).0, &mut stream)
            .is_err()
    );
    state
        .message(
            &delta(&call("call_one", "eevee_write_file", "{}")).0,
            &mut stream,
        )
        .unwrap();
    assert!(
        state
            .message(
                &delta(&call("call_one", "different_tool", "{}")).0,
                &mut stream
            )
            .is_err()
    );
}
#[test]
fn missing_completion_and_failed_stop_never_promote_tool_authority() {
    let mut state = Response::default();
    let mut stream = Stream::new(Protocol::Devin);
    state
        .message(
            &delta(&call(
                "call_one",
                "eevee_write_file",
                r#"{"path":"a","content":"x"}"#,
            ))
            .0,
            &mut stream,
        )
        .unwrap();
    assert!(state.finish(&mut stream).is_err());
    assert!(stream.calls.finish(true).is_err());
    state.ended = true;
    state.stop = 3;
    assert!(state.finish(&mut stream).is_err());
    assert!(stream.failed);
}
#[test]
fn gzip_decoding_is_real_and_bounded() {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(b"actual compressed frame").unwrap();
    let bytes = encoder.finish().unwrap();
    assert_eq!(
        decompress(1, &bytes).unwrap().as_ref(),
        b"actual compressed frame"
    );
    assert!(decompress(1, b"not gzip").is_err());
}

#[test]
fn google_nullable_schema_keeps_properties_and_all_union_options() {
    let mut schema = json!({"type":"object","properties":{
        "nullable":{"type":["string","null"]},
        "union":{"type":["string","integer"],"description":"literal"},
        "literal":{"type":"object","default":{"type":["this is data"]}}
    }});
    google_schema(&mut schema).unwrap();
    assert_eq!(schema["properties"]["nullable"]["type"], "string");
    assert_eq!(schema["properties"]["nullable"]["nullable"], true);
    assert_eq!(
        schema["properties"]["union"]["allOf"][0]["anyOf"],
        json!([{"type":"string"},{"type":"integer"}])
    );
    assert_eq!(
        schema["properties"]["literal"]["default"]["type"],
        json!(["this is data"])
    );
}

#[test]
fn usage_and_opaque_binary_reasoning_signature_survive_without_public_thinking() {
    let mut state = Response::default();
    let mut stream = Stream::new(Protocol::Devin);
    let mut message = delta(&call(
        "signature_call",
        "eevee_write_file",
        r#"{"path":"x","content":"y"}"#,
    ));
    message.text(9, "private");
    message.bytes(20, &[0, 255, 1]);
    message.text(21, "native-signature-kind");
    let mut usage = Encode::default();
    for (field, value) in [(2, 100), (3, 20), (4, 3), (5, 4)] {
        usage.number(field, value);
    }
    message.bytes(7, &usage.0);
    state.message(&message.0, &mut stream).unwrap();
    ended(&mut state, &mut stream).unwrap();
    assert_eq!(
        stream.usage,
        Some(Counters {
            input: Some(100),
            output: Some(20),
            cache_write: Some(3),
            cache_read: Some(4),
            reasoning: None
        })
    );
    let mut body = json!({"messages":[]});
    stream
        .calls
        .append_results(
            &mut body,
            &[json!({"status":"success","path":"x","message":"actual"})],
        )
        .unwrap();
    let prompt = prompt(&body["messages"][0], "cascade", 0).unwrap();
    assert_eq!(
        Message(&prompt.0).bytes(17).unwrap(),
        Some([0, 255, 1].as_slice())
    );
    assert_eq!(
        Message(&prompt.0).text(18).unwrap(),
        "native-signature-kind"
    );
    assert!(stream.text.is_empty());
}

#[test]
fn server_directed_destinations_must_match_the_entire_reviewed_base() {
    let reviewed = "https://reviewed.example/native";
    assert!(reviewed_route(reviewed, "").is_ok());
    assert!(reviewed_route(reviewed, "https://reviewed.example/native/").is_ok());
    for unreviewed in [
        "https://foreign.example/native",
        "https://reviewed.example/other-path",
        "https://reviewed.example:8443/native",
        "http://reviewed.example/native",
        "https://user:private@foreign.example/native",
    ] {
        let error = reviewed_route(reviewed, unreviewed).unwrap_err();
        assert!(!error.contains("foreign.example"));
        assert!(!error.contains("reviewed.example"));
        assert!(!error.contains("private"));
    }
}
