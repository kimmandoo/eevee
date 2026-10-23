use super::*;
use crate::{
    app::{App, Overlay, StorageState},
    config::{ColorMode, Config, ImageMode, LayoutMode, Theme},
    i18n::Language,
    workspace::{context::optimizer, runner},
};
use ratatui::{Terminal, backend::TestBackend};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const ORIGINAL: &[u8] = b"pub fn parse_boundary(input: &str) -> bool { !input.is_empty() }\n";
const CHANGED: &[u8] = b"pub fn parse_boundary(input: &str) -> bool { input.len() < 64 }\n";

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ev-sources-optimizer-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        let path = fs::canonicalize(path).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        Self(path)
    }

    // Same report/immutable-file identity pattern as workspace/context/tests.rs.
    // The real context worker still performs visibility, safe-reader and digest checks.
    fn report(&self, bytes: &[u8]) -> Arc<repo::Report> {
        let path = PathBuf::from("parser.rs");
        fs::write(self.0.join(&path), bytes).unwrap();
        let mut files = BTreeMap::new();
        files.insert(
            path.clone(),
            repo::FileIdentity {
                path,
                tracked: false,
                head: None,
                index: Vec::new(),
                dirty: Some(true),
                state: repo::FileState::Regular {
                    sha256: Sha256::digest(bytes).into(),
                    bytes: bytes.len() as u64,
                    binary: false,
                    executable: false,
                },
            },
        );
        let snapshot = Arc::new(repo::Snapshot {
            head: None,
            files,
            working_diff: repo::WorkingDiff::default(),
            complete: true,
            stable: true,
            warnings: Vec::new(),
        });
        Arc::new(repo::Report {
            repository: repo::RepositoryIdentity {
                root: self.0.clone(),
                git_root: Some(self.0.clone()),
                git_dir: Some(self.0.join(".git")),
                storage_root: None,
            },
            baseline: Arc::clone(&snapshot),
            current: snapshot,
            changes: Vec::new(),
            initial: true,
        })
    }

    fn task(&self) -> store::Snapshot {
        let mut task = store::Snapshot::empty(self.0.to_str().unwrap().into());
        task.id = 7;
        task.revision = 9;
        task.goal = "Keep parse_boundary bounded and preserve failed evidence".into();
        task.criteria = vec![store::Criterion {
            text: "boundary check passes offline".into(),
            met: false,
        }];
        task.next_action = "Inspect the failed boundary check before changing parsing".into();
        task
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn event(worker: &context::Worker) -> context::Event {
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        if let Some(event) = worker.try_event() {
            return event;
        }
        assert!(Instant::now() < deadline, "local context worker timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn capture(worker: &context::Worker, report: &Arc<repo::Report>) -> context::Excerpt {
    let mut pending = Some(context::Request {
        task_id: 7,
        operation: context::Operation::Inspect {
            report: Arc::clone(report),
            path: "parser.rs".into(),
            start_line: 1,
            end_line: 1,
        },
    });
    assert!(worker.try_request(&mut pending));
    assert!(pending.is_none());
    let event = event(worker);
    assert_eq!(event.task_id, 7);
    let context::Response::Inspected(mut capture) = event.result.unwrap() else {
        panic!("expected real source capture")
    };
    // Capture persistence assigns an ID; no source bytes or digests are substituted.
    capture.id = 1;
    capture
}

fn app(task: &store::Snapshot, capture: context::Excerpt, width: u16, height: u16) -> App {
    let mut app = App::new(task.project.clone(), width, height);
    app.task_id = task.id;
    app.goal = task.goal.clone();
    app.criteria = task.criteria.clone();
    app.next_action = task.next_action.clone();
    app.revision = task.revision;
    app.saved_revision = task.revision;
    app.save_requested = false;
    app.storage = StorageState::Saved;
    app.editing = false;
    app.input = "untouched draft 한글 /optimize 1".into();
    app.cursor = "untouched draft 한글".len();
    app.tasks.push(task.clone());
    app.overlay = Some(Overlay::Sources);
    app.sources.configure_optimizer(task);
    app.sources.basket = vec![capture];
    app.sources.show_basket();
    app
}

fn config(tag: &str) -> Config {
    Config {
        ascii: true,
        no_animation: true,
        animation_override: Some(false),
        language: Language::parse(tag).unwrap(),
        theme: Theme::Dark,
        color: ColorMode::Mono,
        images: ImageMode::Off,
        assets_dir: "unused".into(),
        layout: LayoutMode::Sanctuary,
        layout_explicit: false,
        data_dir: "unused-data".into(),
        storage_policy: store::Policy::default(),
        context_exclusions: Vec::new(),
        capture_capacity: 64 * 1024,
    }
}

fn render(app: &mut App, tag: &str, width: u16, height: u16) -> String {
    let config = config(tag);
    let mut artwork = crate::ui::artwork::Artwork::load(&config);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::ui::render(frame, app, &config, &mut artwork, "off"))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(state: &mut State, code: KeyCode, report: &Arc<repo::Report>) -> Action {
    state.key(KeyEvent::new(code, KeyModifiers::NONE), 7, Some(report))
}

fn finish(state: &mut State, worker: &context::Worker) -> optimizer::Plan {
    assert!(state.source_busy);
    assert!(worker.try_request(&mut state.pending));
    assert!(state.pending.is_none());
    let event = event(worker);
    assert!(
        matches!(&event.result, Ok(context::Response::Optimized(_))),
        "{event:?}"
    );
    state.event(7, event);
    assert!(!state.source_busy);
    assert!(!state.optimizer_reviewed);
    let plan = state.optimizer_preview.as_ref().unwrap().clone();
    assert_eq!(plan.tokenizer, optimizer::TOKENIZER);
    assert_eq!(plan.packet_tokens, optimizer::token_count(&plan.packet));
    assert_eq!(plan.tokens, plan.packet_tokens + plan.mandatory_note_tokens);
    plan
}

fn settle_source_catalogue(app: &mut App, worker: &context::Worker) {
    assert!(matches!(
        app.sources
            .pending
            .as_ref()
            .map(|request| &request.operation),
        Some(context::Operation::Catalogue { .. })
    ));
    assert!(worker.try_request(&mut app.sources.pending));
    let response = event(worker);
    assert!(matches!(
        &response.result,
        Ok(context::Response::Catalogue(_))
    ));
    app.sources.event(app.task_id, response);
    assert!(app.sources.pending.is_none());
    assert!(!app.sources.source_busy);
}

fn preview(
    state: &mut State,
    worker: &context::Worker,
    report: &Arc<repo::Report>,
    command: &str,
) -> optimizer::Plan {
    state.command(command, 7, Some(report));
    finish(state, worker)
}

fn applied(app: &mut App, worker: &context::Worker, report: &Arc<repo::Report>) -> optimizer::Plan {
    let plan = preview(&mut app.sources, worker, report, "optimize 20000");
    assert!(!plan.overflow);
    let (width, height) = (app.width, app.height);
    render(app, "en", width, height);
    assert!(app.sources.optimizer_reviewed);
    key(&mut app.sources, KeyCode::Char('a'), report);
    assert_eq!(app.sources.applied_plan(), Some(&plan));
    plan
}

fn assert_no_dispatch(app: &App) {
    assert!(app.planning.pending.is_none());
    assert!(app.planning.request.is_none());
    assert!(app.execution.prepared.is_none());
    assert!(app.execution.active.is_none());
    assert!(app.sources.pending.is_none());
    assert!(app.sources.pending_store.is_none());
}

#[test]
fn global_optimizer_command_queues_requested_work_without_incidental_catalogue() {
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let task = fixture.task();
    let mut app = app(&task, capture(&worker, &report), 120, 36);
    app.repository = Some(Arc::clone(&report));
    app.overlay = None;
    let basket = app.sources.basket.clone();
    let draft = (app.input.clone(), app.cursor);
    assert!(app.command("optimize 20000"));
    assert_eq!(app.overlay, Some(Overlay::Sources));
    assert!(matches!(
        app.sources
            .pending
            .as_ref()
            .map(|request| &request.operation),
        Some(context::Operation::Optimize { budget: 20000, .. })
    ));
    let plan = finish(&mut app.sources, &worker);
    assert!(app.sources.applied_plan().is_none());
    render(&mut app, "en", 120, 36);
    assert!(app.command("context-apply"));
    assert_eq!(app.sources.applied_plan(), Some(&plan));
    assert_eq!(app.sources.basket, basket);
    assert_eq!((app.input.clone(), app.cursor), draft);
    assert_no_dispatch(&app);
    // Ordinary Sources browsing still owns its catalogue operation. A direct
    // optimizer action cannot replace that pending work merely to get started.
    app.close_overlay();
    assert!(app.overlay.is_none());
    app.sources.query = "parser".into();
    app.open_overlay(Overlay::Sources);
    assert!(matches!(
        app.sources.pending.as_ref().map(|request| &request.operation),
        Some(context::Operation::Catalogue { query, .. }) if query == "parser"
    ));
    assert!(app.command("optimize 4000"));
    assert!(app.sources.source_busy);
    assert!(matches!(
        app.sources.pending.as_ref().map(|request| &request.operation),
        Some(context::Operation::Catalogue { query, .. }) if query == "parser"
    ));
    assert!(!app.sources.error.is_empty());
    settle_source_catalogue(&mut app, &worker);
    assert_eq!(app.sources.applied_plan(), Some(&plan));
    assert_eq!(app.sources.basket, basket);
    assert_eq!((app.input.clone(), app.cursor), draft);
    assert_no_dispatch(&app);
}

#[test]
fn budget_editor_treats_shortcuts_as_literal_and_esc_keeps_existing_context() {
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let mut app = app(&fixture.task(), capture(&worker, &report), 120, 36);
    let previous = applied(&mut app, &worker, &report);
    let basket = app.sources.basket.clone();
    let draft = (app.input.clone(), app.cursor);
    let budget = app.sources.optimizer_budget;
    key(&mut app.sources, KeyCode::Char('o'), &report);
    assert!(matches!(
        app.sources.editor.as_ref().unwrap().kind,
        EditKind::Budget
    ));
    let original = app.sources.editor.as_ref().unwrap().text.clone();
    for ch in "alon".chars() {
        key(&mut app.sources, KeyCode::Char(ch), &report);
    }
    assert_eq!(
        app.sources.editor.as_ref().unwrap().text,
        format!("{original}alon")
    );
    key(&mut app.sources, KeyCode::Enter, &report);
    assert_eq!(
        app.sources.editor.as_ref().unwrap().text,
        format!("{original}alon")
    );
    assert!(!app.sources.error.is_empty());
    assert!(matches!(
        key(&mut app.sources, KeyCode::Esc, &report),
        Action::Changed
    ));
    assert!(app.sources.editor.is_none());
    assert_eq!(app.sources.optimizer_budget, budget);
    assert_eq!(app.sources.optimizer_preview.as_ref(), Some(&previous));
    assert_eq!(app.sources.applied_plan(), Some(&previous));
    assert_eq!(app.sources.basket, basket);
    assert_eq!((app.input.clone(), app.cursor), draft);
    assert_no_dispatch(&app);

    key(&mut app.sources, KeyCode::Char('o'), &report);
    let length = app.sources.editor.as_ref().unwrap().text.len();
    for _ in 0..length {
        key(&mut app.sources, KeyCode::Backspace, &report);
    }
    for ch in "4000".chars() {
        key(&mut app.sources, KeyCode::Char(ch), &report);
    }
    assert!(
        app.sources.pending.is_none(),
        "typing alone must not queue optimization"
    );
    key(&mut app.sources, KeyCode::Enter, &report);
    assert!(app.sources.editor.is_none());
    let plan = finish(&mut app.sources, &worker);
    assert_eq!(plan.budget, 4000);
    assert_eq!(app.sources.applied_plan(), Some(&previous));
    assert_eq!(app.sources.basket, basket);
    assert_eq!((app.input.clone(), app.cursor), draft);
    assert_no_dispatch(&app);
}

#[test]
fn rendered_en_ko_preview_requires_review_and_only_explicit_apply_stages_exact_packet() {
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let capture = capture(&worker, &report);
    for tag in ["en", "ko"] {
        for (width, height) in [(60, 18), (120, 36)] {
            let mut app = app(&fixture.task(), capture.clone(), width, height);
            let draft = (app.input.clone(), app.cursor);
            let basket = app.sources.basket.clone();
            let plan = preview(&mut app.sources, &worker, &report, "optimize 20000");
            key(&mut app.sources, KeyCode::Char('a'), &report);
            assert!(app.sources.applied_plan().is_none());
            assert!(app.sources.error.contains("Review"));
            render(&mut app, tag, 40, 10);
            assert!(!app.sources.optimizer_reviewed);
            key(&mut app.sources, KeyCode::Char('a'), &report);
            assert!(app.sources.applied_plan().is_none());
            let screen = render(&mut app, tag, width, height);
            assert!(
                app.sources.optimizer_reviewed,
                "{tag} {width}x{height}\n{screen}"
            );
            app.update(crossterm::event::Event::Resize(40, 10));
            assert!(
                !app.sources.optimizer_reviewed,
                "resize must revoke visible review"
            );
            render(&mut app, tag, 40, 10);
            assert!(!app.sources.optimizer_reviewed);
            app.update(crossterm::event::Event::Resize(width, height));
            key(&mut app.sources, KeyCode::Char('a'), &report);
            assert!(
                app.sources.applied_plan().is_none(),
                "restoring size alone is not review"
            );
            render(&mut app, tag, width, height);
            assert!(app.sources.optimizer_reviewed);
            key(&mut app.sources, KeyCode::Char('a'), &report);
            assert_eq!(app.sources.applied_plan(), Some(&plan));
            assert_eq!(
                app.sources.applied_plan().unwrap().packet.as_bytes(),
                plan.packet.as_bytes()
            );
            assert_eq!(app.sources.basket, basket);
            assert_eq!((app.input.clone(), app.cursor), draft);
            assert_no_dispatch(&app);
        }
    }
}

#[test]
fn reviewed_overflow_refuses_apply_and_cancel_or_esc_preserve_previous_plan_and_draft() {
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let mut app = app(&fixture.task(), capture(&worker, &report), 120, 36);
    let previous = applied(&mut app, &worker, &report);
    assert_eq!(
        app.sources.optimizer_notice(),
        crate::i18n::Message::ContextOptimizerApplied
    );
    let basket = app.sources.basket.clone();
    let draft = (app.input.clone(), app.cursor);
    for cancel in ["context-cancel", "n", "Esc"] {
        let overflow = preview(&mut app.sources, &worker, &report, "optimize 64");
        assert!(overflow.overflow);
        assert!(overflow.tokens > overflow.budget);
        let packet: serde_json::Value = serde_json::from_str(&overflow.packet).unwrap();
        assert_eq!(
            packet["selected_sources"]["captures"][0]["text"].as_str(),
            Some(basket[0].text.as_str()),
            "mandatory capture must not be truncated"
        );
        assert_eq!(
            app.sources.optimizer_notice(),
            crate::i18n::Message::ContextOptimizerOverflowPrevious,
        );
        for tag in ["en", "ko"] {
            for (width, height) in [(60, 18), (120, 36)] {
                render(&mut app, tag, width, height);
                assert!(app.sources.optimizer_reviewed);
            }
        }
        key(&mut app.sources, KeyCode::Char('a'), &report);
        assert!(!app.sources.error.is_empty());
        assert_eq!(app.sources.applied_plan(), Some(&previous));
        assert_eq!(app.sources.optimizer_preview.as_ref(), Some(&overflow));
        match cancel {
            "Esc" => assert!(matches!(
                key(&mut app.sources, KeyCode::Esc, &report),
                Action::Close
            )),
            "n" => {
                key(&mut app.sources, KeyCode::Char('n'), &report);
            }
            _ => {
                app.sources.command(cancel, 7, Some(&report));
            }
        }
        assert!(app.sources.optimizer_preview.is_none());
        assert!(!app.sources.optimizer_reviewed);
        assert_eq!(app.sources.applied_plan(), Some(&previous));
        assert_eq!(app.sources.basket, basket);
        assert_eq!((app.input.clone(), app.cursor), draft);
        assert_no_dispatch(&app);
    }
}

#[test]
fn optimizer_notice_distinguishes_unapplied_preview_prior_selection_and_explicit_apply() {
    use crate::i18n::Message;
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let mut app = app(&fixture.task(), capture(&worker, &report), 120, 36);
    assert_eq!(
        app.sources.optimizer_notice(),
        Message::ContextOptimizerUnapplied
    );
    let previous = applied(&mut app, &worker, &report);
    assert_eq!(
        app.sources.optimizer_notice(),
        Message::ContextOptimizerApplied
    );
    let current = preview(&mut app.sources, &worker, &report, "optimize 4000");
    assert!(!current.overflow);
    assert_eq!(
        app.sources.optimizer_notice(),
        Message::ContextOptimizerPriorApplied
    );
    key(&mut app.sources, KeyCode::Char('a'), &report);
    assert_eq!(
        app.sources.applied_plan(),
        Some(&previous),
        "unrendered preview cannot replace prior selection"
    );
    render(&mut app, "ko", 60, 18);
    key(&mut app.sources, KeyCode::Char('a'), &report);
    assert_eq!(app.sources.applied_plan(), Some(&current));
    assert_eq!(
        app.sources.optimizer_notice(),
        Message::ContextOptimizerApplied
    );
    app.sources.command("context-clear", 7, Some(&report));
    assert!(app.sources.applied_plan().is_none());
    assert_eq!(
        app.sources.optimizer_notice(),
        Message::ContextOptimizerUnapplied
    );
    let overflow = preview(&mut app.sources, &worker, &report, "optimize 64");
    assert!(overflow.overflow);
    assert_eq!(
        app.sources.optimizer_notice(),
        Message::ContextOptimizerOverflowUnapplied
    );
    render(&mut app, "en", 60, 18);
    key(&mut app.sources, KeyCode::Char('a'), &report);
    assert!(app.sources.applied_plan().is_none());
    assert_eq!(
        app.sources.optimizer_notice(),
        Message::ContextOptimizerOverflowUnapplied
    );
    assert_no_dispatch(&app);
}

fn failed_run(root: &std::path::Path) -> runner::RunRecord {
    let prepared = runner::Prepared {
        request_id: "fixture-request".into(),
        run_id: "fixture-run".into(),
        task_id: 7,
        spec: runner::CommandSpec::Direct {
            program: "local-check".into(),
            args: vec!["--offline".into()],
        },
        cwd: root.into(),
        cwd_identity: "fixture-root".into(),
        executable: runner::ExecutableIdentity {
            path: root.join("local-check"),
            sha256: "0".repeat(64),
            bytes: 1,
            modified_ns: 0,
            file_id: "fixture-file".into(),
        },
        prepared_at_ms: 1,
        expires_at_ms: 2,
        approval_arrival: 0,
        before: runner::InputSnapshot::default(),
        partner: Some(crate::store::Attribution {
            partner: 23,
            name: "Private local companion".into(),
            form: 8,
        }),
        rerun_of: None,
    };
    let mut run = runner::RunRecord::awaiting(&prepared);
    run.state = runner::RunState::Failed;
    run.exit_code = Some(1);
    run.error = Some("parser rejected boundary input".into());
    run.output_unavailable = true;
    run
}

#[test]
fn keyboard_compaction_refreshes_failed_run_committed_while_sources_stays_open() {
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let task = fixture.task();
    let mut app = app(&task, capture(&worker, &report), 120, 36);
    app.repository = Some(Arc::clone(&report));
    app.execution.set_task(task.id);
    let load = app.execution.take_store_request().unwrap();
    assert_eq!(load.task_id, task.id);
    assert!(matches!(load.operation, store::RunOperation::Load));

    let mut failed = failed_run(&fixture.0);
    failed.started_at_ms = Some(10);
    failed.ended_at_ms = Some(20);
    failed.tree = runner::TreeOutcome::OwnedTreeExited;
    let mut running = failed.clone();
    running.state = runner::RunState::Running;
    running.ended_at_ms = None;
    running.exit_code = None;
    running.error = None;
    running.tree = runner::TreeOutcome::NotStarted;
    running.output_unavailable = false;
    app.store_event(store::Event::Run {
        task_id: task.id,
        request_id: None,
        run_id: None,
        result: Ok(Box::new(store::RunResponse::Runs(vec![running.clone()]))),
    });
    app.execution.active = Some(running.clone());
    app.overlay = None;
    app.open_overlay(Overlay::Sources);
    settle_source_catalogue(&mut app, &worker);
    app.sources.optimizer_budget = 20000;
    assert_eq!(app.sources.optimizer_runs, vec![running.clone()]);
    assert!(!app.execution.history_pending(task.id));
    let basket = app.sources.basket.clone();
    let draft = (app.input.clone(), app.cursor);

    // Finish and acknowledge the existing executor's real terminal event without
    // reopening Sources: its opening-time run snapshot must now be stale.
    app.execution
        .runner_event(runner::Event::Finished(failed.clone()));
    assert!(app.execution.repo_requested());
    app.execution.repo_started();
    app.execution.repo_observed(Some(report.as_ref()));
    let update = app.execution.take_store_request().unwrap();
    assert_eq!(update.task_id, task.id);
    let store::RunOperation::Update(committed) = update.operation else {
        panic!("terminal event must queue its final journal update")
    };
    assert_eq!(committed.state, runner::RunState::Failed);
    assert_eq!(committed.exit_code, Some(1));
    app.store_event(store::Event::Run {
        task_id: task.id,
        request_id: Some(committed.prepared.request_id.clone()),
        run_id: Some(committed.prepared.run_id.clone()),
        result: Ok(Box::new(store::RunResponse::Updated(committed.clone()))),
    });
    assert_eq!(app.overlay, Some(Overlay::Sources));
    assert_eq!(app.execution.history, vec![committed.clone()]);
    assert!(app.execution.active.is_none());
    assert_eq!(app.sources.optimizer_runs, vec![running]);

    app.update(crossterm::event::Event::Key(KeyEvent::new(
        KeyCode::Char('l'),
        KeyModifiers::NONE,
    )));
    let Some(context::Request {
        operation: context::Operation::Compact { runs, .. },
        ..
    }) = app.sources.pending.as_ref()
    else {
        panic!("Sources keyboard l must queue compaction with refreshed run history")
    };
    assert_eq!(runs, &vec![committed.clone()]);
    let plan = finish(&mut app.sources, &worker);
    let ledger: serde_json::Value = serde_json::from_str(&plan.packet).unwrap();
    assert_eq!(ledger["run_observations"].as_array().unwrap().len(), 1);
    assert_eq!(
        ledger["run_observations"][0]["run_id"],
        committed.prepared.run_id
    );
    let observation = &ledger["run_observations"][0];
    assert_eq!(
        observation["state"],
        serde_json::to_value(runner::RunState::Failed).unwrap()
    );
    assert_eq!(observation["exit_code"], 1);
    assert_eq!(observation["ended_at_ms"], 20);
    assert_eq!(observation["error"], "parser rejected boundary input");
    assert_eq!(observation["partner_id"], 23);
    for key in [
        "prepared",
        "partner",
        "approval_arrival",
        "prepared_at_ms",
        "expires_at_ms",
        "name",
        "form",
    ] {
        assert!(observation.get(key).is_none(), "private run key: {key}");
    }
    assert_eq!(app.sources.basket, basket);
    assert_eq!((app.input.clone(), app.cursor), draft);
    assert_no_dispatch(&app);
}

#[test]
fn keyboard_compaction_refuses_unhydrated_restored_journal_load_in_every_pending_phase() {
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let task = fixture.task();
    let capture = capture(&worker, &report);
    for phase in ["queued", "inflight", "failed"] {
        let mut app = app(&task, capture.clone(), 120, 36);
        app.repository = Some(Arc::clone(&report));
        app.execution.set_task(task.id);
        app.overlay = None;
        app.open_overlay(Overlay::Sources);
        settle_source_catalogue(&mut app, &worker);
        if phase != "queued" {
            let load = app.execution.take_store_request().unwrap();
            assert_eq!(load.task_id, task.id);
            assert!(matches!(load.operation, store::RunOperation::Load));
        }
        if phase == "failed" {
            app.store_event(store::Event::Run {
                task_id: task.id,
                request_id: None,
                run_id: None,
                result: Err("restored execution journal unavailable".into()),
            });
        }
        assert!(app.execution.history.is_empty());
        assert!(app.execution.history_pending(task.id), "{phase}");
        let basket = app.sources.basket.clone();
        let draft = (app.input.clone(), app.cursor);

        app.update(crossterm::event::Event::Key(KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::NONE,
        )));

        assert_no_dispatch(&app);
        assert!(!app.sources.source_busy, "{phase}");
        assert!(app.sources.optimizer_preview.is_none(), "{phase}");
        assert!(app.sources.applied_plan().is_none(), "{phase}");
        assert!(
            !app.sources.error.is_empty(),
            "{phase}: missing run history must be an explicit refusal, not an empty ledger"
        );
        assert!(app.execution.history_pending(task.id), "{phase}");
        assert_eq!(app.overlay, Some(Overlay::Sources));
        assert_eq!(app.sources.basket, basket);
        assert_eq!((app.input.clone(), app.cursor), draft);
        if phase == "failed" {
            assert!(app.command("compact 20000"));
            assert!(app.sources.pending.is_none());
            assert!(app.sources.optimizer_preview.is_none());
            assert!(app.execution.history_pending(task.id));
            assert_no_dispatch(&app);

            // Deliberately retry the failed journal read through its real UI
            // action. Its acknowledgement restores evidence, never execution.
            assert!(app.command("run"));
            assert_eq!(app.overlay, Some(Overlay::Execution));
            assert!(app.command("journal-retry"));
            let retry = app.execution.take_store_request().unwrap();
            assert_eq!(retry.task_id, task.id);
            assert!(matches!(retry.operation, store::RunOperation::Load));
            assert_no_dispatch(&app);
            let mut restored_failure = failed_run(&fixture.0);
            restored_failure.started_at_ms = Some(10);
            restored_failure.ended_at_ms = Some(20);
            restored_failure.tree = runner::TreeOutcome::OwnedTreeExited;
            app.store_event(store::Event::Run {
                task_id: task.id,
                request_id: None,
                run_id: None,
                result: Ok(Box::new(store::RunResponse::Runs(vec![
                    restored_failure.clone(),
                ]))),
            });
            assert!(!app.execution.history_pending(task.id));
            assert_eq!(app.execution.history, vec![restored_failure.clone()]);
            assert!(app.command("compact 20000"));
            let plan = finish(&mut app.sources, &worker);
            let ledger: serde_json::Value = serde_json::from_str(&plan.packet).unwrap();
            assert_eq!(ledger["run_observations"].as_array().unwrap().len(), 1);
            assert_eq!(
                ledger["run_observations"][0]["run_id"],
                restored_failure.prepared.run_id
            );
            assert_eq!(ledger["run_observations"][0]["state"], "Failed");
            assert_eq!(ledger["run_observations"][0]["exit_code"], 1);
            assert_eq!(app.sources.basket, basket);
            assert_eq!((app.input.clone(), app.cursor), draft);
            assert!(app.execution.take_store_request().is_none());
            assert_no_dispatch(&app);
        }
    }
}

#[test]
fn compact_reducer_uses_real_worker_and_preserves_conflicts_failure_and_changed_capture() {
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let mut task = fixture.task();
    task.workflows = serde_json::from_value(serde_json::json!({
        "chosen_decision": {"id": "latest", "text": "Use bounded parsing"},
        "criterion_origins": [{"criterion": "boundary check passes offline", "decision_id": "prior", "decision_text": "Use unbounded parsing"}],
        "hypotheses": [{"id": 1, "explanation": "The old parser caused the failure", "evidence": [
            {"relation": "Supports", "reference": {"Run": {"task_id": 7, "run_id": "fixture-run"}}}
        ]}]
    })).unwrap();
    let mut app = app(&task, capture(&worker, &report), 120, 36);
    let previous = applied(&mut app, &worker, &report);
    let basket = app.sources.basket.clone();
    let draft = (app.input.clone(), app.cursor);
    let run = failed_run(&fixture.0);
    app.sources.set_optimizer_runs(std::slice::from_ref(&run));
    let changed = fixture.report(CHANGED);
    key(&mut app.sources, KeyCode::Char('l'), &changed);
    let Some(context::Request {
        operation:
            context::Operation::Compact {
                task: queued_task,
                excerpts,
                runs,
                ..
            },
        ..
    }) = app.sources.pending.as_ref()
    else {
        panic!("l must queue actual compaction")
    };
    assert_eq!(queued_task.as_ref(), &task);
    assert_eq!(excerpts, &basket);
    assert_eq!(runs, &vec![run.clone()]);
    let plan = finish(&mut app.sources, &worker);
    assert_eq!(plan.kind, "Evidence-linked compaction");
    let ledger: serde_json::Value = serde_json::from_str(&plan.packet).unwrap();
    assert_eq!(ledger["current_approved_decision"]["id"], "latest");
    assert_eq!(ledger["superseded_decisions"][0]["decision_id"], "prior");
    assert_eq!(ledger["superseded_decisions"][0]["superseded_by"], "latest");
    assert_eq!(ledger["run_observations"].as_array().unwrap().len(), 1);
    assert_eq!(ledger["run_observations"][0]["run_id"], run.prepared.run_id);
    assert_eq!(ledger["run_observations"][0]["state"], "Failed");
    assert_eq!(
        ledger["source_observations"][0]["observed_status"],
        "changed"
    );
    assert_eq!(ledger["captures"], serde_json::to_value(&basket).unwrap());
    assert_eq!(
        ledger["captures"][0]["text"].as_str().unwrap().as_bytes(),
        ORIGINAL
    );
    assert!(matches!(
        serde_json::from_value::<crate::workspace::workflows::EvidenceRef>(
            ledger["source_observations"][0]["reference"].clone()
        )
        .unwrap(),
        crate::workspace::workflows::EvidenceRef::Source {
            excerpt_id: 1,
            task_id: 7,
            ..
        }
    ));
    assert_eq!(
        ledger["open_hypotheses_inferences"][0]["explanation"],
        "The old parser caused the failure"
    );
    assert_eq!(
        ledger["next_action"],
        serde_json::to_value(&task.next_action).unwrap()
    );
    assert_eq!(app.sources.applied_plan(), Some(&previous));
    app.sources.command("context-cancel", 7, Some(&changed));
    assert!(app.sources.optimizer_preview.is_none());
    assert_eq!(app.sources.applied_plan(), Some(&previous));
    assert_eq!(app.sources.basket, basket);
    assert_eq!((app.input.clone(), app.cursor), draft);
    assert_no_dispatch(&app);
}

#[test]
fn prepared_context_plan_is_frozen_across_later_preview_and_clear() {
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let task = fixture.task();
    let mut app = app(&task, capture(&worker, &report), 120, 36);
    let previous = applied(&mut app, &worker, &report);
    app.prepare_context_tools(&task).unwrap();
    assert_eq!(app.planning.context_plan.as_ref(), Some(&previous));
    let frozen = app.planning.context_plan.clone();
    preview(&mut app.sources, &worker, &report, "optimize 4000");
    app.sources.command("context-clear", 7, Some(&report));
    assert!(app.sources.applied_plan().is_none());
    assert_eq!(app.planning.context_plan, frozen);
    assert_eq!(
        app.planning
            .context_plan
            .as_ref()
            .unwrap()
            .packet
            .as_bytes(),
        previous.packet.as_bytes()
    );
    assert_no_dispatch(&app);
}

fn assert_cache_operation(
    actual: &store::context_cache::Request,
    expected: &store::context_cache::Request,
) {
    use store::context_cache::Operation;
    assert_eq!(actual.task_id, expected.task_id);
    match (&actual.operation, &expected.operation) {
        (Operation::Load, Operation::Load) | (Operation::Purge, Operation::Purge) => {}
        (Operation::Save(actual), Operation::Save(expected)) => assert_eq!(actual, expected),
        _ => panic!("retained cache operation changed: {actual:?} != {expected:?}"),
    }
}

#[test]
fn cache_failure_retains_exact_operation_and_real_store_pump_does_not_retry_automatically() {
    use store::context_cache::{Operation, Request};
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let task = fixture.task();
    let mut app = app(&task, capture(&worker, &report), 120, 36);
    let plan = preview(&mut app.sources, &worker, &report, "optimize 20000");
    assert!(!plan.revisions.is_empty());
    let store = store::Worker::start(
        fixture.0.join("local-data"),
        task.project.clone(),
        store::Policy::default(),
    )
    .unwrap();
    for operation in [
        Operation::Load,
        Operation::Save(plan.revisions.clone()),
        Operation::Purge,
    ] {
        let request = Request {
            task_id: 7,
            operation,
        };
        app.sources.context_cache_request = None;
        app.sources.context_cache_event(
            7,
            &request,
            Err("local revision store unavailable".into()),
        );
        assert!(!app.sources.context_cache_retryable());
        assert_cache_operation(
            app.sources.context_cache_request.as_ref().unwrap(),
            &request,
        );
        app.sources.configure_optimizer(&task);
        for _ in 0..3 {
            assert_eq!(app.submit_context_tool_stores(&store), 0);
            assert_cache_operation(
                app.sources.context_cache_request.as_ref().unwrap(),
                &request,
            );
        }
        assert_eq!(app.sources.optimizer_preview.as_ref(), Some(&plan));
        assert_no_dispatch(&app);
        app.sources.command("context-cache-retry", 7, Some(&report));
        assert!(app.sources.context_cache_retryable());
        assert_cache_operation(
            app.sources.context_cache_request.as_ref().unwrap(),
            &request,
        );
    }
}

#[test]
fn budget_rejected_same_range_roles_persist_and_render_without_cache_failure() {
    let fixture = Fixture::new();
    let source = format!(
        "pub fn subtotal() -> usize {{ let _ = \"{}\"; subtotal() }}\n",
        "subtotal supporting public context ".repeat(160),
    );
    let report = fixture.report(source.as_bytes());
    let worker = context::Worker::start(Vec::new()).unwrap();
    let store = store::Worker::start(
        fixture.0.join("cache-data"),
        fixture.0.to_str().unwrap().into(),
        store::Policy::default(),
    )
    .unwrap();
    let mut snapshot = store::Snapshot::empty(fixture.0.to_str().unwrap().into());
    snapshot.goal = "Repair subtotal underflow".into();
    let mut pending = Some(snapshot);
    assert!(store.try_save(&mut pending));
    let task = loop {
        match source_memory_store_event(&store) {
            store::Event::Saved(task) => break task,
            store::Event::Failed { message, .. } => panic!("{message}"),
            _ => {}
        }
    };
    let mut full_request = Some(context::Request {
        task_id: task.id,
        operation: context::Operation::Optimize {
            report: Arc::clone(&report),
            excerpts: Vec::new(),
            facts: optimizer::Facts::from_task(&task),
            budget: 100_000,
            cache: Vec::new(),
        },
    });
    assert!(worker.try_request(&mut full_request));
    let context::Response::Optimized(full) = event(&worker).result.unwrap() else {
        panic!("expected real full-budget plan")
    };
    assert!(full.included.iter().any(|item| !item.mandatory));
    let mut mandatory: serde_json::Value = serde_json::from_str(&full.packet).unwrap();
    mandatory["ranked_excerpts"] = serde_json::json!([]);
    let budget = optimizer::token_count(&mandatory.to_string()).max(64);
    let mut app = App::new(task.project.clone(), 120, 36);
    app.revision = task.revision;
    app.save_submitted();
    app.store_event(store::Event::Saved(task.clone()));
    app.repository = Some(Arc::clone(&report));
    app.repo_loading = false;
    assert!(app.context_tool_command(&format!("optimize {budget}")));
    assert!(worker.try_request(&mut app.sources.pending));
    app.context_source_event(event(&worker));
    let preview = app.sources.optimizer_preview.as_ref().unwrap().clone();
    assert!(!preview.overflow);
    assert!(preview.included.is_empty());
    assert!(preview.tokens <= budget);
    assert!(store.try_context_cache(&mut app.sources.context_cache_request));
    let saved = loop {
        let reply = source_memory_store_event(&store);
        if let store::Event::ContextCache { request, result } = reply {
            assert!(matches!(
                request.operation,
                store::context_cache::Operation::Save(_)
            ));
            let rows = result
                .as_ref()
                .expect("same-range roles must persist atomically")
                .rows
                .clone();
            app.store_event(store::Event::ContextCache { request, result });
            break rows;
        }
    };
    assert_eq!(saved, preview.revisions);
    assert!(!app.sources.optimizer_cache_failed);
    assert!(app.sources.error.is_empty());
    for tag in ["en", "ko"] {
        for (width, height) in [(60, 18), (120, 36)] {
            let screen = render(&mut app, tag, width, height);
            assert!(
                app.sources.optimizer_reviewed,
                "{tag} {width}x{height}\n{screen}"
            );
            assert!(app.sources.error.is_empty(), "{screen}");
            assert!(!app.sources.optimizer_cache_failed, "{screen}");
            assert_no_dispatch(&app);
        }
    }
    app.source_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert_eq!(app.sources.applied_plan().unwrap().packet, preview.packet);
    let mut load = Some(store::context_cache::Request {
        task_id: task.id,
        operation: store::context_cache::Operation::Load,
    });
    assert!(store.try_context_cache(&mut load));
    loop {
        if let store::Event::ContextCache { request, result } = source_memory_store_event(&store) {
            assert!(matches!(
                request.operation,
                store::context_cache::Operation::Load
            ));
            assert_eq!(result.unwrap().rows, saved);
            break;
        }
    }
    assert_no_dispatch(&app);
}

fn source_memory_app(
    task: &store::Snapshot,
    capture: context::Excerpt,
    report: &Arc<repo::Report>,
) -> App {
    let mut app = app(task, capture, 120, 36);
    app.memory.reset(task.id, task.project.clone());
    app.repository = Some(Arc::clone(report));
    app.repo_loading = false;
    app.sources.set_producer(Some(app.attribution()));
    app
}

fn checked_source_memory(app: &mut App, worker: &context::Worker) {
    let Some(context::Request {
        task_id,
        operation: context::Operation::Check { excerpts, .. },
    }) = app.sources.pending.as_ref()
    else {
        panic!("adoption must queue the existing source check, not another producer")
    };
    assert_eq!(*task_id, app.task_id);
    assert_eq!(
        excerpts,
        &vec![app.sources.basket[app.sources.basket_selection].clone()]
    );
    assert!(worker.try_request(&mut app.sources.pending));
    app.context_source_event(event(worker));
}

fn source_memory_store_event(worker: &store::Worker) -> store::Event {
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        if let Some(event) = worker.try_event() {
            return event;
        }
        assert!(
            Instant::now() < deadline,
            "local memory store worker timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn source_memory_real_capture_check_review_approval_and_explicit_selection() {
    use crate::workspace::memory::{Mutation, Scope, SourceKind};
    let fixture = Fixture::new();
    let report =
        fixture.report(b"pub fn parse_boundary(input: &str) -> bool { !input.is_empty() }\r\n");
    let worker = context::Worker::start(Vec::new()).unwrap();
    let store = store::Worker::start(
        fixture.0.join("memory-data"),
        fixture.0.to_str().unwrap().into(),
        store::Policy::default(),
    )
    .unwrap();
    let mut pending_task = Some(store::Snapshot::empty(fixture.0.to_str().unwrap().into()));
    assert!(store.try_save(&mut pending_task));
    let task = loop {
        match source_memory_store_event(&store) {
            store::Event::Saved(task) => break task,
            store::Event::Failed { message, .. } => panic!("{message}"),
            _ => {}
        }
    };
    let mut inspect = Some(context::Request {
        task_id: task.id,
        operation: context::Operation::Inspect {
            report: Arc::clone(&report),
            path: "parser.rs".into(),
            start_line: 1,
            end_line: 1,
        },
    });
    assert!(worker.try_request(&mut inspect));
    let context::Response::Inspected(capture) = event(&worker).result.unwrap() else {
        panic!("expected real source capture")
    };
    let mut pin = Some(store::ContextRequest {
        task_id: task.id,
        operation: store::ContextOperation::Pin(capture),
        producer: None,
    });
    assert!(store.try_context(&mut pin));
    let capture = loop {
        if let store::Event::Context { task_id, result } = source_memory_store_event(&store) {
            assert_eq!(task_id, task.id);
            break result.unwrap().remove(0);
        }
    };
    assert!(capture.id > 0);
    // The first committed task initializes memory ownership without ever
    // opening the Memory pane; consume its ordinary initial records read.
    let mut app = App::new(task.project.clone(), 120, 36);
    app.revision = task.revision;
    app.save_submitted();
    app.store_event(store::Event::Saved(task.clone()));
    assert_eq!(app.memory.task_id, task.id);
    assert!(app.memory_loading);
    assert!(store.try_memory(&mut app.memory_request));
    loop {
        let reply = source_memory_store_event(&store);
        if matches!(&reply, store::Event::Memory { .. }) {
            app.store_event(reply);
            break;
        }
    }
    assert!(!app.memory_loading);
    app.repo_loading = false;
    app.repository = Some(Arc::clone(&report));
    app.sources.configure_optimizer(&task);
    app.sources.basket = vec![capture.clone()];
    app.sources.show_basket();
    app.sources.set_producer(Some(app.attribution()));
    app.overlay = Some(Overlay::Sources);
    app.input = "untouched draft 한글 /optimize 1".into();
    app.cursor = "untouched draft 한글".len();
    let draft = (app.input.clone(), app.cursor);
    let basket = app.sources.basket.clone();
    app.update(crossterm::event::Event::Key(KeyEvent::new(
        KeyCode::Char('m'),
        KeyModifiers::NONE,
    )));
    assert!(app.memory.candidate.is_none());
    assert!(app.memory.records.is_empty());
    checked_source_memory(&mut app, &worker);
    assert_eq!(app.overlay, Some(Overlay::Memory));
    let proposal = app.memory.candidate.as_ref().unwrap();
    assert_eq!(proposal.content, capture.text);
    assert_eq!(proposal.scope, Scope::Task);
    assert_eq!(proposal.provenance.kind, SourceKind::UserAuthored);
    assert_eq!(
        proposal.provenance.reference,
        format!("task:{}/source:{}", task.id, capture.id)
    );
    assert!(app.memory.pending.is_none());
    assert!(app.memory.selected_ids.is_empty());
    assert!(capture.text.ends_with("\r\n"));
    let mut restored = crate::app::memory::State::new();
    restored.reset(task.id, task.project.clone());
    restored
        .restore_draft(&app.memory.draft_snapshot())
        .unwrap();
    assert_eq!(restored.draft.as_bytes(), capture.text.as_bytes());
    assert_eq!(
        restored.candidate.as_ref().unwrap().content.as_bytes(),
        capture.text.as_bytes(),
    );
    // Opening normal Memory review loads its records through the existing worker.
    assert!(store.try_memory(&mut app.memory_request));
    loop {
        let reply = source_memory_store_event(&store);
        if matches!(&reply, store::Event::Memory { .. }) {
            app.store_event(reply);
            break;
        }
    }
    app.update(crossterm::event::Event::Key(KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
    )));
    assert!(
        app.memory.pending.is_none(),
        "unrendered approval must remain closed"
    );
    render(&mut app, "en", 120, 36);
    assert!(app.memory.reviewed);
    app.update(crossterm::event::Event::Key(KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
    )));
    let mutation = app.memory.pending.take().expect("ordinary y approval");
    assert!(matches!(&mutation, Mutation::Approve(proposal) if proposal.content == capture.text));
    app.memory_request = Some(store::MemoryRequest {
        task_id: task.id,
        mutation: Some(mutation),
    });
    assert!(store.try_memory(&mut app.memory_request));
    loop {
        let reply = source_memory_store_event(&store);
        if matches!(&reply, store::Event::Memory { .. }) {
            app.store_event(reply);
            break;
        }
    }
    assert_eq!(app.memory.records.len(), 1);
    assert_eq!(app.memory.records[0].proposal.content, capture.text);
    assert!(
        app.memory.selected_ids.is_empty(),
        "approval must not select a source note"
    );
    assert_eq!((app.input.clone(), app.cursor), draft);
    assert_eq!(app.sources.basket, basket);
    assert_no_dispatch(&app);
    assert!(app.memory_command("select-memory"));
    assert_eq!(app.memory.selected_snapshot().unwrap().len(), 1);
    assert_no_dispatch(&app);
    let memory_id = app.memory.records[0].id;
    assert!(app.memory_command("edit-memory"));
    assert!(app.memory.paste(" explicit approved-note edit"));
    assert!(
        app.memory
            .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), true)
    );
    assert_eq!(
        app.memory.candidate.as_ref().unwrap().provenance.reference,
        format!("memory:{memory_id}/user-authored-edit"),
    );
    assert!(app.memory.pending.is_none(), "editing is not approval");
    assert_no_dispatch(&app);
}

