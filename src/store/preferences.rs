//! Nonsecret, store-global provider/model preferences; never provider session state.
use super::*;
use crate::providers::{Agent, ModelSelection};
use std::collections::HashMap;

const MAX_PREFERENCE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderPreferences {
    pub provider: Agent,
    pub selections: HashMap<Agent, ModelSelection>,
}

#[derive(Clone, Debug)]
pub enum PreferenceRequest {
    Load,
    Save(ProviderPreferences),
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
