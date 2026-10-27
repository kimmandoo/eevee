//! Bounded project-private retained search. No separate index or transcript copy.
use super::*;
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub const MAX_LANDMARKS: usize = 32;
pub const MAX_POSITION: usize = 32768;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Landmark {
    pub label: String,
    pub journal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<usize>,
}
pub fn validate_landmarks(values: &[Landmark]) -> Result<(), String> {
    if values.len() > MAX_LANDMARKS {
        return Err("Landmarks are full (32); remove one explicitly".into());
    }
    for (index, value) in values.iter().enumerate() {
        if value.label.trim().is_empty() || value.label.len() > 128
            || value.label.chars().any(|c| c.is_control() || matches!(c, '\u{061c}'|'\u{200e}'|'\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
            || crate::workspace::privacy::redact(&value.label) != value.label
            || value.journal_id.is_empty() || value.journal_id.len() > 1024
            || value.journal_id.chars().any(char::is_control)
            || crate::workspace::privacy::redact(&value.journal_id) != value.journal_id
            || value.position.is_some_and(|p| p > MAX_POSITION)
            || values[..index].iter().any(|old| old.label == value.label)
        { return Err("Use a unique, nonsecret landmark label (1–128 UTF-8 bytes) and bounded retained identity/position".into()); }
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Goal,
    Criterion(usize),
    Decision,
    CriterionDecision(usize),
    Note(i64, u64),
    Journal(String),
    Run(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum When {
    TaskModified(i64),
    RunEnded(u64),
    RunPrepared(u64),
    Unknown,
}
impl When {
    pub fn label(&self) -> String {
        match self {
            Self::TaskModified(at) => format!("Task modified: {at} Unix ms"),
            Self::RunEnded(at) => format!("Run ended: {at} Unix ms"),
            Self::RunPrepared(at) => format!("Run prepared: {at} Unix ms"),
            Self::Unknown => "Original observation/approval time: unknown".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub task: i64,
    pub archived: bool,
    pub kind: &'static str,
    pub when: When,
    pub target: Target,
    pub digest: [u8; 32],
    pub excerpt: String,
    pub attribution: String,
}
#[derive(Clone, Debug)]
pub enum Operation {
    Search { literal: String, archived: bool },
    Journals { task: i64 },
    Open { hit: Hit },
    Jump { task: i64, landmark: Landmark },
}
#[derive(Clone, Debug)]
pub struct Request {
    pub generation: u64,
    pub cancel: Arc<AtomicBool>,
    pub operation: Operation,
}
#[derive(Debug)]
pub enum Response {
    Hits {
        values: Vec<Hit>,
        incomplete: bool,
        scanned: usize,
    },
    Opened {
        hit: Hit,
        text: String,
        position: usize,
    },
}
struct Scan<'a> {
    request: &'a Request,
    literal: &'a str,
    values: Vec<Hit>,
    bytes: usize,
    scanned: usize,
    incomplete: bool,
    bounded: bool,
}
impl Scan<'_> {
    fn add(
        &mut self,
        owner: (i64, bool),
        kind: &'static str,
        when: When,
        target: Target,
        text: &str,
        attribution: String,
    ) {
        let capacity = if matches!(&self.request.operation, Operation::Journals { .. }) {
            256
        } else {
            128
        };
        if self.request.cancel.load(Ordering::Relaxed)
            || self.bytes.saturating_add(text.len()) > 4 * 1024 * 1024
            || self.values.len() >= capacity
        {
            self.incomplete = true;
            self.bounded = true;
            return;
        }
        self.bytes += text.len();
        self.scanned += 1;
        if let Some(found) = text.find(self.literal) {
            let mut begin = found.saturating_sub(80);
            while !text.is_char_boundary(begin) {
                begin += 1;
            }
            let mut end = (found + self.literal.len() + 160).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.values.push(Hit {
                task: owner.0,
                archived: owner.1,
                kind,
                when,
                target,
                digest: Sha256::digest(text.as_bytes()).into(),
                excerpt: text[begin..end].into(),
                attribution,
            });
        }
    }
    fn stopped(&self) -> bool {
        self.bounded || self.request.cancel.load(Ordering::Relaxed)
    }
}
fn partner(record: &ProviderRecord) -> String {
    format!(
        "{} / request {} / item {} / {}",
        record.partner.as_ref().map_or_else(
            || "partner unknown".into(),
            |p| format!(
                "{} (#{} / {})",
                p.name,
                p.partner,
                crate::domain::FORMS[p.form].name
            )
        ),
        record.turn_id,
        record.item_id,
        record.origin
    )
}
fn run_summary(run: &crate::workspace::runner::RunRecord) -> String {
    format!(
        "{:?}\nOutcome {:?}; exit {:?}; signal {:?}; captured {} bytes; omitted {} bytes; unavailable {}; {}",
        run.prepared.spec,
        run.state,
        run.exit_code,
        run.signal,
        run.captured_bytes,
        run.discarded_bytes,
        run.output_unavailable,
        run.error.as_deref().unwrap_or("")
    )
}
fn fields(
    connection: &Connection,
    task: &Snapshot,
    scan: &mut Scan<'_>,
    only_journal: bool,
) -> Result<(), String> {
    if !only_journal {
        scan.add(
            (task.id, task.archived),
            "goal",
            When::TaskModified(task.updated_at),
            Target::Goal,
            &task.goal,
            String::new(),
        );
        for (i, c) in task.criteria.iter().enumerate() {
            scan.add(
                (task.id, task.archived),
                "criterion",
                When::TaskModified(task.updated_at),
                Target::Criterion(i),
                &c.text,
                String::new(),
            );
        }
        if let Some(d) = &task.workflows.chosen_decision {
            scan.add(
                (task.id, task.archived),
                "decision",
                When::TaskModified(task.updated_at),
                Target::Decision,
                &d.text,
                d.id.clone(),
            );
        }
        for (index, origin) in task.workflows.criterion_origins.iter().enumerate() {
            if task
                .workflows
                .chosen_decision
                .as_ref()
                .is_some_and(|d| d.id == origin.decision_id && d.text == origin.decision_text)
                || task.workflows.criterion_origins[..index].iter().any(|old| {
                    old.decision_id == origin.decision_id
                        && old.decision_text == origin.decision_text
                })
            {
                continue;
            }
            scan.add(
                (task.id, task.archived),
                "decision",
                When::TaskModified(task.updated_at),
                Target::CriterionDecision(index),
                &origin.decision_text,
                origin.decision_id.clone(),
            );
        }
        let mut stmt = connection
            .prepare(&format!(
                "SELECT {RUN_COLUMNS} FROM executions WHERE task_id=?1 ORDER BY rowid LIMIT 257"
            ))
            .map_err(|e| e.to_string())?;
        for (index, row) in stmt
            .query_map([task.id], stored_run_row)
            .map_err(|e| e.to_string())?
            .enumerate()
        {
            if index >= MAX_RUNS {
                scan.incomplete = true;
                break;
            }
            let run = row.map_err(|e| e.to_string())?.record;
            let text = run_summary(&run);
            let when = run.ended_at_ms.map_or(
                When::RunPrepared(run.prepared.prepared_at_ms),
                When::RunEnded,
            );
            scan.add(
                (task.id, task.archived),
                "run summary",
                when,
                Target::Run(run.prepared.run_id),
                &text,
                run.prepared.partner.map_or_else(
                    || "partner unknown".into(),
                    |p| {
                        format!(
                            "{} (#{}); request {}",
                            p.name, p.partner, run.prepared.request_id
                        )
                    },
                ),
            );
            if scan.stopped() {
                return Ok(());
            }
        }
    }
    let mut statement = connection
        .prepare("SELECT record FROM provider_journal WHERE task_id=?1 ORDER BY rowid LIMIT 257")
        .map_err(|e| e.to_string())?;
    for (index, row) in statement
        .query_map([task.id], |row| {
            json_column::<ProviderRecord>(row, 0, 65536)
        })
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        if scan.stopped() {
            break;
        }
        if index >= 256 {
            scan.incomplete = true;
            break;
        }
        let mut record = row.map_err(|e| e.to_string())?;
        validate_provider_record(&record)?;
        sanitize_provider_record(&mut record);
        scan.add(
            (task.id, task.archived),
            "provider journal",
            When::Unknown,
            Target::Journal(record.id.clone()),
            &record.text,
            partner(&record),
        );
        if !record.complete {
            scan.incomplete = true;
        }
        if scan.stopped() {
            break;
        }
    }
    Ok(())
}
fn project_notes(
    connection: &Connection,
    project: &str,
    include_archived: bool,
    scan: &mut Scan<'_>,
) -> Result<(), String> {
    let mut statement=connection.prepare("SELECT id,revision,project,task_id,proposal FROM memories WHERE project=?1 AND scope!='personal' ORDER BY id LIMIT 257").map_err(|e|e.to_string())?;
    for (index, row) in statement
        .query_map([project], memory_row)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        if index >= MAX_MEMORIES {
            scan.incomplete = true;
            break;
        }
        let note = row.map_err(|e| e.to_string())?;
        validate_memory(&note)?;
        let source = note.task_id.unwrap_or(note.proposal.provenance.task_id);
        let owner = load_task(connection, project, source)?;
        let archived = note.task_id.is_some() && owner.as_ref().is_some_and(|task| task.archived);
        if archived && !include_archived {
            continue;
        }
        let attribution = format!(
            "{} / approved note {} revision {} / provenance task {}{}; shared project notes survive task deletion",
            note.proposal.scope.label(),
            note.id,
            note.revision,
            note.proposal.provenance.task_id,
            if owner.is_none() {
                " (not retained)"
            } else {
                ""
            }
        );
        scan.add(
            (source, archived),
            "approved note",
            When::Unknown,
            Target::Note(note.id, note.revision),
            &note.proposal.content,
            attribution,
        );
        if scan.stopped() {
            break;
        }
    }
    Ok(())
}
fn note(
    connection: &Connection,
    project: &str,
    id: i64,
    revision: u64,
) -> Result<crate::workspace::memory::Record, String> {
    let value=connection.query_row("SELECT id,revision,project,task_id,proposal FROM memories WHERE project=?1 AND id=?2 AND revision=?3 AND scope!='personal'",params![project,id,revision],memory_row).optional().map_err(|e|e.to_string())?.ok_or("Approved note changed or removed")?;
    validate_memory(&value)?;
    Ok(value)
}
impl State {
    pub(super) fn retained_request(&mut self, request: &Request) -> Result<Response, String> {
        let connection = self.connection.as_mut().ok_or("Storage unavailable")?;
        let tx = connection.transaction().map_err(|e| e.to_string())?;
        let mut scan = Scan {
            request,
            literal: "",
            values: Vec::new(),
            bytes: 0,
            scanned: 0,
            incomplete: false,
            bounded: false,
        };
        match &request.operation {
            Operation::Search { literal, archived } => {
                if literal.is_empty()
                    || literal.len() > 256
                    || literal.chars().any(char::is_control)
                {
                    return Err("Enter a literal query (1–256 UTF-8 bytes, no controls)".into());
                }
                scan.literal = literal;
                let mut stmt=tx.prepare("SELECT id FROM tasks WHERE project=?1 AND (?2 OR archived=0) ORDER BY id LIMIT 257").map_err(|e|e.to_string())?;
                let ids = stmt
                    .query_map(params![self.project, archived], |r| r.get::<_, i64>(0))
                    .map_err(|e| e.to_string())?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|e| e.to_string())?;
                if ids.len() > MAX_TASKS {
                    scan.incomplete = true;
                }
                for id in ids.into_iter().take(MAX_TASKS) {
                    if scan.stopped() {
                        break;
                    }
                    let task = load_task(&tx, &self.project, id)?.ok_or("Task unavailable")?;
                    fields(&tx, &task, &mut scan, false)?;
                }
                if !scan.stopped() {
                    project_notes(&tx, &self.project, *archived, &mut scan)?;
                }
            }
            Operation::Journals { task } => {
                let task = load_task(&tx, &self.project, *task)?
                    .ok_or("Task unavailable in this project")?;
                fields(&tx, &task, &mut scan, true)?;
            }
            Operation::Open { hit } => {
                let (text, attribution) = if let Target::Note(id, revision) = &hit.target {
                    let value = note(&tx, &self.project, *id, *revision)?;
                    if value.task_id.unwrap_or(value.proposal.provenance.task_id) != hit.task {
                        return Err("Approved note attribution changed".into());
                    }
                    (value.proposal.content, hit.attribution.clone())
                } else {
                    let task = load_task(&tx, &self.project, hit.task)?
                        .ok_or("Original evidence is unavailable in this project")?;
                    resolve(&tx, &task, &hit.target)?
                };
                if <[u8; 32]>::from(Sha256::digest(text.as_bytes())) != hit.digest {
                    return Err("Original evidence changed or was purged; search again".into());
                }
                let mut hit = hit.clone();
                hit.attribution = attribution;
                return Ok(Response::Opened {
                    hit,
                    text,
                    position: 0,
                });
            }
            Operation::Jump { task, landmark } => {
                validate_landmarks(std::slice::from_ref(landmark))?;
                let task =
                    load_task(&tx, &self.project, *task)?.ok_or("Landmark task unavailable")?;
                let target = Target::Journal(landmark.journal_id.clone());
                let (text, attribution) = resolve(&tx, &task, &target)?;
                let hit = Hit {
                    task: task.id,
                    archived: task.archived,
                    kind: "provider journal",
                    when: When::Unknown,
                    target,
                    digest: Sha256::digest(text.as_bytes()).into(),
                    excerpt: String::new(),
                    attribution,
                };
                return Ok(Response::Opened {
                    hit,
                    text,
                    position: landmark.position.unwrap_or(0),
                });
            }
        }
        if request.cancel.load(Ordering::Relaxed) {
            return Err("Retained search cancelled".into());
        }
        Ok(Response::Hits {
            values: scan.values,
            incomplete: scan.incomplete,
            scanned: scan.scanned,
        })
    }
}
fn resolve(
    connection: &Connection,
    task: &Snapshot,
    target: &Target,
) -> Result<(String, String), String> {
    if let Target::Journal(id) = target {
        let mut record = connection
            .query_row(
                "SELECT record FROM provider_journal WHERE task_id=?1 AND id=?2",
                params![task.id, id],
                |row| json_column::<ProviderRecord>(row, 0, 65536),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("Retained target unavailable or purged; no transcript is reconstructed")?;
        validate_provider_record(&record)?;
        sanitize_provider_record(&mut record);
        let who = partner(&record);
        return Ok((record.text, who));
    }
    // Resolve only named retained fields, never process-local transcript or source bodies.
    let text = match target {
        Target::Goal => task.goal.clone(),
        Target::Criterion(i) => task
            .criteria
            .get(*i)
            .ok_or("Criterion unavailable")?
            .text
            .clone(),
        Target::Decision => task
            .workflows
            .chosen_decision
            .as_ref()
            .ok_or("Decision unavailable")?
            .text
            .clone(),
        Target::CriterionDecision(index) => task
            .workflows
            .criterion_origins
            .get(*index)
            .ok_or("Retained criterion decision unavailable")?
            .decision_text
            .clone(),
        Target::Note(id, revision) => {
            note(connection, &task.project, *id, *revision)?
                .proposal
                .content
        }
        Target::Run(id) => {
            let run = connection
                .query_row(
                    &format!("SELECT {RUN_COLUMNS} FROM executions WHERE task_id=?1 AND run_id=?2"),
                    params![task.id, id],
                    stored_run_row,
                )
                .optional()
                .map_err(|e| e.to_string())?
                .ok_or("Run unavailable")?
                .record;
            let attribution = run.prepared.partner.as_ref().map_or_else(
                || "partner unknown".into(),
                |p| {
                    format!(
                        "{} (#{}); request {}",
                        p.name, p.partner, run.prepared.request_id
                    )
                },
            );
            return Ok((run_summary(&run), attribution));
        }
        Target::Journal(_) => unreachable!(),
    };
    Ok((text, String::new()))
}

#[cfg(test)]
#[path = "retained_tests.rs"]
mod tests;
