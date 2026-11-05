//! Reviewed native OpenAI authorization. Endpoints are fixed; no provider store is read.
use super::{Completion, INPUT_LIMIT, INTERRUPTED, Method, Outcome, Plan, RawInput, read_input};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ISSUER: &str = "https://auth.openai.com";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const BODY_LIMIT: usize = 128 * 1024;
const DEVICE_TIMEOUT: Duration = Duration::from_secs(900);
const EXPIRY_SKEW: u64 = 60;
const DEVICE_TRANSIENT_RETRIES: u32 = 3;
const DEVICE_INTERVAL_LIMIT: Duration = Duration::from_secs(60);

/// Content-free login diagnostics. Only locally authored stages/reasons and an
/// HTTP status may cross back into the workbench; never response bodies or URLs.
#[derive(Clone, Copy, Debug)]
pub struct OpenAiFailure {
    stage: &'static str,
    title: &'static str,
    reason: &'static str,
    status: Option<u16>,
    remedy: &'static str,
    outcome: Outcome,
}
impl OpenAiFailure {
    /// Compact, content-free context for the primary diagnostics header.
    pub fn summary(self) -> String {
        self.status.map_or_else(
            || format!("{}: {}", self.stage, self.title),
            |status| format!("{}: {} (HTTP {status})", self.stage, self.title),
        )
    }

    pub fn remedy(self) -> &'static str {
        self.remedy
    }

    #[cfg(test)]
    pub(crate) fn test_http_failure(status: u16) -> Self {
        Self {
            stage: "Device authorization setup",
            title: http_title(status),
            reason: "Device authorization was refused; no private issuer response was retained.",
            status: Some(status),
            remedy: http_remedy(status, true),
            outcome: Outcome::Failed,
        }
    }

    pub fn message(self) -> String {
        let status = self
            .status
            .map_or(String::new(), |status| format!(" (HTTP {status})"));
        format!(
            "{}{}: {} No connection settings changed; no task sent.",
            self.stage, status, self.reason
        )
    }
}

fn http_title(status: u16) -> &'static str {
    match status {
        400 => "request rejected",
        401 => "authorization rejected",
        403 => "access refused",
        404 => "endpoint unavailable",
        429 => "rate limited",
        500..=599 => "service error",
        _ => "unexpected HTTP status",
    }
}

fn http_remedy(status: u16, device: bool) -> &'static str {
    match status {
        429 => "Wait for the service limit to clear, then explicitly start a fresh login.",
        500..=599 => "The identity service failed; explicitly start a fresh login later.",
        403 if device => {
            "Check device authorization availability in ChatGPT Security; Browser is an explicit alternative."
        }
        400 | 401 => "Start a fresh login; old authorization codes cannot be reused.",
        _ => {
            "Inspect the HTTP status and network/device availability, then explicitly start a fresh login."
        }
    }
}

/// A one-use result of the reviewed OAuth exchange, never a generic bearer guess.
/// No Debug/serialization surface exposes identity or secret bytes. Renewal is
/// deliberately not automatic: reconnect when this access grant expires.
pub struct OpenAiCredential {
    access_token: String,
    account_id: Option<String>,
    residency: Option<String>,
    expires_at: u64,
}
impl std::fmt::Debug for OpenAiCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OpenAiCredential { redacted }")
    }
}
impl Drop for OpenAiCredential {
    fn drop(&mut self) {
        unsafe {
            std::ptr::write_bytes(self.access_token.as_mut_ptr(), 0, self.access_token.len());
        }
    }
}
impl OpenAiCredential {
    pub(crate) fn into_parts(mut self) -> (String, Option<String>, Option<String>, u64) {
        (
            std::mem::take(&mut self.access_token),
            self.account_id.take(),
            self.residency.take(),
            self.expires_at,
        )
    }
}

