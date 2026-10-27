//! Inert user-authored text in the task's strict workflow record; never work.
use serde::{Deserialize, Serialize};

pub const MAX_PER_PARTNER: usize = 8;
pub const MAX_TEXT_BYTES: usize = 4096;
pub const MAX_PARTNER_BYTES: usize = 16 * 1024;
pub const MAX_TASK_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: u64,
    pub partner: i64,
    pub text: String,
}

pub fn validate(entries: &[Entry]) -> Result<(), String> {
    if entries.len() > 6 * MAX_PER_PARTNER
        || entries.iter().map(|e| e.text.len()).sum::<usize>() > MAX_TASK_BYTES
    {
        return Err("Parked drafts exceed the task limit (48 entries / 64 KiB)".into());
    }
    for (index, entry) in entries.iter().enumerate() {
        if entry.id == 0
            || entry.partner < 1
            || entry.text.is_empty()
            || entry.text.len() > MAX_TEXT_BYTES
            || entries[..index].iter().any(|old| old.id == entry.id)
        {
            return Err("Invalid parked draft identity, text or 4096-byte bound".into());
        }
        let mut chars = entry.text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                continue;
            }
            if ch.is_control() && !matches!(ch, '\n' | '\t')
                || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                return Err("Parked text contains unsafe controls (only LF, paired CRLF and tab are accepted)".into());
            }
        }
        let owned = entries
            .iter()
            .filter(|other| other.partner == entry.partner);
        if owned.clone().count() > MAX_PER_PARTNER
            || owned.map(|e| e.text.len()).sum::<usize>() > MAX_PARTNER_BYTES
        {
            return Err("Parked drafts exceed the partner limit (8 entries / 16 KiB)".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_strict_text_records_preserve_unicode_and_literal_newlines() {
        let entry = Entry {
            id: 1,
            partner: 2,
            text: "한글\r\nfollow-up\n".into(),
        };
        validate(std::slice::from_ref(&entry)).unwrap();
        assert_eq!(
            serde_json::from_str::<Entry>(&serde_json::to_string(&entry).unwrap()).unwrap(),
            entry
        );
        assert!(
            serde_json::from_str::<Entry>(r#"{"id":1,"partner":2,"text":"x","grant":true}"#)
                .is_err()
        );
        assert!(validate(&vec![entry.clone(); 2]).is_err());
        for text in ["lone\r", "\u{202e}hidden", "\u{061c}", "\u{1b}[31m"] {
            assert!(
                validate(&[Entry {
                    text: text.into(),
                    ..entry.clone()
                }])
                .is_err()
            );
        }
        assert!(
            validate(&[Entry {
                text: "x".repeat(MAX_TEXT_BYTES + 1),
                ..entry
            }])
            .is_err()
        );
        let entries: Vec<_> = (1..=5)
            .map(|id| Entry {
                id,
                partner: 1,
                text: "x".repeat(4096),
            })
            .collect();
        assert!(validate(&entries).is_err());
    }
}
