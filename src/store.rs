use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

const DATABASE: &str = "workbench.sqlite3";
const LOCK: &str = "workbench.lock";
const BUSY_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_GOAL_BYTES: usize = 4096;
const MAX_TASKS: usize = 256;
const TASK_COLUMNS: &str = "project, goal, form, revision, id, criteria, next_action, complete, drafts, navigation, updated_at";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub project: String,
    pub goal: String,
    pub form: usize,
    pub revision: u64,
    pub id: i64,
    pub criteria: Vec<Criterion>,
    pub next_action: String,
    pub complete: bool,
    pub drafts: Vec<Draft>,
    pub navigation: Navigation,
    pub updated_at: i64,
}

impl Snapshot {
    pub fn empty(project: String) -> Self {
        Self {
            project,
            goal: String::new(),
            form: 0,
            revision: 0,
            id: 0,
            criteria: Vec::new(),
            next_action: String::new(),
            complete: false,
            drafts: Vec::new(),
            navigation: Navigation::default(),
            updated_at: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Criterion {
    pub text: String,
    pub met: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditorField {
    #[default]
    Goal,
    Criteria,
    NextAction,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub field: EditorField,
    pub text: String,
    pub cursor: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Navigation {
    pub focus: u8,
    pub linear: bool,
    pub form_selection: usize,
    pub palette_selection: usize,
    pub criterion_selection: usize,
    pub scroll: [u16; 5],
    pub overlay: Option<u8>,
    pub editing: bool,
    pub editor_field: EditorField,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            focus: 1,
            linear: false,
            form_selection: 0,
            palette_selection: 0,
            criterion_selection: 0,
            scroll: [0; 5],
            overlay: None,
            editing: false,
            editor_field: EditorField::Goal,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Project {
    pub tasks: Vec<Snapshot>,
    pub active_id: Option<i64>,
}

#[derive(Debug)]
pub enum Event {
    Loaded(Project),
    Saved(Snapshot),
    Selected(Snapshot),
    Failed {
        revision: Option<u64>,
        message: String,
    },
}

enum Command {
    Save(Snapshot),
    Select(i64),
    Retry,
}

pub struct Worker {
    commands: Option<SyncSender<Command>>,
    events: Option<Receiver<Event>>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    /// Starts storage off the UI thread. Initialization succeeds or fails via an event.
    pub fn start(data_dir: PathBuf, project: String) -> io::Result<Self> {
        let (commands, incoming) = mpsc::sync_channel(2);
        let (outgoing, events) = mpsc::sync_channel(8);
        let thread = thread::Builder::new()
            .name("ev-store".into())
            .spawn(move || run(data_dir, project, incoming, outgoing))?;
        Ok(Self {
            commands: Some(commands),
            events: Some(events),
            thread: Some(thread),
        })
    }

    /// Queues a pending snapshot, retaining it on backpressure or disconnect.
    /// Acceptance is not a save acknowledgement; an empty pending slot returns false.
    pub fn try_save(&self, pending: &mut Option<Snapshot>) -> bool {
        let Some(commands) = &self.commands else {
            return false;
        };
        let Some(snapshot) = pending.take() else {
            return false;
        };
        match commands.try_send(Command::Save(snapshot)) {
            Ok(()) => true,
            Err(TrySendError::Full(Command::Save(snapshot)))
            | Err(TrySendError::Disconnected(Command::Save(snapshot))) => {
                *pending = Some(snapshot);
                false
            }
            Err(_) => unreachable!("try_send returns the submitted command"),
        }
    }

    pub fn try_select(&self, id: i64) -> bool {
        self.commands
            .as_ref()
            .is_some_and(|commands| commands.try_send(Command::Select(id)).is_ok())
    }

    pub fn try_event(&self) -> Option<Event> {
        self.events.as_ref()?.try_recv().ok()
    }

    /// Queues a reopen attempt. It never removes or steals another writer's lock.
    pub fn retry(&self) -> bool {
        self.commands
            .as_ref()
            .is_some_and(|commands| commands.try_send(Command::Retry).is_ok())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Unblock a full event queue first. The worker still drains accepted commands,
        // including commits, after the UI's event receiver has gone away.
        self.events.take();
        self.commands.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(data_dir: PathBuf, project: String, commands: Receiver<Command>, events: SyncSender<Event>) {
    let mut state = State {
        data_dir,
        project,
        lock: None,
        connection: None,
    };
    let _ = events.send(state.reopen());
    while let Ok(command) = commands.recv() {
        let event = match command {
            Command::Retry => state.reopen(),
            Command::Save(snapshot) => {
                let revision = snapshot.revision;
                match state.save(snapshot) {
                    Ok(saved) => Event::Saved(saved),
                    Err(error) => Event::Failed {
                        revision: Some(revision),
                        message: actionable(error),
                    },
                }
            }
            Command::Select(id) => match state.select(id) {
                Ok(saved) => Event::Selected(saved),
                Err(error) => Event::Failed {
                    revision: None,
                    message: actionable(error),
                },
            },
        };
        // Blocking provides bounded backpressure without losing acknowledgements.
        // Drop disconnects this receiver before joining, so shutdown cannot deadlock.
        let _ = events.send(event);
    }
}

struct State {
    data_dir: PathBuf,
    project: String,
    connection: Option<Connection>,
    // Declared last so SQLite closes before the stable sidecar lock is released.
    // Keep it locked even when initialization or a transaction fails.
    lock: Option<File>,
}

impl State {
    fn reopen(&mut self) -> Event {
        self.connection.take();
        match self.open_and_load() {
            Ok(snapshot) => Event::Loaded(snapshot),
            Err(error) => Event::Failed {
                revision: None,
                message: actionable(error),
            },
        }
    }

    fn open_and_load(&mut self) -> Result<Project, String> {
        if self.project.is_empty() {
            return Err("The canonical Unicode project identity is empty".into());
        }
        if self.data_dir.exists() {
            ensure_writable(&self.data_dir)?;
        }
        fs::create_dir_all(&self.data_dir)
            .map_err(|error| format!("Cannot create the data directory: {error}"))?;
        // Resolve directory aliases (including macOS /var -> /private/var)
        // before SQLite's NOFOLLOW flag checks every path component. Never
        // canonicalize the final DB/lock basename: target links remain refused.
        self.data_dir = fs::canonicalize(&self.data_dir)
            .map_err(|error| format!("Cannot resolve the data directory: {error}"))?;
        ensure_writable(&self.data_dir)?;
        if self.lock.is_none() {
            let path = self.data_dir.join(LOCK);
            regular_target(&path, true)?;
            let mut options = OpenOptions::new();
            options.read(true).write(true).create(true);
            no_follow(&mut options);
            let lock = options
                .open(&path)
                .map_err(|error| format!("Cannot open the writer lock: {error}"))?;
            ensure_regular_metadata(&lock.metadata().map_err(|error| error.to_string())?)?;
            lock.try_lock().map_err(|error| {
                format!("Cannot acquire the writer lock (another ev may own this store): {error}")
            })?;
            self.lock = Some(lock);
        }

        let path = self.data_dir.join(DATABASE);
        if regular_target(&path, true)? {
            ensure_writable(&path)?;
            // Inspect before any writable SQLite open. Unknown, corrupt, and newer
            // databases must not be changed by initialization or journal recovery.
            preflight(&path)?;
        }
        let flags =
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | sqlite_no_follow();
        let mut connection = Connection::open_with_flags(&path, flags)
            .map_err(|error| format!("Cannot open the database for writing: {error}"))?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|error| error.to_string())?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|error| error.to_string())?;
        initialize(&mut connection)?;
        let snapshot = load(&connection, &self.project)?;
        self.connection = Some(connection);
        Ok(snapshot)
    }

    fn save(&mut self, mut snapshot: Snapshot) -> Result<Snapshot, String> {
        validate(&snapshot)?;
        if snapshot.project != self.project {
            return Err(
                "Save project does not match this worker's canonical project identity".into(),
            );
        }
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it before saving")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot begin save transaction: {error}"))?;
        if snapshot.id != 0 {
            let previous = load_task(&transaction, &self.project, snapshot.id)?
                .ok_or("Task does not exist in this project")?;
            // A caller may still have the timestamp from an earlier acknowledgement.
            // Timestamps are storage-owned and never participate in revision conflicts.
            snapshot.updated_at = previous.updated_at;
            if snapshot.revision < previous.revision
                || (snapshot.revision == previous.revision && snapshot != previous)
            {
                return Err("Refusing a stale or conflicting revision; reload the committed task before saving".into());
            }
            if snapshot == previous {
                transaction
                    .commit()
                    .map_err(|error| format!("Cannot commit save transaction: {error}"))?;
                return Ok(previous);
            }
            if previous.complete
                && snapshot.complete
                && (previous.goal != snapshot.goal || previous.criteria != snapshot.criteria)
            {
                return Err(
                    "Reopen the completed task before changing its goal or criteria".into(),
                );
            }
        } else {
            let count: i64 = transaction
                .query_row(
                    "SELECT count(*) FROM tasks WHERE project=?1",
                    [&self.project],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if count >= MAX_TASKS as i64 {
                return Err("Project capacity reached (256 tasks); no tasks were removed".into());
            }
        }
        let criteria =
            serde_json::to_string(&snapshot.criteria).map_err(|error| error.to_string())?;
        let drafts = serde_json::to_string(&snapshot.drafts).map_err(|error| error.to_string())?;
        let navigation =
            serde_json::to_string(&snapshot.navigation).map_err(|error| error.to_string())?;
        let updated_at: i64 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("Cannot timestamp the save: {error}"))?
            .as_millis()
            .try_into()
            .map_err(|_| "Save timestamp exceeds SQLite's integer range")?;
        let id = if snapshot.id == 0 {
            transaction.execute(
                "INSERT INTO tasks(project,goal,form,revision,criteria,next_action,complete,drafts,navigation,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![snapshot.project, snapshot.goal, snapshot.form as i64, snapshot.revision as i64,
                    criteria, snapshot.next_action, snapshot.complete, drafts, navigation, updated_at],
            ).map_err(|error| format!("Cannot insert the task: {error}"))?;
            let id = transaction.last_insert_rowid();
            select_task(&transaction, &self.project, id)?;
            id
        } else {
            transaction.execute(
                "UPDATE tasks SET goal=?1,form=?2,revision=?3,criteria=?4,next_action=?5,complete=?6,
                 drafts=?7,navigation=?8,updated_at=?9 WHERE id=?10 AND project=?11",
                params![snapshot.goal, snapshot.form as i64, snapshot.revision as i64, criteria,
                    snapshot.next_action, snapshot.complete, drafts, navigation, updated_at,
                    snapshot.id, snapshot.project],
            ).map_err(|error| format!("Cannot write the task: {error}"))?;
            snapshot.id
        };
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit the task: {error}"))?;
        snapshot.id = id;
        snapshot.updated_at = updated_at;
        Ok(snapshot)
    }

    fn select(&mut self, id: i64) -> Result<Snapshot, String> {
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it before selecting a task")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot begin selection transaction: {error}"))?;
        let saved = load_task(&transaction, &self.project, id)?
            .ok_or("Task does not exist in this project")?;
        select_task(&transaction, &self.project, id)?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit task selection: {error}"))?;
        Ok(saved)
    }
}

fn actionable(error: String) -> String {
    format!(
        "{error}. Retry after resolving the problem, select an isolated data directory, or export readable tasks with --export-data FILE; the existing store is never reset."
    )
}

fn validate(snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.project.is_empty() {
        return Err("A stored project identity is empty".into());
    }
    if snapshot.goal.len() > MAX_GOAL_BYTES {
        return Err("A goal exceeds the 4096-byte storage limit".into());
    }
    if snapshot.form >= 9 {
        return Err("A stored form is outside 0..=8".into());
    }
    if snapshot.revision > i64::MAX as u64 {
        return Err("A revision exceeds SQLite's signed integer range".into());
    }
    if snapshot.id < 0 || snapshot.updated_at < 0 {
        return Err("Task IDs and timestamps must not be negative".into());
    }
    if snapshot.next_action.len() > MAX_GOAL_BYTES {
        return Err("A next action exceeds the 4096-byte storage limit".into());
    }
    if snapshot.criteria.len() > 64 || snapshot.criteria.iter().any(|item| item.text.len() > 256) {
        return Err("Acceptance criteria exceed 64 entries or 256 bytes per entry".into());
    }
    if snapshot.complete
        && (snapshot.criteria.is_empty() || snapshot.criteria.iter().any(|item| !item.met))
    {
        return Err("Completing a task requires nonempty, all-met acceptance criteria".into());
    }
    if snapshot.drafts.len() > 3 {
        return Err("A task may retain at most three drafts".into());
    }
    for (index, draft) in snapshot.drafts.iter().enumerate() {
        if draft.text.len() > MAX_GOAL_BYTES || !draft.text.is_char_boundary(draft.cursor) {
            return Err(
                "A draft exceeds 4096 bytes or has an invalid UTF-8 cursor boundary".into(),
            );
        }
        if snapshot.drafts[..index]
            .iter()
            .any(|other| other.field == draft.field)
        {
            return Err("A task may retain only one draft per editor field".into());
        }
    }
    let nav = &snapshot.navigation;
    if nav.focus > 2
        || nav.form_selection >= 9
        || nav.palette_selection >= 13
        || nav.criterion_selection >= 64
        || nav.overlay.is_some_and(|overlay| overlay > 5)
    {
        return Err("Stored navigation indices are out of bounds".into());
    }
    Ok(())
}

fn select_task(connection: &Connection, project: &str, id: i64) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO selections(project,active_id) VALUES (?1,?2)
         ON CONFLICT(project) DO UPDATE SET active_id=excluded.active_id",
            params![project, id],
        )
        .map_err(|error| format!("Cannot persist task selection: {error}"))?;
    Ok(())
}

