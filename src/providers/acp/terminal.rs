//! Operation-owned ACP v1 terminal callbacks, not an agent-private executor.
//! Protocol: https://agentclientprotocol.com/protocol/v1/terminals
//! Allocation is asynchronous; approval and execution belong to Event::RunCommand.
//! Output is available only after durable settlement, never invented while pending.
use super::*;
use crate::providers::commands::{Cancellation, validate_cwd};
use crate::workspace::runner::CommandSpec;
use serde::Deserialize;
use std::path::{Component, Path};

const TERMINAL_LIMIT: usize = 16;
const CREATION_LIMIT: u64 = 128;
const RETAIN_LIMIT: usize = 64 * 1024;
const WAIT_LIMIT: usize = 16;
const LIFETIME: Duration = Duration::from_secs(15 * 60);
const SETTLE_GRACE: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Create {
    session_id: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: Vec<Value>,
    cwd: Option<String>,
    output_byte_limit: Option<u64>,
    #[serde(rename = "_meta")]
    _meta: Option<Value>,
}

fn create_spec(root: &Path, params: &Value) -> Result<(CommandSpec, PathBuf, usize), String> {
    let create = Create::deserialize(params)
        .map_err(|_| "Invalid ACP terminal/create parameters; PTY/extensions are unsupported")?;
    // Deliberately no shell guessing: ACP command is the executable, args its literal argv.
    if create.command.is_empty()
        || create.command.len() > 4096
        || create.command.contains('\0')
        || safe(&create.command) != create.command
        || create.args.len() > 256
        || create
            .args
            .iter()
            .any(|arg| arg.contains('\0') || safe(arg) != *arg)
        || create.args.iter().map(String::len).sum::<usize>() > RETAIN_LIMIT
    {
        return Err("ACP terminal executable/argv is empty, oversized or unsafe to review".into());
    }
    if !create.env.is_empty() {
        return Err(
            "ACP terminal special environment is unsupported; nonempty env was NOT applied".into(),
        );
    }
    if create
        ._meta
        .as_ref()
        .is_some_and(|meta| !meta.is_null() && meta != &json!({}))
    {
        return Err(
            "ACP terminal/create metadata extensions, including PTY requests, are unsupported"
                .into(),
        );
    }
    if create.session_id.is_empty() || create.session_id.len() > 256 {
        return Err("Invalid ACP terminal session identity".into());
    }
    let cwd = match create.cwd {
        None => PathBuf::new(),
        Some(raw) => {
            let path = Path::new(&raw);
            if raw.is_empty()
                || raw.len() > 4096
                || raw.chars().any(char::is_control)
                || path
                    .components()
                    .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
            {
                return Err("ACP terminal cwd must be an absolute project path or literal project-relative directory".into());
            }
            if path.is_absolute() {
                path.strip_prefix(root)
                    .map_err(|_| "ACP terminal cwd is outside the project")?
                    .to_path_buf()
            } else {
                path.to_path_buf()
            }
        }
    };
    validate_cwd(&cwd)?;
    let limit = create.output_byte_limit.unwrap_or(RETAIN_LIMIT as u64);
    if limit > RETAIN_LIMIT as u64 {
        return Err("ACP terminal outputByteLimit exceeds the supported 65536-byte bound".into());
    }
    Ok((
        CommandSpec::Direct {
            program: create.command,
            args: create.args,
        },
        cwd,
        limit as usize,
    ))
}

