use super::*;

#[test]
fn complete_codex_proposals_have_guarded_display_only_snapshots() {
    let mut input = crate::providers::tests::claimed(Agent::Codex, Operation::Implement, true);
    let params = json!({"tool":"eevee_write_file","arguments":{
        "path":"src/새.rs","content":"pub fn generated() {}\npassword=private-value\n"
    }});
    let tool = requested_tool(&params).unwrap();
    let text = received_file_preview(&input, &tool).unwrap();
    assert!(text.starts_with("src/새.rs"));
    assert!(text.contains("pub fn generated()"));
    assert!(!text.contains("private-value"));
    assert!(!text.contains("eevee_write_file"));
    assert!(
        requested_tool(
            &json!({"tool":"eevee_write_file","arguments":{"path":"src/새.rs","content":7}})
        )
        .is_err()
    );
    for operation in [Operation::Plan, Operation::Review] {
        input.operation = operation;
        assert!(received_file_preview(&input, &tool).is_none());
    }
    input.operation = Operation::Implement;
    input.inspect_models = true;
    assert!(received_file_preview(&input, &tool).is_none());
    input.inspect_models = false;
    input.inspect_sessions = true;
    assert!(received_file_preview(&input, &tool).is_none());
    input.inspect_sessions = false;
    let read = requested_tool(&json!({"tool":"eevee_read_file","arguments":{"path":"src/새.rs","start_line":1,"end_line":2}})).unwrap();
    assert!(received_file_preview(&input, &read).is_none());
    let edit = requested_tool(&json!({"tool":"eevee_edit_file","arguments":{
        "path":"src/새.rs","sha256":"0".repeat(64),"replacements":[{"old_text":"not displayed","new_text":"replacement\n"}]
    }})).unwrap();
    // Multi-hunk fragments are not a fabricated final-file preview. The
    // complete frozen result appears only after original-snapshot preparation.
    assert!(received_file_preview(&input, &edit).is_none());
}

#[cfg(unix)]
struct WriteFixture {
    root: PathBuf,
    data: PathBuf,
}

#[cfg(unix)]
impl WriteFixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "eevee-codex-write-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let data = root.join(".eevee-data");
        std::fs::create_dir(&data).unwrap();
        Self { root, data }
    }

    fn capture(&self, paths: &[&str]) -> Result<WriteSet, String> {
        let paths = paths
            .iter()
            .map(|path| PathBuf::from(*path))
            .collect::<Vec<_>>();
        WriteSet::capture(&self.root, &self.data, &paths, &[])
    }

    fn claimed_input(&self, operation: Operation) -> (crate::store::Worker, Input) {
        let worker = crate::store::Worker::start(
            self.data.clone(),
            self.root.to_str().unwrap().into(),
            crate::store::Policy::default(),
        )
        .unwrap();
        let next = || {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(event) = worker.try_event() {
                    break event;
                }
                assert!(Instant::now() < deadline, "Store acknowledgement timed out");
                thread::sleep(Duration::from_millis(2));
            }
        };
        let crate::store::Event::Loaded(project) = next() else {
            panic!("Expected loaded project")
        };
        let owner = crate::store::Attribution::of(&project.partners[0]);
        let mut task = crate::store::Snapshot::empty(self.root.to_str().unwrap().into());
        task.goal = "Inspect actual working source without changing it".into();
        task.lead_partner = owner.partner;
        task.form = owner.form;
        task.revision = 1;
        let mut save = Some(task);
        assert!(worker.try_save(&mut save));
        let crate::store::Event::Saved(task) = next() else {
            panic!("Expected saved task")
        };
        let claim = crate::store::WorkClaim {
            task_id: task.id,
            owner,
            request_id: "codex-file-fixture".into(),
            writing: super::super::writing_required(operation),
        };
        let mut acquire = Some(crate::store::WorkRequest::Acquire {
            claim: claim.clone(),
            task: Box::new(task.clone()),
        });
        assert!(worker.try_work(&mut acquire));
        let crate::store::Event::Work { result, .. } = next() else {
            panic!("Expected work claim")
        };
        result.unwrap();
        let mut input = crate::providers::tests::claimed(Agent::Codex, operation, claim.writing);
        input.task = task;
        input.work = Some(claim);
        (worker, input)
    }
}

#[cfg(unix)]
impl Drop for WriteFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

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
        ("default".into(), Some("medium".into()))
    );
    let selected = ModelSelection {
        model: Some("requested".into()),
        effort: Some("high".into()),
    };
    assert_eq!(
        listing.select(&selected).unwrap(),
        ("requested".into(), Some("high".into()))
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
        ("configured".into(), Some("high".into()))
    );
    assert_eq!(
        listing
            .select(&ModelSelection {
                model: Some("new-default".into()),
                effort: None,
            })
            .unwrap(),
        ("new-default".into(), Some("medium".into()))
    );
    listing.refresh();
    listing
        .page(&json!({"data":[model("new-default", true),model("configured", false)]}))
        .unwrap();
    listing
        .revalidate(&ModelSelection::default(), "configured", Some("high"))
        .unwrap();
    let catalogue = listing.into_catalogue(&ModelSelection::default());
    catalogue.validate().unwrap();
    assert_eq!(catalogue.current.as_deref(), Some("configured"));
}

#[test]
fn configured_custom_models_are_preserved_and_known_efforts_are_validated() {
    let mut listing = ModelListing::default();
    listing
        .configuration(&json!({"config":{"model":"missing"}}))
        .unwrap();
    listing
        .page(&json!({"data":[model("available",true)]}))
        .unwrap();
    assert_eq!(
        listing.select(&ModelSelection::default()).unwrap(),
        ("missing".into(), None)
    );
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
        ("available".into(), Some("low".into()))
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
        ("provider-next-generation".into(), Some("max".into()))
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
        ("advertised".into(), Some("medium".into()))
    );
}

