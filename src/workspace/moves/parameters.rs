//! Reviewed request-only substitution. Byte ranges preserve every non-slot byte.
use super::{Proposal, Steps};
use serde::{Deserialize, Serialize};

pub const KEY: &str = "request_parameters";
pub const MAX_SLOTS: usize = 16;
pub const MAX_INSTRUCTION: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Path,
    Quoted,
    Number,
    Identifier,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
    pub default: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub slots: Vec<Slot>,
    /// Observed text only, not portable source identity or proof of dispatch.
    pub variants: Vec<String>,
}

impl Template {
    pub fn of(proposal: &Proposal) -> Result<Option<Self>, String> {
        proposal
            .extra
            .0
            .get(KEY)
            .map(|value| {
                Self::deserialize(value)
                    .map_err(|_| "Invalid typed request template; nothing substituted".into())
            })
            .transpose()
    }

    pub fn derive(steps: &Steps, variants: Vec<String>) -> Result<Option<Self>, String> {
        let Steps::Request { instruction, .. } = steps else {
            return Ok(None);
        };
        let normalized = crate::workspace::game::similarity(instruction);
        let mut slots = Vec::with_capacity(normalized.spans.len());
        for ((kind, _), (mut start, mut end)) in
            normalized.parameters.into_iter().zip(normalized.spans)
        {
            let kind = match kind.as_str() {
                "path" => Kind::Path,
                "quoted" => Kind::Quoted,
                "number" => Kind::Number,
                "identifier" => Kind::Identifier,
                _ => return Err("Unsupported request parameter type".into()),
            };
            if matches!(instruction.as_bytes()[start], b'\'' | b'"' | b'`') {
                start += 1;
                end -= 1;
            }
            slots.push(Slot {
                kind,
                start,
                end,
                default: instruction[start..end].into(),
            });
        }
        if slots.is_empty() {
            return Ok(None);
        }
        let template = Self { slots, variants };
        template.validate(steps)?;
        Ok(Some(template))
    }

    pub fn store(&self, proposal: &mut Proposal) -> Result<(), String> {
        self.validate(&proposal.steps)?;
        proposal.extra.0.insert(
            KEY.into(),
            serde_json::to_value(self).map_err(|_| "Cannot retain request parameters")?,
        );
        let mut critical: Vec<String> = proposal
            .extra
            .0
            .get("critical")
            .map(Vec::<String>::deserialize)
            .transpose()
            .map_err(|_| "Invalid move critical metadata")?
            .unwrap_or_default();
        crate::workspace::forward::set_critical(&mut critical, KEY, true);
        proposal.extra.0.insert(
            "critical".into(),
            serde_json::to_value(critical).map_err(|_| "Cannot retain move critical fields")?,
        );
        Ok(())
    }

    pub fn validate(&self, steps: &Steps) -> Result<(), String> {
        let Steps::Request { instruction, .. } = steps else {
            return Err("Parameters require a request move".into());
        };
        if instruction.len() > MAX_INSTRUCTION
            || self.slots.is_empty()
            || self.slots.len() > MAX_SLOTS
            || self.variants.is_empty()
            || self.variants.len() > 64
        {
            return Err("Request templates require 1–16 slots, 1–64 variants and at most 4096 instruction bytes".into());
        }
        let mut end = 0;
        for slot in &self.slots {
            if slot.start < end
                || slot.end < slot.start
                || instruction.get(slot.start..slot.end) != Some(slot.default.as_str())
            {
                return Err(
                    "Request slot ranges/defaults differ from the exact saved instruction".into(),
                );
            }
            self.validate_value(instruction, slot, &slot.default)?;
            end = slot.end;
        }
        for variant in &self.variants {
            if variant.len() > MAX_INSTRUCTION {
                return Err("Observed variant exceeds 4096 bytes".into());
            }
            super::user_text(variant)?;
        }
        Ok(())
    }

