use super::*;
use std::sync::atomic::AtomicU64;

static NEXT: AtomicU64 = AtomicU64::new(1);

struct Fixture(PathBuf);

impl Fixture {
    fn directory() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ev repo 中文 {} {}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }

    fn git() -> Self {
        let fixture = Self::directory();
        fixture.command(&["init", "--quiet"]);
        fixture.command(&["config", "user.name", "Repository Test"]);
        fixture.command(&["config", "user.email", "repo@example.invalid"]);
        fixture.command(&["config", "core.autocrlf", "false"]);
        fixture
    }

    fn command(&self, args: &[&str]) {
        let mut command = Command::new("git");
        command.current_dir(&self.0).env_clear();
        for key in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let output = command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        fs::write(self.0.join(name), bytes).unwrap();
    }

    fn commit(&self) {
        self.command(&["add", "--all"]);
        self.command(&[
            "-c",
            "core.hooksPath=",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ]);
    }

    fn state(&self, limits: Limits) -> State {
        State::new(self.0.clone(), limits, Arc::new(AtomicBool::new(false)))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn isolated_git_reads_empty_global_and_system_configuration() {
    let fixture = Fixture::git();
    let stopped = AtomicBool::new(false);
    let git = Git::new(&fixture.0, &stopped, Instant::now() + SCAN_TIMEOUT).unwrap();
    for scope in ["--global", "--system"] {
        let output = git.run(&["config", scope, "--list"], None, 1024).unwrap();
        assert!(output.success, "{scope}: {}", output.diagnostic());
        assert!(
            output.stdout.bytes.is_empty(),
            "{scope} configuration leaked"
        );
    }
    let local = git
        .run(&["config", "--local", "user.name"], None, 1024)
        .unwrap();
    assert!(local.success);
    assert_eq!(local.stdout.bytes, b"Repository Test\n");
}

fn report(event: Event) -> Arc<Report> {
    match event {
        Event::Updated(report) => report,
        other => panic!("{other:?}"),
    }
}

#[test]
fn dirty_baseline_external_changes_and_original_bytes_are_preserved() {
    let fixture = Fixture::git();
    fixture.write("tracked 中文.txt", b"committed\r\n");
    fixture.write("removed.txt", b"remove later\n");
    fixture.write("binary.bin", &[0, 255, 1]);
    fixture.commit();
    let dirty = b"pre-existing tracked\r\nno trailing newline";
    let untracked = b"pre-existing untracked\0\xff";
    fixture.write("tracked 中文.txt", dirty);
    fixture.write("new space 中文.txt", untracked);
    fs::remove_file(fixture.0.join("removed.txt")).unwrap();
    let index = fs::read(fixture.0.join(".git/index")).unwrap();
    let mut state = fixture.state(Limits::default());
    let initial = report(state.refresh());
    assert!(initial.initial && initial.current.complete && initial.current.stable);
    assert_eq!(initial.changes.len(), 3);
    assert!(
        initial
            .changes
            .iter()
            .all(|change| change.origin == ChangeOrigin::PreExisting)
    );
    assert_eq!(
        initial.current.files[Path::new("removed.txt")].state,
        FileState::Missing
    );
    assert!(matches!(
        initial.current.files[Path::new("new space 中文.txt")].state,
        FileState::Regular { binary: true, .. }
    ));
    assert!(
        initial
            .current
            .working_diff
            .text
            .contains("+pre-existing tracked\r\n")
    );
    assert_eq!(fs::read(fixture.0.join("tracked 中文.txt")).unwrap(), dirty);
    assert_eq!(
        fs::read(fixture.0.join("new space 中文.txt")).unwrap(),
        untracked
    );
    assert_eq!(fs::read(fixture.0.join(".git/index")).unwrap(), index);
    fixture.write("tracked 中文.txt", b"external replacement\r\n");
    fixture.write("external.txt", b"external new\n");
    let refreshed = report(state.refresh());
    assert!(!refreshed.initial);
    assert!(Arc::ptr_eq(&initial.baseline, &refreshed.baseline));
    let changes: BTreeMap<_, _> = refreshed
        .changes
        .iter()
        .map(|c| (c.path.as_path(), c))
        .collect();
    assert_eq!(
        changes[Path::new("tracked 中文.txt")].origin,
        ChangeOrigin::Unattributed
    );
    assert!(changes[Path::new("tracked 中文.txt")].was_dirty);
    assert_eq!(changes[Path::new("external.txt")].kind, ChangeKind::Added);
    assert_eq!(
        changes[Path::new("external.txt")].origin,
        ChangeOrigin::Unattributed
    );
    assert_eq!(
        changes[Path::new("new space 中文.txt")].origin,
        ChangeOrigin::PreExisting
    );
    assert_eq!(
        fs::read(fixture.0.join("new space 中文.txt")).unwrap(),
        untracked
    );
    assert_eq!(fs::read(fixture.0.join(".git/index")).unwrap(), index);
}

#[test]
fn worker_reports_non_git_without_creating_or_losing_task_data() {
    let fixture = Fixture::directory();
    fixture.write("task draft.txt", b"my local task\r\n");
    let worker = Worker::start(fixture.0.clone()).unwrap();
    let event = worker
        .events
        .as_ref()
        .unwrap()
        .recv_timeout(Duration::from_secs(15))
        .unwrap();
    assert!(matches!(event, Event::Unsupported { .. }));
    assert_eq!(
        fs::read(fixture.0.join("task draft.txt")).unwrap(),
        b"my local task\r\n"
    );
    assert!(!fixture.0.join(".git").exists());
}

#[test]
fn actual_worker_refresh_keeps_initial_baseline() {
    let fixture = Fixture::git();
    fixture.write("before.txt", b"before");
    let worker = Worker::start(fixture.0.clone()).unwrap();
    let initial = report(
        worker
            .events
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(15))
            .unwrap(),
    );
    fixture.write("before.txt", b"after");
    assert!(worker.refresh());
    let current = report(
        worker
            .events
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(15))
            .unwrap(),
    );
    assert!(Arc::ptr_eq(&initial.baseline, &current.baseline));
    assert_eq!(current.changes[0].origin, ChangeOrigin::Unattributed);
    assert!(worker.try_event().is_none());
}

