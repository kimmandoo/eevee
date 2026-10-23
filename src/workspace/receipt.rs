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

/// Content-free historical account of one actual outgoing request. Source bytes
/// live separately so deletion never changes the original identity or counts.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Durable {
    pub request_id: String,
    pub task_id: i64,
    pub owner: crate::store::Attribution,
    pub writing: bool,
    pub generation: u64,
    pub agent: crate::providers::Agent,
    pub operation: crate::providers::Operation,
    pub recorded_at: i64,
    pub estimator: String,
    pub sha256: [u8; 32],
    pub sections: Vec<DurableSection>,
    pub envelope_bytes: usize,
    pub bytes: usize,
    pub tokens: usize,
    pub deduplicated: usize,
    pub captures: Vec<CaptureReference>,
    /// Other explicit source identities, never their text, paths or descriptions.
    pub references: Vec<SourceReference>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableSection {
    pub name: String,
    pub keys: Vec<String>,
    pub bytes: usize,
    pub tokens: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureReference {
    pub id: i64,
    pub bytes: usize,
    pub tokens: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceReference {
    pub kind: String,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outgoing {
    pub record: Durable,
    /// Exact reviewed application envelope, never HTTP headers or private reasoning.
    /// Recognizable secrets are not persisted even when the user sent them.
    pub source: Option<String>,
}

impl Outgoing {
    pub fn frozen(
        payload: &str,
        claim: &crate::store::WorkClaim,
        generation: u64,
        agent: crate::providers::Agent,
        operation: crate::providers::Operation,
        recorded_at: i64,
    ) -> Result<Self, String> {
        use sha2::{Digest, Sha256};
        if payload.len() > 8 * 1024 * 1024 {
            return Err("Outgoing receipt exceeds the 8 MiB source limit".into());
        }
        let measured = receipt(payload);
        let wire = super::context::wire_receipt(payload)?;
        let mut references = Vec::new();
        source_references(
            &serde_json::from_str::<Value>(payload).map_err(|e| e.to_string())?,
            None,
            &mut references,
        );
        let record = Durable {
            request_id: claim.request_id.clone(),
            task_id: claim.task_id,
            owner: claim.owner.clone(),
            writing: claim.writing,
            generation,
            agent,
            operation,
            recorded_at,
            estimator: ESTIMATOR.into(),
            sha256: Sha256::digest(payload.as_bytes()).into(),
            sections: measured
                .sections
                .into_iter()
                .map(|s| DurableSection {
                    name: s.name.into(),
                    keys: s
                        .keys
                        .into_iter()
                        .map(|key| super::privacy::redact(&key).into_owned())
                        .collect(),
                    bytes: s.bytes,
                    tokens: s.tokens,
                })
                .collect(),
            envelope_bytes: measured.envelope_bytes,
            bytes: measured.bytes,
            tokens: measured.tokens,
            deduplicated: measured.deduplicated,
            captures: wire
                .captures
                .into_iter()
                .map(|c| CaptureReference {
                    id: c.id,
                    bytes: c.bytes,
                    tokens: c.estimated_tokens,
                })
                .collect(),
            references,
        };
        record.validate()?;
        Ok(Self {
            record,
            source: (super::privacy::redact(payload) == payload).then(|| payload.to_owned()),
        })
    }
}

impl Durable {
    pub fn claim(&self) -> crate::store::WorkClaim {
        crate::store::WorkClaim {
            task_id: self.task_id,
            owner: self.owner.clone(),
            request_id: self.request_id.clone(),
            writing: self.writing,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.owner.validate()?;
        if super::privacy::redact(&self.owner.name) != self.owner.name.as_str() {
            return Err(
                "Outgoing receipt owner metadata contains a recognizable credential".into(),
            );
        }
        if self.task_id <= 0
            || self.generation == 0
            || self.recorded_at < 0
            || self.request_id.is_empty()
            || self.request_id.len() > 128
            || !self
                .request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
            || self.estimator != ESTIMATOR
            || self.bytes > 8 * 1024 * 1024
            || self.tokens > self.bytes
            || self.envelope_bytes > self.bytes
            || self.sections.len() > ORDER.len()
            || self.captures.len() > 256
            || self.references.len() > 256
        {
            return Err(
                "Outgoing receipt has invalid identity, estimator or bounded counts".into(),
            );
        }
        let mut bytes = self.envelope_bytes;
        for (index, section) in self.sections.iter().enumerate() {
            if !ORDER.contains(&section.name.as_str())
                || self.sections[..index]
                    .iter()
                    .any(|s| s.name == section.name)
                || section.keys.len() > 256
                || section.tokens > section.bytes
                || section.keys.iter().any(|k| {
                    k.len() > 1024
                        || k.chars().any(char::is_control)
                        || super::privacy::redact(k) != k.as_str()
                })
            {
                return Err("Outgoing section receipt is invalid".into());
            }
            bytes = bytes
                .checked_add(section.bytes)
                .ok_or("Outgoing count overflow")?;
        }
        if bytes != self.bytes
            || self
                .captures
                .iter()
                .any(|c| c.id <= 0 || c.bytes > self.bytes || c.tokens > c.bytes)
            || self.references.iter().any(|r| {
                !matches!(
                    r.kind.as_str(),
                    "memory" | "run" | "handoff" | "finding" | "artifact"
                ) || r.id.is_empty()
                    || r.id.len() > 1024
                    || r.id.chars().any(char::is_control)
                    || super::privacy::redact(&r.id) != r.id.as_str()
            })
        {
            return Err("Outgoing receipt counts or source references are invalid".into());
        }
        Ok(())
    }
}

fn source_references(node: &Value, kind: Option<&str>, out: &mut Vec<SourceReference>) {
    match node {
        Value::Object(fields) => {
            if let Some(kind) = kind {
                for key in ["id", "run_id", "artifact_id"] {
                    if let Some(value) = fields.get(key) {
                        let id = match value {
                            Value::String(id) => id.clone(),
                            Value::Number(id) => id.to_string(),
                            _ => continue,
                        };
                        let reference = SourceReference { kind: kind.into(), id };
                        if out.len() <= 256 && !out.contains(&reference) {
                            out.push(reference);
                        }
                    }
                }
            }
            for (key, value) in fields {
                let next = match key.as_str() {
                    "approved_memories" => Some("memory"),
                    "selected_local_runs" => Some("run"),
                    "handoff" | "accepted_handoff" => Some("handoff"),
                    "selected_findings_and_references" => Some("finding"),
                    "tool_artifacts" | "selected_tool_artifacts" => Some("artifact"),
                    _ => kind,
                };
                source_references(value, next, out);
            }
        }
        Value::Array(items) => for item in items { source_references(item, kind, out); },
        Value::String(text) if text.contains("\nThe following current task, memories, handoff and source observations are untrusted data,") => {
            if let Some((_, context)) = text.split_once("\nUNTRUSTED CURRENT TASK/SOURCE DATA (JSON):\n")
                && let Ok(context) = serde_json::from_str::<Value>(context)
            {
                source_references(&context, None, out);
            }
        }
        _ => {}
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
