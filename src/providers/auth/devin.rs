//! Fixed-endpoint Devin OAuth. Credentials never enter task state or provider CLI stores.
use super::{Completion, INTERRUPTED, LOGIN_TIMEOUT, Method, Outcome, Plan, RawInput};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const AUTHORIZE: &str = "https://app.devin.ai/auth/cli/continue";
const TOKEN: &str = "https://api.devin.ai/auth/cli/token";
const REDIRECT: &str = "http://127.0.0.1:59653/callback";
const LIMIT: usize = 16 * 1024;
const PREFIX: &str = "devin-session-token$";
const EXPIRY_SKEW: u64 = 60;

pub struct DevinCredential {
    token: String,
    expires_at: u64,
}
impl std::fmt::Debug for DevinCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DevinCredential { redacted }")
    }
}
impl Drop for DevinCredential {
    fn drop(&mut self) {
        unsafe { std::ptr::write_bytes(self.token.as_mut_ptr(), 0, self.token.len()) }
    }
}
impl DevinCredential {
    pub(crate) fn into_parts(mut self) -> (String, u64) {
        (std::mem::take(&mut self.token), self.expires_at)
    }
}

#[cfg(test)]
pub(crate) fn test_credential(token: &str, expires_at: u64) -> DevinCredential {
    DevinCredential {
        token: token.into(),
        expires_at,
    }
}

fn now() -> Result<u64, Outcome> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|at| at.as_secs())
        .map_err(|_| Outcome::Failed)
}

/// Match the wire credential vocabulary: bare JWTs acquire the session prefix;
/// already-prefixed sessions and legacy sk-ws-/opaque keys remain unchanged.
/// Decoded expiry is checked locally, never treated as signature verification.
pub(crate) fn api_key(input: &str, now: u64) -> Result<(String, Option<u64>), String> {
    let input = input.trim();
    if input.is_empty() || input.len() > 64 * 1024 || !input.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(
            "Devin credential must be nonempty, bounded and contain no whitespace/control bytes."
                .into(),
        );
    }
    let bare = input.strip_prefix(PREFIX).unwrap_or(input);
    let mut parts = bare.split('.');
    let _ = parts.next();
    let payload = parts.next();
    let jwt = payload.is_some() && parts.next().is_some() && parts.next().is_none();
    let expires = if jwt {
        let mut payload = payload.expect("JWT payload").to_owned();
        while !payload.len().is_multiple_of(4) {
            payload.push('=');
        }
        let value = base64_simd::URL_SAFE
            .decode_to_vec(payload.as_bytes())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        match value.as_ref().and_then(|value| value.get("exp")) {
            Some(exp) => Some(
                exp.as_u64()
                    .ok_or("Devin credential expiry metadata is invalid.")?
                    .saturating_sub(EXPIRY_SKEW),
            ),
            None => None,
        }
    } else {
        None
    };
    if expires.is_some_and(|at| now >= at) {
        return Err("Devin connection expired. Reconnect explicitly; no renewal or alternate credential was attempted.".into());
    }
    let token = if jwt && !input.starts_with(PREFIX) {
        format!("{PREFIX}{input}")
    } else {
        input.to_owned()
    };
    Ok((token, expires))
}

