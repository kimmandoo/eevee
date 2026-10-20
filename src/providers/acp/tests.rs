use super::*;

#[test]
fn message_snapshots_release_words_without_exposing_split_secrets() {
    assert_eq!(safe_stream("First ordinary ", false), "First ordinary ");
    assert_eq!(
        safe_stream("First ordinary words 日本語 tail", false),
        "First ordinary words 日本語 "
    );
    for text in [
        "前 ghp_secret123456 後 ",
        "前 api_key=\"opaque value\" 後 ",
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

#[test]
fn already_current_values_still_require_explicit_setter_acknowledgment() {
    let state = Models::parse(&opencode_options()).unwrap();
    let selection = ModelSelection {
        model: Some("anthropic/claude/sonnet-4".into()),
        effort: Some("default".into()),
    };
    assert!(
        state
            .require_acknowledged(&selection, false, false)
            .is_err()
    );
    assert!(state.require_acknowledged(&selection, true, false).is_err());
    state.require_acknowledged(&selection, true, true).unwrap();
    assert!(
        state
            .next_change(&selection, false, false)
            .unwrap()
            .is_some()
    );
    assert!(
        state
            .next_change(&selection, true, false)
            .unwrap()
            .is_some()
    );
    assert!(state.next_change(&selection, true, true).unwrap().is_none());
}

// Exact expected output from OpenCode 1.18.0's primary config-option fixture:
// https://github.com/anomalyco/opencode/blob/v1.18.0/packages/opencode/test/acp/config-option.test.ts
fn opencode_options() -> Value {
    json!({"configOptions":[
        {"id":"model","name":"Model","category":"model","type":"select",
         "currentValue":"anthropic/claude/sonnet-4","options":[
            {"value":"anthropic/claude-haiku","name":"Anthropic/Claude Haiku"},
            {"value":"anthropic/claude/sonnet-4","name":"Anthropic/Claude Sonnet 4"},
            {"value":"openai/gpt-5","name":"OpenAI/GPT-5"}]},
        {"id":"effort","name":"Effort","description":"Available effort levels for this model",
         "category":"thought_level","type":"select","currentValue":"default","options":[
            {"value":"low","name":"Low"},{"value":"default","name":"Default"},{"value":"high","name":"High"}]}
    ]})
}

#[test]
fn real_opencode_selectors_preserve_exact_ids_and_current_model_reasoning_scope() {
    let state = Models::parse(&opencode_options()).unwrap();
    let catalogue = state.catalogue(Agent::OpenCode).unwrap();
    assert_eq!(
        catalogue.current.as_deref(),
        Some("anthropic/claude/sonnet-4")
    );
    assert_eq!(
        catalogue
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        [
            "anthropic/claude-haiku",
            "anthropic/claude/sonnet-4",
            "openai/gpt-5"
        ]
    );
    assert_eq!(catalogue.models[1].efforts, ["low", "default", "high"]);
    assert_eq!(
        catalogue.models[1].default_effort.as_deref(),
        Some("default")
    );
    assert!(catalogue.models[0].efforts.is_empty());
    assert!(catalogue.models[2].efforts.is_empty());
    let selection = ModelSelection {
        model: Some("anthropic/claude/sonnet-4".into()),
        effort: Some("high".into()),
    };
    let model = state
        .next_change(&selection, false, false)
        .unwrap()
        .unwrap();
    assert_eq!(
        model.request("owned-session"),
        (
            "session/set_config_option",
            json!({"sessionId":"owned-session","configId":"model","value":"anthropic/claude/sonnet-4"})
        )
    );
}

#[test]
fn official_grouped_select_fixture_is_flattened_without_guessing_or_duplicate_values() {
    // Verbatim group example from the ACP v1 primary contract:
    // https://agentclientprotocol.com/protocol/v1/session-config-options#grouped-select-options
    let group = json!({"group":"recommended","name":"Recommended","options":[
        {"value":"model-1","name":"Model 1","description":"Fast for everyday tasks"}
    ]});
    let mut fixture = json!({"configOptions":[{
        "id":"provider-selector","name":"Model","category":"model","type":"select",
        "currentValue":"model-1","options":[group.clone()]
    }]});
    let catalogue = Models::parse(&fixture)
        .unwrap()
        .catalogue(Agent::Gemini)
        .unwrap();
    assert_eq!(catalogue.models.len(), 1);
    assert_eq!(catalogue.models[0].id, "model-1");
    assert_eq!(catalogue.models[0].description, "Fast for everyday tasks");
    fixture["configOptions"][0]["options"]
        .as_array_mut()
        .unwrap()
        .push(json!({"group":"other","name":"Other","options":group["options"]}));
    assert!(Models::parse(&fixture).is_err());
    fixture["configOptions"][0]["options"] = json!([
        group, {"value":"model-2","name":"Model 2"}
    ]);
    assert!(Models::parse(&fixture).is_err());
    fixture["configOptions"][0]["options"] = json!([{
        "group":"outer","name":"Outer","options":[{"group":"nested","name":"Nested","options":[]}]
    }]);
    assert!(Models::parse(&fixture).is_err());
}

#[test]
fn empty_devin_selector_does_not_invent_model_options_or_reasoning_controls() {
    // Verbatim sanitized selector from real isolated Devin 3000.11.3 session/new.
    let fixture = json!({"configOptions":[{"id":"model","name":"Model","description":"AI model to use","category":"model","type":"select","currentValue":"swe-1-6-fast","options":[]}]});
    let state = Models::parse(&fixture).unwrap();
    let catalogue = state.catalogue(Agent::Devin).unwrap();
    assert!(catalogue.models.is_empty());
    assert_eq!(catalogue.current.as_deref(), Some("swe-1-6-fast"));
    assert!(catalogue.effort_note.is_none());
    assert!(
        state
            .next_change(&ModelSelection::default(), false, false)
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .next_change(
                &ModelSelection {
                    model: None,
                    effort: Some("high".into()),
                },
                false,
                false
            )
            .is_err()
    );
}

#[test]
fn requested_values_need_exact_acknowledgment_and_dependent_reasoning_revalidation() {
    let mut state = Models::parse(&opencode_options()).unwrap();
    let selection = ModelSelection {
        model: Some("openai/gpt-5".into()),
        effort: Some("minimal".into()),
    };
    // Reasoning for the new model is not invented from the old model's list.
    let change = state
        .next_change(&selection, false, false)
        .unwrap()
        .unwrap();
    assert_eq!(
        change,
        SelectionChange::Config {
            id: "model".into(),
            value: "openai/gpt-5".into(),
            thought: false
        }
    );
    assert!(state.acknowledge(&json!({}), &change).is_err());
    assert!(state.acknowledge(&opencode_options(), &change).is_err());
    let mut acknowledged = opencode_options();
    acknowledged["configOptions"][0]["currentValue"] = json!("openai/gpt-5");
    acknowledged["configOptions"][1]["currentValue"] = json!("low");
    acknowledged["configOptions"][1]["options"] = json!([
        {"value":"minimal","name":"Minimal"},{"value":"low","name":"Low"}
    ]);
    state.acknowledge(&acknowledged, &change).unwrap();
    let reasoning = state.next_change(&selection, true, false).unwrap().unwrap();
    assert_eq!(
        reasoning.request("owned-session"),
        (
            "session/set_config_option",
            json!({"sessionId":"owned-session","configId":"effort","value":"minimal"})
        )
    );
    assert!(state.acknowledge(&acknowledged, &reasoning).is_err());
    acknowledged["configOptions"][1]["currentValue"] = json!("minimal");
    state.acknowledge(&acknowledged, &reasoning).unwrap();
    assert!(state.next_change(&selection, true, true).unwrap().is_none());
    let mut updated = acknowledged.clone();
    updated["configOptions"][1]["currentValue"] = json!("low");
    assert!(
        state
            .updated(&updated)
            .unwrap()
            .validate_selected(&selection, true, true)
            .is_err()
    );
    updated = acknowledged;
    updated["configOptions"].as_array_mut().unwrap().pop();
    assert!(
        state
            .updated(&updated)
            .unwrap()
            .validate_selected(&selection, true, true)
            .is_err()
    );
}

#[test]
fn custom_model_selection_requires_exact_backend_ack_not_catalogue_membership() {
    let selection = ModelSelection {
        model: Some("custom/new-model".into()),
        effort: None,
    };
    for fixture in [
        opencode_options(),
        json!({"models":{"currentModelId":"old","availableModels":[]}}),
    ] {
        let mut state = Models::parse(&fixture).unwrap();
        let change = state
            .next_change(&selection, false, false)
            .unwrap()
            .unwrap();
        assert!(
            state
                .require_acknowledged(&selection, false, false)
                .is_err()
        );
        assert!(state.acknowledge(&Value::Null, &change).is_err());
        let mut acknowledgment = fixture.clone();
        if acknowledgment.get("configOptions").is_some() {
            acknowledgment["configOptions"][0]["currentValue"] = json!("custom/new-model");
        } else {
            acknowledgment["models"]["currentModelId"] = json!("custom/new-model");
        }
        state.acknowledge(&acknowledgment, &change).unwrap();
        assert_eq!(
            state.catalogue(Agent::OpenCode).unwrap().current,
            selection.model
        );
        assert!(
            state
                .next_change(&selection, true, false)
                .unwrap()
                .is_none()
        );
        let substituted = ModelSelection {
            model: Some("substituted".into()),
            effort: None,
        };
        assert!(
            state
                .require_acknowledged(&substituted, true, false)
                .is_err()
        );
    }
}

#[test]
fn legacy_model_contract_requires_success_and_never_invents_reasoning() {
    // Released primary legacy schema: @agentclientprotocol/sdk 0.14.1
    // https://unpkg.com/@agentclientprotocol/sdk@0.14.1/dist/schema/types.gen.d.ts
    let fixture = json!({"models":{"currentModelId":"model-1","availableModels":[
        {"modelId":"model-1","name":"Model 1"},{"modelId":"model-2","name":"Model 2"}
    ]}});
    let mut state = Models::parse(&fixture).unwrap();
    let selection = ModelSelection {
        model: Some("model-2".into()),
        effort: None,
    };
    let change = state
        .next_change(&selection, false, false)
        .unwrap()
        .unwrap();
    assert_eq!(
        change.request("owned-session"),
        (
            "session/set_model",
            json!({"sessionId":"owned-session","modelId":"model-2"})
        )
    );
    assert!(state.next_change(&selection, true, false).is_err());
    assert!(state.acknowledge(&Value::Null, &change).is_err());
    assert!(state.acknowledge(&fixture, &change).is_err());
    state.acknowledge(&json!({}), &change).unwrap();
    assert!(
        state
            .next_change(&selection, true, false)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        state.catalogue(Agent::Gemini).unwrap().current.as_deref(),
        Some("model-2")
    );
    assert!(
        state
            .next_change(
                &ModelSelection {
                    model: None,
                    effort: Some("high".into())
                },
                false,
                false
            )
            .is_err()
    );
    let mut preferred = opencode_options();
    preferred["models"] = fixture["models"].clone();
    let state = Models::parse(&preferred).unwrap();
    assert_eq!(state.catalogue(Agent::OpenCode).unwrap().models.len(), 3);
}

#[test]
fn selector_names_unknown_categories_and_types_do_not_create_reasoning_capability() {
    for category in [Value::Null, json!("_reasoning"), json!("model_config")] {
        let mut fixture = opencode_options();
        fixture["configOptions"][1]["category"] = category;
        let state = Models::parse(&fixture).unwrap();
        assert!(
            state
                .catalogue(Agent::OpenCode)
                .unwrap()
                .models
                .iter()
                .all(|model| model.efforts.is_empty())
        );
        assert!(
            state
                .next_change(
                    &ModelSelection {
                        model: None,
                        effort: Some("high".into())
                    },
                    false,
                    false
                )
                .is_err()
        );
    }
    let mut fixture = opencode_options();
    fixture["configOptions"][1]["type"] = json!("future-control");
    assert!(Models::parse(&fixture).unwrap().thought.is_none());
    fixture["configOptions"][0]["category"] = Value::Null;
    assert!(
        Models::parse(&fixture)
            .unwrap()
            .catalogue(Agent::OpenCode)
            .unwrap()
            .models
            .is_empty()
    );
    assert!(
        Models::parse(&json!({"configOptions":null,"models":null}))
            .unwrap()
            .active_model()
            .is_none()
    );
}

#[test]
fn official_selector_bounds_reject_malformed_duplicate_and_unsafe_metadata() {
    let base = opencode_options();
    let mut duplicate = base.clone();
    duplicate["configOptions"]
        .as_array_mut()
        .unwrap()
        .push(base["configOptions"][0].clone());
    assert!(Models::parse(&duplicate).is_err());
    for key in ["value", "name", "description"] {
        let mut unsafe_value = base.clone();
        unsafe_value["configOptions"][0]["options"][0][key] = json!("hidden\u{202e}text");
        assert!(Models::parse(&unsafe_value).is_err());
        unsafe_value["configOptions"][0]["options"][0][key] = json!("line\nbreak");
        assert!(Models::parse(&unsafe_value).is_err());
    }
    let mut oversized = base.clone();
    oversized["configOptions"][0]["options"] = json!(
        (0..4097)
            .map(|id| json!({"value":id.to_string(),"name":format!("Model {id}")}))
            .collect::<Vec<_>>()
    );
    assert!(Models::parse(&oversized).is_err());
    oversized = base.clone();
    oversized["configOptions"][1]["options"] = json!(
        (0..33)
            .map(|id| json!({"value":id.to_string(),"name":format!("Effort {id}")}))
            .collect::<Vec<_>>()
    );
    assert!(Models::parse(&oversized).is_err());
    oversized = base.clone();
    oversized["configOptions"][0]["currentValue"] = json!("x".repeat(257));
    assert!(Models::parse(&oversized).is_err());
    let mut malformed = base;
    malformed["configOptions"][0]["options"][0]["name"] = Value::Null;
    assert!(Models::parse(&malformed).is_err());
}
#[test]
fn managed_review_is_refused_before_loading_or_dispatching_an_acp_session() {
    let workspace = Workspace {
        project: PathBuf::from("unopened-project"),
        data_dir: PathBuf::from("unopened-storage"),
        exclusions: Vec::new(),
    };
    for agent in [Agent::OpenCode, Agent::Gemini, Agent::Devin] {
        let input = Input {
            task: crate::store::Snapshot::empty("unopened-project".into()),
            work: None,
            excerpts: Vec::new(),
            report: None,
            resume: None,
            agent,
            operation: Operation::Review,
            edit_paths: Vec::new(),
            handoff: None,
            inspect_sessions: false,
            inspect_models: false,
            selection: ModelSelection::default(),
            conversation: None,
            credential: None,
        };
        assert!(
            payload(&workspace, &input)
                .unwrap_err()
                .contains("no-source-write")
        );
    }
}

#[test]
fn model_metadata_needs_no_saved_task_or_managed_role_and_retains_identity_checks() {
    let workspace = Workspace {
        project: PathBuf::from("unopened-project"),
        data_dir: PathBuf::from("unopened-storage"),
        exclusions: Vec::new(),
    };
    for agent in [Agent::OpenCode, Agent::Gemini, Agent::Devin] {
        for operation in [Operation::Plan, Operation::Implement, Operation::Review] {
            let mut input = Input {
                task: crate::store::Snapshot::empty("unopened-project".into()),
                work: None,
                excerpts: Vec::new(),
                report: None,
                resume: None,
                agent,
                operation,
                edit_paths: vec!["never-read.txt".into()],
                handoff: None,
                inspect_sessions: false,
                inspect_models: true,
                selection: ModelSelection::default(),
                conversation: None,
                credential: None,
            };
            input.task.goal = "private-goal-canary".into();
            input.task.archived = true;
            let text = payload(&workspace, &input).unwrap();
            assert!(!text.contains("private-goal-canary"));
            assert!(!text.contains("never-read.txt"));
            let metadata: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(metadata["model_prompts_sent"], 0);
            assert_eq!(metadata["session/new"]["mcpServers"], json!([]));
            assert_eq!(metadata["retain_resume_grant"], false);
            input.resume = Some("unowned-session".into());
            assert!(payload(&workspace, &input).is_err());
            input.resume = None;
            input.task.project = "different-project".into();
            assert!(payload(&workspace, &input).is_err());
            input.task.project = "unopened-project".into();
            input.agent = Agent::Codex;
            assert!(payload(&workspace, &input).is_err());
        }
    }
}

#[test]
fn cancellation_requests_do_not_override_authoritative_stop_reason() {
    assert_eq!(prompt_outcome("end_turn"), Outcome::Completed);
    assert_eq!(prompt_outcome("cancelled"), Outcome::Cancelled);
    assert_eq!(prompt_outcome("max_tokens"), Outcome::Failed);
    assert_eq!(prompt_outcome("unknown"), Outcome::Unknown);
}
#[test]
fn session_list_does_not_expose_foreign_project_metadata_or_accept_malformed_results() {
    let project = std::path::Path::new("/reviewed-project");
    let report = session_list_report(&json!({}), project, &json!({"sessions":[
            {"sessionId":"owned-session","cwd":"/reviewed-project","title":"Approved workflow"},
            {"sessionId":"foreign-session","cwd":"/unreviewed-project","title":"Private foreign task"}
        ]})).unwrap();
    assert!(report.contains("owned-session"));
    assert!(report.contains("Approved workflow"));
    assert!(!report.contains("foreign-session"));
    assert!(!report.contains("Private foreign task"));
    assert!(session_list_report(&json!({}), project, &json!({})).is_err());
    assert!(
        session_list_report(
            &json!({}),
            project,
            &json!({"sessions":[{"cwd":"/reviewed-project","title":"Missing identity"}]})
        )
        .is_err()
    );
    assert!(
        session_list_report(
            &json!({}),
            project,
            &json!({"sessions":vec![json!({});257]})
        )
        .is_err()
    );
}
#[test]
fn negotiated_capabilities_are_not_profile_assumptions() {
    let open=negotiated(&json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"list":{}},"future":true},"futureField":123})).unwrap();
    let gemini =
        negotiated(&json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}})).unwrap();
    assert!(supports_method(&open, "session/list"));
    assert!(!supports_method(&gemini, "session/list"));
    assert!(supports_method(&gemini, "session/load"));
    assert!(!supports_method(&json!({}), "session/load"));
    assert!(negotiated(&json!({"protocolVersion":2})).is_err());
    assert!(!supports_method(&open, "session/elicitation"));
}

