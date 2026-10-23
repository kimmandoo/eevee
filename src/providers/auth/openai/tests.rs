use super::*;
use std::sync::{Arc, Mutex};

struct Reply {
    status: u16,
    body: String,
    headers: String,
    delay: Duration,
}
impl Reply {
    fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            body: body.to_string(),
            headers: String::new(),
            delay: Duration::ZERO,
        }
    }
}

fn fixture_request(stream: &mut TcpStream) -> String {
    // macOS inherits O_NONBLOCK from the accepting listener. Socket read
    // timeouts do not make a nonblocking socket wait for headers or body bytes.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    let mut expected = None;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "fixture request deadline expired");
        stream.set_read_timeout(Some(remaining)).unwrap();
        let mut buffer = [0u8; 4096];
        let count = match stream.read(&mut buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.expect("fixture request read failed"),
        };
        assert!(count > 0, "fixture request ended before its complete body");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= BODY_LIMIT);
        if expected.is_none()
            && let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n")
        {
            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
            let len = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .expect("fixture POST must declare its body length");
            expected = Some((end + 4).checked_add(len).unwrap());
            assert!(expected.unwrap() <= BODY_LIMIT);
        }
        if let Some(expected) = expected
            && bytes.len() >= expected
        {
            assert_eq!(bytes.len(), expected, "unexpected bytes after fixture body");
            return String::from_utf8(bytes).unwrap();
        }
    }
}

struct Fixture {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    worker: std::thread::JoinHandle<()>,
}
impl Fixture {
    fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let worker = std::thread::spawn(move || {
            for reply in replies {
                let start = Instant::now();
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                start.elapsed() < Duration::from_secs(5),
                                "fixture request missing"
                            );
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => panic!("fixture accept failed: {error}"),
                    }
                };
                let request = fixture_request(&mut stream);
                recorded.lock().unwrap().push(request);
                std::thread::sleep(reply.delay);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{}\r\n{}",
                    reply.status,
                    reply.body.len(),
                    reply.headers,
                    reply.body
                );
            }
        });
        Self {
            origin,
            requests,
            worker,
        }
    }
    fn finish(self) -> Vec<String> {
        self.worker.join().unwrap();
        Arc::try_unwrap(self.requests)
            .unwrap()
            .into_inner()
            .unwrap()
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn token(claims: Value) -> String {
    format!(
        "e30.{}.signature",
        base64_simd::URL_SAFE_NO_PAD.encode_to_string(claims.to_string().as_bytes())
    )
}
fn tokens() -> Value {
    serde_json::json!({
        "access_token": "fixture-access-secret", "token_type": "Bearer", "expires_in": 3600,
        "account_id": "fixture-account", "refresh_token": "discarded-refresh-secret"
    })
}
fn usercode() -> Reply {
    Reply::json(
        200,
        serde_json::json!({"device_auth_id":"device-secret","user_code":"ABCD-EFGH","interval":"0"}),
    )
}
fn device_grant() -> Reply {
    let verifier = "v".repeat(43);
    Reply::json(
        200,
        serde_json::json!({"authorization_code":"code-secret", "code_verifier":verifier, "code_challenge":challenge(&verifier)}),
    )
}

#[test]
fn native_fixture_reads_fragmented_nonblocking_request_by_content_length() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (ready, accepted) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut stream = listener.accept().unwrap().0;
        // Exercise macOS's inherited mode on every platform.
        stream.set_nonblocking(true).unwrap();
        ready.send(()).unwrap();
        fixture_request(&mut stream)
    });
    let mut stream = TcpStream::connect(address).unwrap();
    accepted.recv().unwrap();
    let body = "code=fixture-code&code_verifier=fixture-verifier";
    let headers = format!(
        "POST /oauth/token HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes()).unwrap();
    stream.write_all(&body.as_bytes()[..5]).unwrap();
    std::thread::sleep(Duration::from_millis(10));
    stream.write_all(&body.as_bytes()[5..]).unwrap();
    // Keep the connection open: framing must finish without a newline or EOF.
    assert_eq!(worker.join().unwrap(), format!("{headers}{body}"));
    drop(stream);
}

