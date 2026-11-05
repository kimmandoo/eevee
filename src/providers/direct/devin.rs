//! Devin's native Connect/protobuf transport. All effects remain in coding.rs.
mod proto;
#[cfg(test)]
mod tests;
use super::*;
use proto::{Encode, Message, Value as Wire};
use std::collections::BTreeMap;
use std::io::Read;

const MODELS: &str = "/exa.api_server_pb.ApiServerService/GetCliModelConfigs";
const AUTH: &str = "/exa.auth_pb.AuthService/GetUserJwt";
const ASSIGN: &str = "/exa.api_server_pb.ApiServerService/AssignModel";
const CHAT: &str = "/exa.api_server_pb.ApiServerService/GetChatMessage";

fn metadata(connection: &Connection, jwt: &str, discovery: bool) -> Encode {
    let mut out = Encode::default();
    let key = connection.secret().unwrap_or_default();
    let encoded;
    let key = if !key.starts_with("devin-session-token$") && key.split('.').count() == 3 {
        encoded = format!("devin-session-token${key}");
        encoded.as_str()
    } else {
        key
    };
    out.text(3, key);
    out.text(21, jwt);
    out.text(1, if discovery { "chisel" } else { "devin-cli" });
    let version = if discovery {
        "0.0.0-dev"
    } else {
        "3000.11.205"
    };
    out.text(7, version);
    out.text(12, "chisel");
    out.text(2, version);
    if !discovery {
        out.text(28, "chisel");
    }
    out.text(4, "en");
    out.text(
        5,
        if cfg!(target_os = "macos") {
            "darwin"
        } else if cfg!(windows) {
            "windows"
        } else {
            "linux"
        },
    );
    out.number(6, 1);
    if discovery {
        // Packed repeated display enum: router, review, internal, unclassified, normal.
        out.bytes(30, &[3, 4, 6, 7, 8]);
    }
    out
}

async fn unary(
    http: &reqwest::Client,
    connection: &Connection,
    path: &str,
    body: Vec<u8>,
    control: &Control<'_>,
    run: &Prepared,
    events: &Events,
) -> Result<Vec<u8>, String> {
    connection.revalidate()?;
    control.check()?;
    let mut response = wait_http(
        http.post(format!("{}{path}", connection.base_url))
            .header("Content-Type", "application/proto")
            .header("Connect-Protocol-Version", "1")
            .header("Accept-Encoding", "identity")
            .body(body)
            .send(),
        control,
    )
    .await
    .inspect_err(|_| {
        if !control.cancelled() {
            connection_failure(events, run, super::super::diagnostics::Failure::Transport);
        }
    })?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        connection_failure(
            events,
            run,
            super::super::diagnostics::Failure::Http(status),
        );
        return Err(rejection(http_status(status), response, control).await);
    }
    let framed = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/connect+proto"));
    let bytes = read_bounded(&mut response, PAGE_LIMIT, control, run, events).await?;
    if framed {
        decode_unary(&bytes)
    } else {
        // Some Connect implementations frame unary output despite application/proto.
        if bytes.first() == Some(&0) {
            decode_unary(&bytes)
        } else {
            Ok(bytes)
        }
    }
}

fn decode_unary(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = Frames::default();
    let frames = decoder.feed(bytes)?;
    decoder.finish()?;
    let mut message = None;
    let mut ended = false;
    for (flags, payload) in frames {
        if ended {
            return Err("Devin unary data after trailer".into());
        }
        let payload = decompress(flags, &payload)?;
        if flags & 2 != 0 {
            trailer(&payload)?;
            ended = true;
        } else if message.replace(payload.into_owned()).is_some() {
            return Err("Multiple Devin unary messages".into());
        }
    }
    message.ok_or_else(|| "Empty Devin unary response".into())
}

