use super::*;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "eevee-workflow-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        Self(fs::canonicalize(root).unwrap())
    }

    fn source(&self) -> context::Excerpt {
        fs::write(self.0.join("bug.rs"), "fn bug() { panic!(\"seeded\"); }\n").unwrap();
        let (bytes, hash) = repo::read_source(
            &self.0,
            Path::new("bug.rs"),
            context::MAX_SOURCE_BYTES,
            &AtomicBool::new(false),
        )
        .unwrap();
        context::Excerpt {
            origin_run: None,
            id: 1,
            task_id: 7,
            root: self.0.to_str().unwrap().into(),
            path: "bug.rs".into(),
            start_line: 1,
            end_line: 1,
            source_sha256: hash,
            text: String::from_utf8(bytes).unwrap(),
            available: true,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn snapshot(identity: &str) -> runner::InputSnapshot {
    runner::InputSnapshot {
        fingerprint: Some(identity.into()),
        complete: true,
        stable: true,
    }
}

// Explicit journal fixtures exercise consumer refusal, not live-run acceptance.
fn record(id: &str, identity: &str, start: u64) -> runner::RunRecord {
    let prepared = runner::Prepared {
        request_id: format!("request-{id}"),
        run_id: id.into(),
        task_id: 7,
        spec: runner::CommandSpec::Direct {
            program: "check".into(),
            args: vec!["behavior".into()],
        },
        cwd: PathBuf::from("/fixture"),
        cwd_identity: "fixture-directory".into(),
        executable: runner::ExecutableIdentity {
            path: PathBuf::from("/fixture/check"),
            sha256: "fixture-executable".into(),
            bytes: 1,
            modified_ns: 1,
            file_id: "fixture".into(),
        },
        prepared_at_ms: start,
        expires_at_ms: start + 1000,
        approval_arrival: 0,
        before: snapshot(identity),
        partner: None,
        rerun_of: None,
    };
    let mut run = runner::RunRecord::awaiting(&prepared);
    run.state = runner::RunState::Succeeded;
    run.started_at_ms = Some(start);
    run.ended_at_ms = Some(start + 10);
    run.exit_code = Some(0);
    run.tree = runner::TreeOutcome::OwnedTreeExited;
    run.after = snapshot(identity);
    run
}

fn view<'a>(
    excerpts: &'a [context::Excerpt],
    runs: &'a [runner::RunRecord],
    current: &'a runner::InputSnapshot,
) -> EvidenceView<'a> {
    EvidenceView {
        task_id: 7,
        excerpts,
        runs,
        current,
    }
}

fn recipe() -> &'static str {
    r#"schema_version = 1
name = "Local bug fix"
scope = ["bug.rs"]
context = ["bug.rs"]
validation_commands = [["cargo", "test", "seeded"]]
completion_criteria = ["The seeded check passes"]
[[steps]]
form = "Umbreon"
action = "Reproduce and diagnose"
[[steps]]
form = "Flareon"
action = "Request a bounded fix through normal approval"
[[steps]]
form = "Jolteon"
action = "Request normal approval for the validation command"
"#
}

