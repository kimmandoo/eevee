use super::*;
use crate::providers::Question;

fn task() -> store::Snapshot {
    let mut task = store::Snapshot::empty("project".into());
    task.id = 7;
    task.goal = "Plan a change".into();
    task
}

fn press(state: &mut State, code: KeyCode, large: bool) {
    state.key(
        KeyEvent::new(code, KeyModifiers::NONE),
        large,
        &task(),
        &[],
        None,
    );
}

fn reply_key(state: &mut State, code: KeyCode) {
    state.key(
        KeyEvent::new(code, KeyModifiers::ALT),
        true,
        &task(),
        &[],
        None,
    );
}

fn command(state: &mut State, name: &str, large: bool) {
    state.command(name, large, &task(), &[], None);
}

#[test]
fn ordinary_coding_requires_writer_ownership_without_overriding_explicit_readonly() {
    for agent in [
        Agent::Compatible,
        Agent::Codex,
        Agent::Claude,
        Agent::OpenCode,
        Agent::Gemini,
        Agent::Devin,
    ] {
        let mut state = State::new();
        state.reset(7);
        state.agent = agent;
        state.set_operation(Operation::Plan);
        state.prepare_ordinary_operation();
        command(&mut state, "new", true);
        let Some(Request::Preview(input)) = state.pending.take() else {
            panic!("missing preparation")
        };
        assert_eq!(input.operation, Operation::Plan);
        assert!(!crate::providers::writing_required(input.operation));
        let mut state = State::new();
        state.reset(7);
        state.agent = agent;
        state.prepare_ordinary_operation();
        command(&mut state, "new", true);
        let Some(Request::Preview(input)) = state.pending.take() else {
            panic!("missing implementation preparation")
        };
        assert!(crate::providers::writing_required(input.operation));
        let mut state = State::new();
        state.reset(7);
        state.agent = agent;
        state.set_operation(Operation::Review);
        state.prepare_ordinary_operation();
        assert_eq!(state.operation, Operation::Review);
    }
    for agent in [Agent::OpenCode, Agent::Gemini, Agent::Devin] {
        let mut state = State::new();
        state.reset(7);
        state.agent = agent;
        state.prepare_ordinary_operation();
        command(&mut state, "new", true);
        let Some(Request::Preview(input)) = state.pending.take() else {
            panic!("missing external preparation")
        };
        assert_eq!(input.operation, Operation::Implement);
        assert!(crate::providers::writing_required(input.operation));
    }
}

#[test]
fn exact_new_paths_are_local_selection_and_only_implement_receives_scope() {
    for agent in [Agent::Compatible, Agent::Codex, Agent::Claude] {
        for operation in [Operation::Plan, Operation::Implement, Operation::Review] {
            let mut state = State::new();
            state.reset(7);
            state.agent = agent;
            command(&mut state, "paths", true);
            let answer = state.paths_editor.as_mut().unwrap();
            answer.text = r#"["missing/nested/새.rs","empty.rs"]"#.into();
            answer.cursor = answer.text.len();
            press(&mut state, KeyCode::Enter, false);
            assert!(state.edit_paths.is_empty() && state.paths_editor.is_some());
            press(&mut state, KeyCode::Enter, true);
            let paths = vec![
                PathBuf::from("missing/nested/새.rs"),
                PathBuf::from("empty.rs"),
            ];
            assert_eq!(state.edit_paths, paths);
            assert!(state.pending.is_none() && state.work.is_none());
            assert!(state.preview.is_none() && !state.active && state.pending_journal.is_empty());
            state.set_operation(operation);
            command(&mut state, "new", true);
            let Some(Request::Preview(input)) = state.pending.take() else {
                panic!("local preparation must produce an input");
            };
            assert_eq!(
                input.edit_paths,
                if operation == Operation::Implement {
                    paths
                } else {
                    vec![]
                }
            );
            assert_eq!(input.operation, operation);
        }
    }
}

#[test]
fn local_effect_uncertainty_is_correlated_and_survives_terminal_route_and_reset() {
    for outcome in [Outcome::Completed, Outcome::Cancelled, Outcome::Unknown] {
        let mut state = active();
        state.work = Some(store::WorkClaim {
            task_id: 7,
            owner: store::Attribution::of(&store::Partner::default()),
            request_id: "owned-local-write".into(),
            writing: true,
        });
        state.work_acquired = true;
        state.event(journal_event(11, "local-write-outcome"));
        for (generation, task_id) in [(10, 7), (11, 8)] {
            state.event(Event::LocalEffectsUnknown {
                generation,
                task_id,
                message: "foreign uncertainty".into(),
            });
            assert!(!state.unknown_effects());
        }
        state.event(Event::LocalEffectsUnknown {
            generation: 11,
            task_id: 7,
            message: "Partial local write; effects unknown".into(),
        });
        assert!(state.unknown_effects() && state.outcome_unknown());
        state.event(Event::Finished {
            generation: 11,
            task_id: 7,
            outcome,
            message: "transport settled".into(),
        });
        let store::JournalOperation::Put(record) =
            &state.pending_journal.front().unwrap().operation
        else {
            panic!("the actual outcome must remain queued");
        };
        let saved = (**record).clone();
        state.journal_started();
        state.journal_event(7, Ok(store::JournalResponse::Saved(saved)));
        assert!(state.pending_journal.is_empty() && state.journal_inflight.is_empty());
        state.settle_work();
        let release = state
            .work_pending
            .take()
            .expect("known local transport releases the durable reservation");
        state.work_event(release, Ok(vec![]));
        assert!(state.work.is_none() && state.unknown_effects());
        state.switch_to = Some(Agent::Compatible);
        state.complete_switch();
        state.invalidate_authentication();
        state.reset(8);
        assert_eq!(state.task_id(), 7);
        assert!(state.unknown_effects() && state.outcome_unknown() && state.owns_work());
        assert!(state.pending.is_none());
        state.prepare(false, false, false, &task(), &[], None);
        assert!(
            state.pending.is_none(),
            "a route change cannot erase local uncertainty"
        );
    }
}

#[test]
fn native_model_acknowledgment_does_not_create_a_resumable_session() {
    let mut state = State::new();
    state.reset(7);
    state.agent = Agent::Compatible;
    state.active = true;
    state.generation = Some(11);
    state.event(Event::ModelObserved {
        generation: 11,
        task_id: 7,
        model: "confirmed-native".into(),
    });
    assert_eq!(state.observed_model.as_deref(), Some("confirmed-native"));
    assert!(state.thread_id.is_none());
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Completed,
        message: "done".into(),
    });
    command(&mut state, "new", true);
    let Some(Request::Preview(input)) = state.pending.take() else {
        panic!("fresh follow-up must remain available");
    };
    assert!(input.resume.is_none());
    assert_eq!(input.agent, Agent::Compatible);
}

fn catalogue() -> ModelCatalogue {
    ModelCatalogue {
        models: vec![crate::providers::ModelOption {
            id: "advertised-model".into(),
            name: "Advertised model".into(),
            description: "From provider metadata".into(),
            efforts: vec!["high".into()],
            default_effort: Some("high".into()),
        }],
        current: Some("advertised-model".into()),
        source: "Official metadata".into(),
        effort_note: None,
    }
}

fn custom_entry(state: &mut State) {
    state.picker_selected = state.picker_len() - 2;
    press(state, KeyCode::Enter, true);
    assert!(state.custom_model.is_some());
}

#[test]
fn unicode_display_search_is_case_insensitive_without_changing_exact_id() {
    let mut state = State::new();
    state.reset(7);
    let mut models = catalogue();
    models.models[0].name = "ÉCLAIR 模型".into();
    state.catalogues.insert(Agent::Codex, models);
    command(&mut state, "model", true);
    state.paste("éclair");
    assert_eq!(state.picker_matches(), 1);
    press(&mut state, KeyCode::Enter, true);
    assert!(state.picker == Some(Picker::Effort));
    assert!(state.selection.model.is_none());
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.selection.model.as_deref(), Some("advertised-model"));
}

#[test]
fn model_search_ranks_exact_then_prefix_then_contiguous_in_provider_order() {
    let mut state = State::new();
    state.reset(7);
    let mut models = catalogue();
    models.models.clear();
    for (id, name) in [
        ("suffix-first", "Other Target"),
        ("prefix-first", "Target Alpha"),
        ("suffix-second", "Another Target"),
        ("vendor/TaRgEt", "Different name"),
        ("exact-display", "vendor/target"),
        ("prefix-second", "vendor/target Beta"),
        ("prefix-third", "vendor/target Gamma"),
        ("suffix-third", "Other vendor/target"),
    ] {
        let mut model = catalogue().models.remove(0);
        model.id = id.into();
        model.name = name.into();
        models.models.push(model);
    }
    state.catalogues.insert(Agent::Codex, models);
    state.selection.model = Some("prefix-second".into());
    command(&mut state, "model", true);
    state.paste("VENDOR/target");
    let ids: Vec<_> = (1..=state.picker_matches())
        .map(|index| state.picker_model(index).unwrap().id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "vendor/TaRgEt",
            "exact-display",
            "prefix-second",
            "prefix-third",
            "suffix-third",
        ]
    );
    assert_eq!(state.picker_current(), Some(3));
    assert_eq!(state.picker_selected, 1);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.selection.model.as_deref(), Some("prefix-second"));
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.selection.model.as_deref(), Some("vendor/TaRgEt"));
    assert!(state.pending.is_none() && !state.auto_models);
}

#[test]
fn unicode_model_search_uses_the_same_exact_and_prefix_ranking() {
    let mut state = State::new();
    state.reset(7);
    let mut models = catalogue();
    models.models.clear();
    for (id, name) in [
        ("suffix", "Un ÉCLAIR"),
        ("prefix", "ÉCLAIR 模型"),
        ("exact", "ÉCLAIR"),
    ] {
        let mut model = catalogue().models.remove(0);
        model.id = id.into();
        model.name = name.into();
        models.models.push(model);
    }
    state.catalogues.insert(Agent::Codex, models);
    command(&mut state, "model", true);
    state.paste("éclair");
    assert_eq!(
        (1..=state.picker_matches())
            .map(|index| state.picker_model(index).unwrap().id.as_str())
            .collect::<Vec<_>>(),
        ["exact", "prefix", "suffix"],
    );
}

#[test]
fn reasoning_back_restores_query_caret_and_candidate_before_atomic_reselection() {
    let mut state = State::new();
    state.reset(7);
    let mut models = catalogue();
    let mut second = models.models[0].clone();
    second.id = "advertised-second".into();
    models.models.push(second);
    state.catalogues.insert(Agent::Codex, models);
    state.selection = ModelSelection {
        model: Some("retained/custom".into()),
        effort: Some("retained-effort".into()),
    };
    let committed = state.selection.clone();
    command(&mut state, "model", true);
    state.paste("advertised");
    press(&mut state, KeyCode::Left, true);
    let query_cursor = state.picker_query.cursor;
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    press(&mut state, KeyCode::Down, true);
    assert!(state.can_back_to_models());
    assert_eq!(state.selection, committed);
    press(&mut state, KeyCode::Left, true);
    assert!(state.picker == Some(Picker::Model));
    assert!(state.pending_model.is_none() && !state.can_back_to_models());
    assert_eq!(state.picker_query.text, "advertised");
    assert_eq!(state.picker_query.cursor, query_cursor);
    assert_eq!(
        state.picker_model(state.picker_selected).unwrap().id,
        "advertised-second",
    );
    assert_eq!(state.picker_current(), None);
    press(&mut state, KeyCode::Up, true);
    press(&mut state, KeyCode::Enter, true);
    press(&mut state, KeyCode::Down, true);
    assert_eq!(state.selection, committed);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.selection.model.as_deref(), Some("advertised-model"));
    assert_eq!(state.selection.effort.as_deref(), Some("high"));
    assert!(state.picker.is_none() && state.pending_model.is_none());
    assert!(state.pending.is_none() && state.pending_journal.is_empty());
    assert!(!state.auto_models && !state.auto_picker && !state.active);
}

#[test]
fn reasoning_back_keeps_exact_candidate_identity_after_catalogue_reordering() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    command(&mut state, "model", true);
    state.paste("advertised");
    press(&mut state, KeyCode::Enter, true);
    let mut refreshed = catalogue();
    let mut first = refreshed.models[0].clone();
    first.id = "advertised-first".into();
    refreshed.models.insert(0, first);
    state.publish_catalogue(refreshed);
    press(&mut state, KeyCode::Left, true);
    assert_eq!(state.picker_query.text, "advertised");
    assert_eq!(
        state.picker_model(state.picker_selected).unwrap().id,
        "advertised-model",
    );
    assert_eq!(state.picker_selected, 2);
    press(&mut state, KeyCode::Up, true);
    press(&mut state, KeyCode::Enter, true);
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Esc, true);
    assert_eq!(state.selection, ModelSelection::default());
    assert!(state.picker.is_none() && state.pending_model.is_none());
    assert!(state.pending.is_none() && !state.auto_models);
}

#[test]
fn reasoning_back_restores_custom_action_without_committing() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    command(&mut state, "model", true);
    state.paste("advertised");
    custom_entry(&mut state);
    state.paste("advertised-model");
    press(&mut state, KeyCode::Enter, true);
    press(&mut state, KeyCode::Left, true);
    assert!(state.picker == Some(Picker::Model));
    assert_eq!(state.picker_query.text, "advertised");
    assert_eq!(
        state.picker_choice(state.picker_selected).unwrap().command,
        "model-custom",
    );
    assert!(state.custom_model.is_none());
    assert_eq!(state.selection, ModelSelection::default());
    assert!(state.pending.is_none());
}

#[test]
fn standalone_effort_and_direct_model_reasoning_have_no_model_back_level() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    command(&mut state, "effort", true);
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Left, true);
    assert!(state.picker == Some(Picker::Effort));
    assert_eq!(state.picker_selected, 1);
    assert!(!state.can_back_to_models());
    press(&mut state, KeyCode::Esc, true);
    assert!(state.select_model_id("advertised-model"));
    press(&mut state, KeyCode::Left, true);
    assert!(state.picker == Some(Picker::Effort));
    assert!(!state.can_back_to_models());
    press(&mut state, KeyCode::Esc, true);
    assert_eq!(state.selection, ModelSelection::default());
    assert!(state.pending.is_none() && !state.auto_models);
}

#[test]
fn direct_exact_model_and_effort_reuse_atomic_local_selection() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    assert!(state.select_model_id("advertised-model"));
    assert!(state.picker == Some(Picker::Effort));
    assert_eq!(state.selection, ModelSelection::default());
    assert!(!state.select_effort_value("HIGH"));
    assert_eq!(state.selection, ModelSelection::default());
    assert!(state.select_effort_value("high"));
    assert_eq!(state.selection.model.as_deref(), Some("advertised-model"));
    assert_eq!(state.selection.effort.as_deref(), Some("high"));
    assert!(state.select_effort_value("default"));
    assert!(state.selection.effort.is_none() && state.picker.is_none());
    state.auto_models = true;
    state.auto_picker = true;
    assert!(state.select_model_id("vendor/Exact.Model:2026"));
    assert_eq!(
        state.selection.model.as_deref(),
        Some("vendor/Exact.Model:2026")
    );
    assert!(state.picker.is_none() && state.pending.is_none());
    assert!(!state.auto_models && !state.auto_picker && state.catalogue_dismissed);
    assert!(state.select_model_id("default"));
    assert_eq!(state.selection.model.as_deref(), Some("default"));
    assert!(!state.select_effort_value("high"));
    assert!(state.pending.is_none() && !state.active);
}

