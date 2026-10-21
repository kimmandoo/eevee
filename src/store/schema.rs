//! Schema inspection, creation and migration.
use super::*;

pub(super) fn migrate_workflows(connection: &Connection) -> Result<(), String> {
    connection.execute_batch(
        "ALTER TABLE tasks ADD COLUMN workflows TEXT NOT NULL DEFAULT '{}' CHECK(length(CAST(workflows AS BLOB))<=524288);
         ALTER TABLE tasks ADD COLUMN memory_draft TEXT NOT NULL DEFAULT 'null' CHECK(length(CAST(memory_draft AS BLOB))<=65536);
         ALTER TABLE tasks ADD COLUMN approved_memories TEXT NOT NULL DEFAULT '[]' CHECK(length(CAST(approved_memories AS BLOB))<=524288);
         ALTER TABLE tasks ADD COLUMN workflow_draft TEXT NOT NULL DEFAULT 'null' CHECK(length(CAST(workflow_draft AS BLOB))<=524288);
         CREATE TABLE memories(
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           revision INTEGER NOT NULL CHECK(revision>=1),
           project TEXT NOT NULL CHECK(length(CAST(project AS BLOB))<=4096),
           task_id INTEGER,
           scope TEXT NOT NULL CHECK(scope IN ('task','project','personal')),
           proposal TEXT NOT NULL CHECK(length(CAST(proposal AS BLOB))<=65536),
           CHECK((scope='task' AND task_id IS NOT NULL AND task_id>0) OR (scope!='task' AND task_id IS NULL)));
         PRAGMA user_version=6;"
    ).map_err(|error| format!("Cannot migrate workflow and memory storage: {error}"))
}

/// Schema 7: the party. Creates `partners` with partner #1 derived from task
/// history, and reserves `tasks.lead_partner` plus the per-partner draft
/// column used by per-partner editing.
pub(super) fn migrate_partners(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(&format!(
            "CREATE TABLE partners(
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           nickname TEXT CHECK(nickname IS NULL OR length(CAST(nickname AS BLOB))<={MAX_NICKNAME_BYTES}),
           form INTEGER NOT NULL CHECK(form>=0 AND form<9),
           met_at INTEGER NOT NULL CHECK(met_at>=0),
           met_derived INTEGER NOT NULL CHECK(met_derived IN (0,1)),
           created_project TEXT NOT NULL CHECK(length(CAST(created_project AS BLOB))<=4096));
         INSERT INTO partners(nickname,form,met_at,met_derived,created_project)
           SELECT NULL,
                  coalesce((SELECT form FROM tasks ORDER BY updated_at,id LIMIT 1),0),
                  coalesce((SELECT MIN(updated_at) FROM tasks),0),
                  1,
                  coalesce((SELECT project FROM tasks ORDER BY updated_at,id LIMIT 1),'');
         ALTER TABLE tasks ADD COLUMN lead_partner INTEGER NOT NULL DEFAULT 1 CHECK(lead_partner>=1);
         ALTER TABLE tasks ADD COLUMN partner_drafts TEXT NOT NULL DEFAULT '{{}}' CHECK(length(CAST(partner_drafts AS BLOB))<=65536);
         PRAGMA user_version=7;"
        ))
        .map_err(|error| format!("Cannot migrate partner storage: {error}"))
}

/// Schema 8 preserves the full six-partner draft capacity, including JSON escaping.
pub(super) fn migrate_partner_draft_capacity(connection: &Connection) -> Result<(), String> {
    connection.execute_batch(&format!(
        "ALTER TABLE tasks RENAME TO old_tasks;
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
         INSERT INTO tasks({TASK_COLUMNS}) SELECT {TASK_COLUMNS} FROM old_tasks;
         UPDATE sqlite_sequence SET seq=max(seq,coalesce(
           (SELECT seq FROM sqlite_sequence WHERE name='old_tasks'),seq)) WHERE name='tasks';
         INSERT INTO sqlite_sequence(name,seq)
           SELECT 'tasks',seq FROM sqlite_sequence WHERE name='old_tasks'
           AND NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='tasks');
         DROP TABLE old_tasks;
         PRAGMA user_version=9;"
    )).map_err(|error| format!("Cannot migrate partner draft capacity: {error}"))
}

