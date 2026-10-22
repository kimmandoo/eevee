use super::*;
use crate::{
    app::{Pane, StorageState, execution},
    store,
    workspace::context,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn app() -> App {
    let mut app = App::new("/fixture".into(), 120, 36);
    app.task_id = 7;
    app.storage = StorageState::Saved;
    app.repo_loading = false;
    app
}

fn key(app: &mut App, code: KeyCode) {
    app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

fn switch(app: &mut App, form: usize) -> CarryOver {
    app.request_navigation(execution::Action::SelectForm(form));
    assert!(app.refresh_carry_over());
    app.carry_over().unwrap().clone()
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "eevee-carry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("check.rs"), "fn check() {}\n").unwrap();
        Self(fs::canonicalize(path).unwrap())
    }

    fn pin(&self) -> context::Excerpt {
        let text = "fn check() {}\n";
        context::Excerpt {
            origin_run: None,
            id: 1,
            task_id: 7,
            root: self.0.to_string_lossy().into_owned(),
            path: "check.rs".into(),
            start_line: 1,
            end_line: 1,
            source_sha256: Sha256::digest(text.as_bytes()).into(),
            text: text.into(),
            available: true,
        }
    }

    fn report(&self) -> Arc<repo::Report> {
        let snapshot = Arc::new(repo::Snapshot {
            head: Some("fixture-head".into()),
            files: BTreeMap::new(),
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
            baseline: snapshot.clone(),
            current: snapshot,
            changes: Vec::new(),
            initial: true,
        })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(app: &App) -> runner::RunRecord {
    let current = repo::fingerprint(app.repository.as_deref().unwrap());
    let prepared = runner::Prepared {
        approval_arrival: crate::providers::approval_arrival(),
        request_id: "request-fixture".into(),
        run_id: "run-fixture".into(),
        task_id: 7,
        spec: runner::CommandSpec::Direct {
            program: "check".into(),
            args: Vec::new(),
        },
        cwd: PathBuf::from("/fixture"),
        cwd_identity: "fixture-dir".into(),
        executable: runner::ExecutableIdentity {
            path: PathBuf::from("/fixture/check"),
            sha256: "fixture".into(),
            bytes: 1,
            modified_ns: 1,
            file_id: "fixture".into(),
        },
        prepared_at_ms: 1,
        expires_at_ms: 1000,
        before: current.clone(),
        partner: None,
        rerun_of: None,
    };
    let mut run = runner::RunRecord::awaiting(&prepared);
    run.state = runner::RunState::Succeeded;
    run.started_at_ms = Some(2);
    run.ended_at_ms = Some(3);
    run.exit_code = Some(0);
    run.tree = runner::TreeOutcome::OwnedTreeExited;
    run.after = current;
    run
}

fn link(
    reference: workflows::EvidenceRef,
    relation: workflows::EvidenceRelation,
) -> workflows::HypothesisEvidence {
    workflows::HypothesisEvidence {
        relation,
        reference,
    }
}

fn hypothesis(id: u64, evidence: Vec<workflows::HypothesisEvidence>) -> workflows::Hypothesis {
    workflows::Hypothesis {
        id,
        explanation: "Provider says supported; only local evidence decides".into(),
        evidence,
    }
}

fn supported(app: &mut App, fixture: &Fixture) {
    use workflows::EvidenceRelation::Supports;
    app.repository = Some(fixture.report());
    app.sources.basket = vec![fixture.pin()];
    app.execution.history = vec![run(app)];
    app.workflows.hypotheses = vec![hypothesis(
        1,
        vec![
            link(
                workflows::EvidenceRef::source(&app.sources.basket[0]).unwrap(),
                Supports,
            ),
            link(
                workflows::EvidenceRef::run(&app.execution.history[0]).unwrap(),
                Supports,
            ),
        ],
    )];
}

#[test]
fn accepted_switch_pairs_capture_counts_and_capability_changes_without_work() {
    let mut app = app();
    app.criteria = vec![
        store::Criterion {
            text: "done".into(),
            met: true,
        },
        store::Criterion {
            text: "open".into(),
            met: false,
        },
    ];
    let mut pin = Fixture::new().pin();
    // Pin counts are committed ownership, not source freshness or availability.
    pin.available = false;
    pin.text.clear();
    app.sources.basket = vec![pin];
    let drafts = app.input.clone();
    let from_intake = switch(&mut app, 3);
    assert_eq!(
        (
            from_intake.from,
            from_intake.to,
            from_intake.criteria_met,
            from_intake.criteria_total,
            from_intake.pins
        ),
        (0, 3, 1, 2, 1)
    );
    let to_execution = switch(&mut app, 2);
    assert_eq!((to_execution.from, to_execution.to), (3, 2));
    let to_planning = switch(&mut app, 4);
    assert_eq!((to_planning.from, to_planning.to), (2, 4));
    assert_eq!(app.input, drafts);
    assert!(app.execution.active.is_none() && app.execution.prepared.is_none());
    assert!(app.planning.pending.is_none() && !app.planning.busy());
    assert!(app.sources.pending_store.is_none());
}

#[test]
fn mixed_statuses_count_only_open_hypotheses_findings_and_owned_committed_pins() {
    use workflows::EvidenceRelation::{Refutes, Supports};
    let fixture = Fixture::new();
    let mut app = app();
    supported(&mut app, &fixture);
    let source = workflows::EvidenceRef::source(&app.sources.basket[0]).unwrap();
    let run = workflows::EvidenceRef::run(&app.execution.history[0]).unwrap();
    let mut foreign = run.clone();
    if let workflows::EvidenceRef::Run { task_id, .. } = &mut foreign {
        *task_id = 99;
    }
    app.workflows.hypotheses.extend([
        hypothesis(2, vec![link(run.clone(), Refutes)]), // refuted, not open
        hypothesis(
            3,
            vec![
                link(source, Supports),
                link(run.clone(), Supports),
                link(run.clone(), Refutes),
            ],
        ), // conflicting
        hypothesis(4, Vec::new()),
        hypothesis(5, vec![link(foreign, Refutes)]), // another task cannot close it
        hypothesis(
            6,
            vec![link(
                workflows::EvidenceRef::Run {
                    task_id: 7,
                    run_id: "missing".into(),
                },
                Supports,
            )],
        ),
    ]);
    app.workflows.findings = [
        None,
        Some("explicitly resolved".into()),
        Some(String::new()),
    ]
    .into_iter()
    .enumerate()
    .map(|(id, resolution)| workflows::Finding {
        id: id as u64 + 1,
        criterion: "criterion".into(),
        detail: "finding".into(),
        next_action: "review".into(),
        evidence: Vec::new(),
        resolution,
    })
    .collect();
    let mut foreign_pin = fixture.pin();
    foreign_pin.task_id = 99;
    let mut draft_pin = fixture.pin();
    draft_pin.id = 0;
    app.sources.basket.extend([foreign_pin, draft_pin]);
    app.sources.preview = Some(fixture.pin());
    app.criteria = vec![store::Criterion {
        text: "local met is authoritative".into(),
        met: true,
    }];
    let record = switch(&mut app, 5);
    assert_eq!(
        (
            record.open_hypotheses,
            record.open_findings,
            record.pins,
            record.criteria_met
        ),
        (4, 1, 1, 1)
    );
    assert_eq!(app.workflows.hypotheses.len(), 6);
    assert_eq!(app.workflows.findings.len(), 3);
}

#[test]
fn cache_ignores_form_draft_nickname_but_revalidates_fresh_sources_and_run_metadata() {
    let fixture = Fixture::new();
    let mut app = app();
    supported(&mut app, &fixture);
    assert_eq!(switch(&mut app, 5).open_hypotheses, 0);
    fs::write(fixture.0.join("check.rs"), "fn changed() {}\n").unwrap();
    app.input = "draft alone cannot cause source IO".into();
    app.lead.nickname = Some("local-name".into());
    assert!(!app.refresh_carry_over());
    assert_eq!(switch(&mut app, 3).open_hypotheses, 0); // no reread on form alone
    app.repository = Some(fixture.report()); // new observation, identical fingerprint
    assert!(!app.refresh_carry_over());
    assert_eq!(app.last_carry_over().unwrap().open_hypotheses, 0); // prior snapshot remains exact
    assert_eq!(switch(&mut app, 5).open_hypotheses, 1);
    fs::write(fixture.0.join("check.rs"), "fn check() {}\n").unwrap();
    app.sources.basket[0].id = 2; // metadata change makes the old source reference unavailable
    assert_eq!(switch(&mut app, 3).open_hypotheses, 1);
    app.sources.basket[0].id = 1;
    assert_eq!(switch(&mut app, 5).open_hypotheses, 0);
    app.execution.history[0].output_unavailable = true;
    assert_eq!(switch(&mut app, 3).open_hypotheses, 1);
    app.execution.history[0].output_unavailable = false;
    assert_eq!(switch(&mut app, 5).open_hypotheses, 0);
    app.repo_refresh = true; // unknown current run identity cannot support a conclusion
    assert_eq!(switch(&mut app, 3).open_hypotheses, 1);
}

#[test]
fn deliberate_specialist_refresh_revalidates_identical_carry_count_metadata() {
    let fixture = Fixture::new();
    let mut app = app();
    supported(&mut app, &fixture);
    assert_eq!(switch(&mut app, 5).open_hypotheses, 0);
    fs::write(fixture.0.join("check.rs"), "fn changed() {}\n").unwrap();
    app.workflow_refresh();
    assert_eq!(switch(&mut app, 3).open_hypotheses, 1);
    fs::write(fixture.0.join("check.rs"), "fn check() {}\n").unwrap();
    app.workflow_refresh();
    assert_eq!(switch(&mut app, 5).open_hypotheses, 0);
}

#[test]
fn preview_same_form_refusal_and_task_or_lead_restoration_do_not_create_records() {
    let mut app = app();
    app.command("forms");
    key(&mut app, KeyCode::Right);
    assert_eq!(app.form(), 0);
    assert!(!app.refresh_carry_over());
    key(&mut app, KeyCode::Esc);
    app.request_navigation(execution::Action::SelectForm(0));
    assert!(!app.refresh_carry_over());
    assert!(app.last_carry_over().is_none());
    app.memory.saving = true;
    app.request_navigation(execution::Action::SelectForm(3));
    assert_eq!(app.form(), 0);
    assert!(!app.refresh_carry_over());
    app.memory.saving = false;
    app.overlay = None;
    switch(&mut app, 3);
    app.load_task(store::Snapshot {
        id: 8,
        form: 7,
        ..store::Snapshot::empty("/fixture".into())
    });
    assert!(!app.refresh_carry_over());
    assert!(app.last_carry_over().is_none());
    // Lead restoration is independent of the newly loaded task's pending evidence hydration.
    let mut app = self::app();
    switch(&mut app, 5);
    app.party.push(store::Partner {
        id: 2,
        form: 4,
        ..store::Partner::default()
    });
    app.request_lead(2);
    assert_eq!(app.form(), 4);
    assert!(!app.refresh_carry_over());
    assert!(app.last_carry_over().is_none());
}

#[test]
fn next_unicode_key_and_paste_hide_without_losing_text_or_caret() {
    for paste in [false, true] {
        let mut app = app();
        app.input = "앞suffix".into();
        app.cursor = "앞".len();
        app.command("flareon");
        assert!(app.editing && app.overlay.is_none());
        assert!(app.refresh_carry_over());
        let record = app.last_carry_over().unwrap().clone();
        app.update(Event::Resize(100, 30));
        let mut release = KeyEvent::new(KeyCode::Char('한'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert!(!app.update(Event::Key(release)));
        assert!(!app.refresh_carry_over());
        assert_eq!(app.carry_over(), Some(&record));
        if paste {
            app.update(Event::Paste("한🙂".into()));
        } else {
            key(&mut app, KeyCode::Char('한'));
        }
        let inserted = if paste { "한🙂" } else { "한" };
        assert_eq!(app.input, format!("앞{inserted}suffix"));
        assert_eq!(app.cursor, "앞".len() + inserted.len());
        assert!(app.editing && app.carry_over().is_none());
        assert_eq!(app.last_carry_over(), Some(&record));
        let before = (app.input.clone(), app.cursor, app.revision);
        app.command("carry-over");
        assert_eq!((app.input.clone(), app.cursor, app.revision), before);
        assert!(app.editing && app.overlay.is_none());
        assert_eq!(app.carry_over(), Some(&record));
    }
}

#[test]
fn companion_reopen_preserves_exact_record_and_all_work_state() {
    let mut app = app();
    let record = switch(&mut app, 3);
    app.input = "keep suffix".into();
    app.cursor = 4;
    app.command("companion");
    let before = (
        app.workflows.clone(),
        app.sources.basket.clone(),
        app.planning.pending.is_some(),
        app.execution.prepared.is_some(),
    );
    key(&mut app, KeyCode::Char('c'));
    assert_eq!(app.carry_over(), Some(&record));
    assert!(app.editing && app.overlay.is_none());
    assert_eq!((app.input.as_str(), app.cursor), ("keep suffix", 4));
    assert_eq!(
        (
            app.workflows.clone(),
            app.sources.basket.clone(),
            app.planning.pending.is_some(),
            app.execution.prepared.is_some()
        ),
        before
    );
    app.retain_editor();
    app.focus = Pane::Companion;
    key(&mut app, KeyCode::Char('c'));
    assert_eq!(app.carry_over(), Some(&record));
    assert!(app.execution.active.is_none() && app.planning.pending.is_none());
}

#[test]
fn all_capability_pairs_fit_the_narrow_strip_at_maximum_stored_counts() {
    for from in 0..CAPABILITIES.len() {
        for to in 0..CAPABILITIES.len() {
            let record = CarryOver {
                from,
                to,
                criteria_met: 64,
                criteria_total: 64,
                pins: context::MAX_EXCERPTS,
                open_hypotheses: workflows::MAX_ENTRIES,
                open_findings: workflows::MAX_ENTRIES,
                summary: String::new(),
            };
            let text = summary(&record);
            assert!(text.is_ascii());
            assert!(text.len() <= 56, "{from}->{to}: {text}");
        }
    }
}

#[test]
fn cancelled_execution_navigation_never_records_a_switch() {
    let fixture = Fixture::new();
    let mut app = app();
    supported(&mut app, &fixture);
    let prepared = app.execution.history[0].prepared.clone();
    app.execution.prepared = Some(prepared.clone());
    app.request_navigation(execution::Action::SelectForm(3));
    assert_eq!(app.overlay, Some(Overlay::Execution));
    assert_eq!(app.form(), 0);
    assert!(!app.refresh_carry_over());
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.form(), 0);
    assert_eq!(app.execution.prepared.as_ref(), Some(&prepared));
    assert!(app.last_carry_over().is_none());
}

#[test]
fn switch_key_survives_input_hide_and_snapshot_waits_for_settlement() {
    let mut app = app();
    switch(&mut app, 3);
    app.command("forms");
    // Navigate to the adjacent Espeon and accept via a real key event.
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.form(), 4);
    assert!(app.carry_over().is_none());
    // Settlement sees hydrated committed/local state, not the old cached card.
    app.criteria.push(store::Criterion {
        text: "settled".into(),
        met: true,
    });
    assert!(app.refresh_carry_over());
    assert_eq!(app.carry_over().unwrap().criteria_met, 1);
    assert_eq!(
        (app.carry_over().unwrap().from, app.carry_over().unwrap().to),
        (3, 4)
    );
    // Even ignored real input hides, unlike resize/release/background settlement.
    app.update(Event::Paste(String::new()));
    assert!(app.carry_over().is_none());
    assert!(!app.refresh_carry_over());
}

#[test]
fn new_task_clears_the_prior_record_after_durable_acknowledgment() {
    let mut app = app();
    switch(&mut app, 3);
    let snapshot = app.snapshot("/fixture");
    app.store_event(store::Event::Saved(snapshot));
    app.apply_execution_action(execution::Action::NewTask);
    assert_eq!(app.task_id, 0);
    assert_eq!(app.form(), 0);
    assert!(app.last_carry_over().is_none());
    assert!(!app.refresh_carry_over());
}

#[test]
fn loading_context_does_not_publish_zero_pins_or_lose_the_next_input() {
    let fixture = Fixture::new();
    let mut app = app();
    supported(&mut app, &fixture);
    let captures = app.sources.basket.clone();
    app.sources.reset(7);
    app.begin_edit_field(store::EditorField::Request);
    app.update(Event::Paste("kept".into()));
    key(&mut app, KeyCode::Home);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Right);
    app.command("flareon");
    assert_eq!(app.form(), 3);
    assert!(!app.refresh_carry_over());
    assert!(app.last_carry_over().is_none());
    key(&mut app, KeyCode::Char('界'));
    assert_eq!(app.input, "ke界pt");
    assert_eq!(app.cursor, "ke界".len());
    app.sources.stored(7, 7, Ok(captures));
    assert!(app.refresh_carry_over());
    assert!(
        app.carry_over().is_none(),
        "late settlement must not reshow a hidden strip"
    );
    assert_eq!(app.last_carry_over().unwrap().pins, 1);
    app.reopen_carry_over();
    assert_eq!(app.carry_over().unwrap().pins, 1);
    assert_eq!(app.input, "ke界pt");
    assert_eq!(app.cursor, "ke界".len());
}

#[test]
fn loading_run_history_does_not_classify_a_supported_hypothesis_as_open() {
    let fixture = Fixture::new();
    let mut app = app();
    supported(&mut app, &fixture);
    let records = app.execution.history.clone();
    app.execution.open(7);
    app.request_navigation(execution::Action::SelectForm(3));
    assert!(!app.refresh_carry_over());
    let _load = app.execution.take_store_request().unwrap();
    assert!(!app.refresh_carry_over());
    app.execution
        .store_event(7, None, None, Ok(store::RunResponse::Runs(records)));
    assert!(app.refresh_carry_over());
    assert_eq!(app.carry_over().unwrap().open_hypotheses, 0);
    assert_eq!(app.carry_over().unwrap().pins, 1);
}
