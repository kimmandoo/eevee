use super::*;

#[test]
fn semantic_specialist_actions_keep_editor_text_and_context_guards() {
    let mut app = App::new("/project".into(), 120, 36);
    app.task_id = 7;
    assert!(app.workflow_command("edit-hypothesis"));
    app.workflow_paste("Literal /diagnostic, not a command.");
    assert!(!app.workflow_command("diagnostic"));
    assert_eq!(
        app.workflow.value(Field::Hypothesis),
        "Literal /diagnostic, not a command."
    );
    app.workflow_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.workflow_command("add-hypothesis"));
    assert_eq!(app.workflows.hypotheses.len(), 1);
    assert!(app.workflow_command("hypothesis-evidence"));
    assert!(!app.workflow_command("specialist-fix"));
    assert!(app.workflow_command("attach-evidence"));
    assert!(app.workflows.hypotheses[0].evidence.is_empty());
    assert!(app.workflow.chooser.is_some());
    assert!(app.workflow.notice.contains("No evidence selected"));
}

#[test]
fn arbitrary_next_action_never_becomes_a_chosen_decision() {
    let mut app = App::new("/project".into(), 120, 36);
    app.set_form(4);
    app.next_action = "Arbitrary response or manually typed next action".into();
    app.workflow_commit_decision();
    assert!(app.workflows.chosen_decision.is_none());
    app.workflow.pending_decision = Some("User-selected proposal".into());
    app.next_action = "Criterion one\nCriterion two\nCriterion three".into();
    app.workflow_commit_decision();
    assert_eq!(
        app.workflows.chosen_decision.as_ref().unwrap().text,
        app.next_action
    );
}

#[test]
fn decision_criteria_preview_edit_duplicates_cancel_confirm_and_restart() {
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/project".into(), width, height);
        app.task_id = 7;
        app.workflows.chosen_decision = Some(workflows::ChosenDecision {
            id: "decision:chosen".into(),
            text: "One\nTwo\nThree".into(),
        });
        app.workflow.tab = Tab::Recipe;
        app.workflow_action(Action::DecisionCriteria).unwrap();
        app.workflow.editor = None;
        app.workflow.criteria_text = "One\nTwo edited\nThree\nOne".into();
        assert!(
            app.workflow_criteria_preview()
                .contains("duplicate: skipped")
        );
        app.workflow_action(Action::ConfirmCriteria).unwrap();
        app.workflow_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.criteria.is_empty());
        assert!(app.workflows.criterion_origins.is_empty());
        app.workflow_apply_criteria().unwrap();
        assert_eq!(app.criteria.len(), 3);
        assert_eq!(app.criteria[1].text, "Two edited");
        assert!(app.criteria.iter().all(|c| !c.met));
        let serialized = serde_json::to_string(&app.workflows).unwrap();
        let restored: workflows::State = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored.criterion_origins.len(), 3);
        assert!(
            restored
                .criterion_origins
                .iter()
                .all(|o| o.decision_id == "decision:chosen")
        );
    }
}

#[test]
fn relay_cancel_and_nonempty_target_preserve_both_drafts() {
    let mut app = App::new("/project".into(), 60, 18);
    app.task_id = 7;
    // Model a retained Request draft, not an empty live editor superseding it.
    app.editing = false;
    app.workflows.chosen_decision = Some(workflows::ChosenDecision {
        id: "chosen".into(),
        text: "criteria proposal".into(),
    });
    app.drafts.push(crate::store::Draft {
        field: crate::store::EditorField::Request,
        text: "sender draft 漢字".into(),
        cursor: 7,
    });
    let before = app.drafts.clone();
    let request = super::super::relays::RelayRequest {
        target_form: 0,
        text: "criteria reference".into(),
        paths: Vec::new(),
        source: super::super::relays::RelaySource::Decision("chosen".into()),
        command: None,
    };
    app.stage_relay(request.clone()).unwrap();
    app.relay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.drafts, before);
    app.stage_relay(request).unwrap();
    app.relay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.workflow.relay.as_mut().unwrap().reviewed = true;
    app.relay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.workflow.notice.contains("Replacement refused"));
    assert_eq!(app.drafts, before);
    assert!(app.planning.pending.is_none());
}