pub(super) fn plan(profile: usize, method: Method) -> Plan {
    let mut lines = vec![
        format!("Selected method: {}", method.label()),
        "No provider executable, CLI installation or provider-owned credential store is required. Authorization uses the fixed public client at https://auth.openai.com only; redirects on token/device HTTP requests are disabled.".into(),
        if method == Method::OpenAiBrowser {
            "Browser: /oauth/authorize, public client app_EMoamEEZ73f0CkXaXp7hrann, openid profile email offline_access; S256 PKCE and fresh random state. Registered callback http://localhost:1455/auth/callback (1457 if the first port is unavailable), served on IPv4 and IPv6 loopback. A complete pasted redirect is accepted only for this exact callback and state; arbitrary tokens/codes are not guessed. Exchange: /oauth/token.".into()
        } else {
            "Device: POST /api/accounts/deviceauth/usercode; display the official https://auth.openai.com/codex/device link and one-time code; poll /api/accounts/deviceauth/token for at most 15 minutes. Pending 403/404 responses are polled again; 429 slows polling by 5 seconds and up to three consecutive transient network/5xx polling failures are retried with the same device code. Other statuses stop. Exchange returned authorization code + verifier at /oauth/token with https://auth.openai.com/deviceauth/callback. No localhost listener or browser launch.".into()
        },
        "An approved successful result replaces the active Eevee ChatGPT connection override, never another provider route. Session-only shadows any previous saved connection until exit; explicit private persistence replaces ~/.eevee-connections/chatgpt.json and clears the old session override. Provider-owned stores and environment variables are never modified.".into(),
        "The access credential is scoped only to https://chatgpt.com/backend-api/codex. Returned refresh tokens are not retained or used: this connection expires and requires explicit reconnect. No automatic renewal, account rotation, API billing/protocol fallback, or inference retry.".into(),
        "No task context or model call is sent. Model metadata selection is separate and does not prove eligibility. Any later model request needs its own review and Run approval. Cancellation cannot revoke a grant already issued by the provider.".into(),
        "Secrets, redirect URLs and account identifiers are not journaled or included in notices. The authorization link/device code is transient terminal output; do not share it. Esc/Ctrl+C cancels. Approval expires after five minutes.".into(),
    ];
    if let Some(guidance) = method.guidance() {
        lines.push(guidance.into());
    }
    Plan {
        profile,
        method,
        executable: profile,
        summary: vec![
            format!("Method: {}", method.label()),
            "Direct official OpenAI sign-in; no installed CLI.".into(),
            "Replaces selected app subscription connection only.".into(),
            "Fixed subscription endpoint; no API billing fallback.".into(),
            "Access grant expires; explicit reconnect, no renewal.".into(),
            "No task/model call; cancellation does not revoke.".into(),
            if method == Method::OpenAiDevice {
                "Callback-free · 15 min · Esc/Ctrl+C cancels".into()
            } else {
                "PKCE/state callback or pasted redirect · 10 min".into()
            },
        ],
        lines,
        trust: None,
        created: Instant::now(),
        directory: None,
    }
}

fn safe_value(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.bytes().all(|byte| byte.is_ascii_graphic())
}
fn now() -> Result<u64, Outcome> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|at| at.as_secs())
        .map_err(|_| Outcome::Failed)
}
fn random_secret() -> Result<String, Outcome> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|_| Outcome::Failed)?;
    Ok(base64_simd::URL_SAFE_NO_PAD.encode_to_string(bytes))
}
fn challenge(verifier: &str) -> String {
    base64_simd::URL_SAFE_NO_PAD.encode_to_string(Sha256::digest(verifier.as_bytes()))
}
fn pkce_valid(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
}