#[test]
fn limits_preserve_partial_evidence_without_partial_hashes_or_false_deletions() {
    let fixture = Fixture::git();
    fixture.write("a.txt", b"old\n");
    fixture.commit();
    fixture.write("a.txt", &"long changed line 中文\n".as_bytes().repeat(100));
    fixture.write("large.bin", &vec![0; 4096]);
    let limits = Limits {
        file_bytes: 3000,
        diff_bytes: 180,
        inventory_bytes: 160,
        files: 2,
        ..Limits::default()
    };
    let mut state = fixture.state(limits);
    let initial = report(state.refresh());
    assert!(initial.current.working_diff.truncated);
    assert!(initial.current.working_diff.text.len() <= 180);
    assert!(matches!(
        initial.current.files[Path::new("large.bin")].state,
        FileState::Unavailable { .. }
    ));
    assert!(!initial.current.complete);
    for index in 0..100 {
        fixture.write(&format!("path {index:03}.txt"), b"extra");
    }
    let current = report(state.refresh());
    assert!(current.current.files.len() <= 2);
    assert!(!current.current.complete);
    assert!(
        current
            .current
            .warnings
            .iter()
            .any(|warning| warning.contains("inventory"))
    );
    assert!(
        !current
            .changes
            .iter()
            .any(|change| change.kind == ChangeKind::Deleted)
    );
}

