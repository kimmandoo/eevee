use super::*;
use crate::app::{Overlay, StorageState, evolution};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

fn app(size: (u16, u16)) -> App {
    let mut app = App::new("/workspace".into(), size.0, size.1);
    app.task_id = 7;
    app.storage = StorageState::Saved;
    app.planning.reset(7);
    app.planning.preference_event(Ok(None));
    app.motion_visuals = true;
    app
}

#[test]
fn bounded_motion_finishes_static_without_an_idle_deadline_or_replay() {
    for size in [(120, 36), (60, 18)] {
        let mut app = app(size);
        app.set_form(3);
        let now = Instant::now();
        app.arm_picker_motion(now);
        assert_eq!(app.evolution_frame(), Some(0));
        assert_eq!(app.evolution_wait(now), Some(STEP));
        assert!(!app.tick_evolution(now + Duration::from_millis(79)));
        assert!(app.tick_evolution(now + STEP));
        assert_eq!(app.evolution_frame(), Some(1));
        assert!(app.tick_evolution(now + STEP * 2));
        assert_eq!(app.evolution_frame(), Some(2));
        assert!(app.tick_evolution(now + END));
        assert_eq!(app.evolution_frame(), None);
        assert!(
            app.evolution.transition.is_some(),
            "complete factual receipt remains"
        );
        assert_eq!(app.evolution_wait(now + END), None);
        assert!(!app.tick_evolution(now + Duration::from_secs(60)));
    }
}

#[test]
fn only_actual_picker_commits_arm_and_same_form_or_restoration_never_replays() {
    let mut app = app((120, 36));
    app.set_form(3);
    assert_eq!(
        app.evolution_frame(),
        None,
        "programmatic and formation switches stay static"
    );
    app.overlay = Some(Overlay::Forms);
    app.picker_select_form(4);
    assert_eq!(app.form(), 4);
    assert!(app.evolution_frame().is_some());
    app.evolution.motion = None;
    app.picker_select_form(4);
    assert!(app.evolution.motion.is_none());
    let snapshot = app.snapshot("/workspace");
    app.load_task(snapshot);
    assert!(app.evolution.transition.is_none());
    assert!(app.evolution_wait(Instant::now()).is_none());
    app.picker_return_to_eevee();
    assert_eq!(app.form(), 0);
    assert!(app.lead().auto_form);
    assert!(app.planning.pending.is_none());
}

#[test]
fn preference_override_text_route_and_unseen_surfaces_are_permanently_static() {
    for route in 0..5 {
        let mut app = app((120, 36));
        app.set_form(3);
        match route {
            0 => app.planning.set_reduced_motion(true),
            1 => app.motion_override = Some(false),
            2 => app.motion_visuals = false,
            3 => app.overlay = Some(Overlay::Forms),
            _ => app.width = 59,
        }
        app.arm_picker_motion(Instant::now());
        assert!(app.evolution.motion.is_none());
        app.overlay = None;
        app.width = 120;
        assert!(!app.tick_evolution(Instant::now() + END));
        assert!(app.evolution.transition.is_some());
    }
}

#[test]
fn key_paste_and_resize_stop_motion_without_losing_editor_input() {
    for event in [
        Event::Paste("한글e\u{301}".into()),
        Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
    ] {
        let mut app = app((60, 18));
        app.set_form(3);
        app.input = "ab".into();
        app.cursor = 1;
        app.evolution.popup_shown = true;
        app.arm_picker_motion(Instant::now());
        let paste = matches!(event, Event::Paste(_));
        app.update(event);
        assert_eq!(app.input, if paste { "a한글e\u{301}b" } else { "axb" });
        assert!(app.evolution.transition.is_none());
        assert!(app.evolution_wait(Instant::now()).is_none());
        assert!(
            app.planning.pending.is_none(),
            "skip input cannot dispatch or grant"
        );
    }
    let mut app = app((120, 36));
    app.set_form(3);
    app.arm_picker_motion(Instant::now());
    let mut release = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    app.update(Event::Key(release));
    assert!(app.evolution.motion.is_some());
    assert!(
        app.update(Event::Resize(120, 36)),
        "same-size resize must repaint the completed static result"
    );
    assert!(app.evolution.motion.is_none());
    app.arm_picker_motion(Instant::now());
    app.update(Event::Resize(60, 18));
    assert!(app.evolution.motion.is_none());
    assert!(
        app.evolution.transition.is_some(),
        "resize keeps the complete static receipt"
    );
    app.evolution = evolution::State::default();
    assert!(app.evolution_wait(Instant::now()).is_none());
}

#[test]
fn busy_owner_refuses_picker_without_animation_or_queued_replay() {
    let mut app = app((120, 36));
    app.planning.work = Some(crate::store::WorkClaim {
        task_id: app.task_id,
        owner: app.attribution(),
        request_id: "picker-busy-owner".into(),
        writing: false,
    });
    let before = app.form();
    app.picker_select_form(3);
    assert_eq!(app.form(), before);
    assert!(app.evolution.motion.is_none());
    app.planning.work = None;
    assert!(!app.tick_evolution(Instant::now() + END));
    assert_eq!(app.form(), before);
}

