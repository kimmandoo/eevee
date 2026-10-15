use super::*;

#[test]
fn version_evidence_is_strict_and_never_retains_arbitrary_output() {
    assert_eq!(
        parse_version(0, b"codex-cli 0.152.1\n"),
        Some("0.152.1".into())
    );
    assert_eq!(
        parse_version(1, b"2.1.91 (Claude Code)\n"),
        Some("2.1.91".into())
    );
    assert_eq!(
        parse_version(0, b"codex-cli 0.147.0\nOPENAI_API_KEY=secret"),
        None
    );
    assert_eq!(parse_version(0, b"codex-cli \x1b[31m0.147.0"), None);
    assert_eq!(parse_version(0, b"codex-cli sk-secret"), None);
    assert_eq!(
        parse_version(ANT, b"ant version 1.40.0\n"),
        Some("1.40.0".into())
    );
    assert_eq!(parse_version(ANT, b"1.40.0\n"), None);
    assert_eq!(
        parse_version(DEVIN, b"devin 3000.11.3 (9c803229faa4)\n"),
        Some("3000.11.3".into())
    );
    for output in [
        b"3000.11.3".as_slice(),
        b"devin 3000.11.3 (secret)",
        b"devin 3000.11.3 (9c803229faa4)\nWINDSURF_API_KEY=secret",
        b"devin \x1b[31m3000.11.3 (9c803229faa4)",
    ] {
        assert_eq!(parse_version(DEVIN, output), None);
    }
}

#[test]
fn release_numbers_are_evidence_not_protocol_admission_pins() {
    for (index, output, version) in [
        (0, "codex-cli 0.162.1", "0.162.1"),
        (
            0,
            "codex-cli 0.163.0-beta.1+build.2",
            "0.163.0-beta.1+build.2",
        ),
        (2, "1.19.0", "1.19.0"),
        (3, "0.64.0", "0.64.0"),
        (DEVIN, "devin 3000.12.0 (9c803229faa4)", "3000.12.0"),
        (ANT, "ant version 1.41.0", "1.41.0"),
    ] {
        assert_eq!(
            parse_version(index, output.as_bytes()).as_deref(),
            Some(version)
        );
        let mut profile = discover(index, Path::new(".")).profile;
        let blockers = profile.blockers.clone();
        apply_version(index, &mut profile, version);
        assert_eq!(profile.blockers, blockers);
        assert!(profile.version.contains(version));
        assert!(profile.version.contains("remain unqualified"));
    }
}

#[test]
fn observed_version_is_immutable_for_one_reviewed_launch() {
    let observed = std::sync::OnceLock::new();
    bind_version(&observed, "0.162.1".into()).unwrap();
    bind_version(&observed, "0.162.1".into()).unwrap();
    assert!(bind_version(&observed, "0.163.0".into()).is_err());
    assert_eq!(observed.get().map(String::as_str), Some("0.162.1"));
    for output in ["codex-cli 0.163", "codex-cli 0.163.0\nextra"] {
        assert!(parse_version(0, output.as_bytes()).is_none());
    }
}

#[cfg(unix)]
#[test]
fn consented_newer_version_probe_binds_without_a_release_pin() {
    let fixture = Fixture::new();
    let snapshot = fixture.executable("printf 'codex-cli 0.162.1\\n'");
    let observed = std::sync::OnceLock::new();
    bind_version(
        &observed,
        probe(0, &snapshot, &AtomicBool::new(false)).unwrap(),
    )
    .unwrap();
    assert_eq!(observed.get().map(String::as_str), Some("0.162.1"));
}

#[test]
fn configuration_discloses_only_fixed_nonsecret_policy_values() {
    let text = nonsecret_declarations(Path::new("config.toml"), b"api_key = 'secret-canary'\nsandbox_mode = 'read-only'\n[profiles.other]\napproval_policy = 'never'\n");
    assert!(text.contains("sandbox_mode=read-only"));
    assert!(!text.contains("secret-canary"));
    assert!(!text.contains("approval_policy"));
    let text = nonsecret_declarations(
        Path::new("settings.json"),
        br#"{"apiKey":"secret-canary","hooks":{"command":"leak-canary"},"disableAllHooks":true}"#,
    );
    assert!(text.contains("disableAllHooks=true"));
    assert!(text.contains("hooks: declared"));
    assert!(!text.contains("canary"));
}