pub(super) fn create_evidence_schema(connection: &Connection) -> Result<(), String> {
    connection.execute_batch(
        "CREATE TABLE excerpts(id INTEGER PRIMARY KEY AUTOINCREMENT,task_id INTEGER NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB)) <= 524288));
         CREATE TABLE executions(run_id TEXT PRIMARY KEY,task_id INTEGER NOT NULL,request_id TEXT NOT NULL UNIQUE,
           record TEXT NOT NULL CHECK(length(CAST(record AS BLOB)) <= 131072),
           output_name TEXT NOT NULL UNIQUE,output_identity TEXT NOT NULL,capacity INTEGER NOT NULL CHECK(capacity >= 1024 AND capacity <= 8388608),
           reserved INTEGER NOT NULL CHECK(reserved IN (0,1)));
         CREATE TABLE provider_journal(id TEXT PRIMARY KEY,task_id INTEGER NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB)) <= 65536));
         PRAGMA user_version=5;"
    ).map_err(|error| format!("Cannot initialize source/journal storage: {error}"))
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

#[derive(Clone, Copy)]
pub(super) enum Schema {
    Empty,
    Legacy,
    Projects,
    Tasks,
    Lifecycle,
    Evidence,
    ProviderJournal,
    Workflows,
    Partners,
    Current,
}

