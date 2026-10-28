//! Pure inspection and explicit-request freezing share one effort resolver.
use super::State;
use crate::{
    providers::{ModelOption, effort},
    store,
};

impl State {
    fn selected_effort_model(&self) -> Option<&ModelOption> {
        let catalogue = self.catalogues.get(&self.agent)?;
        let id = self
            .selection
            .model
            .as_ref()
            .or(catalogue.current.as_ref())?;
        catalogue.models.iter().find(|model| &model.id == id)
    }

    /// EV19 inspection is pure: no cached resolution, choice, draft or request changes.
    pub fn effort_preview(
        &self,
        text: &str,
        active_label: Option<&str>,
    ) -> Option<effort::Resolution> {
        if self.selection.effort.as_deref() != Some("auto") {
            return None;
        }
        let model = self.selected_effort_model();
        let model_id = self
            .selection
            .model
            .as_deref()
            .or_else(|| model.map(|model| model.id.as_str()));
        let last = self.effort_last.iter().find_map(|((agent, id), value)| {
            (*agent == self.agent && id.as_deref() == model_id).then_some(value.as_str())
        });
        // The committed request uses the same existing privacy transform.
        let text = crate::workspace::privacy::redact(text);
        let active_label = active_label.map(crate::workspace::privacy::redact);
        Some(effort::resolve(
            &text,
            active_label.as_deref(),
            model,
            last,
            crate::providers::capabilities::route(self.agent).effort_control,
        ))
    }

    /// Called only after preparation's busy/ownership/storage guards. Metadata
    /// inspection never classifies, and the authored `auto` selection stays intact.
    pub(super) fn freeze_effort(&mut self, task: &store::Snapshot) {
        self.request_effort = None;
        self.request_effort_model = None;
        self.request_progress = None;
        if self.inspect_models || self.inspect_sessions {
            if self.request_selection.effort.as_deref() == Some("auto") {
                self.request_selection.effort = None;
            }
            return;
        }
        self.request_progress = task
            .workflows
            .progress
            .as_ref()
            .map(|progress| progress.summary());
        let text = if self.agent.is_direct() {
            crate::providers::native_request_text(task)
        } else {
            task.request_text()
        };
        let active = task
            .workflows
            .progress
            .as_ref()
            .and_then(|progress| progress.focus_label());
        let resolution = self.effort_preview(text, active);
        if let Some(resolution) = resolution {
            self.request_effort_model = self
                .selection
                .model
                .clone()
                .or_else(|| self.selected_effort_model().map(|model| model.id.clone()));
            self.request_selection
                .effort
                .clone_from(&resolution.resolved);
            self.request_effort = Some(resolution);
        }
    }

