use super::*;
use std::io::Seek;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("ev runner 中文 {}", unique_id()));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
    fn spool(&self) -> File {
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(self.0.join(format!("{}.output", unique_id())))
            .unwrap()
    }
    fn fixture(&self, mode: &str) {
        fs::write(self.0.join("runner-fixture"), mode).unwrap();
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fixture_spec(extra: &[&str]) -> CommandSpec {
    let mut args = vec![
        "--exact".into(),
        "workspace::runner::tests::native_fixture".into(),
        "--nocapture".into(),
        "--ignored".into(),
        "--".into(),
    ];
    args.extend(extra.iter().map(|arg| (*arg).to_owned()));
    CommandSpec::Direct {
        program: std::env::current_exe().unwrap().to_str().unwrap().into(),
        args,
    }
}
fn alias_fixture_spec(temp: &Temp, image: &Path, extra: &[&str]) -> CommandSpec {
    let name = if cfg!(windows) {
        "requested multicall 中文.exe"
    } else {
        "requested multicall 中文"
    };
    let alias = temp.0.join(name);
    #[cfg(unix)]
    std::os::unix::fs::symlink(image, &alias).unwrap();
    #[cfg(windows)]
    fs::copy(image, &alias).unwrap();
    let mut spec = fixture_spec(extra);
    if let CommandSpec::Direct { program, .. } = &mut spec {
        *program = Path::new(".").join(name).to_str().unwrap().into();
    }
    spec
}
fn prepared(temp: &Temp, spec: CommandSpec) -> Prepared {
    prepare(
        unique_id(),
        1,
        spec,
        temp.0.clone(),
        InputSnapshot::default(),
    )
    .unwrap()
}
fn execute(
    temp: &Temp,
    prepared: Prepared,
    capacity: u64,
    cancel_when: Option<&str>,
) -> (RunRecord, String, VecDeque<Event>) {
    let process = native::Process::spawn(&prepared).unwrap();
    let file = temp.spool();
    let mut reader = file.try_clone().unwrap();
    let events = Arc::new(Mutex::new(VecDeque::new()));
    let run_events = Arc::clone(&events);
    let (live, _undrained) = mpsc::sync_channel(2);
    let gate = Arc::new(Mutex::new(Gate {
        active: Some(prepared.run_id.clone()),
        ..Gate::default()
    }));
    let run_gate = Arc::clone(&gate);
    let thread = thread::spawn(move || {
        run(
            process,
            RunRecord::awaiting(&prepared),
            (file, capacity),
            &run_events,
            &live,
            &run_gate,
            &AtomicBool::new(false),
        )
    });
    if let Some(marker) = cancel_when {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !temp.0.join(marker).exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            temp.0.join(marker).exists(),
            "fixture never established its descendants"
        );
        gate.lock().cancelled = true;
    }
    // The tested run itself is bounded after cancellation/root exit; this harness does not
    // run the renderer or depend on global shell profiles/environment mutation.
    thread.join().unwrap();
    reader.rewind().unwrap();
    let mut output = String::new();
    reader.read_to_string(&mut output).unwrap();
    let events = std::mem::take(&mut *events.lock());
    let record = events
        .iter()
        .find_map(|event| match event {
            Event::Finished(record) => Some(record.clone()),
            _ => None,
        })
        .unwrap();
    (record, output, events)
}

#[test]
fn piped_native_transport_roundtrips_and_backpressure_does_not_block() {
    let temp = Temp::new();
    temp.fixture("stdin-echo");
    let mut process = native::Process::spawn_piped(&prepared(&temp, fixture_spec(&[]))).unwrap();
    let input = "literal 한국어 input\n".as_bytes();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut written = 0;
    while written < input.len() {
        match process.write_stdin(&input[written..]) {
            Ok(n) => {
                assert_ne!(n, 0);
                written += n;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(error) => panic!("{error}"),
        }
        assert!(Instant::now() < deadline);
    }
    process.close_stdin();
    let mut output = Vec::new();
    let mut bytes = [0; 4096];
    loop {
        match process.read(0, &mut bytes) {
            Ok(0) => break,
            Ok(n) => output.extend_from_slice(&bytes[..n]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(error) => panic!("{error}"),
        }
        assert!(Instant::now() < deadline);
    }
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("EEVEE_STDIN=literal 한국어 input")
    );
    drop(process);
    temp.fixture("stdin-stall");
    let mut process = native::Process::spawn_piped(&prepared(&temp, fixture_spec(&[]))).unwrap();
    let bytes = [b'x'; 8192];
    let start = Instant::now();
    let mut blocked = false;
    for _ in 0..256 {
        match process.write_stdin(&bytes) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                blocked = true;
                break;
            }
            Err(error) => panic!("{error}"),
        }
    }
    assert!(blocked, "bounded OS pipe must expose backpressure");
    assert!(start.elapsed() < Duration::from_secs(2));
    process.terminate(true).unwrap();
}

