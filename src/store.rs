use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

const DATABASE: &str = "workbench.sqlite3";
const LOCK: &str = "workbench.lock";
const BUSY_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_GOAL_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub project: String,
    pub goal: String,
    pub form: usize,
    pub revision: u64,
}

#[derive(Debug)]
pub enum Event {
    Loaded(Option<Snapshot>),
    Saved(u64),
    Failed {
        revision: Option<u64>,
        message: String,
    },
}

enum Command {
    Save(Snapshot),
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

    /// Returns ownership on a full or disconnected queue; acceptance is not a save acknowledgement.
    pub fn try_save(&self, snapshot: Snapshot) -> Result<(), Snapshot> {
        let Some(commands) = &self.commands else {
            return Err(snapshot);
        };
        match commands.try_send(Command::Save(snapshot)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(Command::Save(snapshot)))
            | Err(TrySendError::Disconnected(Command::Save(snapshot))) => Err(snapshot),
            Err(_) => unreachable!("try_send returns the submitted command"),
        }
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
            Command::Save(snapshot) => match state.save(&snapshot) {
                Ok(()) => Event::Saved(snapshot.revision),
                Err(error) => Event::Failed {
                    revision: Some(snapshot.revision),
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

    fn open_and_load(&mut self) -> Result<Option<Snapshot>, String> {
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

    fn save(&mut self, snapshot: &Snapshot) -> Result<(), String> {
        validate(snapshot)?;
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
        if let Some(previous) = load(&transaction, &snapshot.project)? {
            if snapshot.revision < previous.revision
                || (snapshot.revision == previous.revision && snapshot != &previous)
            {
                return Err("Refusing a stale or conflicting revision; reload the committed project before saving".into());
            }
            if snapshot == &previous {
                transaction
                    .commit()
                    .map_err(|error| format!("Cannot commit save transaction: {error}"))?;
                return Ok(());
            }
        }
        transaction.execute(
            "INSERT INTO projects(project, goal, form, revision) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(project) DO UPDATE SET goal=excluded.goal, form=excluded.form, revision=excluded.revision
             WHERE excluded.revision > projects.revision",
            params![snapshot.project, snapshot.goal, snapshot.form as i64, snapshot.revision as i64],
        ).map_err(|error| format!("Cannot write the goal: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit the goal: {error}"))?;
        Ok(())
    }
}

fn actionable(error: String) -> String {
    format!(
        "{error}. Retry after resolving the problem, select an isolated data directory, or export readable goals with --export-data FILE; the existing store is never reset."
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
    Ok(())
}

fn load(connection: &Connection, project: &str) -> Result<Option<Snapshot>, String> {
    connection
        .query_row(
            "SELECT project, goal, form, revision FROM projects WHERE project=?1",
            [project],
            snapshot_row,
        )
        .optional()
        .map_err(|error| format!("Cannot load the saved goal: {error}"))
}

fn snapshot_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Snapshot> {
    Ok(Snapshot {
        project: row.get(0)?,
        goal: row.get(1)?,
        form: row.get(2)?,
        revision: row.get(3)?,
    })
}

#[derive(Clone, Copy)]
enum Schema {
    Empty,
    Legacy,
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
    if !(0..=1).contains(&version) {
        return Err(format!(
            "Unsupported database schema version {version}; this ev supports versions 0 and 1"
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
        (1, [(kind, name)]) if kind == "table" && name == "projects" => Schema::Current,
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
        Schema::Current => (
            "projects",
            &[
                ("project", "TEXT", false, 1),
                ("goal", "TEXT", true, 0),
                ("form", "INTEGER", true, 0),
                ("revision", "INTEGER", true, 0),
            ],
            "SELECT project, goal, form, revision FROM projects",
        ),
    };
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
    let mut rows = connection
        .prepare(query)
        .map_err(|error| error.to_string())?;
    for row in rows
        .query_map([], snapshot_row)
        .map_err(|error| error.to_string())?
    {
        validate(&row.map_err(|error| format!("Invalid stored goal: {error}"))?)?;
    }
    Ok(schema)
}

fn initialize(connection: &mut Connection) -> Result<(), String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("Cannot begin database initialization: {error}"))?;
    match inspect(&transaction)? {
        Schema::Current => {}
        schema => {
            transaction
                .execute_batch(
                    "CREATE TABLE projects (
                    project TEXT PRIMARY KEY,
                    goal TEXT NOT NULL CHECK(length(CAST(goal AS BLOB)) <= 4096),
                    form INTEGER NOT NULL CHECK(form >= 0 AND form < 9),
                    revision INTEGER NOT NULL CHECK(revision >= 0)
                );",
                )
                .map_err(|error| format!("Cannot initialize projects: {error}"))?;
            if matches!(schema, Schema::Legacy) {
                // Schema 0: goals(project TEXT PRIMARY KEY, goal TEXT NOT NULL,
                // form INTEGER NOT NULL), PRAGMA user_version=0. Copy, removal,
                // and version change are one transaction, never a partial migration.
                transaction.execute_batch(
                    "INSERT INTO projects(project, goal, form, revision) SELECT project, goal, form, 0 FROM goals;
                     DROP TABLE goals;",
                ).map_err(|error| format!("Cannot migrate schema 0: {error}"))?;
            }
            transaction
                .pragma_update(None, "user_version", 1)
                .map_err(|error| error.to_string())?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("Cannot commit database initialization: {error}"))
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
    for (table, query) in [
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
                Ok(ReadableRow {
                    project: row.get(0)?,
                    goal: row.get(1)?,
                    form: row.get(2)?,
                    revision: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?;
        for row in readable {
            rows.push(row.map_err(|error| format!("Cannot export a {table} row: {error}"))?);
        }
    }
    if !found {
        return Err("No readable known goals or projects table was found".into());
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

    fn snapshot(project: &str, revision: u64) -> Snapshot {
        Snapshot {
            project: project.into(),
            goal: "保持 Unicode goal".into(),
            form: 3,
            revision,
        }
    }

    #[test]
    fn saves_reopens_and_isolates_projects() {
        let directory = Directory::new();
        let first = snapshot("/项目/one", 1);
        let second = snapshot("/项目/two", 4);
        for saved in [&first, &second] {
            let worker = worker(&directory, &saved.project);
            assert!(matches!(event(&worker), Event::Loaded(None)));
            worker.try_save(saved.clone()).unwrap();
            assert!(matches!(event(&worker), Event::Saved(revision) if revision == saved.revision));
        }
        for saved in [&first, &second] {
            let worker = worker(&directory, &saved.project);
            assert!(matches!(event(&worker), Event::Loaded(Some(actual)) if actual == *saved));
        }
    }

    #[test]
    fn writer_conflict_can_retry_without_takeover() {
        let directory = Directory::new();
        let first = worker(&directory, "project");
        assert!(matches!(event(&first), Event::Loaded(None)));
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
        assert!(matches!(event(&second), Event::Loaded(None)));
        second.try_save(snapshot("project", 1)).unwrap();
        assert!(matches!(event(&second), Event::Saved(1)));
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
        assert!(matches!(event(&first), Event::Loaded(None)));
        first.try_save(snapshot("project", 1)).unwrap();
        assert!(matches!(event(&first), Event::Saved(1)));
        let second = worker(&directory, "project");
        assert!(
            matches!(event(&second), Event::Failed { revision: None, message } if message.contains("writer lock"))
        );
        let readonly = open_readonly(&alias.join(DATABASE)).unwrap();
        assert_eq!(
            load(&readonly, "project").unwrap(),
            Some(snapshot("project", 1))
        );
        drop(readonly);
        drop(first);
        assert!(second.retry());
        assert!(
            matches!(event(&second), Event::Loaded(Some(saved)) if saved == snapshot("project", 1))
        );
    }

    #[test]
    fn migrates_documented_schema_zero_atomically() {
        let directory = Directory::new();
        let path = directory.0.join(DATABASE);
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE goals(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL); INSERT INTO goals VALUES ('project','legacy goal',2);").unwrap();
        drop(connection);
        let worker = worker(&directory, "project");
        assert!(
            matches!(event(&worker), Event::Loaded(Some(saved)) if saved.goal == "legacy goal" && saved.form == 2 && saved.revision == 0)
        );
        drop(worker);
        let connection = open_readonly(&path).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(connection.prepare("SELECT * FROM goals").is_err());
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
        assert!(matches!(event(&worker), Event::Loaded(None)));
        let connection = Connection::open(directory.0.join(DATABASE)).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_save BEFORE INSERT ON projects BEGIN SELECT RAISE(ABORT, 'injected transaction failure'); END;").unwrap();
        worker.try_save(snapshot("project", 1)).unwrap();
        assert!(
            matches!(event(&worker), Event::Failed { revision: Some(1), message } if message.contains("injected transaction failure"))
        );
        assert!(worker.try_event().is_none());
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM projects", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn stale_revision_cannot_replace_committed_goal() {
        let directory = Directory::new();
        let worker = worker(&directory, "project");
        assert!(matches!(event(&worker), Event::Loaded(None)));
        let latest = snapshot("project", 2);
        worker.try_save(latest.clone()).unwrap();
        assert!(matches!(event(&worker), Event::Saved(2)));
        worker.try_save(snapshot("project", 1)).unwrap();
        assert!(
            matches!(event(&worker), Event::Failed { revision: Some(1), message } if message.contains("stale"))
        );
        assert!(worker.retry());
        assert!(matches!(event(&worker), Event::Loaded(Some(saved)) if saved == latest));
    }

    #[test]
    fn drop_drains_accepted_saves_even_with_full_event_queue() {
        let directory = Directory::new();
        let store = worker(&directory, "project");
        assert!(matches!(event(&store), Event::Loaded(None)));
        // Fill the event queue with eight acknowledgements without receiving them.
        for revision in 1..=10 {
            let mut pending = snapshot("project", revision);
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                match store.try_save(pending) {
                    Ok(()) => break,
                    Err(returned) => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "save queue did not drain"
                        );
                        pending = returned;
                        thread::yield_now();
                    }
                }
            }
        }
        drop(store);
        let reopened = worker(&directory, "project");
        assert!(matches!(event(&reopened), Event::Loaded(Some(saved)) if saved.revision == 10));
    }

    #[test]
    fn export_is_read_only_and_never_overwrites() {
        let directory = Directory::new();
        let output = directory.0.join("goals.json");
        assert!(export_readable(&directory.0, &output).is_err());
        assert!(!directory.0.join(DATABASE).exists());
        let path = directory.0.join(DATABASE);
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE goals(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL); INSERT INTO goals VALUES ('项目','readable',1); PRAGMA user_version=42;").unwrap();
        drop(connection);
        let before = fs::read(&path).unwrap();
        export_readable(&directory.0, &output).unwrap();
        let exported = fs::read(&output).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&exported).unwrap();
        assert_eq!(json[0]["project"], "项目");
        assert!(export_readable(&directory.0, &output).is_err());
        assert_eq!(fs::read(&output).unwrap(), exported);
        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn read_only_and_symlink_targets_are_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = Directory::new();
        let first = worker(&directory, "project");
        assert!(matches!(event(&first), Event::Loaded(None)));
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
        assert!(matches!(event(&worker), Event::Loaded(None)));
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
            snapshot("other-project", 1),
        ] {
            let expected_revision = invalid.revision;
            worker.try_save(invalid).unwrap();
            assert!(
                matches!(event(&worker), Event::Failed { revision: Some(revision), .. } if revision == expected_revision)
            );
        }
        assert!(worker.retry());
        assert!(matches!(event(&worker), Event::Loaded(None)));
    }

    #[test]
    fn failed_revision_does_not_hide_a_later_accepted_save() {
        let directory = Directory::new();
        let worker = worker(&directory, "project");
        assert!(matches!(event(&worker), Event::Loaded(None)));
        worker
            .try_save(Snapshot {
                form: 9,
                ..snapshot("project", 1)
            })
            .unwrap();
        worker.try_save(snapshot("project", 2)).unwrap();
        assert!(matches!(
            event(&worker),
            Event::Failed {
                revision: Some(1),
                ..
            }
        ));
        assert!(matches!(event(&worker), Event::Saved(2)));
        assert!(worker.retry());
        assert!(matches!(event(&worker), Event::Loaded(Some(saved)) if saved.revision == 2));
    }

    #[test]
    fn final_form_index_eight_round_trips() {
        let directory = Directory::new();
        let saved = Snapshot {
            form: 8,
            ..snapshot("project", 1)
        };
        let store = worker(&directory, "project");
        assert!(matches!(event(&store), Event::Loaded(None)));
        store.try_save(saved.clone()).unwrap();
        assert!(matches!(event(&store), Event::Saved(1)));
        drop(store);
        let reopened = worker(&directory, "project");
        assert!(matches!(event(&reopened), Event::Loaded(Some(actual)) if actual == saved));
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
        assert!(matches!(state.reopen(), Event::Loaded(None)));
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
        let error = state.save(&oversized).unwrap_err();
        assert!(error.contains("database or disk is full"), "{error}");
        assert_eq!(
            load(state.connection.as_ref().unwrap(), "project").unwrap(),
            None
        );
        drop(state);
        let reopened = worker(&directory, "project");
        assert!(matches!(event(&reopened), Event::Loaded(None)));
    }

    #[test]
    fn sqlite_full_during_migration_preserves_committed_legacy_schema() {
        let directory = Directory::new();
        let path = directory.0.join(DATABASE);
        let mut connection = Connection::open(&path).unwrap();
        connection.execute_batch(
            "CREATE TABLE goals(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL);
             INSERT INTO goals VALUES ('project','committed before migration',4);",
        ).unwrap();
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
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT goal FROM goals WHERE project='project'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "committed before migration"
        );
        assert!(connection.prepare("SELECT * FROM projects").is_err());
        drop(connection);
        let reopened = worker(&directory, "project");
        assert!(
            matches!(event(&reopened), Event::Loaded(Some(saved)) if saved.goal == "committed before migration" && saved.revision == 0)
        );
    }

    #[test]
    fn interrupted_migration_transaction_rolls_back_all_schema_changes() {
        let directory = Directory::new();
        let path = directory.0.join(DATABASE);
        let mut connection = Connection::open(&path).unwrap();
        connection.execute_batch(
            "CREATE TABLE goals(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL);
             INSERT INTO goals VALUES ('project','survives interruption',6);",
        ).unwrap();
        {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            transaction.execute_batch(
                "CREATE TABLE projects(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL,revision INTEGER NOT NULL);
                 INSERT INTO projects SELECT project,goal,form,0 FROM goals;
                 DROP TABLE goals;
                 PRAGMA user_version=1;",
            ).unwrap();
            // Interruption before commit: the transaction guard rolls back the copy,
            // original-table removal, and schema version together.
        }
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT goal FROM goals", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "survives interruption"
        );
        assert!(connection.prepare("SELECT * FROM projects").is_err());
        drop(connection);
        let reopened = worker(&directory, "project");
        assert!(
            matches!(event(&reopened), Event::Loaded(Some(saved)) if saved.goal == "survives interruption" && saved.form == 6)
        );
    }