#[test]
fn direct_invalid_values_preserve_selection_and_session() {
    let mut state = State::new();
    state.reset(7);
    state.selection = ModelSelection {
        model: Some("retained/custom".into()),
        effort: Some("retained-effort".into()),
    };
    state.thread_id = Some("owned-session".into());
    let committed = state.selection.clone();
    for id in ["", " model ", "model\n", "org/\u{202e}model"] {
        assert!(!state.select_model_id(id));
        assert_eq!(state.selection, committed);
        assert!(state.picker.is_none() && state.selection_change.is_none());
    }
    assert!(!state.select_model_id(&"m".repeat(257)));
    assert!(!state.select_effort_value("unsupported"));
    assert_eq!(state.selection, committed);
    assert_eq!(state.thread_id.as_deref(), Some("owned-session"));
    assert!(state.pending.is_none() && !state.auto_models);
}

#[test]
fn direct_unknown_model_keeps_existing_session_review_without_a_phantom_picker() {
    let mut state = State::new();
    state.reset(7);
    state.thread_id = Some("owned-session".into());
    assert!(state.select_model_id("vendor/Exact.Custom"));
    assert!(state.picker.is_none() && state.selection_change.is_some());
    assert_eq!(state.selection, ModelSelection::default());
    press(&mut state, KeyCode::Esc, true);
    assert!(state.picker.is_none() && state.selection_change.is_none());
    assert_eq!(state.thread_id.as_deref(), Some("owned-session"));
    assert!(state.select_model_id("vendor/Exact.Custom"));
    state.reviewed = true;
    press(&mut state, KeyCode::Tab, true);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(
        state.selection.model.as_deref(),
        Some("vendor/Exact.Custom")
    );
    assert!(state.thread_id.is_none() && state.pending.is_none());
    assert!(state.picker.is_none() && state.selection_change.is_none());
}

#[test]
fn reasoning_back_uses_neutral_custom_action_if_exact_candidate_disappeared() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    command(&mut state, "model", true);
    state.paste("advertised");
    press(&mut state, KeyCode::Enter, true);
    let mut refreshed = catalogue();
    refreshed.models.clear();
    state.publish_catalogue(refreshed);
    press(&mut state, KeyCode::Left, true);
    assert!(state.picker == Some(Picker::Model));
    assert_eq!(state.picker_query.text, "advertised");
    assert_eq!(
        state.picker_choice(state.picker_selected).unwrap().command,
        "model-custom",
    );
    assert_eq!(state.selection, ModelSelection::default());
    assert!(state.pending.is_none());
}

#[test]
fn direct_known_model_reviews_complete_reasoning_before_replacing_a_session() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    state.thread_id = Some("owned-session".into());
    assert!(state.select_model_id("advertised-model"));
    assert!(state.select_effort_value("high"));
    let change = state.selection_change.as_ref().unwrap();
    assert_eq!(change.model.as_deref(), Some("advertised-model"));
    assert_eq!(change.effort.as_deref(), Some("high"));
    assert_eq!(state.selection, ModelSelection::default());
    press(&mut state, KeyCode::Esc, true);
    assert!(state.picker == Some(Picker::Effort) && state.selection_change.is_none());
    assert!(!state.can_back_to_models());
    assert!(state.select_effort_value("high"));
    state.reviewed = true;
    press(&mut state, KeyCode::Tab, true);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.selection.model.as_deref(), Some("advertised-model"));
    assert_eq!(state.selection.effort.as_deref(), Some("high"));
    assert!(state.thread_id.is_none() && state.pending.is_none());
}

#[test]
fn advertised_default_effort_value_is_not_reinterpreted_as_reset() {
    let mut state = State::new();
    state.reset(7);
    let mut models = catalogue();
    models.models[0].efforts.push("default".into());
    state.catalogues.insert(Agent::Codex, models);
    assert!(state.select_model_id("advertised-model"));
    assert!(state.select_effort_value("default"));
    assert_eq!(state.selection.effort.as_deref(), Some("default"));
    assert!(state.pending.is_none() && !state.auto_models);
}

#[test]
fn model_reasoning_flow_is_atomic_and_default_enter_accepts() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    state.selection = ModelSelection {
        model: Some("retained/custom".into()),
        effort: Some("retained-effort".into()),
    };
    let committed = state.selection.clone();
    command(&mut state, "model", true);
    press(&mut state, KeyCode::Home, true);
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    assert!(state.picker == Some(Picker::Effort));
    assert_eq!(state.picker_selected, 0);
    assert_eq!(
        state.picker_current(),
        None,
        "candidate is not a committed current model"
    );
    press(&mut state, KeyCode::Down, true);
    assert_eq!(state.selection, committed);
    assert!(state.pending.is_none());
    press(&mut state, KeyCode::Esc, true);
    assert!(state.picker.is_none() && state.pending_model.is_none());
    assert_eq!(state.selection, committed);
    command(&mut state, "model", true);
    press(&mut state, KeyCode::Home, true);
    press(&mut state, KeyCode::Enter, true);
    assert!(state.picker == Some(Picker::Effort));
    assert!(state.picker_selection().model.is_none());
    assert_eq!(state.selection, committed);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.selection, ModelSelection::default());
    assert!(state.picker.is_none() && state.pending.is_none());
}

#[test]
fn accepting_current_model_retains_effort_and_session_without_another_picker() {
    for model in [None, Some("advertised-model")] {
        for effort in [None, Some("high")] {
            let mut state = State::new();
            state.reset(7);
            state.catalogues.insert(Agent::Codex, catalogue());
            state.selection = ModelSelection {
                model: model.map(str::to_owned),
                effort: effort.map(str::to_owned),
            };
            state.thread_id = Some("owned-session".into());
            let retained = state.selection.clone();
            command(&mut state, "model", true);
            press(&mut state, KeyCode::Enter, true);
            assert!(state.picker.is_none() && state.pending_model.is_none());
            assert!(state.selection_change.is_none() && state.pending.is_none());
            assert!(state.pending_journal.is_empty());
            assert_eq!(state.selection, retained);
            assert_eq!(state.thread_id.as_deref(), Some("owned-session"));
            command(&mut state, "effort", true);
            assert!(state.picker == Some(Picker::Effort));
            press(&mut state, KeyCode::Esc, true);
            assert_eq!(state.selection, retained);
        }
    }
}

#[test]
fn accepting_unchanged_model_cannot_reuse_an_armed_request_review() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    command(&mut state, "new", true);
    assert!(matches!(state.pending.take(), Some(Request::Preview(_))));
    state.event(Event::Preview(Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: true,
        generation: 11,
        task_id: 7,
        thread_id: None,
        trust: Vec::new(),
        payload: "{}".into(),
        blockers: Vec::new(),
    }));
    state.reviewed = true;
    state.affirmative = true;
    command(&mut state, "model", true);
    press(&mut state, KeyCode::Enter, true);
    assert!(state.picker.is_none() && state.preview.is_some());
    assert!(!state.affirmative && state.pending.is_none());
    state.reviewed = true;
    press(&mut state, KeyCode::Enter, true);
    assert!(!matches!(state.pending, Some(Request::Run { .. })));
}

#[test]
fn exact_custom_id_uses_advertised_reasoning_but_unknown_models_finish_locally() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    command(&mut state, "model", true);
    custom_entry(&mut state);
    state.paste("advertised-model");
    press(&mut state, KeyCode::Enter, true);
    assert!(state.picker == Some(Picker::Effort));
    assert!(state.selection.model.is_none());
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.selection.effort.as_deref(), Some("high"));
    command(&mut state, "model", true);
    custom_entry(&mut state);
    state.paste("org/Exact.Custom");
    press(&mut state, KeyCode::Enter, true);
    assert!(state.picker.is_none());
    assert_eq!(state.selection.model.as_deref(), Some("org/Exact.Custom"));
    assert!(state.selection.effort.is_none() && state.pending.is_none());
}

#[test]
fn cached_rows_remain_selectable_after_refresh_failure_and_error_is_optional() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    state.inspect_models = true;
    state.awaiting_preview = true;
    state.event(Event::Error {
        generation: None,
        message: "Offline".into(),
    });
    assert!(state.picker == Some(Picker::Model));
    assert!(!state.review_details);
    state.key(
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        true,
        &task(),
        &[],
        None,
    );
    assert!(state.review_details);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.selection, ModelSelection::default());
    press(&mut state, KeyCode::Esc, true);
    assert!(!state.review_details && state.picker.is_some());
    press(&mut state, KeyCode::Home, true);
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.selection.model.as_deref(), Some("advertised-model"));
    assert!(state.picker.is_none() && state.pending.is_none());
    assert_eq!(state.catalogue_error(), Some("Offline"));
}

#[test]
fn catalogue_publication_keeps_focused_exact_model_or_neutral_custom_action() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    command(&mut state, "model", true);
    press(&mut state, KeyCode::Down, true);
    let mut refreshed = catalogue();
    let mut first = refreshed.models[0].clone();
    first.id = "new-first-model".into();
    refreshed.models.insert(0, first);
    state.publish_catalogue(refreshed);
    assert_eq!(
        state.picker_model(state.picker_selected).unwrap().id,
        "advertised-model"
    );
    let mut removed = catalogue();
    removed.models.clear();
    state.publish_catalogue(removed);
    assert_eq!(
        state.picker_choice(state.picker_selected).unwrap().command,
        "model-custom"
    );
    assert_eq!(state.selection, ModelSelection::default());
    assert!(state.pending.is_none());
}

#[test]
fn authentication_change_clears_cache_and_blocks_late_old_account_metadata() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    state.active = true;
    state.inspect_models = true;
    state.generation = Some(11);
    state.invalidate_authentication();
    assert!(state.catalogues.is_empty());
    state.event(Event::Models {
        generation: 11,
        task_id: 7,
        catalogue: catalogue(),
    });
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Completed,
        message: "Old account complete".into(),
    });
    assert!(state.catalogues.is_empty());
    assert!(state.catalogue_error().is_some());
}

#[test]
fn worker_retention_covers_current_and_cached_provider_operation_sessions() {
    let mut state = active();
    assert!(!state.has_resumable_sessions());
    state
        .sessions
        .insert((Agent::Claude, Operation::Review), Session::default());
    assert!(!state.has_resumable_sessions());
    state.thread_id = Some("owned-plan-thread".into());
    assert!(state.has_resumable_sessions());
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Completed,
        message: "Ordinary successful turn".into(),
    });
    assert!(state.has_resumable_sessions());
    state.set_operation(Operation::Implement);
    assert!(state.thread_id.is_none());
    assert!(
        state.has_resumable_sessions(),
        "The cached Plan session still owns its worker."
    );
    state.set_operation(Operation::Plan);
    assert_eq!(state.thread_id.as_deref(), Some("owned-plan-thread"));
    state.switch_to = Some(Agent::Gemini);
    state.complete_switch();
    assert!(state.thread_id.is_none());
    assert!(
        state.has_resumable_sessions(),
        "Another provider's cached session still owns its worker."
    );
    state.invalidate_authentication();
    assert!(!state.has_resumable_sessions());
    state.switch_to = Some(Agent::Codex);
    state.complete_switch();
    assert!(state.thread_id.is_none());
    command(&mut state, "resume", true);
    assert!(
        state.pending.is_none(),
        "Revoked Resume must not silently become New."
    );
    assert!(state.notice.contains("Resume requires"));
}

#[test]
fn authentication_change_revokes_idle_sessions_even_while_claims_and_journals_settle() {
    let mut state = State::new();
    state.reset(7);
    let claim = store::WorkClaim {
        task_id: 7,
        owner: partner(1, "Bolt", 2),
        request_id: "settling-old-account".into(),
        writing: false,
    };
    state.work = Some(claim.clone());
    state.work_acquired = true;
    state.work_pending = Some(store::WorkRequest::Release(claim.clone()));
    state.pending_journal.push_back(store::JournalRequest {
        task_id: 7,
        operation: store::JournalOperation::List,
    });
    state.journal_inflight.push_back(store::JournalRequest {
        task_id: 8,
        operation: store::JournalOperation::Purge,
    });
    state.failed_journal.push_back(store::JournalRequest {
        task_id: 9,
        operation: store::JournalOperation::List,
    });
    state.thread_id = Some("idle-old-account".into());
    state.sessions.insert(
        (Agent::Claude, Operation::Implement),
        Session {
            thread_id: Some("cached-old-account".into()),
            ..Session::default()
        },
    );
    state.items.push(message());
    state.catalogues.insert(Agent::Codex, catalogue());
    state.set_devin_credential(Some(crate::providers::auth::test_credential("old-account")));
    state.reviewed = true;
    state.affirmative = true;
    assert!(state.busy());
    state.invalidate_authentication();
    assert!(state.thread_id.is_none() && state.sessions.is_empty() && state.items.is_empty());
    assert!(!state.has_resumable_sessions());
    assert!(state.catalogues.is_empty() && state.devin_credential.is_none());
    assert!(!state.reviewed && !state.affirmative && !state.transport_pending());
    assert_eq!(state.work.as_ref(), Some(&claim));
    assert!(state.work_acquired);
    assert_eq!(state.work_pending, Some(store::WorkRequest::Release(claim)));
    assert!(matches!(
        state.pending_journal.front(),
        Some(store::JournalRequest {
            task_id: 7,
            operation: store::JournalOperation::List
        })
    ));
    assert!(matches!(
        state.journal_inflight.front(),
        Some(store::JournalRequest {
            task_id: 8,
            operation: store::JournalOperation::Purge
        })
    ));
    assert!(matches!(
        state.failed_journal.front(),
        Some(store::JournalRequest {
            task_id: 9,
            operation: store::JournalOperation::List
        })
    ));
}

#[test]
fn parked_and_new_partner_states_only_inherit_current_future_credentials() {
    let mut front = State::new();
    front.reset(7);
    front.agent = Agent::Devin;
    front.catalogues.insert(Agent::Devin, catalogue());
    front.set_devin_credential(Some(crate::providers::auth::test_credential("account-one")));
    let mut parked = front.new_partner(partner(2, "Mizu", 1), 7);
    front.thread_id = Some("front-old-account".into());
    parked.thread_id = Some("parked-old-account".into());
    for state in [&mut front, &mut parked] {
        state.invalidate_authentication();
        assert!(state.devin_credential.is_none());
        assert!(state.catalogues.is_empty() && !state.has_resumable_sessions());
        state.set_devin_credential(Some(crate::providers::auth::test_credential("account-two")));
        assert!(state.pending.is_none());
    }
    let mut newcomer = parked.new_partner(partner(3, "Leaf", 6), 7);
    assert_eq!(
        newcomer.devin_credential.as_ref().unwrap().token(),
        "account-two"
    );
    assert!(newcomer.catalogues.is_empty() && !newcomer.has_resumable_sessions());
    command(&mut newcomer, "new", true);
    let Some(Request::Preview(input)) = newcomer.pending.take() else {
        panic!("A deliberate new request must use the replacement credential.");
    };
    assert_eq!(input.credential.as_ref().unwrap().token(), "account-two");
    for state in [&mut front, &mut parked, &mut newcomer] {
        state.set_devin_credential(None);
        assert!(state.devin_credential.is_none());
    }
    let fresh = front.new_partner(partner(4, "Snow", 7), 7);
    assert!(fresh.devin_credential.is_none() && fresh.pending.is_none());
    assert_eq!(input.credential.as_ref().unwrap().token(), "account-two");
}