#[test]
fn drain_and_git_inventory_keep_only_bounded_prefixes() {
    let bytes = vec![b'x'; 1024 * 1024];
    let captured = drain(io::Cursor::new(&bytes), 31);
    assert_eq!(captured.bytes, vec![b'x'; 31]);
    assert!(captured.truncated);
    assert!(captured.error.is_none());
    let fixture = Fixture::git();
    for index in 0..200 {
        fixture.write(&format!("file-{index:03}"), b"x");
    }
    let stopped = AtomicBool::new(false);
    let git = Git::new(&fixture.0, &stopped, Instant::now() + SCAN_TIMEOUT).unwrap();
    let output = git
        .required(&["ls-files", "--others", "-z"], None, 17)
        .unwrap();
    assert_eq!(output.bytes.len(), 17);
    assert!(output.truncated);
}

#[test]
fn staged_and_unborn_index_identities_are_not_lost() {
    let fixture = Fixture::git();
    fixture.write("staged.txt", b"in the index\n");
    fixture.command(&["add", "staged.txt"]);
    fixture.write("staged.txt", b"working copy\n");
    let mut state = fixture.state(Limits::default());
    let initial = report(state.refresh());
    assert!(initial.current.head.is_none());
    assert_eq!(
        initial.current.files[Path::new("staged.txt")].index.len(),
        1
    );
    assert!(initial.current.files[Path::new("staged.txt")].tracked);
    assert!(
        initial
            .current
            .working_diff
            .text
            .contains("index differs from HEAD")
    );
    fixture.command(&["add", "staged.txt"]);
    let current = report(state.refresh());
    assert_eq!(current.changes[0].origin, ChangeOrigin::Unattributed);
    assert_eq!(current.changes[0].kind, ChangeKind::MetadataChanged);
}

#[test]
fn failure_keeps_baseline_and_can_retry_without_resetting_it() {
    let fixture = Fixture::git();
    fixture.write("a.txt", b"before");
    let mut state = fixture.state(Limits::default());
    let initial = report(state.refresh());
    fs::rename(fixture.0.join(".git"), fixture.0.join("saved-git")).unwrap();
    assert!(matches!(
        state.refresh(),
        Event::Failed {
            operation: "refresh",
            ..
        }
    ));
    assert!(Arc::ptr_eq(
        state.baseline.as_ref().unwrap(),
        &initial.baseline
    ));
    fs::rename(fixture.0.join("saved-git"), fixture.0.join(".git")).unwrap();
    fixture.write("a.txt", b"after");
    let current = report(state.refresh());
    assert!(Arc::ptr_eq(&initial.baseline, &current.baseline));
    assert_eq!(current.changes[0].origin, ChangeOrigin::Unattributed);
}

#[test]
fn queue_backpressure_retains_pending_intention_and_does_not_block() {
    let (sender, incoming) = mpsc::sync_channel(COMMAND_CAPACITY);
    let (_outgoing, receiver) = mpsc::sync_channel(EVENT_CAPACITY);
    let worker = Worker {
        commands: Some(sender),
        events: Some(receiver),
        stopped: Arc::new(AtomicBool::new(false)),
    };
    assert!(worker.refresh());
    assert!(worker.refresh());
    assert!(!worker.refresh());
    incoming.recv().unwrap();
    assert!(worker.refresh());
}

#[test]
fn expired_git_deadline_fails_before_launch() {
    let fixture = Fixture::git();
    let stopped = AtomicBool::new(false);
    let git = Git::new(
        &fixture.0,
        &stopped,
        Instant::now() - Duration::from_millis(1),
    )
    .unwrap();
    let error = git.run(&["rev-parse", "HEAD"], None, 100).err().unwrap();
    assert!(error.contains("deadline"));
}

