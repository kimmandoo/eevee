//! Explicit acknowledgement of exact unsaved observations after provider termination.
use super::*;
use planning::journal::DiscardSnapshot;

pub struct Review {
    pub owner: store::Attribution,
    pub task: i64,
    pub request_id: String,
    pub count: usize,
    pub ids: Vec<String>,
    pub error: String,
    pub reviewed: bool,
    pub affirmative: bool,
    snapshot: DiscardSnapshot,
}

impl Review {
    fn new(snapshot: DiscardSnapshot) -> Self {
        let ids = snapshot.ids();
        Self {
            owner: snapshot.claim.owner.clone(),
            task: snapshot.claim.task_id,
            request_id: snapshot.claim.request_id.clone(),
            count: ids.len(),
            ids,
            error: snapshot.error(),
            reviewed: false,
            affirmative: false,
            snapshot,
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "Owner: {} · #{} · task {}\nRequest: {}\nAcknowledge {} unsaved failed journal write(s).\nProvider terminal; exact failed Put writes only.\nKeep committed records; retain error/coverage gap.\nThen release this request's claim only.\nNo retry, purge or provider replay.\nDetails: exact IDs and storage errors.",
            store::Attribution::label(Some(&self.owner)),
            self.owner.partner,
            self.task,
            self.request_id,
            self.count,
        )
    }

    fn exact_identity(&self) -> bool {
        self.owner == self.snapshot.claim.owner
            && self.task == self.snapshot.claim.task_id
            && self.request_id == self.snapshot.claim.request_id
            && self.snapshot.matches_ids(&self.ids)
            && self.count == self.ids.len()
    }
}

impl App {
    pub fn request_journal_discard(&mut self) {
        match self.planning.journal_discard_snapshot() {
            Ok(snapshot) => {
                self.cooperation.journal_discard = Some(Review::new(snapshot));
                self.cooperation_ui.journal_discard_details = false;
                self.cooperation_ui.journal_discard_scroll = 0;
                self.cooperation_ui.journal_discard_max = 0;
            }
            Err(error) => {
                self.notice = format!(
                    "Journal discard refused: {error} Retained errors and drafts are unchanged; /journal-retry retries storage only."
                );
            }
        }
    }