struct BrowserGrant {
    state: String,
    verifier: String,
    redirect: String,
    consumed: bool,
}
impl BrowserGrant {
    fn new(port: u16) -> Result<Self, Outcome> {
        Ok(Self {
            state: random_secret()?,
            verifier: random_secret()?,
            // The public client registers the `localhost` spelling; a numeric
            // loopback redirect is rejected by the authorize endpoint.
            redirect: format!("http://localhost:{port}/auth/callback"),
            consumed: false,
        })
    }
    fn authorize_url(&self) -> String {
        let mut url = reqwest::Url::parse(&format!("{ISSUER}/oauth/authorize")).expect("fixed URL");
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", self.redirect.as_str()),
            ("scope", "openid profile email offline_access"),
            ("code_challenge", challenge(&self.verifier).as_str()),
            ("code_challenge_method", "S256"),
            ("state", self.state.as_str()),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", "eevee"),
        ]);
        url.into()
    }
    /// Invalid/mismatched input cannot terminate or consume the live attempt.
    fn callback(&mut self, input: &str) -> Result<Option<String>, Outcome> {
        if self.consumed || input.len() > INPUT_LIMIT || input.bytes().any(|b| b.is_ascii_control())
        {
            return Ok(None);
        }
        let Ok(url) = reqwest::Url::parse(input) else {
            return Ok(None);
        };
        let expected = reqwest::Url::parse(&self.redirect).expect("fixed callback");
        if url.origin() != expected.origin()
            || url.path() != expected.path()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || !input.starts_with(&format!("{}?", self.redirect))
        {
            return Ok(None);
        }
        let (mut state, mut code, mut error) = (None, None, None);
        for (key, value) in url.query_pairs() {
            let slot = match key.as_ref() {
                "state" => &mut state,
                "code" => &mut code,
                "error" => &mut error,
                _ => continue,
            };
            if slot.is_some() {
                return Ok(None);
            }
            *slot = Some(value.into_owned());
        }
        if state.as_deref() != Some(self.state.as_str()) || code.is_some() && error.is_some() {
            return Ok(None);
        }
        if error.is_some() {
            self.consumed = true;
            return Err(Outcome::NotAuthenticated);
        }
        let Some(code) = code.filter(|code| safe_value(code, INPUT_LIMIT)) else {
            return Ok(None);
        };
        self.consumed = true;
        Ok(Some(code))
    }
}

/// Poll an in-flight operation without losing its future. Dropping the future on
/// cancellation closes the exchange; there is never a second authorization POST.
async fn guarded<T>(
    future: impl Future<Output = T>,
    control: &mut impl FnMut() -> Result<(), Outcome>,
) -> Result<T, Outcome> {
    let mut future = std::pin::pin!(future);
    loop {
        control()?;
        if let Ok(value) = tokio::time::timeout(Duration::from_millis(25), &mut future).await {
            control()?;
            return Ok(value);
        }
    }
}

