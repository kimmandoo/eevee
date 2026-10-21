use super::*;

#[test]
fn sdk_releases_are_observed_identity_not_an_admission_allowlist() {
    for (sdk, cli) in [
        ("0.3.296", "2.1.296"),
        ("0.3.999", "2.1.999"),
        ("1.5.0", "4.0.0"),
    ] {
        let metadata =
            json!({"name":"@anthropic-ai/claude-agent-sdk","version":sdk,"claudeCodeVersion":cli});
        let versions = sdk_versions(&metadata).unwrap();
        assert_eq!(versions.sdk, sdk);
        assert_eq!(versions.cli, cli);
    }
    let metadata = json!({"name":"@anthropic-ai/claude-agent-sdk","version":"0.3.999","claudeCodeVersion":"2.1.999"});
    for (field, value) in [
        ("name", json!("unrelated-sdk")),
        ("version", json!("latest")),
        ("version", json!("")),
        ("claudeCodeVersion", Value::Null),
        ("claudeCodeVersion", json!("2.1.999\n")),
        ("version", json!("1".repeat(129))),
    ] {
        let mut changed = metadata.clone();
        changed[field] = value;
        assert!(sdk_versions(&changed).is_err());
    }
}

#[test]
fn sdk_inspection_requires_reviewed_identity_query_and_complete_bridge_policy() {
    let versions = SdkVersions {
        sdk: "1.5.0".into(),
        cli: "4.0.0".into(),
    };
    let inspection = json!({
        "type":"capabilities", "sdk_version":"1.5.0", "cli_version":"4.0.0",
        "bridge_revision":6, "query":true,
        "policy":{"tools":"closed-allowlist","permissions":"default-host-once","mcp":"disabled",
            "configuration":"isolated","authentication":"api-console-cloud-only",
            "dispatch":"correlated-sdk-submission","usage":"correlated-sdk-snapshots"},
    });
    assert!(inspection_matches(&inspection, &versions));
    for (field, value) in [
        ("sdk_version", json!("1.5.1")),
        ("cli_version", json!("4.0.1")),
        ("bridge_revision", json!(5)),
        ("query", json!(false)),
        ("policy", Value::Null),
    ] {
        let mut changed = inspection.clone();
        changed[field] = value;
        assert!(!inspection_matches(&changed, &versions));
    }
    for field in [
        "tools",
        "permissions",
        "mcp",
        "configuration",
        "authentication",
        "dispatch",
        "usage",
    ] {
        let mut changed = inspection.clone();
        changed["policy"][field] = json!("unknown");
        assert!(!inspection_matches(&changed, &versions));
        changed["policy"].as_object_mut().unwrap().remove(field);
        assert!(!inspection_matches(&changed, &versions));
    }
}

#[test]
fn model_catalogue_consumer_rejects_private_unbounded_or_ambiguous_metadata() {
    let catalogue = json!({
        "models": [{"id":"live-a","name":"Live A","description":"SDK capability","efforts":["high"],"default_effort":null}],
        "current":null,"source":"SDK supportedModels()","effort_note":"Only advertised effort choices",
    });
    let decoded = decode_catalogue(&catalogue).unwrap();
    assert_eq!(decoded.models[0].efforts, ["high"]);
    assert_eq!(decoded.models[0].default_effort, None);
    let mut empty = catalogue.clone();
    empty["models"] = json!([]);
    assert!(decode_catalogue(&empty).is_ok());
    let mut duplicate = catalogue.clone();
    duplicate["models"]
        .as_array_mut()
        .unwrap()
        .push(catalogue["models"][0].clone());
    assert!(decode_catalogue(&duplicate).is_err());
    for text in [
        "token=canary".to_string(),
        "\u{202e}model".into(),
        "x".repeat(257),
    ] {
        let mut invalid = catalogue.clone();
        invalid["models"][0]["id"] = json!(text);
        assert!(decode_catalogue(&invalid).is_err());
    }
    let mut invalid = catalogue.clone();
    invalid["models"][0]["default_effort"] = json!("unadvertised");
    assert!(decode_catalogue(&invalid).is_err());
    invalid = catalogue.clone();
    invalid["models"][0]["extra"] = json!(true);
    assert!(decode_catalogue(&invalid).is_err());
}
#[test]
fn review_uses_mandatory_read_only_sdk_tools_not_implementation_policy() {
    assert_eq!(sdk_tool_policy(Operation::Review), "plan");
    assert_eq!(sdk_tool_policy(Operation::Plan), "plan");
    assert_eq!(sdk_tool_policy(Operation::Implement), "implement");
}