fn load(connection: &Connection, project: &str) -> Result<Project, String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE project=?1 ORDER BY id"
        ))
        .map_err(|error| error.to_string())?;
    let tasks = statement
        .query_map([project], snapshot_row)
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Cannot load saved tasks: {error}"))?;
    let active_id = connection
        .query_row(
            "SELECT active_id FROM selections WHERE project=?1",
            [project],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(Project { tasks, active_id })
}

fn load_task(connection: &Connection, project: &str, id: i64) -> Result<Option<Snapshot>, String> {
    connection
        .query_row(
            &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE project=?1 AND id=?2"),
            params![project, id],
            snapshot_row,
        )
        .optional()
        .map_err(|error| format!("Cannot load the saved task: {error}"))
}

fn json_column<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    index: usize,
    max: usize,
) -> rusqlite::Result<T> {
    let text: String = row.get(index)?;
    if text.len() > max {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(io::Error::new(
                io::ErrorKind::InvalidData,
                "Stored JSON exceeds its bounded size",
            )),
        ));
    }
    serde_json::from_str(&text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn snapshot_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Snapshot> {
    let complete: i64 = row.get(7)?;
    if !(0..=1).contains(&complete) {
        return Err(rusqlite::Error::IntegralValueOutOfRange(7, complete));
    }
    Ok(Snapshot {
        project: row.get(0)?,
        goal: row.get(1)?,
        form: row.get(2)?,
        revision: row.get(3)?,
        id: row.get(4)?,
        criteria: json_column(row, 5, 110_000)?,
        next_action: row.get(6)?,
        complete: complete != 0,
        drafts: json_column(row, 8, 80_000)?,
        navigation: json_column(row, 9, 1024)?,
        updated_at: row.get(10)?,
    })
}

fn legacy_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Snapshot> {
    Ok(Snapshot {
        goal: row.get(1)?,
        form: row.get(2)?,
        revision: row.get(3)?,
        ..Snapshot::empty(row.get(0)?)
    })
}

#[derive(Clone, Copy)]
enum Schema {
    Empty,
    Legacy,
    Projects,
    Current,
}

type ColumnSpec = (&'static str, &'static str, bool, i64);

fn inspect(connection: &Connection) -> Result<Schema, String> {
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
    if !(0..=2).contains(&version) {
        return Err(format!(
            "Unsupported database schema version {version}; this ev supports versions 0, 1 and 2"
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
        (2, [(kind_a, name_a), (kind_b, name_b)])
            if kind_a == "table"
                && name_a == "selections"
                && kind_b == "table"
                && name_b == "tasks" =>
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
        Schema::Current => {
            inspect_columns(
                connection,
                "selections",
                &[
                    ("project", "TEXT", false, 1),
                    ("active_id", "INTEGER", true, 0),
                ],
            )?;
            inspect_columns(
                connection,
                "tasks",
                &[
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
                ],
            )?;
            let mut rows = connection
                .prepare(&format!("SELECT {TASK_COLUMNS} FROM tasks"))
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
                 OR EXISTS(SELECT 1 FROM tasks t LEFT JOIN selections s ON s.project=t.project
                 WHERE s.project IS NULL)
                 OR EXISTS(SELECT 1 FROM tasks GROUP BY project HAVING count(*) > 256)",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if invalid {
                return Err("Invalid project task selection or project capacity".into());
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

fn inspect_columns(
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

fn initialize(connection: &mut Connection) -> Result<(), String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("Cannot begin database initialization: {error}"))?;
    migrate(&transaction, inspect(&transaction)?)?;
    transaction
        .commit()
        .map_err(|error| format!("Cannot commit database initialization: {error}"))
}

fn migrate(connection: &Connection, schema: Schema) -> Result<(), String> {
    if matches!(schema, Schema::Current) {
        return Ok(());
    }
    connection
        .execute_batch(
            "CREATE TABLE tasks (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            goal TEXT NOT NULL CHECK(length(CAST(goal AS BLOB)) <= 4096),
            form INTEGER NOT NULL CHECK(form >= 0 AND form < 9),
            revision INTEGER NOT NULL CHECK(revision >= 0),
            criteria TEXT NOT NULL CHECK(length(CAST(criteria AS BLOB)) <= 110000),
            next_action TEXT NOT NULL CHECK(length(CAST(next_action AS BLOB)) <= 4096),
            complete INTEGER NOT NULL CHECK(complete IN (0,1)),
            drafts TEXT NOT NULL CHECK(length(CAST(drafts AS BLOB)) <= 80000),
            navigation TEXT NOT NULL CHECK(length(CAST(navigation AS BLOB)) <= 1024),
            updated_at INTEGER NOT NULL CHECK(updated_at >= 0)
        );
        CREATE TABLE selections(project TEXT PRIMARY KEY, active_id INTEGER NOT NULL);",
        )
        .map_err(|error| format!("Cannot initialize task storage: {error}"))?;
    let source = match schema {
        Schema::Legacy => Some(("goals", "0")),
        Schema::Projects => Some(("projects", "revision")),
        _ => None,
    };
    if let Some((table, revision)) = source {
        let navigation =
            serde_json::to_string(&Navigation::default()).map_err(|error| error.to_string())?;
        // Migration does not invent past activity: unknown legacy timestamps remain zero.
        connection.execute(&format!(
            "INSERT INTO tasks(project,goal,form,revision,criteria,next_action,complete,drafts,navigation,updated_at)
             SELECT project,goal,form,{revision},'[]','',0,'[]',?1,0 FROM {table} ORDER BY project"
        ), [navigation]).map_err(|error| format!("Cannot migrate {table}: {error}"))?;
        connection
            .execute_batch(&format!(
                "INSERT INTO selections SELECT project,id FROM tasks; DROP TABLE {table};"
            ))
            .map_err(|error| format!("Cannot finish migrating {table}: {error}"))?;
    }
    connection
        .pragma_update(None, "user_version", 2)
        .map_err(|error| error.to_string())
}

fn sqlite_no_follow() -> OpenFlags {
    // SQLITE_OPEN_NOFOLLOW, also covering a race after our metadata check.
    OpenFlags::from_bits_retain(0x0100_0000)
}

fn canonical_parent(path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .ok_or("The storage path has no file name")?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::canonicalize(parent)
        .map(|parent| parent.join(name))
        .map_err(|error| format!("Cannot resolve the storage directory: {error}"))
}

fn open_readonly(path: &Path) -> Result<Connection, String> {
    let path = canonical_parent(path)?;
    regular_target(&path, false)?;
    let connection =
        Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | sqlite_no_follow())
            .map_err(|error| format!("Cannot open the existing database read-only: {error}"))?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

fn preflight(path: &Path) -> Result<(), String> {
    let readonly = open_readonly(path)?;
    match readonly.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0)) {
        Ok(_) => inspect(&readonly).map(|_| ()),
        Err(error) if recovery_required(&error) => {
            drop(readonly);
            inspect_recovered_copy(path)
        }
        Err(error) => Err(format!("Cannot inspect the existing database: {error}")),
    }
}

fn recovery_required(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(code, _)
        if code.extended_code == rusqlite::ffi::SQLITE_READONLY_ROLLBACK
            || code.extended_code == rusqlite::ffi::SQLITE_READONLY_RECOVERY)
}

struct RecoveryDirectory(PathBuf);

impl RecoveryDirectory {
    fn create(parent: &Path) -> Result<Self, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..32 {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos();
            let path = parent.join(format!(
                ".ev-recovery-{}-{stamp}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!(
                        "Cannot create a private recovery directory: {error}"
                    ));
                }
            }
        }
        Err("Cannot allocate a unique private recovery directory".into())
    }
}