struct Endpoint {
    issuer: String,
    client: reqwest::Client,
    poll_floor: Duration,
    slow_down: Duration,
    failure: std::cell::Cell<Option<OpenAiFailure>>,
}
impl Endpoint {
    fn official() -> Result<Self, Outcome> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| Outcome::Failed)?;
        Ok(Self {
            issuer: ISSUER.into(),
            client,
            poll_floor: Duration::from_secs(1),
            slow_down: Duration::from_secs(5),
            failure: std::cell::Cell::new(None),
        })
    }
    #[cfg(test)]
    fn fixture(issuer: String) -> Self {
        let url = reqwest::Url::parse(&issuer).unwrap();
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.scheme(), "http");
        Self {
            issuer,
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            poll_floor: Duration::ZERO,
            slow_down: Duration::ZERO,
            failure: std::cell::Cell::new(None),
        }
    }
    fn failed(
        &self,
        stage: &'static str,
        title: &'static str,
        reason: &'static str,
        status: Option<u16>,
        remedy: &'static str,
        outcome: Outcome,
    ) -> Outcome {
        self.failure.set(Some(OpenAiFailure {
            stage,
            title,
            reason,
            status,
            remedy,
            outcome,
        }));
        outcome
    }
    async fn send(
        &self,
        stage: &'static str,
        request: reqwest::RequestBuilder,
        control: &mut impl FnMut() -> Result<(), Outcome>,
    ) -> Result<(u16, Value), Outcome> {
        self.failure.set(Some(OpenAiFailure {
            stage,
            title: "invalid login data",
            reason: "The identity service returned incomplete or invalid login data. Start a fresh login; no credential was accepted.",
            status: None,
            remedy: "Start a fresh login; the identity service returned invalid data.",
            outcome: Outcome::Failed,
        }));
        let mut response = guarded(request.send(), control)
            .await?
            .map_err(|error| self.failed(stage, if error.is_timeout() { "timed out" } else { "unreachable" }, if error.is_timeout() {
                "The identity service timed out. Check your network/proxy and start a fresh login deliberately."
            } else {
                "Could not reach the identity service. Check your network, proxy and TLS trust settings, then start a fresh login."
            }, None, "Check network/proxy/TLS, then explicitly start a fresh login.", Outcome::Failed))?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(self.failed(stage, "redirect refused", "The identity service redirected this private request; redirects are refused. Check your proxy/network and start a fresh login.", Some(status), "Check network/proxy; private-request redirects are refused.", Outcome::Refused));
        }
        // HTTP errors are classified from their status, never from private bodies.
        // Do not let a large, unreadable or interrupted error body erase that status.
        if !(200..300).contains(&status) {
            return Ok((status, Value::Null));
        }
        if let Some(mut failure) = self.failure.get() {
            failure.status = Some(status);
            self.failure.set(Some(failure));
        }
        if response
            .content_length()
            .is_some_and(|len| len > BODY_LIMIT as u64)
        {
            return Err(self.failed(stage, "response too large", "The identity service response exceeded the safe size limit. Start a fresh login; do not paste tokens into the Request.", Some(status), "Start a fresh login; the response exceeded the safe limit.", Outcome::Failed));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = guarded(response.chunk(), control)
            .await?
            .map_err(|_| self.failed(stage, "response interrupted", "The identity service response was interrupted. Start a fresh login; this exchange was not resent.", Some(status), "Check network/proxy, then explicitly start a fresh login.", Outcome::Failed))?
        {
            if bytes.len() + chunk.len() > BODY_LIMIT {
                return Err(self.failed(stage, "response too large", "The identity service response exceeded the safe size limit. Start a fresh login; do not paste tokens into the Request.", Some(status), "Start a fresh login; the response exceeded the safe limit.", Outcome::Failed));
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = serde_json::from_slice(&bytes).map_err(|_| self.failed(stage, "unreadable response", "The identity service returned an unreadable response. Check your proxy/network and start a fresh login.", Some(status), "Check network/proxy, then explicitly start a fresh login.", Outcome::Failed))?;
        Ok((status, body))
    }
    async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        redirect: &str,
        control: &mut impl FnMut() -> Result<(), Outcome>,
    ) -> Result<OpenAiCredential, Outcome> {
        if !safe_value(code, INPUT_LIMIT) || !pkce_valid(verifier) {
            return Err(Outcome::Failed);
        }
        let request = self
            .client
            .post(format!("{}/oauth/token", self.issuer))
            .header("Accept", "application/json")
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", CLIENT_ID),
                ("code", code),
                ("code_verifier", verifier),
                ("redirect_uri", redirect),
            ]);
        let (status, value) = self.send("Token exchange", request, control).await?;
        if status != 200 {
            return Err(self.failed("Token exchange", http_title(status), "Token exchange did not complete. Address the observed HTTP error, then start a fresh login deliberately; old redirects/codes cannot be reused. HTTP status alone does not establish an account or workspace-policy cause.", Some(status), http_remedy(status, false), if matches!(status, 400 | 401) { Outcome::NotAuthenticated } else { Outcome::Failed }));
        }
        let credential = credential(value, now()?).map_err(|outcome| self.failed("Credential validation", "invalid access grant", "The returned access grant is expired or has invalid identity/lifetime metadata. Start a fresh login; no credential was accepted.", None, "Start a fresh login; no valid access grant was accepted.", outcome))?;
        control()?;
        Ok(credential)
    }
    async fn device(
        &self,
        control: &mut impl FnMut() -> Result<(), Outcome>,
        mut display: impl FnMut(&str),
    ) -> Result<OpenAiCredential, Outcome> {
        let (status, value) = self
            .send(
                "Device authorization setup",
                self.client
                    .post(format!("{}/api/accounts/deviceauth/usercode", self.issuer))
                    .json(&serde_json::json!({"client_id": CLIENT_ID})),
                control,
            )
            .await?;
        if status != 200 {
            return Err(self.failed("Device authorization setup", http_title(status), "Device login was refused or is unavailable. HTTP status alone does not establish an account or workspace-policy cause. Check network access and device authorization availability in ChatGPT Settings > Security; Browser login remains an explicit alternative.", Some(status), http_remedy(status, true), if status == 401 { Outcome::NotAuthenticated } else { Outcome::Failed }));
        }
        let device_id = value
            .get("device_auth_id")
            .and_then(Value::as_str)
            .filter(|s| safe_value(s, 4096))
            .ok_or(Outcome::Failed)?;
        let user_code = value
            .get("user_code")
            .or_else(|| value.get("usercode"))
            .and_then(Value::as_str)
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 64
                    && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
            .ok_or(Outcome::Failed)?;
        let interval = match value.get("interval") {
            None => 5,
            Some(Value::String(value)) => value.parse::<u64>().map_err(|_| Outcome::Failed)?,
            Some(value) => value.as_u64().ok_or(Outcome::Failed)?,
        };
        let mut interval = Duration::from_secs(interval).max(self.poll_floor);
        display(user_code);
        let started = Instant::now();
        // Polling reuses one issued device code, so a transient polling failure
        // can be retried without a second authorization; setup never retries.
        let mut transient = 0;
        loop {
            control()?;
            if started.elapsed() >= DEVICE_TIMEOUT {
                return Err(Outcome::Expired);
            }
            let polled = self.send(
                "Device authorization polling",
                self.client.post(format!("{}/api/accounts/deviceauth/token", self.issuer))
                    .json(&serde_json::json!({"device_auth_id": device_id, "user_code": user_code})), control,
            ).await;
            let (status, result) = match polled {
                Err(Outcome::Failed)
                    if transient < DEVICE_TRANSIENT_RETRIES
                        && self.failure.get().is_some_and(|f| f.status.is_none()) =>
                {
                    (0, Value::Null)
                }
                polled => polled?,
            };
            let pending = matches!(status, 403 | 404);
            let retry = status == 0 || status == 429 || (500..600).contains(&status);
            if retry && transient >= DEVICE_TRANSIENT_RETRIES && status != 429 {
                return Err(self.failed("Device authorization polling", http_title(status), "Device authorization polling kept failing on service errors. Start a fresh login later; the issued code was not reused.", Some(status), http_remedy(status, true), Outcome::Failed));
            }
            if pending || retry {
                if status == 429 {
                    interval = (interval + self.slow_down).min(DEVICE_INTERVAL_LIMIT);
                }
                transient = if retry && status != 429 {
                    transient + 1
                } else {
                    0
                };
                guarded(
                    tokio::time::sleep(
                        interval.min(DEVICE_TIMEOUT.saturating_sub(started.elapsed())),
                    ),
                    control,
                )
                .await?;
                continue;
            }
            if status != 200 {
                return Err(self.failed("Device authorization polling", http_title(status), "Device authorization did not complete. HTTP status alone does not establish an account or workspace-policy cause. Address the observed HTTP error; check network access and device authorization availability in ChatGPT Settings > Security before starting a fresh login.", Some(status), http_remedy(status, true), if status == 401 { Outcome::NotAuthenticated } else { Outcome::Failed }));
            }
            if started.elapsed() >= DEVICE_TIMEOUT {
                return Err(Outcome::Expired);
            }
            let code = result
                .get("authorization_code")
                .and_then(Value::as_str)
                .ok_or(Outcome::Failed)?;
            let verifier = result
                .get("code_verifier")
                .and_then(Value::as_str)
                .ok_or(Outcome::Failed)?;
            let returned_challenge = result
                .get("code_challenge")
                .and_then(Value::as_str)
                .ok_or(Outcome::Failed)?;
            if !pkce_valid(verifier) || challenge(verifier) != returned_challenge {
                return Err(Outcome::Failed);
            }
            return self
                .exchange(
                    code,
                    verifier,
                    &format!("{}/deviceauth/callback", self.issuer),
                    control,
                )
                .await;
        }
    }
}

