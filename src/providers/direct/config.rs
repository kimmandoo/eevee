//! Native connection configuration. Credential material exists only in resolved connections.
use std::env;
#[cfg(not(windows))]
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::providers::{Agent, agent};

#[cfg(windows)]
mod windows;

const CODEX_BASE: &str = "https://chatgpt.com/backend-api/codex";
const FILE_LIMIT: u64 = 128 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Protocol {
    Codex,
    Responses,
    Messages,
    Gemini,
    ChatCompletions,
}

pub(super) struct Connection {
    pub protocol: Protocol,
    pub base_url: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub description: String,
    provider: Agent,
    secret: Option<String>,
    account_id: Option<String>,
    residency: Option<String>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Connection { credentials and configuration redacted }")
    }
}

impl Connection {
    /// Auth is attached only to this connection's exact API endpoints. The caller must
    /// supply a client with redirects disabled; no alternate origin receives a token.
    pub fn request(
        &self,
        agent: &reqwest::Client,
        method: &str,
        url: &str,
    ) -> reqwest::RequestBuilder {
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .expect("native transport supplies a valid HTTP method");
        let request = agent.request(method, url);
        if !self.scopes(url) {
            return request;
        }
        let mut request = request
            .header("User-Agent", concat!("eevee/", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/json");
        if let Some(secret) = &self.secret {
            request = match self.protocol {
                Protocol::Messages => request.header("x-api-key", private_header(secret)),
                Protocol::Gemini => request.header("x-goog-api-key", private_header(secret)),
                _ => request.bearer_auth(secret),
            };
        }
        if self.protocol == Protocol::Messages {
            request = request.header("anthropic-version", "2023-06-01");
        }
        if self.protocol == Protocol::Codex {
            request = request
                .header("originator", "eevee")
                .header("version", "0.162.1")
                .header("OpenAI-Beta", "responses=experimental");
            if let Some(account) = &self.account_id {
                request = request.header("ChatGPT-Account-Id", private_header(account));
            }
            if let Some(residency) = &self.residency {
                request = request.header("x-openai-internal-codex-residency", residency);
            }
        }
        request
    }

    pub fn fingerprint(&self) -> String {
        let mut digest = Sha256::new();
        digest.update([self.provider as u8, self.protocol as u8]);
        for part in [
            self.base_url.as_str(),
            self.model.as_deref().unwrap_or_default(),
            self.effort.as_deref().unwrap_or_default(),
            self.secret.as_deref().unwrap_or_default(),
            self.account_id.as_deref().unwrap_or_default(),
            self.residency.as_deref().unwrap_or_default(),
            self.description.as_str(),
        ] {
            digest.update((part.len() as u64).to_le_bytes());
            digest.update(part.as_bytes());
        }
        format!("{:x}", digest.finalize())
    }

    pub fn revalidate(&self) -> Result<(), String> {
        if resolve(self.provider)?.fingerprint() == self.fingerprint() {
            Ok(())
        } else {
            Err("Connection settings or credential identity changed; review a new request.".into())
        }
    }

    pub fn models_url(&self) -> String {
        if self.protocol == Protocol::Codex {
            format!("{}/models?client_version=0.162.1", self.base_url)
        } else {
            format!("{}/models", self.base_url)
        }
    }

    pub fn inference_url(&self, model: &str) -> String {
        let endpoint = match self.protocol {
            Protocol::Codex | Protocol::Responses => "responses".into(),
            Protocol::Messages => "messages".into(),
            Protocol::ChatCompletions => "chat/completions".into(),
            Protocol::Gemini => format!(
                "models/{}:streamGenerateContent?alt=sse",
                path_segment(model.strip_prefix("models/").unwrap_or(model))
            ),
        };
        format!("{}/{endpoint}", self.base_url)
    }

    fn scopes(&self, url: &str) -> bool {
        if url == self.models_url() {
            return true;
        }
        let models = reqwest::Url::parse(&self.models_url());
        let candidate = reqwest::Url::parse(url);
        if let (Ok(models), Ok(candidate)) = (models, candidate)
            && candidate.origin() == models.origin()
            && candidate.path() == models.path()
            && candidate.fragment().is_none()
            && candidate.username().is_empty()
            && candidate.password().is_none()
        {
            let allowed: &[&str] = match self.protocol {
                Protocol::Codex => &["client_version", "cursor"],
                Protocol::Messages => &["after_id", "limit"],
                Protocol::Gemini => &["pageToken", "pageSize"],
                Protocol::Responses | Protocol::ChatCompletions => &["after", "limit", "cursor"],
            };
            return candidate
                .query_pairs()
                .all(|(key, _)| allowed.contains(&key.as_ref()));
        }
        if self.protocol != Protocol::Gemini {
            return url == self.inference_url("");
        }
        let prefix = format!("{}/models/", self.base_url);
        let Some(model) = url
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix(":streamGenerateContent?alt=sse"))
        else {
            return false;
        };
        !model.is_empty()
            && model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.%".contains(&b))
    }
}

fn path_segment(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.".contains(&byte) {
            result.push(char::from(byte));
        } else {
            use std::fmt::Write;
            let _ = write!(result, "%{byte:02X}");
        }
    }
    result
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderFile {
    openai: Option<Entry>,
    anthropic: Option<Entry>,
    google: Option<Entry>,
    compatible: Option<Entry>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    base_url: Option<String>,
    model: Option<String>,
    api_key_env: Option<String>,
    keyless: Option<bool>,
    protocol: Option<String>,
}

struct Inputs<F> {
    get: F,
    home: Option<PathBuf>,
    config_home: Option<PathBuf>,
    project: Option<PathBuf>,
    now: u64,
}

fn inputs() -> Inputs<impl Fn(&str) -> Option<String>> {
    let home = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let config_home = if cfg!(windows) {
        env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home.as_ref().map(|p| p.join("Library/Application Support"))
    } else {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|p| p.join(".config")))
    };
    Inputs {
        get: |key: &str| env::var(key).ok(),
        home,
        config_home,
        project: env::current_dir().ok().and_then(|p| p.canonicalize().ok()),
        now: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    }
}