#[test]
fn malformed_trailing_frames_do_not_discard_prior_observations() {
    let observation = json!({"protocol":1,"type":"item","kind":"tool","text":"partial"});
    let mut fault = None;
    assert_eq!(
        preserve_observations(vec![observation.clone()], &mut fault, "malformed").unwrap(),
        vec![observation]
    );
    assert_eq!(fault.take().as_deref(), Some("malformed"));
    assert!(preserve_observations(Vec::new(), &mut fault, "malformed").is_err());
}
#[test]
fn cloud_auth_chains_require_explicit_consent_and_selected_backend() {
    let route = |pairs: &[(&str, &str)]| {
        let profile = pairs
            .iter()
            .find(|(name, _)| *name == "console-profile")
            .map(|(_, value)| *value);
        credential_route_from(
            |key| {
                pairs
                    .iter()
                    .any(|(name, value)| *name == key && !value.is_empty())
            },
            |key, expected| {
                pairs
                    .iter()
                    .any(|(name, value)| *name == key && *value == expected)
            },
            profile,
        )
    };
    let opt_in = ("EEVEE_CLAUDE_CLOUD_AUTH", "provider-chain");
    let bedrock = ("CLAUDE_CODE_USE_BEDROCK", "1");
    let vertex = ("CLAUDE_CODE_USE_VERTEX", "1");
    assert!(route(&[bedrock, ("AWS_PROFILE", "work")]).is_err());
    assert!(
        route(&[bedrock, opt_in, ("AWS_PROFILE", "work")])
            .unwrap()
            .contains("provider credential chain")
    );
    assert!(
        route(&[bedrock, ("AWS_BEARER_TOKEN_BEDROCK", "unit-test")])
            .unwrap()
            .contains("bearer")
    );
    assert!(
        route(&[
            bedrock,
            ("AWS_WEB_IDENTITY_TOKEN_FILE", "/selected/cloud-file"),
            ("AWS_ROLE_ARN", "unit-role")
        ])
        .unwrap()
        .contains("web identity")
    );
    assert!(
        route(&[
            vertex,
            opt_in,
            ("ANTHROPIC_VERTEX_PROJECT_ID", "unit-project")
        ])
        .unwrap()
        .contains("ADC")
    );
    assert!(route(&[vertex, opt_in]).is_err());
    assert!(route(&[opt_in]).is_err());
    assert!(route(&[("ANTHROPIC_API_KEY", "unit-test"), opt_in]).is_err());
    assert!(route(&[bedrock, vertex, opt_in]).is_err());
    let console = ("console-profile", "eevee");
    assert!(
        route(&[console])
            .unwrap()
            .contains("Console OAuth profile eevee")
    );
    assert!(route(&[console, ("ANTHROPIC_API_KEY", "unit-test")]).is_err());
    assert!(route(&[console, bedrock, opt_in, ("AWS_PROFILE", "work")]).is_err());
    assert!(route(&[console, opt_in]).is_err());
}
#[test]
fn sdk_callbacks_never_offer_hidden_or_unsupported_grants() {
    let deny = json!({"label":"Deny","answer":{"behavior":"deny","message":"No"}});
    let oversized = json!({"kind":"approval","input":{"options":[deny.clone(), {"label":"Allow","answer":{"behavior":"allow","updatedInput":{"content":"x".repeat(ITEM_LIMIT)}}}]}});
    assert!(prompt_kind(&oversized).is_err());
    let elicitation = json!({"kind":"elicitation","input":{"options":[{"label":"Decline","answer":{"action":"decline"}},{"label":"Accept","answer":{"action":"accept"}}]}});
    assert!(prompt_kind(&elicitation).is_err());
    let escalation = json!({"kind":"approval","input":{"options":[deny, {"label":"Allow","answer":{"behavior":"allow","updatedInput":{},"updatedPermissions":[]}}]}});
    assert!(prompt_kind(&escalation).is_err());
}
#[test]
fn sdk_tool_snapshots_retain_partial_and_preserve_terminal_identity() {
    let mut items = SdkItems::default();
    let pending =
        json!({"item_id":"tool-1","kind":"tool","text":"requested edit","complete":false});
    assert!(!items.observe(&pending).unwrap().complete);
    assert_eq!(items.items["tool-1"].text, "requested edit");
    let finished =
        json!({"item_id":"tool-1","kind":"tool","text":"provider result","complete":true});
    assert!(items.observe(&finished).unwrap().complete);
    assert_eq!(items.items.len(), 1);
    assert!(items.observe(&pending).is_err());
    assert!(items.observe(&json!({"item_id":"tool-1","kind":"message","text":"provider result","complete":true})).is_err());
    assert!(items.observe(&finished).is_ok());
}

