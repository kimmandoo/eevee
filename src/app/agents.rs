//! Passive profile selection and explicit, generation-correlated executable trust.
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::agent::{Event, Profile, Request};

pub struct State {
    pub pending: Option<Request>,
    pub profiles: Vec<Profile>,
    pub selected: usize,
    pub preview: bool,
    pub affirmative: bool,
    pub reviewed: bool,
    pub scroll: u16,
    pub scroll_max: u16,
    pub notice: String,
    discovering: bool,
    generation: u64,
    inflight: Option<(usize, u64)>,
    pub auth_preview: Option<crate::auth::Plan>,
    pub pending_auth: Option<crate::auth::Plan>,
    auth_preparing: Option<(usize, u64)>,
    pub method: usize,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            pending: Some(Request::Discover),
            profiles: Vec::new(),
            selected: 0,
            preview: false,
            affirmative: false,
            reviewed: false,
            scroll: 0,
            scroll_max: 0,
            notice: "Passive discovery only; no program has been launched.".into(),
            discovering: true,
            generation: 0,
            inflight: None,
            auth_preview: None,
            pending_auth: None,
            auth_preparing: None,
            method: 0,
        }
    }

    pub fn busy(&self) -> bool {
        self.discovering || self.inflight.is_some() || self.pending.is_some() || self.auth_busy()
    }

    pub fn auth_busy(&self) -> bool {
        self.auth_preparing.is_some() || self.pending_auth.is_some()
    }

    pub fn auth_returned(&mut self, profile: usize, outcome: crate::auth::Outcome) {
        self.notice = outcome.label().into();
        if let Some(profile) = self.profiles.get_mut(profile) {
            profile.authentication = self.notice.clone();
        }
        self.scroll = 0;
    }

    pub fn event(&mut self, event: Event) {
        match event {
            Event::Discovered(profiles) if self.discovering => {
                self.method = 0;
                self.profiles = profiles;
                self.selected = self.selected.min(self.profiles.len().saturating_sub(1));
                self.discovering = false;
                self.notice =
                    "Discovery is passive. Enter previews a version probe; it does not run it."
                        .into();
            }
            Event::Probed {
                profile,
                generation,
                result,
            } if self.inflight == Some((profile, generation)) => {
                self.inflight = None;
                if let Some(current) = self.profiles.get_mut(profile) {
                    *current = *result;
                }
                self.notice = "Preflight returned. Review evidence and blockers; e separately previews the selected provider and action.".into();
            }
            Event::AuthPrepared {
                profile,
                generation,
                result,
            } if self.auth_preparing == Some((profile, generation)) => {
                self.auth_preparing = None;
                match result {
                    Ok(plan) => {
                        self.auth_preview = Some(*plan);
                        self.affirmative = false;
                        self.reviewed = false;
                        self.scroll = 0;
                        self.scroll_max = 0;
                    }
                    Err(reason) => self.notice = reason,
                }
            }
            _ => {}
        }
    }

    pub fn cancel_preview(&mut self) {
        self.preview = false;
        self.auth_preview = None;
        self.pending_auth = None;
        self.auth_preparing = None;
        if matches!(self.pending, Some(Request::PrepareAuth { .. })) {
            self.pending = None;
        }
        self.affirmative = false;
        self.reviewed = false;
        self.scroll = 0;
        self.scroll_max = 0;
    }

    pub fn key(&mut self, key: KeyEvent, large_enough: bool) -> bool {
        if key.kind == KeyEventKind::Release
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        if key.code == KeyCode::Esc {
            self.cancel_preview();
            return true;
        }
        if !large_enough {
            self.affirmative = false;
            self.reviewed = false;
            return false;
        }
        if key.kind == KeyEventKind::Repeat
            && !matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Home
                    | KeyCode::End
            )
        {
            return false;
        }
        match key.code {
            KeyCode::Enter if self.auth_preview.is_some() => {
                if !self.affirmative {
                    self.cancel_preview();
                } else if self.reviewed && !self.busy() {
                    self.pending_auth = self.auth_preview.take();
                    self.affirmative = false;
                    self.reviewed = false;
                    self.notice =
                        "Authentication authorized once; final app and trust admission pending."
                            .into();
                }
            }
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right
                if self.auth_preview.is_some() =>
            {
                if self.reviewed {
                    self.affirmative = !self.affirmative;
                }
            }
            KeyCode::Char('m') if !self.preview && self.auth_preview.is_none() && !self.busy() => {
                let count = crate::auth::methods(self.selected).len();
                if count > 0 {
                    self.method = (self.method + 1) % count;
                }
                self.scroll = 0;
            }
            KeyCode::Char('l' | 's')
                if !self.preview && self.auth_preview.is_none() && !self.busy() =>
            {
                let method = if key.code == KeyCode::Char('s') {
                    crate::auth::status_method(self.selected)
                } else {
                    crate::auth::methods(self.selected)
                        .get(self.method)
                        .copied()
                };
                if let Some(method) = method {
                    self.generation += 1;
                    self.auth_preparing = Some((self.selected, self.generation));
                    self.pending = Some(Request::PrepareAuth {
                        profile: self.selected,
                        generation: self.generation,
                        method,
                    });
                    self.notice =
                        "Preparing passive authentication review; no executable launched.".into();
                } else {
                    self.notice = if self.selected == 1 { crate::auth::CLAUDE_POLICY } else {
                        "Gemini has no qualified read-only auth-status command. Review l to open its official native login chooser. Native exit leaves authentication unknown; e separately reviews a model request and provider eligibility."
                    }.into();
                    self.scroll = 0;
                }
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(5),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(5).min(self.scroll_max),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.scroll_max,
            KeyCode::Up | KeyCode::Down if self.preview || self.auth_preview.is_some() => {
                self.scroll = if key.code == KeyCode::Up {
                    self.scroll.saturating_sub(1)
                } else {
                    self.scroll.saturating_add(1).min(self.scroll_max)
                };
            }
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right if self.preview => {
                if self.reviewed {
                    self.affirmative = !self.affirmative;
                }
            }
            KeyCode::Enter if self.preview => {
                if !self.affirmative {
                    self.cancel_preview();
                } else if self.reviewed
                    && !self.busy()
                    && self
                        .profiles
                        .get(self.selected)
                        .is_some_and(|p| p.probe_allowed)
                {
                    self.generation += 1;
                    self.inflight = Some((self.selected, self.generation));
                    self.pending = Some(Request::Probe {
                        profile: self.selected,
                        generation: self.generation,
                    });
                    self.cancel_preview();
                    self.notice = "Trusted version probe pending. No authentication or model request. Esc closes the drawer, not the bounded probe.".into();
                }
            }
            KeyCode::Enter if !self.busy() => {
                if self
                    .profiles
                    .get(self.selected)
                    .is_some_and(|p| p.probe_allowed)
                {
                    self.preview = true;
                    self.affirmative = false;
                    self.reviewed = false;
                    self.scroll = 0;
                    self.scroll_max = 0;
                } else {
                    self.notice =
                        "Probe unavailable. Review this profile's prerequisites and blockers."
                            .into();
                }
            }
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Tab
            | KeyCode::BackTab
                if !self.preview && self.auth_preview.is_none() && !self.auth_busy() =>
            {
                let count = self.profiles.len();
                if count == 0 {
                    return false;
                }
                if matches!(key.code, KeyCode::Up | KeyCode::Left | KeyCode::BackTab) {
                    self.selected = (self.selected + count - 1) % count;
                } else {
                    self.selected = (self.selected + 1) % count;
                }
                self.method = 0;
                self.scroll = 0;
                self.scroll_max = 0;
            }
            KeyCode::Char('1'..='4')
                if !self.preview && self.auth_preview.is_none() && !self.auth_busy() =>
            {
                if let KeyCode::Char(digit) = key.code {
                    let index = digit as usize - '1' as usize;
                    if index < self.profiles.len() {
                        self.selected = index;
                        self.method = 0;
                        self.scroll = 0;
                        self.scroll_max = 0;
                    }
                }
            }
            KeyCode::Char('r') if !self.preview && self.auth_preview.is_none() && !self.busy() => {
                self.discovering = true;
                self.pending = Some(Request::Discover);
                self.scroll = 0;
                self.notice = "Refreshing passive discovery; no executable probe.".into();
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile {
            name: "Test profile".into(),
            executable: Some("/trusted/test-agent".into()),
            argv: vec!["--version".into()],
            installed: "present".into(),
            version: "unknown".into(),
            authentication: "unknown".into(),
            configuration: "no configuration read".into(),
            os: "test".into(),
            adapter: "absent".into(),
            action: "disabled".into(),
            probe_allowed: true,
            trust_preview: vec!["Executes trusted test binary".into()],
            revision: 1,
            architecture: "test".into(),
            billing: "unknown".into(),
            blockers: vec!["No adapter".into()],
            capabilities: Vec::new(),
        }
    }

    fn discovered() -> State {
        let mut state = State::new();
        assert!(matches!(state.pending.take(), Some(Request::Discover)));
        state.event(Event::Discovered(vec![
            profile(),
            profile(),
            profile(),
            profile(),
        ]));
        state
    }

    fn press(state: &mut State, code: KeyCode) {
        state.key(KeyEvent::new(code, KeyModifiers::NONE), true);
    }

    #[test]
    fn discovery_selection_and_neutral_enter_never_probe() {
        let mut state = discovered();
        press(&mut state, KeyCode::Char('4'));
        assert_eq!(state.selected, 3);
        assert!(!state.busy());
        press(&mut state, KeyCode::Enter);
        assert!(state.preview);
        assert!(!state.affirmative);
        press(&mut state, KeyCode::Tab);
        assert!(!state.affirmative, "unreviewed preview cannot authorize");
        press(&mut state, KeyCode::Enter);
        assert!(!state.preview);
        assert!(state.pending.is_none());
    }

    #[test]
    fn reviewed_deliberate_consent_is_generation_correlated() {
        let mut state = discovered();
        press(&mut state, KeyCode::Enter);
        state.reviewed = true;
        press(&mut state, KeyCode::Tab);
        press(&mut state, KeyCode::Enter);
        assert!(matches!(
            state.pending.take(),
            Some(Request::Probe {
                profile: 0,
                generation: 1
            })
        ));
        assert!(state.busy());
        press(&mut state, KeyCode::Down);
        state.event(Event::Probed {
            profile: 0,
            generation: 2,
            result: Box::new(profile()),
        });
        assert!(
            state.busy(),
            "stale result must not clear the in-flight request"
        );
        let mut result = profile();
        result.version = "observed".into();
        state.event(Event::Probed {
            profile: 0,
            generation: 1,
            result: Box::new(result),
        });
        assert!(!state.busy());
        assert_eq!(state.profiles[0].version, "observed");
        assert_eq!(state.profiles[1].version, "unknown");
    }

    #[test]
    fn undersize_repeat_and_cancel_cannot_approve() {
        let mut state = discovered();
        press(&mut state, KeyCode::Enter);
        state.reviewed = true;
        press(&mut state, KeyCode::Tab);
        let mut repeated = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        repeated.kind = KeyEventKind::Repeat;
        state.key(repeated, true);
        assert!(state.pending.is_none());
        state.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), false);
        assert!(!state.affirmative);
        assert!(!state.reviewed);
        assert!(state.pending.is_none());
        press(&mut state, KeyCode::Esc);
        assert!(!state.preview);
    }

    #[test]
    fn missing_profile_cannot_open_trust_and_refresh_is_passive() {
        let mut state = discovered();
        state.profiles[0].probe_allowed = false;
        press(&mut state, KeyCode::Enter);
        assert!(!state.preview);
        assert!(state.pending.is_none());
        press(&mut state, KeyCode::Char('r'));
        assert!(matches!(state.pending, Some(Request::Discover)));
        assert!(state.busy());
    }

    #[test]
    fn login_review_is_passive_and_cancel_discards_late_generation() {
        let mut state = discovered();
        press(&mut state, KeyCode::Char('m'));
        press(&mut state, KeyCode::Char('l'));
        assert!(matches!(
            state.pending.take(),
            Some(Request::PrepareAuth {
                profile: 0,
                generation: 1,
                method: crate::auth::Method::CodexDevice
            })
        ));
        assert!(state.auth_busy());
        assert!(state.pending_auth.is_none());
        press(&mut state, KeyCode::Esc);
        assert!(!state.auth_busy());
        let notice = state.notice.clone();
        state.event(Event::AuthPrepared {
            profile: 0,
            generation: 1,
            result: Err("late result".into()),
        });
        assert_eq!(state.notice, notice);
        assert!(state.auth_preview.is_none());
        press(&mut state, KeyCode::Char('l'));
        assert!(matches!(
            state.pending.take(),
            Some(Request::PrepareAuth { generation: 2, .. })
        ));
        state.event(Event::AuthPrepared {
            profile: 0,
            generation: 1,
            result: Err("stale".into()),
        });
        assert!(state.auth_busy());
    }

    #[test]
    fn repeated_or_undersized_login_keys_never_prepare_or_change_method() {
        let mut state = discovered();
        for code in [KeyCode::Char('l'), KeyCode::Char('s'), KeyCode::Char('m')] {
            let mut key = KeyEvent::new(code, KeyModifiers::NONE);
            key.kind = KeyEventKind::Repeat;
            state.key(key, true);
            state.key(KeyEvent::new(code, KeyModifiers::NONE), false);
        }
        assert!(state.pending.is_none());
        assert!(state.pending_auth.is_none());
        assert!(!state.auth_busy());
        assert_eq!(state.method, 0);
    }

    #[test]
    fn provider_switch_resets_method_and_claude_never_launches_cli_login() {
        let mut state = discovered();
        press(&mut state, KeyCode::Char('m'));
        assert_eq!(state.method, 1);
        press(&mut state, KeyCode::Char('2'));
        assert_eq!(state.method, 0);
        press(&mut state, KeyCode::Char('l'));
        assert_eq!(state.notice, crate::auth::CLAUDE_POLICY);
        assert!(state.pending.is_none());
        press(&mut state, KeyCode::Char('s'));
        assert!(state.pending.is_none());
    }

    #[test]
    fn login_result_keeps_only_classification_and_never_replays_task() {
        let mut state = discovered();
        state.auth_returned(0, crate::auth::Outcome::Unknown);
        assert_eq!(
            state.profiles[0].authentication,
            crate::auth::Outcome::Unknown.label()
        );
        assert_eq!(state.profiles[1].authentication, "unknown");
        assert!(state.pending.is_none());
        assert!(state.pending_auth.is_none());
    }
}