struct Settings {
    protocol: Protocol,
    base_url: String,
    model: Option<String>,
    effort: Option<String>,
    key_env: Option<String>,
    keyless: bool,
}

fn selected_entry(file: &ProviderFile, provider: Agent) -> Option<&Entry> {
    match provider {
        Agent::OpenAI => file.openai.as_ref(),
        Agent::Anthropic => file.anthropic.as_ref(),
        Agent::Google => file.google.as_ref(),
        Agent::Compatible => file.compatible.as_ref(),
        _ => None,
    }
}

fn parse_file(bytes: &[u8]) -> Result<ProviderFile, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "Provider config must be UTF-8 TOML.".to_string())?;
    toml::from_str(text).map_err(|_| "Invalid provider config: only provider tables with base_url, model, api_key_env, keyless and protocol are accepted; no literal keys, custom headers or commands.".into())
}

fn load_file<F: Fn(&str) -> Option<String>>(input: &Inputs<F>) -> Result<ProviderFile, String> {
    let explicit = (input.get)("EV_PROVIDERS_CONFIG");
    let path = explicit.as_ref().map(PathBuf::from).or_else(|| {
        input
            .config_home
            .as_ref()
            .map(|p| p.join("eevee/providers.toml"))
    });
    let Some(path) = path else {
        return Ok(ProviderFile::default());
    };
    if explicit.is_none() && !path.exists() {
        return Ok(ProviderFile::default());
    }
    let bytes = read_user_file(&path, false, input.project.as_deref())?;
    parse_file(&bytes)
}

fn settings<F: Fn(&str) -> Option<String>>(
    provider: Agent,
    input: &Inputs<F>,
) -> Result<Settings, String> {
    if provider == Agent::ChatGPT {
        let (model, effort) = codex_preferences(input)?;
        return Ok(Settings {
            protocol: Protocol::Codex,
            base_url: CODEX_BASE.into(),
            model,
            effort,
            key_env: None,
            keyless: false,
        });
    }
    let file = load_file(input)?;
    let entry = selected_entry(&file, provider);
    let (prefix, default_base, default_key, protocol) = match provider {
        Agent::OpenAI => (
            "OPENAI",
            "https://api.openai.com/v1",
            "OPENAI_API_KEY",
            Protocol::Responses,
        ),
        Agent::Anthropic => (
            "ANTHROPIC",
            "https://api.anthropic.com/v1",
            "ANTHROPIC_API_KEY",
            Protocol::Messages,
        ),
        Agent::Google => (
            "GEMINI",
            "https://generativelanguage.googleapis.com/v1beta",
            "GEMINI_API_KEY",
            Protocol::Gemini,
        ),
        Agent::Compatible => ("EV_API", "", "EV_API_KEY", Protocol::ChatCompletions),
        _ => return Err("This provider does not use native HTTP connections.".into()),
    };
    let get = &input.get;
    let base_url = get(&format!("{prefix}_BASE_URL"))
        .or_else(|| entry.and_then(|e| e.base_url.clone()))
        .unwrap_or_else(|| default_base.into());
    let base_url = validate_base(&base_url)?;
    let model = get(&format!("{prefix}_MODEL")).or_else(|| entry.and_then(|e| e.model.clone()));
    if model.as_ref().is_some_and(|m| m.is_empty() || m.len() > 256 || crate::workspace::privacy::redact(m) != *m || m.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))) {
        return Err("Configured model must be a nonempty, bounded identifier without control characters.".into());
    }
    let key_env = entry
        .and_then(|e| e.api_key_env.clone())
        .unwrap_or_else(|| default_key.into());
    if !valid_env_name(&key_env) {
        return Err(
            "api_key_env must name an environment variable, not contain a key value.".into(),
        );
    }
    let keyless = if provider == Agent::Compatible {
        match get("EV_API_KEYLESS") {
            Some(value) if value == "1" => true,
            Some(value) if value == "0" => false,
            Some(_) => return Err("EV_API_KEYLESS must be 1 or 0.".into()),
            None => entry.and_then(|e| e.keyless).unwrap_or(false),
        }
    } else {
        if entry.and_then(|e| e.keyless) == Some(true) {
            return Err("Keyless mode is available only for the Compatible connection.".into());
        }
        false
    };
    let selected_protocol = if provider == Agent::Compatible {
        get("EV_API_PROTOCOL").or_else(|| entry.and_then(|e| e.protocol.clone()))
    } else {
        entry.and_then(|e| e.protocol.clone())
    };
    let protocol =
        match (provider, selected_protocol.as_deref()) {
            (Agent::Compatible, Some("responses")) => Protocol::Responses,
            (Agent::Compatible, Some("chat_completions" | "chat-completions")) => {
                Protocol::ChatCompletions
            }
            (_, None) => protocol,
            (Agent::OpenAI, Some("responses")) => protocol,
            (Agent::Anthropic, Some("messages")) => protocol,
            (Agent::Google, Some("gemini")) => protocol,
            _ => return Err(
                "Unsupported provider protocol; Compatible supports chat_completions or responses."
                    .into(),
            ),
        };
    Ok(Settings {
        protocol,
        base_url,
        model,
        effort: None,
        key_env: Some(key_env),
        keyless,
    })
}

fn private_header(value: &str) -> reqwest::header::HeaderValue {
    let mut value = reqwest::header::HeaderValue::from_bytes(value.as_bytes())
        .expect("resolved credentials are validated HTTP header values");
    value.set_sensitive(true);
    value
}