#[test]
fn explicit_custom_models_are_forwarded_exactly_without_catalogue_authorization() {
    let mut listing = ModelListing::default();
    listing
        .page(&json!({"data":[model("advertised", true)]}))
        .unwrap();
    for requested in ["missing", "Advertised", "advertised-extra"] {
        assert_eq!(
            listing
                .select(&ModelSelection {
                    model: Some(requested.into()),
                    effort: None
                })
                .unwrap(),
            (requested.into(), None)
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
fn revalidation_keeps_exact_custom_selection_but_detects_changed_known_defaults() {
    let selection = ModelSelection {
        model: Some("selected".into()),
        effort: Some("high".into()),
    };
    let mut removed = ModelListing::default();
    removed
        .page(&json!({"data":[model("replacement", true)]}))
        .unwrap();
    assert!(
        removed
            .revalidate(&selection, "selected", Some("high"))
            .is_ok()
    );
    let mut changed = model("selected", true);
    changed["supportedReasoningEfforts"] = json!([{"reasoningEffort":"medium"}]);
    let mut listing = ModelListing::default();
    listing.page(&json!({"data":[changed]})).unwrap();
    assert!(
        listing
            .revalidate(&selection, "selected", Some("high"))
            .is_err()
    );
    assert!(
        removed
            .revalidate(&ModelSelection::default(), "selected", Some("medium"))
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
                Some("medium")
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
                Some("medium")
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
    for page in 0..63 {
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
    listing.page(&json!({"data":models})).unwrap();
    assert_eq!(listing.models.len(), 257);
}

#[test]
fn models_without_display_or_reasoning_metadata_remain_selectable() {
    let mut listing = ModelListing::default();
    listing
        .page(&json!({"data":[{"model":"plain-text","isDefault":true}]}))
        .unwrap();
    assert_eq!(
        listing.select(&ModelSelection::default()).unwrap(),
        ("plain-text".into(), None)
    );
    let catalogue = listing.into_catalogue(&ModelSelection::default());
    catalogue.validate().unwrap();
    assert_eq!(catalogue.models[0].name, "plain-text");
    assert!(catalogue.models[0].efforts.is_empty());
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
        public_context: None,
        tool_catalog: None,
        context_plan: None,
        screenshot: None,
        prior_plan: None,
        task: crate::store::Snapshot::empty("unopened-project".into()),
        work: None,
        local_only: false,
        instruction_override: None,
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
        conversation: None,
        credential: None,
    };
    input.task.goal = "private-task-goal-canary".into();
    let text = payload(Path::new("unopened-data"), &input, &[], None).unwrap();
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
    assert!(payload(Path::new("unopened-data"), &input, &[], None).is_err());
}

#[cfg(unix)]
#[test]
fn readonly_roles_capture_real_source_without_mutation_targets_or_writer_lease() {
    for operation in [Operation::Plan, Operation::Review] {
        let fixture = WriteFixture::new();
        std::fs::write(
            fixture.root.join("actual.rs"),
            "// actual dirty bytes\r\nfn source() {}\r\n",
        )
        .unwrap();
        let (_worker, mut input) = fixture.claimed_input(operation);
        if operation == Operation::Review {
            input.report = Some(crate::providers::tests::repository_report(
                &fixture.root,
                &fixture.data,
            ));
        }
        assert!(!input.work.as_ref().unwrap().writing);
        let mut scope = capture_writes(&fixture.root, &fixture.data, &input, &[])
            .unwrap()
            .unwrap();
        assert!(!scope.writing());
        let list = requested_tool(
            &json!({"tool":"eevee_list_files","arguments":{"directory":"","query":".rs","offset":0}}),
        )
        .unwrap();
        assert_eq!(
            scope.observe_tool(&list).unwrap()["paths"],
            json!(["actual.rs"])
        );
        let read = requested_tool(&json!({"tool":"eevee_read_file","arguments":{"path":"actual.rs","start_line":1,"end_line":0}})).unwrap();
        assert_eq!(
            scope.observe_tool(&read).unwrap()["content"],
            "// actual dirty bytes\r\nfn source() {}\r\n"
        );
        for attempted in [
            json!({"tool":"eevee_write_file","arguments":{"path":"unapproved/new.rs","content":"no effect"}}),
            json!({"tool":"eevee_edit_file","arguments":{"path":"actual.rs","sha256":"0".repeat(64),"replacements":[{"old_text":"source","new_text":"mutated"}]}}),
            json!({"tool":"eevee_delete_file","arguments":{"path":"actual.rs","sha256":"0".repeat(64)}}),
            json!({"tool":"eevee_rename_file","arguments":{"path":"actual.rs","destination":"renamed.rs","sha256":"0".repeat(64)}}),
        ] {
            assert!(
                proposed_write(&mut scope, &attempted)
                    .unwrap_err()
                    .contains("Read-only")
            );
            assert_eq!(scope.payload(&[]), json!([]));
        }
        let declarations = definitions(scope.writing());
        assert_eq!(declarations.len(), 3);
        assert!(declarations.iter().all(|entry| matches!(
            entry["name"].as_str(),
            Some("eevee_read_file" | "eevee_list_files" | "eevee_structural_search")
        )));
        let text = payload(&fixture.data, &input, &[], Some(&scope)).unwrap();
        let envelope: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(envelope["project_context"]["root"], json!(fixture.root));
        assert_eq!(
            envelope["project_context"]["filenames"]["paths"],
            json!(["actual.rs"])
        );
        assert_eq!(envelope["project_context"]["filenames"]["complete"], true);
        assert!(
            !envelope["project_context"]
                .to_string()
                .contains("actual dirty bytes")
        );
        assert!(!fixture.root.join("unapproved").exists());
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("actual.rs")).unwrap(),
            "// actual dirty bytes\r\nfn source() {}\r\n"
        );
        input.edit_paths.push("src/file.rs".into());
        assert!(capture_writes(&fixture.root, &fixture.data, &input, &[]).is_err());
        input.edit_paths.clear();
        input.work.as_mut().unwrap().writing = true;
        assert!(capture_writes(&fixture.root, &fixture.data, &input, &[]).is_err());
        input.work = None;
        assert!(capture_writes(&fixture.root, &fixture.data, &input, &[]).is_err());
    }
}

#[cfg(unix)]
#[test]
fn implement_without_preselection_captures_workspace_and_invalid_proposals_do_not_enroll() {
    let fixture = WriteFixture::new();
    std::fs::write(fixture.root.join("unselected-binary.txt"), b"\0").unwrap();
    let (_worker, mut input) = fixture.claimed_input(Operation::Implement);
    let mut writes = capture_writes(&fixture.root, &fixture.data, &input, &["excluded".into()])
        .unwrap()
        .unwrap();
    assert_eq!(writes.payload(&[]), json!([]));
    for params in [
        json!({"tool":TOOL_NAME,"arguments":{"path":"new/invalid.txt","content":"x".repeat(crate::providers::writes::MAX_BYTES + 1)}}),
        json!({"tool":TOOL_NAME,"arguments":{"path":"new/invalid.txt","content":"unsafe\rcontrol"}}),
        json!({"tool":TOOL_NAME,"arguments":{"path":"new/invalid.txt","content":"ok","grantRoot":"/"}}),
        json!({"tool":TOOL_NAME,"arguments":{"path":"excluded/file.txt","content":"ok"}}),
        json!({"tool":TOOL_NAME,"arguments":{"path":".eevee-data/file.txt","content":"ok"}}),
        json!({"tool":TOOL_NAME,"arguments":{"path":"../outside.txt","content":"ok"}}),
    ] {
        assert!(proposed_write(&mut writes, &params).is_err());
        assert_eq!(writes.payload(&[]), json!([]));
        assert!(!fixture.root.join("new").exists());
    }
    let content = format!("{}\r\n", "안".repeat(21_844));
    assert_eq!(content.len(), 65_534);
    let proposal = proposed_write(
        &mut writes,
        &json!({
            "tool":TOOL_NAME,
            "arguments":{"path":"new/한글/generated.txt","content":content}
        }),
    )
    .unwrap();
    assert_eq!(proposal.content, content);
    assert!(!proposal.description().contains('\r'));
    let permission = write_permission(&proposal);
    assert!(validate_answer(&permission, &json!({"decision":"decline"})).is_ok());
    assert!(!fixture.root.join("new").exists());
    // Abandoned/denied review is not an effect. A later appeared target remains
    // stale rather than being recaptured as replacement authority.
    std::fs::create_dir_all(fixture.root.join("new/한글")).unwrap();
    std::fs::write(fixture.root.join("new/한글/generated.txt"), "user").unwrap();
    assert!(
        proposed_write(
            &mut writes,
            &json!({
                "tool":TOOL_NAME,
                "arguments":{"path":"new/한글/generated.txt","content":"must not replace"}
            })
        )
        .is_err()
    );
    assert!(!writes.apply(&proposal).unwrap_err().uncertain);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("new/한글/generated.txt")).unwrap(),
        "user"
    );
    input.edit_paths = vec!["only.txt".into()];
    let mut selected = capture_writes(&fixture.root, &fixture.data, &input, &[])
        .unwrap()
        .unwrap();
    assert!(
        proposed_write(
            &mut selected,
            &json!({
                "tool":TOOL_NAME,
                "arguments":{"path":"unselected.txt","content":"no"}
            })
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn controlled_route_refuses_protected_unselected_and_escaping_paths() {
    let fixture = WriteFixture::new();
    let writes = fixture.capture(&["src/main.rs"]).unwrap();
    assert!(
        writes
            .propose("src/main.rs".into(), "selected\n".into())
            .is_ok()
    );
    for path in [
        ".git/config",
        ".env",
        "credentials.json",
        "../main.rs",
        "/tmp/outside",
        ".DS_Store",
        ".ev-install/file",
    ] {
        assert!(fixture.capture(&[path]).is_err(), "{path}");
    }
    assert!(
        writes
            .propose("src/unselected.rs".into(), "outside grant\n".into())
            .is_err()
    );
    assert!(
        WriteSet::capture(
            &fixture.root,
            &fixture.data,
            &["src/main.rs".into()],
            &["src".into()],
        )
        .is_err()
    );
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
    let fixture = WriteFixture::new();
    let path = fixture.root.join("target.txt");
    std::fs::write(&path, "user dirty line\nold requirement\n").unwrap();
    let mut writes = fixture.capture(&["target.txt"]).unwrap();
    assert_eq!(
        writes.payload(&[])[0]["working_content"],
        "user dirty line\nold requirement\n"
    );
    let after = "user dirty line\nnew requirement\n";
    let proposal = proposed_write(
        &mut writes,
        &json!({
            "tool": TOOL_NAME,
            "arguments": {"path": "target.txt", "content": after},
        }),
    )
    .unwrap();
    assert!(proposal.patch.contains("-old requirement\n"));
    assert!(proposal.patch.contains("+new requirement\n"));
    writes.apply(&proposal).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), after);
    let stale = writes
        .propose("target.txt".into(), "must not overwrite".into())
        .unwrap();
    std::fs::write(&path, "later user change\n").unwrap();
    let failure = writes.apply(&stale).unwrap_err();
    assert!(!failure.uncertain);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "later user change\n"
    );
    std::os::unix::fs::symlink("target.txt", fixture.root.join("linked.txt")).unwrap();
    assert!(fixture.capture(&["linked.txt"]).is_err());
    std::fs::hard_link(&path, fixture.root.join("hard.txt")).unwrap();
    assert!(fixture.capture(&["target.txt"]).is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "later user change\n"
    );
}
#[cfg(unix)]
#[test]
fn exact_write_permissions_do_not_offer_persistent_grants() {
    let fixture = WriteFixture::new();
    let writes = fixture.capture(&["nested/new.txt"]).unwrap();
    let proposal = writes
        .propose("nested/new.txt".into(), "새 내용\n".into())
        .unwrap();
    let kind = write_permission(&proposal);
    let PromptKind::Permission {
        description,
        options,
        file,
    } = &kind
    else {
        panic!("Expected permission")
    };
    assert_eq!(description, &proposal.description());
    assert!(description.contains(&proposal.patch));
    let file = file.as_ref().expect("typed file review");
    assert_eq!(file.path, "nested/new.txt");
    assert_eq!(file.kind, crate::providers::FileReviewKind::Create);
    assert_eq!(file.directories, ["nested"]);
    assert_eq!(file.bytes, "새 내용\n".len());
    assert_eq!(file.excerpt, "새 내용\n");
    assert!(!file.edit && !file.external);
    assert_eq!(options.len(), 2);
    assert_eq!(options[0].answer, json!({"decision":"decline"}));
    assert_eq!(options[1].answer, json!({"decision":"accept"}));
    assert!(!fixture.root.join("nested").exists());
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

#[cfg(unix)]
#[test]
fn codex_create_scope_supports_empty_unicode_siblings_and_later_replacement() {
    use crate::providers::writes::WriteKind;
    let fixture = WriteFixture::new();
    let mut writes = WriteSet::capture_workspace(&fixture.root, &fixture.data, &[]).unwrap();
    let empty = proposed_write(
        &mut writes,
        &json!({
            "tool": TOOL_NAME,
            "arguments": {"path": "새 폴더/하위/empty.txt", "content": ""},
        }),
    )
    .unwrap();
    assert_eq!(empty.kind, WriteKind::Create);
    assert_eq!(
        empty.directories,
        vec![PathBuf::from("새 폴더"), PathBuf::from("새 폴더/하위")]
    );
    assert!(empty.patch.starts_with("--- /dev/null\n"));
    let permission = write_permission(&empty);
    let PromptKind::Permission {
        file: Some(file), ..
    } = &permission
    else {
        panic!("typed file permission");
    };
    assert_eq!(file.path, "새 폴더/하위/empty.txt");
    assert_eq!(file.bytes, 0);
    assert!(file.excerpt.is_empty());
    assert!(validate_answer(&permission, &json!({"decision":"decline"})).is_ok());
    assert!(!fixture.root.join("새 폴더").exists());
    assert!(validate_answer(&permission, &json!({"decision":"accept"})).is_ok());
    writes.apply(&empty).unwrap();
    assert_eq!(
        std::fs::read(fixture.root.join("새 폴더/하위/empty.txt")).unwrap(),
        b""
    );
    writes.verify().unwrap();
    let sibling = proposed_write(
        &mut writes,
        &json!({
            "tool": TOOL_NAME,
            "arguments": {"path": "새 폴더/하위/sibling.txt", "content": "안녕하세요\n"},
        }),
    )
    .unwrap();
    assert_eq!(sibling.kind, WriteKind::Create);
    writes.apply(&sibling).unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("새 폴더/하위/sibling.txt")).unwrap(),
        "안녕하세요\n"
    );
    let replacement = proposed_write(
        &mut writes,
        &json!({
            "tool": TOOL_NAME,
            "arguments": {"path": "새 폴더/하위/empty.txt", "content": "now nonempty\n"},
        }),
    )
    .unwrap();
    assert_eq!(replacement.kind, WriteKind::Replace);
    assert!(replacement.directories.is_empty());
    let PromptKind::Permission {
        file: Some(file), ..
    } = write_permission(&replacement)
    else {
        panic!("typed file permission");
    };
    assert_eq!(file.kind, crate::providers::FileReviewKind::Modify);
    assert_eq!(file.bytes, replacement.content.len());
    assert_eq!(file.excerpt, "now nonempty\n");
    assert!(
        replacement
            .patch
            .starts_with("--- a/새 폴더/하위/empty.txt\n")
    );
    writes.apply(&replacement).unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("새 폴더/하위/empty.txt")).unwrap(),
        "now nonempty\n"
    );
}