#[test]
fn source_note_retention_is_visible_before_approval_without_compact_lockout() {
    use crate::workspace::memory::{Mutation, Provenance, SourceKind};
    let fixture = Fixture::new();
    let report = fixture.report(b"public first line\r\npublic second line\r\n");
    let worker = context::Worker::start(Vec::new()).unwrap();
    let captured = capture(&worker, &report);
    let task = fixture.task();
    for tag in ["en", "ko"] {
        let language = Language::parse(tag).unwrap();
        let disclosure = language.text(crate::i18n::Message::SourceMemoryRetention);
        let compact_text = |value: &str| {
            value
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>()
        };
        for (width, height) in [(60, 18), (120, 36)] {
            for verbose in [false, true] {
                for (kind, reference, expected) in [
                    (
                        SourceKind::UserAuthored,
                        format!("task:{}/source:{}", task.id, captured.id),
                        true,
                    ),
                    (
                        SourceKind::UserAuthored,
                        "independent authored instruction".into(),
                        false,
                    ),
                    (
                        SourceKind::VerifiedNote,
                        format!("task:{}/source:{}", task.id, captured.id),
                        false,
                    ),
                    (
                        SourceKind::UserAuthored,
                        format!("task:{}/source:{}", task.id + 1, captured.id),
                        false,
                    ),
                ] {
                    let mut app = source_memory_app(&task, captured.clone(), &report);
                    app.overlay = Some(Overlay::Memory);
                    app.verbose = verbose;
                    app.memory
                        .propose(
                            &captured.text,
                            Provenance {
                                kind,
                                reference,
                                task_id: task.id,
                            },
                            false,
                        )
                        .unwrap();
                    app.update(crossterm::event::Event::Resize(width, height));
                    app.update(crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::Char('y'),
                        KeyModifiers::NONE,
                    )));
                    assert!(
                        app.memory.pending.is_none(),
                        "unrendered approval must stay closed"
                    );
                    let screen = render(&mut app, tag, width, height);
                    for line in disclosure.lines() {
                        assert_eq!(
                            compact_text(&screen).contains(&compact_text(line)),
                            expected,
                            "{tag} {width}x{height} verbose={verbose}\n{screen}",
                        );
                    }
                    assert!(app.memory.reviewed, "{tag} {width}x{height}\n{screen}");
                    if !verbose {
                        assert_eq!(app.memory.scroll_max, 0, "{tag} {width}x{height}\n{screen}");
                    }
                    assert!(app.memory.pending.is_none(), "rendering is not approval");
                    app.update(crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::Char('y'),
                        KeyModifiers::NONE,
                    )));
                    assert!(matches!(
                        &app.memory.pending,
                        Some(Mutation::Approve(proposal))
                            if proposal.content.as_bytes() == captured.text.as_bytes()
                    ));
                    assert_no_dispatch(&app);
                }
            }
        }
    }
}