#[test]
fn hypotheses_need_actual_task_links_and_missing_proof_remains_unresolved() {
    let fixture = Fixture::new();
    let excerpts = [fixture.source()];
    let mut failed = record("reproduce", "before", 1);
    failed.state = runner::RunState::Failed;
    failed.exit_code = Some(1);
    let runs = [failed];
    let current = snapshot("before");
    let evidence = view(&excerpts, &runs, &current);
    let mut state = State::default();
    let wrong = state.add_hypothesis("Network instability").unwrap();
    let cause = state
        .add_hypothesis("The source explicitly panics")
        .unwrap();
    assert_eq!(
        state.hypothesis_status(cause, &evidence),
        HypothesisStatus::Unresolved
    );
    let source = EvidenceRef::source(&excerpts[0]).unwrap();
    state
        .attach_hypothesis(cause, EvidenceRelation::Supports, source, &evidence)
        .unwrap();
    assert_eq!(
        state.hypothesis_status(cause, &evidence),
        HypothesisStatus::Unresolved
    );
    let run = EvidenceRef::run(&runs[0]).unwrap();
    state
        .attach_hypothesis(cause, EvidenceRelation::Supports, run.clone(), &evidence)
        .unwrap();
    state
        .attach_hypothesis(wrong, EvidenceRelation::Refutes, run, &evidence)
        .unwrap();
    assert_eq!(
        state.hypothesis_status(cause, &evidence),
        HypothesisStatus::Supported
    );
    assert_eq!(
        state.hypothesis_status(wrong, &evidence),
        HypothesisStatus::Refuted
    );
    let foreign = EvidenceRef::Run {
        task_id: 9,
        run_id: "reproduce".into(),
    };
    assert!(
        state
            .attach_hypothesis(cause, EvidenceRelation::Supports, foreign, &evidence)
            .is_err()
    );
    assert_eq!(
        state.hypothesis_status(cause, &view(&excerpts, &[], &current)),
        HypothesisStatus::Unresolved
    );
    fs::write(fixture.0.join("bug.rs"), "fn bug() {}\n").unwrap();
    assert_eq!(
        state.hypothesis_status(cause, &evidence),
        HypothesisStatus::Unresolved
    );
}

#[test]
fn refactor_compares_only_same_ordered_check_and_current_post_source() {
    let fixture = Fixture::new();
    let excerpts = [fixture.source()];
    let mut runs = [record("baseline", "before", 1), record("post", "after", 20)];
    let before = snapshot("before");
    let after = snapshot("after");
    let mut state = State::default();
    state
        .set_refactor(
            "The selected behavior check must still pass",
            vec![EvidenceRef::source(&excerpts[0]).unwrap()],
            "baseline",
            &view(&excerpts, &runs, &before),
        )
        .unwrap();
    state
        .set_post_run("post", &view(&excerpts, &runs, &after))
        .unwrap();
    let comparison = state.compare_refactor(&view(&excerpts, &runs, &after));
    assert!(comparison.behavior_preserved);
    assert!(comparison.reason.contains("do not establish a speedup"));
    assert_eq!(comparison.baseline_ms, Some(10));
    assert!(
        !state
            .compare_refactor(&view(&excerpts, &runs, &snapshot("later")))
            .behavior_preserved
    );
    runs[1].state = runner::RunState::Failed;
    runs[1].exit_code = Some(1);
    assert!(
        !state
            .compare_refactor(&view(&excerpts, &runs, &after))
            .behavior_preserved
    );
    runs[1] = record("post", "after", 20);
    runs[1].prepared.spec = runner::CommandSpec::Direct {
        program: "true".into(),
        args: vec![],
    };
    assert!(
        !state
            .compare_refactor(&view(&excerpts, &runs, &after))
            .behavior_preserved
    );
    runs[1] = record("post", "after", 20);
    runs[1].after = snapshot("changed-during-run");
    assert!(
        !state
            .compare_refactor(&view(&excerpts, &runs, &after))
            .behavior_preserved
    );
    runs[1] = record("post", "after", 20);
    runs[0].output_unavailable = true;
    assert!(
        !state
            .compare_refactor(&view(&excerpts, &runs, &after))
            .behavior_preserved
    );
}