struct Model {
    option: ModelOption,
    router: bool,
    max_output: u64,
}
fn models(bytes: &[u8]) -> Result<Vec<Model>, String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut entries = 0;
    for item in Message(bytes).fields() {
        let (field, value) = item?;
        if field != 1 {
            continue;
        }
        entries += 1;
        if entries > CATALOGUE_LIMIT {
            return Err("Devin catalogue exceeds 4096 entries".into());
        }
        let Wire::Bytes(bytes) = value else {
            return Err("Invalid Devin model config".into());
        };
        let config = Message(bytes);
        let info = config.message(23)?;
        let display = info.number(22)?.unwrap_or(0);
        if config.number(4)?.unwrap_or(0) != 0 || matches!(display, 4 | 6) {
            continue;
        }
        let id = config.text(22)?;
        if id.is_empty() {
            continue;
        }
        let id = checked_identity(id)?;
        if !seen.insert(id.clone()) {
            return Err("Duplicate Devin model identity".into());
        }
        // Native fusion sidekick pairings are not standalone inference models.
        // Do not silently replace an exact selection with a different lead uid.
        if id.starts_with("fusion-") && id.contains("-sidekick-") {
            continue;
        }
        let router = (display == 3 || info.number(25)?.unwrap_or(0) != 0)
            && !info.fields().any(|item| matches!(item, Ok((20, _))));
        result.push(Model {
            option: ModelOption {
                name: metadata_text(config.text(1)?, &id, 512),
                description: metadata_text(config.text(27)?, "", 4096),
                id,
                efforts: Vec::new(),
                default_effort: None,
            },
            router,
            max_output: info
                .number(13)?
                .filter(|v| *v > 0 && *v <= 1_000_000)
                .unwrap_or(64000),
        });
    }
    if result.is_empty() {
        return Err(
            "Devin returned no visible usable model configurations; no fallback accepted".into(),
        );
    }
    Ok(result)
}
fn metadata_text(text: &str, fallback: &str, limit: usize) -> String {
    super::metadata(
        Some(if text.is_empty() { fallback } else { text }),
        fallback,
        limit,
    )
}
async fn fetch_models(
    http: &reqwest::Client,
    connection: &Connection,
    control: &Control<'_>,
    run: &Prepared,
    events: &Events,
) -> Result<Vec<Model>, String> {
    let mut body = Encode::default();
    body.bytes(1, &metadata(connection, "", true).0);
    models(&unary(http, connection, MODELS, body.0, control, run, events).await?)
}
pub(super) async fn discover(
    http: &reqwest::Client,
    connection: &Connection,
    control: &Control<'_>,
    run: &Prepared,
    events: &Events,
) -> Result<ModelCatalogue, String> {
    let catalogue = ModelCatalogue {
        models: fetch_models(http, connection, control, run, events).await?.into_iter().map(|model| model.option).collect(),
        current: connection.model.clone(),
        source: "Devin authenticated native model configurations; exact server wire identities".into(),
        effort_note: Some("Effort variants remain separate exact models; no independent effort control is advertised. Disabled/internal configurations and native-only sidekick pairings are not selectable.".into()),
    };
    catalogue.validate()?;
    Ok(catalogue)
}

pub(super) struct Session {
    base: String,
    jwt: String,
    cascade: String,
    model: Model,
}