    fn validate_value(&self, literal: &str, slot: &Slot, value: &str) -> Result<(), String> {
        if value.len() > MAX_INSTRUCTION || value.chars().any(char::is_control) {
            return Err("Parameter values must be single-line safe text within 4096 bytes".into());
        }
        super::recoverable_text(value)?;
        match slot.kind {
            Kind::Path => super::literal_path(value)?,
            Kind::Number if value.is_empty() || !value.chars().any(|ch| ch.is_ascii_digit()) || value.parse::<f64>().is_err() => return Err("Number parameter requires a literal number".into()),
            Kind::Identifier if !value.split("::").all(|part| {
                let mut chars = part.chars();
                chars.next().is_some_and(|ch| ch.is_alphabetic() || ch == '_')
                    && chars.all(|ch| ch.is_alphanumeric() || ch == '_')
            }) => return Err("Identifier parameter requires a letter/underscore-led name, optionally separated by ::".into()),
            Kind::Quoted => {
                let delimiter = slot.start.checked_sub(1).and_then(|at| literal.as_bytes().get(at)).copied();
                if !matches!(delimiter, Some(b'\'' | b'"' | b'`')) || literal.as_bytes().get(slot.end).copied() != delimiter
                    || value != slot.default && (value.contains(delimiter.unwrap() as char) || value.contains('\\')) {
                    return Err("Quoted parameter must retain its delimiters and cannot contain that quote or a backslash".into());
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn defaults_json(&self) -> String {
        serde_json::to_string(
            &self
                .slots
                .iter()
                .map(|slot| slot.default.as_str())
                .collect::<Vec<_>>(),
        )
        .expect("string list serialization")
    }

    pub fn fill(&self, steps: &Steps, json: &str) -> Result<String, String> {
        self.validate(steps)?;
        if json.len() > MAX_INSTRUCTION {
            return Err("Parameter input exceeds 4096 bytes".into());
        }
        let values: Vec<String> = serde_json::from_str(json)
            .map_err(|_| "Enter a JSON array with one string for each parameter")?;
        if values.len() != self.slots.len() {
            return Err("Parameter count must exactly match the reviewed template".into());
        }
        let Steps::Request { instruction, .. } = steps else {
            unreachable!()
        };
        let bytes = instruction.len()
            - self
                .slots
                .iter()
                .map(|slot| slot.default.len())
                .sum::<usize>()
            + values.iter().map(String::len).sum::<usize>();
        if bytes > MAX_INSTRUCTION {
            return Err("Filled instruction exceeds 4096 bytes".into());
        }
        let mut output = String::with_capacity(bytes);
        let mut from = 0;
        for (slot, value) in self.slots.iter().zip(values) {
            self.validate_value(instruction, slot, &value)?;
            output.push_str(&instruction[from..slot.start]);
            output.push_str(&value);
            from = slot.end;
        }
        output.push_str(&instruction[from..]);
        super::user_text(&output)?;
        Ok(output)
    }

    pub fn review(&self, steps: &Steps, json: &str) -> Result<(String, String), String> {
        let output = self.fill(steps, json)?;
        let Steps::Request { instruction, .. } = steps else {
            unreachable!()
        };
        let mut details = format!(
            "Fill request parameters / no Send or approval\nExact saved literal: {:?}\nValues (in slot order): {}\n",
            instruction, json
        );
        for (index, slot) in self.slots.iter().enumerate() {
            details.push_str(&format!(
                "Slot {} {:?} bytes {}..{} / default {:?}\n",
                index + 1,
                slot.kind,
                slot.start,
                slot.end,
                slot.default
            ));
        }
        details.push_str(&format!("Complete substituted instruction:\n{}\nOnly reviewed slot bytes change. Ordinary outgoing receipt and Run remain required.\n", output));
        Ok((output, details))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slots_replace_only_reviewed_bytes_and_reject_wrong_types_or_ranges() {
        let steps = Steps::Request {
            instruction: "Fix the bug in src/b.rs!".into(),
            pins: vec![],
        };
        let template = Template::derive(
            &steps,
            vec![
                "fix the bug in src/a.rs".into(),
                "Fix bug in src/b.rs".into(),
            ],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            template.fill(&steps, "[\"src/c.rs\"]").unwrap(),
            "Fix the bug in src/c.rs!"
        );
        for input in [
            "[]",
            "[1]",
            "[\"../outside.rs\"]",
            "[\".env\"]",
            "[\"src/c.rs\",\"extra\"]",
        ] {
            assert!(template.fill(&steps, input).is_err(), "{input}");
        }
        let mut stale = template.clone();
        stale.slots[0].start = 1;
        assert!(stale.fill(&steps, "[\"src/c.rs\"]").is_err());
        let korean = Steps::Request {
            instruction: "b.rs의 버그 고쳐줘".into(),
            pins: vec![],
        };
        let template = Template::derive(&korean, vec!["a.rs 버그 고쳐줘".into()])
            .unwrap()
            .unwrap();
        assert_eq!(
            template.fill(&korean, "[\"c.rs\"]").unwrap(),
            "c.rs의 버그 고쳐줘"
        );
        let quoted = Steps::Request {
            instruction: "fix \"hello world\" and user_name with 12".into(),
            pins: vec![],
        };
        let template = Template::derive(
            &quoted,
            vec!["fix \"hello world\" and user_name with 12".into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            template
                .fill(&quoted, "[\"new text\",\"other_name\",\"42\"]")
                .unwrap(),
            "fix \"new text\" and other_name with 42"
        );
    }
    #[test]
    fn xc07_behavioral_parameters_require_known_critical_admission_and_preserve_extras() {
        let mut proposal = Proposal {
            partner: 1,
            form: 3,
            name: "Typed fix".into(),
            scope: super::super::Scope::Project,
            steps: Steps::Request {
                instruction: "fix src/a.rs".into(),
                pins: vec![],
            },
            provenance: super::super::Provenance {
                task_id: 1,
                sources: vec![],
                taught_by: None,
                teacher_name: None,
            },
            extra: Default::default(),
        };
        let template = Template::derive(&proposal.steps, vec!["fix src/a.rs".into()])
            .unwrap()
            .unwrap();
        template.store(&mut proposal).unwrap();
        assert_eq!(proposal.extra.0["critical"], serde_json::json!([KEY]));
        super::super::validate_proposal(&proposal).unwrap();
        proposal.extra.0.insert(
            "future_hint".into(),
            serde_json::json!({"bytes": [1, 2, 3]}),
        );
        let json = serde_json::to_value(&proposal).unwrap();
        let mut roundtrip: Proposal = serde_json::from_value(json.clone()).unwrap();
        roundtrip.name = "Renamed typed fix".into();
        assert_eq!(roundtrip.extra, proposal.extra);
        super::super::validate_proposal(&roundtrip).unwrap();
        roundtrip
            .extra
            .0
            .insert("critical".into(), serde_json::json!([KEY, "future_hint"]));
        assert!(
            super::super::validate_proposal(&roundtrip)
                .unwrap_err()
                .contains("future_hint")
        );
        roundtrip
            .extra
            .0
            .insert("critical".into(), serde_json::json!(false));
        assert!(super::super::validate_proposal(&roundtrip).is_err());
        roundtrip.extra.0.remove("critical");
        assert!(
            super::super::validate_proposal(&roundtrip)
                .unwrap_err()
                .contains("critical-field marker")
        );
        roundtrip.extra.0.remove(KEY);
        roundtrip
            .extra
            .0
            .insert("quick_slot".into(), serde_json::json!(0));
        super::super::validate_proposal(&roundtrip).unwrap();
    }
}
