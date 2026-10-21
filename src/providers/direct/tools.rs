//! Bounded, request-local tool transcripts. Opaque provider metadata never becomes a Call.
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{Value, json};

use super::{BODY_LIMIT, Protocol};
use crate::providers::writes::{MAX_BYTES, TOOL_NAME};

const CALL_LIMIT: usize = 16;
const ID_LIMIT: usize = 256;
const NAME_LIMIT: usize = 128;
pub(super) const PATH_LIMIT: usize = 4096;
// JSON may escape every source byte as a six-byte Unicode escape.
const ARGUMENT_LIMIT: usize = 6 * (MAX_BYTES + PATH_LIMIT) + 128;

pub(super) struct Call {
    pub(super) id: Option<String>,
    pub(super) path: PathBuf,
    pub(super) content: String,
}

pub(super) fn advertise(body: &mut Value, protocol: Protocol) {
    let parameters = json!({
        "type":"object",
        "properties":{
            "path":{"type":"string","description":"Exact project-relative file path, within any selected-path restriction."},
            "content":{"type":"string","description":"Complete intended UTF-8 file contents, not a patch."}
        },
        "required":["path","content"],
        "additionalProperties":false
    });
    let description = "Propose creating or replacing one project-relative file with complete contents (at most 64 KiB). Exact local permission and frozen workspace or selected-path scope checks are required before any write.";
    body["tools"] = match protocol {
        Protocol::Responses | Protocol::Codex => json!([{
            "type":"function","name":TOOL_NAME,"description":description,
            "parameters":parameters,"strict":true
        }]),
        Protocol::ChatCompletions => json!([{
            "type":"function","function":{
                "name":TOOL_NAME,"description":description,"parameters":parameters,"strict":true
            }
        }]),
        Protocol::Messages => json!([{
            "name":TOOL_NAME,"description":description,"input_schema":parameters
        }]),
        Protocol::Gemini => json!([{"functionDeclarations":[{
            "name":TOOL_NAME,"description":description,"parametersJsonSchema":parameters
        }]}]),
    };
    if matches!(protocol, Protocol::Responses | Protocol::Codex) {
        // Stateless Responses continuations need encrypted reasoning, not a server-side conversation.
        body["include"] = json!(["reasoning.encrypted_content"]);
    }
}

struct ResponseItem {
    value: Value,
    closed: bool,
    arguments_done: bool,
}

#[derive(Default)]
struct ChatCall {
    id: String,
    name: String,
    arguments: String,
    function_type: bool,
}

struct Block {
    value: Value,
    arguments: String,
    closed: bool,
}

pub(super) struct Calls {
    protocol: Protocol,
    bytes: usize,
    responses: BTreeMap<usize, ResponseItem>,
    output: Option<Vec<Value>>,
    chat: BTreeMap<usize, ChatCall>,
    text: String,
    blocks: BTreeMap<usize, Block>,
    parts: Vec<Value>,
    terminal: bool,
    unindexed_text: bool,
    message_reason: Option<String>,
}

