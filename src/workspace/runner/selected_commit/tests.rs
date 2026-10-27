use super::*;
use std::process::Command;
use std::sync::atomic::AtomicUsize;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("eevee-selected-commit-{}", unique_id()));
        fs::create_dir(&root).unwrap();
        let f = Self(fs::canonicalize(root).unwrap());
        f.git(&["init", "-q"]);
        f.git(&["config", "user.name", "Selected Commit Fixture"]);
        f.git(&["config", "user.email", "fixture@example.invalid"]);
        fs::write(f.0.join("selected space.txt"), b"original selected\n").unwrap();
        fs::write(f.0.join("unselected.txt"), b"original unselected\n").unwrap();
        f.git(&["add", "--", "selected space.txt", "unselected.txt"]);
        f.git(&["commit", "-qm", "initial fixture"]);
        fs::write(
            f.0.join("selected space.txt"),
            b"exact selected private bytes\r\n",
        )
        .unwrap();
        fs::write(
            f.0.join("unselected.txt"),
            b"unselected unchanged bytes\0\xff\n",
        )
        .unwrap();
        f
    }
    fn git(&self, args: &[&str]) -> Vec<u8> {
        self.git_input(args, &[])
    }
    fn git_input(&self, args: &[&str], input: &[u8]) -> Vec<u8> {
        use std::process::Stdio;
        let mut child = Command::new(resolve_git(&self.0).unwrap())
            .current_dir(&self.0)
            .env_clear()
            .envs(environment())
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgSign=false",
                "-c",
                "core.autocrlf=false",
            ])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "fixture Git {:?}: {}",
            args,
            String::from_utf8_lossy(&result.stderr)
        );
        result.stdout
    }
    fn draft(&self) -> CommandSpec {
        CommandSpec::SelectedCommit {
            paths: vec!["selected space.txt".into()],
            message: "Exact authored message\n\nWhole-file selection only.\n".into(),
            review: None,
        }
    }
    fn prepare(&self) -> Prepared {
        prepare(
            unique_id(),
            1,
            self.draft(),
            self.0.clone(),
            InputSnapshot::default(),
        )
        .unwrap()
    }
    fn head(&self) -> Vec<u8> {
        self.git(&["rev-parse", "HEAD"])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn canonical_unicode_repository_paths_reach_config_and_isolated_index() {
    let mut f = Fixture::new();
    let named =
        f.0.with_file_name(format!("eevee-selected-commit 空 格 한글-{}", unique_id()));
    fs::rename(&f.0, &named).unwrap();
    f.0 = fs::canonicalize(named).unwrap();
    #[cfg(windows)]
    assert!(f.0.to_str().unwrap().starts_with(r"\\?\"));
    let p = f.prepare();
    let CommandSpec::SelectedCommit {
        review: Some(review),
        ..
    } = &p.spec
    else {
        panic!("selected commit preparation omitted its review");
    };
    assert_eq!(review.author, "Selected Commit Fixture");
    let selected = fs::read(f.0.join("selected space.txt")).unwrap();
    let other = fs::read(f.0.join("unselected.txt")).unwrap();
    let outcome = execute(&p, &|| false);
    assert!(outcome.error.is_none(), "{outcome:?}");
    assert!(outcome.ref_published && outcome.index_published && outcome.owned_tree_exited);
    assert_eq!(f.git(&["show", "HEAD:selected space.txt"]), selected);
    assert_eq!(fs::read(f.0.join("unselected.txt")).unwrap(), other);
}

#[test]
fn only_selected_whole_file_commits_and_other_dirty_bytes_remain_exact() {
    let f = Fixture::new();
    let p = f.prepare();
    let before = f.head();
    let selected = fs::read(f.0.join("selected space.txt")).unwrap();
    let other = fs::read(f.0.join("unselected.txt")).unwrap();
    let outcome = execute(&p, &|| false);
    assert!(outcome.error.is_none(), "{:?}", outcome);
    assert!(outcome.ref_published && outcome.index_published && outcome.owned_tree_exited);
    assert_ne!(f.head(), before);
    assert_eq!(f.git(&["show", "HEAD:selected space.txt"]), selected);
    assert_eq!(
        f.git(&["show", "HEAD:unselected.txt"]),
        b"original unselected\n"
    );
    assert_eq!(fs::read(f.0.join("unselected.txt")).unwrap(), other);
    assert_eq!(
        f.git(&["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"]),
        b"selected space.txt\n"
    );
    assert_eq!(
        text(f.head()).unwrap(),
        outcome.created_commit.clone().unwrap()
    );
    assert_eq!(outcome.observed_head, outcome.created_commit);
    assert_eq!(outcome.observed_index_sha256, outcome.expected_index_sha256);
    let commit = f.git(&["cat-file", "commit", "HEAD"]);
    let message = commit.windows(2).position(|v| v == b"\n\n").unwrap() + 2;
    let CommandSpec::SelectedCommit {
        message: expected, ..
    } = &p.spec
    else {
        panic!()
    };
    assert_eq!(&commit[message..], expected.as_bytes());
}

#[test]
fn staged_merge_and_conflict_state_never_prepare() {
    for state in ["staged", "merge", "conflict", "assume"] {
        let f = Fixture::new();
        let head = f.head();
        match state {
            "staged" => {
                f.git(&["add", "--", "unselected.txt"]);
            }
            "merge" => fs::write(f.0.join(".git/MERGE_HEAD"), &head).unwrap(),
            "conflict" => {
                let object = text(f.git(&["rev-parse", "HEAD:unselected.txt"])).unwrap();
                let input=format!("0 {}\\tunselected.txt\n100644 {object} 1\\tunselected.txt\n100644 {object} 2\\tunselected.txt\n", "0".repeat(40)).replace("\\t","\t");
                f.git_input(&["update-index", "--index-info"], input.as_bytes());
            }
            "assume" => {
                f.git(&["update-index", "--assume-unchanged", "unselected.txt"]);
            }
            _ => unreachable!(),
        }
        assert!(
            prepare(
                unique_id(),
                1,
                f.draft(),
                f.0.clone(),
                InputSnapshot::default()
            )
            .is_err(),
            "{state}"
        );
        assert_eq!(f.head(), head);
    }
}

#[test]
fn head_source_index_configuration_and_mode_drift_refuse_without_publication() {
    for drift in ["head", "ref", "source", "index", "config", "merge", "mode"] {
        let f = Fixture::new();
        let p = f.prepare();
        match drift {
            "head" => {
                f.git(&["symbolic-ref", "HEAD", "refs/heads/different"]);
            }
            "ref" => {
                let reference = text(f.git(&["symbolic-ref", "HEAD"])).unwrap();
                let head = text(f.head()).unwrap();
                f.git(&["update-ref", "refs/heads/other", &head]);
                f.git(&["symbolic-ref", &reference, "refs/heads/other"]);
            }
            "source" => fs::write(f.0.join("selected space.txt"), b"changed after review").unwrap(),
            "index" => {
                f.git(&["add", "--", "unselected.txt"]);
            }
            "config" => {
                f.git(&["config", "user.name", "Changed identity"]);
            }
            "merge" => fs::write(f.0.join(".git/MERGE_HEAD"), b"external merge").unwrap(),
            "mode" => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(
                        f.0.join("selected space.txt"),
                        fs::Permissions::from_mode(0o755),
                    )
                    .unwrap();
                }
                #[cfg(windows)]
                {
                    fs::write(
                        f.0.join("selected space.txt"),
                        b"changed platform mode fixture",
                    )
                    .unwrap();
                }
            }
            _ => unreachable!(),
        }
        let outcome = execute(&p, &|| false);
        assert!(outcome.error.is_some(), "{drift}: {outcome:?}");
        assert!(
            !outcome.publication_started && !outcome.ref_published && !outcome.index_published,
            "{drift}: {outcome:?}"
        );
        assert!(outcome.created_commit.is_none(), "{drift}: {outcome:?}");
    }
}

#[test]
fn head_lock_acquired_during_materialization_refuses_and_preserves_external_ownership() {
    let f = Fixture::new();
    let p = f.prepare();
    let head = f.head();
    let index = fs::read(f.0.join(".git/index")).unwrap();
    let head_lock = f.0.join(".git/HEAD.lock");
    let introduced = std::cell::Cell::new(false);
    let outcome = execute(&p, &|| {
        // A different Git writer takes HEAD ownership after our initial validation.
        // The observer remains non-cancelling, exercising final publication validation.
        if f.0.join(".git/index.lock").is_file()
            && f.0.join(".git/config.lock").is_file()
            && !introduced.replace(true)
        {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&head_lock)
                .unwrap()
                .write_all(b"external HEAD lock ownership\n")
                .unwrap();
        }
        false
    });
    assert!(
        introduced.get(),
        "external ownership boundary was not reached"
    );
    assert!(outcome.created_commit.is_some(), "{outcome:?}");
    assert!(
        outcome
            .error
            .as_deref()
            .is_some_and(|error| error.contains("Repository is locked: HEAD.lock")),
        "{outcome:?}"
    );
    assert!(!outcome.publication_started && !outcome.ref_published && !outcome.index_published);
    assert_eq!(f.head(), head);
    assert_eq!(fs::read(f.0.join(".git/index")).unwrap(), index);
    assert_eq!(
        fs::read(&head_lock).unwrap(),
        b"external HEAD lock ownership\n"
    );
    assert!(!f.0.join(".git/index.lock").exists());
    assert!(!f.0.join(".git/config.lock").exists());
}