#[test]
#[ignore = "subprocess fixture, invoked explicitly by isolated runner tests"]
fn native_fixture() {
    let mode = std::env::var("EEVEE_RUNNER_FIXTURE")
        .unwrap_or_else(|_| fs::read_to_string("runner-fixture").unwrap());
    match mode.as_str() {
        "literal" => {
            let args: Vec<_> = std::env::args()
                .skip_while(|arg| arg != "--")
                .skip(1)
                .collect();
            println!("EEVEE_ARGV={}", serde_json::to_string(&args).unwrap());
            let mut input = String::new();
            assert_eq!(
                io::stdin().read_to_string(&mut input).unwrap(),
                0,
                "stdin must be null"
            );
        }
        "multicall" => {
            let args: Vec<_> = std::env::args().collect();
            assert_eq!(
                args[0],
                fs::read_to_string("expected-argv0").unwrap(),
                "multicall dispatch must use the requested alias, not the canonical image"
            );
            fs::write("multicall.started", serde_json::to_vec(&args).unwrap()).unwrap();
            println!(
                "EEVEE_MULTICALL_ARGV={}",
                serde_json::to_string(&args).unwrap()
            );
        }
        "stdin-echo" => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).unwrap();
            println!("EEVEE_STDIN={input}");
        }
        "stdin-stall" => thread::sleep(Duration::from_secs(30)),
        "fail" => {
            println!("partial evidence before failure");
            std::process::exit(23);
        }
        "stream" => {
            println!(
                "\u{1b}]52;c;do-not-render\u{7}\u{1b}[31mvisible\u{1b}[0m password=opaque-value"
            );
            for n in 0..20000 {
                println!("line {n}: abcdefghijklmnopqrstuvwxyz");
            }
            eprintln!("stderr remains drained");
        }
        "oversize" => {
            println!("retained before long line");
            print!("{}", "x".repeat(LINE_LIMIT + 100));
            println!("\n-----BEGIN PRIVATE KEY-----\nopaque-value\n-----END PRIVATE KEY-----");
            eprintln!("independent stderr retained");
        }
        "tree" | "stubborn" | "root-exits" | "child" | "grandchild" | "escape-root" | "escape" => {
            #[cfg(unix)]
            unsafe {
                if mode == "stubborn" {
                    libc::signal(libc::SIGTERM, libc::SIG_IGN);
                }
                if mode == "escape" {
                    assert!(libc::setsid() >= 0);
                }
            }
            let name = match mode.as_str() {
                "child" => "child",
                "grandchild" => "grandchild",
                "escape" => "escape",
                _ => "root",
            };
            fs::write(format!("{name}.pid"), std::process::id().to_string()).unwrap();
            // Beat before spawning so a descendant's marker implies every ancestor's beat.
            fs::write(format!("{name}.beat"), "start").unwrap();
            println!("{name} partial output start");
            let mut child = if mode != "grandchild" && mode != "escape" {
                let next = if mode == "child" {
                    "grandchild"
                } else if mode == "escape-root" {
                    "escape"
                } else {
                    "child"
                };
                Some(
                    std::process::Command::new(std::env::current_exe().unwrap())
                        .args([
                            "--exact",
                            "workspace::runner::tests::native_fixture",
                            "--nocapture",
                            "--ignored",
                        ])
                        .env("EEVEE_RUNNER_FIXTURE", next)
                        .spawn()
                        .unwrap(),
                )
            } else {
                None
            };
            if mode == "root-exits" || mode == "escape-root" {
                let marker = if mode == "escape-root" {
                    "escape.beat"
                } else {
                    "grandchild.beat"
                };
                let deadline = Instant::now() + Duration::from_secs(10);
                while !Path::new(marker).exists() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(10));
                }
                assert!(Path::new(marker).exists());
                // Deliberately return without waiting: native ownership, not this fixture,
                // must tear down descendants after a normally completed root.
                return;
            }
            for n in 0..1000 {
                fs::write(format!("{name}.beat"), n.to_string()).unwrap();
                println!("{name} partial output {n}");
                thread::sleep(Duration::from_millis(20));
            }
            if let Some(child) = child.as_mut() {
                let _ = child.wait();
            }
            panic!("owned fixture outlived cancellation");
        }
        _ => panic!("unknown fixture mode"),
    }
}