#[cfg(unix)]
#[test]
fn codex_write_arguments_and_permission_answers_never_expand_selected_authority() {
    let fixture = WriteFixture::new();
    let mut writes = fixture.capture(&["only.txt"]).unwrap();
    for params in [
        json!({"tool":"unadvertised-tool","arguments":{"path":"only.txt","content":"text"}}),
        json!({"tool":TOOL_NAME,"namespace":"other","arguments":{"path":"only.txt","content":"text"}}),
        json!({"tool":TOOL_NAME,"arguments":{"path":"only.txt","content":"text","grantRoot":"/"}}),
        json!({"tool":TOOL_NAME,"arguments":{"path":"unselected.txt","content":"text"}}),
        json!({"tool":TOOL_NAME,"arguments":{"path":"only.txt","content":null}}),
        json!({"tool":TOOL_NAME,"arguments":"{\"path\":\"only.txt\",\"content\":\"text\"}"}),
    ] {
        assert!(proposed_write(&mut writes, &params).is_err());
    }
    let proposal = proposed_write(
        &mut writes,
        &json!({
            "tool": TOOL_NAME,
            "arguments": {"path": "only.txt", "content": "text"},
        }),
    )
    .unwrap();
    let permission = write_permission(&proposal);
    for answer in [
        json!({"decision":"accept","path":"unselected.txt"}),
        json!({"decision":"accept","content":"different"}),
        json!({"decision":"accept","grantRoot":"/"}),
        json!({"decision":"acceptForSession"}),
    ] {
        assert!(validate_answer(&permission, &answer).is_err());
    }
    assert!(!fixture.root.join("only.txt").exists());
    assert!(!fixture.root.join("unselected.txt").exists());
}

