use super::*;
use crate::store;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

fn report(revision: u8, dirty: bool) -> Arc<repo::Report> {
    let path = PathBuf::from("check.rs");
    let snapshot = Arc::new(repo::Snapshot {
        head: Some("fixture-head".into()),
        files: BTreeMap::from([(path.clone(), repo::FileIdentity {
            path,
            tracked: true,
            head: Some(repo::ObjectIdentity { oid: "fixture-object".into(), mode: "100644".into() }),
            index: Vec::new(),
            state: repo::FileState::Regular {
                sha256: [revision; 32], bytes: 20, binary: false, executable: false,
            },
            dirty: Some(dirty),
        })]),
        // Real clean reports have a heading too; nonempty text is not a diff.
        working_diff: repo::WorkingDiff {
            text: "Raw HEAD → working tree diff; no filters/textconv. Index identities are recorded separately.\n".into(),
            ..Default::default()
        },
        complete: true,
        stable: true,
        warnings: Vec::new(),
    });
    Arc::new(repo::Report {
        repository: repo::RepositoryIdentity {
            root: PathBuf::from("/fixture"),
            git_dir: PathBuf::from("/fixture/.git"),
        },
        baseline: snapshot.clone(),
        current: snapshot,
        changes: Vec::new(),
        initial: true,
    })
}

fn pin() -> context::Excerpt {
    context::Excerpt {
        origin_run: None,
        id: 1,
        task_id: 7,
        root: "/fixture".into(),
        path: "check.rs".into(),
        start_line: 1,
        end_line: 1,
        source_sha256: [1; 32],
        text: "fn check() {}\n".into(),
        available: true,
    }
}

fn app() -> App {
    let mut app = App::new("/fixture".into(), 120, 36);
    app.task_id = 7;
    app.goal = "Deliver the requested change".into();
    app.criteria = vec![store::Criterion {
        text: "The check passes".into(),
        met: false,
    }];
    app.sources.basket = vec![pin()];
    app.repository = Some(report(1, true));
    app.repo_loading = false;
    app.repo_error.clear();
    app
}

// Explicit persisted-journal fixtures, not claims that a process was executed.
fn run(app: &App, id: &str, start: u64) -> runner::RunRecord {
    let current = execution::fingerprint(app.repository.as_deref().unwrap());
    let prepared = runner::Prepared {
        request_id: format!("request-{id}"),
        run_id: id.into(),
        task_id: app.task_id,
        spec: runner::CommandSpec::Direct {
            program: "cargo".into(),
            args: vec!["test".into()],
        },
        cwd: PathBuf::from("/fixture"),
        cwd_identity: "fixture-directory".into(),
        executable: runner::ExecutableIdentity {
            path: PathBuf::from("/fixture/cargo"),
            sha256: "fixture-executable".into(),
            bytes: 1,
            modified_ns: 1,
            file_id: "fixture".into(),
        },
        prepared_at_ms: start,
        expires_at_ms: start + 1000,
        before: current.clone(),
        partner: None,
        rerun_of: None,
    };
    let mut run = runner::RunRecord::awaiting(&prepared);
    run.state = runner::RunState::Succeeded;
    run.started_at_ms = Some(start);
    run.ended_at_ms = Some(start + 10);
    run.exit_code = Some(0);
    run.tree = runner::TreeOutcome::OwnedTreeExited;
    run.after = current;
    run
}

fn failed(app: &App, id: &str, start: u64) -> runner::RunRecord {
    let mut record = run(app, id, start);
    record.state = runner::RunState::Failed;
    record.exit_code = Some(17);
    record
}

fn assert_form(app: &mut App, expected: Option<usize>) {
    app.refresh_suggestion();
    assert_eq!(
        app.suggestion.as_ref().map(|suggestion| suggestion.form),
        expected
    );
}