fn codex_preferences<F: Fn(&str) -> Option<String>>(
    input: &Inputs<F>,
) -> Result<(Option<String>, Option<String>), String> {
    let path = codex_auth_path(input).map(|p| p.with_file_name("config.toml"));
    let preferences = if let Some(path) = path.filter(|p| p.exists()) {
        let bytes = read_user_file(&path, false, input.project.as_deref())?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| "Provider config.toml must be UTF-8 TOML.".to_string())?;
        let value: toml::Value = toml::from_str(text).map_err(|_| {
            "Provider config.toml is invalid TOML; no raw provider configuration is displayed."
                .to_string()
        })?;
        let nonsecret = |name: &str| -> Result<Option<String>, String> {
            match value.get(name) {
                None => Ok(None),
                Some(value) => value.as_str().map(|s| Some(s.to_owned())).ok_or_else(|| {
                    "Provider model and reasoning-effort preferences must be strings.".into()
                }),
            }
        };
        (nonsecret("model")?, nonsecret("model_reasoning_effort")?)
    } else {
        (None, None)
    };
    let model = (input.get)("CODEX_MODEL").or(preferences.0);
    let effort = (input.get)("CODEX_REASONING_EFFORT").or(preferences.1);
    for value in [&model, &effort].into_iter().flatten() {
        if value.is_empty() || value.len() > 256 || crate::workspace::privacy::redact(value) != *value || value.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
            return Err("Provider model and reasoning-effort preferences must be bounded nonempty strings without control characters.".into());
        }
    }
    Ok((model, effort))
}

fn valid_env_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// A deliberately narrow URL grammar avoids parser disagreement, encoded authority
/// tricks, network-path references, and credentials hidden in a query or userinfo.
fn validate_base(value: &str) -> Result<String, String> {
    let error = || {
        "API base URL must be HTTPS, or HTTP on a numeric loopback/localhost address, with no credentials, query, fragment, escapes or traversal.".to_string()
    };
    let (scheme, rest) = value.split_once("://").ok_or_else(error)?;
    if !matches!(scheme, "https" | "http")
        || !value.is_ascii()
        || value
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
        || value.contains(['@', '?', '#', '\\', '%'])
    {
        return Err(error());
    }
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = if authority.starts_with('[') {
        let end = authority.find(']').ok_or_else(error)?;
        let host = &authority[..=end];
        let remainder = &authority[end + 1..];
        let port = if remainder.is_empty() {
            None
        } else {
            Some(remainder.strip_prefix(':').ok_or_else(error)?)
        };
        if host[1..host.len() - 1]
            .parse::<std::net::Ipv6Addr>()
            .is_err()
        {
            return Err(error());
        }
        (host, port)
    } else {
        let (host, port) = authority
            .split_once(':')
            .map_or((authority, None), |(h, p)| (h, Some(p)));
        if host.is_empty()
            || host.starts_with('.')
            || host.ends_with('.')
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return Err(error());
        }
        (host, port)
    };
    if let Some(port) = port
        && (port.is_empty()
            || !port.bytes().all(|b| b.is_ascii_digit())
            || port.parse::<u16>().ok().is_none_or(|n| n == 0))
    {
        return Err(error());
    }
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .and_then(|h| h.parse::<std::net::Ipv6Addr>().ok())
            .is_some_and(|ip| ip.is_loopback())
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback());
    if scheme == "http" && !loopback {
        return Err(error());
    }
    if path.split('/').any(|part| part == "." || part == "..")
        || !path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-_.~:".contains(&b))
    {
        return Err(error());
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn read_user_file(
    path: &Path,
    credential: bool,
    project: Option<&Path>,
) -> Result<Vec<u8>, String> {
    let error = || {
        if credential {
            "Cannot read provider-owned auth.json safely. Use CODEX_ACCESS_TOKEN (and optionally CODEX_ACCOUNT_ID), or a regular, owner-private CODEX_HOME/auth.json; keychain-only login is not readable by this transport.".to_string()
        } else {
            "Cannot read user provider config safely: use an absolute, non-project, user-owned regular file without group/other write permission.".to_string()
        }
    };
    if !path.is_absolute() {
        return Err(error());
    }
    let canonical = path.canonicalize().map_err(|_| error())?;
    if project.is_some_and(|root| canonical.starts_with(root)) {
        return Err(error());
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| error())?;
    if !metadata.is_file() || metadata.len() > FILE_LIMIT {
        return Err(error());
    }
    let mut file: File = {
        #[cfg(windows)]
        {
            windows::open_user_file(path, &canonical, credential).map_err(|_| error())?
        }
        #[cfg(not(windows))]
        {
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
                if metadata.uid() != unsafe { libc::geteuid() }
                    || metadata.permissions().mode() & (if credential { 0o077 } else { 0o022 }) != 0
                {
                    return Err(error());
                }
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            options.open(path).map_err(|_| error())?
        }
    };
    let opened = file.metadata().map_err(|_| error())?;
    if !opened.is_file() || opened.len() > FILE_LIMIT {
        return Err(error());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.dev() != opened.dev()
            || metadata.ino() != opened.ino()
            || metadata.uid() != opened.uid()
            || metadata.mode() != opened.mode()
        {
            return Err(error());
        }
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.by_ref()
        .take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error())?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(error());
    }
    Ok(bytes)
}