fn reviewed_route(base: &str, custom: &str) -> Result<(), String> {
    if custom.is_empty() {
        return Ok(());
    }
    let custom = config::validate_base(custom)
        .map_err(|_| "Devin returned an invalid API destination; no chat request sent")?;
    if custom != base {
        return Err("Devin requested a different API destination; explicitly configure and review that endpoint before sending credentials or task content".into());
    }
    Ok(())
}
fn uuid() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|_| "Cannot create Devin request identity")?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    ))
}
impl Session {
    pub(super) async fn start(
        http: &reqwest::Client,
        connection: &Connection,
        control: &Control<'_>,
        run: &Prepared,
        events: &Events,
        selected: &str,
    ) -> Result<Self, String> {
        let model = fetch_models(http, connection, control, run, events).await?.into_iter()
            .find(|model| model.option.id == selected)
            .ok_or("Selected Devin model is not in the account's current visible configurations; choose an available exact model")?;
        let base = connection.base_url.clone();
        let mut jwt = String::new();
        // Legacy enterprise API keys authenticate directly in metadata.
        if !connection
            .secret()
            .is_some_and(|key| key.starts_with("sk-ws-"))
        {
            let mut request = Encode::default();
            request.bytes(1, &metadata(connection, "", false).0);
            let bytes = unary(http, connection, AUTH, request.0, control, run, events).await?;
            let response = Message(&bytes);
            jwt = response.text(1)?.to_string();
            if jwt.is_empty() || jwt.len() > 64 * 1024 || !jwt.bytes().all(|b| b.is_ascii_graphic())
            {
                return Err("Devin GetUserJwt returned no valid user JWT".into());
            }
            let custom = response.text(2)?.trim();
            reviewed_route(&base, custom)?;
        }
        Ok(Self {
            base,
            jwt,
            cascade: uuid()?,
            model,
        })
    }
    pub(super) async fn request(
        &self,
        http: &reqwest::Client,
        connection: &Connection,
        body: &Value,
        control: &Control<'_>,
        run: &Prepared,
        events: &Events,
    ) -> Result<Vec<u8>, String> {
        let mut policy = String::new();
        let messages = body["messages"]
            .as_array()
            .ok_or("Missing Devin messages")?;
        for message in messages {
            if message["role"] == "system" {
                if !policy.is_empty() {
                    policy.push_str("\n\n");
                }
                policy.push_str(
                    message["content"]
                        .as_str()
                        .ok_or("Invalid Devin system policy")?,
                );
            }
        }
        let mut model: std::borrow::Cow<'_, str> = self.model.option.id.as_str().into();
        let mut assignment = String::new();
        if self.model.router {
            let mut request = Encode::default();
            request.bytes(1, &metadata(connection, &self.jwt, false).0);
            request.text(2, &model);
            request.text(3, &self.cascade);
            if let Some(message) = body["messages"]
                .as_array()
                .and_then(|messages| messages.iter().rfind(|message| message["role"] == "user"))
            {
                request.bytes(5, &prompt(message, &self.cascade, 0)?.0);
            }
            let bytes = unary(http, connection, ASSIGN, request.0, control, run, events).await?;
            let assigned = Message(&bytes).message(1)?;
            assignment = assigned.text(1)?.to_string();
            model = checked_identity(assigned.text(2)?)?.into();
            if assignment.is_empty()
                || assignment.len() > 64 * 1024
                || !assignment.bytes().all(|b| b.is_ascii_graphic())
            {
                return Err("Devin AssignModel returned no valid assignment JWT".into());
            }
        }
        let mut out = Encode::default();
        out.bytes(1, &metadata(connection, &self.jwt, false).0);
        out.text(2, &policy);
        for (index, message) in messages.iter().enumerate() {
            if message["role"] != "system" {
                out.bytes(3, &prompt(message, &self.cascade, index)?.0);
            }
        }
        out.text(21, &model);
        out.text(26, &assignment);
        out.number(7, 5);
        out.number(20, 1);
        out.text(16, &self.cascade);
        out.text(22, &uuid()?);
        let mut configuration = Encode::default();
        configuration.number(1, 1);
        configuration.number(2, self.model.max_output);
        configuration.number(3, 400);
        configuration.double(5, 1.0);
        configuration.number(7, 40);
        configuration.double(8, f64::from(0.95f32));
        out.bytes(8, &configuration.0);
        out.number(11, 1);
        let mut choice = Encode::default();
        choice.text(1, "auto");
        out.bytes(12, &choice.0);
        let mut cache = Encode::default();
        cache.number(1, 1);
        out.bytes(13, &cache.0);
        let google = model
            .as_bytes()
            .windows(6)
            .any(|part| part.eq_ignore_ascii_case(b"gemini"));
        if let Some(tools) = body["tools"].as_array() {
            for tool in tools {
                let tool = &tool["function"];
                let mut definition = Encode::default();
                definition.text(1, tool["name"].as_str().ok_or("Missing Devin tool name")?);
                definition.text(2, tool["description"].as_str().unwrap_or_default());
                let schema = if google {
                    let mut schema = tool["parameters"].clone();
                    google_schema(&mut schema)?;
                    serde_json::to_string(&schema)
                } else {
                    serde_json::to_string(&tool["parameters"])
                }
                .map_err(|_| "Invalid Devin tool schema")?;
                definition.text(3, &schema);
                if tool["strict"] == true {
                    definition.number(12, 1);
                }
                out.bytes(10, &definition.0);
            }
        }
        if out.0.len() > BODY_LIMIT {
            return Err("Devin protobuf request exceeds native body capacity; nothing sent".into());
        }
        Ok(frame(&out.0))
    }
    pub(super) fn url(&self) -> String {
        format!("{}{CHAT}", self.base)
    }
}