impl Calls {
    pub(super) fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            bytes: 0,
            responses: BTreeMap::new(),
            output: None,
            chat: BTreeMap::new(),
            text: String::new(),
            blocks: BTreeMap::new(),
            parts: Vec::new(),
            terminal: false,
            unindexed_text: false,
            message_reason: None,
        }
    }

    fn charge(&mut self, bytes: usize) -> Result<(), String> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or("Tool transcript exceeds byte capacity")?;
        if self.bytes > BODY_LIMIT {
            return Err("Tool transcript exceeds byte capacity".into());
        }
        Ok(())
    }

    pub(super) fn observe(&mut self, kind: &str, value: &Value) -> Result<(), String> {
        match self.protocol {
            Protocol::Responses | Protocol::Codex => self.responses_event(kind, value),
            Protocol::ChatCompletions => self.chat_event(value),
            Protocol::Messages => self.messages_event(kind, value),
            Protocol::Gemini => self.gemini_event(value),
        }
    }

    /// Bounded opaque progress only; never expose names, paths or argument text.
    pub(super) fn proposal_progress(&self) -> Option<(usize, usize)> {
        let count = match self.protocol {
            Protocol::Responses | Protocol::Codex => self.output.as_ref().map_or_else(
                || {
                    self.responses
                        .values()
                        .filter(|item| item.value["type"] == "function_call")
                        .count()
                },
                |output| {
                    output
                        .iter()
                        .filter(|item| item["type"] == "function_call")
                        .count()
                },
            ),
            Protocol::ChatCompletions => self.chat.len(),
            Protocol::Messages => self
                .blocks
                .values()
                .filter(|block| block.value["type"] == "tool_use")
                .count(),
            Protocol::Gemini => self
                .parts
                .iter()
                .filter(|part| part.get("functionCall").is_some())
                .count(),
        };
        (count > 0).then_some((count.min(CALL_LIMIT), self.bytes / (16 * 1024)))
    }

    fn responses_event(&mut self, kind: &str, value: &Value) -> Result<(), String> {
        match kind {
            "response.output_text.delta" | "response.refusal.delta" if self.terminal => {
                return Err("Response text after completion".into());
            }
            "response.output_item.added" => {
                let index = index(value, "output_index", BODY_LIMIT)?;
                let item = value.get("item").ok_or("Response item missing")?;
                response_item(item)?;
                self.charge(encoded_len(item)?)?;
                if self.terminal || self.responses.contains_key(&index) {
                    return Err("Repeated response output index".into());
                }
                self.responses.insert(
                    index,
                    ResponseItem {
                        value: item.clone(),
                        closed: false,
                        arguments_done: false,
                    },
                );
            }
            "response.function_call_arguments.delta" => {
                let index = index(value, "output_index", BODY_LIMIT)?;
                let delta = string(value, "delta")?;
                self.charge(delta.len())?;
                let item = self
                    .responses
                    .get_mut(&index)
                    .ok_or("Arguments without response item")?;
                response_fragment(item, value)?;
                let Value::String(arguments) = item
                    .value
                    .get_mut("arguments")
                    .ok_or("Missing response arguments")?
                else {
                    return Err("Invalid response arguments".into());
                };
                append(arguments, delta, ARGUMENT_LIMIT)?;
            }
            "response.function_call_arguments.done" => {
                let index = index(value, "output_index", BODY_LIMIT)?;
                let arguments = string(value, "arguments")?;
                if arguments.len() > ARGUMENT_LIMIT {
                    return Err("Tool arguments exceed capacity".into());
                }
                let item = self
                    .responses
                    .get_mut(&index)
                    .ok_or("Arguments without response item")?;
                response_fragment(item, value)?;
                let previous = string(&item.value, "arguments")?;
                if !previous.is_empty() && previous != arguments {
                    return Err("Inconsistent completed arguments".into());
                }
                if previous.is_empty() {
                    self.charge(arguments.len())?;
                    self.responses
                        .get_mut(&index)
                        .ok_or("Response item missing")?
                        .value["arguments"] = Value::String(arguments.into());
                }
                self.responses
                    .get_mut(&index)
                    .ok_or("Response item missing")?
                    .arguments_done = true;
            }
            "response.output_item.done" => {
                let index = index(value, "output_index", BODY_LIMIT)?;
                let done = value.get("item").ok_or("Response item missing")?;
                response_item(done)?;
                let previous_size = encoded_len(
                    &self
                        .responses
                        .get(&index)
                        .ok_or("Completed response item without start")?
                        .value,
                )?;
                self.bytes = self.bytes.saturating_sub(previous_size);
                self.charge(encoded_len(done)?)?;
                let item = self
                    .responses
                    .get_mut(&index)
                    .ok_or("Completed response item without start")?;
                if item.closed {
                    return Err("Repeated completed response item".into());
                }
                consistent_item(&item.value, done)?;
                item.value = done.clone();
                item.closed = true;
            }
            "response.completed" => {
                if self.terminal
                    && self
                        .output
                        .as_ref()
                        .is_some_and(|v| v.iter().any(|v| v["type"] == "function_call"))
                {
                    return Err("Repeated response tool completion".into());
                }
                let output = value.get("response").and_then(|v| v.get("output"));
                if let Some(output) = output {
                    let output = output.as_array().ok_or("Response output is not an array")?;
                    if self.terminal && output.iter().any(|v| v["type"] == "function_call") {
                        return Err("Tool calls after response completion".into());
                    }
                    let output_size = encoded_len(output)?;
                    for item in output {
                        response_item(item)?;
                    }
                    for (index, item) in &self.responses {
                        let authoritative = output
                            .get(*index)
                            .ok_or("Response completion omitted streamed item")?;
                        consistent_item(&item.value, authoritative)?;
                    }
                    self.bytes = 0;
                    self.charge(output_size)?;
                    self.responses.clear();
                    self.output = Some(output.clone());
                } else if !self.responses.is_empty() {
                    return Err("Response completion omitted authoritative output".into());
                }
                self.terminal = true;
            }
            _ => {}
        }
        Ok(())
    }

    fn chat_event(&mut self, value: &Value) -> Result<(), String> {
        let Some(choices) = value.get("choices").and_then(Value::as_array) else {
            return Ok(());
        };
        for choice in choices {
            if choice.get("index").and_then(Value::as_u64).unwrap_or(0) != 0 {
                continue;
            }
            if let Some(delta) = choice.get("delta") {
                if delta.get("function_call").is_some() {
                    return Err("Legacy function calls are unsupported".into());
                }
                if let Some(content) = delta.get("content").filter(|v| !v.is_null()) {
                    let text = content.as_str().ok_or("Invalid assistant text")?;
                    self.charge(text.len())?;
                    if self.terminal && !text.is_empty() {
                        return Err("Assistant text after completion".into());
                    }
                    append(&mut self.text, text, BODY_LIMIT)?;
                }
                if let Some(calls) = delta.get("tool_calls") {
                    let calls = calls.as_array().ok_or("Invalid chat tool calls")?;
                    for fragment in calls {
                        if self.terminal {
                            return Err("Tool fragment after completion".into());
                        }
                        let index = index(fragment, "index", CALL_LIMIT)?;
                        self.charge(encoded_len(fragment)?)?;
                        if let Some(kind) = fragment.get("type")
                            && kind.as_str() != Some("function")
                        {
                            return Err("Unsupported chat tool type".into());
                        }
                        let call = self.chat.entry(index).or_default();
                        if fragment.get("type").is_some() {
                            call.function_type = true;
                        }
                        if let Some(id) = fragment.get("id") {
                            let id = id.as_str().ok_or("Invalid tool ID")?;
                            validate_id(id)?;
                            if !call.id.is_empty() && call.id != id {
                                return Err("Inconsistent chat call ID".into());
                            }
                            if call.id.is_empty() {
                                call.id.push_str(id);
                            }
                        }
                        if let Some(function) = fragment.get("function") {
                            if !function.is_object() {
                                return Err("Invalid tool function".into());
                            }
                            if let Some(name) = function.get("name") {
                                append(
                                    &mut call.name,
                                    name.as_str().ok_or("Invalid tool name")?,
                                    NAME_LIMIT,
                                )?;
                            }
                            if let Some(arguments) = function.get("arguments") {
                                append(
                                    &mut call.arguments,
                                    arguments.as_str().ok_or("Invalid tool arguments")?,
                                    ARGUMENT_LIMIT,
                                )?;
                            }
                        }
                    }
                }
            }
            if choice.get("finish_reason").is_some_and(|v| !v.is_null()) {
                if self.terminal && !self.chat.is_empty() {
                    return Err("Repeated chat tool completion".into());
                }
                let reason = string(choice, "finish_reason")?;
                if !matches!(reason, "stop" | "tool_calls")
                    || (reason == "tool_calls" && self.chat.is_empty())
                {
                    return Err("Chat finish reason does not match its tool call set".into());
                }
                self.terminal = true;
            }
        }
        Ok(())
    }

    fn messages_event(&mut self, kind: &str, value: &Value) -> Result<(), String> {
        match kind {
            "message_start" => {
                if self.terminal {
                    return Err("Message start after completion".into());
                }
                if value
                    .get("message")
                    .and_then(|v| v.get("content"))
                    .is_some_and(|v| v.as_array().is_none_or(|v| !v.is_empty()))
                {
                    return Err("Unexpected initial Messages content".into());
                }
            }
            "content_block_start" => {
                if self.terminal {
                    return Err("Content after Messages completion".into());
                }
                let block = value.get("content_block").ok_or("Missing content block")?;
                if block["type"] == "text" && value.get("index").is_none() {
                    if self.blocks.values().any(|v| v.value["type"] == "tool_use") {
                        return Err("Unindexed text in tool round".into());
                    }
                    self.unindexed_text = true;
                    self.charge(encoded_len(block)?)?;
                    return Ok(());
                }
                let index = index(value, "index", BODY_LIMIT)?;
                if block["type"] == "tool_use" && self.unindexed_text {
                    return Err("Unindexed text in tool round".into());
                }
                match string(block, "type")? {
                    "text" => {
                        string(block, "text")?;
                    }
                    "thinking" => {
                        string(block, "thinking")?;
                    }
                    "redacted_thinking" => {
                        string(block, "data")?;
                    }
                    "tool_use" => {
                        validate_name(string(block, "name")?)?;
                        validate_id(string(block, "id")?)?;
                    }
                    _ => return Err("Unsupported Messages content block".into()),
                }
                self.charge(encoded_len(block)?)?;
                if self.blocks.contains_key(&index) {
                    return Err("Repeated Messages block index".into());
                }
                if string(block, "type")? == "tool_use"
                    && self
                        .blocks
                        .values()
                        .filter(|v| v.value["type"] == "tool_use")
                        .count()
                        >= CALL_LIMIT
                {
                    return Err("Too many tool calls".into());
                }
                self.blocks.insert(
                    index,
                    Block {
                        value: block.clone(),
                        arguments: String::new(),
                        closed: false,
                    },
                );
            }
            "content_block_delta" => {
                if self.terminal {
                    return Err("Messages content after completion".into());
                }
                let delta = value.get("delta").ok_or("Missing content delta")?;
                let block_index = value
                    .get("index")
                    .and_then(Value::as_u64)
                    .and_then(|i| usize::try_from(i).ok());
                if delta["type"] == "thinking_delta" && block_index.is_none() {
                    if self.blocks.values().any(|v| v.value["type"] == "tool_use") {
                        return Err("Unindexed thinking in tool round".into());
                    }
                    self.unindexed_text = true;
                    self.charge(string(delta, "thinking")?.len())?;
                    return Ok(());
                }
                if delta["type"] == "text_delta"
                    && block_index.is_none_or(|i| !self.blocks.contains_key(&i))
                {
                    if self.blocks.values().any(|v| v.value["type"] == "tool_use") {
                        return Err("Unindexed text in tool round".into());
                    }
                    self.unindexed_text = true;
                    self.charge(string(delta, "text")?.len())?;
                    return Ok(());
                }
                let index = index(value, "index", BODY_LIMIT)?;
                self.charge(encoded_len(delta)?)?;
                let block = self
                    .blocks
                    .get_mut(&index)
                    .ok_or("Delta without Messages block")?;
                if block.closed || self.terminal {
                    return Err("Delta after block completion".into());
                }
                let (block_type, field, fragment) = match string(delta, "type")? {
                    "text_delta" => ("text", "text", string(delta, "text")?),
                    "thinking_delta" => ("thinking", "thinking", string(delta, "thinking")?),
                    "signature_delta" => ("thinking", "signature", string(delta, "signature")?),
                    "input_json_delta" => ("tool_use", "input", string(delta, "partial_json")?),
                    _ => return Err("Unsupported Messages delta".into()),
                };
                if string(&block.value, "type")? != block_type {
                    return Err("Delta does not match Messages block".into());
                }
                if block_type == "tool_use" {
                    if block.value.get("input").is_some_and(|v| {
                        !v.is_object() || !v.as_object().is_some_and(|v| v.is_empty())
                    }) {
                        return Err("Tool input supplied both whole and fragmented".into());
                    }
                    append(&mut block.arguments, fragment, ARGUMENT_LIMIT)?;
                } else {
                    let entry = block
                        .value
                        .as_object_mut()
                        .ok_or("Invalid Messages block")?
                        .entry(field)
                        .or_insert_with(|| Value::String(String::new()));
                    let Value::String(text) = entry else {
                        return Err("Invalid Messages text/signature".into());
                    };
                    append(text, fragment, BODY_LIMIT)?;
                }
            }
            "content_block_stop" => {
                if self.terminal && self.blocks.values().any(|v| v.value["type"] == "tool_use") {
                    return Err("Messages block stop after tool completion".into());
                }
                if value.get("index").is_none() && self.unindexed_text {
                    return Ok(());
                }
                let index = index(value, "index", BODY_LIMIT)?;
                let block = self
                    .blocks
                    .get_mut(&index)
                    .ok_or("Stop without Messages block")?;
                if block.closed {
                    return Err("Repeated Messages block stop".into());
                }
                block.closed = true;
            }
            "message_delta" => {
                if let Some(reason) = value
                    .get("delta")
                    .and_then(|v| v.get("stop_reason"))
                    .filter(|v| !v.is_null())
                {
                    let reason = reason.as_str().ok_or("Invalid Messages stop reason")?;
                    if self.terminal
                        && (self.blocks.values().any(|v| v.value["type"] == "tool_use")
                            || self.message_reason.as_deref() != Some(reason))
                    {
                        return Err("Messages stop reason after completion".into());
                    }
                    if !matches!(reason, "end_turn" | "stop_sequence" | "tool_use") {
                        return Err("Unsuccessful Messages stop reason".into());
                    }
                    if self
                        .message_reason
                        .as_deref()
                        .is_some_and(|previous| previous != reason)
                    {
                        return Err("Inconsistent Messages stop reason".into());
                    }
                    if self.message_reason.is_none() {
                        self.message_reason = Some(reason.into());
                    }
                }
            }
            "message_stop" => {
                let has_tools = self.blocks.values().any(|v| v.value["type"] == "tool_use");
                if (self.terminal && has_tools) || self.blocks.values().any(|v| !v.closed) {
                    return Err("Unfinished or repeated Messages completion".into());
                }
                self.terminal = true;
            }
            _ => {}
        }
        Ok(())
    }

    fn gemini_event(&mut self, value: &Value) -> Result<(), String> {
        let Some(candidates) = value.get("candidates").and_then(Value::as_array) else {
            return Ok(());
        };
        for candidate in candidates {
            if candidate.get("index").and_then(Value::as_u64).unwrap_or(0) != 0 {
                continue;
            }
            if let Some(content) = candidate.get("content") {
                if content
                    .get("role")
                    .is_some_and(|v| v.as_str() != Some("model"))
                {
                    return Err("Unexpected Gemini content role".into());
                }
                let parts = content
                    .get("parts")
                    .and_then(Value::as_array)
                    .ok_or("Invalid Gemini parts")?;
                if self.terminal && !parts.is_empty() {
                    return Err("Gemini content after completion".into());
                }
                for part in parts {
                    if !part.is_object() {
                        return Err("Invalid Gemini part".into());
                    }
                    if let Some(signature) = part.get("thoughtSignature")
                        && signature.as_str().is_none_or(str::is_empty)
                    {
                        return Err("Invalid Gemini thought signature".into());
                    }
                    if let Some(call) = part.get("functionCall") {
                        validate_name(string(call, "name")?)?;
                        if let Some(id) = call.get("id") {
                            validate_id(id.as_str().ok_or("Invalid Gemini call ID")?)?;
                        }
                        if self
                            .parts
                            .iter()
                            .filter(|p| p.get("functionCall").is_some())
                            .count()
                            >= CALL_LIMIT
                        {
                            return Err("Too many tool calls".into());
                        }
                    } else if part.get("text").is_some() {
                        string(part, "text")?;
                    } else if part.get("thoughtSignature").is_none() {
                        return Err("Unsupported Gemini part".into());
                    }
                    if part.get("executableCode").is_some()
                        || part.get("codeExecutionResult").is_some()
                        || part.get("functionResponse").is_some()
                    {
                        return Err("Unsupported Gemini tool type".into());
                    }
                    self.charge(encoded_len(part)?)?;
                    self.parts.push(part.clone());
                }
            }
            if candidate.get("finishReason").is_some() {
                if self.terminal && self.parts.iter().any(|p| p.get("functionCall").is_some()) {
                    return Err("Repeated Gemini tool completion".into());
                }
                self.terminal = true;
            }
        }
        Ok(())
    }

    pub(super) fn finish(&self, enabled: bool) -> Result<Vec<Call>, String> {
        let mut calls = Vec::new();
        match self.protocol {
            Protocol::Responses | Protocol::Codex => {
                if let Some(output) = &self.output {
                    let mut item_ids = HashSet::new();
                    for item in output {
                        response_item(item)?;
                        if let Some(id) = item.get("id").and_then(Value::as_str)
                            && !item_ids.insert(id)
                        {
                            return Err("Duplicate response output item ID".into());
                        }
                        if item["type"] == "function_call" {
                            if item
                                .get("status")
                                .is_some_and(|v| v.as_str() != Some("completed"))
                            {
                                return Err("Unfinished authoritative function call".into());
                            }
                            push_call(
                                &mut calls,
                                Some(string(item, "call_id")?),
                                string(item, "name")?,
                                decode(string(item, "arguments")?)?,
                            )?;
                        }
                    }
                } else if !self.responses.is_empty() {
                    return Err("Missing authoritative Responses output".into());
                }
            }
            Protocol::ChatCompletions => {
                contiguous(self.chat.keys().copied())?;
                for call in self.chat.values() {
                    if !call.function_type {
                        return Err("Chat tool call is missing function type".into());
                    }
                    push_call(
                        &mut calls,
                        Some(&call.id),
                        &call.name,
                        decode(&call.arguments)?,
                    )?;
                }
            }
            Protocol::Messages => {
                contiguous(self.blocks.keys().copied())?;
                for block in self.blocks.values() {
                    if !block.closed {
                        return Err("Unfinished Messages block".into());
                    }
                    if block.value["type"] == "tool_use" {
                        let args = if block.arguments.is_empty() {
                            arguments_value(block.value.get("input").ok_or("Missing tool input")?)?
                        } else {
                            decode(&block.arguments)?
                        };
                        push_call(
                            &mut calls,
                            Some(string(&block.value, "id")?),
                            string(&block.value, "name")?,
                            args,
                        )?;
                    }
                }
                if (!calls.is_empty() && self.message_reason.is_none())
                    || (calls.is_empty() && self.message_reason.as_deref() == Some("tool_use"))
                {
                    return Err("Messages stop reason does not match its tool call set".into());
                }
                if !calls.is_empty() {
                    for block in self
                        .blocks
                        .values()
                        .filter(|v| v.value["type"] == "thinking")
                    {
                        if string(&block.value, "signature")?.is_empty() {
                            return Err("Thinking continuation is missing its signature".into());
                        }
                    }
                }
            }
            Protocol::Gemini => {
                for part in &self.parts {
                    if let Some(call) = part.get("functionCall") {
                        let id = call
                            .get("id")
                            .map(|v| v.as_str().ok_or("Invalid Gemini call ID"))
                            .transpose()?;
                        push_call(
                            &mut calls,
                            id,
                            string(call, "name")?,
                            arguments_value(call.get("args").ok_or("Missing Gemini arguments")?)?,
                        )?;
                    }
                }
            }
        }
        if !calls.is_empty() && (!enabled || !self.terminal) {
            return Err("Tool calls require enabled scope and authoritative completion".into());
        }
        let mut ids = HashSet::new();
        for call in &calls {
            if let Some(id) = &call.id
                && !ids.insert(id.as_str())
            {
                return Err("Duplicate or replayed tool call ID".into());
            }
        }
        Ok(calls)
    }

    pub(super) fn append_results(&self, body: &mut Value, results: &[Value]) -> Result<(), String> {
        let calls = self.finish(true)?;
        if calls.is_empty() || calls.len() != results.len() {
            return Err("Tool result count does not match call set".into());
        }
        for result in results {
            if !result.is_object()
                || result.get("status").and_then(Value::as_str).is_none()
                || result.get("message").and_then(Value::as_str).is_none()
                || result.get("path").and_then(Value::as_str).is_none()
            {
                return Err("Tool result is not an actual structured local outcome".into());
            }
        }
        let mut additions = Vec::new();
        let field = match self.protocol {
            Protocol::Responses | Protocol::Codex => {
                additions.extend(
                    self.output
                        .as_ref()
                        .ok_or("Missing authoritative Responses output")?
                        .iter()
                        .cloned(),
                );
                for (call, result) in calls.iter().zip(results) {
                    additions.push(json!({"type":"function_call_output","call_id":call.id,"output":serde_json::to_string(result).map_err(|_| "Cannot encode tool result")?}));
                }
                "input"
            }
            Protocol::ChatCompletions => {
                let tool_calls: Vec<_> = self.chat.values().map(|call| json!({"id":call.id,"type":"function","function":{"name":call.name,"arguments":call.arguments}})).collect();
                additions.push(json!({"role":"assistant","content":if self.text.is_empty() { Value::Null } else { Value::String(self.text.clone()) },"tool_calls":tool_calls}));
                for (call, result) in calls.iter().zip(results) {
                    additions.push(json!({"role":"tool","tool_call_id":call.id,"content":serde_json::to_string(result).map_err(|_| "Cannot encode tool result")?}));
                }
                "messages"
            }
            Protocol::Messages => {
                let mut content = Vec::with_capacity(self.blocks.len());
                for block in self.blocks.values() {
                    let mut value = block.value.clone();
                    if value["type"] == "tool_use" && !block.arguments.is_empty() {
                        value["input"] = serde_json::from_str(&block.arguments)
                            .map_err(|_| "Invalid complete tool JSON")?;
                    }
                    content.push(value);
                }
                additions.push(json!({"role":"assistant","content":content}));
                let content: Vec<_> = calls.iter().zip(results).map(|(call, result)| {
                    Ok(json!({"type":"tool_result","tool_use_id":call.id,"is_error":result["status"] != "success","content":serde_json::to_string(result).map_err(|_| "Cannot encode tool result")?}))
                }).collect::<Result<_, String>>()?;
                additions.push(json!({"role":"user","content":content}));
                "messages"
            }
            Protocol::Gemini => {
                additions.push(json!({"role":"model","parts":self.parts}));
                let parts: Vec<_> = calls
                    .iter()
                    .zip(results)
                    .map(|(call, result)| {
                        let mut response = if result["status"] == "success" {
                            json!({"name":TOOL_NAME,"response":{"output":result}})
                        } else {
                            json!({"name":TOOL_NAME,"response":{"error":result}})
                        };
                        if let Some(id) = &call.id {
                            response["id"] = Value::String(id.clone());
                        }
                        json!({"functionResponse":response})
                    })
                    .collect();
                additions.push(json!({"role":"user","parts":parts}));
                "contents"
            }
        };
        // Preflight the exact serialized growth before touching the immutable original prefix.
        let original = encoded_len(body)?;
        let array = body
            .get(field)
            .and_then(Value::as_array)
            .ok_or("Native continuation array missing")?;
        let growth = additions.iter().try_fold(0usize, |size, value| {
            Ok::<_, String>(size + encoded_len(value)? + 1)
        })?;
        let comma_adjustment = usize::from(array.is_empty());
        if original
            .saturating_add(growth)
            .saturating_sub(comma_adjustment)
            > BODY_LIMIT
        {
            return Err("Native continuation body exceeds byte capacity".into());
        }
        body.get_mut(field)
            .and_then(Value::as_array_mut)
            .ok_or("Native continuation array missing")?
            .extend(additions);
        Ok(())
    }
}

