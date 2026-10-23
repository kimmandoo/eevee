//! Remembered routes only alter an idle local selection, never grant authority.
use super::{ModelSelection, State};
use crate::workspace::records::Pairing;

impl State {
    /// Call after the working-owner settlement and global-preference hydration.
    /// False leaves the current request/owner untouched and requires a later retry.
    pub fn preselect_pairing(&mut self, pairing: &Pairing) -> bool {
        if self.busy() || pairing.partner <= 0 || pairing.form >= crate::domain::FORMS.len() {
            return false;
        }
        if self
            .front
            .as_ref()
            .is_some_and(|owner| owner.partner != pairing.partner || owner.form != pairing.form)
        {
            return false;
        }
        if self.global_selection().is_none() {
            return false;
        }
        if self.agent == pairing.agent && self.selection == pairing.selection {
            self.remembered_pairing = Some((pairing.partner, pairing.form));
            self.pairing_offer = false;
            return true;
        }
        self.reset_pairing_authority();
        self.agent = pairing.agent;
        self.selection = pairing.selection.clone();
        self.remembered_pairing = Some((pairing.partner, pairing.form));
        self.notice = format!(
            "Remembered local pairing: {} / {} / effort {}. Availability is unverified; no connection, inspection, request or permission was granted.",
            pairing.agent.label(),
            pairing.selection.model.as_deref().unwrap_or("Default"),
            pairing.selection.effort.as_deref().unwrap_or("Default"),
        );
        true
    }

    /// On removal or a route with no pairing, restore the independent global
    /// default. This never selects another partner's pairing or a live fallback.
    pub fn clear_pairing(&mut self) -> bool {
        if self.remembered_pairing.is_none() {
            return true;
        }
        if self.busy() {
            return false;
        }
        let Some((agent, selection)) = self.global_selection() else {
            return false;
        };
        self.reset_pairing_authority();
        self.agent = agent;
        self.selection = selection;
        self.remembered_pairing = None;
        self.notice = "Remembered pairing removed from this local selection; global provider/model default restored. No connection or request was made.".into();
        true
    }

    pub fn remembered_pairing(&self) -> Option<(i64, usize)> {
        self.remembered_pairing
    }

    pub fn take_pairing_offer(&mut self) -> bool {
        if self.busy() {
            return false;
        }
        std::mem::take(&mut self.pairing_offer)
    }

