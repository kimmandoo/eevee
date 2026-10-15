use super::*;

#[test]
fn cumulative_streams_release_words_without_exposing_split_secrets() {
    assert_eq!(safe_stream("First ordinary ", false), "First ordinary ");
    assert_eq!(
        safe_stream("First ordinary words 日本語 tail", false),
        "First ordinary words 日本語 "
    );
    for text in [
        "前 sk-secret123456 後 ",
        "前 password=\"opaque value\" 後 ",
        "前 https://user:opaque@example.test/path 後 ",
        "前 -----BEGIN PRIVATE KEY-----\nopaque\n-----END PRIVATE KEY----- 後 ",
    ] {
        let final_text = safe(text);
        for end in text
            .char_indices()
            .map(|(index, _)| index)
            .chain([text.len()])
        {
            assert!(final_text.starts_with(&safe_stream(&text[..end], false)));
        }
        assert_eq!(safe_stream(text, true), final_text);
    }
}
fn model(id: &str, default: bool) -> Value {
    json!({
        "model": id, "displayName": id, "description": "Advertised model",
        "hidden": false, "isDefault": default, "inputModalities": ["text"],
        "defaultReasoningEffort": "medium",
        "supportedReasoningEfforts": [{"reasoningEffort":"low"}, {"reasoningEffort":"medium"}, {"reasoningEffort":"high"}],
    })
}