struct Waiting {
    id: Value,
    action: Action,
}
#[derive(Clone, Copy)]
enum Action {
    Output,
    Wait,
    Kill,
    Release,
}
struct Handle {
    cancellation: Cancellation,
    reply: Receiver<Result<Value, String>>,
    result: Option<Result<Value, String>>,
    settled: bool,
    reply_received: bool,
    output_limit: usize,
    output: Option<Result<String, String>>,
    output_read: Option<OutputRead>,
    waiting: Vec<Waiting>,
    released: bool,
    created: Instant,
    cancellation_started: Option<Instant>,
}
struct OutputRead {
    reply: Receiver<Result<crate::workspace::artifacts::Response, String>>,
    handle: crate::workspace::artifacts::Handle,
    offset: u64,
    boundary_skips: u8,
    started: Instant,
}
impl Handle {
    fn cancel(&mut self) {
        self.cancellation.0.store(true, Ordering::Release);
        self.cancellation_started.get_or_insert_with(Instant::now);
    }
    fn poll(&mut self) {
        if self.reply_received {
            return;
        }
        if self.created.elapsed() >= LIFETIME {
            self.cancel();
        }
        self.result = match self.reply.try_recv() {
            Ok(result) => {
                self.reply_received = true;
                self.settled = match &result {
                    Err(_) => true, // Common bridge errors refuse before effects or after lost authority.
                    Ok(value) => {
                        !crate::providers::commands::outcome_unknown(value)
                            && (value["run"]["tree"].as_str() == Some("NotStarted")
                                || value["run"]["tree"].as_str() == Some("OwnedTreeExited")
                                    && value["run"]["ended_at_ms"].is_u64()
                                    && matches!(
                                        value["run"]["state"].as_str(),
                                        Some("Succeeded" | "Failed" | "Cancelled")
                                    ))
                    }
                };
                if !self.settled {
                    self.cancel();
                }
                Some(result)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.cancel();
                Some(Err(
                    "Owned runner reply was lost; command outcome unknown, never replayed".into(),
                ))
            }
        };
        if self.result.is_none()
            && self
                .cancellation_started
                .is_some_and(|start| start.elapsed() >= SETTLE_GRACE)
        {
            self.result = Some(Err("Owned command cancellation has not durably settled; outcome unknown, no kill/release success asserted and no replay".into()));
        }
    }
    fn evidence(&self) -> Value {
        match &self.result {
            None => {
                json!({"state":"pending","output_availability":"Only after durable command settlement; empty output is not completion","deadline_seconds":LIFETIME.as_secs()})
            }
            Some(Err(message)) => json!({"state":"unavailable","message":safe(message)}),
            Some(Ok(value)) => {
                json!({"state":if self.settled {"settled"} else {"outcome_unknown"},"run":value["run"],"artifact":value["artifact"],"artifact_lookup":value["artifact_lookup"],"output_unavailable":value["output_unavailable"],"output_source":"Exact tail of durable merged/redacted capture, not the lossy live preview; omitted capture bytes remain unavailable"})
            }
        }
    }
    fn exited(&self) -> Result<Value, String> {
        let value = self
            .result
            .as_ref()
            .ok_or("Owned command is still pending")?
            .as_ref()
            .map_err(Clone::clone)?;
        let run = &value["run"];
        if run["ended_at_ms"].as_u64().is_none()
            || run["tree"].as_str() != Some("OwnedTreeExited")
            || !matches!(
                run["state"].as_str(),
                Some("Succeeded" | "Failed" | "Cancelled")
            )
        {
            return Err("Owned command has no established process-tree exit; outcome unknown, no exit status invented".into());
        }
        let signal = match run["signal"].as_i64() {
            Some(signal) => json!(signal.to_string()), // Actual numeric OS signal, not a guessed name.
            None => Value::Null,
        };
        Ok(json!({"exitCode":run["exit_code"],"signal":signal}))
    }
    fn action(&self, action: Action) -> Result<Value, String> {
        match action {
            Action::Output => self.output(),
            Action::Wait => self.exited(),
            Action::Kill | Action::Release => {
                let value = self
                    .result
                    .as_ref()
                    .ok_or("Cancellation is pending durable settlement")?
                    .as_ref()
                    .map_err(Clone::clone)?;
                if !matches!(
                    value["run"]["tree"].as_str(),
                    Some("OwnedTreeExited" | "NotStarted")
                ) {
                    return Err("Owned command termination is unestablished; no kill/release success asserted".into());
                }
                Ok(json!({}))
            }
        }
    }
    fn output(&self) -> Result<Value, String> {
        if let Some(Err(message)) = &self.result {
            return Err(message.clone());
        }
        let (output, truncated) = if let Some(Ok(value)) = &self.result {
            let output = self
                .output
                .as_ref()
                .ok_or("Durable output retrieval is pending")?
                .as_ref()
                .map_err(Clone::clone)?;
            let captured = value["run"]["captured_bytes"].as_u64().unwrap_or(0);
            (
                output.as_str(),
                captured > output.len() as u64
                    || value["run"]["truncated"].as_bool().unwrap_or(false)
                    || value["run"]["discarded_bytes"].as_u64().unwrap_or(0) > 0,
            )
        } else {
            ("", false)
        };
        let mut result = json!({"output":output,"truncated":truncated,"_meta":{"eevee.dev/terminal":self.evidence()}});
        if self.result.is_some() {
            result["exitStatus"] = self.exited()?;
        }
        Ok(result)
    }
    fn poll_output(
        &mut self,
        events: &Events,
        generation: u64,
        task_id: i64,
    ) -> Result<(), String> {
        if self.output.is_some() {
            return Ok(());
        }
        let Some(Ok(value)) = &self.result else {
            return Ok(());
        };
        if value["output_unavailable"].as_bool() == Some(true) {
            self.output = Some(Err(
                "Durable command output is unavailable; no empty-success output substituted".into(),
            ));
            return Ok(());
        }
        let captured = value["run"]["captured_bytes"].as_u64().unwrap_or(0);
        if self.output_limit == 0 || captured == 0 {
            self.output = Some(Ok(String::new()));
            return Ok(());
        }
        let mut next = None;
        if let Some(read) = self.output_read.as_ref() {
            match read.reply.try_recv() {
                Ok(Ok(crate::workspace::artifacts::Response::Page(page)))
                    if page.handle == read.handle && page.offset == read.offset =>
                {
                    self.output = Some(Ok(page.text));
                }
                Ok(Err(error))
                    if error == "Artifact offset is not a UTF-8 boundary"
                        && read.boundary_skips < 3 =>
                {
                    next = Some((
                        read.handle.clone(),
                        read.offset + 1,
                        read.boundary_skips + 1,
                        read.started,
                    ));
                }
                Ok(Err(error)) => self.output = Some(Err(error)),
                Ok(Ok(_)) => {
                    self.output = Some(Err("Durable output response identity/type mismatch".into()))
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.output = Some(Err(
                        "Durable output retrieval disconnected; command was not replayed".into(),
                    ))
                }
                Err(mpsc::TryRecvError::Empty)
                    if read.started.elapsed() > Duration::from_secs(45) =>
                {
                    self.output = Some(Err(
                        "Durable output lookup deadline expired; command was not replayed".into(),
                    ));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        } else {
            let handle = match serde_json::from_value(value["artifact"].clone()) {
                Ok(handle) => handle,
                Err(_) => {
                    self.output = Some(Err("Owned run lacks an exact retained artifact identity; no preview substituted".into()));
                    return Ok(());
                }
            };
            next = Some((
                handle,
                captured.saturating_sub(self.output_limit as u64),
                0,
                Instant::now(),
            ));
        }
        if let Some((handle, offset, boundary_skips, started)) = next {
            let (reply, receiver) = mpsc::sync_channel(1);
            // The read ends at the capture end even after skipping a partial UTF-8 prefix.
            let limit = (captured - offset).min(self.output_limit as u64) as usize;
            if limit == 0 {
                self.output = Some(Ok(String::new()));
                return Ok(());
            }
            emit(
                events,
                Event::ArtifactLookup {
                    generation,
                    task_id,
                    operation: crate::workspace::artifacts::Operation::Read {
                        handle: handle.clone(),
                        offset,
                        limit,
                    },
                    reply,
                },
            )?;
            self.output_read = Some(OutputRead {
                reply: receiver,
                handle,
                offset,
                boundary_skips,
                started,
            });
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Terminals {
    handles: HashMap<String, Handle>,
    sequence: u64,
}
impl Terminals {
    pub(super) fn busy(&self) -> bool {
        self.handles.values().any(|handle| !handle.settled)
    }
    pub(super) fn cancel_all(&mut self) {
        for handle in self.handles.values_mut().filter(|handle| !handle.settled) {
            handle.cancel();
        }
    }
}
fn terminal_scope(input: &Input, session: Option<&str>, params: &Value) -> Result<(), String> {
    if client_capabilities(input)["terminal"] != true
        || input.work.as_ref().is_none_or(|claim| !claim.writing)
    {
        return Err("ACP terminal callbacks require the exact Implement writer claim; Plan/Review/inspection are nonexecuting".into());
    }
    if session.is_none() || params.get("sessionId").and_then(Value::as_str) != session {
        return Err("ACP terminal callback has wrong session identity".into());
    }
    Ok(())
}

impl Engine<'_> {
    fn terminal_error(
        &mut self,
        id: Value,
        code: i64,
        message: &str,
        evidence: Value,
    ) -> Result<(), String> {
        self.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":safe(message),"data":{"eevee.dev/terminal":evidence}}}))
    }
    fn terminal_authority(&self, params: &Value) -> Result<(), String> {
        if client_capabilities(&self.run.input)["terminal"] != true
            || !self.dispatched
            || self.terminal.is_some()
            || self.cancelling.is_some()
            || self.stop.load(Ordering::Acquire)
            || self.cancelled.load(Ordering::Acquire) == self.run.generation
        {
            return Err("ACP terminal callbacks require an active Implement turn; Plan/Review/inspection never authorize commands".into());
        }
        terminal_scope(&self.run.input, self.session.as_deref(), params)?;
        self.validate_file_owner()
    }
    pub(super) fn terminal_request(
        &mut self,
        id: Value,
        method: &str,
        params: &Value,
    ) -> Result<(), String> {
        if self.permissions.seen.len() >= 4096 || !self.permissions.seen.insert(id.to_string()) {
            return Err("Repeated/excessive ACP callback identity; command never replayed".into());
        }
        if let Err(error) = self.terminal_authority(params) {
            return self.terminal_error(id, -32602, &error, Value::Null);
        }
        self.poll_terminals()?;
        if method == "terminal/create" {
            if self.terminals.busy()
                || self.pending_write.is_some()
                || !self.permissions.pending.is_empty()
                || self.terminals.handles.len() >= TERMINAL_LIMIT
                || self.terminals.sequence >= CREATION_LIMIT
            {
                return self.terminal_error(id, -32602, "ACP terminal allocation bound or overlapping write/permission/command; no execution queued", Value::Null);
            }
            let (command, cwd, output_limit) = match create_spec(&self.workspace.project, params) {
                Ok(spec) => spec,
                Err(error) => return self.terminal_error(id, -32602, &error, Value::Null),
            };
            self.terminals.sequence += 1;
            let terminal_id = format!(
                "eevee-acp-{}-{}-{}-{}",
                std::process::id(),
                self.journal_nonce,
                self.run.generation,
                self.terminals.sequence
            );
            let (reply, receiver) = mpsc::sync_channel(1);
            let cancellation = Cancellation::new();
            let claim = self
                .run
                .input
                .work
                .clone()
                .ok_or("ACP terminal writer claim missing")?;
            emit(
                self.events,
                Event::RunCommand {
                    generation: self.run.generation,
                    task_id: self.run.input.task.id,
                    claim,
                    data_dir: self.workspace.data_dir.clone(),
                    call_id: terminal_id.clone(),
                    cwd,
                    command,
                    cancelled: cancellation.0.clone(),
                    reply,
                },
            )?;
            self.terminals.handles.insert(
                terminal_id.clone(),
                Handle {
                    cancellation,
                    reply: receiver,
                    result: None,
                    output_limit,
                    output: None,
                    output_read: None,
                    waiting: Vec::new(),
                    released: false,
                    settled: false,
                    reply_received: false,
                    created: Instant::now(),
                    cancellation_started: None,
                },
            );
            return self.send(json!({"jsonrpc":"2.0","id":id,"result":{"terminalId":terminal_id,"_meta":{"eevee.dev/terminal":{"state":"pending","approval":"Each exact command needs separate user approval before effects; allocation is NOT a grant","output_availability":"After durable settlement only","lifetime_seconds":LIFETIME.as_secs(),"output_byte_limit":output_limit}}}}));
        }
        let terminal_id = match identity(params, "terminalId") {
            Ok(identity) => identity,
            Err(error) => return self.terminal_error(id, -32602, &error, Value::Null),
        };
        let Some(handle) = self
            .terminals
            .handles
            .get_mut(&terminal_id)
            .filter(|handle| !handle.released)
        else {
            return self.terminal_error(
                id,
                -32602,
                "Unknown/released terminal ID for this exact operation; no command affected",
                Value::Null,
            );
        };
        if method == "terminal/output"
            && (handle.result.is_none()
                || handle.output.is_some()
                || matches!(handle.result, Some(Err(_))))
        {
            let result = handle.output();
            let evidence = handle.evidence();
            return match result {
                Ok(result) => self.send(json!({"jsonrpc":"2.0","id":id,"result":result})),
                Err(error) => self.terminal_error(id, -32603, &error, evidence),
            };
        }
        let action = match method {
            "terminal/output" => Action::Output,
            "terminal/wait_for_exit" => Action::Wait,
            "terminal/kill" => Action::Kill,
            "terminal/release" => Action::Release,
            _ => {
                return self.terminal_error(
                    id,
                    -32601,
                    "Unsupported ACP terminal method; no execution",
                    Value::Null,
                );
            }
        };
        if handle.waiting.len() >= WAIT_LIMIT {
            return self.terminal_error(
                id,
                -32602,
                "ACP terminal wait/control request bound exceeded",
                Value::Null,
            );
        }
        if matches!(action, Action::Kill | Action::Release) {
            handle.cancel();
        }
        if matches!(action, Action::Release) {
            handle.released = true;
        }
        handle.waiting.push(Waiting { id, action });
        self.poll_terminals()
    }
    pub(super) fn poll_terminals(&mut self) -> Result<(), String> {
        let mut responses = Vec::new();
        let mut released = Vec::new();
        let mut unknown = None;
        for (terminal_id, handle) in &mut self.terminals.handles {
            handle.poll();
            if handle.result.is_some() && !handle.settled {
                unknown = Some(format!(
                    "ACP terminal {terminal_id} has an unestablished local command outcome. Adapter stopped; owned cancellation requested, no further file/command effects or replay permitted."
                ));
                break;
            }
            if handle.output_read.is_some()
                || handle
                    .waiting
                    .iter()
                    .any(|waiting| matches!(waiting.action, Action::Output))
            {
                handle.poll_output(self.events, self.run.generation, self.run.input.task.id)?;
            }
            if handle.result.is_none() {
                continue;
            }
            let mut index = 0;
            while index < handle.waiting.len() {
                if matches!(handle.waiting[index].action, Action::Output)
                    && matches!(handle.result, Some(Ok(_)))
                    && handle.output.is_none()
                {
                    index += 1;
                    continue;
                }
                let waiting = handle.waiting.remove(index);
                responses.push((waiting.id, handle.action(waiting.action), handle.evidence()));
            }
            if handle.released && handle.settled && handle.waiting.is_empty() {
                released.push(terminal_id.clone());
            }
        }
        if let Some(message) = unknown {
            self.terminals.cancel_all();
            self.terminal = Some((Outcome::Unknown, message.clone()));
            emit(
                self.events,
                Event::LocalEffectsUnknown {
                    generation: self.run.generation,
                    task_id: self.run.input.task.id,
                    message: message.clone(),
                },
            )?;
            return Err(message);
        }
        for terminal_id in released {
            self.terminals.handles.remove(&terminal_id);
        }
        for (id, result, evidence) in responses {
            match result {
                Ok(mut result) => {
                    result["_meta"] = json!({"eevee.dev/terminal":evidence});
                    self.send(json!({"jsonrpc":"2.0","id":id,"result":result}))?;
                }
                Err(error) => self.terminal_error(id, -32603, &error, evidence)?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