#[cfg(unix)]
#[test]
fn symlinks_are_hashed_as_links_and_outside_targets_never_read() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::git();
    let outside = Fixture::directory();
    outside.write("secret.txt", b"not source content");
    symlink(outside.0.join("secret.txt"), fixture.0.join("outside-link")).unwrap();
    let mut state = fixture.state(Limits::default());
    let initial = report(state.refresh());
    assert!(matches!(
        initial.current.files[Path::new("outside-link")].state,
        FileState::Symlink { .. }
    ));
    assert!(
        !initial
            .current
            .working_diff
            .text
            .contains("not source content")
    );
    fs::create_dir(fixture.0.join("nested")).unwrap();
    fixture.write("nested/file.txt", b"tracked\n");
    fixture.commit();
    fs::remove_file(fixture.0.join("nested/file.txt")).unwrap();
    fs::remove_dir(fixture.0.join("nested")).unwrap();
    symlink(&outside.0, fixture.0.join("nested")).unwrap();
    let current = report(state.refresh());
    assert!(matches!(
        current.current.files[Path::new("nested/file.txt")].state,
        FileState::Unavailable { .. }
    ));
    assert_eq!(
        fs::read(outside.0.join("secret.txt")).unwrap(),
        b"not source content"
    );
}

#[cfg(unix)]
#[test]
fn configured_helpers_and_filters_are_never_executed() {
    let fixture = Fixture::git();
    fixture.write("file.txt", b"head\n");
    fixture.commit();
    fixture.write(".gitattributes", b"*.txt filter=evil diff=evil\n");
    fixture.write("file.txt", b"working\n");
    let executable = "touch helper-was-run; cat";
    fixture.command(&["config", "core.fsmonitor", executable]);
    fixture.command(&["config", "core.pager", executable]);
    fixture.command(&["config", "diff.external", executable]);
    fixture.command(&["config", "diff.evil.textconv", executable]);
    fixture.command(&["config", "filter.evil.clean", executable]);
    fixture.command(&["config", "filter.evil.process", executable]);
    fixture.command(&["config", "filter.evil.required", "true"]);
    let mut state = fixture.state(Limits::default());
    let initial = report(state.refresh());
    assert!(initial.current.working_diff.text.contains("+working"));
    assert!(!fixture.0.join("helper-was-run").exists());
}

#[cfg(unix)]
#[test]
fn canonical_directory_aliases_share_repository_identity() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::git();
    let aliases = Fixture::directory();
    fixture.write("file.txt", b"bytes\r\n");
    symlink(&fixture.0, aliases.0.join("project alias")).unwrap();
    let mut direct = fixture.state(Limits::default());
    let mut alias = State::new(
        aliases.0.join("project alias"),
        Limits::default(),
        Arc::new(AtomicBool::new(false)),
    );
    let direct = report(direct.refresh());
    let alias = report(alias.refresh());
    assert_eq!(direct.repository, alias.repository);
    assert_eq!(direct.current.files, alias.current.files);
    let expected: [u8; 32] = Sha256::digest(b"bytes\r\n").into();
    assert!(matches!(
        direct.current.files[Path::new("file.txt")].state,
        FileState::Regular { sha256, .. } if sha256 == expected
    ));
}

#[test]
fn byte_only_external_edits_remain_unattributed_after_reverting_to_head() {
    let fixture = Fixture::git();
    fixture.write("line.txt", b"line\n");
    fixture.commit();
    fixture.write("line.txt", b"line\r\n");
    let mut state = fixture.state(Limits::default());
    let initial = report(state.refresh());
    assert_eq!(
        initial.current.files[Path::new("line.txt")].dirty,
        Some(true)
    );
    fixture.write("line.txt", b"line\n");
    let current = report(state.refresh());
    assert_eq!(
        current.current.files[Path::new("line.txt")].dirty,
        Some(false)
    );
    assert_eq!(current.changes.len(), 1);
    assert_eq!(current.changes[0].origin, ChangeOrigin::Unattributed);
    assert!(current.changes[0].was_dirty);
    assert!(Arc::ptr_eq(&initial.baseline, &current.baseline));
}