#[test]
fn ordered_rules_cover_goal_criteria_failure_diff_pass_and_no_match() {
    let mut app = app();
    app.criteria.clear();
    app.sources.basket.clear();
    app.execution.history = vec![failed(&app, "failed", 1)];
    assert_form(&mut app, Some(4)); // goal outranks missing pins, failure, and diff
    app.criteria.push(store::Criterion {
        text: "The check passes".into(),
        met: false,
    });
    assert_form(&mut app, Some(1)); // criteria outrank failure and diff
    app.sources.basket.push(pin());
    assert_form(&mut app, Some(5)); // failure outranks diff
    assert!(
        app.suggestion
            .as_ref()
            .unwrap()
            .reason
            .contains("[\"cargo\",\"test\"]")
    );
    assert!(app.suggestion.as_ref().unwrap().reason.contains("exit 17"));
    app.execution.history.clear();
    assert_form(&mut app, Some(2)); // pre-existing dirt, with no baseline changes
    app.execution.history.push(run(&app, "pass", 2));
    assert_form(&mut app, Some(7)); // current complete pass outranks observed diff
    app.repository = Some(report(1, false));
    app.execution.history.clear();
    assert_form(&mut app, None); // clean observation, no evidence to recommend
    app.goal.clear();
    app.criteria.clear();
    app.sources.basket.clear();
    assert_form(&mut app, None);
}

#[test]
fn only_committed_current_task_pins_and_local_runs_count() {
    let mut app = app();
    app.sources.basket[0].task_id = 99;
    app.sources.preview = Some(pin());
    assert_form(&mut app, Some(1));
    app.sources.basket[0] = pin();
    app.sources.basket[0].id = 0;
    assert_form(&mut app, Some(1));
    app.sources.basket[0] = pin();
    let mut other_task_failure = failed(&app, "other-task", 100);
    other_task_failure.prepared.task_id = 99;
    app.execution.history = vec![other_task_failure.clone()];
    app.execution.active = Some(other_task_failure);
    assert_form(&mut app, Some(2));
    let mut other_task_pass = run(&app, "other-task-pass", 200);
    other_task_pass.prepared.task_id = 99;
    app.execution.history = vec![other_task_pass];
    app.execution.active = None;
    assert_form(&mut app, Some(2));
    app.execution.history.push(run(&app, "owned", 1));
    assert_form(&mut app, Some(7));
}

#[test]
fn newest_first_history_and_newer_active_record_determine_run_rule() {
    let mut app = app();
    // Deliberately tied timestamps: newest persisted row, not timestamp sorting.
    app.execution.history = vec![run(&app, "new-pass", 1), failed(&app, "old-fail", 1)];
    app.execution.history_index = 1;
    assert_form(&mut app, Some(7));
    app.execution.active = Some(failed(&app, "new-active-fail", 2));
    assert_form(&mut app, Some(5));
    app.execution.active = Some(app.execution.history[1].clone());
    assert_form(&mut app, Some(7)); // old active cannot supersede a newer persisted row
    let mut running = run(&app, "new-active-running", 3);
    running.state = runner::RunState::Running;
    running.ended_at_ms = None;
    running.exit_code = None;
    app.execution.active = Some(running);
    assert_form(&mut app, Some(2)); // don't reuse an older pass over a newer active run
    app.execution.active = None;
    app.execution.history.swap(0, 1);
    assert_form(&mut app, Some(5));
}

#[test]
fn unknown_cancelled_interrupted_and_partial_evidence_never_become_a_pass() {
    let mut app = app();
    for state in [
        runner::RunState::OutcomeUnknown,
        runner::RunState::Cancelled,
        runner::RunState::Interrupted,
        runner::RunState::AwaitingDispatch,
        runner::RunState::Running,
        runner::RunState::Cancelling,
    ] {
        let mut record = run(&app, "nonterminal-or-unknown", 2);
        record.state = state;
        // Even an apparent exit zero cannot override actual journal state.
        app.execution.history = vec![record, run(&app, "older-pass", 1)];
        assert_form(&mut app, Some(2));
    }
    let partials: &[fn(&mut runner::RunRecord)] = &[
        |run| run.truncated = true,
        |run| run.output_unavailable = true,
        |run| run.discarded_bytes = 1,
        |run| run.cancelled = true,
        |run| run.tree = runner::TreeOutcome::OwnershipUnknown,
        |run| run.started_at_ms = None,
        |run| run.ended_at_ms = None,
        |run| run.ended_at_ms = Some(0),
        |run| run.exit_code = None,
        |run| run.exit_code = Some(1),
        |run| run.signal = Some(9),
        |run| run.error = Some("capture failed".into()),
        |run| run.prepared.before.complete = false,
        |run| run.after.stable = false,
        |run| run.after.fingerprint = Some("changed-during-run".into()),
    ];
    for (index, mutate) in partials.iter().enumerate() {
        let mut record = run(&app, &format!("partial-{index}"), 2);
        mutate(&mut record);
        app.execution.history = vec![record, run(&app, "older-pass", 1)];
        assert_form(&mut app, Some(2));
    }
}