#[test]
fn credential_changes_preserve_held_input_and_authentication_cancel_settles_exact_claim() {
    let owner = partner(1, "Bolt", 2);
    let mut state = State::new().new_partner(owner.clone(), 7);
    state.agent = Agent::Devin;
    state.thread_id = Some("old-account-thread".into());
    state.set_devin_credential(Some(crate::providers::auth::test_credential(
        "held-account",
    )));
    command(&mut state, "resume", true);
    let claim = store::WorkClaim {
        task_id: 7,
        owner,
        request_id: "held-old-account".into(),
        writing: false,
    };
    state.stage_work(claim.clone());
    let held_address = state.held_input.as_deref().unwrap() as *const crate::providers::Input;
    let task_bytes = serde_json::to_vec(&state.held_input.as_ref().unwrap().task).unwrap();
    for credential in [
        Some(crate::providers::auth::test_credential(
            "replacement-account",
        )),
        None,
    ] {
        state.set_devin_credential(credential);
        let held = state.held_input.as_deref().unwrap();
        assert_eq!(held as *const _, held_address);
        assert_eq!(held.credential.as_ref().unwrap().token(), "held-account");
        assert_eq!(held.resume.as_deref(), Some("old-account-thread"));
        assert_eq!(serde_json::to_vec(&held.task).unwrap(), task_bytes);
    }
    state.invalidate_authentication();
    let held = state.held_input.as_deref().unwrap();
    assert_eq!(held as *const _, held_address);
    assert_eq!(held.credential.as_ref().unwrap().token(), "held-account");
    assert_eq!(held.resume.as_deref(), Some("old-account-thread"));
    assert_eq!(serde_json::to_vec(&held.task).unwrap(), task_bytes);
    assert!(state.claim_cancelled && state.pending.is_none());
    assert!(!state.has_resumable_sessions());
    let acquire = state.work_pending.take().unwrap();
    assert!(
        matches!(&acquire, store::WorkRequest::Acquire { claim: owned, .. } if owned == &claim)
    );
    state.work_inflight = true;
    state.work_event(acquire, Ok(vec![claim.clone()]));
    assert!(state.held_input.is_none() && state.pending.is_none());
    assert!(!state.transport_pending());
    assert_eq!(state.work.as_ref(), Some(&claim));
    let release = state.work_pending.take().unwrap();
    assert_eq!(release, store::WorkRequest::Release(claim));
    state.work_inflight = true;
    state.work_event(release, Ok(Vec::new()));
    assert!(state.work.is_none() && !state.owns_work());
}

#[test]
fn authentication_change_cancels_stale_resume_previews_without_fresh_fallback() {
    for already_prepared in [false, true] {
        let mut state = State::new();
        state.reset(7);
        state.thread_id = Some("old-account-thread".into());
        command(&mut state, "resume", true);
        let Some(Request::Preview(input)) = state.pending.take() else {
            panic!("Owned Resume must prepare an explicit resume.");
        };
        assert_eq!(input.resume.as_deref(), Some("old-account-thread"));
        let preview = Preview {
            arrival: crate::providers::approval_arrival(),
            generation: 11,
            task_id: 7,
            thread_id: input.resume.clone(),
            destination: None,
            requires_connection_review: true,
            trust: Vec::new(),
            payload: "immutable old-account payload".into(),
            blockers: Vec::new(),
        };
        if already_prepared {
            state.event(Event::Preview(preview.clone()));
        }
        state.send_after_prepare = true;
        state.reviewed = true;
        state.affirmative = true;
        state.invalidate_authentication();
        assert!(!state.send_after_prepare && !state.reviewed && !state.affirmative);
        assert!(!state.has_resumable_sessions());
        if !already_prepared {
            assert!(state.cancel_when_ready && state.awaiting_preview);
            state.event(Event::Preview(preview));
        }
        assert!(state.preview.is_none() && state.cancelling);
        assert!(!state.dispatch_prepared());
        assert!(matches!(
            state.pending.take(),
            Some(Request::Cancel { generation: 11 })
        ));
        state.event(Event::Finished {
            generation: 11,
            task_id: 7,
            outcome: Outcome::Cancelled,
            message: "Stale preparation cancelled".into(),
        });
        assert!(!state.transport_pending() && !state.has_resumable_sessions());
        command(&mut state, "resume", true);
        assert!(
            state.pending.is_none(),
            "Revoked Resume must never silently launch New."
        );
        assert!(state.notice.contains("Resume requires"));
        assert_eq!(input.resume.as_deref(), Some("old-account-thread"));
    }
}

#[test]
fn active_account_change_preserves_submitted_bytes_and_revokes_every_terminal_session() {
    for outcome in [
        Outcome::Completed,
        Outcome::Failed,
        Outcome::Cancelled,
        Outcome::Unknown,
    ] {
        let mut state = State::new();
        state.reset(7);
        state.agent = Agent::Devin;
        state.thread_id = Some("submitted-old-account".into());
        state.set_devin_credential(Some(crate::providers::auth::test_credential(
            "submitted-account",
        )));
        command(&mut state, "resume", true);
        let Some(Request::Preview(input)) = state.pending.take() else {
            panic!("The submitted transport must own an immutable input.");
        };
        let task_bytes = serde_json::to_vec(&input.task).unwrap();
        state.committed_input = Some(Box::new(input.task.clone()));
        let committed_address = state.committed_input.as_deref().unwrap() as *const store::Snapshot;
        state.event(Event::Preview(Preview {
            arrival: crate::providers::approval_arrival(),
            generation: 11,
            task_id: 7,
            thread_id: input.resume.clone(),
            destination: None,
            requires_connection_review: true,
            trust: Vec::new(),
            payload: "submitted payload".into(),
            blockers: Vec::new(),
        }));
        assert!(!state.dispatch_prepared());
        state.reviewed = true;
        press(&mut state, KeyCode::Char('y'), true);
        assert!(matches!(
            state.pending.take(),
            Some(Request::Run { generation: 11 })
        ));
        state.sessions.insert(
            (Agent::Codex, Operation::Implement),
            Session {
                thread_id: Some("cached-old-account".into()),
                ..Session::default()
            },
        );
        state.invalidate_authentication();
        state.set_devin_credential(Some(crate::providers::auth::test_credential(
            "future-account",
        )));
        assert!(state.active && !state.cancelling && state.pending.is_none());
        assert_eq!(state.generation, Some(11));
        assert_eq!(state.thread_id.as_deref(), Some("submitted-old-account"));
        assert!(state.sessions.is_empty() && !state.has_resumable_sessions());
        assert_eq!(
            input.credential.as_ref().unwrap().token(),
            "submitted-account"
        );
        assert_eq!(input.resume.as_deref(), Some("submitted-old-account"));
        assert_eq!(serde_json::to_vec(&input.task).unwrap(), task_bytes);
        assert_eq!(
            state.committed_input.as_deref().unwrap() as *const _,
            committed_address
        );
        state.event(Event::Error {
            generation: Some(11),
            message: "Transport still settling".into(),
        });
        assert!(state.active && !state.has_resumable_sessions());
        state.event(Event::Thread {
            generation: 11,
            task_id: 7,
            thread_id: "late-old-account-thread".into(),
            model: "observed-model".into(),
            account: "old account".into(),
        });
        assert_eq!(state.thread_id.as_deref(), Some("late-old-account-thread"));
        assert!(!state.has_resumable_sessions());
        state.items.push(message());
        state.event(Event::Finished {
            generation: 11,
            task_id: 7,
            outcome,
            message: "Owned transport settled".into(),
        });
        assert!(!state.active && !state.transport_pending());
        assert!(state.thread_id.is_none() && state.items.is_empty() && state.sessions.is_empty());
        assert!(!state.has_resumable_sessions());
        assert_eq!(state.outcome_unknown(), outcome == Outcome::Unknown);
        assert!(state.notice.starts_with("Owned transport settled"));
        assert_eq!(
            state.committed_input.as_deref().unwrap() as *const _,
            committed_address
        );
        assert_eq!(
            serde_json::to_vec(state.committed_input.as_deref().unwrap()).unwrap(),
            task_bytes
        );
        assert_eq!(
            input.credential.as_ref().unwrap().token(),
            "submitted-account"
        );
        state.event(Event::Thread {
            generation: 11,
            task_id: 7,
            thread_id: "too-late-old-account".into(),
            model: "old-model".into(),
            account: "old account".into(),
        });
        command(&mut state, "resume", true);
        assert!(
            state.pending.is_none(),
            "A terminal old-account session cannot silently become New."
        );
        command(&mut state, "new", true);
        if outcome == Outcome::Unknown {
            assert!(state.unknown_effects());
            assert!(
                state.pending.is_none(),
                "Changing credentials cannot clear unknown external effects."
            );
            continue;
        }
        let Some(Request::Preview(future)) = state.pending.take() else {
            panic!("Only a deliberate fresh request can use the replacement credential.");
        };
        assert!(future.resume.is_none());
        assert_eq!(
            future.credential.as_ref().unwrap().token(),
            "future-account"
        );
    }
}

#[test]
fn deferred_authentication_revocation_preserves_terminal_claim_and_journal_operations() {
    let mut state = active();
    let claim = store::WorkClaim {
        task_id: 7,
        owner: partner(1, "Bolt", 2),
        request_id: "active-old-account".into(),
        writing: false,
    };
    state.work = Some(claim.clone());
    state.work_acquired = true;
    state.thread_id = Some("active-old-account-thread".into());
    state.event(journal_event(11, "inflight-record"));
    state.journal_started();
    state.event(journal_event(11, "pending-record"));
    state.failed_journal.push_back(store::JournalRequest {
        task_id: 7,
        operation: store::JournalOperation::List,
    });
    state.invalidate_authentication();
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Completed,
        message: "Provider settled; storage still pending".into(),
    });
    assert!(!state.has_resumable_sessions() && state.thread_id.is_none());
    assert_eq!(state.work.as_ref(), Some(&claim));
    assert!(state.work_acquired && state.work_pending.is_none());
    assert_eq!(state.pending_journal.len(), 1);
    assert_eq!(state.journal_inflight.len(), 1);
    assert_eq!(state.failed_journal.len(), 1);
    assert!(matches!(
        &state.pending_journal[0].operation,
        store::JournalOperation::Put(record) if record.id == "pending-record"
    ));
    assert!(matches!(
        &state.journal_inflight[0].operation,
        store::JournalOperation::Put(record) if record.id == "inflight-record"
    ));
    state.settle_work();
    assert!(
        state.work_pending.is_none(),
        "Authentication cannot discard journal settlement to release a claim."
    );
}

#[test]
fn typed_slash_is_search_text_and_never_a_command() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    command(&mut state, "model", true);
    for ch in "/new".chars() {
        press(&mut state, KeyCode::Char(ch), true);
    }
    assert_eq!(state.picker_query.text, "/new");
    assert!(state.pending.is_none());
    custom_entry(&mut state);
    state.paste("/new");
    assert_eq!(state.custom_model.as_ref().unwrap().text, "/new");
    assert!(state.pending.is_none() && state.selection.model.is_none());
}

#[test]
fn late_refresh_failure_discards_staged_rows_not_the_successful_snapshot() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    state.inspect_models = true;
    state.active = true;
    state.generation = Some(11);
    let mut updated = catalogue();
    updated.models[0].name = "Uncommitted metadata".into();
    state.event(Event::Models {
        generation: 11,
        task_id: 7,
        catalogue: updated,
    });
    assert_eq!(
        state.catalogues[&Agent::Codex].models[0].name,
        "Advertised model"
    );
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Failed,
        message: "Late failure".into(),
    });
    assert_eq!(
        state.catalogues[&Agent::Codex].models[0].name,
        "Advertised model"
    );
    assert!(state.catalogue_error().is_some());
}

#[test]
fn unknown_effort_can_keep_default_and_metadata_picker_does_not_swallow_cancel() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    state.selection.model = Some("custom-model".into());
    command(&mut state, "effort", true);
    assert_eq!(state.picker_current(), Some(0));
    assert!(state.picker_choice(1).unwrap().reason.is_some());
    press(&mut state, KeyCode::Enter, true);
    assert!(state.picker.is_none() && state.selection.effort.is_none());
    state.active = true;
    state.inspect_models = true;
    state.generation = Some(11);
    state.open_picker(Picker::Model);
    assert!(state.cancel());
    assert!(matches!(
        state.pending,
        Some(Request::Cancel { generation: 11 })
    ));
}

#[test]
fn direct_routes_offer_fresh_requests_not_resume_or_session_metadata() {
    let mut state = State::new();
    state.reset(7);
    for agent in [
        Agent::ChatGPT,
        Agent::OpenAI,
        Agent::Anthropic,
        Agent::Google,
        Agent::Compatible,
    ] {
        state.agent = agent;
        state.thread_id = Some("not-a-direct-session".into());
        for name in ["resume", "sessions"] {
            assert!(
                state
                    .command_choices()
                    .iter()
                    .find(|row| row.command == name)
                    .unwrap()
                    .reason
                    .is_some()
            );
            command(&mut state, name, true);
            assert!(state.pending.is_none());
        }
        assert!(
            state
                .command_choices()
                .iter()
                .any(|row| row.command == "new" && row.reason.is_none())
        );
    }
}

#[test]
fn search_matches_name_and_exact_id_without_losing_filtered_row_identity() {
    let mut state = State::new();
    state.reset(7);
    let mut models = catalogue();
    models.models.push(crate::providers::ModelOption {
        id: "vendor/EXACT-2026".into(),
        name: "Different display".into(),
        description: String::new(),
        efforts: vec![],
        default_effort: None,
    });
    state.catalogues.insert(Agent::Codex, models);
    state.selection.model = Some("advertised-model".into());
    command(&mut state, "model", true);
    assert!(state.paste("different"));
    assert_eq!(state.picker_matches(), 1);
    assert_eq!(state.picker_current(), None);
    assert!(
        state
            .picker_choice(1)
            .unwrap()
            .label
            .contains("vendor/EXACT-2026")
    );
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.selection.model.as_deref(), Some("vendor/EXACT-2026"));
    assert!(state.selection.effort.is_none() && state.pending.is_none());
    command(&mut state, "model", true);
    state.paste("exact-2026");
    assert_eq!(state.picker_current(), Some(1));
    state.key(
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        true,
        &task(),
        &[],
        None,
    );
    assert!(state.picker_query.text.is_empty());
    assert_eq!(state.picker_matches(), 2);
    assert_eq!(state.picker_current(), Some(2));
}

#[test]
fn no_match_and_bounded_paste_keep_default_custom_and_refresh_reachable() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    command(&mut state, "model", true);
    state.paste(&"z".repeat(1024));
    assert_eq!(state.picker_query.text.len(), 256);
    assert_eq!(state.picker_matches(), 0);
    assert_eq!(state.picker_len(), 4);
    assert!(state.picker_choice(1).unwrap().reason.is_some());
    assert_eq!(state.picker_choice(2).unwrap().command, "model-custom");
    assert_eq!(state.picker_choice(3).unwrap().command, "models-refresh");
    assert_eq!(state.picker_current(), Some(0));
    assert!(state.pending.is_none());
}

#[test]
fn custom_id_is_exact_local_and_invalidates_previous_review_and_effort() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    state.selection.effort = Some("high".into());
    command(&mut state, "new", true);
    state.pending.take();
    state.event(Event::Preview(Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: true,
        generation: 11,
        task_id: 7,
        thread_id: None,
        trust: vec![],
        payload: "{}".into(),
        blockers: vec![],
    }));
    state.reviewed = true;
    state.affirmative = true;
    command(&mut state, "model", true);
    custom_entry(&mut state);
    state.paste("vendor/Exact.Model:2026-10");
    assert!(state.custom_model.is_some());
    assert!(state.pending.is_none(), "paste never applies or dispatches");
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(
        state.selection.model.as_deref(),
        Some("vendor/Exact.Model:2026-10")
    );
    assert!(state.selection.effort.is_none());
    assert!(!state.reviewed && !state.affirmative && state.preview.is_none());
    assert!(matches!(
        state.pending,
        Some(Request::Cancel { generation: 11 })
    ));
}

