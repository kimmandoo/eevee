//! Schema inspection and creation. Pre-release stores have no migrations: a
//! fresh store is created at `CURRENT_SCHEMA` and any other version refuses.
use super::*;

/// Core task, evidence, memory and party tables at the current schema, with
/// partner #1.
fn create_core(connection: &Connection) -> Result<(), String> {
    connection.execute_batch(&format!(
        "CREATE TABLE selections(project TEXT PRIMARY KEY, active_id INTEGER NOT NULL);
         CREATE TABLE tasks (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           project TEXT NOT NULL,
           goal TEXT NOT NULL CHECK(length(CAST(goal AS BLOB))<=4096),
           form INTEGER NOT NULL CHECK(form>=0 AND form<9),
           revision INTEGER NOT NULL CHECK(revision>=0),
           criteria TEXT NOT NULL CHECK(length(CAST(criteria AS BLOB))<=110000),
           next_action TEXT NOT NULL CHECK(length(CAST(next_action AS BLOB))<=4096),
           complete INTEGER NOT NULL CHECK(complete IN (0,1)),
           drafts TEXT NOT NULL CHECK(length(CAST(drafts AS BLOB))<=110000),
           navigation TEXT NOT NULL CHECK(length(CAST(navigation AS BLOB))<=1024),
           updated_at INTEGER NOT NULL CHECK(updated_at>=0),
           archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0,1)),
           archived_at INTEGER NOT NULL DEFAULT 0 CHECK(archived_at>=0),
           redacted INTEGER NOT NULL DEFAULT 0 CHECK(redacted IN (0,1)),
           command TEXT NOT NULL DEFAULT '' CHECK(length(CAST(command AS BLOB))<=4096),
           workflows TEXT NOT NULL DEFAULT '{{}}' CHECK(length(CAST(workflows AS BLOB))<=524288),
           memory_draft TEXT NOT NULL DEFAULT 'null' CHECK(length(CAST(memory_draft AS BLOB))<=65536),
           approved_memories TEXT NOT NULL DEFAULT '[]' CHECK(length(CAST(approved_memories AS BLOB))<=524288),
           workflow_draft TEXT NOT NULL DEFAULT 'null' CHECK(length(CAST(workflow_draft AS BLOB))<=524288),
           lead_partner INTEGER NOT NULL DEFAULT 1 CHECK(lead_partner>=1),
           partner_drafts TEXT NOT NULL DEFAULT '{{}}' CHECK(length(CAST(partner_drafts AS BLOB))<={MAX_PARTNER_DRAFTS_BYTES}));
         CREATE TABLE excerpts(id INTEGER PRIMARY KEY AUTOINCREMENT,task_id INTEGER NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB)) <= 524288));
         CREATE TABLE executions(
           run_id TEXT PRIMARY KEY,task_id INTEGER NOT NULL,request_id TEXT NOT NULL UNIQUE,
           record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=131072),
           output_name TEXT NOT NULL UNIQUE,output_identity TEXT NOT NULL,
           capacity INTEGER NOT NULL CHECK(capacity>=1024 AND capacity<=16777216),
           reserved INTEGER NOT NULL CHECK(reserved IN (0,1)));
         CREATE TABLE provider_journal(id TEXT PRIMARY KEY,task_id INTEGER NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB)) <= 65536));
         CREATE TABLE memories(
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           revision INTEGER NOT NULL CHECK(revision>=1),
           project TEXT NOT NULL CHECK(length(CAST(project AS BLOB))<=4096),
           task_id INTEGER,
           scope TEXT NOT NULL CHECK(scope IN ('task','project','personal')),
           proposal TEXT NOT NULL CHECK(length(CAST(proposal AS BLOB))<=65536),
           CHECK((scope='task' AND task_id IS NOT NULL AND task_id>0) OR (scope!='task' AND task_id IS NULL)));
         CREATE TABLE partners(
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           nickname TEXT CHECK(nickname IS NULL OR length(CAST(nickname AS BLOB))<={MAX_NICKNAME_BYTES}),
           form INTEGER NOT NULL CHECK(form>=0 AND form<9),
           met_at INTEGER NOT NULL CHECK(met_at>=0),
           met_derived INTEGER NOT NULL CHECK(met_derived IN (0,1)),
           created_project TEXT NOT NULL CHECK(length(CAST(created_project AS BLOB))<=4096),
           auto_form INTEGER NOT NULL DEFAULT 1 CHECK(auto_form IN (0,1)));
         INSERT INTO partners(nickname,form,met_at,met_derived,created_project) VALUES(NULL,0,0,1,'');
         CREATE TABLE provider_preferences(
           id INTEGER PRIMARY KEY CHECK(id=1),
           record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=16384));
         CREATE TABLE active_work(
           task_id INTEGER NOT NULL CHECK(task_id>0),
           partner INTEGER NOT NULL CHECK(partner>0),
           request_id TEXT NOT NULL UNIQUE CHECK(length(CAST(request_id AS BLOB)) BETWEEN 1 AND 128),
           owner TEXT NOT NULL CHECK(length(CAST(owner AS BLOB))<=2048),
           writing INTEGER NOT NULL CHECK(writing IN (0,1)),
           kind TEXT NOT NULL CHECK(kind IN ('provider','command')),
           PRIMARY KEY(task_id,partner));"
    ))
    .map_err(|error| format!("Cannot initialize task storage: {error}"))
}