impl Drop for RecoveryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read_regular(path: &Path) -> Result<File, String> {
    regular_target(path, false)?;
    let mut options = OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    let file = options
        .open(path)
        .map_err(|error| format!("Cannot read recovery source: {error}"))?;
    ensure_regular_metadata(&file.metadata().map_err(|error| error.to_string())?)?;
    Ok(file)
}

fn recovery_digest(path: &Path, copy: Option<&Path>) -> Result<[u8; 32], String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    // Automatic recovery is deliberately bounded, including files growing while
    // inspected. Oversized stores stay untouched and can be isolated/exported.
    const MAX_RECOVERY_FILE: u64 = 64 * 1024 * 1024;
    let mut source = read_regular(path)?;
    if source.metadata().map_err(|error| error.to_string())?.len() > MAX_RECOVERY_FILE {
        return Err("Automatic interrupted-migration recovery is limited to 64 MiB per database/journal file".into());
    }
    let mut destination = copy
        .map(|path| OpenOptions::new().write(true).create_new(true).open(path))
        .transpose()
        .map_err(|error| format!("Cannot create private recovery copy: {error}"))?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 8192];
    let mut total = 0_u64;
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Ok(digest.finalize().into());
        }
        total += count as u64;
        if total > MAX_RECOVERY_FILE {
            return Err("Recovery source grew beyond the 64 MiB automatic recovery limit".into());
        }
        digest.update(&buffer[..count]);
        if let Some(destination) = &mut destination {
            destination
                .write_all(&buffer[..count])
                .map_err(|error| error.to_string())?;
        }
    }
}