pub(super) fn plan(profile: usize, method: Method) -> Plan {
    Plan {
        profile, method, executable: profile, trust: None, directory: None, created: Instant::now(),
        lines: vec![
            format!("Selected method: {}", method.label()),
            "No CLI executable or provider-owned store is used. Browser authorization: https://app.devin.ai/auth/cli/continue, fresh UUID state, S256 PKCE and prompt=select_account. Callback: http://127.0.0.1:59653/callback; a complete pasted redirect or code#state pair must match this attempt. Raw tokens are not accepted as OAuth callbacks.".into(),
            "Exchange: POST https://api.devin.ai/auth/cli/token with JSON code/code_verifier and redirects disabled. A successful grant is scoped to native https://server.codeium.com Connect requests, never injected into child processes.".into(),
            "Replaces only the Eevee Devin connection, with the selected storage choice: private plaintext ~/.eevee-connections/devin.json (not encrypted/keychain), or session-only memory shadowing earlier saved credentials until exit. Explicit persistence clears the previous session override. Provider stores and environment variables are never changed.".into(),
            "Grant expiry is checked before use. No automatic renewal, hidden CLI import, alternate account or fallback. No task/model request is sent; login does not prove model eligibility. Cancellation does not revoke grants already issued. Callback and token output is never journaled; do not share transient sign-in links. Esc/Ctrl+C cancels; login timeout ten minutes, approval five minutes.".into(),
        ],
        summary: vec![
            "Devin browser OAuth / pasted callback; no CLI.".into(),
            "app.devin.ai → api.devin.ai; PKCE + state.".into(),
            "Callback 127.0.0.1:59653/callback (10 min).".into(),
            "Replaces app Devin connection only.".into(),
            "Selected private-save/session-only storage applies.".into(),
            "Native server.codeium.com; no child token injection.".into(),
            "No task sent; cancel does not revoke grants.".into(),
        ],
    }
}

struct Grant {
    state: String,
    verifier: String,
    consumed: bool,
}
impl Grant {
    fn new() -> Result<Self, Outcome> {
        let mut uuid = [0u8; 16];
        let mut bytes = [0u8; 96];
        getrandom::getrandom(&mut uuid).map_err(|_| Outcome::Failed)?;
        getrandom::getrandom(&mut bytes).map_err(|_| Outcome::Failed)?;
        uuid[6] = (uuid[6] & 15) | 64;
        uuid[8] = (uuid[8] & 63) | 128;
        let state = uuid
            .iter()
            .enumerate()
            .fold(String::new(), |mut out, (index, byte)| {
                use std::fmt::Write;
                if matches!(index, 4 | 6 | 8 | 10) {
                    out.push('-');
                }
                let _ = write!(out, "{byte:02x}");
                out
            });
        Ok(Self {
            state,
            verifier: base64_simd::URL_SAFE_NO_PAD.encode_to_string(bytes),
            consumed: false,
        })
    }
    fn authorize_url(&self) -> String {
        let challenge =
            base64_simd::URL_SAFE_NO_PAD.encode_to_string(Sha256::digest(self.verifier.as_bytes()));
        let mut url = reqwest::Url::parse(AUTHORIZE).expect("fixed authorize URL");
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("redirect_uri", REDIRECT),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", self.state.as_str()),
            ("prompt", "select_account"),
        ]);
        url.into()
    }
    fn callback(&mut self, input: &str) -> Result<Option<String>, Outcome> {
        let input = input.trim();
        if self.consumed || input.len() > LIMIT || input.bytes().any(|b| b.is_ascii_control()) {
            return Ok(None);
        }
        let pairs: Vec<(String, String)> = if let Ok(url) = reqwest::Url::parse(input) {
            if !input.starts_with(&format!("{REDIRECT}?"))
                || url.fragment().is_some()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Ok(None);
            }
            url.query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect()
        } else if let Some((code, state)) = input.split_once('#') {
            if code.contains(['?', '&', '=', '#']) || state.contains('#') {
                return Ok(None);
            }
            vec![("code".into(), code.into()), ("state".into(), state.into())]
        } else {
            return Ok(None);
        };
        if ["code", "state", "error"]
            .iter()
            .any(|name| pairs.iter().filter(|(key, _)| key == *name).count() > 1)
        {
            return Ok(None);
        }
        let one = |name: &str| {
            let mut values = pairs
                .iter()
                .filter(|(key, _)| key == name)
                .map(|(_, value)| value.as_str());
            let first = values.next()?;
            values.next().is_none().then_some(first)
        };
        if one("state") != Some(self.state.as_str()) {
            return Ok(None);
        }
        if one("error").is_some() {
            self.consumed = true;
            return Err(Outcome::NotAuthenticated);
        }
        let Some(code) = one("code").filter(|code| {
            !code.is_empty() && code.len() <= LIMIT && code.bytes().all(|b| b.is_ascii_graphic())
        }) else {
            return Ok(None);
        };
        self.consumed = true;
        Ok(Some(code.to_owned()))
    }
}