#[test]
fn custom_id_uses_existing_owned_session_confirmation() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    state.thread_id = Some("owned".into());
    command(&mut state, "model", true);
    custom_entry(&mut state);
    state.paste("org/exact");
    press(&mut state, KeyCode::Enter, true);
    assert!(state.selection.model.is_none());
    assert_eq!(
        state.selection_change.as_ref().unwrap().model.as_deref(),
        Some("org/exact")
    );
    press(&mut state, KeyCode::Esc, true);
    assert_eq!(state.thread_id.as_deref(), Some("owned"));
    custom_entry(&mut state);
    state.paste("org/exact");
    press(&mut state, KeyCode::Enter, true);
    state.reviewed = true;
    press(&mut state, KeyCode::Tab, true);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.selection.model.as_deref(), Some("org/exact"));
    assert!(state.thread_id.is_none() && state.pending.is_none());
}

#[test]
fn custom_editor_rejects_control_paste_and_invalid_ids_without_normalizing() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    command(&mut state, "model", true);
    custom_entry(&mut state);
    for value in ["model\n/new", &"x".repeat(257)] {
        state.paste(value);
        assert!(state.custom_model.as_ref().unwrap().text.is_empty());
    }
    for value in ["", " model ", "org/\u{202e}model"] {
        state.custom_model = Some(Answer {
            text: value.into(),
            cursor: value.len(),
        });
        press(&mut state, KeyCode::Enter, true);
        assert!(state.custom_model.is_some() && state.selection.model.is_none());
    }
    state.custom_model = Some(Answer::default());
    state.paste("vendor/模型");
    press(&mut state, KeyCode::Left, true);
    press(&mut state, KeyCode::Backspace, true);
    assert_eq!(state.custom_model.as_ref().unwrap().text, "vendor/型");
    press(&mut state, KeyCode::Esc, true);
    assert!(state.custom_model.is_none() && state.picker.is_some() && state.pending.is_none());
}

#[test]
fn refresh_failure_retains_last_successful_rows_and_retry_replaces_them() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    command(&mut state, "models-refresh", true);
    assert!(
        state.catalogues.contains_key(&Agent::Codex),
        "keep rows during refresh"
    );
    state.pending.take();
    state.event(Event::Error {
        generation: None,
        message: "Offline".into(),
    });
    assert_eq!(
        state.catalogues[&Agent::Codex].models[0].id,
        "advertised-model"
    );
    assert_eq!(state.catalogue_error(), Some("Offline"));
    assert!(state.picker_choice(1).unwrap().reason.is_none());
    command(&mut state, "model", true);
    assert!(state.catalogue_due());
    state.start_automatic_catalogue(&task(), &[], None);
    assert!(state.picker.is_some() && state.catalogue_loading());
    assert!(state.catalogues.contains_key(&Agent::Codex));
    state.pending.take();
    state.event(Event::Preview(Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: false,
        generation: 12,
        task_id: 7,
        thread_id: None,
        trust: vec![],
        payload: "{}".into(),
        blockers: vec![],
    }));
    state.pending.take();
    let mut replacement = catalogue();
    replacement.models[0].name = "Fresh metadata".into();
    state.event(Event::Models {
        generation: 12,
        task_id: 7,
        catalogue: replacement,
    });
    state.event(Event::Finished {
        generation: 12,
        task_id: 7,
        outcome: Outcome::Completed,
        message: "Loaded".into(),
    });
    assert!(state.catalogue_error().is_none());
    assert_eq!(
        state.catalogues[&Agent::Codex].models[0].name,
        "Fresh metadata"
    );
}

#[test]
fn first_fetch_failure_remains_retryable_and_empty_success_is_not_fake_failure() {
    let mut state = State::new();
    state.reset(7);
    command(&mut state, "model", true);
    state.start_automatic_catalogue(&task(), &[], None);
    state.pending.take();
    state.event(Event::Error {
        generation: None,
        message: "Unavailable".into(),
    });
    assert!(state.picker.is_some() && state.catalogues.is_empty());
    assert_eq!(
        state.picker_choice(state.picker_len() - 1).unwrap().command,
        "models-refresh"
    );
    command(&mut state, "model", true);
    assert!(state.catalogue_due());
    state.start_automatic_catalogue(&task(), &[], None);
    state.pending.take();
    state.event(Event::Preview(Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: false,
        generation: 13,
        task_id: 7,
        thread_id: None,
        trust: vec![],
        payload: "{}".into(),
        blockers: vec![],
    }));
    state.pending.take();
    let mut empty = catalogue();
    empty.models.clear();
    state.event(Event::Models {
        generation: 13,
        task_id: 7,
        catalogue: empty,
    });
    state.event(Event::Finished {
        generation: 13,
        task_id: 7,
        outcome: Outcome::Completed,
        message: "Metadata only".into(),
    });
    assert!(state.catalogue_error().is_none() && state.picker.is_some());
    custom_entry(&mut state);
    assert!(state.pending.is_none());
}

#[test]
fn failed_or_missing_refresh_result_keeps_cache_stale() {
    for outcome in [Outcome::Completed, Outcome::Failed] {
        let mut state = State::new();
        state.reset(7);
        state.catalogues.insert(Agent::Codex, catalogue());
        state.inspect_models = true;
        state.active = true;
        state.generation = Some(11);
        state.event(Event::Finished {
            generation: 11,
            task_id: 7,
            outcome,
            message: "Refresh ended".into(),
        });
        assert!(state.catalogues.contains_key(&Agent::Codex));
        assert!(state.catalogue_error().is_some() && state.picker.is_some());
    }
}

#[test]
fn model_picker_is_local_and_refresh_requires_preview_run_without_journal() {
    let mut state = State::new();
    command(&mut state, "model", true);
    assert!(state.pending.is_none());
    assert!(state.catalogues.is_empty());
    command(&mut state, "models-refresh", true);
    let Some(Request::Preview(input)) = state.pending.take() else {
        panic!("inspection preview");
    };
    assert!(input.inspect_models && !input.inspect_sessions && input.resume.is_none());
    assert_eq!(input.selection, ModelSelection::default());
    state.event(Event::Preview(Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: true,
        generation: 11,
        task_id: 7,
        thread_id: None,
        trust: vec!["Metadata connection".into()],
        payload: "{}".into(),
        blockers: vec![],
    }));
    assert!(state.pending_journal.is_empty());
    state.reviewed = true;
    press(&mut state, KeyCode::Tab, true);
    press(&mut state, KeyCode::Enter, true);
    assert!(matches!(
        state.pending.take(),
        Some(Request::Run { generation: 11 })
    ));
    state.event(Event::Models {
        generation: 12,
        task_id: 7,
        catalogue: catalogue(),
    });
    state.event(Event::Models {
        generation: 11,
        task_id: 8,
        catalogue: catalogue(),
    });
    assert!(state.catalogues.is_empty());
    state.event(Event::Models {
        generation: 11,
        task_id: 7,
        catalogue: catalogue(),
    });
    assert!(
        state.catalogues.is_empty(),
        "publish only after successful completion"
    );
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Completed,
        message: "Metadata complete".into(),
    });
    assert!(state.picker == Some(Picker::Model));
    assert!(state.pending_journal.is_empty());
    assert!(state.thread_id.is_none() && state.items.is_empty());
}

#[test]
fn model_change_reviews_complete_model_and_reasoning_without_silent_resume() {
    let mut state = State::new();
    state.reset(7);
    state.thread_id = Some("owned-session".into());
    state.catalogues.insert(Agent::Codex, catalogue());
    command(&mut state, "model", true);
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    assert!(state.picker == Some(Picker::Effort));
    assert!(state.selection_change.is_none());
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(
        state.selection_change.as_ref().unwrap().effort.as_deref(),
        Some("high")
    );
    assert_eq!(state.selection, ModelSelection::default());
    press(&mut state, KeyCode::Esc, true);
    assert!(state.picker == Some(Picker::Effort) && state.selection_change.is_none());
    assert_eq!(state.effort_model().unwrap().id, "advertised-model");
    press(&mut state, KeyCode::Enter, true);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.thread_id.as_deref(), Some("owned-session"));
    assert_eq!(state.selection, ModelSelection::default());
    command(&mut state, "model", true);
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    state.reviewed = true;
    press(&mut state, KeyCode::Tab, true);
    press(&mut state, KeyCode::Enter, true);
    assert!(state.thread_id.is_none() && state.pending.is_none());
    assert_eq!(state.selection.model.as_deref(), Some("advertised-model"));
    assert_eq!(state.selection.effort.as_deref(), Some("high"));
    command(&mut state, "resume", true);
    assert!(state.pending.is_none());
}

#[test]
fn advertised_effort_and_model_are_snapshotted_and_preview_change_revokes_consent() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    command(&mut state, "model", true);
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    assert!(state.picker == Some(Picker::Effort));
    assert_eq!(state.selection, ModelSelection::default());
    assert!(state.pending.is_none());
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    assert!(state.picker.is_none());
    command(&mut state, "new", true);
    let Some(Request::Preview(input)) = state.pending.take() else {
        panic!("turn preview");
    };
    assert_eq!(input.selection.model.as_deref(), Some("advertised-model"));
    assert_eq!(input.selection.effort.as_deref(), Some("high"));
    state.event(Event::Preview(Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: true,
        generation: 11,
        task_id: 7,
        thread_id: None,
        trust: vec![],
        payload: "payload".into(),
        blockers: vec![],
    }));
    command(&mut state, "effort", true);
    assert_eq!(
        state.picker_selected, 1,
        "picker opens on the current effort"
    );
    press(&mut state, KeyCode::Home, true);
    press(&mut state, KeyCode::Enter, true);
    assert!(state.selection.effort.is_none());
    assert!(state.preview.is_none() && !state.affirmative);
    assert!(matches!(
        state.pending,
        Some(Request::Cancel { generation: 11 })
    ));
    assert_eq!(input.selection.effort.as_deref(), Some("high"));
}

#[test]
fn missing_retained_selection_is_never_marked_as_default_or_changed_on_open() {
    let mut state = State::new();
    state.reset(7);
    state.catalogues.insert(Agent::Codex, catalogue());
    state.selection = ModelSelection {
        model: Some("previously-selected".into()),
        effort: Some("previous-effort".into()),
    };
    let selected = state.selection.clone();
    command(&mut state, "model", true);
    assert_eq!(state.picker_current(), None);
    assert_eq!(state.selection, selected);
    assert!(state.pending.is_none());
    press(&mut state, KeyCode::Enter, true);
    assert!(state.custom_model.is_some());
    assert!(state.pending.is_none());
    assert_eq!(state.selection, selected);
    press(&mut state, KeyCode::Esc, true);
    press(&mut state, KeyCode::Esc, true);
    command(&mut state, "effort", true);
    assert_eq!(state.picker_current(), None);
    assert_eq!(state.selection, selected);
    assert!(state.pending.is_none());
}

#[test]
fn newer_catalogue_choices_keep_exact_model_and_effort_without_dispatch() {
    let mut state = State::new();
    state.reset(7);
    let mut models = catalogue();
    models.models.push(crate::providers::ModelOption {
        id: "provider-next-generation".into(),
        name: "New native model".into(),
        description: "Native discovery response".into(),
        efforts: vec!["max".into()],
        default_effort: Some("max".into()),
    });
    state.catalogues.insert(Agent::Codex, models);
    command(&mut state, "model", true);
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.selection, ModelSelection::default());
    assert!(state.picker == Some(Picker::Effort));
    assert_eq!(state.effort_model().unwrap().id, "provider-next-generation");
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.selection.effort.as_deref(), Some("max"));
    assert!(state.pending.is_none() && state.thread_id.is_none());
}

#[test]
fn active_model_change_is_refused_and_unsupported_effort_cannot_be_selected() {
    let mut state = active();
    command(&mut state, "model", true);
    assert!(state.picker.is_none() && state.pending.is_none());
    state.active = false;
    state.generation = None;
    command(&mut state, "effort", true);
    assert!(state.picker_choice(1).unwrap().reason.is_some());
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    assert!(state.selection.effort.is_none() && state.pending.is_none());
    assert!(state.picker.is_some());
}

#[test]
fn provider_selection_is_local_to_provider_and_survives_task_reset() {
    let mut state = State::new();
    state.reset(7);
    state.selection.model = Some("codex-model".into());
    state.switch_to = Some(Agent::Claude);
    state.complete_switch();
    assert_eq!(state.selection, ModelSelection::default());
    state.selection.model = Some("claude-model".into());
    state.switch_to = Some(Agent::Codex);
    state.complete_switch();
    assert_eq!(state.selection.model.as_deref(), Some("codex-model"));
    state.reset(8);
    assert_eq!(state.selection.model.as_deref(), Some("codex-model"));
}

#[test]
fn obsolete_action_letters_and_digits_never_launch_or_change_idle_selection() {
    let mut state = State::new();
    state.reset(7);
    for action in "nrsfgxdhbcq12345".chars() {
        press(&mut state, KeyCode::Char(action), true);
        assert!(state.pending.is_none() && state.switch_to.is_none());
        assert!(state.paths_editor.is_none() && !state.edit_goal && !state.quit_review);
        assert_eq!(state.agent, Agent::Codex);
        assert_eq!(state.operation, Operation::Plan);
    }
}

#[test]
fn unsafe_model_catalogue_is_rejected_before_caching() {
    let mut state = active();
    state.inspect_models = true;
    let mut invalid = catalogue();
    invalid.models[0].name = "unsafe\ncontrol".into();
    state.event(Event::Models {
        generation: 11,
        task_id: 7,
        catalogue: invalid,
    });
    assert!(state.catalogues.is_empty());
    assert!(state.notice.starts_with("Model catalogue rejected:"));
}

fn start_catalogue(state: &mut State, task: &store::Snapshot, generation: u64) {
    state.command("model", true, task, &[], None);
    assert!(state.catalogue_due());
    state.start_automatic_catalogue(task, &[], None);
    assert!(matches!(state.pending.take(), Some(Request::Preview(_))));
    state.event(Event::Preview(Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: false,
        generation,
        task_id: task.id,
        thread_id: None,
        trust: Vec::new(),
        payload: "{}".into(),
        blockers: Vec::new(),
    }));
    assert!(
        matches!(state.pending.take(), Some(Request::Run { generation: found }) if found == generation)
    );
}