fn inspect_recovered_copy(path: &Path) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom};
    let parent = path
        .parent()
        .ok_or("The database has no parent directory")?;
    let journal = parent.join("workbench.sqlite3-journal");
    regular_target(&journal, false)?;
    // WAL recovery needs a consistent multi-file snapshot and is not this
    // rollback-journal migration path. Never guess at a mixed-mode store.
    for suffix in ["workbench.sqlite3-wal", "workbench.sqlite3-shm"] {
        match fs::symlink_metadata(parent.join(suffix)) {
            Ok(_) => {
                return Err(
                    "Automatic rollback recovery refuses a store with WAL/SHM files".into(),
                );
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Cannot inspect recovery sidecars: {error}")),
        }
    }
    let scratch = RecoveryDirectory::create(parent)?;
    let database_copy = scratch.0.join(DATABASE);
    let journal_copy = scratch.0.join("workbench.sqlite3-journal");
    let database_digest = recovery_digest(path, Some(&database_copy))?;
    let journal_digest = recovery_digest(&journal, Some(&journal_copy))?;
    {
        let mut file = read_regular(&journal_copy)?;
        if file.metadata().map_err(|error| error.to_string())?.len() < 16 {
            return Err("The interrupted migration journal is incomplete".into());
        }
        file.seek(SeekFrom::End(-8))
            .map_err(|error| error.to_string())?;
        let mut footer = [0; 8];
        file.read_exact(&mut footer)
            .map_err(|error| error.to_string())?;
        if footer == [0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7] {
            // SQLite's super-journal footer can reference paths outside the copy.
            return Err("Automatic recovery refuses a multi-database super-journal".into());
        }
    }
    {
        let recovered = Connection::open_with_flags(
            &database_copy,
            OpenFlags::SQLITE_OPEN_READ_WRITE | sqlite_no_follow(),
        )
        .map_err(|error| format!("Cannot open the private recovery copy: {error}"))?;
        recovered
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|error| error.to_string())?;
        // SQLite rolls back only the private copy. Original files are writable
        // only after its recovered, committed schema and contents are known.
        inspect(&recovered)
            .map_err(|error| format!("Interrupted migration recovery was refused: {error}"))?;
    }
    if database_digest != recovery_digest(path, None)?
        || journal_digest != recovery_digest(&journal, None)?
    {
        return Err("The database or journal changed during recovery inspection; retry after other writers stop".into());
    }
    Ok(())
}

fn regular_target(path: &Path, allow_missing: bool) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure_regular_metadata(&metadata)?;
            Ok(true)
        }
        Err(error) if allow_missing && error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "Cannot inspect storage file {}: {error}",
            path.display()
        )),
    }
}

fn ensure_regular_metadata(metadata: &fs::Metadata) -> Result<(), String> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Storage files must be regular files, not symlinks or devices".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Storage files must not be Windows reparse points".into());
        }
    }
    Ok(())
}

