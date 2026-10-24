use super::*;

fn handle() -> (Handle, SyncSender<Result<Value, String>>) {
    let (sender, reply) = mpsc::sync_channel(1);
    (
        Handle {
            cancellation: Cancellation::new(),
            reply,
            result: None,
            settled: false,
            reply_received: false,
            output_limit: RETAIN_LIMIT,
            output: None,
            output_read: None,
            waiting: Vec::new(),
            released: false,
            created: Instant::now(),
            cancellation_started: None,
        },
        sender,
    )
}
fn result(code: i32) -> Value {
    json!({"run":{"run_id":"immutable-run","ended_at_ms":10,"state":if code == 0 {"Succeeded"} else {"Failed"},"exit_code":code,"signal":null,"tree":"OwnedTreeExited","captured_bytes":7,"discarded_bytes":0,"truncated":false},"artifact":{"id":"artifact","revision":"frozen"},"output_unavailable":false})
}

#[test]
fn implement_and_exact_session_are_required_independent_of_agent_profile() {
    for agent in [Agent::OpenCode, Agent::Gemini, Agent::Devin] {
        for operation in [Operation::Plan, Operation::Review, Operation::Implement] {
            let mut input = crate::providers::tests::claimed(
                agent,
                operation,
                operation == Operation::Implement,
            );
            let params = json!({"sessionId":"owned"});
            assert_eq!(
                terminal_scope(&input, Some("owned"), &params).is_ok(),
                operation == Operation::Implement
            );
            assert!(terminal_scope(&input, Some("another"), &params).is_err());
            assert!(terminal_scope(&input, None, &params).is_err());
            input.inspect_sessions = true;
            assert!(terminal_scope(&input, Some("owned"), &params).is_err());
            input.inspect_sessions = false;
            input.inspect_models = true;
            assert!(terminal_scope(&input, Some("owned"), &params).is_err());
        }
    }
    let mut input = crate::providers::tests::claimed(Agent::OpenCode, Operation::Implement, true);
    input.work.as_mut().unwrap().writing = false;
    assert!(terminal_scope(&input, Some("owned"), &json!({"sessionId":"owned"})).is_err());
}

#[test]
fn literal_protocol_argv_and_safe_cwd_never_guess_shell_or_ignore_env_pty() {
    let root = std::env::temp_dir().join("eevee-acp-root");
    let params = json!({"sessionId":"s","command":"rustc","args":["main.rs","-o","program"],"cwd":root,"env":[],"outputByteLimit":10});
    let (command, cwd, limit) = create_spec(&root, &params).unwrap();
    assert_eq!(
        command,
        CommandSpec::Direct {
            program: "rustc".into(),
            args: vec!["main.rs".into(), "-o".into(), "program".into()]
        }
    );
    assert!(cwd.as_os_str().is_empty());
    assert_eq!(limit, 10);
    let mut relative = params.clone();
    relative["cwd"] = json!("src");
    assert_eq!(
        create_spec(&root, &relative).unwrap().1,
        PathBuf::from("src")
    );
    for change in [
        json!({"env":[{"name":"SPECIAL","value":"1"}]}),
        json!({"pty":true}),
        json!({"_meta":{"pty":true}}),
        json!({"cwd":"../outside"}),
        json!({"cwd":"src/../outside"}),
        json!({"cwd":root.parent().unwrap()}),
        json!({"outputByteLimit":65537}),
        json!({"command":""}),
        json!({"args":["secret\0argument"]}),
    ] {
        let mut invalid = params.clone();
        for (key, value) in change.as_object().unwrap() {
            invalid[key] = value.clone();
        }
        assert!(create_spec(&root, &invalid).is_err(), "{invalid}");
    }
    let mut shell = params;
    shell["command"] = json!("sh");
    shell["args"] = json!(["-c", "printf 'literal command'"]);
    assert!(matches!(
        create_spec(&root, &shell).unwrap().0,
        CommandSpec::Direct { .. }
    ));
}

#[test]
fn allocation_and_pending_output_are_not_execution_or_exit_evidence() {
    let (handle, _sender) = handle();
    let output = handle.output().unwrap();
    assert_eq!(output["output"], "");
    assert_eq!(output["truncated"], false);
    assert!(output.get("exitStatus").is_none());
    assert_eq!(output["_meta"]["eevee.dev/terminal"]["state"], "pending");
    assert!(handle.exited().is_err());
    assert!(handle.action(Action::Kill).is_err());
    assert!(handle.action(Action::Release).is_err());
}

#[test]
fn kill_release_and_drop_cancel_only_the_exact_owned_callback() {
    let (mut first, _sender) = handle();
    let (second, _other_sender) = handle();
    let flag = first.cancellation.0.clone();
    first.cancel();
    assert!(flag.load(Ordering::Acquire));
    assert!(!second.cancellation.0.load(Ordering::Acquire));
    assert!(
        first.action(Action::Kill).is_err(),
        "flag alone is not a successful kill"
    );
    let other_flag = second.cancellation.0.clone();
    drop(second);
    assert!(
        other_flag.load(Ordering::Acquire),
        "EOF/drop must cancel its owned host run"
    );
}

