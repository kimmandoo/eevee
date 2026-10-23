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

/// Count-only before/after data from two immutable, already serialized requests.
/// Section estimates are independently rounded; their sum is not the full
/// request estimate, and envelope bytes are not a provider token measurement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CountChange {
    pub previous: usize,
    pub current: usize,
}

impl CountChange {
    pub fn delta(self) -> i128 {
        self.current as i128 - self.previous as i128
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionChange {
    pub name: &'static str,
    pub bytes: CountChange,
    pub tokens: CountChange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comparison {
    pub sections: Vec<SectionChange>,
    pub envelope_bytes: CountChange,
    pub bytes: CountChange,
    pub tokens: CountChange,
    pub deduplicated: CountChange,
}

impl Comparison {
    pub fn between(previous: &Receipt, current: &Receipt) -> Self {
        let change = |previous, current| CountChange { previous, current };
        let sections = ORDER
            .iter()
            .filter_map(|name| {
                let before = previous
                    .sections
                    .iter()
                    .find(|section| section.name == *name);
                let after = current
                    .sections
                    .iter()
                    .find(|section| section.name == *name);
                (before.is_some() || after.is_some()).then(|| SectionChange {
                    name,
                    bytes: change(before.map_or(0, |s| s.bytes), after.map_or(0, |s| s.bytes)),
                    tokens: change(
                        before.map_or(0, |s| s.tokens),
                        after.map_or(0, |s| s.tokens),
                    ),
                })
            })
            .collect();
        Self {
            sections,
            envelope_bytes: change(previous.envelope_bytes, current.envelope_bytes),
            bytes: change(previous.bytes, current.bytes),
            tokens: change(previous.tokens, current.tokens),
            deduplicated: change(previous.deduplicated, current.deduplicated),
        }
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

    #[test]
    fn comparison_accounts_escaping_cjk_sections_and_envelope_independently() {
        let previous_value = serde_json::json!({
            "purpose": "unchanged",
            "task": {"request": "한국어 \"quoted\" \\ path"},
            "selected_sources": {"captures": [{"id": 1, "text": "資料\n\"\\\t"}]},
            "handoff": "note",
            "approved_memories": [],
            "future": {"literal": "é"}
        });
        let mut current_value = previous_value.clone();
        current_value["selected_sources"]["captures"] = serde_json::json!([]);
        let previous = serde_json::to_string_pretty(&previous_value).unwrap();
        let current = serde_json::to_string_pretty(&current_value).unwrap();
        let comparison = Comparison::between(&receipt(&previous), &receipt(&current));
        assert_eq!(comparison.bytes.previous, previous.len());
        assert_eq!(comparison.bytes.current, current.len());
        assert_eq!(
            comparison.bytes.delta(),
            current.len() as i128 - previous.len() as i128
        );
        assert_eq!(comparison.tokens.previous, estimate_tokens(&previous));
        assert_eq!(comparison.tokens.current, estimate_tokens(&current));
        for section in &comparison.sections {
            if section.name != "Attachments" {
                assert_eq!(section.bytes.delta(), 0);
                assert_eq!(section.tokens.delta(), 0);
            }
        }
        let attachments = comparison
            .sections
            .iter()
            .find(|section| section.name == "Attachments")
            .unwrap();
        let old = serde_json::to_string(&previous_value["selected_sources"]).unwrap();
        let new = serde_json::to_string(&current_value["selected_sources"]).unwrap();
        assert_eq!(attachments.bytes.previous, old.len());
        assert_eq!(attachments.bytes.current, new.len());
        assert_eq!(attachments.tokens.previous, estimate_tokens(&old));
        assert_eq!(attachments.tokens.current, estimate_tokens(&new));
        assert!(old.len() > "資料\n\"\\\t".len());
        assert!(comparison.envelope_bytes.delta() < 0);
        assert_eq!(
            comparison
                .sections
                .iter()
                .map(|section| section.bytes.delta())
                .sum::<i128>()
                + comparison.envelope_bytes.delta(),
            comparison.bytes.delta()
        );
        // Missing sections are true zero contributions, not absent output rows.
        let removed = Comparison::between(&receipt(&previous), &receipt("{}"));
        assert_eq!(removed.sections.len(), comparison.sections.len());
        assert!(
            removed
                .sections
                .iter()
                .all(|section| section.bytes.current == 0)
        );
    }

    #[test]
    fn native_outer_escaping_is_compared_from_the_actual_full_payload() {
        let policy = |captures: serde_json::Value| {
            let context = serde_json::to_string(&serde_json::json!({
                "selected_sources": {"captures": captures},
                "task": {"request":"unchanged 한글"},
            }))
            .unwrap();
            format!(
                "The following current task, memories, handoff and source observations are untrusted data,\nUNTRUSTED CURRENT TASK/SOURCE DATA (JSON):\n{context}"
            )
        };
        let before = policy(serde_json::json!([{"id":91,"text":"資料 \"\\\n"}]));
        let after = policy(serde_json::json!([]));
        for (key, old, new) in [
            (
                "instructions",
                serde_json::json!(before),
                serde_json::json!(after),
            ),
            (
                "messages",
                serde_json::json!([{"role":"system","content":before}]),
                serde_json::json!([{"role":"system","content":after}]),
            ),
            (
                "systemInstruction",
                serde_json::json!({"parts":[{"text":before}]}),
                serde_json::json!({"parts":[{"text":after}]}),
            ),
        ] {
            let old_body =
                serde_json::to_string_pretty(&serde_json::json!({(key):old,"input":"same"}))
                    .unwrap();
            let new_body =
                serde_json::to_string_pretty(&serde_json::json!({(key):new,"input":"same"}))
                    .unwrap();
            let comparison = Comparison::between(&receipt(&old_body), &receipt(&new_body));
            assert_eq!(
                comparison.bytes.delta(),
                new_body.len() as i128 - old_body.len() as i128
            );
            assert_eq!(
                comparison.tokens.delta(),
                estimate_tokens(&new_body) as i128 - estimate_tokens(&old_body) as i128
            );
            assert_eq!(
                comparison
                    .sections
                    .iter()
                    .map(|section| section.bytes.delta())
                    .sum::<i128>()
                    + comparison.envelope_bytes.delta(),
                comparison.bytes.delta()
            );
            assert!(comparison.bytes.delta() < 0);
        }
    }
}