#[test]
fn configured_hooks_filters_signers_and_helpers_are_not_invoked() {
    let f = Fixture::new();
    let marker = f.0.join("MUST-NOT-EXECUTE");
    let helper = f.0.join("marker-helper");
    fs::write(
        &helper,
        format!(
            "#!/bin/sh\nprintf invoked > '{}'\nexit 99\n",
            marker.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let helper = helper.to_str().unwrap();
    for hook in [
        "pre-commit",
        "prepare-commit-msg",
        "commit-msg",
        "post-commit",
        "reference-transaction",
    ] {
        fs::copy(helper, f.0.join(".git/hooks").join(hook)).unwrap();
    }
    let hooks = f.0.join(".git/hooks");
    for (key, value) in [
        ("core.hooksPath", hooks.to_str().unwrap()),
        ("filter.tripwire.clean", helper),
        ("filter.tripwire.smudge", helper),
        ("filter.tripwire.process", helper),
        ("filter.tripwire.required", "true"),
        ("commit.gpgSign", "true"),
        ("gpg.program", helper),
        ("core.fsmonitor", helper),
        ("core.sshCommand", helper),
        ("credential.helper", helper),
        ("diff.external", helper),
    ] {
        f.git(&["config", key, value]);
    }
    fs::write(
        f.0.join(".gitattributes"),
        b"* filter=tripwire diff=tripwire\n",
    )
    .unwrap();
    let p = f.prepare();
    let outcome = execute(&p, &|| false);
    assert!(outcome.error.is_none(), "{outcome:?}");
    assert!(!marker.exists(), "repository executable effect ran");
    assert_eq!(
        f.git(&["show", "HEAD:selected space.txt"]),
        b"exact selected private bytes\r\n"
    );
}

#[test]
fn unsupported_includes_and_external_object_storage_refuse() {
    for effect in [
        "include",
        "alternates",
        "promisor",
        "configured-hook",
        "linked-worktrees",
    ] {
        let f = Fixture::new();
        match effect {
            "include" => {
                f.git(&["config", "include.path", "outside-config"]);
            }
            "promisor" => {
                f.git(&["config", "remote.origin.promisor", "true"]);
            }
            "configured-hook" => {
                f.git(&["config", "hook.tripwire.command", "must-not-run"]);
            }
            "alternates" => fs::write(
                f.0.join(".git/objects/info/alternates"),
                b"/outside/object/store\n",
            )
            .unwrap(),
            "linked-worktrees" => fs::create_dir(f.0.join(".git/worktrees")).unwrap(),
            _ => unreachable!(),
        }
        assert!(
            prepare(
                unique_id(),
                1,
                f.draft(),
                f.0.clone(),
                InputSnapshot::default()
            )
            .is_err()
        );
    }
}

#[test]
fn cancellation_retains_truthful_unreachable_object_and_partial_ref_index_facts() {
    let f = Fixture::new();
    let p = f.prepare();
    let initial = f.head();
    let CommandSpec::SelectedCommit {
        review: Some(review),
        ..
    } = &p.spec
    else {
        panic!()
    };
    let reference = f.0.join(".git").join(&review.reference);
    let calls = AtomicUsize::new(0);
    let outcome = execute(&p, &|| {
        calls.fetch_add(1, Ordering::Relaxed);
        // External interference with our index publication is a controlled failure,
        // not a production fault-injection flag. Never touch any user repository.
        let published = fs::read(&reference).unwrap_or_default();
        if !published.is_empty() && published != initial && f.0.join(".git/index.lock").is_file() {
            fs::remove_file(f.0.join(".git/index.lock")).unwrap();
        }
        false
    });
    assert!(calls.load(Ordering::Relaxed) > 0);
    assert!(
        outcome.created_commit.is_some() && outcome.ref_published && !outcome.index_published,
        "{outcome:?}"
    );
    assert!(outcome.publication_started && outcome.error.is_some());
    assert_eq!(
        text(f.head()).unwrap(),
        outcome.created_commit.clone().unwrap()
    );
    assert_eq!(
        fs::read(f.0.join("unselected.txt")).unwrap(),
        b"unselected unchanged bytes\0\xff\n"
    );
    let fresh = Fixture::new();
    let next = fresh.prepare();
    assert!(!execute(&next, &|| true).publication_started);
}

#[test]
fn cancellation_after_object_creation_never_claims_rollback_or_ref_publication() {
    let f = Fixture::new();
    let p = f.prepare();
    let initial = f.head();
    let index = fs::read(f.0.join(".git/index")).unwrap();
    let outcome = execute(&p, &|| {
        fs::metadata(f.0.join(".git/index.lock")).is_ok_and(|m| m.len() > 0)
    });
    assert!(outcome.created_commit.is_some(), "{outcome:?}");
    assert!(!outcome.publication_started && !outcome.ref_published && !outcome.index_published);
    assert!(outcome.error.is_some());
    assert_eq!(f.head(), initial);
    assert_eq!(fs::read(f.0.join(".git/index")).unwrap(), index);
    let commit = outcome.created_commit.unwrap();
    assert_eq!(f.git(&["cat-file", "-t", &commit]), b"commit\n");
    assert!(
        !String::from_utf8(f.git(&["rev-list", "--all"]))
            .unwrap()
            .lines()
            .any(|line| line == commit)
    );
}

#[test]
fn selected_addition_and_deletion_are_whole_file_changes_only() {
    let f = Fixture::new();
    fs::remove_file(f.0.join("selected space.txt")).unwrap();
    fs::write(f.0.join("added.txt"), b"new whole file\0raw").unwrap();
    let spec = CommandSpec::SelectedCommit {
        paths: vec!["selected space.txt".into(), "added.txt".into()],
        message: "Add and delete exactly\n".into(),
        review: None,
    };
    let p = prepare(unique_id(), 1, spec, f.0.clone(), InputSnapshot::default()).unwrap();
    let outcome = execute(&p, &|| false);
    assert!(outcome.error.is_none(), "{outcome:?}");
    assert_eq!(f.git(&["show", "HEAD:added.txt"]), b"new whole file\0raw");
    let tree = String::from_utf8(f.git(&["ls-tree", "-r", "--name-only", "HEAD"])).unwrap();
    assert!(!tree.lines().any(|line| line == "selected space.txt"));
    assert_eq!(
        fs::read(f.0.join("unselected.txt")).unwrap(),
        b"unselected unchanged bytes\0\xff\n"
    );
}

#[test]
fn persisted_and_provider_observation_cannot_restore_or_leak_frozen_authority() {
    let f = Fixture::new();
    let p = f.prepare();
    let bytes = serde_json::to_vec(&p).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("exact selected private bytes"));
    let restored: Prepared = serde_json::from_slice(&bytes).unwrap();
    assert!(
        execute(&restored, &|| false)
            .error
            .unwrap()
            .contains("no authority")
    );
    let record = RunRecord::awaiting(&p);
    let projection = serde_json::to_string(&RunObservation::from(&record)).unwrap();
    for private in [
        "frozen",
        "config_sha256",
        "index_sha256",
        "fixture@example.invalid",
        "exact selected private bytes",
    ] {
        assert!(
            !projection.contains(private),
            "private authority projected: {private}"
        );
    }
    assert!(projection.contains("selected space.txt"));
}

#[test]
fn selected_commit_run_produces_real_owned_durable_evidence() {
    let f = Fixture::new();
    let p = f.prepare();
    let spool = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(f.0.join("outside-source-output"))
        .unwrap();
    let events = Mutex::new(VecDeque::new());
    let (live, _) = mpsc::sync_channel(8);
    let gate = Mutex::new(Gate {
        active: Some(p.run_id.clone()),
        ..Gate::default()
    });
    run_selected_commit(
        RunRecord::awaiting(&p),
        (spool, 64 * 1024),
        &events,
        &live,
        &gate,
        &AtomicBool::new(false),
    );
    let events = events.lock();
    assert!(matches!(events.front(), Some(Event::Started(_))));
    let Some(Event::Finished(record)) = events.back() else {
        panic!("no final evidence")
    };
    assert_eq!(record.state, RunState::Succeeded);
    assert_eq!(record.tree, TreeOutcome::OwnedTreeExited);
    assert_eq!(
        record.selected_commit.as_ref().unwrap().created_commit,
        Some(text(f.head()).unwrap())
    );
    assert!(
        fs::read_to_string(f.0.join("outside-source-output"))
            .unwrap()
            .contains("not verification")
    );
}

#[cfg(unix)]
#[test]
fn native_backslash_and_metacharacter_path_is_not_reinterpreted() {
    let f = Fixture::new();
    let path = "literal\\name;[glob].txt";
    fs::write(f.0.join(path), b"literal native pathname\n").unwrap();
    let p = prepare(
        unique_id(),
        1,
        CommandSpec::SelectedCommit {
            paths: vec![path.into()],
            message: "Native path, no shell\n".into(),
            review: None,
        },
        f.0.clone(),
        InputSnapshot::default(),
    )
    .unwrap();
    let outcome = execute(&p, &|| false);
    assert!(outcome.error.is_none(), "{outcome:?}");
    assert_eq!(
        f.git(&["show", &format!("HEAD:{path}")]),
        b"literal native pathname\n"
    );
    assert_eq!(
        f.git(&["show", "HEAD:selected space.txt"]),
        b"original selected\n"
    );
}

#[test]
fn existing_head_and_branch_reflog_hardlinks_refuse_without_touching_unselected_bytes() {
    for head_log in [false, true] {
        let f = Fixture::new();
        let branch = text(f.git(&["symbolic-ref", "HEAD"])).unwrap();
        let log =
            f.0.join(".git/logs")
                .join(if head_log { "HEAD" } else { &branch });
        let before = fs::read(f.0.join("unselected.txt")).unwrap();
        let head = f.head();
        fs::remove_file(&log).unwrap();
        fs::hard_link(f.0.join("unselected.txt"), &log).unwrap();
        let result = prepare(
            unique_id(),
            1,
            f.draft(),
            f.0.clone(),
            InputSnapshot::default(),
        );
        assert!(result.is_err(), "hardlinked writable reflog was admitted");
        assert_eq!(fs::read(f.0.join("unselected.txt")).unwrap(), before);
        assert_eq!(f.head(), head);
    }
}

#[test]
fn touched_reflog_native_identity_and_hardlink_drift_refuse_after_review() {
    for alias in [false, true] {
        let f = Fixture::new();
        let p = f.prepare();
        let head = f.head();
        let branch = text(f.git(&["symbolic-ref", "HEAD"])).unwrap();
        let log = f.0.join(".git/logs").join(branch);
        let original = fs::read(&log).unwrap();
        let other = fs::read(f.0.join("unselected.txt")).unwrap();
        fs::remove_file(&log).unwrap();
        if alias {
            fs::hard_link(f.0.join("unselected.txt"), &log).unwrap();
        } else {
            fs::write(&log, original).unwrap();
        }
        let outcome = execute(&p, &|| false);
        assert!(
            outcome.error.is_some() && outcome.created_commit.is_none(),
            "{outcome:?}"
        );
        assert!(!outcome.publication_started && !outcome.ref_published && !outcome.index_published);
        assert_eq!(f.head(), head);
        assert_eq!(fs::read(f.0.join("unselected.txt")).unwrap(), other);
    }
}

#[cfg(unix)]
#[test]
fn reflog_file_and_ancestor_symlinks_are_not_followed() {
    for ancestor in [false, true] {
        let f = Fixture::new();
        let outside = Fixture::new();
        let head = f.head();
        let other = fs::read(f.0.join("unselected.txt")).unwrap();
        let marker = outside.0.join("outside-private");
        fs::write(&marker, b"outside must remain unchanged\n").unwrap();
        if ancestor {
            fs::rename(f.0.join(".git/logs"), f.0.join(".git/original-logs")).unwrap();
            std::os::unix::fs::symlink(&outside.0, f.0.join(".git/logs")).unwrap();
        } else {
            fs::remove_file(f.0.join(".git/logs/HEAD")).unwrap();
            std::os::unix::fs::symlink(&marker, f.0.join(".git/logs/HEAD")).unwrap();
        }
        assert!(
            prepare(
                unique_id(),
                1,
                f.draft(),
                f.0.clone(),
                InputSnapshot::default()
            )
            .is_err()
        );
        assert_eq!(
            fs::read(&marker).unwrap(),
            b"outside must remain unchanged\n"
        );
        assert_eq!(fs::read(f.0.join("unselected.txt")).unwrap(), other);
        assert_eq!(f.head(), head);
    }
}

#[cfg(unix)]
#[test]
fn source_ancestor_substitution_never_captures_outside_bytes_or_outside_mode() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    let outside = Fixture::new();
    let nested = f.0.join("nested");
    let parked = f.0.join("parked");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("input"), b"inside authorized bytes\n").unwrap();
    fs::set_permissions(nested.join("input"), fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(
        outside.0.join("input"),
        b"OUTSIDE PRIVATE BYTES MUST NEVER BE CAPTURED\n",
    )
    .unwrap();
    fs::set_permissions(outside.0.join("input"), fs::Permissions::from_mode(0o755)).unwrap();
    let initial = source(&f.0, Path::new("nested/input")).unwrap();
    assert_eq!(
        initial.1.as_deref(),
        Some(&b"inside authorized bytes\n"[..])
    );
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stopped);
    let target = outside.0.clone();
    let race_nested = nested.clone();
    let worker = thread::spawn(move || {
        while !worker_stop.load(Ordering::Acquire) {
            fs::rename(&race_nested, &parked).unwrap();
            symlink(&target, &race_nested).unwrap();
            thread::yield_now();
            fs::remove_file(&race_nested).unwrap();
            fs::rename(&parked, &race_nested).unwrap();
        }
    });
    let mut escaped = false;
    for _ in 0..4000 {
        if let Ok((file, Some(bytes), _)) = source(&f.0, Path::new("nested/input")) {
            escaped |= bytes != b"inside authorized bytes\n" || file.mode != "100644";
        }
    }
    stopped.store(true, Ordering::Release);
    worker.join().unwrap();
    assert!(
        !escaped,
        "ancestor substitution leaked outside bytes or path-derived mode"
    );
    fs::rename(&nested, f.0.join("original-nested")).unwrap();
    symlink(&outside.0, &nested).unwrap();
    assert!(
        source(&f.0, Path::new("nested/input")).is_err(),
        "unsafe ancestors must never become Missing"
    );
}

