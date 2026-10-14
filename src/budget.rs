//! Local token economy: request receipts, estimates and per-task budget gates.
//! Deterministic and offline: no provider, tokenizer, pricing or telemetry request.
use std::collections::HashSet;

use serde_json::Value;

/// Labelled heuristic; never presented as provider usage or billing.
pub const ESTIMATOR: &str =
    "eevee-local-v1 estimate: ASCII bytes/4, 1 per CJK/kana/Hangul char, 1 per 2 other chars";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Provider model turns committed to this task's journal.
    pub provider_turns: usize,
    /// Local Jolteon command runs recorded for this task.
    pub commands: usize,
    /// Estimated input tokens Eevee itself sends in one request.
    pub input_tokens: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            provider_turns: 20,
            commands: 40,
            input_tokens: 100_000,
        }
    }
}

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
    Receipt {
        envelope_bytes: bytes.saturating_sub(counted),
        sections,
        bytes,
        tokens,
    }
}

impl Receipt {
    pub fn lines(&self, limits: &Limits) -> Vec<String> {
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
        lines.push(format!("JSON keys/indentation: {} B", self.envelope_bytes));
        lines.push(format!(
            "Total sent by Eevee: {} B ≈ {} tok (per-request estimate limit {})",
            self.bytes, self.tokens, limits.input_tokens
        ));
        lines.push("Not counted: provider instructions, tool schemas, resumed history and cache hits. Provider usage and billing are unknown unless the provider reports them; unknown is not zero.".into());
        lines
    }
}

/// Distinct provider turns already committed to this task's journal.
pub fn provider_turns(records: &[crate::store::ProviderRecord]) -> usize {
    records
        .iter()
        .filter(|r| !r.turn_id.is_empty())
        .map(|r| (r.agent, r.thread_id.as_str(), r.turn_id.as_str()))
        .collect::<HashSet<_>>()
        .len()
}

/// Reasons to pause before one more provider turn; empty when within budget.
pub fn provider_gate(turns: usize, tokens: usize, limits: &Limits) -> Vec<String> {
    let mut reasons = Vec::new();
    if turns >= limits.provider_turns {
        reasons.push(format!(
            "Provider turn budget reached: {turns} journaled turns, limit {} for this task (--max-provider-turns).",
            limits.provider_turns
        ));
    }
    if tokens > limits.input_tokens {
        reasons.push(format!(
            "Request estimate ≈ {tokens} tok exceeds the per-request limit {} (--max-input-tokens). Reduce the source basket or continue once.",
            limits.input_tokens
        ));
    }
    reasons
}

