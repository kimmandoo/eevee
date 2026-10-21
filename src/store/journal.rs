//! Append-only provider journal records.
use super::*;

impl State {
    pub(super) fn journal_request(
        &mut self,
        request: JournalRequest,
    ) -> Result<JournalResponse, String> {
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage unavailable; provider evidence was not saved")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let task = load_task(&transaction, &self.project, request.task_id)?
            .ok_or("Provider journal task is missing or belongs to another project")?;
        let response = match request.operation {
            JournalOperation::Put(record) => {
                let mut record = *record;
                if task.archived {
                    return Err("Archived tasks cannot accept provider evidence".into());
                }
                validate_provider_record(&record)?;
                sanitize_provider_record(&mut record);
                validate_provider_record(&record)?;
                let json = serde_json::to_string(&record).map_err(|error| error.to_string())?;
                if json.len() > 65536 {
                    return Err("Provider journal record exceeds 64 KiB".into());
                }
                let existing: Option<(i64, String)> = transaction
                    .query_row(
                        "SELECT task_id,record FROM provider_journal WHERE id=?1",
                        [&record.id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
                    .map_err(|error| error.to_string())?;
                if let Some((owner, saved)) = existing {
                    if owner != request.task_id || saved != json {
                        return Err("Provider record ID cannot overwrite different task/origin/content; append a new identity".into());
                    }
                } else {
                    let count: i64 = transaction
                        .query_row(
                            "SELECT count(*) FROM provider_journal WHERE task_id=?1",
                            [request.task_id],
                            |row| row.get(0),
                        )
                        .map_err(|error| error.to_string())?;
                    if count >= 256 {
                        return Err("Provider journal reached 256 records for this task; explicitly purge before capturing more".into());
                    }
                    transaction
                        .execute(
                            "INSERT INTO provider_journal(id,task_id,record) VALUES(?1,?2,?3)",
                            params![record.id, request.task_id, json],
                        )
                        .map_err(|error| format!("Provider evidence was not saved: {error}"))?;
                }
                super::records::observe_provider(
                    &transaction,
                    &self.project,
                    request.task_id,
                    &record,
                )?;
                enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
                JournalResponse::Saved(record)
            }
            JournalOperation::List => {
                let mut statement = transaction.prepare("SELECT record FROM provider_journal WHERE task_id=?1 ORDER BY rowid LIMIT 257").map_err(|error| error.to_string())?;
                let mut records = statement
                    .query_map([request.task_id], |row| {
                        json_column::<ProviderRecord>(row, 0, 65536)
                    })
                    .map_err(|error| error.to_string())?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|error| error.to_string())?;
                if records.len() > 256 {
                    return Err("Provider journal exceeds bounded capacity".into());
                }
                for record in &mut records {
                    validate_provider_record(record)?;
                    // Older observations may predate the current masking rules. Keep the
                    // append-only bytes intact, but never expose an unmasked read response.
                    sanitize_provider_record(record);
                    validate_provider_record(record)?;
                }
                JournalResponse::Records(records)
            }
            JournalOperation::Purge => {
                no_active(&transaction, Some(request.task_id))?;
                super::records::purge_provider(&transaction, &self.project, request.task_id)?;
                transaction
                    .execute(
                        "DELETE FROM provider_journal WHERE task_id=?1",
                        [request.task_id],
                    )
                    .map_err(|error| error.to_string())?;
                JournalResponse::Purged
            }
        };
        transaction.commit().map_err(|error| {
            format!("Provider journal commit failed; no persistence claimed: {error}")
        })?;
        Ok(response)
    }
}
pub(super) fn sanitize_provider_record(record: &mut ProviderRecord) {
    for text in [&mut record.origin, &mut record.coverage, &mut record.text] {
        // Normalize before masking: removing controls can join a split credential name
        // (for example, "pass\u{0008}word") into one recognized by the privacy boundary.
        text.retain(|ch| (!ch.is_control() || ch == '\n' || ch == '\t')
            && !matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'));
        if let std::borrow::Cow::Owned(clean) = crate::workspace::privacy::redact(text) {
            *text = clean;
        }
    }
}

pub(super) fn validate_provider_record(record: &ProviderRecord) -> Result<(), String> {
    for identity in [
        &record.id,
        &record.thread_id,
        &record.turn_id,
        &record.item_id,
    ] {
        if identity.is_empty()
            || identity.len() > 1024
            || identity.chars().any(|ch| {
                ch.is_control()
                    || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            })
            || crate::workspace::privacy::redact(identity) != *identity
        {
            return Err("Provider journal identity is invalid or credential-bearing".into());
        }
    }
    if let Some(partner) = &record.partner {
        partner.validate()?;
    }
    if record.origin.trim().is_empty()
        || record.coverage.trim().is_empty()
        || record.origin.len() > 4096
        || record.coverage.len() > 4096
        || record.text.len() > 32768
    {
        return Err(
            "Provider journal requires bounded public observations and explicit origin/coverage"
                .into(),
        );
    }
    Ok(())
}
