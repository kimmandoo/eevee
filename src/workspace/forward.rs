//! Forward-compatible task data (XC08). Fields a newer Eevee adds are kept
//! byte-for-byte by this reader and written back unchanged, unless the newer
//! writer declared them *critical* — fields whose absence would change privacy,
//! cost or effects. An unknown critical field refuses the task, as before.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Unknown fields of one struct, preserved for round-trip.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Extra(pub BTreeMap<String, Value>);

impl Extra {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn contains(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }
}

/// Refuses when any declared critical path names a field this reader does not
/// know. `known` resolves the first path segment to the nested struct's extras
/// (for `progress.eager_off`, the `progress` extras); `top` is the top-level one.
pub fn check_critical<'a>(
    critical: &[String],
    top: &Extra,
    nested: impl Fn(&str) -> Option<&'a Extra>,
) -> Result<(), String> {
    for path in critical {
        let (head, rest) = match path.split_once('.') {
            Some((head, rest)) => (head, Some(rest)),
            None => (path.as_str(), None),
        };
        let unknown = top.contains(head)
            || rest.is_some_and(|field| {
                let field = field.split('.').next().unwrap_or(field);
                nested(head).is_some_and(|extra| extra.contains(field))
            });
        if unknown {
            return Err(format!(
                "This task uses `{path}` from a newer Eevee and needs that version to open safely; nothing was changed"
            ));
        }
    }
    Ok(())
}

pub fn set_critical(critical: &mut Vec<String>, path: &str, required: bool) {
    let present = critical.iter().any(|known| known == path);
    if required && !present {
        critical.push(path.into());
        critical.sort();
    } else if !required && present {
        critical.retain(|known| known != path);
    }
}

#[cfg(test)]
mod tests {
    use crate::workspace::workflows::State;
    use serde_json::json;

    fn task(value: serde_json::Value) -> State {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn newer_fields_round_trip_unchanged_through_this_reader() {
        let original = json!({
            "review_generation": 3,
            "future_panel": {"open": true, "rows": [1, 2]},
            "progress": {
                "revision": 1, "adopted_revision": null, "source": "user", "steps": [],
                "adopted_steps": [], "eager": true, "future_hint": "x"
            },
            "game": {"dismissed_moves": ["a".repeat(64)], "future_pocket": [7]}
        });
        let mut state = task(original.clone());
        state.validate(1).unwrap();
        assert!(state.extra.contains("future_panel"));
        assert!(
            state
                .progress
                .as_ref()
                .unwrap()
                .extra
                .contains("future_hint")
        );
        // Editing a known field keeps every unknown one.
        state.review_generation = 4;
        let mut written = serde_json::to_value(&state).unwrap();
        assert_eq!(written["future_panel"], original["future_panel"]);
        assert_eq!(written["progress"]["future_hint"], "x");
        assert_eq!(written["game"]["future_pocket"], json!([7]));
        written["review_generation"] = json!(3);
        assert_eq!(task(written), task(original));
    }

    #[test]
    fn unknown_critical_fields_refuse_and_known_ones_do_not() {
        let refused = task(json!({"critical": ["future_panel"], "future_panel": 1}));
        assert!(refused.validate(1).unwrap_err().contains("future_panel"));
        let nested = task(json!({
            "critical": ["progress.future_hint"],
            "progress": {"revision": 0, "adopted_revision": null, "source": "", "steps": [],
                         "adopted_steps": [], "eager": false, "future_hint": 1}
        }));
        assert!(
            nested
                .validate(1)
                .unwrap_err()
                .contains("progress.future_hint")
        );
        let known = task(json!({
            "critical": ["progress.eager_off"],
            "progress": {"revision": 0, "adopted_revision": null, "source": "", "steps": [],
                         "adopted_steps": [], "eager": false, "eager_off": true}
        }));
        known.validate(1).unwrap();
        // A critical name with no data present is not a refusal.
        task(json!({"critical": ["absent_feature"]}))
            .validate(1)
            .unwrap();
    }

    #[test]
    fn set_critical_is_idempotent_and_sorted() {
        let mut critical = vec!["z".to_string()];
        super::set_critical(&mut critical, "a.b", true);
        super::set_critical(&mut critical, "a.b", true);
        assert_eq!(critical, ["a.b", "z"]);
        super::set_critical(&mut critical, "a.b", false);
        assert_eq!(critical, ["z"]);
    }
}
