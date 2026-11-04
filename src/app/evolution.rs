//! Request-owned automatic appearance changes; never a model classifier.
use super::App;
use crate::{messages::Message, providers::Operation};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub form: usize,
    pub operation: Operation,
    pub reason: Message,
}

#[derive(Clone, Copy)]
pub struct Transition {
    pub from: usize,
    pub to: usize,
    pub reason: Message,
    pub fresh: bool,
}

/// A local prediction, not a prepared request or an authority-bearing receipt.
pub struct Preview {
    pub selection: Selection,
    pub automatic: bool,
    pub restriction: Option<Operation>,
    pub effort: Option<crate::providers::effort::Resolution>,
    pub todo_step: Option<String>,
    input: String,
    task: i64,
    partner: i64,
    form: usize,
    operation: Operation,
    completed_plan: bool,
}

#[derive(Default)]
pub struct State {
    pub transition: Option<Transition>,
    pub popup_shown: bool,
    pub(super) motion: Option<super::evolution_motion::Motion>,
    preview: Option<Preview>,
    // Display-only, process-local latch per acquired owner, including parked
    // requests. Released owners are pruned before the next adaptation attempt.
    adapted_requests: HashMap<(i64, i64), String>,
}

/// The appearance a role leads to. Capability is never chosen by the form:
/// every role keeps the ordinary approved tool set, narrowed only by the
/// user's explicit restriction.
fn role_selection(form: usize) -> Selection {
    Selection {
        form,
        operation: Operation::Implement,
        reason: match form {
            1 => Message::CoreEvolutionDiscovery,
            2 => Message::CoreEvolutionVerification,
            3 => Message::CoreEvolutionImplementation,
            4 => Message::CoreEvolutionPlanning,
            5 => Message::CoreEvolutionDiagnosis,
            6 => Message::CoreEvolutionRefactor,
            7 => Message::CoreEvolutionReview,
            _ => Message::CoreEvolutionDocumentation,
        },
    }
}

/// The form a todo step leads to: only the structured role the provider's own
/// plan names (native phase or a `[role]` tag on any route). Step text is never
/// re-guessed from word lists; an untagged step steers nothing.
pub fn classify_step(step: &crate::workspace::progress::Step) -> Option<Selection> {
    step.role().map(|role| role_selection(role.form()))
}

impl App {
    /// The notification yields to consent, live code and current non-success results.
    pub fn evolution_popup_ready(&self) -> bool {
        self.evolution
            .transition
            .is_some_and(|transition| transition.from != transition.to)
            && self.approval_shortcut_available()
            && !self.carry_over_explicit()
            && self.approval_count() == 0
            && self.planning.request.is_none()
            && !self.planning.items.iter().any(|item| {
                std::sync::Arc::ptr_eq(&item.request, &self.planning.submitted_request)
                    && (matches!(
                        item.kind,
                        crate::providers::ItemKind::File(
                            crate::providers::FilePhase::Generating
                                | crate::providers::FilePhase::Received
                                | crate::providers::FilePhase::AwaitingReview
                        )
                    ) || item.status.as_ref().is_some_and(|status| {
                        status
                            .outcome
                            .is_some_and(|outcome| outcome != crate::providers::Outcome::Completed)
                    }))
            })
    }

    /// The adopted todo step that automatic evolution continues, if any.
    fn todo_focus(&self) -> Option<Selection> {
        (self.lead.auto_form && self.planning.explicit_operation() != Some(Operation::Review))
            .then(|| self.workflows.progress.as_ref()?.focus_step())
            .flatten()
            .and_then(|(_, step)| classify_step(step))
    }

    /// Current todo step position and the form it steers automatic evolution
    /// toward, for the companion card. Purely a local prediction.
    pub fn todo_guide(&self) -> Option<(String, Option<usize>)> {
        let position = self.workflows.progress.as_ref()?.focus_position()?;
        Some((position, self.todo_focus().map(|todo| todo.form)))
    }

    /// Whether the next Send starts from the adopted plan's focus step. Any
    /// Send continues the current plan until the model replaces it; the request
    /// text is never interpreted.
    pub(super) fn follows_todo(&self) -> bool {
        self.todo_focus().is_some()
    }

    /// Shared by inspection and ordinary Submit; does not mutate or prepare work.
    ///
    /// The request text never picks a form or a capability. A Send starts in the
    /// adopted focus step's role form (otherwise the current form) and the
    /// model's role-tagged plan may adapt it once while it runs. Capability is the
    /// ordinary approved tool set (every file effect and command still needs
    /// exact one-shot approval), narrowed only by an explicit Plan/Review mode.
    pub(super) fn request_evolution(&self) -> Option<Selection> {
        let mut choice = self
            .todo_focus()
            .map(|todo| Selection {
                reason: Message::CoreEvolutionTodo,
                ..todo
            })
            .unwrap_or(Selection {
                form: self.form(),
                operation: Operation::Implement,
                reason: Message::CoreEvolutionGeneral,
            });
        if !self.lead.auto_form {
            choice.form = self.form();
        }
        if self.planning.explicit_operation() == Some(Operation::Review) {
            choice = Selection {
                form: if self.lead.auto_form { 7 } else { self.form() },
                operation: Operation::Review,
                reason: Message::CoreEvolutionReview,
            };
        } else if self.planning.explicit_operation() == Some(Operation::Plan) {
            choice.operation = Operation::Plan;
            choice.reason = Message::CoreEvolutionReadonly;
        }
        Some(choice)
    }