#[test]
fn sdk_cumulative_message_snapshots_release_words_and_hide_every_secret_split() {
    let mut items = SdkItems::default();
    for (text, expected) in [
        ("First ordinary ", "First ordinary "),
        (
            "First ordinary words 日本語 tail",
            "First ordinary words 日本語 ",
        ),
    ] {
        let value = json!({"item_id":"message","kind":"message","text":text,"complete":false});
        assert_eq!(items.observe(&value).unwrap().text, expected);
    }
    for text in [
        "前 sk-secret123456 後 ",
        "前 password=\"opaque value\" 後 ",
        "前 https://user:opaque@example.test/path 後 ",
        "前 -----BEGIN PRIVATE KEY-----\nopaque\n-----END PRIVATE KEY----- 後 ",
        "前 sk\r-secret123456 後 ",
    ] {
        let mut items = SdkItems::default();
        let final_text = safe(text);
        for end in text
            .char_indices()
            .map(|(index, _)| index)
            .chain([text.len()])
        {
            let value =
                json!({"item_id":"message","kind":"message","text":&text[..end],"complete":false});
            assert!(final_text.starts_with(&items.observe(&value).unwrap().text));
        }
        let value = json!({"item_id":"message","kind":"message","text":text,"complete":true});
        assert_eq!(items.observe(&value).unwrap().text, final_text);
        assert!(items.observe(&value).is_ok());
    }
}
#[test]
fn provider_permission_options_preserve_exact_answers_without_persistent_grants() {
    let value = json!({"kind":"approval","input":{"options":[{"label":"Deny","answer":{"behavior":"deny","message":"No"}},{"label":"Allow once","answer":{"behavior":"allow","updatedInput":{"file_path":"a.rs"}}}]}});
    let kind = prompt_kind(&value).unwrap();
    assert!(provider_answer(&kind, json!({"behavior":"allow"})).is_err());
    assert!(
        provider_answer(
            &kind,
            json!({"behavior":"allow","updatedInput":{"file_path":"a.rs"},"updatedPermissions":[]})
        )
        .is_err()
    );
    assert!(provider_answer(&kind, value["input"]["options"][1]["answer"].clone()).is_ok());
}
#[test]
fn questions_are_distinct_from_tool_and_elicitation_answers() {
    let value = json!({"kind":"questions","input":{"questions":[{"question":"Which database?","header":"Database","multiSelect":false,"options":[{"label":"SQLite","description":"Local"}]}]}});
    let kind = prompt_kind(&value).unwrap();
    assert_eq!(
        provider_answer(&kind, json!({"answers":{"0":{"answers":["SQLite"]}}})).unwrap(),
        json!({"behavior":"allow","updatedInput":{"answers":{"Which database?":"SQLite"}}})
    );
    assert!(provider_answer(&kind, json!({"action":"accept"})).is_err());
    assert!(provider_answer(&kind, json!({"answers":{"stale":{"answers":["SQLite"]}}})).is_err());
}
#[test]
fn terminal_controls_and_recognizable_credentials_are_never_rendered() {
    assert!(!safe("\u{1b}[31msecret token=canary\u{202e}").contains("\u{1b}"));
    assert!(!safe("token=canary").contains("canary"));
}