fn jwt(token: &str) -> Result<Option<Value>, Outcome> {
    let mut parts = token.split('.');
    let header = parts.next().unwrap_or_default();
    let (Some(payload), Some(_)) = (parts.next(), parts.next()) else {
        return Ok(None);
    };
    if parts.next().is_some() {
        return Ok(None);
    }
    let recognized = base64_simd::URL_SAFE_NO_PAD
        .decode_to_vec(header.as_bytes())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|value| value.is_object());
    if !recognized {
        return Ok(None);
    }
    let bytes = base64_simd::URL_SAFE_NO_PAD
        .decode_to_vec(payload.as_bytes())
        .map_err(|_| Outcome::Failed)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| Outcome::Failed)?;
    if !value.is_object() {
        return Err(Outcome::Failed);
    }
    Ok(Some(value))
}
fn metadata(value: &Value, name: &str) -> Result<Option<String>, Outcome> {
    let auth = value.get("https://api.openai.com/auth");
    let nested = auth.and_then(|a| a.get(name));
    let top = value.get(name);
    if nested.is_some() && top.is_some() && nested != top {
        return Err(Outcome::Failed);
    }
    nested
        .or(top)
        .map(|value| {
            value
                .as_str()
                .filter(|s| safe_value(s, 4096))
                .map(str::to_owned)
                .ok_or(Outcome::Failed)
        })
        .transpose()
}
fn consistent(a: Option<String>, b: Option<String>) -> Result<Option<String>, Outcome> {
    if a.is_some() && b.is_some() && a != b {
        return Err(Outcome::Failed);
    }
    Ok(a.or(b))
}
fn credential(mut value: Value, now: u64) -> Result<OpenAiCredential, Outcome> {
    let token = match value.get_mut("access_token").map(Value::take) {
        Some(Value::String(token)) if safe_value(&token, 64 * 1024) => token,
        _ => return Err(Outcome::Failed),
    };
    if value
        .get("token_type")
        .is_some_and(|v| v.as_str().is_none_or(|s| !s.eq_ignore_ascii_case("bearer")))
    {
        return Err(Outcome::Failed);
    }
    let access = jwt(&token)?;
    let id = value
        .get("id_token")
        .map(|v| {
            v.as_str()
                .ok_or(Outcome::Failed)
                .and_then(|s| jwt(s)?.ok_or(Outcome::Failed))
        })
        .transpose()?;
    let mut expiry = value
        .get("expires_in")
        .map(|v| {
            v.as_u64()
                .and_then(|seconds| now.checked_add(seconds))
                .ok_or(Outcome::Failed)
        })
        .transpose()?;
    let (mut account, mut residency) = (None, None);
    for (is_access, claims) in [(true, access.as_ref()), (false, id.as_ref())]
        .into_iter()
        .filter_map(|(access, value)| value.map(|value| (access, value)))
    {
        if claims
            .get("iss")
            .is_some_and(|v| v.as_str() != Some(ISSUER))
        {
            return Err(Outcome::Failed);
        }
        if let Some(exp) = claims.get("exp") {
            let exp = exp.as_u64().ok_or(Outcome::Failed)?;
            if exp <= now.saturating_add(EXPIRY_SKEW) {
                return Err(Outcome::Expired);
            }
            // ID-token expiry bounds validation of this exchange, not access-token lifetime.
            if is_access {
                expiry = Some(expiry.map_or(exp, |old| old.min(exp)));
            }
        }
        account = consistent(account, metadata(claims, "chatgpt_account_id")?)?;
        residency = consistent(
            residency,
            metadata(claims, "chatgpt_data_residency")?
                .or(metadata(claims, "chatgpt_compute_residency")?),
        )?;
    }
    if let Some(id) = &id
        && let Some(aud) = id.get("aud")
    {
        let matches = aud.as_str() == Some(CLIENT_ID)
            || aud
                .as_array()
                .is_some_and(|values| values.iter().any(|v| v.as_str() == Some(CLIENT_ID)));
        if !matches {
            return Err(Outcome::Failed);
        }
    }
    account = consistent(account, metadata(&value, "account_id")?)?;
    let expires_at = expiry.ok_or(Outcome::Failed)?.saturating_sub(EXPIRY_SKEW);
    if expires_at <= now {
        return Err(Outcome::Expired);
    }
    Ok(OpenAiCredential {
        access_token: token,
        account_id: account,
        residency,
        expires_at,
    })
}