    /// Follow the adopted plan at most once per owned logical request. The
    /// focus step's role changes appearance only; owner, frozen profile,
    /// operation and approvals stay unchanged. Nothing is sent or granted.
    pub(super) fn follow_todo_role(&mut self) {
        if let Some(target) = self.todo_focus() {
            self.evolve_running(target.form, Message::CoreEvolutionTodo);
        }
    }

    /// Observed tool activity of the lead's running request steers appearance
    /// only while the plan's focus step names no role (or there is no plan):
    /// a write proposal is implementation, a command request is verification.
    pub(super) fn follow_activity(&mut self, role: crate::workspace::progress::Role) {
        if !self.lead.auto_form
            || self.planning.explicit_operation() == Some(Operation::Review)
            || self.todo_focus().is_some()
            || self.form() == role.form()
        {
            return;
        }
        let target = role_selection(role.form());
        if self.evolve_running(target.form, target.reason) {
            self.changed_task();
        }
    }

    fn evolve_running(&mut self, form: usize, reason: Message) -> bool {
        let from = self.form();
        if form == from || !self.planning.work_acquired || self.planning.cancelling {
            return false;
        }
        let Some(claim) = self.planning.work.as_ref() else {
            return false;
        };
        let owner = (claim.task_id, claim.owner.partner);
        if owner != (self.task_id, self.lead.id) {
            return false;
        }
        if self
            .evolution
            .adapted_requests
            .get(&owner)
            .is_some_and(|request| request == &claim.request_id)
        {
            return false;
        }
        // Only live claims can need a latch. In particular, completed tasks
        // must not accumulate display history for the lifetime of the app.
        let planning = &self.planning;
        let parked = &self.cooperation.parked;
        self.evolution.adapted_requests.retain(|owner, request| {
            std::iter::once(planning)
                .chain(parked.values())
                .any(|state| {
                    state.work_acquired
                        && state.work.as_ref().is_some_and(|claim| {
                            *owner == (claim.task_id, claim.owner.partner)
                                && *request == claim.request_id
                        })
                })
        });
        // Spend only an actual automatic transition. Send-time selection and
        // deliberate choices do not spend or reset this request's adaptation.
        self.evolution
            .adapted_requests
            .insert(owner, claim.request_id.clone());
        self.set_form(form);
        // A carry-over snapshot is for deliberate switches between requests.
        self.carry_over.pending = None;
        self.carry_over.pending_visible = false;
        self.evolution.transition = Some(Transition {
            from,
            to: form,
            reason,
            fresh: false,
        });
        self.evolution.popup_shown = false;
        true
    }

    pub(super) fn open_evolution_preview(&mut self) {
        if !self.composer_is_request() {
            self.notice =
                "Open the Request composer to inspect its next evolution; nothing sent.".into();
            return;
        }
        let selection = self.request_evolution().unwrap_or(Selection {
            form: self.form(),
            operation: self.planning.operation,
            reason: Message::CoreEvolutionUnchanged,
        });
        self.evolution.preview = Some(Preview {
            selection,
            automatic: self.lead.auto_form,
            restriction: self.planning.explicit_operation(),
            input: self.input.clone(),
            task: self.task_id,
            partner: self.lead.id,
            form: self.form(),
            operation: self.planning.operation,
            completed_plan: self.planning.has_completed_plan(),
            effort: self.planning.effort_preview(
                &self.input,
                self.workflows
                    .progress
                    .as_ref()
                    .and_then(|p| p.focus_label()),
            ),
            todo_step: self.workflows.progress.as_ref().map(|p| p.summary()),
        });
        self.open_overlay(super::Overlay::EvolutionPreview);
    }

    pub fn evolution_preview(&self) -> Option<&Preview> {
        self.evolution.preview.as_ref().filter(|preview| {
            preview.input == self.input
                && preview.task == self.task_id
                && preview.partner == self.lead.id
                && preview.form == self.form()
                && preview.operation == self.planning.operation
                && preview.automatic == self.lead.auto_form
                && preview.restriction == self.planning.explicit_operation()
                && preview.completed_plan == self.planning.has_completed_plan()
                && preview.todo_step == self.workflows.progress.as_ref().map(|p| p.summary())
                && preview.effort
                    == self.planning.effort_preview(
                        &self.input,
                        self.workflows
                            .progress
                            .as_ref()
                            .and_then(|p| p.focus_label()),
                    )
        })
    }

