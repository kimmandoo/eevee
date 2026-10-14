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
const TASK_COLUMNS_V2: &str = "project, goal, form, revision, id, criteria, next_action, complete, drafts, navigation, updated_at";
const TASK_COLUMNS_V3: &str = "project, goal, form, revision, id, criteria, next_action, complete, drafts, navigation, updated_at, archived, archived_at, redacted";
const TASK_COLUMNS_V5: &str = "project, goal, form, revision, id, criteria, next_action, complete, drafts, navigation, updated_at, archived, archived_at, redacted, command";
const TASK_COLUMNS: &str = "project, goal, form, revision, id, criteria, next_action, complete, drafts, navigation, updated_at, archived, archived_at, redacted, command, workflows, memory_draft, approved_memories, workflow_draft";
const EMPTY_WORKFLOW_COLUMNS: &str = "'{}','null','[]','null'";
const MAX_WORKFLOW_BYTES: usize = 512 * 1024;
const MAX_MEMORY_BYTES: usize = 64 * 1024;
const MAX_MEMORIES: usize = 256;

#[derive(Clone, Copy, Debug)]
pub struct Policy {
    pub quota_bytes: u64,
    pub retention_days: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            quota_bytes: 64 * 1024 * 1024,
            retention_days: 30,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Management {
    Archive {
        id: i64,
        archived: bool,
    },
    PreviewDelete {
        ids: Vec<i64>,
        expired_before: Option<i64>,
    },
    Delete {
        ids: Vec<i64>,
        expired_before: Option<i64>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeletionTarget {
    pub task: Snapshot,
    pub excerpts: Vec<(i64, String)>,
    pub runs: Vec<(String, bool)>,
    pub provider_records: Vec<(String, crate::provider::Agent)>,
    pub memories: Vec<crate::memory::Record>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub project: String,
    pub goal: String,
    pub form: usize,
    pub revision: u64,
    pub id: i64,
    pub criteria: Vec<Criterion>,
    pub next_action: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub workflows: crate::workflows::State,
    #[serde(default)]
    pub memory_draft: serde_json::Value,
    #[serde(default)]
    pub approved_memories: Vec<crate::memory::Record>,
    #[serde(default)]
    pub workflow_draft: serde_json::Value,
    pub complete: bool,
    pub drafts: Vec<Draft>,
    pub navigation: Navigation,
    pub updated_at: i64,
    pub archived: bool,
    pub archived_at: i64,
    pub redacted: bool,
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
            command: String::new(),
            workflows: crate::workflows::State::default(),
            memory_draft: serde_json::Value::Null,
            approved_memories: Vec::new(),
            workflow_draft: serde_json::Value::Null,
            complete: false,
            drafts: Vec::new(),
            navigation: Navigation::default(),
            updated_at: 0,
            archived: false,
            archived_at: 0,
            redacted: false,
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
    Command,
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
    /// Live SQLite pages, retained output and committed capture reservations.
    pub usage_bytes: u64,
}

#[derive(Debug)]
pub struct CommittedRunPermit {
    pub request_id: String,
    pub task_id: i64,
    pub run_id: String,
    pub file: File,
    pub capacity: u64,
    _committed: (),
}

pub struct ContextRequest {
    pub task_id: i64,
    pub operation: ContextOperation,
}

pub enum ContextOperation {
    Load,
    Pin(crate::context::Excerpt),
    Remove(i64),
    Purge(Vec<i64>),
    Validate(Vec<crate::context::Excerpt>),
}

pub struct RunRequest {
    pub task_id: i64,
    pub operation: RunOperation,
}

pub enum RunOperation {
    Reserve {
        prepared: crate::runner::Prepared,
        capacity: u64,
    },
    Update(crate::runner::RunRecord),
    Abandon {
        request_id: String,
        run_id: String,
    },
    Load,
    ReadOutput {
        run_id: String,
        offset: u64,
        limit: usize,
    },
    PurgeOutput {
        run_id: String,
    },
}

#[derive(Debug)]
pub enum RunResponse {
    Reserved(CommittedRunPermit),
    Updated(crate::runner::RunRecord),
    Runs(Vec<crate::runner::RunRecord>),
    Output(OutputPage),
    // A committed purge always carries the unavailable record, even if cleanup fails.
    Purged {
        record: crate::runner::RunRecord,
        warning: Option<String>,
    },
}

#[derive(Debug)]
pub struct OutputPage {
    pub offset: u64,
    pub text: String,
    pub total_bytes: u64,
    pub unavailable: bool,
    pub truncated: bool,
}

impl RunRequest {
    fn correlation(&self) -> (Option<String>, Option<String>) {
        match &self.operation {
            RunOperation::Reserve { prepared, .. } => (
                Some(prepared.request_id.clone()),
                Some(prepared.run_id.clone()),
            ),
            RunOperation::Update(record) => (
                Some(record.prepared.request_id.clone()),
                Some(record.prepared.run_id.clone()),
            ),
            RunOperation::Abandon { request_id, run_id } => {
                (Some(request_id.clone()), Some(run_id.clone()))
            }
            RunOperation::ReadOutput { run_id, .. } | RunOperation::PurgeOutput { run_id } => {
                (None, Some(run_id.clone()))
            }
            RunOperation::Load => (None, None),
        }
    }
}

/// Provider observations are not local command evidence. IDs are scoped by an adapter
/// session nonce so a later process cannot overwrite a previous process's journal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderRecord {
    pub id: String,
    pub agent: crate::provider::Agent,
    pub generation: u64,
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub origin: String,
    pub coverage: String,
    pub text: String,
    pub complete: bool,
}
#[derive(Clone)]
pub struct JournalRequest {
    pub task_id: i64,
    pub operation: JournalOperation,
}
#[derive(Clone)]
pub enum JournalOperation {
    Put(ProviderRecord),
    List,
    Purge,
}
#[derive(Debug)]
pub enum JournalResponse {
    Saved(ProviderRecord),
    Records(Vec<ProviderRecord>),
    Purged,
}
pub struct MemoryRequest {
    pub task_id: i64,
    pub mutation: Option<crate::memory::Mutation>,
}
#[derive(Debug)]
pub enum Event {
    Loaded(Project),
    Saved(Snapshot),
    Selected(Snapshot),
    Managed {
        project: Project,
        affected: Vec<i64>,
        warning: Option<String>,
    },
    DeletePreview {
        targets: Vec<DeletionTarget>,
        expired_before: Option<i64>,
    },
    ManagementFailed(String),
    Failed {
        revision: Option<u64>,
        message: String,
    },
    Context {
        task_id: i64,
        result: Result<Vec<crate::context::Excerpt>, String>,
    },
    Run {
        task_id: i64,
        request_id: Option<String>,
        run_id: Option<String>,
        result: Result<Box<RunResponse>, String>,
    },
    Journal {
        task_id: i64,
        result: Result<JournalResponse, String>,
    },
    Memory {
        task_id: i64,
        result: Result<Vec<crate::memory::Record>, String>,
    },
}

struct Managed {
    project: Project,
    affected: Vec<i64>,
    warning: Option<String>,
}

enum Command {
    Save(Box<Snapshot>),
    Select(i64),
    Manage(Management),
    Context(ContextRequest),
    Run(Box<RunRequest>),
    Journal(JournalRequest),
    Memory(MemoryRequest),
    Retry,
}

pub struct Worker {
    commands: Option<SyncSender<Command>>,
    events: Option<Receiver<Event>>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    /// Starts storage off the UI thread. Initialization succeeds or fails via an event.
    pub fn start(data_dir: PathBuf, project: String, policy: Policy) -> io::Result<Self> {
        let (commands, incoming) = mpsc::sync_channel(2);
        let (outgoing, events) = mpsc::sync_channel(8);
        let thread = thread::Builder::new()
            .name("ev-store".into())
            .spawn(move || run(data_dir, project, policy, incoming, outgoing))?;
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
        match commands.try_send(Command::Save(Box::new(snapshot))) {
            Ok(()) => true,
            Err(TrySendError::Full(Command::Save(snapshot)))
            | Err(TrySendError::Disconnected(Command::Save(snapshot))) => {
                *pending = Some(*snapshot);
                false
            }
            Err(_) => unreachable!("try_send returns the submitted command"),
        }
    }

    /// Transfers ownership only when accepted; full/disconnected queues retain the request.
    pub fn try_manage(&self, pending: &mut Option<Management>) -> bool {
        let Some(commands) = &self.commands else {
            return false;
        };
        let Some(request) = pending.take() else {
            return false;
        };
        match commands.try_send(Command::Manage(request)) {
            Ok(()) => true,
            Err(TrySendError::Full(Command::Manage(request)))
            | Err(TrySendError::Disconnected(Command::Manage(request))) => {
                *pending = Some(request);
                false
            }
            Err(_) => unreachable!("try_send returns the submitted command"),
        }
    }

    pub fn try_context(&self, pending: &mut Option<ContextRequest>) -> bool {
        let Some(commands) = &self.commands else {
            return false;
        };
        let Some(request) = pending.take() else {
            return false;
        };
        match commands.try_send(Command::Context(request)) {
            Ok(()) => true,
            Err(
                TrySendError::Full(Command::Context(request))
                | TrySendError::Disconnected(Command::Context(request)),
            ) => {
                *pending = Some(request);
                false
            }
            Err(_) => unreachable!("try_send returns the submitted command"),
        }
    }

    pub fn try_run(&self, pending: &mut Option<RunRequest>) -> bool {
        let Some(commands) = &self.commands else {
            return false;
        };
        let Some(request) = pending.take() else {
            return false;
        };
        match commands.try_send(Command::Run(Box::new(request))) {
            Ok(()) => true,
            Err(
                TrySendError::Full(Command::Run(request))
                | TrySendError::Disconnected(Command::Run(request)),
            ) => {
                *pending = Some(*request);
                false
            }
            Err(_) => unreachable!("try_send returns the submitted command"),
        }
    }
    pub fn try_journal(&self, pending: &mut Option<JournalRequest>) -> bool {
        let Some(commands) = &self.commands else {
            return false;
        };
        let Some(request) = pending.take() else {
            return false;
        };
        match commands.try_send(Command::Journal(request)) {
            Ok(()) => true,
            Err(
                TrySendError::Full(Command::Journal(request))
                | TrySendError::Disconnected(Command::Journal(request)),
            ) => {
                *pending = Some(request);
                false
            }
            Err(_) => unreachable!("try_send returns the submitted command"),
        }
    }

    pub fn try_memory(&self, pending: &mut Option<MemoryRequest>) -> bool {
        let Some(commands) = &self.commands else {
            return false;
        };
        let Some(request) = pending.take() else {
            return false;
        };
        match commands.try_send(Command::Memory(request)) {
            Ok(()) => true,
            Err(TrySendError::Full(Command::Memory(request)))
            | Err(TrySendError::Disconnected(Command::Memory(request))) => {
                *pending = Some(request);
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

fn run(
    data_dir: PathBuf,
    project: String,
    policy: Policy,
    commands: Receiver<Command>,
    events: SyncSender<Event>,
) {
    let mut state = State {
        data_dir,
        project,
        policy,
        lock: None,
        connection: None,
    };
    let _ = events.send(state.reopen());
    while let Ok(command) = commands.recv() {
        let event = match command {
            Command::Retry => state.reopen(),
            Command::Context(request) => Event::Context {
                task_id: request.task_id,
                result: state.context(request).map_err(actionable),
            },
            Command::Journal(request) => Event::Journal {
                task_id: request.task_id,
                result: state.journal_request(request).map_err(actionable),
            },
            Command::Memory(request) => Event::Memory {
                task_id: request.task_id,
                result: state.memory_request(request).map_err(actionable),
            },
            Command::Run(request) => {
                let task_id = request.task_id;
                let (request_id, run_id) = request.correlation();
                Event::Run {
                    task_id,
                    request_id,
                    run_id,
                    result: state
                        .run_request(*request)
                        .map(Box::new)
                        .map_err(actionable),
                }
            }
            Command::Manage(Management::PreviewDelete {
                ids,
                expired_before,
            }) => match state.preview_delete(&ids, expired_before) {
                Ok(targets) => Event::DeletePreview {
                    targets,
                    expired_before,
                },
                Err(error) => Event::ManagementFailed(actionable(error)),
            },
            Command::Manage(request) => match state.manage(request) {
                Ok(Managed {
                    project,
                    affected,
                    warning,
                }) => Event::Managed {
                    project,
                    affected,
                    warning,
                },
                Err(error) => Event::ManagementFailed(actionable(error)),
            },
            Command::Save(snapshot) => {
                let revision = snapshot.revision;
                match state.save(*snapshot) {
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
    policy: Policy,
    connection: Option<Connection>,
    // Declared last so SQLite closes before the stable sidecar lock is released.
    // Keep it locked even when initialization or a transaction fails.
    lock: Option<File>,
}

impl Drop for State {
    fn drop(&mut self) {
        self.connection.take();
        if let Some(lock) = self.lock.take() {
            // A concurrent fork can retain this open-file description until exec.
            // Closing our descriptor alone would keep the writer lock alive there.
            let _ = lock.unlock();
        }
    }
}

impl State {
    fn reopen(&mut self) -> Event {
        if let Some(connection) = &self.connection
            && let Err(message) = no_active(connection, None)
        {
            return Event::Failed {
                revision: None,
                message,
            };
        }
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
        reject_secret_identity(&self.project)?;
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
        reconcile_runs(&mut connection, &self.data_dir)?;
        set_quota(&connection, self.policy)?;
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
        redact_snapshot(&mut snapshot);
        validate(&snapshot)?;
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
            if previous.archived {
                return Err("Restore the archived task before saving edits".into());
            }
            snapshot.archived = previous.archived;
            snapshot.archived_at = previous.archived_at;
            snapshot.redacted |= previous.redacted;
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
            if snapshot.archived || snapshot.archived_at != 0 {
                return Err("New tasks cannot be archived through save".into());
            }
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
        let workflows = bounded_json(&snapshot.workflows, MAX_WORKFLOW_BYTES)?;
        let memory_draft = bounded_json(&snapshot.memory_draft, MAX_MEMORY_BYTES)?;
        let approved_memories = bounded_json(&snapshot.approved_memories, MAX_WORKFLOW_BYTES)?;
        let workflow_draft = bounded_json(&snapshot.workflow_draft, MAX_WORKFLOW_BYTES)?;
        let updated_at: i64 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("Cannot timestamp the save: {error}"))?
            .as_millis()
            .try_into()
            .map_err(|_| "Save timestamp exceeds SQLite's integer range")?;
        let id = if snapshot.id == 0 {
            transaction.execute(
                "INSERT INTO tasks(project,goal,form,revision,criteria,next_action,complete,drafts,navigation,updated_at,redacted,command,workflows,memory_draft,approved_memories,workflow_draft)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
                params![snapshot.project, snapshot.goal, snapshot.form as i64, snapshot.revision as i64,
                    criteria, snapshot.next_action, snapshot.complete, drafts, navigation, updated_at, snapshot.redacted, snapshot.command,
                    workflows, memory_draft, approved_memories, workflow_draft],
            ).map_err(|error| format!("Cannot insert the task: {error}"))?;
            let id = transaction.last_insert_rowid();
            select_task(&transaction, &self.project, id)?;
            id
        } else {
            transaction.execute(
                "UPDATE tasks SET goal=?1,form=?2,revision=?3,criteria=?4,next_action=?5,complete=?6,
                 drafts=?7,navigation=?8,updated_at=?9,redacted=?12,command=?13,
                 workflows=?14,memory_draft=?15,approved_memories=?16,workflow_draft=?17 WHERE id=?10 AND project=?11",
                params![snapshot.goal, snapshot.form as i64, snapshot.revision as i64, criteria,
                    snapshot.next_action, snapshot.complete, drafts, navigation, updated_at,
                    snapshot.id, snapshot.project, snapshot.redacted, snapshot.command,
                    workflows, memory_draft, approved_memories, workflow_draft],
            ).map_err(|error| format!("Cannot write the task: {error}"))?;
            snapshot.id
        };
        enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
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
        if saved.archived {
            return Err("Archived tasks must be restored before selection".into());
        }
        select_task(&transaction, &self.project, id)?;
        enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit task selection: {error}"))?;
        Ok(saved)
    }

    fn manage(&mut self, request: Management) -> Result<Managed, String> {
        self.manage_at(request, timestamp()?)
    }

    fn preview_delete(
        &self,
        ids: &[i64],
        expired_before: Option<i64>,
    ) -> Result<Vec<DeletionTarget>, String> {
        let connection = self
            .connection
            .as_ref()
            .ok_or("Storage is unavailable; retry opening it")?;
        let eligible_before = deletion_scope(ids, expired_before, self.policy, timestamp()?)?;
        let directory = SpoolDirectory::open(&self.data_dir);
        let mut targets = Vec::with_capacity(ids.len());
        for id in ids {
            let task = deletion_task(
                connection,
                &self.project,
                *id,
                expired_before,
                eligible_before,
            )?;
            let mut statement = connection.prepare("SELECT id,json_extract(record,'$.path') FROM excerpts WHERE task_id=?1 ORDER BY id LIMIT 17")
                .map_err(|error| error.to_string())?;
            let excerpts = statement
                .query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|error| error.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?;
            if excerpts.len() > crate::context::MAX_EXCERPTS {
                return Err("Excerpt preview exceeds its bounded capacity".into());
            }
            let mut statement = connection.prepare("SELECT run_id,output_name,output_identity FROM executions WHERE task_id=?1 ORDER BY rowid LIMIT 257")
                .map_err(|error| error.to_string())?;
            let references = statement
                .query_map([id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|error| error.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?;
            if references.len() > MAX_RUNS {
                return Err("Run preview exceeds its bounded capacity".into());
            }
            let runs = references
                .into_iter()
                .map(|(run_id, name, identity)| {
                    let has_output = directory
                        .as_ref()
                        .ok()
                        .is_some_and(|directory| directory.read(&name, &identity).is_ok());
                    (run_id, has_output)
                })
                .collect();
            let mut statement = connection
                .prepare(
                    "SELECT record FROM provider_journal WHERE task_id=?1 ORDER BY rowid LIMIT 257",
                )
                .map_err(|error| error.to_string())?;
            let records = statement
                .query_map([id], |row| json_column::<ProviderRecord>(row, 0, 65536))
                .map_err(|error| error.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?;
            if records.len() > 256 {
                return Err("Provider journal deletion preview exceeds its bound".into());
            }
            let provider_records = records
                .into_iter()
                .map(|record| (record.id, record.agent))
                .collect();
            let mut statement = connection
                .prepare(
                    "SELECT id,revision,project,task_id,proposal FROM memories
                 WHERE task_id=?1 AND scope='task' ORDER BY id LIMIT 257",
                )
                .map_err(|error| error.to_string())?;
            let memories = statement
                .query_map([id], memory_row)
                .map_err(|error| error.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?;
            if memories.len() > MAX_MEMORIES {
                return Err("Memory deletion preview exceeds its bounded capacity".into());
            }
            for record in &memories {
                validate_memory(record)?;
                if !crate::memory::visible(record, &self.project, *id)
                    || record.proposal.scope != crate::memory::Scope::Task
                {
                    return Err("Memory deletion preview has conflicting task ownership".into());
                }
            }
            targets.push(DeletionTarget {
                task,
                excerpts,
                runs,
                provider_records,
                memories,
            });
        }
        Ok(targets)
    }

    fn manage_at(&mut self, request: Management, now: i64) -> Result<Managed, String> {
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot begin task management: {error}"))?;
        let mut deleted_outputs = Vec::new();
        let affected = match request {
            Management::PreviewDelete { .. } => {
                return Err(
                    "Deletion preview is read-only; no management mutation was performed".into(),
                );
            }
            Management::Archive { id, archived } => {
                no_active(&transaction, Some(id))?;
                let task = load_task(&transaction, &self.project, id)?
                    .ok_or("Task is missing or belongs to another project")?;
                if task.archived == archived {
                    return Err("Task archive state changed; review the task again".into());
                }
                transaction
                    .execute(
                        "UPDATE tasks SET archived=?1,archived_at=?2 WHERE id=?3 AND project=?4",
                        params![archived, if archived { now } else { 0 }, id, self.project],
                    )
                    .map_err(|error| format!("Cannot change archive state: {error}"))?;
                vec![id]
            }
            Management::Delete {
                ids,
                expired_before,
            } => {
                let eligible_before = deletion_scope(&ids, expired_before, self.policy, now)?;
                for id in &ids {
                    deletion_task(
                        &transaction,
                        &self.project,
                        *id,
                        expired_before,
                        eligible_before,
                    )?;
                }
                for id in &ids {
                    let mut outputs = transaction
                        .prepare(
                            "SELECT output_name,output_identity FROM executions WHERE task_id=?1",
                        )
                        .map_err(|error| error.to_string())?;
                    deleted_outputs.extend(
                        outputs
                            .query_map([id], |row| {
                                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                            })
                            .map_err(|error| error.to_string())?
                            .collect::<rusqlite::Result<Vec<_>>>()
                            .map_err(|error| error.to_string())?,
                    );
                    transaction
                        .execute("DELETE FROM excerpts WHERE task_id=?1", [id])
                        .map_err(|error| error.to_string())?;
                    transaction
                        .execute("DELETE FROM executions WHERE task_id=?1", [id])
                        .map_err(|error| error.to_string())?;
                    transaction
                        .execute("DELETE FROM provider_journal WHERE task_id=?1", [id])
                        .map_err(|error| error.to_string())?;
                    transaction
                        .execute("DELETE FROM memories WHERE task_id=?1", [id])
                        .map_err(|error| error.to_string())?;
                    transaction
                        .execute(
                            "DELETE FROM tasks WHERE project=?1 AND id=?2",
                            params![self.project, id],
                        )
                        .map_err(|error| format!("Cannot delete task: {error}"))?;
                }
                ids
            }
        };
        // A restore never creates an active selection. Repair only an invalidated one.
        let active: Option<i64> = transaction
            .query_row(
                "SELECT active_id FROM selections WHERE project=?1",
                [&self.project],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(active) = active {
            let valid =
                load_task(&transaction, &self.project, active)?.is_some_and(|task| !task.archived);
            if !valid {
                transaction
                    .execute("DELETE FROM selections WHERE project=?1", [&self.project])
                    .map_err(|error| error.to_string())?;
                let replacement: Option<i64> = transaction
                    .query_row(
                        "SELECT id FROM tasks WHERE project=?1 AND archived=0 ORDER BY id LIMIT 1",
                        [&self.project],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|error| error.to_string())?;
                if let Some(id) = replacement {
                    select_task(&transaction, &self.project, id)?;
                }
            }
        }
        let mut project = load(&transaction, &self.project)?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit task management: {error}"))?;
        // From this point the UI must receive the committed task list. Cleanup and
        // accounting failures are warnings, never rollback-shaped errors.
        let warning = if deleted_outputs.is_empty() {
            None
        } else {
            (|| -> Result<(), String> {
                let directory = SpoolDirectory::open(&self.data_dir)
                    .map_err(|error| format!("Task deletion committed, but artifact cleanup failed: {error}"))?;
                for (name, identity) in deleted_outputs {
                    directory.remove(&name, &identity)
                        .map_err(|error| format!("Task deletion committed, but artifact cleanup failed: {error}"))?;
                }
                project.usage_bytes = evidence_usage(connection, &self.data_dir)
                    .map_err(|error| format!("Task deletion committed, but updated quota accounting is unavailable: {error}"))?;
                Ok(())
            })().err()
        };
        Ok(Managed {
            project,
            affected,
            warning,
        })
    }
}

impl State {
    fn memory_request(
        &mut self,
        request: MemoryRequest,
    ) -> Result<Vec<crate::memory::Record>, String> {
        use crate::memory::{Mutation, Scope};
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot begin memory transaction: {error}"))?;
        let task = load_task(&transaction, &self.project, request.task_id)?
            .ok_or("Memory task is missing or belongs to another project")?;
        let mut records = memories(&transaction, &self.project, request.task_id)?;
        if let Some(mutation) = request.mutation {
            if task.archived {
                return Err("Restore the archived task before changing memories".into());
            }
            let (previous, proposal) = match mutation {
                Mutation::Approve(proposal) => (None, Some(proposal)),
                Mutation::Edit {
                    id,
                    expected_revision,
                    proposal,
                } => {
                    let previous = records
                        .iter()
                        .find(|record| record.id == id)
                        .ok_or("Memory is missing or outside this task's visible scope")?;
                    if previous.revision != expected_revision {
                        return Err("Memory changed; reload before editing".into());
                    }
                    (Some(previous), Some(proposal))
                }
                Mutation::Delete {
                    id,
                    expected_revision,
                } => {
                    let previous = records
                        .iter()
                        .find(|record| record.id == id)
                        .ok_or("Memory is missing or outside this task's visible scope")?;
                    if previous.revision != expected_revision {
                        return Err("Memory changed; reload before deleting".into());
                    }
                    (Some(previous), None)
                }
            };
            if let Some(proposal) = proposal {
                crate::memory::validate_proposal(&proposal)?;
                // Existing shared provenance is historical and may survive source-task purge.
                if proposal.provenance.task_id != request.task_id
                    && !previous
                        .is_some_and(|record| record.proposal.provenance == proposal.provenance)
                {
                    return Err("Memory provenance must identify this task or its unchanged approved source".into());
                }
                let task_id = (proposal.scope == Scope::Task).then_some(request.task_id);
                let scope = match proposal.scope {
                    Scope::Task => "task",
                    Scope::Project => "project",
                    Scope::Personal => "personal",
                };
                let json = bounded_json(&proposal, MAX_MEMORY_BYTES)?;
                if let Some(previous) = previous {
                    let revision = previous
                        .revision
                        .checked_add(1)
                        .filter(|revision| *revision <= i64::MAX as u64)
                        .ok_or("Memory revision exhausted")?;
                    transaction.execute(
                        "UPDATE memories SET revision=?1,project=?2,task_id=?3,scope=?4,proposal=?5 WHERE id=?6 AND revision=?7",
                        params![revision as i64, self.project, task_id, scope, json, previous.id, previous.revision as i64],
                    ).map_err(|error| format!("Cannot edit memory: {error}"))?;
                } else {
                    let count: i64 = transaction
                        .query_row("SELECT count(*) FROM memories", [], |row| row.get(0))
                        .map_err(|error| error.to_string())?;
                    if count >= MAX_MEMORIES as i64 {
                        return Err(
                            "Memory capacity reached (256 records); nothing was evicted".into()
                        );
                    }
                    transaction.execute(
                        "INSERT INTO memories(revision,project,task_id,scope,proposal) VALUES(1,?1,?2,?3,?4)",
                        params![self.project, task_id, scope, json],
                    ).map_err(|error| format!("Cannot approve memory: {error}"))?;
                }
                enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
            } else if let Some(previous) = previous {
                transaction
                    .execute(
                        "DELETE FROM memories WHERE id=?1 AND revision=?2",
                        params![previous.id, previous.revision as i64],
                    )
                    .map_err(|error| format!("Cannot delete memory: {error}"))?;
            }
            records = memories(&transaction, &self.project, request.task_id)?;
        }
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit memory transaction: {error}"))?;
        Ok(records)
    }
}

fn validate_memory(record: &crate::memory::Record) -> Result<(), String> {
    crate::memory::validate_record(record)?;
    reject_secret_identity(&record.project)?;
    if record.revision > i64::MAX as u64 || record.project.len() > 4096 {
        return Err("Memory identity exceeds its bounded storage range".into());
    }
    Ok(())
}

fn memory_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::memory::Record> {
    Ok(crate::memory::Record {
        id: row.get(0)?,
        revision: row.get(1)?,
        project: row.get(2)?,
        task_id: row.get(3)?,
        proposal: json_column(row, 4, MAX_MEMORY_BYTES)?,
    })
}

fn memories(
    connection: &Connection,
    project: &str,
    task_id: i64,
) -> Result<Vec<crate::memory::Record>, String> {
    let mut statement = connection.prepare(
        "SELECT id,revision,project,task_id,proposal FROM memories WHERE scope='personal'
         OR (project=?1 AND (scope='project' OR (scope='task' AND task_id=?2))) ORDER BY id LIMIT 257",
    ).map_err(|error| error.to_string())?;
    let records = statement
        .query_map(params![project, task_id], memory_row)
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    if records.len() > MAX_MEMORIES {
        return Err("Memory capacity exceeds its bounded limit".into());
    }
    for record in &records {
        validate_memory(record)?;
        if !crate::memory::visible(record, project, task_id) {
            return Err("Stored memory scope differs from its ownership".into());
        }
    }
    Ok(records)
}

fn inspect_memories(connection: &Connection) -> Result<(), String> {
    inspect_columns(
        connection,
        "memories",
        &[
            ("id", "INTEGER", false, 1),
            ("revision", "INTEGER", true, 0),
            ("project", "TEXT", true, 0),
            ("task_id", "INTEGER", false, 0),
            ("scope", "TEXT", true, 0),
            ("proposal", "TEXT", true, 0),
        ],
    )?;
    let invalid: bool = connection
        .query_row(
            "SELECT (SELECT count(*) FROM memories)>256 OR EXISTS(
         SELECT 1 FROM memories m LEFT JOIN tasks t ON t.id=m.task_id
         WHERE (m.scope='task' AND (t.id IS NULL OR t.project!=m.project))
         OR (m.scope!='task' AND m.task_id IS NOT NULL))",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if invalid {
        return Err("Stored memories have invalid ownership or capacity".into());
    }
    let mut statement = connection
        .prepare("SELECT id,revision,project,task_id,proposal,scope FROM memories")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| Ok((memory_row(row)?, row.get::<_, String>(5)?)))
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (record, scope) = row.map_err(|error| error.to_string())?;
        validate_memory(&record)?;
        let expected = match record.proposal.scope {
            crate::memory::Scope::Task => "task",
            crate::memory::Scope::Project => "project",
            crate::memory::Scope::Personal => "personal",
        };
        if scope != expected {
            return Err("Stored memory scope differs from its proposal".into());
        }
    }
    Ok(())
}

fn migrate_workflows(connection: &Connection) -> Result<(), String> {
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

fn check_json_size<T: Serialize>(value: &T, max: usize) -> Result<(), String> {
    struct Capacity(usize);
    impl Write for Capacity {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self.0.checked_sub(bytes.len()).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Stored JSON exceeds its bounded size",
                )
            })?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Capacity(max), value).map_err(|error| error.to_string())
}

fn validate_draft_shape(value: &serde_json::Value, depth: usize) -> Result<(), String> {
    if depth > 32 {
        return Err("Stored drawer draft exceeds its nesting limit".into());
    }
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                validate_draft_shape(value, depth + 1)?;
            }
        }
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                if crate::privacy::redact(key) != key.as_str() {
                    return Err("Stored draft field names must not contain credentials".into());
                }
                validate_draft_shape(value, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn bounded_json<T: Serialize>(value: &T, max: usize) -> Result<String, String> {
    let json = serde_json::to_string(value).map_err(|error| error.to_string())?;
    if json.len() > max {
        return Err(format!(
            "Stored workflow or memory JSON exceeds {max} bytes"
        ));
    }
    Ok(json)
}

fn redact_json(value: &mut serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => match crate::privacy::redact(text) {
            std::borrow::Cow::Owned(clean) if clean != *text => {
                *text = clean;
                true
            }
            _ => false,
        },
        serde_json::Value::Array(values) => values
            .iter_mut()
            .fold(false, |changed, value| redact_json(value) | changed),
        serde_json::Value::Object(values) => values
            .values_mut()
            .fold(false, |changed, value| redact_json(value) | changed),
        _ => false,
    }
}

impl State {
    fn journal_request(&mut self, request: JournalRequest) -> Result<JournalResponse, String> {
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
            JournalOperation::Put(mut record) => {
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
                    enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
                }
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
fn sanitize_provider_record(record: &mut ProviderRecord) {
    for text in [&mut record.origin, &mut record.coverage, &mut record.text] {
        // Normalize before masking: removing controls can join a split credential name
        // (for example, "pass\u{0008}word") into one recognized by the privacy boundary.
        text.retain(|ch| (!ch.is_control() || ch == '\n' || ch == '\t')
            && !matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'));
        if let std::borrow::Cow::Owned(clean) = crate::privacy::redact(text) {
            *text = clean;
        }
    }
}

fn validate_provider_record(record: &ProviderRecord) -> Result<(), String> {
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
            || crate::privacy::redact(identity) != *identity
        {
            return Err("Provider journal identity is invalid or credential-bearing".into());
        }
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
const MAX_RUN_BYTES: usize = 128 * 1024;
const MAX_RUNS: usize = 256;
const RUN_HEADROOM: u64 = 256 * 1024;
const MAX_SPOOL_BYTES: u64 = 8 * 1024 * 1024;
const RUN_COLUMNS: &str = "record,output_name,output_identity,capacity,reserved";

struct StoredRun {
    record: crate::runner::RunRecord,
    output_name: String,
    output_identity: String,
    capacity: u64,
    reserved: bool,
}

fn active_state(state: &crate::runner::RunState) -> bool {
    use crate::runner::RunState;
    matches!(
        state,
        RunState::AwaitingDispatch | RunState::Running | RunState::Cancelling
    )
}

pub(crate) fn valid_run_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
fn deletion_scope(
    ids: &[i64],
    expired_before: Option<i64>,
    policy: Policy,
    now: i64,
) -> Result<i64, String> {
    if ids.is_empty() || ids.len() > MAX_TASKS {
        return Err("Deletion requires a bounded, nonempty list of task IDs".into());
    }
    for (index, id) in ids.iter().enumerate() {
        if ids[..index].contains(id) {
            return Err("Deletion contains duplicate task IDs".into());
        }
    }
    let eligible_before = now.saturating_sub(i64::from(policy.retention_days) * 86_400_000);
    if expired_before.is_some_and(|cutoff| cutoff < 0 || cutoff > eligible_before) {
        return Err(
            "Retention preview is incompatible with the configured policy; review again".into(),
        );
    }
    Ok(eligible_before)
}

fn deletion_task(
    connection: &Connection,
    project: &str,
    id: i64,
    expired_before: Option<i64>,
    eligible_before: i64,
) -> Result<Snapshot, String> {
    no_active(connection, Some(id))?;
    let task = load_task(connection, project, id)?
        .ok_or("Deletion target is missing or belongs to another project; review again")?;
    if expired_before.is_some_and(|cutoff| {
        !task.archived
            || task.archived_at <= 0
            || task.archived_at > cutoff
            || task.archived_at > eligible_before
    }) {
        return Err("Retention target changed or is not old enough; review again".into());
    }
    Ok(task)
}

fn spool_name(run_id: &str) -> Result<String, String> {
    if !valid_run_id(run_id) {
        return Err("Invalid run identity".into());
    }
    Ok(format!("ev-run-{run_id}.output"))
}

fn run_json(record: &crate::runner::RunRecord) -> Result<String, String> {
    use crate::runner::{RunState, TreeOutcome};
    let prepared = &record.prepared;
    if prepared
        .before
        .fingerprint
        .as_ref()
        .is_some_and(|value| value.len() > 256)
        || record
            .after
            .fingerprint
            .as_ref()
            .is_some_and(|value| value.len() > 256)
    {
        return Err("Source identity exceeds the journal bound".into());
    }
    if !valid_run_id(&prepared.run_id)
        || !valid_run_id(&prepared.request_id)
        || prepared.task_id <= 0
    {
        return Err("Invalid run/request/task correlation".into());
    }
    if matches!(record.state, RunState::Running | RunState::Cancelling)
        && record.started_at_ms.is_none()
        || active_state(&record.state) && record.ended_at_ms.is_some()
        || record.state == RunState::AwaitingDispatch && record.started_at_ms.is_some()
    {
        return Err("Run lifecycle timestamps do not match its observed state".into());
    }
    if record.state == RunState::Succeeded
        && (record.started_at_ms.is_none()
            || record.exit_code != Some(0)
            || record.tree != TreeOutcome::OwnedTreeExited
            || record.cancelled
            || record.signal.is_some())
    {
        return Err(
            "Successful command metadata lacks observed start, exit zero or owned-tree exit".into(),
        );
    }
    if record.state == RunState::Cancelling && !record.cancelled
        || record.state == RunState::Cancelled
            && (!record.cancelled
                || record.started_at_ms.is_none()
                || record.tree != TreeOutcome::OwnedTreeExited)
    {
        return Err(
            "Cancelled command metadata lacks an observed cancellation and owned-tree exit".into(),
        );
    }
    if record
        .ended_at_ms
        .is_some_and(|end| record.started_at_ms.is_some_and(|start| end < start))
    {
        return Err("Run end precedes its start".into());
    }
    let json = serde_json::to_string(record).map_err(|error| error.to_string())?;
    if json.len() > MAX_RUN_BYTES {
        return Err("Run metadata exceeds 128 KiB".into());
    }
    Ok(json)
}

fn no_active(connection: &Connection, task_id: Option<i64>) -> Result<(), String> {
    let active: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM executions WHERE reserved=1 AND (?1 IS NULL OR task_id=?1))",
        [task_id], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if active {
        Err("Task owns reserved or active work; cancel and wait before removing, purging or archiving evidence".into())
    } else {
        Ok(())
    }
}

fn excerpts(connection: &Connection, task_id: i64) -> Result<Vec<crate::context::Excerpt>, String> {
    let mut statement = connection
        .prepare("SELECT record FROM excerpts WHERE task_id=?1 ORDER BY id LIMIT 17")
        .map_err(|error| error.to_string())?;
    let result = statement
        .query_map([task_id], |row| json_column(row, 0, 512 * 1024))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<crate::context::Excerpt>>>()
        .map_err(|error| error.to_string())?;
    if result.len() > crate::context::MAX_EXCERPTS {
        return Err("Stored basket exceeds its bound".into());
    }
    for excerpt in &result {
        excerpt.validate()?;
    }
    Ok(result)
}

fn stored_run(connection: &Connection, task_id: i64, run_id: &str) -> Result<StoredRun, String> {
    connection
        .query_row(
            &format!("SELECT {RUN_COLUMNS} FROM executions WHERE task_id=?1 AND run_id=?2"),
            params![task_id, run_id],
            stored_run_row,
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Run is missing or belongs to another task".into())
}

fn stored_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRun> {
    Ok(StoredRun {
        record: json_column(row, 0, MAX_RUN_BYTES)?,
        output_name: row.get(1)?,
        output_identity: row.get(2)?,
        capacity: row.get(3)?,
        reserved: row.get(4)?,
    })
}

fn write_run(connection: &Connection, stored: &StoredRun) -> Result<(), String> {
    connection
        .execute(
            "UPDATE executions SET record=?1,reserved=?2 WHERE run_id=?3 AND task_id=?4",
            params![
                run_json(&stored.record)?,
                stored.reserved,
                stored.record.prepared.run_id,
                stored.record.prepared.task_id
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

impl State {
    fn context(&mut self, request: ContextRequest) -> Result<Vec<crate::context::Excerpt>, String> {
        let growing = matches!(&request.operation, ContextOperation::Pin(_));
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let task = load_task(&transaction, &self.project, request.task_id)?
            .ok_or("Task is missing or belongs to another project")?;
        let current = excerpts(&transaction, task.id)?;
        match request.operation {
            ContextOperation::Load => return Ok(current),
            ContextOperation::Validate(expected) => {
                if expected.is_empty() || expected.len() > crate::context::MAX_EXCERPTS {
                    return Err("A dispatch basket requires 1–16 committed excerpts".into());
                }
                for (index, excerpt) in expected.iter().enumerate() {
                    if excerpt.task_id != task.id
                        || !excerpt.available
                        || expected[..index].iter().any(|old| old.id == excerpt.id)
                        || !current.iter().any(|stored| stored == excerpt)
                    {
                        return Err(
                            "Basket changed, was removed or purged; load and review it again"
                                .into(),
                        );
                    }
                }
                return Ok(expected);
            }
            ContextOperation::Pin(mut excerpt) => {
                no_active(&transaction, Some(task.id))?;
                if task.archived {
                    return Err("Restore the task before pinning context".into());
                }
                if current.len() >= crate::context::MAX_EXCERPTS {
                    return Err("Basket capacity reached (16 excerpts); nothing was evicted".into());
                }
                if excerpt.task_id != task.id || excerpt.id != 0 || !excerpt.available {
                    return Err("Pin requires a new available capture for this task".into());
                }
                excerpt.text = crate::privacy::redact(&excerpt.text).into_owned();
                excerpt.validate()?;
                // The row identity belongs to storage, not to a user-supplied reference.
                transaction
                    .execute(
                        "INSERT INTO excerpts(task_id,record) VALUES (?1,'')",
                        [task.id],
                    )
                    .map_err(|error| error.to_string())?;
                excerpt.id = transaction.last_insert_rowid();
                transaction
                    .execute(
                        "UPDATE excerpts SET record=?1 WHERE id=?2",
                        params![
                            serde_json::to_string(&excerpt).map_err(|error| error.to_string())?,
                            excerpt.id
                        ],
                    )
                    .map_err(|error| error.to_string())?;
            }
            ContextOperation::Remove(id) => {
                no_active(&transaction, Some(task.id))?;
                if !current.iter().any(|excerpt| excerpt.id == id) {
                    return Err("Excerpt is missing or belongs to another task".into());
                }
                transaction
                    .execute(
                        "DELETE FROM excerpts WHERE task_id=?1 AND id=?2",
                        params![task.id, id],
                    )
                    .map_err(|error| error.to_string())?;
            }
            ContextOperation::Purge(ids) => {
                no_active(&transaction, Some(task.id))?;
                if ids.is_empty() || ids.len() > crate::context::MAX_EXCERPTS {
                    return Err("Purge requires a bounded nonempty excerpt selection".into());
                }
                for (index, id) in ids.iter().enumerate() {
                    if ids[..index].contains(id) {
                        return Err("Duplicate purge reference".into());
                    }
                    let mut excerpt = current
                        .iter()
                        .find(|excerpt| excerpt.id == *id)
                        .cloned()
                        .ok_or("Purge reference is missing or belongs to another task")?;
                    excerpt.text.clear();
                    excerpt.available = false;
                    transaction
                        .execute(
                            "UPDATE excerpts SET record=?1 WHERE task_id=?2 AND id=?3",
                            params![
                                serde_json::to_string(&excerpt)
                                    .map_err(|error| error.to_string())?,
                                task.id,
                                id
                            ],
                        )
                        .map_err(|error| error.to_string())?;
                }
            }
        }
        if growing {
            enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
        }
        let result = excerpts(&transaction, task.id)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(result)
    }

    fn run_request(&mut self, request: RunRequest) -> Result<RunResponse, String> {
        let task_id = request.task_id;
        let connection = self
            .connection
            .as_ref()
            .ok_or("Storage is unavailable; retry opening it")?;
        load_task(connection, &self.project, task_id)?
            .ok_or("Task is missing or belongs to another project")?;
        match request.operation {
            RunOperation::Reserve { prepared, capacity } => self
                .reserve_run(task_id, prepared, capacity)
                .map(RunResponse::Reserved),
            RunOperation::Update(record) => {
                self.update_run(task_id, record).map(RunResponse::Updated)
            }
            RunOperation::Abandon { request_id, run_id } => {
                let mut stored = stored_run(connection, task_id, &run_id)?;
                if stored.record.prepared.request_id != request_id
                    || stored.record.state != crate::runner::RunState::AwaitingDispatch
                {
                    return Err(
                        "Only the correlated, undispatched reservation can be abandoned".into(),
                    );
                }
                stored.record.state = crate::runner::RunState::Interrupted;
                stored.record.ended_at_ms = Some(timestamp()? as u64);
                stored.record.error = Some(
                    "Approval was invalidated or dispatch was refused; command was not started"
                        .into(),
                );
                stored.record.output_unavailable = true;
                self.update_run(task_id, stored.record)
                    .map(RunResponse::Updated)
            }
            RunOperation::Load => self.load_runs(task_id).map(RunResponse::Runs),
            RunOperation::ReadOutput {
                run_id,
                offset,
                limit,
            } => self
                .read_output(task_id, &run_id, offset, limit)
                .map(RunResponse::Output),
            RunOperation::PurgeOutput { run_id } => self
                .purge_output(task_id, &run_id)
                .map(|(record, warning)| RunResponse::Purged { record, warning }),
        }
    }

    fn reserve_run(
        &mut self,
        task_id: i64,
        prepared: crate::runner::Prepared,
        capacity: u64,
    ) -> Result<CommittedRunPermit, String> {
        if prepared.task_id != task_id || !(1024..=MAX_SPOOL_BYTES).contains(&capacity) {
            return Err("Run owner or capture reservation bound is invalid (1 KiB–8 MiB)".into());
        }
        if timestamp()? as u64 >= prepared.expires_at_ms {
            return Err("Prepared request expired before quota reservation; review again".into());
        }
        let mut record = crate::runner::RunRecord::awaiting(&prepared);
        record.redact_for_storage();
        let json = run_json(&record)?;
        let name = spool_name(&prepared.run_id)?;
        let connection = self.connection.as_mut().ok_or("Storage is unavailable")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        no_active(&transaction, None)?;
        let task = load_task(&transaction, &self.project, task_id)?.ok_or("Task is missing")?;
        if task.archived {
            return Err("Restore the task before execution".into());
        }
        let count: usize = transaction
            .query_row(
                "SELECT count(*) FROM executions WHERE task_id=?1",
                [task_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if count >= MAX_RUNS {
            return Err("Run journal reached 256 retained records; no evidence was evicted".into());
        }
        let directory = SpoolDirectory::open(&self.data_dir)?;
        let file = directory.create(&name)?;
        let identity = file_identity(&file)?;
        let result = (|| {
            file.sync_all().map_err(|error| error.to_string())?;
            directory.sync()?;
            transaction.execute("INSERT INTO executions(run_id,task_id,request_id,record,output_name,output_identity,capacity,reserved) VALUES (?1,?2,?3,?4,?5,?6,?7,1)",
                params![prepared.run_id, task_id, prepared.request_id, json, name, identity, capacity]).map_err(|error| error.to_string())?;
            enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
            transaction.commit().map_err(|error| error.to_string())
        })();
        if let Err(error) = result {
            drop(file);
            let _ = directory.remove(&name, &identity);
            return Err(error);
        }
        Ok(CommittedRunPermit {
            request_id: prepared.request_id,
            task_id,
            run_id: prepared.run_id,
            file,
            capacity,
            _committed: (),
        })
    }

    fn update_run(
        &mut self,
        task_id: i64,
        mut record: crate::runner::RunRecord,
    ) -> Result<crate::runner::RunRecord, String> {
        use crate::runner::RunState;
        if record.prepared.task_id != task_id {
            return Err("Run event belongs to another task".into());
        }
        record.redact_for_storage();
        run_json(&record)?;
        let connection = self.connection.as_mut().ok_or("Storage is unavailable")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let mut stored = stored_run(&transaction, task_id, &record.prepared.run_id)?;
        if stored.record.prepared != record.prepared {
            return Err("Run event correlation or approved inputs changed".into());
        }
        if !stored.reserved || !active_state(&stored.record.state) {
            return Err("Late run event cannot rewrite terminal journal evidence".into());
        }
        let transition = match stored.record.state {
            // Each notification carries a complete observation. A failed intermediate
            // commit must not strand the reservation when a later observed state arrives.
            RunState::AwaitingDispatch => record.state != RunState::AwaitingDispatch,
            RunState::Running => record.state != RunState::AwaitingDispatch,
            RunState::Cancelling => {
                !matches!(record.state, RunState::AwaitingDispatch | RunState::Running)
            }
            _ => false,
        };
        if !transition || record.captured_bytes > stored.capacity {
            return Err("Invalid run transition or capture exceeds its reservation".into());
        }
        if stored.record.started_at_ms.is_some()
            && stored.record.started_at_ms != record.started_at_ms
        {
            return Err("Run start metadata is immutable".into());
        }
        if record.captured_bytes < stored.record.captured_bytes
            || record.discarded_bytes < stored.record.discarded_bytes
            || record.dropped_live_bytes < stored.record.dropped_live_bytes
            || stored.record.cancelled && !record.cancelled
            || stored.record.truncated && !record.truncated
            || stored.record.output_unavailable && !record.output_unavailable
        {
            return Err("Run observation cannot erase previously committed capture or cancellation evidence".into());
        }
        stored.reserved = active_state(&record.state);
        if !stored.reserved {
            record.ended_at_ms.get_or_insert(timestamp()? as u64);
            match SpoolDirectory::open(&self.data_dir)
                .and_then(|directory| directory.read(&stored.output_name, &stored.output_identity))
            {
                Ok(file) => {
                    if file.metadata().map_err(|error| error.to_string())?.len()
                        != record.captured_bytes
                    {
                        record.output_unavailable = true;
                    }
                }
                Err(_) => record.output_unavailable = true,
            }
        }
        stored.record = record;
        write_run(&transaction, &stored)?;
        // Reservation includes bounded terminal metadata headroom. Do not evict evidence
        // or reject an honest terminal transition solely because the quota was lowered.
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(stored.record)
    }

    fn load_runs(&self, task_id: i64) -> Result<Vec<crate::runner::RunRecord>, String> {
        let connection = self.connection.as_ref().ok_or("Storage is unavailable")?;
        let mut statement = connection.prepare(&format!("SELECT {RUN_COLUMNS} FROM executions WHERE task_id=?1 ORDER BY rowid DESC LIMIT 257")).map_err(|error| error.to_string())?;
        let stored = statement
            .query_map([task_id], stored_run_row)
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        if stored.len() > MAX_RUNS {
            return Err("Journal exceeds its bounded record capacity".into());
        }
        let directory = SpoolDirectory::open(&self.data_dir);
        Ok(stored
            .into_iter()
            .map(|mut stored| {
                if directory
                    .as_ref()
                    .ok()
                    .and_then(|directory| {
                        directory
                            .read(&stored.output_name, &stored.output_identity)
                            .ok()
                    })
                    .and_then(|file| file.metadata().ok())
                    .is_none_or(|metadata| {
                        !stored.reserved && metadata.len() != stored.record.captured_bytes
                    })
                {
                    stored.record.output_unavailable = true;
                }
                stored.record
            })
            .collect())
    }

    fn read_output(
        &self,
        task_id: i64,
        run_id: &str,
        offset: u64,
        limit: usize,
    ) -> Result<OutputPage, String> {
        use std::io::{Read, Seek, SeekFrom};
        if limit == 0 || limit > 64 * 1024 {
            return Err("Output reads require a 1–65536-byte bound".into());
        }
        let connection = self.connection.as_ref().ok_or("Storage is unavailable")?;
        let stored = stored_run(connection, task_id, run_id)?;
        let unavailable = || OutputPage {
            offset,
            text: String::new(),
            total_bytes: stored.record.captured_bytes,
            unavailable: true,
            truncated: stored.record.truncated,
        };
        if stored.record.output_unavailable {
            return Ok(unavailable());
        }
        let Ok(mut file) = SpoolDirectory::open(&self.data_dir)
            .and_then(|directory| directory.read(&stored.output_name, &stored.output_identity))
        else {
            return Ok(unavailable());
        };
        let total = file.metadata().map_err(|error| error.to_string())?.len();
        if total > stored.capacity || (!stored.reserved && total != stored.record.captured_bytes) {
            return Ok(unavailable());
        }
        if offset > total {
            return Err("Output offset is beyond retained bytes".into());
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        let mut bytes = vec![0; limit.min((total - offset) as usize)];
        file.read_exact(&mut bytes)
            .map_err(|error| error.to_string())?;
        match std::str::from_utf8(&bytes) {
            Ok(_) => {}
            Err(error) if error.error_len().is_none() => bytes.truncate(error.valid_up_to()),
            Err(_) => {
                return Err("Output offset is not a UTF-8 boundary or capture is invalid".into());
            }
        }
        if bytes.is_empty() && offset < total {
            return Err("Output read bound is too small for the next UTF-8 character".into());
        }
        let text = String::from_utf8(bytes).map_err(|error| error.to_string())?;
        Ok(OutputPage {
            offset,
            text,
            total_bytes: total,
            unavailable: false,
            truncated: stored.record.truncated,
        })
    }

    fn purge_output(
        &mut self,
        task_id: i64,
        run_id: &str,
    ) -> Result<(crate::runner::RunRecord, Option<String>), String> {
        let connection = self.connection.as_mut().ok_or("Storage is unavailable")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        no_active(&transaction, Some(task_id))?;
        let mut stored = stored_run(&transaction, task_id, run_id)?;
        stored.record.output_unavailable = true;
        write_run(&transaction, &stored)?;
        transaction.commit().map_err(|error| error.to_string())?;
        // Metadata becomes unavailable before physical cleanup. A replaced link is
        // never followed, and a cleanup failure cannot resurrect successful evidence.
        let warning = SpoolDirectory::open(&self.data_dir)
            .and_then(|directory| directory.remove(&stored.output_name, &stored.output_identity))
            .err()
            .map(|error| {
                format!(
                    "Output metadata is committed unavailable, but physical cleanup failed: {error}"
                )
            });
        Ok((stored.record, warning))
    }
}

fn reconcile_runs(connection: &mut Connection, data_dir: &Path) -> Result<(), String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let mut records = {
        let mut statement = transaction
            .prepare(&format!(
                "SELECT {RUN_COLUMNS} FROM executions WHERE reserved=1"
            ))
            .map_err(|error| error.to_string())?;
        statement
            .query_map([], stored_run_row)
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?
    };
    for stored in &mut records {
        stored.reserved = false;
        stored.record.state = crate::runner::RunState::Interrupted;
        stored.record.tree = crate::runner::TreeOutcome::OwnershipUnknown;
        stored.record.ended_at_ms = Some(timestamp()? as u64);
        stored.record.error = Some("Eevee restarted without re-established process ownership; outcome is unknown and nothing was re-executed".into());
        stored.record.output_unavailable = true;
        stored.record.truncated = true;
        if let Ok(file) = SpoolDirectory::open(data_dir)
            .and_then(|directory| directory.read(&stored.output_name, &stored.output_identity))
        {
            let length = file.metadata().map_err(|error| error.to_string())?.len();
            if length <= stored.capacity {
                stored.record.captured_bytes = length;
                stored.record.output_unavailable = false;
            }
        }
        write_run(&transaction, stored)?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

fn evidence_usage(connection: &Connection, data_dir: &Path) -> Result<u64, String> {
    let mut total = live_bytes(connection)?;
    let mut sizes = std::collections::BTreeMap::new();
    for entry in fs::read_dir(data_dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with("ev-run-") || !name.ends_with(".output") {
            continue;
        }
        if sizes.len() >= 65_536 {
            return Err("Spool inventory exceeds the bounded store capacity".into());
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        // Account all owned-named retained files, including an interrupted pre-commit
        // orphan. Links are never followed to measure external target bytes.
        let size = if ensure_regular_metadata(&metadata).is_ok() {
            metadata.len()
        } else {
            0
        };
        total = total.saturating_add(size);
        sizes.insert(name.to_owned(), size);
    }
    let mut statement = connection
        .prepare("SELECT output_name,capacity FROM executions WHERE reserved=1")
        .map_err(|error| error.to_string())?;
    for row in statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })
        .map_err(|error| error.to_string())?
    {
        let (name, capacity) = row.map_err(|error| error.to_string())?;
        total = total
            .saturating_add(capacity.saturating_sub(sizes.get(&name).copied().unwrap_or(0)))
            .saturating_add(RUN_HEADROOM);
    }
    Ok(total)
}

fn enforce_evidence_quota(
    connection: &Connection,
    data_dir: &Path,
    policy: Policy,
) -> Result<(), String> {
    if evidence_usage(connection, data_dir)? > policy.quota_bytes {
        Err("Local storage quota exceeded (live SQLite + retained output + committed reservations); nothing was evicted and no command was started".into())
    } else {
        Ok(())
    }
}
/// A directory handle anchors POSIX operations. On Windows, deny-delete handles
/// retain every ancestor while an operation uses its path; final files are no-follow.
struct SpoolDirectory {
    root: File,
    #[cfg(windows)]
    path: PathBuf,
    #[cfg(windows)]
    _ancestors: Vec<File>,
}

impl SpoolDirectory {
    fn open(path: &Path) -> Result<Self, String> {
        if fs::canonicalize(path).map_err(|error| error.to_string())? != path {
            return Err("Storage directory identity changed; refusing artifact access".into());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options
                .share_mode(3)
                .custom_flags(0x0200_0000 | 0x0020_0000);
        }
        #[cfg(windows)]
        let ancestors = {
            let mut handles = Vec::new();
            for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
                if ancestor == path || ancestor.as_os_str().is_empty() {
                    continue;
                }
                let file = options.open(ancestor).map_err(|error| error.to_string())?;
                ensure_spool_directory(&file.metadata().map_err(|error| error.to_string())?)?;
                handles.push(file);
            }
            handles
        };
        let root = options.open(path).map_err(|error| error.to_string())?;
        ensure_spool_directory(&root.metadata().map_err(|error| error.to_string())?)?;
        if fs::canonicalize(path).map_err(|error| error.to_string())? != path {
            return Err("Storage directory was replaced while opening".into());
        }
        Ok(Self {
            root,
            #[cfg(windows)]
            path: path.to_path_buf(),
            #[cfg(windows)]
            _ancestors: ancestors,
        })
    }

    fn sync(&self) -> Result<(), String> {
        ensure_spool_directory(&self.root.metadata().map_err(|error| error.to_string())?)?;
        #[cfg(unix)]
        self.root.sync_all().map_err(|error| error.to_string())?;
        Ok(())
    }

    #[cfg(unix)]
    fn open_file(&self, name: &str, flags: i32) -> Result<File, String> {
        use std::os::fd::{AsRawFd, FromRawFd};
        validate_spool_name(name)?;
        let name = std::ffi::CString::new(name).map_err(|error| error.to_string())?;
        // SAFETY: root is owned and live, the filename is a single validated component,
        // and the returned descriptor is adopted exactly once.
        let fd = unsafe {
            libc::openat(
                self.root.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        // SAFETY: successful openat returned a new owned descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        ensure_regular_metadata(&file.metadata().map_err(|error| error.to_string())?)?;
        Ok(file)
    }

    fn create(&self, name: &str) -> Result<File, String> {
        #[cfg(unix)]
        {
            self.open_file(name, libc::O_RDWR | libc::O_CREAT | libc::O_EXCL)
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use std::os::windows::io::FromRawHandle;
            use windows_sys::Win32::Foundation::{INVALID_HANDLE_VALUE, LocalFree};
            use windows_sys::Win32::Security::{
                Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SECURITY_ATTRIBUTES,
            };
            use windows_sys::Win32::Storage::FileSystem::{
                CREATE_NEW, CreateFileW, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
                FILE_GENERIC_WRITE,
            };
            validate_spool_name(name)?;
            let path: Vec<u16> = self
                .path
                .join(name)
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            // A protected owner-rights-only DACL, not permissions inherited from an
            // arbitrary --data-dir. The owner remains the creating user's token.
            let sddl: Vec<u16> = "D:P(A;;GA;;;OW)".encode_utf16().chain(Some(0)).collect();
            let mut descriptor = std::ptr::null_mut();
            // SAFETY: both strings are NUL-terminated and the descriptor out pointer is valid.
            if unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    1,
                    &mut descriptor,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error().to_string());
            }
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            // SAFETY: attributes and path remain live until CreateFileW returns.
            let handle = unsafe {
                CreateFileW(
                    path.as_ptr(),
                    FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                    7,
                    &attributes,
                    CREATE_NEW,
                    FILE_FLAG_OPEN_REPARSE_POINT,
                    std::ptr::null_mut(),
                )
            };
            let error = io::Error::last_os_error();
            // SAFETY: the conversion routine allocated descriptor with LocalAlloc.
            unsafe {
                LocalFree(descriptor);
            }
            if handle == INVALID_HANDLE_VALUE {
                return Err(error.to_string());
            }
            // SAFETY: the successful file handle is owned exactly once.
            let file = unsafe { File::from_raw_handle(handle) };
            ensure_regular_metadata(&file.metadata().map_err(|error| error.to_string())?)?;
            Ok(file)
        }
    }

    fn read(&self, name: &str, identity: &str) -> Result<File, String> {
        validate_spool_name(name)?;
        #[cfg(unix)]
        let file = self.open_file(name, libc::O_RDONLY)?;
        #[cfg(windows)]
        let file = {
            let mut options = OpenOptions::new();
            options.read(true);
            no_follow(&mut options);
            options
                .open(self.path.join(name))
                .map_err(|error| error.to_string())?
        };
        ensure_regular_metadata(&file.metadata().map_err(|error| error.to_string())?)?;
        if file_identity(&file)? != identity {
            return Err("Output file identity changed; retained evidence is unavailable".into());
        }
        Ok(file)
    }

    fn remove(&self, name: &str, identity: &str) -> Result<(), String> {
        validate_spool_name(name)?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let name = std::ffi::CString::new(name).map_err(|error| error.to_string())?;
            let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: root and name are live; stat points to writable initialized-on-success memory.
            if unsafe {
                libc::fstatat(
                    self.root.as_raw_fd(),
                    name.as_ptr(),
                    stat.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                let error = io::Error::last_os_error();
                return if error.kind() == io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(error.to_string())
                };
            }
            // SAFETY: fstatat succeeded.
            let stat = unsafe { stat.assume_init() };
            // Match MetadataExt::dev's unsigned identity on macOS's signed dev_t.
            #[cfg(target_os = "macos")]
            let device = stat.st_dev as u64;
            #[cfg(not(target_os = "macos"))]
            let device = stat.st_dev;
            if stat.st_mode & libc::S_IFMT != libc::S_IFREG
                || format!("{}:{}", device, stat.st_ino) != identity
            {
                // A link/replacement is not this artifact. In particular never traverse
                // it to remove an external target or destroy another task's reference.
                return Ok(());
            }
            // SAFETY: unlinkat removes only this anchored directory entry, never a
            // link target even if another process swaps it after the metadata check.
            if unsafe { libc::unlinkat(self.root.as_raw_fd(), name.as_ptr(), 0) } != 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::NotFound {
                    return Err(error.to_string());
                }
            }
            self.sync()
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_DISPOSITION_INFO, FileDispositionInfo, SetFileInformationByHandle,
            };
            let mut options = OpenOptions::new();
            options.access_mode(0x0001_0000 | 0x80).share_mode(7);
            no_follow(&mut options);
            let file = match options.open(self.path.join(name)) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.to_string()),
            };
            if ensure_regular_metadata(&file.metadata().map_err(|error| error.to_string())?)
                .is_err()
                || file_identity(&file)? != identity
            {
                return Ok(());
            }
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            // SAFETY: this exact no-follow handle was identity-checked; disposition is
            // live and has the native ABI size. Deletion follows this handle, not a path.
            if unsafe {
                SetFileInformationByHandle(
                    file.as_raw_handle(),
                    FileDispositionInfo,
                    (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                    std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            } == 0
            {
                return Err(io::Error::last_os_error().to_string());
            }
            Ok(())
        }
    }
}

fn validate_spool_name(name: &str) -> Result<(), String> {
    let id = name
        .strip_prefix("ev-run-")
        .and_then(|name| name.strip_suffix(".output"))
        .ok_or("Invalid owned output filename")?;
    if valid_run_id(id) {
        Ok(())
    } else {
        Err("Invalid owned output filename".into())
    }
}

fn ensure_spool_directory(metadata: &fs::Metadata) -> Result<(), String> {
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Artifact directory is not an unlinked directory".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Artifact directory is a Windows reparse point".into());
        }
    }
    Ok(())
}

fn file_identity(file: &File) -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if metadata.nlink() != 1 {
            return Err("Output must have exactly one owned filesystem link".into());
        }
        Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: file owns a live handle; information is writable and ABI-correct.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        if information.nNumberOfLinks != 1 {
            return Err("Output must have exactly one owned filesystem link".into());
        }
        Ok(format!(
            "{}:{}:{}",
            information.dwVolumeSerialNumber, information.nFileIndexHigh, information.nFileIndexLow
        ))
    }
}
fn create_evidence_schema(connection: &Connection) -> Result<(), String> {
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

fn inspect_evidence(connection: &Connection) -> Result<(), String> {
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
         OR EXISTS(SELECT 1 FROM executions GROUP BY task_id HAVING count(*) > 256)
         OR (SELECT count(*) FROM executions WHERE reserved=1) > 1",
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
                    json_column::<crate::context::Excerpt>(row, 2, 512 * 1024)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (id, task_id, excerpt) = row.map_err(|error| error.to_string())?;
            excerpt.validate()?;
            if excerpt.id != id || excerpt.task_id != task_id || id <= 0 {
                return Err("Invalid stored excerpt identity".into());
            }
            if crate::privacy::redact(&excerpt.text) != excerpt.text {
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

fn actionable(error: String) -> String {
    let error = crate::privacy::redact(&error);
    format!(
        "{error}. Retry after resolving the problem, select an isolated data directory, or export readable tasks with --export-data FILE; the existing store is never reset."
    )
}

fn timestamp() -> Result<i64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System clock predates Unix epoch".to_string())?
        .as_millis()
        .try_into()
        .map_err(|_| "Timestamp exceeds SQLite range".into())
}

fn reject_secret_identity(project: &str) -> Result<(), String> {
    if crate::privacy::redact(project).as_ref() != project {
        return Err("Canonical project identity contains recognizable credentials; choose a credential-free path (identity was not rewritten)".into());
    }
    Ok(())
}

fn redact_snapshot(snapshot: &mut Snapshot) {
    fn redact_field(text: &mut String) -> bool {
        match crate::privacy::redact(text) {
            std::borrow::Cow::Owned(clean) if clean != *text => {
                *text = clean;
                true
            }
            _ => false,
        }
    }
    snapshot.redacted |= redact_field(&mut snapshot.goal);
    snapshot.redacted |= redact_field(&mut snapshot.next_action);
    snapshot.redacted |= redact_field(&mut snapshot.command);
    snapshot.redacted |= redact_json(&mut snapshot.memory_draft);
    snapshot.redacted |= redact_json(&mut snapshot.workflow_draft);
    for criterion in &mut snapshot.criteria {
        snapshot.redacted |= redact_field(&mut criterion.text);
    }
    for draft in &mut snapshot.drafts {
        if redact_field(&mut draft.text) {
            snapshot.redacted = true;
            draft.cursor = draft.cursor.min(draft.text.len());
            while !draft.text.is_char_boundary(draft.cursor) {
                draft.cursor -= 1;
            }
        }
    }
}

fn live_bytes(connection: &Connection) -> Result<u64, String> {
    let pages: u64 = connection
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let free: u64 = connection
        .pragma_query_value(None, "freelist_count", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let size: u64 = connection
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    Ok(pages.saturating_sub(free).saturating_mul(size))
}

fn set_quota(connection: &Connection, policy: Policy) -> Result<(), String> {
    let size: u64 = connection
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    // SQLite cannot shrink an existing file here. The live-page check also limits
    // freelist reuse when a store was previously larger than its current quota.
    connection
        .pragma_update(None, "max_page_count", (policy.quota_bytes / size).max(1))
        .map_err(|error| format!("Cannot apply task storage quota: {error}"))
}

fn validate(snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.project.is_empty() {
        return Err("A stored project identity is empty".into());
    }
    reject_secret_identity(&snapshot.project)?;
    if snapshot.archived_at < 0 || (!snapshot.archived && snapshot.archived_at != 0) {
        return Err("Invalid archive timestamp".into());
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
    if snapshot.command.len() > MAX_GOAL_BYTES {
        return Err("A command exceeds the 4096-byte storage limit".into());
    }
    snapshot.workflows.validate(snapshot.id)?;
    validate_draft_shape(&snapshot.memory_draft, 0)?;
    validate_draft_shape(&snapshot.workflow_draft, 0)?;
    check_json_size(&snapshot.workflows, MAX_WORKFLOW_BYTES)?;
    check_json_size(&snapshot.memory_draft, MAX_MEMORY_BYTES)?;
    check_json_size(&snapshot.workflow_draft, MAX_WORKFLOW_BYTES)?;
    check_json_size(&snapshot.approved_memories, MAX_WORKFLOW_BYTES)?;
    if snapshot.approved_memories.len() > crate::memory::MAX_SELECTED {
        return Err("Selected memories exceed their bounded capacity".into());
    }
    for (index, record) in snapshot.approved_memories.iter().enumerate() {
        validate_memory(record)?;
        if !crate::memory::visible(record, &snapshot.project, snapshot.id)
            || snapshot.approved_memories[..index]
                .iter()
                .any(|other| other.id == record.id)
        {
            return Err("Selected memory has duplicate or foreign ownership".into());
        }
    }
    if snapshot.criteria.len() > 64 || snapshot.criteria.iter().any(|item| item.text.len() > 256) {
        return Err("Acceptance criteria exceed 64 entries or 256 bytes per entry".into());
    }
    if snapshot.complete
        && (snapshot.criteria.is_empty() || snapshot.criteria.iter().any(|item| !item.met))
    {
        return Err("Completing a task requires nonempty, all-met acceptance criteria".into());
    }
    if snapshot.drafts.len() > 4 {
        return Err("A task may retain at most four drafts".into());
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
        || nav.criterion_selection >= 64
        || nav.overlay.is_some_and(|overlay| overlay > 7)
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
    Ok(Project {
        tasks,
        active_id,
        usage_bytes: match connection.path().and_then(|path| Path::new(path).parent()) {
            Some(directory) => evidence_usage(connection, directory)?,
            None => live_bytes(connection)?,
        },
    })
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
        drafts: json_column(row, 8, 110_000)?,
        navigation: json_column(row, 9, 1024)?,
        updated_at: row.get(10)?,
        archived: row.get(11)?,
        archived_at: row.get(12)?,
        redacted: row.get(13)?,
        command: row.get(14)?,
        workflows: json_column(row, 15, MAX_WORKFLOW_BYTES)?,
        memory_draft: json_column(row, 16, MAX_MEMORY_BYTES)?,
        approved_memories: json_column(row, 17, MAX_WORKFLOW_BYTES)?,
        workflow_draft: json_column(row, 18, MAX_WORKFLOW_BYTES)?,
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
    Tasks,
    Lifecycle,
    Evidence,
    ProviderJournal,
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
    if !(0..=6).contains(&version) {
        return Err(format!(
            "Unsupported database schema version {version}; this ev supports versions 0 through 6"
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
            inspect_columns(connection, "tasks", &expected)?;
            let columns = if version >= 6 {
                TASK_COLUMNS.to_string()
            } else if version >= 4 {
                format!("{TASK_COLUMNS_V5},{EMPTY_WORKFLOW_COLUMNS}")
            } else if version == 3 {
                format!("{TASK_COLUMNS_V3},'',{EMPTY_WORKFLOW_COLUMNS}")
            } else {
                format!("{TASK_COLUMNS_V2},0,0,0,'',{EMPTY_WORKFLOW_COLUMNS}")
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
    if matches!(schema, Schema::Evidence) {
        connection.execute_batch("CREATE TABLE provider_journal(id TEXT PRIMARY KEY,task_id INTEGER NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB)) <= 65536)); PRAGMA user_version=5;").map_err(|error| format!("Cannot migrate provider journal: {error}"))?;
        return migrate_workflows(connection);
    }
    if matches!(schema, Schema::ProviderJournal) {
        return migrate_workflows(connection);
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
                "SELECT {TASK_COLUMNS_V2},0,0,0,'',{EMPTY_WORKFLOW_COLUMNS} FROM old_tasks ORDER BY id"
            ),
        )),
        Schema::Lifecycle => Some((
            "old_tasks",
            format!(
                "SELECT {TASK_COLUMNS_V3},'',{EMPTY_WORKFLOW_COLUMNS} FROM old_tasks ORDER BY id"
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
                        serde_json::to_string(&task.drafts).map_err(|error| error.to_string())?,
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
    migrate_workflows(connection)
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

/// Read-only, transaction-consistent admission check for a reviewed provider request.
/// This grants no command/file permission and never repairs or migrates storage.
pub fn validate_planning_input(
    data_dir: &Path,
    expected: &Snapshot,
    selected: &[crate::context::Excerpt],
) -> Result<(), String> {
    validate(expected)?;
    if expected.id <= 0 || expected.archived || expected.goal.trim().is_empty() {
        return Err("Planning requires a saved, active task with a goal".into());
    }
    if selected.len() > crate::context::MAX_EXCERPTS {
        return Err("Planning context exceeds the excerpt limit".into());
    }
    let mut connection = open_readonly(&data_dir.join(DATABASE))?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    validate_committed_task(&transaction, expected)?;
    let retained = excerpts(&transaction, expected.id)?;
    for (index, excerpt) in selected.iter().enumerate() {
        excerpt.validate()?;
        if !excerpt.available
            || excerpt.task_id != expected.id
            || excerpt.id <= 0
            || selected[..index].iter().any(|other| other.id == excerpt.id)
            || !retained.iter().any(|stored| stored == excerpt)
        {
            return Err("Selected context changed or was purged; reload and preview again".into());
        }
    }
    Ok(())
}

fn validate_committed_task(connection: &Connection, expected: &Snapshot) -> Result<(), String> {
    let current = load_task(connection, &expected.project, expected.id)?
        .ok_or("The previewed task no longer exists in this project")?;
    if current.revision != expected.revision
        || current.goal != expected.goal
        || current.criteria != expected.criteria
        || current.next_action != expected.next_action
        || current.command != expected.command
        || current.form != expected.form
        || current.complete != expected.complete
        || current.archived != expected.archived
        || current.drafts != expected.drafts
        || current.workflows != expected.workflows
        || current.approved_memories != expected.approved_memories
        || current.memory_draft != expected.memory_draft
        || current.workflow_draft != expected.workflow_draft
    {
        return Err("Task changed since preview; save and review a new request".into());
    }
    if expected.approved_memories.is_empty() {
        return Ok(());
    }
    crate::memory::revalidate(
        &expected.approved_memories,
        &memories(connection, &expected.project, expected.id)?,
        &expected.project,
        expected.id,
    )
}

/// Reads only explicitly linked local observations, not provider claims or spool content.
/// Missing/purged references remain absent so the review reports them as unavailable.
pub fn review_run_observations(
    data_dir: &Path,
    task: &Snapshot,
) -> Result<Vec<crate::runner::RunRecord>, String> {
    validate(task)?;
    if task.id <= 0 || task.archived {
        return Err("Review observations require a committed active task".into());
    }
    let mut connection = open_readonly(&data_dir.join(DATABASE))?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    validate_committed_task(&transaction, task)?;
    let references = task
        .workflows
        .review
        .iter()
        .flat_map(|review| {
            review
                .criteria
                .iter()
                .flat_map(|criterion| &criterion.evidence)
        })
        .chain(
            task.workflows
                .findings
                .iter()
                .flat_map(|finding| &finding.evidence),
        );
    let mut ids = Vec::new();
    for reference in references {
        if let crate::workflows::EvidenceRef::Run { task_id, run_id } = reference {
            if *task_id != task.id {
                return Err("Review run reference belongs to another task".into());
            }
            if !ids.contains(&run_id.as_str()) {
                if ids.len() == 32 {
                    return Err("Review may include at most 32 linked local runs".into());
                }
                ids.push(run_id.as_str());
            }
        }
    }
    let mut records = Vec::with_capacity(ids.len());
    let mut statement = transaction
        .prepare("SELECT record FROM executions WHERE task_id=?1 AND run_id=?2")
        .map_err(|error| error.to_string())?;
    for id in ids {
        let record = statement
            .query_row(params![task.id, id], |row| {
                json_column::<crate::runner::RunRecord>(row, 0, MAX_RUN_BYTES)
            })
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(record) = record {
            run_json(&record)?;
            if record.prepared.task_id != task.id || record.prepared.run_id != id {
                return Err("Stored review observation has conflicting ownership".into());
            }
            let mut sanitized = record.clone();
            sanitized.redact_for_storage();
            if record != sanitized {
                return Err("Stored review observation violates before-storage masking".into());
            }
            records.push(record);
        }
    }
    Ok(records)
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
    let columns = if workflows {
        TASK_COLUMNS.to_string()
    } else if command {
        format!("{TASK_COLUMNS_V5},{EMPTY_WORKFLOW_COLUMNS}")
    } else if modern {
        format!("{TASK_COLUMNS_V3},'',{EMPTY_WORKFLOW_COLUMNS}")
    } else {
        format!("{TASK_COLUMNS_V2},0,0,0,'',{EMPTY_WORKFLOW_COLUMNS}")
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
        Worker::start(directory.0.clone(), project.into(), Policy::default()).unwrap()
    }

    fn usage(directory: &Directory) -> u64 {
        live_bytes(&open_readonly(&directory.0.join(DATABASE)).unwrap()).unwrap()
    }

    fn empty_project(directory: &Directory) -> Project {
        Project {
            tasks: Vec::new(),
            active_id: None,
            usage_bytes: usage(directory),
        }
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

    fn downgrade_workflows(connection: &Connection) {
        connection
            .execute_batch(
                "DROP TABLE memories;
             ALTER TABLE tasks DROP COLUMN workflows;
             ALTER TABLE tasks DROP COLUMN memory_draft;
             ALTER TABLE tasks DROP COLUMN approved_memories;
             ALTER TABLE tasks DROP COLUMN workflow_draft;
             PRAGMA user_version=5;",
            )
            .unwrap();
    }
    #[test]
    fn provider_journal_survives_restart_with_owner_origin_and_explicit_purge() {
        let directory = Directory::new();
        let worker = worker(&directory, "journal-project");
        loaded(&worker);
        let task = create(&worker, Snapshot::empty("journal-project".into()));
        let record = ProviderRecord {
            id: "process-nonce-generation-item".into(),
            agent: crate::provider::Agent::Codex,
            generation: 1,
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "item".into(),
            origin: "Codex provider-native".into(),
            coverage: "Public observed result only; not a local check".into(),
            text: "Final public output".into(),
            complete: true,
        };
        let mut pending = Some(JournalRequest {
            task_id: task.id,
            operation: JournalOperation::Put(record.clone()),
        });
        assert!(worker.try_journal(&mut pending));
        assert!(
            matches!(event(&worker), Event::Journal { result: Ok(JournalResponse::Saved(saved)), .. } if saved == record)
        );
        drop(worker);
        let reopened = super::Worker::start(
            directory.0.clone(),
            "journal-project".into(),
            Policy::default(),
        )
        .unwrap();
        loaded(&reopened);
        let mut pending = Some(JournalRequest {
            task_id: task.id,
            operation: JournalOperation::List,
        });
        assert!(reopened.try_journal(&mut pending));
        assert!(
            matches!(event(&reopened), Event::Journal { result: Ok(JournalResponse::Records(records)), .. } if records == vec![record.clone()])
        );
        let mut changed = record.clone();
        changed.text = "A later process must not overwrite".into();
        let mut pending = Some(JournalRequest {
            task_id: task.id,
            operation: JournalOperation::Put(changed),
        });
        assert!(reopened.try_journal(&mut pending));
        assert!(matches!(
            event(&reopened),
            Event::Journal { result: Err(_), .. }
        ));
        let mut pending = Some(JournalRequest {
            task_id: task.id + 999,
            operation: JournalOperation::List,
        });
        assert!(reopened.try_journal(&mut pending));
        assert!(matches!(
            event(&reopened),
            Event::Journal { result: Err(_), .. }
        ));
        let mut pending = Some(JournalRequest {
            task_id: task.id,
            operation: JournalOperation::Purge,
        });
        assert!(reopened.try_journal(&mut pending));
        assert!(matches!(
            event(&reopened),
            Event::Journal {
                result: Ok(JournalResponse::Purged),
                ..
            }
        ));
        let mut pending = Some(JournalRequest {
            task_id: task.id,
            operation: JournalOperation::List,
        });
        assert!(reopened.try_journal(&mut pending));
        assert!(
            matches!(event(&reopened), Event::Journal { result: Ok(JournalResponse::Records(records)), .. } if records.is_empty())
        );
    }
    #[test]
    fn provider_journal_migrates_v4_and_enforces_quota_without_partial_record() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        downgrade_workflows(&connection);
        connection
            .execute_batch("DROP TABLE provider_journal; PRAGMA user_version=4;")
            .unwrap();
        assert!(matches!(inspect(&connection).unwrap(), Schema::Evidence));
        initialize(&mut connection).unwrap();
        assert!(matches!(inspect(&connection).unwrap(), Schema::Current));
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(Snapshot::empty("project".into())).unwrap();
        let record = ProviderRecord {
            id: "quota-session-item".into(),
            agent: crate::provider::Agent::Codex,
            generation: 1,
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "item".into(),
            origin: "provider-native".into(),
            coverage: "observed only".into(),
            text: "bounded result".into(),
            complete: true,
        };
        store.policy.quota_bytes = 0;
        assert!(
            store
                .journal_request(JournalRequest {
                    task_id: task.id,
                    operation: JournalOperation::Put(record)
                })
                .is_err()
        );
        assert!(
            matches!(store.journal_request(JournalRequest { task_id: task.id, operation: JournalOperation::List }).unwrap(), JournalResponse::Records(records) if records.is_empty())
        );
        store.project = "different-project".into();
        assert!(
            store
                .journal_request(JournalRequest {
                    task_id: task.id,
                    operation: JournalOperation::List
                })
                .is_err()
        );
    }

    #[test]
    fn provider_journal_masks_after_normalization_and_rejects_empty_provenance() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        let record = ProviderRecord {
            id: "normalized-session-item".into(),
            agent: crate::provider::Agent::Codex,
            generation: 1,
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "item".into(),
            origin: "provider-native".into(),
            coverage: "observed only".into(),
            text: "pass\u{0008}word=journal-private-value\nsafe".into(),
            complete: true,
        };
        let response = store
            .journal_request(JournalRequest {
                task_id: task.id,
                operation: JournalOperation::Put(record.clone()),
            })
            .unwrap();
        let JournalResponse::Saved(saved) = response else {
            panic!("expected committed provider record");
        };
        assert!(!saved.text.contains("journal-private-value"));
        assert!(saved.text.contains("[REDACTED]") && saved.text.contains("safe"));
        // Retrying the original observation is idempotent, including its normalization.
        assert!(matches!(
            store.journal_request(JournalRequest {
                task_id: task.id,
                operation: JournalOperation::Put(record.clone()),
            }).unwrap(),
            JournalResponse::Saved(retried) if retried == saved
        ));
        for coverage in ["\u{0008}", "\u{202e}", " \n\t "] {
            let mut invalid = record.clone();
            invalid.id = "invalid-provenance".into();
            invalid.coverage = coverage.into();
            assert!(
                store
                    .journal_request(JournalRequest {
                        task_id: task.id,
                        operation: JournalOperation::Put(invalid),
                    })
                    .is_err()
            );
        }
        let mut invalid = record;
        invalid.id = "misleading\u{202e}identity".into();
        assert!(
            store
                .journal_request(JournalRequest {
                    task_id: task.id,
                    operation: JournalOperation::Put(invalid),
                })
                .is_err()
        );
        drop(store);
        let mut reopened = state(&directory, "project", Policy::default());
        assert!(matches!(
            reopened.journal_request(JournalRequest {
                task_id: task.id,
                operation: JournalOperation::List,
            }).unwrap(),
            JournalResponse::Records(records) if records == vec![saved.clone()]
        ));
        let bytes = fs::read(directory.0.join(DATABASE)).unwrap();
        assert!(
            !bytes
                .windows(b"journal-private-value".len())
                .any(|window| window == b"journal-private-value")
        );
        // An observation written before the masking fix remains append-only on disk,
        // but loading it must apply the current privacy boundary.
        let mut legacy = saved.clone();
        legacy.text = "password=legacy-journal-value".into();
        let json = serde_json::to_string(&legacy).unwrap();
        reopened
            .connection
            .as_ref()
            .unwrap()
            .execute(
                "UPDATE provider_journal SET record=?1 WHERE id=?2",
                params![json, legacy.id],
            )
            .unwrap();
        let JournalResponse::Records(records) = reopened
            .journal_request(JournalRequest {
                task_id: task.id,
                operation: JournalOperation::List,
            })
            .unwrap()
        else {
            panic!("expected provider journal records");
        };
        assert!(!records[0].text.contains("legacy-journal-value"));
        let retained: String = reopened
            .connection
            .as_ref()
            .unwrap()
            .query_row(
                "SELECT record FROM provider_journal WHERE id=?1",
                [&legacy.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            retained, json,
            "a journal read must not rewrite retained evidence"
        );
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
            assert_eq!(loaded(&worker), empty_project(&directory));
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
                    usage_bytes: usage(&directory),
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
        assert_eq!(loaded(&first), empty_project(&directory));
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
        assert_eq!(loaded(&second), empty_project(&directory));
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
        let first = Worker::start(alias.clone(), "project".into(), Policy::default()).unwrap();
        assert_eq!(loaded(&first), empty_project(&directory));
        let saved = create(&first, snapshot("project", 1));
        let second = worker(&directory, "project");
        assert!(
            matches!(event(&second), Event::Failed { revision: None, message } if message.contains("writer lock"))
        );
        let readonly = open_readonly(&alias.join(DATABASE)).unwrap();
        let expected = Project {
            active_id: Some(saved.id),
            tasks: vec![saved],
            usage_bytes: usage(&directory),
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
                6
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
        assert_eq!(loaded(&worker), empty_project(&directory));
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
        assert_eq!(loaded(&worker), empty_project(&directory));
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
        assert_eq!(loaded(&store), empty_project(&directory));
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
        assert_eq!(loaded(&first), empty_project(&directory));
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
        assert_eq!(loaded(&worker), empty_project(&directory));
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
                drafts: vec![duplicate; 5],
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
                    criterion_selection: 64,
                    ..Navigation::default()
                },
                ..snapshot("project", 1)
            },
            Snapshot {
                navigation: Navigation {
                    overlay: Some(8),
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
        assert_eq!(loaded(&worker), empty_project(&directory));
    }

    #[test]
    fn failed_revision_does_not_hide_a_later_accepted_save() {
        let directory = Directory::new();
        let worker = worker(&directory, "project");
        assert_eq!(loaded(&worker), empty_project(&directory));
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
        assert_eq!(loaded(&store), empty_project(&directory));
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
            policy: Policy::default(),
            connection: None,
            lock: None,
        };
        assert!(
            matches!(state.reopen(), Event::Loaded(project) if project == empty_project(&directory))
        );
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
            empty_project(&directory)
        );
        drop(state);
        let reopened = worker(&directory, "project");
        assert_eq!(loaded(&reopened), empty_project(&directory));
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
                6
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

    #[cfg(unix)]
    #[test]
    fn closed_store_releases_writer_lock_despite_inherited_descriptor() {
        let directory = Directory::new();
        let store = state(&directory, "project", Policy::default());
        let inherited = store.lock.as_ref().unwrap().try_clone().unwrap();
        drop(store);
        let mut reopened = state(&directory, "project", Policy::default());
        let saved = reopened.save(snapshot("project", 1)).unwrap();
        assert_eq!(
            load(reopened.connection.as_ref().unwrap(), "project")
                .unwrap()
                .tasks,
            vec![saved]
        );
        drop(inherited);
    }

    #[test]
    fn two_tasks_preserve_drafts_navigation_selection_and_timestamps_across_restart() {
        let directory = Directory::new();
        let store = worker(&directory, "project");
        assert_eq!(loaded(&store), empty_project(&directory));
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
                usage_bytes: usage(&directory),
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
                usage_bytes: usage(&directory),
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
                usage_bytes: usage(&directory),
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
                usage_bytes: usage(&directory),
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

    fn state(directory: &Directory, project: &str, policy: Policy) -> State {
        let mut state = State {
            data_dir: directory.0.clone(),
            project: project.into(),
            policy,
            connection: None,
            lock: None,
        };
        assert!(matches!(state.reopen(), Event::Loaded(_)));
        state
    }

    #[test]
    fn management_queue_retains_ownership_on_backpressure_and_disconnect() {
        let (commands, incoming) = mpsc::sync_channel(1);
        let mut worker = Worker {
            commands: Some(commands),
            events: None,
            thread: None,
        };
        let request = Management::Delete {
            ids: vec![17],
            expired_before: None,
        };
        let mut pending = Some(request.clone());
        assert!(worker.try_manage(&mut pending));
        assert!(pending.is_none());
        assert!(!worker.try_manage(&mut pending));
        pending = Some(request.clone());
        assert!(!worker.try_manage(&mut pending));
        assert_eq!(pending, Some(request.clone()));
        drop(incoming);
        assert!(!worker.try_manage(&mut pending));
        assert_eq!(pending, Some(request.clone()));
        worker.commands.take();
        assert!(!worker.try_manage(&mut pending));
        assert_eq!(pending, Some(request));
    }

    #[test]
    fn archive_restore_delete_are_scoped_atomic_and_survive_restart() {
        let directory = Directory::new();
        let mut other = state(&directory, "other", Policy::default());
        let foreign = other.save(snapshot("other", 1)).unwrap();
        drop(other);
        let mut store = state(&directory, "project", Policy::default());
        let first = store.save(snapshot("project", 1)).unwrap();
        let second = store.save(snapshot("project", 1)).unwrap();
        let Managed {
            project: archived,
            affected,
            warning,
        } = store
            .manage_at(
                Management::Archive {
                    id: second.id,
                    archived: true,
                },
                100,
            )
            .unwrap();
        assert!(warning.is_none());
        assert_eq!(affected, vec![second.id]);
        assert_eq!(archived.active_id, Some(first.id));
        assert!(archived.tasks[1].archived && !archived.tasks[1].complete);
        assert_eq!(archived.tasks[1].archived_at, 100);
        assert!(store.select(second.id).is_err());
        let export = directory.0.join("external-export.json");
        export_readable(&directory.0, &export).unwrap();
        let exported: Vec<Snapshot> = serde_json::from_slice(&fs::read(&export).unwrap()).unwrap();
        assert!(
            exported
                .iter()
                .any(|task| task.id == second.id && task.archived)
        );
        let Managed {
            project: restored, ..
        } = store
            .manage(Management::Archive {
                id: second.id,
                archived: false,
            })
            .unwrap();
        assert_eq!(restored.active_id, Some(first.id));
        assert!(!restored.tasks[1].archived);
        assert_eq!(restored.tasks[1].archived_at, 0);
        for ids in [
            vec![first.id, foreign.id],
            vec![first.id, 99999],
            vec![first.id, first.id],
        ] {
            assert!(
                store
                    .manage(Management::Delete {
                        ids,
                        expired_before: None
                    })
                    .is_err()
            );
            assert_eq!(
                load(store.connection.as_ref().unwrap(), "project")
                    .unwrap()
                    .tasks
                    .len(),
                2
            );
        }
        let Managed {
            project: remaining, ..
        } = store
            .manage(Management::Delete {
                ids: vec![first.id],
                expired_before: None,
            })
            .unwrap();
        assert_eq!(remaining.active_id, Some(second.id));
        assert_eq!(remaining.tasks.len(), 1);
        assert!(export.exists());
        assert_eq!(
            load(store.connection.as_ref().unwrap(), "other")
                .unwrap()
                .tasks,
            vec![foreign]
        );
        store
            .manage(Management::Archive {
                id: second.id,
                archived: true,
            })
            .unwrap();
        drop(store);
        let mut store = state(&directory, "project", Policy::default());
        assert_eq!(
            load(store.connection.as_ref().unwrap(), "project")
                .unwrap()
                .active_id,
            None
        );
        let Managed {
            project: restored, ..
        } = store
            .manage(Management::Archive {
                id: second.id,
                archived: false,
            })
            .unwrap();
        assert_eq!(restored.active_id, None);
        store
            .manage(Management::Delete {
                ids: vec![second.id],
                expired_before: None,
            })
            .unwrap();
        let third = store.save(snapshot("project", 1)).unwrap();
        assert!(
            third.id > second.id,
            "deleted IDs must not be reused by a stale deletion preview"
        );
        assert!(
            store
                .manage(Management::Delete {
                    ids: vec![second.id],
                    expired_before: None
                })
                .is_err()
        );
    }

    #[test]
    fn retention_revalidates_archive_age_policy_and_every_target() {
        let directory = Directory::new();
        let policy = Policy {
            retention_days: 1,
            ..Policy::default()
        };
        let mut store = state(&directory, "project", policy);
        let first = store.save(snapshot("project", 1)).unwrap();
        let second = store.save(snapshot("project", 1)).unwrap();
        store
            .manage_at(
                Management::Archive {
                    id: first.id,
                    archived: true,
                },
                1_000,
            )
            .unwrap();
        store
            .manage_at(
                Management::Archive {
                    id: second.id,
                    archived: true,
                },
                2_000,
            )
            .unwrap();
        let now = 86_400_000 + 3_000;
        let request = Management::Delete {
            ids: vec![first.id, second.id],
            expired_before: Some(3_000),
        };
        store
            .manage_at(
                Management::Archive {
                    id: second.id,
                    archived: false,
                },
                now,
            )
            .unwrap();
        assert!(store.manage_at(request.clone(), now).is_err());
        store
            .manage_at(
                Management::Archive {
                    id: second.id,
                    archived: true,
                },
                now,
            )
            .unwrap();
        assert!(store.manage_at(request, now).is_err());
        assert!(
            store
                .manage_at(
                    Management::Delete {
                        ids: vec![first.id],
                        expired_before: Some(3_001)
                    },
                    now
                )
                .is_err()
        );
        assert_eq!(
            load(store.connection.as_ref().unwrap(), "project")
                .unwrap()
                .tasks
                .len(),
            2
        );
        let Managed {
            project: remaining,
            affected,
            warning,
        } = store
            .manage_at(
                Management::Delete {
                    ids: vec![first.id],
                    expired_before: Some(3_000),
                },
                now,
            )
            .unwrap();
        assert!(warning.is_none());
        assert_eq!(affected, vec![first.id]);
        assert_eq!(remaining.tasks.len(), 1);
        assert_eq!(remaining.tasks[0].id, second.id);
    }

    #[test]
    fn secrets_are_redacted_before_sqlite_and_export_and_history_is_sticky() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let raw = "前 password=super-secret 後";
        let task = Snapshot {
            goal: "sk-secret123456".into(),
            next_action: "Bearer bearer-value".into(),
            criteria: vec![Criterion {
                text: "https://user:pass@example.test".into(),
                met: false,
            }],
            drafts: vec![Draft {
                field: EditorField::Goal,
                text: raw.into(),
                cursor: raw.len() - 3,
            }],
            ..snapshot("project", 1)
        };
        let mut saved = store.save(task).unwrap();
        assert!(saved.redacted);
        assert!(
            saved.drafts[0]
                .text
                .is_char_boundary(saved.drafts[0].cursor)
        );
        let connection = store.connection.as_ref().unwrap();
        let raw: String = connection
            .query_row(
                "SELECT goal||criteria||next_action||drafts FROM tasks",
                [],
                |row| row.get(0),
            )
            .unwrap();
        for secret in ["secret123456", "bearer-value", "user:pass", "super-secret"] {
            assert!(!raw.contains(secret));
        }
        saved.revision += 1;
        saved.goal = "ordinary goal".into();
        saved.redacted = false;
        assert!(store.save(saved).unwrap().redacted);
        let output = directory.0.join("export.json");
        export_readable(&directory.0, &output).unwrap();
        let text = fs::read_to_string(output).unwrap();
        assert!(!text.contains("super-secret"));
        assert!(text.contains("\"redacted\": true"));
    }

    #[test]
    fn migrations_redact_all_legacy_payloads_and_exports_without_rewriting_identities() {
        for version in [0, 1, 2] {
            let directory = Directory::new();
            let path = directory.0.join(DATABASE);
            let connection = Connection::open(&path).unwrap();
            if version < 2 {
                legacy(&connection, version);
                let table = if version == 0 { "goals" } else { "projects" };
                connection
                    .execute(
                        &format!("UPDATE {table} SET goal='password=legacy-secret'"),
                        [],
                    )
                    .unwrap();
            } else {
                connection.execute_batch(
                    "CREATE TABLE tasks(id INTEGER PRIMARY KEY,project TEXT NOT NULL,goal TEXT NOT NULL,form INTEGER NOT NULL,revision INTEGER NOT NULL,criteria TEXT NOT NULL,next_action TEXT NOT NULL,complete INTEGER NOT NULL,drafts TEXT NOT NULL,navigation TEXT NOT NULL,updated_at INTEGER NOT NULL);
                     CREATE TABLE selections(project TEXT PRIMARY KEY,active_id INTEGER NOT NULL);
                     PRAGMA user_version=2;"
                ).unwrap();
                connection.execute("INSERT INTO tasks VALUES (7,'project','password=legacy-secret',0,1,?1,'Bearer legacy-bearer',0,?2,?3,0)",
                    params![r#"[{"text":"token=legacy-token","met":false}]"#, r#"[{"field":"Goal","text":"ghp_legacysecret","cursor":15}]"#, serde_json::to_string(&Navigation::default()).unwrap()]).unwrap();
                connection
                    .execute("INSERT INTO selections VALUES ('project',7)", [])
                    .unwrap();
            }
            drop(connection);
            let before = fs::read(&path).unwrap();
            let output = directory.0.join("legacy-export.json");
            export_readable(&directory.0, &output).unwrap();
            assert_eq!(fs::read(&path).unwrap(), before);
            assert!(
                !fs::read_to_string(output)
                    .unwrap()
                    .contains("legacy-secret")
            );
            {
                let mut connection = Connection::open(&path).unwrap();
                {
                    let transaction = connection
                        .transaction_with_behavior(TransactionBehavior::Immediate)
                        .unwrap();
                    migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
                    // Dropping the transaction models interruption before commit.
                }
                assert_eq!(
                    connection
                        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                        .unwrap(),
                    i64::from(version)
                );
                let pages: i64 = connection
                    .pragma_query_value(None, "page_count", |row| row.get(0))
                    .unwrap();
                connection
                    .pragma_update(None, "max_page_count", pages)
                    .unwrap();
                assert!(initialize(&mut connection).is_err());
                assert_eq!(
                    connection
                        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                        .unwrap(),
                    i64::from(version)
                );
            }
            let store = state(&directory, "project", Policy::default());
            let connection = store.connection.as_ref().unwrap();
            let raw: String = connection
                .query_row(
                    "SELECT goal||criteria||next_action||drafts FROM tasks WHERE project='project'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            for secret in [
                "legacy-secret",
                "legacy-bearer",
                "legacy-token",
                "legacysecret",
            ] {
                assert!(!raw.contains(secret));
            }
            assert!(load(connection, "project").unwrap().tasks[0].redacted);
        }
        let directory = Directory::new();
        let secret = "/project/ghp_secret123456";
        let worker = worker(&directory, secret);
        assert!(
            matches!(event(&worker), Event::Failed { message, .. } if !message.contains(secret) && message.contains("identity"))
        );
        assert!(!directory.0.join(DATABASE).exists());
    }

    #[test]
    fn secret_bearing_legacy_identity_is_refused_without_rewriting_or_merging() {
        let directory = Directory::new();
        let path = directory.0.join(DATABASE);
        let connection = Connection::open(&path).unwrap();
        legacy(&connection, 1);
        connection.execute("UPDATE projects SET project='/projects/ghp_identitysecret' WHERE project='project'", []).unwrap();
        drop(connection);
        let before = fs::read(&path).unwrap();
        let store = worker(&directory, "ordinary-project");
        assert!(
            matches!(event(&store), Event::Failed { message, .. } if message.contains("identity") && !message.contains("ghp_identitysecret"))
        );
        drop(store);
        assert_eq!(fs::read(&path).unwrap(), before);
        let output = directory.0.join("refused.json");
        assert!(export_readable(&directory.0, &output).is_err());
        assert!(!output.exists());
    }

    #[test]
    fn management_events_follow_commit_and_failures_preserve_records() {
        let directory = Directory::new();
        let store = worker(&directory, "project");
        loaded(&store);
        let task = create(&store, snapshot("project", 1));
        let mut request = Some(Management::Archive {
            id: task.id,
            archived: true,
        });
        assert!(store.try_manage(&mut request));
        assert!(request.is_none());
        let managed = match event(&store) {
            Event::Managed {
                project,
                affected,
                warning,
            } => {
                assert!(warning.is_none());
                assert_eq!(affected, vec![task.id]);
                project
            }
            other => panic!("expected committed management: {other:?}"),
        };
        let readonly = open_readonly(&directory.0.join(DATABASE)).unwrap();
        assert_eq!(load(&readonly, "project").unwrap(), managed);
        let mut request = Some(Management::Delete {
            ids: vec![task.id, 99999],
            expired_before: None,
        });
        assert!(store.try_manage(&mut request));
        assert!(matches!(event(&store), Event::ManagementFailed(_)));
        assert_eq!(load(&readonly, "project").unwrap(), managed);
    }

    #[test]
    fn real_quota_rolls_back_and_deletion_recovers_without_eviction() {
        let directory = Directory::new();
        let initial = state(&directory, "project", Policy::default());
        let connection = initial.connection.as_ref().unwrap();
        let schema_bytes = live_bytes(connection).unwrap();
        let page_bytes: u64 = connection
            .pragma_query_value(None, "page_size", |row| row.get(0))
            .unwrap();
        drop(initial);
        let policy = Policy {
            quota_bytes: schema_bytes + 4 * page_bytes,
            ..Policy::default()
        };
        let mut store = state(&directory, "project", policy);
        let mut committed = Vec::new();
        let large = Snapshot {
            goal: "q".repeat(4096),
            ..snapshot("project", 1)
        };
        loop {
            match store.save(large.clone()) {
                Ok(saved) => committed.push(saved),
                Err(error) => {
                    assert!(error.contains("full") || error.contains("quota"), "{error}");
                    break;
                }
            }
            assert!(
                committed.len() < 64,
                "a bounded SQLite quota must stop actual payload growth"
            );
        }
        assert!(!committed.is_empty());
        let connection = store.connection.as_ref().unwrap();
        assert_eq!(load(connection, "project").unwrap().tasks, committed);
        assert!(live_bytes(connection).unwrap() <= policy.quota_bytes);
        let max: u64 = connection
            .pragma_query_value(None, "max_page_count", |row| row.get(0))
            .unwrap();
        let size: u64 = connection
            .pragma_query_value(None, "page_size", |row| row.get(0))
            .unwrap();
        assert!(max * size <= policy.quota_bytes);
        let deleted = committed.remove(0);
        store
            .manage(Management::Delete {
                ids: vec![deleted.id],
                expired_before: None,
            })
            .unwrap();
        let retry = store.save(large).unwrap();
        committed.push(retry);
        assert_eq!(
            load(store.connection.as_ref().unwrap(), "project")
                .unwrap()
                .tasks,
            committed
        );
        drop(store);
        let small = Policy {
            quota_bytes: schema_bytes + page_bytes,
            ..policy
        };
        let mut store = state(&directory, "project", small);
        assert_eq!(
            load(store.connection.as_ref().unwrap(), "project")
                .unwrap()
                .tasks,
            committed
        );
        assert!(
            store.save(snapshot("project", 1)).is_err(),
            "freelist reuse must still obey the smaller live-page quota"
        );
        store
            .manage(Management::Delete {
                ids: committed.iter().map(|task| task.id).collect(),
                expired_before: None,
            })
            .unwrap();
        assert!(
            store.save(snapshot("project", 1)).is_ok(),
            "deletion must recover capacity even without shrinking the physical SQLite file"
        );
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
                command: text.clone(),
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
                    EditorField::Command,
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
    fn captured_excerpt(task_id: i64, source: &Path, text: &str) -> crate::context::Excerpt {
        use sha2::{Digest, Sha256};
        crate::context::Excerpt {
            id: 0,
            task_id,
            root: source.parent().unwrap().to_str().unwrap().into(),
            path: source.file_name().unwrap().to_str().unwrap().into(),
            start_line: 1,
            end_line: 1,
            source_sha256: Sha256::digest(text.as_bytes()).into(),
            text: text.into(),
            available: true,
        }
    }

    fn prepared_run(task_id: i64, root: &Path, suffix: &str) -> crate::runner::Prepared {
        crate::runner::Prepared {
            request_id: format!("request-{suffix}"),
            run_id: format!("run-{suffix}"),
            task_id,
            spec: crate::runner::CommandSpec::Direct {
                program: "echo".into(),
                args: vec!["password=journal-value".into()],
            },
            cwd: root.to_path_buf(),
            cwd_identity: "fixture-directory".into(),
            executable: crate::runner::ExecutableIdentity {
                path: root.join("fixture-executable"),
                sha256: "0".repeat(64),
                bytes: 1,
                modified_ns: 1,
                file_id: "fixture-file".into(),
            },
            prepared_at_ms: timestamp().unwrap() as u64,
            expires_at_ms: timestamp().unwrap() as u64 + 60_000,
            before: crate::runner::InputSnapshot::default(),
        }
    }

    #[test]
    fn context_is_independent_masked_task_owned_and_purge_preserves_unavailable_metadata() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let first = store.save(snapshot("project", 1)).unwrap();
        let second = store.save(snapshot("project", 1)).unwrap();
        let source = directory.0.join("source.rs");
        fs::write(&source, "password=source-value\n").unwrap();
        let pin = |task_id| ContextRequest {
            task_id,
            operation: ContextOperation::Pin(captured_excerpt(
                task_id,
                &source,
                "password=source-value\n",
            )),
        };
        let one = store.context(pin(first.id)).unwrap();
        let two = store.context(pin(second.id)).unwrap();
        assert_eq!(one[0].text, "password=[REDACTED]\n");
        let connection = store.connection.as_ref().unwrap();
        let raw: String = connection
            .query_row(
                "SELECT record FROM excerpts WHERE task_id=?1",
                [first.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!raw.contains("source-value"));
        let mut updated = first.clone();
        updated.revision += 1;
        updated.goal = "changed without touching source payload".into();
        store.save(updated).unwrap();
        assert_eq!(
            store
                .context(ContextRequest {
                    task_id: first.id,
                    operation: ContextOperation::Load
                })
                .unwrap(),
            one
        );
        assert!(
            store
                .context(ContextRequest {
                    task_id: second.id,
                    operation: ContextOperation::Remove(one[0].id)
                })
                .is_err()
        );
        let purged = store
            .context(ContextRequest {
                task_id: first.id,
                operation: ContextOperation::Purge(vec![one[0].id]),
            })
            .unwrap();
        assert!(!purged[0].available && purged[0].text.is_empty());
        assert!(
            store
                .context(ContextRequest {
                    task_id: first.id,
                    operation: ContextOperation::Validate(one)
                })
                .is_err()
        );
        assert_eq!(
            store
                .context(ContextRequest {
                    task_id: second.id,
                    operation: ContextOperation::Load
                })
                .unwrap(),
            two
        );
        assert_eq!(
            fs::read_to_string(&source).unwrap(),
            "password=source-value\n"
        );
        store
            .manage(Management::Delete {
                ids: vec![first.id],
                expired_before: None,
            })
            .unwrap();
        assert_eq!(
            store
                .context(ContextRequest {
                    task_id: second.id,
                    operation: ContextOperation::Load
                })
                .unwrap(),
            two
        );
    }

    #[test]
    fn reservations_commit_before_permit_mask_journal_and_refuse_active_deletion_and_purge() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        let prepared = prepared_run(task.id, &store.data_dir, "active");
        let mut permit = store.reserve_run(task.id, prepared.clone(), 4096).unwrap();
        let connection = open_readonly(&store.data_dir.join(DATABASE)).unwrap();
        let stored = stored_run(&connection, task.id, &prepared.run_id).unwrap();
        assert!(stored.reserved);
        assert_eq!(
            stored.record.state,
            crate::runner::RunState::AwaitingDispatch
        );
        assert!(stored.record.started_at_ms.is_none());
        assert!(
            !serde_json::to_string(&stored.record)
                .unwrap()
                .contains("journal-value")
        );
        assert_eq!(permit.capacity, 4096);
        assert!(
            store
                .manage(Management::Delete {
                    ids: vec![task.id],
                    expired_before: None
                })
                .is_err()
        );
        assert!(
            store
                .manage(Management::Archive {
                    id: task.id,
                    archived: true
                })
                .is_err()
        );
        assert!(store.purge_output(task.id, &prepared.run_id).is_err());
        assert!(
            store
                .reserve_run(
                    task.id,
                    prepared_run(task.id, &store.data_dir, "second"),
                    1024
                )
                .is_err()
        );
        permit.file.write_all(b"partial output\n").unwrap();
        permit.file.sync_all().unwrap();
        let mut record = crate::runner::RunRecord::awaiting(&prepared);
        record.state = crate::runner::RunState::Running;
        record.started_at_ms = Some(timestamp().unwrap() as u64);
        store.update_run(task.id, record.clone()).unwrap();
        record.state = crate::runner::RunState::Succeeded;
        record.ended_at_ms = Some(timestamp().unwrap() as u64);
        record.exit_code = Some(0);
        record.tree = crate::runner::TreeOutcome::OwnedTreeExited;
        record.captured_bytes = 15;
        drop(permit);
        store.update_run(task.id, record.clone()).unwrap();
        assert!(
            store.update_run(task.id, record).is_err(),
            "late duplicate finish must not rewrite history"
        );
        let output = store
            .read_output(task.id, &prepared.run_id, 0, 4096)
            .unwrap();
        assert_eq!(output.text, "partial output\n");
        assert!(!output.unavailable);
        let (purged, warning) = store.purge_output(task.id, &prepared.run_id).unwrap();
        assert!(warning.is_none());
        assert!(purged.output_unavailable);
        assert!(
            store
                .read_output(task.id, &prepared.run_id, 0, 4096)
                .unwrap()
                .unavailable
        );
    }

    #[test]
    fn committed_purge_and_deletion_return_metadata_when_artifact_cleanup_fails() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        let survivor = store.save(snapshot("project", 1)).unwrap();
        store.select(task.id).unwrap();
        let prepared = prepared_run(task.id, &store.data_dir, "cleanup-failure");
        let mut permit = store.reserve_run(task.id, prepared.clone(), 1024).unwrap();
        permit.file.write_all(b"retained\n").unwrap();
        permit.file.sync_all().unwrap();
        drop(permit);
        let mut record = crate::runner::RunRecord::awaiting(&prepared);
        record.state = crate::runner::RunState::Running;
        record.started_at_ms = Some(timestamp().unwrap() as u64);
        store.update_run(task.id, record.clone()).unwrap();
        record.state = crate::runner::RunState::Succeeded;
        record.ended_at_ms = Some(timestamp().unwrap() as u64);
        record.exit_code = Some(0);
        record.tree = crate::runner::TreeOutcome::OwnedTreeExited;
        record.captured_bytes = 9;
        let record = store.update_run(task.id, record).unwrap();
        assert!(!record.output_unavailable);

        let data_dir = store.data_dir.clone();
        let output = data_dir.join(spool_name(&prepared.run_id).unwrap());
        // Keep SQLite available but make artifact-directory access fail deterministically,
        // without relying on filesystem permissions or the test runner's privileges.
        let blocked_directory = data_dir.join("not-a-directory");
        fs::write(&blocked_directory, b"preserve").unwrap();
        store.data_dir = blocked_directory.clone();
        let response = store
            .run_request(RunRequest {
                task_id: task.id,
                operation: RunOperation::PurgeOutput {
                    run_id: prepared.run_id.clone(),
                },
            })
            .unwrap();
        let RunResponse::Purged {
            record: purged,
            warning,
        } = response
        else {
            panic!("expected committed purge response");
        };
        assert!(warning.is_some());
        assert!(purged.output_unavailable);
        assert_eq!(purged.state, crate::runner::RunState::Succeeded);
        assert_eq!(purged.prepared.run_id, prepared.run_id);
        let readonly = open_readonly(&data_dir.join(DATABASE)).unwrap();
        assert!(
            stored_run(&readonly, task.id, &prepared.run_id)
                .unwrap()
                .record
                .output_unavailable
        );
        assert_eq!(fs::read(&output).unwrap(), b"retained\n");
        store.data_dir = data_dir.clone();
        assert!(
            store
                .read_output(task.id, &prepared.run_id, 0, 64)
                .unwrap()
                .unavailable
        );

        store.data_dir = blocked_directory;
        let Managed {
            project,
            affected,
            warning,
        } = store
            .manage(Management::Delete {
                ids: vec![task.id],
                expired_before: None,
            })
            .unwrap();
        assert!(warning.is_some());
        assert_eq!(affected, vec![task.id]);
        assert_eq!(project.tasks, vec![survivor.clone()]);
        assert_eq!(project.active_id, Some(survivor.id));
        assert_eq!(load(&readonly, "project").unwrap(), project);
        assert!(stored_run(&readonly, task.id, &prepared.run_id).is_err());
        assert_eq!(fs::read(&output).unwrap(), b"retained\n");
        drop(readonly);
        store.data_dir = data_dir;
        drop(store);
        let store = state(&directory, "project", Policy::default());
        let reopened = load(store.connection.as_ref().unwrap(), "project").unwrap();
        assert_eq!(reopened.tasks, vec![survivor]);
        assert_eq!(reopened.active_id, project.active_id);
    }

    #[test]
    fn quota_accounts_live_sqlite_retained_spools_and_reserved_capacity_without_eviction() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        let initial = live_bytes(store.connection.as_ref().unwrap()).unwrap();
        store.policy.quota_bytes = initial + RUN_HEADROOM + 4096;
        let before = fs::read(store.data_dir.join(DATABASE)).unwrap();
        assert!(
            store
                .reserve_run(
                    task.id,
                    prepared_run(task.id, &store.data_dir, "quota"),
                    8192
                )
                .is_err()
        );
        assert!(store.load_runs(task.id).unwrap().is_empty());
        assert_eq!(
            load_task(store.connection.as_ref().unwrap(), "project", task.id).unwrap(),
            Some(task.clone())
        );
        assert_eq!(fs::read(store.data_dir.join(DATABASE)).unwrap(), before);
        store.policy = Policy::default();
        let permit = store
            .reserve_run(
                task.id,
                prepared_run(task.id, &store.data_dir, "reserved"),
                8192,
            )
            .unwrap();
        let used = evidence_usage(store.connection.as_ref().unwrap(), &store.data_dir).unwrap();
        assert!(used >= initial + 8192 + RUN_HEADROOM);
        store.policy.quota_bytes = used;
        let large = Snapshot {
            goal: "x".repeat(4096),
            ..snapshot("project", 1)
        };
        assert!(store.save(large).is_err());
        assert!(
            store.load_runs(task.id).unwrap()[0].state == crate::runner::RunState::AwaitingDispatch
        );
        drop(permit);
    }

    #[test]
    fn restart_interrupts_reservations_and_retains_partial_bytes_without_claiming_running() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        let prepared = prepared_run(task.id, &store.data_dir, "restart");
        let mut permit = store.reserve_run(task.id, prepared.clone(), 4096).unwrap();
        let mut record = crate::runner::RunRecord::awaiting(&prepared);
        record.state = crate::runner::RunState::Running;
        record.started_at_ms = Some(timestamp().unwrap() as u64);
        store.update_run(task.id, record).unwrap();
        permit.file.write_all(b"retained\n").unwrap();
        permit.file.sync_all().unwrap();
        assert!(
            matches!(store.reopen(), Event::Failed { .. }),
            "retry cannot abandon live ownership"
        );
        drop(permit);
        drop(store);
        let mut store = state(&directory, "project", Policy::default());
        let records = store.load_runs(task.id).unwrap();
        assert_eq!(records[0].state, crate::runner::RunState::Interrupted);
        assert_eq!(
            records[0].tree,
            crate::runner::TreeOutcome::OwnershipUnknown
        );
        assert!(records[0].truncated);
        assert_eq!(
            store
                .read_output(task.id, &prepared.run_id, 0, 4096)
                .unwrap()
                .text,
            "retained\n"
        );
        assert!(
            store
                .reserve_run(
                    task.id,
                    prepared_run(task.id, &store.data_dir, "after-restart"),
                    1024
                )
                .is_ok()
        );
    }

    #[cfg(unix)]
    #[test]
    fn replaced_output_links_are_unavailable_and_purge_never_deletes_external_files() {
        use std::os::unix::fs::symlink;
        let directory = Directory::new();
        let external = Directory::new();
        let outside = external.0.join("preserve.txt");
        fs::write(&outside, "external bytes").unwrap();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        let prepared = prepared_run(task.id, &store.data_dir, "swap");
        let permit = store.reserve_run(task.id, prepared.clone(), 1024).unwrap();
        drop(permit);
        store
            .run_request(RunRequest {
                task_id: task.id,
                operation: RunOperation::Abandon {
                    request_id: prepared.request_id.clone(),
                    run_id: prepared.run_id.clone(),
                },
            })
            .unwrap();
        let output = store.data_dir.join(spool_name(&prepared.run_id).unwrap());
        fs::remove_file(&output).unwrap();
        symlink(&outside, &output).unwrap();
        assert!(
            store
                .read_output(task.id, &prepared.run_id, 0, 64)
                .unwrap()
                .unavailable
        );
        store.purge_output(task.id, &prepared.run_id).unwrap();
        assert_eq!(fs::read_to_string(&outside).unwrap(), "external bytes");
        store
            .manage(Management::Delete {
                ids: vec![task.id],
                expired_before: None,
            })
            .unwrap();
        assert_eq!(fs::read_to_string(&outside).unwrap(), "external bytes");
    }

    #[test]
    fn schema_three_migration_is_transactional_and_preserves_archives_and_four_command_drafts() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let first = store.save(snapshot("project", 1)).unwrap();
        let second = store.save(snapshot("project", 1)).unwrap();
        store
            .manage(Management::Archive {
                id: first.id,
                archived: true,
            })
            .unwrap();
        drop(store);
        let path = directory.0.join(DATABASE);
        let mut connection = Connection::open(&path).unwrap();
        downgrade_workflows(&connection);
        connection.execute_batch("DROP TABLE provider_journal; DROP TABLE excerpts; DROP TABLE executions; ALTER TABLE tasks DROP COLUMN command; PRAGMA user_version=3;").unwrap();
        {
            let transaction = connection.transaction().unwrap();
            migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
            // No commit: the complete schema-three task/selection remains usable.
        }
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert!(matches!(inspect(&connection).unwrap(), Schema::Lifecycle));
        drop(connection);
        let mut store = state(&directory, "project", Policy::default());
        let project = load(store.connection.as_ref().unwrap(), "project").unwrap();
        assert!(project.tasks[0].archived && project.tasks[0].command.is_empty());
        assert_eq!(project.active_id, Some(second.id));
        let mut task = project.tasks[1].clone();
        task.revision += 1;
        task.command = "echo password=command-value".into();
        task.drafts = [
            EditorField::Goal,
            EditorField::Criteria,
            EditorField::NextAction,
            EditorField::Command,
        ]
        .into_iter()
        .map(|field| Draft {
            field,
            text: "token=draft-value".into(),
            cursor: 17,
        })
        .collect();
        task.navigation.editor_field = EditorField::Command;
        let saved = store.save(task).unwrap();
        assert_eq!(saved.command, "echo password=[REDACTED]");
        assert_eq!(saved.drafts.len(), 4);
        assert!(
            saved
                .drafts
                .iter()
                .all(|draft| !draft.text.contains("draft-value"))
        );
        drop(store);
        let store = state(&directory, "project", Policy::default());
        assert_eq!(
            load_task(store.connection.as_ref().unwrap(), "project", second.id).unwrap(),
            Some(saved)
        );
    }

    #[test]
    fn excerpt_capacity_and_lowered_quota_never_evict_and_still_allow_explicit_reclamation() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        let source = store.data_dir.join("source.rs");
        fs::write(&source, "visible\n").unwrap();
        for _ in 0..crate::context::MAX_EXCERPTS {
            store
                .context(ContextRequest {
                    task_id: task.id,
                    operation: ContextOperation::Pin(captured_excerpt(
                        task.id,
                        &source,
                        "visible\n",
                    )),
                })
                .unwrap();
        }
        assert!(
            store
                .context(ContextRequest {
                    task_id: task.id,
                    operation: ContextOperation::Pin(captured_excerpt(
                        task.id,
                        &source,
                        "visible\n"
                    ))
                })
                .is_err()
        );
        let basket = store
            .context(ContextRequest {
                task_id: task.id,
                operation: ContextOperation::Load,
            })
            .unwrap();
        assert_eq!(basket.len(), 16);
        store.policy.quota_bytes = 1;
        let reduced = store
            .context(ContextRequest {
                task_id: task.id,
                operation: ContextOperation::Remove(basket[0].id),
            })
            .unwrap();
        assert_eq!(reduced.len(), 15);
        let purged = store
            .context(ContextRequest {
                task_id: task.id,
                operation: ContextOperation::Purge(vec![basket[1].id]),
            })
            .unwrap();
        assert!(!purged[0].available);
        assert_eq!(fs::read_to_string(source).unwrap(), "visible\n");
    }

    #[test]
    fn completed_output_is_bounded_missing_is_unavailable_and_task_purge_is_isolated() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let first = store.save(snapshot("project", 1)).unwrap();
        let second = store.save(snapshot("project", 1)).unwrap();
        let mut runs = Vec::new();
        for (task, suffix) in [(first.id, "first"), (second.id, "second")] {
            let prepared = prepared_run(task, &store.data_dir, suffix);
            let mut permit = store.reserve_run(task, prepared.clone(), 1024).unwrap();
            permit.file.write_all("前\n".as_bytes()).unwrap();
            permit.file.sync_all().unwrap();
            let mut record = crate::runner::RunRecord::awaiting(&prepared);
            record.state = crate::runner::RunState::Running;
            record.started_at_ms = Some(timestamp().unwrap() as u64);
            store.update_run(task, record.clone()).unwrap();
            record.state = crate::runner::RunState::Succeeded;
            record.ended_at_ms = Some(timestamp().unwrap() as u64);
            record.exit_code = Some(0);
            record.tree = crate::runner::TreeOutcome::OwnedTreeExited;
            record.captured_bytes = 4;
            drop(permit);
            store.update_run(task, record).unwrap();
            runs.push(prepared.run_id);
        }
        assert!(store.read_output(first.id, &runs[0], 0, 2).is_err());
        assert_eq!(
            store.read_output(first.id, &runs[0], 0, 3).unwrap().text,
            "前"
        );
        assert_eq!(
            store.read_output(first.id, &runs[0], 3, 1).unwrap().text,
            "\n"
        );
        assert!(store.read_output(first.id, &runs[0], 0, 65_537).is_err());
        assert!(store.read_output(second.id, &runs[0], 0, 64).is_err());
        fs::remove_file(store.data_dir.join(spool_name(&runs[0]).unwrap())).unwrap();
        let records = store.load_runs(first.id).unwrap();
        assert_eq!(records[0].state, crate::runner::RunState::Succeeded);
        assert!(records[0].output_unavailable);
        assert!(
            store
                .read_output(first.id, &runs[0], 0, 64)
                .unwrap()
                .unavailable
        );
        store.purge_output(first.id, &runs[0]).unwrap();
        store
            .manage(Management::Delete {
                ids: vec![first.id],
                expired_before: None,
            })
            .unwrap();
        assert_eq!(
            store.read_output(second.id, &runs[1], 0, 64).unwrap().text,
            "前\n"
        );
    }

    #[test]
    fn foreign_output_reference_is_refused_before_writable_open() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        let prepared = prepared_run(task.id, &store.data_dir, "foreign");
        let permit = store.reserve_run(task.id, prepared.clone(), 1024).unwrap();
        drop(permit);
        store
            .run_request(RunRequest {
                task_id: task.id,
                operation: RunOperation::Abandon {
                    request_id: prepared.request_id,
                    run_id: prepared.run_id,
                },
            })
            .unwrap();
        store
            .connection
            .as_ref()
            .unwrap()
            .execute("UPDATE executions SET output_name='../outside'", [])
            .unwrap();
        drop(store);
        let path = directory.0.join(DATABASE);
        let before = fs::read(&path).unwrap();
        let worker = worker(&directory, "project");
        assert!(matches!(event(&worker), Event::Failed { .. }));
        drop(worker);
        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[test]
    fn deletion_preview_lists_exact_owned_artifacts_without_mutation_and_rechecks_active_work() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        let source = store.data_dir.join("source.rs");
        fs::write(&source, "visible\n").unwrap();
        let basket = store
            .context(ContextRequest {
                task_id: task.id,
                operation: ContextOperation::Pin(captured_excerpt(task.id, &source, "visible\n")),
            })
            .unwrap();
        let prepared = prepared_run(task.id, &store.data_dir, "preview");
        let permit = store.reserve_run(task.id, prepared.clone(), 1024).unwrap();
        assert!(store.preview_delete(&[task.id], None).is_err());
        assert!(
            store
                .context(ContextRequest {
                    task_id: task.id,
                    operation: ContextOperation::Purge(vec![basket[0].id])
                })
                .is_err()
        );
        drop(permit);
        store
            .run_request(RunRequest {
                task_id: task.id,
                operation: RunOperation::Abandon {
                    request_id: prepared.request_id,
                    run_id: prepared.run_id.clone(),
                },
            })
            .unwrap();
        let before = fs::read(store.data_dir.join(DATABASE)).unwrap();
        let preview = store.preview_delete(&[task.id], None).unwrap();
        assert_eq!(preview.len(), 1);
        assert_eq!(preview[0].task.id, task.id);
        assert_eq!(
            preview[0].excerpts,
            vec![(basket[0].id, "source.rs".into())]
        );
        assert_eq!(preview[0].runs, vec![(prepared.run_id, true)]);
        assert_eq!(fs::read(store.data_dir.join(DATABASE)).unwrap(), before);
        assert_eq!(fs::read_to_string(&source).unwrap(), "visible\n");
        let permit = store
            .reserve_run(
                task.id,
                prepared_run(task.id, &store.data_dir, "after-preview"),
                1024,
            )
            .unwrap();
        assert!(
            store
                .manage(Management::Delete {
                    ids: vec![task.id],
                    expired_before: None
                })
                .is_err()
        );
        assert!(
            load_task(store.connection.as_ref().unwrap(), "project", task.id)
                .unwrap()
                .is_some()
        );
        drop(permit);
    }
    #[test]
    fn planning_admission_rejects_changed_task_and_purged_context() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let saved = store.save(snapshot("project", 1)).unwrap();
        validate_planning_input(&store.data_dir, &saved, &[]).unwrap();
        let source = directory.0.join("source.rs");
        fs::write(&source, "visible\n").unwrap();
        let basket = store
            .context(ContextRequest {
                task_id: saved.id,
                operation: ContextOperation::Pin(captured_excerpt(saved.id, &source, "visible\n")),
            })
            .unwrap();
        validate_planning_input(&store.data_dir, &saved, &basket).unwrap();
        store
            .context(ContextRequest {
                task_id: saved.id,
                operation: ContextOperation::Purge(vec![basket[0].id]),
            })
            .unwrap();
        assert!(validate_planning_input(&store.data_dir, &saved, &basket).is_err());
        let mut changed = saved.clone();
        changed.revision += 1;
        changed.goal = "A different decision needs a new preview".into();
        let current = store.save(changed).unwrap();
        assert!(validate_planning_input(&store.data_dir, &saved, &[]).is_err());
        validate_planning_input(&store.data_dir, &current, &[]).unwrap();
    }

    #[test]
    fn terminal_run_observations_recover_after_failed_start_commit() {
        use crate::runner::{RunRecord, RunState, TreeOutcome};
        for terminal in [RunState::Succeeded, RunState::Cancelled] {
            let directory = Directory::new();
            let mut store = state(&directory, "project", Policy::default());
            let task = store.save(snapshot("project", 1)).unwrap();
            let prepared = prepared_run(task.id, &store.data_dir, "failed-start");
            let permit = store.reserve_run(task.id, prepared.clone(), 4096).unwrap();
            let mut record = RunRecord::awaiting(&prepared);
            record.state = RunState::Running;
            record.started_at_ms = Some(timestamp().unwrap() as u64);
            store
                .connection
                .as_ref()
                .unwrap()
                .pragma_update(None, "query_only", true)
                .unwrap();
            assert!(store.update_run(task.id, record.clone()).is_err());
            store
                .connection
                .as_ref()
                .unwrap()
                .pragma_update(None, "query_only", false)
                .unwrap();
            assert_eq!(
                stored_run(
                    store.connection.as_ref().unwrap(),
                    task.id,
                    &prepared.run_id
                )
                .unwrap()
                .record
                .state,
                RunState::AwaitingDispatch
            );
            record.state = terminal;
            record.ended_at_ms = record.started_at_ms;
            record.exit_code = Some(0);
            record.tree = TreeOutcome::OwnedTreeExited;
            record.cancelled = terminal == RunState::Cancelled;
            drop(permit);
            store.update_run(task.id, record).unwrap();
            no_active(store.connection.as_ref().unwrap(), Some(task.id)).unwrap();
            drop(store);
            let reopened = state(&directory, "project", Policy::default());
            assert_eq!(reopened.load_runs(task.id).unwrap()[0].state, terminal);
        }
    }

    #[test]
    fn stale_run_observation_cannot_erase_committed_capture_evidence() {
        use crate::runner::{RunRecord, RunState};
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        let prepared = prepared_run(task.id, &store.data_dir, "monotonic");
        let permit = store.reserve_run(task.id, prepared.clone(), 4096).unwrap();
        let mut record = RunRecord::awaiting(&prepared);
        record.state = RunState::Running;
        record.started_at_ms = Some(timestamp().unwrap() as u64);
        record.captured_bytes = 10;
        record.discarded_bytes = 20;
        record.dropped_live_bytes = 5;
        record.truncated = true;
        record.output_unavailable = true;
        let committed = store.update_run(task.id, record.clone()).unwrap();
        for mutation in 0..5 {
            let mut stale = record.clone();
            match mutation {
                0 => stale.captured_bytes = 0,
                1 => stale.discarded_bytes = 0,
                2 => stale.dropped_live_bytes = 0,
                3 => stale.truncated = false,
                _ => stale.output_unavailable = false,
            }
            assert!(store.update_run(task.id, stale).is_err());
            assert_eq!(
                stored_run(
                    store.connection.as_ref().unwrap(),
                    task.id,
                    &prepared.run_id
                )
                .unwrap()
                .record,
                committed
            );
        }
        drop(permit);
    }

    #[test]
    fn lifecycle_migration_never_reuses_deleted_task_identities() {
        for keep_survivor in [false, true] {
            let directory = Directory::new();
            let mut store = state(&directory, "project", Policy::default());
            let first = store.save(snapshot("project", 1)).unwrap();
            let deleted = store.save(snapshot("project", 1)).unwrap();
            let ids = if keep_survivor {
                vec![deleted.id]
            } else {
                vec![first.id, deleted.id]
            };
            store
                .manage(Management::Delete {
                    ids,
                    expired_before: None,
                })
                .unwrap();
            drop(store);
            let mut connection = Connection::open(directory.0.join(DATABASE)).unwrap();
            downgrade_workflows(&connection);
            connection
                .execute_batch(
                    "DROP TABLE provider_journal; DROP TABLE excerpts; DROP TABLE executions;
                 ALTER TABLE tasks DROP COLUMN command; PRAGMA user_version=3;",
                )
                .unwrap();
            {
                let transaction = connection.transaction().unwrap();
                migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
                // Aborting the migration must preserve both schema and the high-water mark.
            }
            assert!(matches!(inspect(&connection).unwrap(), Schema::Lifecycle));
            let sequence: i64 = connection
                .query_row(
                    "SELECT seq FROM sqlite_sequence WHERE name='tasks'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(sequence, deleted.id);
            drop(connection);
            let mut reopened = state(&directory, "project", Policy::default());
            assert!(reopened.save(snapshot("project", 1)).unwrap().id > deleted.id);
        }
    }
    fn memory_proposal(task_id: i64, scope: crate::memory::Scope) -> crate::memory::Proposal {
        crate::memory::Proposal {
            scope,
            content: "Prefer explicit, bounded changes".into(),
            provenance: crate::memory::Provenance {
                kind: crate::memory::SourceKind::UserAuthored,
                reference: "User-authored task note".into(),
                task_id,
            },
        }
    }

    #[test]
    fn schema_six_migration_is_atomic_and_defaults_preserve_old_snapshots() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        drop(store);
        let mut connection = Connection::open(directory.0.join(DATABASE)).unwrap();
        downgrade_workflows(&connection);
        {
            let transaction = connection.transaction().unwrap();
            migrate(&transaction, inspect(&transaction).unwrap()).unwrap();
            assert!(matches!(inspect(&transaction).unwrap(), Schema::Current));
        }
        assert!(matches!(
            inspect(&connection).unwrap(),
            Schema::ProviderJournal
        ));
        initialize(&mut connection).unwrap();
        assert_eq!(
            load_task(&connection, "project", task.id).unwrap().unwrap(),
            task
        );
        assert!(
            memories(&connection, "project", task.id)
                .unwrap()
                .is_empty()
        );
        assert!(matches!(inspect(&connection).unwrap(), Schema::Current));
    }

    #[test]
    fn memories_are_scoped_cas_guarded_and_survive_only_shared_task_purge() {
        use crate::memory::{Mutation, Scope};
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        let other = store.save(snapshot("project", 1)).unwrap();
        for scope in [Scope::Task, Scope::Project, Scope::Personal] {
            store
                .memory_request(MemoryRequest {
                    task_id: task.id,
                    mutation: Some(Mutation::Approve(memory_proposal(task.id, scope))),
                })
                .unwrap();
        }
        let records = store
            .memory_request(MemoryRequest {
                task_id: task.id,
                mutation: None,
            })
            .unwrap();
        assert_eq!(records.len(), 3);
        let shared = store
            .memory_request(MemoryRequest {
                task_id: other.id,
                mutation: None,
            })
            .unwrap();
        assert_eq!(shared.len(), 2);
        assert_eq!(
            memories(store.connection.as_ref().unwrap(), "foreign", 777)
                .unwrap()
                .len(),
            1
        );
        assert!(
            store
                .memory_request(MemoryRequest {
                    task_id: other.id,
                    mutation: Some(Mutation::Delete {
                        id: records[0].id,
                        expected_revision: 1
                    })
                })
                .is_err()
        );
        let mut proposal = records[1].proposal.clone();
        proposal.content = "Updated explicit note".into();
        let changed = store
            .memory_request(MemoryRequest {
                task_id: task.id,
                mutation: Some(Mutation::Edit {
                    id: records[1].id,
                    expected_revision: 1,
                    proposal: proposal.clone(),
                }),
            })
            .unwrap();
        assert_eq!(changed[1].revision, 2);
        assert!(
            store
                .memory_request(MemoryRequest {
                    task_id: task.id,
                    mutation: Some(Mutation::Edit {
                        id: records[1].id,
                        expected_revision: 1,
                        proposal
                    })
                })
                .is_err()
        );
        assert!(
            store
                .memory_request(MemoryRequest {
                    task_id: task.id,
                    mutation: Some(Mutation::Delete {
                        id: records[1].id,
                        expected_revision: 1
                    })
                })
                .is_err()
        );
        let preview = store.preview_delete(&[task.id], None).unwrap();
        assert_eq!(preview[0].memories, vec![records[0].clone()]);
        store
            .manage(Management::Delete {
                ids: vec![task.id],
                expired_before: None,
            })
            .unwrap();
        let retained = store
            .memory_request(MemoryRequest {
                task_id: other.id,
                mutation: None,
            })
            .unwrap();
        assert_eq!(retained.len(), 2);
        assert!(retained.iter().all(|record| record.task_id.is_none()));
        drop(store);
        let mut store = state(&directory, "project", Policy::default());
        assert_eq!(
            store
                .memory_request(MemoryRequest {
                    task_id: other.id,
                    mutation: None
                })
                .unwrap(),
            retained
        );
    }

    #[test]
    fn explicit_memory_snapshot_is_stable_and_dispatch_rejects_changed_approval() {
        use crate::memory::{Mutation, Scope};
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let mut task = store.save(snapshot("project", 1)).unwrap();
        let records = store
            .memory_request(MemoryRequest {
                task_id: task.id,
                mutation: Some(Mutation::Approve(memory_proposal(task.id, Scope::Project))),
            })
            .unwrap();
        task.memory_draft =
            serde_json::json!({"selected_ids":[records[0].id],"text":"unfinished note"});
        task.workflow_draft = serde_json::json!({"text":"password=private-draft"});
        task.approved_memories =
            crate::memory::snapshot(&records, &[records[0].id], "project", task.id).unwrap();
        task.revision += 1;
        let committed = store.save(task).unwrap();
        assert!(committed.redacted);
        assert!(
            !committed
                .workflow_draft
                .to_string()
                .contains("private-draft")
        );
        assert_eq!(committed.approved_memories, records);
        validate_planning_input(&directory.0, &committed, &[]).unwrap();
        let mut proposal = records[0].proposal.clone();
        proposal.content = "Changed after preview".into();
        store
            .memory_request(MemoryRequest {
                task_id: committed.id,
                mutation: Some(Mutation::Edit {
                    id: records[0].id,
                    expected_revision: 1,
                    proposal,
                }),
            })
            .unwrap();
        assert!(validate_planning_input(&directory.0, &committed, &[]).is_err());
        assert_eq!(
            load_task(store.connection.as_ref().unwrap(), "project", committed.id)
                .unwrap()
                .unwrap(),
            committed
        );
        assert_eq!(store.save(committed.clone()).unwrap(), committed);
        drop(store);
        let mut store = state(&directory, "project", Policy::default());
        assert_eq!(store.select(committed.id).unwrap(), committed);
        store
            .memory_request(MemoryRequest {
                task_id: committed.id,
                mutation: Some(Mutation::Delete {
                    id: records[0].id,
                    expected_revision: 2,
                }),
            })
            .unwrap();
        assert!(validate_planning_input(&directory.0, &committed, &[]).is_err());
    }

    #[test]
    fn memory_worker_events_and_backpressure_retain_owned_requests() {
        let directory = Directory::new();
        let store = worker(&directory, "project");
        loaded(&store);
        let task = create(&store, snapshot("project", 1));
        let mut pending = Some(MemoryRequest {
            task_id: task.id,
            mutation: None,
        });
        assert!(store.try_memory(&mut pending));
        assert!(pending.is_none());
        assert!(
            matches!(event(&store), Event::Memory { task_id, result: Ok(records) }
            if task_id == task.id && records.is_empty())
        );
        drop(store);
        let (commands, incoming) = mpsc::sync_channel(1);
        let store = Worker {
            commands: Some(commands),
            events: None,
            thread: None,
        };
        let mut pending = Some(MemoryRequest {
            task_id: 1,
            mutation: None,
        });
        assert!(store.try_memory(&mut pending));
        pending = Some(MemoryRequest {
            task_id: 2,
            mutation: None,
        });
        assert!(!store.try_memory(&mut pending));
        assert_eq!(pending.as_ref().unwrap().task_id, 2);
        drop(incoming);
        assert!(!store.try_memory(&mut pending));
        assert_eq!(pending.as_ref().unwrap().task_id, 2);
    }

    #[test]
    fn memory_secrets_and_quota_fail_without_partial_approval() {
        use crate::memory::{Mutation, Scope};
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let task = store.save(snapshot("project", 1)).unwrap();
        let mut proposal = memory_proposal(task.id, Scope::Task);
        proposal.content = "password=memory-secret".into();
        assert!(
            store
                .memory_request(MemoryRequest {
                    task_id: task.id,
                    mutation: Some(Mutation::Approve(proposal))
                })
                .is_err()
        );
        store.policy.quota_bytes = 1;
        assert!(
            store
                .memory_request(MemoryRequest {
                    task_id: task.id,
                    mutation: Some(Mutation::Approve(memory_proposal(task.id, Scope::Task)))
                })
                .is_err()
        );
        assert!(
            store
                .memory_request(MemoryRequest {
                    task_id: task.id,
                    mutation: None
                })
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn review_reads_only_linked_owned_observations_and_rejects_unsaved_selection() {
        let directory = Directory::new();
        let mut store = state(&directory, "project", Policy::default());
        let mut task = store.save(snapshot("project", 1)).unwrap();
        let prepared = prepared_run(task.id, &store.data_dir, "review");
        let _permit = store.reserve_run(task.id, prepared.clone(), 4096).unwrap();
        assert!(
            review_run_observations(&directory.0, &task)
                .unwrap()
                .is_empty()
        );
        let reference = crate::workflows::EvidenceRef::Run {
            task_id: task.id,
            run_id: prepared.run_id.clone(),
        };
        task.workflows.review = Some(crate::workflows::Review {
            requested: crate::workflows::Verdict::Inconclusive,
            criteria: vec![
                crate::workflows::CriterionEvidence {
                    criterion: "Observed result".into(),
                    evidence: vec![reference.clone()],
                },
                crate::workflows::CriterionEvidence {
                    criterion: "Explicit duplicate link".into(),
                    evidence: vec![reference],
                },
            ],
        });
        assert!(review_run_observations(&directory.0, &task).is_err());
        task.revision += 1;
        task = store.save(task).unwrap();
        let records = review_run_observations(&directory.0, &task).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].prepared.run_id, prepared.run_id);
        assert_eq!(records[0].prepared.task_id, task.id);
        assert!(
            !serde_json::to_string(&records)
                .unwrap()
                .contains("journal-value")
        );
        let mut foreign = task.clone();
        foreign.workflows.review.as_mut().unwrap().criteria[0].evidence =
            vec![crate::workflows::EvidenceRef::Run {
                task_id: task.id + 1,
                run_id: prepared.run_id,
            }];
        assert!(review_run_observations(&directory.0, &foreign).is_err());
    }

    #[test]
    fn drawer_drafts_reject_excess_bytes_depth_and_secret_field_names() {
        let mut task = snapshot("project", 1);
        task.memory_draft = serde_json::Value::String("x".repeat(MAX_MEMORY_BYTES));
        assert!(validate(&task).is_err());
        task.memory_draft = serde_json::Value::Null;
        for _ in 0..34 {
            task.memory_draft = serde_json::Value::Array(vec![task.memory_draft]);
        }
        assert!(validate(&task).is_err());
        task.memory_draft = serde_json::json!({"password=private-key":"draft"});
        assert!(validate(&task).is_err());
    }

    #[test]
    fn old_palette_navigation_preserves_remaining_navigation() {
        let stored = serde_json::json!({
            "focus":2, "linear":true, "form_selection":8,
            "palette_selection":21, "criterion_selection":4,
            "scroll":[1,2,3,4,5], "overlay":5,
            "editing":true, "editor_field":"NextAction"
        });
        let navigation: Navigation = serde_json::from_value(stored).unwrap();
        assert_eq!(navigation.focus, 2);
        assert_eq!(navigation.form_selection, 8);
        assert_eq!(navigation.criterion_selection, 4);
        assert_eq!(navigation.scroll, [1, 2, 3, 4, 5]);
        assert_eq!(navigation.editor_field, EditorField::NextAction);
    }
}
