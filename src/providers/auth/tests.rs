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
    assert!(Plan::prepare(0, Method::DevinWeb, Path::new(".")).is_err());
    assert!(
        Plan::prepare(
            crate::providers::Agent::Devin.index(),
            Method::CodexBrowser,
            Path::new(".")
        )
        .is_err()
    );
    assert!(Plan::prepare(agent::ANT, Method::DevinWeb, Path::new(".")).is_err());
    assert!(methods(1).iter().all(|method| method.is_claude()));
    assert!(Plan::prepare(0, Method::OpenAiBrowser, Path::new(".")).is_err());
    assert!(
        Plan::prepare(
            crate::providers::Agent::ChatGPT.index(),
            Method::CodexBrowser,
            Path::new(".")
        )
        .is_err()
    );
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
fn native_openai_methods_need_no_cli_or_external_trust() {
    let profile = crate::providers::Agent::ChatGPT.index();
    assert_eq!(
        methods(profile),
        &[Method::OpenAiBrowser, Method::OpenAiDevice]
    );
    assert_eq!(status_method(profile), None);
    for method in methods(profile) {
        let plan = Plan::prepare(profile, *method, Path::new(".")).unwrap();
        assert!(method.is_web());
        assert!(plan.trust.is_none() && plan.directory.is_none());
        assert!(method.args().is_empty());
        assert!(!format!("{plan:?}").contains("app_EMoam"));
    }
}