pub(super) type ColumnSpec = (&'static str, &'static str, bool, i64);

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
    if !(0..=15).contains(&version) {
        return Err(format!(
            "Unsupported database schema version {version}; this ev supports versions 0 through 15"
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
    let schema = match (version, objects.as_slice()) {
        (0, []) => Schema::Empty,
        (0, [(kind, name)]) if kind == "table" && name == "goals" => Schema::Legacy,
        (1, [(kind, name)]) if kind == "table" && name == "projects" => Schema::Projects,
        (2 | 3, [(kind_a, name_a), (kind_b, name_b)])
            if kind_a == "table"
                && name_a == "selections"
                && kind_b == "table"
                && name_b == "tasks" =>
        {
            if version == 2 {
                Schema::Tasks
            } else {
                Schema::Lifecycle
            }
        }
        (4, objects)
            if objects.len() == 4
                && objects
                    .iter()
                    .zip(["excerpts", "executions", "selections", "tasks"])
                    .all(|((kind, name), expected)| kind == "table" && name == expected) =>
        {
            Schema::Evidence
        }
        (5, objects)
            if objects.len() == 5
                && objects
                    .iter()
                    .zip([
                        "excerpts",
                        "executions",
                        "provider_journal",
                        "selections",
                        "tasks",
                    ])
                    .all(|((kind, name), expected)| kind == "table" && name == expected) =>
        {
            Schema::ProviderJournal
        }
        (6, objects)
            if objects.len() == 6
                && objects
                    .iter()
                    .zip([
                        "excerpts",
                        "executions",
                        "memories",
                        "provider_journal",
                        "selections",
                        "tasks",
                    ])
                    .all(|((kind, name), expected)| kind == "table" && name == expected) =>
        {
            Schema::Workflows
        }
        (7..=9, objects)
            if objects.len() == 7
                && objects
                    .iter()
                    .zip([
                        "excerpts",
                        "executions",
                        "memories",
                        "partners",
                        "provider_journal",
                        "selections",
                        "tasks",
                    ])
                    .all(|((kind, name), expected)| kind == "table" && name == expected) =>
        {
            if version == 7 {
                Schema::Partners
            } else {
                Schema::Current
            }
        }
        (10..=12, objects)
            if objects.len() == 8
                && objects
                    .iter()
                    .zip([
                        "excerpts",
                        "executions",
                        "memories",
                        "partners",
                        "provider_journal",
                        "provider_preferences",
                        "selections",
                        "tasks",
                    ])
                    .all(|((kind, name), expected)| kind == "table" && name == expected) =>
        {
            Schema::Current
        }
        (13, objects)
            if objects.len() == 9
                && objects
                    .iter()
                    .zip([
                        "active_work",
                        "excerpts",
                        "executions",
                        "memories",
                        "partners",
                        "provider_journal",
                        "provider_preferences",
                        "selections",
                        "tasks",
                    ])
                    .all(|((kind, name), expected)| kind == "table" && name == expected) =>
        {
            Schema::Current
        }
        (14, objects)
            if objects.len() == 14
                && objects
                    .iter()
                    .zip([
                        "active_work",
                        "excerpts",
                        "executions",
                        "memories",
                        "move_candidates",
                        "move_dispatches",
                        "move_sources",
                        "move_usages",
                        "moves",
                        "partners",
                        "provider_journal",
                        "provider_preferences",
                        "selections",
                        "tasks",
                    ])
                    .all(|((kind, name), expected)| kind == "table" && name == expected) =>
        {
            Schema::Current
        }
        (15, objects)
            if objects.len() == 27
                && objects
                    .iter()
                    .zip([
                        "active_work",
                        "diagnosis_transitions",
                        "excerpts",
                        "executions",
                        "failure_observations",
                        "failure_resolutions",
                        "failure_signatures",
                        "form_routes",
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
                        "partner_pairings",
                        "partners",
                        "provider_journal",
                        "provider_preferences",
                        "provider_usage",
                        "provider_usage_events",
                        "selections",
                        "tasks",
                    ])
                    .all(|((kind, name), expected)| kind == "table" && name == expected) =>
        {
            Schema::Current
        }
        _ => return Err("Unrecognized or nonempty foreign database schema".into()),
    };
    let (table, expected, query): (&str, &[ColumnSpec], &str) = match schema {
        Schema::Empty => return Ok(schema),
        Schema::Legacy => (
            "goals",
            &[
                ("project", "TEXT", false, 1),
                ("goal", "TEXT", true, 0),
                ("form", "INTEGER", true, 0),
            ],
            "SELECT project, goal, form, 0 FROM goals",
        ),
        Schema::Projects => (
            "projects",
            &[
                ("project", "TEXT", false, 1),
                ("goal", "TEXT", true, 0),
                ("form", "INTEGER", true, 0),
                ("revision", "INTEGER", true, 0),
            ],
            "SELECT project, goal, form, revision FROM projects",
        ),
        Schema::Tasks
        | Schema::Lifecycle
        | Schema::Evidence
        | Schema::ProviderJournal
        | Schema::Workflows
        | Schema::Partners
        | Schema::Current => {
            inspect_columns(
                connection,
                "selections",
                &[
                    ("project", "TEXT", false, 1),
                    ("active_id", "INTEGER", true, 0),
                ],
            )?;
            let mut expected = vec![
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
            ];
            if version >= 3 {
                expected.extend([
                    ("archived", "INTEGER", true, 0),
                    ("archived_at", "INTEGER", true, 0),
                    ("redacted", "INTEGER", true, 0),
                ]);
            }
            if version >= 4 {
                expected.push(("command", "TEXT", true, 0));
            }
            if version >= 6 {
                expected.extend([
                    ("workflows", "TEXT", true, 0),
                    ("memory_draft", "TEXT", true, 0),
                    ("approved_memories", "TEXT", true, 0),
                    ("workflow_draft", "TEXT", true, 0),
                ]);
            }
            if version >= 7 {
                expected.extend([
                    ("lead_partner", "INTEGER", true, 0),
                    ("partner_drafts", "TEXT", true, 0),
                ]);
            }
            inspect_columns(connection, "tasks", &expected)?;
            let columns = if version >= 7 {
                TASK_COLUMNS.to_string()
            } else if version >= 6 {
                format!("{TASK_COLUMNS_V6},1")
            } else if version >= 4 {
                format!("{TASK_COLUMNS_V5},{EMPTY_WORKFLOW_COLUMNS},1")
            } else if version == 3 {
                format!("{TASK_COLUMNS_V3},'',{EMPTY_WORKFLOW_COLUMNS},1")
            } else {
                format!("{TASK_COLUMNS_V2},0,0,0,'',{EMPTY_WORKFLOW_COLUMNS},1")
            };
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
            if version >= 3 {
                let archived_selected: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM selections s JOIN tasks t ON t.id=s.active_id WHERE t.archived != 0)",
                    [], |row| row.get(0),
                ).map_err(|error| error.to_string())?;
                if archived_selected {
                    return Err("Archived task cannot be active".into());
                }
            }
            if invalid {
                return Err("Invalid project task selection or project capacity".into());
            }
            if version >= 4 {
                inspect_evidence(connection)?;
            }
            if version >= 5 {
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
            }
            if version >= 6 {
                inspect_memories(connection)?;
            }
            if version >= 7 {
                inspect_partners(connection)?;
            }
            if version >= 10 {
                super::preferences::inspect_preferences(connection)?;
            }
            if version >= 13 {
                super::work::inspect_work(connection)?;
            }
            if version >= 14 {
                super::moves::inspect(connection)?;
            }
            if version >= 15 {
                super::records::inspect(connection)?;
            }
            return Ok(schema);
        }
    };
    inspect_columns(connection, table, expected)?;
    let mut rows = connection
        .prepare(query)
        .map_err(|error| error.to_string())?;
    for row in rows
        .query_map([], legacy_row)
        .map_err(|error| error.to_string())?
    {
        validate(&row.map_err(|error| format!("Invalid stored goal: {error}"))?)?;
    }
    Ok(schema)
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
    migrate(&transaction, inspect(&transaction)?)?;
    transaction
        .commit()
        .map_err(|error| format!("Cannot commit database initialization: {error}"))
}