#[test]
fn failure_reason_uses_only_observed_exit_signal_and_error() {
    let mut app = app();
    let mut record = failed(&app, "signal", 1);
    record.exit_code = None;
    record.signal = Some(9);
    app.execution.history = vec![record];
    assert_form(&mut app, Some(5));
    let reason = &app.suggestion.as_ref().unwrap().reason;
    assert!(reason.contains("signal 9"));
    assert!(!reason.contains("exit"));
    app.execution.history[0].signal = None;
    app.execution.history[0].error = Some("spawn refused\npermission denied".into());
    assert_form(&mut app, Some(5));
    let reason = &app.suggestion.as_ref().unwrap().reason;
    assert!(reason.contains("spawn refused\\npermission denied"));
    assert!(!reason.contains("signal"));
    assert!(!reason.contains("exit"));
    app.execution.history[0].error = None;
    assert_form(&mut app, Some(5));
    assert!(
        app.suggestion
            .as_ref()
            .unwrap()
            .reason
            .contains("outcome details unavailable")
    );
}

#[test]
fn source_revision_change_invalidates_current_pass_and_cache() {
    let mut app = app();
    app.execution.history = vec![run(&app, "pass", 1)];
    assert!(app.refresh_suggestion());
    assert_eq!(app.suggestion.as_ref().unwrap().form, 7);
    assert!(!app.refresh_suggestion());
    let first = app.suggestion.clone();
    app.activity_frame += 1;
    app.input = "Uncommitted alternative goal".into();
    app.cursor = app.input.len();
    app.command = "Uncommitted alternative command".into();
    app.lead.nickname = Some("New nickname".into());
    app.lead.form = 5;
    app.drafts.push(store::Draft {
        field: store::EditorField::Goal,
        text: "draft".into(),
        cursor: 5,
    });
    assert!(!app.refresh_suggestion());
    assert_eq!(app.suggestion, first);
    app.repository = Some(report(2, true));
    assert!(app.refresh_suggestion());
    assert_eq!(app.suggestion.as_ref().unwrap().form, 2);
    assert!(!app.refresh_suggestion());
}

#[test]
fn new_failure_changes_trigger_but_identity_presentation_does_not() {
    let mut app = app();
    app.execution.history = vec![failed(&app, "failure-one", 1)];
    assert_form(&mut app, Some(5));
    let first = app.suggestion.as_ref().unwrap().trigger.clone();
    app.lead.nickname = Some("Night".into());
    app.lead.form = 8;
    app.input = "draft goal".into();
    app.command = "draft command".into();
    assert!(!app.refresh_suggestion());
    assert_eq!(app.suggestion.as_ref().unwrap().trigger, first);
    app.execution
        .history
        .insert(0, failed(&app, "failure-two", 2));
    assert!(app.refresh_suggestion());
    assert_ne!(app.suggestion.as_ref().unwrap().trigger, first);
}

#[test]
fn incomplete_unstable_or_unavailable_observations_cannot_confirm_success() {
    for kind in 0..5 {
        let mut app = app();
        app.execution.history = vec![run(&app, "pass", 1)];
        assert_form(&mut app, Some(7));
        match kind {
            0 => {
                let report = Arc::make_mut(app.repository.as_mut().unwrap());
                Arc::make_mut(&mut report.current).complete = false;
            }
            1 => {
                let report = Arc::make_mut(app.repository.as_mut().unwrap());
                Arc::make_mut(&mut report.current).stable = false;
            }
            2 => app.repo_error = "Repository refresh unavailable".into(),
            3 => app.repo_loading = true,
            _ => app.repository = None,
        }
        app.refresh_suggestion();
        assert_ne!(
            app.suggestion.as_ref().map(|suggestion| suggestion.form),
            Some(7)
        );
        assert_ne!(
            app.suggestion.as_ref().map(|suggestion| suggestion.form),
            Some(8)
        );
    }
}