/// Cascade's Google backend expects scalar types. Preserve all union branches
/// rather than silently dropping alternatives or weakening host validation.
fn google_schema(schema: &mut Value) -> Result<(), String> {
    let Some(object) = schema.as_object_mut() else {
        return Ok(());
    };
    for key in ["properties", "$defs", "definitions", "patternProperties"] {
        if let Some(children) = object.get_mut(key).and_then(Value::as_object_mut) {
            for child in children.values_mut() {
                google_schema(child)?;
            }
        }
    }
    for key in ["items", "additionalProperties", "contains", "not"] {
        if let Some(child) = object.get_mut(key) {
            google_schema(child)?;
        }
    }
    for key in ["anyOf", "oneOf", "allOf", "prefixItems"] {
        if let Some(children) = object.get_mut(key).and_then(Value::as_array_mut) {
            for child in children {
                google_schema(child)?;
            }
        }
    }
    if object.get("type").is_some_and(Value::is_array) {
        let types = object.remove("type").ok_or("Missing Devin schema types")?;
        let types = types.as_array().ok_or("Invalid Devin schema types")?;
        let mut non_null = Vec::new();
        for kind in types {
            let kind = kind.as_str().ok_or("Invalid Devin schema type")?;
            if kind == "null" {
                object.insert("nullable".into(), json!(true));
            } else if !non_null.contains(&kind) {
                non_null.push(kind);
            }
        }
        match non_null.as_slice() {
            [] => {
                object.insert("type".into(), json!("null"));
            }
            [kind] => {
                object.insert("type".into(), json!(kind));
            }
            kinds => {
                let union = json!({"anyOf":kinds.iter().map(|kind| json!({"type":kind})).collect::<Vec<_>>()});
                object
                    .entry("allOf")
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                    .ok_or("Invalid Devin schema intersection")?
                    .push(union);
            }
        }
    }
    Ok(())
}