pub(super) fn migrate(connection: &Connection, schema: Schema) -> Result<(), String> {
    migrate_tasks(connection, schema)?;
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version < 10 {
        connection
            .execute_batch(
                "CREATE TABLE provider_preferences(
             id INTEGER PRIMARY KEY CHECK(id=1),
             record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=16384));
             PRAGMA user_version=10;",
            )
            .map_err(|error| format!("Cannot migrate provider preferences: {error}"))?;
    }
    if version < 11 {
        // Schema 11 adds dismissal metadata to navigation's JSON contract. Only
        // bump the version: legacy JSON and all task/evidence bytes stay intact,
        // while Schema 10 binaries refuse before they can drop the new field.
        connection
            .execute_batch("PRAGMA user_version=11;")
            .map_err(|error| format!("Cannot migrate suggestion dismissals: {error}"))?;
    }
    if version < 12 {
        // Wave-three provenance extends existing bounded JSON contracts only.
        // Fence older binaries without rewriting task/evidence/schema rows.
        connection
            .execute_batch("PRAGMA user_version=12;")
            .map_err(|error| format!("Cannot migrate workflow and evidence provenance: {error}"))?;
    }
    if version < 13 {
        // Ownership is additive: preserve every old task and evidence JSON byte.
        connection.execute_batch(
            "CREATE TABLE active_work(
                task_id INTEGER NOT NULL CHECK(task_id>0),
                partner INTEGER NOT NULL CHECK(partner>0),
                request_id TEXT NOT NULL UNIQUE CHECK(length(CAST(request_id AS BLOB)) BETWEEN 1 AND 128),
                owner TEXT NOT NULL CHECK(length(CAST(owner AS BLOB))<=2048),
                writing INTEGER NOT NULL CHECK(writing IN (0,1)),
                kind TEXT NOT NULL CHECK(kind IN ('provider','command')),
                PRIMARY KEY(task_id,partner));
             PRAGMA user_version=13;"
        ).map_err(|error| format!("Cannot migrate durable work ownership: {error}"))?;
    }
    if version < 14 {
        super::moves::migrate(connection)?;
    }
    if version < 15 {
        super::records::migrate(connection)?;
    }
    Ok(())
}