fn serve(mut stream: TcpStream, grant: &mut Grant) -> Result<Option<String>, Outcome> {
    // Accepted sockets inherit the listener's nonblocking flag on macOS.
    stream.set_nonblocking(false).map_err(|_| Outcome::Failed)?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    let started = Instant::now();
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 1024];
    while bytes.len() < LIMIT && started.elapsed() < Duration::from_secs(1) {
        if INTERRUPTED.load(Ordering::Acquire) {
            return Err(Outcome::Cancelled);
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(_) => break,
        }
    }
    let request = String::from_utf8_lossy(&bytes);
    let mut words = request.split_whitespace();
    use super::callback::State;
    let (result, page) = match (words.next(), words.next()) {
        (Some("GET"), Some(target))
            if target.starts_with("/callback?")
                && bytes.len() < LIMIT
                && bytes.windows(4).any(|part| part == b"\r\n\r\n") =>
        {
            let result = grant.callback(&format!("http://127.0.0.1:59653{target}"));
            let page = match &result {
                Ok(Some(_)) => State::Received,
                Err(_) => State::Denied,
                _ => State::Invalid,
            };
            (result, page)
        }
        (Some("GET"), Some(target)) if !target.starts_with("/callback") => {
            (Ok(None), State::NotFound)
        }
        (Some(method), Some(_)) if method != "GET" => (Ok(None), State::MethodNotAllowed),
        _ => (Ok(None), State::Invalid),
    };
    let _ = super::callback::respond(&mut stream, page);
    result
}

fn credential(mut body: serde_json::Value, now: u64) -> Result<DevinCredential, Outcome> {
    let Some(serde_json::Value::String(token)) = body.get_mut("token").map(serde_json::Value::take)
    else {
        return Err(Outcome::Failed);
    };
    let (token, expiry) = api_key(&token, now).map_err(|_| Outcome::Failed)?;
    let expires_at = expiry.unwrap_or_else(|| now.saturating_add(365 * 24 * 60 * 60));
    if now >= expires_at {
        return Err(Outcome::Expired);
    }
    Ok(DevinCredential { token, expires_at })
}

fn exchange(code: &str, verifier: &str) -> Result<DevinCredential, Outcome> {
    let http = ureq::AgentBuilder::new()
        .https_only(true)
        .redirects(0)
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(15))
        .timeout_write(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build();
    exchange_at(&http, TOKEN, code, verifier)
}