#[test]
fn idle_recipient_gets_references_without_sender_draft_or_request_transfer() {
    let mut app = App::new("/project".into(), 120, 36);
    app.task_id = 7;
    app.party.push(crate::store::Partner {
        id: 2,
        ..Default::default()
    });
    app.workflows.chosen_decision = Some(workflows::ChosenDecision {
        id: "chosen".into(),
        text: "chosen requirements".into(),
    });
    app.drafts.push(crate::store::Draft {
        field: crate::store::EditorField::Request,
        text: "sender 漢字".into(),
        cursor: 3,
    });
    let sender = app.drafts.clone();
    app.stage_relay(super::super::relays::RelayRequest {
        target_form: 0,
        text: "Decision chosen: chosen requirements".into(),
        paths: Vec::new(),
        source: super::super::relays::RelaySource::Decision("chosen".into()),
        command: None,
    })
    .unwrap();
    app.relay_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.relay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.workflow.relay.as_mut().unwrap().reviewed = true;
    app.relay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.workflow.relay.is_none());
    assert_eq!(app.drafts, sender);
    assert_eq!(app.lead.id, 1);
    assert!(
        app.partner_drafts[&2]
            .drafts
            .iter()
            .any(|d| d.text.contains("Decision chosen"))
    );
    assert!(app.planning.pending.is_none());
    assert!(app.execution.prepared.is_none());
}

#[test]
fn reviewed_relay_never_redirects_when_recipient_disappears() {
    let mut app = App::new("/project".into(), 60, 18);
    app.task_id = 7;
    app.party.push(crate::store::Partner {
        id: 2,
        ..Default::default()
    });
    app.party.push(crate::store::Partner {
        id: 3,
        ..Default::default()
    });
    app.workflows.chosen_decision = Some(workflows::ChosenDecision {
        id: "chosen".into(),
        text: "chosen requirements".into(),
    });
    app.stage_relay(super::super::relays::RelayRequest {
        target_form: 0,
        text: "Decision chosen".into(),
        paths: Vec::new(),
        source: super::super::relays::RelaySource::Decision("chosen".into()),
        command: None,
    })
    .unwrap();
    app.relay_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.relay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.workflow.relay.as_mut().unwrap().reviewed = true;
    app.party.retain(|p| p.id != 2);
    app.relay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.workflow.notice.contains("no longer idle"));
    assert!(!app.partner_drafts.contains_key(&3));
}

#[test]
fn relay_scope_limit_is_explicit_and_never_silently_trims_paths() {
    let mut app = App::new("/project".into(), 120, 36);
    app.task_id = 7;
    app.workflows.chosen_decision = Some(workflows::ChosenDecision {
        id: "chosen".into(),
        text: "chosen requirements".into(),
    });
    assert!(
        app.stage_relay(super::super::relays::RelayRequest {
            target_form: 3,
            text: "bounded fix".into(),
            paths: (0..9)
                .map(|i| std::path::PathBuf::from(format!("file{i}.rs")))
                .collect(),
            source: super::super::relays::RelaySource::Decision("chosen".into()),
            command: None,
        })
        .is_err()
    );
    assert!(app.workflow.relay.is_none());
    assert!(app.drafts.is_empty());
    assert!(app.planning.edit_paths.is_empty());
}