#[test]
fn fresh_and_archived_catalogues_strip_content_role_and_stale_selection() {
    for agent in Agent::ALL {
        for id in [0, 7] {
            let mut state = State::new();
            state.agent = agent;
            state.operation = Operation::Review;
            state.edit_paths.push("never-send.txt".into());
            state.selection.model = Some("removed-model".into());
            state.selection.effort = Some("removed-effort".into());
            let mut task = store::Snapshot::empty("project".into());
            task.id = id;
            task.archived = id > 0;
            task.goal = "private-goal-canary".into();
            task.next_action = "private-next-action".into();
            state.prepare(false, false, true, &task, &[], None);
            let Some(Request::Preview(input)) = state.pending.take() else {
                panic!("metadata preview must not require task storage");
            };
            assert_eq!(input.task.id, id);
            assert_eq!(input.task.project, "project");
            assert!(input.task.goal.is_empty() && input.task.next_action.is_empty());
            assert!(input.task.request_text().is_empty());
            assert!(input.excerpts.is_empty() && input.report.is_none());
            assert!(input.edit_paths.is_empty() && input.handoff.is_none());
            assert_eq!(input.operation, Operation::Plan);
            assert_eq!(input.selection, ModelSelection::default());
            assert!(!state.needs_saved_command("models-refresh"));
            assert!(
                !state.needs_saved_input(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            );
        }
    }
    let mut state = State::new();
    state.prepare(
        false,
        false,
        false,
        &store::Snapshot::empty("project".into()),
        &[],
        None,
    );
    assert!(
        state.pending.is_none(),
        "prompt admission still requires a saved task"
    );
}

#[test]
fn failed_metadata_reopens_persistent_reason_and_retry_recovers() {
    let mut state = State::new();
    let task = task();
    start_catalogue(&mut state, &task, 11);
    state.event(Event::Finished {
        generation: 11,
        task_id: task.id,
        outcome: Outcome::Failed,
        message: "Authentication unavailable: use /connect.".into(),
    });
    assert!(!state.busy_except_picker());
    assert!(state.picker == Some(Picker::Model));
    state.notice = "Unrelated transient notice".into();
    assert!(
        state
            .picker_choice(1)
            .unwrap()
            .reason
            .unwrap()
            .contains("/connect")
    );
    start_catalogue(&mut state, &task, 12);
    state.event(Event::Models {
        generation: 11,
        task_id: task.id,
        catalogue: catalogue(),
    });
    assert!(
        state.catalogues.is_empty(),
        "late failed-request metadata must be ignored"
    );
    state.event(Event::Models {
        generation: 12,
        task_id: task.id,
        catalogue: catalogue(),
    });
    state.event(Event::Finished {
        generation: 12,
        task_id: task.id,
        outcome: Outcome::Completed,
        message: "Complete".into(),
    });
    assert!(state.picker == Some(Picker::Model));
    assert!(state.picker_choice(1).unwrap().reason.is_none());
    assert!(state.catalogue_errors.is_empty());
    assert!(state.pending_journal.is_empty() && state.thread_id.is_none());
}

#[test]
fn blocked_preview_and_preview_error_retain_reason_without_automatic_manual_run() {
    for preview_error in [false, true] {
        let mut state = State::new();
        let task = task();
        state.command("model", true, &task, &[], None);
        state.start_automatic_catalogue(&task, &[], None);
        state.pending.take();
        if preview_error {
            state.event(Event::Error {
                generation: None,
                message: "Executable missing: install the provider explicitly.".into(),
            });
        } else {
            state.event(Event::Preview(Preview {
                arrival: crate::providers::approval_arrival(),
                destination: None,
                requires_connection_review: true,
                generation: 11,
                task_id: task.id,
                thread_id: None,
                trust: Vec::new(),
                payload: "{}".into(),
                blockers: vec!["Executable missing: install the provider explicitly.".into()],
            }));
            assert!(matches!(
                state.pending.take(),
                Some(Request::Cancel { generation: 11 })
            ));
            state.event(Event::Finished {
                generation: 11,
                task_id: task.id,
                outcome: Outcome::Cancelled,
                message: "Preview discarded".into(),
            });
        }
        assert!(state.picker == Some(Picker::Model));
        assert!(!state.busy_except_picker());
        assert!(
            state
                .picker_choice(1)
                .unwrap()
                .reason
                .unwrap()
                .contains("install the provider explicitly")
        );
        state.command("models-refresh", true, &task, &[], None);
        assert!(matches!(state.pending.take(), Some(Request::Preview(_))));
        state.event(Event::Preview(Preview {
            arrival: crate::providers::approval_arrival(),
            destination: None,
            requires_connection_review: true,
            generation: 12,
            task_id: task.id,
            thread_id: None,
            trust: Vec::new(),
            payload: "{}".into(),
            blockers: Vec::new(),
        }));
        assert!(state.preview.is_some() && state.pending.is_none(),);
    }
}

#[test]
fn refresh_retains_cache_and_completed_without_valid_metadata_marks_it_stale() {
    for invalid in [false, true] {
        let mut state = State::new();
        state.reset(7);
        state.catalogues.insert(Agent::Codex, catalogue());
        state.command("models-refresh", true, &task(), &[], None);
        assert!(state.catalogues.contains_key(&Agent::Codex));
        state.pending.take();
        state.awaiting_preview = false;
        state.generation = Some(11);
        state.active = true;
        if invalid {
            let mut bad = catalogue();
            bad.models[0].name = "invalid\nname".into();
            state.event(Event::Models {
                generation: 11,
                task_id: 7,
                catalogue: bad,
            });
        }
        state.event(Event::Finished {
            generation: 11,
            task_id: 7,
            outcome: Outcome::Completed,
            message: "Complete".into(),
        });
        assert!(state.catalogues.contains_key(&Agent::Codex));
        assert!(state.picker_choice(1).unwrap().reason.is_none());
        assert!(state.catalogue_error().unwrap().contains(if invalid {
            "rejected"
        } else {
            "without a usable model catalogue"
        }));
        assert!(!state.notice.contains("Models loaded."));
    }
}

#[test]
fn provider_switch_does_not_transfer_queued_inspection_or_failure() {
    let mut state = State::new();
    state.reset(7);
    state
        .catalogue_errors
        .insert(Agent::Codex, "Codex-only failure".into());
    state.auto_models = true;
    state.auto_picker = true;
    state.switch_to = Some(Agent::Claude);
    state.complete_switch();
    assert!(!state.catalogue_due());
    state.open_picker(Picker::Model);
    assert!(
        !state
            .picker_choice(1)
            .unwrap()
            .reason
            .unwrap()
            .contains("Codex-only")
    );
    state.picker = None;
    state.switch_to = Some(Agent::Codex);
    state.complete_switch();
    state.open_picker(Picker::Model);
    assert!(
        state
            .picker_choice(1)
            .unwrap()
            .reason
            .unwrap()
            .contains("Codex-only")
    );
}

#[test]
fn fresh_catalogue_owns_original_task_until_completion_after_save() {
    let mut state = State::new();
    let fresh = store::Snapshot::empty("project".into());
    start_catalogue(&mut state, &fresh, 11);
    // An autosave can assign a task ID while metadata is in flight.
    state.key(
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        true,
        &task(),
        &[],
        None,
    );
    assert_eq!(state.task_id(), 0);
    state.event(Event::Models {
        generation: 11,
        task_id: 0,
        catalogue: catalogue(),
    });
    state.event(Event::Finished {
        generation: 11,
        task_id: 0,
        outcome: Outcome::Completed,
        message: "Complete".into(),
    });
    assert!(state.picker == Some(Picker::Model));
    assert!(state.catalogues.contains_key(&Agent::Codex));
}

fn preview() -> State {
    let mut state = State::new();
    command(&mut state, "new", true);
    assert!(matches!(state.pending.take(), Some(Request::Preview(_))));
    state.event(Event::Preview(Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: true,
        generation: 11,
        task_id: 7,
        thread_id: None,
        trust: vec!["Executable trust".into()],
        payload: "exact goal".into(),
        blockers: Vec::new(),
    }));
    state
}

#[test]
fn preview_shows_local_receipt() {
    let mut state = State::new();
    command(&mut state, "new", true);
    state.pending.take();
    state.event(Event::Preview(Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: true,
        generation: 11,
        task_id: 7,
        thread_id: None,
        trust: Vec::new(),
        payload: "{\"purpose\":\"Plan\",\"task\":{\"goal\":\"한국어\"}}".into(),
        blockers: Vec::new(),
    }));
    assert!(
        state
            .receipt
            .iter()
            .any(|line| line.starts_with("Instructions:"))
    );
    assert!(state.pending_journal.is_empty());
}

fn active() -> State {
    let mut state = preview();
    state.reviewed = true;
    press(&mut state, KeyCode::Tab, true);
    press(&mut state, KeyCode::Enter, true);
    assert!(matches!(
        state.pending.take(),
        Some(Request::Run { generation: 11 })
    ));
    state
}

#[test]
fn review_never_resumes_or_carries_implementation_grants_or_handoff() {
    let mut state = State::new();
    state.reset(7);
    state.set_operation(Operation::Implement);
    state.thread_id = Some("implementation-session".into());
    state.edit_paths.push("src/file.rs".into());
    state.handoff = Some(crate::workspace::handoff::from_task(&task(), &[], &[]).unwrap());
    state.set_operation(Operation::Review);
    assert!(state.thread_id.is_none());
    state.thread_id = Some("previous-review-session".into());
    command(&mut state, "resume", true);
    assert!(state.pending.is_none());
    assert!(state.notice.contains("fresh session"));
    command(&mut state, "paths", true);
    assert!(state.paths_editor.is_none());
    command(&mut state, "role-implement", true);
    assert_eq!(state.operation, Operation::Review);
    command(&mut state, "new", true);
    let Some(Request::Preview(input)) = state.pending.take() else {
        panic!("New review must prepare a local preview");
    };
    assert_eq!(input.operation, Operation::Review);
    assert!(input.resume.is_none());
    assert!(input.edit_paths.is_empty());
    assert!(input.handoff.is_none());
    assert!(!input.inspect_sessions);
}

#[test]
fn preview_is_passive_and_run_needs_last_page_and_deliberate_choice() {
    let mut state = preview();
    press(&mut state, KeyCode::Tab, true);
    assert!(!state.affirmative);
    assert!(state.pending.is_none());
    state.reviewed = true;
    press(&mut state, KeyCode::Tab, true);
    state.key(
        KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat),
        true,
        &task(),
        &[],
        None,
    );
    assert!(state.pending.is_none());
    press(&mut state, KeyCode::Enter, false);
    assert!(state.pending.is_none());
    assert!(!state.affirmative);
    press(&mut state, KeyCode::Enter, true);
    assert!(matches!(
        state.pending,
        Some(Request::Cancel { generation: 11 })
    ));
}

#[test]
fn cancel_invalidates_prompt_and_rejects_stale_generation() {
    let mut state = active();
    state.event(Event::Status {
        generation: 12,
        task_id: 7,
        message: "stale".into(),
    });
    assert_ne!(state.notice, "stale");
    assert!(state.cancel());
    state.event(Event::Prompt {
        generation: 11,
        task_id: 7,
        request: Prompt {
            arrival: crate::providers::approval_arrival(),
            request_id: json!(1),
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "item".into(),
            kind: PromptKind::Approval {
                command: "echo".into(),
                cwd: ".".into(),
                reason: "test".into(),
                allow_once: true,
                deny_decision: "cancel".into(),
            },
        },
    });
    assert!(state.request.is_none());
    assert!(state.busy());
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Cancelled,
        message: "cancelled".into(),
    });
    assert!(!state.busy());
}

#[test]
fn pasted_commands_are_literal_and_cjk_cursor_stays_on_graphemes() {
    let mut state = active();
    state.event(Event::Prompt {
        generation: 11,
        task_id: 7,
        request: Prompt {
            arrival: crate::providers::approval_arrival(),
            request_id: json!(1),
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "item".into(),
            kind: PromptKind::Questions(vec![Question {
                id: "q".into(),
                header: "Choice".into(),
                question: "What next?".into(),
                options: Vec::new(),
                other: true,
                secret: false,
            }]),
        },
    });
    assert!(!state.paste("accept\n"));
    command(&mut state, "answer", true);
    assert!(state.paste("計画e\u{301}"));
    press(&mut state, KeyCode::Backspace, true);
    assert_eq!(state.answers[0].text, "計画");
    assert!(state.paste("\nq\taccept\r"));
    assert_eq!(state.answers[0].text, "計画qaccept");
    assert!(state.pending.is_none());
    press(&mut state, KeyCode::Enter, true);
    assert!(state.pending.is_none());
    command(&mut state, "answers-review", true);
    assert!(state.submit_review);
    assert!(!state.affirmative);
}

#[test]
fn small_exit_is_neutral_and_unknown_outcome_requires_second_choice() {
    let mut state = active();
    state.request_quit();
    press(&mut state, KeyCode::Enter, false);
    assert!(!state.quit_ready);
    assert!(!state.cancelling);
    state.request_quit();
    press(&mut state, KeyCode::Tab, false);
    press(&mut state, KeyCode::Enter, false);
    assert!(state.cancelling);
    assert!(!state.quit_ready);
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Unknown,
        message: "unknown outcome".into(),
    });
    assert!(state.quit_review);
    assert!(!state.affirmative);
    assert!(!state.quit_ready);
    press(&mut state, KeyCode::Tab, false);
    press(&mut state, KeyCode::Enter, false);
    assert!(state.quit_ready);
}

#[test]
fn denial_uses_the_server_offered_cancel_not_invented_decline() {
    let mut state = active();
    state.event(Event::Prompt {
        generation: 11,
        task_id: 7,
        request: Prompt {
            arrival: crate::providers::approval_arrival(),
            request_id: json!(23),
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "item".into(),
            kind: PromptKind::Approval {
                command: "echo".into(),
                cwd: ".".into(),
                reason: "test".into(),
                allow_once: true,
                deny_decision: "cancel".into(),
            },
        },
    });
    assert!(!state.paste("accept"));
    press(&mut state, KeyCode::Enter, true);
    match state.pending.take() {
        Some(Request::Answer {
            generation: 11,
            request_id,
            answer,
        }) => {
            assert_eq!(request_id, json!(23));
            assert_eq!(answer, json!({"decision":"cancel"}));
        }
        _ => panic!("explicit deny should answer exactly the offered server decision"),
    }
}
#[test]
fn idle_switch_preserves_old_session_without_portable_grants_or_ids() {
    let mut state = State::new();
    state.reset(7);
    state.thread_id = Some("opaque-old-thread".into());
    command(&mut state, "provider-claude", true);
    assert_eq!(state.agent, Agent::Codex);
    assert!(state.pending.is_none());
    let document = serde_json::to_string(state.handoff.as_ref().unwrap()).unwrap();
    assert!(!document.contains("opaque-old-thread"));
    state.reviewed = true;
    press(&mut state, KeyCode::Tab, true);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.agent, Agent::Claude);
    assert!(state.thread_id.is_none());
    assert!(state.pending.is_none());
    command(&mut state, "provider-codex", true);
    state.reviewed = true;
    press(&mut state, KeyCode::Tab, true);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.thread_id.as_deref(), Some("opaque-old-thread"));
}

#[test]
fn fifth_provider_switch_requires_review_and_does_not_transfer_session() {
    let mut state = State::new();
    state.reset(7);
    state.thread_id = Some("codex-owned-thread".into());
    assert!(state.needs_saved_command("provider-devin"));
    command(&mut state, "provider-devin", true);
    assert_eq!(state.agent, Agent::Codex);
    assert_eq!(state.switch_to, Some(Agent::Devin));
    assert!(state.pending.is_none());
    state.reviewed = true;
    press(&mut state, KeyCode::Tab, true);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.agent, Agent::Devin);
    assert!(state.thread_id.is_none());
    assert!(state.pending.is_none());
}

#[test]
fn failed_journal_is_retained_for_storage_only_retry_and_prevents_reset() {
    let mut state = State::new();
    state.reset(7);
    state.pending_journal.push_back(store::JournalRequest {
        task_id: 7,
        operation: store::JournalOperation::List,
    });
    state.journal_started();
    state.journal_event(7, Err("locked storage".into()));
    state.reset(9);
    assert_eq!(state.task_id(), 7);
    assert!(state.journal_failed());
    state.retry_journal();
    assert!(matches!(
        state.pending_journal.front().unwrap().operation,
        store::JournalOperation::List
    ));
    assert!(
        state.pending.is_none(),
        "Journal retry must not create a provider request"
    );
}

fn message() -> Item {
    Item {
        thread_id: "old-thread".into(),
        turn_id: "old-turn".into(),
        item_id: "old-item".into(),
        kind: ItemKind::Message,
        text: "Visible decision".into(),
        complete: true,
        request: "Original request".into(),
        owner: None,
    }
}