#[cfg(unix)]
#[test]
fn codex_creation_refuses_appeared_targets_and_parents_without_adopting_them() {
    let fixture = WriteFixture::new();
    let mut writes = fixture.capture(&["new-parent/file.txt"]).unwrap();
    let proposal = proposed_write(
        &mut writes,
        &json!({
            "tool": TOOL_NAME,
            "arguments": {"path": "new-parent/file.txt", "content": "must not publish"},
        }),
    )
    .unwrap();
    std::fs::create_dir(fixture.root.join("new-parent")).unwrap();
    assert!(writes.verify().is_err());
    assert!(!writes.apply(&proposal).unwrap_err().uncertain);
    assert!(!fixture.root.join("new-parent/file.txt").exists());

    let mut writes = fixture.capture(&["appeared.txt"]).unwrap();
    let proposal = proposed_write(
        &mut writes,
        &json!({
            "tool": TOOL_NAME,
            "arguments": {"path": "appeared.txt", "content": "must not replace"},
        }),
    )
    .unwrap();
    std::fs::write(fixture.root.join("appeared.txt"), "").unwrap();
    assert!(writes.verify().is_err());
    assert!(!writes.apply(&proposal).unwrap_err().uncertain);
    assert_eq!(
        std::fs::read(fixture.root.join("appeared.txt")).unwrap(),
        b""
    );
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
        public_context: None,
        tool_catalog: None,
        context_plan: None,
        screenshot: None,
        prior_plan: None,
        task: saved,
        work: None,
        local_only: false,
        instruction_override: None,
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
        conversation: None,
        credential: None,
    };
    let scope = WriteSet::capture_readonly(&cleanup.0, &data, &[]).unwrap();
    let text = payload(&data, &input, &[], Some(&scope)).unwrap();
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
#[cfg(unix)]
#[test]
fn baseline_cites_identical_selected_capture_instead_of_resending_bytes() {
    use crate::workspace::context::Excerpt;
    let fixture = WriteFixture::new();
    std::fs::create_dir(fixture.root.join("src")).unwrap();
    let text = "fn answer() -> u8 { 42 }\n";
    std::fs::write(fixture.root.join("src/lib.rs"), text).unwrap();
    let baseline = fixture.capture(&["src/lib.rs"]).unwrap();
    use sha2::{Digest, Sha256};
    let pinned = Excerpt {
        origin_run: None,
        id: 77,
        task_id: 1,
        root: fixture.root.to_str().unwrap().into(),
        path: "src/lib.rs".into(),
        start_line: 1,
        end_line: 1,
        source_sha256: Sha256::digest(text.as_bytes()).into(),
        text: "fn answer() -> u8 { 42 }\n".into(),
        available: true,
    };
    // Whole-file pin with identical text -> citation, not duplication.
    let payload = baseline.payload(std::slice::from_ref(&pinned));
    let entry = &payload[0];
    assert_eq!(entry["working_content_same_as_capture"], 77);
    assert!(entry.get("working_content").is_none());
    // A partial range never replaces the baseline's current bytes.
    let mut partial = pinned.clone();
    partial.start_line = 2;
    partial.text = "fn".into();
    let payload = baseline.payload(&[partial]);
    let entry = &payload[0];
    assert!(entry.get("working_content_same_as_capture").is_none());
    assert_eq!(entry["working_content"], "fn answer() -> u8 { 42 }\n");
    // Same path with different content stays separate (distinct evidence).
    let mut changed = pinned.clone();
    changed.text = "fn answer() -> u8 { 43 }\n".into();
    let payload = baseline.payload(&[changed]);
    let entry = &payload[0];
    assert!(entry.get("working_content_same_as_capture").is_none());
    // A different path with identical text also stays separate.
    let mut other = pinned;
    other.path = "src/other.rs".into();
    let payload = baseline.payload(&[other]);
    let entry = &payload[0];
    assert!(entry.get("working_content_same_as_capture").is_none());
}

