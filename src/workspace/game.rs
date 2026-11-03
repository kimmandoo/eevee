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
/// Language-agnostic: typed slots come from character classes and script
/// boundaries, never from per-language word lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Similarity {
    /// Lowercased text with each slot replaced by its kind marker.
    pub shape: String,
    pub parameters: Vec<(String, String)>,
    pub spans: Vec<(usize, usize)>,
}

/// Slot-abstracted shapes longer than this compare exactly, bounding work.
const FUZZY_LIMIT: usize = 512;
/// Minimum character similarity of two shapes in one proposal group.
const SIMILAR: f64 = 0.75;

pub fn similarity(text: &str) -> Similarity {
    let mut result = Similarity {
        shape: String::new(),
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
        let word = raw.trim_end_matches(|ch: char| {
            ch.is_ascii_punctuation() && !matches!(ch, '"' | '\'' | '`')
                || matches!(ch, '。' | '、' | '~')
        });
        if word.is_empty() {
            continue;
        }
        // A slot written in ASCII may carry attached text in another script
        // (an inflection or particle); the slot ends at that boundary.
        let (slot_text, rest) = if quoted {
            (word, "")
        } else {
            let split = word
                .char_indices()
                .find(|(_, ch)| !ch.is_ascii())
                .map_or(word.len(), |(index, _)| index);
            if split > 0 && word.is_char_boundary(split) {
                (&word[..split], &word[split..])
            } else {
                (word, "")
            }
        };
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
        } else {
            slot_kind(slot_text)
        };
        if !result.shape.is_empty() {
            result.shape.push(' ');
        }
        match kind {
            Some(kind) => {
                let value = if quoted { raw } else { slot_text };
                result.shape.push_str(&format!("<{kind}>"));
                result.shape.push_str(&rest.to_lowercase());
                result.parameters.push((kind.into(), value.to_owned()));
                let offset = value.as_ptr() as usize - text.as_ptr() as usize;
                result.spans.push((offset, offset + value.len()));
            }
            None => result.shape.push_str(&word.to_lowercase()),
        }
    }
    result
}

fn slot_kind(word: &str) -> Option<&'static str> {
    if word.is_empty() {
        None
    } else if word.chars().any(|ch| ch.is_ascii_digit()) && word.parse::<f64>().is_ok() {
        Some("number")
    } else if word
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '\\' | '.' | '_' | '-' | ':'))
        && (word.contains('/')
            || word.contains('\\')
            || word.rsplit_once('.').is_some_and(|(stem, ext)| {
                !stem.is_empty()
                    && !ext.is_empty()
                    && ext.chars().all(|ch| ch.is_ascii_alphanumeric())
            }))
    {
        Some("path")
    } else if word
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':'))
        && (word.contains('_')
            || word.contains("::")
            || word.chars().skip(1).any(|ch| ch.is_ascii_uppercase())
                && word.chars().any(|ch| ch.is_ascii_lowercase())
                && word
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_lowercase()))
    {
        Some("identifier")
    } else {
        None
    }
}

fn slot_kinds(similarity: &Similarity) -> impl Iterator<Item = &str> {
    similarity.parameters.iter().map(|(kind, _)| kind.as_str())
}

/// Whether two instructions are variants of one reusable request: the same
/// slot kinds in the same order and nearly the same remaining text. Small
/// wording differences (articles, particles, politeness endings) merge in any
/// language; different requests do not.
pub fn similar(a: &str, b: &str) -> bool {
    let (a, b) = (similarity(a), similarity(b));
    if !slot_kinds(&a).eq(slot_kinds(&b)) {
        return false;
    }
    if a.shape == b.shape {
        return true;
    }
    let (x, y): (Vec<char>, Vec<char>) = (a.shape.chars().collect(), b.shape.chars().collect());
    if x.len().max(y.len()) > FUZZY_LIMIT {
        return false;
    }
    let longest = x.len().max(y.len()) as f64;
    1.0 - edit_distance(&x, &y) as f64 / longest >= SIMILAR
}

fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, x) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            current[j + 1] = (previous[j] + usize::from(x != y))
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// Stable identity of a proposal group, from its representative instruction.
pub fn similar_key(text: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(similarity(text).shape.as_bytes()))
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
    fn similarity_merges_small_wording_differences_in_any_language_without_word_lists() {
        use super::similar;
        for (a, b) in [
            ("fix the bug in src/a.rs", "Fix bug in src/b.rs"),
            ("a.rs 버그 고쳐줘", "b.rs의 버그 고쳐줘"),
            ("a.rs 버그 고쳐줘", "b.rs 버그 고쳐주세요"),
            (
                "fix user_name with 12 and \"old text\"",
                "fix other_name with 42 and \"new text\"",
            ),
            ("fix userName", "fix otherName"),
            (
                "Ejecuta las pruebas de src/a.rs",
                "ejecuta pruebas de src/b.rs",
            ),
        ] {
            assert!(similar(a, b), "{a} ~ {b}");
        }
        for (a, b) in [
            ("fix src/a.rs", "delete src/a.rs"),
            ("fix src/a.rs", "fix 12"),
            ("fix src/a.rs", "fix \"src/a.rs\""),
            ("fix the bug", "write the release notes"),
            ("테스트 돌려줘", "문서 정리해줘"),
        ] {
            assert!(!similar(a, b), "{a} !~ {b}");
        }
        // A slot ends where attached text in another script begins.
        let normalized = super::similarity("b.rs의 버그와 1.5 \"hello world\" user_name");
        assert_eq!(
            normalized.parameters,
            vec![
                ("path".into(), "b.rs".into()),
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