#[test]
fn selected_review_retains_actual_qualified_git_version() {
    let f = Fixture::new();
    let p = f.prepare();
    let CommandSpec::SelectedCommit {
        review: Some(review),
        ..
    } = &p.spec
    else {
        panic!()
    };
    assert!(validate_git_version(&review.git_version).is_ok());
}

#[cfg(unix)]
#[test]
fn owned_unsupported_version_candidate_never_receives_an_index_command() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let outside = Fixture::new();
    let original = f.prepare();
    let head = f.head();
    let index = fs::read(f.0.join(".git/index")).unwrap();
    let candidate = outside.0.join("version-candidate");
    let marker = outside.0.join("INDEX-COMMAND-MUST-NOT-RUN");
    let quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
    for version in [
        "git version 2.35.1".to_owned(),
        "unrecognized candidate".to_owned(),
        "x".repeat(129),
    ] {
        let script = format!(
            "#!/bin/sh\nfor arg do last=\"$arg\"; done\nif [ \"$last\" = '--version' ]; then printf '%s\\n' {}; else printf invoked > {}; exit 91; fi\n",
            quote(&version),
            quote(marker.to_str().unwrap())
        );
        fs::write(&candidate, script).unwrap();
        fs::set_permissions(&candidate, fs::Permissions::from_mode(0o700)).unwrap();
        let mut p = original.clone();
        p.spec = f.draft();
        p.executable = executable_identity(&candidate).unwrap();
        assert!(freeze(&p, &|| false).is_err());
        assert!(
            !marker.exists(),
            "unsupported candidate received index-reading plumbing"
        );
        assert_eq!(fs::read(f.0.join(".git/index")).unwrap(), index);
        assert_eq!(f.head(), head);
    }
}