struct LocalSourceFixture(std::path::PathBuf);
impl LocalSourceFixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "eevee-relay-guard-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        Self(root)
    }

    fn report(&self) -> crate::workspace::repo::Report {
        let snapshot = std::sync::Arc::new(crate::workspace::repo::Snapshot {
            head: None,
            files: Default::default(),
            working_diff: Default::default(),
            complete: true,
            stable: true,
            warnings: Vec::new(),
        });
        crate::workspace::repo::Report {
            repository: crate::workspace::repo::RepositoryIdentity {
                root: self.0.clone(),
                git_dir: self.0.join(".git"),
            },
            baseline: snapshot.clone(),
            current: snapshot,
            changes: Vec::new(),
            initial: false,
        }
    }
}
impl Drop for LocalSourceFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(any(unix, windows))]
#[test]
fn relay_admits_native_sized_sources_only_as_a_neutral_recipient_draft() {
    let fixture = LocalSourceFixture::new();
    let content = "x".repeat(crate::providers::writes::MAX_BYTES);
    for path in ["first.rs", "second.rs"] {
        std::fs::write(fixture.0.join(path), &content).unwrap();
    }
    let mut app = App::new(fixture.0.to_str().unwrap().into(), 60, 18);
    app.task_id = 7;
    app.repository = Some(std::sync::Arc::new(fixture.report()));
    app.workflows.chosen_decision = Some(workflows::ChosenDecision {
        id: "chosen".into(),
        text: "chosen requirements".into(),
    });
    app.input = "보존할 초안".into();
    app.cursor = "보존할 ".len();
    let before = (app.input.clone(), app.cursor);
    app.stage_relay(super::super::relays::RelayRequest {
        target_form: 3,
        text: "Review these exact paths locally before changing anything.".into(),
        paths: vec!["first.rs".into(), "second.rs".into()],
        source: super::super::relays::RelaySource::Decision("chosen".into()),
        command: None,
    })
    .unwrap();
    let pending = app.workflow.relay.as_ref().unwrap();
    assert_eq!(pending.request.paths.len(), 2);
    assert!(!pending.confirm && !pending.reviewed && pending.target.is_none());
    assert_eq!((app.input.clone(), app.cursor), before);
    assert!(app.drafts.is_empty() && app.planning.edit_paths.is_empty());
    assert!(app.planning.pending.is_none() && app.planning.work.is_none());
    assert_eq!(
        std::fs::read(fixture.0.join("first.rs")).unwrap(),
        content.as_bytes()
    );
    assert_eq!(
        std::fs::read(fixture.0.join("second.rs")).unwrap(),
        content.as_bytes()
    );

    app.workflow.relay = None;
    std::fs::write(
        fixture.0.join("first.rs"),
        "x".repeat(crate::providers::writes::MAX_BYTES + 1),
    )
    .unwrap();
    assert!(
        app.stage_relay(super::super::relays::RelayRequest {
            target_form: 3,
            text: "Oversized source must refuse without partial staging.".into(),
            paths: vec!["first.rs".into(), "second.rs".into()],
            source: super::super::relays::RelaySource::Decision("chosen".into()),
            command: None,
        })
        .is_err()
    );
    assert!(app.workflow.relay.is_none() && app.drafts.is_empty());
    assert!(app.planning.pending.is_none() && app.planning.work.is_none());
}

