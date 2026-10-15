//! SQLite file safety, interrupted-migration recovery and export.
use super::*;

pub(super) fn sqlite_no_follow() -> OpenFlags {
    // SQLITE_OPEN_NOFOLLOW, also covering a race after our metadata check.
    OpenFlags::from_bits_retain(0x0100_0000)
}

pub(super) fn canonical_parent(path: &Path) -> Result<PathBuf, String> {
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

pub(super) fn open_readonly(path: &Path) -> Result<Connection, String> {
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

pub(super) fn preflight(path: &Path) -> Result<(), String> {
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

pub(super) fn recovery_required(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(code, _)
        if code.extended_code == rusqlite::ffi::SQLITE_READONLY_ROLLBACK
            || code.extended_code == rusqlite::ffi::SQLITE_READONLY_RECOVERY)
}

pub(super) struct RecoveryDirectory(PathBuf);

impl RecoveryDirectory {
    pub(super) fn create(parent: &Path) -> Result<Self, String> {
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
            #[cfg(unix)]
            let mut builder = fs::DirBuilder::new();
            #[cfg(not(unix))]
            let builder = fs::DirBuilder::new();
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

pub(super) fn read_regular(path: &Path) -> Result<File, String> {
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

pub(super) fn recovery_digest(path: &Path, copy: Option<&Path>) -> Result<[u8; 32], String> {
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

pub(super) fn inspect_recovered_copy(path: &Path) -> Result<(), String> {
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

pub(super) fn regular_target(path: &Path, allow_missing: bool) -> Result<bool, String> {
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

pub(super) fn ensure_regular_metadata(metadata: &fs::Metadata) -> Result<(), String> {
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

pub(super) fn ensure_writable(path: &Path) -> Result<(), String> {
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

pub(super) fn no_follow(options: &mut OpenOptions) {
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

pub(super) fn export_inner(data_dir: &Path, output: &Path) -> Result<(), String> {
    let connection = open_readonly(&data_dir.join(DATABASE))?;
    let mut rows = Vec::new();
    let mut found = false;
    let modern: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('tasks') WHERE name='archived')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let command: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('tasks') WHERE name='command')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let workflows: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('tasks') WHERE name='workflows')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let lead_partner: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('tasks') WHERE name='lead_partner')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let columns = if lead_partner {
        TASK_COLUMNS.to_string()
    } else if workflows {
        format!("{TASK_COLUMNS_V6},1")
    } else if command {
        format!("{TASK_COLUMNS_V5},{EMPTY_WORKFLOW_COLUMNS},1")
    } else if modern {
        format!("{TASK_COLUMNS_V3},'',{EMPTY_WORKFLOW_COLUMNS},1")
    } else {
        format!("{TASK_COLUMNS_V2},0,0,0,'',{EMPTY_WORKFLOW_COLUMNS},1")
    };
    let task_query = format!("SELECT {columns} FROM tasks ORDER BY project,id");
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
                let mut task = if table == "tasks" {
                    snapshot_row(row)?
                } else {
                    legacy_row(row)?
                };
                reject_secret_identity(&task.project).map_err(|error| {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(io::Error::new(
                        io::ErrorKind::InvalidData,
                        error,
                    )))
                })?;
                redact_snapshot(&mut task);
                serde_json::to_value(task)
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
    let has_journal: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='provider_journal')", [], |row| row.get(0)).map_err(|error| error.to_string())?;
    if has_journal {
        let mut statement = connection
            .prepare("SELECT task_id,record FROM provider_journal ORDER BY task_id,rowid")
            .map_err(|error| error.to_string())?;
        for row in statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    json_column::<ProviderRecord>(row, 1, 65536)?,
                ))
            })
            .map_err(|error| error.to_string())?
        {
            let (task_id, mut record) = row.map_err(|error| error.to_string())?;
            validate_provider_record(&record)?;
            sanitize_provider_record(&mut record);
            rows.push(serde_json::json!({"provider_journal":{"task_id":task_id,"record":record}}));
        }
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