fn migrate_tasks(connection: &Connection, schema: Schema) -> Result<(), String> {
    if matches!(schema, Schema::Current) {
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if version >= 9 {
            return Ok(());
        }
        // Schema 9 adds the Request draft variant to the existing JSON contract.
        // Bump even unchanged layouts so older binaries refuse these new values.
        return connection
            .execute_batch("PRAGMA user_version=9;")
            .map_err(|error| format!("Cannot migrate request drafts: {error}"));
    }
    if matches!(schema, Schema::Partners) {
        return migrate_partner_draft_capacity(connection);
    }
    if matches!(schema, Schema::Workflows) {
        migrate_partners(connection)?;
        return migrate_partner_draft_capacity(connection);
    }
    if matches!(schema, Schema::Evidence) {
        connection.execute_batch("CREATE TABLE provider_journal(id TEXT PRIMARY KEY,task_id INTEGER NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB)) <= 65536)); PRAGMA user_version=5;").map_err(|error| format!("Cannot migrate provider journal: {error}"))?;
        migrate_workflows(connection)?;
        migrate_partners(connection)?;
        return migrate_partner_draft_capacity(connection);
    }
    if matches!(schema, Schema::ProviderJournal) {
        migrate_workflows(connection)?;
        migrate_partners(connection)?;
        return migrate_partner_draft_capacity(connection);
    }
    if matches!(schema, Schema::Tasks | Schema::Lifecycle) {
        connection
            .execute_batch("ALTER TABLE tasks RENAME TO old_tasks;")
            .map_err(|error| format!("Cannot stage task migration: {error}"))?;
    }
    connection
        .execute_batch(
            "CREATE TABLE tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project TEXT NOT NULL,
            goal TEXT NOT NULL CHECK(length(CAST(goal AS BLOB)) <= 4096),
            form INTEGER NOT NULL CHECK(form >= 0 AND form < 9),
            revision INTEGER NOT NULL CHECK(revision >= 0),
            criteria TEXT NOT NULL CHECK(length(CAST(criteria AS BLOB)) <= 110000),
            next_action TEXT NOT NULL CHECK(length(CAST(next_action AS BLOB)) <= 4096),
            complete INTEGER NOT NULL CHECK(complete IN (0,1)),
            drafts TEXT NOT NULL CHECK(length(CAST(drafts AS BLOB)) <= 110000),
            navigation TEXT NOT NULL CHECK(length(CAST(navigation AS BLOB)) <= 1024),
            updated_at INTEGER NOT NULL CHECK(updated_at >= 0),
            archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0,1)),
            archived_at INTEGER NOT NULL DEFAULT 0 CHECK(archived_at >= 0),
            redacted INTEGER NOT NULL DEFAULT 0 CHECK(redacted IN (0,1)),
            command TEXT NOT NULL DEFAULT '' CHECK(length(CAST(command AS BLOB)) <= 4096)
        );",
        )
        .map_err(|error| format!("Cannot initialize task storage: {error}"))?;
    if !matches!(schema, Schema::Tasks | Schema::Lifecycle) {
        connection
            .execute_batch(
                "CREATE TABLE selections(project TEXT PRIMARY KEY, active_id INTEGER NOT NULL);",
            )
            .map_err(|error| error.to_string())?;
    }
    let source = match schema {
        Schema::Legacy => Some((
            "goals",
            "SELECT project,goal,form,0 FROM goals ORDER BY project".to_string(),
        )),
        Schema::Projects => Some((
            "projects",
            "SELECT project,goal,form,revision FROM projects ORDER BY project".to_string(),
        )),
        Schema::Tasks => Some((
            "old_tasks",
            format!(
                "SELECT {TASK_COLUMNS_V2},0,0,0,'',{EMPTY_WORKFLOW_COLUMNS},1 FROM old_tasks ORDER BY id"
            ),
        )),
        Schema::Lifecycle => Some((
            "old_tasks",
            format!(
                "SELECT {TASK_COLUMNS_V3},'',{EMPTY_WORKFLOW_COLUMNS},1 FROM old_tasks ORDER BY id"
            ),
        )),
        _ => None,
    };
    if let Some((table, query)) = source {
        {
            let mut statement = connection
                .prepare(&query)
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| {
                    if matches!(schema, Schema::Tasks | Schema::Lifecycle) {
                        snapshot_row(row)
                    } else {
                        legacy_row(row)
                    }
                })
                .map_err(|error| error.to_string())?;
            for row in rows {
                let mut task = row.map_err(|error| error.to_string())?;
                redact_snapshot(&mut task);
                validate(&task)?;
                connection.execute(
                    "INSERT INTO tasks(id,project,goal,form,revision,criteria,next_action,complete,drafts,navigation,updated_at,redacted,archived,archived_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                    params![if task.id == 0 { None } else { Some(task.id) }, task.project, task.goal, task.form as i64, task.revision as i64,
                        serde_json::to_string(&task.criteria).map_err(|error| error.to_string())?, task.next_action, task.complete,
                        serde_json::to_string(&task.partner_drafts.get(&task.lead_partner).map(|state| state.drafts.as_slice()).unwrap_or_default()).map_err(|error| error.to_string())?,
                        serde_json::to_string(&task.navigation).map_err(|error| error.to_string())?, task.updated_at, task.redacted, task.archived, task.archived_at],
                ).map_err(|error| format!("Cannot migrate task: {error}"))?;
            }
        }
        if !matches!(schema, Schema::Tasks | Schema::Lifecycle) {
            connection
                .execute_batch("INSERT INTO selections SELECT project,id FROM tasks;")
                .map_err(|error| error.to_string())?;
        }
        if matches!(schema, Schema::Tasks | Schema::Lifecycle) {
            // Rebuilding a task table must preserve deleted identities too. Otherwise
            // AUTOINCREMENT restarts at the highest surviving row and reuses old task IDs.
            connection
                .execute_batch(
                    "UPDATE sqlite_sequence SET seq=max(seq,coalesce(
                    (SELECT seq FROM sqlite_sequence WHERE name='old_tasks'),seq))
                 WHERE name='tasks';
                 INSERT INTO sqlite_sequence(name,seq)
                 SELECT 'tasks',seq FROM sqlite_sequence
                 WHERE name='old_tasks'
                 AND NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='tasks');",
                )
                .map_err(|error| format!("Cannot preserve task identity sequence: {error}"))?;
        }
        connection
            .execute_batch(&format!("DROP TABLE {table};"))
            .map_err(|error| error.to_string())?;
    }
    create_evidence_schema(connection)?;
    migrate_workflows(connection)?;
    migrate_partners(connection)?;
    migrate_partner_draft_capacity(connection)
}
