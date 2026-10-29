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

/// Proposal-only normalization. Original instruction bytes are never rewritten.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Similarity {
    pub tokens: Vec<String>,
    pub leading_verb: String,
    pub parameters: Vec<(String, String)>,
    pub spans: Vec<(usize, usize)>,
}

pub fn similarity(text: &str) -> Similarity {
    normalize(text, true)
}

fn normalize(text: &str, parameters: bool) -> Similarity {
    let mut result = Similarity {
        tokens: Vec::new(),
        leading_verb: String::new(),
        parameters: Vec::new(),
        spans: Vec::new(),
    };
    let mut chars = text.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        let quoted = matches!(ch, '"' | '\'' | '`');
        let mut end = start + ch.len_utf8();
        if quoted {
            while let Some((index, next)) = chars.next() {
                end = index + next.len_utf8();
                if next == '\\' {
                    if let Some((index, escaped)) = chars.next() {
                        end = index + escaped.len_utf8();
                    }
                } else if next == ch {
                    break;
                }
            }
        } else {
            while let Some(&(index, next)) = chars.peek() {
                if next.is_whitespace() {
                    break;
                }
                chars.next();
                end = index + next.len_utf8();
            }
        }
        let raw = &text[start..end];
        let mut word = raw.trim_end_matches(['.', '!', '?', '。', '~', ',', ';', ':']);
        // Closed suffix list: do not stem arbitrary Korean words.
        for suffix in [
            "에서", "으로", "에게", "부터", "까지", "의", "을", "를", "은", "는", "이", "가", "에",
            "로",
        ] {
            if word.ends_with(suffix) {
                let stem = &word[..word.len() - suffix.len()];
                if !quoted
                    && (stem.contains('.')
                        || stem.contains('/')
                        || stem.contains('_')
                        || stem.chars().count() >= 2
                            && stem.chars().all(|ch| ('가'..='힣').contains(&ch)))
                {
                    word = stem;
                    break;
                }
            }
        }
        if word.is_empty() {
            continue;
        }
        let mut lower = word.to_lowercase();
        if matches!(lower.as_str(), "a" | "an" | "the") {
            continue;
        }
        let kind = if quoted {
            (raw.len() > ch.len_utf8() && raw.ends_with(ch)).then(|| {
                if ch == '`'
                    && raw[1..raw.len() - 1]
                        .chars()
                        .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | ':'))
                {
                    "identifier"
                } else {
                    "quoted"
                }
            })
        } else if word.chars().any(|ch| ch.is_ascii_digit()) && word.parse::<f64>().is_ok() {
            Some("number")
        } else if word
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '/' | '\\' | '.' | '_' | '-' | ':'))
            && (word.contains('/')
                || word.contains('\\')
                || word.rsplit_once('.').is_some_and(|(stem, ext)| {
                    !stem.is_empty() && !ext.is_empty() && ext.chars().all(char::is_alphanumeric)
                }))
        {
            Some("path")
        } else if word
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | ':'))
            && (word.contains('_')
                || word.contains("::")
                || word.chars().skip(1).any(char::is_uppercase)
                    && word.chars().any(char::is_lowercase)
                    && word.chars().next().is_some_and(char::is_lowercase))
        {
            Some("identifier")
        } else {
            None
        };
        if let Some(kind) = kind {
            result.tokens.push(format!("<{kind}>"));
            if parameters {
                result.parameters.push((kind.into(), word.to_owned()));
                let offset = word.as_ptr() as usize - text.as_ptr() as usize;
                result.spans.push((offset, offset + word.len()));
            }
        } else {
            let ending = match lower.as_str() {
                "고쳐주세요" | "고쳐줘요" | "고쳐줘" | "고쳐" => Some("고쳐"),
                "삭제해주세요" | "삭제해줘" | "삭제해" => Some("삭제해"),
                "실행해주세요" | "실행해줘" | "실행해" => Some("실행해"),
                _ => None,
            };
            if let Some(ending) = ending {
                lower.clear();
                lower.push_str(ending);
            }
            if result.leading_verb.is_empty()
                && matches!(
                    lower.as_str(),
                    "fix"
                        | "delete"
                        | "remove"
                        | "run"
                        | "review"
                        | "explain"
                        | "plan"
                        | "rename"
                        | "add"
                        | "update"
                        | "고쳐"
                        | "삭제해"
                        | "실행해"
                )
            {
                result.leading_verb.clone_from(&lower);
            }
            result.tokens.push(lower);
        }
    }
    if result.leading_verb.is_empty() {
        result.leading_verb = text
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_lowercase();
    }
    result
}

pub fn similar_key(text: &str, classifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let normalized = normalize(text, false);
    let mut digest = Sha256::new();
    for token in std::iter::once(classifier)
        .chain(std::iter::once(normalized.leading_verb.as_str()))
        .chain(normalized.tokens.iter().map(String::as_str))
    {
        digest.update((token.len() as u64).to_le_bytes());
        digest.update(token.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

/// Exact variant table for the teach review, with every observed slot value.
pub fn variant_review(variants: &[String]) -> String {
    let mut text = String::from("\nObserved variants / parameters (saved text stays exact):\n");
    for (index, variant) in variants.iter().enumerate() {
        text.push_str(&format!("Variant {}: {:?}\n", index + 1, variant));
        for (slot, (kind, value)) in similarity(variant).parameters.iter().enumerate() {
            text.push_str(&format!("  {} {} = {:?}\n", kind, slot + 1, value));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    #[test]
    fn xc07_typed_similarity_keeps_operation_and_token_boundaries() {
        let key = |text| super::similar_key(text, "3:Implement");
        assert_eq!(key("fix the bug in src/a.rs"), key("Fix bug in src/b.rs"));
        assert_eq!(key("a.rs 버그 고쳐줘"), key("b.rs의 버그 고쳐줘"));
        assert_eq!(
            key("fix 버그를 a.rs에서 고쳐주세요"),
            key("fix 버그 a.rs 고쳐줘")
        );
        assert_eq!(
            key("fix user_name with 12 and \"old text\""),
            key("fix other_name with 42 and \"new text\"")
        );
        assert_eq!(key("fix userName"), key("fix otherName"));
        for different in [
            "delete src/a.rs",
            "fix src/a.rs now",
            "fix 12",
            "fix \"src/a.rs\"",
        ] {
            assert_ne!(key("fix src/a.rs"), key(different), "{different}");
        }
        assert_ne!(
            key("fix src/a.rs"),
            super::similar_key("fix src/a.rs", "7:Review")
        );
        assert_ne!(key("fix the bug"), key("repair the bug"));
        let normalized =
            super::similarity("fix src/a.rs with 1.5 and \"hello world\" and user_name");
        assert_eq!(
            normalized.parameters,
            vec![
                ("path".into(), "src/a.rs".into()),
                ("number".into(), "1.5".into()),
                ("quoted".into(), "\"hello world\"".into()),
                ("identifier".into(), "user_name".into()),
            ]
        );
        let variants = vec![
            "fix the bug in src/a.rs".into(),
            "Fix bug in src/b.rs".into(),
        ];
        let review = super::variant_review(&variants);
        assert!(review.contains("Variant 1: \"fix the bug in src/a.rs\""));
        assert!(review.contains("path 1 = \"src/b.rs\""));
        assert_eq!(variants[0], "fix the bug in src/a.rs");
    }

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