fn prompt(message: &Value, cascade: &str, index: usize) -> Result<Encode, String> {
    use sha2::Digest;
    let mut out = Encode::default();
    let role = message["role"]
        .as_str()
        .ok_or("Invalid Devin message role")?;
    let digest = sha2::Sha256::digest(format!("{cascade}\0{index}\0{role}"));
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    id[6] = (id[6] & 15) | 80;
    id[8] = (id[8] & 63) | 128;
    let hex: String = id.iter().map(|b| format!("{b:02x}")).collect();
    out.text(
        1,
        &format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        ),
    );
    out.number(
        2,
        match role {
            "user" => 1,
            "assistant" => 2,
            "tool" => 4,
            _ => return Err("Unsupported Devin message role".into()),
        },
    );
    if let Some(content) = message["content"].as_str() {
        out.text(3, content);
    } else if !message["content"].is_null() {
        return Err("Devin supports text messages only on this native route".into());
    }
    out.text(
        11,
        message["reasoning_content"].as_str().unwrap_or_default(),
    );
    out.text(12, message["devin_signature"].as_str().unwrap_or_default());
    out.text(
        18,
        message["devin_signature_type"].as_str().unwrap_or_default(),
    );
    if let Some(encoded) = message["devin_gemini_signature"].as_str() {
        let bytes = base64_simd::STANDARD
            .decode_to_vec(encoded.as_bytes())
            .map_err(|_| "Invalid retained Devin thought signature")?;
        out.bytes(17, &bytes);
    }
    if role == "tool" {
        out.text(
            7,
            message["tool_call_id"]
                .as_str()
                .ok_or("Missing Devin tool-result identity")?,
        );
        if let Some(content) = message["content"].as_str() {
            let result: Value =
                serde_json::from_str(content).map_err(|_| "Invalid Devin local tool result")?;
            out.number(9, u64::from(result["status"] != "success"));
        }
    }
    if let Some(calls) = message["tool_calls"].as_array() {
        for call in calls {
            let mut encoded = Encode::default();
            encoded.text(1, call["id"].as_str().ok_or("Missing Devin call identity")?);
            encoded.text(
                2,
                call["function"]["name"]
                    .as_str()
                    .ok_or("Missing Devin call name")?,
            );
            encoded.text(
                3,
                call["function"]["arguments"]
                    .as_str()
                    .ok_or("Missing Devin call arguments")?,
            );
            out.bytes(6, &encoded.0);
        }
    }
    Ok(out)
}
fn frame(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 5);
    out.push(0);
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
    out
}
#[derive(Default)]
struct Frames {
    pending: Vec<u8>,
}
impl Frames {
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<(u8, Vec<u8>)>, String> {
        // Network chunks and payloads are independently bounded.
        if self.pending.len().saturating_add(bytes.len()) > 2 * FRAME_LIMIT + 10 {
            return Err("Devin Connect buffered data exceeds capacity".into());
        }
        self.pending.extend_from_slice(bytes);
        let mut frames = Vec::new();
        let mut consumed = 0;
        while self.pending.len() - consumed >= 5 {
            let header = &self.pending[consumed..consumed + 5];
            let flags = header[0];
            if flags & !3 != 0 {
                return Err("Invalid Devin Connect frame flags".into());
            }
            let length = u32::from_be_bytes(
                header[1..5]
                    .try_into()
                    .map_err(|_| "Invalid Connect header")?,
            ) as usize;
            if length > FRAME_LIMIT {
                return Err("Devin Connect frame exceeds 16 MiB capacity".into());
            }
            if self.pending.len() - consumed - 5 < length {
                break;
            }
            let start = consumed + 5;
            frames.push((flags, self.pending[start..start + length].to_vec()));
            consumed = start + length;
        }
        if consumed != 0 {
            self.pending.drain(..consumed);
        }
        Ok(frames)
    }
    fn finish(&self) -> Result<(), String> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err("Truncated Devin Connect frame".into())
        }
    }
}
fn decompress(flags: u8, bytes: &[u8]) -> Result<std::borrow::Cow<'_, [u8]>, String> {
    if flags & 1 == 0 {
        return Ok(std::borrow::Cow::Borrowed(bytes));
    }
    let mut decoder = flate2::read::GzDecoder::new(bytes).take((FRAME_LIMIT + 1) as u64);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|_| "Invalid Devin Connect gzip payload")?;
    if out.len() > FRAME_LIMIT {
        return Err("Devin Connect inflated frame exceeds capacity".into());
    }
    Ok(std::borrow::Cow::Owned(out))
}
fn trailer(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Invalid Devin Connect trailer UTF-8")?;
    let value = tools::frame(text)?;
    if !value.is_object() {
        return Err("Invalid Devin Connect trailer".into());
    }
    if value.get("error").is_some_and(|error| !error.is_null()) {
        return Err(fault::from_envelope(&value).describe("Devin Connect stream rejected"));
    }
    Ok(())
}