#[test]
fn callback_reply_and_exact_exit_status_are_not_inferred_from_peer_success() {
    let (mut handle, sender) = handle();
    handle.waiting.push(Waiting {
        id: json!("correlated-wait"),
        action: Action::Wait,
    });
    handle.poll();
    assert!(!handle.settled);
    sender.send(Ok(result(7))).unwrap();
    handle.poll();
    assert!(handle.settled);
    assert_eq!(handle.waiting[0].id, "correlated-wait");
    assert_eq!(
        handle.exited().unwrap(),
        json!({"exitCode":7,"signal":null})
    );
    assert_eq!(handle.action(Action::Kill).unwrap(), json!({}));
    handle.result.as_mut().unwrap().as_mut().unwrap()["run"]["tree"] = json!("OwnershipUnknown");
    assert!(handle.exited().is_err());
    assert!(handle.action(Action::Kill).is_err());
    assert!(handle.action(Action::Release).is_err());
}

#[test]
fn denied_disconnected_and_unknown_callbacks_never_manufacture_exit_zero_or_release() {
    let (mut denied, sender) = handle();
    sender.send(Err("Denied before execution".into())).unwrap();
    denied.poll();
    assert!(denied.settled);
    assert!(denied.output().is_err());
    assert!(denied.exited().is_err());
    let (mut lost, sender) = handle();
    drop(sender);
    lost.poll();
    assert!(!lost.settled);
    assert!(lost.output().unwrap_err().contains("unknown"));
    let mut terminals = Terminals::default();
    terminals
        .handles
        .insert("exact-generation-terminal".into(), lost);
    assert!(
        terminals.busy(),
        "an unknown outcome must continue blocking writes/commands"
    );
    assert!(
        !terminals
            .handles
            .contains_key("another-generation-terminal")
    );
    terminals
        .handles
        .get_mut("exact-generation-terminal")
        .unwrap()
        .released = true;
    assert!(
        terminals
            .handles
            .get("exact-generation-terminal")
            .filter(|handle| !handle.released)
            .is_none()
    );
    assert!(
        terminals.busy(),
        "release cannot clear uncertain command ownership"
    );
    let (mut uncertain, sender) = handle();
    let mut unknown = result(0);
    unknown["run"]["tree"] = json!("OwnershipUnknown");
    sender.send(Ok(unknown)).unwrap();
    uncertain.poll();
    assert!(!uncertain.settled);
    assert!(uncertain.cancellation.0.load(Ordering::Acquire));
    uncertain.poll();
    assert!(
        uncertain.result.as_ref().unwrap().as_ref().unwrap()["run"]["run_id"].is_string(),
        "unknown retained run identity must not disappear on another poll"
    );
    let (mut state_unknown, sender) = handle();
    let mut unknown = result(0);
    unknown["run"]["state"] = json!("OutcomeUnknown");
    sender.send(Ok(unknown)).unwrap();
    state_unknown.poll();
    assert!(
        !state_unknown.settled,
        "an exited tree does not override an explicitly unknown execution state"
    );
    assert!(state_unknown.cancellation.0.load(Ordering::Acquire));
    assert!(state_unknown.exited().is_err());
}

#[test]
fn tail_output_uses_exact_durable_artifact_and_utf8_boundary_without_live_preview() {
    use crate::workspace::artifacts::{Handle as Artifact, Response, Slice};
    let (mut handle, sender) = handle();
    sender.send(Ok(result(0))).unwrap();
    handle.poll();
    handle.output_limit = 3;
    let events = Arc::new(Mutex::new(VecDeque::new()));
    handle.poll_output(&events, 9, 12).unwrap();
    let Event::ArtifactLookup {
        generation,
        task_id,
        operation,
        reply,
    } = events.lock().pop_front().unwrap()
    else {
        panic!("Expected artifact read");
    };
    assert_eq!((generation, task_id), (9, 12));
    let crate::workspace::artifacts::Operation::Read {
        handle: identity,
        offset,
        limit,
    } = operation
    else {
        panic!("Expected exact bounded read");
    };
    assert_eq!(
        identity,
        Artifact {
            id: "artifact".into(),
            revision: "frozen".into()
        }
    );
    assert_eq!((offset, limit), (4, 3));
    reply
        .send(Err("Artifact offset is not a UTF-8 boundary".into()))
        .unwrap();
    handle.poll_output(&events, 9, 12).unwrap();
    let Event::ArtifactLookup {
        operation, reply, ..
    } = events.lock().pop_front().unwrap()
    else {
        panic!("Expected boundary retry");
    };
    let crate::workspace::artifacts::Operation::Read { offset, limit, .. } = operation else {
        panic!("Expected read");
    };
    assert_eq!((offset, limit), (5, 2));
    reply
        .send(Ok(Response::Page(Slice {
            handle: identity,
            offset,
            text: "ab".into(),
            next_offset: None,
        })))
        .unwrap();
    handle.poll_output(&events, 9, 12).unwrap();
    let output = handle.output().unwrap();
    assert_eq!(output["output"], "ab");
    assert_eq!(output["truncated"], true);
    assert_eq!(output["exitStatus"]["exitCode"], 0);
}

#[test]
fn terminal_deadline_requests_cancellation_without_unblocking_unsettled_effects() {
    let (mut handle, _sender) = handle();
    let deadline = handle.created + LIFETIME;
    handle.poll_at(handle.created + (LIFETIME - Duration::from_nanos(1)));
    assert!(!handle.cancellation.0.load(Ordering::Acquire));
    handle.poll_at(deadline);
    assert!(handle.cancellation.0.load(Ordering::Acquire));
    assert!(!handle.settled);
    assert!(handle.result.is_none());
    handle.poll_at(deadline + (SETTLE_GRACE - Duration::from_nanos(1)));
    assert!(handle.result.is_none());
    handle.poll_at(deadline + SETTLE_GRACE);
    assert!(handle.exited().is_err());
    assert!(handle.action(Action::Release).is_err());
    assert!(!handle.settled);
}