    #[test]
    #[ignore = "subprocess-only fixture; invoked by hot-journal recovery tests"]
    fn interrupted_migration_process_fixture() {
        let path = std::env::var_os("EV_STORE_CRASH_FIXTURE").expect("fixture database path");
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
        transaction.execute_batch(
            "CREATE TABLE projects(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL,revision INTEGER NOT NULL);
             INSERT INTO projects SELECT project,goal,form,0 FROM goals;
             DROP TABLE goals;
             PRAGMA user_version=1;",
        ).unwrap();
        // Terminate the actual process without running SQLite or transaction
        // destructors. The spilled transaction leaves a hot rollback journal.
        std::process::exit(73);
    }

    fn crash_during_migration(directory: &Directory, kind: &str) {
        let path = directory.0.join(DATABASE);
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(
            "CREATE TABLE goals(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL);
             INSERT INTO goals VALUES ('project','committed before process interruption',8);
             WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<100)
             INSERT INTO goals SELECT 'project-' || n, printf('%02000d',n), 0 FROM numbers;",
        ).unwrap();
        match kind {
            "newer" => connection.execute_batch("PRAGMA user_version=99;").unwrap(),
            "foreign" => connection.execute_batch("CREATE TABLE unrelated(value TEXT); INSERT INTO unrelated VALUES ('preserve');").unwrap(),
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
    fn process_interrupted_migration_recovers_committed_legacy_goal() {
        let directory = Directory::new();
        crash_during_migration(&directory, "known");
        let worker = worker(&directory, "project");
        assert!(matches!(event(&worker), Event::Loaded(Some(saved))
            if saved.goal == "committed before process interruption" && saved.form == 8 && saved.revision == 0));
        drop(worker);
        let connection = open_readonly(&directory.0.join(DATABASE)).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM projects", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            101
        );
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
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
}