#[test]
fn direct_arguments_are_literal_and_cwd_is_unicode() {
    let temp = Temp::new();
    temp.fixture("literal");
    let args = [
        "space value",
        "中文 路径",
        "$(touch forbidden)",
        "a;b|c&d",
        "quote\"slash\\",
        "",
        "--literal-switch",
    ];
    let prepared = prepared(&temp, fixture_spec(&args));
    assert_eq!(prepared.cwd, temp.0);
    let (record, output, events) = execute(&temp, prepared, 128 * 1024, None);
    assert_eq!(record.state, RunState::Succeeded, "{record:?}");
    assert_eq!(record.tree, TreeOutcome::OwnedTreeExited);
    assert!(
        output.contains(&format!(
            "EEVEE_ARGV={}",
            serde_json::to_string(&args).unwrap()
        )),
        "{output}"
    );
    assert!(!temp.0.join("forbidden").exists());
    assert!(matches!(events.front(), Some(Event::Started(_))));
    assert!(record.started_at_ms <= record.ended_at_ms);
}

#[test]
fn requested_multicall_argv0_reaches_the_frozen_image_with_literal_arguments() {
    let temp = Temp::new();
    temp.fixture("multicall");
    let args = [
        "space value",
        "中文 路径",
        "$(touch forbidden)",
        "a;b|c&d",
        "quote\"slash\\",
        "trailing\\",
        "",
        "--literal-switch",
    ];
    let spec = alias_fixture_spec(&temp, &std::env::current_exe().unwrap(), &args);
    fs::write(temp.0.join("expected-argv0"), program(&spec)).unwrap();
    let expected: Vec<_> = std::iter::once(program(&spec))
        .chain(arguments(&spec))
        .map(str::to_owned)
        .collect();
    let p = prepared(&temp, spec);
    let frozen = p.executable.clone();
    #[cfg(unix)]
    {
        assert_eq!(
            frozen.path,
            fs::canonicalize(std::env::current_exe().unwrap()).unwrap()
        );
        // Retargeting the requested name cannot select a different image at dispatch.
        let alias = temp.0.join(program(&p.spec));
        fs::remove_file(&alias).unwrap();
        fs::write(alias, "not the approved executable").unwrap();
    }
    let (record, output, _) = execute(&temp, p, 128 * 1024, None);
    assert_eq!(record.state, RunState::Succeeded, "{record:?}\n{output}");
    assert_eq!(record.tree, TreeOutcome::OwnedTreeExited);
    assert_eq!(record.prepared.executable, frozen);
    let actual: Vec<String> =
        serde_json::from_slice(&fs::read(temp.0.join("multicall.started")).unwrap()).unwrap();
    assert_eq!(actual, expected);
    assert!(!temp.0.join("forbidden").exists());
}

#[cfg(unix)]
#[test]
fn explicit_shell_preserves_its_requested_argv0() {
    let temp = Temp::new();
    let name = "./reviewed shell 中文";
    std::os::unix::fs::symlink("/bin/sh", temp.0.join(name)).unwrap();
    let spec = CommandSpec::Shell {
        executable: name.into(),
        dialect: ShellDialect::Posix,
        text: "printf 'EEVEE_SHELL_ARGV0=%s\\n' \"$0\"".into(),
    };
    let (record, output, _) = execute(&temp, prepared(&temp, spec), 8192, None);
    assert_eq!(record.state, RunState::Succeeded, "{record:?}");
    assert!(
        output.contains(&format!("EEVEE_SHELL_ARGV0={name}")),
        "{output}"
    );
}

