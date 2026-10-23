//! One bounded visual enhancement of the existing receipt, never another popup.
use super::{App, execution};
use std::time::{Duration, Instant};

const STEP: Duration = Duration::from_millis(80);
const END: Duration = Duration::from_millis(240);

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

    pub fn evolution_wait(&self, now: Instant) -> Option<Duration> {
        self.evolution.motion.map(|motion| {
            let next = motion.started + STEP * u32::from(motion.frame + 1);
            next.saturating_duration_since(now)
        })
    }

    pub fn tick_evolution(&mut self, now: Instant) -> bool {
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