#[test]
fn sdk_dispatch_frames_require_exact_frozen_owner_and_reject_duplicates_or_inspection() {
    let input = crate::providers::tests::claimed(Agent::Claude, Operation::Plan, false);
    let frame = json!({
        "protocol":1, "type":"dispatched", "generation":31, "task_id":7,
        "request_id":"provider-admission-test", "submission_id":"11234567-89ab-4cde-8123-456789abcdef",
    });
    for (field, changed) in [
        ("generation", json!(32)),
        ("task_id", json!(8)),
        ("request_id", json!("another-request")),
        ("request_id", Value::Null),
        ("submission_id", Value::Null),
        ("submission_id", json!("not-a-uuid")),
        ("type", json!("status")),
    ] {
        let mut value = frame.clone();
        value[field] = changed;
        let mut seen = None;
        assert!(dispatch_receipt(&input, 31, &value, &mut seen).is_err());
        assert!(seen.is_none());
    }
    let mut seen = None;
    assert!(matches!(
        dispatch_receipt(&input, 31, &frame, &mut seen).unwrap(),
        Event::Dispatched { generation:31, task_id:7, request_id }
            if request_id == "provider-admission-test"
    ));
    assert_eq!(seen.as_deref(), frame["submission_id"].as_str());
    assert!(dispatch_receipt(&input, 31, &frame, &mut seen).is_err());
    for models in [false, true] {
        let mut inspection = input.clone();
        inspection.inspect_models = models;
        inspection.inspect_sessions = !models;
        assert!(dispatch_receipt(&inspection, 31, &frame, &mut None).is_err());
    }
    let mut unclaimed = input;
    unclaimed.work = None;
    assert!(dispatch_receipt(&unclaimed, 31, &frame, &mut None).is_err());
}

fn sdk_usage_fixture(submission: &str, scope: &str, input: u64, baseline: Option<u64>) -> Value {
    json!({
        "protocol":1,"type":"usage","generation":31,"task_id":7,
        "request_id":"provider-admission-test","submission_id":submission,
        "session_id":"01234567-89ab-4cde-8123-456789abcdef",
        "report":{
            "identity":{"session":"01234567-89ab-4cde-8123-456789abcdef","turn":submission,"response":null,"event":"sdk-result"},
            "scope":scope,"kind":"Snapshot",
            "counters":{"input":input,"output":null,"cache_read":null,"cache_write":null,"reasoning":null},
            "baseline":baseline.map(|input| json!({"input":input,"output":null,"cache_read":null,"cache_write":null,"reasoning":null})),
            "model":if scope == "ModelPipeline" { Some("provider-model-alias") } else { None },
            "tokenizer":null,
            "semantics":{"cache_read_in_input":false,"cache_write_in_input":false,"reasoning_in_output":null},
            "complete":true,
        }
    })
}