#[test]
fn current_gemini_uses_stable_acp_flag_without_an_experimental_fallback() {
    assert_eq!(profile(Agent::Gemini).unwrap(), (3, vec!["--acp".into()]));
    assert_eq!(
        profile(Agent::OpenCode).unwrap(),
        (2, vec!["acp".into(), "--pure".into()])
    );
    assert!(profile(Agent::Codex).is_err());
}

#[test]
fn newer_peer_metadata_cannot_override_protocol_or_missing_capabilities() {
    let current = negotiated(&json!({
        "protocolVersion": 1,
        "agentInfo": {"name": "gemini", "version": "0.64.0"},
        "agentCapabilities": {"loadSession": true}
    }))
    .unwrap();
    assert!(supports_method(&current, "session/load"));
    assert!(!supports_method(&current, "session/list"));
    assert!(!supports_method(&current, "session/elicitation"));
    for invalid in [
        json!({"protocolVersion": 0, "agentInfo": {"version": "0.64.0"}}),
        json!({"protocolVersion": 2, "agentCapabilities": {"loadSession": true}}),
        json!({"protocolVersion": 1, "agentCapabilities": []}),
        json!({"agentInfo": {"version": "999.0.0"}}),
    ] {
        assert!(negotiated(&invalid).is_err());
    }
}
#[test]
fn persistent_options_are_filtered_exact_ids_survive() {
    let params = json!({"options":[{"optionId":"forever","name":"Always","kind":"allow_always"},{"optionId":"no-future","name":"Never","kind":"reject_always"},{"optionId":"deny exact","name":"No","kind":"reject_once"},{"optionId":"yes exact","name":"Yes","kind":"allow_once"}]});
    let options = permission_options(&params).unwrap();
    assert_eq!(options.len(), 2);
    assert_eq!(
        options[0].answer,
        json!({"outcome":{"outcome":"selected","optionId":"deny exact"}})
    );
    assert_eq!(
        options[1].answer,
        json!({"outcome":{"outcome":"selected","optionId":"yes exact"}})
    );
    let fallback = permission_options(
        &json!({"options":[{"optionId":"always","name":"Always","kind":"allow_always"}]}),
    )
    .unwrap();
    assert_eq!(fallback.len(), 1);
    assert_eq!(
        fallback[0].answer,
        json!({"outcome":{"outcome":"cancelled"}})
    );
}
#[test]
fn cancellation_invalidates_pending_and_late_grants() {
    let options = permission_options(
        &json!({"options":[{"optionId":"one","name":"Once","kind":"allow_once"}]}),
    )
    .unwrap();
    let mut ledger = Permissions::default();
    ledger.insert(&json!(7), &options).unwrap();
    assert_eq!(ledger.invalidate(), vec![json!(7)]);
    assert!(!ledger.take_answer(&json!(7), &options[1].answer));
    assert!(ledger.insert(&json!(7), &options).is_err());
    ledger.insert(&json!(8), &options).unwrap();
    assert!(!ledger.take_answer(
        &json!(8),
        &json!({"outcome":{"outcome":"selected","optionId":"always"}})
    ));
    assert!(ledger.take_answer(&json!(8), &options[1].answer));
    assert!(!ledger.take_answer(&json!(8), &options[1].answer));
}
#[test]
fn malformed_stdout_is_not_completion_and_optional_fields_survive() {
    let mut framer = Framer::default();
    assert!(framer.push(b"{\"jsonrpc\":\"2.0\",").unwrap().is_empty());
    assert_eq!(
        framer
            .push(b"\"id\":1,\"result\":{\"future\":1}}\n")
            .unwrap()
            .len(),
        1
    );
    assert!(framer.push(b"provider log\n").is_err());
    assert!(
        Framer::default()
            .push(&vec![b'x'; FRAME_LIMIT + 1])
            .is_err()
    );
}
#[test]
fn oversized_tool_final_is_bounded_and_labelled_not_a_run_abort() {
    let mut item = Item::new(ItemKind::Tool);
    item.tool_snapshot(&json!({"title":"Read","status":"in_progress"}))
        .unwrap();
    item.tool_snapshot(&json!({"status":"completed","rawOutput":"x".repeat(64 * 1024)}))
        .unwrap();
    assert!(item.complete);
    assert!(item.text.len() <= 32 * 1024);
    let fields: Value = serde_json::from_str(&item.text).unwrap();
    let output = &fields["rawOutput"];
    assert_eq!(output["truncated_by_eevee"], true);
    assert_eq!(output["total_bytes"], 64 * 1024);
    assert!(output["omitted_bytes"].as_u64().unwrap() > 0);
    assert_eq!(
        output["head"].as_str().unwrap().len()
            + output["tail"].as_str().unwrap().len()
            + output["omitted_bytes"].as_u64().unwrap() as usize,
        64 * 1024
    );
}
#[test]
fn tool_final_snapshots_do_not_duplicate_delta_or_completion() {
    let mut item = Item::new(ItemKind::Tool);
    item.tool_snapshot(&json!({"title":"Read","status":"in_progress","content":[{"type":"content","content":{"type":"text","text":"part"}}]})).unwrap();
    item.tool_snapshot(&json!({"status":"completed","content":[{"type":"content","content":{"type":"text","text":"part final"}}]})).unwrap();
    let final_text = item.text.clone();
    item.tool_snapshot(&json!({"status":"completed","content":[{"type":"content","content":{"type":"text","text":"part final"}}],"futureOptional":true})).unwrap();
    assert_eq!(item.text, final_text);
    assert!(item.complete);
    item.tool_snapshot(&json!({"locations":[]})).unwrap();
    assert!(item.complete);
    assert!(
        item.tool_snapshot(&json!({"status":"in_progress"}))
            .is_err()
    );
    assert!(item.complete);
}