fn encoded_len(value: &(impl serde::Serialize + ?Sized)) -> Result<usize, String> {
    // Count without allocating a second serialized transcript.
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).map_err(|_| "Cannot encode tool transcript")?;
    Ok(counter.0)
}

/// Preserve object integrity before Value would overwrite duplicate wire keys,
/// including whole Messages/Gemini tool arguments and terminal metadata.
pub(super) fn frame(data: &str) -> Result<Value, String> {
    struct Unique(Value);
    impl<'de> Deserialize<'de> for Unique {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Unique;
                fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.write_str("JSON with unique object keys")
                }
                fn visit_unit<E: serde::de::Error>(self) -> Result<Unique, E> {
                    Ok(Unique(Value::Null))
                }
                fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Unique, E> {
                    Ok(Unique(Value::Bool(value)))
                }
                fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Unique, E> {
                    Ok(Unique(Value::Number(value.into())))
                }
                fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Unique, E> {
                    Ok(Unique(Value::Number(value.into())))
                }
                fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Unique, E> {
                    serde_json::Number::from_f64(value)
                        .map(|value| Unique(Value::Number(value)))
                        .ok_or_else(|| E::custom("Invalid JSON number"))
                }
                fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Unique, E> {
                    Ok(Unique(Value::String(value.into())))
                }
                fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Unique, E> {
                    Ok(Unique(Value::String(value)))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut sequence: A,
                ) -> Result<Unique, A::Error> {
                    let mut values = Vec::new();
                    while let Some(Unique(value)) = sequence.next_element()? {
                        values.push(value);
                    }
                    Ok(Unique(Value::Array(values)))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> Result<Unique, A::Error> {
                    let mut values = serde_json::Map::new();
                    while let Some(key) = map.next_key::<String>()? {
                        if values.contains_key(&key) {
                            return Err(serde::de::Error::custom("Duplicate JSON object key"));
                        }
                        let Unique(value) = map.next_value()?;
                        values.insert(key, value);
                    }
                    Ok(Unique(Value::Object(values)))
                }
            }
            deserializer.deserialize_any(Visitor)
        }
    }
    serde_json::from_str::<Unique>(data)
        .map(|value| value.0)
        .map_err(|_| "Native SSE data is not valid JSON with unique object keys".into())
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing or invalid tool {key}"))
}

