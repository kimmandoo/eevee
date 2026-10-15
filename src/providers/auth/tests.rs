use super::*;
#[test]
fn callback_free_default_covers_wsl_and_remote_without_changing_provider_contracts() {
    for key in [
        "WSL_DISTRO_NAME",
        "WSL_INTEROP",
        "SSH_CONNECTION",
        "SSH_CLIENT",
        "SSH_TTY",
    ] {
        assert!(callback_free_environment(|name| name == key, ""));
    }
    assert!(callback_free_environment(
        |_| false,
        "6.18.54.1-microsoft-standard-WSL2"
    ));
    assert!(callback_free_environment(|_| false, "4.4.0-Microsoft"));
    assert!(!callback_free_environment(|_| false, "6.18.1-generic"));
    assert!(!callback_free_environment(|_| false, ""));
    for profile in 2..4 {
        assert_eq!(default_method(profile), 0);
    }
    assert_eq!(
        methods(1),
        &[Method::ClaudeConsole, Method::ClaudeConsoleCode]
    );
    assert_eq!(methods(0), &[Method::CodexBrowser, Method::CodexDevice]);
    assert_eq!(Method::CodexBrowser.args(), ["login"]);
    assert_eq!(Method::CodexDevice.args(), ["login", "--device-auth"]);
    assert_eq!(Method::CodexStatus.args(), ["login", "status"]);
}

#[test]
fn status_requires_exact_provider_evidence_not_success_exit() {
    assert_eq!(parse_codex_status(Some(0), b""), Outcome::Unknown);
    assert_eq!(
        parse_codex_status(Some(0), b"Logged in using ChatGPT\n"),
        Outcome::Authenticated
    );
    assert_eq!(
        parse_codex_status(Some(1), b"Logged in using ChatGPT"),
        Outcome::Unknown
    );
    assert_eq!(
        parse_codex_status(Some(1), b"Not logged in\n"),
        Outcome::NotAuthenticated
    );
    assert_eq!(
        parse_codex_status(
            Some(1),
            b"WARNING: refusing temporary PATH helpers\nNot logged in\n"
        ),
        Outcome::NotAuthenticated
    );
    assert_eq!(
        parse_codex_status(Some(0), b"Not logged in\nLogged in using ChatGPT\n"),
        Outcome::Unknown
    );
    assert_eq!(
        parse_codex_status(Some(0), b"Logged in using ChatGPT\nsecret"),
        Outcome::Unknown
    );
    assert_eq!(
        parse_codex_status(Some(0), b"Logged in using an API key - sk-secret"),
        Outcome::Unknown
    );
}
#[test]
fn authentication_cwd_rejects_inserted_configuration_and_preserves_native_work() {
    let parent = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    let directory = AuthDirectory::create(&parent).unwrap();
    let path = directory.path.clone();
    assert!(directory.unchanged_empty());
    let artifact = path.join("native-work.txt");
    std::fs::write(&artifact, b"User work created in the provider").unwrap();
    assert!(!directory.unchanged_empty());
    drop(directory);
    assert_eq!(
        std::fs::read(&artifact).unwrap(),
        b"User work created in the provider"
    );
    std::fs::remove_file(artifact).unwrap();
    std::fs::remove_dir(path).unwrap();
}

#[test]
fn authentication_methods_cannot_cross_provider() {
    assert!(Plan::prepare(1, Method::CodexBrowser, Path::new(".")).is_err());
    assert!(Plan::prepare(0, Method::GeminiNative, Path::new(".")).is_err());
    assert!(Plan::prepare(0, Method::ClaudeConsole, Path::new(".")).is_err());
    assert!(Plan::prepare(3, Method::ClaudeConsoleStatus, Path::new(".")).is_err());
    assert!(Plan::prepare(0, Method::DevinBrowser, Path::new(".")).is_err());
    assert!(Plan::prepare(agent::DEVIN, Method::CodexBrowser, Path::new(".")).is_err());
    assert!(Plan::prepare(agent::ANT, Method::DevinBrowser, Path::new(".")).is_err());
    assert!(methods(1).iter().all(|method| method.is_claude()));
}