#[test]
fn native_browser_state_pkce_exact_return_and_single_use() {
    assert_eq!(
        challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    let mut grant = BrowserGrant::new(1455).unwrap();
    let other = BrowserGrant::new(1455).unwrap();
    assert_ne!(grant.state, other.state);
    assert_ne!(grant.verifier, other.verifier);
    assert!(pkce_valid(&grant.verifier));
    let url = reqwest::Url::parse(&grant.authorize_url()).unwrap();
    assert_eq!(url.origin().ascii_serialization(), ISSUER);
    let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(pairs["code_challenge"], challenge(&grant.verifier));
    assert_eq!(pairs["code_challenge_method"], "S256");
    assert_eq!(pairs["client_id"], CLIENT_ID);
    assert_eq!(pairs["redirect_uri"], grant.redirect);
    assert!(!grant.authorize_url().contains(&grant.verifier));
    let valid = format!("{}?code=code-secret&state={}", grant.redirect, grant.state);
    for invalid in [
        valid.replace("127.0.0.1", "foreign.invalid"),
        valid.replace(":1455", ":1457"),
        valid.replace("/auth/callback", "/other"),
        valid.replace(&grant.state, &other.state),
        format!("{valid}&state={}", grant.state),
        format!("{valid}&code=duplicate"),
        format!("{valid}#fragment"),
        format!("{valid}&error=denied"),
        "code-secret".into(),
        format!("code-secret#{}", grant.state),
    ] {
        assert_eq!(grant.callback(&invalid).unwrap(), None);
    }
    assert_eq!(
        grant.callback(&valid).unwrap().as_deref(),
        Some("code-secret")
    );
    assert_eq!(grant.callback(&valid).unwrap(), None);
    let mut denied = BrowserGrant::new(1455).unwrap();
    assert_eq!(
        denied.callback(&format!(
            "{}?error=access_denied&state={}",
            denied.redirect, denied.state
        )),
        Err(Outcome::NotAuthenticated)
    );
    assert!(denied.consumed);
}

#[test]
fn native_browser_loopback_callback_and_real_token_exchange_fixture() {
    let fixture = Fixture::new(vec![Reply::json(200, tokens())]);
    let runtime = runtime();
    runtime.block_on(async {
        let endpoint = Endpoint::fixture(fixture.origin.clone());
        let callback = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut grant = BrowserGrant::new(callback.local_addr().unwrap().port()).unwrap();
        let target = format!("/auth/callback?code=browser-code&state={}", grant.state);
        let address = callback.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            write!(stream, "GET {target} HTTP/1.1\r\nHost: {address}\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200"));
            assert!(!response.contains("browser-code"));
            assert!(response.contains("Cache-Control: no-store"));
        });
        let stream = callback.accept().unwrap().0;
        stream.set_nonblocking(true).unwrap();
        let code = serve(stream, &mut grant, &mut || Ok(())).unwrap().unwrap();
        client.join().unwrap();
        let result = endpoint
            .exchange(&code, &grant.verifier, &grant.redirect, &mut || Ok(()))
            .await
            .unwrap();
        assert_eq!(result.account_id.as_deref(), Some("fixture-account"));
        assert_eq!(result.access_token, "fixture-access-secret");
        assert!(result.expires_at > now().unwrap());
        assert_eq!(format!("{result:?}"), "OpenAiCredential { redacted }");
    });
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("POST /oauth/token "));
    assert!(requests[0].contains("grant_type=authorization_code"));
    assert!(requests[0].contains("code=browser-code"));
    assert!(requests[0].contains("code_verifier="));
    assert!(!requests[0].contains("fixture-access-secret"));
    assert!(!requests[0].contains("discarded-refresh-secret"));
}