#[test]
fn source_memory_global_command_never_guesses_a_capture_and_local_command_checks_exact_selection() {
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let task = fixture.task();
    let mut app = source_memory_app(&task, capture(&worker, &report), &report);
    let mut second = app.sources.basket[0].clone();
    second.id = 2;
    app.sources.basket.push(second);
    app.sources.basket_selection = 1;
    app.overlay = None;
    let draft = (app.input.clone(), app.cursor);
    assert!(app.context_tool_command("remember-source"));
    assert_eq!(app.overlay, Some(Overlay::Sources));
    assert!(app.sources.mode == Mode::Basket);
    assert!(app.sources.pending.is_none());
    assert!(app.memory.candidate.is_none());
    assert_eq!(
        app.sources.memory_notice,
        Some(crate::i18n::Message::SourceMemorySelect)
    );
    for tag in ["en", "ko"] {
        render(&mut app, tag, 120, 36);
        assert!(app.sources.pending.is_none());
        assert!(app.memory.candidate.is_none());
        assert_no_dispatch(&app);
    }
    assert!(app.context_tool_command("remember-source"));
    let Some(context::Request {
        operation: context::Operation::Check { excerpts, .. },
        ..
    }) = app.sources.pending.as_ref()
    else {
        panic!("expected single exact source check")
    };
    assert_eq!(excerpts.len(), 1);
    assert_eq!(excerpts[0].id, 2);
    checked_source_memory(&mut app, &worker);
    assert_eq!(
        app.memory.candidate.as_ref().unwrap().provenance.reference,
        "task:7/source:2"
    );
    assert_eq!((app.input.clone(), app.cursor), draft);
    assert!(app.memory.selected_ids.is_empty());
    assert_no_dispatch(&app);
}

