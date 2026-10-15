//! Party partner rows: loading, validation and the lead partner's form write.
use super::*;

const PARTNER_COLUMNS: &[ColumnSpec] = &[
    ("id", "INTEGER", false, 1),
    ("nickname", "TEXT", false, 0),
    ("form", "INTEGER", true, 0),
    ("met_at", "INTEGER", true, 0),
    ("met_derived", "INTEGER", true, 0),
    ("created_project", "TEXT", true, 0),
];

pub(super) fn validate_partner(partner: &Partner) -> Result<(), String> {
    // Ids are AUTOINCREMENT and never reused, so they grow past the six-seat
    // capacity after releases; the roster cap applies to the row count.
    if partner.id < 1 {
        return Err("A partner id must be a positive AUTOINCREMENT value".into());
    }
    if partner.form >= 9 {
        return Err("A stored partner form is outside 0..=8".into());
    }
    if partner.met_at < 0 {
        return Err("A partner met timestamp must not be negative".into());
    }
    if partner
        .nickname
        .as_deref()
        .is_some_and(|n| n.len() > MAX_NICKNAME_BYTES)
    {
        return Err("A partner nickname exceeds the 512-byte storage limit".into());
    }
    if partner.created_project.len() > 4096 {
        return Err("A partner origin project exceeds the 4096-byte limit".into());
    }
    Ok(())
}

impl State {
    /// Welcome a new partner: starts in Eevee form with today's met date on the
    /// current project. The six-seat party capacity is enforced inside the
    /// transaction, and AUTOINCREMENT never reuses a released partner's id.
    pub(super) fn add_partner(&mut self) -> Result<Vec<Partner>, String> {
        let met_at = timestamp()?;
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot begin partner addition: {error}"))?;
        let seated: i64 = transaction
            .query_row("SELECT count(*) FROM partners", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if seated >= MAX_PARTNERS as i64 {
            return Err(
                "The party is full: a partner can hold at most 6 members; release one first".into(),
            );
        }
        transaction
            .execute(
                "INSERT INTO partners(nickname,form,met_at,met_derived,created_project)
                 VALUES (NULL,0,?1,0,?2)",
                params![met_at, self.project],
            )
            .map_err(|error| format!("Cannot add the partner: {error}"))?;
        enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
        let roster = partners(&transaction)?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit the new partner: {error}"))?;
        Ok(roster)
    }
}

fn partner_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Partner> {
    Ok(Partner {
        id: row.get(0)?,
        nickname: row.get(1)?,
        form: row.get(2)?,
        met_at: row.get(3)?,
        met_derived: row.get(4)?,
        created_project: row.get(5)?,
    })
}

pub(super) fn partners(connection: &Connection) -> Result<Vec<Partner>, String> {
    let mut statement = connection
        .prepare("SELECT id,nickname,form,met_at,met_derived,created_project FROM partners ORDER BY id LIMIT 7")
        .map_err(|error| error.to_string())?;
    let partners = statement
        .query_map([], partner_row)
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    if partners.len() > MAX_PARTNERS {
        return Err("Party exceeds its bounded capacity of six partners".into());
    }
    for partner in &partners {
        validate_partner(partner)?;
    }
    Ok(partners)
}

/// Persist the lead partner's form when its task is saved. The partner row is
/// the durable copy of the form a task resumes into.
pub(super) fn set_partner_form(
    connection: &Connection,
    partner: i64,
    form: usize,
) -> Result<(), String> {
    let updated = connection
        .execute(
            "UPDATE partners SET form=?1 WHERE id=?2",
            params![form as i64, partner],
        )
        .map_err(|error| format!("Cannot persist the partner form: {error}"))?;
    if updated != 1 {
        return Err("The task's lead partner does not exist".into());
    }
    Ok(())
}

/// Partner draft JSON uses the column reserved in Schema 7 and enlarged in
/// Schema 8. Typed draft bounds are validated with the task; inspection also
/// refuses a non-object container before any writable open.
fn valid_partner_drafts(text: &str) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| format!("Invalid partner drafts JSON: {error}"))?;
    if !value.is_object() {
        return Err("Partner drafts must be a JSON object keyed by partner id".into());
    }
    validate_draft_shape(&value, 0)
}
pub(super) fn inspect_partners(connection: &Connection) -> Result<(), String> {
    inspect_columns(connection, "partners", PARTNER_COLUMNS)?;
    let rows = {
        let mut statement = connection
            .prepare("SELECT id,nickname,form,met_at,met_derived,created_project FROM partners ORDER BY id")
            .map_err(|error| error.to_string())?;
        statement
            .query_map([], partner_row)
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?
    };
    if rows.is_empty() {
        return Err("A party store must hold at least one partner".into());
    }
    for (index, partner) in rows.iter().enumerate() {
        validate_partner(partner)?;
        if rows[..index].iter().any(|other| other.id == partner.id) {
            return Err("Duplicate partner identity".into());
        }
    }
    let invalid: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks t LEFT JOIN partners p ON p.id=t.lead_partner WHERE p.id IS NULL)",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if invalid {
        return Err("A task lead partner refers to a missing partner".into());
    }
    let mut statement = connection
        .prepare("SELECT partner_drafts FROM tasks")
        .map_err(|error| error.to_string())?;
    let drafts = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    for text in drafts {
        valid_partner_drafts(&text.map_err(|error| error.to_string())?)?;
    }
    Ok(())
}
