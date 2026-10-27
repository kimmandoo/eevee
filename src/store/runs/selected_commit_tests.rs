use super::*;
use crate::workspace::runner::{
    self, CommandSpec, Event as RunEvent, InputSnapshot, Prepared, RunState,
};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "eevee-gp07-store-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let project = root.join("project");
        let data = root.join("data");
        fs::create_dir(&project).unwrap();
        let f = Self {
            root,
            project,
            data,
        };
        f.git(&["init", "-q"]);
        f.git(&["config", "user.name", "GP07 storage fixture"]);
        f.git(&["config", "user.email", "fixture@example.invalid"]);
        fs::write(f.project.join("selected.txt"), b"base selected\n").unwrap();
        fs::write(f.project.join("other.txt"), b"base other\n").unwrap();
        f.git(&["add", "--", "selected.txt", "other.txt"]);
        f.git(&["commit", "-qm", "fixture"]);
        fs::write(f.project.join("selected.txt"), b"actual reviewed bytes\n").unwrap();
        fs::write(f.project.join("other.txt"), b"other dirty unchanged\n").unwrap();
        f
    }
    fn git(&self, args: &[&str]) -> Vec<u8> {
        let mut command = Command::new(runner::resolve_executable("git", &self.project).unwrap());
        command
            .current_dir(&self.project)
            .env_clear()
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_CONFIG_GLOBAL", "/dev/null");
        for key in ["SystemRoot", "WINDIR"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let result = command
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgSign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        result.stdout
    }
    fn open(&self) -> State {
        let mut state = State {
            data_dir: self.data.clone(),
            project: self.project.to_str().unwrap().into(),
            policy: Policy::default(),
            connection: None,
            lock: None,
        };
        assert!(matches!(state.reopen(), Event::Loaded(_)));
        state
    }
    fn task(&self, store: &mut State, lead: i64) -> Snapshot {
        let mut task = Snapshot::empty(self.project.to_str().unwrap().into());
        task.goal = "Commit only explicit selected bytes".into();
        task.lead_partner = lead;
        store.save(task).unwrap()
    }
    fn prepared(&self, worker: &runner::Worker, task: &Snapshot, owner: Attribution) -> Prepared {
        let request = worker
            .prepare_for(
                task.id,
                Some(owner),
                CommandSpec::SelectedCommit {
                    paths: vec!["selected.txt".into()],
                    message: "Actual local selected commit\n".into(),
                    review: None,
                },
                self.project.clone(),
                InputSnapshot::default(),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            assert!(
                Instant::now() < deadline,
                "selected commit preparation did not finish"
            );
            match worker.try_event() {
                Some(RunEvent::Prepared(p)) => {
                    assert_eq!(p.request_id, request);
                    return p;
                }
                Some(RunEvent::Rejected { message, .. }) => panic!("{message}"),
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn project_exclusive_reservation_blocks_existing_and_later_cross_task_writers() {
    let f = Fixture::new();
    let mut store = f.open();
    let roster = store.add_partner().unwrap();
    let task = f.task(&mut store, roster[0].id);
    let other = f.task(&mut store, roster[1].id);
    let claim = WorkClaim {
        task_id: other.id,
        owner: Attribution::of(&roster[1]),
        request_id: "gp07-existing-writer".into(),
        writing: true,
    };
    store
        .work_request(&WorkRequest::Acquire {
            claim: claim.clone(),
            task: Box::new(other.clone()),
        })
        .unwrap();
    let worker = runner::Worker::start().unwrap();
    let p = f.prepared(&worker, &task, Attribution::of(&roster[0]));
    assert!(store.reserve_run(task.id, p.clone(), 64 * 1024).is_err());
    store
        .work_request(&WorkRequest::Release(claim.clone()))
        .unwrap();
    let permit = store.reserve_run(task.id, p.clone(), 64 * 1024).unwrap();
    assert!(!permit.revision_moving);
    assert!(
        store
            .work_request(&WorkRequest::Acquire {
                claim: claim.clone(),
                task: Box::new(other)
            })
            .is_err()
    );
    let claims = super::super::work::claims(store.connection.as_ref().unwrap()).unwrap();
    assert_eq!(claims.len(), 1);
    assert!(claims[0].writing);
    assert_eq!(claims[0].request_id, p.request_id);
    assert!(f.git(&["show", "HEAD:selected.txt"]) == b"base selected\n");
    drop(permit);
    store
        .run_request(RunRequest {
            task_id: task.id,
            operation: RunOperation::Abandon {
                request_id: p.request_id,
                run_id: p.run_id,
            },
        })
        .unwrap();
    assert!(
        super::super::work::claims(store.connection.as_ref().unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn ordinary_one_shot_runner_permit_records_actual_commit_and_restores_evidence_only() {
    let f = Fixture::new();
    let mut store = f.open();
    let task = f.task(&mut store, 1);
    let owner = Attribution::of(&partners(store.connection.as_ref().unwrap()).unwrap()[0]);
    let worker = runner::Worker::start().unwrap();
    let p = f.prepared(&worker, &task, owner);
    let CommandSpec::SelectedCommit {
        review: Some(review),
        ..
    } = &p.spec
    else {
        panic!()
    };
    assert!(
        !review.has_authority(),
        "public/UI preparation must not retain raw authority"
    );
    let permit = store.reserve_run(task.id, p.clone(), 64 * 1024).unwrap();
    worker.dispatch(&p.request_id, permit).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let final_record = loop {
        assert!(
            Instant::now() < deadline,
            "owned selected commit did not finish"
        );
        match worker.try_event() {
            Some(RunEvent::Started(record))
            | Some(RunEvent::Cancelling(record))
            | Some(RunEvent::CaptureChanged(record)) => {
                store.update_run(task.id, record).unwrap();
            }
            Some(RunEvent::Finished(record)) => break store.update_run(task.id, record).unwrap(),
            Some(RunEvent::Rejected { message, .. }) => panic!("{message}"),
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    assert_eq!(final_record.state, RunState::Succeeded);
    assert_eq!(
        final_record.exit_code, None,
        "a multi-process transaction cannot fabricate one OS exit code"
    );
    let outcome = final_record.selected_commit.as_ref().unwrap();
    assert!(outcome.ref_published && outcome.index_published);
    assert_eq!(
        format!("{}\n", outcome.created_commit.as_ref().unwrap()).as_bytes(),
        f.git(&["rev-parse", "HEAD"])
    );
    assert_eq!(
        f.git(&["show", "HEAD:selected.txt"]),
        b"actual reviewed bytes\n"
    );
    assert_eq!(f.git(&["show", "HEAD:other.txt"]), b"base other\n");
    assert_eq!(
        fs::read(f.project.join("other.txt")).unwrap(),
        b"other dirty unchanged\n"
    );
    let page = store.read_output(task.id, &p.run_id, 0, 64 * 1024).unwrap();
    assert!(!page.unavailable);
    drop(store);
    let store = f.open();
    let restored = store.load_runs(task.id).unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0], final_record);
    let CommandSpec::SelectedCommit {
        review: Some(review),
        ..
    } = &restored[0].prepared.spec
    else {
        panic!()
    };
    assert!(!review.has_authority());
    assert!(
        super::super::work::claims(store.connection.as_ref().unwrap())
            .unwrap()
            .is_empty()
    );
    assert_eq!(f.git(&["rev-list", "--count", "HEAD"]), b"2\n");
}

#[test]
fn serialized_public_review_and_resource_permit_cannot_restore_worker_dispatch_authority() {
    let f = Fixture::new();
    let mut store = f.open();
    let task = f.task(&mut store, 1);
    let owner = Attribution::of(&partners(store.connection.as_ref().unwrap()).unwrap()[0]);
    let original = runner::Worker::start().unwrap();
    let p = f.prepared(&original, &task, owner);
    let bytes = serde_json::to_vec(&p).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("actual reviewed bytes"));
    drop(original);
    let restored: Prepared = serde_json::from_slice(&bytes).unwrap();
    // Storage reserves resources, not approval. Only the live originating
    // worker owns the pending one-shot authority and exact frozen bytes.
    let permit = store.reserve_run(task.id, restored, 64 * 1024).unwrap();
    let fresh = runner::Worker::start().unwrap();
    let head = f.git(&["rev-parse", "HEAD"]);
    assert!(fresh.dispatch(&p.request_id, permit).is_err());
    assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
    assert_eq!(f.git(&["show", "HEAD:selected.txt"]), b"base selected\n");
    store
        .run_request(RunRequest {
            task_id: task.id,
            operation: RunOperation::Abandon {
                request_id: p.request_id,
                run_id: p.run_id,
            },
        })
        .unwrap();
    assert!(
        super::super::work::claims(store.connection.as_ref().unwrap())
            .unwrap()
            .is_empty()
    );
}
