//! One owned logical native request, including per-call permissions and continuations.
use super::*;

type Failure = (Outcome, String);
type Revalidate<'a> = &'a dyn Fn(&Connection) -> Result<(), String>;

fn validate_active(
    workspace: &Workspace,
    run: &Prepared,
    connection: &Connection,
    control: &Control<'_>,
    revalidate: Revalidate<'_>,
) -> Result<(), String> {
    if control.cancelled() {
        return Err("Native request cancelled locally; no further write or continuation".into());
    }
    if connection.fingerprint() != run.fingerprint
        || native_writing(&run.input) != run.writes.is_some()
    {
        return Err("Native connection or frozen tool capability changed".into());
    }
    run.input.validate_committed(&workspace.data_dir)?;
    revalidate(connection)?;
    if let Some(writes) = &run.writes {
        writes.verify()?;
    }
    if control.cancelled() {
        return Err("Native request cancelled locally; no further write or continuation".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn inference_loop(
    workspace: &Workspace,
    run: &mut Prepared,
    events: &Events,
    control: &Control<'_>,
    incoming: &Receiver<Request>,
    http: &reqwest::Client,
    connection: &Connection,
    revalidate: Revalidate<'_>,
) -> (Outcome, String) {
    match inference_rounds(
        workspace, run, events, control, incoming, http, connection, revalidate,
    )
    .await
    {
        Ok(message) => (Outcome::Completed, message),
        Err(failure) => failure,
    }
}

#[allow(clippy::too_many_arguments)]
async fn inference_rounds(
    workspace: &Workspace,
    run: &mut Prepared,
    events: &Events,
    control: &Control<'_>,
    incoming: &Receiver<Request>,
    http: &reqwest::Client,
    connection: &Connection,
    revalidate: Revalidate<'_>,
) -> Result<String, Failure> {
    let model = selected_model(&run.input, connection)
        .map_err(|error| (Outcome::Failed, error))?
        .to_owned();
    let mut body: Value = serde_json::from_str(&run.payload)
        .map_err(|_| (Outcome::Failed, "Frozen native HTTP body is invalid".into()))?;
    let mut body_bytes = 0usize;
    let mut stream_bytes = 0usize;
    let mut public = String::new();
    let mut call_count = 0usize;
    let mut seen_calls = HashSet::new();
    for round in 1..=ROUND_LIMIT {
        validate_active(workspace, run, connection, control, revalidate).map_err(|error| {
            (
                if control.cancelled() {
                    Outcome::Cancelled
                } else {
                    Outcome::Failed
                },
                error,
            )
        })?;
        let wire = if round == 1 {
            run.payload.clone()
        } else {
            serde_json::to_string(&body)
                .map_err(|_| (Outcome::Failed, "Cannot encode native continuation".into()))?
        };
        body_bytes = body_bytes.checked_add(wire.len()).ok_or_else(|| {
            (
                Outcome::Failed,
                "Native aggregate request size overflow".into(),
            )
        })?;
        if body_bytes > BODY_LIMIT {
            return Err((
                Outcome::Failed,
                "Native aggregate HTTP bodies exceed 1 MiB; no further continuation sent".into(),
            ));
        }
        // HTTP deadlines start anew only when a new bounded round is dispatched.
        // Approval waiting never consumes this deadline.
        let round_control = Control {
            stopped: control.stopped,
            cancelled: control.cancelled,
            generation: control.generation,
            deadline: Instant::now() + CALL_TIMEOUT,
        };
        emit(events, Event::Status {
            generation: run.generation,
            task_id: run.input.task.id,
            message: format!("Native HTTP round {round}/{ROUND_LIMIT}; requested model {model}; {}; no shell or command verification",
                if run.writes.is_some() { "selected file tool requires a separate one-shot approval" } else { "no local write tools" }),
        }).map_err(|error| (Outcome::Failed, error))?;
        // No retry, resend, redirect, model/account substitution or second dispatch receipt.
        let response = connection
            .request(http, "POST", &connection.inference_url(&model))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .body(wire)
            .send();
        let mut response = if round == 1 {
            wait_inference(response, run, events, &round_control).await
        } else {
            wait_http(response, &round_control).await
        }
        .map_err(|error| {
            (
                Outcome::Unknown,
                format!("{error}; remote outcome unknown after dispatch; no retry"),
            )
        })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err((
                if (400..500).contains(&status) {
                    Outcome::Failed
                } else {
                    Outcome::Unknown
                },
                http_status(status),
            ));
        }
        let mut stream = Stream::new(connection.protocol);
        stream.round = round;
        stream.prefix = std::mem::take(&mut public);
        stream.wire_limit = STREAM_LIMIT - stream_bytes;
        if let Err(error) = stream
            .read(&mut response, &round_control, |stream| {
                stream.publish(events, run)
            })
            .await
        {
            let _ = stream.publish_partial(events, run);
            return Err((
                if stream.failed {
                    Outcome::Failed
                } else {
                    Outcome::Unknown
                },
                format!(
                    "{error}; local reader settled; remote completion/cancellation is not confirmed; no automatic retry"
                ),
            ));
        }
        stream_bytes += stream.wire_bytes;
        stream
            .publish_model(events, run)
            .map_err(|error| (Outcome::Failed, error))?;
        stream
            .publish_usage(events, run, true)
            .map_err(|error| (Outcome::Failed, error))?;
        let calls = stream
            .calls
            .finish(run.writes.is_some())
            .map_err(|error| (Outcome::Failed, error))?;
        if calls.is_empty() {
            stream
                .publish_final(events, run, &connection.conversation_scope())
                .map_err(|error| (Outcome::Failed, error))?;
            return Ok(format!(
                "Native response completed after {round} HTTP round(s); requested model {model}; observed model {}. {} No commands or checks ran through this route.",
                stream
                    .model
                    .as_deref()
                    .unwrap_or("unobserved (provider supplied no model identity)"),
                if run.writes.is_some() {
                    "File effects are recorded only by the actual Eevee-mediated tool results."
                } else {
                    "No local writes or tools executed."
                }
            ));
        }
        if round == ROUND_LIMIT || call_count + calls.len() > TOOL_CALL_LIMIT {
            return Err((Outcome::Failed, "Native round/function-call limit reached; terminal call set not executed and no continuation sent".into()));
        }
        let writes = run
            .writes
            .as_ref()
            .ok_or_else(|| (Outcome::Failed, "Unadvertised native tools refused".into()))?;
        {
            let mut paths = HashSet::new();
            for call in &calls {
                if !paths.insert(&call.path)
                    || call.id.as_ref().is_some_and(|id| seen_calls.contains(id))
                {
                    return Err((
                        Outcome::Failed,
                        "Repeated target or replayed native function-call identity refused".into(),
                    ));
                }
            }
        }
        call_count += calls.len();
        let mut results = Vec::with_capacity(calls.len());
        // Validate every exact proposal before the first prompt; transfer the
        // decoded full contents instead of copying them into another owner.
        let proposals = calls
            .into_iter()
            .map(|call| {
                writes
                    .propose(call.path, call.content)
                    .map(|proposal| (call.id, proposal))
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| (Outcome::Failed, error))?;
        for (index, (call_id, proposal)) in proposals.into_iter().enumerate() {
            let result = execute_write(
                workspace,
                run,
                events,
                control,
                incoming,
                connection,
                revalidate,
                round,
                index,
                call_id.as_deref(),
                &proposal,
            )
            .map_err(|error| {
                (
                    if control.cancelled() {
                        Outcome::Cancelled
                    } else {
                        Outcome::Failed
                    },
                    error,
                )
            })?;
            if let Some(id) = call_id {
                seen_calls.insert(id);
            }
            results.push(result);
        }
        stream
            .calls
            .append_results(&mut body, &results)
            .map_err(|error| (Outcome::Failed, error))?;
        public = stream.prefix;
        if !stream.text.is_empty() {
            if !public.is_empty() {
                public.push_str("\n\n");
            }
            public.push_str(&stream.text);
        }
        if public.len() > OUTPUT_LIMIT {
            return Err((
                Outcome::Failed,
                "Native aggregate public output exceeds 512 KiB".into(),
            ));
        }
    }
    Err((Outcome::Failed, "Native HTTP round limit reached".into()))
}

#[allow(clippy::too_many_arguments)]
fn permission(
    run: &Prepared,
    events: &Events,
    control: &Control<'_>,
    incoming: &Receiver<Request>,
    proposal: &WriteProposal,
    round: usize,
    index: usize,
    call_id: Option<&str>,
    timeout: Duration,
) -> Result<bool, String> {
    let arrival = super::super::approval_arrival();
    let owner = run
        .input
        .work
        .as_ref()
        .ok_or("Native file permission lost its durable owner")?;
    let request_id = json!({"eevee-native-write": {
        "request": owner.request_id, "generation": run.generation, "task": run.input.task.id,
        "round": round, "index": index, "provider_call_id": call_id, "permission": arrival,
    }});
    let deny = json!({"decision":"deny", "permission":arrival});
    let allow = json!({"decision":"allow-once", "permission":arrival});
    let prompt = Prompt {
        arrival,
        request_id: request_id.clone(),
        thread_id: String::new(),
        turn_id: format!("eevee-native-request:{}", owner.request_id),
        item_id: format!("eevee-native-write:{round}:{index}"),
        kind: PromptKind::Permission {
            // Keep the shared kind/path/parent scope headers first, verbatim.
            description: proposal.description(),
            options: vec![
                PermissionOption {
                    label: "Deny".into(),
                    description: "Do not write; return this denial to the model".into(),
                    answer: deny.clone(),
                },
                PermissionOption {
                    label: "Allow once".into(),
                    description: "Only this exact file and reviewed parents.".into(),
                    answer: allow.clone(),
                },
            ],
        },
    };
    emit(
        events,
        Event::Prompt {
            generation: run.generation,
            task_id: run.input.task.id,
            request: prompt,
        },
    )?;
    let deadline = Instant::now() + timeout;
    let result = (|| loop {
        if control.cancelled() {
            break Err("Native write permission cancelled; no write authorized".into());
        }
        if Instant::now() >= deadline {
            break Err("Native write permission expired; no write authorized".into());
        }
        match incoming.recv_timeout(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(20)),
        ) {
            Ok(Request::Answer {
                generation,
                request_id: offered_id,
                answer,
            }) => {
                if Instant::now() >= deadline {
                    break Err("Native write permission expired; no write authorized".into());
                }
                if generation == run.generation
                    && offered_id == request_id
                    && (answer == deny || answer == allow)
                {
                    // Cancellation is checked again by the actual write boundary.
                    break Ok(answer == allow);
                }
                emit(events, Event::Status { generation: run.generation, task_id: run.input.task.id,
                    message: "Foreign, stale, replayed or non-offered native permission answer rejected; no grant".into() })?;
            }
            Ok(Request::Cancel { generation }) if generation == run.generation => {
                control.cancelled.store(run.generation, Ordering::Release);
                break Err("Native write permission cancelled; no write authorized".into());
            }
            Ok(_) => {
                emit(events, Event::Status { generation: run.generation, task_id: run.input.task.id,
                    message: "Native request retains its owner; unrelated request rejected during file permission".into() })?;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err("Native permission channel closed; no write authorized".into());
            }
        }
    })();
    let closed = emit(
        events,
        Event::PromptClosed {
            generation: run.generation,
            request_id,
        },
    );
    result.and_then(|allowed| closed.map(|()| allowed))
}

fn journal(
    run: &Prepared,
    events: &Events,
    round: usize,
    index: usize,
    text: &str,
) -> Result<(), String> {
    let owner = run
        .input
        .work
        .as_ref()
        .ok_or("Native tool journal lost request ownership")?;
    let item_id = format!("eevee-native-write:{round}:{index}");
    emit(events, Event::Journal {
        generation: run.generation,
        task_id: run.input.task.id,
        record: crate::store::ProviderRecord {
            id: format!("native-write:{}:{round}:{index}", owner.request_id),
            agent: run.input.agent,
            generation: run.generation,
            thread_id: format!("eevee-native-request:{}", owner.request_id),
            turn_id: format!("eevee-native-request:{}", owner.request_id),
            item_id: item_id.clone(),
            origin: "Eevee-mediated native file tool".into(),
            coverage: "Exact selected create/replace and approved parent scope only; observed local filesystem result, not command verification, commit or an OS sandbox".into(),
            text: safe(text, true),
            complete: true,
            partner: Some(owner.owner.clone()),
        },
    })?;
    emit(
        events,
        Event::Item {
            generation: run.generation,
            task_id: run.input.task.id,
            thread_id: String::new(),
            turn_id: format!("eevee-native-request:{}", owner.request_id),
            item_id,
            kind: ItemKind::Tool,
            text: safe(text, true),
            complete: true,
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_write(
    workspace: &Workspace,
    run: &mut Prepared,
    events: &Events,
    control: &Control<'_>,
    incoming: &Receiver<Request>,
    connection: &Connection,
    revalidate: Revalidate<'_>,
    round: usize,
    index: usize,
    call_id: Option<&str>,
    proposal: &WriteProposal,
) -> Result<Value, String> {
    validate_active(workspace, run, connection, control, revalidate)?;
    let allowed = match permission(
        run,
        events,
        control,
        incoming,
        proposal,
        round,
        index,
        call_id,
        APPROVAL_TIMEOUT,
    ) {
        Ok(allowed) => allowed,
        Err(error) => {
            journal(
                run,
                events,
                round,
                index,
                &format!("{}: {error}", proposal.path.display()),
            )?;
            return Err(error);
        }
    };
    let result = if !allowed {
        json!({"status":"denied", "path":proposal.path, "message":"User denied this exact file proposal; no Eevee write occurred"})
    } else if let Err(error) = validate_active(workspace, run, connection, control, revalidate) {
        json!({"status":"refused", "path":proposal.path, "message":error})
    } else {
        let writes = run.writes.as_mut().ok_or("Native file scope disappeared")?;
        match writes.apply(proposal) {
            Ok(applied) => json!({"status":"success", "effect":"applied", "path":applied.path,
                "kind":format!("{:?}", applied.kind), "directories":applied.directories,
                "message":applied.message}),
            Err(failure) => failed_write(run, events, round, index, proposal, failure)?,
        }
    };
    let text =
        serde_json::to_string(&result).map_err(|_| "Cannot encode actual native file result")?;
    journal(run, events, round, index, &text)?;
    Ok(result)
}

fn failed_write(
    run: &Prepared,
    events: &Events,
    round: usize,
    index: usize,
    proposal: &WriteProposal,
    failure: crate::providers::writes::WriteFailure,
) -> Result<Value, String> {
    if !failure.uncertain {
        return Ok(json!({"status":"refused", "path":proposal.path, "message":failure.message}));
    }
    // Only the actual filesystem engine can establish local uncertainty.
    // Keep this event ahead of journals and the terminal event, including
    // when ordinary event capacity is exhausted.
    emit(
        events,
        Event::LocalEffectsUnknown {
            generation: run.generation,
            task_id: run.input.task.id,
            message: safe(&failure.message, true),
        },
    )?;
    journal(
        run,
        events,
        round,
        index,
        &format!(
            "Local filesystem effects uncertain for {}: {}",
            proposal.path.display(),
            failure.message
        ),
    )?;
    Err(format!(
        "Local filesystem effects remain uncertain: {}",
        failure.message
    ))
}

#[cfg(test)]
mod tests;