#[cfg(unix)]
#[test]
fn credential_bearing_source_refuses_actual_finding_relay_without_retaining_raw_metadata() {
    let fixture = LocalSourceFixture::new();
    let secret = "sk-secret123456";
    let source = format!("let token = \"{secret}\";\n");
    assert_ne!(crate::workspace::privacy::redact(&source), source);
    std::fs::write(fixture.0.join("source.rs"), &source).unwrap();
    let (_, source_sha256) = crate::workspace::repo::read_source(
        &fixture.0,
        std::path::Path::new("source.rs"),
        12 * 1024,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new(fixture.0.to_str().unwrap().into(), width, height);
        app.task_id = 7;
        app.repo_loading = false;
        app.repo_refresh = false;
        let mut report = fixture.report();
        std::sync::Arc::make_mut(&mut report.current).files.insert(
            "source.rs".into(),
            crate::workspace::repo::FileIdentity {
                path: "source.rs".into(),
                tracked: false,
                head: None,
                index: Vec::new(),
                state: crate::workspace::repo::FileState::Regular {
                    sha256: source_sha256,
                    bytes: source.len() as u64,
                    binary: false,
                    executable: false,
                },
                dirty: Some(true),
            },
        );
        app.repository = Some(std::sync::Arc::new(report));
        let excerpt = crate::workspace::context::Excerpt {
            id: 1,
            task_id: 7,
            root: fixture.0.to_str().unwrap().into(),
            path: "source.rs".into(),
            start_line: 1,
            end_line: 1,
            source_sha256,
            text: crate::workspace::privacy::redact(&source).into_owned(),
            available: true,
            origin_run: None,
        };
        let reference = EvidenceRef::source(&excerpt).unwrap();
        app.sources.basket.push(excerpt);
        app.workflows.findings.push(workflows::Finding {
            id: 1,
            criterion: "Requirement".into(),
            detail: "Review this source".into(),
            next_action: "Fix the selected range".into(),
            evidence: vec![reference],
            resolution: None,
        });
        app.workflows.review = Some(workflows::Review {
            requested: Verdict::RequestChanges,
            criteria: Vec::new(),
        });
        app.workflow.tab = Tab::Review;
        let before = app.workflows.clone();
        let basket = app.sources.basket.clone();
        assert!(app.workflow_command("specialist-fix"));
        assert!(app.workflow.notice.contains("recognizable credentials"));
        assert!(app.workflow.relay.is_none());
        assert_eq!(app.workflows, before);
        assert_eq!(app.sources.basket, basket);
        assert_eq!(
            std::fs::read_to_string(fixture.0.join("source.rs")).unwrap(),
            source
        );
        assert!(
            !serde_json::to_string(&app.snapshot(fixture.0.to_str().unwrap()))
                .unwrap()
                .contains(secret),
            "No outgoing storage snapshot may retain the raw source credential"
        );
        let mut bypass = before.clone();
        bypass.relayed_findings.push(workflows::RelayedFinding {
            finding: 1,
            author: 2,
            addressed: false,
            review_generation: 0,
            ranges: vec![workflows::RangeBaseline {
                path: "source.rs".into(),
                start: 1,
                end: 1,
                text: source.clone(),
            }],
        });
        assert!(
            bypass
                .validate(7)
                .unwrap_err()
                .contains("Credential-bearing"),
            "The storage validation boundary also refuses raw credential-bearing cache data"
        );
    }
}

#[test]
fn relayed_decision_keeps_original_identity_when_a_new_decision_is_chosen() {
    let mut app = App::new("/project".into(), 60, 18);
    app.task_id = 7;
    app.party.push(crate::store::Partner {
        id: 2,
        ..Default::default()
    });
    let decision_a = workflows::ChosenDecision {
        id: "decision-a".into(),
        text: "A acceptance criterion".into(),
    };
    app.workflows.chosen_decision = Some(decision_a.clone());
    app.stage_relay(super::super::relays::RelayRequest {
        target_form: 0,
        text: "Chosen decision decision-a: A acceptance criterion".into(),
        paths: Vec::new(),
        source: super::super::relays::RelaySource::Decision(decision_a.id.clone()),
        command: None,
    })
    .unwrap();
    app.relay_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.relay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.workflow.relay.as_mut().unwrap().reviewed = true;
    app.relay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert_eq!(
        app.workflows.draft_relays[0].decision.as_ref(),
        Some(&decision_a)
    );
    app.workflows.chosen_decision = Some(workflows::ChosenDecision {
        id: "decision-b".into(),
        text: "Unrelated B criterion".into(),
    });
    app.workflows = serde_json::from_str(&serde_json::to_string(&app.workflows).unwrap()).unwrap();
    app.request_lead(2);
    assert_eq!(app.workflow.criteria_decision.as_ref(), Some(&decision_a));
    assert_eq!(app.workflow.criteria_text, decision_a.text);
    assert!(
        app.workflow_action(Action::ConfirmCriteria)
            .unwrap_err()
            .contains("changed")
    );
    assert!(app.criteria.is_empty());
    assert!(app.workflows.criterion_origins.is_empty());
    assert!(!app.workflow.reviewed);
}