#[derive(Default)]
struct Response {
    calls: BTreeMap<String, (usize, String, String)>,
    active: Option<String>,
    stop: u64,
    ended: bool,
    thinking_bytes: usize,
}
impl Response {
    fn message(&mut self, bytes: &[u8], stream: &mut Stream) -> Result<(), String> {
        let message = Message(bytes);
        let id = message.text(1)?;
        if !id.is_empty() {
            checked_identity(id)?;
            if stream.response_id.as_deref().is_some_and(|old| old != id) {
                return Err("Devin response identity changed within round".into());
            }
            stream.response_id = Some(id.into());
        }
        let model = message.text(23)?;
        if !model.is_empty() {
            stream.observe(Some(model))?;
        }
        let text = message.text(3)?;
        let thinking = message.text(9)?;
        if self.stop != 0
            && (!text.is_empty()
                || !thinking.is_empty()
                || message.fields().any(|field| matches!(field, Ok((6, _)))))
        {
            return Err("Devin content after model stop".into());
        }
        self.thinking_bytes = self.thinking_bytes.saturating_add(thinking.len());
        if self.thinking_bytes > BODY_LIMIT {
            return Err("Devin thinking exceeds transcript capacity".into());
        }
        let signature = message.text(10)?;
        if signature.len() > BODY_LIMIT {
            return Err("Devin signature exceeds capacity".into());
        }
        stream.append(text)?;
        let gemini = message.bytes(20)?.unwrap_or_default();
        if gemini.len() > BODY_LIMIT {
            return Err("Devin thought signature exceeds capacity".into());
        }
        let mut delta =
            json!({"content":text,"reasoning_content":thinking,"devin_signature":signature});
        let signature_type = message.text(21)?;
        if !signature_type.is_empty() {
            delta["devin_signature_type"] = json!(signature_type);
        }
        if !gemini.is_empty() {
            delta["devin_gemini_signature"] = json!(base64_simd::STANDARD.encode_to_string(gemini));
        }
        stream
            .calls
            .observe("", &json!({"choices":[{"delta":delta}]}))?;
        if let Some(usage) = message.bytes(7)? {
            let usage = Message(usage);
            let counters = stream.usage.get_or_insert_with(Counters::default);
            if let Some(value) = usage.number(2)? {
                counters.input = Some(value);
            }
            if let Some(value) = usage.number(3)? {
                counters.output = Some(value);
            }
            if let Some(value) = usage.number(4)? {
                counters.cache_write = Some(value);
            }
            if let Some(value) = usage.number(5)? {
                counters.cache_read = Some(value);
            }
        }
        for field in message.fields() {
            let (number, value) = field?;
            if number != 6 {
                continue;
            }
            let Wire::Bytes(bytes) = value else {
                return Err("Invalid Devin tool-call envelope".into());
            };
            let call = Message(bytes);
            if !call.text(4)?.is_empty() || !call.text(5)?.is_empty() {
                return Err("Devin reported malformed tool arguments".into());
            }
            let id = call.text(1)?;
            let id = if id.is_empty() {
                self.active
                    .as_deref()
                    .ok_or("Devin tool fragment has no active identity")?
            } else {
                id
            };
            if id.len() > 40
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err("Invalid Devin tool call identity".into());
            }
            if !self.calls.contains_key(id) && self.calls.len() >= 64 {
                return Err("Devin round exceeds tool-call capacity".into());
            }
            let index = self.calls.len();
            let entry = self
                .calls
                .entry(id.to_string())
                .or_insert_with(|| (index, String::new(), String::new()));
            let name = call.text(2)?;
            if !name.is_empty() {
                if !entry.1.is_empty() && entry.1 != name {
                    return Err("Devin tool name changed within call".into());
                }
                entry.1 = checked_identity(name)?;
            }
            let arguments = call.text(3)?;
            let delta = if arguments.starts_with(&entry.2) {
                &arguments[entry.2.len()..]
            } else {
                arguments
            };
            if entry.2.len().saturating_add(delta.len()) > BODY_LIMIT {
                return Err("Devin tool arguments exceed capacity".into());
            }
            entry.2.push_str(delta);
            let mut wire = json!({"index":entry.0,"function":{"arguments":delta}});
            if self.active.as_deref() != Some(id) || entry.2.len() == delta.len() {
                wire["id"] = json!(id);
            }
            if !name.is_empty() {
                wire["function"]["name"] = json!(name);
            }
            stream
                .calls
                .observe("", &json!({"choices":[{"delta":{"tool_calls":[wire]}}]}))?;
            self.active = Some(id.to_string());
        }
        if let Some(stop) = message.number(5)?.filter(|value| *value != 0) {
            if self.stop != 0 && self.stop != stop {
                return Err("Devin stop reason changed within response".into());
            }
            self.stop = stop;
        }
        Ok(())
    }
    fn finish(&self, stream: &mut Stream) -> Result<(), String> {
        if !self.ended {
            return Err("Devin Connect ended without authoritative trailer".into());
        }
        if !matches!(self.stop, 0 | 2 | 6 | 8 | 10 | 12) {
            stream.failed = true;
            return Err(match self.stop {
                3 | 5 => "Devin output limit reached; response incomplete",
                11 => "Devin content filter stopped the response",
                _ => "Devin returned a non-success stop reason",
            }
            .into());
        }
        if self.stop == 10 && self.calls.is_empty() {
            return Err("Devin function stop has no calls".into());
        }
        stream.calls.observe("", &json!({"choices":[{"delta":{},"finish_reason":if self.calls.is_empty() {"stop"} else {"tool_calls"}}]}))?;
        stream.complete = true;
        Ok(())
    }
}
pub(super) async fn read(
    response: &mut reqwest::Response,
    stream: &mut Stream,
    control: &Control<'_>,
    run: &Prepared,
    events: &Events,
) -> Result<(), String> {
    let encoding = response
        .headers()
        .get("connect-content-encoding")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("identity")
        .to_string();
    if !matches!(encoding.as_str(), "identity" | "gzip") {
        return Err("Unsupported Devin Connect compression".into());
    }
    let mut frames = Frames::default();
    let mut state = Response::default();
    loop {
        let mut next = std::pin::pin!(response.chunk());
        let chunk = loop {
            control.check()?;
            match tokio::time::timeout(Duration::from_millis(20), &mut next).await {
                Ok(result) => {
                    break result.map_err(|_| {
                        stream.transport_failed = !control.cancelled();
                        "Devin Connect HTTP transport failed".to_string()
                    })?;
                }
                Err(_) if stream.calls.has_pending_previews() => {
                    stream.publish_file_previews(events, run)?
                }
                Err(_) => {}
            }
        };
        let Some(chunk) = chunk else {
            break;
        };
        stream.wire_bytes = stream.wire_bytes.saturating_add(chunk.len());
        if stream.wire_bytes > stream.wire_limit {
            return Err("Devin stream exceeds cumulative byte capacity".into());
        }
        for (flags, payload) in frames.feed(&chunk)? {
            control.check()?;
            if state.ended {
                return Err("Devin Connect data after terminal trailer".into());
            }
            if flags & 1 != 0 && encoding != "gzip" {
                return Err("Devin compressed frame without negotiated gzip".into());
            }
            let payload = decompress(flags, &payload)?;
            if flags & 2 != 0 {
                trailer(&payload).inspect_err(|_| {
                    stream.failed = true;
                })?;
                state.ended = true;
            } else {
                state.message(&payload, stream)?;
            }
            stream.publish(events, run)?;
        }
    }
    frames.finish().inspect_err(|_| {
        stream.transport_failed = !control.cancelled();
    })?;
    if !state.ended {
        stream.transport_failed = !control.cancelled();
    }
    state.finish(stream)
}