    fn reset_pairing_authority(&mut self) {
        self.pairing_offer = false;
        self.clear_provider_sessions();
        self.close_request();
        self.generation = None;
        self.decision = None;
        self.inspection = None;
        self.inspect_models = false;
        self.inspect_sessions = false;
        self.auto_models = false;
        self.auto_picker = false;
        self.auto_run = false;
        self.auto_blocked = None;
        self.catalogue_pending = None;
        self.pending_model = None;
        self.model_picker_return = None;
        self.session_selection = ModelSelection::default();
        self.review_start();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::Agent;

    fn pairing(partner: i64, agent: Agent, model: &str) -> Pairing {
        Pairing {
            partner,
            form: 0,
            agent,
            selection: ModelSelection {
                model: Some(model.into()),
                effort: Some("high".into()),
            },
        }
    }

    #[test]
    fn two_partner_preselections_never_replace_global_default_or_authorize_io() {
        let mut state = State::new();
        state.preference_event(Ok(None));
        let default_agent = state.agent;
        let default_selection = state.selection.clone();
        let bolt = pairing(1, Agent::Codex, "exact/Bolt");
        let mizu = pairing(2, Agent::OpenCode, "exact/Mizu");
        assert!(state.preselect_pairing(&bolt));
        state.remember_used_selection();
        assert!(state.preselect_pairing(&mizu));
        assert_eq!(state.agent, Agent::OpenCode);
        assert_eq!(state.selection, mizu.selection);
        assert!(state.pending.is_none());
        assert!(state.work_pending.is_none());
        assert!(state.preview.is_none());
        assert!(state.thread_id.is_none());
        assert!(!state.preferences_pending());
        assert!(state.clear_pairing());
        assert_eq!(state.agent, default_agent);
        assert_eq!(state.selection, default_selection);
        assert!(state.remembered_pairing().is_none());
    }

    #[test]
    fn busy_owner_and_unhydrated_defaults_cannot_be_replaced() {
        let mut state = State::new();
        let saved = pairing(1, Agent::Codex, "unavailable/exact");
        assert!(!state.preselect_pairing(&saved));
        state.preference_event(Ok(None));
        state.active = true;
        assert!(!state.preselect_pairing(&saved));
        state.active = false;
        assert!(state.preselect_pairing(&saved));
        state.thread_id = Some("old-owned-session".into());
        state.clear_pairing();
        assert!(state.thread_id.is_none());
        assert!(state.pending.is_none());
    }

    #[test]
    fn late_global_hydration_survives_pairing_restore_and_removal() {
        let mut state = State::new();
        state.preference_event(Ok(None));
        let passive_agent = state.agent;
        let passive_selection = state.selection.clone();
        let latest = ModelSelection {
            model: Some("latest-global-model".into()),
            effort: None,
        };
        state.merge_preferences(&crate::store::ProviderPreferences {
            provider: Agent::Compatible,
            selections: [(Agent::Compatible, latest.clone())].into_iter().collect(),
            reduced_motion: false,
            text_density: crate::store::TextDensity::Compact,
        });
        assert_eq!(state.agent, passive_agent);
        assert_eq!(state.selection, passive_selection);

        let next = state.new_partner(
            crate::store::Attribution {
                partner: 2,
                name: "Owner2".into(),
                form: 0,
            },
            7,
        );
        assert_eq!(next.agent, Agent::Compatible);
        assert_eq!(next.selection, latest);

        assert!(state.preselect_pairing(&pairing(1, Agent::Codex, "remembered-model")));
        assert!(state.clear_pairing());
        assert_eq!(state.agent, Agent::Compatible);
        assert_eq!(state.selection, latest);
    }

    #[test]
    fn late_global_model_does_not_inherit_a_stale_same_provider_selection() {
        let mut state = State::new();
        let provider = Agent::Compatible;
        state.preference_event(Ok(Some(crate::store::ProviderPreferences {
            provider,
            reduced_motion: false,
            text_density: crate::store::TextDensity::Compact,
            selections: [(
                provider,
                ModelSelection {
                    model: Some("previous-global-model".into()),
                    effort: None,
                },
            )]
            .into_iter()
            .collect(),
        })));
        let latest = ModelSelection {
            model: Some("latest-global-model".into()),
            effort: None,
        };
        state.merge_preferences(&crate::store::ProviderPreferences {
            provider,
            reduced_motion: false,
            text_density: crate::store::TextDensity::Compact,
            selections: [(provider, latest.clone())].into_iter().collect(),
        });
        assert_eq!(
            state.selection.model.as_deref(),
            Some("previous-global-model")
        );
        let next = state.new_partner(
            crate::store::Attribution {
                partner: 2,
                name: "Owner2".into(),
                form: 0,
            },
            7,
        );
        assert_eq!(next.agent, provider);
        assert_eq!(next.selection, latest);
    }

    #[test]
    fn restart_preselection_and_removal_follow_latest_committed_global_default() {
        let bolt = pairing(1, Agent::Codex, "exact/Bolt");
        let mizu = pairing(2, Agent::OpenCode, "exact/Mizu");
        let durable = serde_json::to_string(&[bolt, mizu]).unwrap();
        let restored: Vec<Pairing> = serde_json::from_str(&durable).unwrap();
        let global = crate::store::ProviderPreferences {
            provider: Agent::OpenAI,
            selections: [(
                Agent::OpenAI,
                ModelSelection {
                    model: Some("global/default".into()),
                    effort: None,
                },
            )]
            .into_iter()
            .collect(),
            reduced_motion: false,
            text_density: crate::store::TextDensity::Compact,
        };
        let mut state = State::new();
        state.preference_event(Ok(Some(global.clone())));
        assert!(state.preselect_pairing(&restored[0]));
        assert!(state.preselect_pairing(&restored[1]));
        assert_eq!(state.selection.model.as_deref(), Some("exact/Mizu"));
        let newer = crate::store::ProviderPreferences {
            provider: Agent::Anthropic,
            selections: [(
                Agent::Anthropic,
                ModelSelection {
                    model: Some("global/new-default".into()),
                    effort: None,
                },
            )]
            .into_iter()
            .collect(),
            reduced_motion: false,
            text_density: crate::store::TextDensity::Compact,
        };
        state.merge_preferences(&newer);
        assert_eq!(state.selection.model.as_deref(), Some("exact/Mizu"));
        assert!(state.clear_pairing());
        assert_eq!(state.agent, Agent::Anthropic);
        assert_eq!(state.selection.model.as_deref(), Some("global/new-default"));
        assert!(state.pending.is_none());
    }

    #[test]
    fn explicit_selection_offers_but_automatic_preselection_never_does() {
        let mut state = State::new();
        state.preference_event(Ok(None));
        state.apply_selection(
            ModelSelection {
                model: Some("chosen/model".into()),
                effort: None,
            },
            false,
        );
        assert!(state.take_pairing_offer());
        assert!(!state.take_pairing_offer());
        assert!(state.preselect_pairing(&pairing(1, Agent::Codex, "remembered/model")));
        assert!(!state.take_pairing_offer());
    }

    #[test]
    fn saving_current_route_preserves_owned_session_native_history_and_handoff() {
        use crate::{
            providers::{
                ConversationMessage, ConversationOwner, ConversationRole, NativeConversation,
            },
            store::Attribution,
        };
        use std::sync::Arc;
        let mut state = State::new();
        state.preference_event(Ok(None));
        state.task = 7;
        state.front = Some(Attribution {
            partner: 1,
            name: "Bolt".into(),
            form: 0,
        });
        state.agent = Agent::ChatGPT;
        state.selection = ModelSelection {
            model: Some("same/model".into()),
            effort: Some("high".into()),
        };
        state.thread_id = Some("owned-session".into());
        state.submitted_request = "previous authored instruction".into();
        let history = Arc::new(NativeConversation {
            owner: ConversationOwner {
                task_id: 7,
                partner: 1,
                agent: state.agent,
                operation: state.operation,
                connection_scope: "unchanged-connection".into(),
            },
            messages: vec![
                ConversationMessage {
                    role: ConversationRole::User,
                    text: "previous authored instruction".into(),
                },
                ConversationMessage {
                    role: ConversationRole::Assistant,
                    text: "retained reply".into(),
                },
            ]
            .into(),
        });
        state
            .native_histories
            .insert((state.agent, state.operation), history.clone());
        let handoff = crate::workspace::handoff::Document {
            schema_version: crate::workspace::handoff::SCHEMA_VERSION,
            task: crate::workspace::handoff::Task {
                id: 7,
                revision: 1,
                goal: "Preserve current work".into(),
                next_action: "Review changes".into(),
                criteria: Vec::new(),
            },
            facts: Vec::new(),
            sources: Vec::new(),
            decisions: Vec::new(),
            unresolved: Vec::new(),
        };
        state.handoff = Some(handoff.clone());
        let saved = Pairing {
            partner: 1,
            form: 0,
            agent: state.agent,
            selection: state.selection.clone(),
        };
        assert!(state.preselect_pairing(&saved));
        assert_eq!(state.remembered_pairing(), Some((1, 0)));
        assert_eq!(state.thread_id.as_deref(), Some("owned-session"));
        assert_eq!(
            state.submitted_request.as_ref(),
            "previous authored instruction"
        );
        assert!(Arc::ptr_eq(
            &state.native_histories[&(state.agent, state.operation)],
            &history
        ));
        assert_eq!(state.handoff.as_ref(), Some(&handoff));
        assert!(state.pending.is_none());
        assert!(state.work_pending.is_none());
        assert!(!state.take_pairing_offer());
    }
}
