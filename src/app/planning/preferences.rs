//! Durable preferences are separate from request/session ownership and consent.
use super::{Agent, State, store};

#[derive(Default)]
pub(super) struct Preferences {
    pub pending: Option<store::PreferenceRequest>,
    inflight: bool,
    loaded: bool,
    restore_required: bool,
    remembered_selection: Option<(Agent, super::ModelSelection)>,
    dirty: bool,
    explicit_provider: bool,
    provider_changed: bool,
    error: Option<String>,
    quit_waiting: bool,
    dirty_routes: std::collections::HashSet<Agent>,
}

impl State {
    pub fn set_startup_provider(&mut self, provider: Agent, explicit: bool) {
        self.agent = provider;
        self.preferences.explicit_provider = explicit;
        self.preferences.restore_required = true;
    }

    pub(super) fn preference_changed(&mut self) {
        self.preferences.provider_changed = true;
        self.selections.insert(self.agent, self.selection.clone());
        self.preferences.remembered_selection = Some((self.agent, self.selection.clone()));
        self.preferences.dirty = true;
        self.preferences.dirty_routes.insert(self.agent);
        self.preferences.error = None;
        // A queued, not-yet-submitted save must contain the latest choice.
        self.preferences.pending = None;
    }

    /// Records the selected route actually used for content, not metadata inspection.
    /// Repeated turns do not write unchanged preferences or silently retry a failed save.
    pub fn remember_used_selection(&mut self) {
        if self
            .preferences
            .remembered_selection
            .as_ref()
            .is_some_and(|(provider, selection)| {
                *provider == self.agent && selection == &self.selection
            })
        {
            return;
        }
        self.preferences.remembered_selection = Some((self.agent, self.selection.clone()));
        self.preferences.provider_changed = true;
        self.selections.insert(self.agent, self.selection.clone());
        self.preferences.dirty = true;
        self.preferences.dirty_routes.insert(self.agent);
        self.preferences.pending = None;
    }

    fn preference_snapshot(&self) -> store::ProviderPreferences {
        store::ProviderPreferences {
            provider: self.agent,
            selections: self.selections.clone(),
        }
    }

    pub fn preference_request(&mut self) -> &mut Option<store::PreferenceRequest> {
        if !self.preferences.inflight
            && self.preferences.error.is_none()
            && self.preferences.pending.is_none()
        {
            if !self.preferences.loaded {
                self.preferences.restore_required = true;
                self.preferences.pending = Some(store::PreferenceRequest::Load);
            } else if self.preferences.dirty {
                self.preferences.pending =
                    Some(store::PreferenceRequest::Save(self.preference_snapshot()));
            }
        }
        &mut self.preferences.pending
    }

    pub fn preference_started(&mut self) {
        self.preferences.inflight = true;
    }

    pub fn merge_preferences(&mut self, saved: &store::ProviderPreferences) {
        for (agent, selection) in &saved.selections {
            if !self.preferences.dirty_routes.contains(agent) {
                self.selections.insert(*agent, selection.clone());
            }
        }
        if matches!(
            self.preferences.pending,
            Some(store::PreferenceRequest::Save(_))
        ) {
            self.preferences.pending =
                Some(store::PreferenceRequest::Save(self.preference_snapshot()));
        }
    }
    pub fn retry_preferences(&mut self) {
        self.preferences.error = None;
    }

    pub fn preference_event(&mut self, result: Result<Option<store::ProviderPreferences>, String>) {
        self.preferences.inflight = false;
        match result {
            Err(error) => {
                self.notice = format!(
                    "Provider/model preference was NOT saved/restored: {error}. Your current selection and request draft remain in memory. /storage-retry retries; choose a different model if the ID is invalid."
                );
                self.preferences.error = Some(self.notice.clone());
                self.preferences.quit_waiting = false;
            }
            Ok(saved) if !self.preferences.loaded => {
                if let Some(mut saved) = saved {
                    if self.preferences.remembered_selection.is_none() {
                        self.preferences.remembered_selection = Some((
                            saved.provider,
                            saved
                                .selections
                                .get(&saved.provider)
                                .cloned()
                                .unwrap_or_default(),
                        ));
                    }
                    // Choices made while loading win only their own provider rows.
                    saved
                        .selections
                        .extend(std::mem::take(&mut self.selections));
                    self.selections = saved.selections;
                    if !self.preferences.explicit_provider && !self.preferences.provider_changed {
                        self.agent = saved.provider;
                    }
                    self.selection = self
                        .selections
                        .get(&self.agent)
                        .cloned()
                        .unwrap_or_default();
                }
                self.preferences.loaded = true;
            }
            Ok(saved) => {
                self.preferences.dirty_routes.retain(|agent| {
                    saved.as_ref().is_none_or(|saved| {
                        saved.selections.get(agent) != self.selections.get(agent)
                    })
                });
                self.preferences.dirty = !saved.as_ref().is_some_and(|saved| {
                    saved.provider == self.agent && saved.selections == self.selections
                });
            }
        }
    }
    pub fn preference_error(&self) -> Option<&str> {
        self.preferences.error.as_deref()
    }