#[test]
fn source_memory_inflight_owner_revision_task_cancel_and_basket_changes_revoke_intent() {
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let task = fixture.task();
    let captured = capture(&worker, &report);
    for change in [
        "task",
        "owner",
        "revision",
        "close",
        "escape",
        "control-c",
        "surface",
        "basket",
        "selection",
        "available",
        "repository",
        "repository-event",
        "cancel",
        "quit",
        "quit-after-save",
        "reset-after-save",
    ] {
        let mut app = source_memory_app(&task, captured.clone(), &report);
        let draft = (app.input.clone(), app.cursor);
        assert!(app.context_tool_command("remember-source"));
        assert!(worker.try_request(&mut app.sources.pending));
        let response = event(&worker);
        match change {
            "task" => app.task_id += 1,
            "owner" => app.lead.id += 1,
            "revision" => app.revision += 1,
            "quit" => app.quit = true,
            "quit-after-save" => app.quit_after_save = true,
            "reset-after-save" => app.reset_after_save = true,
            "close" => app.close_overlay(),
            "escape" => {
                app.source_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            }
            "control-c" => {
                app.source_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
            }
            "surface" => app.open_overlay(Overlay::Repository),
            "basket" => app.sources.basket[0].text.push_str("changed"),
            "selection" => app.sources.basket_selection = 1,
            "available" => app.sources.basket[0].available = false,
            "repository" => app.repository = None,
            "repository-event" => app.sources.observed_repository(task.id, Some(&report)),
            "cancel" => {
                app.sources
                    .command("context-cancel", task.id, Some(&report));
            }
            _ => unreachable!(),
        }
        app.context_source_event(response);
        assert!(app.memory.candidate.is_none(), "{change}");
        assert!(app.memory.pending.is_none(), "{change}");
        assert!(app.memory.selected_ids.is_empty(), "{change}");
        assert_eq!((app.input.clone(), app.cursor), draft, "{change}");
        assert_no_dispatch(&app);
    }
}

