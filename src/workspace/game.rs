//! Task-owned game-loop inventory: one-shot Berries and dismissed move
//! proposals. Labels and inventory never enter provider packets or handoffs.
use serde::{Deserialize, Serialize};

pub const MAX_BERRIES: usize = 8;
pub const MAX_DISMISSED: usize = 64;

/// User-selected context that the next explicit Send includes exactly once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum Berry {
    /// A committed basket excerpt, removed after dispatch acknowledgment.
    Source { excerpt_id: i64 },
    /// An approved selected memory, deselected after dispatch acknowledgment.
    Memory { id: i64 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub berries: Vec<Berry>,
    /// Loose-key digests of repeated instructions the user chose not to learn.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dismissed_moves: Vec<String>,
    /// Newer writers' fields, preserved (XC08).
    #[serde(flatten)]
    pub extra: super::forward::Extra,
}

impl State {
    pub fn is_empty(&self) -> bool {
        self.berries.is_empty() && self.dismissed_moves.is_empty() && self.extra.is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.berries.len() > MAX_BERRIES {
            return Err("At most eight Berries can wait for the next Send".into());
        }
        for (index, berry) in self.berries.iter().enumerate() {
            let id = match berry {
                Berry::Source { excerpt_id } => *excerpt_id,
                Berry::Memory { id } => *id,
            };
            if id <= 0 || self.berries[..index].contains(berry) {
                return Err("Berry references must be distinct positive identities".into());
            }
        }
        if self.dismissed_moves.len() > MAX_DISMISSED
            || self.dismissed_moves.iter().enumerate().any(|(index, key)| {
                key.len() != 64
                    || !key.bytes().all(|byte| byte.is_ascii_hexdigit())
                    || self.dismissed_moves[..index].contains(key)
            })
        {
            return Err("Dismissed move proposals must be distinct SHA-256 digests".into());
        }
        Ok(())
    }

    pub fn toggle(&mut self, berry: Berry) -> Result<bool, String> {
        if let Some(index) = self.berries.iter().position(|existing| *existing == berry) {
            self.berries.remove(index);
            return Ok(false);
        }
        if self.berries.len() >= MAX_BERRIES {
            return Err("At most eight Berries can wait for the next Send".into());
        }
        self.berries.push(berry);
        Ok(true)
    }

    pub fn dismiss(&mut self, key: &str) {
        if !self.dismissed_moves.iter().any(|existing| existing == key) {
            if self.dismissed_moves.len() >= MAX_DISMISSED {
                self.dismissed_moves.remove(0);
            }
            self.dismissed_moves.push(key.to_owned());
        }
    }
}

/// Case-, whitespace- and trailing-punctuation-insensitive identity of an
/// instruction. Literal paths and quoted text stay part of the identity.
pub fn loose_key(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let collapsed = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let trimmed = collapsed
        .trim_end_matches(['.', '!', '?', '。', '~'])
        .trim_end();
    format!("{:x}", Sha256::digest(trimmed.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loose_key_ignores_case_spacing_and_trailing_punctuation_only() {
        let base = loose_key("Run the tests and fix failures");
        for same in [
            "run the tests and fix failures.",
            "  Run   the tests and fix failures!  ",
            "RUN THE TESTS AND FIX FAILURES?",
        ] {
            assert_eq!(loose_key(same), base, "{same}");
        }
        assert_ne!(
            loose_key("Run the tests in src/a.rs"),
            loose_key("Run the tests in src/b.rs")
        );
        assert_eq!(
            loose_key("테스트 돌리고 고쳐줘"),
            loose_key("테스트  돌리고 고쳐줘~")
        );
    }

    #[test]
    fn berries_and_dismissals_are_bounded_and_default_bytes_are_empty() {
        let mut state = State::default();
        assert_eq!(serde_json::to_string(&state).unwrap(), "{}");
        assert!(state.toggle(Berry::Source { excerpt_id: 4 }).unwrap());
        assert!(!state.toggle(Berry::Source { excerpt_id: 4 }).unwrap());
        for id in 1..=8 {
            state.toggle(Berry::Memory { id }).unwrap();
        }
        assert!(state.toggle(Berry::Memory { id: 9 }).is_err());
        state.dismiss(&loose_key("x"));
        state.dismiss(&loose_key("x"));
        assert_eq!(state.dismissed_moves.len(), 1);
        state.validate().unwrap();
        let mut bad = state.clone();
        bad.dismissed_moves.push("not-a-digest".into());
        assert!(bad.validate().is_err());
    }
}