// Wire shape follows the released generated schema, not `codex exec` JSON:
// https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/app-server-protocol/schema/json/v2/ThreadTokenUsageUpdatedNotification.json
// Local behavioral fixtures below are not live/account conformance evidence.
fn usage_notification(turn: &str, last: Value, total: Value) -> Value {
    json!({
        "method": "thread/tokenUsage/updated",
        "params": {
            "threadId": "usage-thread",
            "turnId": turn,
            "tokenUsage": { "last": last, "total": total, "modelContextWindow": 258400 }
        }
    })
}

fn usage_breakdown(input: u64, output: u64, cached: u64, reasoning: u64) -> Value {
    json!({
        "inputTokens": input, "outputTokens": output,
        "cachedInputTokens": cached, "reasoningOutputTokens": reasoning,
        "totalTokens": input + output
    })
}

fn usage_record(request: &str) -> crate::workspace::usage::Record {
    crate::workspace::usage::Record::new(
        request.into(),
        3,
        crate::store::Attribution {
            partner: 1,
            name: "Partner".into(),
            form: 0,
        },
        Agent::Codex,
    )
}

fn usage_reports(usage: &UsageSnapshots, turn: &str) -> Vec<Report> {
    let events = Arc::new(Mutex::new(VecDeque::new()));
    usage.emit(&events, 7, 3, "usage-thread", turn).unwrap();
    finish(
        &events,
        7,
        3,
        Outcome::Completed,
        "Fixture completed".into(),
    );
    let mut queue = events.lock();
    assert!(matches!(queue.pop_back(), Some(Event::Finished { .. })));
    queue
        .drain(..)
        .map(|event| match event {
            Event::Usage {
                generation: 7,
                task_id: 3,
                report,
            } => report,
            _ => panic!("Unexpected fixture event"),
        })
        .collect()
}