fn sdk_decoded_report(
    input: &Input,
    submission: &str,
    frame: &Value,
) -> crate::workspace::usage::Report {
    let Event::Usage {
        generation: 31,
        task_id: 7,
        report,
    } = usage_event(
        input,
        31,
        "01234567-89ab-4cde-8123-456789abcdef",
        Some(submission),
        frame,
    )
    .unwrap()
    else {
        panic!("expected typed SDK usage");
    };
    report
}

fn sdk_worker_report(
    usage: &mut SdkTurnUsage,
    input: &Input,
    submission: &str,
    frame: &Value,
) -> crate::workspace::usage::Report {
    let event = usage_event(
        input,
        31,
        "01234567-89ab-4cde-8123-456789abcdef",
        Some(submission),
        frame,
    )
    .unwrap();
    let Event::Usage { report, .. } = usage.observe(event).unwrap() else {
        panic!("expected correlated worker usage");
    };
    report
}

#[test]
fn sdk_usage_requires_acknowledged_current_uuid_and_frozen_request() {
    let input = crate::providers::tests::claimed(Agent::Claude, Operation::Plan, false);
    let submission = "11234567-89ab-4cde-8123-456789abcdef";
    let frame = sdk_usage_fixture(submission, "MainLoopTurn", 10, None);
    let session = frame["session_id"].as_str().unwrap();
    assert!(usage_event(&input, 31, session, None, &frame).is_err());
    for (field, value) in [
        ("generation", json!(32)),
        ("task_id", json!(8)),
        ("session_id", json!("other-session")),
        ("request_id", json!("another-request")),
        ("submission_id", json!("prior-turn")),
        (
            "report",
            json!({"scope":"query-pipeline cumulative snapshot"}),
        ),
    ] {
        let mut changed = frame.clone();
        changed[field] = value;
        assert!(usage_event(&input, 31, session, Some(submission), &changed).is_err());
    }
    let mut changed = frame.clone();
    changed["report"]["identity"]["turn"] = json!("prior-history-uuid");
    assert!(usage_event(&input, 31, session, Some(submission), &changed).is_err());
    let mut unclaimed = input.clone();
    unclaimed.work = None;
    assert!(usage_event(&unclaimed, 31, session, Some(submission), &frame).is_err());
    for models in [false, true] {
        let mut inspection = input.clone();
        inspection.inspect_models = models;
        inspection.inspect_sessions = !models;
        assert!(usage_event(&inspection, 31, session, Some(submission), &frame).is_err());
    }
    let report = sdk_decoded_report(&input, submission, &frame);
    assert_eq!(report.counters.output, None);
    assert_eq!(report.model, None);
    assert_eq!(report.tokenizer, None);
}