#[test]
fn removed_untracked_source_marks_its_ranges_without_closing_findings() {
    let fixture = LocalSourceFixture::new();
    let text = "one\ntwo\n";
    std::fs::write(fixture.0.join("untracked.rs"), text).unwrap();
    let mut app = App::new(fixture.0.to_str().unwrap().into(), 60, 18);
    app.task_id = 7;
    app.repo_loading = false;
    app.repo_refresh = false;
    for id in 1..=2 {
        app.workflows.findings.push(workflows::Finding {
            id,
            criterion: "Requirement".into(),
            detail: "Review range".into(),
            next_action: "Fix".into(),
            evidence: Vec::new(),
            resolution: None,
        });
        app.workflows
            .relayed_findings
            .push(workflows::RelayedFinding {
                finding: id,
                author: 2,
                addressed: false,
                review_generation: 1,
                ranges: vec![workflows::RangeBaseline {
                    path: "untracked.rs".into(),
                    start: id as usize,
                    end: id as usize,
                    text: text.into(),
                }],
            });
    }
    app.workflows.review = Some(workflows::Review {
        requested: Verdict::RequestChanges,
        criteria: Vec::new(),
    });
    std::fs::remove_file(fixture.0.join("untracked.rs")).unwrap();
    app.repository = Some(std::sync::Arc::new(fixture.report()));
    app.workflow_refresh_addressed();
    assert!(
        app.workflows
            .relayed_findings
            .iter()
            .all(|finding| finding.addressed)
    );
    assert!(app.workflows.review_stale);
    assert!(
        app.workflows
            .findings
            .iter()
            .all(|finding| finding.resolution.is_none())
    );
}

#[test]
fn normal_criteria_editor_reclaims_origins_and_preserves_only_unambiguous_identity() {
    for (width, height) in [(120, 36), (60, 18)] {
        let mut app = App::new("/project".into(), width, height);
        app.task_id = 7;
        let text = (0..64)
            .map(|index| format!("Criterion {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.workflows.chosen_decision = Some(workflows::ChosenDecision {
            id: "chosen-64".into(),
            text: text.clone(),
        });
        app.workflow.criteria_decision = app.workflows.chosen_decision.clone();
        app.workflow.criteria_text = text;
        app.workflow_apply_criteria().unwrap();
        assert_eq!(app.workflows.criterion_origins.len(), 64);
        app.open_editor(crate::store::EditorField::Criteria);
        app.input.clear();
        app.cursor = 0;
        assert!(app.apply_editor());
        assert!(app.criteria.is_empty());
        assert!(app.workflows.criterion_origins.is_empty());
        app.workflow.criteria_text = "new criterion".into();
        app.workflow_apply_criteria().unwrap();
        app.open_editor(crate::store::EditorField::Criteria);
        app.input = "renamed criterion".into();
        app.cursor = app.input.len();
        assert!(app.apply_editor());
        let restored: workflows::State =
            serde_json::from_str(&serde_json::to_string(&app.workflows).unwrap()).unwrap();
        assert_eq!(restored.criterion_origins[0].criterion, "renamed criterion");
        assert_eq!(restored.criterion_origins[0].decision_id, "chosen-64");
        let mut reopened = App::new("/project".into(), width, height);
        reopened.load_task(app.snapshot("/project"));
        assert_eq!(reopened.criteria[0].text, "renamed criterion");
        assert_eq!(
            reopened.workflows.criterion_origins[0],
            restored.criterion_origins[0]
        );
        app.workflow.criteria_text = "retained".into();
        app.workflow_apply_criteria().unwrap();
        app.open_editor(crate::store::EditorField::Criteria);
        app.input = "retained\nunrelated".into();
        app.cursor = app.input.len();
        assert!(app.apply_editor());
        assert_eq!(app.workflows.criterion_origins.len(), 1);
        assert_eq!(app.workflows.criterion_origins[0].criterion, "retained");
        let before = app.criteria.clone();
        let origins = app.workflows.criterion_origins.clone();
        app.open_editor(crate::store::EditorField::Criteria);
        app.input = "retained\nretained\nunrelated".into();
        app.cursor = app.input.len();
        assert!(app.apply_editor());
        assert!(app.notice.contains("Duplicate"));
        assert_eq!(app.criteria, before);
        assert_eq!(app.workflows.criterion_origins, origins);
        app.input = "unrelated\nretained".into();
        app.cursor = app.input.len();
        assert!(app.apply_editor());
        assert_eq!(app.workflows.criterion_origins.len(), 1);
        assert_eq!(
            app.workflows.criterion_origins[0].criterion, "retained",
            "Exact criterion identity survives reorder without linking unrelated text"
        );
    }
}

