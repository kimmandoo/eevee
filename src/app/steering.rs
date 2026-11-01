//! Explicit native steering and owner-bound queued Send. No focus transfer or replay.
use super::*;
use std::collections::BTreeMap;

const RECOVERY_NOTICE: &str = "Unsent follow-up kept in memory: Request is full. Park or shorten the owning partner's newer draft, then /queue-cancel. Nothing sent.";

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
    recovery: BTreeMap<(i64, i64), Vec<Draft>>,
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
        if self
            .steering
            .recovery
            .contains_key(&(self.task_id, self.lead.id))
        {
            self.notice = RECOVERY_NOTICE.into();
            return true;
        }
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
        if self.has_queued_send() || self.has_recoverable_followup() {
            self.notice = "One unsent follow-up is already retained for this partner; /queue-cancel returns it. Draft kept.".into();
            return true;
        }
        self.planning.last_outcome = None;
        let draft = self.take_draft();
        self.steering.queued.insert(owner, draft);
        self.notice = "Next Send queued for this partner while in front; it is freshly prepared/reviewed only after this exact run completes. /queue-cancel returns it.".into();
        true
    }
    fn restore_draft(&mut self, draft: Draft) {
        let owner = (self.task_id, self.lead.id);
        if let Err(draft) = self.restore_owned_followup(owner, draft) {
            self.steering.recovery.entry(owner).or_default().push(draft);
            self.notice = RECOVERY_NOTICE.into();
        } else {
            self.changed_task();
        }
    }
    fn restore_owned_followup(&mut self, owner: (i64, i64), draft: Draft) -> Result<(), Draft> {
        if owner.0 != self.task_id
            || owner.1 != self.lead.id && !self.party.iter().any(|partner| partner.id == owner.1)
        {
            return Err(draft);
        }
        if owner.1 == self.lead.id && self.composer_is_request() {
            append_recovered(&mut self.input, &mut self.cursor, draft)?;
            self.sources.clear_wire_request();
            return Ok(());
        }
        let drafts = if owner.1 == self.lead.id {
            &mut self.drafts
        } else {
            &mut self.partner_drafts.entry(owner.1).or_default().drafts
        };
        if let Some(existing) = drafts
            .iter_mut()
            .find(|existing| existing.field == store::EditorField::Request)
        {
            append_recovered(&mut existing.text, &mut existing.cursor, draft)
        } else if draft.text.len() <= MAX_GOAL_BYTES {
            drafts.push(store::Draft {
                field: store::EditorField::Request,
                text: draft.text,
                cursor: draft.cursor,
            });
            Ok(())
        } else {
            Err(draft)
        }
    }

    pub(super) fn followup_recovery_pending(&self) -> bool {
        !self.steering.recovery.is_empty()
    }

    /// Retire Send intent, then recover only text that never reached a tool boundary.
    pub(super) fn recover_cancelled_followups(&mut self) -> bool {
        if !self.steering.choice
            && self.steering.queued.is_empty()
            && self.steering.recovery.is_empty()
            && !std::iter::once(&self.planning)
                .chain(self.cooperation.parked.values())
                .any(|state| !state.running() && !state.steering.is_empty())
        {
            return false;
        }
        self.steering.choice = false;
        self.steering.choice_owner = None;
        let mut pending = std::mem::take(&mut self.steering.recovery);
        for (owner, mut draft) in std::mem::take(&mut self.steering.queued) {
            draft.generation = None;
            pending.entry(owner).or_default().push(draft);
        }
        let partners: Vec<_> = std::iter::once(self.lead.id)
            .chain(self.cooperation.parked.keys().copied())
            .collect();
        for partner in partners {
            let Some(state) = self.planning_state_mut(partner) else {
                continue;
            };
            if state.running() {
                continue;
            }
            let unsent = std::mem::take(&mut state.steering);
            state.steer_pending = None;
            let owner = (self.task_id, partner);
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
                pending.entry(owner).or_default().push(draft);
            }
        }
        let mut changed = false;
        for (owner, drafts) in pending {
            for draft in drafts {
                match self.restore_owned_followup(owner, draft) {
                    Ok(()) => changed = true,
                    Err(draft) => self.steering.recovery.entry(owner).or_default().push(draft),
                }
            }
        }
        if changed {
            self.changed_task();
        }
        if self.followup_recovery_pending() {
            self.notice = RECOVERY_NOTICE.into();
        }
        changed
    }
    pub fn has_queued_send(&self) -> bool {
        self.steering
            .queued
            .contains_key(&(self.task_id, self.lead.id))
    }
    pub fn has_recoverable_followup(&self) -> bool {
        self.steering
            .recovery
            .contains_key(&(self.task_id, self.lead.id))
    }
    pub(super) fn queue_cancel(&mut self) -> bool {
        let owner = (self.task_id, self.lead.id);
        let mut pending = self.steering.recovery.remove(&owner).unwrap_or_default();
        if let Some(draft) = self.steering.queued.remove(&owner) {
            pending.push(draft);
        }
        let mut restored = false;
        for draft in pending {
            match self.restore_owned_followup(owner, draft) {
                Ok(()) => restored = true,
                Err(draft) => self.steering.recovery.entry(owner).or_default().push(draft),
            }
        }
        if restored {
            self.changed_task();
        }
        self.notice = if self.has_recoverable_followup() {
            RECOVERY_NOTICE
        } else if restored {
            "Unsent follow-up returned to the Request draft; nothing sent."
        } else {
            "No unsent follow-up is retained for this partner."
        }
        .into();
        true
    }
    pub(super) fn settle_steering(&mut self) -> bool {
        if self.cancelling_owned_work() || self.cooperation.quitting {
            return false;
        }
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
            self.notice = if self.followup_recovery_pending() {
                RECOVERY_NOTICE
            } else {
                "The run ended before a tool-result boundary; unsent steering returned to the composer. Enter sends it as a new request."
            }.into();
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
            self.notice = if self.followup_recovery_pending() {
                RECOVERY_NOTICE
            } else {
                "The exact run did not complete into an empty owning composer; queued Send returned without dispatch."
            }.into();
        }
        true
    }
}