#[test]
fn explicit_shell_uses_reviewed_dialect_and_failed_exit_keeps_output() {
    let temp = Temp::new();
    #[cfg(unix)]
    let spec = CommandSpec::Shell {
        executable: "/bin/sh".into(),
        dialect: ShellDialect::Posix,
        text: "printf 'explicit shell evidence\\n'; exit 23".into(),
    };
    #[cfg(windows)]
    let spec = CommandSpec::Shell {
        executable: std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into()),
        dialect: ShellDialect::Cmd,
        text: "echo explicit shell evidence&exit /b 23".into(),
    };
    let (record, output, _) = execute(&temp, prepared(&temp, spec), 8192, None);
    assert_eq!(record.state, RunState::Failed, "{record:?}");
    assert_eq!(record.exit_code, Some(23));
    assert!(output.contains("explicit shell evidence"));
}

#[test]
fn failed_direct_command_is_not_retried_and_keeps_partial_evidence() {
    let temp = Temp::new();
    temp.fixture("fail");
    let (record, output, events) = execute(&temp, prepared(&temp, fixture_spec(&[])), 8192, None);
    assert_eq!(record.exit_code, Some(23));
    assert_eq!(record.state, RunState::Failed);
    assert!(output.contains("partial evidence before failure"));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Started(_)))
            .count(),
        1
    );
}

#[test]
fn full_live_queue_and_capture_quota_keep_draining() {
    let temp = Temp::new();
    temp.fixture("stream");
    let (record, output, _) = execute(&temp, prepared(&temp, fixture_spec(&[])), 2048, None);
    assert_eq!(record.state, RunState::Succeeded, "{record:?}");
    assert!(record.truncated && record.discarded_bytes > 100_000);
    assert!(record.dropped_live_bytes > 0);
    assert_eq!(record.captured_bytes, output.len() as u64);
    assert!(output.len() <= 2048);
    assert!(output.contains("visible"));
    assert!(
        !output.contains('\u{1b}')
            && !output.contains("do-not-render")
            && !output.contains("opaque-value")
    );
}

#[test]
fn oversized_lines_cannot_leak_a_later_private_key_continuation() {
    let temp = Temp::new();
    temp.fixture("oversize");
    let (record, output, _) = execute(&temp, prepared(&temp, fixture_spec(&[])), 128 * 1024, None);
    assert!(record.truncated);
    assert!(output.contains("retained before long line"));
    assert!(output.contains("independent stderr retained"));
    assert!(!output.contains("opaque-value") && !output.contains("xxxxx"));
}

fn assert_heartbeats_stopped(temp: &Temp) {
    let before: Vec<_> = ["child.beat", "grandchild.beat"]
        .iter()
        .map(|name| fs::read(temp.0.join(name)).unwrap())
        .collect();
    thread::sleep(Duration::from_millis(150));
    let after: Vec<_> = ["child.beat", "grandchild.beat"]
        .iter()
        .map(|name| fs::read(temp.0.join(name)).unwrap())
        .collect();
    assert_eq!(before, after, "owned descendants survived teardown");
}

#[test]
fn cancelling_owns_child_and_grandchild_and_retains_partial_output() {
    let temp = Temp::new();
    temp.fixture("tree");
    let (record, output, events) = execute(
        &temp,
        prepared(&temp, fixture_spec(&[])),
        128 * 1024,
        Some("grandchild.beat"),
    );
    assert!(record.cancelled);
    assert!(matches!(
        record.state,
        RunState::Cancelled | RunState::OutcomeUnknown
    ));
    if record.state == RunState::OutcomeUnknown {
        assert_eq!(record.tree, TreeOutcome::OwnershipUnknown);
    }
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Cancelling(_)))
    );
    assert!(output.contains("partial output"));
    assert_heartbeats_stopped(&temp);
}