#[test]
fn role_action_relays_are_reviewed_references_without_replaying_owner_actions() {
    for (width, height) in [(120, 36), (60, 18)] {
        for (action, form) in [("command-edit", 2), ("paths", 3), ("role-implement", 3)] {
            let mut app = App::new("/project".into(), width, height);
            app.task_id = 7;
            app.goal = "Retain the task requirement".into();
            app.set_form(1);
            app.party.push(crate::store::Partner {
                id: 2,
                form,
                nickname: Some("IdleOwner".into()),
                ..Default::default()
            });
            app.drafts.push(crate::store::Draft {
                field: crate::store::EditorField::Request,
                text: "sender draft 漢字".into(),
                cursor: 3,
            });
            app.planning.edit_paths = vec!["existing-scope.rs".into()];
            let sender = app.partner_draft();
            let scope = app.planning.edit_paths.clone();
            let recipient = crate::store::Attribution::of(
                app.party.iter().find(|partner| partner.id == 2).unwrap(),
            );
            let mut recipient_state = app.planning.new_partner(recipient, app.task_id);
            let recipient_role = crate::providers::Operation::Review;
            recipient_state.set_operation(recipient_role);
            recipient_state.edit_paths = vec!["recipient-existing-scope.rs".into()];
            let recipient_scope = recipient_state.edit_paths.clone();
            app.cooperation.parked.insert(2, recipient_state);
            app.stage_role_action(action, 2).unwrap();
            let pending = app.workflow.relay.as_ref().unwrap();
            assert!(pending.confirm);
            assert!(!pending.reviewed);
            assert_eq!(pending.target, Some(2));
            assert_eq!(
                app.relay_targets()
                    .iter()
                    .map(|row| row.0)
                    .collect::<Vec<_>>(),
                vec![2]
            );
            assert!(
                app.partner_drafts
                    .get(&2)
                    .is_none_or(|draft| draft.drafts.is_empty())
            );
            assert!(app.planning.pending.is_none());
            assert!(app.planning.paths_editor.is_none());
            assert!(app.execution.prepared.is_none());
            app.workflow.relay.as_mut().unwrap().reviewed = true;
            app.relay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
            assert!(app.workflow.relay.is_none());
            assert_eq!(app.partner_draft(), sender);
            assert_eq!(app.lead.id, 1);
            assert_eq!(app.form(), 1);
            let request = app.partner_drafts[&2]
                .drafts
                .iter()
                .find(|draft| draft.field == crate::store::EditorField::Request)
                .unwrap();
            assert!(request.text.contains(&format!("requested /{action}")));
            assert!(
                !request.text.contains("IdleOwner"),
                "Partner flavor is not part of the reference draft"
            );
            let metadata = &app.workflows.draft_relays[0];
            assert_eq!(metadata.role_action.as_deref(), Some(action));
            assert!(metadata.paths.is_empty());
            assert!(metadata.decision.is_none());
            assert!(app.workflows.command_relays.is_empty());
            app.workflows =
                serde_json::from_str(&serde_json::to_string(&app.workflows).unwrap()).unwrap();
            app.request_lead(2);
            assert_eq!(
                app.planning.operation, recipient_role,
                "A role-action reference does not replace the recipient's own read-only role"
            );
            assert_eq!(
                app.planning.edit_paths, recipient_scope,
                "A reference relay preserves the recipient's own scope instead of copying the sender's or selecting new paths"
            );
            assert_eq!(
                app.planning_state(1).unwrap().edit_paths,
                scope,
                "The sender's scope remains with the sender"
            );
            assert!(app.planning.paths_editor.is_none());
            assert!(app.planning.pending.is_none());
            assert!(app.execution.prepared.is_none());
        }
    }
}