#[test]
fn review_accept_is_guarded_on_every_consumer_and_survives_restart_honestly() {
    let fixture = Fixture::new();
    let excerpts = [fixture.source()];
    let runs = [record("check", "before", 1)];
    let current = snapshot("before");
    let evidence = view(&excerpts, &runs, &current);
    let mut state = State::default();
    let criteria = vec!["No seeded panic".to_string()];
    let refs = vec![
        EvidenceRef::source(&excerpts[0]).unwrap(),
        EvidenceRef::run(&runs[0]).unwrap(),
    ];
    state
        .record_review(
            Verdict::Accept,
            vec![CriterionEvidence {
                criterion: criteria[0].clone(),
                evidence: refs.clone(),
            }],
            &evidence,
        )
        .unwrap();
    assert_eq!(
        state.review_guard(&criteria, &evidence).verdict,
        Verdict::Accept
    );
    let id = state
        .add_finding(
            &criteria[0],
            "Source still panics",
            "Hand off a bounded fix to Flareon",
            refs,
            &evidence,
        )
        .unwrap();
    assert_eq!(
        state.review_guard(&criteria, &evidence).verdict,
        Verdict::RequestChanges
    );
    let saved = serde_json::to_string(&state).unwrap();
    let mut restored: State = serde_json::from_str(&saved).unwrap();
    restored.validate(7).unwrap();
    assert_eq!(
        restored.findings[0].next_action,
        "Hand off a bounded fix to Flareon"
    );
    assert_eq!(
        restored.review_guard(&criteria, &evidence).verdict,
        Verdict::RequestChanges
    );
    restored
        .resolve_finding(id, "User reviewed the resolution")
        .unwrap();
    fs::write(fixture.0.join("bug.rs"), "fn bug() {}\n").unwrap();
    let guard = restored.review_guard(&criteria, &evidence);
    assert_eq!(guard.verdict, Verdict::Inconclusive);
    assert!(guard.reasons.iter().any(|reason| reason.contains("Stale")));
    assert_eq!(
        restored
            .review_guard(&["A different requirement".into()], &evidence)
            .verdict,
        Verdict::Inconclusive
    );
    assert_eq!(
        restored
            .review_guard(&criteria, &view(&excerpts, &[], &current))
            .verdict,
        Verdict::Inconclusive
    );
}

#[test]
fn strict_recipe_preview_and_apply_only_stage_ungranted_operations() {
    let recipe = Recipe::decode(recipe()).unwrap();
    let preview = recipe.preview(7).unwrap();
    assert!(preview.text().contains("not executed"));
    assert!(preview.text().contains("not granted paths"));
    let mut state = State::default();
    state.apply_recipe(&preview, 7).unwrap();
    state.validate(7).unwrap();
    assert_eq!(state.recipe.as_ref().unwrap().recipe, recipe);
    assert!(state.hypotheses.is_empty() && state.findings.is_empty() && state.review.is_none());
    assert!(state.apply_recipe(&preview, 8).is_err());
    assert!(
        Recipe::decode(&super::tests::recipe().replace("schema_version = 1", "schema_version = 2"))
            .is_err()
    );
    assert!(
        Recipe::decode(&format!(
            "install_hook = 'touch owned'\n{}",
            super::tests::recipe()
        ))
        .is_err()
    );
    assert!(
        Recipe::decode(&super::tests::recipe().replace(
            "form = \"Umbreon\"",
            "form = \"Umbreon\"\ngrants = [\"write\"]"
        ))
        .is_err()
    );
    assert!(Recipe::decode(&super::tests::recipe().replace("bug.rs", "../outside")).is_err());
    assert!(Recipe::decode(&super::tests::recipe().replace("Umbreon", "Unknown")).is_err());
    assert!(Recipe::decode(&"x".repeat(MAX_RECIPE_BYTES + 1)).is_err());
}

#[test]
fn safe_recipe_read_is_bounded_and_preview_never_writes_source() {
    let fixture = Fixture::new();
    let source = fixture.source();
    let path = fixture.0.join("recipe.toml");
    fs::write(&path, recipe()).unwrap();
    let loaded = Recipe::read_file(&path).unwrap();
    let preview = loaded.preview(7).unwrap();
    State::default().apply_recipe(&preview, 7).unwrap();
    assert_eq!(
        fs::read_to_string(fixture.0.join("bug.rs")).unwrap(),
        source.text
    );
    fs::write(&path, "x".repeat(MAX_RECIPE_BYTES + 1)).unwrap();
    assert!(Recipe::read_file(&path).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let link = fixture.0.join("linked.toml");
        symlink(&path, &link).unwrap();
        assert!(Recipe::read_file(&link).is_err());
    }
}