#[test]
fn source_memory_refuses_changed_missing_and_policy_excluded_real_sources() {
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let task = fixture.task();
    let captured = capture(&worker, &report);
    for refusal in ["changed", "missing", "excluded"] {
        fs::write(fixture.0.join("parser.rs"), ORIGINAL).unwrap();
        let mut app = source_memory_app(&task, captured.clone(), &report);
        assert!(app.context_tool_command("remember-source"));
        match refusal {
            "changed" => fs::write(fixture.0.join("parser.rs"), CHANGED).unwrap(),
            "missing" => fs::remove_file(fixture.0.join("parser.rs")).unwrap(),
            "excluded" => {}
            _ => unreachable!(),
        }
        if refusal == "excluded" {
            let policy_worker = context::Worker::start(vec![PathBuf::from("parser.rs")]).unwrap();
            checked_source_memory(&mut app, &policy_worker);
        } else {
            checked_source_memory(&mut app, &worker);
        }
        assert!(app.memory.candidate.is_none(), "{refusal}");
        assert!(app.memory.pending.is_none());
        assert!(app.memory.selected_ids.is_empty());
        assert_eq!(
            app.sources.memory_notice,
            Some(crate::i18n::Message::SourceMemoryStale)
        );
        assert_no_dispatch(&app);
    }
}