#[test]
fn normal_root_exit_also_tears_down_owned_descendants() {
    let temp = Temp::new();
    temp.fixture("root-exits");
    let (record, _, _) = execute(&temp, prepared(&temp, fixture_spec(&[])), 128 * 1024, None);
    assert_eq!(record.exit_code, Some(0));
    assert!(!record.cancelled);
    assert!(matches!(
        record.state,
        RunState::Succeeded | RunState::OutcomeUnknown
    ));
    assert_heartbeats_stopped(&temp);
}

#[test]
fn split_credentials_private_keys_and_escape_sequences_are_redacted_before_spooling() {
    let temp = Temp::new();
    let p = prepared(&temp, fixture_spec(&[]));
    let file = temp.spool();
    let mut reader = file.try_clone().unwrap();
    let (live, _output) = mpsc::sync_channel(1);
    let events = Mutex::new(VecDeque::new());
    let mut capture = Capture {
        file,
        capacity: 128 * 1024,
        record: RunRecord::awaiting(&p),
        live: &live,
        events: &events,
    };
    let bytes = b"visible \x1b[31mpassword=opaque-value\x1b[0m\n-----BEGIN RSA PRIVATE KEY-----\nopaque-key-body\n-----END RSA PRIVATE KEY-----\napi_key=\"opaque\ncontinuation\"\nafter\n";
    let mut sanitizer = Sanitizer::default();
    for byte in bytes {
        sanitizer.push(&[*byte], &mut capture, OutputStream::Stdout);
    }
    sanitizer.finish(&mut capture, OutputStream::Stdout);
    reader.rewind().unwrap();
    let mut output = String::new();
    reader.read_to_string(&mut output).unwrap();
    assert!(output.contains("visible") && output.contains("after"));
    assert!(
        !output.contains("opaque")
            && !output.contains("continuation")
            && !output.contains('\u{1b}')
    );
    assert!(output.contains("[REDACTED]"));
}

#[test]
fn denial_and_invalidating_preparation_never_execute() {
    let temp = Temp::new();
    temp.fixture("literal");
    let worker = Worker::start().unwrap();
    let first = worker
        .prepare(
            1,
            fixture_spec(&[]),
            temp.0.clone(),
            InputSnapshot::default(),
        )
        .unwrap();
    worker.invalidate();
    worker.invalidate();
    let second = worker
        .prepare(
            1,
            fixture_spec(&[]),
            temp.0.clone(),
            InputSnapshot::default(),
        )
        .unwrap();
    assert_ne!(first, second);
    worker.invalidate();
    assert!(!worker.is_active());
    assert!(!temp.0.join("root.pid").exists());
}

#[test]
fn lifecycle_backpressure_never_evicts_owned_run_metadata() {
    let temp = Temp::new();
    let p = prepared(&temp, fixture_spec(&[]));
    let events = Mutex::new(VecDeque::new());
    for _ in 0..100 {
        publish(&events, Event::Prepared(p.clone()));
    }
    publish(&events, Event::Started(RunRecord::awaiting(&p)));
    publish(&events, Event::Cancelling(RunRecord::awaiting(&p)));
    publish(&events, Event::Finished(RunRecord::awaiting(&p)));
    let events = events.lock();
    assert_eq!(events.len(), EVENT_CAPACITY);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                Event::Started(_) | Event::Cancelling(_) | Event::Finished(_)
            ))
            .count(),
        3
    );
}

#[test]
fn stored_command_redaction_does_not_change_the_executed_argv() {
    let temp = Temp::new();
    let p = prepared(
        &temp,
        fixture_spec(&["--password", "opaque-value", "--token=other-value"]),
    );
    let mut record = RunRecord::awaiting(&p);
    record.redact_for_storage();
    let stored = serde_json::to_string(&record).unwrap();
    assert!(!stored.contains("opaque-value") && !stored.contains("other-value"));
    assert!(serde_json::to_string(&p).unwrap().contains("opaque-value"));
}