    /// A failed initial restore must never launch the passive fallback route.
    pub fn preference_restore_error(&self) -> Option<&str> {
        if self.preferences.loaded {
            None
        } else {
            self.preferences.error.as_deref()
        }
    }

    pub fn preferences_loading(&self) -> bool {
        !self.preferences.loaded
            && self.preferences.error.is_none()
            && (self.preferences.restore_required
                || self.preferences.inflight
                || self.preferences.pending.is_some())
    }

    pub fn preferences_pending(&self) -> bool {
        self.preferences.dirty || self.preferences.inflight || self.preferences.pending.is_some()
    }

    /// A quit waits for the actual commit acknowledgement, never just queue acceptance.
    pub fn preferences_allow_quit(&mut self, requested: bool) -> bool {
        if !requested && !self.preferences.quit_waiting {
            return false;
        }
        if !self.preferences_pending() {
            self.preferences.quit_waiting = false;
            return true;
        }
        if let Some(error) = &self.preferences.error {
            self.notice.clone_from(error);
            self.preferences.quit_waiting = false;
        } else {
            self.preferences.quit_waiting = true;
            self.notice = "Saving the last provider/model preference before quitting…".into();
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ModelSelection;

    fn saved() -> store::ProviderPreferences {
        store::ProviderPreferences {
            provider: Agent::ChatGPT,
            selections: [
                (
                    Agent::Codex,
                    ModelSelection {
                        model: Some("cli/Exact-Unlisted".into()),
                        effort: Some("high".into()),
                    },
                ),
                (
                    Agent::ChatGPT,
                    ModelSelection {
                        model: Some("native/Exact-Unlisted".into()),
                        effort: Some("medium".into()),
                    },
                ),
            ]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn preparation_cannot_capture_a_fallback_route_before_hydration() {
        let mut state = State::new();
        state.set_startup_provider(Agent::Codex, false);
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.goal = "send with the remembered model".into();
        state.command("new", true, &task, &[], None);
        assert!(state.pending.is_none());
        state.preference_event(Ok(Some(saved())));
        state.command("new", true, &task, &[], None);
        let Some(crate::providers::Request::Preview(input)) = state.pending.take() else {
            panic!("hydrated request was not prepared");
        };
        assert_eq!(input.agent, Agent::ChatGPT);
        assert_eq!(input.selection, saved().selections[&Agent::ChatGPT]);
    }

    fn submit(state: &mut State) -> store::PreferenceRequest {
        let request = state
            .preference_request()
            .take()
            .expect("pending preference");
        state.preference_started();
        request
    }

    #[test]
    fn restart_restores_exact_custom_ids_and_effort_without_catalogues_and_new_tasks_keep_them() {
        let mut state = State::new();
        assert!(matches!(submit(&mut state), store::PreferenceRequest::Load));
        state.preference_event(Ok(Some(saved())));
        assert_eq!(state.agent, Agent::ChatGPT);
        assert_eq!(state.selection, saved().selections[&Agent::ChatGPT]);
        assert!(state.catalogues.is_empty() && state.pending.is_none());
        state.reset(37);
        assert_eq!(state.agent, Agent::ChatGPT);
        assert_eq!(state.selection, saved().selections[&Agent::ChatGPT]);
        state.switch_to = Some(Agent::Codex);
        state.complete_switch();
        assert_eq!(state.selection, saved().selections[&Agent::Codex]);
        state.switch_to = Some(Agent::ChatGPT);
        state.complete_switch();
        assert_eq!(state.selection, saved().selections[&Agent::ChatGPT]);
    }

    #[test]
    fn explicit_startup_provider_wins_and_restores_only_its_own_model_and_effort() {
        let mut state = State::new();
        state.set_startup_provider(Agent::Codex, true);
        submit(&mut state);
        state.preference_event(Ok(Some(saved())));
        assert_eq!(state.agent, Agent::Codex);
        assert_eq!(state.selection, saved().selections[&Agent::Codex]);
        assert!(
            state.preference_request().is_none(),
            "startup override is not a deliberate saved choice"
        );
    }

    #[test]
    fn deliberate_choice_during_load_wins_without_losing_other_provider_preferences() {
        let mut state = State::new();
        submit(&mut state);
        let chosen = ModelSelection {
            model: Some("new/Exact".into()),
            effort: None,
        };
        state.apply_selection(chosen.clone(), false);
        state.preference_event(Ok(Some(saved())));
        assert_eq!(state.agent, Agent::Codex);
        assert_eq!(state.selection, chosen);
        let store::PreferenceRequest::Save(snapshot) = submit(&mut state) else {
            panic!("expected save");
        };
        assert_eq!(
            snapshot.selections[&Agent::ChatGPT],
            saved().selections[&Agent::ChatGPT]
        );
        assert_eq!(snapshot.selections[&Agent::Codex], chosen);
    }

    #[test]
    fn failed_save_retains_selection_and_draft_and_retry_waits_for_latest_commit_before_quit() {
        let mut app = crate::app::App::new("/workspace".into(), 120, 36);
        app.input = "unsaved request draft".into();
        app.cursor = app.input.len();
        submit(&mut app.planning);
        app.store_event(store::Event::Preferences(Ok(None)));
        let chosen = ModelSelection {
            model: Some("exact".into()),
            effort: Some("high".into()),
        };
        app.planning.apply_selection(chosen.clone(), false);
        assert!(!app.planning.preferences_allow_quit(true));
        submit(&mut app.planning);
        app.store_event(store::Event::Preferences(Err("disk full".into())));
        assert_eq!(app.input, "unsaved request draft");
        assert_eq!(app.cursor, app.input.len());
        assert_eq!(app.planning.selection, chosen);
        assert!(app.notice.contains("NOT saved/restored") && app.notice.contains("/storage-retry"));
        assert!(app.planning.preference_request().is_none());
        assert!(!app.planning.preferences_allow_quit(true));
        app.planning.retry_preferences();
        let store::PreferenceRequest::Save(first) = submit(&mut app.planning) else {
            panic!("expected save");
        };
        app.planning.apply_selection(
            ModelSelection {
                model: Some("newest".into()),
                effort: None,
            },
            false,
        );
        app.store_event(store::Event::Preferences(Ok(Some(first))));
        assert!(!app.planning.preferences_allow_quit(true));
        let store::PreferenceRequest::Save(latest) = submit(&mut app.planning) else {
            panic!("expected save");
        };
        app.store_event(store::Event::Preferences(Ok(Some(latest))));
        assert!(app.planning.preferences_allow_quit(false));
        assert_eq!(app.input, "unsaved request draft");
    }

    #[test]
    fn failed_load_retry_retains_new_choice_and_merges_unrelated_provider_rows() {
        let mut state = State::new();
        submit(&mut state);
        assert!(state.preferences_loading());
        state.preference_event(Err("writer unavailable".into()));
        assert!(!state.preferences_loading());
        assert!(state.preference_error().is_some());
        assert!(state.preference_request().is_none());
        let chosen = ModelSelection {
            model: Some("offline/Exact".into()),
            effort: Some("high".into()),
        };
        state.apply_selection(chosen.clone(), false);
        state.retry_preferences();
        assert!(matches!(submit(&mut state), store::PreferenceRequest::Load));
        state.preference_event(Ok(Some(saved())));
        assert_eq!(state.selection, chosen);
        assert_eq!(state.agent, Agent::Codex);
        let store::PreferenceRequest::Save(snapshot) = submit(&mut state) else {
            panic!("expected save");
        };
        assert_eq!(snapshot.selections[&Agent::Codex], chosen);
        assert_eq!(
            snapshot.selections[&Agent::ChatGPT],
            saved().selections[&Agent::ChatGPT]
        );
    }

    #[test]
    fn first_use_remembers_passive_default_route_once_and_survives_new_tasks() {
        let mut state = State::new();
        state.set_startup_provider(Agent::OpenAI, false);
        submit(&mut state);
        state.preference_event(Ok(None));
        assert!(state.preference_request().is_none());
        state.remember_used_selection();
        let store::PreferenceRequest::Save(snapshot) = submit(&mut state) else {
            panic!("expected used route save");
        };
        assert_eq!(snapshot.provider, Agent::OpenAI);
        assert_eq!(
            snapshot.selections[&Agent::OpenAI],
            ModelSelection::default()
        );
        state.remember_used_selection();
        assert!(
            state.preference_request().is_none(),
            "another turn must not replace an in-flight save"
        );
        state.preference_event(Ok(Some(snapshot.clone())));
        state.remember_used_selection();
        assert!(state.preference_request().is_none());
        state.reset(37);
        state.remember_used_selection();
        assert!(state.preference_request().is_none());
        let mut restarted = State::new();
        restarted.set_startup_provider(Agent::Anthropic, false);
        submit(&mut restarted);
        restarted.preference_event(Ok(Some(snapshot)));
        assert_eq!(
            restarted.agent,
            Agent::OpenAI,
            "newly configured route must not replace last used route"
        );
    }

    #[test]
    fn unchanged_hydrated_identity_needs_no_write_but_first_used_startup_override_does() {
        let mut restored = State::new();
        submit(&mut restored);
        restored.preference_event(Ok(Some(saved())));
        restored.remember_used_selection();
        assert!(restored.preference_request().is_none());
        let mut overridden = State::new();
        overridden.set_startup_provider(Agent::Codex, true);
        submit(&mut overridden);
        overridden.preference_event(Ok(Some(saved())));
        assert!(
            overridden.preference_request().is_none(),
            "startup alone does not save an override"
        );
        overridden.remember_used_selection();
        let store::PreferenceRequest::Save(snapshot) = submit(&mut overridden) else {
            panic!("expected used override save");
        };
        assert_eq!(snapshot.provider, Agent::Codex);
        assert_eq!(snapshot.selections, saved().selections);
        overridden.preference_event(Ok(Some(snapshot)));
        overridden.remember_used_selection();
        assert!(overridden.preference_request().is_none());
    }

    #[test]
    fn repeated_use_preserves_save_error_until_explicit_retry() {
        let mut state = State::new();
        submit(&mut state);
        state.preference_event(Ok(None));
        state.remember_used_selection();
        let store::PreferenceRequest::Save(snapshot) = submit(&mut state) else {
            panic!("expected first use save");
        };
        state.preference_event(Err("disk full".into()));
        state.remember_used_selection();
        assert!(state.preference_error().is_some());
        assert!(
            state.preference_restore_error().is_none(),
            "save failure is not a failed startup restore"
        );
        assert!(state.preference_request().is_none());
        state.retry_preferences();
        let store::PreferenceRequest::Save(retry) = submit(&mut state) else {
            panic!("expected retry");
        };
        assert_eq!(retry, snapshot);
    }

    #[test]
    fn failed_initial_restore_exposes_request_gate_until_successful_retry() {
        let mut state = State::new();
        assert!(
            !state.preferences_loading(),
            "standalone state fixtures do not require runtime startup"
        );
        assert!(state.preference_restore_error().is_none());
        state.set_startup_provider(Agent::OpenAI, false);
        assert!(
            state.preferences_loading(),
            "runtime startup gates requests before Load is queued"
        );
        submit(&mut state);
        state.preference_event(Err("writer unavailable".into()));
        assert!(!state.preferences_loading());
        assert!(
            state
                .preference_restore_error()
                .unwrap()
                .contains("writer unavailable")
        );
        state.retry_preferences();
        assert!(
            state.preferences_loading(),
            "retry cannot briefly expose the passive fallback route"
        );
        submit(&mut state);
        state.preference_event(Ok(Some(saved())));
        assert!(!state.preferences_loading());
        assert!(state.preference_restore_error().is_none());
        assert_eq!(state.agent, Agent::ChatGPT);
    }
}