static SEQUENCE: AtomicU64 = AtomicU64::new(1);
struct SourceFixture(PathBuf);
impl SourceFixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "eevee-suggestion-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        Self(fs::canonicalize(path).unwrap())
    }

    fn pin(&self) -> context::Excerpt {
        fs::write(self.0.join("check.rs"), "fn check() {}\n").unwrap();
        let (bytes, hash) = repo::read_source(
            &self.0,
            Path::new("check.rs"),
            context::MAX_SOURCE_BYTES,
            &AtomicBool::new(false),
        )
        .unwrap();
        context::Excerpt {
            origin_run: None,
            root: self.0.to_str().unwrap().into(),
            text: String::from_utf8(bytes).unwrap(),
            source_sha256: hash,
            ..pin()
        }
    }
}
impl Drop for SourceFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn reviewed(app: &mut App) {
    let current = execution::fingerprint(app.repository.as_deref().unwrap());
    let view = workflows::EvidenceView {
        task_id: app.task_id,
        excerpts: &app.sources.basket,
        runs: &app.execution.history,
        current: &current,
    };
    app.workflows
        .record_review(
            workflows::Verdict::Accept,
            vec![workflows::CriterionEvidence {
                criterion: app.criteria[0].text.clone(),
                evidence: vec![
                    workflows::EvidenceRef::source(&app.sources.basket[0]).unwrap(),
                    workflows::EvidenceRef::run(&app.execution.history[0]).unwrap(),
                ],
            }],
            &view,
        )
        .unwrap();
    app.workflow_refresh();
}

#[test]
fn accepted_review_reads_real_sources_and_rechecks_only_on_relevant_settlement() {
    let fixture = SourceFixture::new();
    let mut app = app();
    app.sources.basket = vec![fixture.pin()];
    app.execution.history = vec![run(&app, "pass", 1)];
    reviewed(&mut app);
    assert_form(&mut app, Some(8));
    let accepted = app.suggestion.clone();
    fs::write(fixture.0.join("check.rs"), "fn changed_after_review() {}\n").unwrap();
    app.input = "draft does not refresh observed sources".into();
    app.activity_frame += 1;
    assert!(!app.refresh_suggestion()); // proves the source guard was not reread
    assert_eq!(app.suggestion, accepted);
    // Explicit /refresh yields a new report, even if its fingerprint is unchanged.
    app.repository = Some(Arc::new((**app.repository.as_ref().unwrap()).clone()));
    assert!(app.refresh_suggestion());
    assert_eq!(app.suggestion, None); // stale source must not become Sylveon
    app.sources.basket = vec![fixture.pin()];
    reviewed(&mut app);
    assert_form(&mut app, Some(8));
    fs::remove_file(fixture.0.join("check.rs")).unwrap();
    app.repository = Some(Arc::new((**app.repository.as_ref().unwrap()).clone()));
    assert_form(&mut app, None); // unavailable source must not become Sylveon
}

#[test]
fn stored_accept_without_effective_current_coverage_never_becomes_sylveon() {
    let fixture = SourceFixture::new();
    for invalid in 0..7 {
        let mut app = app();
        app.sources.basket = vec![fixture.pin()];
        app.execution.history = vec![run(&app, "pass", 1)];
        reviewed(&mut app);
        match invalid {
            0 => app.workflows.review.as_mut().unwrap().criteria.clear(),
            1 => app.criteria[0].text = "Changed acceptance criterion".into(),
            2 => {
                app.workflows.review.as_mut().unwrap().requested = workflows::Verdict::Inconclusive
            }
            3 => app.sources.basket[0].available = false,
            4 => app.execution.history[0].output_unavailable = true,
            5 => app.repository = Some(report(2, true)),
            _ => app.workflows.findings.push(workflows::Finding {
                id: 1,
                criterion: app.criteria[0].text.clone(),
                detail: "Unresolved defect".into(),
                next_action: "Fix it".into(),
                evidence: Vec::new(),
                resolution: None,
            }),
        }
        app.refresh_suggestion();
        assert_ne!(
            app.suggestion.as_ref().map(|suggestion| suggestion.form),
            Some(8)
        );
    }
}