#[test]
fn sdk_worker_retains_completed_owned_resume_baselines_without_overlapping_scopes() {
    use crate::workspace::usage::Record;
    let mut input = crate::providers::tests::claimed(Agent::Claude, Operation::Plan, false);
    let owner = input.work.as_ref().unwrap().owner.clone();
    let session = "01234567-89ab-4cde-8123-456789abcdef";
    let mut sessions = HashMap::new();
    let first_id = "11234567-89ab-4cde-8123-456789abcdef";
    let second_id = "21234567-89ab-4cde-8123-456789abcdef";
    let resume_id = "31234567-89ab-4cde-8123-456789abcdef";
    let mut usage = SdkTurnUsage::begin(&input, &mut sessions).unwrap();
    sessions.insert(
        session.into(),
        SdkSession {
            task_id: 7,
            ..SdkSession::default()
        },
    );
    let mut first = Record::new("request-1".into(), 7, owner.clone(), Agent::Claude);
    for (scope, total) in [("MainLoopTurn", 10), ("ModelPipeline", 100)] {
        first
            .apply_retained(sdk_worker_report(
                &mut usage,
                &input,
                first_id,
                &sdk_usage_fixture(first_id, scope, total, None),
            ))
            .unwrap();
    }
    usage.finish(sessions.get_mut(session).unwrap(), Outcome::Completed);
    assert_eq!(first.measurements[0].attributable.input, Some(10));
    assert_eq!(first.measurements[1].attributable.input, None);

    // Each query has a fresh sidecar ledger: raw pipeline baseline is null.
    // The existing worker privately freezes its last complete same-model total.
    input.resume = Some(session.into());
    let mut usage = SdkTurnUsage::begin(&input, &mut sessions).unwrap();
    assert!(sessions[session].model_totals.is_empty());
    let mut second = Record::new("request-2".into(), 7, owner.clone(), Agent::Claude);
    second
        .apply_retained(sdk_worker_report(
            &mut usage,
            &input,
            second_id,
            &sdk_usage_fixture(second_id, "MainLoopTurn", 20, None),
        ))
        .unwrap();
    let frame = sdk_usage_fixture(second_id, "ModelPipeline", 120, None);
    let cumulative = sdk_worker_report(&mut usage, &input, second_id, &frame);
    assert_eq!(cumulative.baseline.unwrap().input, Some(100));
    assert_eq!(cumulative.identity.session.as_deref(), Some(session));
    assert_eq!(cumulative.model.as_deref(), Some("provider-model-alias"));
    assert!(second.apply_retained(cumulative.clone()).unwrap());
    assert!(!second.apply_retained(cumulative).unwrap());
    let mut partial = frame;
    partial["report"]["counters"]["input"] = Value::Null;
    partial["report"]["counters"]["output"] = json!(9);
    second
        .apply_retained(sdk_worker_report(&mut usage, &input, second_id, &partial))
        .unwrap();
    usage.finish(sessions.get_mut(session).unwrap(), Outcome::Completed);
    assert_eq!(second.measurements.len(), 2);
    assert_eq!(second.measurements[0].attributable.input, Some(20));
    assert_eq!(second.measurements[1].attributable.input, Some(20));
    assert_eq!(second.measurements[1].attributable.output, None);
    assert_eq!(
        sessions[session].model_totals["provider-model-alias"].input,
        Some(120)
    );

    let mut usage = SdkTurnUsage::begin(&input, &mut sessions).unwrap();
    let mut resumed = Record::new("request-resumed".into(), 7, owner, Agent::Claude);
    for (scope, total) in [("MainLoopTurn", 7), ("ModelPipeline", 127)] {
        resumed
            .apply_retained(sdk_worker_report(
                &mut usage,
                &input,
                resume_id,
                &sdk_usage_fixture(resume_id, scope, total, None),
            ))
            .unwrap();
    }
    assert_eq!(resumed.measurements[0].attributable.input, Some(7));
    assert_eq!(resumed.measurements[1].attributable.input, Some(7));
}