fn ensure_writable(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.permissions().readonly() {
        return Err(format!("The store is read-only: {}", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o222 == 0 {
            return Err(format!("The store is read-only: {}", path.display()));
        }
    }
    Ok(())
}

fn no_follow(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
}

/// Exports readable known rows without creating or migrating a database. The
/// destination must not exist. Unlike normal opening, export permits newer
/// schema versions and does not require quick_check to pass before trying rows.
pub fn export_readable(data_dir: &Path, output: &Path) -> Result<(), String> {
    export_inner(data_dir, output).map_err(actionable)
}

fn export_inner(data_dir: &Path, output: &Path) -> Result<(), String> {
    #[derive(Serialize)]
    struct ReadableRow {
        project: String,
        goal: String,
        form: i64,
        revision: i64,
    }
    let connection = open_readonly(&data_dir.join(DATABASE))?;
    let mut rows = Vec::new();
    let mut found = false;
    let task_query = format!("SELECT {TASK_COLUMNS} FROM tasks ORDER BY project,id");
    for (table, query) in [
        ("tasks", task_query.as_str()),
        (
            "projects",
            "SELECT project, goal, form, revision FROM projects ORDER BY project",
        ),
        (
            "goals",
            "SELECT project, goal, form, 0 FROM goals ORDER BY project",
        ),
    ] {
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot inspect readable tables: {error}"))?;
        if !exists {
            continue;
        }
        let mut statement = connection
            .prepare(query)
            .map_err(|error| format!("Cannot read the known {table} columns: {error}"))?;
        found = true;
        let readable = statement
            .query_map([], |row| {
                if table == "tasks" {
                    serde_json::to_value(snapshot_row(row)?)
                } else {
                    serde_json::to_value(ReadableRow {
                        project: row.get(0)?,
                        goal: row.get(1)?,
                        form: row.get(2)?,
                        revision: row.get(3)?,
                    })
                }
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
            })
            .map_err(|error| error.to_string())?;
        for row in readable {
            rows.push(row.map_err(|error| format!("Cannot export a {table} row: {error}"))?);
        }
    }
    if !found {
        return Err("No readable known tasks, goals or projects table was found".into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|error| {
            format!("Cannot create export (the output must not already exist): {error}")
        })?;
    serde_json::to_writer_pretty(&mut file, &rows)
        .map_err(|error| format!("Cannot write export: {error}"))?;
    file.write_all(b"\n")
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("Cannot finish export: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "ev-store-{}-{stamp}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn worker(directory: &Directory, project: &str) -> Worker {
        Worker::start(directory.0.clone(), project.into()).unwrap()
    }

    fn event(worker: &Worker) -> Event {
        let event = worker
            .events
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        // Captured by the test harness and displayed on assertion failures.
        eprintln!("storage event: {event:?}");
        event
    }

    fn loaded(worker: &Worker) -> Project {
        match event(worker) {
            Event::Loaded(project) => project,
            other => panic!("expected loaded project: {other:?}"),
        }
    }

    fn saved(worker: &Worker) -> Snapshot {
        match event(worker) {
            Event::Saved(snapshot) => snapshot,
            other => panic!("expected committed snapshot: {other:?}"),
        }
    }

    fn enqueue(worker: &Worker, snapshot: Snapshot) {
        let mut pending = Some(snapshot);
        assert!(worker.try_save(&mut pending));
        assert!(pending.is_none());
    }

    fn create(worker: &Worker, snapshot: Snapshot) -> Snapshot {
        enqueue(worker, snapshot);
        saved(worker)
    }

    #[test]
    fn save_queue_retains_pending_on_full_disconnected_or_missing_sender() {
        let (commands, incoming) = mpsc::sync_channel(1);
        let mut worker = Worker {
            commands: Some(commands),
            events: None,
            thread: None,
        };
        let mut empty = None;
        assert!(!worker.try_save(&mut empty));
        let mut first = Some(snapshot("project", 1));
        assert!(worker.try_save(&mut first));
        assert!(first.is_none());

        let mut pending = Some(snapshot("project", 2));
        assert!(!worker.try_save(&mut pending));
        assert_eq!(pending.as_ref().unwrap().revision, 2);
        assert!(matches!(incoming.try_recv(), Ok(Command::Save(saved)) if saved.revision == 1));
        assert!(worker.try_save(&mut pending));
        assert!(pending.is_none());
        assert!(matches!(incoming.try_recv(), Ok(Command::Save(saved)) if saved.revision == 2));

        drop(incoming);
        let expected = snapshot("project", 3);
        let mut pending = Some(expected.clone());
        assert!(!worker.try_save(&mut pending));
        assert_eq!(pending.as_ref(), Some(&expected));
        worker.commands.take();
        assert!(!worker.try_save(&mut pending));
        assert_eq!(pending.as_ref(), Some(&expected));
    }

    fn snapshot(project: &str, revision: u64) -> Snapshot {
        Snapshot {
            goal: "保持 Unicode goal".into(),
            form: 3,
            revision,
            ..Snapshot::empty(project.into())
        }
    }

    #[test]
    fn saves_reopens_and_isolates_projects() {
        let directory = Directory::new();
        let mut committed = Vec::new();
        for project in ["/项目/one", "/项目/two"] {
            let worker = worker(&directory, project);
            assert_eq!(loaded(&worker), Project::default());
            let saved = create(&worker, snapshot(project, 1));
            assert!(saved.id > 0 && saved.updated_at > 0);
            committed.push(saved);
        }
        assert_ne!(committed[0].id, committed[1].id);
        for saved in committed {
            let worker = worker(&directory, &saved.project);
            assert_eq!(
                loaded(&worker),
                Project {
                    active_id: Some(saved.id),
                    tasks: vec![saved]
                }
            );
        }
    }

    #[test]
    fn writer_conflict_can_retry_without_takeover() {
        let directory = Directory::new();
        let first = worker(&directory, "project");
        assert_eq!(loaded(&first), Project::default());
        let second = worker(&directory, "project");
        assert!(
            matches!(event(&second), Event::Failed { revision: None, message } if message.contains("writer lock"))
        );
        assert!(second.retry());
        assert!(matches!(
            event(&second),
            Event::Failed { revision: None, .. }
        ));
        drop(first);
        assert!(second.retry());
        assert_eq!(loaded(&second), Project::default());
        enqueue(&second, snapshot("project", 1));
        assert_eq!(saved(&second).revision, 1);
    }

    #[cfg(unix)]
    #[test]
    fn directory_aliases_share_writer_identity_and_allow_readonly_open() {
        use std::os::unix::fs::symlink;
        let directory = Directory::new();
        let aliases = Directory::new();
        let alias = aliases.0.join("store-alias");
        symlink(&directory.0, &alias).unwrap();
        let first = Worker::start(alias.clone(), "project".into()).unwrap();
        assert_eq!(loaded(&first), Project::default());
        let saved = create(&first, snapshot("project", 1));
        let second = worker(&directory, "project");
        assert!(
            matches!(event(&second), Event::Failed { revision: None, message } if message.contains("writer lock"))
        );
        let readonly = open_readonly(&alias.join(DATABASE)).unwrap();
        let expected = Project {
            active_id: Some(saved.id),
            tasks: vec![saved],
        };
        assert_eq!(load(&readonly, "project").unwrap(), expected);
        drop(readonly);
        drop(first);
        assert!(second.retry());
        assert_eq!(loaded(&second), expected);
    }

    #[test]
    fn migrates_documented_schema_zero_and_one_atomically() {
        for version in [0, 1] {
            let directory = Directory::new();
            let path = directory.0.join(DATABASE);
            let connection = Connection::open(&path).unwrap();
            legacy(&connection, version);
            drop(connection);
            for (project, goal, form, revision) in [
                (
                    "project",
                    "legacy goal",
                    2,
                    if version == 0 { 0 } else { 37 },
                ),
                ("other", "other goal", 8, if version == 0 { 0 } else { 91 }),
            ] {
                let store = worker(&directory, project);
                let actual = loaded(&store);
                assert_eq!(actual.tasks.len(), 1);
                let task = &actual.tasks[0];
                assert_eq!(actual.active_id, Some(task.id));
                assert!(task.id > 0);
                assert_eq!(task.goal, goal);
                assert_eq!(task.form, form);
                assert_eq!(task.revision, revision);
                assert_eq!(task.updated_at, 0);
                assert!(task.drafts.is_empty() && task.criteria.is_empty() && !task.complete);
            }
            let connection = open_readonly(&path).unwrap();
            assert_eq!(
                connection
                    .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                    .unwrap(),
                2
            );
            assert!(connection.prepare("SELECT * FROM goals").is_err());
            assert!(connection.prepare("SELECT * FROM projects").is_err());
        }
    }

    #[test]
    fn newer_foreign_and_corrupt_databases_are_untouched() {
        for fixture in [0, 1, 2] {
            let directory = Directory::new();
            let path = directory.0.join(DATABASE);
            if fixture == 2 {
                fs::write(&path, b"not a SQLite database\0do not overwrite").unwrap();
            } else {
                let connection = Connection::open(&path).unwrap();
                connection.execute_batch(if fixture == 0 {
                    "CREATE TABLE projects(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL,revision INTEGER NOT NULL); PRAGMA user_version=99;"
                } else {
                    "CREATE TABLE unrelated(value TEXT); INSERT INTO unrelated VALUES ('keep');"
                }).unwrap();
            }
            let before = fs::read(&path).unwrap();
            let worker = worker(&directory, "project");
            assert!(matches!(
                event(&worker),
                Event::Failed { revision: None, .. }
            ));
            drop(worker);
            assert_eq!(fs::read(path).unwrap(), before);
        }
    }

    #[test]
    fn failed_transaction_is_not_acknowledged_or_committed() {
        let directory = Directory::new();
        let worker = worker(&directory, "project");
        assert_eq!(loaded(&worker), Project::default());
        let connection = Connection::open(directory.0.join(DATABASE)).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_save BEFORE INSERT ON selections BEGIN SELECT RAISE(ABORT, 'injected transaction failure'); END;").unwrap();
        enqueue(&worker, snapshot("project", 1));
        assert!(
            matches!(event(&worker), Event::Failed { revision: Some(1), message } if message.contains("injected transaction failure"))
        );
        assert!(worker.try_event().is_none());
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM tasks", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn stale_revision_cannot_replace_committed_goal() {
        let directory = Directory::new();
        let worker = worker(&directory, "project");
        assert_eq!(loaded(&worker), Project::default());
        let latest = create(&worker, snapshot("project", 2));
        for revision in [1, 2] {
            let mut stale = latest.clone();
            stale.revision = revision;
            stale.goal = "conflict".into();
            enqueue(&worker, stale);
            assert!(
                matches!(event(&worker), Event::Failed { revision: Some(actual), message }
                if actual == revision && message.contains("stale"))
            );
        }
        assert!(worker.retry());
        assert_eq!(loaded(&worker).tasks, vec![latest]);
    }

    #[test]
    fn drop_drains_accepted_saves_even_with_full_event_queue() {
        let directory = Directory::new();
        let store = worker(&directory, "project");
        assert_eq!(loaded(&store), Project::default());
        let initial = create(&store, snapshot("project", 1));
        // Fill the event queue with eight acknowledgements without receiving them.
        for revision in 2..=11 {
            let mut pending = initial.clone();
            pending.revision = revision;
            let mut pending = Some(pending);
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !store.try_save(&mut pending) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "save queue did not drain"
                );
                assert_eq!(pending.as_ref().unwrap().revision, revision);
                thread::yield_now();
            }
            assert!(pending.is_none());
        }
        drop(store);
        let reopened = worker(&directory, "project");
        let project = loaded(&reopened);
        assert_eq!(project.tasks.len(), 1);
        assert_eq!(project.tasks[0].revision, 11);
        assert_eq!(project.tasks[0].id, initial.id);
    }

    #[test]
    fn export_is_read_only_and_never_overwrites() {
        for version in [0, 1, 2] {
            let directory = Directory::new();
            let output = directory.0.join("tasks.json");
            assert!(export_readable(&directory.0, &output).is_err());
            assert!(!directory.0.join(DATABASE).exists());
            let path = directory.0.join(DATABASE);
            let expected = if version == 2 {
                let store = worker(&directory, "project");
                loaded(&store);
                Some(create(
                    &store,
                    Snapshot {
                        drafts: vec![Draft {
                            field: EditorField::Goal,
                            text: "保留".into(),
                            cursor: 6,
                        }],
                        criteria: vec![Criterion {
                            text: "reviewed".into(),
                            met: true,
                        }],
                        complete: true,
                        next_action: "Follow up".into(),
                        ..snapshot("project", 1)
                    },
                ))
            } else {
                let connection = Connection::open(&path).unwrap();
                legacy(&connection, version);
                None
            };
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch("PRAGMA user_version=42;").unwrap();
            drop(connection);
            let before = fs::read(&path).unwrap();
            export_readable(&directory.0, &output).unwrap();
            let exported = fs::read(&output).unwrap();
            let json: serde_json::Value = serde_json::from_slice(&exported).unwrap();
            if let Some(expected) = expected {
                let tasks: Vec<Snapshot> = serde_json::from_value(json).unwrap();
                assert_eq!(tasks, vec![expected]);
            } else {
                assert_eq!(json[0]["project"], "other");
                assert_eq!(json[1]["goal"], "legacy goal");
            }
            assert!(export_readable(&directory.0, &output).is_err());
            assert_eq!(fs::read(&output).unwrap(), exported);
            assert_eq!(fs::read(path).unwrap(), before);
        }
    }

    #[cfg(unix)]
    #[test]
    fn read_only_and_symlink_targets_are_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = Directory::new();
        let first = worker(&directory, "project");
        assert_eq!(loaded(&first), Project::default());
        drop(first);
        let path = directory.0.join(DATABASE);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let readonly = worker(&directory, "project");
        assert!(
            matches!(event(&readonly), Event::Failed { revision: None, message } if message.contains("read-only"))
        );
        drop(readonly);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let other = Directory::new();
        symlink(&path, other.0.join(DATABASE)).unwrap();
        let linked = worker(&other, "project");
        assert!(
            matches!(event(&linked), Event::Failed { revision: None, message } if message.contains("regular"))
        );
        assert!(export_readable(&other.0, &other.0.join("export.json")).is_err());
    }

    #[test]
    fn invalid_save_bounds_are_never_acknowledged() {
        let directory = Directory::new();
        let worker = worker(&directory, "project");
        assert_eq!(loaded(&worker), Project::default());
        let draft = Draft {
            field: EditorField::Goal,
            text: "目".into(),
            cursor: 1,
        };
        let duplicate = Draft {
            cursor: 3,
            ..draft.clone()
        };
        for invalid in [
            Snapshot {
                goal: "x".repeat(4097),
                ..snapshot("project", 1)
            },
            Snapshot {
                form: 9,
                ..snapshot("project", 1)
            },
            Snapshot {
                revision: i64::MAX as u64 + 1,
                ..snapshot("project", 1)
            },
            Snapshot {
                next_action: "目".repeat(1366),
                ..snapshot("project", 1)
            },
            Snapshot {
                id: -1,
                ..snapshot("project", 1)
            },
            Snapshot {
                criteria: vec![Criterion {
                    text: "x".repeat(257),
                    met: false,
                }],
                ..snapshot("project", 1)
            },
            Snapshot {
                criteria: vec![
                    Criterion {
                        text: String::new(),
                        met: false
                    };
                    65
                ],
                ..snapshot("project", 1)
            },
            Snapshot {
                drafts: vec![draft],
                ..snapshot("project", 1)
            },
            Snapshot {
                drafts: vec![duplicate.clone(), duplicate.clone()],
                ..snapshot("project", 1)
            },
            Snapshot {
                drafts: vec![duplicate; 4],
                ..snapshot("project", 1)
            },
            Snapshot {
                drafts: vec![Draft {
                    field: EditorField::Goal,
                    text: "x".repeat(4097),
                    cursor: 0,
                }],
                ..snapshot("project", 1)
            },
            Snapshot {
                drafts: vec![Draft {
                    field: EditorField::Goal,
                    text: "x".into(),
                    cursor: 2,
                }],
                ..snapshot("project", 1)
            },
            Snapshot {
                navigation: Navigation {
                    focus: 3,
                    ..Navigation::default()
                },
                ..snapshot("project", 1)
            },
            Snapshot {
                navigation: Navigation {
                    form_selection: 9,
                    ..Navigation::default()
                },
                ..snapshot("project", 1)
            },
            Snapshot {
                navigation: Navigation {
                    palette_selection: 13,
                    ..Navigation::default()
                },
                ..snapshot("project", 1)
            },
            Snapshot {
                navigation: Navigation {
                    criterion_selection: 64,
                    ..Navigation::default()
                },
                ..snapshot("project", 1)
            },
            Snapshot {
                navigation: Navigation {
                    overlay: Some(6),
                    ..Navigation::default()
                },
                ..snapshot("project", 1)
            },
            snapshot("other-project", 1),
        ] {
            let expected_revision = invalid.revision;
            enqueue(&worker, invalid);
            assert!(
                matches!(event(&worker), Event::Failed { revision: Some(revision), .. } if revision == expected_revision)
            );
        }
        assert!(worker.retry());
        assert_eq!(loaded(&worker), Project::default());
    }

    #[test]
    fn failed_revision_does_not_hide_a_later_accepted_save() {
        let directory = Directory::new();
        let worker = worker(&directory, "project");
        assert_eq!(loaded(&worker), Project::default());
        enqueue(
            &worker,
            Snapshot {
                form: 9,
                ..snapshot("project", 1)
            },
        );
        enqueue(&worker, snapshot("project", 2));
        assert!(matches!(
            event(&worker),
            Event::Failed {
                revision: Some(1),
                ..
            }
        ));
        let committed = saved(&worker);
        assert_eq!(committed.revision, 2);
        assert!(worker.retry());
        assert_eq!(loaded(&worker).tasks, vec![committed]);
    }

    #[test]
    fn final_form_index_eight_round_trips() {
        let directory = Directory::new();
        let saved = Snapshot {
            form: 8,
            ..snapshot("project", 1)
        };
        let store = worker(&directory, "project");
        assert_eq!(loaded(&store), Project::default());
        let saved = create(&store, saved);
        drop(store);
        let reopened = worker(&directory, "project");
        assert_eq!(loaded(&reopened).tasks, vec![saved]);
    }

    #[test]
    fn sqlite_full_rolls_back_save_without_success() {
        let directory = Directory::new();
        let mut state = State {
            data_dir: directory.0.clone(),
            project: "project".into(),
            connection: None,
            lock: None,
        };
        assert!(matches!(state.reopen(), Event::Loaded(project) if project == Project::default()));
        let connection = state.connection.as_ref().unwrap();
        let page_count: i64 = connection
            .pragma_query_value(None, "page_count", |row| row.get(0))
            .unwrap();
        connection
            .pragma_update(None, "max_page_count", page_count)
            .unwrap();
        let oversized = Snapshot {
            goal: "x".repeat(4096),
            ..snapshot("project", 1)
        };
        // The goal is within the application limit, but requires an overflow page.
        let error = state.save(oversized).unwrap_err();
        assert!(error.contains("database or disk is full"), "{error}");
        assert_eq!(
            load(state.connection.as_ref().unwrap(), "project").unwrap(),
            Project::default()
        );
        drop(state);
        let reopened = worker(&directory, "project");
        assert_eq!(loaded(&reopened), Project::default());
    }

    #[test]
    fn sqlite_full_during_migration_preserves_committed_legacy_schema() {
        for version in [0, 1] {
            let directory = Directory::new();
            let path = directory.0.join(DATABASE);
            let mut connection = Connection::open(&path).unwrap();
            legacy(&connection, version);
            let page_count: i64 = connection
                .pragma_query_value(None, "page_count", |row| row.get(0))
                .unwrap();
            connection
                .pragma_update(None, "max_page_count", page_count)
                .unwrap();
            let error = initialize(&mut connection).unwrap_err();
            assert!(error.contains("database or disk is full"), "{error}");
            assert_eq!(
                connection
                    .pragma_query_value(None, "user_version", |row| row.get::<_, u8>(0))
                    .unwrap(),
                version
            );
            let table = if version == 0 { "goals" } else { "projects" };
            assert_eq!(
                connection
                    .query_row(
                        &format!("SELECT goal FROM {table} WHERE project='project'"),
                        [],
                        |row| row.get::<_, String>(0)
                    )
                    .unwrap(),
                "legacy goal"
            );
            assert!(connection.prepare("SELECT * FROM tasks").is_err());
            assert!(connection.prepare("SELECT * FROM selections").is_err());
            drop(connection);
            let reopened = worker(&directory, "project");
            assert_eq!(loaded(&reopened).tasks[0].goal, "legacy goal");
        }
    }

    #[test]
    fn interrupted_migration_transaction_rolls_back_all_schema_changes() {
        for version in [0, 1] {
            let directory = Directory::new();
            let path = directory.0.join(DATABASE);
            let mut connection = Connection::open(&path).unwrap();
            legacy(&connection, version);
            {
                let transaction = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .unwrap();
                migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
                // Drop the actual migration transaction before commit.
            }
            assert_eq!(
                connection
                    .pragma_query_value(None, "user_version", |row| row.get::<_, u8>(0))
                    .unwrap(),
                version
            );
            let table = if version == 0 { "goals" } else { "projects" };
            assert_eq!(
                connection
                    .query_row(
                        &format!("SELECT goal FROM {table} WHERE project='project'"),
                        [],
                        |row| row.get::<_, String>(0)
                    )
                    .unwrap(),
                "legacy goal"
            );
            assert!(connection.prepare("SELECT * FROM tasks").is_err());
            assert!(connection.prepare("SELECT * FROM selections").is_err());
            drop(connection);
            let reopened = worker(&directory, "project");
            assert_eq!(loaded(&reopened).tasks[0].goal, "legacy goal");
        }
    }

    #[test]
    #[ignore = "subprocess-only fixture; invoked by hot-journal recovery tests"]
    fn interrupted_migration_process_fixture() {
        let path = std::env::var_os("EV_STORE_CRASH_FIXTURE").expect("fixture database path");
        let kind = std::env::var("EV_STORE_CRASH_KIND").unwrap();
        let mut connection = Connection::open(PathBuf::from(path)).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=DELETE;
             PRAGMA synchronous=FULL;
             PRAGMA cache_size=1;
             PRAGMA cache_spill=ON;",
            )
            .unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        match kind.as_str() {
            "current" => {
                transaction
                    .execute(
                        "UPDATE tasks SET goal=printf('%04000d',id),revision=revision+1",
                        [],
                    )
                    .unwrap();
                transaction
                    .execute("UPDATE selections SET active_id=999999", [])
                    .unwrap();
            }
            "one" => migrate(&transaction, Schema::Projects).unwrap(),
            _ => migrate(&transaction, Schema::Legacy).unwrap(),
        }
        // Terminate the actual process without running SQLite or transaction
        // destructors. The spilled transaction leaves a hot rollback journal.
        std::process::exit(73);
    }

    fn crash_during_migration(directory: &Directory, kind: &str) {
        let path = directory.0.join(DATABASE);
        let mut connection = Connection::open(&path).unwrap();
        connection.execute_batch(
            "CREATE TABLE goals(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL);
             INSERT INTO goals VALUES ('project','committed before process interruption',8);
             WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<100)
             INSERT INTO goals SELECT 'project-' || n, printf('%02000d',n), 0 FROM numbers;",
        ).unwrap();
        match kind {
            "newer" => connection.execute_batch("PRAGMA user_version=99;").unwrap(),
            "foreign" => connection.execute_batch("CREATE TABLE unrelated(value TEXT); INSERT INTO unrelated VALUES ('preserve');").unwrap(),
            "one" => connection.execute_batch(
                "CREATE TABLE projects(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL,revision INTEGER NOT NULL);
                 INSERT INTO projects SELECT project,goal,form,17 FROM goals;
                 DROP TABLE goals; PRAGMA user_version=1;",
            ).unwrap(),
            "current" => initialize(&mut connection).unwrap(),
            "known" => {}
            _ => panic!("unknown fixture"),
        }
        drop(connection);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "store::tests::interrupted_migration_process_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("EV_STORE_CRASH_FIXTURE", &path)
            .env("EV_STORE_CRASH_KIND", kind)
            .status()
            .unwrap();
        assert_eq!(
            status.code(),
            Some(73),
            "fixture must exit without transaction cleanup"
        );
        let readonly = open_readonly(&path).unwrap();
        let error = readonly
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap_err();
        assert!(
            recovery_required(&error),
            "fixture must require real journal recovery: {error}"
        );
    }

    #[test]
    fn process_interruption_recovers_legacy_and_current_committed_tasks() {
        for kind in ["known", "one", "current"] {
            let directory = Directory::new();
            crash_during_migration(&directory, kind);
            let store = worker(&directory, "project");
            let project = loaded(&store);
            assert_eq!(project.tasks.len(), 1);
            let task = &project.tasks[0];
            assert_eq!(task.goal, "committed before process interruption");
            assert_eq!(task.form, 8);
            assert_eq!(task.revision, if kind == "one" { 17 } else { 0 });
            assert_eq!(project.active_id, Some(task.id));
            drop(store);
            let connection = open_readonly(&directory.0.join(DATABASE)).unwrap();
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM tasks", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                101
            );
            assert_eq!(
                connection
                    .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                    .unwrap(),
                2
            );
        }
    }

    #[test]
    fn hot_journal_recovery_never_modifies_newer_or_foreign_originals() {
        for kind in ["newer", "foreign"] {
            let directory = Directory::new();
            crash_during_migration(&directory, kind);
            let path = directory.0.join(DATABASE);
            let journal = directory.0.join("workbench.sqlite3-journal");
            let database_before = fs::read(&path).unwrap();
            let journal_before = fs::read(&journal).unwrap();
            let worker = worker(&directory, "project");
            assert!(
                matches!(event(&worker), Event::Failed { revision: None, message } if message.contains("recovery was refused"))
            );
            drop(worker);
            assert_eq!(fs::read(path).unwrap(), database_before);
            assert_eq!(fs::read(journal).unwrap(), journal_before);
        }
    }

    fn legacy(connection: &Connection, version: u8) {
        if version == 0 {
            connection.execute_batch(
                "CREATE TABLE goals(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL);
                 INSERT INTO goals VALUES ('project','legacy goal',2),('other','other goal',8);",
            ).unwrap();
        } else {
            connection.execute_batch(
                "CREATE TABLE projects(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL,revision INTEGER NOT NULL);
                 INSERT INTO projects VALUES ('project','legacy goal',2,37),('other','other goal',8,91);
                 PRAGMA user_version=1;",
            ).unwrap();
        }
    }

    #[test]
    fn two_tasks_preserve_drafts_navigation_selection_and_timestamps_across_restart() {
        let directory = Directory::new();
        let store = worker(&directory, "project");
        assert_eq!(loaded(&store), Project::default());
        let first = create(
            &store,
            Snapshot {
                criteria: vec![Criterion {
                    text: "用户检查".into(),
                    met: true,
                }],
                next_action: "Review the result".into(),
                drafts: vec![
                    Draft {
                        field: EditorField::Goal,
                        text: "新目标".into(),
                        cursor: 3,
                    },
                    Draft {
                        field: EditorField::Criteria,
                        text: "first\nsecond".into(),
                        cursor: 6,
                    },
                    Draft {
                        field: EditorField::NextAction,
                        text: "later".into(),
                        cursor: 5,
                    },
                ],
                navigation: Navigation {
                    focus: 2,
                    linear: true,
                    form_selection: 8,
                    palette_selection: 12,
                    criterion_selection: 63,
                    scroll: [1, 2, 3, 4, u16::MAX],
                    overlay: Some(5),
                    editing: true,
                    editor_field: EditorField::NextAction,
                },
                ..snapshot("project", 7)
            },
        );
        let second = create(
            &store,
            Snapshot {
                form: 8,
                ..snapshot("project", 1)
            },
        );
        assert_ne!(first.id, second.id);
        let mut changed = first.clone();
        changed.revision += 1;
        changed.next_action = "Updated without selecting".into();
        changed.updated_at = 0;
        let first = create(&store, changed);
        assert!(store.retry());
        assert_eq!(
            loaded(&store),
            Project {
                active_id: Some(second.id),
                tasks: vec![first.clone(), second.clone()],
            }
        );
        assert!(store.try_select(first.id));
        assert!(matches!(event(&store), Event::Selected(actual) if actual == first));
        drop(store);
        let reopened = worker(&directory, "project");
        assert_eq!(
            loaded(&reopened),
            Project {
                active_id: Some(first.id),
                tasks: vec![first, second],
            }
        );
    }

    #[test]
    fn selection_rejects_missing_and_cross_project_ids_without_changing_active_task() {
        let directory = Directory::new();
        let other = worker(&directory, "other");
        loaded(&other);
        let foreign = create(&other, snapshot("other", 1));
        drop(other);
        let store = worker(&directory, "project");
        loaded(&store);
        let own = create(&store, snapshot("project", 1));
        for id in [0, -1, foreign.id, own.id + 100] {
            assert!(store.try_select(id));
            assert!(matches!(
                event(&store),
                Event::Failed { revision: None, .. }
            ));
        }
        let mut forged = foreign;
        forged.project = "project".into();
        forged.revision += 1;
        enqueue(&store, forged);
        assert!(matches!(
            event(&store),
            Event::Failed {
                revision: Some(2),
                ..
            }
        ));
        assert!(store.retry());
        assert_eq!(
            loaded(&store),
            Project {
                active_id: Some(own.id),
                tasks: vec![own]
            }
        );
    }

    #[test]
    fn completion_requires_reviewed_criteria_and_explicit_reopen_for_edits() {
        let directory = Directory::new();
        let store = worker(&directory, "project");
        loaded(&store);
        for criteria in [
            vec![],
            vec![Criterion {
                text: "check".into(),
                met: false,
            }],
        ] {
            enqueue(
                &store,
                Snapshot {
                    complete: true,
                    criteria,
                    ..snapshot("project", 1)
                },
            );
            assert!(matches!(
                event(&store),
                Event::Failed {
                    revision: Some(1),
                    ..
                }
            ));
        }
        let complete = create(
            &store,
            Snapshot {
                complete: true,
                criteria: vec![Criterion {
                    text: "check".into(),
                    met: true,
                }],
                ..snapshot("project", 2)
            },
        );
        for edit_criteria in [false, true] {
            let mut rejected = complete.clone();
            rejected.revision += 1;
            if edit_criteria {
                rejected.criteria[0].text = "different".into();
            } else {
                rejected.goal = "different".into();
            }
            enqueue(&store, rejected);
            assert!(
                matches!(event(&store), Event::Failed { revision: Some(3), message } if message.contains("Reopen"))
            );
        }
        let mut allowed = complete;
        allowed.revision += 1;
        allowed.next_action = "A completed task can retain a next action".into();
        let mut reopened = create(&store, allowed);
        reopened.revision += 1;
        reopened.complete = false;
        reopened.goal = "reopened".into();
        reopened.criteria[0].met = false;
        let reopened = create(&store, reopened);
        drop(store);
        let store = worker(&directory, "project");
        assert_eq!(loaded(&store).tasks, vec![reopened]);
    }

    #[test]
    fn current_schema_reopens_without_rewriting_or_inventing_activity() {
        let directory = Directory::new();
        let store = worker(&directory, "project");
        loaded(&store);
        let saved = create(&store, snapshot("project", 4));
        drop(store);
        let path = directory.0.join(DATABASE);
        let before = fs::read(&path).unwrap();
        let store = worker(&directory, "project");
        assert_eq!(loaded(&store).tasks, vec![saved.clone()]);
        assert_eq!(create(&store, saved.clone()), saved);
        drop(store);
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn failed_selection_preserves_previous_selection() {
        let directory = Directory::new();
        let store = worker(&directory, "project");
        loaded(&store);
        let first = create(&store, snapshot("project", 1));
        let second = create(&store, snapshot("project", 1));
        let connection = Connection::open(directory.0.join(DATABASE)).unwrap();
        connection.execute_batch(
            "CREATE TRIGGER reject_select BEFORE UPDATE ON selections BEGIN SELECT RAISE(ABORT, 'injected selection failure'); END;"
        ).unwrap();
        assert!(store.try_select(first.id));
        assert!(
            matches!(event(&store), Event::Failed { revision: None, message } if message.contains("injected selection failure"))
        );
        assert_eq!(
            load(&connection, "project").unwrap().active_id,
            Some(second.id)
        );
    }

    #[test]
    fn capacity_is_explicit_and_never_evicts_existing_tasks() {
        let directory = Directory::new();
        let store = worker(&directory, "project");
        loaded(&store);
        let mut tasks = Vec::new();
        for _ in 0..MAX_TASKS {
            tasks.push(create(&store, snapshot("project", 1)));
        }
        enqueue(&store, snapshot("project", 2));
        assert!(
            matches!(event(&store), Event::Failed { revision: Some(2), message } if message.contains("capacity"))
        );
        let mut updated = tasks[0].clone();
        updated.revision += 1;
        updated.goal = "still editable at capacity".into();
        tasks[0] = create(&store, updated);
        assert!(store.retry());
        assert_eq!(
            loaded(&store),
            Project {
                active_id: Some(tasks[MAX_TASKS - 1].id),
                tasks
            }
        );
    }

    #[test]
    fn malformed_current_schema_data_is_refused_without_rewriting() {
        for mutation in [
            "UPDATE tasks SET navigation='{}'",
            "UPDATE tasks SET criteria='not json'",
            "UPDATE selections SET active_id=99999",
            "DELETE FROM selections",
            "UPDATE tasks SET complete=1",
            "UPDATE tasks SET drafts='[{\"field\":\"Goal\",\"text\":\"目\",\"cursor\":1}]'",
        ] {
            let directory = Directory::new();
            let store = worker(&directory, "project");
            loaded(&store);
            create(&store, snapshot("project", 1));
            drop(store);
            let path = directory.0.join(DATABASE);
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch(mutation).unwrap();
            drop(connection);
            let before = fs::read(&path).unwrap();
            let store = worker(&directory, "project");
            assert!(matches!(
                event(&store),
                Event::Failed { revision: None, .. }
            ));
            drop(store);
            assert_eq!(fs::read(&path).unwrap(), before);
        }
    }

    #[test]
    fn maximum_valid_text_and_json_escaping_round_trip() {
        let directory = Directory::new();
        let store = worker(&directory, "project");
        loaded(&store);
        let text = "\0".repeat(4096);
        let expected = create(
            &store,
            Snapshot {
                goal: text.clone(),
                next_action: text.clone(),
                criteria: vec![
                    Criterion {
                        text: "\0".repeat(256),
                        met: true
                    };
                    64
                ],
                complete: true,
                drafts: [
                    EditorField::Goal,
                    EditorField::Criteria,
                    EditorField::NextAction,
                ]
                .into_iter()
                .map(|field| Draft {
                    field,
                    text: text.clone(),
                    cursor: 4096,
                })
                .collect(),
                ..snapshot("project", 1)
            },
        );
        drop(store);
        let store = worker(&directory, "project");
        assert_eq!(loaded(&store).tasks, vec![expected]);
    }
}