#[test]
fn source_memory_busy_drafts_unsaved_foreign_unavailable_and_oversized_captures_are_retained() {
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let task = fixture.task();
    let captured = capture(&worker, &report);
    for refusal in [
        "draft",
        "literal",
        "loading",
        "saving",
        "unsaved",
        "foreign",
        "unavailable",
        "uncommitted",
    ] {
        let mut app = source_memory_app(&task, captured.clone(), &report);
        match refusal {
            "draft" => {
                assert!(app.memory.command("create-memory", true));
                assert!(app.memory.paste("exact existing memory 한글"));
            }
            "literal" => {
                app.sources.command("search", task.id, Some(&report));
                app.sources.editor.as_mut().unwrap().text =
                    "literal /remember-source m 한글".into();
            }
            "loading" => app.memory_loading = true,
            "saving" => app.memory.saving = true,
            "unsaved" => app.revision += 1,
            "foreign" => app.sources.basket[0].task_id += 1,
            "unavailable" => app.sources.basket[0].available = false,
            "uncommitted" => app.sources.basket[0].id = 0,
            _ => unreachable!(),
        }
        let memory = app.memory.draft_snapshot();
        let literal = app
            .sources
            .editor
            .as_ref()
            .map(|editor| (editor.text.clone(), editor.cursor));
        let draft = (app.input.clone(), app.cursor);
        assert!(app.context_tool_command("remember-source"));
        assert!(app.sources.pending.is_none(), "{refusal}");
        assert_eq!(app.memory.draft_snapshot(), memory, "{refusal}");
        assert_eq!(
            app.sources
                .editor
                .as_ref()
                .map(|editor| (editor.text.clone(), editor.cursor)),
            literal
        );
        assert_eq!((app.input.clone(), app.cursor), draft);
        assert!(app.sources.memory_notice.is_some(), "{refusal}");
        assert_no_dispatch(&app);
    }
    // This is a real worker capture above the existing memory limit, not a
    // shortened proposal or a synthetic special case.
    let bytes = vec![b'x'; crate::workspace::memory::MAX_CONTENT_BYTES + 1];
    let large_report = fixture.report(&bytes);
    let large = capture(&worker, &large_report);
    assert_eq!(large.text.len(), bytes.len());
    let mut app = source_memory_app(&task, large.clone(), &large_report);
    assert!(app.context_tool_command("remember-source"));
    assert!(app.sources.pending.is_none());
    assert!(app.memory.candidate.is_none());
    assert_eq!(app.sources.basket[0], large);
    assert_eq!(
        app.sources.memory_notice,
        Some(crate::i18n::Message::SourceMemoryTooLarge)
    );
    assert_no_dispatch(&app);
}