fn store_event(store: &crate::store::Worker) -> crate::store::Event {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(event) = store.try_event() {
            return event;
        }
        assert!(Instant::now() < deadline, "storage event timed out");
        thread::sleep(Duration::from_millis(10));
    }
}
fn committed_permit(
    temp: &Temp,
    p: &Prepared,
) -> (crate::store::Worker, crate::store::CommittedRunPermit) {
    use crate::store::{
        Event as StoreEvent, Policy, RunOperation, RunRequest, RunResponse, Snapshot,
    };
    let project = p.cwd.to_str().unwrap().to_owned();
    let store =
        crate::store::Worker::start(temp.0.join("storage"), project.clone(), Policy::default())
            .unwrap();
    assert!(matches!(store_event(&store), StoreEvent::Loaded(_)));
    let mut snapshot = Snapshot::empty(project);
    snapshot.goal = "isolated runner admission test".into();
    snapshot.revision = 1;
    assert!(store.try_save(&mut Some(snapshot)));
    match store_event(&store) {
        StoreEvent::Saved(snapshot) => assert_eq!(snapshot.id, p.task_id),
        other => panic!("task save failed: {other:?}"),
    }
    let mut request = Some(RunRequest {
        task_id: p.task_id,
        operation: RunOperation::Reserve {
            prepared: p.clone(),
            capacity: 8192,
        },
    });
    assert!(store.try_run(&mut request));
    match store_event(&store) {
        StoreEvent::Run {
            result: Ok(response),
            ..
        } => match *response {
            RunResponse::Reserved(permit) => (store, permit),
            other => panic!("expected committed reservation: {other:?}"),
        },
        other => panic!("committed reservation failed: {other:?}"),
    }
}

#[test]
fn stale_double_and_expired_grants_fail_at_native_dispatch_with_real_reservation() {
    let temp = Temp::new();
    temp.fixture("literal");
    let p = prepared(&temp, fixture_spec(&[]));
    let (_store, permit) = committed_permit(&temp, &p);
    let gate = Mutex::new(Gate::default());
    let stopped = AtomicBool::new(false);
    assert!(
        admit(&p, &permit, &gate, &stopped).is_err(),
        "invalidated request dispatched"
    );
    gate.lock().pending = Some(p.request_id.clone());
    let mut expired = p.clone();
    expired.expires_at_ms = 0;
    assert!(
        admit(&expired, &permit, &gate, &stopped).is_err(),
        "expired request dispatched"
    );
    gate.lock().callback_cancelled = Some(Arc::new(AtomicBool::new(true)));
    assert!(
        admit(&p, &permit, &gate, &stopped).is_err(),
        "cancelled exact callback dispatched"
    );
    assert!(gate.lock().active.is_none());
    gate.lock().callback_cancelled = None;
    let process = admit(&p, &permit, &gate, &stopped).unwrap();
    assert!(
        admit(&p, &permit, &gate, &stopped).is_err(),
        "one-shot grant dispatched twice"
    );
    drop(process);
}