#[test]
fn persisted_text_is_bounded_redacted_and_references_cannot_change_task() {
    let mut state = State::default();
    state.add_hypothesis("token=secret-value").unwrap();
    assert!(!state.hypotheses[0].explanation.contains("secret-value"));
    state.validate(7).unwrap();
    state.hypotheses[0].explanation = "token=secret-value".into();
    assert!(state.validate(7).is_err());
    state.hypotheses[0].explanation = "Bounded explanation".into();
    state.hypotheses[0].evidence.push(HypothesisEvidence {
        relation: EvidenceRelation::Supports,
        reference: EvidenceRef::Run {
            task_id: 8,
            run_id: "foreign".into(),
        },
    });
    assert!(state.validate(7).is_err());
    assert!(
        State::default()
            .add_hypothesis(&"x".repeat(MAX_TEXT_BYTES + 1))
            .is_err()
    );
}

#[test]
fn migration_and_portable_context_never_promote_saved_inference_to_evidence() {
    let empty: State = serde_json::from_str("{}").unwrap();
    assert_eq!(empty, State::default());
    let mut state = empty;
    state
        .add_hypothesis("The selected source causes the failure")
        .unwrap();
    state.hypotheses[0].evidence.push(HypothesisEvidence {
        relation: EvidenceRelation::Supports,
        reference: EvidenceRef::Run {
            task_id: 7,
            run_id: "actual-run-id".into(),
        },
    });
    let (facts, unresolved) = state.portable_notes();
    assert!(facts.is_empty());
    assert!(unresolved[0].contains("task:7/run:actual-run-id"));
    assert!(unresolved[0].contains("unverified"));
    let observations = state.review_observations().unwrap();
    assert!(observations.contains("must be rechecked locally"));
    assert!(
        !observations.contains("actual-run-id"),
        "Diagnosis is not silently injected into independent review"
    );
}

#[test]
fn range_addressing_uses_changed_lines_not_whole_file_or_hunk_context() {
    let before = "one\ntwo\nthree\nfour\nfive\nsix\n";
    let after = "one\nTWO\nthree\nfour\nfive\nsix\n";
    assert!(range_touched(before, after, 2, 2));
    assert!(!range_touched(before, after, 5, 5));
    assert!(!range_touched(
        before,
        "inserted\none\ntwo\nthree\nfour\nfive\nsix\n",
        5,
        5
    ));
    assert!(range_touched(before, "one\nthree\nfour\nfive\nsix\n", 2, 2));
    let two_lines = "one\ntwo\n";
    assert!(
        !range_touched(two_lines, "one\ntwo\nthree\n", 2, 2),
        "Appending strictly after the last range is not addressing it"
    );
    assert!(
        range_touched(two_lines, "zero\none\ntwo\n", 1, 1),
        "Insertion at the first range boundary is included"
    );
    assert!(
        !range_touched(two_lines, "one\ninserted\ntwo\n", 1, 1),
        "Insertion after a one-line range is excluded"
    );
    assert!(
        range_touched(two_lines, "one\ninserted\ntwo\n", 2, 2),
        "Insertion before a selected line is included"
    );
    assert!(
        range_touched(two_lines, "one\ninserted\ntwo\n", 1, 2),
        "Interior insertion is included"
    );
    assert!(range_touched(two_lines, "one\nTWO\n", 2, 2));
    assert!(range_touched(two_lines, "one\n", 2, 2));
}

