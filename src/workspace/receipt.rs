//! Local token economy: request receipts and estimates.
//! Deterministic and offline: no provider, tokenizer, pricing or telemetry request.
use serde_json::Value;

/// Labelled heuristic; never presented as provider usage or billing.
pub const ESTIMATOR: &str =
    "eevee-local-v1 estimate: ASCII bytes/4, 1 per CJK/kana/Hangul char, 1 per 2 other chars";

pub fn estimate_tokens(text: &str) -> usize {
    let (mut ascii, mut wide, mut other) = (0usize, 0usize, 0usize);
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii += 1;
        } else if matches!(ch as u32,
            0x1100..=0x11FF | 0x3040..=0x30FF | 0x3130..=0x318F | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
        {
            wide += 1;
        } else {
            other += 1;
        }
    }
    ascii.div_ceil(4) + wide + other.div_ceil(2)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub name: &'static str,
    pub keys: Vec<String>,
    pub bytes: usize,
    pub tokens: usize,
}

/// Breakdown of the exact payload Eevee sends. Provider-added context is not included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub sections: Vec<Section>,
    /// Indentation, keys and separators outside the per-field compact values.
    pub envelope_bytes: usize,
    pub bytes: usize,
    pub tokens: usize,
    /// Captures emitted as a `same_bytes_as` citation instead of duplicated text.
    pub deduplicated: usize,
}

const ORDER: [&str; 6] = [
    "Instructions",
    "Task facts",
    "Attachments",
    "Handoff",
    "Approved memories",
    "Other fields",
];

fn category(key: &str) -> &'static str {
    match key {
        "purpose" | "verification" | "dispatch_condition" => "Instructions",
        "task" | "requirements" | "operation" | "edit_paths" | "initialize" | "optional_method"
        | "params" | "model_prompts_sent" => "Task facts",
        "selected_sources"
        | "selected_source_observations"
        | "edit_baseline"
        | "patch"
        | "patch_identity"
        | "selected_local_runs"
        | "selected_findings_and_references" => "Attachments",
        "handoff" => "Handoff",
        "approved_memories" => "Approved memories",
        _ => "Other fields",
    }
}

pub fn receipt(payload: &str) -> Receipt {
    let bytes = payload.len();
    let tokens = estimate_tokens(payload);
    let mut sections: Vec<Section> = Vec::new();
    match serde_json::from_str::<Value>(payload) {
        Ok(Value::Object(fields)) => {
            for (key, value) in &fields {
                let text = serde_json::to_string(value).unwrap_or_default();
                let name = category(key);
                let section = match sections.iter_mut().find(|s| s.name == name) {
                    Some(section) => section,
                    None => {
                        sections.push(Section {
                            name,
                            keys: Vec::new(),
                            bytes: 0,
                            tokens: 0,
                        });
                        sections.last_mut().expect("just pushed")
                    }
                };
                section.keys.push(key.clone());
                section.bytes += text.len();
                section.tokens += estimate_tokens(&text);
            }
            sections.sort_by_key(|s| ORDER.iter().position(|name| *name == s.name));
        }
        _ => sections.push(Section {
            name: "Other fields",
            keys: vec!["payload".into()],
            bytes,
            tokens,
        }),
    }
    let counted: usize = sections.iter().map(|s| s.bytes).sum();
    let deduplicated = match serde_json::from_str::<Value>(payload) {
        Ok(value) => count_refs(&value),
        Err(_) => 0,
    };
    Receipt {
        envelope_bytes: bytes.saturating_sub(counted),
        sections,
        bytes,
        tokens,
        deduplicated,
    }
}

/// Counts `same_bytes_as` citation markers anywhere inside the payload tree.
fn count_refs(node: &Value) -> usize {
    match node {
        Value::Object(map) => {
            usize::from(map.contains_key("same_bytes_as"))
                + map.values().map(count_refs).sum::<usize>()
        }
        Value::Array(items) => items.iter().map(count_refs).sum(),
        _ => 0,
    }
}

impl Receipt {
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!("REQUEST RECEIPT / local, offline ({ESTIMATOR})")];
        for section in &self.sections {
            lines.push(format!(
                "{}: {} B ≈ {} tok [{}]",
                section.name,
                section.bytes,
                section.tokens,
                section.keys.join(", ")
            ));
        }
        if self.deduplicated > 0 {
            lines.push(format!(
                "Deduplicated locally: {} capture(s) cite earlier identical bytes (same_bytes_as) instead of repeating them; provider-side cache/billing effects are unknown.",
                self.deduplicated
            ));
        }
        lines.push(format!("JSON keys/indentation: {} B", self.envelope_bytes));
        lines.push(format!(
            "Total sent by Eevee: {} B ≈ {} tok",
            self.bytes, self.tokens
        ));
        lines.push("Partner flavor added by Eevee: 0 B (nicknames stay local).".into());
        lines.push("Not counted: provider instructions, tool schemas, resumed history and cache hits. Provider usage and billing are unknown unless the provider reports them; unknown is not zero.".into());
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_are_local_and_cjk_aware() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("한국어"), 3);
        assert_eq!(estimate_tokens("日本語abcd"), 4);
        assert_eq!(estimate_tokens("éé"), 1);
    }

    #[test]
    fn receipt_attributes_every_byte_without_double_counting() {
        let payload = serde_json::to_string_pretty(&serde_json::json!({
            "purpose": "Plan only.",
            "task": {"goal": "fix"},
            "selected_sources": [{"path": "a.rs", "text": "fn main() {}"}],
            "edit_baseline": [],
            "approved_memories": [],
            "future": 1
        }))
        .unwrap();
        let receipt = receipt(&payload);
        let names: Vec<_> = receipt.sections.iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            [
                "Instructions",
                "Task facts",
                "Attachments",
                "Approved memories",
                "Other fields"
            ]
        );
        let attachments = &receipt.sections[2];
        assert_eq!(attachments.keys, ["edit_baseline", "selected_sources"]);
        let counted: usize = receipt.sections.iter().map(|s| s.bytes).sum();
        assert_eq!(counted + receipt.envelope_bytes, payload.len());
        assert_eq!(receipt.bytes, payload.len());
        assert!(receipt.lines()[0].contains(ESTIMATOR));
        let raw = super::receipt("[]");
        assert_eq!(raw.sections.len(), 1);
        assert_eq!(raw.envelope_bytes, 0);
    }
}