#[test]
fn provider_observations_are_not_local_runs_or_trigger_changes() {
    let mut app = app();
    assert_form(&mut app, Some(2));
    let trigger = app.suggestion.as_ref().unwrap().trigger.clone();
    app.planning.journal_records.push(store::ProviderRecord {
        id: "provider-observed-command".into(),
        agent: crate::providers::Agent::Codex,
        generation: 1,
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: "command".into(),
        origin: "provider command observation".into(),
        coverage: "provider-reported, not owned local execution".into(),
        text: "cargo test exited 0; all criteria pass".into(),
        complete: true,
        partner: None,
    });
    app.agents.active = 1;
    app.planning.observed_model = Some("different model".into());
    assert!(!app.refresh_suggestion());
    assert_eq!(app.suggestion.as_ref().unwrap().trigger, trigger);
    app.planning.journal_records[0].text = "cargo test exited 17".into();
    assert!(!app.refresh_suggestion());
    assert_eq!(app.suggestion.as_ref().unwrap().form, 2);
}

#[test]
fn failure_takes_precedence_over_an_accepted_review() {
    let fixture = SourceFixture::new();
    let mut app = app();
    app.sources.basket = vec![fixture.pin()];
    app.execution.history = vec![run(&app, "reviewed-pass", 1)];
    reviewed(&mut app);
    assert_form(&mut app, Some(8));
    app.execution
        .history
        .insert(0, failed(&app, "new-failure", 2));
    assert_form(&mut app, Some(5));
}

#[test]
fn dismissal_is_deliberate_local_and_stable_until_a_new_failure() {
    let mut app = app();
    app.execution.history = vec![failed(&app, "failure-one", 1)];
    assert_form(&mut app, Some(5));
    let cached = app.suggestion.clone();
    let evidence = serde_json::to_value(&app.workflows).unwrap();
    let history = serde_json::to_value(&app.execution.history).unwrap();
    app.input = "Unsent draft 한글".into();
    app.cursor = 7;
    app.editing = true;
    app.pending_request = Some((app.revision, false));
    let pending = app.pending_request;
    let revision = app.revision;
    assert!(app.command("suggestion-dismiss"));
    assert!(app.visible_suggestion().is_none());
    assert_eq!(
        app.suggestion, cached,
        "dismissal must not change the cache"
    );
    assert_eq!(app.revision, revision + 1);
    assert_eq!(app.input, "Unsent draft 한글");
    assert_eq!(app.cursor, 7);
    assert!(app.editing);
    assert_eq!(app.pending_request, pending);
    assert_eq!(app.form(), 0);
    assert!(app.pending_command.is_none());
    assert!(app.planning.pending.is_none());
    assert_eq!(serde_json::to_value(&app.workflows).unwrap(), evidence);
    assert_eq!(
        serde_json::to_value(&app.execution.history).unwrap(),
        history
    );
    app.lead.nickname = Some("Night".into());
    app.set_form(8);
    app.input = "Different unsent draft".into();
    assert!(!app.refresh_suggestion());
    assert!(app.visible_suggestion().is_none());
    app.execution
        .history
        .insert(0, failed(&app, "failure-two", 2));
    assert!(app.refresh_suggestion());
    assert_eq!(app.visible_suggestion().unwrap().form, 5);
    assert_ne!(
        app.visible_suggestion().unwrap().trigger,
        app.dismissed_suggestion.as_deref().unwrap()
    );
}