    /// Called only after ordinary Submit's owner/safety guards, before save.
    pub(super) fn evolve_request(&mut self) -> bool {
        let Some(choice) = self.request_evolution() else {
            return false;
        };
        let from = self.form();
        let fresh = self.planning.prepare_automatic_operation(choice.operation);
        if from != choice.form || fresh {
            self.set_form(choice.form);
            self.evolution.transition = Some(Transition {
                from,
                to: choice.form,
                reason: choice.reason,
                fresh,
            });
            self.evolution.popup_shown = false;
            if self.follows_todo()
                && let Some(progress) = &self.workflows.progress
                && let Some(step) = progress.focus_position()
            {
                self.notice = format!(
                    "Automatic form follows todo revision {} / step {step}",
                    progress.revision
                );
            }
        }
        true
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    pub(crate) fn role_focus(app: &mut crate::app::App, phase: &str) {
        use crate::workspace::progress::{State, Status, Step};
        let mut progress = State::default();
        progress
            .propose(
                "native todo",
                vec![Step {
                    label: "Current step".into(),
                    phase: Some(phase.into()),
                    status: Status::Active,
                }],
            )
            .unwrap();
        progress.adopt().unwrap();
        app.workflows.progress = Some(progress);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{Overlay, StorageState, execution},
        providers::{self, Agent, ModelSelection, Request},
        store,
    };
    use crossterm::event::Event;

    fn app() -> App {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.task_id = 7;
        app.storage = StorageState::Saved;
        app.planning.reset(7);
        app.planning.preference_event(Ok(None));
        app
    }
    fn own_request(app: &mut App, request_id: &str) -> store::WorkClaim {
        let claim = store::WorkClaim {
            task_id: app.task_id,
            owner: app.attribution(),
            request_id: request_id.into(),
            writing: true,
        };
        app.planning.work = Some(claim.clone());
        app.planning.work_acquired = true;
        claim
    }
    fn role_plan(
        steps: &[(&str, &str, crate::workspace::progress::Status)],
    ) -> crate::workspace::progress::State {
        use crate::workspace::progress::{State, Step};
        let mut progress = State::default();
        progress
            .propose(
                "native todo",
                steps
                    .iter()
                    .map(|(label, phase, status)| Step {
                        label: (*label).into(),
                        phase: (!phase.is_empty()).then(|| (*phase).into()),
                        status: *status,
                    })
                    .collect(),
            )
            .unwrap();
        progress.adopt().unwrap();
        progress
    }

    #[test]
    fn structured_step_roles_choose_the_form_and_untagged_steps_steer_nothing() {
        use crate::workspace::progress::{Role, Status, Step};
        let step = |label: &str, phase: Option<&str>| Step {
            label: label.into(),
            phase: phase.map(Into::into),
            status: Status::Active,
        };
        for role in Role::ALL {
            // Labels no word table recognizes still evolve from their role, and
            // the role never narrows or widens the request's tools.
            let selection = classify_step(&step("결제 모듈 흐름 정리", Some(role.name()))).unwrap();
            assert_eq!(selection.form, role.form(), "{role:?}");
            assert_eq!(selection.operation, Operation::Implement, "{role:?}");
        }
        // The role wins over any word in the label.
        assert_eq!(
            classify_step(&step("Review the code", Some("diagnose")))
                .unwrap()
                .form,
            5
        );
        // Step text is never re-guessed: untagged or free-text phases steer nothing.
        assert_eq!(classify_step(&step("Review the code", None)), None);
        assert_eq!(classify_step(&step("결제 모듈", Some("investigate"))), None);
        // A `[role]` tag from a CLI/ACP plan becomes the same structured role.
        let tagged = step("[Verify] cargo test", None).untag();
        assert_eq!(
            (tagged.label.as_str(), tagged.role()),
            ("cargo test", Some(Role::Verify))
        );
        assert_eq!(classify_step(&tagged).unwrap().form, 2);
    }

    #[test]
    fn sends_start_from_the_plan_focus_role_and_never_read_the_request_text() {
        use crate::workspace::progress::Status;
        let mut app = app();
        // The request text never picks a form, however specific it sounds.
        for text in [
            "Review the code",
            "이 에러 원인 찾아줘",
            "Read src/main.rs",
            "Fix the bug",
        ] {
            app.input = text.into();
            let choice = app.request_evolution().unwrap();
            assert_eq!(
                (choice.form, choice.operation),
                (0, Operation::Implement),
                "{text}"
            );
        }
        app.workflows.progress = Some(role_plan(&[
            ("결제 모듈 흐름 정리", "discover", Status::Done),
            ("결제 모듈 개편안", "plan", Status::Active),
        ]));
        // Any Send continues the adopted plan's focus step, whatever its wording;
        // a new topic gets a new plan from the model, which evolves it mid-run.
        for followup in ["계속", "continue", "Tell me more about this task."] {
            app.input = followup.into();
            assert!(app.follows_todo());
            let choice = app.request_evolution().unwrap();
            assert_eq!(
                (choice.form, choice.operation),
                (4, Operation::Implement),
                "{followup}"
            );
            assert_eq!(choice.reason, Message::CoreEvolutionTodo);
        }
        // A settled plan steers nothing.
        app.workflows.progress = Some(role_plan(&[("결제 모듈 개편안", "plan", Status::Done)]));
        assert!(!app.follows_todo());
        assert_eq!(app.request_evolution().unwrap().form, 0);
        assert_eq!(app.form(), 0, "prediction evolved before Send");
        assert!(app.pending_request.is_none() && app.planning.pending.is_none());
    }

    #[test]
    fn explicit_user_policy_and_pinned_forms_still_win_over_the_plan() {
        use crate::workspace::progress::Status;
        let mut app = app();
        app.workflows.progress = Some(role_plan(&[("결제 모듈", "implement", Status::Active)]));
        // Wording never narrows tools; the model reads it and approvals gate effects.
        app.input = "계속, 읽기만 해".into();
        assert_eq!(
            app.request_evolution().unwrap().operation,
            Operation::Implement
        );
        // Explicit Plan mode narrows tools without stopping evolution.
        app.input = "continue".into();
        app.planning.set_operation(Operation::Plan);
        let choice = app.request_evolution().unwrap();
        assert_eq!((choice.form, choice.operation), (3, Operation::Plan));
        // A pinned form keeps its appearance.
        app.lead.auto_form = false;
        assert_eq!(app.request_evolution().unwrap().form, app.form());
    }

    #[test]
    fn activity_evolution_survives_an_inflight_snapshot_acknowledgement() {
        use crate::workspace::progress::Role;
        let mut app = app();
        own_request(&mut app, "snapshot-request");
        app.changed_task();
        let before = app.snapshot("/workspace");
        app.save_submitted();
        app.follow_activity(Role::Implement);
        app.store_event(store::Event::Saved(before));
        assert_eq!(app.form(), 3);
        assert!(app.save_due(std::time::Instant::now()));
        let evolved = app.snapshot("/workspace");
        assert!(evolved.revision > app.saved_revision);
        assert_eq!(evolved.form, 3);
        app.save_submitted();
        app.store_event(store::Event::Saved(evolved));
        assert_eq!(app.storage, StorageState::Saved);
        app.follow_activity(Role::Implement);
        assert!(!app.save_due(std::time::Instant::now()));
    }

    #[test]
    fn tool_activity_adapts_once_and_only_without_a_structured_role() {
        use crate::workspace::progress::{Role, Status};
        let mut app = app();
        own_request(&mut app, "activity-request");
        // A role-tagged focus step wins without spending the activity fallback.
        app.workflows.progress = Some(role_plan(&[("검토", "review", Status::Active)]));
        app.follow_activity(Role::Verify);
        assert_eq!(app.form(), 0);
        app.workflows.progress = None;
        app.follow_activity(Role::Verify);
        assert_eq!(app.form(), 2);
        assert_eq!(app.evolution.transition.unwrap().to, 2);
        let revision = app.snapshot("/workspace").revision;
        for role in [Role::Implement, Role::Verify, Role::Implement] {
            app.follow_activity(role);
            assert_eq!(app.form(), 2, "write/command oscillation stays stable");
            assert_eq!(app.snapshot("/workspace").revision, revision);
        }
        // Role and activity updates share the same request-owned bound.
        app.workflows.progress = Some(role_plan(&[("검토", "review", Status::Active)]));
        app.follow_todo_role();
        assert_eq!(app.form(), 2);
    }

    #[test]
    fn running_request_adapts_once_without_changing_owner_tools_or_frozen_profile() {
        use crate::workspace::progress::Status;
        for (width, height) in [(120, 36), (60, 18)] {
            let mut app = app();
            app.width = width;
            app.height = height;
            let owner = own_request(&mut app, "role-request");
            let profile = crate::workspace::form_profiles::built_in(0);
            app.planning.behavior_profile = Some(profile.clone());
            for operation in [Operation::Implement, Operation::Plan] {
                app.planning.operation = operation;
                for role in ["discover", "diagnose", "implement", "verify", "discover"] {
                    app.workflows.progress =
                        Some(role_plan(&[("중립 단계", role, Status::Active)]));
                    app.follow_todo_role();
                    assert_eq!(app.form(), 1, "role oscillation stays on first adaptation");
                    let transition = app.evolution.transition.unwrap();
                    assert_eq!(transition.to, 1);
                    assert_eq!(transition.reason, Message::CoreEvolutionTodo);
                    assert_eq!(app.planning.work.as_ref(), Some(&owner));
                    assert_eq!(app.planning.operation, operation, "tools stay frozen");
                    assert_eq!(app.planning.behavior_profile.as_ref(), Some(&profile));
                    assert!(app.pending_request.is_none() && app.planning.pending.is_none());
                }
            }
            app.update(Event::Resize(60, 18));
            app.follow_todo_role();
            assert_eq!(app.form(), 1, "resize does not re-arm adaptation");
            app.planning.set_operation(Operation::Review);
            app.evolution.transition = None;
            app.follow_todo_role();
            assert_eq!(app.form(), 1);
            assert!(app.evolution.transition.is_none());
        }
    }

    #[test]
    fn same_form_does_not_spend_adaptation_and_a_new_send_gets_its_own_bound() {
        use crate::workspace::progress::{Role, Status};
        let mut app = app();
        app.set_form(1);
        own_request(&mut app, "first-send");
        app.workflows.progress = Some(role_plan(&[("첫 단계", "discover", Status::Active)]));
        app.follow_todo_role();
        app.workflows.progress = Some(role_plan(&[("다음 단계", "diagnose", Status::Active)]));
        app.follow_todo_role();
        assert_eq!(app.form(), 5, "same-form role did not consume adaptation");
        app.workflows.progress = None;
        app.follow_activity(Role::Verify);
        assert_eq!(app.form(), 5);
        app.planning.work = None;
        app.planning.work_acquired = false;
        app.workflows.progress = Some(role_plan(&[("새 요청", "implement", Status::Active)]));
        app.input = "Continue this task.".into();
        app.submit_request();
        assert_eq!(app.form(), 3, "new explicit Send selects its adopted focus");
        app.pending_request = None;
        own_request(&mut app, "second-send");
        app.workflows.progress = Some(role_plan(&[("검증", "verify", Status::Active)]));
        app.follow_todo_role();
        assert_eq!(
            app.form(),
            2,
            "new Send still gets its first in-flight adaptation"
        );
        app.workflows.progress = None;
        app.follow_activity(Role::Implement);
        assert_eq!(app.form(), 2);
        assert!(
            app.evolution
                .adapted_requests
                .values()
                .all(|request| request != "first-send"),
            "released requests are pruned before the next adaptation"
        );
    }

    #[test]
    fn parked_live_owners_keep_their_latch_and_released_owners_are_pruned() {
        use crate::workspace::progress::Role;
        let mut app = app();
        own_request(&mut app, "first-owner");
        app.follow_activity(Role::Verify);
        let mut second = app.planning.new_partner(app.attribution(), app.task_id);
        second.work = Some(store::WorkClaim {
            task_id: app.task_id,
            owner: store::Attribution {
                partner: 2,
                ..app.attribution()
            },
            request_id: "second-owner".into(),
            writing: true,
        });
        second.work_acquired = true;
        let first = std::mem::replace(&mut app.planning, second);
        app.cooperation.parked.insert(1, first);
        app.lead.id = 2;
        app.follow_activity(Role::Implement);
        assert_eq!(app.form(), 3);
        assert_eq!(app.evolution.adapted_requests.len(), 2);
        let first = app.cooperation.parked.remove(&1).unwrap();
        let second = std::mem::replace(&mut app.planning, first);
        app.cooperation.parked.insert(2, second);
        app.lead.id = 1;
        app.follow_activity(Role::Verify);
        assert_eq!(
            app.form(),
            3,
            "returning to a parked owner does not re-arm it"
        );
        app.cooperation.parked.clear();
        own_request(&mut app, "replacement-owner");
        app.follow_activity(Role::Verify);
        assert_eq!(app.form(), 2);
        assert_eq!(app.evolution.adapted_requests.len(), 1);
    }

    #[test]
    fn deliberate_form_choice_and_pin_are_not_blocked_or_spent_by_the_latch() {
        use crate::workspace::progress::{Role, Status};
        let mut app = app();
        let owner = own_request(&mut app, "manual-request");
        app.follow_activity(Role::Verify);
        app.planning.work = None;
        app.planning.work_acquired = false;
        app.command("flareon");
        assert_eq!((app.form(), app.lead.auto_form), (3, false));
        app.planning.work = Some(owner);
        app.planning.work_acquired = true;
        app.workflows.progress = Some(role_plan(&[("진단", "diagnose", Status::Active)]));
        app.follow_todo_role();
        assert_eq!(app.form(), 3);
        app.lead.auto_form = true;
        app.follow_todo_role();
        assert_eq!(
            app.form(),
            3,
            "manual choice did not reset the spent request"
        );
        own_request(&mut app, "pinned-new-request");
        app.lead.auto_form = false;
        app.follow_todo_role();
        app.lead.auto_form = true;
        app.follow_todo_role();
        assert_eq!(
            app.form(),
            5,
            "pinned suppression did not spend a new request"
        );
    }

    #[test]
    fn absent_unacquired_cancelled_or_foreign_owners_cannot_adapt_or_spend() {
        use crate::workspace::progress::{Role, Status};
        for invalid in 0..5 {
            let mut app = app();
            let owner = own_request(&mut app, "owned-request");
            match invalid {
                0 => app.planning.work = None,
                1 => app.planning.work_acquired = false,
                2 => app.planning.cancelling = true,
                3 => app.planning.work.as_mut().unwrap().task_id += 1,
                _ => app.planning.work.as_mut().unwrap().owner.partner += 1,
            }
            app.follow_activity(Role::Implement);
            app.workflows.progress = Some(role_plan(&[("진단", "diagnose", Status::Active)]));
            app.follow_todo_role();
            assert_eq!(app.form(), 0);
            assert!(app.evolution.transition.is_none());
            assert!(app.evolution.adapted_requests.is_empty());
            app.planning.work = Some(owner);
            app.planning.work_acquired = true;
            app.planning.cancelling = false;
            app.follow_todo_role();
            assert_eq!(
                app.form(),
                5,
                "invalid owner did not spend the actual owner's adaptation"
            );
        }
    }
    #[test]
    fn prediction_preserves_draft_and_matches_submitted_appearance_and_policy() {
        for draft in [
            "Review the code and fix src/main.rs.",
            "코드를 검토하고 src/main.rs를 수정해줘.",
            "Improve the todo tool UI and remove duplicate display rows.",
            "todo 툴 UI가 중복으로 출력돼. 이부분 개선해줘.",
            "Diagnose the tool UI and improve its layout.",
            "UI를 분석해서 레이아웃을 개선해줘.",
            "Diagnose how to fix the error.",
            "Diagnose how to fix src/main.rs.",
            "Review how to repair the code.",
            "오류 수정 방법을 분석해줘.",
            "Diagnose the error and fix it if needed.",
            "Review the patch; then repair it.",
            "Explain how to build the project and then build the project.",
            "Explain how to fix the error and then fix it.",
            "오류 수정 방법을 분석하고 수정해줘.",
            "오류를 분석하고 수정해줘.",
            "Review the patch; do not fix it.",
            "Review the patch: \"fix src/main.rs\".",
            "Review tool UI, then fix its layout.",
            "UI를 개선하지 말고 점검해줘.",
            "Read src/main.rs; do not change files.",
            "\"Create src/main.rs\" is quoted text.",
            "계속 설명해줘.",
        ] {
            for manual in [false, true] {
                for restriction in [None, Some(Operation::Plan), Some(Operation::Review)] {
                    let mut app = app();
                    app.open_editor(store::EditorField::Request);
                    app.lead.auto_form = !manual;
                    app.input = draft.into();
                    app.cursor = draft.len();
                    if let Some(restriction) = restriction {
                        app.planning.set_operation(restriction);
                    }
                    let before = (app.form(), app.planning.operation, app.cursor);
                    app.open_evolution_preview();
                    let predicted = app.evolution_preview().unwrap().selection;
                    assert_eq!((app.form(), app.planning.operation, app.cursor), before);
                    assert_eq!(app.input, draft);
                    assert!(app.pending_request.is_none() && app.planning.pending.is_none());
                    app.close_overlay();
                    app.submit_request();
                    assert_eq!(
                        (app.form(), app.planning.operation),
                        (predicted.form, predicted.operation),
                        "{draft}"
                    );
                }
            }
        }
    }

    #[test]
    fn prediction_invalidates_changed_draft_owner_and_policy() {
        for change in 0..5 {
            let mut app = app();
            app.open_editor(store::EditorField::Request);
            app.input = "Create src/new.rs".into();
            app.open_evolution_preview();
            match change {
                0 => app.input.push('!'),
                1 => app.task_id += 1,
                2 => app.lead.id += 1,
                3 => app.lead.auto_form = false,
                _ => app.planning.set_operation(Operation::Review),
            }
            assert!(app.evolution_preview().is_none());
            assert!(app.pending_request.is_none() && app.planning.pending.is_none());
        }
    }

    #[test]
    fn prediction_inspector_cannot_send_or_edit_the_hidden_request() {
        let mut app = app();
        app.open_editor(store::EditorField::Request);
        app.input = "Create src/한글.rs".into();
        app.cursor = app.input.len() - 3;
        let original = (app.input.clone(), app.cursor);
        app.open_evolution_preview();
        let now = std::time::Instant::now();
        app.update_at(
            Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            )),
            now,
        );
        app.update_at(Event::Paste("unrequested replacement".into()), now);
        assert!(app.pending_request.is_none() && app.planning.pending.is_none());
        assert_eq!((&app.input, app.cursor), (&original.0, original.1));
        app.update_at(
            Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            )),
            now,
        );
        assert_eq!(app.overlay, None);
        assert!(app.composer_is_request());
        assert_eq!((&app.input, app.cursor), (&original.0, original.1));
    }
    fn planned_app() -> App {
        let mut app = app();
        app.planning.agent = Agent::Compatible;
        // An explicit Plan mode, not wording, makes planning read-only.
        app.planning.set_operation(Operation::Plan);
        app.input = "새 프로젝트 구현 계획 세워줘".into();
        app.submit_request();
        app.pending_request = None;
        app.planning.submitted_request = app.input.as_str().into();
        app.planning.front = Some(app.attribution());
        app.planning.items.push(super::super::planning::Item {
            thread_id: "planning-thread".into(),
            turn_id: "planning-turn".into(),
            item_id: "public-plan".into(),
            kind: providers::ItemKind::Message,
            text: "Create planned/한글/saved.txt containing the chosen result.".into(),
            complete: true,
            request: app.planning.submitted_request.clone(),
            owner: Some(app.attribution()),
            status: None,
        });
        app.planning.thread_id = Some("planning-thread".into());
        app.planning.plan_completed = true;
        app
    }

    #[test]
    fn leaving_plan_mode_carries_the_completed_plan_without_old_authority() {
        let mut app = planned_app();
        // Wording cannot leave an explicit Plan mode.
        app.input = "구현해줘".into();
        app.submit_request();
        assert_eq!(app.planning.operation, Operation::Plan);
        app.pending_request = None;
        app.planning.set_operation(Operation::Implement);
        assert_eq!(app.planning.operation, Operation::Implement);
        assert!(app.planning.thread_id.is_none());
        app.input = "구현해줘".into();
        app.submit_request();
        let snapshot = app.snapshot("/workspace");
        app.planning.command("new", true, &snapshot, &[], None);
        let Some(Request::Preview(input)) = app.planning.pending.take() else {
            panic!("missing implementation preview");
        };
        assert_eq!(input.operation, Operation::Implement);
        assert!(input.resume.is_none() && input.conversation.is_none());
        let context = input.prior_plan_payload().expect("completed plan carried");
        assert!(context.to_string().contains("planned/한글/saved.txt"));
        assert!(!context.to_string().contains("planning-thread"));
        assert!(input.validate_selection().is_ok());
    }
    #[test]
    fn every_request_gets_the_approved_tool_set_unless_plan_or_review_mode_is_on() {
        // Wording never changes tools: the model reads "read only" itself and
        // every effect it might still propose needs exact approval.
        for text in [
            "make a snake game in python",
            "이 코드 설명해줘, 읽기만",
            "Read src/main.rs; do not change files",
            "Diagnose how to fix src/main.rs.",
            "뭐 더하면 좋을까?",
        ] {
            let mut app = app();
            app.input = text.into();
            app.submit_request();
            assert_eq!(app.planning.operation, Operation::Implement, "{text}");
            let snapshot = app.snapshot("/workspace");
            app.planning.command("new", true, &snapshot, &[], None);
            let Some(Request::Preview(input)) = app.planning.pending.take() else {
                panic!("missing preparation: {text}");
            };
            assert!(providers::writing_required(input.operation), "{text}");
        }
        for operation in [Operation::Plan, Operation::Review] {
            let mut app = app();
            app.planning.set_operation(operation);
            app.input = "make a game".into();
            app.submit_request();
            assert_eq!(app.planning.operation, operation);
        }
        // Consecutive ordinary requests keep one conversation: no operation switch.
        let mut app = app();
        app.input = "Analyze the error.".into();
        app.submit_request();
        app.pending_request = None;
        app.planning.thread_id = Some("diagnosis-thread".into());
        app.input = "고쳐줘".into();
        app.submit_request();
        assert_eq!(app.planning.operation, Operation::Implement);
        assert_eq!(app.planning.thread_id.as_deref(), Some("diagnosis-thread"));
    }
    #[test]
    fn only_submit_routes_before_save_and_frozen_input_without_an_extra_dispatch() {
        let mut app = app();
        let text = "Create a new Python code file.";
        app.update(Event::Paste(text.into()));
        let caret = app.cursor;
        assert_eq!(app.form(), 0);
        assert!(app.planning.pending.is_none() && app.pending_request.is_none());
        app.command("help");
        assert_eq!(app.overlay, Some(Overlay::Help));
        assert_eq!(app.form(), 0);
        app.close_overlay();
        app.submit_request();
        // The request text does not evolve; the running plan will.
        assert_eq!(app.form(), 0);
        assert!(app.lead.auto_form);
        assert_eq!((app.input.as_str(), app.cursor), (text, caret));
        assert!(app.pending_request.is_some() && app.save_requested);
        assert!(app.planning.pending.is_none() && !app.execution.busy());
        let snapshot = app.snapshot("/workspace");
        assert_eq!((snapshot.form, snapshot.auto_form), (0, true));
        assert_eq!(snapshot.request_text(), text);
        assert!(app.planning.command("new", true, &snapshot, &[], None));
        let Some(Request::Preview(input)) = app.planning.pending.take() else {
            panic!("missing frozen input");
        };
        assert_eq!(input.task.form, 0);
        assert!(input.task.auto_form);
        assert_eq!(input.task.request_text(), text);
        assert_eq!(input.operation, Operation::Implement);
        assert_eq!(app.planning.owner().unwrap().form, 0);
        assert!(!app.execution.busy());
    }

    #[test]
    fn first_saved_identity_preserves_requested_capability_and_explicit_readonly_policy() {
        for policy in [None, Some(Operation::Plan), Some(Operation::Review)] {
            let mut app = App::new("/workspace".into(), 120, 36);
            app.storage = StorageState::Saved;
            app.planning.preference_event(Ok(None));
            app.planning.agent = Agent::Compatible;
            if let Some(policy) = policy {
                app.planning.set_operation(policy);
            }
            let text = "Generate three project files; propose their paths and preserve literal CRLF/Unicode and an empty file.";
            app.input = text.into();
            app.submit_request();
            let mut snapshot = app.snapshot("/workspace");
            snapshot.id = 7;
            app.store_event(store::Event::Saved(snapshot.clone()));
            assert!(app.planning.command("new", true, &snapshot, &[], None));
            let Some(Request::Preview(input)) = app.planning.pending.take() else {
                panic!("missing first saved request preparation");
            };
            assert_eq!(input.task.id, 7);
            assert_eq!(input.task.request_text(), text);
            assert_eq!(input.operation, policy.unwrap_or(Operation::Implement));
        }
    }

    #[test]
    fn manual_pin_and_eevee_mode_respect_explicit_readonly_during_automatic_selection() {
        let mut app = app();
        app.apply_execution_action(execution::Action::SelectForm(3));
        app.input = "Read the project source files.".into();
        app.submit_request();
        assert_eq!((app.form(), app.lead.auto_form), (3, false));
        app.pending_request = None;
        app.apply_execution_action(execution::Action::SelectForm(0));
        app.planning.set_operation(Operation::Plan);
        assert_eq!((app.form(), app.lead.auto_form), (0, true));
        app.input = "Create a code file.".into();
        app.submit_request();
        assert_eq!((app.form(), app.lead.auto_form), (0, true));
        assert_eq!(app.planning.operation, Operation::Plan);
        app.pending_request = None;
        app.apply_execution_action(execution::Action::SelectForm(0));
        app.planning.set_operation(Operation::Review);
        app.input = "Implement the code feature.".into();
        app.submit_request();
        assert_eq!((app.form(), app.planning.operation), (7, Operation::Review));
    }

    #[test]
    fn form_choice_and_restore_preserve_explicit_readonly_policy() {
        let mut app = app();
        app.planning.set_operation(Operation::Plan);
        app.apply_execution_action(execution::Action::SelectForm(3));
        assert_eq!(app.planning.operation, Operation::Plan);
        app.planning.set_operation(Operation::Review);
        let snapshot = app.snapshot("/workspace");
        app.load_task(snapshot);
        assert_eq!(app.planning.operation, Operation::Review);
        assert!(!app.lead.auto_form);
    }

    #[test]
    fn pending_busy_cancel_and_approval_keep_frozen_owner_and_future_draft() {
        for state in 0..4 {
            let mut app = app();
            app.input = "Create a code file.".into();
            app.submit_request();
            let owner = app.attribution();
            let snapshot = app.snapshot("/workspace");
            if state != 0 {
                app.pending_request = None;
                app.planning.command("new", true, &snapshot, &[], None);
            }
            if state == 2 {
                app.planning.cancelling = true;
            }
            if state == 3 {
                app.planning.request = Some(providers::Prompt {
                    arrival: providers::approval_arrival(),
                    request_id: serde_json::json!("exact-owner"),
                    thread_id: "owned".into(),
                    turn_id: "turn".into(),
                    item_id: "item".into(),
                    kind: providers::PromptKind::Questions(Vec::new()),
                });
            }
            app.input = "Review the code changes independently.".into();
            app.cursor = 8;
            app.submit_request();
            assert_eq!(app.attribution(), owner);
            assert_eq!(
                (app.input.as_str(), app.cursor),
                ("Review the code changes independently.", 8)
            );
            if state != 0 {
                assert!(app.pending_request.is_none());
                assert_eq!(app.planning.owner(), Some(&owner));
            }
        }
    }

    #[test]
    fn canonical_mode_hydrates_without_pinning_and_survives_roster_merging() {
        for auto_form in [false, true] {
            let mut app = app();
            let mut snapshot = store::Snapshot::empty("/workspace".into());
            snapshot.id = 7;
            snapshot.form = 1;
            snapshot.auto_form = auto_form;
            app.load_task(snapshot);
            assert_eq!((app.form(), app.lead.auto_form), (1, auto_form));
            assert_eq!(app.snapshot("/workspace").auto_form, auto_form);
            app.party = vec![
                app.lead.clone(),
                store::Partner {
                    id: 2,
                    auto_form: !auto_form,
                    ..Default::default()
                },
            ];
            app.request_lead(2);
            assert_eq!(app.lead.auto_form, !auto_form);
            app.request_lead(1);
            assert_eq!((app.form(), app.lead.auto_form), (1, auto_form));
            app.lead.auto_form = !auto_form;
            assert_eq!(app.partners()[0].auto_form, !auto_form);
        }
    }

    #[test]
    fn evolution_preserves_selected_route_and_ignores_saved_specialist_pairings() {
        let mut app = app();
        app.planning.agent = Agent::Compatible;
        app.planning.selection = ModelSelection {
            model: Some("chosen/model".into()),
            effort: Some("high".into()),
        };
        let selection = app.planning.selection.clone();
        app.records.loaded = true;
        app.records
            .snapshot
            .pairings
            .push(crate::workspace::records::Pairing {
                partner: app.lead.id,
                form: 3,
                agent: Agent::OpenAI,
                selection: ModelSelection {
                    model: Some("must-not-activate".into()),
                    effort: None,
                },
            });
        app.planning.thread_id = Some("opaque-old-role".into());
        app.input = "Create a code file.".into();
        app.submit_request();
        assert_eq!(app.planning.agent, Agent::Compatible);
        assert_eq!(app.planning.selection, selection);
        app.records_restore_pairing();
        assert_eq!(app.planning.agent, Agent::Compatible);
        assert_eq!(app.planning.selection, selection);
        assert!(app.planning.thread_id.is_none());
        assert!(app.evolution.transition.unwrap().fresh);
        assert!(!app.pending_request.unwrap().1);
    }
}