fn index(value: &Value, key: &str, limit: usize) -> Result<usize, String> {
    let index = value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or("Missing tool fragment index")?;
    if index >= limit as u64 {
        return Err("Tool fragment index exceeds capacity".into());
    }
    Ok(index as usize)
}

fn append(target: &mut String, fragment: &str, limit: usize) -> Result<(), String> {
    if target.len().saturating_add(fragment.len()) > limit {
        return Err("Tool fragment exceeds byte capacity".into());
    }
    target.push_str(fragment);
    Ok(())
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.len() > NAME_LIMIT || name != TOOL_NAME {
        return Err("Unsupported tool name".into());
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > ID_LIMIT || id.chars().any(char::is_control) {
        return Err("Invalid or oversized tool call ID".into());
    }
    Ok(())
}

fn response_item(item: &Value) -> Result<(), String> {
    match string(item, "type")? {
        "function_call" => {
            validate_name(string(item, "name")?)?;
            validate_id(string(item, "call_id")?)?;
            if string(item, "arguments")?.len() > ARGUMENT_LIMIT {
                return Err("Tool arguments exceed capacity".into());
            }
        }
        "message" | "reasoning" => {}
        _ => return Err("Unsupported Responses output/tool type".into()),
    }
    if let Some(id) = item.get("id") {
        validate_id(id.as_str().ok_or("Invalid response item ID")?)?;
    }
    Ok(())
}

fn response_fragment(item: &ResponseItem, event: &Value) -> Result<(), String> {
    if item.closed || item.arguments_done || item.value["type"] != "function_call" {
        return Err("Arguments for closed or non-function item".into());
    }
    if let Some(id) = event.get("item_id")
        && item.value.get("id") != Some(id)
    {
        return Err("Inconsistent response item ID".into());
    }
    Ok(())
}

fn consistent_item(before: &Value, after: &Value) -> Result<(), String> {
    for field in ["type", "id", "name", "call_id"] {
        if let Some(value) = before.get(field)
            && after.get(field) != Some(value)
        {
            return Err("Inconsistent authoritative response item".into());
        }
    }
    if before["type"] == "function_call" {
        let arguments = string(before, "arguments")?;
        if !arguments.is_empty() && string(after, "arguments")? != arguments {
            return Err("Inconsistent authoritative tool arguments".into());
        }
    }
    Ok(())
}

fn contiguous(indices: impl Iterator<Item = usize>) -> Result<(), String> {
    for (expected, actual) in indices.enumerate() {
        if expected != actual {
            return Err("Missing indexed tool/content fragment".into());
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    path: String,
    content: String,
}

fn decode(json: &str) -> Result<Arguments, String> {
    if json.len() > ARGUMENT_LIMIT {
        return Err("Tool arguments exceed capacity".into());
    }
    if !json.trim_start().starts_with('{') {
        return Err("Tool arguments must be an object".into());
    }
    serde_json::from_str(json)
        .map_err(|_| "Tool arguments must be exactly one path/content JSON object".into())
}

fn arguments_value(value: &Value) -> Result<Arguments, String> {
    if encoded_len(value)? > ARGUMENT_LIMIT {
        return Err("Tool arguments exceed capacity".into());
    }
    let object = value
        .as_object()
        .ok_or("Tool arguments must be an object")?;
    if object.len() != 2 {
        return Err("Tool arguments must contain only path and content".into());
    }
    let path = string(value, "path")?;
    let content = string(value, "content")?;
    Ok(Arguments {
        path: path.into(),
        content: content.into(),
    })
}

fn push_call(
    calls: &mut Vec<Call>,
    id: Option<&str>,
    name: &str,
    args: Arguments,
) -> Result<(), String> {
    if calls.len() >= CALL_LIMIT {
        return Err("Too many tool calls".into());
    }
    validate_name(name)?;
    if let Some(id) = id {
        validate_id(id)?;
    }
    calls.push(Call {
        id: id.map(str::to_owned),
        path: PathBuf::from(args.path),
        content: args.content,
    });
    Ok(())
}

#[cfg(test)]
mod tests;
