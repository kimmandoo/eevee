//! Single-writer local record persistence and factual projections.
use super::*;
use crate::workspace::records::*;
use crate::workspace::{runner, workflows};
use serde::de::DeserializeOwned;
use std::io::Read;

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub task_id: i64,
    pub operation: Operation,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    Load {
        current: Option<runner::InputSnapshot>,
    },
    ObserveRun {
        run_id: String,
    },
    ObserveInstruction {
        request_id: String,
        owner: Attribution,
        instruction: String,
    },
    SetPairing {
        partner: i64,
        form: usize,
        selection: Option<Pairing>,
    },
    SetFlavor {
        enabled: bool,
    },
    DismissInstruction {
        key: String,
    },
    BindInstructionMemory {
        key: String,
        memory: i64,
        revision: u64,
    },
    ResolveFailure {
        signature: String,
        support_artifact: String,
        passing_run: String,
    },
    Usage {
        record: crate::workspace::usage::Record,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    Snapshot(Box<crate::workspace::records::Snapshot>),
    UsageSaved(crate::workspace::usage::Record),
}
const MAX_RECORD: usize = 128 * 1024;
const MAX_PROJECT_RECORDS: i64 = 8192;

pub(super) fn migrate(connection: &Connection) -> Result<(), String> {
    connection.execute_batch("CREATE TABLE local_artifacts(id TEXT PRIMARY KEY,project TEXT NOT NULL,task_id INTEGER NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=131072));
CREATE TABLE diagnosis_transitions(id TEXT PRIMARY KEY,project TEXT NOT NULL,task_id INTEGER NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=8192));
CREATE TABLE form_routes(id TEXT PRIMARY KEY,project TEXT NOT NULL,task_id INTEGER NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=4096));
CREATE TABLE local_activity(id TEXT PRIMARY KEY,project TEXT NOT NULL,task_id INTEGER NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=8192),delivered INTEGER NOT NULL CHECK(delivered IN(0,1)));
CREATE TABLE local_settings(project TEXT PRIMARY KEY,flavor INTEGER NOT NULL CHECK(flavor IN(0,1)),sequence INTEGER NOT NULL DEFAULT 0 CHECK(sequence>=0));
CREATE TABLE failure_signatures(id TEXT PRIMARY KEY,project TEXT NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=8192));
CREATE TABLE failure_observations(id TEXT PRIMARY KEY,project TEXT NOT NULL,task_id INTEGER NOT NULL,signature TEXT NOT NULL,run_id TEXT NOT NULL,at INTEGER NOT NULL CHECK(at>=0),paths TEXT NOT NULL CHECK(length(CAST(paths AS BLOB))<=8192),UNIQUE(signature,run_id));
CREATE TABLE failure_resolutions(id TEXT PRIMARY KEY,project TEXT NOT NULL,task_id INTEGER NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=8192));
CREATE TABLE instruction_candidates(id TEXT PRIMARY KEY,project TEXT NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=131072));
CREATE TABLE instruction_sources(id TEXT PRIMARY KEY,project TEXT NOT NULL,task_id INTEGER NOT NULL,signature TEXT NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=4096));
CREATE TABLE partner_pairings(id TEXT PRIMARY KEY,project TEXT NOT NULL,partner INTEGER NOT NULL,form INTEGER NOT NULL CHECK(form>=0 AND form<9),record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=8192),UNIQUE(project,partner,form));
CREATE TABLE provider_usage(id TEXT PRIMARY KEY,project TEXT NOT NULL,task_id INTEGER NOT NULL,record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=131072));
CREATE TABLE provider_usage_events(id TEXT PRIMARY KEY,project TEXT NOT NULL,task_id INTEGER NOT NULL,owner TEXT NOT NULL CHECK(length(CAST(owner AS BLOB))<=2048),agent TEXT NOT NULL CHECK(length(CAST(agent AS BLOB))<=128));
PRAGMA user_version=15;").map_err(|e|format!("Cannot migrate local records: {e}"))
}
fn key(project: &str, local: &str) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(format!("{project}\0{local}").as_bytes())
    )
}
fn identity(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 1024
        || value.chars().any(char::is_control)
        || crate::workspace::privacy::redact(value) != value
    {
        Err("Invalid or credential-bearing local record identity".into())
    } else {
        Ok(())
    }
}
fn rows<T: DeserializeOwned>(
    connection: &Connection,
    table: &str,
    project: &str,
) -> Result<Vec<T>, String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT record FROM {table} WHERE project=?1 ORDER BY rowid LIMIT 8193"
        ))
        .map_err(|e| e.to_string())?;
    let values = statement
        .query_map([project], |row| json_column(row, 0, MAX_RECORD))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<T>>>()
        .map_err(|e| e.to_string())?;
    if values.len() > MAX_PROJECT_RECORDS as usize {
        return Err("Local records exceed bounded project capacity".into());
    }
    Ok(values)
}
fn put<T: Serialize>(
    connection: &Connection,
    table: &str,
    id: &str,
    project: &str,
    task: i64,
    value: &T,
) -> Result<(), String> {
    let json = bounded_json(value, MAX_RECORD)?;
    let old: Option<(String, i64, String)> = connection
        .query_row(
            &format!("SELECT project,task_id,record FROM {table} WHERE id=?1"),
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some((p, t, saved)) = old {
        if p != project || t != task || saved != json {
            return Err(
                "Immutable local identity cannot overwrite a different owner or record".into(),
            );
        }
        return Ok(());
    }
    capacity(connection, table, project, id)?;
    connection
        .execute(
            &format!("INSERT INTO {table}(id,project,task_id,record) VALUES(?1,?2,?3,?4)"),
            params![id, project, task, json],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn next_ordinal(connection: &Connection, project: &str) -> Result<u64, String> {
    connection.query_row("INSERT INTO local_settings(project,flavor,sequence) VALUES(?1,1,1) ON CONFLICT(project) DO UPDATE SET sequence=sequence+1 RETURNING sequence",[project],|r|r.get(0)).map_err(|e|e.to_string())
}
fn artifact(connection: &Connection, project: &str, mut value: Artifact) -> Result<(), String> {
    let old: Option<Artifact> = connection
        .query_row(
            "SELECT record FROM local_artifacts WHERE id=?1 AND project=?2",
            params![value.id, project],
            |r| json_column(r, 0, MAX_RECORD),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(old) = old {
        value.ordinal = old.ordinal;
        if old != value {
            return Err("Artifact identity cannot change committed facts".into());
        }
        return Ok(());
    }
    value.ordinal = next_ordinal(connection, project)?;
    put(
        connection,
        "local_artifacts",
        &value.id,
        project,
        value.task_id,
        &value,
    )
}
fn activity(connection: &Connection, project: &str, value: &Activity) -> Result<(), String> {
    capacity(connection, "local_activity", project, &value.id)?;
    connection.execute("INSERT OR IGNORE INTO local_activity(id,project,task_id,record,delivered) VALUES(?1,?2,?3,?4,CASE WHEN coalesce((SELECT flavor FROM local_settings WHERE project=?2),1)=0 THEN 1 ELSE 0 END)",params![value.id,project,value.task_id,bounded_json(value,8192)?]).map_err(|e|e.to_string())?;
    Ok(())
}
pub(super) fn baseline(
    connection: &mut Connection,
    data_dir: &Path,
    project: &str,
    startup: bool,
) -> Result<(), String> {
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let initialized: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM local_settings WHERE project=?1)",
            [project],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !initialized {
        let mut statement = tx
            .prepare(&format!(
                "SELECT {TASK_COLUMNS} FROM tasks WHERE project=?1 ORDER BY id"
            ))
            .map_err(|e| e.to_string())?;
        let tasks = statement
            .query_map([project], snapshot_row)
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        for task in tasks {
            for excerpt in excerpts(&tx, task.id)?.into_iter().filter(|e| e.available) {
                artifact(
                    &tx,
                    project,
                    Artifact {
                        id: format!("capture:{}:{}", task.id, excerpt.id),
                        ordinal: 0,
                        supported_diagnosis: false,
                        handoff_revision: None,
                        task_id: task.id,
                        kind: ArtifactKind::Capture,
                        role_form: 1,
                        observation: None,
                        producer: None,
                        reference: excerpt.id.to_string(),
                        created_at: task.updated_at,
                        evidence: vec![workflows::EvidenceRef::source(&excerpt)?],
                        detail: format!(
                            "Legacy capture {}:{}-{}; producer and capture time unknown (task timestamp shown)",
                            excerpt.path, excerpt.start_line, excerpt.end_line
                        ),
                    },
                )?;
            }
            let mut statement = tx
                .prepare(&format!(
                    "SELECT {RUN_COLUMNS} FROM executions WHERE task_id=?1 ORDER BY rowid"
                ))
                .map_err(|e| e.to_string())?;
            for stored in statement
                .query_map([task.id], stored_run_row)
                .map_err(|e| e.to_string())?
            {
                let stored = stored.map_err(|e| e.to_string())?;
                if !stored.reserved
                    && stored.record.started_at_ms.is_some()
                    && stored.record.ended_at_ms.is_some()
                {
                    observe_run(&tx, data_dir, project, &stored)?;
                }
            }
            for hypothesis in &task.workflows.hypotheses {
                artifact(&tx,project,Artifact{id:format!("legacy-hypothesis:{}:{}",task.id,hypothesis.id),ordinal:0,supported_diagnosis:false,handoff_revision:None,task_id:task.id,kind:ArtifactKind::Hypothesis,role_form:5,observation:None,producer:None,reference:hypothesis.id.to_string(),created_at:task.updated_at,evidence:hypothesis.evidence.iter().map(|e|e.reference.clone()).collect(),detail:"Legacy diagnosis; producer/support receipt/time unknown (task timestamp shown)".into()})?;
            }
            if let Some(refactor) = &task.workflows.refactor {
                let mut evidence = refactor.hotspot.clone();
                evidence.push(workflows::EvidenceRef::Run {
                    task_id: task.id,
                    run_id: refactor.baseline_run.clone(),
                });
                if let Some(run) = &refactor.post_run {
                    evidence.push(workflows::EvidenceRef::Run {
                        task_id: task.id,
                        run_id: run.clone(),
                    });
                }
                artifact(
                    &tx,
                    project,
                    Artifact {
                        id: format!("legacy-refactor:{}", task.id),
                        ordinal: 0,
                        supported_diagnosis: false,
                        handoff_revision: None,
                        task_id: task.id,
                        kind: ArtifactKind::Refactor,
                        role_form: 6,
                        observation: None,
                        producer: None,
                        reference: "legacy".into(),
                        created_at: task.updated_at,
                        evidence,
                        detail: "Legacy refactor; producer/time unknown (task timestamp shown)"
                            .into(),
                    },
                )?;
            }
            if let Some(review) = &task.workflows.review {
                artifact(&tx,project,Artifact{id:format!("legacy-review:{}",task.id),ordinal:0,supported_diagnosis:false,handoff_revision:None,task_id:task.id,kind:ArtifactKind::Review,role_form:7,observation:None,producer:None,reference:"legacy".into(),created_at:task.updated_at,evidence:review.criteria.iter().flat_map(|c|c.evidence.clone()).collect(),detail:"Legacy review; producer/effective verdict/time unknown (task timestamp shown)".into()})?;
            }
            if let Some(decision) = &task.workflows.chosen_decision {
                artifact(&tx,project,Artifact{id:format!("legacy-decision:{}",task.id),ordinal:0,supported_diagnosis:false,handoff_revision:None,task_id:task.id,kind:ArtifactKind::Decision,role_form:4,observation:None,producer:None,reference:decision.id.clone(),created_at:task.updated_at,evidence:Vec::new(),detail:"Legacy chosen decision; producer/time unknown (task timestamp shown); not execution evidence.".into()})?;
            }
            let mut provider_records = tx
                .prepare("SELECT record FROM provider_journal WHERE task_id=?1 ORDER BY rowid")
                .map_err(|e| e.to_string())?;
            for record in provider_records
                .query_map([task.id], |r| json_column::<ProviderRecord>(r, 0, 65536))
                .map_err(|e| e.to_string())?
            {
                let record = record.map_err(|e| e.to_string())?;
                if record.complete
                    && record.origin == "Codex-requested Eevee-controlled file edit"
                    && record.item_id.starts_with("eevee-edit-")
                    && record.text.starts_with("Applied reviewed replacement to ")
                {
                    artifact(&tx,project,Artifact{id:key(project,&format!("patch:{}:{}",task.id,record.id)),ordinal:0,supported_diagnosis:false,handoff_revision:None,task_id:task.id,kind:ArtifactKind::Patch,role_form:if record.partner.as_ref().is_some_and(|p|p.form==6){6}else{3},observation:None,producer:record.partner.clone(),reference:record.id,created_at:task.updated_at,evidence:Vec::new(),detail:"Legacy controlled patch journal; commit time unknown (task timestamp shown); original immutable producer retained when present.".into()})?;
                }
            }
        }
        tx.execute(
            "INSERT OR IGNORE INTO local_settings(project,flavor,sequence) VALUES(?1,1,0)",
            [project],
        )
        .map_err(|e| e.to_string())?;
    }
    if startup || !initialized {
        tx.execute(
            "UPDATE local_activity SET delivered=1 WHERE project=?1",
            [project],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

fn live_owner(
    connection: &Connection,
    task: i64,
    request: &str,
    owner: &Attribution,
) -> Result<(), String> {
    identity(request)?;
    owner.validate()?;
    let saved: Option<Attribution> = connection
        .query_row(
            "SELECT owner FROM active_work WHERE task_id=?1 AND request_id=?2 AND kind='provider'",
            params![task, request],
            |r| json_column(r, 0, 2048),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if saved.as_ref() != Some(owner) {
        return Err(
            "Instruction/usage request does not own its exact active provider claim".into(),
        );
    }
    Ok(())
}
impl State {
    pub(super) fn records(&mut self, request: &Request) -> Result<Response, String> {
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage unavailable; local record was not saved")?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let task = if request.task_id > 0 {
            Some(
                load_task(&tx, &self.project, request.task_id)?
                    .ok_or("Record task is missing or belongs to another project")?,
            )
        } else {
            None
        };
        let mut current = None;
        let mut usage_saved = None;
        match &request.operation {
            Operation::Load {
                current: observation,
            } => {
                current = observation.as_ref();
            }
            Operation::SetFlavor { enabled } => {
                tx.execute("INSERT INTO local_settings(project,flavor) VALUES(?1,?2) ON CONFLICT(project) DO UPDATE SET flavor=excluded.flavor",params![self.project,enabled]).map_err(|e|e.to_string())?;
            }
            Operation::SetPairing {
                partner,
                form,
                selection,
            } => {
                if *form >= 9 || !partners(&tx)?.iter().any(|p| p.id == *partner) {
                    return Err("Pairing owner/form is missing or invalid".into());
                }
                let id = key(&self.project, &format!("pairing:{partner}:{form}"));
                if let Some(pairing) = selection {
                    if pairing.partner != *partner || pairing.form != *form {
                        return Err("Pairing scope does not match selection".into());
                    }
                    for value in [&pairing.selection.model, &pairing.selection.effort]
                        .into_iter()
                        .flatten()
                    {
                        identity(value)?;
                    }
                    tx.execute("INSERT INTO partner_pairings(id,project,partner,form,record) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET record=excluded.record",params![id,self.project,partner,*form as i64,bounded_json(pairing,8192)?]).map_err(|e|e.to_string())?;
                } else {
                    tx.execute("DELETE FROM partner_pairings WHERE id=?1", [id])
                        .map_err(|e| e.to_string())?;
                }
            }
            Operation::ObserveInstruction {
                request_id,
                owner,
                instruction,
            } => {
                let task = task
                    .as_ref()
                    .ok_or("Instruction observation requires an owned saved task")?;
                if task.archived {
                    return Err("Archived task cannot add instruction observations".into());
                }
                let (hash, normalized) = instruction_key(instruction)?;
                let id = key(&self.project, &format!("instruction:{request_id}"));
                let previous: Option<(String, InstructionSource)> = tx
                    .query_row(
                        "SELECT signature,record FROM instruction_sources WHERE id=?1",
                        [&id],
                        |r| Ok((r.get(0)?, json_column(r, 1, 4096)?)),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                if let Some((saved, source)) = previous {
                    if saved != hash || source.task_id != task.id || source.owner != *owner {
                        return Err("Dispatched instruction identity cannot be rewritten".into());
                    }
                } else {
                    live_owner(&tx, task.id, request_id, owner)?;
                    let sources: i64 = tx
                        .query_row(
                            "SELECT count(*) FROM instruction_sources WHERE project=?1",
                            [&self.project],
                            |r| r.get(0),
                        )
                        .map_err(|e| e.to_string())?;
                    if sources >= MAX_PROJECT_RECORDS {
                        return Err(
                            "Instruction source capacity reached; nothing was evicted".into()
                        );
                    }
                    let candidate = key(&self.project, &hash);
                    capacity(&tx, "instruction_candidates", &self.project, &candidate)?;
                    let habit = Habit {
                        key: hash.clone(),
                        instruction: normalized,
                        sources: Vec::new(),
                        dismissed: false,
                        memory: None,
                    };
                    tx.execute("INSERT OR IGNORE INTO instruction_candidates(id,project,record) VALUES(?1,?2,?3)",params![candidate,self.project,bounded_json(&habit,MAX_RECORD)?]).map_err(|e|e.to_string())?;
                    let source = InstructionSource {
                        request_id: request_id.clone(),
                        task_id: task.id,
                        owner: owner.clone(),
                        at: timestamp()?,
                    };
                    tx.execute("INSERT INTO instruction_sources(id,project,task_id,signature,record) VALUES(?1,?2,?3,?4,?5)",params![id,self.project,task.id,hash,bounded_json(&source,4096)?]).map_err(|e|e.to_string())?;
                }
            }
            Operation::DismissInstruction { key: hash }
            | Operation::BindInstructionMemory { key: hash, .. } => {
                identity(hash)?;
                let id = key(&self.project, hash);
                let mut habit: Habit = tx
                    .query_row(
                        "SELECT record FROM instruction_candidates WHERE id=?1 AND project=?2",
                        params![id, self.project],
                        |r| json_column(r, 0, MAX_RECORD),
                    )
                    .map_err(|_| "Instruction candidate is missing from this project")?;
                if let Operation::BindInstructionMemory {
                    memory, revision, ..
                } = &request.operation
                {
                    let approved=memories(&tx,&self.project,request.task_id)?.into_iter().find(|m|m.id==*memory&&m.revision==*revision&&m.project==self.project&&m.proposal.scope==crate::workspace::memory::Scope::Project&&m.proposal.content==habit.instruction&&m.proposal.provenance.reference==format!("habit:{}",habit.key)).ok_or("Exact approved project habit memory is missing, edited or belongs to another owner")?;
                    let count:i64=tx.query_row("SELECT count(*) FROM instruction_sources WHERE project=?1 AND signature=?2",params![self.project,hash],|r|r.get(0)).map_err(|e|e.to_string())?;
                    if count < 3 {
                        return Err(
                            "Habit binding requires three distinct dispatched instruction sources"
                                .into(),
                        );
                    }
                    habit.memory = Some(MemoryBinding {
                        id: approved.id,
                        revision: approved.revision,
                    });
                } else {
                    habit.dismissed = true;
                }
                tx.execute(
                    "UPDATE instruction_candidates SET record=?1 WHERE id=?2",
                    params![bounded_json(&habit, MAX_RECORD)?, id],
                )
                .map_err(|e| e.to_string())?;
            }
            Operation::Usage { record } => {
                record.validate()?;
                record.owner.validate()?;
                let task = task.as_ref().ok_or("Usage requires an owned saved task")?;
                if record.task_id != task.id {
                    return Err("Usage task differs from request owner".into());
                }
                let id = key(&self.project, &format!("usage:{}", record.request_id));
                let origin:Option<(Attribution,crate::providers::Agent)>=tx.query_row("SELECT owner,agent FROM provider_usage_events WHERE id=?1 AND project=?2 AND task_id=?3",params![id,self.project,task.id],|r|Ok((json_column(r,0,2048)?,json_column(r,1,128)?))).optional().map_err(|e|e.to_string())?;
                if let Some((owner, agent)) = origin {
                    if owner != record.owner || agent != record.agent {
                        return Err("Usage cannot change original request owner or agent".into());
                    }
                } else {
                    live_owner(&tx, task.id, &record.request_id, &record.owner)?;
                    capacity(&tx, "provider_usage_events", &self.project, &id)?;
                    tx.execute("INSERT INTO provider_usage_events(id,project,task_id,owner,agent) VALUES(?1,?2,?3,?4,?5)",params![id,self.project,task.id,bounded_json(&record.owner,2048)?,bounded_json(&record.agent,128)?]).map_err(|e|e.to_string())?;
                }
                let saved: Option<crate::workspace::usage::Record> = tx
                    .query_row(
                        "SELECT record FROM provider_usage WHERE id=?1",
                        [&id],
                        |r| json_column(r, 0, MAX_RECORD),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                if let Some(old) = saved {
                    for entry in &old.measurements {
                        if !record.measurements.iter().any(|new| {
                            metadata_retained(
                                &entry.report.identity.session,
                                &new.report.identity.session,
                            ) && metadata_retained(
                                &entry.report.identity.turn,
                                &new.report.identity.turn,
                            ) && metadata_retained(
                                &entry.report.identity.response,
                                &new.report.identity.response,
                            ) && new.report.scope == entry.report.scope
                                && new.report.kind == entry.report.kind
                                && metadata_retained(&entry.report.model, &new.report.model)
                                && metadata_retained(&entry.report.tokenizer, &new.report.tokenizer)
                                && entry
                                    .delta_events
                                    .iter()
                                    .all(|event| new.delta_events.contains(event))
                                && report_retained(&entry.report, &new.report)
                        }) {
                            return Err("Usage replacement cannot erase a previously committed counter/boundary/event".into());
                        }
                    }
                }
                capacity(&tx, "provider_usage", &self.project, &id)?;
                tx.execute("INSERT INTO provider_usage(id,project,task_id,record) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET record=excluded.record",params![id,self.project,task.id,bounded_json(record,MAX_RECORD)?]).map_err(|e|e.to_string())?;
                usage_saved = Some(record.clone());
            }
            Operation::ObserveRun { run_id } => {
                let task = task
                    .as_ref()
                    .ok_or("Run observation requires a saved owned task")?;
                let stored = stored_run(&tx, task.id, run_id)?;
                observe_run(&tx, &self.data_dir, &self.project, &stored)?;
            }
            Operation::ResolveFailure {
                signature,
                support_artifact,
                passing_run,
            } => {
                let task = task
                    .as_ref()
                    .ok_or("Resolution requires an owned saved task")?;
                resolve(
                    &tx,
                    &self.project,
                    task,
                    signature,
                    support_artifact,
                    passing_run,
                )?;
            }
        }
        enforce_evidence_quota(&tx, &self.data_dir, self.policy)?;
        let response = if let Some(record) = usage_saved {
            Response::UsageSaved(record)
        } else {
            Response::Snapshot(Box::new(snapshot(
                &tx,
                &self.project,
                task.as_ref(),
                current,
            )?))
        };
        tx.commit()
            .map_err(|e| format!("Local record transaction was not committed: {e}"))?;
        Ok(response)
    }
}

fn runs_for(connection: &Connection, task: i64) -> Result<Vec<runner::RunRecord>, String> {
    let mut statement = connection
        .prepare("SELECT record FROM executions WHERE task_id=?1 ORDER BY rowid LIMIT 257")
        .map_err(|e| e.to_string())?;
    let records = statement
        .query_map([task], |r| json_column(r, 0, MAX_RUN_BYTES))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if records.len() > 256 {
        return Err("Run journal exceeds capacity".into());
    }
    Ok(records)
}
fn freshness(artifact: &Artifact, view: &workflows::EvidenceView<'_>) -> (Freshness, String) {
    if artifact.evidence.is_empty() {
        return (
            Freshness::Ambiguous,
            "Produced locally; no current verification support was recorded".into(),
        );
    }
    let mut result = Freshness::Current;
    for reference in &artifact.evidence {
        match view.status(reference) {
            workflows::EvidenceStatus::Stale => {
                return (
                    Freshness::Stale,
                    "Previously recorded support is known stale".into(),
                );
            }
            workflows::EvidenceStatus::Unavailable => result = Freshness::Unavailable,
            workflows::EvidenceStatus::Ambiguous if result != Freshness::Unavailable => {
                result = Freshness::Ambiguous
            }
            _ => {}
        }
    }
    let explanation = match result {
        Freshness::Current => {
            "Retained support is current; production alone does not establish correctness"
        }
        Freshness::Stale => "Support is stale",
        Freshness::Unavailable => "Support is missing, purged or unavailable",
        Freshness::Ambiguous => "Current support cannot be established",
    };
    (result, explanation.into())
}
fn snapshot(
    connection: &Connection,
    project: &str,
    task: Option<&super::Snapshot>,
    current: Option<&runner::InputSnapshot>,
) -> Result<crate::workspace::records::Snapshot, String> {
    let artifacts: Vec<Artifact> = rows(connection, "local_artifacts", project)?;
    let unknown = artifacts.iter().filter(|a| a.producer.is_none()).count();
    let mut summaries = Vec::new();
    let shared_bond:usize=connection.query_row("SELECT count(*) FROM memories WHERE scope='personal' OR (project=?1 AND scope='project')",[project],|r|r.get(0)).map_err(|e|e.to_string())?;
    for partner in partners(connection)? {
        let mut produced_tasks = [0; 9];
        let mut moves = [0; 9];
        let mut move_names: [Vec<String>; 9] = std::array::from_fn(|_| Vec::new());
        for form in 0..9 {
            produced_tasks[form]=connection.query_row("SELECT count(DISTINCT task_id) FROM local_artifacts WHERE project=?1 AND json_extract(record,'$.producer.partner')=?2 AND json_extract(record,'$.producer.form')=?3",params![project,partner.id,form as i64],|r|r.get(0)).map_err(|e|e.to_string())?;
            let mut statement=connection.prepare("SELECT json_extract(proposal,'$.name') FROM moves WHERE partner=?1 AND form=?2 AND forgotten=0 AND (scope='personal' OR project=?3) ORDER BY id LIMIT 513").map_err(|e|e.to_string())?;
            move_names[form] = statement
                .query_map(params![partner.id, form as i64, project], |r| r.get(0))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            if move_names[form].len() > 512 {
                return Err("Moves exceed bounded summary capacity".into());
            }
            moves[form] = move_names[form].len();
        }
        summaries.push(PartnerSummary {
            partner,
            produced_tasks,
            moves,
            move_names,
            shared_bond_memories: shared_bond,
            unknown_producer_artifacts: unknown,
        });
    }
    let mut views = Vec::new();
    let mut resolution_choices = Vec::new();
    if let Some(task) = task {
        let captures = excerpts(connection, task.id)?;
        let runs = runs_for(connection, task.id)?;
        let unknown = runner::InputSnapshot::default();
        let view = workflows::EvidenceView {
            task_id: task.id,
            excerpts: &captures,
            runs: &runs,
            current: current.unwrap_or(&unknown),
        };
        for artifact in artifacts.into_iter().filter(|a| a.task_id == task.id) {
            let (mut status, mut explanation) = freshness(&artifact, &view);
            match artifact.kind {
                ArtifactKind::Review => {
                    let criteria: Vec<String> =
                        task.criteria.iter().map(|c| c.text.clone()).collect();
                    let guard = task.workflows.review_guard(&criteria, &view);
                    if let Some(review) = &task.workflows.review {
                        if task.workflows.review_stale
                            || !review
                                .criteria
                                .iter()
                                .flat_map(|c| c.evidence.iter())
                                .eq(artifact.evidence.iter())
                        {
                            status = Freshness::Stale;
                        } else if guard.verdict == workflows::Verdict::Inconclusive
                            && status == Freshness::Current
                        {
                            status = Freshness::Ambiguous;
                        }
                    } else {
                        status = Freshness::Unavailable;
                    }
                    explanation = format!(
                        "Current effective review {:?}; {}",
                        guard.verdict, explanation
                    );
                }
                ArtifactKind::Refactor => {
                    let comparison = task.workflows.compare_refactor(&view);
                    if let Some(refactor) = &task.workflows.refactor {
                        let same=artifact.evidence.len()==refactor.hotspot.len()+1+usize::from(refactor.post_run.is_some())&&artifact.evidence.starts_with(&refactor.hotspot)
                            &&matches!(artifact.evidence.get(refactor.hotspot.len()),Some(workflows::EvidenceRef::Run{run_id,..}) if run_id==&refactor.baseline_run)
                            &&refactor.post_run.as_ref().is_none_or(|post|matches!(artifact.evidence.last(),Some(workflows::EvidenceRef::Run{run_id,..}) if run_id==post));
                        if !same {
                            status = Freshness::Stale;
                        } else if comparison.behavior_preserved {
                            status = Freshness::Current;
                        } else if let Some(post) = &refactor.post_run {
                            status = match view.status(&workflows::EvidenceRef::Run {
                                task_id: task.id,
                                run_id: post.clone(),
                            }) {
                                workflows::EvidenceStatus::Stale => Freshness::Stale,
                                workflows::EvidenceStatus::Unavailable => Freshness::Unavailable,
                                _ => Freshness::Ambiguous,
                            };
                        } else {
                            status = Freshness::Ambiguous;
                        }
                    } else {
                        status = Freshness::Unavailable;
                    }
                    explanation = format!(
                        "Current behavior-preservation guard: {}; {}",
                        comparison.behavior_preserved, comparison.reason
                    );
                }
                ArtifactKind::Hypothesis => {
                    if let Some(hypothesis) = artifact
                        .reference
                        .parse::<u64>()
                        .ok()
                        .and_then(|id| task.workflows.hypotheses.iter().find(|h| h.id == id))
                    {
                        let supported = task.workflows.hypothesis_status(hypothesis.id, &view)
                            == workflows::HypothesisStatus::Supported;
                        if !hypothesis
                            .evidence
                            .iter()
                            .map(|e| &e.reference)
                            .eq(artifact.evidence.iter())
                            || artifact.supported_diagnosis
                                && !supported
                                && status == Freshness::Current
                        {
                            status = Freshness::Stale;
                        }
                        explanation =
                            format!("Current diagnosis support guard: {supported}; {explanation}");
                    } else {
                        status = Freshness::Unavailable;
                        explanation = "The committed hypothesis is no longer available.".into();
                    }
                }
                ArtifactKind::Patch => {
                    (status, explanation) =
                        patch_status(connection, project, task.id, &artifact.reference)?;
                }
                ArtifactKind::Decision => match task.workflows.chosen_decision.as_ref() {
                    Some(decision)
                        if decision.id == artifact.reference
                            && decision.text == task.next_action
                            && decision_reference(task).as_deref()
                                == Some(decision.id.as_str()) =>
                    {
                        status = Freshness::Current;
                        explanation="Chosen decision and exact saved next action still match; not verification.".into();
                    }
                    Some(_) => {
                        status = Freshness::Stale;
                        explanation = "Chosen decision or saved next action has changed.".into();
                    }
                    None => {
                        status = Freshness::Unavailable;
                        explanation = "The chosen decision is no longer available.".into();
                    }
                },
                ArtifactKind::Handoff => match artifact.handoff_revision {
                    Some(revision) if revision < task.revision => {
                        status = Freshness::Stale;
                        explanation = format!(
                            "Historical handoff receipt for revision {revision}; current task revision is {}. It does not establish current handoff readiness. {explanation}",
                            task.revision
                        );
                    }
                    Some(revision) if revision == task.revision => {
                        explanation = format!(
                            "Handoff receipt matches task revision {revision}; inspect retained evidence and caveats. {explanation}"
                        );
                    }
                    _ => {
                        status = Freshness::Ambiguous;
                        explanation =
                            "Handoff receipt revision cannot be matched to the current task."
                                .into();
                    }
                },
                _ => {}
            }
            if artifact.supported_diagnosis
                && let Some(current) = current
            {
                observe_transition(connection, project, &artifact, current)?;
            }
            if status == Freshness::Stale && artifact.kind == ArtifactKind::Run {
                let was_current = artifact
                    .observation
                    .as_ref()
                    .is_some_and(|o| o.complete && o.stable && o.fingerprint.is_some());
                if was_current {
                    activity(connection,project,&Activity{id:format!("stale:{}",artifact.id),task_id:task.id,owner:artifact.producer.clone(),at:timestamp()?,detail:"Source changed after the recorded check; it is no longer current verification.".into()})?;
                }
            }
            views.push(ArtifactView {
                artifact,
                freshness: status,
                explanation,
            });
        }
        let transitions = rows::<SourceTransition>(connection, "diagnosis_transitions", project)?;
        let observations = observation_pairs(connection, project, task.id)?;
        let facts = ResolutionFacts {
            captures: &captures,
            runs: &runs,
            transitions: &transitions,
            observations: &observations,
        };
        for support in views
            .iter()
            .map(|v| &v.artifact)
            .filter(|a| a.supported_diagnosis)
        {
            let signatures:std::collections::BTreeSet<&str>=observations.iter().filter(|(_,run)|support.evidence.iter().any(|reference|matches!(reference,workflows::EvidenceRef::Run{run_id,..} if run_id==run))).map(|(signature,_)|signature.as_str()).collect();
            for pass in runs
                .iter()
                .filter(|run| run.state == runner::RunState::Succeeded)
            {
                for signature in &signatures {
                    if resolution_eligible(support, pass, signature, &facts).is_ok() {
                        if resolution_choices.len() >= 8192 {
                            return Err(
                                "Historical resolution choices exceed bounded capacity".into()
                            );
                        }
                        resolution_choices.push(ResolutionChoice {
                            signature: (*signature).into(),
                            support_artifact: support.id.clone(),
                            passing_run: pass.prepared.run_id.clone(),
                            cause: RESOLUTION_CAUSE.into(),
                        });
                    }
                }
            }
        }
    }
    let mut habits: Vec<Habit> = rows(connection, "instruction_candidates", project)?;
    for habit in &mut habits {
        let mut statement=connection.prepare("SELECT record FROM instruction_sources WHERE project=?1 AND signature=?2 ORDER BY rowid LIMIT 8193").map_err(|e|e.to_string())?;
        habit.sources = statement
            .query_map(params![project, habit.key], |r| json_column(r, 0, 4096))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        if let Some(binding) = &habit.memory
            && !memories(connection, project, task.map_or(0, |t| t.id))?
                .iter()
                .any(|m| {
                    m.id == binding.id
                        && m.revision == binding.revision
                        && m.proposal.content == habit.instruction
                        && m.project == project
                })
        {
            habit.memory = None;
        }
    }
    habits.retain(|h| h.sources.len() >= 3 || h.dismissed || h.memory.is_some());
    let mut signatures: Vec<FailureSignature> = rows(connection, "failure_signatures", project)?;
    for signature in &mut signatures {
        let mut statement=connection.prepare("SELECT run_id,paths FROM failure_observations WHERE project=?1 AND signature=?2 ORDER BY at,run_id LIMIT 8193").map_err(|e|e.to_string())?;
        let observations = statement
            .query_map(params![project, signature.key], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    json_column::<Vec<String>>(r, 1, 8192)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        let mut paths = std::collections::BTreeSet::new();
        signature.runs = observations
            .into_iter()
            .map(|(run, locations)| {
                paths.extend(locations);
                run
            })
            .collect();
        signature.paths = paths.into_iter().take(128).collect();
        signature.seen_count = signature.runs.len();
    }
    signatures.retain(|s| s.seen_count > 0);
    let flavor_enabled = connection
        .query_row(
            "SELECT flavor FROM local_settings WHERE project=?1",
            [project],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or(true);
    let mut statement=connection.prepare("SELECT record FROM local_activity WHERE project=?1 AND delivered=0 ORDER BY rowid LIMIT 8193").map_err(|e|e.to_string())?;
    let activities: Vec<Activity> = statement
        .query_map([project], |r| json_column(r, 0, 8192))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    connection
        .execute(
            "UPDATE local_activity SET delivered=1 WHERE project=?1 AND delivered=0",
            [project],
        )
        .map_err(|e| e.to_string())?;
    Ok(crate::workspace::records::Snapshot {
        artifacts: views,
        routes: rows::<Route>(connection, "form_routes", project)?
            .into_iter()
            .filter(|r| task.is_none_or(|t| r.task_id == t.id))
            .collect(),
        activities: if flavor_enabled {
            activities
        } else {
            Vec::new()
        },
        signatures,
        resolutions: rows(connection, "failure_resolutions", project)?,
        resolution_choices,
        habits,
        pairings: rows(connection, "partner_pairings", project)?,
        usages: rows(connection, "provider_usage", project)?,
        summaries,
        flavor_enabled,
    })
}

pub(super) fn capture(
    connection: &Connection,
    project: &str,
    excerpt: &crate::workspace::context::Excerpt,
    producer: Option<&Attribution>,
) -> Result<(), String> {
    if let Some(owner) = producer {
        owner.validate()?;
        if !partners(connection)?.iter().any(|p| p.id == owner.partner) {
            return Err("Capture producer was released before capture committed".into());
        }
    }
    artifact(
        connection,
        project,
        Artifact {
            id: format!("capture:{}:{}", excerpt.task_id, excerpt.id),
            ordinal: 0,
            supported_diagnosis: false,
            handoff_revision: None,
            task_id: excerpt.task_id,
            kind: ArtifactKind::Capture,
            role_form: 1,
            observation: None,
            producer: producer.cloned(),
            reference: excerpt.id.to_string(),
            created_at: timestamp()?,
            evidence: vec![workflows::EvidenceRef::source(excerpt)?],
            detail: format!(
                "Captured {}:{}-{}",
                excerpt.path, excerpt.start_line, excerpt.end_line
            ),
        },
    )
}
pub(super) fn save_intents(
    connection: &Connection,
    task: &super::Snapshot,
    intents: &[LocalIntent],
) -> Result<(), String> {
    if intents.len() > 256 {
        return Err("Local save intents exceed bounded batch capacity".into());
    }
    for intent in intents {
        identity(intent.id())?;
        let owner = match intent {
            LocalIntent::FormRoute { owner, .. }
            | LocalIntent::Artifact { owner, .. }
            | LocalIntent::HandoffReady { owner, .. } => owner,
        };
        owner.validate()?;
        if !partners(connection)?.iter().any(|p| p.id == owner.partner) {
            return Err("Local intent owner is no longer in the party".into());
        }
        match intent {
            LocalIntent::FormRoute {
                id,
                owner,
                from,
                to,
                accepted_suggestion,
            } => {
                if *from >= 9 || *to >= 9 || owner.form != *to {
                    return Err("Route form or owner does not match accepted switch".into());
                }
                if let Some(trigger) = accepted_suggestion {
                    identity(trigger)?;
                }
                let stored: Option<Route> = connection
                    .query_row("SELECT record FROM form_routes WHERE id=?1", [id], |r| {
                        json_column(r, 0, 4096)
                    })
                    .optional()
                    .map_err(|e| e.to_string())?;
                if let Some(old) = stored {
                    if old.task_id != task.id
                        || old.owner != *owner
                        || old.from != *from
                        || old.to != *to
                        || old.accepted_suggestion != *accepted_suggestion
                    {
                        return Err("Route identity cannot change on retry".into());
                    }
                    continue;
                }
                put(
                    connection,
                    "form_routes",
                    id,
                    &task.project,
                    task.id,
                    &Route {
                        id: id.clone(),
                        ordinal: next_ordinal(connection, &task.project)?,
                        task_id: task.id,
                        owner: owner.clone(),
                        from: *from,
                        to: *to,
                        at: timestamp()?,
                        accepted_suggestion: accepted_suggestion.clone(),
                    },
                )?;
            }
            LocalIntent::Artifact {
                id,
                owner,
                kind,
                reference,
                current,
            } => {
                let existing: Option<Artifact> = connection
                    .query_row(
                        "SELECT record FROM local_artifacts WHERE id=?1",
                        [id],
                        |r| json_column(r, 0, MAX_RECORD),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                if let Some(old) = existing {
                    if old.task_id != task.id
                        || old.kind != *kind
                        || old.reference != *reference
                        || old.producer.as_ref() != Some(owner)
                    {
                        return Err("Artifact retry cannot change original provenance".into());
                    }
                    continue;
                }
                let captures = excerpts(connection, task.id)?;
                let runs = runs_for(connection, task.id)?;
                let unknown = runner::InputSnapshot::default();
                let view = workflows::EvidenceView {
                    task_id: task.id,
                    excerpts: &captures,
                    runs: &runs,
                    current: current.as_ref().unwrap_or(&unknown),
                };
                let mut supported_diagnosis = false;
                let (evidence,role,detail)=match kind{
                    ArtifactKind::Hypothesis=>{
                        let hypothesis=reference.parse::<u64>().ok().and_then(|id|task.workflows.hypotheses.iter().find(|h|h.id==id)).ok_or("Hypothesis reference is not committed in this task")?;
                        supported_diagnosis=task.workflows.hypothesis_status(hypothesis.id,&view)==workflows::HypothesisStatus::Supported;
                        (hypothesis.evidence.iter().map(|e|e.reference.clone()).collect(),5,format!("Committed diagnosis; current supported source/run relation: {supported_diagnosis}. Historical support is not a current fix."))
                    }
                    ArtifactKind::Refactor=>{let r=task.workflows.refactor.as_ref().ok_or("No committed refactor")?;if r.task_id!=task.id{return Err("Foreign refactor owner".into())}let comparison=task.workflows.compare_refactor(&view);let mut refs=r.hotspot.clone();refs.push(workflows::EvidenceRef::Run{task_id:task.id,run_id:r.baseline_run.clone()});if let Some(run)=&r.post_run{refs.push(workflows::EvidenceRef::Run{task_id:task.id,run_id:run.clone()});}(refs,6,format!("Behavior-preservation guard: {}; {}",comparison.behavior_preserved,comparison.reason))}
                    ArtifactKind::Review=>{let review=task.workflows.review.as_ref().ok_or("No committed review")?;let criteria:Vec<String>=task.criteria.iter().map(|c|c.text.clone()).collect();let guard=task.workflows.review_guard(&criteria,&view);(review.criteria.iter().flat_map(|c|c.evidence.clone()).collect(),7,format!("Effective review: {:?}",guard.verdict))}
                    ArtifactKind::Patch=>{let record:ProviderRecord=connection.query_row("SELECT record FROM provider_journal WHERE id=?1 AND task_id=?2",params![reference,task.id],|r|json_column(r,0,65536)).map_err(|_|"Patch journal reference is unavailable")?;if workflows::controlled_patch_author(&record).is_none()||record.partner.as_ref()!=Some(owner){return Err("Patch attribution requires exact actual committed controlled patch producer".into())}(Vec::new(),owner.form,"Committed controlled patch journal".into())}
                    ArtifactKind::Decision=>{let decision=task.workflows.chosen_decision.as_ref().ok_or("No chosen decision is committed")?;if decision_reference(task).as_deref()!=Some(reference.as_str())||decision.id!=*reference{return Err("Decision reference must match the exact committed chosen decision and next-action hash".into())}(Vec::new(),4,"Explicit chosen decision committed to Next action; not execution evidence.".into())}
                    _=>return Err("Capture/run/handoff artifacts require their authoritative worker operation".into()),
                };
                validate_references(task.id, &evidence, &captures, &runs)?;
                let value = Artifact {
                    id: id.clone(),
                    ordinal: 0,
                    supported_diagnosis,
                    handoff_revision: None,
                    task_id: task.id,
                    kind: *kind,
                    role_form: role,
                    producer: Some(owner.clone()),
                    reference: reference.clone(),
                    created_at: timestamp()?,
                    observation: if *kind == ArtifactKind::Hypothesis && !supported_diagnosis {
                        None
                    } else {
                        current.clone()
                    },
                    evidence,
                    detail,
                };
                if *kind == ArtifactKind::Review {
                    activity(
                        connection,
                        &task.project,
                        &Activity {
                            id: format!("review:{}:{}", task.id, task.workflows.review_generation),
                            task_id: task.id,
                            owner: Some(owner.clone()),
                            at: timestamp()?,
                            detail: value.detail.clone(),
                        },
                    )?;
                }
                artifact(connection, &task.project, value)?;
            }
            LocalIntent::HandoffReady {
                id,
                owner,
                revision,
                digest,
                evidence,
            } => {
                if *revision > task.revision
                    || digest.len() != 64
                    || !digest.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(
                        "Handoff receipt does not match committed task revision/digest".into(),
                    );
                }
                let old: Option<Artifact> = connection
                    .query_row(
                        "SELECT record FROM local_artifacts WHERE id=?1",
                        [id],
                        |r| json_column(r, 0, MAX_RECORD),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                if let Some(old) = old {
                    if old.task_id != task.id
                        || old.kind != ArtifactKind::Handoff
                        || old.handoff_revision != Some(*revision)
                        || old.evidence != *evidence
                        || old.reference != *digest
                        || old.producer.as_ref() != Some(owner)
                    {
                        return Err("Handoff receipt cannot change on retry".into());
                    }
                    continue;
                }
                // A purged receipt is acknowledged only for its exact original intent.
                // Keep the hash, not the deleted evidence metadata, as the tombstone.
                let purged: Option<Activity> = connection
                    .query_row(
                        "SELECT record FROM local_activity WHERE id=?1",
                        [format!("purged-handoff:{id}")],
                        |r| json_column(r, 0, 8192),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                if let Some(purged) = purged {
                    if purged.task_id != task.id
                        || purged.detail != handoff_fingerprint(&task.project, intent)?
                    {
                        return Err("Purged handoff receipt cannot change on retry".into());
                    }
                    continue;
                }
                validate_references(
                    task.id,
                    evidence,
                    &excerpts(connection, task.id)?,
                    &runs_for(connection, task.id)?,
                )?;
                artifact(
                    connection,
                    &task.project,
                    Artifact {
                        id: id.clone(),
                        ordinal: 0,
                        supported_diagnosis: false,
                        handoff_revision: Some(*revision),
                        task_id: task.id,
                        kind: ArtifactKind::Handoff,
                        role_form: 8,
                        producer: Some(owner.clone()),
                        reference: digest.clone(),
                        created_at: timestamp()?,
                        observation: None,
                        evidence: evidence.clone(),
                        detail: format!("Handoff ready receipt for revision {revision}"),
                    },
                )?;
                activity(
                    connection,
                    &task.project,
                    &Activity {
                        id: format!("handoff:{id}"),
                        task_id: task.id,
                        owner: Some(owner.clone()),
                        at: timestamp()?,
                        detail: if *revision == task.revision {
                            "Handoff ready receipt committed; inspect its evidence and caveats."
                                .into()
                        } else {
                            format!(
                                "Historical handoff receipt for revision {revision} committed; current task revision is {}. This is not current handoff readiness.",
                                task.revision
                            )
                        },
                    },
                )?;
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct FailureParser {
    pending_code: Option<String>,
    signatures: std::collections::BTreeSet<(String, String)>,
    paths: std::collections::BTreeSet<String>,
}
impl FailureParser {
    fn line(&mut self, line: &str) {
        let clean = crate::workspace::privacy::redact(line);
        let line = clean.trim();
        if let Some(code) = line
            .strip_prefix("error[")
            .and_then(|s| s.split_once(']').map(|p| p.0))
            && code.len() == 5
            && code.starts_with('E')
            && code[1..].bytes().all(|b| b.is_ascii_digit())
        {
            self.pending_code = Some(code.into());
        }
        let locations = crate::workspace::locations::parse([line]);
        for location in &locations {
            if self.paths.len() < 128
                && location.path.len() <= 2048
                && !self.paths.contains(&location.path)
            {
                self.paths.insert(location.path.clone());
            }
        }
        if self.signatures.len() >= 128 {
            return;
        }
        if line.starts_with("-->")
            && let (Some(code), Some(location)) = (self.pending_code.take(), locations.first())
        {
            let detail = format!("{code} at {location}");
            if detail.len() <= 2048 {
                self.signatures.insert(("rustc".into(), detail));
            }
        }
        if let Some(name) = line
            .strip_prefix("test ")
            .and_then(|s| s.strip_suffix(" ... FAILED"))
            && !name.is_empty()
            && name.len() <= 1024
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | ':' | '-' | '.'))
        {
            self.signatures.insert(("test".into(), name.into()));
        }
        if line.contains("panicked at") {
            for location in locations {
                let detail = location.to_string();
                if detail.len() <= 2048 {
                    self.signatures.insert(("panic".into(), detail));
                }
            }
        }
    }
    fn read(&mut self, mut reader: impl std::io::Read) -> Result<(), String> {
        let mut page = [0u8; 64 * 1024];
        let mut line = Vec::with_capacity(16 * 1024);
        let mut oversized = false;
        loop {
            let count = reader.read(&mut page).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            for byte in &page[..count] {
                if *byte == b'\n' {
                    if !oversized {
                        self.line(&String::from_utf8_lossy(&line));
                    }
                    line.clear();
                    oversized = false;
                } else if line.len() < 16 * 1024 && !oversized {
                    line.push(*byte);
                } else {
                    line.clear();
                    oversized = true;
                }
            }
        }
        if !oversized && !line.is_empty() {
            self.line(&String::from_utf8_lossy(&line));
        }
        Ok(())
    }
}
pub(super) fn record_run(
    connection: &Connection,
    project: &str,
    stored: &StoredRun,
) -> Result<bool, String> {
    let run = &stored.record;
    if stored.reserved
        || active_state(&run.state)
        || run.started_at_ms.is_none()
        || run.ended_at_ms.is_none()
    {
        return Err("Only committed started terminal runs can produce local records".into());
    }
    let tombstone: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM local_activity WHERE id=?1 AND project=?2)",
            params![
                format!(
                    "purged-run:{}:{}",
                    run.prepared.task_id, run.prepared.run_id
                ),
                project
            ],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if tombstone {
        return Ok(false);
    }
    let time = run.ended_at_ms.unwrap().min(i64::MAX as u64) as i64;
    let id = format!("run:{}:{}", run.prepared.task_id, run.prepared.run_id);
    let reference = workflows::EvidenceRef::run(run)?;
    let view = workflows::EvidenceView {
        task_id: run.prepared.task_id,
        excerpts: &[],
        runs: std::slice::from_ref(run),
        current: &run.after,
    };
    let observation = if view.status(&reference) == workflows::EvidenceStatus::Current {
        Some(run.after.clone())
    } else {
        None
    };
    artifact(
        connection,
        project,
        Artifact {
            id: id.clone(),
            ordinal: 0,
            supported_diagnosis: false,
            handoff_revision: None,
            task_id: run.prepared.task_id,
            kind: ArtifactKind::Run,
            role_form: 2,
            observation,
            producer: run.prepared.partner.clone(),
            reference: run.prepared.run_id.clone(),
            created_at: time,
            evidence: vec![reference],
            detail: format!(
                "Started local command; exit {:?}; state {:?}; tree {:?}. Exit alone is not correctness.",
                run.exit_code, run.state, run.tree
            ),
        },
    )?;
    let fact = if run.tree != runner::TreeOutcome::OwnedTreeExited {
        "local command has unknown process-tree ownership; no success is established"
    } else {
        match run.state {
            runner::RunState::Succeeded if run.exit_code == Some(0) && !run.cancelled => {
                "inspect coverage before treating the command as verification"
            }
            runner::RunState::Failed => "owned command failed; Umbreon can investigate",
            runner::RunState::Cancelled => "owned command was cancelled; no success is established",
            _ => "owned command has an uncertain terminal outcome; no success is established",
        }
    };
    let command = CommandLabel(&run.prepared.spec);
    let producer = run
        .prepared
        .partner
        .as_ref()
        .map_or("Unknown producer", |owner| owner.name.as_str());
    let detail = match run.exit_code {
        Some(code) => format!(
            "{producer}: exit {code}; {fact}. Run {} · {command}",
            run.prepared.run_id
        ),
        None => format!(
            "{producer}: exit unavailable; {fact}. Run {} · {command}",
            run.prepared.run_id
        ),
    };
    activity(
        connection,
        project,
        &Activity {
            id: format!("activity:{id}"),
            task_id: run.prepared.task_id,
            owner: run.prepared.partner.clone(),
            at: time,
            detail,
        },
    )?;
    Ok(true)
}

pub(super) fn observe_run(
    connection: &Connection,
    data_dir: &Path,
    project: &str,
    stored: &StoredRun,
) -> Result<(), String> {
    if !record_run(connection, project, stored)? {
        return Ok(());
    }
    let run = &stored.record;
    let time = run.ended_at_ms.unwrap().min(i64::MAX as u64) as i64;
    if run.tree != runner::TreeOutcome::OwnedTreeExited
        || run.state != runner::RunState::Failed
        || run.cancelled
        || run.output_unavailable
    {
        return Ok(());
    }
    let Ok(file) = SpoolDirectory::open(data_dir)
        .and_then(|directory| directory.read(&stored.output_name, &stored.output_identity))
    else {
        return Ok(());
    };
    if file.metadata().map_err(|e| e.to_string())?.len() != run.captured_bytes {
        return Ok(());
    }
    let mut parser = FailureParser::default();
    parser.read(file.take(run.captured_bytes))?;
    let paths = bounded_failure_paths(project, &run.prepared.cwd, parser.paths);
    let paths_json = bounded_json(&paths, 8192)?;
    for (kind, detail) in parser.signatures {
        let signature = key(project, &format!("{kind}:{detail}"));
        let value = FailureSignature {
            key: signature.clone(),
            kind,
            detail,
            first_seen: time,
            seen_count: 0,
            runs: Vec::new(),
            paths: Vec::new(),
        };
        capacity(connection, "failure_signatures", project, &signature)?;
        connection
            .execute(
                "INSERT OR IGNORE INTO failure_signatures(id,project,record) VALUES(?1,?2,?3)",
                params![signature, project, bounded_json(&value, 8192)?],
            )
            .map_err(|e| e.to_string())?;
        let observation = key(
            project,
            &format!("failure:{}:{signature}", run.prepared.run_id),
        );
        capacity(connection, "failure_observations", project, &observation)?;
        connection.execute("INSERT OR IGNORE INTO failure_observations(id,project,task_id,signature,run_id,at,paths) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![observation,project,run.prepared.task_id,signature,run.prepared.run_id,time,paths_json]).map_err(|e|e.to_string())?;
    }
    Ok(())
}
const RESOLUTION_CAUSE: &str = "Observed source transition; author/cause unknown. Historical lead, not a current fix or verification.";
struct ResolutionFacts<'a> {
    captures: &'a [crate::workspace::context::Excerpt],
    runs: &'a [runner::RunRecord],
    transitions: &'a [SourceTransition],
    observations: &'a [(String, String)],
}
fn observation_pairs(
    connection: &Connection,
    project: &str,
    task: i64,
) -> Result<Vec<(String, String)>, String> {
    let mut statement=connection.prepare("SELECT DISTINCT signature,run_id FROM failure_observations WHERE project=?1 AND task_id=?2 ORDER BY signature,run_id LIMIT 8193").map_err(|e|e.to_string())?;
    let pairs = statement
        .query_map(params![project, task], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if pairs.len() > 8192 {
        return Err("Failure observations exceed capacity".into());
    }
    Ok(pairs)
}
fn resolution_eligible(
    support: &Artifact,
    pass: &runner::RunRecord,
    signature: &str,
    facts: &ResolutionFacts<'_>,
) -> Result<(), String> {
    if support.kind != ArtifactKind::Hypothesis
        || !support.supported_diagnosis
        || support.observation.is_none()
    {
        return Err("Resolution requires a retained currently-supported diagnosis receipt".into());
    }
    if support.task_id != pass.prepared.task_id {
        return Err("Passing run belongs to another task".into());
    }
    let evidence = workflows::EvidenceRef::run(pass)?;
    let view = workflows::EvidenceView {
        task_id: support.task_id,
        excerpts: &[],
        runs: std::slice::from_ref(pass),
        current: &pass.after,
    };
    if pass.state != runner::RunState::Succeeded
        || pass.exit_code != Some(0)
        || view.status(&evidence) != workflows::EvidenceStatus::Current
        || pass.started_at_ms.unwrap_or(0) < support.created_at as u64
    {
        return Err(
            "Resolution requires a later complete retained relevant passing owned run".into(),
        );
    }
    let before = support.observation.as_ref().unwrap();
    if !before.complete
        || !before.stable
        || before.fingerprint.is_none()
        || before.fingerprint == pass.after.fingerprint
    {
        return Err(
            "Resolution requires an actual before/after source transition, not chronology alone"
                .into(),
        );
    }
    if !facts.transitions.iter().any(|transition| {
        transition.support == support.id
            && transition.at >= support.created_at
            && transition.at as u64 <= pass.started_at_ms.unwrap_or(0)
            && transition.current == pass.prepared.before
    }) {
        return Err("Resolution needs an observed changed diagnosis source before the matching passing check, not chronology alone".into());
    }
    let mut relevant = false;
    for reference in &support.evidence {
        match reference {
            workflows::EvidenceRef::Run { run_id, task_id } => {
                if *task_id != support.task_id {
                    return Err("Foreign diagnosis run".into());
                }
                let failed = facts
                    .runs
                    .iter()
                    .find(|run| run.prepared.run_id == *run_id)
                    .ok_or("Diagnosis run was removed or purged")?;
                let linked = facts
                    .observations
                    .iter()
                    .any(|(sig, run)| sig == signature && run == run_id);
                if linked
                    && failed.prepared.spec == pass.prepared.spec
                    && failed.prepared.cwd == pass.prepared.cwd
                    && failed.prepared.cwd_identity == pass.prepared.cwd_identity
                    && failed.after.fingerprint == before.fingerprint
                {
                    relevant = true;
                }
            }
            workflows::EvidenceRef::Source {
                excerpt_id,
                task_id,
                source_sha256,
                ..
            } => {
                if *task_id != support.task_id {
                    return Err("Foreign diagnosis source".into());
                }
                if !facts.captures.iter().any(|e| {
                    e.id == *excerpt_id && e.available && e.source_sha256 == *source_sha256
                }) {
                    return Err("Diagnosis source was removed or purged".into());
                }
            }
        }
    }
    if !relevant {
        return Err(
            "Resolution lacks the matching failing command identity for this signature".into(),
        );
    }
    Ok(())
}
fn resolve(
    connection: &Connection,
    project: &str,
    task: &super::Snapshot,
    signature: &str,
    support_id: &str,
    passing_id: &str,
) -> Result<(), String> {
    identity(signature)?;
    identity(support_id)?;
    identity(passing_id)?;
    let support: Artifact = connection
        .query_row(
            "SELECT record FROM local_artifacts WHERE id=?1 AND project=?2 AND task_id=?3",
            params![support_id, project, task.id],
            |r| json_column(r, 0, MAX_RECORD),
        )
        .map_err(|_| "Supported diagnosis receipt is missing or foreign")?;
    let pass = stored_run(connection, task.id, passing_id)?.record;
    let runs = runs_for(connection, task.id)?;
    let captures = excerpts(connection, task.id)?;
    let transitions = rows(connection, "diagnosis_transitions", project)?;
    let observations = observation_pairs(connection, project, task.id)?;
    resolution_eligible(
        &support,
        &pass,
        signature,
        &ResolutionFacts {
            captures: &captures,
            runs: &runs,
            transitions: &transitions,
            observations: &observations,
        },
    )?;
    let value = Resolution {
        signature: signature.into(),
        task_id: task.id,
        support_artifact: support_id.into(),
        passing_run: passing_id.into(),
        at: timestamp()?,
        cause: RESOLUTION_CAUSE.into(),
    };
    let id = key(
        project,
        &format!("resolution:{signature}:{support_id}:{passing_id}"),
    );
    let existing: Option<Resolution> = connection
        .query_row(
            "SELECT record FROM failure_resolutions WHERE id=?1",
            [&id],
            |r| json_column(r, 0, 8192),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if existing.is_none() {
        put(
            connection,
            "failure_resolutions",
            &id,
            project,
            task.id,
            &value,
        )?;
    }
    Ok(())
}

pub(super) fn purge_task(connection: &Connection, project: &str, task: i64) -> Result<(), String> {
    for table in [
        "diagnosis_transitions",
        "local_artifacts",
        "form_routes",
        "local_activity",
        "failure_observations",
        "failure_resolutions",
        "instruction_sources",
        "provider_usage",
        "provider_usage_events",
    ] {
        connection
            .execute(
                &format!("DELETE FROM {table} WHERE project=?1 AND task_id=?2"),
                params![project, task],
            )
            .map_err(|e| e.to_string())?;
    }
    prune(connection, project)
}
pub(super) fn purge_project(connection: &Connection, project: &str) -> Result<(), String> {
    for table in [
        "diagnosis_transitions",
        "local_artifacts",
        "form_routes",
        "local_activity",
        "local_settings",
        "failure_signatures",
        "failure_observations",
        "failure_resolutions",
        "instruction_candidates",
        "instruction_sources",
        "partner_pairings",
        "provider_usage",
        "provider_usage_events",
    ] {
        connection
            .execute(&format!("DELETE FROM {table} WHERE project=?1"), [project])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn prune(connection: &Connection, project: &str) -> Result<(), String> {
    connection.execute("DELETE FROM failure_signatures WHERE project=?1 AND NOT EXISTS(SELECT 1 FROM failure_observations o WHERE o.signature=failure_signatures.id AND o.project=?1)",[project]).map_err(|e|e.to_string())?;
    for mut habit in rows::<Habit>(connection, "instruction_candidates", project)? {
        let remaining:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM instruction_sources WHERE project=?1 AND signature=?2)",params![project,habit.key],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !remaining {
            if habit.dismissed {
                habit.instruction.clear();
                habit.memory = None;
                connection
                    .execute(
                        "UPDATE instruction_candidates SET record=?1 WHERE id=?2 AND project=?3",
                        params![
                            bounded_json(&habit, MAX_RECORD)?,
                            key(project, &habit.key),
                            project
                        ],
                    )
                    .map_err(|e| e.to_string())?;
            } else {
                connection
                    .execute(
                        "DELETE FROM instruction_candidates WHERE id=?1 AND project=?2",
                        params![key(project, &habit.key), project],
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}
fn purge_related(
    connection: &Connection,
    project: &str,
    task: i64,
    source: Option<i64>,
    run: Option<&str>,
    provider: bool,
) -> Result<(), String> {
    let artifacts: Vec<Artifact> = rows(connection, "local_artifacts", project)?;
    for artifact in artifacts.into_iter().filter(|a| a.task_id == task) {
        let related = provider && artifact.kind == ArtifactKind::Patch
            || source.is_some_and(|id| {
                artifact.kind == ArtifactKind::Capture && artifact.reference == id.to_string()
            })
            || run.is_some_and(|id| artifact.kind == ArtifactKind::Run && artifact.reference == id)
            || artifact.evidence.iter().any(|reference| match reference {
                workflows::EvidenceRef::Source { excerpt_id, .. } => source == Some(*excerpt_id),
                workflows::EvidenceRef::Run { run_id, .. } => run == Some(run_id.as_str()),
            });
        if related {
            if artifact.kind == ArtifactKind::Handoff
                && let (Some(revision), Some(owner)) =
                    (artifact.handoff_revision, artifact.producer.as_ref())
            {
                // Reuse the receipt activity's slot even when the table is full.
                connection
                    .execute(
                        "DELETE FROM local_activity WHERE id=?1 AND project=?2",
                        params![format!("handoff:{}", artifact.id), project],
                    )
                    .map_err(|e| e.to_string())?;
                let intent = LocalIntent::HandoffReady {
                    id: artifact.id.clone(),
                    owner: owner.clone(),
                    revision,
                    digest: artifact.reference.clone(),
                    evidence: artifact.evidence.clone(),
                };
                let marker = Activity {
                    id: format!("purged-handoff:{}", artifact.id),
                    task_id: task,
                    owner: None,
                    at: timestamp()?,
                    detail: handoff_fingerprint(project, &intent)?,
                };
                activity(connection, project, &marker)?;
                connection
                    .execute(
                        "UPDATE local_activity SET delivered=1 WHERE id=?1 AND project=?2",
                        params![marker.id, project],
                    )
                    .map_err(|e| e.to_string())?;
            }
            connection.execute("DELETE FROM failure_resolutions WHERE project=?1 AND task_id=?2 AND json_extract(record,'$.support_artifact')=?3",params![project,task,artifact.id]).map_err(|e|e.to_string())?;
            connection.execute("DELETE FROM diagnosis_transitions WHERE project=?1 AND task_id=?2 AND json_extract(record,'$.support')=?3",params![project,task,artifact.id]).map_err(|e|e.to_string())?;
            if artifact.kind == ArtifactKind::Review {
                connection.execute("UPDATE local_activity SET delivered=1 WHERE project=?1 AND task_id=?2 AND id LIKE ?3",params![project,task,format!("review:{task}:%")]).map_err(|e|e.to_string())?;
            }
            connection.execute("DELETE FROM local_activity WHERE project=?1 AND task_id=?2 AND (id=?3 OR id=?4 OR id=?5 OR id=?6)",params![project,task,format!("activity:{}",artifact.id),format!("stale:{}",artifact.id),format!("review:{}",artifact.id),format!("handoff:{}",artifact.id)]).map_err(|e|e.to_string())?;
            connection
                .execute(
                    "DELETE FROM local_artifacts WHERE id=?1 AND project=?2",
                    params![artifact.id, project],
                )
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
pub(super) fn purge_capture(
    connection: &Connection,
    project: &str,
    task: i64,
    id: i64,
) -> Result<(), String> {
    purge_related(connection, project, task, Some(id), None, false)
}
pub(super) fn purge_run(
    connection: &Connection,
    project: &str,
    task: i64,
    run: &str,
) -> Result<(), String> {
    purge_related(connection, project, task, None, Some(run), false)?;
    connection
        .execute(
            "DELETE FROM failure_observations WHERE project=?1 AND task_id=?2 AND run_id=?3",
            params![project, task, run],
        )
        .map_err(|e| e.to_string())?;
    connection.execute("DELETE FROM failure_resolutions WHERE project=?1 AND task_id=?2 AND json_extract(record,'$.passing_run')=?3",params![project,task,run]).map_err(|e|e.to_string())?;
    let marker = Activity {
        id: format!("purged-run:{task}:{run}"),
        task_id: task,
        owner: None,
        at: timestamp()?,
        detail: "Output reference purged; never re-observe.".into(),
    };
    activity(connection, project, &marker)?;
    connection
        .execute(
            "UPDATE local_activity SET delivered=1 WHERE id=?1",
            [marker.id],
        )
        .map_err(|e| e.to_string())?;
    prune(connection, project)
}
pub(super) fn purge_provider(
    connection: &Connection,
    project: &str,
    task: i64,
) -> Result<(), String> {
    purge_related(connection, project, task, None, None, true)?;
    for table in [
        "instruction_sources",
        "provider_usage",
        "provider_usage_events",
    ] {
        connection
            .execute(
                &format!("DELETE FROM {table} WHERE project=?1 AND task_id=?2"),
                params![project, task],
            )
            .map_err(|e| e.to_string())?;
    }
    prune(connection, project)
}

pub(super) fn inspect(connection: &Connection) -> Result<(), String> {
    let common: &[ColumnSpec] = &[
        ("id", "TEXT", false, 1),
        ("project", "TEXT", true, 0),
        ("task_id", "INTEGER", true, 0),
        ("record", "TEXT", true, 0),
    ];
    for table in [
        "diagnosis_transitions",
        "local_artifacts",
        "form_routes",
        "failure_resolutions",
        "provider_usage",
    ] {
        inspect_columns(connection, table, common)?;
    }
    inspect_columns(
        connection,
        "local_activity",
        &[
            ("id", "TEXT", false, 1),
            ("project", "TEXT", true, 0),
            ("task_id", "INTEGER", true, 0),
            ("record", "TEXT", true, 0),
            ("delivered", "INTEGER", true, 0),
        ],
    )?;
    inspect_columns(
        connection,
        "local_settings",
        &[
            ("project", "TEXT", false, 1),
            ("flavor", "INTEGER", true, 0),
            ("sequence", "INTEGER", true, 0),
        ],
    )?;
    for table in ["failure_signatures", "instruction_candidates"] {
        inspect_columns(
            connection,
            table,
            &[
                ("id", "TEXT", false, 1),
                ("project", "TEXT", true, 0),
                ("record", "TEXT", true, 0),
            ],
        )?;
    }
    inspect_columns(
        connection,
        "failure_observations",
        &[
            ("id", "TEXT", false, 1),
            ("project", "TEXT", true, 0),
            ("task_id", "INTEGER", true, 0),
            ("signature", "TEXT", true, 0),
            ("run_id", "TEXT", true, 0),
            ("at", "INTEGER", true, 0),
            ("paths", "TEXT", true, 0),
        ],
    )?;
    inspect_columns(
        connection,
        "instruction_sources",
        &[
            ("id", "TEXT", false, 1),
            ("project", "TEXT", true, 0),
            ("task_id", "INTEGER", true, 0),
            ("signature", "TEXT", true, 0),
            ("record", "TEXT", true, 0),
        ],
    )?;
    inspect_columns(
        connection,
        "partner_pairings",
        &[
            ("id", "TEXT", false, 1),
            ("project", "TEXT", true, 0),
            ("partner", "INTEGER", true, 0),
            ("form", "INTEGER", true, 0),
            ("record", "TEXT", true, 0),
        ],
    )?;
    inspect_columns(
        connection,
        "provider_usage_events",
        &[
            ("id", "TEXT", false, 1),
            ("project", "TEXT", true, 0),
            ("task_id", "INTEGER", true, 0),
            ("owner", "TEXT", true, 0),
            ("agent", "TEXT", true, 0),
        ],
    )?;
    for table in [
        "diagnosis_transitions",
        "local_artifacts",
        "form_routes",
        "local_activity",
        "local_settings",
        "failure_signatures",
        "failure_observations",
        "failure_resolutions",
        "instruction_candidates",
        "instruction_sources",
        "partner_pairings",
        "provider_usage",
        "provider_usage_events",
    ] {
        let invalid:bool=connection.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE length(CAST(project AS BLOB)) NOT BETWEEN 1 AND 4096) OR EXISTS(SELECT 1 FROM {table} GROUP BY project HAVING count(*)>8192)"),[],|r|r.get(0)).map_err(|e|e.to_string())?;
        if invalid {
            return Err(format!("Invalid {table} project identity/capacity"));
        }
    }
    for table in [
        "diagnosis_transitions",
        "local_artifacts",
        "form_routes",
        "local_activity",
        "failure_resolutions",
        "failure_observations",
        "instruction_sources",
        "provider_usage",
        "provider_usage_events",
    ] {
        let orphan:bool=connection.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table} r LEFT JOIN tasks t ON t.id=r.task_id AND t.project=r.project WHERE t.id IS NULL)"),[],|r|r.get(0)).map_err(|e|e.to_string())?;
        if orphan {
            return Err(format!("Foreign or missing {table} task owner"));
        }
    }
    for table in [
        "diagnosis_transitions",
        "local_artifacts",
        "form_routes",
        "local_activity",
        "failure_resolutions",
        "instruction_sources",
        "provider_usage",
    ] {
        let mismatch:bool=connection.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE json_extract(record,'$.task_id') IS NOT task_id)"),[],|r|r.get(0)).map_err(|e|e.to_string())?;
        if mismatch {
            return Err(format!("{table} row/JSON task identities differ"));
        }
    }
    let invalid:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM failure_observations o LEFT JOIN failure_signatures s ON s.id=o.signature AND s.project=o.project LEFT JOIN executions e ON e.task_id=o.task_id AND e.run_id=o.run_id WHERE s.id IS NULL OR e.run_id IS NULL OR o.at<0)
OR EXISTS(SELECT 1 FROM instruction_sources o LEFT JOIN instruction_candidates c ON c.project=o.project AND json_extract(c.record,'$.key')=o.signature WHERE c.id IS NULL)
OR EXISTS(SELECT 1 FROM partner_pairings p LEFT JOIN partners o ON o.id=p.partner WHERE o.id IS NULL OR p.form<0 OR p.form>=9)
OR EXISTS(SELECT 1 FROM provider_usage_events e LEFT JOIN provider_usage u ON u.id=e.id AND u.project=e.project AND u.task_id=e.task_id WHERE u.id IS NULL)
OR EXISTS(SELECT 1 FROM local_settings WHERE flavor NOT IN(0,1) OR sequence<0)
OR EXISTS(SELECT 1 FROM local_activity WHERE delivered NOT IN(0,1))",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if invalid {
        return Err(
            "Invalid records relation, setting, event delivery or pairing ownership".into(),
        );
    }
    let mut projects=connection.prepare("SELECT project FROM tasks UNION SELECT project FROM local_settings UNION SELECT project FROM local_artifacts UNION SELECT project FROM instruction_candidates UNION SELECT project FROM partner_pairings UNION SELECT project FROM provider_usage UNION SELECT project FROM failure_signatures").map_err(|e|e.to_string())?;
    for project in projects
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
    {
        let project = project.map_err(|e| e.to_string())?;
        reject_secret_identity(&project)?;
        let sequence: u64 = connection
            .query_row(
                "SELECT sequence FROM local_settings WHERE project=?1",
                [&project],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or(0);
        let mut ordinals = std::collections::BTreeSet::new();
        for a in rows::<Artifact>(connection, "local_artifacts", &project)? {
            identity(&a.id)?;
            identity(&a.reference)?;
            if a.task_id <= 0
                || a.role_form >= 9
                || a.created_at < 0
                || a.ordinal == 0
                || a.ordinal > sequence
                || !ordinals.insert(a.ordinal)
                || a.evidence.len() > 128
                || a.detail.len() > 65536
                || crate::workspace::privacy::redact(&a.detail) != a.detail
            {
                return Err("Invalid artifact metadata, ordinal or masking".into());
            }
            if let Some(producer) = &a.producer {
                producer.validate()?;
            }
            if a.supported_diagnosis
                && (a.kind != ArtifactKind::Hypothesis
                    || a.observation
                        .as_ref()
                        .is_none_or(|o| !o.complete || !o.stable || o.fingerprint.is_none()))
            {
                return Err("Invalid supported diagnosis receipt".into());
            }
            let json_id:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM local_artifacts WHERE id=?1 AND project=?2 AND task_id=?3)",params![a.id,project,a.task_id],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !json_id {
                return Err("Artifact row and JSON ownership differ".into());
            }
            for evidence in &a.evidence {
                match evidence {
                    workflows::EvidenceRef::Run { task_id, run_id } => {
                        if *task_id != a.task_id || !valid_run_id(run_id) {
                            return Err("Invalid artifact run reference".into());
                        }
                    }
                    workflows::EvidenceRef::Source {
                        task_id,
                        excerpt_id,
                        start_line,
                        end_line,
                        ..
                    } => {
                        if *task_id != a.task_id
                            || *excerpt_id <= 0
                            || *start_line == 0
                            || end_line < start_line
                        {
                            return Err("Invalid artifact source reference".into());
                        }
                    }
                }
            }
        }
        for r in rows::<Route>(connection, "form_routes", &project)? {
            identity(&r.id)?;
            r.owner.validate()?;
            if r.from >= 9
                || r.to >= 9
                || r.owner.form != r.to
                || r.at < 0
                || r.ordinal == 0
                || r.ordinal > sequence
                || !ordinals.insert(r.ordinal)
            {
                return Err("Invalid route metadata or ordering".into());
            }
            if let Some(trigger) = r.accepted_suggestion {
                identity(&trigger)?;
            }
            let matches:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM form_routes WHERE id=?1 AND task_id=?2 AND project=?3)",params![r.id,r.task_id,project],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !matches {
                return Err("Route row/record ownership differs".into());
            }
        }
        for a in rows::<Activity>(connection, "local_activity", &project)? {
            identity(&a.id)?;
            if let Some(owner) = a.owner {
                owner.validate()?;
            }
            if a.at < 0 || crate::workspace::privacy::redact(&a.detail) != a.detail {
                return Err("Invalid activity metadata".into());
            }
        }
        for h in rows::<Habit>(connection, "instruction_candidates", &project)? {
            identity(&h.key)?;
            let matches:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM instruction_candidates WHERE id=?1 AND project=?2)",params![key(&project,&h.key),project],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !matches {
                return Err("Instruction candidate row/hash scope differs".into());
            }
            if h.instruction.is_empty() {
                if !h.dismissed || h.memory.is_some() {
                    return Err("Invalid instruction dismissal tombstone".into());
                }
            } else {
                let (hash, normalized) = instruction_key(&h.instruction)?;
                if h.key != hash || normalized != h.instruction {
                    return Err("Invalid stored instruction normalization/hash".into());
                }
            }
            if !h.sources.is_empty() {
                return Err("Stored instruction candidate cannot copy source records".into());
            }
            if h.memory
                .as_ref()
                .is_some_and(|m| m.id <= 0 || m.revision == 0)
            {
                return Err("Invalid habit memory revision".into());
            }
        }
        for s in rows::<InstructionSource>(connection, "instruction_sources", &project)? {
            identity(&s.request_id)?;
            s.owner.validate()?;
            if s.at < 0 {
                return Err("Invalid instruction source time".into());
            }
            let matches:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM instruction_sources WHERE id=?1 AND project=?2 AND task_id=?3)",params![key(&project,&format!("instruction:{}",s.request_id)),project,s.task_id],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !matches {
                return Err("Instruction source row/request identity differs".into());
            }
        }
        for p in rows::<Pairing>(connection, "partner_pairings", &project)? {
            if p.partner <= 0 || p.form >= 9 {
                return Err("Invalid pairing metadata".into());
            }
            for value in [&p.selection.model, &p.selection.effort]
                .into_iter()
                .flatten()
            {
                identity(value)?;
            }
            let matches:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM partner_pairings WHERE project=?1 AND partner=?2 AND form=?3 AND id=?4)",params![project,p.partner,p.form as i64,key(&project,&format!("pairing:{}:{}",p.partner,p.form))],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !matches {
                return Err("Pairing row/record scope differs".into());
            }
        }
        for u in rows::<crate::workspace::usage::Record>(connection, "provider_usage", &project)? {
            u.validate()?;
            u.owner.validate()?;
            let id = key(&project, &format!("usage:{}", u.request_id));
            let origin:Option<(Attribution,crate::providers::Agent)>=connection.query_row("SELECT owner,agent FROM provider_usage_events WHERE id=?1 AND project=?2 AND task_id=?3",params![id,project,u.task_id],|r|Ok((json_column(r,0,2048)?,json_column(r,1,128)?))).optional().map_err(|e|e.to_string())?;
            if origin != Some((u.owner, u.agent)) {
                return Err("Usage origin does not match its durable submission receipt".into());
            }
        }
        for s in rows::<FailureSignature>(connection, "failure_signatures", &project)? {
            identity(&s.key)?;
            identity(&s.kind)?;
            if s.key != key(&project, &format!("{}:{}", s.kind, s.detail))
                || s.first_seen < 0
                || s.detail.len() > 2048
                || crate::workspace::privacy::redact(&s.detail) != s.detail
                || s.seen_count != 0
                || !s.runs.is_empty()
                || !s.paths.is_empty()
            {
                return Err("Invalid failure signature or copied derived metadata".into());
            }
        }
        let mut observations = connection
            .prepare("SELECT id,signature,run_id,paths FROM failure_observations WHERE project=?1")
            .map_err(|e| e.to_string())?;
        for row in observations
            .query_map([&project], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    json_column::<Vec<String>>(r, 3, 8192)?,
                ))
            })
            .map_err(|e| e.to_string())?
        {
            let (id, signature, run, paths) = row.map_err(|e| e.to_string())?;
            if id != key(&project, &format!("failure:{run}:{signature}"))
                || paths.len() > 64
                || paths.iter().map(String::len).sum::<usize>() > 3000
                || paths.iter().any(|p| {
                    p.is_empty()
                        || p.len() > 1024
                        || Path::new(p).is_absolute()
                        || crate::workspace::context::excluded(Path::new(p), &[])
                        || crate::workspace::privacy::redact(p) != p.as_str()
                })
            {
                return Err("Invalid failure location reference metadata".into());
            }
        }
        for transition in rows::<SourceTransition>(connection, "diagnosis_transitions", &project)? {
            identity(&transition.support)?;
            if transition.at < 0
                || !transition.current.complete
                || !transition.current.stable
                || transition.current.fingerprint.is_none()
            {
                return Err("Invalid source transition receipt".into());
            }
            let exists:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM local_artifacts WHERE id=?1 AND project=?2 AND task_id=?3 AND json_extract(record,'$.supported_diagnosis')=1)",params![transition.support,project,transition.task_id],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !exists {
                return Err(
                    "Source transition diagnosis was removed or belongs to another owner".into(),
                );
            }
        }
        for r in rows::<Resolution>(connection, "failure_resolutions", &project)? {
            identity(&r.signature)?;
            identity(&r.support_artifact)?;
            identity(&r.passing_run)?;
            if r.at < 0
                || r.cause.len() > 4096
                || crate::workspace::privacy::redact(&r.cause) != r.cause
            {
                return Err("Invalid historical resolution".into());
            }
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct SourceTransition {
    task_id: i64,
    support: String,
    current: runner::InputSnapshot,
    at: i64,
}
fn observe_transition(
    connection: &Connection,
    project: &str,
    support: &Artifact,
    current: &runner::InputSnapshot,
) -> Result<bool, String> {
    if !support.supported_diagnosis
        || support.kind != ArtifactKind::Hypothesis
        || !current.complete
        || !current.stable
        || current.fingerprint.is_none()
        || support
            .observation
            .as_ref()
            .is_none_or(|before| before.fingerprint == current.fingerprint)
    {
        return Ok(false);
    }
    let id = key(
        project,
        &format!(
            "transition:{}:{}",
            support.id,
            current.fingerprint.as_ref().unwrap()
        ),
    );
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM diagnosis_transitions WHERE id=?1 AND project=?2)",
            params![id, project],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists {
        return Ok(true);
    }
    let captures = excerpts(connection, support.task_id)?;
    let view = workflows::EvidenceView {
        task_id: support.task_id,
        excerpts: &captures,
        runs: &[],
        current,
    };
    let changed = support.evidence.iter().any(|reference| {
        matches!(reference, workflows::EvidenceRef::Source { .. })
            && view.status(reference) == workflows::EvidenceStatus::Stale
    });
    if !changed {
        return Ok(false);
    }
    let transition = SourceTransition {
        task_id: support.task_id,
        support: support.id.clone(),
        current: current.clone(),
        at: timestamp()?,
    };
    put(
        connection,
        "diagnosis_transitions",
        &id,
        project,
        support.task_id,
        &transition,
    )?;
    Ok(true)
}

// Storage accepts unknown -> reported metadata, never deletion or reassignment.
fn metadata_retained(old: &Option<String>, new: &Option<String>) -> bool {
    old.as_ref().is_none_or(|value| new.as_ref() == Some(value))
}

fn handoff_fingerprint(project: &str, intent: &LocalIntent) -> Result<String, String> {
    Ok(key(project, &bounded_json(intent, MAX_RECORD)?))
}

fn counters_retained(
    old: &crate::workspace::usage::Counters,
    new: &crate::workspace::usage::Counters,
) -> bool {
    (old.input.is_none() || new.input.is_some())
        && (old.output.is_none() || new.output.is_some())
        && (old.cache_read.is_none() || new.cache_read.is_some())
        && (old.cache_write.is_none() || new.cache_write.is_some())
        && (old.reasoning.is_none() || new.reasoning.is_some())
}
fn report_retained(
    old: &crate::workspace::usage::Report,
    new: &crate::workspace::usage::Report,
) -> bool {
    counters_retained(&old.counters, &new.counters)
        && (!old.complete || new.complete)
        && old.baseline.as_ref().is_none_or(|base| {
            new.baseline
                .as_ref()
                .is_some_and(|next| counters_retained(base, next))
        })
        && (old.semantics.cache_read_in_input.is_none()
            || new.semantics.cache_read_in_input.is_some())
        && (old.semantics.cache_write_in_input.is_none()
            || new.semantics.cache_write_in_input.is_some())
        && (old.semantics.reasoning_in_output.is_none()
            || new.semantics.reasoning_in_output.is_some())
}
fn capacity(connection: &Connection, table: &str, project: &str, id: &str) -> Result<(), String> {
    let exists: bool = connection
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id=?1)"),
            [id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists {
        return Ok(());
    }
    let count: i64 = connection
        .query_row(
            &format!("SELECT count(*) FROM {table} WHERE project=?1"),
            [project],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let slots = match table {
        "failure_signatures" | "failure_observations" => 128,
        "local_artifacts" | "local_activity" => 1,
        _ => 0,
    };
    let reserved: i64 = if slots == 0 {
        0
    } else {
        connection.query_row("SELECT count(*) FROM executions e JOIN tasks t ON t.id=e.task_id WHERE t.project=?1 AND e.reserved=1",[project],|r|r.get(0)).map_err(|e|e.to_string())?
    };
    if count + reserved * slots >= MAX_PROJECT_RECORDS {
        return Err(format!(
            "Local record capacity reached ({table}); explicitly purge before adding records"
        ));
    }
    Ok(())
}
pub(super) fn reserve_capacity(connection: &Connection, project: &str) -> Result<(), String> {
    let reserved:i64=connection.query_row("SELECT count(*) FROM executions e JOIN tasks t ON t.id=e.task_id WHERE t.project=?1 AND e.reserved=1",[project],|r|r.get(0)).map_err(|e|e.to_string())?;
    for (table, slots) in [
        ("failure_signatures", 128),
        ("failure_observations", 128),
        ("local_artifacts", 1),
        ("local_activity", 1),
    ] {
        let count: i64 = connection
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE project=?1"),
                [project],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if count + (reserved + 1) * slots > MAX_PROJECT_RECORDS {
            return Err("Command record capacity unavailable; purge retained records before approving another run".into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "records_tests.rs"]
mod tests;

pub(super) fn observe_provider(
    connection: &Connection,
    project: &str,
    task: i64,
    record: &ProviderRecord,
) -> Result<(), String> {
    if workflows::controlled_patch_author(record).is_none() {
        return Ok(());
    }
    let Some(owner) = &record.partner else {
        return Ok(());
    };
    let id = key(project, &format!("patch:{task}:{}", record.id));
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM local_artifacts WHERE id=?1 AND project=?2)",
            params![id, project],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists {
        return Ok(());
    }
    artifact(connection,project,Artifact{id,ordinal:0,supported_diagnosis:false,handoff_revision:None,task_id:task,kind:ArtifactKind::Patch,role_form:if owner.form==6{6}else{3},producer:Some(owner.clone()),reference:record.id.clone(),created_at:timestamp()?,observation:None,evidence:Vec::new(),detail:"Actual committed Eevee-controlled patch journal; source identity is revalidated separately, never a correctness verdict.".into()})
}
fn patch_status(
    connection: &Connection,
    project: &str,
    task: i64,
    id: &str,
) -> Result<(Freshness, String), String> {
    let record: Option<ProviderRecord> = connection
        .query_row(
            "SELECT record FROM provider_journal WHERE id=?1 AND task_id=?2",
            params![id, task],
            |r| json_column(r, 0, 65536),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(author) = record.as_ref().and_then(workflows::controlled_patch_author) else {
        return Ok((
            Freshness::Unavailable,
            "Controlled patch provenance is missing or unavailable".into(),
        ));
    };
    let result = crate::workspace::repo::read_source(
        Path::new(project),
        Path::new(&author.path),
        crate::workspace::context::MAX_SOURCE_BYTES,
        &std::sync::atomic::AtomicBool::new(false),
    );
    Ok(match result {
        Ok((_, hash)) if hash == author.source_sha256 => (
            Freshness::Current,
            "Actual controlled patch source identity remains current; this is not verification."
                .into(),
        ),
        Ok(_) => (
            Freshness::Stale,
            "Source differs from the committed controlled patch identity.".into(),
        ),
        Err(_) => (
            Freshness::Unavailable,
            "Controlled patch source identity cannot be read.".into(),
        ),
    })
}

fn decision_reference(task: &super::Snapshot) -> Option<String> {
    use sha2::{Digest, Sha256};
    let decision = task.workflows.chosen_decision.as_ref()?;
    if decision.text != task.next_action {
        return None;
    }
    let reference = format!("decision:{:x}", Sha256::digest(task.next_action.as_bytes()));
    (reference == decision.id).then_some(reference)
}

fn validate_references(
    task: i64,
    evidence: &[workflows::EvidenceRef],
    captures: &[crate::workspace::context::Excerpt],
    runs: &[runner::RunRecord],
) -> Result<(), String> {
    for reference in evidence {
        match reference {
            workflows::EvidenceRef::Source {
                task_id,
                excerpt_id,
                path,
                start_line,
                end_line,
                source_sha256,
            } => {
                if *task_id != task
                    || !captures.iter().any(|e| {
                        e.id == *excerpt_id
                            && e.available
                            && e.path == *path
                            && e.start_line == *start_line
                            && e.end_line == *end_line
                            && e.source_sha256 == *source_sha256
                    })
                {
                    return Err(
                        "Source reference is missing, purged, stale-identity or foreign".into(),
                    );
                }
            }
            workflows::EvidenceRef::Run { task_id, run_id } => {
                if *task_id != task
                    || !runs
                        .iter()
                        .any(|r| r.prepared.run_id == *run_id && !r.output_unavailable)
                {
                    return Err("Run reference is missing, purged, unavailable or foreign".into());
                }
            }
        }
    }
    Ok(())
}

fn bounded_failure_paths(
    project: &str,
    cwd: &Path,
    raw: std::collections::BTreeSet<String>,
) -> Vec<String> {
    use std::path::Component;
    let root = Path::new(project);
    let mut paths = std::collections::BTreeSet::new();
    let mut bytes = 0;
    for path in raw {
        let candidate = Path::new(&path);
        let combined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            cwd.join(candidate)
        };
        let Ok(relative) = combined.strip_prefix(root) else {
            continue;
        };
        let mut normalized = PathBuf::new();
        let mut contained = true;
        for component in relative.components() {
            match component {
                Component::Normal(part) => normalized.push(part),
                Component::CurDir => {}
                Component::ParentDir => {
                    if !normalized.pop() {
                        contained = false;
                        break;
                    }
                }
                _ => {
                    contained = false;
                    break;
                }
            }
        }
        let Some(path) = normalized.to_str() else {
            continue;
        };
        if !contained
            || path.is_empty()
            || path.len() > 1024
            || crate::workspace::context::excluded(&normalized, &[])
            || crate::workspace::privacy::redact(path) != path
        {
            continue;
        }
        if paths.contains(path) {
            continue;
        }
        if paths.len() == 64 || bytes + path.len() > 3000 {
            continue;
        }
        bytes += path.len();
        paths.insert(path.to_owned());
    }
    paths.into_iter().collect()
}

#[cfg(test)]
pub(super) fn remove_schema(connection: &Connection) {
    connection.execute_batch("DROP TABLE diagnosis_transitions;DROP TABLE local_artifacts;DROP TABLE form_routes;DROP TABLE local_activity;DROP TABLE local_settings;DROP TABLE failure_signatures;DROP TABLE failure_observations;DROP TABLE failure_resolutions;DROP TABLE instruction_candidates;DROP TABLE instruction_sources;DROP TABLE partner_pairings;DROP TABLE provider_usage;DROP TABLE provider_usage_events;PRAGMA user_version=14;").unwrap();
}

struct CommandLabel<'a>(&'a runner::CommandSpec);
impl std::fmt::Display for CommandLabel<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn parts<'a>(
            f: &mut std::fmt::Formatter<'_>,
            values: impl Iterator<Item = &'a str>,
        ) -> std::fmt::Result {
            let mut remaining = 160;
            let mut first = true;
            for value in values {
                if !first {
                    if remaining == 0 {
                        return f.write_str("...");
                    }
                    f.write_str(" ")?;
                    remaining -= 1;
                }
                first = false;
                for ch in value.chars() {
                    if ch.is_control()
                        || matches!(ch,'\u{061c}'|'\u{200e}'|'\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')
                    {
                        for escaped in ch.escape_default() {
                            if remaining == 0 {
                                return f.write_str("...");
                            }
                            let mut bytes = [0; 4];
                            f.write_str(escaped.encode_utf8(&mut bytes))?;
                            remaining -= 1;
                        }
                        continue;
                    }
                    if ch.len_utf8() > remaining {
                        return f.write_str("...");
                    }
                    let mut bytes = [0; 4];
                    f.write_str(ch.encode_utf8(&mut bytes))?;
                    remaining -= ch.len_utf8();
                }
            }
            Ok(())
        }
        match self.0 {
            runner::CommandSpec::Direct { program, args } => parts(
                f,
                std::iter::once(program.as_str()).chain(args.iter().map(String::as_str)),
            ),
            runner::CommandSpec::Shell { executable, .. } => parts(
                f,
                [executable.as_str(), "[reviewed shell request]"].into_iter(),
            ),
        }
    }
}
