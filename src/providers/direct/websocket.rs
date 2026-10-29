//! One logical Run's bounded Responses connection. The IO owner has no input,
//! permission, tool execution or replay authority; it only services this socket.
use super::*;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

/// Counts actual socket bytes, including fragmented/control framing and TLS.
struct BoundedIo {
    socket: tokio::net::TcpStream,
    incoming: usize,
    outgoing: usize,
}
impl AsyncRead for BoundedIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buffer.filled().len();
        match Pin::new(&mut this.socket).poll_read(cx, buffer) {
            Poll::Ready(Ok(())) => {
                this.incoming = this.incoming.saturating_add(buffer.filled().len() - before);
                Poll::Ready(if this.incoming > STREAM_LIMIT {
                    Err(io::Error::other(
                        "WebSocket actual incoming wire capacity exceeded",
                    ))
                } else {
                    Ok(())
                })
            }
            other => other,
        }
    }
}
impl AsyncWrite for BoundedIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        // Bounded framing/TLS/control overhead, besides inference-frame bounds.
        let remaining = (REQUEST_BYTES_LIMIT + FRAME_LIMIT).saturating_sub(this.outgoing);
        if bytes.len() > remaining {
            return Poll::Ready(Err(io::Error::other(
                "WebSocket actual outgoing wire capacity exceeded",
            )));
        }
        match Pin::new(&mut this.socket).poll_write(cx, bytes) {
            Poll::Ready(Ok(written)) => {
                this.outgoing += written;
                Poll::Ready(Ok(written))
            }
            other => other,
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().socket).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().socket).poll_shutdown(cx)
    }
}

pub(super) fn initial_frame(body: &mut Value) -> Result<(), String> {
    let fields = body
        .as_object_mut()
        .ok_or("Responses request is not an object")?;
    fields.remove("stream");
    fields.remove("background");
    fields.insert("type".into(), json!("response.create"));
    fields.insert("store".into(), json!(false));
    Ok(())
}

pub(super) fn continuation(
    body: &Value,
    response: &str,
    outputs: Vec<Value>,
) -> Result<String, String> {
    checked_identity(response)?;
    // Actual function outputs, optionally followed by explicit user steering (XC03).
    let steering = outputs
        .iter()
        .rev()
        .take_while(|item| item["type"] == "message" && item["role"] == "user")
        .count();
    if outputs.len() == steering
        || outputs[..outputs.len() - steering]
            .iter()
            .any(|item| item["type"] != "function_call_output")
    {
        return Err("WebSocket continuation requires only actual function outputs".into());
    }
    // Copy only frozen options, never the growing full logical input.
    let mut next = Value::Object(
        body.as_object()
            .ok_or("Responses options missing")?
            .iter()
            .filter(|(key, _)| key.as_str() != "input")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    );
    next["input"] = Value::Array(outputs);
    next["previous_response_id"] = json!(response);
    initial_frame(&mut next)?;
    serde_json::to_string(&next).map_err(|_| "Cannot encode WebSocket continuation".into())
}

struct Frame {
    value: Value,
    bytes: usize,
}
struct Queue {
    frames: VecDeque<Frame>,
    bytes: usize,
    error: Option<String>,
}
struct SendFrame {
    text: String,
    reply: SyncSender<Result<(), String>>,
}