#[test]
fn usage_two_turns_and_unknown_resume_do_not_charge_prior_history() {
    let first_wire = usage_notification(
        "turn-one",
        usage_breakdown(100, 20, 40, 5),
        usage_breakdown(100, 20, 40, 5),
    );
    let mut first = UsageSnapshots::default();
    first
        .observe(
            &first_wire["params"],
            Some("usage-thread"),
            Some("turn-one"),
            true,
        )
        .unwrap();
    first.complete = true;
    let mut first_record = usage_record("first-request");
    for report in usage_reports(&first, "turn-one") {
        first_record.apply_retained(report).unwrap();
    }
    assert_eq!(first_record.measurements.len(), 2);
    let response = &first_record.measurements[0];
    assert_eq!(response.report.scope, Scope::Response);
    assert_eq!(response.attributable.input, Some(100));
    assert_eq!(response.report.identity.turn.as_deref(), Some("turn-one"));
    assert_eq!(response.report.model, None);
    assert_eq!(response.report.tokenizer, None);
    assert_eq!(response.report.semantics.cache_read_in_input, Some(true));
    assert_eq!(response.report.semantics.reasoning_in_output, Some(true));
    assert_eq!(response.attributable.cache_write, None); // optional, never synthesized zero
    assert!(first_record.measurements[1].attributable.is_unknown());

    // Resume the owned thread with an observed completed-turn baseline.
    let mut second = UsageSnapshots {
        baseline: Some(UsageBaseline {
            turn_id: "turn-one".into(),
            counters: first.total.unwrap(),
        }),
        ..UsageSnapshots::default()
    };
    second
        .observe(&first_wire["params"], Some("usage-thread"), None, false)
        .unwrap();
    assert!(second.last.is_none()); // historical `last` is never current usage
    let second_wire = usage_notification(
        "turn-two",
        usage_breakdown(60, 12, 10, 3),
        usage_breakdown(160, 32, 50, 8),
    );
    for _ in 0..3 {
        second
            .observe(
                &second_wire["params"],
                Some("usage-thread"),
                Some("turn-two"),
                true,
            )
            .unwrap();
    }
    second.complete = true;
    let mut second_record = usage_record("second-request");
    let reports = usage_reports(&second, "turn-two");
    for report in &reports {
        assert!(second_record.apply_retained(report.clone()).unwrap());
    }
    for report in &reports {
        assert!(!second_record.apply_retained(report.clone()).unwrap());
    }
    assert_eq!(second_record.measurements.len(), 2);
    let session = &second_record.measurements[1];
    assert_eq!(session.report.scope, Scope::Session);
    assert_eq!(session.report.counters.input, Some(160));
    assert_eq!(session.attributable.input, Some(60));
    assert_eq!(session.attributable.output, Some(12));
    assert_eq!(session.attributable.cache_read, Some(10));
    assert_eq!(session.attributable.reasoning, Some(3));

    // An unknown resumed baseline cannot be inferred from its first current total.
    let mut resumed = UsageSnapshots::default();
    resumed
        .observe(&first_wire["params"], Some("usage-thread"), None, false)
        .unwrap();
    resumed
        .observe(
            &second_wire["params"],
            Some("usage-thread"),
            Some("turn-two"),
            true,
        )
        .unwrap();
    resumed.complete = true;
    let mut resumed_record = usage_record("unknown-resume");
    for report in usage_reports(&resumed, "turn-two") {
        resumed_record.apply_retained(report).unwrap();
    }
    assert_eq!(resumed_record.measurements[0].attributable.input, Some(60));
    assert_eq!(resumed_record.measurements[1].report.baseline, None);
    assert!(resumed_record.measurements[1].attributable.is_unknown());
}

#[test]
fn usage_agentic_last_replaces_response_snapshot_and_partial_finals_merge() {
    let mut usage = UsageSnapshots::default();
    let initial = usage_notification(
        "agentic-turn",
        usage_breakdown(100, 20, 40, 5),
        usage_breakdown(100, 20, 40, 5),
    );
    usage
        .observe(
            &initial["params"],
            Some("usage-thread"),
            Some("agentic-turn"),
            true,
        )
        .unwrap();
    let mut record = usage_record("agentic-request");
    for report in usage_reports(&usage, "agentic-turn") {
        record.apply_retained(report).unwrap();
    }
    // Another model response in the same turn: last is replaced, not summed and
    // not described as the complete 160-input-token agentic turn.
    let next = usage_notification(
        "agentic-turn",
        usage_breakdown(60, 12, 10, 3),
        usage_breakdown(160, 32, 50, 8),
    );
    usage
        .observe(
            &next["params"],
            Some("usage-thread"),
            Some("agentic-turn"),
            true,
        )
        .unwrap();
    let partial = usage_notification(
        "agentic-turn",
        json!({"inputTokens":null,"cacheWriteInputTokens":2}),
        json!({"outputTokens":null,"cacheWriteInputTokens":7}),
    );
    usage
        .observe(
            &partial["params"],
            Some("usage-thread"),
            Some("agentic-turn"),
            true,
        )
        .unwrap();
    usage.complete = true;
    let reports = usage_reports(&usage, "agentic-turn");
    for report in &reports {
        record.apply_retained(report.clone()).unwrap();
    }
    for report in reports {
        assert!(!record.apply_retained(report).unwrap());
    }
    assert_eq!(record.measurements.len(), 2);
    let response = &record.measurements[0];
    assert_eq!(response.report.scope, Scope::Response);
    assert_eq!(response.attributable.input, Some(60));
    assert_eq!(response.attributable.output, Some(12));
    assert_eq!(response.attributable.cache_write, Some(2));
    assert!(response.report.complete);
    assert_eq!(record.measurements[1].report.counters.input, Some(160));
    assert_eq!(record.measurements[1].report.counters.output, Some(32));
    assert_eq!(record.measurements[1].report.counters.cache_write, Some(7));
}

