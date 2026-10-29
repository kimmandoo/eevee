//! MCP calls reuse the native permission queue and durable tool-result receipt.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn mcp_call(
    workspace: &Workspace,
    run: &Prepared,
    events: &Events,
    control: &Control<'_>,
    incoming: &Receiver<Request>,
    name: &str,
    arguments: &Value,
    round: usize,
    index: usize,
) -> Result<Value, String> {
    mcp_call_inner(
        workspace, run, events, control, incoming, name, arguments, round, index,
    )
    .map(|mut result| {
        result["path"] = json!("");
        if !result["message"].is_string() {
            result["message"] =
                json!("Untrusted unmanaged MCP server result; not host verification");
        }
        result["unmanaged"] = json!(true);
        result
    })
}

#[allow(clippy::too_many_arguments)]
fn mcp_call_inner(
    _workspace: &Workspace,
    run: &Prepared,
    events: &Events,
    control: &Control<'_>,
    incoming: &Receiver<Request>,
    name: &str,
    arguments: &Value,
    round: usize,
    index: usize,
) -> Result<Value, String> {
    let Some(tool) = run.input.mcp.iter().find(|tool| tool.wire_name() == name) else {
        return Ok(json!({"status":"refused","message":"Unselected MCP tool; no call dispatched"}));
    };
    if let Err(error) = tool.validate(&run.input.task.project, native_writing(&run.input)) {
        return Ok(json!({"status":"refused","message":error}));
    }
    if run.input.delegation.is_some() && !tool.user_read {
        return Ok(
            json!({"status":"refused","message":"Delegation has no effect MCP capability; server annotations cannot grant it"}),
        );
    }
    let encoded = serde_json::to_string(arguments).map_err(|_| "Cannot encode MCP arguments")?;
    if encoded.len() > crate::providers::mcp::LIMIT
        || crate::workspace::privacy::redact(&encoded) != encoded
    {
        return Ok(
            json!({"status":"refused","message":"MCP arguments exceed 64 KiB or contain recognizable credentials"}),
        );
    }
    let arrival = crate::providers::approval_arrival();
    let request_id = json!({"eevee-mcp":{"generation":run.generation,"task":run.input.task.id,"round":round,"index":index,"permission":arrival,"server":tool.server,"tool":tool.name,"session":tool.session_id,"arguments_sha256":crate::providers::catalog::digest(encoded.as_bytes())}});
    let deny = json!({"decision":"deny","permission":arrival});
    let allow = json!({"decision":"allow-once","permission":arrival});
    emit(
        events,
        Event::Prompt {
            generation: run.generation,
            task_id: run.input.task.id,
            request: Prompt {
                arrival,
                request_id: request_id.clone(),
                thread_id: String::new(),
                turn_id: format!("round-{round}"),
                item_id: format!("mcp-{round}-{index}"),
                kind: PromptKind::Permission {
                    description: format!(
                        "UNMANAGED MCP call — NOT sandboxed. Eevee enforces selection, review and 64 KiB output bounds, not server effects.\nServer: {}\nTool: {}\nArguments (exact JSON): {}\nUser-marked read: {} (assertion only; server annotations never grant)\nProject unmanaged effects already unknown/pending: {}. Retained uncertainty disables every automatic effect grant; this fresh review does not erase earlier missing evidence.\nAllow once invokes this exact tools/call; cancellation kills its owned process tree but does not roll back external effects.",
                        tool.server,
                        tool.name,
                        encoded,
                        tool.user_read,
                        tool.effects.unknown()
                    ),
                    file: None,
                    options: vec![
                        PermissionOption {
                            label: "Deny".into(),
                            description: "Return denial; do not call the MCP server".into(),
                            answer: deny.clone(),
                        },
                        PermissionOption {
                            label: "Allow once".into(),
                            description: "Invoke this exact unmanaged server/tool/JSON once".into(),
                            answer: allow.clone(),
                        },
                    ],
                },
            },
        },
    )?;
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut cause = "Fresh explicit user review".to_owned();
    let mut automatic = false;
    let decision = (|| loop {
        if control.cancelled() {
            break Err("MCP permission cancelled; no call dispatched".to_owned());
        }
        control.connection_check()?;
        if Instant::now() >= deadline {
            break Err("MCP permission expired; no call dispatched".into());
        }
        match incoming.recv_timeout(Duration::from_millis(20)) {
            Ok(Request::Answer {
                generation,
                request_id: offered,
                mut answer,
            }) if generation == run.generation && offered == request_id => {
                let rule = answer
                    .as_object_mut()
                    .and_then(|answer| answer.remove("mcp_rule"));
                if answer != deny && answer != allow {
                    continue;
                }
                if let Some(rule) = rule {
                    automatic = true;
                    cause = format!(
                        "{} by user-created MCP rule #{} · {}",
                        if answer == allow { "Allowed" } else { "Denied" },
                        rule["id"],
                        rule["sentence"].as_str().unwrap_or_default()
                    );
                }
                break Ok(answer == allow);
            }
            Ok(Request::Cancel { generation }) if generation == run.generation => {
                control.cancelled.store(run.generation, Ordering::Release);
                break Err("MCP cancelled".into());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err("MCP permission channel closed".into());
            }
            _ => {}
        }
    })();
    emit(
        events,
        Event::PromptClosed {
            generation: run.generation,
            request_id,
        },
    )?;
    let permitted = match decision {
        Ok(allowed) => allowed,
        Err(error) if control.cancelled() => {
            tool.session.lock().stop();
            return Err(error);
        }
        Err(error) => {
            return Ok(
                json!({"status":"error","server":tool.server,"tool":tool.name,"message":error}),
            );
        }
    };
    emit(
        events,
        Event::Item {
            generation: run.generation,
            task_id: run.input.task.id,
            thread_id: format!("native-{}", run.generation),
            turn_id: format!("round-{round}"),
            item_id: format!("mcp-decision-{round}-{index}"),
            kind: ItemKind::Tool,
            text: format!(
                "{cause} · UNMANAGED MCP {} / {} · decision {}",
                tool.server,
                tool.name,
                if permitted { "allow once" } else { "deny" }
            ),
            complete: true,
        },
    )?;
    if !permitted {
        return Ok(
            json!({"status":"denied","server":tool.server,"tool":tool.name,"message":"User or user-created MCP rule denied this unmanaged call"}),
        );
    }
    control.connection_check()?;
    if control.cancelled() {
        tool.session.lock().stop();
        return Err("MCP cancelled before call".into());
    }
    tool.validate(&run.input.task.project, native_writing(&run.input))?;
    emit(
        events,
        Event::Item {
            generation: run.generation,
            task_id: run.input.task.id,
            thread_id: format!("native-{}", run.generation),
            turn_id: format!("round-{round}"),
            item_id: format!("mcp-review-{round}-{index}"),
            kind: ItemKind::Tool,
            text: format!(
                "Unmanaged MCP tools/call {} / {} · exact JSON {} · {cause}",
                tool.server, tool.name, encoded
            ),
            complete: true,
        },
    )?;
    let mut session = tool.session.lock();
    let lease = match tool.effects.begin(automatic) {
        Ok(lease) => lease,
        Err(error) => {
            return Ok(
                json!({"status":"refused","server":tool.server,"tool":tool.name,"message":error}),
            );
        }
    };
    let mut result = session.call(&tool.name, arguments, &|| {
        control.cancelled() || control.connection_check().is_err()
    });
    let unknown = match lease.finish(result["status"] == "success") {
        Ok(unknown) => unknown,
        Err(error) => {
            result["uncertainty_error"] = json!(error);
            true
        }
    };
    result["effects_unknown"] = json!(unknown);
    if unknown {
        emit(
            events,
            Event::Item {
                generation: run.generation,
                task_id: run.input.task.id,
                thread_id: format!("native-{}", run.generation),
                turn_id: format!("round-{round}"),
                item_id: format!("mcp-effects-unknown-{round}-{index}"),
                kind: ItemKind::Tool,
                text: format!(
                    "UNMANAGED MCP effects unknown or still pending for this project after {} / {}. Server failure/cancellation is not proof of no effects. Persisted marker disables automatic Command/Write/MCP grants, including task auto-approval; inference continues and fresh explicit reviews remain available. No rollback.",
                    tool.server, tool.name
                ),
                complete: true,
            },
        )?;
    }
    result["server"] = json!(tool.server);
    result["tool"] = json!(tool.name);
    result["arguments_sha256"] = json!(crate::providers::catalog::digest(encoded.as_bytes()));
    result["approval_cause"] = json!(cause);
    if result.to_string().len() > crate::providers::mcp::LIMIT {
        result = json!({"status":"error","unmanaged":true,"server":tool.server,"tool":tool.name,"effects_unknown":unknown,"message":"MCP result envelope exceeds 64 KiB; withheld"});
    }
    if control.cancelled() {
        session.stop();
        return Err(
            "MCP cancelled; owned server tree terminated; external effects not rolled back".into(),
        );
    }
    Ok(result)
}