#[test]
fn delayed_committed_reservation_does_not_revive_a_denied_request() {
    let temp = Temp::new();
    temp.fixture("tree");
    let worker = Worker::start().unwrap();
    let id = worker
        .prepare(
            1,
            fixture_spec(&[]),
            temp.0.clone(),
            InputSnapshot::default(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let p = loop {
        if let Some(Event::Prepared(p)) = worker.try_event() {
            break p;
        }
        assert!(Instant::now() < deadline, "prepare timed out");
        thread::sleep(Duration::from_millis(10));
    };
    let (_store, permit) = committed_permit(&temp, &p);
    worker.invalidate();
    assert!(worker.dispatch(&id, permit).is_err());
    assert!(!worker.is_active());
    assert!(
        !temp.0.join("root.pid").exists(),
        "denied request made a side effect"
    );
}

#[test]
fn callback_rechecks_original_project_sources_but_plain_coverage_stays_unknown() {
    let temp = Temp::new();
    let data = Temp::new();
    temp.fixture("literal");
    fs::write(temp.0.join("source.txt"), "before review\n").unwrap();
    let mut prepared = prepared(&temp, fixture_spec(&[]));
    // Ordinary folders remain executable under explicitly unknown source coverage.
    prepared.before = crate::workspace::repo::observe_inputs(&temp.0, &data.0).unwrap();
    assert!(prepared.before.fingerprint.is_none());
    assert!(validate_source_guard(&prepared, &temp.0, &data.0, false).is_ok());
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(&temp.0)
            .status()
            .unwrap()
            .success()
    );
    prepared.before = crate::workspace::repo::observe_inputs(&temp.0, &data.0).unwrap();
    assert!(prepared.before.fingerprint.is_some());
    assert!(validate_source_guard(&prepared, &temp.0, &data.0, false).is_ok());
    fs::write(temp.0.join("source.txt"), "changed after exact review\n").unwrap();
    assert!(validate_source_guard(&prepared, &temp.0, &data.0, false).is_err());
}

#[test]
fn changed_executable_invalidates_committed_approval_before_any_effect() {
    let temp = Temp::new();
    temp.fixture("multicall");
    let path = temp.0.join(if cfg!(windows) {
        "fixture.exe"
    } else {
        "fixture"
    });
    fs::copy(std::env::current_exe().unwrap(), &path).unwrap();
    let spec = alias_fixture_spec(&temp, &path, &[]);
    fs::write(temp.0.join("expected-argv0"), program(&spec)).unwrap();
    let p = prepared(&temp, spec);
    let (_store, permit) = committed_permit(&temp, &p);
    let gate = Mutex::new(Gate {
        pending: Some(p.request_id.clone()),
        ..Gate::default()
    });
    let mut changed = fs::OpenOptions::new()
        .append(true)
        .open(&p.executable.path)
        .unwrap();
    changed.write_all(b"changed after approval").unwrap();
    drop(changed);
    let error = match admit(&p, &permit, &gate, &AtomicBool::new(false)) {
        Ok(_) => panic!("changed multicall image dispatched"),
        Err(error) => error,
    };
    assert!(
        error.contains("executable changed since preview"),
        "{error}"
    );
    assert!(gate.lock().active.is_none());
    assert!(!temp.0.join("multicall.started").exists());
}

#[cfg(unix)]
#[test]
fn executable_identity_refuses_fifo_and_link_substitutions_without_blocking() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let fifo = temp.0.join("replaced-executable");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o700) }, 0);
    let start = Instant::now();
    assert!(executable_identity(&fifo).is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    fs::remove_file(&fifo).unwrap();
    symlink(std::env::current_exe().unwrap(), &fifo).unwrap();
    assert!(executable_identity(&fifo).is_err());
}

#[cfg(windows)]
#[test]
fn windows_batch_files_are_refused_without_an_explicit_shell() {
    let temp = Temp::new();
    let path = temp.0.join("command.cmd");
    fs::write(&path, "@echo off\r\necho forbidden>side-effect\r\n").unwrap();
    let result = prepare(
        unique_id(),
        1,
        CommandSpec::Direct {
            program: path.to_str().unwrap().into(),
            args: vec![],
        },
        temp.0.clone(),
        InputSnapshot::default(),
    );
    assert!(result.unwrap_err().contains("explicit cmd.exe"));
    assert!(!temp.0.join("side-effect").exists());
}

#[cfg(unix)]
#[test]
fn ignored_soft_cancellation_escalates_and_stops_the_owned_tree() {
    let temp = Temp::new();
    temp.fixture("stubborn");
    let (record, _, _) = execute(
        &temp,
        prepared(&temp, fixture_spec(&[])),
        128 * 1024,
        Some("grandchild.beat"),
    );
    assert!(record.cancelled);
    assert_eq!(record.signal, Some(libc::SIGKILL));
    assert_heartbeats_stopped(&temp);
}

#[cfg(unix)]
#[test]
fn escaped_pipe_owner_is_unknown_not_a_sandboxed_success() {
    let temp = Temp::new();
    temp.fixture("escape-root");
    let (record, _, _) = execute(&temp, prepared(&temp, fixture_spec(&[])), 128 * 1024, None);
    struct EscapedFixture(libc::pid_t);
    impl Drop for EscapedFixture {
        fn drop(&mut self) {
            unsafe {
                libc::kill(self.0, libc::SIGKILL);
            }
        }
    }
    let escaped = EscapedFixture(
        fs::read_to_string(temp.0.join("escape.pid"))
            .unwrap()
            .parse()
            .unwrap(),
    );
    assert_eq!(record.state, RunState::OutcomeUnknown);
    assert_eq!(record.tree, TreeOutcome::OwnershipUnknown);
    assert!(record.truncated);
    // Only this test's explicit PID cleanup can kill a voluntarily escaped POSIX session.
    // The runtime correctly refuses to claim that a process group was an OS sandbox.
    drop(escaped);
}

