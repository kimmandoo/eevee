use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

mod admission;
mod journal;
mod memories;
mod recovery;
mod runs;
mod schema;
mod spool;
mod tasks;
#[cfg(test)]
mod tests;

use journal::*;
use memories::*;
use recovery::*;
use runs::*;
use schema::*;
use spool::*;
use tasks::*;

pub use admission::{review_run_observations, validate_planning_input};
pub use recovery::export_readable;
pub(crate) use runs::valid_run_id;

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
    pub provider_records: Vec<(String, crate::providers::Agent)>,
    pub memories: Vec<crate::workspace::memory::Record>,
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
    pub workflows: crate::workspace::workflows::State,
    #[serde(default)]
    pub memory_draft: serde_json::Value,
    #[serde(default)]
    pub approved_memories: Vec<crate::workspace::memory::Record>,
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
            workflows: crate::workspace::workflows::State::default(),
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
    Pin(crate::workspace::context::Excerpt),
    Remove(i64),
    Purge(Vec<i64>),
    Validate(Vec<crate::workspace::context::Excerpt>),
}

pub struct RunRequest {
    pub task_id: i64,
    pub operation: RunOperation,
}

pub enum RunOperation {
    Reserve {
        prepared: crate::workspace::runner::Prepared,
        capacity: u64,
    },
    Update(crate::workspace::runner::RunRecord),
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
    Updated(crate::workspace::runner::RunRecord),
    Runs(Vec<crate::workspace::runner::RunRecord>),
    Output(OutputPage),
    // A committed purge always carries the unavailable record, even if cleanup fails.
    Purged {
        record: crate::workspace::runner::RunRecord,
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
    pub agent: crate::providers::Agent,
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
    pub mutation: Option<crate::workspace::memory::Mutation>,
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
        result: Result<Vec<crate::workspace::context::Excerpt>, String>,
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
        result: Result<Vec<crate::workspace::memory::Record>, String>,
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