#[test]
fn source_memory_edited_proposal_uses_existing_user_authored_provenance_reset() {
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let task = fixture.task();
    let mut app = source_memory_app(&task, capture(&worker, &report), &report);
    assert!(app.context_tool_command("remember-source"));
    checked_source_memory(&mut app, &worker);
    app.memory
        .key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), true);
    assert_eq!(app.memory.mode, crate::app::memory::Mode::Edit);
    assert!(app.memory.paste(" edited note"));
    app.memory
        .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), true);
    assert_eq!(app.memory.mode, crate::app::memory::Mode::Preview);
    let proposal = app.memory.candidate.as_ref().unwrap();
    assert_eq!(
        proposal.provenance.kind,
        crate::workspace::memory::SourceKind::UserAuthored
    );
    assert_eq!(
        proposal.provenance.reference,
        "task:7/user-authored-memory-note"
    );
    assert!(proposal.content.ends_with(" edited note"));
    assert!(app.memory.pending.is_none());
    assert!(app.memory.selected_ids.is_empty());
    assert_no_dispatch(&app);
}

#[test]
fn source_memory_checks_exact_reply_identity_and_retains_newly_busy_memory() {
    let fixture = Fixture::new();
    let report = fixture.report(ORIGINAL);
    let worker = context::Worker::start(Vec::new()).unwrap();
    let task = fixture.task();
    let captured = capture(&worker, &report);
    for mismatch in ["task", "capture"] {
        let mut app = source_memory_app(&task, captured.clone(), &report);
        assert!(app.context_tool_command("remember-source"));
        let request = app.sources.pending.as_mut().unwrap();
        if mismatch == "task" {
            request.task_id += 1;
        } else {
            let context::Operation::Check { excerpts, .. } = &mut request.operation else {
                panic!("expected exact capture check")
            };
            excerpts[0].id += 1;
        }
        assert!(worker.try_request(&mut app.sources.pending));
        app.context_source_event(event(&worker));
        assert!(app.memory.candidate.is_none(), "{mismatch}");
        assert!(app.memory.pending.is_none());
        assert_no_dispatch(&app);
    }
    for busy in ["loading", "saving", "draft"] {
        let mut app = source_memory_app(&task, captured.clone(), &report);
        assert!(app.context_tool_command("remember-source"));
        assert!(worker.try_request(&mut app.sources.pending));
        let response = event(&worker);
        match busy {
            "loading" => app.memory_loading = true,
            "saving" => app.memory.saving = true,
            "draft" => {
                assert!(app.memory.command("create-memory", true));
                assert!(
                    app.memory
                        .paste("new exact draft while source check was pending 한글")
                );
            }
            _ => unreachable!(),
        }
        let draft = app.memory.draft_snapshot();
        app.context_source_event(response);
        assert_eq!(app.memory.draft_snapshot(), draft, "{busy}");
        assert_eq!(
            app.sources.memory_notice,
            Some(crate::i18n::Message::SourceMemoryBusy)
        );
        assert!(app.memory.pending.is_none());
        assert!(app.memory.selected_ids.is_empty());
        assert_no_dispatch(&app);
    }
}
