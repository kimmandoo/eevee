//! User-registered stdio MCP capabilities. Process ownership is not a sandbox.
use crate::workspace::{forward, runner};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
mod effects;
pub use effects::Effects;

pub const LIMIT: usize = 64 * 1024;
static SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Registration {
    pub name: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_cwd")]
    pub cwd: PathBuf,
    /// Environment variable names only; never persist credential values.
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub critical: Vec<String>,
    #[serde(default, flatten)]
    pub extra: forward::Extra,
}
fn default_cwd() -> PathBuf {
    PathBuf::from(".")
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && !value.contains("__")
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
impl Registration {
    pub fn prepare(&self, project: &Path) -> Result<runner::Prepared, String> {
        if !identifier(&self.name)
            || self.args.len() > 64
            || self.env.len() > 32
            || self.env.iter().any(|key| !identifier(key))
            || self.program.chars().any(char::is_control)
            || self.args.iter().any(|arg| {
                arg.len() > 4096
                    || arg.chars().any(char::is_control)
                    || crate::workspace::privacy::redact(arg) != arg.as_str()
            })
        {
            return Err("MCP requires a safe server name, direct argv without credentials/controls, and environment names only".into());
        }
        forward::check_critical(&self.critical, &self.extra, |_| None)?;
        let encoded = serde_json::to_string(self).map_err(|_| "Cannot encode MCP registration")?;
        if encoded.len() > LIMIT || crate::workspace::privacy::redact(&encoded) != encoded {
            return Err("MCP registration contains recognizable credentials or exceeds 64 KiB; use environment names, not plaintext values".into());
        }
        let root = project.canonicalize().map_err(|e| e.to_string())?;
        let cwd = root
            .join(&self.cwd)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !cwd.starts_with(&root) {
            return Err("MCP cwd must be inside this project".into());
        }
        let environment = ["PATH", "SystemRoot"]
            .into_iter()
            .chain(self.env.iter().map(String::as_str))
            .map(|key| (key, std::env::var(key).ok()))
            .collect::<Vec<_>>();
        let fingerprint = super::catalog::digest(
            &serde_json::to_vec(&environment).map_err(|_| "Cannot fingerprint environment")?,
        );
        let prepared = runner::prepare(
            format!("mcp-{}", self.name),
            0,
            runner::CommandSpec::Direct {
                program: self.program.clone(),
                args: self.args.clone(),
            },
            cwd,
            runner::InputSnapshot {
                fingerprint: Some(fingerprint),
                complete: true,
                stable: true,
            },
        )?;
        let basename = prepared
            .executable
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        if matches!(
            basename.as_str(),
            "sh" | "bash" | "zsh" | "fish" | "cmd" | "powershell" | "pwsh"
        ) {
            return Err(
                "MCP shell launchers are refused; register the direct executable and argv".into(),
            );
        }
        Ok(prepared)
    }
    pub fn review(&self, prepared: &runner::Prepared) -> String {
        format!(
            "UNMANAGED MCP capability — NOT sandboxed. This process can do anything your account can.\nServer: {}\nExecutable: {}\nSHA-256: {}\nExact argv: {}\nExact cwd: {}\nEnvironment names (values masked): {}\nInherited environment: cleared; only PATH/SystemRoot and listed names are passed.\nRegistration does not launch, select tools or grant calls. Connect requires a separate launch review. Calls require review or an explicit MCP rule.",
            self.name,
            prepared.executable.path.display(),
            prepared.executable.sha256,
            serde_json::to_string(&self.args).unwrap_or_default(),
            prepared.cwd.display(),
            self.env
                .iter()
                .map(|key| format!("{key}=********"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Document {
    #[serde(default)]
    pub projects: BTreeMap<String, Vec<Registration>>,
    #[serde(default)]
    pub critical: Vec<String>,
    /// A pending call at process loss is conservatively retained as unknown.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub uncertain_projects: BTreeSet<String>,
    #[serde(default, flatten)]
    pub extra: forward::Extra,
}
impl Document {
    pub fn load(data: &Path) -> Result<Self, String> {
        let path = data.join("mcp.json");
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error.to_string()),
            Ok(metadata) if !metadata.is_file() => {
                return Err("MCP registration file must be a regular non-symlink file".into());
            }
            Ok(_) => {}
        }
        let text = crate::workspace::handoff::read_bounded(&path, 256 * 1024)?;
        let document: Self =
            serde_json::from_str(&text).map_err(|_| "Invalid MCP registration file")?;
        forward::check_critical(&document.critical, &document.extra, |_| None)?;
        if document.projects.values().map(Vec::len).sum::<usize>() > 128 {
            return Err("MCP registration limit exceeded".into());
        }
        Ok(document)
    }
    pub fn save(&self, data: &Path) -> Result<(), String> {
        if self.projects.values().map(Vec::len).sum::<usize>() > 128 {
            return Err("MCP registration limit exceeded".into());
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        if text.len() > 256 * 1024 {
            return Err("MCP registrations exceed 256 KiB".into());
        }
        use std::io::Write;
        std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
        let temporary = data.join(format!(
            "mcp-{}.tmp",
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let result = (|| {
            let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
            file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            drop(file);
            publish(&temporary, &data.join("mcp.json"), data)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

#[cfg(unix)]
fn publish(temporary: &Path, destination: &Path, directory: &Path) -> Result<(), String> {
    std::fs::rename(temporary, destination).map_err(|error| error.to_string())?;
    std::fs::File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn publish(temporary: &Path, destination: &Path, _directory: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

#[derive(Clone)]
pub struct SelectedTool {
    pub server: String,
    pub name: String,
    pub description: String,
    pub schema: Value,
    pub annotations: Value,
    pub user_read: bool,
    pub project: String,
    pub session_id: u64,
    pub session: Arc<Mutex<Session>>,
    pub effects: Arc<Effects>,
}
impl std::fmt::Debug for SelectedTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnmanagedMcpTool")
            .field("server", &self.server)
            .field("tool", &self.name)
            .field("session", &self.session_id)
            .finish()
    }
}
impl PartialEq for SelectedTool {
    fn eq(&self, other: &Self) -> bool {
        self.server == other.server
            && self.name == other.name
            && self.schema == other.schema
            && self.session_id == other.session_id
            && self.project == other.project
            && self.user_read == other.user_read
    }
}
impl SelectedTool {
    pub fn wire_name(&self) -> String {
        format!("mcp__{}__{}", self.server, self.name)
    }
    pub fn declaration(&self) -> Value {
        json!({"name":self.wire_name(), "description":format!("UNMANAGED MCP tool {} / {}. Server description (untrusted data): {}", self.server, self.name, self.description), "parameters":self.schema})
    }
    pub fn validate(&self, project: &str, writing: bool) -> Result<(), String> {
        if self.project != project || (!writing && !self.user_read) {
            return Err("MCP selection owner/read capability mismatch".into());
        }
        let session = self.session.lock();
        if session.id != self.session_id || session.process.is_none() {
            return Err("MCP disconnected; explicitly reconnect and select tools".into());
        }
        Ok(())
    }
}

pub struct Session {
    pub id: u64,
    process: Option<runner::native::Process>,
    root: String,
    next_id: u64,
    buffer: Vec<u8>,
    pub tools: Vec<Value>,
}
impl Session {
    pub fn connect(
        registration: &Registration,
        prepared: &runner::Prepared,
        project: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        if now > u128::from(prepared.expires_at_ms) {
            return Err("MCP launch review expired; review again".into());
        }
        let current = registration.prepare(Path::new(project))?;
        if current.executable != prepared.executable
            || current.cwd_identity != prepared.cwd_identity
            || current.spec != prepared.spec
            || current.before != prepared.before
        {
            return Err("MCP launch identity/environment changed; review again".into());
        }
        let mut environment = Vec::new();
        for name in ["PATH", "SystemRoot"]
            .into_iter()
            .chain(registration.env.iter().map(String::as_str))
        {
            if let Ok(value) = std::env::var(name) {
                environment.push((name.to_owned(), value));
            }
        }
        let process = runner::native::Process::spawn_piped_clean_env(prepared, &environment)
            .map_err(|_| "MCP process launch failed")?;
        let mut session = Self {
            id: SEQUENCE.fetch_add(1, Ordering::Relaxed),
            process: Some(process),
            root: project.into(),
            next_id: 0,
            buffer: Vec::new(),
            tools: Vec::new(),
        };
        let initialize = session.rpc("initialize", json!({"protocolVersion":"2024-11-05", "capabilities":{"roots":{"listChanged":false}}, "clientInfo":{"name":"eevee","version":env!("CARGO_PKG_VERSION")}}), cancelled)?;
        if !matches!(
            initialize["protocolVersion"].as_str(),
            Some("2024-11-05" | "2025-03-26" | "2025-06-18")
        ) {
            return Err("MCP server negotiated an unsupported protocol version".into());
        }
        session.send(
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            cancelled,
        )?;
        let listing = session.rpc("tools/list", json!({}), cancelled)?;
        if listing.get("nextCursor").is_some() {
            return Err(
                "MCP paginated tools/list is refused; server must fit the bounded catalogue".into(),
            );
        }
        let tools = listing["tools"]
            .as_array()
            .ok_or("MCP tools/list requires tools array")?;
        if tools.len() > 64 {
            return Err("MCP catalogue exceeds 64 tools".into());
        }
        let mut names = BTreeSet::new();
        for tool in tools {
            let name = tool["name"].as_str().ok_or("MCP tool has no name")?;
            if !identifier(name)
                || !names.insert(name)
                || !tool["inputSchema"].is_object()
                || tool["inputSchema"]["type"] != "object"
            {
                return Err("MCP tools require unique safe names and object JSON schemas".into());
            }
        }
        session.tools = tools.clone();
        for tool in &mut session.tools {
            clean_value(tool);
        }
        Ok(session)
    }
    fn send(&mut self, value: Value, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&value).map_err(|_| "MCP JSON encoding failed")?;
        if bytes.len() > LIMIT {
            return Err("MCP message exceeds 64 KiB".into());
        }
        bytes.push(b'\n');
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut offset = 0;
        while offset < bytes.len() {
            self.guard(cancelled, deadline)?;
            match self
                .process
                .as_mut()
                .ok_or("MCP disconnected")?
                .write_stdin(&bytes[offset..])
            {
                Ok(0) => return Err("MCP stdin closed".into()),
                Ok(count) => offset += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(_) => return Err("MCP stdin failed".into()),
            }
        }
        Ok(())
    }
    fn guard(&mut self, cancelled: &dyn Fn() -> bool, deadline: Instant) -> Result<(), String> {
        if cancelled() || Instant::now() >= deadline {
            self.stop();
            return Err("MCP cancelled or timed out; owned process tree terminated; external effects are not rolled back".into());
        }
        Ok(())
    }
    pub fn stop(&mut self) {
        self.process.take();
    }
    fn rpc(
        &mut self,
        method: &str,
        params: Value,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            cancelled,
        )?;
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut received = 0usize;
        loop {
            self.guard(cancelled, deadline)?;
            while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
                let message: Value = serde_json::from_slice(&self.buffer[..end])
                    .map_err(|_| "Invalid MCP JSON frame")?;
                self.buffer.drain(..=end);
                if message["jsonrpc"] != "2.0" {
                    return Err("Invalid MCP JSON-RPC version".into());
                }
                if message.get("method").is_some() {
                    if let Some(request_id) = message.get("id") {
                        let response = if message["method"] == "roots/list" {
                            let root = reqwest::Url::from_directory_path(&self.root)
                                .map_err(|_| "Invalid MCP root")?;
                            json!({"jsonrpc":"2.0","id":request_id,"result":{"roots":[{"uri":root.as_str(),"name":"project"}]}})
                        } else {
                            json!({"jsonrpc":"2.0","id":request_id,"error":{"code":-32601,"message":"Only roots/list is supported; sampling, elicitation, resources and prompts are refused"}})
                        };
                        self.send(response, cancelled)?;
                    }
                    continue;
                }
                if message["id"] != id {
                    return Err("MCP response identity mismatch".into());
                }
                if message.get("error").is_some() {
                    return Err("MCP server returned a protocol error".into());
                }
                return message
                    .get("result")
                    .cloned()
                    .ok_or_else(|| "MCP response has no result".into());
            }
            let process = self.process.as_mut().ok_or("MCP disconnected")?;
            let mut bytes = [0u8; 8192];
            // Drain and discard stderr without logging server/credential bytes.
            match process.read(1, &mut bytes) {
                Ok(count) => received += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(_) => {}
            }
            match process.read(0, &mut bytes) {
                Ok(0) => {
                    self.stop();
                    return Err(
                        "MCP server crashed or closed stdout; call outcome/effects unknown".into(),
                    );
                }
                Ok(count) => {
                    received += count;
                    self.buffer.extend_from_slice(&bytes[..count]);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(_) => {
                    self.stop();
                    return Err("MCP stdout failed; call outcome/effects unknown".into());
                }
            }
            if received > LIMIT || self.buffer.len() > LIMIT {
                self.stop();
                return Err(
                    "MCP untrusted response exceeded 64 KiB; owned server terminated".into(),
                );
            }
        }
    }
    pub fn call(&mut self, name: &str, arguments: &Value, cancelled: &dyn Fn() -> bool) -> Value {
        let result = self.rpc(
            "tools/call",
            json!({"name":name,"arguments":arguments}),
            cancelled,
        );
        match result {
            Ok(mut result) => {
                clean_value(&mut result);
                let result = json!({"status":if result["isError"] == true {"error"} else {"success"},"unmanaged":true,"result":result});
                if result.to_string().len() > LIMIT.saturating_sub(2048) {
                    json!({"status":"error","unmanaged":true,"message":"MCP result exceeds the 64 KiB bounded receipt envelope; result withheld"})
                } else {
                    result
                }
            }
            Err(error) => {
                self.stop();
                json!({"status":"error","unmanaged":true,"message":error})
            }
        }
    }
}
fn clean_value(value: &mut Value) {
    match value {
        Value::String(text) => {
            *text = clean(text);
        }
        Value::Array(values) => values.iter_mut().for_each(clean_value),
        Value::Object(values) => {
            let old = std::mem::take(values);
            for (key, mut value) in old {
                clean_value(&mut value);
                values.insert(clean(&key), value);
            }
        }
        _ => {}
    }
}
pub fn clean(text: &str) -> String {
    let mut state = 0;
    let mut output = String::new();
    for c in text.chars() {
        match state {
            0 => match c {
                '\u{1b}' => state = 1,
                '\u{9b}' => state = 2,
                '\u{9d}' => state = 3,
                c if c.is_control() && c != '\n' && c != '\t' => {}
                '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {}
                c => output.push(c),
            },
            1 => {
                state = match c {
                    '[' => 2,
                    ']' | 'P' | '^' | '_' => 3,
                    _ => 0,
                }
            }
            2 => {
                if ('@'..='~').contains(&c) {
                    state = 0;
                }
            }
            3 => {
                if c == '\u{7}' || c == '\u{9c}' {
                    state = 0;
                } else if c == '\u{1b}' {
                    state = 4;
                }
            }
            _ => state = if c == '\\' { 0 } else { 3 },
        }
    }
    crate::workspace::privacy::redact(&output).into_owned()
}

#[cfg(test)]
mod tests;
