//! Bounded coalescing for local draft/caret/navigation updates only.
use std::time::{Duration, Instant};

use super::{App, StorageState};

const QUIET: Duration = Duration::from_millis(500);
const MAX_PENDING: Duration = Duration::from_secs(2);

pub(super) struct Window {
    first: Instant,
    last: Instant,
}

impl Window {
    fn deadline(&self) -> Instant {
        (self.last + QUIET).min(self.first + MAX_PENDING)
    }
}

impl App {
    /// Time until a pending snapshot can be submitted. In-flight writes retain
    /// ownership until their acknowledgement; failed/loading storage cannot send.
    pub fn save_wait(&self, now: Instant) -> Option<Duration> {
        if !self.save_requested
            || self.transitioning
            || self.save_inflight.is_some()
            || matches!(
                self.storage,
                StorageState::Failed(_) | StorageState::Loading
            )
        {
            return None;
        }
        Some(self.autosave.as_ref().map_or(Duration::ZERO, |window| {
            window.deadline().saturating_duration_since(now)
        }))
    }

    /// Check before allocating/cloning a task snapshot, not after constructing it.
    pub fn save_due(&self, now: Instant) -> bool {
        self.save_wait(now).is_some_and(|wait| wait.is_zero())
    }

    /// Called only after the store worker accepts the current task snapshot.
    pub fn save_submitted(&mut self) {
        self.save_inflight = Some(self.revision);
        self.save_requested = false;
        self.autosave = None;
        self.refresh_save_state();
    }

    /// Promote pending edits to an immediate write without applying the editor,
    /// changing task ownership, or dispatching an action after acknowledgement.
    pub(super) fn flush_save(&mut self) {
        self.autosave = None;
        self.refresh_save_state();
    }

    pub(super) fn changed_implicitly(&mut self, now: Instant) {
        self.revision = self.revision.saturating_add(1);
        if let Some(window) = &mut self.autosave {
            window.last = now;
        } else if !self.save_requested {
            self.autosave = Some(Window {
                first: now,
                last: now,
            });
        }
        // A semantic action already awaiting submission must never be delayed
        // by a subsequent caret move or keystroke.
        self.save_requested = true;
        self.refresh_save_state();
    }