#[test]
fn native_device_fixture_pending_success_and_exact_exchange_contract() {
    let fixture = Fixture::new(vec![
        usercode(),
        Reply::json(403, Value::Null),
        Reply::json(404, Value::Null),
        device_grant(),
        Reply::json(200, tokens()),
    ]);
    runtime().block_on(async {
        let endpoint = Endpoint::fixture(fixture.origin.clone());
        let mut displayed = Vec::new();
        let credential = endpoint
            .device(&mut || Ok(()), |code| displayed.push(code.to_owned()))
            .await
            .unwrap();
        assert_eq!(displayed, ["ABCD-EFGH"]);
        assert_eq!(credential.access_token, "fixture-access-secret");
    });
    let requests = fixture.finish();
    assert_eq!(requests.len(), 5);
    assert!(requests[0].starts_with("POST /api/accounts/deviceauth/usercode "));
    assert_eq!(
        serde_json::from_str::<Value>(requests[0].split("\r\n\r\n").nth(1).unwrap()).unwrap(),
        serde_json::json!({"client_id":CLIENT_ID})
    );
    for request in &requests[1..4] {
        assert!(request.starts_with("POST /api/accounts/deviceauth/token "));
        assert_eq!(
            serde_json::from_str::<Value>(request.split("\r\n\r\n").nth(1).unwrap()).unwrap(),
            serde_json::json!({"device_auth_id":"device-secret","user_code":"ABCD-EFGH"})
        );
    }
    assert!(requests[4].starts_with("POST /oauth/token "));
    let form = reqwest::Url::parse(&format!(
        "https://unused.invalid/?{}",
        requests[4].split("\r\n\r\n").nth(1).unwrap()
    ))
    .unwrap();
    let values: std::collections::HashMap<_, _> = form.query_pairs().into_owned().collect();
    assert_eq!(
        values["redirect_uri"],
        format!("{}/deviceauth/callback", fixture_origin(&requests[4]))
    );
    assert_eq!(values["code_verifier"], "v".repeat(43));
    assert_eq!(values["code"], "code-secret");
    assert!(
        requests
            .iter()
            .all(|request| !request.to_ascii_lowercase().contains("authorization:"))
    );
}
fn fixture_origin(request: &str) -> String {
    let host = request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("host").then_some(value.trim())
        })
        .unwrap();
    format!("http://{host}")
}