pub(super) fn inspect_evidence(connection: &Connection) -> Result<(), String> {
    inspect_columns(
        connection,
        "excerpts",
        &[
            ("id", "INTEGER", false, 1),
            ("task_id", "INTEGER", true, 0),
            ("record", "TEXT", true, 0),
        ],
    )?;
    inspect_columns(
        connection,
        "executions",
        &[
            ("run_id", "TEXT", false, 1),
            ("task_id", "INTEGER", true, 0),
            ("request_id", "TEXT", true, 0),
            ("record", "TEXT", true, 0),
            ("output_name", "TEXT", true, 0),
            ("output_identity", "TEXT", true, 0),
            ("capacity", "INTEGER", true, 0),
            ("reserved", "INTEGER", true, 0),
        ],
    )?;
    let invalid: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM excerpts e LEFT JOIN tasks t ON t.id=e.task_id WHERE t.id IS NULL)
         OR EXISTS(SELECT 1 FROM executions e LEFT JOIN tasks t ON t.id=e.task_id WHERE t.id IS NULL)
         OR EXISTS(SELECT 1 FROM excerpts GROUP BY task_id HAVING count(*) > 16)
         OR EXISTS(SELECT 1 FROM executions GROUP BY task_id HAVING count(*) > 256)",
        [], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if invalid {
        return Err(
            "Invalid evidence ownership, basket/journal capacity or run reservation count".into(),
        );
    }
    {
        let mut statement = connection
            .prepare("SELECT id,task_id,record FROM excerpts")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    json_column::<crate::workspace::context::Excerpt>(row, 2, 512 * 1024)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (id, task_id, excerpt) = row.map_err(|error| error.to_string())?;
            excerpt.validate()?;
            if excerpt.id != id || excerpt.task_id != task_id || id <= 0 {
                return Err("Invalid stored excerpt identity".into());
            }
            if crate::workspace::privacy::redact(&excerpt.text) != excerpt.text {
                return Err("Stored excerpt violates before-storage masking".into());
            }
        }
    }
    let mut statement = connection
        .prepare(&format!(
            "SELECT {RUN_COLUMNS},run_id,task_id,request_id FROM executions"
        ))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                stored_run_row(row)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (stored, run_id, task_id, request_id) = row.map_err(|error| error.to_string())?;
        run_json(&stored.record)?;
        if stored.record.prepared.run_id != run_id
            || stored.record.prepared.task_id != task_id
            || stored.record.prepared.request_id != request_id
            || stored.output_name != spool_name(&run_id)?
            || !(1024..=MAX_SPOOL_BYTES).contains(&stored.capacity)
            || stored.output_identity.is_empty()
            || stored.output_identity.len() > 128
            || !stored
                .output_identity
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b':')
            || stored.reserved != active_state(&stored.record.state)
            || stored.record.captured_bytes > stored.capacity
        {
            return Err("Invalid stored run correlation, output reference or reservation".into());
        }
        let mut masked = stored.record.clone();
        masked.redact_for_storage();
        if masked != stored.record {
            return Err("Stored run violates before-storage masking".into());
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Schema {
    Empty,
    Current,
}

pub(super) type ColumnSpec = (&'static str, &'static str, bool, i64);

/// Every table at `CURRENT_SCHEMA`, sorted by name.
const TABLES: [&str; 34] = [
    "active_work",
    "context_revisions",
    "diagnosis_transitions",
    "excerpts",
    "executions",
    "failure_observations",
    "failure_resolutions",
    "failure_signatures",
    "form_routes",
    "held_items",
    "instruction_candidates",
    "instruction_sources",
    "local_activity",
    "local_artifacts",
    "local_settings",
    "memories",
    "move_candidates",
    "move_dispatches",
    "move_sources",
    "move_usages",
    "moves",
    "outgoing_receipts",
    "partner_pairings",
    "partners",
    "provider_journal",
    "provider_preferences",
    "provider_usage",
    "provider_usage_events",
    "ribbons",
    "screenshots",
    "selections",
    "tasks",
    "tool_artifacts",
    "tool_catalog",
];

pub(super) fn inspect(connection: &Connection) -> Result<Schema, String> {
    let mut check = connection
        .prepare("PRAGMA quick_check")
        .map_err(|error| format!("Cannot inspect database integrity: {error}"))?;
    let results = check
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    for result in results {
        let result = result.map_err(|error| error.to_string())?;
        if result != "ok" {
            return Err(format!("Database integrity check failed: {result}"));
        }
    }
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if !(0..=CURRENT_SCHEMA).contains(&version) {
        return Err(format!(
            "Unsupported database schema version {version}; this ev supports only version {CURRENT_SCHEMA}"
        ));
    }
    let mut statement = connection
        .prepare(
            "SELECT type, name FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' ORDER BY name",
        )
        .map_err(|error| error.to_string())?;
    let objects = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    if version == 0 && objects.is_empty() {
        return Ok(Schema::Empty);
    }
    if version != CURRENT_SCHEMA {
        // Pre-release stores are not migrated. Refuse before any write so the
        // old bytes stay intact for `--export-data`.
        return Err(format!(
            "Pre-release database schema version {version} is no longer supported; this ev opens only version {CURRENT_SCHEMA}. Nothing was changed: export tasks with --export-data PATH, then move this data directory aside to start fresh"
        ));
    }
    if objects.len() != TABLES.len()
        || objects
            .iter()
            .zip(TABLES)
            .any(|((kind, name), expected)| kind != "table" || name != expected)
    {
        return Err("Unrecognized or nonempty foreign database schema".into());
    }
    inspect_columns(
        connection,
        "selections",
        &[
            ("project", "TEXT", false, 1),
            ("active_id", "INTEGER", true, 0),
        ],
    )?;
    let expected = [
        ("id", "INTEGER", false, 1),
        ("project", "TEXT", true, 0),
        ("goal", "TEXT", true, 0),
        ("form", "INTEGER", true, 0),
        ("revision", "INTEGER", true, 0),
        ("criteria", "TEXT", true, 0),
        ("next_action", "TEXT", true, 0),
        ("complete", "INTEGER", true, 0),
        ("drafts", "TEXT", true, 0),
        ("navigation", "TEXT", true, 0),
        ("updated_at", "INTEGER", true, 0),
        ("archived", "INTEGER", true, 0),
        ("archived_at", "INTEGER", true, 0),
        ("redacted", "INTEGER", true, 0),
        ("command", "TEXT", true, 0),
        ("workflows", "TEXT", true, 0),
        ("memory_draft", "TEXT", true, 0),
        ("approved_memories", "TEXT", true, 0),
        ("workflow_draft", "TEXT", true, 0),
        ("lead_partner", "INTEGER", true, 0),
        ("partner_drafts", "TEXT", true, 0),
    ];
    inspect_columns(connection, "tasks", &expected)?;
    let columns = format!("{TASK_COLUMNS},{TASK_AUTO_FORM}");
    let mut rows = connection
        .prepare(&format!("SELECT {columns} FROM tasks"))
        .map_err(|error| error.to_string())?;
    for row in rows
        .query_map([], snapshot_row)
        .map_err(|error| error.to_string())?
    {
        let task = row.map_err(|error| format!("Invalid stored task: {error}"))?;
        validate(&task)?;
        if task.id == 0 {
            return Err("Stored task ID must be positive".into());
        }
    }
    let invalid: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM selections s LEFT JOIN tasks t
         ON t.id=s.active_id AND t.project=s.project WHERE t.id IS NULL)
         OR EXISTS(SELECT 1 FROM tasks GROUP BY project HAVING count(*) > 256)",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let archived_selected: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM selections s JOIN tasks t ON t.id=s.active_id WHERE t.archived != 0)",
        [], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if archived_selected {
        return Err("Archived task cannot be active".into());
    }
    if invalid {
        return Err("Invalid project task selection or project capacity".into());
    }
    inspect_evidence(connection)?;
    inspect_columns(
        connection,
        "provider_journal",
        &[
            ("id", "TEXT", false, 1),
            ("task_id", "INTEGER", true, 0),
            ("record", "TEXT", true, 0),
        ],
    )?;
    let invalid: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM provider_journal p LEFT JOIN tasks t ON t.id=p.task_id WHERE t.id IS NULL) OR EXISTS(SELECT 1 FROM provider_journal GROUP BY task_id HAVING count(*) > 256)", [], |row| row.get(0)).map_err(|error| error.to_string())?;
    if invalid {
        return Err("Provider journal contains orphaned or excess task records".into());
    }
    let mut statement = connection
        .prepare("SELECT id,record FROM provider_journal")
        .map_err(|error| error.to_string())?;
    for row in statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                json_column::<ProviderRecord>(row, 1, 65536)?,
            ))
        })
        .map_err(|error| error.to_string())?
    {
        let (id, record) = row.map_err(|error| error.to_string())?;
        validate_provider_record(&record)?;
        if id != record.id {
            return Err("Provider journal row identity differs from record".into());
        }
    }
    inspect_memories(connection)?;
    inspect_partners(connection, true)?;
    super::preferences::inspect_preferences(connection)?;
    super::work::inspect_work(connection)?;
    super::moves::inspect(connection)?;
    super::records::inspect(connection)?;
    super::held::inspect(connection)?;
    super::ribbons::inspect(connection)?;
    super::receipts::inspect(connection)?;
    super::artifacts::inspect(connection)?;
    super::context_cache::inspect(connection)?;
    super::tool_catalog::inspect(connection)?;
    super::screenshots::inspect(connection)?;
    Ok(Schema::Current)
}