#[test]
fn packed_existing_branch_publishes_without_creating_a_new_branch() {
    let f = Fixture::new();
    let reference = text(f.git(&["symbolic-ref", "HEAD"])).unwrap();
    f.git(&["pack-refs", "--all", "--prune"]);
    let heads = f.0.join(".git/refs/heads");
    if heads.exists() {
        fs::remove_dir(&heads).unwrap();
    }
    assert!(!f.0.join(".git").join(&reference).exists());
    let p = f.prepare();
    let outcome = execute(&p, &|| false);
    assert!(
        outcome.error.is_none() && outcome.ref_published && outcome.index_published,
        "{outcome:?}"
    );
    assert_eq!(text(f.git(&["symbolic-ref", "HEAD"])).unwrap(), reference);
    assert_eq!(
        f.git(&["show", "HEAD:unselected.txt"]),
        b"original unselected\n"
    );
    assert_eq!(
        fs::read(f.0.join("unselected.txt")).unwrap(),
        b"unselected unchanged bytes\0\xff\n"
    );
}

#[cfg(unix)]
#[test]
fn unselected_symlink_and_gitlink_entries_remain_opaque_and_unchanged() {
    let f = Fixture::new();
    std::os::unix::fs::symlink("unselected.txt", f.0.join("untouched-link")).unwrap();
    f.git(&["add", "--", "untouched-link"]);
    let original_head = text(f.head()).unwrap();
    f.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{original_head},untouched-submodule"),
    ]);
    f.git(&["commit", "-qm", "fixture opaque entries"]);
    let entries = f.git(&[
        "ls-tree",
        "HEAD",
        "--",
        "untouched-link",
        "untouched-submodule",
    ]);
    let p = f.prepare();
    let outcome = execute(&p, &|| false);
    assert!(outcome.error.is_none(), "{outcome:?}");
    assert_eq!(
        f.git(&[
            "ls-tree",
            "HEAD",
            "--",
            "untouched-link",
            "untouched-submodule"
        ]),
        entries
    );
    assert_eq!(
        fs::read_link(f.0.join("untouched-link")).unwrap(),
        PathBuf::from("unselected.txt")
    );
    assert!(!f.0.join("untouched-submodule").exists());
    assert_eq!(
        fs::read(f.0.join("unselected.txt")).unwrap(),
        b"unselected unchanged bytes\0\xff\n"
    );
    for path in ["untouched-link", "untouched-submodule"] {
        let spec = CommandSpec::SelectedCommit {
            paths: vec![path.into()],
            message: "Unsupported selected nonregular entry\n".into(),
            review: None,
        };
        assert!(prepare(unique_id(), 1, spec, f.0.clone(), InputSnapshot::default()).is_err());
    }
}