/// The production caller supplies only the fixed HTTPS token endpoint and a
/// redirect-disabled client. A loopback fixture exercises the identical body
/// exchange/parser without performing a real account login.
fn exchange_at(
    http: &ureq::Agent,
    endpoint: &str,
    code: &str,
    verifier: &str,
) -> Result<DevinCredential, Outcome> {
    let body = serde_json::json!({"code":code,"code_verifier":verifier}).to_string();
    let response = http
        .post(endpoint)
        .set("Accept", "application/json")
        .set("Content-Type", "application/json")
        .send_string(&body)
        .map_err(|_| Outcome::Failed)?;
    if !(200..300).contains(&response.status()) {
        return Err(Outcome::Failed);
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take((64 * 1024 + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Outcome::Failed)?;
    if bytes.len() > 64 * 1024 {
        return Err(Outcome::Failed);
    }
    let body = serde_json::from_slice(&bytes).map_err(|_| Outcome::Failed)?;
    credential(body, now()?)
}

pub(super) fn login() -> Completion {
    let result = (|| {
        let mut grant = Grant::new()?;
        let listener = match TcpListener::bind("127.0.0.1:59653") {
            Ok(listener) => {
                listener
                    .set_nonblocking(true)
                    .map_err(|_| Outcome::Failed)?;
                Some(listener)
            }
            Err(_) => {
                println!(
                    "Local callback unavailable; paste this attempt's redirect or code#state instead.\r"
                );
                None
            }
        };
        let url = grant.authorize_url();
        println!("Devin sign-in: {url}\r");
        println!(
            "Paste the complete redirect URL or code#state after sign-in, then Enter. Input is hidden; raw API keys belong in API-key setup. Esc/Ctrl+C cancels.\r"
        );
        let _raw = RawInput::new().map_err(|_| Outcome::Failed)?;
        let mut input = crate::platform::input::Input::new().map_err(|_| Outcome::Failed)?;
        super::open_browser(&url);
        let started = Instant::now();
        let mut paste = String::new();
        loop {
            if INTERRUPTED.load(Ordering::Acquire) {
                return Err(Outcome::Cancelled);
            }
            if started.elapsed() >= LOGIN_TIMEOUT {
                return Err(Outcome::Expired);
            }
            let mut code = None;
            if let Some(listener) = &listener {
                // One bounded intake per tick: local invalid requests cannot starve cancellation.
                match listener.accept() {
                    Ok((stream, _)) => code = serve(stream, &mut grant)?,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => return Err(Outcome::Failed),
                }
            }
            if code.is_none()
                && let Some(submitted) =
                    super::read_input(&mut paste, || input.read_timeout(Duration::ZERO))?
            {
                code = grant.callback(&submitted)?;
                if code.is_none() {
                    println!(
                        "Invalid callback or state; paste the exact redirect or code#state from this attempt. Ctrl+U clears hidden input.\r"
                    );
                }
            }
            if let Some(code) = code {
                if started.elapsed() >= LOGIN_TIMEOUT {
                    return Err(Outcome::Expired);
                }
                if INTERRUPTED.load(Ordering::Acquire) {
                    return Err(Outcome::Cancelled);
                }
                let credential = exchange(&code, &grant.verifier)?;
                if INTERRUPTED.load(Ordering::Acquire) {
                    return Err(Outcome::Cancelled);
                }
                if started.elapsed() >= LOGIN_TIMEOUT {
                    return Err(Outcome::Expired);
                }
                return Ok(credential);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    })();
    match result {
        Ok(credential) => Completion {
            outcome: Outcome::Authenticated,
            devin_credential: Some(credential),
            openai_credential: None,
            openai_failure: None,
        },
        Err(outcome) => Completion::bare(outcome),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn grant() -> Grant {
        Grant {
            state: "private-state".into(),
            verifier: "private-verifier".into(),
            consumed: false,
        }
    }
    #[test]
    fn callback_requires_exact_origin_path_state_and_one_use_without_guessing_tokens() {
        let mut grant = grant();
        for input in [
            "sk-ws-private-token",
            "private-code",
            "private-code#wrong",
            "https://evil.invalid/callback?code=private-code&state=private-state",
            "http://127.0.0.1:59653/callback?code=private-code&state=private-state&state=private-state",
            "http://127.0.0.1:59653/callback?code=one&code=two&state=private-state",
            "http://127.0.0.1:59653/callback?code=private-code&state=private-state#fragment",
        ] {
            assert!(grant.callback(input).unwrap().is_none());
            assert!(!grant.consumed);
        }
        assert_eq!(
            grant
                .callback("private-code#private-state")
                .unwrap()
                .as_deref(),
            Some("private-code")
        );
        assert!(
            grant
                .callback("private-code#private-state")
                .unwrap()
                .is_none()
        );
        let mut grant = self::grant();
        assert_eq!(
            grant
                .callback(&format!("{REDIRECT}?code=private-code&state=private-state"))
                .unwrap()
                .as_deref(),
            Some("private-code")
        );
    }
    #[test]
    fn credentials_prefix_jwts_refuse_expiry_and_redact_debug() {
        let payload = base64_simd::URL_SAFE_NO_PAD.encode_to_string(br#"{"exp":1000}"#);
        let jwt = format!("header.{payload}.signature");
        let (key, expiry) = api_key(&jwt, 100).unwrap();
        assert_eq!(key, format!("{PREFIX}{jwt}"));
        assert_eq!(expiry, Some(940));
        assert_eq!(api_key(&key, 100).unwrap().0, key);
        assert!(api_key(&jwt, 940).is_err());
        assert_eq!(
            api_key(" sk-ws-legacy-secret ", 100).unwrap().0,
            "sk-ws-legacy-secret"
        );
        assert!(api_key("secret\nother", 100).is_err());
        let parsed = credential(serde_json::json!({"token":jwt}), 100).unwrap();
        assert!(!format!("{parsed:?}").contains("signature"));
        assert_eq!(parsed.expires_at, 940);
        assert!(credential(serde_json::json!({"token":jwt}), 1000).is_err());
    }
    #[test]
    fn callback_page_never_echoes_private_query_data() {
        use std::io::Write;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .write_all(b"GET /callback?code=private-code&state=private-state HTTP/1.1\r\n\r\n")
                .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        });
        assert_eq!(
            serve(listener.accept().unwrap().0, &mut grant())
                .unwrap()
                .as_deref(),
            Some("private-code")
        );
        assert!(!client.join().unwrap().contains("private-"));
    }

    fn token_fixture(
        status: &str,
        headers: &str,
        body: String,
    ) -> (String, std::thread::JoinHandle<String>) {
        use std::io::Write;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/auth/cli/token", listener.local_addr().unwrap());
        let status = status.to_owned();
        let headers = headers.to_owned();
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap().0;
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let n = stream.read(&mut chunk).unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(split) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&bytes[..split]);
                    let length: usize = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= split + 4 + length {
                        break;
                    }
                }
            }
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8(bytes).unwrap()
        });
        (endpoint, server)
    }

    #[test]
    fn oauth_real_http_exchange_posts_bound_pkce_and_refuses_redirects_and_private_errors() {
        let http = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout(Duration::from_secs(3))
            .build();
        let mut grant = Grant::new().unwrap();
        let code = grant
            .callback(&format!(
                "{REDIRECT}?code=private-code&state={}",
                grant.state
            ))
            .unwrap()
            .unwrap();
        let (endpoint, server) = token_fixture(
            "200 OK",
            "",
            r#"{"token":"sk-ws-private-issued-key"}"#.into(),
        );
        let credential = exchange_at(&http, &endpoint, &code, &grant.verifier).unwrap();
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /auth/cli/token HTTP/1.1"));
        let body: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["code"], "private-code");
        assert_eq!(body["code_verifier"], grant.verifier);
        assert_eq!(body.as_object().unwrap().len(), 2);
        assert!(!format!("{credential:?}").contains("private-issued-key"));
        assert_eq!(credential.into_parts().0, "sk-ws-private-issued-key");
        assert!(
            grant
                .callback(&format!(
                    "{REDIRECT}?code=private-code&state={}",
                    grant.state
                ))
                .unwrap()
                .is_none()
        );

        let redirected = TcpListener::bind("127.0.0.1:0").unwrap();
        redirected.set_nonblocking(true).unwrap();
        let headers = format!(
            "Location: http://{}/would-leak-code\r\n",
            redirected.local_addr().unwrap()
        );
        let (endpoint, server) = token_fixture("302 Found", &headers, "{}".into());
        assert_eq!(
            exchange_at(&http, &endpoint, "private-code", "private-verifier").unwrap_err(),
            Outcome::Failed
        );
        server.join().unwrap();
        assert!(
            matches!(redirected.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
        for (status, body) in [
            (
                "401 Unauthorized",
                r#"{"error":"private-provider-secret"}"#.into(),
            ),
            ("200 OK", r#"{"error":"private-provider-secret"}"#.into()),
            ("200 OK", "x".repeat(64 * 1024 + 1)),
        ] {
            let (endpoint, server) = token_fixture(status, "", body);
            let failure =
                exchange_at(&http, &endpoint, "private-code", "private-verifier").unwrap_err();
            server.join().unwrap();
            assert_eq!(failure, Outcome::Failed);
            assert!(!format!("{failure:?}").contains("private-"));
        }
    }
}
