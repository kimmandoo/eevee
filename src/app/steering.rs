//! Explicit native steering and owner-bound queued Send. No focus transfer or replay.
use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Draft {
    pub text: String,
    pub cursor: usize,
    generation: Option<u64>,
}
#[derive(Default)]
pub struct State {
    pub choice: bool,
    choice_owner: Option<(i64, i64, Option<u64>)>,
    /// One slot per task/partner; never sent through another front owner.
    pub queued: BTreeMap<(i64, i64), Draft>,
    originals: BTreeMap<(i64, i64), Vec<Draft>>,
}
impl App {
    fn steerable(&self) -> Option<u64> {
        (self.planning.running()
            && !self.planning.cancelling
            && !self.planning.inspect_models
            && !self.planning.inspect_sessions
            && self.planning.agent.is_direct()
            && self
                .planning
                .owner()
                .is_some_and(|owner| owner.partner == self.lead.id))
        .then_some(self.planning.running_generation())
        .flatten()
    }
    pub(super) fn offer_steer_choice(&mut self) -> bool {
        if !self.planning.running()
            || self.planning.cancelling
            || self.pending_request.is_some()
            || self.input.trim().is_empty()
        {
            return false;
        }
        self.steering.choice = true;
        self.steering.choice_owner = Some((
            self.task_id,
            self.lead.id,
            self.planning.running_generation(),
        ));
        self.notice = if self.steerable().is_some() {
            "Lead is working: s Steer this run | q Queue as next Send | Esc Keep draft"
        } else {
            "Lead is working (this route cannot be steered): q Queue as next Send | Esc Keep draft"
        }
        .into();
        true
    }
    pub(super) fn steer_choice_key(&mut self, key: KeyEvent) -> Option<bool> {
        if !self.steering.choice || key.kind == KeyEventKind::Release {
            return None;
        }
        self.steering.choice = false;
        if !self.planning.running()
            || self.planning.cancelling
            || self.steering.choice_owner.take()
                != Some((
                    self.task_id,
                    self.lead.id,
                    self.planning.running_generation(),
                ))
        {
            self.notice =
                "Run ended or draft owner changed; exact draft kept, nothing sent.".into();
            return Some(true);
        }
        match key.code {
            KeyCode::Char('s') if key.modifiers.is_empty() => Some(self.steer_draft()),
            KeyCode::Char('q') if key.modifiers.is_empty() => Some(self.queue_draft()),
            KeyCode::Esc => {
                self.notice = "Draft kept; nothing sent.".into();
                Some(true)
            }
            _ => None,
        }
    }
    fn take_draft(&mut self) -> Draft {
        let draft = Draft {
            text: std::mem::take(&mut self.input),
            cursor: self.cursor,
            generation: self.planning.running_generation(),
        };
        self.cursor = 0;
        draft
    }
    fn steer_draft(&mut self) -> bool {
        let Some(generation) = self.steerable() else {
            self.notice = "This route cannot be steered; draft kept. q queues it instead.".into();
            return true;
        };
        let text = self.input.as_str();
        if text.len() > 4096 || crate::workspace::handoff::sanitize(text) != text {
            self.notice =
                "Steering is at most 4 KiB of control-safe nonsecret plain text; draft kept."
                    .into();
            return true;
        }
        if self.planning.steering.len() >= 8 || self.planning.steer_pending.is_some() {
            self.notice =
                "Steering queue is full or waiting for worker acknowledgment; draft kept.".into();
            return true;
        }
        if self.planning.steer_generation != Some(generation) {
            self.planning.steered.clear();
            self.planning.steer_generation = Some(generation);
            self.steering
                .originals
                .remove(&(self.task_id, self.lead.id));
        }
        let draft = self.take_draft();
        self.planning.steering.push(draft.text.clone());
        self.planning.steer_pending = Some(crate::providers::Request::Steer {
            generation,
            text: draft.text.clone(),
        });
        self.steering
            .originals
            .entry((self.task_id, self.lead.id))
            .or_default()
            .push(draft);
        self.notice = "Steer queued for the next tool-result boundary; if the run ends first it returns here.".into();
        true
    }
    fn queue_draft(&mut self) -> bool {
        let owner = (self.task_id, self.lead.id);
        if self.steering.queued.contains_key(&owner) {
            self.notice = "One Send is already queued for this partner; /queue-cancel returns it. Draft kept.".into();
            return true;
        }
        self.planning.last_outcome = None;
        let draft = self.take_draft();
        self.steering.queued.insert(owner, draft);
        self.notice = "Next Send queued for this partner while in front; it is freshly prepared/reviewed only after this exact run completes. /queue-cancel returns it.".into();
        true
    }
    fn restore_draft(&mut self, draft: Draft) {
        if self.input.is_empty() {
            self.input = draft.text;
            self.cursor = draft.cursor.min(self.input.len());
        } else {
            // Newer text/caret stay authoritative; recover unsent bytes after it.
            self.input.push('\n');
            self.input.push_str(&draft.text);
        }
    }
    pub fn has_queued_send(&self) -> bool {
        self.steering
            .queued
            .contains_key(&(self.task_id, self.lead.id))
    }
    pub(super) fn queue_cancel(&mut self) -> bool {
        match self.steering.queued.remove(&(self.task_id, self.lead.id)) {
            Some(draft) => {
                self.restore_draft(draft);
                self.notice = "Queued Send returned to the composer; nothing sent.".into();
            }
            None => self.notice = "No Send is queued for this partner.".into(),
        }
        true
    }
    pub(super) fn settle_steering(&mut self) -> bool {
        let mut changed = false;
        let owner = (self.task_id, self.lead.id);
        if self.steering.choice && !self.planning.running() {
            self.steering.choice = false;
            self.steering.choice_owner = None;
            self.notice = "Run ended; the exact follow-up draft and caret were kept. Enter sends it as a new request.".into();
            changed = true;
        }
        if !self.planning.running() && !self.planning.steering.is_empty() {
            let unsent = std::mem::take(&mut self.planning.steering);
            self.planning.steer_pending = None;
            let mut originals = self.steering.originals.remove(&owner).unwrap_or_default();
            for text in unsent {
                let draft = originals
                    .iter()
                    .position(|draft| draft.text == text)
                    .map(|index| originals.remove(index))
                    .unwrap_or_else(|| Draft {
                        cursor: text.len(),
                        text,
                        generation: None,
                    });
                self.restore_draft(draft);
            }
            self.notice = "The run ended before a tool-result boundary; unsent steering returned to the composer. Enter sends it as a new request.".into();
            changed = true;
        } else if !self.planning.running() {
            self.steering.originals.remove(&owner);
        }
        let Some((generation, outcome)) = self.planning.last_outcome else {
            return changed;
        };
        if !self.steering.queued.contains_key(&owner)
            || self.planning.busy()
            || self.pending_request.is_some()
            || self.partner_busy(self.lead.id)
        {
            return changed;
        }
        let draft = self
            .steering
            .queued
            .remove(&owner)
            .expect("owned queued Send");
        self.planning.last_outcome = None;
        if outcome == crate::providers::Outcome::Completed
            && self.input.is_empty()
            && draft.generation == Some(generation)
        {
            self.input = draft.text;
            self.cursor = draft.cursor;
            self.submit_request();
            self.notice = format!("Queued Send: {}", self.notice);
        } else {
            self.restore_draft(draft);
            self.notice = "The exact run did not complete into an empty owning composer; queued Send returned without dispatch.".into();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_cancel_restores_exact_whitespace_and_caret_without_touching_another_owner() {
        let mut app = App::new("/project".into(), 120, 36);
        app.input = "  한글 draft \n".into();
        app.cursor = 2;
        app.queue_draft();
        let original = (app.task_id, app.lead.id);
        app.steering.queued.insert(
            (999, 999),
            Draft {
                text: "foreign".into(),
                cursor: 3,
                generation: None,
            },
        );
        app.queue_cancel();
        assert_eq!((app.input.as_str(), app.cursor), ("  한글 draft \n", 2));
        assert!(!app.steering.queued.contains_key(&original));
        assert_eq!(app.steering.queued[&(999, 999)].text, "foreign");
        assert!(app.pending_request.is_none());
    }
    #[test]
    fn returning_unsent_text_preserves_a_newer_drafts_caret() {
        let mut app = App::new("/project".into(), 60, 18);
        app.input = "new draft".into();
        app.cursor = 3;
        app.restore_draft(Draft {
            text: "  old bytes\n".into(),
            cursor: 0,
            generation: None,
        });
        assert_eq!(
            (app.input.as_str(), app.cursor),
            ("new draft\n  old bytes\n", 3)
        );
    }
}
