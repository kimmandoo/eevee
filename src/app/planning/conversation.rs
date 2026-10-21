//! Process-local native history is not reconstructed from display or durable observations.
use super::*;
use crate::providers::ConversationRole;

impl State {
    /// Invalidate only one route's future state. Submitted Inputs stay frozen;
    /// an old-account completion cannot seed a new connection's local chat.
    pub fn invalidate_connection(&mut self, agent: Agent) {
        self.catalogues.remove(&agent);
        self.catalogue_errors.remove(&agent);
        self.sessions.retain(|(route, _), _| *route != agent);
        self.native_histories
            .retain(|(route, _), _| *route != agent);
        if self.agent != agent {
            return;
        }
        self.native_candidate = None;
        self.picker = None;
        self.custom_model = None;
        self.selection_change = None;
        self.pending_model = None;
        self.model_picker_return = None;
        self.switch_to = None;
        self.catalogue_pending = None;
        self.picker_models.clear();
        self.auto_models = false;
        self.auto_picker = false;
        self.auto_run = false;
        self.auto_blocked = None;
        self.catalogue_dismissed = true;
        self.send_after_prepare = false;
        self.invalidate_review();
        self.thread_id = None;
        self.observed_model = None;
        if self.transport_pending() || self.held_input.is_some() {
            self.connection_invalidated = true;
            if self.held_input.is_some() {
                self.claim_cancelled = true;
                self.awaiting_preview = false;
            } else if !self.active {
                self.cancel();
            }
            self.notice = "Connection changed; new local chat. Submitted input remains frozen; stale preparations cannot run and old-account output cannot seed future chat. Nothing is replayed.".into();
        } else {
            self.notice = "Connection changed; new local chat. Cached models, provider sessions and consent revoked only for this route. No request sent or replayed.".into();
        }
    }

    pub(super) fn capture_native(&mut self, conversation: Arc<NativeConversation>) {
        let owner = &conversation.owner;
        if !self.agent.is_direct()
            || self.operation == Operation::Review
            || owner.task_id != self.task
            || owner.agent != self.agent
            || owner.operation != self.operation
            || self.native_request_partner != Some(owner.partner)
            || self
                .owner()
                .is_some_and(|slot| slot.partner != owner.partner)
        {
            return;
        }
        let prior = self.native_histories.get(&(self.agent, self.operation));
        let messages = conversation.messages.as_ref();
        let previous = prior.map_or(&[][..], |history| history.messages.as_ref());
        if messages.len() != previous.len() + 2
            || prior.is_some_and(|history| history.owner != *owner)
            || !messages
                .iter()
                .zip(previous)
                .all(|(next, old)| next.role == old.role && Arc::ptr_eq(&next.text, &old.text))
            || messages[messages.len() - 2].role != ConversationRole::User
            || messages[messages.len() - 2].text.as_ref() != self.submitted_instruction()
            || messages[messages.len() - 1].role != ConversationRole::Assistant
            || messages[messages.len() - 1].text.is_empty()
        {
            return;
        }
        self.native_candidate = Some(conversation);
    }

    pub(super) fn finish_native(&mut self, completed: bool) {
        let candidate = self.native_candidate.take();
        if completed
            && self.active
            && !self.cancelling
            && !self.authentication_invalidated
            && !self.connection_invalidated
            && let Some(conversation) = candidate
        {
            self.native_histories
                .insert((self.agent, self.operation), conversation);
        }
    }