/// Reasons to pause before one more local command; `history` is newest first.
pub fn command_gate(
    history: &[crate::runner::RunRecord],
    next: &crate::runner::Prepared,
    limits: &Limits,
) -> Vec<String> {
    use crate::runner::RunState;
    let mut reasons = Vec::new();
    if history.len() >= limits.commands {
        reasons.push(format!(
            "Command budget reached: {} recorded runs, limit {} for this task (--max-commands).",
            history.len(),
            limits.commands
        ));
    }
    // A repeat only counts when nothing observable changed since the identical run
    // finished; an edit, a timeout/cancellation or an unknown snapshot is not a loop.
    if let Some(last) = history.first()
        && last.prepared.spec == next.spec
        && last.prepared.cwd == next.cwd
        && matches!(last.state, RunState::Succeeded | RunState::Failed)
        && !last.cancelled
        && last.after.complete
        && next.before.complete
        && last.after.fingerprint.is_some()
        && last.after.fingerprint == next.before.fingerprint
    {
        reasons.push(format!(
            "Repeated unchanged command: the previous identical run ({}, exit {}) saw the same source fingerprint. Revise it, or continue once if a rerun adds evidence.",
            if last.state == RunState::Succeeded { "succeeded" } else { "failed" },
            last.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into())
        ));
    }
    reasons
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{CommandSpec, InputSnapshot, Prepared, RunRecord, RunState};

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
        assert!(receipt.lines(&Limits::default())[0].contains(ESTIMATOR));
        let raw = super::receipt("[]");
        assert_eq!(raw.sections.len(), 1);
        assert_eq!(raw.envelope_bytes, 0);
    }

    #[test]
    fn provider_gate_counts_distinct_journaled_turns() {
        let record = |turn: &str, item: &str| crate::store::ProviderRecord {
            id: format!("{turn}-{item}"),
            agent: crate::provider::Agent::Codex,
            generation: 1,
            thread_id: "thread".into(),
            turn_id: turn.into(),
            item_id: item.into(),
            origin: String::new(),
            coverage: String::new(),
            text: String::new(),
            complete: true,
        };
        let records = [record("t1", "a"), record("t1", "b"), record("t2", "a")];
        assert_eq!(provider_turns(&records), 2);
        let limits = Limits {
            provider_turns: 2,
            commands: 1,
            input_tokens: 10,
        };
        assert_eq!(provider_gate(1, 10, &limits), Vec::<String>::new());
        assert_eq!(provider_gate(2, 10, &limits).len(), 1);
        assert_eq!(provider_gate(1, 11, &limits).len(), 1);
    }

    fn prepared(args: &[&str], fingerprint: &str) -> Prepared {
        let mut record = RunRecord::awaiting(&Prepared {
            request_id: "r".into(),
            run_id: "run".into(),
            task_id: 1,
            spec: CommandSpec::Direct {
                program: "/bin/cat".into(),
                args: args.iter().map(|a| (*a).into()).collect(),
            },
            cwd: "/project".into(),
            cwd_identity: String::new(),
            executable: crate::runner::ExecutableIdentity {
                path: "/bin/cat".into(),
                sha256: String::new(),
                bytes: 0,
                modified_ns: 0,
                file_id: String::new(),
            },
            prepared_at_ms: 0,
            expires_at_ms: 0,
            before: InputSnapshot::default(),
        });
        record.prepared.before = InputSnapshot {
            fingerprint: Some(fingerprint.into()),
            complete: true,
            stable: true,
        };
        record.prepared
    }

    fn finished(prepared: Prepared, state: RunState, after: &str) -> RunRecord {
        let mut record = RunRecord::awaiting(&prepared);
        record.state = state;
        record.exit_code = Some(0);
        record.after = InputSnapshot {
            fingerprint: Some(after.into()),
            complete: true,
            stable: true,
        };
        record
    }

    #[test]
    fn command_gate_pauses_unchanged_repeats_and_exhausted_budget_only() {
        let limits = Limits::default();
        let history = vec![finished(prepared(&["a"], "f0"), RunState::Succeeded, "f1")];
        assert_eq!(
            command_gate(&history, &prepared(&["a"], "f1"), &limits).len(),
            1
        );
        // Changed source, changed arguments, or a cancelled/unknown previous run proceed.
        assert!(command_gate(&history, &prepared(&["a"], "f2"), &limits).is_empty());
        assert!(command_gate(&history, &prepared(&["b"], "f1"), &limits).is_empty());
        let cancelled = vec![finished(prepared(&["a"], "f0"), RunState::Cancelled, "f1")];
        assert!(command_gate(&cancelled, &prepared(&["a"], "f1"), &limits).is_empty());
        let unknown = vec![finished(
            prepared(&["a"], "f0"),
            RunState::OutcomeUnknown,
            "f1",
        )];
        assert!(command_gate(&unknown, &prepared(&["a"], "f1"), &limits).is_empty());
        let tight = Limits {
            commands: 1,
            ..limits
        };
        let reasons = command_gate(&history, &prepared(&["b"], "f2"), &tight);
        assert_eq!(reasons.len(), 1);
        assert!(reasons[0].contains("Command budget reached"));
    }
}