    /// Modal routing precedes editors; release/repeat/modifier keys grant nothing.
    pub fn journal_discard_key(&mut self, key: KeyEvent) -> bool {
        let Some(review) = self.cooperation.journal_discard.as_mut() else {
            return false;
        };
        if key.kind != KeyEventKind::Press {
            return true;
        }
        if key.code == KeyCode::Esc || key.modifiers.is_empty() && key.code == KeyCode::Char('n') {
            self.cooperation.journal_discard = None;
            return true;
        }
        if !key.modifiers.is_empty() {
            return true;
        }
        if key.code == KeyCode::Char('d') {
            self.cooperation_ui.journal_discard_details =
                !self.cooperation_ui.journal_discard_details;
            review.reviewed = false;
            review.affirmative = false;
            return true;
        }
        if self.cooperation_ui.journal_discard_details {
            review.reviewed = false;
            review.affirmative = false;
            match key.code {
                KeyCode::PageUp => {
                    self.cooperation_ui.journal_discard_scroll =
                        self.cooperation_ui.journal_discard_scroll.saturating_sub(3)
                }
                KeyCode::PageDown => {
                    self.cooperation_ui.journal_discard_scroll = self
                        .cooperation_ui
                        .journal_discard_scroll
                        .saturating_add(3)
                        .min(self.cooperation_ui.journal_discard_max)
                }
                KeyCode::Home => self.cooperation_ui.journal_discard_scroll = 0,
                KeyCode::End => {
                    self.cooperation_ui.journal_discard_scroll =
                        self.cooperation_ui.journal_discard_max
                }
                _ => {}
            }
            return true;
        }
        if key.code == KeyCode::Enter && !review.affirmative {
            self.cooperation.journal_discard = None;
            return true;
        }
        match key.code {
            KeyCode::Left => review.affirmative = false,
            KeyCode::Right if review.reviewed => review.affirmative = true,
            KeyCode::Char('y') if review.reviewed => review.affirmative = true,
            _ => {}
        }
        if !matches!(key.code, KeyCode::Enter | KeyCode::Char('y'))
            || !review.reviewed
            || !review.affirmative
        {
            return true;
        }
        let mut review = self
            .cooperation
            .journal_discard
            .take()
            .expect("active discard review");
        let result = if self.task_id != review.task || !review.exact_identity() {
            Err(
                "The displayed task/owner/request no longer matches the reviewed failure snapshot."
                    .into(),
            )
        } else if let Some(state) = self.planning_state_mut(review.owner.partner) {
            state.acknowledge_journal_discard(&review.snapshot)
        } else {
            Err("The exact request owner is no longer present.".into())
        };
        match result {
            Ok(()) => {
                self.notice = "Unsaved journal writes explicitly acknowledged with an unverified coverage gap; exact request ownership will settle normally. No provider replay, retry or purge.".into();
            }
            Err(error) => {
                review.reviewed = false;
                review.affirmative = false;
                self.cooperation_ui.journal_discard_details = true;
                self.cooperation_ui.journal_discard_scroll = 0;
                self.cooperation_ui.journal_discard_max = 0;
                review.error = format!(
                    "{}\nDiscard refused: {error}\nClose and request /journal-discard again to review current failures.",
                    review.snapshot.error()
                );
                self.notice = format!(
                    "Journal discard refused: {error} Original errors and drafts retained."
                );
                self.cooperation.journal_discard = Some(review);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.task_id = 7;
        app.planning = planning::State::terminal_failure_for_test(2);
        app.input = "Keep this follow-up 漢字".into();
        app.cursor = 5;
        app.editing = true;
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        assert!(app.journal_discard_key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    #[test]
    fn discard_starts_neutral_and_unreviewed_keys_or_dismissal_keep_errors_and_drafts() {
        for code in [KeyCode::Enter, KeyCode::Esc, KeyCode::Char('n')] {
            let mut app = app();
            let before = (
                app.input.clone(),
                app.cursor,
                app.revision,
                app.goal.clone(),
            );
            let claim = app.planning.work.clone();
            app.request_journal_discard();
            let review = app.cooperation.journal_discard.as_ref().unwrap();
            assert!(!review.reviewed && !review.affirmative);
            press(&mut app, KeyCode::Char('y'));
            assert!(app.cooperation.journal_discard.is_some());
            assert!(app.planning.journal_failed() && app.planning.work_pending.is_none());
            press(&mut app, code);
            assert!(app.cooperation.journal_discard.is_none());
            assert_eq!(app.planning.work, claim);
            assert!(app.planning.journal_failed() && app.planning.work_pending.is_none());
            assert_eq!((app.input, app.cursor, app.revision, app.goal), before);
        }
    }

    #[test]
    fn reviewed_exact_acknowledgement_preserves_metadata_and_queues_only_owned_release() {
        let mut app = app();
        let before = (
            app.input.clone(),
            app.cursor,
            app.revision,
            app.goal.clone(),
            app.drafts.clone(),
        );
        let claim = app.planning.work.clone().unwrap();
        app.request_journal_discard();
        app.cooperation.journal_discard.as_mut().unwrap().reviewed = true;
        press(&mut app, KeyCode::Right);
        assert!(
            app.planning.journal_failed(),
            "selection alone is not acknowledgement"
        );
        press(&mut app, KeyCode::Enter);
        assert!(app.cooperation.journal_discard.is_none());
        assert!(!app.planning.journal_failed());
        assert!(
            matches!(app.planning.work_pending.as_ref(), Some(store::WorkRequest::Release(owned)) if owned == &claim)
        );
        assert_eq!(app.planning.work, Some(claim));
        assert!(app.planning.pending.is_none());
        assert!(
            app.planning
                .journal_notice
                .contains("Unverified provider coverage gap")
        );
        assert_eq!(
            (app.input, app.cursor, app.revision, app.goal, app.drafts),
            before
        );
    }

    #[test]
    fn identity_change_or_storage_retry_refuses_and_keeps_original_error_review() {
        for identity_change in [false, true] {
            let mut app = app();
            let before = (app.input.clone(), app.cursor, app.revision);
            app.request_journal_discard();
            let original = app
                .cooperation
                .journal_discard
                .as_ref()
                .unwrap()
                .error
                .clone();
            if identity_change {
                app.planning.work.as_mut().unwrap().request_id = "successor".into();
            } else {
                app.planning.retry_journal();
            }
            app.cooperation.journal_discard.as_mut().unwrap().reviewed = true;
            press(&mut app, KeyCode::Char('y'));
            let review = app.cooperation.journal_discard.as_ref().unwrap();
            assert!(!review.reviewed && !review.affirmative);
            assert!(
                review.error.starts_with(&original) && review.error.contains("Discard refused:")
            );
            assert!(app.cooperation_ui.journal_discard_details);
            assert!(app.planning.work_pending.is_none());
            assert_eq!((app.input, app.cursor, app.revision), before);
        }
    }

    #[test]
    fn details_and_nonpress_or_modified_keys_never_acknowledge() {
        let mut app = app();
        app.request_journal_discard();
        app.cooperation.journal_discard.as_mut().unwrap().reviewed = true;
        let mut key = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        key.kind = KeyEventKind::Repeat;
        assert!(app.journal_discard_key(key));
        assert!(app.planning.journal_failed());
        assert!(app.journal_discard_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)));
        assert!(app.planning.journal_failed());
        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('y'));
        press(&mut app, KeyCode::Enter);
        assert!(app.cooperation.journal_discard.is_some() && app.planning.journal_failed());
        press(&mut app, KeyCode::Char('d'));
        assert!(!app.cooperation.journal_discard.as_ref().unwrap().reviewed);
        assert!(app.planning.work_pending.is_none());
    }
}
