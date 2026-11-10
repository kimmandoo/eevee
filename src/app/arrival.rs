//! Direct single-key answers on an auto-presented permission.
//!
//! A permission that pops up while the user is typing must not consume their
//! keystrokes as consent. Direct `a`/`n` answers therefore unlock only after
//! the review has been on screen for a short settle time and no other key has
//! arrived since it appeared; otherwise Enter opens the full review as before.
use std::time::{Duration, Instant};

pub const SETTLE: Duration = Duration::from_millis(800);

#[derive(Clone, Copy, Debug, Default)]
pub struct Guard {
    presented: Option<Instant>,
    disturbed: bool,
    /// The unlock has been repainted once, so the hint matches the keys.
    announced: bool,
}

impl Guard {
    pub fn present(&mut self) {
        self.present_at(Instant::now());
    }

    pub fn present_at(&mut self, now: Instant) {
        self.presented = Some(now);
        self.disturbed = false;
        self.announced = false;
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Any other key while the review is up keeps the two-step path.
    pub fn disturb(&mut self) {
        self.disturbed = true;
    }

    pub fn ready(&self) -> bool {
        self.ready_at(Instant::now())
    }

    pub fn ready_at(&self, now: Instant) -> bool {
        !self.disturbed
            && self
                .presented
                .is_some_and(|at| now.saturating_duration_since(at) >= SETTLE)
    }
}

impl Guard {
    /// Time left until the direct keys unlock, while that is still ahead.
    pub fn wait_at(&self, now: Instant) -> Option<Duration> {
        (!self.disturbed && !self.announced)
            .then_some(self.presented?)
            .map(|at| (at + SETTLE).saturating_duration_since(now))
    }

    /// True exactly once, when the keys unlock, so the visible hint can switch
    /// from the Enter path to the direct keys without waiting for input.
    pub fn settle_at(&mut self, now: Instant) -> bool {
        if self.announced || !self.ready_at(now) {
            return false;
        }
        self.announced = true;
        true
    }
}

impl super::App {
    pub fn arrival_wait(&self, now: Instant) -> Option<Duration> {
        let planning = self
            .planning
            .direct_answer
            .wait_at(now)
            .filter(|_| self.planning.auto_permission_review);
        let execution = self
            .execution
            .direct_answer
            .wait_at(now)
            .filter(|_| self.execution.auto_presented);
        match (planning, execution) {
            (Some(planning), Some(execution)) => Some(planning.min(execution)),
            (planning, execution) => planning.or(execution),
        }
    }

    pub fn tick_arrival(&mut self, now: Instant) -> bool {
        let planning =
            self.planning.auto_permission_review && self.planning.direct_answer.settle_at(now);
        let execution =
            self.execution.auto_presented && self.execution.direct_answer.settle_at(now);
        planning || execution
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlocks_after_settle_and_locks_after_other_keys() {
        let start = Instant::now();
        let mut guard = Guard::default();
        assert!(!guard.ready_at(start + SETTLE));
        guard.present_at(start);
        assert!(!guard.ready_at(start + SETTLE / 2));
        assert!(guard.ready_at(start + SETTLE));
        guard.disturb();
        assert!(!guard.ready_at(start + SETTLE * 4));
        guard.present_at(start);
        assert!(guard.ready_at(start + SETTLE));
        guard.clear();
        assert!(!guard.ready_at(start + SETTLE));
    }

    #[test]
    fn announces_the_unlock_once_and_stops_waiting() {
        let start = Instant::now();
        let mut guard = Guard::default();
        assert_eq!(guard.wait_at(start), None);
        guard.present_at(start);
        assert_eq!(guard.wait_at(start), Some(SETTLE));
        assert!(!guard.settle_at(start + SETTLE / 2));
        assert!(guard.settle_at(start + SETTLE));
        assert!(!guard.settle_at(start + SETTLE * 2));
        assert_eq!(guard.wait_at(start + SETTLE), None);
        guard.present_at(start);
        guard.disturb();
        assert_eq!(guard.wait_at(start), None);
        assert!(!guard.settle_at(start + SETTLE));
    }
}