#[test]
fn metadata_inspection_cannot_select_a_hidden_message_or_replace_its_session() {
    let mut state = active();
    state.thread_id = Some("old-thread".into());
    state.items.push(message());
    state.inspect_sessions = true;
    state.event(Event::Thread {
        generation: 11,
        task_id: 7,
        thread_id: "listed-not-selected".into(),
        model: "metadata".into(),
        account: "metadata".into(),
    });
    state.event(Event::Item {
        generation: 11,
        task_id: 7,
        thread_id: "listed-not-selected".into(),
        turn_id: "not-a-turn".into(),
        item_id: "metadata".into(),
        kind: ItemKind::Message,
        text: "not a decision".into(),
        complete: true,
    });
    state.event(Event::Inspection {
        generation: 11,
        task_id: 7,
        text: "Metadata only".into(),
    });
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Completed,
        message: "Inspection completed".into(),
    });
    assert_eq!(state.thread_id.as_deref(), Some("old-thread"));
    assert_eq!(state.items.len(), 1);
    command(&mut state, "decision", true);
    for code in [KeyCode::Enter, KeyCode::Down] {
        press(&mut state, code, true);
        assert!(state.decision.is_none());
        assert_eq!(state.selected, 0);
    }
    press(&mut state, KeyCode::Esc, true);
    assert!(state.inspection.is_none());
    command(&mut state, "decision", true);
    assert!(
        state
            .decision
            .as_ref()
            .unwrap()
            .starts_with("Visible decision")
    );
}

#[test]
fn leaving_live_metadata_inspection_cancels_and_rejects_late_metadata() {
    let mut state = active();
    state.inspect_sessions = true;
    state.inspection = Some("Capabilities; session list still loading".into());
    press(&mut state, KeyCode::Esc, true);
    assert!(state.inspection.is_none());
    assert!(state.cancelling);
    assert!(matches!(
        state.pending,
        Some(Request::Cancel { generation: 11 })
    ));
    state.event(Event::Inspection {
        generation: 11,
        task_id: 7,
        text: "Late list".into(),
    });
    assert!(state.inspection.is_none());
    assert!(
        state.inspect_sessions,
        "Cancellation must not turn metadata into model output"
    );
}

#[test]
fn resize_and_quit_reset_permission_to_deny() {
    let mut state = active();
    state.event(Event::Prompt {
        generation: 11,
        task_id: 7,
        request: Prompt {
            arrival: crate::providers::approval_arrival(),
            request_id: json!(1),
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "item".into(),
            kind: PromptKind::Permission {
                description: "Review exact scope".into(),
                file: None,
                options: vec![
                    crate::providers::PermissionOption {
                        label: "Deny".into(),
                        description: "No permission".into(),
                        answer: json!({"permission":"deny"}),
                    },
                    crate::providers::PermissionOption {
                        label: "Allow once".into(),
                        description: "One request".into(),
                        answer: json!({"permission":"once"}),
                    },
                ],
            },
        },
    });
    state.reviewed = true;
    press(&mut state, KeyCode::Tab, true);
    assert_eq!(state.option, 1);
    press(&mut state, KeyCode::Enter, false);
    assert_eq!(state.option, 0);
    assert!(state.pending.is_none());
    state.reviewed = true;
    press(&mut state, KeyCode::Tab, true);
    state.request_quit();
    press(&mut state, KeyCode::Esc, true);
    assert_eq!(state.option, 0);
    press(&mut state, KeyCode::Enter, true);
    assert!(matches!(
        state.pending,
        Some(Request::Answer { answer, .. }) if answer == json!({"permission":"deny"})
    ));
}

#[test]
fn prior_unknown_outcome_survives_later_completion_and_authentication_reset() {
    let mut state = active();
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Unknown,
        message: "No authoritative completion".into(),
    });
    command(&mut state, "sessions", true);
    state.pending.take();
    state.event(Event::Preview(Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: true,
        generation: 12,
        task_id: 7,
        thread_id: None,
        trust: Vec::new(),
        payload: String::new(),
        blockers: Vec::new(),
    }));
    state.reviewed = true;
    press(&mut state, KeyCode::Tab, true);
    press(&mut state, KeyCode::Enter, true);
    state.pending.take();
    assert!(state.outcome_unknown());
    state.event(Event::Finished {
        generation: 12,
        task_id: 7,
        outcome: Outcome::Completed,
        message: "Metadata inspection succeeded".into(),
    });
    state.thread_id = Some("old-auth-thread".into());
    state.items.push(message());
    state.sessions.insert(
        (Agent::Claude, Operation::Implement),
        Session {
            thread_id: Some("other-auth-thread".into()),
            ..Session::default()
        },
    );
    state.failed_journal.push_back(store::JournalRequest {
        task_id: 7,
        operation: store::JournalOperation::List,
    });
    state.invalidate_authentication();
    assert!(state.outcome_unknown());
    assert!(state.journal_failed());
    assert!(state.thread_id.is_none());
    assert!(state.items.is_empty());
    assert!(state.sessions.is_empty());
    assert!(!state.reviewed && !state.affirmative);
}

#[test]
fn journal_retry_is_not_blocked_by_provider_save_gate_and_selection_starts_at_top() {
    let mut state = State::new();
    state.reset(7);
    state.failed_journal.push_back(store::JournalRequest {
        task_id: 7,
        operation: store::JournalOperation::List,
    });
    command(&mut state, "provider-journal", true);
    state.journal_started();
    state.journal_event(7, Ok(store::JournalResponse::Records(Vec::new())));
    assert!(!state.needs_saved_command("journal-retry"));
    command(&mut state, "journal-retry", true);
    assert!(!state.journal_failed());
    assert_eq!(state.pending_journal.len(), 1);
    assert!(state.pending.is_none());
    state.scroll = 40;
    state.scroll_max = 80;
    press(&mut state, KeyCode::Down, true);
    assert_eq!(state.scroll, 0);
    assert_eq!(state.scroll_max, 0);
}

#[test]
fn unsolicited_or_wrong_kind_journal_reply_cannot_clear_retained_failure() {
    let mut state = State::new();
    state.reset(7);
    state.failed_journal.push_back(store::JournalRequest {
        task_id: 7,
        operation: store::JournalOperation::List,
    });
    state.journal_event(7, Ok(store::JournalResponse::Purged));
    assert!(state.journal_failed());
    state.pending_journal.push_back(store::JournalRequest {
        task_id: 7,
        operation: store::JournalOperation::List,
    });
    state.journal_started();
    state.journal_event(7, Ok(store::JournalResponse::Purged));
    assert_eq!(state.failed_journal.len(), 2);
    state.retry_journal();
    assert!(
        state
            .pending_journal
            .iter()
            .all(|request| matches!(request.operation, store::JournalOperation::List))
    );
}

fn streamed(state: &mut State, id: &str, text: &str, complete: bool) {
    state.event(Event::Item {
        generation: 11,
        task_id: 7,
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: id.into(),
        kind: ItemKind::Message,
        text: text.into(),
        complete,
    });
}

fn file_snapshot(state: &mut State, id: &str, phase: FilePhase, text: &str) {
    state.event(Event::Item {
        generation: 11,
        task_id: 7,
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: id.into(),
        kind: ItemKind::File(phase),
        text: text.into(),
        complete: phase != FilePhase::Generating,
    });
}

#[test]
fn file_cards_settle_only_owned_unfinished_stages_and_never_become_decisions() {
    for outcome in [
        Outcome::Completed,
        Outcome::Failed,
        Outcome::Cancelled,
        Outcome::Unknown,
    ] {
        let mut state = active();
        state.submitted_request = "owned request".into();
        let phases = [
            FilePhase::Generating,
            FilePhase::Received,
            FilePhase::AwaitingReview,
            FilePhase::Applied,
            FilePhase::Denied,
            FilePhase::Refused,
            FilePhase::Reported,
            FilePhase::ReportedFailed,
            FilePhase::Unknown,
        ];
        for (index, phase) in phases.into_iter().enumerate() {
            file_snapshot(
                &mut state,
                &index.to_string(),
                phase,
                "src/example.rs\n1 | observed",
            );
        }
        let mut retained = message();
        retained.kind = ItemKind::File(FilePhase::Generating);
        state.items.push(retained);
        state.event(Event::Finished {
            generation: 11,
            task_id: 7,
            outcome,
            message: "terminal".into(),
        });
        for (index, original) in phases.into_iter().enumerate() {
            let expected = match original {
                FilePhase::Generating | FilePhase::AwaitingReview => {
                    if outcome == Outcome::Unknown {
                        FilePhase::Unknown
                    } else {
                        FilePhase::Interrupted
                    }
                }
                FilePhase::Received if outcome != Outcome::Completed => {
                    if outcome == Outcome::Unknown {
                        FilePhase::Unknown
                    } else {
                        FilePhase::Interrupted
                    }
                }
                _ => original,
            };
            assert_eq!(state.items[index].kind, ItemKind::File(expected));
            state.selected = index;
            command(&mut state, "decision", true);
            assert!(state.decision.is_none());
        }
        assert_eq!(
            state.items.last().unwrap().kind,
            ItemKind::File(FilePhase::Generating)
        );
        assert!(state.pending_journal.is_empty() && state.native_histories.is_empty());
    }
}

#[test]
fn cancellation_retains_real_file_outcomes_and_rejects_late_generation() {
    let mut state = active();
    state.submitted_request = "request".into();
    file_snapshot(
        &mut state,
        "write",
        FilePhase::AwaitingReview,
        "new.rs\n1 | proposal",
    );
    assert!(state.cancel());
    assert_eq!(state.items[0].kind, ItemKind::File(FilePhase::Interrupted));
    file_snapshot(
        &mut state,
        "write",
        FilePhase::Generating,
        "new.rs\n1 | stale",
    );
    assert_eq!(state.items[0].text, "new.rs\n1 | proposal");
    file_snapshot(
        &mut state,
        "write",
        FilePhase::Applied,
        "new.rs\n1 | proposal",
    );
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Cancelled,
        message: "cancel settled after receipt".into(),
    });
    assert_eq!(state.items[0].kind, ItemKind::File(FilePhase::Applied));
    assert!(state.items[0].complete && state.pending_journal.is_empty());
}

#[test]
fn file_card_updates_keep_chronology_owner_and_paused_transcript_anchor() {
    let mut state = active();
    state.submitted_request = "first request".into();
    let owner = store::Attribution::of(&store::Partner::default());
    state.slots.insert(
        owner.partner,
        Slot {
            owner: owner.clone(),
            task: 7,
            generation: Some(11),
        },
    );
    streamed(&mut state, "before", "held first\nheld second", true);
    file_snapshot(
        &mut state,
        "write",
        FilePhase::Generating,
        "new.rs\n1 | first",
    );
    streamed(&mut state, "after", "latest response", true);
    state.response_rows(60, 2, false).count();
    press(&mut state, KeyCode::Home, true);
    state.scroll_response_lines(2);
    let held = state
        .response_rows(60, 2, false)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    state.submitted_request = "later request".into();
    state.slots.get_mut(&owner.partner).unwrap().owner.name = "Later owner".into();
    file_snapshot(
        &mut state,
        "write",
        FilePhase::Received,
        "new.rs\n1 | first\n2 | second",
    );
    assert_eq!(
        state
            .items
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<Vec<_>>(),
        ["before", "write", "after"]
    );
    assert_eq!(state.items[1].owner.as_ref(), Some(&owner));
    assert_eq!(state.items[1].request.as_ref(), "first request");
    assert_eq!(
        state.response_rows(60, 2, false).collect::<Vec<_>>(),
        held.iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert!(!state.following && state.new_output);
    state.resume_follow();
    assert_eq!(
        state.response_rows(60, 1, false).next(),
        Some("latest response")
    );
}

fn run_request(state: &mut State, generation: u64, request: &str) {
    command(state, "new", true);
    assert!(matches!(state.pending.take(), Some(Request::Preview(_))));
    state.event(Event::Preview(Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: true,
        generation,
        task_id: 7,
        thread_id: None,
        trust: vec!["Executable trust".into()],
        payload: request.into(),
        blockers: Vec::new(),
    }));
    state.reviewed = true;
    press(state, KeyCode::Char('y'), true);
    assert!(
        matches!(state.pending.take(), Some(Request::Run { generation: actual }) if actual == generation)
    );
    // App captures the exact saved request when Run is approved.
    state.submitted_request = request.into();
}

#[test]
fn retained_output_keeps_request_ownership_during_startup_failure_and_navigation() {
    let mut state = active();
    state.submitted_request = "First question".into();
    streamed(&mut state, "first", "First answer", true);
    assert_eq!(state.response_request(), ("First question", false));
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Completed,
        message: "Complete".into(),
    });
    run_request(&mut state, 12, "Second question");
    assert_eq!(state.response_request(), ("First question", true));
    assert_eq!(state.submitted_request.as_ref(), "Second question");
    state.event(Event::Finished {
        generation: 12,
        task_id: 7,
        outcome: Outcome::Failed,
        message: "No output".into(),
    });
    assert_eq!(state.response_request(), ("First question", true));
    assert!(state.failed_request);

    // Repeated text is still a distinct request, not the old answer's owner.
    run_request(&mut state, 13, "First question");
    assert_eq!(state.response_request(), ("First question", true));
    press(&mut state, KeyCode::Home, true);
    for (item_id, text, complete) in [
        ("second", "Partial", false),
        ("second", "Second answer", true),
        ("third", "Another observation", true),
    ] {
        state.event(Event::Item {
            generation: 13,
            task_id: 7,
            thread_id: "thread".into(),
            turn_id: "second-turn".into(),
            item_id: item_id.into(),
            kind: ItemKind::Message,
            text: text.into(),
            complete,
        });
    }
    assert_eq!(state.response_request(), ("First question", true));
    assert!(state.new_output && !state.following);
    assert!(Arc::ptr_eq(
        &state.items[1].request,
        &state.items[2].request
    ));
    assert!(Arc::ptr_eq(
        &state.items[1].request,
        &state.submitted_request
    ));
    press(&mut state, KeyCode::End, true);
    assert_eq!(state.response_request(), ("First question", false));
    reply_key(&mut state, KeyCode::Up);
    reply_key(&mut state, KeyCode::Up);
    assert_eq!(state.response_request(), ("First question", true));
    assert_eq!(state.items[state.selected].text, "First answer");
}

#[test]
fn request_ownership_follows_provider_role_sessions_and_retained_turn_replays() {
    let mut state = active();
    state.submitted_request = "Original request".into();
    streamed(&mut state, "first", "Original answer", true);
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Completed,
        message: "Complete".into(),
    });
    run_request(&mut state, 12, "New request");
    state.event(Event::Item {
        generation: 12,
        task_id: 7,
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: "replayed-item".into(),
        kind: ItemKind::Message,
        text: "Retained turn observation".into(),
        complete: true,
    });
    assert_eq!(state.response_request(), ("Original request", true));
    state.event(Event::Finished {
        generation: 12,
        task_id: 7,
        outcome: Outcome::Completed,
        message: "Complete".into(),
    });
    state.set_operation(Operation::Review);
    assert_eq!(state.response_request(), ("", false));
    state.set_operation(Operation::Plan);
    assert_eq!(state.response_request(), ("Original request", true));
    assert_eq!(state.submitted_request.as_ref(), "New request");
    state.switch_to = Some(Agent::Gemini);
    state.complete_switch();
    assert_eq!(state.response_request(), ("", false));
    state.switch_to = Some(Agent::Codex);
    state.complete_switch();
    assert_eq!(state.response_request(), ("Original request", true));
    state.invalidate_authentication();
    assert_eq!(state.response_request(), ("", false));
}