#[test]
fn baseline_reuse_requires_revision_argv_cwd_executable_and_retained_pass() {
    let run = record("baseline", "before", 10);
    let expected = run.prepared.clone();
    assert!(baseline_eligibility(&run, &expected, &snapshot("before")).is_ok());
    assert!(
        baseline_eligibility(&run, &expected, &snapshot("after"))
            .unwrap_err()
            .contains("revision")
    );
    let mut changed = expected.clone();
    changed.spec = runner::CommandSpec::Direct {
        program: "different".into(),
        args: Vec::new(),
    };
    assert!(
        baseline_eligibility(&run, &changed, &snapshot("before"))
            .unwrap_err()
            .contains("argv")
    );
    changed = expected.clone();
    changed.cwd_identity = "replaced-directory".into();
    assert!(
        baseline_eligibility(&run, &changed, &snapshot("before"))
            .unwrap_err()
            .contains("cwd")
    );
    changed = expected;
    changed.executable.sha256 = "new-executable".into();
    assert!(
        baseline_eligibility(&run, &changed, &snapshot("before"))
            .unwrap_err()
            .contains("executable")
    );
    let mut unstable = run;
    unstable.prepared.before.stable = false;
    assert!(baseline_eligibility(&unstable, &changed, &snapshot("before")).is_err());
}

#[test]
fn addressed_findings_stale_verdict_without_closing_and_require_new_verdict() {
    let mut state = State::default();
    let current = snapshot("before");
    let evidence = view(&[], &[], &current);
    let id = state
        .add_finding("criterion", "finding", "fix", Vec::new(), &evidence)
        .unwrap();
    state
        .record_review(Verdict::RequestChanges, Vec::new(), &evidence)
        .unwrap();
    state.relayed_findings.push(RelayedFinding {
        finding: id,
        author: 2,
        ranges: vec![RangeBaseline {
            path: "bug.rs".into(),
            start: 2,
            end: 2,
            text: "one\ntwo\n".into(),
        }],
        addressed: true,
        review_generation: state.review_generation,
    });
    state.review_stale = true;
    assert!(state.resolve_finding(id, "fixed").is_err());
    assert_eq!(
        state.review_guard(&["criterion".into()], &evidence).verdict,
        Verdict::Inconclusive
    );
    assert!(state.findings[0].resolution.is_none());
    state
        .record_review(Verdict::Inconclusive, Vec::new(), &evidence)
        .unwrap();
    assert!(state.findings[0].resolution.is_none());
    state
        .resolve_finding(id, "Explicit resolution after a new verdict")
        .unwrap();
}

#[test]
fn only_verified_mediated_mutation_receipts_can_attribute_a_patch() {
    use sha2::{Digest, Sha256};
    let digest = <[u8; 32]>::from(Sha256::digest(b"new\r\n"));
    let success = serde_json::json!({"status":"success","path":"calc.py","source_sha256":digest});
    let mut record = crate::store::ProviderRecord {
        id: "controlled-write-record".into(),
        agent: crate::providers::Agent::Codex,
        generation: 1,
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: "eevee-write-call".into(),
        origin: "Codex-requested Eevee-controlled file write".into(),
        coverage: "Exact verified filesystem mutation".into(),
        text: success.to_string(),
        complete: true,
        partner: Some(crate::store::Attribution {
            partner: 2,
            name: "Local flavor".into(),
            form: 3,
        }),
    };
    let author = controlled_patch_author(&record).unwrap();
    assert_eq!(
        (author.partner, author.path.as_str(), author.source_sha256),
        (2, "calc.py", digest)
    );
    record.origin = "Codex provider-native".into();
    assert!(controlled_patch_author(&record).is_none());
    record.agent = crate::providers::Agent::Compatible;
    record.origin = "Eevee-mediated native file tool".into();
    record.item_id = "eevee-native-write:1:0".into();
    assert_eq!(
        controlled_patch_author(&record).unwrap().source_sha256,
        digest
    );
    for body in [
        serde_json::json!({"status":"denied","path":"calc.py","source_sha256":digest}),
        serde_json::json!({"status":"success","path":"calc.py","sha256":"read-only observation"}),
        serde_json::json!({"status":"success","path":"../outside","source_sha256":digest}),
        serde_json::json!({"status":"success","path":"calc.py","source_sha256":vec![256u16; 32]}),
    ] {
        record.text = body.to_string();
        assert!(controlled_patch_author(&record).is_none());
    }
    record.text = success.to_string();
    record.complete = false;
    assert!(controlled_patch_author(&record).is_none());
}