    pub(super) fn refresh_save_state(&mut self) {
        if matches!(
            self.storage,
            StorageState::Failed(_) | StorageState::Loading
        ) {
            return;
        }
        self.storage = if self.save_inflight.is_some() {
            StorageState::Saving
        } else if self.save_requested {
            if self.autosave.is_some() {
                StorageState::Pending
            } else {
                StorageState::Saving
            }
        } else {
            StorageState::Saved
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Pane, execution};
    use crate::store;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    fn project(tasks: Vec<store::Snapshot>) -> store::Project {
        store::Project {
            active_id: tasks.first().map(|task| task.id),
            tasks,
            partners: vec![store::Partner::default()],
            usage_bytes: 0,
        }
    }

    fn editor() -> App {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.store_event(store::Event::Loaded(project(Vec::new())));
        app
    }

    fn key(app: &mut App, code: KeyCode, now: Instant) {
        app.update_at(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)), now);
    }

    fn control(app: &mut App, character: char, now: Instant) {
        app.update_at(
            Event::Key(KeyEvent::new(
                KeyCode::Char(character),
                KeyModifiers::CONTROL,
            )),
            now,
        );
    }

    fn submit(app: &mut App, now: Instant) -> store::Snapshot {
        assert!(app.save_due(now));
        let mut snapshot = app.snapshot("/workspace");
        snapshot.id = snapshot.id.max(7);
        app.save_submitted();
        assert!(!app.save_due(now));
        snapshot
    }

    #[test]
    fn quiet_draft_and_caret_changes_wait_until_the_exact_boundary() {
        let now = Instant::now();
        let mut app = editor();
        app.update_at(Event::Paste("hello".into()), now);
        assert_eq!(app.storage, StorageState::Pending);
        assert!(!app.authentication_ready());
        assert_eq!(app.save_wait(now), Some(QUIET));
        assert!(!app.save_due(now + QUIET - Duration::from_nanos(1)));
        assert!(app.save_due(now + QUIET));

        key(&mut app, KeyCode::Left, now + Duration::from_millis(300));
        assert!(!app.save_due(now + Duration::from_millis(799)));
        let snapshot = submit(&mut app, now + Duration::from_millis(800));
        assert_eq!(app.storage, StorageState::Saving);
        assert!(app.save_wait(now + MAX_PENDING).is_none());
        let draft = &snapshot.partner_drafts[&app.lead().id].drafts[0];
        assert_eq!((&draft.text, draft.cursor), (&"hello".to_owned(), 4));
        app.store_event(store::Event::Saved(snapshot));
        assert_eq!(app.storage, StorageState::Saved);
        assert_eq!(app.saved_revision, app.revision);
        assert!(!app.save_requested);
    }

    #[test]
    fn twenty_keys_at_four_hertz_collapse_to_three_bounded_writes() {
        let start = Instant::now();
        let mut app = editor();
        let mut writes = Vec::new();
        for index in 0..20 {
            let now = start + Duration::from_millis(index * 250);
            key(&mut app, KeyCode::Char('x'), now);
            if app.save_due(now) {
                let snapshot = submit(&mut app, now);
                writes.push((now.duration_since(start), app.input.len()));
                app.store_event(store::Event::Saved(snapshot));
            }
        }
        assert_eq!(
            writes,
            [
                (Duration::from_millis(2000), 9),
                (Duration::from_millis(4250), 18),
            ]
        );
        assert!(!app.save_due(start + Duration::from_millis(5249)));
        let snapshot = submit(&mut app, start + Duration::from_millis(5250));
        assert_eq!(
            snapshot.partner_drafts[&app.lead().id].drafts[0].text,
            "xxxxxxxxxxxxxxxxxxxx"
        );
        app.store_event(store::Event::Saved(snapshot));
        assert_eq!(app.storage, StorageState::Saved);
        assert!(app.planning.pending.is_none());
        assert!(app.execution.active.is_none());
        assert!(app.pending_command.is_none());
    }

    #[test]
    fn continuous_edits_cannot_extend_the_maximum_pending_deadline() {
        let start = Instant::now();
        let mut app = editor();
        for millis in [0, 400, 800, 1200, 1600, 1999] {
            key(
                &mut app,
                KeyCode::Char('x'),
                start + Duration::from_millis(millis),
            );
        }
        assert!(!app.save_due(start + MAX_PENDING - Duration::from_nanos(1)));
        assert!(app.save_due(start + MAX_PENDING));
    }

    #[test]
    fn explicit_apply_or_retain_is_immediate_and_not_redebounced() {
        let now = Instant::now();
        for apply in [false, true] {
            let mut app = editor();
            app.update_at(Event::Paste("draft".into()), now);
            key(&mut app, KeyCode::Left, now);
            if apply {
                key(&mut app, KeyCode::Enter, now);
                assert_eq!(app.goal, "draft");
            } else {
                control(&mut app, 's', now);
                assert_eq!(app.drafts[0].text, "draft");
                assert_eq!(app.drafts[0].cursor, 4);
            }
            assert!(app.save_due(now));
            key(&mut app, KeyCode::Tab, now + Duration::from_millis(1));
            assert!(app.save_due(now + Duration::from_millis(1)));
            assert_eq!(app.storage, StorageState::Saving);
        }
    }

    #[test]
    fn navigation_is_deferred_and_nonediting_save_flushes_it() {
        let now = Instant::now();
        let mut app = editor();
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        app.load_task(task);
        app.editing = false;
        let old_focus = app.focus;
        key(&mut app, KeyCode::Tab, now);
        assert_ne!(app.focus, old_focus);
        assert_eq!(app.storage, StorageState::Pending);
        assert!(!app.save_due(now));
        control(&mut app, 's', now);
        assert!(app.save_due(now));
        let snapshot = submit(&mut app, now);
        assert_eq!(
            snapshot.navigation.focus,
            match app.focus {
                Pane::Companion => 0,
                Pane::Workspace => 1,
                Pane::Inspector => 2,
            }
        );
    }

    #[test]
    fn immediate_quit_waits_for_draft_and_caret_ack_before_restart() {
        let now = Instant::now();
        let mut app = editor();
        app.update_at(Event::Paste("한글 draft".into()), now);
        key(&mut app, KeyCode::Left, now);
        let input = app.input.clone();
        let cursor = app.cursor;
        control(&mut app, 'c', now);
        key(&mut app, KeyCode::Right, now);
        key(&mut app, KeyCode::Enter, now);
        assert!(app.quit_after_save);
        assert!(!app.quit);
        let snapshot = submit(&mut app, now);
        assert!(!app.quit);
        app.store_event(store::Event::Saved(snapshot.clone()));
        assert!(app.quit);

        let mut restarted = App::new("/workspace".into(), 120, 36);
        restarted.store_event(store::Event::Loaded(project(vec![snapshot])));
        assert_eq!((restarted.input, restarted.cursor), (input, cursor));
        assert!(restarted.editing);
    }

    #[test]
    fn task_transition_flushes_but_keeps_ownership_until_acknowledged() {
        let now = Instant::now();
        for action in [execution::Action::SwitchTask(8), execution::Action::NewTask] {
            let mut app = editor();
            app.update_at(Event::Paste("retain me".into()), now);
            key(&mut app, KeyCode::Left, now);
            app.apply_execution_action(action);
            assert!(app.save_due(now));
            assert!(!app.transitioning);
            assert!(app.switch_task.is_none());
            assert_eq!(app.input, "retain me");
            let snapshot = submit(&mut app, now);
            let cursor = app.cursor;
            app.store_event(store::Event::Saved(snapshot));
            app.apply_execution_action(execution::Action::SwitchTask(8));
            assert_eq!(app.switch_task, Some(8));
            assert!(app.transitioning);
            let saved = app.tasks.iter().find(|task| task.id == 7).unwrap();
            assert_eq!(
                saved.partner_drafts[&app.lead().id].drafts[0].cursor,
                cursor
            );
        }
    }

    #[test]
    fn partner_transition_flushes_and_retains_the_outgoing_caret() {
        let now = Instant::now();
        let mut app = editor();
        let outgoing = app.lead().id;
        app.party.push(store::Partner {
            id: 2,
            ..store::Partner::default()
        });
        app.update_at(Event::Paste("partner draft".into()), now);
        key(&mut app, KeyCode::Left, now);
        let cursor = app.cursor;
        app.request_lead(2);
        assert!(app.save_due(now));
        let snapshot = submit(&mut app, now);
        assert_eq!(snapshot.lead_partner, 2);
        assert_eq!(
            snapshot.partner_drafts[&outgoing].drafts[0].text,
            "partner draft"
        );
        assert_eq!(snapshot.partner_drafts[&outgoing].drafts[0].cursor, cursor);
        app.store_event(store::Event::Saved(snapshot));
        app.request_lead(outgoing);
        assert_eq!(app.input, "partner draft");
        assert_eq!(app.cursor, cursor);
    }

    #[test]
    fn initial_inflight_insert_and_stale_ack_cannot_overwrite_newer_draft() {
        let now = Instant::now();
        let mut app = editor();
        key(&mut app, KeyCode::Char('a'), now);
        let first = submit(&mut app, now + QUIET);
        assert_eq!(app.task_id, 0);
        key(&mut app, KeyCode::Char('b'), now + QUIET);
        assert!(!app.save_due(now + MAX_PENDING + QUIET));
        assert_eq!(app.storage, StorageState::Saving);
        app.store_event(store::Event::Saved(first.clone()));
        assert_eq!(app.task_id, 7);
        assert_eq!(app.input, "ab");
        assert_eq!(app.storage, StorageState::Pending);
        assert!(!app.save_due(now + QUIET));
        let newest = submit(&mut app, now + QUIET + QUIET);
        assert_eq!(newest.id, 7);
        app.store_event(store::Event::Saved(newest.clone()));
        app.store_event(store::Event::Saved(first));
        assert_eq!(app.input, "ab");
        assert_eq!(app.saved_revision, newest.revision);
        assert_eq!(app.tasks[0].revision, newest.revision);
        assert_eq!(app.storage, StorageState::Saved);
    }

    #[test]
    fn failed_ack_retains_pending_bytes_and_retry_does_not_dispatch_work() {
        let now = Instant::now();
        let mut app = editor();
        app.update_at(Event::Paste("recover bytes".into()), now);
        key(&mut app, KeyCode::Left, now);
        let snapshot = submit(&mut app, now + QUIET);
        app.store_event(store::Event::Failed {
            revision: Some(snapshot.revision),
            message: "disk full".into(),
        });
        assert_eq!(app.input, "recover bytes");
        assert_eq!(app.cursor, 12);
        assert!(app.save_requested);
        assert!(matches!(app.storage, StorageState::Failed(_)));
        assert!(app.save_wait(now + MAX_PENDING).is_none());
        app.store_event(store::Event::Loaded(project(Vec::new())));
        assert!(app.save_due(now + MAX_PENDING));
        let recovered = submit(&mut app, now + MAX_PENDING);
        app.store_event(store::Event::Saved(recovered));
        assert_eq!(app.storage, StorageState::Saved);
        assert!(app.planning.pending.is_none());
        assert!(app.execution.active.is_none());
        assert!(app.pending_command.is_none());
    }

    #[test]
    fn loading_retains_the_debounce_and_quit_flushes_before_load_finishes() {
        let now = Instant::now();
        let mut app = App::new("/workspace".into(), 120, 36);
        app.update_at(Event::Paste("startup draft".into()), now);
        assert!(app.save_wait(now + MAX_PENDING).is_none());
        assert_eq!(app.storage, StorageState::Loading);
        control(&mut app, 'c', now);
        key(&mut app, KeyCode::Right, now);
        key(&mut app, KeyCode::Enter, now);
        assert!(app.quit_after_save);
        assert!(!app.quit);
        let mut existing = store::Snapshot::empty("/workspace".into());
        existing.id = 9;
        existing.goal = "prior task".into();
        app.store_event(store::Event::Loaded(project(vec![existing])));
        assert_eq!(app.task_id, 0);
        assert_eq!(app.input, "startup draft");
        let snapshot = submit(&mut app, now);
        app.store_event(store::Event::Saved(snapshot));
        assert!(app.quit);
        assert_eq!(app.tasks[0].goal, "prior task");
    }

    #[test]
    fn initial_load_does_not_skip_a_quiet_draft_deadline() {
        let now = Instant::now();
        let mut app = App::new("/workspace".into(), 120, 36);
        key(&mut app, KeyCode::Char('x'), now);
        app.store_event(store::Event::Loaded(project(Vec::new())));
        assert_eq!(app.storage, StorageState::Pending);
        assert!(!app.save_due(now + QUIET - Duration::from_nanos(1)));
        assert!(app.save_due(now + QUIET));
    }

    #[test]
    fn older_failed_write_releases_ownership_without_dropping_newer_edits() {
        let now = Instant::now();
        let mut app = editor();
        key(&mut app, KeyCode::Char('a'), now);
        let first = submit(&mut app, now + QUIET);
        key(&mut app, KeyCode::Char('b'), now + QUIET);
        app.store_event(store::Event::Failed {
            revision: Some(first.revision),
            message: "older write failed".into(),
        });
        assert_eq!(app.input, "ab");
        assert_eq!(app.task_id, 0);
        assert_eq!(app.storage, StorageState::Pending);
        assert!(app.save_requested);
        assert!(!app.save_due(now + QUIET));
        let recovered = submit(&mut app, now + QUIET + QUIET);
        app.store_event(store::Event::Saved(recovered));
        assert_eq!(app.input, "ab");
        assert_eq!(app.storage, StorageState::Saved);
    }

    #[test]
    fn memory_draft_and_caret_autosaves_are_deferred_but_explicit_save_flushes() {
        let now = Instant::now();
        let mut app = editor();
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        app.load_task(task);
        app.memory_loading = false;
        app.overlay = Some(crate::app::Overlay::Memory);
        assert!(app.memory_command("create-memory"));
        let initial = submit(&mut app, now);
        app.store_event(store::Event::Saved(initial));

        app.update_at(Event::Paste("memory note".into()), now);
        assert_eq!(app.storage, StorageState::Pending);
        assert!(!app.save_due(now));
        key(&mut app, KeyCode::Left, now + Duration::from_millis(250));
        assert!(!app.save_due(now + QUIET));
        control(&mut app, 's', now + Duration::from_millis(250));
        assert!(app.save_due(now + Duration::from_millis(250)));
        let snapshot = submit(&mut app, now + Duration::from_millis(250));
        assert_eq!(snapshot.memory_draft["text"], "memory note");
        assert_eq!(snapshot.memory_draft["cursor"], 10);
        assert!(app.memory.pending.is_none());
        assert!(!app.memory.saving);
    }
}