pub(super) fn inspect_columns(
    connection: &Connection,
    table: &str,
    expected: &[ColumnSpec],
) -> Result<(), String> {
    let mut columns = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| error.to_string())?;
    let columns = columns
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    if columns.len() != expected.len()
        || columns.iter().zip(expected).any(|(actual, expected)| {
            actual.0 != expected.0
                || !actual.1.eq_ignore_ascii_case(expected.1)
                || actual.2 != expected.2
                || actual.3 != expected.3
        })
    {
        return Err(format!("Unrecognized {table} table layout"));
    }
    Ok(())
}

pub(super) fn initialize(connection: &mut Connection) -> Result<(), String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("Cannot begin database initialization: {error}"))?;
    if inspect(&transaction)? == Schema::Empty {
        create(&transaction)?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Cannot commit database initialization: {error}"))
}

/// Creates every table of an empty store at `CURRENT_SCHEMA`.
fn create(connection: &Connection) -> Result<(), String> {
    create_core(connection)?;
    super::moves::create(connection)?;
    super::records::create(connection)?;
    super::held::create(connection)?;
    super::ribbons::create(connection)?;
    super::receipts::create(connection)?;
    super::artifacts::create(connection)?;
    super::context_cache::create(connection)?;
    super::tool_catalog::create(connection)?;
    super::screenshots::create(connection)?;
    connection
        .execute_batch(&format!("PRAGMA user_version={CURRENT_SCHEMA};"))
        .map_err(|error| format!("Cannot initialize storage version: {error}"))
}