#[test]
fn each_partners_prepared_run_records_its_own_attribution() {
    let temp = Temp::new();
    temp.fixture("literal");
    let worker = Worker::start().unwrap();
    for (id, name, form) in [(1, "Bolt", 2), (2, "Mizu", 1)] {
        let partner = crate::store::Attribution {
            partner: id,
            name: name.into(),
            form,
        };
        worker
            .prepare_for(
                1,
                Some(partner.clone()),
                fixture_spec(&[]),
                temp.0.clone(),
                InputSnapshot::default(),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let p = loop {
            if let Some(Event::Prepared(p)) = worker.try_event() {
                break p;
            }
            assert!(Instant::now() < deadline, "prepare timed out");
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(p.partner.as_ref(), Some(&partner));
        let (record, _, _) = execute(&temp, p, 4096, None);
        assert_eq!(record.prepared.partner, Some(partner));
        worker.invalidate();
    }
    let invalid = crate::store::Attribution {
        partner: 1,
        name: "line\nbreak".into(),
        form: 2,
    };
    assert!(
        worker
            .prepare_for(
                1,
                Some(invalid),
                fixture_spec(&[]),
                temp.0.clone(),
                InputSnapshot::default(),
            )
            .is_err()
    );
}

#[test]
fn revision_moving_is_optional_but_true_survives_journal_roundtrip() {
    let temp = Temp::new();
    let p = prepared(&temp, fixture_spec(&[]));
    let mut record = RunRecord::awaiting(&p);
    let legacy = serde_json::to_value(&record).unwrap();
    assert!(legacy.get("revision_moving").is_none());
    let restored: RunRecord = serde_json::from_value(legacy).unwrap();
    assert!(!restored.revision_moving);
    record.revision_moving = true;
    let serialized = serde_json::to_value(&record).unwrap();
    assert_eq!(serialized["revision_moving"], true);
    let restored: RunRecord = serde_json::from_value(serialized).unwrap();
    assert_eq!(restored, record);
    assert_eq!(restored.prepared.spec, p.spec);
    assert_eq!(restored.prepared.cwd, p.cwd);
}

#[test]
fn prepared_commands_stamp_producer_arrival_and_legacy_records_default_to_zero() {
    let temp = Temp::new();
    let p = prepared(&temp, fixture_spec(&[]));
    assert!(p.approval_arrival > 0);
    let mut legacy = serde_json::to_value(&p).unwrap();
    legacy.as_object_mut().unwrap().remove("approval_arrival");
    let restored: Prepared = serde_json::from_value(legacy).unwrap();
    assert_eq!(restored.approval_arrival, 0);
    assert_eq!(restored.spec, p.spec);
    assert_eq!(restored.cwd, p.cwd);
}

#[test]
fn provider_run_observation_omits_local_identity_and_dispatch_approval() {
    let temp = Temp::new();
    let mut preparation = prepared(&temp, fixture_spec(&[]));
    preparation.partner = Some(crate::store::Attribution {
        partner: 7,
        name: "Only-local companion label".into(),
        form: 2,
    });
    let mut record = RunRecord::awaiting(&preparation);
    record.state = RunState::OutcomeUnknown;
    record.output_unavailable = true;
    record.revision_moving = true;
    let public = serde_json::to_value(RunObservation::from(&record)).unwrap();
    assert_eq!(public["partner_id"], 7);
    let encoded = public.to_string();
    for private in [
        "Only-local companion label",
        "\"prepared\"",
        "\"partner\"",
        "\"form\"",
        "\"approval_arrival\"",
        "\"expires_at_ms\"",
        "\"cwd_identity\"",
    ] {
        assert!(!encoded.contains(private), "{private}");
    }
}