#[test]
fn config_content_is_not_fingerprinted_or_exposed() {
    let a = nonsecret_declarations(Path::new("config.toml"), b"api_key='first-secret'");
    let b = nonsecret_declarations(Path::new("config.toml"), b"api_key='second-secret'");
    assert_eq!(a, b);
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let path = env::temp_dir().join(format!(
            "ev-agent-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    #[cfg(unix)]
    fn executable(&self, script: &str) -> Snapshot {
        use std::os::unix::fs::PermissionsExt;
        let path = self.0.join("codex");
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let mut snapshot = discover(0, &self.0);
        snapshot.identity = Some(executable_identity(&path).unwrap());
        snapshot.cwd = Some(self.0.clone());
        snapshot
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn configuration_changes_invalidate_the_consent_snapshot() {
    let fixture = Fixture::new();
    let path = fixture.0.join("config.toml");
    let absent = source(path.clone(), true);
    fs::write(&path, "sandbox_mode='read-only'\n").unwrap();
    let initial = source(path.clone(), true);
    assert!(initial != absent);
    fs::write(&path, "sandbox_mode='danger-full-access'\n").unwrap();
    assert!(initial != source(path, true));
}

#[test]
fn devin_config_metadata_tracks_replacement_without_reading_secrets_or_credentials() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join(".devin")).unwrap();
    let path = fixture.0.join(".devin/config.local.json");
    let absent = configuration_sources(DEVIN, &fixture.0);
    fs::write(
        &path,
        b"{\"token\":\"private-canary\",\"hooks\":{\"command\":\"hook-canary\"}}",
    )
    .unwrap();
    let initial = configuration_sources(DEVIN, &fixture.0);
    assert!(initial != absent);
    let config = initial.iter().find(|entry| entry.path == path).unwrap();
    assert_eq!(
        config.declaration,
        "present; metadata only (no loader or plugin executed)"
    );
    assert!(
        initial
            .iter()
            .all(|entry| !entry.declaration.contains("canary"))
    );
    assert!(
        initial
            .iter()
            .all(|entry| !entry.path.ends_with("credentials.toml"))
    );
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::rename(&path, fixture.0.join("old-config")).unwrap();
    fs::write(
        &path,
        b"{\"token\":\"private-canary\",\"hooks\":{\"command\":\"hook-canary\"}}",
    )
    .unwrap();
    // Same size and modified time, as on a fast CI runner; on Windows NTFS
    // tunneling also restores the creation time, so only identity differs.
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert!(initial != configuration_sources(DEVIN, &fixture.0));
}

#[cfg(unix)]
#[test]
fn devin_version_probe_rejects_extra_output_without_exposing_it() {
    let fixture = Fixture::new();
    let stop = AtomicBool::new(false);
    let snapshot =
        fixture.executable("printf 'devin 3000.11.3 (9c803229faa4)\\nprivate-canary\\n'");
    let error = probe(DEVIN, &snapshot, &stop).unwrap_err();
    assert!(!error.contains("canary"));
}

#[test]
fn executable_replacement_invalidates_identity_even_with_identical_bytes() {
    let fixture = Fixture::new();
    let path = fixture.0.join("program");
    fs::write(&path, b"same executable bytes").unwrap();
    let original = executable_identity(&path).unwrap();
    fs::rename(&path, fixture.0.join("old-program")).unwrap();
    fs::write(&path, b"same executable bytes").unwrap();
    assert_ne!(original, executable_identity(&path).unwrap());
}

#[cfg(unix)]
#[test]
fn consented_probe_keeps_only_a_recognized_version() {
    let fixture = Fixture::new();
    let snapshot = fixture.executable("printf 'codex-cli 0.152.1\\n'");
    assert_eq!(
        probe(0, &snapshot, &AtomicBool::new(false)).unwrap(),
        "0.152.1"
    );
    let snapshot = fixture.executable("printf 'codex-cli 0.152.1\\nsecret-canary\\n'");
    let error = probe(0, &snapshot, &AtomicBool::new(false)).unwrap_err();
    assert!(error.contains("withheld"));
    assert!(!error.contains("secret-canary"));
}

#[cfg(unix)]
#[test]
fn excessive_version_output_is_bounded() {
    let fixture = Fixture::new();
    let snapshot = fixture.executable("while :; do printf '0123456789012345678901234567890123456789012345678901234567890123456789'; done");
    let start = Instant::now();
    let error = probe(0, &snapshot, &AtomicBool::new(false)).unwrap_err();
    assert!(error.contains("16 KiB"));
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[cfg(unix)]
#[test]
fn hung_probe_and_shutdown_have_bounded_owned_cleanup() {
    let fixture = Fixture::new();
    let snapshot = fixture.executable("while :; do :; done");
    let start = Instant::now();
    let error = probe(0, &snapshot, &AtomicBool::new(false)).unwrap_err();
    assert!(error.contains("timed out"));
    assert!(start.elapsed() < Duration::from_secs(5));
    let start = Instant::now();
    let error = probe(0, &snapshot, &AtomicBool::new(true)).unwrap_err();
    assert!(error.contains("cancelled"));
    assert!(start.elapsed() < Duration::from_secs(2));
}