#[test]
fn partial_response_follows_each_snapshot_until_explicit_scroll_and_end() {
    let mut state = active();
    streamed(
        &mut state,
        "first",
        "one\ntwo\nthree\nfour\nfive\nsix\nseven",
        false,
    );
    assert_eq!(
        state.response_rows(40, 2, false).collect::<Vec<_>>(),
        ["six", "seven"]
    );
    assert_eq!(state.activity_label(), Some("Receiving response"));
    press(&mut state, KeyCode::PageUp, true);
    assert!(!state.following);
    let held = state.scroll;
    streamed(
        &mut state,
        "first",
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight",
        false,
    );
    assert!(state.new_output);
    assert_eq!(
        state.response_rows(40, 2, false).collect::<Vec<_>>(),
        ["five", "six"]
    );
    assert_eq!(state.scroll, held);
    press(&mut state, KeyCode::PageDown, true);
    assert!(
        !state.following,
        "reaching the end manually must not resume follow"
    );
    press(&mut state, KeyCode::End, true);
    assert!(state.following && !state.new_output);
    assert_eq!(
        state.response_rows(40, 2, false).collect::<Vec<_>>(),
        ["seven", "eight"]
    );
    streamed(
        &mut state,
        "first",
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine",
        true,
    );
    assert_eq!(
        state.response_rows(40, 2, false).collect::<Vec<_>>(),
        ["eight", "nine"]
    );
}

#[test]
fn newest_response_follows_but_older_selection_stays_until_end() {
    let mut state = active();
    streamed(&mut state, "first", "first response", true);
    streamed(&mut state, "second", "second response", false);
    assert_eq!(state.selected, 1);
    reply_key(&mut state, KeyCode::Up);
    streamed(&mut state, "second", "second response grows", false);
    streamed(&mut state, "third", "third response", false);
    assert_eq!(state.items[state.selected].item_id, "first");
    assert!(state.new_output && !state.following);
    press(&mut state, KeyCode::End, true);
    assert_eq!(state.items[state.selected].item_id, "third");
    assert_eq!(
        state.response_rows(40, 1, false).collect::<Vec<_>>(),
        ["third response"]
    );
}

#[test]
fn output_does_not_move_prompt_question_or_journal_review() {
    let mut state = active();
    streamed(&mut state, "first", "one\ntwo\nthree\nfour", false);
    state.response_rows(40, 1, false).count();
    let response_scroll = state.scroll;
    for kind in [
        PromptKind::Approval {
            command: "echo".into(),
            cwd: ".".into(),
            reason: "test".into(),
            allow_once: true,
            deny_decision: "decline".into(),
        },
        PromptKind::Questions(vec![Question {
            id: "q".into(),
            header: "Question".into(),
            question: "Which?".into(),
            options: vec![],
            other: true,
            secret: false,
        }]),
    ] {
        state.event(Event::Prompt {
            generation: 11,
            task_id: 7,
            request: Prompt {
                arrival: crate::providers::approval_arrival(),
                request_id: "request".into(),
                thread_id: "thread".into(),
                turn_id: "turn".into(),
                item_id: "first".into(),
                kind,
            },
        });
        state.scroll_max = 8;
        state.scroll = 2;
        let selected = state.selected;
        streamed(&mut state, "second", "response behind request", false);
        assert_eq!(state.selected, selected);
        assert_eq!(state.scroll, 2);
        assert!(!state.reviewed && !state.affirmative);
        assert_eq!(state.activity_label(), None);
        press(&mut state, KeyCode::End, true);
        assert_eq!(state.scroll, 8);
        assert_eq!(state.selected, selected);
        state.event(Event::PromptClosed {
            generation: 11,
            request_id: "request".into(),
        });
        assert_eq!(state.scroll, response_scroll);
    }
    state.journal_mode = true;
    state.scroll = 3;
    state.scroll_max = 9;
    let selected = state.selected;
    streamed(&mut state, "third", "journal stays open", false);
    press(&mut state, KeyCode::End, true);
    assert!(state.journal_mode);
    assert_eq!(state.selected, selected);
    assert_eq!(state.scroll, 9);
}

#[test]
fn terminal_outcomes_stop_activity_and_preserve_partial_scroll() {
    for outcome in [
        Outcome::Completed,
        Outcome::Cancelled,
        Outcome::Failed,
        Outcome::Unknown,
    ] {
        let mut state = active();
        streamed(&mut state, "first", "one\ntwo\nthree\nfour", false);
        state.response_rows(40, 1, false).count();
        press(&mut state, KeyCode::Home, true);
        state.event(Event::Finished {
            generation: 11,
            task_id: 7,
            outcome,
            message: "terminal".into(),
        });
        assert_eq!(state.activity_label(), None);
        assert!(!state.active && !state.cancelling);
        assert_eq!(
            state.response_rows(40, 1, false).collect::<Vec<_>>(),
            ["one"]
        );
        assert_eq!(state.scroll, 0);
        assert!(
            !state.items[0].complete,
            "completion must not fabricate a complete item"
        );
        streamed(&mut state, "late", "ignored", false);
        assert_eq!(state.items.len(), 1);
    }
}

#[test]
fn activity_excludes_local_review_and_reports_cancellation_until_acknowledged() {
    let mut state = State::new();
    assert_eq!(state.activity_label(), None);
    command(&mut state, "new", true);
    assert_eq!(state.activity_label(), Some("Preparing request"));
    let mut state = preview();
    assert_eq!(state.activity_label(), None);
    state.scroll_max = 20;
    press(&mut state, KeyCode::End, true);
    assert!(!state.affirmative && state.selected == 0);
    let mut state = active();
    assert_eq!(state.activity_label(), Some("Connecting"));
    state.request_quit();
    assert_eq!(state.activity_label(), None);
    press(&mut state, KeyCode::Esc, true);
    state.cancel();
    assert_eq!(state.activity_label(), Some("Cancelling"));
}

#[test]
fn bounded_output_keeps_the_manually_selected_item_and_scroll() {
    let mut state = active();
    state.submitted_request = "Bounded request".into();
    streamed(&mut state, "pinned", "one\ntwo\nthree", true);
    state.response_rows(40, 1, false).count();
    press(&mut state, KeyCode::Home, true);
    assert!(state.scroll_response_lines(1));
    for index in 0..ITEM_LIMIT + 3 {
        streamed(&mut state, &format!("item-{index}"), "new", false);
    }
    assert_eq!(state.items.len(), ITEM_LIMIT);
    assert_eq!(state.items[state.selected].item_id, "pinned");
    assert_eq!(
        state.response_rows(40, 1, false).collect::<Vec<_>>(),
        ["two"]
    );
    assert_eq!(state.omitted, 4);
    assert_eq!(Arc::strong_count(&state.submitted_request), ITEM_LIMIT + 1);
}

#[test]
fn response_layout_reflows_unicode_without_copying_or_splitting_graphemes() {
    let mut state = active();
    streamed(
        &mut state,
        "first",
        "界界界\ne\u{301}e\u{301}e\u{301}",
        false,
    );
    press(&mut state, KeyCode::Home, true);
    assert_eq!(
        state.response_rows(4, 8, false).collect::<Vec<_>>(),
        ["界界", "界", "e\u{301}e\u{301}e\u{301}"]
    );
    assert_eq!(
        state.response_rows(2, 8, false).collect::<Vec<_>>(),
        ["界", "界", "界", "e\u{301}e\u{301}", "e\u{301}"]
    );
    // A same-sized replacement must invalidate the cached byte ranges too.
    streamed(&mut state, "first", "aa bb\ncc dd", false);
    assert_eq!(
        state.response_rows(4, 8, false).collect::<Vec<_>>(),
        ["aa ", "bb", "cc ", "dd"]
    );
}

#[test]
fn tool_observations_share_the_transcript_and_hidden_cache_stays_fresh() {
    let mut state = active();
    streamed(&mut state, "first", "first response", true);
    state.response_rows(40, 2, false).count();
    state.event(Event::Item {
        generation: 11,
        task_id: 7,
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: "tool".into(),
        kind: ItemKind::Tool,
        text: "tool observation".into(),
        complete: false,
    });
    assert_eq!(state.selected, 1);
    assert!(!state.new_output);
    // An explicit inspection jump pauses before an older item changes.
    reply_key(&mut state, KeyCode::Up);
    streamed(&mut state, "first", "replacement\nwith two rows", true);
    assert_eq!(
        state.response_rows(40, 2, false).collect::<Vec<_>>(),
        ["replacement", "with two rows"]
    );
}

#[test]
fn response_tail_and_manual_scrolling_are_not_limited_to_u16_rows() {
    let mut state = active();
    let text = format!("{}tail", "row\n".repeat(70_000));
    streamed(&mut state, "large", &text, false);
    assert_eq!(
        state.response_rows(40, 1, false).collect::<Vec<_>>(),
        ["tail"]
    );
    assert_eq!(state.response_scroll, 70_000);
    press(&mut state, KeyCode::PageUp, true);
    assert_eq!(state.response_scroll, 69_999);
    assert_eq!(
        state.response_rows(40, 1, false).collect::<Vec<_>>(),
        ["row"]
    );
    press(&mut state, KeyCode::End, true);
    assert_eq!(
        state.response_rows(40, 1, false).collect::<Vec<_>>(),
        ["tail"]
    );
    press(&mut state, KeyCode::Home, true);
    assert_eq!(state.response_scroll, 0);
}

#[test]
fn verbose_response_retains_provenance_without_stale_cached_content() {
    let mut state = active();
    streamed(&mut state, "source-item", "partial text", false);
    press(&mut state, KeyCode::Home, true);
    let details = state
        .response_rows(100, 30, true)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        details.contains("source-item") && details.contains("thread") && details.contains("turn")
    );
    assert!(details.ends_with("partial text"));
    assert_eq!(
        state.response_rows(100, 30, false).collect::<Vec<_>>(),
        ["partial text"]
    );
    streamed(&mut state, "source-item", "completed text", true);
    let details = state
        .response_rows(100, 30, true)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(details.ends_with("completed text") && !details.contains("partial text"));
}

#[test]
fn response_lines_and_viewport_pages_traverse_attributed_item_boundaries() {
    let mut state = active();
    state.submitted_request = "First input".into();
    streamed(&mut state, "older", "old response", true);
    state.submitted_request = "Second input".into();
    streamed(
        &mut state,
        "newer",
        "0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11",
        false,
    );
    state.response_rows(120, 8, false).count();
    let (_, _, total) = state.response_position();
    assert!(total > 13, "attribution participates in the same layout");
    assert!(state.scroll_response_lines(-10_000));
    let top = state
        .response_rows(120, 8, false)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert!(
        top.iter().any(|line| line == "old response") && top.iter().any(|line| line == "0"),
        "{top:?}"
    );
    assert!(top.iter().any(|line| line.contains("older")));
    assert!(top.iter().any(|line| line.contains("newer")));
    let start = state.response_position().0;
    press(&mut state, KeyCode::PageDown, true);
    state.response_rows(120, 8, false).count();
    assert_eq!(state.response_position().0, start + 7);
    assert_eq!(state.selected, 1);
    press(&mut state, KeyCode::PageUp, true);
    assert_eq!(state.response_rows(120, 8, false).collect::<Vec<_>>(), top);
    assert_eq!(state.selected, 0);
    assert!(!state.following, "manual traversal never resumes following");
    reply_key(&mut state, KeyCode::Down);
    assert_eq!(state.response_rows(120, 2, false).next(), Some("0"));
    reply_key(&mut state, KeyCode::Up);
    assert_eq!(
        state.response_rows(120, 2, false).next(),
        Some("old response")
    );
    press(&mut state, KeyCode::End, true);
    assert_eq!(state.response_rows(120, 1, false).next(), Some("11"));
    assert!(state.following && !state.new_output);
}

#[test]
fn paused_unicode_content_keeps_its_byte_anchor_across_reflow_and_appends() {
    let mut state = active();
    let initial = "前置\n界界e\u{301}界界e\u{301}界界e\u{301}\ntail\nlast";
    streamed(&mut state, "unicode", initial, false);
    state.response_rows(4, 1, false).count();
    state.home_response_key(KeyCode::Home);
    assert!(state.scroll_response_lines(3));
    let source = state.items[0].text.as_ptr();
    let anchored_byte = initial.find("界界e\u{301}").unwrap() + "界界e\u{301}界".len();
    let held = state.response_rows(4, 1, false).next().unwrap();
    assert_eq!(held, "界e\u{301}");
    assert_eq!(held.as_ptr(), source.wrapping_add(anchored_byte));
    // The wide row contains the same anchored grapheme even though its start
    // moves earlier. Repeated narrow/wide reflows must not drift toward row 0.
    assert_eq!(
        state.response_rows(20, 1, false).next(),
        Some("界界e\u{301}界界e\u{301}界界e\u{301}")
    );
    assert_eq!(state.response_rows(4, 1, false).next(), Some("界e\u{301}"));
    streamed(
        &mut state,
        "unicode",
        &format!("{initial}\nnew tail"),
        false,
    );
    assert!(state.new_output);
    assert_eq!(state.response_rows(4, 1, false).next(), Some("界e\u{301}"));
    assert!(!state.following);
    state.home_response_key(KeyCode::End);
    assert_eq!(state.response_rows(40, 1, false).next(), Some("new tail"));
    assert!(state.following && !state.new_output);
}

#[test]
fn paused_output_survives_retention_index_changes_and_tool_completion() {
    let mut state = active();
    streamed(&mut state, "discard", "discard", true);
    streamed(&mut state, "pinned", "zero\none\ntwo\nthree\nfour", false);
    state.response_rows(40, 2, false).count();
    assert!(state.scroll_response_lines(-2));
    assert_eq!(state.response_rows(40, 2, false).next(), Some("one"));
    for index in 0..ITEM_LIMIT {
        streamed(&mut state, &format!("later-{index}"), "later output", true);
    }
    assert_eq!(state.items[state.selected].item_id, "pinned");
    assert_eq!(state.response_rows(40, 2, false).next(), Some("one"));
    state.event(Event::Item {
        generation: 11,
        task_id: 7,
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: "tool".into(),
        kind: ItemKind::Tool,
        text: "tool completed".into(),
        complete: true,
    });
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Completed,
        message: "Complete".into(),
    });
    assert_eq!(state.items[state.selected].item_id, "pinned");
    assert_eq!(state.response_rows(40, 2, false).next(), Some("one"));
    assert!(!state.following && state.new_output);
}

#[test]
fn response_wheel_uses_only_the_rendered_body_and_never_grants_prompt_consent() {
    use ratatui::layout::Rect;

    let mut state = active();
    streamed(&mut state, "response", "0\n1\n2\n3\n4\n5\n6\n7", false);
    state.response_rows(20, 3, false).count();
    let body = Rect::new(5, 4, 20, 3);
    state.set_response_body(body);
    for (column, row) in [(4, 4), (25, 4), (5, 3), (5, 7)] {
        assert!(!state.response_wheel(column, row, -3));
    }
    assert!(state.following);
    assert!(state.response_wheel(24, 6, -3));
    assert_eq!(state.response_rows(20, 3, false).next(), Some("2"));
    assert_eq!(state.selected, 0);
    assert!(!state.following);
    assert!(state.response_wheel(5, 4, 100));
    assert_eq!(state.response_rows(20, 3, false).next(), Some("5"));
    assert!(!state.following, "manual bottom does not resume follow");

    state.event(Event::Prompt {
        generation: 11,
        task_id: 7,
        request: Prompt {
            arrival: crate::providers::approval_arrival(),
            request_id: "approval".into(),
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "response".into(),
            kind: PromptKind::Approval {
                command: "echo".into(),
                cwd: ".".into(),
                reason: "review".into(),
                allow_once: true,
                deny_decision: "decline".into(),
            },
        },
    });
    // Opening any review clears the registered response body, even though the
    // cached layout remains available behind it.
    state.set_response_body(Rect::default());
    assert!(!state.response_wheel(5, 4, -3));
    let selected = state.selected;
    press(&mut state, KeyCode::End, true);
    assert_eq!(state.selected, selected);
    assert!(state.decision.is_none());
    assert!(!state.affirmative && !state.reviewed);
    // Esc-deferred prompt: home is visible again, and may be scrolled without
    // dismissing or answering the still-owned approval.
    state.set_response_body(body);
    assert!(state.response_wheel(5, 4, -3));
    assert_eq!(state.response_rows(20, 3, false).next(), Some("2"));
    assert!(state.request.is_some() && state.decision.is_none());
    assert!(!state.affirmative && !state.reviewed);
    state.set_response_body(Rect::new(5, 4, 0, 3));
    assert!(!state.response_wheel(5, 4, 1));
}