#[test]
fn native_device_denial_cancellation_expiry_and_bad_pkce_never_exchange() {
    for replies in [
        vec![Reply::json(400, Value::Null)],
        vec![usercode(), Reply::json(400, Value::Null)],
    ] {
        let fixture = Fixture::new(replies);
        let outcome = runtime().block_on(async {
            Endpoint::fixture(fixture.origin.clone())
                .device(&mut || Ok(()), |_| {})
                .await
                .unwrap_err()
        });
        assert_eq!(outcome, Outcome::NotAuthenticated);
        assert!(
            fixture
                .finish()
                .iter()
                .all(|request| !request.starts_with("POST /oauth/token "))
        );
    }
    for outcome in [Outcome::Cancelled, Outcome::Expired] {
        let fixture = Fixture::new(vec![usercode(), Reply::json(403, Value::Null)]);
        let requests = Arc::clone(&fixture.requests);
        let result = runtime().block_on(async {
            Endpoint::fixture(fixture.origin.clone())
                .device(
                    &mut || {
                        if requests.lock().unwrap().len() >= 2 {
                            Err(outcome)
                        } else {
                            Ok(())
                        }
                    },
                    |_| {},
                )
                .await
                .unwrap_err()
        });
        assert_eq!(result, outcome);
        drop(requests);
        assert_eq!(fixture.finish().len(), 2);
    }
    let mut invalid = device_grant();
    let mut value: Value = serde_json::from_str(&invalid.body).unwrap();
    value["code_challenge"] = "different".into();
    invalid.body = value.to_string();
    let fixture = Fixture::new(vec![usercode(), invalid]);
    let outcome = runtime().block_on(async {
        Endpoint::fixture(fixture.origin.clone())
            .device(&mut || Ok(()), |_| {})
            .await
            .unwrap_err()
    });
    assert_eq!(outcome, Outcome::Failed);
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn native_token_cancelled_or_expired_completion_is_discarded() {
    for outcome in [Outcome::Cancelled, Outcome::Expired] {
        let mut reply = Reply::json(200, tokens());
        reply.delay = Duration::from_millis(100);
        let fixture = Fixture::new(vec![reply]);
        let requests = Arc::clone(&fixture.requests);
        let result = runtime().block_on(async {
            Endpoint::fixture(fixture.origin.clone())
                .exchange(
                    "code",
                    &"v".repeat(43),
                    "http://127.0.0.1:1455/auth/callback",
                    &mut || {
                        if requests.lock().unwrap().is_empty() {
                            Ok(())
                        } else {
                            Err(outcome)
                        }
                    },
                )
                .await
                .unwrap_err()
        });
        assert_eq!(result, outcome);
        drop(requests);
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[test]
fn native_auth_redirects_never_forward_secrets_to_another_origin() {
    let sink = TcpListener::bind("127.0.0.1:0").unwrap();
    sink.set_nonblocking(true).unwrap();
    let mut redirect = Reply::json(307, Value::Null);
    redirect.headers = format!("Location: http://{}/stolen\r\n", sink.local_addr().unwrap());
    let fixture = Fixture::new(vec![redirect]);
    let result = runtime().block_on(async {
        Endpoint::fixture(fixture.origin.clone())
            .exchange(
                "private-code",
                &"v".repeat(43),
                "http://127.0.0.1:1455/auth/callback",
                &mut || Ok(()),
            )
            .await
            .unwrap_err()
    });
    assert_eq!(result, Outcome::Refused);
    assert_eq!(fixture.finish().len(), 1);
    assert_eq!(
        sink.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn native_auth_bodies_are_bounded_and_errors_never_include_secrets() {
    for body in [
        "x".repeat(BODY_LIMIT + 1),
        "malformed-private-secret".into(),
    ] {
        let fixture = Fixture::new(vec![Reply {
            status: 200,
            body,
            headers: String::new(),
            delay: Duration::ZERO,
        }]);
        let result = runtime().block_on(async {
            Endpoint::fixture(fixture.origin.clone())
                .device(&mut || Ok(()), |_| {})
                .await
                .unwrap_err()
        });
        assert_eq!(result, Outcome::Failed);
        assert!(!result.label().contains("private-secret"));
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[test]
fn native_credentials_validate_expiry_identity_and_redact_all_private_values() {
    let valid = token(
        serde_json::json!({"iss":ISSUER,"exp":1000,"https://api.openai.com/auth":{"chatgpt_account_id":"first","chatgpt_data_residency":"eu"}}),
    );
    let parsed = credential(serde_json::json!({"access_token":valid}), 100).unwrap();
    assert_eq!(parsed.expires_at, 940);
    assert_eq!(parsed.account_id.as_deref(), Some("first"));
    assert_eq!(parsed.residency.as_deref(), Some("eu"));
    assert!(!format!("{parsed:?}").contains("first"));
    for value in [
        serde_json::json!({"access_token":valid,"account_id":"second"}),
        serde_json::json!({"access_token":valid,"id_token":token(serde_json::json!({"exp":1000,"chatgpt_account_id":"second"}))}),
        serde_json::json!({"access_token":valid,"id_token":token(serde_json::json!({"exp":1000,"aud":"other-client"}))}),
        serde_json::json!({"access_token":token(serde_json::json!({"exp":1000,"iss":"https://other.invalid"}))}),
        serde_json::json!({"access_token":"opaque-without-expiry"}),
    ] {
        assert!(credential(value, 100).is_err());
    }
    assert_eq!(
        credential(serde_json::json!({"access_token":valid}), 1000).unwrap_err(),
        Outcome::Expired
    );
    let opaque = credential(serde_json::json!({"access_token":"opaque.dotted.value","expires_in":1000,"account_id":"chosen"}), 100).unwrap();
    assert_eq!(opaque.account_id.as_deref(), Some("chosen"));
    assert_eq!(opaque.expires_at, 1040);
}

#[test]
fn native_cancel_before_start_never_contacts_identity_or_displays_a_code() {
    let sink = TcpListener::bind("127.0.0.1:0").unwrap();
    sink.set_nonblocking(true).unwrap();
    let result = runtime().block_on(async {
        Endpoint::fixture(format!("http://{}", sink.local_addr().unwrap()))
            .device(&mut || Err(Outcome::Cancelled), |_| {
                panic!("cancelled login displayed a code")
            })
            .await
            .unwrap_err()
    });
    assert_eq!(result, Outcome::Cancelled);
    assert_eq!(
        sink.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