    pub(super) fn reset_native_chat(&mut self) {
        // Resetting a reviewed-but-unsent snapshot retires it, never edits its
        // frozen history or turns its old consent into a fresh request.
        if self.preview.is_some() {
            self.cancel();
        }
        self.native_histories.remove(&(self.agent, self.operation));
        self.native_candidate = None;
        self.send_after_prepare = false;
        self.notice = "Native chat history reset locally for this task, partner, provider and role. No request sent; draft, displayed responses and durable journal unchanged. New history belongs to the current task/partner state and is cleared on task change or app restart.".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{ConversationMessage, ConversationOwner, Input};

    fn task(text: &str) -> store::Snapshot {
        let mut task = store::Snapshot::empty("project".into());
        task.id = 7;
        task.goal = "A saved task goal".into();
        task.partner_drafts
            .entry(task.lead_partner)
            .or_default()
            .drafts
            .push(store::Draft {
                field: store::EditorField::Request,
                text: text.into(),
                cursor: text.len(),
            });
        task
    }

    fn start(state: &mut State, task: &store::Snapshot, generation: u64) -> Box<Input> {
        assert!(state.command("new", true, task, &[], None));
        let Some(Request::Preview(input)) = state.pending.take() else {
            panic!("Native preparation must freeze an input");
        };
        state.event(Event::Preview(Preview {
            arrival: crate::providers::approval_arrival(),
            destination: None,
            requires_connection_review: false,
            generation,
            task_id: task.id,
            thread_id: None,
            trust: Vec::new(),
            payload: "exact frozen body".into(),
            blockers: Vec::new(),
        }));
        assert!(state.dispatch_prepared());
        assert!(
            matches!(state.pending.take(), Some(Request::Run { generation: value }) if value == generation)
        );
        input
    }

    fn conversation(input: &Input, assistant: &str) -> Arc<NativeConversation> {
        let mut messages = input
            .conversation
            .as_deref()
            .map_or_else(Vec::new, |history| history.messages.to_vec());
        messages.push(ConversationMessage {
            role: ConversationRole::User,
            text: input.native_instruction().into(),
        });
        messages.push(ConversationMessage {
            role: ConversationRole::Assistant,
            text: assistant.into(),
        });
        Arc::new(NativeConversation {
            owner: ConversationOwner {
                task_id: input.task.id,
                partner: input.task.lead_partner,
                agent: input.agent,
                operation: input.operation,
                connection_scope: "opaque-account-endpoint-a".into(),
            },
            messages: messages.into(),
        })
    }

    fn finish(state: &mut State, generation: u64, outcome: Outcome) {
        state.event(Event::Finished {
            generation,
            task_id: 7,
            outcome,
            message: "terminal".into(),
        });
    }

    fn native() -> State {
        let mut state = State::new();
        state.reset(7);
        state.agent = Agent::Compatible;
        state
    }

    #[test]
    fn native_followups_freeze_canonical_roles_independent_of_display_and_journal() {
        let mut state = native();
        let first = start(&mut state, &task("First raw fact"), 1);
        assert!(first.conversation.is_none());
        state.event(Event::Item {
            generation: 1,
            task_id: 7,
            thread_id: String::new(),
            turn_id: "display".into(),
            item_id: "display".into(),
            kind: ItemKind::Plan,
            text: "DISPLAY PREFIX / unrelated UI wording".into(),
            complete: true,
        });
        let canonical = conversation(&first, "First canonical answer");
        state.event(Event::NativeCompleted {
            generation: 1,
            task_id: 7,
            conversation: Arc::clone(&canonical),
        });
        assert!(
            state.native_histories.is_empty(),
            "a typed event is staged until successful settlement"
        );
        finish(&mut state, 1, Outcome::Completed);
        assert!(Arc::ptr_eq(
            &canonical,
            &state.native_histories[&(Agent::Compatible, Operation::Plan)]
        ));
        state.items.clear();
        let second = start(&mut state, &task("Second raw fact"), 2);
        let frozen = second.conversation.as_ref().unwrap();
        assert!(Arc::ptr_eq(frozen, &canonical));
        assert_eq!(frozen.messages[0].text.as_ref(), "First raw fact");
        assert_eq!(frozen.messages[1].text.as_ref(), "First canonical answer");
        state.event(Event::NativeCompleted {
            generation: 2,
            task_id: 7,
            conversation: conversation(&second, "Second canonical answer"),
        });
        finish(&mut state, 2, Outcome::Completed);
        assert_eq!(
            frozen.messages.len(),
            2,
            "later completion never mutates a frozen input"
        );
        let third = start(&mut state, &task("Recall both facts"), 3);
        let history = third.conversation.as_ref().unwrap();
        assert_eq!(history.messages.len(), 4);
        assert_eq!(history.messages[2].text.as_ref(), "Second raw fact");
        assert_eq!(history.messages[3].text.as_ref(), "Second canonical answer");
        assert!(
            state.pending_journal.is_empty(),
            "canonical history is not a durable schema or journal record"
        );
    }

    #[test]
    fn native_history_requires_successful_matching_task_partner_route_operation_and_generation() {
        for mismatch in 0..6 {
            let mut state = native();
            let input = start(&mut state, &task("Owned request"), 11);
            let mut candidate = (*conversation(&input, "Owned answer")).clone();
            let generation = if mismatch == 0 { 12 } else { 11 };
            let task_id = if mismatch == 1 { 8 } else { 7 };
            match mismatch {
                2 => candidate.owner.partner += 1,
                3 => candidate.owner.agent = Agent::OpenAI,
                4 => candidate.owner.operation = Operation::Implement,
                5 => candidate.owner.task_id += 1,
                _ => {}
            }
            state.event(Event::NativeCompleted {
                generation,
                task_id,
                conversation: Arc::new(candidate),
            });
            finish(&mut state, 11, Outcome::Completed);
            assert!(state.native_histories.is_empty());
        }
        for outcome in [Outcome::Failed, Outcome::Cancelled, Outcome::Unknown] {
            let mut state = native();
            let input = start(&mut state, &task("Owned request"), 11);
            state.event(Event::NativeCompleted {
                generation: 11,
                task_id: 7,
                conversation: conversation(&input, "Possibly completed output"),
            });
            finish(&mut state, 11, outcome);
            assert!(
                state.native_histories.is_empty(),
                "failed/interrupted output is never assistant history"
            );
        }
        let mut state = native();
        let input = start(&mut state, &task("Owned request"), 11);
        state.cancelling = true;
        state.event(Event::NativeCompleted {
            generation: 11,
            task_id: 7,
            conversation: conversation(&input, "Late answer"),
        });
        finish(&mut state, 11, Outcome::Completed);
        assert!(state.native_histories.is_empty());
    }

    #[test]
    fn terminal_native_unknown_releases_future_work_without_promoting_partial_history() {
        let mut state = native();
        let first = start(&mut state, &task("Completed fact"), 1);
        let canonical = conversation(&first, "Completed answer");
        state.event(Event::NativeCompleted {
            generation: 1,
            task_id: 7,
            conversation: Arc::clone(&canonical),
        });
        finish(&mut state, 1, Outcome::Completed);

        let interrupted = start(&mut state, &task("Interrupted request"), 2);
        state.event(Event::Item {
            generation: 2,
            task_id: 7,
            thread_id: String::new(),
            turn_id: "partial".into(),
            item_id: "partial".into(),
            kind: ItemKind::Plan,
            text: "Unverified partial answer".into(),
            complete: false,
        });
        state.cancelling = true;
        state.event(Event::NativeCompleted {
            generation: 2,
            task_id: 7,
            conversation: conversation(&interrupted, "Late unverified answer"),
        });
        finish(&mut state, 2, Outcome::Unknown);
        assert!(state.outcome_unknown());
        assert!(!state.unknown_effects() && !state.owns_work());
        assert!(state.items.iter().any(|item| !item.complete));
        assert!(Arc::ptr_eq(
            &state.native_histories[&(Agent::Compatible, Operation::Plan)],
            &canonical
        ));

        let followup = start(&mut state, &task("Explicit follow-up"), 3);
        assert!(Arc::ptr_eq(
            followup.conversation.as_ref().unwrap(),
            &canonical
        ));
        state.event(Event::NativeCompleted {
            generation: 3,
            task_id: 7,
            conversation: conversation(&followup, "Verified follow-up answer"),
        });
        finish(&mut state, 3, Outcome::Completed);
        assert!(
            state.outcome_unknown(),
            "later success cannot erase remote uncertainty"
        );
        assert!(!state.unknown_effects() && !state.owns_work());
        state.request_quit();
        assert!(
            state.quit_review,
            "remote uncertainty still requires informed quit"
        );
    }

    #[test]
    fn unknown_ownership_kind_follows_frozen_request_not_selected_provider() {
        for agent in [Agent::Compatible, Agent::Codex] {
            let mut state = State::new();
            state.reset(7);
            state.agent = agent;
            start(&mut state, &task("Frozen submitted request"), 1);
            // A local provider selection cannot reclassify submitted work.
            state.agent = if agent.is_direct() {
                Agent::Codex
            } else {
                Agent::Compatible
            };
            finish(&mut state, 1, Outcome::Unknown);
            assert!(state.outcome_unknown());
            assert_eq!(state.unknown_effects(), !agent.is_direct());
            assert_eq!(state.owns_work(), !agent.is_direct());
            state.switch_to = Some(Agent::Compatible);
            state.complete_switch();
            assert_eq!(state.unknown_effects(), !agent.is_direct());
            assert_eq!(state.owns_work(), !agent.is_direct());
            state.invalidate_authentication();
            assert!(state.outcome_unknown());
            assert_eq!(state.unknown_effects(), !agent.is_direct());
        }
    }

    #[test]
    fn native_local_uncertainty_is_not_remote_only_or_canonical_success() {
        for selected in [false, true] {
            for outcome in [Outcome::Completed, Outcome::Cancelled, Outcome::Unknown] {
                let mut state = native();
                state.set_operation(Operation::Implement);
                if selected {
                    state.edit_paths.push("missing/nested/new.rs".into());
                }
                let input = start(&mut state, &task("Create a new workspace target"), 1);
                assert_eq!(input.edit_paths.is_empty(), !selected);
                state.event(Event::LocalEffectsUnknown {
                    generation: 1,
                    task_id: 7,
                    message: "Filesystem publication outcome uncertain".into(),
                });
                state.event(Event::NativeCompleted {
                    generation: 1,
                    task_id: 7,
                    conversation: conversation(&input, "Unverified success claim"),
                });
                finish(&mut state, 1, outcome);
                assert!(state.unknown_effects() && state.outcome_unknown() && state.owns_work());
                assert!(
                    !state
                        .native_histories
                        .contains_key(&(Agent::Compatible, Operation::Implement))
                );
                state.switch_to = Some(Agent::Codex);
                state.complete_switch();
                state.invalidate_authentication();
                state.reset(8);
                assert_eq!(state.task_id(), 7);
                assert!(state.unknown_effects() && state.owns_work());
            }
        }
    }

    #[test]
    fn managed_review_and_new_partner_never_inherit_native_chat() {
        let mut state = native();
        let input = start(&mut state, &task("Planning request"), 1);
        state.event(Event::NativeCompleted {
            generation: 1,
            task_id: 7,
            conversation: conversation(&input, "Planning answer"),
        });
        finish(&mut state, 1, Outcome::Completed);
        let owner = store::Attribution {
            partner: 2,
            name: "Local display identity".into(),
            form: 0,
        };
        let other = state.new_partner(owner, 7);
        assert!(other.native_histories.is_empty());
        state.set_operation(Operation::Review);
        let review = start(&mut state, &task("Review independently"), 2);
        assert!(review.conversation.is_none());
        state.event(Event::NativeCompleted {
            generation: 2,
            task_id: 7,
            conversation: conversation(&review, "Independent review"),
        });
        finish(&mut state, 2, Outcome::Completed);
        assert!(
            !state
                .native_histories
                .contains_key(&(Agent::Compatible, Operation::Review))
        );
        state.set_operation(Operation::Plan);
        assert_eq!(
            state.native_histories[&(Agent::Compatible, Operation::Plan)]
                .messages
                .len(),
            2
        );
        let restarted = State::new();
        assert!(
            restarted.native_histories.is_empty(),
            "native chat is deliberately not restart resumable"
        );
    }

    #[test]
    fn chat_new_is_local_owner_scoped_and_cannot_mutate_a_live_input() {
        let mut state = native();
        let input = start(&mut state, &task("First request"), 1);
        let history = conversation(&input, "First answer");
        state.event(Event::NativeCompleted {
            generation: 1,
            task_id: 7,
            conversation: Arc::clone(&history),
        });
        finish(&mut state, 1, Outcome::Completed);
        state
            .native_histories
            .insert((Agent::OpenAI, Operation::Plan), Arc::clone(&history));
        state.native_histories.insert(
            (Agent::Compatible, Operation::Implement),
            Arc::clone(&history),
        );
        let next = start(&mut state, &task("Second request"), 2);
        state.command("chat-new", true, &task("Second request"), &[], None);
        assert!(
            state
                .native_histories
                .contains_key(&(Agent::Compatible, Operation::Plan))
        );
        assert!(Arc::ptr_eq(next.conversation.as_ref().unwrap(), &history));
        finish(&mut state, 2, Outcome::Failed);
        let item_count = state.items.len();
        assert!(state.command("chat-new", true, &task("Unsent draft"), &[], None));
        assert!(
            !state
                .native_histories
                .contains_key(&(Agent::Compatible, Operation::Plan))
        );
        assert!(
            state
                .native_histories
                .contains_key(&(Agent::Compatible, Operation::Implement))
        );
        assert!(
            state
                .native_histories
                .contains_key(&(Agent::OpenAI, Operation::Plan))
        );
        assert!(state.pending.is_none() && state.pending_journal.is_empty());
        assert_eq!(state.items.len(), item_count);
        assert_eq!(next.task.request_text(), "Second request");
        assert_eq!(next.conversation.as_ref().unwrap().messages.len(), 2);
    }

    #[test]
    fn explicit_connection_change_clears_only_that_route_without_replay_or_redundant_reset() {
        let mut state = native();
        let input = start(&mut state, &task("Original account request"), 1);
        let history = conversation(&input, "Original account answer");
        state.event(Event::NativeCompleted {
            generation: 1,
            task_id: 7,
            conversation: Arc::clone(&history),
        });
        finish(&mut state, 1, Outcome::Completed);
        state
            .native_histories
            .insert((Agent::OpenAI, Operation::Plan), Arc::clone(&history));
        state.invalidate_connection(Agent::OpenAI);
        assert!(
            state
                .native_histories
                .contains_key(&(Agent::Compatible, Operation::Plan))
        );
        assert!(
            !state
                .native_histories
                .contains_key(&(Agent::OpenAI, Operation::Plan))
        );
        let next = start(&mut state, &task("Frozen old account input"), 2);
        state.invalidate_connection(Agent::Compatible);
        assert!(state.active && !state.cancelling && state.pending.is_none());
        assert_eq!(next.task.request_text(), "Frozen old account input");
        assert!(Arc::ptr_eq(next.conversation.as_ref().unwrap(), &history));
        state.event(Event::NativeCompleted {
            generation: 2,
            task_id: 7,
            conversation: conversation(&next, "Late old account output"),
        });
        finish(&mut state, 2, Outcome::Completed);
        assert!(state.native_histories.is_empty());
        assert!(state.notice.contains("Connection changed; new local chat"));
        let fresh = start(&mut state, &task("Deliberate new account request"), 3);
        assert!(fresh.conversation.is_none());
        assert!(Arc::ptr_eq(next.conversation.as_ref().unwrap(), &history));
    }
    #[test]
    fn blocked_native_preview_exposes_actionable_reset_without_editing_or_sending_frozen_input() {
        let mut state = native();
        let first = start(&mut state, &task("Original fact"), 1);
        let retained = conversation(&first, "Original output");
        state.event(Event::NativeCompleted {
            generation: 1,
            task_id: 7,
            conversation: Arc::clone(&retained),
        });
        finish(&mut state, 1, Outcome::Completed);
        state.command("new", true, &task("Unsent follow-up"), &[], None);
        let Some(Request::Preview(frozen)) = state.pending.take() else {
            panic!("missing preview");
        };
        state.send_after_prepare = true;
        state.event(Event::Preview(Preview {
            arrival: crate::providers::approval_arrival(),
            destination: None,
            requires_connection_review: false,
            generation: 2,
            task_id: 7,
            thread_id: None,
            trust: Vec::new(),
            payload: String::new(),
            blockers: vec!["History budget reached; use /chat-new".into()],
        }));
        state.settle_background(true);
        assert!(state.preview.is_some() && state.pending.is_none());
        assert!(
            !state.cancelling,
            "blocked context remains inspectable until local reset"
        );
        assert!(
            state
                .command_choices()
                .iter()
                .any(|choice| choice.command == "chat-new" && choice.reason.is_none())
        );
        state.command("chat-new", true, &task("Unsent follow-up"), &[], None);
        assert!(state.native_histories.is_empty());
        assert!(state.preview.is_none() && state.cancelling);
        assert!(matches!(
            state.pending,
            Some(Request::Cancel { generation: 2 })
        ));
        assert!(Arc::ptr_eq(
            frozen.conversation.as_ref().unwrap(),
            &retained
        ));
        assert_eq!(frozen.task.request_text(), "Unsent follow-up");
    }
}
