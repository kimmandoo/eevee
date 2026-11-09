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
}

impl Guard {
    pub fn present(&mut self) {
        self.present_at(Instant::now());
    }

    pub fn present_at(&mut self, now: Instant) {
        self.presented = Some(now);
        self.disturbed = false;
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
}
