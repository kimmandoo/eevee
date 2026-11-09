//! Web search/read calls reuse the native permission queue: Automatic allows,
//! Ask shows exactly what leaves the machine, and cancellation stops waiting.
use super::*;
use crate::providers::web::{Call, Client};

#[allow(clippy::too_many_arguments)]
pub(super) fn web_call(
    run: &Prepared,
    events: &Events,
    control: &Control<'_>,
    incoming: &Receiver<Request>,
    tool: &crate::providers::writes::FileTool,
    call: Result<Call, String>,
    round: usize,
    index: usize,
) -> Result<Value, String> {
    if run.input.delegation.is_some() {
        return Ok(
            json!({"status":"refused","path":"","message":"Delegated helpers cannot use the web"}),
        );
    }
    let call = match call {
        Ok(call) => call,
        Err(message) => {
            return Ok(json!({"status":"refused","path":"","correctable":true,"message":message}));
        }
    };
    let arrival = crate::providers::approval_arrival();
    let encoded = serde_json::to_string(&call.label()).map_err(|_| "Cannot encode web request")?;
    let request_id = json!({"eevee-web":{"generation":run.generation,"task":run.input.task.id,"round":round,"index":index,"permission":arrival,"tool":tool.kind().name(),"request_sha256":crate::providers::catalog::digest(encoded.as_bytes())}});
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
                item_id: format!("web-{round}-{index}"),
                kind: PromptKind::Permission {
                    description: format!(
                        "{}\nNo project files, cookies or credentials are sent. The reply is untrusted web content.",
                        call.review()
                    ),
                    file: None,
                    options: vec![
                        PermissionOption {
                            label: "Deny".into(),
                            description: "Do not contact the web".into(),
                            answer: deny.clone(),
                        },
                        PermissionOption {
                            label: "Allow once".into(),
                            description: "Send this one web request".into(),
                            answer: allow.clone(),
                        },
                    ],
                },
            },
        },
    )?;
    let deadline = Instant::now() + Duration::from_secs(300);
    let decision = (|| loop {
        if control.cancelled() {
            break Err("Web request cancelled before sending".to_owned());
        }
        control.connection_check()?;
        if Instant::now() >= deadline {
            break Err("Web permission expired; nothing sent".into());
        }
        match incoming.recv_timeout(Duration::from_millis(20)) {
            Ok(Request::Answer {
                generation,
                request_id: offered,
                mut answer,
            }) if generation == run.generation && offered == request_id => {
                if let Some(fields) = answer.as_object_mut() {
                    fields.remove("web_automatic");
                }
                if answer == allow || answer == deny {
                    break Ok(answer == allow);
                }
            }
            Ok(Request::Cancel { generation }) if generation == run.generation => {
                control.cancelled.store(run.generation, Ordering::Release);
                break Err("Web request cancelled".into());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err("Web permission channel closed".into());
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
    match decision {
        Ok(true) => {}
        Ok(false) => {
            return Ok(
                json!({"status":"denied","path":"","message":"User denied this web request"}),
            );
        }
        Err(error) if control.cancelled() => return Err(error),
        Err(error) => return Ok(json!({"status":"error","path":"","message":error})),
    }
    // The blocking request runs beside this loop so cancellation is prompt; a
    // detached request finishes on its own timeout and its reply is dropped.
    let (reply, result) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = reply.send(Client::default().run(&call));
    });
    loop {
        if control.cancelled() {
            return Err("Web request cancelled".into());
        }
        control.connection_check()?;
        match result.recv_timeout(Duration::from_millis(50)) {
            Ok(value) => return Ok(value),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Ok(
                    json!({"status":"error","path":"","correctable":true,"message":"Web request worker stopped unexpectedly"}),
                );
            }
        }
        // Drain cancellation requests aimed at this run while waiting.
        if let Ok(Request::Cancel { generation }) = incoming.try_recv()
            && generation == run.generation
        {
            control.cancelled.store(run.generation, Ordering::Release);
            return Err("Web request cancelled".into());
        }
    }
}