#[test]
fn complete_catalogue_pages_past_default_and_uses_exact_selected_model() {
    let mut listing = ModelListing::default();
    assert_eq!(
        listing
            .page(&json!({"data":[model("default", true)],"nextCursor":"second"}))
            .unwrap(),
        Some("second".into())
    );
    let mut hidden = model("hidden", false);
    hidden["hidden"] = json!(true);
    let mut image_only = model("image-only", false);
    image_only["inputModalities"] = json!(["image"]);
    assert!(
        listing
            .page(
                &json!({"data":[hidden, image_only, model("requested", false)],"nextCursor":null})
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(
        listing.select(&ModelSelection::default()).unwrap(),
        ("default".into(), "medium".into())
    );
    let selected = ModelSelection {
        model: Some("requested".into()),
        effort: Some("high".into()),
    };
    assert_eq!(
        listing.select(&selected).unwrap(),
        ("requested".into(), "high".into())
    );
    let catalogue = listing.into_catalogue(&selected);
    catalogue.validate().unwrap();
    assert_eq!(
        catalogue
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        vec!["default", "requested"]
    );
    assert_eq!(catalogue.current.as_deref(), Some("requested"));
}

#[test]
fn configured_default_is_not_replaced_by_a_newer_catalogue_default() {
    let mut listing = ModelListing::default();
    listing
        .configuration(&json!({"config":{
            "model":"configured", "model_reasoning_effort":"high",
            "model_provider":"custom-endpoint",
            "unrelated_secret":"never retain this"
        }}))
        .unwrap();
    listing
        .page(&json!({"data":[model("new-default", true)],"nextCursor":"next"}))
        .unwrap();
    listing
        .page(&json!({"data":[model("configured", false)]}))
        .unwrap();
    assert_eq!(
        listing.select(&ModelSelection::default()).unwrap(),
        ("configured".into(), "high".into())
    );
    assert_eq!(
        listing
            .select(&ModelSelection {
                model: Some("new-default".into()),
                effort: None,
            })
            .unwrap(),
        ("new-default".into(), "medium".into())
    );
    listing.refresh();
    listing
        .page(&json!({"data":[model("new-default", true),model("configured", false)]}))
        .unwrap();
    listing
        .revalidate(&ModelSelection::default(), "configured", "high")
        .unwrap();
    let catalogue = listing.into_catalogue(&ModelSelection::default());
    catalogue.validate().unwrap();
    assert_eq!(catalogue.current.as_deref(), Some("configured"));
    assert!(catalogue.source.contains("configured model: configured"));
    assert!(catalogue.source.contains("catalogue default: new-default"));
    assert!(catalogue.source.contains("custom-endpoint"));
    assert!(catalogue.source.contains("Update the same executable"));
    assert!(!catalogue.source.contains("never retain"));
}

#[test]
fn missing_configured_models_and_unsupported_configured_effort_never_fall_back() {
    let mut listing = ModelListing::default();
    listing
        .configuration(&json!({"config":{"model":"missing"}}))
        .unwrap();
    listing
        .page(&json!({"data":[model("available",true)]}))
        .unwrap();
    assert!(listing.select(&ModelSelection::default()).is_err());
    let catalogue = listing.into_catalogue(&ModelSelection::default());
    assert_eq!(catalogue.current.as_deref(), Some("missing"));
    assert_eq!(catalogue.models.len(), 1);
    assert_eq!(catalogue.models[0].id, "available");

    let mut listing = ModelListing::default();
    listing
        .configuration(&json!({"config":{
            "model":"available", "model_reasoning_effort":"not-advertised"
        }}))
        .unwrap();
    listing
        .page(&json!({"data":[model("available",true)]}))
        .unwrap();
    assert!(listing.select(&ModelSelection::default()).is_err());
    assert_eq!(
        listing
            .select(&ModelSelection {
                model: None,
                effort: Some("low".into()),
            })
            .unwrap(),
        ("available".into(), "low".into())
    );
}

#[test]
fn new_provider_model_names_and_max_effort_are_not_version_filtered() {
    let mut listing = ModelListing::default();
    let mut latest = model("provider-next-generation", false);
    latest["supportedReasoningEfforts"] = json!([
        {"reasoningEffort":"medium"}, {"reasoningEffort":"max"}
    ]);
    listing
        .page(&json!({"data":[model("prior-generation",true)],"nextCursor":"newer"}))
        .unwrap();
    listing.page(&json!({"data":[latest]})).unwrap();
    assert_eq!(
        listing
            .select(&ModelSelection {
                model: Some("provider-next-generation".into()),
                effort: Some("max".into()),
            })
            .unwrap(),
        ("provider-next-generation".into(), "max".into())
    );
}

#[test]
fn model_configuration_requires_valid_allowlisted_identity_fields() {
    for config in [
        Value::Null,
        json!([]),
        json!({"model":123}),
        json!({"model_provider":"bad\nprovider"}),
    ] {
        assert!(
            ModelListing::default()
                .configuration(&json!({"config":config}))
                .is_err()
        );
    }
    let mut listing = ModelListing::default();
    listing.configuration(&json!({"config":{}})).unwrap();
    listing
        .page(&json!({"data":[model("advertised",true)]}))
        .unwrap();
    assert_eq!(
        listing.select(&ModelSelection::default()).unwrap(),
        ("advertised".into(), "medium".into())
    );
}

#[test]
fn unavailable_model_and_unsupported_effort_never_fall_back() {
    let mut listing = ModelListing::default();
    listing
        .page(&json!({"data":[model("advertised", true)]}))
        .unwrap();
    for requested in ["missing", "Advertised", "advertised-extra"] {
        assert!(
            listing
                .select(&ModelSelection {
                    model: Some(requested.into()),
                    effort: None
                })
                .is_err()
        );
    }
    assert!(
        listing
            .select(&ModelSelection {
                model: None,
                effort: Some("ultra".into())
            })
            .is_err()
    );
    let mut no_default = ModelListing::default();
    no_default
        .page(&json!({"data":[model("advertised", false)]}))
        .unwrap();
    assert!(no_default.select(&ModelSelection::default()).is_err());
    assert!(
        no_default
            .select(&ModelSelection {
                model: Some("advertised".into()),
                effort: None
            })
            .is_ok()
    );
    let unavailable = ModelSelection {
        model: Some("missing".into()),
        effort: None,
    };
    assert_eq!(
        listing.into_catalogue(&unavailable).current.as_deref(),
        Some("missing")
    );
}

#[test]
fn stale_catalogue_rejects_removed_model_effort_or_changed_default() {
    let selection = ModelSelection {
        model: Some("selected".into()),
        effort: Some("high".into()),
    };
    let mut removed = ModelListing::default();
    removed
        .page(&json!({"data":[model("replacement", true)]}))
        .unwrap();
    assert!(removed.revalidate(&selection, "selected", "high").is_err());
    let mut changed = model("selected", true);
    changed["supportedReasoningEfforts"] = json!([{"reasoningEffort":"medium"}]);
    let mut listing = ModelListing::default();
    listing.page(&json!({"data":[changed]})).unwrap();
    assert!(listing.revalidate(&selection, "selected", "high").is_err());
    assert!(
        removed
            .revalidate(&ModelSelection::default(), "selected", "medium")
            .is_err()
    );
    let mut new_effort = model("selected", true);
    new_effort["defaultReasoningEffort"] = json!("low");
    let mut listing = ModelListing::default();
    listing.page(&json!({"data":[new_effort]})).unwrap();
    assert!(
        listing
            .revalidate(
                &ModelSelection {
                    model: Some("selected".into()),
                    effort: None
                },
                "selected",
                "medium"
            )
            .is_err()
    );
    assert!(
        listing
            .revalidate(
                &ModelSelection {
                    model: Some("selected".into()),
                    effort: Some("medium".into())
                },
                "selected",
                "medium"
            )
            .is_ok()
    );
}

#[test]
fn malformed_or_unbounded_catalogue_is_not_partially_accepted() {
    let mut listing = ModelListing::default();
    listing
        .page(&json!({"data":[],"nextCursor":"cycle"}))
        .unwrap();
    assert!(
        listing
            .page(&json!({"data":[],"nextCursor":"cycle"}))
            .is_err()
    );
    let mut listing = ModelListing::default();
    for page in 0..9 {
        listing
            .page(&json!({"data":[],"nextCursor":format!("cursor-{page}")}))
            .unwrap();
    }
    assert!(
        listing
            .page(&json!({"data":[],"nextCursor":"too-many"}))
            .is_err()
    );
    assert!(
        ModelListing::default()
            .page(&json!({"data":vec![model("same", false);101]}))
            .is_err()
    );
    assert!(
        ModelListing::default()
            .page(&json!({"data":[model("same", false),model("same", false)]}))
            .is_err()
    );
    assert!(
        ModelListing::default()
            .page(&json!({"data":[model("a", true),model("b", true)]}))
            .is_err()
    );
    let mut unsupported_default = model("bad-effort", true);
    unsupported_default["defaultReasoningEffort"] = json!("unsupported");
    assert!(
        ModelListing::default()
            .page(&json!({"data":[unsupported_default]}))
            .is_err()
    );
    let mut listing = ModelListing::default();
    for page in 0..2 {
        let models: Vec<_> = (0..100)
            .map(|index| model(&format!("m-{page}-{index}"), false))
            .collect();
        listing
            .page(&json!({"data":models,"nextCursor":format!("page-{page}")}))
            .unwrap();
    }
    let models: Vec<_> = (0..57)
        .map(|index| model(&format!("last-{index}"), false))
        .collect();
    assert!(listing.page(&json!({"data":models})).is_err());
}

#[test]
fn catalogue_metadata_is_bounded_single_line_and_validated() {
    let mut advertised = model("safe-model", true);
    advertised["displayName"] = json!("Display\nname\t");
    advertised["description"] = json!("é".repeat(4096));
    let mut listing = ModelListing::default();
    listing.page(&json!({"data":[advertised]})).unwrap();
    let catalogue = listing.into_catalogue(&ModelSelection::default());
    catalogue.validate().unwrap();
    assert_eq!(catalogue.models[0].name, "Display name ");
    assert!(catalogue.models[0].description.len() <= 4096);
}

#[test]
fn model_modalities_follow_installed_protocol_default_without_accepting_malformed_values() {
    // Codex 0.147.0's generated Model schema makes inputModalities optional,
    // with the explicit default ["text", "image"].
    let mut advertised = model("schema-default", true);
    advertised
        .as_object_mut()
        .unwrap()
        .remove("inputModalities");
    let mut listing = ModelListing::default();
    listing.page(&json!({"data":[advertised.clone()]})).unwrap();
    assert_eq!(
        listing.select(&ModelSelection::default()).unwrap().0,
        "schema-default"
    );
    for value in [Value::Null, json!("text"), json!([true, "text"])] {
        advertised["inputModalities"] = value;
        assert!(
            ModelListing::default()
                .page(&json!({"data":[advertised.clone()]}))
                .is_err()
        );
    }
    for value in [json!([]), json!(["image"]), json!(["audio"])] {
        advertised["inputModalities"] = value;
        let mut listing = ModelListing::default();
        listing.page(&json!({"data":[advertised.clone()]})).unwrap();
        assert!(
            listing
                .into_catalogue(&ModelSelection::default())
                .models
                .is_empty()
        );
    }
}

#[test]
fn model_inspection_has_no_task_prompt_or_source_or_edit_lookup() {
    let mut input = Input {
        task: crate::store::Snapshot::empty("unopened-project".into()),
        excerpts: Vec::new(),
        report: None,
        resume: None,
        agent: Agent::Codex,
        operation: Operation::Implement,
        edit_paths: vec!["never-read.txt".into()],
        handoff: None,
        inspect_sessions: false,
        inspect_models: true,
        selection: ModelSelection {
            model: Some("selected".into()),
            effort: Some("high".into()),
        },
        credential: None,
    };
    input.task.goal = "private-task-goal-canary".into();
    let text = payload(Path::new("unopened-data"), &input, &[]).unwrap();
    assert!(!text.contains("private-task-goal-canary"));
    assert!(!text.contains("never-read.txt"));
    let preview: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        preview.pointer("/selection/model").and_then(Value::as_str),
        Some("selected")
    );
    assert_eq!(
        preview.pointer("/selection/effort").and_then(Value::as_str),
        Some("high")
    );
    input.resume = Some("owned-thread".into());
    assert!(payload(Path::new("unopened-data"), &input, &[]).is_err());
}

#[test]
fn review_has_no_edit_baseline_and_refuses_selected_write_paths() {
    let mut input = Input {
        task: crate::store::Snapshot::empty("unopened-project".into()),
        excerpts: Vec::new(),
        report: None,
        resume: None,
        agent: Agent::Codex,
        operation: Operation::Review,
        edit_paths: Vec::new(),
        handoff: None,
        inspect_sessions: false,
        inspect_models: false,
        selection: ModelSelection::default(),
        credential: None,
    };
    let root = Path::new(&input.task.project);
    assert!(capture_edit_baseline(root, &input, &[]).unwrap().is_empty());
    input.edit_paths.push("src/file.rs".into());
    assert!(capture_edit_baseline(root, &input, &[]).is_err());
}

#[test]
fn controlled_route_refuses_protected_unselected_and_escaping_paths() {
    let paths = vec![
        PathBuf::from("src/main.rs"),
        PathBuf::from(".git/config"),
        PathBuf::from(".env"),
        PathBuf::from("credentials.json"),
    ];
    assert!(edit_path_allowed(Path::new("src/main.rs"), &paths, &[]));
    for path in [
        ".git/config",
        ".env",
        "credentials.json",
        "../main.rs",
        "/tmp/outside",
        "src/unselected.rs",
    ] {
        assert!(!edit_path_allowed(Path::new(path), &paths, &[]));
    }
    assert!(!edit_path_allowed(
        Path::new("src/main.rs"),
        &paths,
        &[PathBuf::from("src")]
    ));
}
#[test]
fn terminal_turn_does_not_finalize_unfinished_native_tools() {
    for kind in [
        "commandExecution",
        "fileChange",
        "mcpToolCall",
        "dynamicToolCall",
    ] {
        assert!(!provider_item_terminal(
            &json!({"type":kind,"status":"inProgress"})
        ));
        assert!(provider_item_terminal(
            &json!({"type":kind,"status":"completed"})
        ));
        assert!(provider_item_terminal(
            &json!({"type":kind,"status":"failed"})
        ));
    }
    assert!(provider_item_terminal(
        &json!({"type":"dynamicToolCall","success":false})
    ));
    assert!(provider_item_terminal(
        &json!({"type":"agentMessage","text":"complete text"})
    ));
}
#[cfg(unix)]
#[test]
fn exact_dirty_baseline_preserved_and_stale_or_linked_write_has_no_side_effect() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("eevee-edit-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("target.txt");
    std::fs::write(&path, "user dirty line\nold requirement\n").unwrap();
    let baseline = read_edit_file(&root, Path::new("target.txt")).unwrap();
    let after = "user dirty line\nnew requirement\n";
    let patch = replacement_patch(&baseline, after);
    assert!(
        patch.contains("-user dirty line\n-old requirement\n+user dirty line\n+new requirement\n")
    );
    let applied = apply_replacement(&root, &baseline, after).unwrap();
    assert_eq!(applied.text, after);
    std::fs::write(&path, "later user change\n").unwrap();
    assert!(apply_replacement(&root, &applied, "must not overwrite").is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "later user change\n"
    );
    std::os::unix::fs::symlink("target.txt", root.join("linked.txt")).unwrap();
    assert!(read_edit_file(&root, Path::new("linked.txt")).is_err());
    std::fs::hard_link(&path, root.join("hard.txt")).unwrap();
    assert!(read_edit_file(&root, Path::new("target.txt")).is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "later user change\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn exact_edit_permissions_do_not_offer_persistent_grants() {
    let kind = PromptKind::Permission {
        description: "exact patch".into(),
        options: vec![
            PermissionOption {
                label: "Deny".into(),
                description: String::new(),
                answer: json!({"decision":"decline"}),
            },
            PermissionOption {
                label: "Allow once".into(),
                description: String::new(),
                answer: json!({"decision":"accept"}),
            },
        ],
    };
    assert!(validate_answer(&kind, &json!({"decision":"decline"})).is_ok());
    assert!(validate_answer(&kind, &json!({"decision":"accept"})).is_ok());
    for answer in [
        json!({"decision":"acceptForSession"}),
        json!({"decision":"accept","grantRoot":"/"}),
        json!({"decision":"allow-always"}),
    ] {
        assert!(validate_answer(&kind, &answer).is_err());
    }
}
#[test]
fn bounded_events_replace_item_snapshots_and_reserve_terminal_capacity() {
    let events = Arc::new(Mutex::new(VecDeque::new()));
    let make_item = |text: &str, complete| Event::Item {
        generation: 7,
        task_id: 3,
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: "item".into(),
        kind: ItemKind::Message,
        text: text.into(),
        complete,
    };
    emit(&events, make_item("first delta snapshot", false)).unwrap();
    emit(&events, make_item("completed authoritative text", true)).unwrap();
    assert_eq!(events.lock().len(), 1);
    assert!(
        matches!(events.lock().front(), Some(Event::Item { text, complete: true, .. }) if text == "completed authoritative text")
    );
    for _ in 1..EVENTS_LIMIT - EVENT_RESERVE {
        emit(
            &events,
            Event::Status {
                generation: 7,
                task_id: 3,
                message: "bounded".into(),
            },
        )
        .unwrap();
    }
    assert!(
        emit(
            &events,
            Event::Status {
                generation: 7,
                task_id: 3,
                message: "overflow".into()
            }
        )
        .is_err()
    );
    finish(
        &events,
        7,
        3,
        Outcome::Unknown,
        "Event overflow; no authoritative completion".into(),
    );
    assert!(matches!(
        events.lock().back(),
        Some(Event::Finished {
            outcome: Outcome::Unknown,
            ..
        })
    ));
}
#[test]
fn nonblocking_and_duplicate_question_requests_are_refused() {
    let question = json!({"id":"q","header":"Choice","question":"Pick one","options":[{"label":"A","description":"literal"}],"isOther":true,"isSecret":false});
    assert!(questions(&json!({"isBlocking":false,"questions":[question.clone()]})).is_err());
    assert!(
        questions(&json!({"isBlocking":true,"questions":[question.clone(),question.clone()]}))
            .is_err()
    );
    assert_eq!(
        questions(&json!({"isBlocking":true,"questions":[question]}))
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn jsonl_handles_utf8_and_multiple_frames_without_accepting_partial_eof() {
    let mut framer = Framer::default();
    let bytes = "{\"result\":\"문자\"}\n{\"id\":1}\n".as_bytes();
    assert!(framer.push(&bytes[..13]).unwrap().is_empty());
    assert!(framer.partial());
    let frames = framer.push(&bytes[13..]).unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["result"], "문자");
    assert!(!framer.partial());
    assert!(framer.push(b"{\"id\":").unwrap().is_empty());
    assert!(framer.partial());
}
#[test]
fn malformed_and_oversized_frames_fail_closed_without_raw_diagnostics() {
    let error = Framer::default().push(b"secret-canary\n").unwrap_err();
    assert!(!error.contains("secret-canary"));
    assert!(
        Framer::default()
            .push(&vec![b' '; FRAME_LIMIT + 1])
            .is_err()
    );
    assert!(Framer::default().push(b"\n").is_err());
}
#[test]
fn request_identity_preserves_string_vs_number_and_refuses_unsafe_ids() {
    assert_ne!(
        request_key(&json!(1)).unwrap(),
        request_key(&json!("1")).unwrap()
    );
    for id in [
        Value::Null,
        json!(true),
        json!(1.5),
        json!("\u{001b}[31m"),
        json!("sk-sensitive123"),
    ] {
        assert!(request_key(&id).is_err());
    }
}
#[test]
fn approvals_never_accept_session_rules_or_hidden_choices() {
    let kind = PromptKind::Approval {
        command: "echo hello".into(),
        cwd: "/tmp".into(),
        reason: String::new(),
        allow_once: true,
        deny_decision: "decline".into(),
    };
    assert!(validate_answer(&kind, &json!({"decision":"accept"})).is_ok());
    assert!(validate_answer(&kind, &json!({"decision":"decline"})).is_ok());
    for decision in [
        json!("acceptForSession"),
        json!({"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["echo"]}}),
    ] {
        assert!(validate_answer(&kind, &json!({"decision":decision})).is_err());
    }
    assert!(validate_answer(&kind, &json!({"decision":"accept","extra":true})).is_err());
    let kind = PromptKind::Approval {
        command: "printf literal".into(),
        cwd: "/tmp".into(),
        reason: String::new(),
        allow_once: true,
        deny_decision: "cancel".into(),
    };
    assert!(validate_answer(&kind, &json!({"decision":"cancel"})).is_ok());
    assert!(validate_answer(&kind, &json!({"decision":"decline"})).is_err());
}
#[test]
fn answers_require_exact_current_question_ids_and_literal_user_input() {
    let kind = PromptKind::Questions(vec![Question {
        id: "current".into(),
        header: String::new(),
        question: String::new(),
        options: vec![],
        other: true,
        secret: false,
    }]);
    assert!(
        validate_answer(
            &kind,
            &json!({"answers":{"current":{"answers":["literal custom choice"]}}})
        )
        .is_ok()
    );
    assert!(
        validate_answer(
            &kind,
            &json!({"answers":{"stale":{"answers":["literal custom choice"]}}})
        )
        .is_err()
    );
    assert!(validate_answer(&kind, &json!({"answers":{"current":{"answers":[""]}}})).is_err());
    assert!(
        validate_answer(
            &kind,
            &json!({"answers":{"current":{"answers":["token=secret-canary"]}}})
        )
        .is_err()
    );
}
#[test]
fn terminal_strings_and_cumulative_credentials_are_masked() {
    assert!(!safe("\u{001b}[31m\u{202e} token=secret-canary").contains("secret-canary"));
    assert!(!safe("\u{001b}[31m\u{202e}").contains('\u{001b}'));
    assert!(!safe("\u{001b}[31m\u{202e}").contains('\u{202e}'));
}
#[test]
fn outbound_task_facts_do_not_leak_generated_provider_session_provenance() {
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root = std::env::temp_dir().join(format!(
        "ev-payload-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let cleanup = Cleanup(root.canonicalize().unwrap());
    let data = cleanup.0.join("data");
    let worker = crate::store::Worker::start(
        data.clone(),
        cleanup.0.to_str().unwrap().into(),
        crate::store::Policy::default(),
    )
    .unwrap();
    let mut task = crate::store::Snapshot::empty(cleanup.0.to_str().unwrap().into());
    task.goal = "Continue the approved note workflow".into();
    task.revision = 1;
    task.next_action = "Choose Markdown.\n\n[User-selected, unverified OpenCode ACP Plan; thread opaque-old-session; turn opaque-old-turn; item opaque-old-item. Not local execution evidence.]".into();
    let mut pending = Some(task);
    let deadline = Instant::now() + Duration::from_secs(5);
    let saved = loop {
        worker.try_save(&mut pending);
        if let Some(crate::store::Event::Saved(task)) = worker.try_event() {
            break task;
        }
        assert!(
            Instant::now() < deadline,
            "Committed fixture task was not acknowledged"
        );
        thread::sleep(Duration::from_millis(5));
    };
    let handoff = crate::workspace::handoff::from_task(&saved, &[], &[]).unwrap();
    let input = Input {
        task: saved,
        excerpts: Vec::new(),
        report: None,
        resume: None,
        agent: Agent::Codex,
        operation: Operation::Plan,
        edit_paths: Vec::new(),
        handoff: Some(handoff),
        inspect_sessions: false,
        inspect_models: false,
        selection: ModelSelection::default(),
        credential: None,
    };
    let text = payload(&data, &input, &[]).unwrap();
    let outgoing: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        outgoing
            .pointer("/task/next_action")
            .and_then(Value::as_str),
        Some("Choose Markdown.")
    );
    for private_id in ["opaque-old-session", "opaque-old-turn", "opaque-old-item"] {
        assert!(
            !text.contains(private_id),
            "Provider control identity crossed the portable request boundary"
        );
    }
    drop(worker);
}
#[test]
fn baseline_cites_identical_selected_capture_instead_of_resending_bytes() {
    use crate::workspace::context::Excerpt;
    let baseline = EditBaseline {
        path: PathBuf::from("src/lib.rs"),
        text: "fn answer() -> u8 { 42 }\n".into(),
        digest: "ab".repeat(32),
        identity: "identity".into(),
    };
    let pinned = Excerpt {
        id: 77,
        task_id: 1,
        root: "/repo".into(),
        path: "src/lib.rs".into(),
        start_line: 1,
        end_line: 1,
        source_sha256: [0; 32],
        text: "fn answer() -> u8 { 42 }\n".into(),
        available: true,
    };
    // Whole-file pin with identical text -> citation, not duplication.
    let entry = baseline_entry(&baseline, std::slice::from_ref(&pinned));
    assert_eq!(entry["working_content_same_as_capture"], 77);
    assert!(entry.get("working_content").is_none());
    // A partial range never replaces the baseline's current bytes.
    let mut partial = pinned.clone();
    partial.start_line = 2;
    partial.text = "fn".into();
    let entry = baseline_entry(&baseline, &[partial]);
    assert!(entry.get("working_content_same_as_capture").is_none());
    assert_eq!(entry["working_content"], "fn answer() -> u8 { 42 }\n");
    // Same path with different content stays separate (distinct evidence).
    let mut changed = pinned.clone();
    changed.text = "fn answer() -> u8 { 43 }\n".into();
    let entry = baseline_entry(&baseline, &[changed]);
    assert!(entry.get("working_content_same_as_capture").is_none());
    // A different path with identical text also stays separate.
    let mut other = pinned;
    other.path = "src/other.rs".into();
    let entry = baseline_entry(&baseline, &[other]);
    assert!(entry.get("working_content_same_as_capture").is_none());
}