#[test]
fn cancelling_connection_review_does_not_reopen_a_hidden_model_picker() {
    let mut state = State::new();
    command(&mut state, "models-refresh", true);
    state.pending.take();
    state.event(Event::Preview(Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: true,
        generation: 99,
        task_id: 7,
        thread_id: None,
        trust: Vec::new(),
        payload: "{}".into(),
        blockers: Vec::new(),
    }));
    assert!(state.preview.is_some() && state.picker.is_none() && state.pending.is_none());
    press(&mut state, KeyCode::Esc, true);
    assert!(matches!(
        state.pending.take(),
        Some(Request::Cancel { generation: 99 })
    ));
    state.event(Event::Finished {
        generation: 99,
        task_id: 7,
        outcome: Outcome::Cancelled,
        message: "Metadata preparation cancelled".into(),
    });
    assert!(state.picker.is_none() && !state.busy());
    assert!(state.pending.is_none());
}

fn partner(id: i64, name: &str, form: usize) -> store::Attribution {
    store::Attribution {
        partner: id,
        name: name.into(),
        form,
    }
}

#[test]
fn journal_records_carry_the_partner_bound_when_the_request_was_prepared() {
    let mut state = State::new();
    state.reset(7);
    state.front = Some(partner(1, "Bolt", 2));
    command(&mut state, "new", true);
    state.pending.take();
    state.event(Event::Preview(Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: true,
        generation: 11,
        task_id: 7,
        thread_id: None,
        trust: vec![],
        payload: "{}".into(),
        blockers: vec![],
    }));
    // A later lead change never re-attributes the prepared request.
    state.front = Some(partner(2, "Mizu", 1));
    state.event(Event::Journal {
        generation: 11,
        task_id: 7,
        record: store::ProviderRecord {
            id: "nonce-11-item".into(),
            agent: Agent::Codex,
            generation: 11,
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "item".into(),
            origin: "Codex provider-native".into(),
            coverage: "Public observed result only".into(),
            text: "output".into(),
            complete: true,
            partner: None,
        },
    });
    let Some(store::JournalRequest {
        operation: store::JournalOperation::Put(record),
        ..
    }) = state.pending_journal.back()
    else {
        panic!("journal record was not queued");
    };
    assert_eq!(record.partner, Some(partner(1, "Bolt", 2)));
}

fn journal_event(generation: u64, id: &str) -> Event {
    Event::Journal {
        generation,
        task_id: 7,
        record: store::ProviderRecord {
            id: id.into(),
            agent: Agent::Codex,
            generation,
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: id.into(),
            origin: "Codex provider-native".into(),
            coverage: "Public observed result only".into(),
            text: "output".into(),
            complete: true,
            partner: None,
        },
    }
}

fn prepared_at(state: &mut State, generation: u64) {
    state.pending.take();
    state.event(Event::Preview(Preview {
        arrival: crate::providers::approval_arrival(),
        destination: None,
        requires_connection_review: true,
        generation,
        task_id: 7,
        thread_id: None,
        trust: vec![],
        payload: "{}".into(),
        blockers: vec![],
    }));
}

#[test]
fn late_events_for_another_or_released_partner_are_refused() {
    let mut state = State::new();
    state.reset(7);
    state.front = Some(partner(1, "Bolt", 2));
    command(&mut state, "new", true);
    prepared_at(&mut state, 11);
    state.event(journal_event(11, "bolt-item"));
    assert_eq!(state.pending_journal.len(), 1);
    // Bolt's review closes; Mizu then owns the next request slot.
    state.preview = None;
    state.pending_journal.clear();
    state.front = Some(partner(2, "Mizu", 1));
    command(&mut state, "new", true);
    prepared_at(&mut state, 12);
    assert_eq!(state.owner().map(|owner| owner.partner), Some(2));
    state.event(journal_event(11, "late-bolt-item"));
    assert!(state.pending_journal.is_empty(), "late event reached Mizu");
    state.event(journal_event(12, "mizu-item"));
    assert_eq!(state.pending_journal.len(), 1);
    // Live work keeps its slot even if the roster changes underneath it.
    state.retain_partners(&[1]);
    assert_eq!(state.owner().map(|owner| owner.partner), Some(2));
    // Once idle, a released owner's slot is dropped and its events refused.
    state.preview = None;
    state.pending_journal.clear();
    state.retain_partners(&[1]);
    assert!(state.owner().is_none());
    assert!(state.notice.contains("left the party"));
    state.event(journal_event(12, "released-item"));
    assert!(state.pending_journal.is_empty());
}

#[test]
fn fresh_turn_dispatch_keeps_manually_selected_previous_response_until_explicit_latest() {
    let mut state = active();
    streamed(
        &mut state,
        "previous",
        "one\ntwo\nthree\nfour\nfive\nsix",
        true,
    );
    state.response_rows(40, 2, false).count();
    assert!(state.scroll_response_lines(-2));
    assert_eq!(
        state.response_rows(40, 2, false).collect::<Vec<_>>(),
        ["three", "four"]
    );
    state.event(Event::Finished {
        generation: 11,
        task_id: 7,
        outcome: Outcome::Completed,
        message: "Complete".into(),
    });
    run_request(&mut state, 12, "Next request");
    assert!(!state.following);
    assert_eq!(
        state.response_rows(40, 2, false).collect::<Vec<_>>(),
        ["three", "four"]
    );
    state.event(Event::Item {
        generation: 12,
        task_id: 7,
        thread_id: "next-thread".into(),
        turn_id: "next-turn".into(),
        item_id: "next-response".into(),
        kind: ItemKind::Message,
        text: "new response".into(),
        complete: true,
    });
    assert!(state.new_output);
    assert_eq!(state.items[state.selected].item_id, "previous");
    assert_eq!(
        state.response_rows(40, 2, false).collect::<Vec<_>>(),
        ["three", "four"]
    );
    reply_key(&mut state, KeyCode::End);
    assert_eq!(
        state.response_rows(40, 1, false).collect::<Vec<_>>(),
        ["new response"]
    );
}

#[test]
fn later_segment_anchor_survives_earlier_growth_new_segments_and_reflow() {
    let mut state = active();
    streamed(&mut state, "earlier", "earlier", false);
    streamed(
        &mut state,
        "reading",
        "start\n界界e\u{301}界界e\u{301}\nend\ntail",
        false,
    );
    state.response_rows(4, 1, false).count();
    assert!(state.scroll_response_lines(-3));
    let held = state.response_rows(4, 1, false).next().unwrap().to_owned();
    assert_eq!(state.items[state.selected].item_id, "reading");
    streamed(&mut state, "earlier", &"earlier grew\n".repeat(30), true);
    streamed(&mut state, "latest", "latest tail", false);
    assert_eq!(state.response_rows(4, 1, false).next(), Some(held.as_str()));
    state.response_rows(20, 1, false).count();
    assert_eq!(state.response_rows(4, 1, false).next(), Some(held.as_str()));
    assert_eq!(state.items[state.selected].item_id, "reading");
    assert!(state.new_output && !state.following);
    press(&mut state, KeyCode::End, true);
    assert_eq!(
        state.response_rows(120, 1, false).next(),
        Some("latest tail")
    );
}

#[test]
fn transcript_segments_retain_origin_owner_and_replayed_turn_request() {
    let mut state = active();
    let first = store::Attribution {
        partner: 1,
        name: "First".into(),
        form: 0,
    };
    let second = store::Attribution {
        partner: 2,
        name: "Second".into(),
        form: 3,
    };
    state.slots.insert(
        1,
        Slot {
            owner: first.clone(),
            task: 7,
            generation: Some(11),
        },
    );
    state.submitted_request = "Original literal request".into();
    streamed(&mut state, "original", "original answer", true);
    state.slots.clear();
    state.slots.insert(
        2,
        Slot {
            owner: second.clone(),
            task: 7,
            generation: Some(11),
        },
    );
    state.front = Some(second.clone());
    state.submitted_request = "New literal request".into();
    streamed(&mut state, "replayed", "original turn observation", true);
    state.event(Event::Item {
        generation: 11,
        task_id: 7,
        thread_id: "thread".into(),
        turn_id: "new-turn".into(),
        item_id: "new-item".into(),
        kind: ItemKind::Message,
        text: "new answer".into(),
        complete: true,
    });
    assert_eq!(state.items[0].owner.as_ref(), Some(&first));
    assert_eq!(state.items[1].owner.as_ref(), Some(&first));
    assert_eq!(state.items[2].owner.as_ref(), Some(&second));
    assert!(Arc::ptr_eq(
        &state.items[0].request,
        &state.items[1].request
    ));
    assert!(!Arc::ptr_eq(
        &state.items[0].request,
        &state.items[2].request
    ));
    state.response_rows(120, 20, false).count();
    press(&mut state, KeyCode::Home, true);
    let transcript = state
        .response_rows(120, 20, false)
        .collect::<Vec<_>>()
        .join("\n");
    for attribution in [
        "First",
        "Second",
        "Original literal request",
        "New literal request",
        "original",
        "replayed",
        "new-item",
    ] {
        assert!(transcript.contains(attribution), "{transcript}");
    }
}

#[test]
fn restored_session_reanchors_to_its_selected_response_instead_of_foreign_rows() {
    let item = |id: &str, text: &str| Item {
        thread_id: id.into(),
        turn_id: "turn".into(),
        item_id: id.into(),
        kind: ItemKind::Message,
        text: text.into(),
        complete: true,
        request: Arc::from(id),
        owner: None,
    };
    let mut state = State::new();
    state.reset(7);
    state.items = vec![item(
        "old",
        "old zero\nold one\nold two\nold three\nold four",
    )];
    state.response_rows(80, 1, false).count();
    press(&mut state, KeyCode::Home, true);
    for _ in 0..3 {
        press(&mut state, KeyCode::Down, true);
    }
    assert_eq!(state.response_rows(80, 1, false).next(), Some("old three"));
    state.sessions.insert(
        (state.agent, Operation::Implement),
        Session {
            items: vec![
                item(
                    "other",
                    "unrelated zero\nunrelated one\nunrelated two\nunrelated three",
                ),
                item("selected", "selected restored answer\nselected tail"),
            ],
            selected: 1,
            ..Session::default()
        },
    );
    state.set_operation(Operation::Implement);
    assert_eq!(
        state.response_rows(80, 1, false).next(),
        Some("selected restored answer")
    );
    assert_eq!(state.response_request().0, "selected");
}

#[test]
fn continued_assistant_after_tool_receipts_is_visible_without_moving_a_paused_reader() {
    for paused in [false, true] {
        let mut state = active();
        streamed(&mut state, "answer", "starting\nheld line", false);
        state.response_rows(80, 1, false).count();
        if paused {
            press(&mut state, KeyCode::Home, true);
        }
        state.event(Event::Item {
            generation: 11,
            task_id: 7,
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "tool".into(),
            kind: ItemKind::Tool,
            text: "actual tool receipt".into(),
            complete: true,
        });
        streamed(
            &mut state,
            "answer",
            "starting\nheld line\nfinal answer after tools",
            true,
        );
        assert_eq!(
            state.response_rows(80, 1, false).next(),
            Some(if paused {
                "starting"
            } else {
                "final answer after tools"
            })
        );
        assert_eq!(state.following, !paused);
    }
}

#[test]
fn one_row_follow_shows_completed_text_instead_of_a_synthetic_newline_row() {
    let mut state = active();
    streamed(&mut state, "answer", "visible final line\n", false);
    assert_eq!(
        state.response_rows(60, 1, false).next(),
        Some("visible final line")
    );
    streamed(&mut state, "answer", "visible final line\n\n", true);
    assert_eq!(state.response_rows(60, 1, false).next(), Some(""));
}

fn file_review_state() -> State {
    let mut state = active();
    state.event(Event::Prompt {
        generation: 11,
        task_id: 7,
        request: Prompt {
            arrival: crate::providers::approval_arrival(),
            request_id: json!("file-review"),
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "file".into(),
            kind: PromptKind::Permission {
                description: "Exact full content".into(),
                file: Some(crate::providers::FileReview {
                    path: "src/새.py".into(),
                    kind: crate::providers::FileReviewKind::Create,
                    directories: vec!["src".into()],
                    bytes: 12,
                    excerpt: "print('hi')".into(),
                    edit: false,
                    external: false,
                }),
                options: vec![
                    crate::providers::PermissionOption {
                        label: "Deny".into(),
                        description: String::new(),
                        answer: json!({"permission":"deny"}),
                    },
                    crate::providers::PermissionOption {
                        label: "Allow once".into(),
                        description: String::new(),
                        answer: json!({"permission":"once"}),
                    },
                ],
            },
        },
    });
    state
}

#[test]
fn neutral_file_review_enter_inspects_without_sending_the_initial_denial() {
    let mut state = file_review_state();
    state.reviewed = true;
    press(&mut state, KeyCode::Enter, true);
    press(&mut state, KeyCode::Enter, true);
    assert!(state.review_details && state.request.is_some());
    assert!(state.pending.is_none());
    // Explicit denial remains available from full details, without end-scrolling.
    press(&mut state, KeyCode::Char('n'), true);
    assert!(matches!(
        state.pending,
        Some(Request::Answer { request_id, answer, .. })
            if request_id == json!("file-review") && answer == json!({"permission":"deny"})
    ));
}

#[test]
fn file_allow_keys_require_a_visible_summary_and_a_deliberate_press() {
    for code in [KeyCode::Char('a'), KeyCode::Char('y')] {
        let mut state = file_review_state();
        press(&mut state, code, true);
        assert!(state.pending.is_none());
        state.reviewed = true;
        for key in [
            KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Repeat),
            KeyEvent::new(code, KeyModifiers::ALT),
        ] {
            state.key(key, true, &task(), &[], None);
            assert!(state.pending.is_none());
        }
        state.invalidate_review();
        press(&mut state, code, true);
        assert!(
            state.pending.is_none(),
            "invalidated scope authorized a write"
        );
        state.reviewed = true;
        press(&mut state, code, true);
        assert!(matches!(
            state.pending.as_ref(),
            Some(Request::Answer { request_id, answer, .. })
                if request_id == &json!("file-review") && answer == &json!({"permission":"once"})
        ));
        assert!(state.request.is_none());
    }
}

#[test]
fn automatic_file_buttons_need_deliberate_selection_and_rendered_scope() {
    for (choices, answer) in [
        (&[KeyCode::Left][..], "deny"),
        (&[KeyCode::Right][..], "once"),
        (&[KeyCode::Tab, KeyCode::Tab][..], "deny"),
    ] {
        let mut state = file_review_state();
        state.auto_permission_review = true;
        state.reviewed = true;
        for &choice in choices {
            press(&mut state, choice, true);
        }
        assert!(!state.auto_permission_review && state.pending.is_none());
        assert!(!state.reviewed);
        if answer == "once" {
            press(&mut state, KeyCode::Enter, true);
            assert!(
                state.pending.is_none(),
                "selection must render before approval"
            );
        }
        state.reviewed = true;
        state.key(
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat),
            true,
            &task(),
            &[],
            None,
        );
        assert!(state.pending.is_none());
        press(&mut state, KeyCode::Enter, true);
        assert!(matches!(
            state.pending.as_ref(),
            Some(Request::Answer { request_id, answer: actual, .. })
                if request_id == &json!("file-review") && actual == &json!({"permission":answer})
        ));
    }
}