#[test]
fn confirmed_deletion_requires_contained_no_follow_not_found() {
    let fixture = Fixture::new();
    let path = Path::new("untracked.rs");
    fs::write(fixture.0.join(path), "one\ntwo\n").unwrap();
    assert!(!confirmed_source_absence(&fixture.0, path));
    fs::remove_file(fixture.0.join(path)).unwrap();
    assert!(confirmed_source_absence(&fixture.0, path));
    assert!(!confirmed_source_absence(
        &fixture.0,
        Path::new("../outside.rs")
    ));
    fs::write(fixture.0.join("not-a-directory"), "ordinary file").unwrap();
    assert!(!confirmed_source_absence(
        &fixture.0,
        Path::new("not-a-directory/file.rs")
    ));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(fixture.0.join("absent.rs"), fixture.0.join(path)).unwrap();
        assert!(
            !confirmed_source_absence(&fixture.0, path),
            "A dangling link is not proof of deletion"
        );
        std::os::unix::fs::symlink(std::env::temp_dir(), fixture.0.join("linked-parent")).unwrap();
        assert!(!confirmed_source_absence(
            &fixture.0,
            Path::new("linked-parent/absent.rs")
        ));
    }
}

#[test]
fn moving_runs_cannot_support_review_diagnosis_or_refactor_after_restore() {
    let fixture = Fixture::new();
    let excerpts = [fixture.source()];
    let mut runs = [
        record("baseline", "before", 1),
        record("post", "before", 20),
    ];
    let current = snapshot("before");
    let criteria = vec!["Behavior remains unchanged".to_string()];
    let mut state = State::default();
    let refs = vec![
        EvidenceRef::source(&excerpts[0]).unwrap(),
        EvidenceRef::run(&runs[0]).unwrap(),
    ];
    let hypothesis = state
        .add_hypothesis("Selected check supports the explanation")
        .unwrap();
    for reference in &refs {
        state
            .attach_hypothesis(
                hypothesis,
                EvidenceRelation::Supports,
                reference.clone(),
                &view(&excerpts, &runs, &current),
            )
            .unwrap();
    }
    state
        .set_refactor(
            &criteria[0],
            vec![refs[0].clone()],
            "baseline",
            &view(&excerpts, &runs, &current),
        )
        .unwrap();
    state
        .set_post_run("post", &view(&excerpts, &runs, &current))
        .unwrap();
    state
        .record_review(
            Verdict::Accept,
            vec![CriterionEvidence {
                criterion: criteria[0].clone(),
                evidence: refs,
            }],
            &view(&excerpts, &runs, &current),
        )
        .unwrap();
    assert_eq!(
        state
            .review_guard(&criteria, &view(&excerpts, &runs, &current))
            .verdict,
        Verdict::Accept
    );
    assert!(
        state
            .compare_refactor(&view(&excerpts, &runs, &current))
            .behavior_preserved
    );
    for run in &mut runs {
        run.revision_moving = true;
    }
    let restored_runs: Vec<runner::RunRecord> =
        serde_json::from_str(&serde_json::to_string(&runs).unwrap()).unwrap();
    let evidence = view(&excerpts, &restored_runs, &current);
    assert_eq!(
        evidence.status(&EvidenceRef::run(&restored_runs[0]).unwrap()),
        EvidenceStatus::Ambiguous
    );
    assert!(!passed(&restored_runs[0]));
    assert!(
        baseline_eligibility(&restored_runs[0], &restored_runs[0].prepared, &current)
            .unwrap_err()
            .contains("moving")
    );
    assert_eq!(
        state.hypothesis_status(hypothesis, &evidence),
        HypothesisStatus::Unresolved
    );
    assert_eq!(
        state.review_guard(&criteria, &evidence).verdict,
        Verdict::Inconclusive
    );
    assert!(!state.compare_refactor(&evidence).behavior_preserved);
    let (facts, unresolved) = state.portable_notes();
    assert!(facts.is_empty());
    assert!(unresolved.iter().all(|note| note.contains("unverified")));
}