#[test]
fn role_action_send_cancel_staleness_and_nonempty_recipient_preserve_drafts() {
    let mut app = App::new("/project".into(), 60, 18);
    app.task_id = 7;
    app.goal = "Original requirement".into();
    app.set_form(1);
    app.party.push(crate::store::Partner {
        id: 2,
        form: 3,
        ..Default::default()
    });
    app.drafts.push(crate::store::Draft {
        field: crate::store::EditorField::Request,
        text: "sender".into(),
        cursor: 2,
    });
    let sender = app.partner_draft();
    app.stage_role_action("paths", 2).unwrap();
    app.relay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.partner_draft(), sender);
    assert!(!app.partner_drafts.contains_key(&2));
    assert!(app.workflows.draft_relays.is_empty());
    app.stage_role_action("paths", 2).unwrap();
    app.goal = "Changed requirement".into();
    app.workflow.relay.as_mut().unwrap().reviewed = true;
    app.relay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.workflow.notice.contains("source changed"));
    assert!(!app.partner_drafts.contains_key(&2));
    app.relay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.partner_drafts.insert(
        2,
        crate::store::PartnerDrafts {
            drafts: vec![crate::store::Draft {
                field: crate::store::EditorField::Request,
                text: "recipient keeps this".into(),
                cursor: 4,
            }],
            ..Default::default()
        },
    );
    let recipient = app.partner_drafts[&2].clone();
    app.stage_role_action("role-implement", 2).unwrap();
    app.workflow.relay.as_mut().unwrap().reviewed = true;
    app.relay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.workflow.notice.contains("Replacement refused"));
    assert_eq!(app.partner_drafts[&2], recipient);
    assert_eq!(app.partner_draft(), sender);
}

#[test]
fn role_action_relay_cannot_redirect_or_bypass_fix_scope_limits() {
    use super::super::relays::{RelayRequest, RelaySource};
    let mut app = App::new("/project".into(), 120, 36);
    app.task_id = 7;
    app.set_form(1);
    app.party.push(crate::store::Partner {
        id: 2,
        form: 3,
        ..Default::default()
    });
    app.party.push(crate::store::Partner {
        id: 3,
        form: 3,
        ..Default::default()
    });
    assert!(app.stage_role_action("run", 2).is_err());
    app.stage_role_action("paths", 2).unwrap();
    app.party.retain(|partner| partner.id != 2);
    app.workflow.relay.as_mut().unwrap().reviewed = true;
    app.relay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.workflow.notice.contains("no longer idle"));
    assert!(!app.partner_drafts.contains_key(&3));
    app.relay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let error = app
        .stage_relay(RelayRequest {
            target_form: 3,
            text: "Invalid scope-bearing role relay".into(),
            paths: vec!["invented.rs".into()],
            source: RelaySource::RoleAction {
                action: "paths",
                target: 3,
            },
            command: None,
        })
        .unwrap_err();
    assert!(error.contains("references only"));
    assert!(
        app.stage_relay(RelayRequest {
            target_form: 3,
            text: "Fix still needs bounded paths".into(),
            paths: Vec::new(),
            source: RelaySource::Review,
            command: None,
        })
        .is_err()
    );
    assert!(app.workflow.relay.is_none());
}