#[test]
fn sdk_resume_unknown_models_failed_turns_and_wrong_owners_never_seed_baselines() {
    let mut input = crate::providers::tests::claimed(Agent::Claude, Operation::Plan, false);
    let session = "01234567-89ab-4cde-8123-456789abcdef";
    let submission = "11234567-89ab-4cde-8123-456789abcdef";
    input.resume = Some(session.into());
    let mut sessions = HashMap::from([(
        session.into(),
        SdkSession {
            task_id: 7,
            ..SdkSession::default()
        },
    )]);
    let mut usage = SdkTurnUsage::begin(&input, &mut sessions).unwrap();
    let frame = sdk_usage_fixture(submission, "ModelPipeline", 1007, None);
    let unknown = sdk_worker_report(&mut usage, &input, submission, &frame);
    assert!(
        unknown.baseline.is_none(),
        "external history is not an observed baseline"
    );
    usage.finish(sessions.get_mut(session).unwrap(), Outcome::Completed);

    let mut wrong_owner = input.clone();
    wrong_owner.task.id = 8;
    assert!(SdkTurnUsage::begin(&wrong_owner, &mut sessions).is_err());
    let mut usage = SdkTurnUsage::begin(&input, &mut sessions).unwrap();
    let mut changed_model = frame.clone();
    changed_model["report"]["model"] = json!("different-reported-model");
    assert!(
        sdk_worker_report(&mut usage, &input, submission, &changed_model)
            .baseline
            .is_none()
    );
    let mut failed = frame.clone();
    failed["report"]["complete"] = json!(false);
    let report = sdk_worker_report(&mut usage, &input, submission, &failed);
    assert_eq!(report.baseline.unwrap().input, Some(1007));
    assert!(!report.complete);
    usage.finish(sessions.get_mut(session).unwrap(), Outcome::Failed);
    let mut usage = SdkTurnUsage::begin(&input, &mut sessions).unwrap();
    assert!(
        sdk_worker_report(&mut usage, &input, submission, &frame)
            .baseline
            .is_none()
    );

    // A complete-looking usage observation cannot refresh a failed/unknown
    // turn. Nor may a sidecar invent its own private historical baseline.
    usage.finish(sessions.get_mut(session).unwrap(), Outcome::Unknown);
    let mut usage = SdkTurnUsage::begin(&input, &mut sessions).unwrap();
    let forged = sdk_usage_fixture(submission, "ModelPipeline", 1008, Some(1007));
    let event = usage_event(&input, 31, session, Some(submission), &forged).unwrap();
    assert!(usage.observe(event).is_err());
    assert!(sessions[session].model_totals.is_empty());
}

struct SdkTestDirectory(PathBuf);
impl SdkTestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "eevee-sdk-scope-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for SdkTestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn sdk_frozen_scope_captures_create_replace_unicode_and_every_parent() {
    let dir = SdkTestDirectory::new();
    let root = fs::canonicalize(dir.path()).unwrap();
    let data = SdkTestDirectory::new();
    fs::write(root.join("dirty.txt"), "user working bytes\n").unwrap();
    fs::create_dir(root.join("existing")).unwrap();
    let mut input = crate::providers::tests::claimed(Agent::Claude, Operation::Implement, false);
    input.task.project = root.to_string_lossy().into();
    input.edit_paths = vec![
        "dirty.txt".into(),
        "existing/새 폴더/empty.txt".into(),
        "nested/child/새 파일.txt".into(),
    ];
    let scope = capture_write_scope(data.path(), &input, &[]).unwrap();
    assert_eq!(scope.files[0].kind, SdkWriteKind::Replace);
    assert_eq!(
        scope.files[0].working_content.as_deref(),
        Some("user working bytes\n")
    );
    assert_eq!(scope.files[1].kind, SdkWriteKind::Create);
    assert!(scope.files[1].identity.is_none());
    assert!(scope.files[1].ancestors[0].identity.is_some());
    assert!(scope.files[1].ancestors[1].identity.is_none());
    assert_eq!(
        scope.files[2]
            .ancestors
            .iter()
            .map(|p| p.path.as_str())
            .collect::<Vec<_>>(),
        ["nested", "nested/child"]
    );
    assert!(
        scope.files[2]
            .ancestors
            .iter()
            .all(|p| p.identity.is_none())
    );
    let wire = serde_json::to_value(&scope).unwrap();
    assert_eq!(
        serde_json::from_value::<SdkWriteScope>(wire).unwrap(),
        scope
    );
    fs::create_dir(root.join("nested")).unwrap();
    assert_ne!(
        capture_write_scope(data.path(), &input, &[]).unwrap(),
        scope
    );
    fs::write(root.join("existing/appeared.txt"), "").unwrap();
    input.edit_paths = vec!["existing/appeared.txt".into()];
    assert_eq!(
        capture_write_scope(data.path(), &input, &[]).unwrap().files[0].kind,
        SdkWriteKind::Replace
    );
}

