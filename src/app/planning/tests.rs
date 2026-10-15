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

fn command(state: &mut State, name: &str, large: bool) {
    state.command(name, large, &task(), &[], None);
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
    assert_eq!(state.catalogues[&Agent::Codex].models.len(), 1);
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
fn model_change_offers_keep_fresh_and_back_without_silent_resume() {
    let mut state = State::new();
    state.reset(7);
    state.thread_id = Some("owned-session".into());
    state.catalogues.insert(Agent::Codex, catalogue());
    command(&mut state, "model", true);
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    assert!(state.selection_change.is_some());
    assert_eq!(state.selection, ModelSelection::default());
    press(&mut state, KeyCode::Esc, true);
    assert!(state.picker.is_some() && state.selection_change.is_none());
    press(&mut state, KeyCode::Enter, true);
    press(&mut state, KeyCode::Enter, true);
    assert_eq!(state.thread_id.as_deref(), Some("owned-session"));
    assert_eq!(state.selection, ModelSelection::default());
    command(&mut state, "model", true);
    press(&mut state, KeyCode::Down, true);
    press(&mut state, KeyCode::Enter, true);
    press(&mut state, KeyCode::Tab, true);
    press(&mut state, KeyCode::Enter, true);
    assert!(state.thread_id.is_none() && state.pending.is_none());
    assert_eq!(state.selection.model.as_deref(), Some("advertised-model"));
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
    // Choosing a model continues straight to its advertised efforts.
    assert!(state.picker == Some(Picker::Effort));
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

fn preview() -> State {
    let mut state = State::new();
    command(&mut state, "new", true);
    assert!(matches!(state.pending.take(), Some(Request::Preview(_))));
    state.event(Event::Preview(Preview {
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
            request_id: json!(1),
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "item".into(),
            kind: PromptKind::Permission {
                description: "Review exact scope".into(),
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