#[test]
fn rapid_changes_replace_the_single_timer_and_priority_loss_never_replays_it() {
    use crate::{
        app::planning::Item,
        providers::{self, FilePhase, ItemKind},
    };
    use std::sync::Arc;
    let mut app = app((120, 36));
    let now = Instant::now();
    app.set_form(3);
    app.arm_picker_motion(now);
    app.set_form(4);
    app.arm_picker_motion(now + STEP);
    assert_eq!(app.evolution.transition.unwrap().to, 4);
    assert_eq!(app.evolution_wait(now + STEP), Some(STEP));
    app.planning.submitted_request = Arc::from("Create generated/live.py");
    app.planning.items.push(Item {
        thread_id: String::new(),
        turn_id: "turn".into(),
        item_id: "file".into(),
        kind: ItemKind::File(FilePhase::Generating),
        text: "ACTUAL_LIVE_CODE".into(),
        complete: false,
        request: Arc::clone(&app.planning.submitted_request),
        owner: None,
        status: None,
    });
    assert!(app.tick_evolution(now + STEP));
    assert!(app.evolution.motion.is_none());
    app.planning.items[0].kind = ItemKind::File(FilePhase::Applied);
    assert!(!app.tick_evolution(now + END));
    assert!(
        app.evolution_popup_ready(),
        "only the static receipt can be deferred"
    );
    app.arm_picker_motion(now + END);
    app.planning.request = Some(providers::Prompt {
        arrival: providers::approval_arrival(),
        request_id: serde_json::json!("owned"),
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: "item".into(),
        kind: providers::PromptKind::Questions(Vec::new()),
    });
    assert!(app.tick_evolution(now + END));
    app.update(Event::Paste("literal no grant".into()));
    assert!(app.planning.pending.is_none());
    assert!(app.planning.request.is_some(), "skip never answers consent");
    app.planning.request = None;
    assert!(!app.tick_evolution(now + END * 2));
    assert!(app.evolution_wait(now + END * 2).is_none());
}

#[test]
fn static_receipt_deadline_starts_on_first_render_and_does_not_extend_or_replay() {
    for reduced_motion in [false, true] {
        let mut app = app((60, 18));
        app.planning.set_reduced_motion(reduced_motion);
        app.motion_visuals = !reduced_motion;
        app.set_form(3);
        let now = Instant::now();
        assert_eq!(app.evolution_wait(now), None, "unseen receipt has no timer");
        assert!(!app.tick_evolution(now + POPUP_LIFETIME));
        assert!(app.evolution.transition.is_some());

        let shown = now + POPUP_LIFETIME;
        app.mark_evolution_popup_shown(shown);
        assert_eq!(app.evolution_wait(shown), Some(POPUP_LIFETIME));
        app.mark_evolution_popup_shown(shown + STEP);
        assert_eq!(
            app.evolution_wait(shown + STEP),
            Some(POPUP_LIFETIME - STEP),
            "redraw extended the receipt's lifetime"
        );
        assert!(!app.tick_evolution(shown + POPUP_LIFETIME - STEP));
        assert!(app.tick_evolution(shown + POPUP_LIFETIME));
        assert!(app.evolution.transition.is_none());
        assert!(!app.evolution.popup_shown);
        assert_eq!(app.evolution_wait(shown + POPUP_LIFETIME), None);
        assert!(!app.tick_evolution(shown + POPUP_LIFETIME * 2));
    }
}

#[test]
fn receipt_expiry_retains_idle_wake_after_picker_motion_finishes() {
    let mut app = app((60, 18));
    let now = Instant::now();
    app.set_form(3);
    app.arm_picker_motion(now);
    app.mark_evolution_popup_shown(now);
    assert_eq!(app.evolution_wait(now), Some(STEP));
    assert!(app.tick_evolution(now + END));
    assert_eq!(app.evolution_frame(), None);
    assert_eq!(
        app.evolution_wait(now + END),
        Some(POPUP_LIFETIME - END),
        "static result lost its idle expiration wake"
    );
    assert!(app.tick_evolution(now + POPUP_LIFETIME));
    assert!(app.evolution.transition.is_none());
}

#[test]
fn replacement_receipt_gets_a_new_lifetime_only_after_it_is_rendered() {
    let mut app = app((60, 18));
    let now = Instant::now();
    app.set_form(3);
    app.mark_evolution_popup_shown(now);
    app.set_form(4);
    assert_eq!(app.evolution_wait(now), None);
    assert!(!app.tick_evolution(now + POPUP_LIFETIME));
    assert_eq!(app.evolution.transition.unwrap().to, 4);
    let shown = now + POPUP_LIFETIME * 2;
    app.mark_evolution_popup_shown(shown);
    assert_eq!(app.evolution_wait(shown), Some(POPUP_LIFETIME));
    assert!(app.tick_evolution(shown + POPUP_LIFETIME));
    assert!(app.evolution.transition.is_none());
}

#[test]
fn receipt_expiry_while_consent_is_pending_cannot_answer_or_replay() {
    let mut app = app((60, 18));
    let now = Instant::now();
    app.set_form(3);
    app.input = "future 界draft".into();
    app.cursor = 7;
    app.mark_evolution_popup_shown(now);
    app.planning.request = Some(crate::providers::Prompt {
        arrival: crate::providers::approval_arrival(),
        request_id: serde_json::json!("owned"),
        thread_id: "thread".into(),
        turn_id: "turn".into(),
        item_id: "item".into(),
        kind: crate::providers::PromptKind::Questions(Vec::new()),
    });
    assert!(!app.evolution_popup_ready());
    assert!(app.tick_evolution(now + POPUP_LIFETIME));
    assert!(app.planning.request.is_some());
    assert!(app.planning.pending.is_none());
    assert!(app.planning.decision.is_none());
    assert_eq!((app.input.as_str(), app.cursor), ("future 界draft", 7));
    app.planning.request = None;
    assert!(!app.evolution_popup_ready());
    assert_eq!(app.evolution_wait(now + POPUP_LIFETIME), None);
}