#[test]
fn sdk_scope_refuses_protected_invalid_directory_and_hardlinked_targets() {
    let dir = SdkTestDirectory::new();
    let root = fs::canonicalize(dir.path()).unwrap();
    let data = SdkTestDirectory::new();
    let mut input = crate::providers::tests::claimed(Agent::Claude, Operation::Implement, false);
    input.task.project = root.to_string_lossy().into();
    fs::write(root.join("file"), "bytes").unwrap();
    fs::hard_link(root.join("file"), root.join("alias")).unwrap();
    fs::create_dir(root.join("directory")).unwrap();
    for path in [
        ".git/new",
        ".DS_Store",
        ".ev-install/new",
        ".env",
        "../outside",
        "x:y",
        "nested//file",
        "CON.txt",
        "directory",
        "alias",
    ] {
        input.edit_paths = vec![path.into()];
        assert!(
            capture_write_scope(data.path(), &input, &[]).is_err(),
            "{path}"
        );
    }
    input.edit_paths = vec!["excluded/new".into()];
    assert!(capture_write_scope(data.path(), &input, &["excluded".into()]).is_err());
}

#[test]
fn sdk_scope_uses_shared_native_name_admission_without_reducing_its_path_cap() {
    let dir = SdkTestDirectory::new();
    let root = fs::canonicalize(dir.path()).unwrap();
    let data = SdkTestDirectory::new();
    let mut input = crate::providers::tests::claimed(Agent::Claude, Operation::Implement, false);
    input.task.project = root.to_string_lossy().into();
    for paths in [
        vec!["new.txt", "NEW.txt"],
        vec!["caf\u{e9}.txt", "cafe\u{301}.txt"],
        vec!["nested/a.txt", "NESTED/b.txt"],
        vec!["duplicate.txt", "duplicate.txt"],
        vec!["parent", "parent/child.txt"],
    ] {
        input.edit_paths = paths.iter().map(|path| PathBuf::from(*path)).collect();
        let admission = crate::providers::writes::verify_path_names(&root, &input.edit_paths);
        let captured = capture_write_scope(data.path(), &input, &[]);
        assert_eq!(
            captured.is_ok(),
            admission.is_ok(),
            "SDK capture must follow native filesystem name admission: {paths:?}"
        );
    }
    input.edit_paths = (0..256)
        .map(|index| PathBuf::from(format!("selected-{index}.txt")))
        .collect();
    assert_eq!(
        capture_write_scope(data.path(), &input, &[])
            .unwrap()
            .files
            .len(),
        256
    );
    input.edit_paths.push("one-too-many.txt".into());
    assert!(capture_write_scope(data.path(), &input, &[]).is_err());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
}

#[test]
fn sdk_creation_permission_exposes_complete_typed_scope_and_exact_answer() {
    let allowed =
        json!({"behavior":"allow","updatedInput":{"file_path":"새 폴더/a.txt","content":""}});
    let frame = json!({"kind":"approval","input":{
        "tool":"Write","input":allowed["updatedInput"],"scope":{"kind":"Create","path":"새 폴더/a.txt","directories":["새 폴더"]},
        "options":[{"label":"Deny","answer":{"behavior":"deny","message":"User declined or request cancelled"}},{"label":"Allow once","answer":allowed}]
    }});
    let kind = prompt_kind(&frame).unwrap();
    let PromptKind::Permission { description, .. } = &kind else {
        panic!("permission");
    };
    assert_eq!(
        description.lines().take(3).collect::<Vec<_>>(),
        [
            "Kind: Create",
            "Path: 새 폴더/a.txt",
            "Missing parents: 새 폴더"
        ]
    );
    assert!(provider_answer(&kind, allowed.clone()).is_ok());
    let mut changed = allowed;
    changed["updatedInput"]["content"] = json!("not approved");
    assert!(provider_answer(&kind, changed).is_err());
    let mut missing = frame;
    missing["input"]["scope"] = Value::Null;
    assert!(prompt_kind(&missing).is_err());
}