#[cfg(test)]
pub(crate) fn test_credential(token: &str, account: &str, expires_at: u64) -> OpenAiCredential {
    OpenAiCredential {
        access_token: token.into(),
        account_id: Some(account.into()),
        residency: None,
        expires_at,
    }
}

struct Terminal {
    input: crate::platform::input::Input,
    started: Instant,
    timeout: Duration,
    paste: String,
    submitted: Option<String>,
}
impl Terminal {
    fn poll(&mut self) -> Result<Option<String>, Outcome> {
        if INTERRUPTED.load(Ordering::Acquire) {
            return Err(Outcome::Cancelled);
        }
        if self.started.elapsed() >= self.timeout {
            return Err(Outcome::Expired);
        }
        let input = &mut self.input;
        read_input(&mut self.paste, || input.read_timeout(Duration::ZERO))
    }
    fn read(&mut self) -> Result<Option<String>, Outcome> {
        let next = self.poll()?;
        Ok(self.submitted.take().or(next))
    }
    fn check(&mut self) -> Result<(), Outcome> {
        if let Some(value) = self.poll()? {
            self.submitted.get_or_insert(value);
        }
        Ok(())
    }
}

fn serve(
    mut stream: TcpStream,
    grant: &mut BrowserGrant,
    control: &mut impl FnMut() -> Result<(), Outcome>,
) -> Result<Option<String>, Outcome> {
    // Accepted sockets inherit the listener's nonblocking mode on macOS.
    // Use bounded blocking reads so the timeout also bounds cancellation polls.
    stream.set_nonblocking(false).map_err(|_| Outcome::Failed)?;
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|_| Outcome::Failed)?;
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .map_err(|_| Outcome::Failed)?;
    let started = Instant::now();
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 1024];
    while bytes.len() < INPUT_LIMIT && started.elapsed() < Duration::from_secs(1) {
        control()?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(len) => {
                bytes.extend_from_slice(&chunk[..len]);
                if bytes.windows(4).any(|s| s == b"\r\n\r\n") {
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
    let request = std::str::from_utf8(&bytes).unwrap_or_default();
    let target = request
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("GET "))
        .and_then(|line| {
            line.strip_suffix(" HTTP/1.1")
                .or_else(|| line.strip_suffix(" HTTP/1.0"))
        });
    let result = if bytes.len() <= INPUT_LIMIT && request.ends_with("\r\n\r\n") {
        match target.filter(|target| target.starts_with("/auth/callback?")) {
            Some(target) => grant.callback(&format!(
                "{}{}",
                grant.redirect.trim_end_matches("/auth/callback"),
                target
            )),
            None => Ok(None),
        }
    } else {
        Ok(None)
    };
    let state = match &result {
        Ok(Some(_)) => super::callback::State::Received,
        Ok(None) => super::callback::State::Invalid,
        Err(_) => super::callback::State::Denied,
    };
    let _ = super::callback::respond(&mut stream, state);
    result
}
async fn browser(
    endpoint: &Endpoint,
    terminal: &mut Terminal,
) -> Result<OpenAiCredential, Outcome> {
    // `localhost` may resolve to either loopback family: serve IPv6 too when
    // the same port is free there.
    let mut listeners = Vec::new();
    for port in [1455, 1457] {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            listeners.push(listener);
            listeners.extend(TcpListener::bind(("::1", port)).ok());
            break;
        }
    }
    for listener in &listeners {
        listener
            .set_nonblocking(true)
            .map_err(|_| Outcome::Failed)?;
    }
    let port = listeners
        .first()
        .and_then(|l| l.local_addr().ok())
        .map_or(1455, |a| a.port());
    let mut grant = BrowserGrant::new(port)?;
    let url = grant.authorize_url();
    println!("OpenAI account sign-in. Open this official link:\r\n{url}\r");
    println!(
        "Paste the COMPLETE localhost redirect URL here if needed, then Enter. Input is hidden; Esc/Ctrl+C cancels.\r"
    );
    if listeners.is_empty() {
        println!(
            "Callback ports unavailable; paste the complete redirect or cancel and choose device authorization.\r"
        );
    }
    endpoint.failure.set(Some(OpenAiFailure {
        stage: "Browser authorization",
        title: "not completed",
        reason: "Browser consent or private callback/input did not complete. Start a fresh Browser login, or explicitly choose Device login when localhost is unavailable. No account/workspace-policy cause was established.",
        status: None,
        remedy: "Complete this Browser login, or explicitly choose Device if localhost is unavailable.",
        outcome: Outcome::NotAuthenticated,
    }));
    super::open_browser(&url);
    loop {
        let pasted = terminal.read()?;
        let mut code = match pasted {
            Some(value) => {
                let result = grant.callback(value.trim())?;
                if result.is_none() {
                    println!(
                        "Redirect not recognized; use this attempt's complete localhost URL.\r"
                    );
                }
                result
            }
            None => None,
        };
        for listener in &listeners {
            if code.is_some() {
                break;
            }
            match listener.accept() {
                Ok((stream, address)) if address.ip().is_loopback() => {
                    code = serve(stream, &mut grant, &mut || terminal.check())?
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return Err(Outcome::Failed),
            }
        }
        if let Some(code) = code {
            return endpoint
                .exchange(&code, &grant.verifier, &grant.redirect, &mut || {
                    terminal.check()
                })
                .await;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub(super) fn login(method: Method) -> Completion {
    let mut failure = None;
    let result = (|| {
        let _raw = RawInput::new().map_err(|_| Outcome::Failed)?;
        let input = crate::platform::input::Input::new().map_err(|_| Outcome::Failed)?;
        let mut terminal = Terminal {
            input,
            started: Instant::now(),
            paste: String::new(),
            submitted: None,
            timeout: if method == Method::OpenAiDevice {
                DEVICE_TIMEOUT
            } else {
                super::LOGIN_TIMEOUT
            },
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| Outcome::Failed)?;
        runtime.block_on(async {
            let endpoint = Endpoint::official()?;
            let result = if method == Method::OpenAiDevice {
                endpoint.device(&mut || terminal.check(), |code| {
                    println!("Open https://auth.openai.com/codex/device and enter: {code}\r");
                    println!("Use only a code from this login you initiated. Expires in 15 minutes; Esc/Ctrl+C cancels.\r");
                }).await
            } else { browser(&endpoint, &mut terminal).await };
            failure = endpoint.failure.get();
            terminal.check()?;
            result
        })
    })();
    match result {
        Ok(credential) => Completion {
            outcome: Outcome::Authenticated,
            devin_credential: None,
            openai_credential: Some(credential),
            openai_failure: None,
        },
        Err(outcome) => {
            let detail = match outcome {
                Outcome::Cancelled => OpenAiFailure {
                    stage: "Sign-in cancelled",
                    title: "cancelled by user",
                    reason: "Start a new Browser/Device login when ready; do not reuse the old redirect or code. Provider-issued grants are not revoked by cancellation.",
                    status: None,
                    remedy: "Start a new login only when ready; cancellation is not an authentication failure.",
                    outcome,
                },
                Outcome::Expired => failure.filter(|failure| failure.outcome == Outcome::Expired).unwrap_or(OpenAiFailure {
                    stage: "Sign-in expired",
                    title: "deadline reached",
                    reason: "Start a new login and complete it before its deadline; old redirects/codes cannot be reused.",
                    status: None,
                    remedy: "Start a new login and complete it before its deadline.",
                    outcome,
                }),
                _ => failure.unwrap_or(OpenAiFailure {
                    stage: "Local sign-in setup",
                    title: "private input unavailable",
                    reason: "Could not prepare or continue private terminal/callback input. Reopen login in an interactive terminal, or choose Device login if localhost is unavailable.",
                    status: None,
                    remedy: "Use an interactive terminal; explicitly choose Device if localhost is unavailable.",
                    outcome,
                }),
            };
            let mut completion = Completion::bare(outcome);
            completion.openai_failure = Some(detail);
            completion
        }
    }
}

#[cfg(test)]
mod tests;