fn append_recovered(text: &mut String, cursor: &mut usize, draft: Draft) -> Result<(), Draft> {
    if text.is_empty() && draft.text.len() <= MAX_GOAL_BYTES {
        *text = draft.text;
        *cursor = draft.cursor.min(text.len());
        return Ok(());
    }
    if text
        .len()
        .saturating_add(1)
        .saturating_add(draft.text.len())
        > MAX_GOAL_BYTES
    {
        return Err(draft);
    }
    text.push('\n');
    text.push_str(&draft.text);
    Ok(())
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

    #[test]
    fn cancellation_recovers_each_current_task_owner_without_dispatch() {
        let mut app = App::new("/project".into(), 60, 18);
        app.task_id = 7;
        app.party.push(store::Partner {
            id: 2,
            ..store::Partner::default()
        });
        app.steering.queued.insert(
            (7, app.lead.id),
            Draft {
                text: "  retained lead\n".into(),
                cursor: 2,
                generation: Some(11),
            },
        );
        app.steering.queued.insert(
            (7, 2),
            Draft {
                text: "retained partner".into(),
                cursor: 3,
                generation: Some(12),
            },
        );
        assert!(app.recover_cancelled_followups());
        assert!(app.steering.queued.is_empty());
        assert!(!app.followup_recovery_pending());
        assert_eq!((app.input.as_str(), app.cursor), ("  retained lead\n", 2));
        let partner = &app.partner_drafts[&2].drafts[0];
        assert_eq!(partner.field, store::EditorField::Request);
        assert_eq!(
            (partner.text.as_str(), partner.cursor),
            ("retained partner", 3)
        );
        assert!(app.pending_request.is_none());
        assert!(app.save_requested);
    }

    #[test]
    fn full_draft_keeps_followup_recoverable_until_space_is_available() {
        let mut app = App::new("/project".into(), 60, 18);
        app.input = "x".repeat(MAX_GOAL_BYTES);
        app.cursor = 3;
        app.steering.queued.insert(
            (app.task_id, app.lead.id),
            Draft {
                text: "  exact unsent bytes\n".into(),
                cursor: 2,
                generation: Some(11),
            },
        );
        assert!(!app.recover_cancelled_followups());
        assert!(app.steering.queued.is_empty());
        assert!(app.followup_recovery_pending());
        assert_eq!(app.input.len(), MAX_GOAL_BYTES);
        assert_eq!(app.cursor, 3);
        assert!(app.pending_request.is_none());
        app.input.clear();
        app.cursor = 0;
        app.queue_cancel();
        assert!(!app.followup_recovery_pending());
        assert_eq!(
            (app.input.as_str(), app.cursor),
            ("  exact unsent bytes\n", 2)
        );
    }
}
