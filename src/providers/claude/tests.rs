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
        "bridge_revision":2, "query":true,
        "policy":{"tools":"closed-allowlist","permissions":"default-host-once","mcp":"disabled",
            "configuration":"isolated","authentication":"api-console-cloud-only"},
    });
    assert!(inspection_matches(&inspection, &versions));
    for (field, value) in [
        ("sdk_version", json!("1.5.1")),
        ("cli_version", json!("4.0.1")),
        ("bridge_revision", json!(3)),
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