fn codex_auth_path<F: Fn(&str) -> Option<String>>(input: &Inputs<F>) -> Option<PathBuf> {
    (input.get)("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| input.home.as_ref().map(|p| p.join(".codex")))
        .map(|p| p.join("auth.json"))
}

fn valid_secret(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 * 1024 && value.bytes().all(|b| b.is_ascii_graphic())
}

struct Claims {
    account_id: Option<String>,
    residency: Option<String>,
}

fn token_claims(token: &str, now: u64) -> Result<Claims, String> {
    let mut segments = token.split('.');
    let _ = segments.next();
    let payload = segments.next();
    // Opaque credentials remain usable; decoded claims are hints, not verification.
    let Some(payload) = payload.filter(|_| segments.next().is_some()) else {
        return Ok(Claims {
            account_id: None,
            residency: None,
        });
    };
    let bytes = base64_simd::URL_SAFE_NO_PAD
        .decode_to_vec(payload.as_bytes())
        .map_err(|_| {
            "ChatGPT access token has malformed JWT claims; supply a current access token."
                .to_string()
        })?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| {
        "ChatGPT access token has malformed JWT claims; supply a current access token.".to_string()
    })?;
    if let Some(expiry) = value.get("exp") {
        let expiry = expiry.as_u64().ok_or_else(|| "ChatGPT access token has invalid expiry metadata; supply a current provider access token.".to_string())?;
        if expiry <= now {
            return Err("ChatGPT access token expired. Renew it with the provider outside Eevee, then review again; Eevee never refreshes or writes credentials.".into());
        }
    }
    let auth = value.get("https://api.openai.com/auth");
    let claim = |name| {
        auth.and_then(|a| a.get(name))
            .or_else(|| value.get(name))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    Ok(Claims {
        account_id: claim("chatgpt_account_id"),
        residency: claim("chatgpt_data_residency").or_else(|| claim("chatgpt_compute_residency")),
    })
}

fn resolve_with<F: Fn(&str) -> Option<String>>(
    provider: Agent,
    input: &Inputs<F>,
) -> Result<Connection, String> {
    let settings = settings(provider, input)?;
    let mut account_id = None;
    let mut residency = None;
    let (secret, source) = if provider == Agent::ChatGPT {
        let (token, file_account, source) = if let Some(token) = (input.get)("CODEX_ACCESS_TOKEN") {
            (
                token,
                None,
                "explicit CODEX_ACCESS_TOKEN environment credential".to_string(),
            )
        } else {
            let path = codex_auth_path(input).ok_or_else(|| guidance(provider))?;
            let bytes = read_user_file(&path, true, input.project.as_deref())?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|_| "Provider-owned auth.json is not valid JSON; supply CODEX_ACCESS_TOKEN or renew provider login outside Eevee.".to_string())?;
            let tokens = value.get("tokens");
            let token = tokens.and_then(|t| t.get("access_token")).and_then(serde_json::Value::as_str)
                .ok_or_else(|| "Provider-owned auth.json has no access token. Keychain-only or API-key-only login is not ChatGPT authentication; supply CODEX_ACCESS_TOKEN or renew the provider login outside Eevee.".to_string())?;
            let account = tokens
                .and_then(|t| t.get("account_id"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            (
                token.to_owned(),
                account,
                format!(
                    "explicit read-only provider-owned credential file {}",
                    path.display()
                ),
            )
        };
        if !valid_secret(&token) {
            return Err(
                "ChatGPT access token is empty or unsuitable for an HTTP authorization header."
                    .into(),
            );
        }
        let claims = token_claims(&token, input.now)?;
        account_id = (input.get)("CODEX_ACCOUNT_ID")
            .or(file_account)
            .or(claims.account_id);
        residency = claims.residency;
        if account_id
            .as_ref()
            .is_some_and(|value| !valid_secret(value))
            || residency.as_ref().is_some_and(|value| !valid_secret(value))
        {
            return Err("ChatGPT account metadata is invalid; provide a current provider-owned access token and account identity.".into());
        }
        (Some(token), source)
    } else if settings.keyless {
        (
            None,
            "explicit keyless connection; no authentication header".into(),
        )
    } else {
        let mut key_name = settings.key_env.as_deref().unwrap_or_default();
        let mut key = (input.get)(key_name);
        if key.is_none() && provider == Agent::Google && key_name == "GEMINI_API_KEY" {
            key_name = "GOOGLE_API_KEY";
            key = (input.get)(key_name);
        }
        if provider == Agent::Anthropic
            && key
                .as_deref()
                .is_some_and(|key| key.starts_with("sk-ant-oat"))
        {
            return Err("Anthropic subscription OAuth tokens are not accepted. Supply an Anthropic API key, not a Claude subscription/session credential.".into());
        }
        match key {
            Some(key) if valid_secret(&key) => (Some(key), format!("explicit environment API key from {key_name} (not authenticated)")),
            Some(_) => return Err("API key is empty or unsuitable for an HTTP header; update the selected environment variable.".into()),
            None => return Err(guidance(provider)),
        }
    };
    let description = format!(
        "{}: {:?} at {}; {}; no host CLI required. Only reviewed input is sent; no provider tools, project hooks, hidden source, automatic edits, credential refresh or persistence. Account access and billing are not authenticated by configuration.",
        provider.label(),
        settings.protocol,
        settings.base_url,
        source
    );
    Ok(Connection {
        protocol: settings.protocol,
        base_url: settings.base_url,
        model: settings.model,
        effort: settings.effort,
        description,
        provider,
        secret,
        account_id,
        residency,
    })
}

pub(super) fn resolve(provider: Agent) -> Result<Connection, String> {
    resolve_with(provider, &inputs())
}

#[cfg(test)]
pub(super) fn test_connection(
    protocol: Protocol,
    base_url: &str,
    model: Option<&str>,
) -> Connection {
    Connection {
        protocol,
        base_url: validate_base(base_url).expect("test endpoint must be a safe URL"),
        model: model.map(str::to_owned),
        effort: None,
        description: "Explicit keyless test endpoint".into(),
        provider: Agent::Compatible,
        secret: None,
        account_id: None,
        residency: None,
    }
}

/// Check only existence, without copying or decoding credential bytes.
fn environment_exists(name: &str) -> bool {
    #[cfg(unix)]
    {
        let Ok(name) = std::ffi::CString::new(name) else {
            return false;
        };
        // No dereference: the credential value is intentionally never read here.
        unsafe { !libc::getenv(name.as_ptr()).is_null() }
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetEnvironmentVariableW(name: *const u16, buffer: *mut u16, size: u32) -> u32;
        }
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // A zero-sized buffer queries presence/size without reading the value.
        unsafe { GetEnvironmentVariableW(name.as_ptr(), std::ptr::null_mut(), 0) != 0 }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = name;
        false
    }
}

fn preferred_with(choice: Option<&str>, configured: impl Fn(Agent) -> bool) -> Agent {
    if let Some(choice) = choice {
        let explicit = match choice.trim().to_ascii_lowercase().as_str() {
            "chatgpt" => Some(Agent::ChatGPT),
            "openai" => Some(Agent::OpenAI),
            "anthropic" => Some(Agent::Anthropic),
            "google" => Some(Agent::Google),
            "compatible" => Some(Agent::Compatible),
            "codex" => Some(Agent::Codex),
            "claude" => Some(Agent::Claude),
            "opencode" => Some(Agent::OpenCode),
            "gemini" => Some(Agent::Gemini),
            "devin" => Some(Agent::Devin),
            _ => None,
        };
        if let Some(provider) = explicit {
            return provider;
        }
    }
    [
        Agent::ChatGPT,
        Agent::OpenAI,
        Agent::Anthropic,
        Agent::Google,
        Agent::Compatible,
    ]
    .into_iter()
    .find(|provider| configured(*provider))
    .unwrap_or(Agent::Codex)
}

pub(crate) fn preferred_agent() -> Agent {
    preferred_with(env::var("EV_PROVIDER").ok().as_deref(), configured)
}

/// Passive source availability is not authentication. No credential file is read.
pub(crate) fn configured(provider: Agent) -> bool {
    let input = inputs();
    let Ok(settings) = settings(provider, &input) else {
        return false;
    };
    if provider == Agent::ChatGPT {
        return environment_exists("CODEX_ACCESS_TOKEN")
            || codex_auth_path(&input).is_some_and(|p| p.is_file());
    }
    settings.keyless
        || settings.key_env.as_deref().is_some_and(environment_exists)
        || provider == Agent::Google
            && settings.key_env.as_deref() == Some("GEMINI_API_KEY")
            && environment_exists("GOOGLE_API_KEY")
}

pub(crate) fn guidance(provider: Agent) -> String {
    let guidance = match provider {
        Agent::ChatGPT => {
            "ChatGPT direct requires explicit CODEX_ACCESS_TOKEN (+ optional CODEX_ACCOUNT_ID), or your reviewed read-only CODEX_HOME/auth.json (default ~/.codex/auth.json) tokens.access_token. The token is sent only to the official ChatGPT Codex backend; no host CLI is needed. Nonsecret model/model_reasoning_effort defaults come from CODEX_HOME/config.toml; CODEX_MODEL/CODEX_REASONING_EFFORT override them. Keychain-only, missing or expired credentials require supplying/renewing provider credentials outside Eevee. No refresh, login, import or credential write is performed."
        }
        Agent::OpenAI => {
            "Set OPENAI_API_KEY; optional OPENAI_BASE_URL and OPENAI_MODEL. User providers.toml may select api_key_env. This is metered API access, not ChatGPT subscription authentication."
        }
        Agent::Anthropic => {
            "Set ANTHROPIC_API_KEY; optional ANTHROPIC_BASE_URL and ANTHROPIC_MODEL. Only an Anthropic API key is accepted; Claude subscription/session tokens are not API authentication."
        }
        Agent::Google => {
            "Set GEMINI_API_KEY (or GOOGLE_API_KEY); optional GEMINI_BASE_URL and GEMINI_MODEL. This is Gemini API access, not a host CLI subscription/login."
        }
        Agent::Compatible => {
            "Set EV_API_BASE_URL and optional EV_API_MODEL. Supply EV_API_KEY, or explicitly enable EV_API_KEYLESS=1 for a trusted keyless endpoint. EV_API_PROTOCOL is chat_completions (default) or responses. HTTPS is required except HTTP loopback. Endpoint trust, account access and billing remain yours."
        }
        _ => "Select a native connection provider.",
    };
    format!(
        "{guidance} Credential bytes are read only when an approved metadata/request launch resolves the connection; passive configuration is not authentication."
    )
}

pub(crate) fn profile(provider: Agent) -> agent::Profile {
    let available = configured(provider);
    let input = inputs();
    let configuration = match settings(provider, &input) {
        Ok(settings) => format!(
            "{:?}; endpoint {}; model {}; effort {}; configured source {}; not authenticated",
            settings.protocol,
            settings.base_url,
            settings
                .model
                .as_deref()
                .unwrap_or("explicit choice from Models"),
            settings.effort.as_deref().unwrap_or("not explicitly set"),
            if available {
                "available"
            } else {
                "not established"
            }
        ),
        Err(error) => error,
    };
    let credential_source = if provider == Agent::ChatGPT {
        if environment_exists("CODEX_ACCESS_TOKEN") {
            "Explicit credential source: CODEX_ACCESS_TOKEN; optional CODEX_ACCOUNT_ID. No credential file is read during passive discovery; credential bytes are read only when an approved metadata/request launch resolves the connection.".into()
        } else {
            format!(
                "Explicit credential source: provider-owned {} (read-only, only when an approved metadata/request launch resolves the connection). Keychain access, refresh and credential writes are not performed.",
                codex_auth_path(&input).map_or_else(
                    || "CODEX_HOME/auth.json (home unavailable)".into(),
                    |path| path.display().to_string()
                )
            )
        }
    } else {
        "Credential source: selected API-key environment variable only; configured is not authenticated. Credential bytes are read only when an approved metadata/request launch resolves the connection.".into()
    };
    let boundary = "Native tool-free HTTP: only the immutable previewed request is sent. No project hooks/tools, hidden files, provider sessions or automatic edits. Implement returns proposals only; choose an existing controlled CLI implementation for writes.";
    agent::Profile {
        revision: 1,
        name: provider.label().into(),
        executable: None,
        argv: Vec::new(),
        installed: "Built-in native transport; no host CLI installation required".into(),
        version: format!("eevee/{}; protocol availability is not authentication", env!("CARGO_PKG_VERSION")),
        authentication: "Configured source only; not authenticated. Passive discovery reads no credential file and sends no requests.".into(),
        configuration: configuration.clone(),
        os: env::consts::OS.into(),
        architecture: env::consts::ARCH.into(),
        adapter: "Native direct HTTP; no subprocess, login or refresh".into(),
        action: "Explicit preview and Run required for inference; Models fetches metadata only after consent".into(),
        billing: "Provider account/API billing, eligibility and quota are unknown until an actual request; no billing lookup".into(),
        probe_allowed: false,
        trust_preview: vec![configuration, credential_source, guidance(provider), boundary.into(), "Review explicitly authorizes reading the selected environment API credential or ChatGPT provider-owned auth.json and sending it only to the displayed endpoint. Credentials are never logged or persisted. Cancellation stops local waiting; it cannot promise a remote request stopped or was not billed. No retries or fallback.".into()],
        blockers: if available { Vec::new() } else { vec![guidance(provider)] },
        capabilities: ["Plan", "Review", "Implement proposals", "Models metadata"].into_iter().map(|action| agent::Capability {
            action: action.into(), boundary: boundary.into(), evidence: "Native protocol adapter; requires reviewed endpoint and explicit request".into(), blocker: "Configuration is not successful authentication; provider may reject credentials, model, quota or protocol".into(),
        }).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn input(values: &[(&str, &str)]) -> Inputs<impl Fn(&str) -> Option<String> + use<>> {
        let values: HashMap<String, String> = values
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Inputs {
            get: move |key: &str| values.get(key).cloned(),
            home: None,
            config_home: None,
            project: None,
            now: 100,
        }
    }

    fn jwt(value: serde_json::Value) -> String {
        format!(
            "e30.{}.signature",
            base64_simd::URL_SAFE_NO_PAD.encode_to_string(value.to_string().as_bytes())
        )
    }

    #[test]
    fn endpoint_validation_rejects_credential_and_authority_tricks() {
        for value in [
            "http://api.example/v1",
            "https://user:secret@api.example",
            "https://api.example?key=secret",
            "https://api.example/#x",
            "https://api.example/%2e%2e",
            "https://api.example/../v1",
            "https://api.example\\@evil",
            "http://127.0.0.1.evil/v1",
            "http://2130706433/v1",
            "http://0177.0.0.1/v1",
            "https://api.example:0",
            "https://api.example:65536",
            "https://api.example/\n",
        ] {
            let error = validate_base(value).unwrap_err();
            assert!(!error.contains("secret"));
        }
        for value in [
            "https://api.example/v1",
            "http://localhost:8080/v1",
            "http://127.0.0.1:1234/v1",
            "http://127.1.2.3/v1",
            "http://[::1]:8080/v1",
        ] {
            assert!(validate_base(value).is_ok(), "{value}");
        }
        assert_eq!(
            validate_base("https://api.example/v1///").unwrap(),
            "https://api.example/v1"
        );
    }

    #[test]
    fn defaults_environment_and_config_precedence_are_deterministic() {
        let default = settings(Agent::OpenAI, &input(&[])).unwrap();
        assert_eq!(default.base_url, "https://api.openai.com/v1");
        assert_eq!(default.protocol, Protocol::Responses);
        let configured = settings(
            Agent::OpenAI,
            &input(&[
                ("OPENAI_BASE_URL", "https://example.test/api"),
                ("OPENAI_MODEL", "custom-model"),
            ]),
        )
        .unwrap();
        assert_eq!(configured.base_url, "https://example.test/api");
        assert_eq!(configured.model.as_deref(), Some("custom-model"));
        assert!(
            resolve_with(
                Agent::Compatible,
                &input(&[("EV_API_BASE_URL", "http://localhost:1234/v1")])
            )
            .is_err()
        );
        let local = resolve_with(
            Agent::Compatible,
            &input(&[
                ("EV_API_BASE_URL", "http://localhost:1234/v1"),
                ("EV_API_KEYLESS", "1"),
                ("EV_API_PROTOCOL", "responses"),
            ]),
        )
        .unwrap();
        assert_eq!(local.protocol, Protocol::Responses);
        assert!(local.secret.is_none());
        assert!(
            settings(
                Agent::Compatible,
                &input(&[
                    ("EV_API_BASE_URL", "http://localhost"),
                    ("EV_API_KEYLESS", "yes")
                ])
            )
            .is_err()
        );
        assert!(
            resolve_with(
                Agent::Anthropic,
                &input(&[("ANTHROPIC_API_KEY", "sk-ant-oat-subscription")])
            )
            .is_err()
        );
        assert!(
            resolve_with(
                Agent::Google,
                &input(&[
                    ("GEMINI_API_KEY", "preferred"),
                    ("GOOGLE_API_KEY", "fallback")
                ])
            )
            .unwrap()
            .secret
            .as_deref()
                == Some("preferred")
        );
        let passive = Inputs {
            get: |name: &str| {
                assert!(!matches!(
                    name,
                    "OPENAI_API_KEY"
                        | "ANTHROPIC_API_KEY"
                        | "GEMINI_API_KEY"
                        | "CODEX_ACCESS_TOKEN"
                ));
                None
            },
            home: None,
            config_home: None,
            project: None,
            now: 100,
        };
        assert!(settings(Agent::OpenAI, &passive).is_ok());
        assert!(settings(Agent::ChatGPT, &passive).is_ok());
    }

    fn header<'a>(request: &'a reqwest::Request, name: &str) -> Option<&'a str> {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    }

    #[test]
    fn explicit_keyless_mode_does_not_forward_an_inherited_api_key() {
        let connection = resolve_with(
            Agent::Compatible,
            &input(&[
                ("EV_API_BASE_URL", "http://127.0.0.1:8080/v1"),
                ("EV_API_KEYLESS", "1"),
                ("EV_API_KEY", "inherited-private-api-key"),
            ]),
        )
        .unwrap();
        let request = connection
            .request(&reqwest::Client::new(), "GET", &connection.models_url())
            .build()
            .unwrap();
        assert!(request.headers().get("authorization").is_none());
        assert!(connection.secret.is_none());
    }

    #[test]
    fn config_parser_accepts_only_nonsecret_schema() {
        let file = parse_file(b"[openai]\nbase_url='https://example.test/v1'\nmodel='custom'\napi_key_env='MY_API_KEY'\n[compatible]\nkeyless=true\nprotocol='responses'\n").unwrap();
        assert_eq!(file.openai.unwrap().model.as_deref(), Some("custom"));
        for text in [
            "[openai]\napi_key='secret'",
            "[openai]\nheaders={Authorization='secret'}",
            "[openai]\ncommand='run'",
            "[chatgpt]\nbase_url='https://evil.test'",
            "[unknown]\nkeyless=true",
        ] {
            let error = parse_file(text.as_bytes()).err().unwrap();
            assert!(!error.contains("'secret'"));
        }
    }

    #[test]
    fn secrets_are_exact_endpoint_scoped_and_debug_is_redacted() {
        let connection = resolve_with(
            Agent::OpenAI,
            &input(&[("OPENAI_API_KEY", "sensitive-value")]),
        )
        .unwrap();
        let agent = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let request = connection
            .request(&agent, "POST", &connection.inference_url("model"))
            .build()
            .unwrap();
        assert_eq!(
            header(&request, "Authorization"),
            Some("Bearer sensitive-value")
        );
        for url in [
            "https://evil.test/v1/responses",
            "https://api.openai.com/v1/responses?x=1",
            "https://api.openai.com/v1/responses/extra",
            "https://api.openai.com/v1/other",
        ] {
            assert!(
                header(
                    &connection.request(&agent, "POST", url).build().unwrap(),
                    "Authorization"
                )
                .is_none()
            );
        }
        assert!(!format!("{connection:?}").contains("sensitive-value"));
        assert!(!connection.fingerprint().contains("sensitive-value"));
        let other = resolve_with(
            Agent::OpenAI,
            &input(&[("OPENAI_API_KEY", "changed-value")]),
        )
        .unwrap();
        assert_ne!(connection.fingerprint(), other.fingerprint());
    }

    #[test]
    fn provider_specific_auth_headers_and_google_model_encoding() {
        let agent = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let anthropic = resolve_with(
            Agent::Anthropic,
            &input(&[("ANTHROPIC_API_KEY", "api-key")]),
        )
        .unwrap();
        let request = anthropic
            .request(&agent, "POST", &anthropic.inference_url("model"))
            .build()
            .unwrap();
        assert_eq!(header(&request, "x-api-key"), Some("api-key"));
        assert!(request.headers().get("x-api-key").unwrap().is_sensitive());
        assert_eq!(header(&request, "anthropic-version"), Some("2023-06-01"));
        assert!(header(&request, "Authorization").is_none());
        let google =
            resolve_with(Agent::Google, &input(&[("GOOGLE_API_KEY", "google-key")])).unwrap();
        let url = google.inference_url("models/custom model?x=1");
        assert_eq!(
            url,
            "https://generativelanguage.googleapis.com/v1beta/models/custom%20model%3Fx%3D1:streamGenerateContent?alt=sse"
        );
        assert_eq!(
            header(
                &google.request(&agent, "POST", &url).build().unwrap(),
                "x-goog-api-key"
            ),
            Some("google-key")
        );
        assert!(!url.contains("google-key"));
        for unscoped in [
            url.replace("?alt=sse", "?alt=json"),
            format!("{url}&key=unexpected"),
            url.replace(":streamGenerateContent?alt=sse", ":generateContent"),
        ] {
            assert!(
                header(
                    &google.request(&agent, "POST", &unscoped).build().unwrap(),
                    "x-goog-api-key"
                )
                .is_none()
            );
        }
    }

    #[test]
    fn paginated_model_requests_keep_auth_without_accepting_unrelated_queries() {
        let client = reqwest::Client::new();
        for (provider, key, header_name, query) in [
            (
                Agent::Anthropic,
                "ANTHROPIC_API_KEY",
                "x-api-key",
                "after_id=next%2Fpage",
            ),
            (
                Agent::Google,
                "GEMINI_API_KEY",
                "x-goog-api-key",
                "pageToken=next%2Fpage",
            ),
            (
                Agent::OpenAI,
                "OPENAI_API_KEY",
                "authorization",
                "cursor=next%2Fpage",
            ),
        ] {
            let connection = resolve_with(provider, &input(&[(key, "test-credential")])).unwrap();
            let url = format!("{}?{query}", connection.models_url());
            let request = connection.request(&client, "GET", &url).build().unwrap();
            assert!(request.headers().contains_key(header_name));
            let altered = format!("{url}&redirect=https://elsewhere.test");
            assert!(
                !connection
                    .request(&client, "GET", &altered)
                    .build()
                    .unwrap()
                    .headers()
                    .contains_key(header_name)
            );
        }
    }

    #[test]
    fn chatgpt_origin_metadata_claims_and_expiry() {
        let token = jwt(
            serde_json::json!({"exp":101,"https://api.openai.com/auth":{"chatgpt_account_id":"account","chatgpt_data_residency":"eu"}}),
        );
        let connection = resolve_with(
            Agent::ChatGPT,
            &input(&[
                ("CODEX_ACCESS_TOKEN", &token),
                ("OPENAI_BASE_URL", "https://evil.test"),
                ("CODEX_ACCOUNT_ID", "explicit-account"),
            ]),
        )
        .unwrap();
        assert_eq!(connection.base_url, CODEX_BASE);
        assert_eq!(connection.account_id.as_deref(), Some("explicit-account"));
        assert_eq!(
            connection.models_url(),
            "https://chatgpt.com/backend-api/codex/models?client_version=0.162.1"
        );
        assert_eq!(
            connection.inference_url("ignored"),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        let agent = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let request = connection
            .request(&agent, "GET", &connection.models_url())
            .build()
            .unwrap();
        assert_eq!(header(&request, "originator"), Some("eevee"));
        assert_eq!(
            header(&request, "ChatGPT-Account-Id"),
            Some("explicit-account")
        );
        assert_eq!(
            header(&request, "x-openai-internal-codex-residency"),
            Some("eu")
        );
        assert!(
            header(
                &connection
                    .request(&agent, "GET", "https://evil.test/models")
                    .build()
                    .unwrap(),
                "Authorization"
            )
            .is_none()
        );
        let expired = jwt(serde_json::json!({"exp":100}));
        assert!(
            resolve_with(Agent::ChatGPT, &input(&[("CODEX_ACCESS_TOKEN", &expired)]))
                .err()
                .unwrap()
                .contains("expired")
        );
    }

    #[cfg(unix)]
    struct Temp(PathBuf);
    #[cfg(unix)]
    impl Temp {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::getrandom(&mut random).unwrap();
            let path = env::temp_dir().join(format!(
                "eevee-native-config-{:x}",
                u128::from_le_bytes(random)
            ));
            fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
        fn file(&self, name: &str, text: &str, mode: u32) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let path = self.0.join(name);
            fs::write(&path, text).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            path
        }
    }
    #[cfg(unix)]
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn file_permissions_symlinks_and_project_boundaries() {
        use std::os::unix::fs::symlink;
        let temp = Temp::new();
        let private = temp.file("auth.json", "{}", 0o600);
        assert!(read_user_file(&private, true, None).is_ok());
        let public = temp.file("public.json", "{}", 0o644);
        assert!(read_user_file(&public, true, None).is_err());
        assert!(read_user_file(&public, false, None).is_ok());
        let writable = temp.file("writable.toml", "", 0o666);
        assert!(read_user_file(&writable, false, None).is_err());
        assert!(read_user_file(&private, true, Some(&temp.0)).is_err());
        let link = temp.0.join("link.json");
        symlink(&private, &link).unwrap();
        assert!(read_user_file(&link, true, None).is_err());
        assert!(read_user_file(&temp.0, true, None).is_err());
        assert!(read_user_file(Path::new("relative.json"), true, None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn explicit_user_config_precedence_and_read_only_auth_file() {
        let temp = Temp::new();
        let config = temp.file("providers.toml", "[openai]\nbase_url='https://file.example/v1'\nmodel='file-model'\napi_key_env='MY_KEY'\n", 0o600);
        let input = input(&[
            ("EV_PROVIDERS_CONFIG", config.to_str().unwrap()),
            ("OPENAI_MODEL", "env-model"),
            ("MY_KEY", "api-key"),
        ]);
        let connection = resolve_with(Agent::OpenAI, &input).unwrap();
        assert_eq!(connection.base_url, "https://file.example/v1");
        assert_eq!(connection.model.as_deref(), Some("env-model"));
        let text =
            "{\"tokens\":{\"access_token\":\"opaque-token\",\"account_id\":\"file-account\"}}";
        let path = temp.file("auth.json", text, 0o600);
        let connection = resolve_with(
            Agent::ChatGPT,
            &super::tests::input(&[("CODEX_HOME", temp.0.to_str().unwrap())]),
        )
        .unwrap();
        assert_eq!(connection.account_id.as_deref(), Some("file-account"));
        assert_eq!(fs::read_to_string(path).unwrap(), text);
        let environment = super::tests::input(&[
            ("CODEX_HOME", "/nonexistent/never-read"),
            ("CODEX_ACCESS_TOKEN", "explicit-token"),
        ]);
        assert_eq!(
            resolve_with(Agent::ChatGPT, &environment)
                .unwrap()
                .secret
                .as_deref(),
            Some("explicit-token")
        );
        fs::write(&config, "[compatible]\nbase_url='http://localhost:4321/v1'\nkeyless=true\nprotocol='responses'\n").unwrap();
        let compatible = resolve_with(
            Agent::Compatible,
            &super::tests::input(&[("EV_PROVIDERS_CONFIG", config.to_str().unwrap())]),
        )
        .unwrap();
        assert!(compatible.secret.is_none());
        assert_eq!(compatible.protocol, Protocol::Responses);
    }
    #[cfg(unix)]
    #[test]
    fn codex_model_and_effort_are_passive_nonsecret_preferences() {
        let temp = Temp::new();
        temp.file("config.toml", "model='custom-native-model'\nmodel_reasoning_effort='high'\n[unrelated]\nignored='not exported'\n", 0o600);
        let mut input = input(&[("CODEX_HOME", temp.0.to_str().unwrap())]);
        let settings = settings(Agent::ChatGPT, &input).unwrap();
        assert_eq!(settings.model.as_deref(), Some("custom-native-model"));
        assert_eq!(settings.effort.as_deref(), Some("high"));
        input.project = Some(temp.0.clone());
        assert!(super::settings(Agent::ChatGPT, &input).is_err());
        let overrides = super::tests::input(&[
            ("CODEX_HOME", temp.0.to_str().unwrap()),
            ("CODEX_MODEL", "explicit-model"),
            ("CODEX_REASONING_EFFORT", "low"),
            ("CODEX_ACCESS_TOKEN", "opaque-token"),
        ]);
        let connection = resolve_with(Agent::ChatGPT, &overrides).unwrap();
        assert_eq!(connection.model.as_deref(), Some("explicit-model"));
        assert_eq!(connection.effort.as_deref(), Some("low"));
        assert!(!connection.description.contains("not exported"));
    }

    #[test]
    fn preferred_provider_selection_is_passive_and_override_is_exact() {
        for (name, expected) in [
            ("chatgpt", Agent::ChatGPT),
            ("openai", Agent::OpenAI),
            ("anthropic", Agent::Anthropic),
            ("google", Agent::Google),
            ("compatible", Agent::Compatible),
            ("codex", Agent::Codex),
            ("claude", Agent::Claude),
            ("opencode", Agent::OpenCode),
            ("gemini", Agent::Gemini),
            ("devin", Agent::Devin),
        ] {
            assert_eq!(
                preferred_with(Some(name), |_| panic!(
                    "explicit choice must not inspect credentials"
                )),
                expected
            );
        }
        assert_eq!(preferred_with(None, |_| true), Agent::ChatGPT);
        assert_eq!(
            preferred_with(None, |p| p == Agent::OpenAI || p == Agent::Google),
            Agent::OpenAI
        );
        assert_eq!(
            preferred_with(None, |p| p == Agent::Compatible),
            Agent::Compatible
        );
        assert_eq!(preferred_with(None, |_| false), Agent::Codex);
        assert_eq!(
            preferred_with(Some("unknown"), |p| p == Agent::Anthropic),
            Agent::Anthropic
        );
    }
}