pub(super) struct Session {
    sends: SyncSender<SendFrame>,
    queue: Arc<Mutex<Queue>>,
    stop: Arc<AtomicBool>,
    pub(super) alive: Arc<AtomicBool>,
    submitted: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    prior_ids: HashSet<String>,
    dispatched: bool,
}
impl Session {
    pub(super) fn start(
        connection: &Connection,
        http: &reqwest::Client,
        run: &Prepared,
        events: &Events,
    ) -> Result<Self, String> {
        let destination = connection.websocket_url()?;
        let request = connection.websocket_request(http, &destination)?;
        let (sends, commands) = mpsc::sync_channel::<SendFrame>(1);
        let queue = Arc::new(Mutex::new(Queue {
            frames: VecDeque::new(),
            bytes: 0,
            error: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        let submitted = Arc::new(AtomicBool::new(false));
        let sending = submitted.clone();
        let (output, stopped, healthy) = (queue.clone(), stop.clone(), alive.clone());
        let events = events.clone();
        let (generation, task_id, agent) = (run.generation, run.input.task.id, run.input.agent);
        let thread = thread::Builder::new().name("ev-native-ws-io".into()).spawn(move || {
            let notify = |failure| {
                let _ = emit(&events, Event::ConnectionFailed { generation, task_id, agent, failure });
            };
            let result = (|| {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()
                    .map_err(|_| "Cannot initialize WebSocket IO runtime".to_string())?;
                let result = runtime.block_on(async {
                    let config = WebSocketConfig::default()
                        .max_message_size(Some(FRAME_LIMIT))
                        .max_frame_size(Some(FRAME_LIMIT))
                        .max_write_buffer_size(BODY_LIMIT * 2);
                    let endpoint = reqwest::Url::parse(&destination)
                        .map_err(|_| "Frozen WebSocket destination is invalid".to_string())?;
                    let host = endpoint.host_str().ok_or("WebSocket host missing")?.trim_matches(['[', ']']).to_owned();
                    let port = endpoint.port_or_known_default().or_else(|| match endpoint.scheme() {
                        "wss" => Some(443), "ws" => Some(80), _ => None,
                    }).ok_or("WebSocket port unavailable")?;
                    let connect = async {
                        let socket = tokio::net::TcpStream::connect((host.as_str(), port)).await
                            .map_err(tokio_tungstenite::tungstenite::Error::Io)?;
                        socket.set_nodelay(true).map_err(tokio_tungstenite::tungstenite::Error::Io)?;
                        let bounded = BoundedIo { socket, incoming: 0, outgoing: 0 };
                        tokio_tungstenite::client_async_tls_with_config(request, bounded, Some(config), None).await
                    };
                    let mut connect = std::pin::pin!(connect);
                    let until = Instant::now() + Duration::from_secs(15);
                    let mut socket = loop {
                        if stopped.load(Ordering::Acquire) { return Ok(()); }
                        if Instant::now() >= until {
                            notify(super::super::diagnostics::Failure::Transport);
                            return Err("WebSocket upgrade timed out; no model receipt".into());
                        }
                        match tokio::time::timeout(Duration::from_millis(20), &mut connect).await {
                            Ok(Ok((socket, _upgrade))) => break socket,
                            Ok(Err(error)) => {
                                if let tokio_tungstenite::tungstenite::Error::Http(response) = error {
                                    let status = response.status().as_u16();
                                    notify(super::super::diagnostics::Failure::Http(status));
                                    return Err(format!("{}; WebSocket upgrade refused; no retry or fallback", http_status(status)));
                                }
                                notify(if matches!(error, tokio_tungstenite::tungstenite::Error::Io(_) | tokio_tungstenite::tungstenite::Error::Tls(_)) {
                                    super::super::diagnostics::Failure::Transport
                                } else {
                                    super::super::diagnostics::Failure::Incompatible
                                });
                                return Err("WebSocket upgrade failed; no retry, redirect or HTTP fallback".into());
                            }
                            Err(_) => {}
                        }
                    };
                    // HTTP101 is deliberately not a Dispatched event.
                    let mut active = false;
                    let mut clean_terminal = false;
                    let mut total = 0usize;
                    loop {
                        if stopped.load(Ordering::Acquire) {
                            let _ = tokio::time::timeout(Duration::from_millis(100), socket.close(None)).await;
                            return Ok(());
                        }
                        match commands.try_recv() {
                            Ok(command) => {
                                if active || !output.lock().frames.is_empty() {
                                    let _ = command.reply.try_send(Err("WebSocket overlapping or queued response refused".into()));
                                    return Err("WebSocket round ownership mismatch".into());
                                }
                                active = true;
                                clean_terminal = false;
                                sending.store(true, Ordering::Release);
                                let send = socket.send(Message::Text(command.text.into()));
                                let mut send = std::pin::pin!(send);
                                let deadline = Instant::now() + CALL_TIMEOUT;
                                let ok = loop {
                                    if stopped.load(Ordering::Acquire) || Instant::now() >= deadline { break false; }
                                    if let Ok(result) = tokio::time::timeout(Duration::from_millis(20), &mut send).await {
                                        break result.is_ok()
                                    }
                                };
                                let _ = command.reply.try_send(if ok { Ok(()) } else { Err("WebSocket send failed; remote outcome unknown; no replay".into()) });
                                if !ok {
                                    if !stopped.load(Ordering::Acquire) { notify(super::super::diagnostics::Failure::Transport); }
                                    return Err("WebSocket send failed; remote outcome unknown; no replay".into());
                                }
                            }
                            Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                            Err(mpsc::TryRecvError::Empty) => {}
                        }
                        let frame = match tokio::time::timeout(Duration::from_millis(20), socket.next()).await {
                            Err(_) => continue,
                            Ok(Some(Ok(frame))) => frame,
                            Ok(Some(Err(error))) => {
                                healthy.store(false, Ordering::Release);
                                if !clean_terminal && matches!(error, tokio_tungstenite::tungstenite::Error::Io(_)
                                    | tokio_tungstenite::tungstenite::Error::Tls(_)
                                    | tokio_tungstenite::tungstenite::Error::ConnectionClosed
                                    | tokio_tungstenite::tungstenite::Error::AlreadyClosed)
                                {
                                    notify(super::super::diagnostics::Failure::Transport);
                                }
                                return Err("WebSocket reader failed; no reconnect or replay".into());
                            }
                            Ok(None) => {
                                healthy.store(false, Ordering::Release);
                                if !clean_terminal { notify(super::super::diagnostics::Failure::Transport); }
                                return Err("WebSocket closed; no reconnect or replay".into());
                            }
                        };
                        match frame {
                            Message::Ping(_) | Message::Pong(_) => {
                                // tungstenite queues the exact automatic pong; flush it even
                                // while the only permission/input owner is synchronously waiting.
                                let flush = socket.flush();
                                let mut flush = std::pin::pin!(flush);
                                let deadline = Instant::now() + Duration::from_secs(5);
                                let flushed = loop {
                                    if stopped.load(Ordering::Acquire) { return Ok(()); }
                                    if Instant::now() >= deadline { break false; }
                                    if let Ok(result) = tokio::time::timeout(Duration::from_millis(20), &mut flush).await {
                                        break result.is_ok()
                                    }
                                };
                                if !flushed {
                                    healthy.store(false, Ordering::Release);
                                    if !clean_terminal { notify(super::super::diagnostics::Failure::Transport); }
                                    return Err("WebSocket control flush failed or timed out".into());
                                }
                            }
                            Message::Close(_) => {
                                healthy.store(false, Ordering::Release);
                                if !clean_terminal { notify(super::super::diagnostics::Failure::Transport); }
                                let _ = tokio::time::timeout(Duration::from_millis(100), socket.flush()).await;
                                return Err("Provider closed the request-owned WebSocket; no replay".into());
                            }
                            Message::Text(text) => {
                                if !active { return Err("Unsolicited or stale WebSocket response refused".into()); }
                                total = total.checked_add(text.len()).filter(|total| *total <= STREAM_LIMIT)
                                    .ok_or("WebSocket aggregate output exceeds byte capacity")?;
                                // Message reassembly is bounded by tungstenite before UTF-8/JSON.
                                let value = tools::frame(&text)?;
                                if matches!(value["type"].as_str(), Some("response.completed" | "response.failed" | "response.incomplete" | "error")) {
                                    active = false;
                                    clean_terminal = value["type"] != "response.completed"
                                        || !value.pointer("/response/output").and_then(Value::as_array)
                                            .is_some_and(|items| items.iter().any(|item| item["type"] == "function_call"));
                                }
                                let mut queue = output.lock();
                                if queue.frames.len() >= QUEUE_LIMIT || queue.bytes.saturating_add(text.len()) > FRAME_LIMIT {
                                    return Err("WebSocket bounded event queue overflow; no continuation".into());
                                }
                                queue.bytes += text.len();
                                queue.frames.push_back(Frame { value, bytes: text.len() });
                            }
                            _ => return Err("Binary or unsupported WebSocket message refused".into()),
                        }
                    }
                });
                runtime.shutdown_timeout(Duration::from_millis(100));
                result
            })();
            healthy.store(false, Ordering::Release);
            if let Err(error) = result { output.lock().error = Some(error); }
        }).map_err(|_| "Cannot start narrowly owned WebSocket IO")?;
        Ok(Self {
            sends,
            queue,
            stop,
            alive,
            submitted,
            thread: Some(thread),
            prior_ids: HashSet::new(),
            dispatched: false,
        })
    }

    pub(super) fn submitted(&self) -> bool {
        self.submitted.load(Ordering::Acquire)
    }

    pub(super) async fn round(
        &mut self,
        wire: String,
        stream: &mut Stream,
        run: &Prepared,
        events: &Events,
        control: &Control<'_>,
    ) -> Result<(), String> {
        control.check()?;
        let (reply, sent) = mpsc::sync_channel(1);
        self.sends
            .try_send(SendFrame { text: wire, reply })
            .map_err(|_| "WebSocket send ownership unavailable")?;
        loop {
            control.check()?;
            match sent.try_recv() {
                Ok(result) => {
                    result?;
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(self.queue.lock().error.clone().unwrap_or_else(|| {
                        if self.submitted() {
                            "WebSocket IO stopped after send attempt; remote outcome unknown".into()
                        } else {
                            "WebSocket IO stopped before any model frame attempt".into()
                        }
                    }));
                }
                Err(mpsc::TryRecvError::Empty) => {
                    tokio::time::sleep(Duration::from_millis(5)).await
                }
            }
        }
        let mut lane = Lane::default();
        loop {
            control.check()?;
            let data = {
                let mut queue = self.queue.lock();
                if let Some(data) = queue.frames.pop_front() {
                    queue.bytes -= data.bytes;
                    Some(data)
                } else if let Some(error) = &queue.error {
                    return Err(error.clone());
                } else {
                    None
                }
            };
            let Some(data) = data else {
                if !self.alive.load(Ordering::Acquire) {
                    return Err("WebSocket IO ended without attributable completion".into());
                }
                if stream.calls.has_pending_previews() {
                    stream.publish_file_previews(events, run)?;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
                continue;
            };
            stream.wire_bytes = stream
                .wire_bytes
                .checked_add(data.bytes)
                .filter(|bytes| *bytes <= stream.wire_limit)
                .ok_or("WebSocket stream exceeds aggregate byte capacity")?;
            let received = lane.observe(&data.value, &self.prior_ids)?;
            if received && !self.dispatched {
                if let Some(event) = run.input.dispatched(run.generation) {
                    emit(events, event)?;
                }
                self.dispatched = true;
            }
            stream.event_value("", &data.value)?;
            stream.publish(events, run)?;
            if stream.complete {
                let id = lane
                    .response
                    .take()
                    .ok_or("WebSocket completion lacks owned response ID")?;
                self.prior_ids.insert(id.clone());
                stream.response_id = Some(id);
                return Ok(());
            }
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
struct Lane {
    response: Option<String>,
    sequence: Option<u64>,
    items: HashMap<String, u64>,
}
impl Lane {
    fn observe(&mut self, value: &Value, prior: &HashSet<String>) -> Result<bool, String> {
        if value
            .get("stream_id")
            .or_else(|| value.pointer("/response/stream_id"))
            .is_some_and(|id| !id.is_null())
        {
            return Err("WebSocket named/interleaved response lane refused".into());
        }
        let kind = value["type"]
            .as_str()
            .ok_or("WebSocket response type missing")?;
        if !kind.starts_with("response.") && kind != "error" {
            return Err("Unattributable WebSocket event; remote outcome unknown".into());
        }
        let seq = value["sequence_number"]
            .as_u64()
            .ok_or("WebSocket sequence number missing")?;
        if self.sequence.is_some_and(|previous| seq <= previous) {
            return Err("WebSocket duplicate or regressed sequence refused".into());
        }
        self.sequence = Some(seq);
        let identity = value
            .get("response_id")
            .or_else(|| value.pointer("/response/id"))
            .and_then(Value::as_str);
        if let (Some(top), Some(nested)) = (value.get("response_id"), value.pointer("/response/id"))
            && top != nested
        {
            return Err("WebSocket conflicting response identities refused".into());
        }
        if kind == "response.created" {
            if self.response.is_some() {
                return Err("WebSocket interleaved response.created refused".into());
            }
            let id =
                checked_identity(identity.ok_or("WebSocket creation response identity missing")?)?;
            if prior.contains(&id) {
                return Err("WebSocket stale response identity refused".into());
            }
            self.response = Some(id);
            return Ok(true);
        }
        let owner = self
            .response
            .as_deref()
            .ok_or("WebSocket response arrived before attributable creation")?;
        if let Some(id) = identity {
            if id != owner {
                return Err("WebSocket mismatched response identity refused".into());
            }
        } else if !value
            .get("item_id")
            .and_then(Value::as_str)
            .is_some_and(|id| self.items.contains_key(id))
        {
            return Err("WebSocket event has no attributable response/item identity".into());
        }
        if kind == "response.output_item.added" {
            let id = checked_identity(
                value
                    .pointer("/item/id")
                    .and_then(Value::as_str)
                    .ok_or("WebSocket item identity missing")?,
            )?;
            let index = value["output_index"]
                .as_u64()
                .ok_or("WebSocket output index missing")?;
            if self.items.len() >= TOOL_CALL_LIMIT * 2 + 32
                // A prior reasoning/message item may appear only in the terminal
                // snapshot. Added events are ordered, not necessarily contiguous.
                || self.items.values().any(|owned| *owned >= index)
                || self.items.insert(id, index).is_some()
            {
                return Err("WebSocket duplicate/interleaved output item refused".into());
            }
        }
        if let Some(index) = value.get("output_index") {
            let index = index.as_u64().ok_or("WebSocket output index is invalid")?;
            if !self.items.values().any(|owned| *owned == index) {
                return Err("WebSocket unknown output lane refused".into());
            }
        }
        if let Some(id) = value
            .get("item_id")
            .or_else(|| value.pointer("/item/id"))
            .and_then(Value::as_str)
        {
            let expected = self.items.get(id).ok_or("WebSocket unknown output item")?;
            if value
                .get("output_index")
                .is_some_and(|index| index.as_u64() != Some(*expected))
            {
                return Err("WebSocket output lane changed".into());
            }
        }
        Ok(false)
    }
}
