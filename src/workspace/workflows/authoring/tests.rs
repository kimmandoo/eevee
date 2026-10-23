use super::*;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "ev-recipe-authoring-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(fs::canonicalize(root).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// Explicit retained-journal fixtures exercise admission, not actual execution qualification.
fn record(root: &Path) -> runner::RunRecord {
    let snapshot = runner::InputSnapshot {
        fingerprint: Some("prior-source-identity".into()),
        complete: true,
        stable: true,
    };
    let prepared = runner::Prepared {
        request_id: "request-validation".into(),
        run_id: "run-validation".into(),
        task_id: 7,
        spec: runner::CommandSpec::Direct {
            program: "cargo".into(),
            args: vec!["test".into(), "".into()],
        },
        cwd: root.into(),
        cwd_identity: "prior-directory".into(),
        executable: runner::ExecutableIdentity {
            path: PathBuf::from("/fixture/cargo"),
            sha256: "prior-executable".into(),
            bytes: 1,
            modified_ns: 1,
            file_id: "prior-file".into(),
        },
        prepared_at_ms: 1,
        expires_at_ms: 100,
        approval_arrival: 0,
        before: snapshot.clone(),
        partner: Some(crate::store::Attribution {
            partner: 1,
            name: "Prior partner".into(),
            form: 2,
        }),
        rerun_of: None,
    };
    let mut record = runner::RunRecord::awaiting(&prepared);
    record.started_at_ms = Some(2);
    record.ended_at_ms = Some(3);
    record.state = runner::RunState::Succeeded;
    record.exit_code = Some(0);
    record.tree = runner::TreeOutcome::OwnedTreeExited;
    record.after = snapshot;
    record
}
fn draft() -> Draft {
    Draft {
        name: "Reviewed change".into(), scope: "src/new.rs".into(), criteria: "Behavior checked independently".into(),
        steps: "Flareon: Request a reviewed change\nJolteon: Review and run validation\nGlaceon: Inspect the actual result".into(),
        path: "workflow.toml".into(), runs: vec!["run-validation".into()],
    }
}
#[test]
fn export_reopen_apply_another_project_has_no_authority_or_imported_evidence() {
    let source = Fixture::new();
    let destination = Fixture::new();
    let runs = vec![record(&source.0)];
    let export = draft().preview(3, 7, 1, 4, &source.0, &runs).unwrap();
    assert!(!source.0.join("workflow.toml").exists()); // Preview/cancel is inert.
    assert!(export.matches(3, 7, 1, 4, &source.0));
    assert!(!export.matches(4, 7, 1, 4, &source.0));
    assert!(!export.matches(3, 8, 1, 4, &source.0));
    export.publish().unwrap();
    let original = fs::read(export.path()).unwrap();
    assert!(export.publish().is_err());
    assert_eq!(fs::read(export.path()).unwrap(), original);
    let text = String::from_utf8(original).unwrap();
    for private in [
        "run-validation",
        "request-validation",
        "Prior partner",
        "prior-source-identity",
        "/fixture",
        source.0.to_str().unwrap(),
    ] {
        assert!(!text.contains(private), "{private}");
    }
    assert!(text.contains("Historical provenance only"));
    fs::write(destination.0.join("imported.toml"), &text).unwrap();
    let recipe = Recipe::read_file(&destination.0.join("imported.toml")).unwrap();
    assert_eq!(recipe.validation_commands[0], vec!["cargo", "test", ""]);
    assert!(recipe.context.is_empty() && recipe.roles.is_empty());
    let mut state = super::super::State::default();
    let preview = recipe.preview(99).unwrap();
    state.apply_recipe(&preview, 99).unwrap();
    assert!(state.review.is_none() && state.findings.is_empty() && state.hypotheses.is_empty());
    assert_eq!(state.recipe.as_ref().unwrap().task_id, 99);
    assert_eq!(fs::read_dir(&destination.0).unwrap().count(), 1); // Applying starts no effect.
    assert!(!destination.0.join("src/new.rs").exists());
}
#[test]
fn only_complete_actual_literal_root_runs_qualify_and_their_argv_are_not_edited() {
    let fixture = Fixture::new();
    let good = record(&fixture.0);
    assert_eq!(
        command(&fixture.0, 7, &good).unwrap(),
        vec!["cargo", "test", ""]
    );
    let mut variants = Vec::new();
    let mut r = good.clone();
    r.prepared.task_id = 8;
    variants.push(r);
    let mut r = good.clone();
    r.state = runner::RunState::Failed;
    r.exit_code = Some(1);
    variants.push(r);
    let mut r = good.clone();
    r.output_unavailable = true;
    variants.push(r);
    let mut r = good.clone();
    r.truncated = true;
    variants.push(r);
    let mut r = good.clone();
    r.cancelled = true;
    variants.push(r);
    let mut r = good.clone();
    r.revision_moving = true;
    variants.push(r);
    let mut r = good.clone();
    r.after.fingerprint = Some("changed".into());
    variants.push(r);
    let mut r = good.clone();
    r.prepared.cwd = fixture.0.join("subdirectory");
    variants.push(r);
    let mut r = good.clone();
    r.prepared.spec = runner::CommandSpec::Shell {
        executable: "sh".into(),
        dialect: runner::ShellDialect::Posix,
        text: "cargo test".into(),
    };
    variants.push(r);
    for argument in [
        "/private/machine.txt",
        "--config=/private/machine.txt",
        "../outside",
        ".env",
        "--token=not-for-export",
        "[MASKED]",
    ] {
        let mut r = good.clone();
        r.prepared.spec = runner::CommandSpec::Direct {
            program: "cargo".into(),
            args: vec![argument.into()],
        };
        variants.push(r);
    }
    for record in variants {
        assert!(
            command(&fixture.0, 7, &record).is_err(),
            "{:?}",
            record.prepared.spec
        );
    }
    let mut edited = draft();
    edited.runs = vec!["invented-command".into()];
    assert!(edited.preview(1, 7, 1, 0, &fixture.0, &[good]).is_err());
}
#[test]
fn user_text_scope_output_and_capability_boundaries_refuse_without_publication() {
    let fixture = Fixture::new();
    let runs = vec![record(&fixture.0)];
    for text in [
        "Flareon: Read /private/machine/file",
        "Flareon: use file=/Users/local/file",
        "Flareon: task:7/run:abc",
        "Flareon: [REDACTED]",
        "Flareon: token=not-for-export",
        "UnsupportedTool: Do work",
    ] {
        let mut draft = draft();
        draft.steps = text.into();
        assert!(
            draft.preview(1, 7, 1, 0, &fixture.0, &runs).is_err(),
            "{text}"
        );
    }
    for scope in ["../outside", ".env", "/absolute", "safe/.env", "src/*"] {
        let mut draft = draft();
        draft.scope = scope.into();
        assert!(
            draft.preview(1, 7, 1, 0, &fixture.0, &runs).is_err(),
            "{scope}"
        );
    }
    for path in ["../outside.toml", ".env/output.toml", "output.txt"] {
        let mut draft = draft();
        draft.path = path.into();
        assert!(
            draft.preview(1, 7, 1, 0, &fixture.0, &runs).is_err(),
            "{path}"
        );
    }
    let mut draft = draft();
    draft.steps.clear();
    assert!(draft.preview(1, 7, 1, 0, &fixture.0, &runs).is_err());
    assert!(!fixture.0.join("workflow.toml").exists());
    #[cfg(unix)]
    {
        let outside = Fixture::new();
        std::os::unix::fs::symlink(&outside.0, fixture.0.join("src")).unwrap();
        let export = super::tests::draft()
            .preview(1, 7, 1, 0, &fixture.0, &runs)
            .unwrap();
        assert!(export.publish().is_err());
        assert!(!fixture.0.join("workflow.toml").exists());
    }
}