#[test]
fn lifecycle_events_and_debug_never_export_worker_private_source_authority() {
    let f = Fixture::new();
    let mut p = f.prepare();
    let weak = {
        let CommandSpec::SelectedCommit {
            review: Some(review),
            ..
        } = &p.spec
        else {
            panic!()
        };
        Arc::downgrade(review.frozen.as_ref().unwrap())
    };
    assert!(!format!("{p:?}").contains("exact selected private bytes"));
    let events = Mutex::new(VecDeque::new());
    let record = RunRecord::awaiting(&p);
    for event in [
        Event::Prepared(p.clone()),
        Event::Started(record.clone()),
        Event::Cancelling(record.clone()),
        Event::CaptureChanged(record.clone()),
        Event::Finished(record),
    ] {
        publish(&events, event);
    }
    for event in events.lock().iter() {
        let prepared = match event {
            Event::Prepared(prepared) => prepared,
            Event::Started(record)
            | Event::Cancelling(record)
            | Event::CaptureChanged(record)
            | Event::Finished(record) => &record.prepared,
            _ => panic!("unexpected lifecycle"),
        };
        let CommandSpec::SelectedCommit {
            review: Some(review),
            ..
        } = &prepared.spec
        else {
            panic!()
        };
        assert!(
            !review.has_authority(),
            "lifecycle queue retained raw source authority"
        );
        assert!(!format!("{event:?}").contains("exact selected private bytes"));
        assert_eq!(
            prepared.spec, p.spec,
            "stripping authority must not redact or alter the exact reviewed operation"
        );
    }
    let CommandSpec::SelectedCommit {
        review: Some(review),
        ..
    } = &p.spec
    else {
        panic!()
    };
    assert!(
        review.has_authority(),
        "the worker's live preparation must retain its private frozen inputs"
    );
    discard_private_authority(&mut p);
    assert!(
        weak.upgrade().is_none(),
        "public lifecycle records must not prolong raw frozen-input lifetime"
    );
}