    /// Only the transport's matching Dispatched event can advance ambiguity's
    /// retained level. Preparing, previewing, typing and provider output cannot.
    pub(super) fn commit_effort_resolution(&mut self) {
        if let Some(resolution) = &self.request_effort
            && resolution.reason != effort::Reason::Ultrathink
            && let Some(resolved) = &resolution.resolved
        {
            if self.effort_last.iter().any(|((agent, model), last)| {
                *agent == self.agent && model == &self.request_effort_model && last == resolved
            }) {
                return;
            }
            self.effort_last.insert(
                (self.agent, self.request_effort_model.clone()),
                resolved.clone(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{Agent, ModelCatalogue, ModelSelection, Operation, Request};

    fn state() -> State {
        let mut state = State::new();
        state.agent = Agent::Compatible;
        state.selection = ModelSelection {
            model: Some("loopback".into()),
            effort: Some("auto".into()),
        };
        state.catalogues.insert(
            state.agent,
            ModelCatalogue {
                models: vec![ModelOption {
                    id: "loopback".into(),
                    name: "loopback".into(),
                    description: String::new(),
                    efforts: vec!["low".into(), "medium".into(), "high".into(), "xhigh".into()],
                    default_effort: Some("medium".into()),
                }],
                current: Some("loopback".into()),
                source: "local fixture".into(),
                effort_note: None,
            },
        );
        state
    }

    fn saved_task(request: &str) -> store::Snapshot {
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.goal = "Saved goal".into();
        task.partner_drafts.insert(
            task.lead_partner,
            store::PartnerDrafts {
                drafts: vec![store::Draft {
                    field: store::EditorField::Request,
                    text: request.into(),
                    cursor: request.len(),
                }],
                ..Default::default()
            },
        );
        task
    }

    #[test]
    fn pure_preview_and_frozen_wire_agree_without_committing_or_rewriting_auto() {
        let mut state = state();
        let task = saved_task("Investigate why the concurrent cache is stale.");
        let expected = state.effort_preview(task.request_text(), None).unwrap();
        assert!(state.effort_last.is_empty());
        state.prepare(false, false, false, &task, &[], None);
        let Some(Request::Preview(input)) = &state.pending else {
            panic!("explicit preparation missing");
        };
        assert_eq!(input.effort_resolution.as_ref(), Some(&expected));
        assert_eq!(input.selection.effort, expected.resolved);
        assert_eq!(state.selection.effort.as_deref(), Some("auto"));
        assert!(state.effort_last.is_empty());
        state.commit_effort_resolution();
        assert_eq!(
            state.effort_preview("continue", None).unwrap().resolved,
            expected.resolved
        );
    }

    #[test]
    fn metadata_and_busy_pending_work_never_reclassify() {
        let mut state = state();
        let task = saved_task("Rename old to new");
        state.prepare(false, false, false, &task, &[], None);
        let frozen = state.request_effort.clone();
        state.prepare(false, false, false, &saved_task("ultrathink"), &[], None);
        assert_eq!(state.request_effort, frozen);
        let mut state = super::tests::state();
        state.prepare(false, false, true, &task, &[], None);
        assert!(state.request_effort.is_none());
        let Some(Request::Preview(input)) = &state.pending else {
            panic!("inspection missing");
        };
        assert!(input.effort_resolution.is_none());
        assert!(input.selection.effort.is_none());
        assert!(state.effort_last.is_empty());
    }

    #[test]
    fn fixed_selection_keeps_exact_wire_and_no_auto_metadata() {
        let mut state = state();
        state.selection.effort = Some("high".into());
        state.operation = Operation::Plan;
        state.prepare(false, false, false, &saved_task("ultrathink"), &[], None);
        let Some(Request::Preview(input)) = state.pending else {
            panic!("preparation missing");
        };
        assert_eq!(input.selection.effort.as_deref(), Some("high"));
        assert!(input.effort_resolution.is_none());
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    use crate::{app::App, providers::ModelSelection};
    use crossterm::event::Event;

    #[test]
    fn typing_paste_form_changes_and_busy_submit_do_not_commit_or_reclassify_effort() {
        for (width, height) in [(120, 36), (60, 18)] {
            let mut app = App::new("/workspace".into(), width, height);
            app.open_editor(store::EditorField::Request);
            app.planning.selection = ModelSelection {
                model: Some("fixture".into()),
                effort: Some("auto".into()),
            };
            app.update(Event::Paste("Rename old to new".into()));
            assert!(app.planning.request_effort.is_none() && app.planning.effort_last.is_empty());
            app.open_evolution_preview();
            let predicted = app.evolution_preview().unwrap().effort.clone();
            assert!(predicted.is_some());
            assert!(app.planning.request_effort.is_none() && app.planning.effort_last.is_empty());
            app.planning.request_effort = predicted;
            let frozen = app.planning.request_effort.clone();
            app.command("leafeon");
            assert_eq!(app.form(), 6);
            assert_eq!(app.planning.request_effort, frozen);
            app.planning.generation = Some(17);
            let task_id = app.planning.task_id();
            app.planning.event(crate::providers::Event::Item {
                generation: 17,
                task_id,
                thread_id: "fixture-thread".into(),
                turn_id: "fixture-turn".into(),
                item_id: "provider-output".into(),
                kind: crate::providers::ItemKind::Message,
                text: "ultrathink: investigate why the concurrent cache is stale".into(),
                complete: true,
            });
            assert_eq!(app.planning.request_effort, frozen);
            app.planning.active = true;
            app.input = "ultrathink: investigate why the race is flaky".into();
            app.submit_request();
            assert_eq!(app.planning.request_effort, frozen);
            assert!(app.planning.effort_last.is_empty());
            assert!(app.pending_request.is_none());
        }
    }

    #[test]
    fn ultrathink_top_is_for_one_request_not_the_ambiguity_default() {
        let mut state = State::new();
        state.agent = crate::providers::Agent::Compatible;
        state.request_effort_model = Some("fixture".into());
        state.effort_last.insert(
            (state.agent, state.request_effort_model.clone()),
            "low".into(),
        );
        state.request_effort = Some(effort::Resolution {
            bucket: None,
            resolved: Some("xhigh".into()),
            reason: effort::Reason::Ultrathink,
            source: effort::Source::UserRequest,
            application: effort::Application::RequestedNotAcknowledged,
        });
        state.commit_effort_resolution();
        assert_eq!(
            state.effort_last[&(state.agent, Some("fixture".into()))],
            "low"
        );
    }
}

#[cfg(test)]
#[test]
fn effort_preview_uses_the_same_privacy_bytes_as_frozen_submission() {
    use crate::providers::{Agent, ModelCatalogue, ModelSelection};
    let mut state = State::new();
    state.agent = Agent::Compatible;
    state.selection = ModelSelection {
        model: Some("fixture".into()),
        effort: Some("auto".into()),
    };
    state.catalogues.insert(
        state.agent,
        ModelCatalogue {
            models: vec![ModelOption {
                id: "fixture".into(),
                name: "fixture".into(),
                description: String::new(),
                efforts: vec!["low".into(), "medium".into(), "high".into(), "xhigh".into()],
                default_effort: Some("medium".into()),
            }],
            current: Some("fixture".into()),
            source: "local fixture".into(),
            effort_note: None,
        },
    );
    let raw = "Authorization: Bearer ultrathink";
    let frozen = crate::workspace::privacy::redact(raw);
    assert_ne!(frozen.as_ref(), raw);
    let preview = state.effort_preview(raw, None).unwrap();
    assert_eq!(preview, state.effort_preview(&frozen, None).unwrap());
    assert_ne!(preview.reason, effort::Reason::Ultrathink);
    assert!(state.effort_last.is_empty());
}

#[cfg(test)]
#[test]
fn remembered_auto_pairing_is_available_even_without_a_provider_effort_ladder() {
    use crate::{
        app::App,
        messages::Message,
        providers::{Agent, ModelCatalogue, ModelSelection},
    };
    let mut app = App::new("/workspace".into(), 60, 18);
    app.planning.agent = Agent::Compatible;
    app.planning.selection = ModelSelection {
        model: Some("fixed-by-provider".into()),
        effort: Some("auto".into()),
    };
    app.planning.remembered_pairing = Some((1, 0));
    app.planning.catalogues.insert(
        Agent::Compatible,
        ModelCatalogue {
            models: vec![ModelOption {
                id: "fixed-by-provider".into(),
                name: "fixed-by-provider".into(),
                description: String::new(),
                efforts: Vec::new(),
                default_effort: None,
            }],
            current: Some("fixed-by-provider".into()),
            source: "local fixture".into(),
            effort_note: None,
        },
    );
    assert_eq!(
        app.records_pairing_badge(true),
        Some(Message::RecordsRemembered)
    );
    app.planning.selection.effort = Some("high".into());
    assert_eq!(
        app.records_pairing_badge(true),
        Some(Message::RecordsRememberedUnavailable)
    );
}