#[test]
fn claude_uses_console_oauth_profile_never_subscription_login() {
    let profile = crate::providers::claude::console_profile_name().unwrap();
    assert_eq!(
        Method::ClaudeConsole.args(),
        ["auth", "login", "--profile", profile.as_str()]
    );
    assert_eq!(
        Method::ClaudeConsoleCode.args(),
        [
            "auth",
            "login",
            "--no-browser",
            "--profile",
            profile.as_str()
        ]
    );
    assert_eq!(
        Method::ClaudeConsoleStatus.args(),
        ["auth", "status", "--profile", profile.as_str()]
    );
    assert_eq!(status_method(1), Some(Method::ClaudeConsoleStatus));
    let at = Some(std::time::SystemTime::UNIX_EPOCH);
    let old = Some((10, at));
    let fresh = Some((12, at));
    assert_eq!(
        console_outcome(Method::ClaudeConsole, &None, &fresh),
        Outcome::Authenticated
    );
    assert_eq!(
        console_outcome(Method::ClaudeConsoleCode, &old, &old),
        Outcome::Unknown
    );
    assert_eq!(
        console_outcome(Method::ClaudeConsole, &old, &None),
        Outcome::NotAuthenticated
    );
    assert_eq!(
        console_outcome(Method::ClaudeConsoleStatus, &old, &fresh),
        Outcome::Unknown
    );
}

#[cfg(unix)]
#[test]
fn native_status_success_is_not_authentication_evidence() {
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(
        native_exit_outcome(std::process::ExitStatus::from_raw(0)),
        Outcome::Unknown
    );
    assert_eq!(
        native_exit_outcome(std::process::ExitStatus::from_raw(1 << 8)),
        Outcome::Failed
    );
    assert_eq!(
        native_exit_outcome(std::process::ExitStatus::from_raw(130 << 8)),
        Outcome::Cancelled
    );
}

#[test]
fn devin_web_callback_input_parses_redirect_query_and_fragment_forms() {
    assert_eq!(
        devin_web::parse_callback_input("http://127.0.0.1:8765/callback?code=abc123&state=xyz789"),
        (Some("abc123".into()), Some("xyz789".into()))
    );
    assert_eq!(
        devin_web::parse_callback_input("?code=abc%20123&state=x"),
        (Some("abc 123".into()), Some("x".into()))
    );
    assert_eq!(
        devin_web::parse_callback_input("abc123#xyz789"),
        (Some("abc123".into()), Some("xyz789".into()))
    );
    assert_eq!(devin_web::parse_callback_input(""), (None, None));
}

#[test]
fn devin_web_token_plausibility_rejects_shell_and_whitespace_bytes() {
    assert!(!devin_web::plausible_token(""));
    assert!(!devin_web::plausible_token(&"x".repeat(15)));
    assert!(!devin_web::plausible_token(&"a".repeat(4097)));
    assert!(!devin_web::plausible_token(&format!(
        "{};rm",
        "a".repeat(16)
    )));
    assert!(!devin_web::plausible_token(&format!("{} ", "a".repeat(16))));
    assert!(!devin_web::plausible_token(&format!(
        "{}$x",
        "a".repeat(16)
    )));
    assert!(devin_web::plausible_token(&"a".repeat(32)));
    let jwt = format!("{}.{}.{}", "a".repeat(8), "b".repeat(8), "c".repeat(8));
    assert!(devin_web::plausible_token(&jwt));
}

#[test]
fn devin_web_jwt_expiry_reads_exp_minus_skew_and_rejects_malformed() {
    let segment = base64_simd::URL_SAFE
        .encode_to_string(br#"{"exp":2000}"#)
        .trim_end_matches('=')
        .to_string();
    let token = format!("header.{segment}.signature");
    assert_eq!(
        devin_web::jwt_expiry(&token),
        Some(2000 - devin_web::EXPIRY_SKEW)
    );
    assert_eq!(devin_web::jwt_expiry("opaque-token-no-dots"), None);
    assert_eq!(devin_web::jwt_expiry("a.b"), None);
    assert_eq!(devin_web::jwt_expiry("a.b.c.d"), None);
    assert_eq!(devin_web::jwt_expiry("a.@@@.c"), None);
}

#[test]
fn session_credential_redacts_debug_and_reports_expiry() {
    let mut credential = crate::providers::auth::SessionCredential {
        env: "WINDSURF_API_KEY",
        token: "opaque-session-secret".into(),
        expires_at: None,
    };
    assert!(!format!("{credential:?}").contains("opaque-session-secret"));
    assert!(!credential.expired());
    credential.expires_at = Some(0);
    assert!(credential.expired());
    credential.expires_at = Some(u64::MAX);
    assert!(!credential.expired());
    assert_eq!(credential.env, "WINDSURF_API_KEY");
    assert_eq!(credential.token(), "opaque-session-secret");
}