#[test]
fn usage_requires_exact_thread_and_already_bound_current_turn() {
    let wire = usage_notification(
        "current-turn",
        usage_breakdown(100, 20, 40, 5),
        usage_breakdown(1000, 200, 400, 50),
    );
    for (thread, turn, dispatched) in [
        (Some("other-thread"), Some("current-turn"), true),
        (Some("usage-thread"), Some("other-turn"), true),
        (Some("usage-thread"), None, true),
        (None, Some("current-turn"), true),
        (Some("usage-thread"), None, false),
    ] {
        let mut usage = UsageSnapshots::default();
        usage
            .observe(&wire["params"], thread, turn, dispatched)
            .unwrap();
        assert!(usage.last.is_none());
        assert!(usage.total.is_none());
        assert!(usage_reports(&usage, "current-turn").is_empty());
    }
    let mut usage = UsageSnapshots::default();
    let mut missing_turn = wire["params"].clone();
    missing_turn.as_object_mut().unwrap().remove("turnId");
    usage
        .observe(
            &missing_turn,
            Some("usage-thread"),
            Some("current-turn"),
            true,
        )
        .unwrap();
    assert!(usage.total.is_none());
    let mut wrong_shape = wire["params"].clone();
    wrong_shape["tokenUsage"] = json!({"input_tokens":1000,"output_tokens":200});
    usage
        .observe(
            &wrong_shape,
            Some("usage-thread"),
            Some("current-turn"),
            true,
        )
        .unwrap();
    assert!(usage.total.is_none()); // exec/Responses usage is not this wire contract
}

#[test]
fn usage_optional_counters_reject_invalid_values_without_defaulting_to_zero() {
    let counters = usage_counters(&json!({"inputTokens":null,"outputTokens":0})).unwrap();
    assert_eq!(counters.input, None);
    assert_eq!(counters.output, Some(0));
    assert_eq!(counters.cache_read, None);
    for value in [json!(-1), json!(1.5), json!("12"), json!(true)] {
        assert!(usage_counters(&json!({"inputTokens":value})).is_err());
    }
    assert!(usage_counters(&Value::Null).is_err());
}

#[test]
fn usage_predispatch_baseline_refresh_is_exact_and_freezes_at_submission() {
    let mut usage = UsageSnapshots {
        baseline: Some(UsageBaseline {
            turn_id: "previous-turn".into(),
            counters: Counters {
                input: Some(100),
                ..Counters::default()
            },
        }),
        ..UsageSnapshots::default()
    };
    let observed = usage_notification(
        "previous-turn",
        usage_breakdown(100, 20, 40, 5),
        json!({"inputTokens":105,"outputTokens":null,"cachedInputTokens":40}),
    );
    usage
        .observe(&observed["params"], Some("usage-thread"), None, false)
        .unwrap();
    assert_eq!(usage.baseline.as_ref().unwrap().counters.input, Some(105));
    assert_eq!(usage.baseline.as_ref().unwrap().counters.output, None);
    assert!(usage.last.is_none());
    assert!(usage.total.is_none());

    let unrelated = usage_notification(
        "unrelated-turn",
        usage_breakdown(9000, 9000, 0, 0),
        usage_breakdown(9000, 9000, 0, 0),
    );
    usage
        .observe(&unrelated["params"], Some("usage-thread"), None, false)
        .unwrap();
    let current = usage_notification(
        "submitted-turn",
        usage_breakdown(45, 12, 10, 3),
        usage_breakdown(150, 32, 50, 8),
    );
    usage
        .observe(
            &current["params"],
            Some("usage-thread"),
            Some("submitted-turn"),
            true,
        )
        .unwrap();
    // Even a well-shaped delayed previous-turn notification cannot shift the
    // submitted request's baseline or replace its current counters.
    usage
        .observe(
            &observed["params"],
            Some("usage-thread"),
            Some("submitted-turn"),
            true,
        )
        .unwrap();
    usage.complete = true;
    let mut record = usage_record("baseline-refresh");
    for report in usage_reports(&usage, "submitted-turn") {
        record.apply_retained(report).unwrap();
    }
    let session = &record.measurements[1];
    assert_eq!(session.report.counters.input, Some(150));
    assert_eq!(session.attributable.input, Some(45));
    assert_eq!(session.attributable.cache_read, Some(10));
    assert_eq!(session.attributable.output, None);
    assert_eq!(session.attributable.reasoning, None);
}

