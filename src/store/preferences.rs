//! Nonsecret, store-global provider/model preferences; never provider session state.
use super::*;
use crate::providers::{Agent, ModelSelection};
use std::collections::HashMap;

const MAX_PREFERENCE_BYTES: usize = 16 * 1024;

/// Reading density is independent of terminal responsiveness, theme and motion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextDensity {
    #[default]
    Compact,
    Comfortable,
}

impl TextDensity {
    pub fn is_compact(&self) -> bool {
        *self == Self::Compact
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderPreferences {
    pub provider: Agent,
    pub selections: HashMap<Agent, ModelSelection>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub reduced_motion: bool,
    #[serde(default, skip_serializing_if = "TextDensity::is_compact")]
    pub text_density: TextDensity,
}

#[derive(Clone, Debug)]
pub enum PreferenceRequest {
    Load,
    Save(ProviderPreferences),
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn validate_preferences(preferences: &ProviderPreferences) -> Result<(), String> {
    for selection in preferences.selections.values() {
        for value in [selection.model.as_deref(), selection.effort.as_deref()]
            .into_iter()
            .flatten()
        {
            if value.is_empty() || value.len() > 256
                || value.chars().any(|ch| ch.is_whitespace() || ch.is_control()
                    || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
                || crate::workspace::privacy::redact(value) != value
            {
                return Err("Provider preference contains an invalid or credential-like model/effort ID; choose a nonsecret exact ID".into());
            }
        }
    }
    Ok(())
}

pub(super) fn load_preferences(
    connection: &Connection,
) -> Result<Option<ProviderPreferences>, String> {
    let preferences = connection
        .query_row(
            "SELECT record FROM provider_preferences WHERE id=1",
            [],
            |row| json_column::<ProviderPreferences>(row, 0, MAX_PREFERENCE_BYTES),
        )
        .optional()
        .map_err(|error| format!("Cannot load provider preferences: {error}"))?;
    if let Some(preferences) = &preferences {
        validate_preferences(preferences)?;
    }
    Ok(preferences)
}

pub(super) fn inspect_preferences(connection: &Connection) -> Result<(), String> {
    inspect_columns(
        connection,
        "provider_preferences",
        &[("id", "INTEGER", false, 1), ("record", "TEXT", true, 0)],
    )?;
    let invalid: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM provider_preferences WHERE id!=1)",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if invalid {
        return Err("Invalid provider preference row identity".into());
    }
    load_preferences(connection)?;
    Ok(())
}

impl State {
    pub(super) fn preference_request(
        &mut self,
        request: PreferenceRequest,
    ) -> Result<Option<ProviderPreferences>, String> {
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; /storage-retry reopens it")?;
        match request {
            PreferenceRequest::Load => load_preferences(connection),
            PreferenceRequest::Save(preferences) => {
                validate_preferences(&preferences)?;
                let record =
                    serde_json::to_string(&preferences).map_err(|error| error.to_string())?;
                if record.len() > MAX_PREFERENCE_BYTES {
                    return Err("Provider preferences exceed their storage limit".into());
                }
                let transaction = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|error| format!("Cannot begin provider preference save: {error}"))?;
                transaction.execute(
                    "INSERT INTO provider_preferences(id,record) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET record=excluded.record",
                    [record],
                ).map_err(|error| format!("Cannot save provider preferences: {error}"))?;
                enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
                transaction
                    .commit()
                    .map_err(|error| format!("Cannot commit provider preferences: {error}"))?;
                Ok(Some(preferences))
            }
        }
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PreviousPreferences {
        #[serde(rename = "provider")]
        _provider: Agent,
        #[serde(rename = "selections")]
        _selections: HashMap<Agent, ModelSelection>,
    }

    #[test]
    fn legacy_default_is_compatible_but_an_old_strict_reader_refuses_motion_data() {
        let legacy = ProviderPreferences {
            provider: Agent::Codex,
            selections: HashMap::new(),
            reduced_motion: false,
            text_density: TextDensity::Compact,
        };
        let bytes = serde_json::to_vec(&legacy).unwrap();
        assert!(serde_json::from_slice::<PreviousPreferences>(&bytes).is_ok());
        assert!(
            !serde_json::from_slice::<ProviderPreferences>(&bytes)
                .unwrap()
                .reduced_motion
        );
        let reduced = ProviderPreferences {
            reduced_motion: true,
            ..legacy
        };
        let bytes = serde_json::to_vec(&reduced).unwrap();
        assert!(serde_json::from_slice::<PreviousPreferences>(&bytes).is_err());
        assert!(
            serde_json::from_slice::<ProviderPreferences>(&bytes)
                .unwrap()
                .reduced_motion
        );
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Version26Preferences {
        #[serde(rename = "provider")]
        _provider: Agent,
        #[serde(rename = "selections")]
        _selections: HashMap<Agent, ModelSelection>,
        #[serde(default, rename = "reduced_motion")]
        _reduced_motion: bool,
    }

    #[test]
    fn compact_preserves_legacy_json_but_v26_refuses_comfortable_without_rewriting_it() {
        let legacy = br#"{"provider":"Codex","selections":{}}"#;
        let mut preferences: ProviderPreferences = serde_json::from_slice(legacy).unwrap();
        assert_eq!(preferences.text_density, TextDensity::Compact);
        assert_eq!(serde_json::to_vec(&preferences).unwrap(), legacy);
        assert!(serde_json::from_slice::<Version26Preferences>(legacy).is_ok());
        preferences.reduced_motion = true;
        let compact = serde_json::to_vec(&preferences).unwrap();
        assert!(serde_json::from_slice::<Version26Preferences>(&compact).is_ok());
        preferences.text_density = TextDensity::Comfortable;
        let comfortable = serde_json::to_vec(&preferences).unwrap();
        assert!(serde_json::from_slice::<Version26Preferences>(&comfortable).is_err());
        assert_eq!(
            serde_json::from_slice::<ProviderPreferences>(&comfortable).unwrap(),
            preferences
        );
        let unsupported = br#"{"provider":"Codex","selections":{},"text_density":"spacious"}"#;
        assert!(serde_json::from_slice::<ProviderPreferences>(unsupported).is_err());
    }
}
