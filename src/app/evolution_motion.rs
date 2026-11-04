//! One bounded visual enhancement of the existing receipt, never another popup.
use super::{App, execution};
use std::time::{Duration, Instant};

const STEP: Duration = Duration::from_millis(80);
const END: Duration = Duration::from_millis(240);
const POPUP_LIFETIME: Duration = Duration::from_secs(3);

#[derive(Clone, Copy)]
pub(super) struct Motion {
    started: Instant,
    pub frame: u8,
}

impl App {
    pub(super) fn picker_select_form(&mut self, form: usize) {
        let from = self.form();
        self.request_navigation(execution::Action::SelectForm(form));
        // Deferred work never queues a transition. Only the synchronous picker
        // commit can arm it, after all ordinary navigation/ownership guards.
        if from != self.form() {
            self.arm_picker_motion(Instant::now());
        }
    }

    /// Let Eevee choose again at the next Send while keeping the current
    /// appearance. Mode affects only future Sends, never drafts or running work.
    pub(super) fn enable_auto_form(&mut self) {
        if self.lead.auto_form {
            self.notice = "Auto is already on: Eevee picks the form at Send.".into();
            return;
        }
        self.lead.auto_form = true;
        self.changed_task();
        self.notice =
            "Auto on: Send selects the focus role, with at most one in-flight adaptation; appearance kept.".into();
    }

    pub(super) fn picker_return_to_eevee(&mut self) {
        let from = self.form();
        self.request_return_to_eevee();
        if from != self.form() {
            self.arm_picker_motion(Instant::now());
        }
    }

    pub(super) fn arm_picker_motion(&mut self, now: Instant) {
        self.evolution.motion = None;
        if self.motion_visuals
            && self.motion_enabled()
            && self.width >= 60
            && self.height >= 18
            && self.evolution_popup_ready()
        {
            self.evolution.motion = Some(Motion {
                started: now,
                frame: 0,
            });
        }
    }

    pub fn evolution_frame(&self) -> Option<u8> {
        self.evolution.motion.map(|motion| motion.frame)
    }

    /// Start the receipt's lifetime only when it actually reaches the screen.
    /// Redraws and priority changes must not extend a one-shot notification.
    pub(crate) fn mark_evolution_popup_shown(&mut self, now: Instant) {
        if !self.evolution.popup_shown {
            self.evolution.popup_deadline = Some(now + POPUP_LIFETIME);
            self.evolution.popup_shown = true;
        }
    }

    pub fn evolution_wait(&self, now: Instant) -> Option<Duration> {
        let motion = self.evolution.motion.map(|motion| {
            let next = motion.started + STEP * u32::from(motion.frame + 1);
            next.saturating_duration_since(now)
        });
        let popup = self
            .evolution
            .popup_deadline
            .filter(|_| self.evolution.popup_shown && self.evolution.transition.is_some())
            .map(|deadline| deadline.saturating_duration_since(now));
        match (motion, popup) {
            (Some(motion), Some(popup)) => Some(motion.min(popup)),
            (motion, popup) => motion.or(popup),
        }
    }

    pub fn tick_evolution(&mut self, now: Instant) -> bool {
        // Expiration must request a repaint even for static/reduced-motion
        // receipts: otherwise their cleared cells remain until the next input.
        if self.evolution.popup_shown
            && self.evolution.transition.is_some()
            && self
                .evolution
                .popup_deadline
                .is_some_and(|deadline| now >= deadline)
        {
            self.evolution.transition = None;
            self.evolution.popup_shown = false;
            self.evolution.popup_deadline = None;
            self.evolution.motion = None;
            return true;
        }
        let Some(motion) = self.evolution.motion else {
            return false;
        };
        // Losing the surface ends motion permanently, even if a later resize,
        // approval settlement or code completion reveals the static receipt.
        if !self.motion_visuals
            || !self.motion_enabled()
            || self.width < 60
            || self.height < 18
            || !self.evolution_popup_ready()
            || now.saturating_duration_since(motion.started) >= END
        {
            self.evolution.motion = None;
            return true;
        }
        let frame =
            (now.saturating_duration_since(motion.started).as_millis() / STEP.as_millis()) as u8;
        if frame != motion.frame {
            self.evolution
                .motion
                .as_mut()
                .expect("present motion")
                .frame = frame;
            return true;
        }
        false
    }
}

#[cfg(test)]
#[path = "evolution_motion_tests.rs"]
mod tests;