#[test]
fn dismissal_restores_only_with_its_task_and_new_task_clears_it() {
    let mut original = app();
    original.execution.history = vec![failed(&original, "failure-one", 1)];
    original.refresh_suggestion();
    assert!(original.dismiss_suggestion());
    let snapshot = original.snapshot("/fixture");
    let navigation = serde_json::to_string(&snapshot.navigation).unwrap();
    assert!(navigation.contains("dismissed_suggestion"));
    let mut restored = app();
    restored.load_task(snapshot.clone());
    restored.execution.history = original.execution.history.clone();
    restored.refresh_suggestion();
    assert!(restored.visible_suggestion().is_none());
    assert_eq!(restored.dismissed_suggestion, original.dismissed_suggestion);
    let mut other = snapshot.clone();
    other.id = 8;
    other.navigation.dismissed_suggestion = None;
    restored.load_task(other);
    restored.execution.history = vec![failed(&restored, "failure-one", 1)];
    restored.refresh_suggestion();
    assert!(restored.visible_suggestion().is_some());
    restored.load_task(snapshot);
    restored.execution.history = original.execution.history.clone();
    restored.refresh_suggestion();
    assert!(restored.visible_suggestion().is_none());
    restored.finish_new_task();
    assert_eq!(restored.task_id, 0);
    assert!(restored.dismissed_suggestion.is_none());
    restored.refresh_suggestion();
    assert!(restored.visible_suggestion().is_none());
}

#[test]
fn dismissal_controls_respect_cancel_editor_review_size_and_empty_state() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let key = |app: &mut App, code| {
        app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    };
    let mut app = app();
    app.execution.history = vec![failed(&app, "failure-one", 1)];
    app.refresh_suggestion();
    app.command("forms");
    app.selected = 4;
    key(&mut app, KeyCode::Esc);
    assert!(
        app.dismissed_suggestion.is_none(),
        "closing picker is neutral"
    );
    app.command("forms");
    app.execution.confirmation = Some(execution::Confirmation::Approval);
    assert!(!app.dismiss_suggestion());
    assert!(app.execution.confirmation.is_some());
    assert!(app.dismissed_suggestion.is_none());
    app.execution.confirmation = None;
    app.width = 59;
    key(&mut app, KeyCode::Char('d'));
    assert!(app.dismissed_suggestion.is_none());
    app.width = 120;
    app.editing = true;
    app.input.clear();
    app.cursor = 0;
    key(&mut app, KeyCode::Char('d'));
    assert_eq!(app.input, "d");
    assert!(app.dismissed_suggestion.is_none());
    app.editing = false;
    app.form_return_selected = true;
    key(&mut app, KeyCode::Char('d'));
    assert!(app.visible_suggestion().is_none());
    assert!(app.form_return_selected);
    assert_eq!(app.selected, 4);
    assert_eq!(app.form(), 0);
    let revision = app.revision;
    assert!(!app.dismiss_suggestion());
    assert_eq!(app.revision, revision);
    app.suggestion = None;
    app.dismissed_suggestion = None;
    assert!(!app.dismiss_suggestion());
    assert!(app.dismissed_suggestion.is_none());
    assert_eq!(app.revision, revision);
}

#[test]
fn idle_partner_hint_tracks_owned_work_without_changing_the_evidence_trigger() {
    let mut app = app();
    app.party.push(store::Partner {
        id: 2,
        form: 5,
        nickname: Some("Mizu".into()),
        ..Default::default()
    });
    let mut failed = run(&app, "failed-owned", 1);
    failed.state = runner::RunState::Failed;
    failed.exit_code = Some(7);
    app.execution.history = vec![failed];
    app.refresh_suggestion();
    let trigger = app.visible_suggestion().unwrap().trigger.clone();
    assert_eq!(app.suggested_partner().map(|partner| partner.id), Some(2));
    let owner = store::Attribution::of(app.party.iter().find(|partner| partner.id == 2).unwrap());
    assert!(
        app.execution
            .prepare_command_for(app.task_id, Some(owner), r#"["tool","held"]"#)
    );
    assert!(app.suggested_partner().is_none());
    assert_eq!(app.visible_suggestion().unwrap().trigger, trigger);
    app.execution.invalidate();
    assert_eq!(app.suggested_partner().map(|partner| partner.id), Some(2));
    app.dismissed_suggestion = Some(trigger);
    assert!(app.suggested_partner().is_none());
}