#[cfg(unix)]
#[test]
fn controlled_dynamic_tools_discover_read_edit_and_create_using_real_executor() {
    let fixture = WriteFixture::new();
    std::fs::write(
        fixture.root.join("existing.rs"),
        "// dirty\r\nfn old() {}\r\n",
    )
    .unwrap();
    let mut writes = WriteSet::capture_workspace(&fixture.root, &fixture.data, &[]).unwrap();
    let list = requested_tool(
        &json!({"tool":"eevee_list_files","arguments":{"directory":"","query":".rs","offset":0}}),
    )
    .unwrap();
    assert_eq!(
        writes.observe_tool(&list).unwrap()["paths"],
        json!(["existing.rs"])
    );
    assert_eq!(writes.payload(&[]), json!([]));
    let read = requested_tool(&json!({"tool":"eevee_read_file","arguments":{"path":"existing.rs","start_line":1,"end_line":0}})).unwrap();
    let observed = writes.observe_tool(&read).unwrap();
    assert_eq!(observed["content"], "// dirty\r\nfn old() {}\r\n");
    let edit = proposed_write(&mut writes, &json!({"tool":"eevee_edit_file","arguments":{
        "path":"existing.rs","sha256":observed["sha256"],"replacements":[{"old_text":"fn old()","new_text":"fn new()"}]
    }})).unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("existing.rs")).unwrap(),
        "// dirty\r\nfn old() {}\r\n"
    );
    writes.apply(&edit).unwrap();
    assert_eq!(
        std::fs::read(fixture.root.join("existing.rs")).unwrap(),
        b"// dirty\r\nfn new() {}\r\n"
    );
    let created = proposed_write(&mut writes, &json!({"tool":TOOL_NAME,"arguments":{
        "path":"new/generated.rs","content":format!("// derived from {} lines\nfn generated() {{}}\n", observed["total_lines"])
    }})).unwrap();
    writes.apply(&created).unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("new/generated.rs")).unwrap(),
        created.content
    );
    assert!(
        requested_tool(
            &json!({"tool":"eevee_read_file","namespace":"external","arguments":{
                "path":"existing.rs","start_line":1,"end_line":0
            }})
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn controlled_delete_and_rename_share_exact_permission_and_real_executor() {
    let fixture = WriteFixture::new();
    let original = "// dirty 한글\r\nfn source() {}\r\n";
    std::fs::write(fixture.root.join("source.rs"), original).unwrap();
    std::fs::write(fixture.root.join("neighbor.txt"), "unrelated dirty\r\n").unwrap();
    let mut writes = WriteSet::capture_workspace(&fixture.root, &fixture.data, &[]).unwrap();
    let read = requested_tool(&json!({"tool":"eevee_read_file","arguments":{
        "path":"source.rs","start_line":1,"end_line":0
    }}))
    .unwrap();
    let source = writes.observe_tool(&read).unwrap();
    let rename = proposed_write(
        &mut writes,
        &json!({"tool":"eevee_rename_file","arguments":{
            "path":"source.rs","destination":"nested/한글.rs","sha256":source["sha256"]
        }}),
    )
    .unwrap();
    let PromptKind::Permission {
        file: Some(review),
        options,
        ..
    } = write_permission(&rename)
    else {
        panic!("Missing exact rename permission")
    };
    assert_eq!(review.kind, crate::providers::FileReviewKind::Rename);
    assert_eq!(review.destination.as_deref(), Some("nested/한글.rs"));
    assert_eq!(options.len(), 2);
    assert_eq!(options[0].answer, json!({"decision":"decline"}));
    assert!(
        !fixture.root.join("nested").exists(),
        "deny/cancel preparation has no effect"
    );
    let outcome = writes.apply(&rename).unwrap();
    assert_eq!(
        outcome.destination.as_deref(),
        Some(Path::new("nested/한글.rs"))
    );
    assert!(!fixture.root.join("source.rs").exists());
    assert_eq!(
        std::fs::read(fixture.root.join("nested/한글.rs")).unwrap(),
        original.as_bytes()
    );
    let read = requested_tool(&json!({"tool":"eevee_read_file","arguments":{
        "path":"nested/한글.rs","start_line":1,"end_line":0
    }}))
    .unwrap();
    let source = writes.observe_tool(&read).unwrap();
    let delete = proposed_write(
        &mut writes,
        &json!({"tool":"eevee_delete_file","arguments":{
            "path":"nested/한글.rs","sha256":source["sha256"]
        }}),
    )
    .unwrap();
    let PromptKind::Permission {
        file: Some(review),
        options,
        ..
    } = write_permission(&delete)
    else {
        panic!("Missing exact deletion permission")
    };
    assert_eq!(review.kind, crate::providers::FileReviewKind::Delete);
    assert_eq!(options[0].answer, json!({"decision":"decline"}));
    assert!(fixture.root.join("nested/한글.rs").exists());
    let outcome = writes.apply(&delete).unwrap();
    let receipt: Value = serde_json::from_str(&outcome.receipt()).unwrap();
    assert_eq!(receipt["source_state"], "absent");
    assert!(receipt["source_sha256"].is_null());
    assert!(!fixture.root.join("nested/한글.rs").exists());
    assert_eq!(
        std::fs::read(fixture.root.join("neighbor.txt")).unwrap(),
        b"unrelated dirty\r\n"
    );
    writes.verify().unwrap();
}

#[test]
fn retained_command_artifact_declarations_parse_into_host_lookup_operations() {
    let declarations = crate::providers::artifacts::definitions();
    for (name, arguments) in [
        ("eevee_artifact_list", json!({})),
        (
            "eevee_artifact_read",
            json!({"id":"run-artifact","revision":"exact","offset":0,"limit":65536}),
        ),
        (
            "eevee_artifact_search",
            json!({"id":"run-artifact","revision":"exact","offset":0,"limit":4,"query":"error"}),
        ),
        (
            "eevee_artifact_locations",
            json!({"id":"run-artifact","revision":"exact","offset":0}),
        ),
    ] {
        assert!(declarations.iter().any(|entry| entry["name"] == name));
        let tool = requested_tool(&json!({"tool":name,"arguments":arguments})).unwrap();
        assert!(tool.artifact_operation().is_some());
        assert!(!tool.mutating());
    }
    assert!(crate::providers::commands::outcome_unknown(
        &json!({"run":{"state":"OutcomeUnknown","tree":"OwnershipUnknown"}})
    ));
    assert!(!crate::providers::commands::outcome_unknown(
        &json!({"run":{"state":"Failed","tree":"OwnedTreeExited","exit_code":1}})
    ));
}
