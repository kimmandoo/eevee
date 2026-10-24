//! Bounded local moves. DispatchPins is a runtime submission receipt, not an
//! external account assertion or a grant; ObserveRun reads only local journals.
//!
//! Load backfills safe attributed command history without acknowledging an offer.
//! The idle UI acknowledges the actually opened prompt with Decision::Offered;
//! offered candidates remain reminders. NotNow requires three distinct further
//! sources, while Never is a durable tombstone. Names never enter actions.
//! Project reminders survive individual task purge with exact retained evidence,
//! including zero sources. Offer thresholds discount deleted observations (floor
//! three), while the approved steps and Offered/NotNow/Never decisions survive.
//!
//! Slots count project plus personal moves in every project, including unseen
//! projects. All changes, replacement forgets, observations and quota checks share
//! the existing worker transaction: no eviction and no partial replacement.
//! Usage command outcomes/revisions resolve from the retained local journal;
//! exit zero remains an observed outcome, never correctness. Provider submission
//! receipts and consent denials are separate local facts, not tool evidence.
use super::*;
use crate::workspace::moves::{
    self as domain, Candidate, Outcome, Pin, Proposal, Record, Scope, Steps, Usage,
};
use unicode_normalization::UnicodeNormalization;

const MAX_RECORDS: usize = 512;
const MAX_CANDIDATES: usize = 512;
const MAX_SOURCES: usize = 16_384;
const MAX_USAGES: usize = 8192;
const MAX_JSON: usize = 64 * 1024;
const RECORD_COLUMNS: &str = "id,revision,project,proposal,forgotten,created";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub task_id: i64,
    pub partner: i64,
    pub form: usize,
    pub operation: Operation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    NotNow,
    Never,
    Offered,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Load,
    ReadPortable {
        path: String,
    },
    ExportPortable {
        id: i64,
        revision: u64,
        path: String,
    },
    ImportPortable {
        definition: domain::portable::Definition,
        name: String,
        scope: Scope,
        replace: Option<(i64, u64)>,
    },
    Validate {
        id: i64,
        revision: u64,
    },
    Teach {
        proposal: Proposal,
        replace: Option<(i64, u64)>,
    },
    Copy {
        proposal: Proposal,
        source: (i64, u64),
        replace: Option<(i64, u64)>,
    },
    Rename {
        id: i64,
        revision: u64,
        name: String,
    },
    Forget {
        id: i64,
        revision: u64,
    },
    Relearn {
        id: i64,
        revision: u64,
        replace: Option<(i64, u64)>,
    },
    ObserveRun {
        run_id: String,
    },
    /// Called only after correlated actual provider submission evidence.
    DispatchPins {
        request_id: String,
        pins: Vec<Pin>,
    },
    CandidateDecision {
        key: String,
        decision: Decision,
    },
    Use {
        usage: Usage,
    },
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    pub records: Vec<Record>,
    pub candidates: Vec<Candidate>,
    pub usages: Vec<Usage>,
    pub portable: Option<domain::portable::Definition>,
}

/// Shared Unicode caseless key. Upper/lower expansion includes sharp-s,
/// ligatures and final sigma; dotless i is a distinct default Unicode fold.
pub fn name_key(name: &str) -> String {
    let mut key = String::with_capacity(name.len());
    for ch in name.nfkc() {
        if ch == '\u{0131}' {
            key.push(ch);
        } else {
            for upper in ch.to_uppercase() {
                key.extend(upper.to_lowercase());
            }
        }
    }
    key
}

pub(super) fn migrate(connection: &Connection) -> Result<(), String> {
    connection.execute_batch(
        "CREATE TABLE moves(
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          revision INTEGER NOT NULL CHECK(revision>=1),
          project TEXT NOT NULL CHECK(length(CAST(project AS BLOB)) BETWEEN 1 AND 4096),
          partner INTEGER NOT NULL CHECK(partner>0), form INTEGER NOT NULL CHECK(form BETWEEN 0 AND 8),
          scope TEXT NOT NULL CHECK(scope IN ('project','personal')),
          proposal TEXT NOT NULL CHECK(length(CAST(proposal AS BLOB))<=65536),
          forgotten INTEGER NOT NULL CHECK(forgotten IN (0,1)), created INTEGER NOT NULL CHECK(created>=0));
        CREATE TABLE move_candidates(
          key TEXT PRIMARY KEY, project TEXT NOT NULL, partner INTEGER NOT NULL, form INTEGER NOT NULL,
          fingerprint TEXT NOT NULL, record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=65536),
          threshold INTEGER NOT NULL CHECK(threshold>=3), offered INTEGER NOT NULL CHECK(offered IN (0,1)),
          never INTEGER NOT NULL CHECK(never IN (0,1)));
        CREATE TABLE move_dispatches(
          id TEXT PRIMARY KEY, task_id INTEGER NOT NULL, project TEXT NOT NULL,
          partner INTEGER NOT NULL, pins TEXT NOT NULL CHECK(length(CAST(pins AS BLOB))<=65536));
        CREATE TABLE move_sources(
          id TEXT PRIMARY KEY, task_id INTEGER NOT NULL, project TEXT NOT NULL,
          partner INTEGER NOT NULL, form INTEGER NOT NULL, fingerprint TEXT NOT NULL,
          steps TEXT NOT NULL CHECK(length(CAST(steps AS BLOB))<=65536),
          observed INTEGER NOT NULL DEFAULT 1 CHECK(observed IN (0,1)));
        CREATE TABLE move_usages(
          id TEXT PRIMARY KEY, task_id INTEGER NOT NULL, project TEXT NOT NULL,
          move_id INTEGER NOT NULL, partner INTEGER NOT NULL, record TEXT NOT NULL CHECK(length(CAST(record AS BLOB))<=65536));
        PRAGMA user_version=14;"
    ).map_err(|error| format!("Cannot migrate moves: {error}"))
}

fn sql(error: rusqlite::Error) -> String {
    error.to_string()
}
fn scope(scope: Scope) -> &'static str {
    match scope {
        Scope::Project => "project",
        Scope::Personal => "personal",
    }
}
fn capacity(connection: &Connection, table: &str, max: usize) -> Result<(), String> {
    let count: usize = connection
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .map_err(sql)?;
    if count >= max {
        Err(format!(
            "Move storage capacity reached ({table}: {max}); nothing was evicted"
        ))
    } else {
        Ok(())
    }
}
fn record_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Record> {
    Ok(Record {
        id: row.get(0)?,
        revision: row.get(1)?,
        project: row.get(2)?,
        proposal: json_column(row, 3, MAX_JSON)?,
        forgotten: row.get(4)?,
        created: row.get(5)?,
    })
}
fn records(
    connection: &Connection,
    project: &str,
    partner: i64,
    form: usize,
) -> Result<Vec<Record>, String> {
    let mut statement = connection.prepare(&format!("SELECT {RECORD_COLUMNS} FROM moves WHERE partner=?1 AND form=?2 AND (scope='personal' OR project=?3) ORDER BY id LIMIT 513")).map_err(sql)?;
    let records = statement
        .query_map(params![partner, form as i64, project], record_row)
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    if records.len() > MAX_RECORDS {
        return Err("Move record capacity exceeded".into());
    }
    for record in &records {
        validate_record(record)?;
    }
    Ok(records)
}
fn validate_record(record: &Record) -> Result<(), String> {
    domain::validate_proposal(&record.proposal)?;
    reject_secret_identity(&record.project)?;
    if record.id <= 0
        || record.revision == 0
        || record.revision > i64::MAX as u64
        || record.created < 0
        || record.project.is_empty()
        || record.project.len() > 4096
    {
        return Err("Invalid stored move identity".into());
    }
    Ok(())
}
fn selected(records: &[Record], id: i64, revision: u64) -> Result<&Record, String> {
    let record = records
        .iter()
        .find(|record| record.id == id)
        .ok_or("Move is missing or outside this partner/form/project; reload")?;
    if record.revision != revision {
        return Err("Move changed; reload before changing it".into());
    }
    Ok(record)
}
fn unique(records: &[Record], name: &str, except: Option<i64>) -> Result<(), String> {
    let key = name_key(name);
    if records
        .iter()
        .any(|record| Some(record.id) != except && name_key(&record.proposal.name) == key)
    {
        return Err(
            "Move name is already used by this partner and form (including reminders)".into(),
        );
    }
    Ok(())
}
fn revise(
    connection: &Connection,
    record: &Record,
    proposal: &Proposal,
    forgotten: bool,
) -> Result<(), String> {
    let next = record
        .revision
        .checked_add(1)
        .filter(|value| *value <= i64::MAX as u64)
        .ok_or("Move revision exhausted")?;
    let changed = connection
        .execute(
            "UPDATE moves SET revision=?1,proposal=?2,forgotten=?3 WHERE id=?4 AND revision=?5",
            params![
                next as i64,
                bounded_json(proposal, MAX_JSON)?,
                forgotten,
                record.id,
                record.revision as i64
            ],
        )
        .map_err(sql)?;
    if changed != 1 {
        return Err("Move changed; reload before changing it".into());
    }
    Ok(())
}
fn make_slot(
    connection: &Connection,
    visible: &[Record],
    replace: Option<(i64, u64)>,
    relearn: Option<i64>,
) -> Result<(), String> {
    if let Some((id, revision)) = replace {
        let record = selected(visible, id, revision)?;
        if record.forgotten || Some(id) == relearn {
            return Err("Replacement must be an exact active different move".into());
        }
        revise(connection, record, &record.proposal, true)?;
    } else if visible
        .iter()
        .filter(|record| !record.forgotten && Some(record.id) != relearn)
        .count()
        >= 4
    {
        return Err(
            "All four move slots are full; choose an exact move to forget or give up learning"
                .into(),
        );
    }
    Ok(())
}

impl State {
    pub(super) fn moves_request(&mut self, request: Request) -> Result<Response, String> {
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let task = load_task(&transaction, &self.project, request.task_id)?
            .ok_or("Move task is missing or belongs to another project")?;
        let roster = partners(&transaction)?;
        if !roster.iter().any(|partner| partner.id == request.partner) || request.form >= 9 {
            return Err("Move partner or form is unavailable".into());
        }
        let visible = records(&transaction, &self.project, request.partner, request.form)?;
        if task.archived && !matches!(request.operation, Operation::Load) {
            return Err("Restore the archived task before changing moves".into());
        }
        let mut portable = None;
        match &request.operation {
            Operation::Load => {
                if !task.archived && backfill_runs(&transaction, &self.project, &request)? {
                    enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
                }
            }
            Operation::ReadPortable { path } => {
                portable = Some(domain::portable::read_file(Path::new(path))?);
            }
            Operation::ExportPortable { id, revision, path } => {
                let record = selected(&visible, *id, *revision)?;
                let definition = domain::portable::Definition::from_proposal(&record.proposal)?;
                domain::portable::write_new(Path::new(path), &definition)?;
            }
            Operation::ImportPortable {
                definition,
                name,
                scope,
                replace,
            } => {
                let mut proposal = definition.proposal(request.task_id, request.partner, *scope)?;
                if proposal.form != request.form {
                    return Err("TM form differs from the reviewed local form".into());
                }
                proposal.name.clone_from(name);
                domain::validate_for_root(Path::new(&self.project), &proposal)?;
                unique_learning(&transaction, &visible, &proposal, *replace, None)?;
                capacity(&transaction, "moves", MAX_RECORDS)?;
                make_slot(&transaction, &visible, *replace, None)?;
                transaction.execute("INSERT INTO moves(revision,project,partner,form,scope,proposal,forgotten,created) VALUES(1,?1,?2,?3,?4,?5,0,?6)", params![self.project,proposal.partner,proposal.form as i64,self::scope(*scope),bounded_json(&proposal,MAX_JSON)?,timestamp()?]).map_err(sql)?;
                let mut owner = Attribution::of(
                    roster
                        .iter()
                        .find(|p| p.id == request.partner)
                        .ok_or("Learning partner unavailable")?,
                );
                owner.form = request.form;
                super::ribbons::move_taught(
                    &transaction,
                    &task,
                    owner,
                    transaction.last_insert_rowid(),
                )?;
            }
            Operation::Validate { id, revision } => {
                let record = selected(&visible, *id, *revision)?;
                if record.forgotten {
                    return Err("Relearn the forgotten move before using it".into());
                }
                domain::validate_for_root(Path::new(&self.project), &record.proposal)?;
            }
            Operation::Teach { proposal, replace }
            | Operation::Copy {
                proposal, replace, ..
            } => {
                if proposal.partner != request.partner || proposal.form != request.form {
                    return Err("Move target differs from the selected partner and form".into());
                }
                domain::validate_for_root(Path::new(&self.project), proposal)?;
                validate_provenance(&transaction, &self.project, &request, proposal, &roster)?;
                // Names belong to partner/form, not a scope; personal records must
                // also remain unambiguous when another project becomes visible.
                unique_learning(&transaction, &visible, proposal, *replace, None)?;
                capacity(&transaction, "moves", MAX_RECORDS)?;
                make_slot(&transaction, &visible, *replace, None)?;
                transaction.execute("INSERT INTO moves(revision,project,partner,form,scope,proposal,forgotten,created) VALUES(1,?1,?2,?3,?4,?5,0,?6)",params![self.project,proposal.partner,proposal.form as i64,scope(proposal.scope),bounded_json(proposal,MAX_JSON)?,timestamp()?]).map_err(sql)?;
                let mut owner = Attribution::of(
                    roster
                        .iter()
                        .find(|partner| partner.id == request.partner)
                        .ok_or("Teaching partner is unavailable")?,
                );
                owner.form = request.form;
                super::ribbons::move_taught(
                    &transaction,
                    &task,
                    owner,
                    transaction.last_insert_rowid(),
                )?;
            }
            Operation::Rename { id, revision, name } => {
                domain::validate_name(name)?;
                let record = selected(&visible, *id, *revision)?;
                let mut proposal = record.proposal.clone();
                proposal.name = name.clone();
                unique_all(&transaction, &proposal, Some(*id))?;
                revise(&transaction, record, &proposal, record.forgotten)?;
            }
            Operation::Forget { id, revision } => {
                let record = selected(&visible, *id, *revision)?;
                if record.forgotten {
                    return Err("Move is already forgotten".into());
                }
                revise(&transaction, record, &record.proposal, true)?;
            }
            Operation::Relearn {
                id,
                revision,
                replace,
            } => {
                let record = selected(&visible, *id, *revision)?;
                if !record.forgotten {
                    return Err("Move is already active".into());
                }
                unique_learning(
                    &transaction,
                    &visible,
                    &record.proposal,
                    *replace,
                    Some(*id),
                )?;
                domain::validate_for_root(Path::new(&self.project), &record.proposal)?;
                make_slot(&transaction, &visible, *replace, Some(*id))?;
                revise(&transaction, record, &record.proposal, false)?;
            }
            Operation::ObserveRun { run_id } => {
                let record = journal_run(&transaction, request.task_id, request.partner, run_id)?;
                let steps = domain::run_steps(Path::new(&self.project), &record)?;
                if record.prepared.partner.as_ref().map(|owner| owner.form) != Some(request.form) {
                    return Err("Run observation must retain its actual producer form".into());
                }
                observe(
                    &transaction,
                    &self.project,
                    &request,
                    &format!("run:{run_id}"),
                    &steps,
                )?;
            }
            Operation::DispatchPins { request_id, pins } => {
                dispatch_pins(&transaction, &self.project, &request, request_id, pins)?;
            }
            Operation::CandidateDecision { key, decision } => {
                decide(&transaction, &self.project, &request, key, *decision)?;
            }
            Operation::Use { usage } => {
                store_usage(&transaction, &self.project, &request, &visible, usage)?;
            }
        }
        enforce_slots(&transaction)?;
        if !matches!(
            request.operation,
            Operation::Load
                | Operation::Validate { .. }
                | Operation::Forget { .. }
                | Operation::ReadPortable { .. }
                | Operation::ExportPortable { .. }
        ) {
            enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
        }
        let mut response = response(&transaction, &self.project, request.partner, request.form)?;
        response.portable = portable;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit moves; no change persisted: {error}"))?;
        Ok(response)
    }
}

fn enforce_slots(connection: &Connection) -> Result<(), String> {
    let full: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM moves m WHERE m.forgotten=0 AND
        (SELECT count(*) FROM moves v WHERE v.partner=m.partner AND v.form=m.form
        AND v.forgotten=0 AND (v.scope='personal' OR v.project=m.project))>4)",
            [],
            |row| row.get(0),
        )
        .map_err(sql)?;
    if full {
        Err("Four visible move slots would be exceeded in a project; nothing changed".into())
    } else {
        Ok(())
    }
}

fn unique_all(
    connection: &Connection,
    proposal: &Proposal,
    except: Option<i64>,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {RECORD_COLUMNS} FROM moves WHERE partner=?1 AND form=?2"
        ))
        .map_err(sql)?;
    let all = statement
        .query_map(params![proposal.partner, proposal.form as i64], record_row)
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    let key = name_key(&proposal.name);
    if all
        .iter()
        .any(|r| Some(r.id) == except && !r.forgotten && name_key(&r.proposal.name) == key)
    {
        // Cosmetic spelling/case changes retain an already owned active label;
        // they do not claim a forgotten record's name for a new action.
        return if all
            .iter()
            .any(|r| Some(r.id) != except && !r.forgotten && name_key(&r.proposal.name) == key)
        {
            Err("Move name is already used by another active move".into())
        } else {
            Ok(())
        };
    }
    unique(&all, &proposal.name, except)
}
/// Only an explicit same-name active replacement can retain duplicate historical
/// labels. Originals keep their immutable IDs, steps, provenance and usage links.
fn unique_learning(
    connection: &Connection,
    visible: &[Record],
    proposal: &Proposal,
    replace: Option<(i64, u64)>,
    relearn: Option<i64>,
) -> Result<(), String> {
    let Some((id, revision)) = replace else {
        return unique_all(connection, proposal, relearn);
    };
    let old = selected(visible, id, revision)?;
    if old.forgotten {
        return Err("Replacement must be an exact active move".into());
    }
    let key = name_key(&proposal.name);
    if name_key(&old.proposal.name) != key {
        return unique_all(connection, proposal, relearn);
    }
    let mut statement = connection
        .prepare(&format!(
            "SELECT {RECORD_COLUMNS} FROM moves WHERE partner=?1 AND form=?2"
        ))
        .map_err(sql)?;
    let all = statement
        .query_map(params![proposal.partner, proposal.form as i64], record_row)
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    if all
        .iter()
        .any(|r| r.id != id && !r.forgotten && name_key(&r.proposal.name) == key)
    {
        return Err("Another active move already has that name".into());
    }
    Ok(())
}

fn validate_provenance(
    connection: &Connection,
    project: &str,
    request: &Request,
    proposal: &Proposal,
    roster: &[Partner],
) -> Result<(), String> {
    if matches!(request.operation, Operation::Copy { .. })
        && proposal.provenance.taught_by.is_none()
    {
        return Err("Teaching source owner is missing".into());
    }
    if let Some(teacher) = proposal.provenance.taught_by {
        if teacher == proposal.partner {
            return Err("A copied move must identify a different teaching partner".into());
        }
        let Operation::Copy { source, .. } = &request.operation else {
            return Err(
                "Party teaching requires the exact reviewed source record and revision".into(),
            );
        };
        let sources = records(connection, project, teacher, proposal.form)?;
        let source = selected(&sources, source.0, source.1)?;
        if source.forgotten
            || source.proposal.steps != proposal.steps
            || source.proposal.provenance.task_id != proposal.provenance.task_id
            || source.proposal.provenance.sources != proposal.provenance.sources
        {
            return Err("Teaching source changed; review the exact copy again".into());
        }
        let member = roster
            .iter()
            .find(|member| member.id == teacher)
            .ok_or("Teaching partner is no longer present")?;
        if proposal.provenance.teacher_name.as_deref()
            != Some(Attribution::of(member).name.as_str())
        {
            return Err("Teaching partner name changed; preview the copy again".into());
        }
    } else if proposal.provenance.task_id != request.task_id
        || proposal.provenance.teacher_name.is_some()
    {
        let mut statement=connection.prepare("SELECT record FROM move_candidates WHERE project=?1 AND partner=?2 AND form=?3 AND never=0").map_err(sql)?;
        let candidates = statement
            .query_map(
                params![project, request.partner, request.form as i64],
                |row| json_column::<Candidate>(row, 0, MAX_JSON),
            )
            .map_err(sql)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql)?;
        if !candidates.iter().any(|candidate| {
            candidate.proposal.provenance == proposal.provenance
                && candidate.proposal.steps == proposal.steps
        }) {
            return Err("Move provenance must identify this task or its unchanged approved candidate/teaching source".into());
        }
    }
    Ok(())
}
fn validate_request_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err("Invalid immutable request identity".into());
    }
    Ok(())
}
fn canonical_pins(pins: &[Pin]) -> Result<Vec<Pin>, String> {
    if pins.len() > crate::workspace::context::MAX_EXCERPTS {
        return Err("Pin receipt exceeds sixteen selectors".into());
    }
    let mut pins = pins.to_vec();
    pins.sort_by(|a, b| {
        (&a.path, a.start_line, a.end_line).cmp(&(&b.path, b.start_line, b.end_line))
    });
    pins.dedup();
    if !pins.is_empty() {
        domain::validate_steps(1, &Steps::Pins { pins: pins.clone() })?;
    }
    Ok(pins)
}
fn pin_selectors(pins: &[Pin]) -> Result<Steps, String> {
    let steps = Steps::Pins {
        pins: canonical_pins(pins)?,
    };
    domain::validate_steps(1, &steps)?;
    Ok(steps)
}
struct Dispatch {
    task_id: i64,
    project: String,
    partner: i64,
    pins: Vec<Pin>,
}
fn dispatch_row(connection: &Connection, id: &str) -> Result<Option<Dispatch>, String> {
    connection
        .query_row(
            "SELECT task_id,project,partner,pins FROM move_dispatches WHERE id=?1",
            [id],
            |row| {
                Ok(Dispatch {
                    task_id: row.get(0)?,
                    project: row.get(1)?,
                    partner: row.get(2)?,
                    pins: json_column(row, 3, MAX_JSON)?,
                })
            },
        )
        .optional()
        .map_err(sql)
}
fn verify_dispatch(
    receipt: &Dispatch,
    project: &str,
    task_id: i64,
    partner: i64,
    pins: &[Pin],
) -> Result<(), String> {
    if receipt.task_id != task_id
        || receipt.project != project
        || receipt.partner != partner
        || receipt.pins != canonical_pins(pins)?
    {
        return Err("Request identity cannot overwrite different task/partner/selectors".into());
    }
    Ok(())
}
fn dispatch_pins(
    connection: &Connection,
    project: &str,
    request: &Request,
    id: &str,
    pins: &[Pin],
) -> Result<(), String> {
    validate_request_id(id)?;
    let pins = canonical_pins(pins)?;
    if let Some(existing) = dispatch_row(connection, id)? {
        verify_dispatch(&existing, project, request.task_id, request.partner, &pins)?;
    } else {
        let admitted:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM active_work WHERE task_id=?1 AND partner=?2 AND request_id=?3 AND kind='provider')",params![request.task_id,request.partner,id],|row|row.get(0)).map_err(sql)?;
        if !admitted {
            return Err("Pin dispatch requires its exact live provider work claim".into());
        }
        let committed = excerpts(connection, request.task_id)?;
        for pin in &pins {
            if !committed.iter().any(|excerpt| {
                excerpt.available && excerpt.root == project && pin.matches_excerpt(excerpt)
            }) {
                return Err(
                    "Dispatch selector does not match an available committed task excerpt".into(),
                );
            }
        }
        capacity(connection, "move_dispatches", MAX_SOURCES)?;
        connection.execute("INSERT INTO move_dispatches(id,task_id,project,partner,pins) VALUES(?1,?2,?3,?4,?5)",params![id,request.task_id,project,request.partner,bounded_json(&pins,MAX_JSON)?]).map_err(sql)?;
    }
    if !pins.is_empty() {
        let observation = Request {
            form: 1,
            ..request.clone()
        };
        observe(
            connection,
            project,
            &observation,
            &format!("request:{id}"),
            &pin_selectors(&pins)?,
        )?;
    }
    Ok(())
}
fn journal_run(
    connection: &Connection,
    task_id: i64,
    partner: i64,
    run_id: &str,
) -> Result<crate::workspace::runner::RunRecord, String> {
    if !valid_run_id(run_id) {
        return Err("Invalid run identity".into());
    }
    let record: crate::workspace::runner::RunRecord = connection
        .query_row(
            "SELECT record FROM executions WHERE run_id=?1 AND task_id=?2",
            params![run_id, task_id],
            |row| json_column(row, 0, MAX_RUN_BYTES),
        )
        .optional()
        .map_err(sql)?
        .ok_or("Run observation has no committed task-owned local journal record")?;
    run_json(&record)?;
    if record.prepared.run_id != run_id
        || record.prepared.task_id != task_id
        || record
            .prepared
            .partner
            .as_ref()
            .is_none_or(|owner| owner.partner != partner)
    {
        return Err("Run attribution differs from the selected partner".into());
    }
    Ok(record)
}

#[derive(Debug)]
struct Source {
    task_id: i64,
    project: String,
    partner: i64,
    form: usize,
    steps: Steps,
}
fn source_row(connection: &Connection, id: &str) -> Result<Option<Source>, String> {
    connection
        .query_row(
            "SELECT task_id,project,partner,form,steps FROM move_sources WHERE id=?1",
            [id],
            |row| {
                Ok(Source {
                    task_id: row.get(0)?,
                    project: row.get(1)?,
                    partner: row.get(2)?,
                    form: row.get(3)?,
                    steps: json_column(row, 4, MAX_JSON)?,
                })
            },
        )
        .optional()
        .map_err(sql)
}
fn verify_source(
    source: &Source,
    project: &str,
    request: &Request,
    steps: &Steps,
) -> Result<(), String> {
    if source.task_id != request.task_id
        || source.project != project
        || source.partner != request.partner
        || source.form != request.form
        || domain::fingerprint(&source.steps)? != domain::fingerprint(steps)?
    {
        return Err(
            "Observation identity cannot overwrite a different task/partner/form/selection".into(),
        );
    }
    Ok(())
}
fn candidate_key(project: &str, partner: i64, form: usize, fingerprint: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update((project.len() as u64).to_le_bytes());
    hash.update(project.as_bytes());
    hash.update(partner.to_le_bytes());
    hash.update((form as u64).to_le_bytes());
    hash.update(fingerprint.as_bytes());
    format!("{:x}", hash.finalize())
}
fn observe(
    connection: &Connection,
    project: &str,
    request: &Request,
    id: &str,
    steps: &Steps,
) -> Result<(), String> {
    if let Some(existing) = source_row(connection, id)? {
        return verify_source(&existing, project, request, steps);
    }
    capacity(connection, "move_sources", MAX_SOURCES)?;
    let fingerprint = domain::fingerprint(steps)?;
    connection.execute("INSERT INTO move_sources(id,task_id,project,partner,form,fingerprint,steps) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,request.task_id,project,request.partner,request.form as i64,fingerprint,bounded_json(steps,MAX_JSON)?]).map_err(sql)?;
    // An actual command is evidence for its historical producer, even when that
    // form cannot select a Run move. Do not turn execution facts into definition
    // authority or relabel them as Jolteon to make a teachable candidate.
    if !domain::form_accepts_steps(request.form, steps) {
        return Ok(());
    }
    let key = candidate_key(project, request.partner, request.form, &fingerprint);
    let previous: Option<(Candidate, i64, bool, bool)> = connection
        .query_row(
            "SELECT record,threshold,offered,never FROM move_candidates WHERE key=?1",
            [&key],
            |row| {
                Ok((
                    json_column(row, 0, MAX_JSON)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                ))
            },
        )
        .optional()
        .map_err(sql)?;
    let uses:u64=connection.query_row("SELECT count(*) FROM move_sources WHERE observed=1 AND project=?1 AND partner=?2 AND form=?3 AND fingerprint=?4",params![project,request.partner,request.form as i64,fingerprint],|row|row.get(0)).map_err(sql)?;
    if uses < 3 && previous.is_none() {
        return Ok(());
    }
    let mut statement=connection.prepare("SELECT id FROM move_sources WHERE observed=1 AND project=?1 AND partner=?2 AND form=?3 AND fingerprint=?4 ORDER BY rowid DESC LIMIT 3").map_err(sql)?;
    let sources = statement
        .query_map(
            params![project, request.partner, request.form as i64, fingerprint],
            |row| row.get::<_, String>(0),
        )
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    let (mut candidate, threshold, offered, never) = if let Some(previous) = previous {
        previous
    } else {
        capacity(connection, "move_candidates", MAX_CANDIDATES)?;
        let used = records(connection, project, request.partner, request.form)?
            .into_iter()
            .map(|record| record.proposal.name)
            .collect::<Vec<_>>();
        (
            Candidate {
                key: key.clone(),
                proposal: Proposal {
                    partner: request.partner,
                    form: request.form,
                    name: domain::suggested_name(request.form, steps, &used),
                    scope: Scope::Project,
                    steps: steps.clone(),
                    provenance: domain::Provenance {
                        task_id: request.task_id,
                        sources: sources.clone(),
                        taught_by: None,
                        teacher_name: None,
                    },
                },
                uses,
                sources: sources.clone(),
                postponed: false,
            },
            3,
            false,
            false,
        )
    };
    candidate.uses = uses;
    candidate.sources = sources.clone();
    candidate.proposal.provenance.sources = sources;
    candidate.postponed = offered || uses < (threshold as u64);
    connection.execute("INSERT INTO move_candidates(key,project,partner,form,fingerprint,record,threshold,offered,never) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(key) DO UPDATE SET record=excluded.record",params![key,project,request.partner,request.form as i64,fingerprint,bounded_json(&candidate,MAX_JSON)?,threshold,offered,never]).map_err(sql)?;
    Ok(())
}
fn backfill_runs(
    connection: &Connection,
    project: &str,
    request: &Request,
) -> Result<bool, String> {
    let mut statement = connection
        .prepare(
            "SELECT e.record FROM executions e JOIN tasks t ON t.id=e.task_id
        WHERE t.project=?1 AND NOT EXISTS(SELECT 1 FROM move_sources s WHERE s.id='run:'||e.run_id)
        ORDER BY e.rowid",
        )
        .map_err(sql)?;
    let runs = statement
        .query_map([project], |row| {
            json_column::<crate::workspace::runner::RunRecord>(row, 0, MAX_RUN_BYTES)
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    let mut changed = false;
    for run in runs {
        let Some(owner) = &run.prepared.partner else {
            continue;
        };
        if owner.partner != request.partner || owner.form != request.form {
            continue;
        }
        // Rejected unsafe/shell/unstarted history never becomes a candidate.
        let Ok(steps) = domain::run_steps(Path::new(project), &run) else {
            continue;
        };
        let observation = Request {
            task_id: run.prepared.task_id,
            ..request.clone()
        };
        observe(
            connection,
            project,
            &observation,
            &format!("run:{}", run.prepared.run_id),
            &steps,
        )?;
        changed = true;
    }
    Ok(changed)
}

fn decide(
    connection: &Connection,
    project: &str,
    request: &Request,
    key: &str,
    decision: Decision,
) -> Result<(), String> {
    let (mut candidate,threshold,_offered,never):(Candidate,i64,bool,bool)=connection.query_row("SELECT record,threshold,offered,never FROM move_candidates WHERE key=?1 AND project=?2 AND partner=?3 AND form=?4",params![key,project,request.partner,request.form as i64],|row|Ok((json_column(row,0,MAX_JSON)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(sql)?.ok_or("Candidate is missing or outside this partner/form/project")?;
    if never {
        return Err("Candidate was permanently declined".into());
    }
    let (threshold, offered, never) = match decision {
        Decision::Never => (threshold, true, true),
        Decision::NotNow => (
            i64::try_from(candidate.uses)
                .map_err(|_| "Candidate count overflow")?
                .checked_add(3)
                .ok_or("Candidate count overflow")?,
            false,
            false,
        ),
        Decision::Offered => {
            if candidate.uses < threshold as u64 {
                return Err("Candidate has not reached its next offer threshold".into());
            }
            (threshold, true, false)
        }
    };
    candidate.postponed = true;
    connection
        .execute(
            "UPDATE move_candidates SET record=?1,threshold=?2,offered=?3,never=?4 WHERE key=?5",
            params![
                bounded_json(&candidate, MAX_JSON)?,
                threshold,
                offered,
                never,
                key
            ],
        )
        .map_err(sql)?;
    Ok(())
}

fn actual_outcome(record: &crate::workspace::runner::RunRecord) -> Outcome {
    use crate::workspace::runner::RunState;
    match record.state {
        RunState::AwaitingDispatch | RunState::Running | RunState::Cancelling => Outcome::Pending,
        RunState::Succeeded => Outcome::Succeeded,
        RunState::Failed => Outcome::Failed,
        RunState::Cancelled => Outcome::Cancelled,
        RunState::Interrupted | RunState::OutcomeUnknown => Outcome::Unknown,
    }
}
fn prepared_steps(
    project: &str,
    record: &crate::workspace::runner::RunRecord,
) -> Result<Steps, String> {
    let crate::workspace::runner::CommandSpec::Direct { program, args } = &record.prepared.spec
    else {
        return Err("Shell journals cannot be move uses".into());
    };
    let cwd = domain::relative_cwd(Path::new(project), &record.prepared.cwd)?;
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(program.clone());
    argv.extend(args.iter().cloned());
    let steps = Steps::Run { argv, cwd };
    domain::validate_steps(2, &steps)?;
    Ok(steps)
}
fn authoritative_usage(
    connection: &Connection,
    project: &str,
    usage: &Usage,
    proposal: Option<&Proposal>,
) -> Result<Usage, String> {
    let mut usage = usage.clone();
    if let Some(run) = &usage.run_id {
        if usage.request_id.is_some() {
            return Err("A move use cannot refer to both a command and a provider request".into());
        }
        let record = journal_run(connection, usage.task_id, usage.partner, run)?;
        if let Some(proposal) = proposal
            && domain::fingerprint(&prepared_steps(project, &record)?)?
                != domain::fingerprint(&proposal.steps)?
        {
            return Err("Move use does not match the actual journal argv/cwd".into());
        }
        usage.outcome = actual_outcome(&record);
        usage.revision = record
            .after
            .fingerprint
            .or(record.prepared.before.fingerprint);
    } else if let Some(request_id) = &usage.request_id {
        validate_request_id(request_id)?;
        let receipt = dispatch_row(connection, request_id)?;
        if let Some(source) = receipt {
            if source.task_id != usage.task_id
                || source.partner != usage.partner
                || source.project != project
            {
                return Err("Move use belongs to another request owner".into());
            }
            if let Some(proposal) = proposal {
                let pins = match &proposal.steps {
                    Steps::Pins { pins } | Steps::Request { pins, .. } => pins,
                    _ => return Err("Request receipt cannot prove this move kind".into()),
                };
                if canonical_pins(pins)? != source.pins {
                    return Err(
                        "Move use selectors differ from the actual dispatched request".into(),
                    );
                }
            }
            // Runtime outcomes after admitted submission are local receipt
            // updates, never tool observations or an external-account assertion.
            if usage.outcome == Outcome::Denied {
                return Err("A dispatched request cannot be a denied use".into());
            }
        } else if !matches!(usage.outcome, Outcome::Denied | Outcome::Cancelled) {
            return Err("Move request use has no committed runtime dispatch receipt".into());
        }
        usage.revision = None;
    } else if !matches!(
        usage.outcome,
        Outcome::Denied | Outcome::Cancelled | Outcome::Pending | Outcome::Completed
    ) {
        return Err(
            "A successful/failed/unknown move result requires actual local journal evidence".into(),
        );
    } else if matches!(usage.outcome, Outcome::Completed)
        && proposal.is_some_and(|proposal| {
            !matches!(proposal.steps, Steps::Task { .. } | Steps::Pins { .. })
        })
    {
        return Err(
            "Run/request completion requires a committed journal or dispatch receipt".into(),
        );
    } else {
        usage.revision = None;
    }
    Ok(usage)
}
fn local_completion(
    connection: &Connection,
    project: &str,
    usage: &Usage,
    proposal: &Proposal,
) -> Result<(), String> {
    if usage.run_id.is_some() || usage.request_id.is_some() || usage.outcome != Outcome::Completed {
        return Ok(());
    }
    match &proposal.steps {
        Steps::Task { goal, criteria } => {
            let task = load_task(connection, project, usage.task_id)?
                .ok_or("Task-template use has no committed local task")?;
            if task.goal != *goal
                || !task
                    .criteria
                    .iter()
                    .map(|criterion| criterion.text.as_str())
                    .eq(criteria.iter().map(String::as_str))
            {
                return Err(
                    "Task-template use does not match its committed goal and criteria".into(),
                );
            }
        }
        Steps::Pins { pins } => {
            let retained = excerpts(connection, usage.task_id)?;
            if !pins.iter().all(|pin| {
                retained.iter().any(|excerpt| {
                    excerpt.available && excerpt.root == project && pin.matches_excerpt(excerpt)
                })
            }) {
                return Err(
                    "Pin move use has no matching available committed source captures".into(),
                );
            }
        }
        _ => {}
    }
    Ok(())
}
fn store_usage(
    connection: &Connection,
    project: &str,
    request: &Request,
    visible: &[Record],
    usage: &Usage,
) -> Result<(), String> {
    validate_request_id(&usage.id)?;
    if usage.task_id != request.task_id || usage.partner != request.partner {
        return Err("Move use identity differs from this task and partner".into());
    }
    let previous: Option<Usage> = connection
        .query_row(
            "SELECT record FROM move_usages WHERE id=?1",
            [&usage.id],
            |row| json_column(row, 0, MAX_JSON),
        )
        .optional()
        .map_err(sql)?;
    let record = visible
        .iter()
        .find(|record| record.id == usage.move_id && (!record.forgotten || previous.is_some()))
        .ok_or("A new move use requires a visible active move")?;
    if previous.is_none() {
        domain::validate_for_root(Path::new(project), &record.proposal)?;
    }
    local_completion(connection, project, usage, &record.proposal)?;
    let usage = authoritative_usage(connection, project, usage, Some(&record.proposal))?;
    if let Some(previous) = previous {
        if previous.id != usage.id
            || previous.move_id != usage.move_id
            || previous.task_id != usage.task_id
            || previous.partner != usage.partner
            || previous.run_id != usage.run_id
            || previous.request_id != usage.request_id
        {
            return Err("Move use identity cannot overwrite a different move/task/request".into());
        }
        if usage.run_id.is_none() && previous != usage && previous.outcome != Outcome::Pending {
            return Err("Settled move-use receipts are immutable".into());
        }
    } else {
        capacity(connection, "move_usages", MAX_USAGES)?;
        let mut statement = connection
            .prepare("SELECT record FROM move_usages WHERE task_id=?1")
            .map_err(sql)?;
        let usages = statement
            .query_map([request.task_id], |row| {
                json_column::<Usage>(row, 0, MAX_JSON)
            })
            .map_err(sql)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql)?;
        if usages.len() >= 256 {
            return Err("Move use capacity reached (256 per task); nothing was evicted".into());
        }
        if usages.iter().any(|other| {
            usage.run_id.is_some() && usage.run_id == other.run_id
                || usage.request_id.is_some() && usage.request_id == other.request_id
        }) {
            return Err(
                "Actual journal/request evidence is already attributed to another move use".into(),
            );
        }
    }
    connection.execute("INSERT INTO move_usages(id,task_id,project,move_id,partner,record) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET record=excluded.record",params![usage.id,usage.task_id,project,usage.move_id,usage.partner,bounded_json(&usage,MAX_JSON)?]).map_err(sql)?;
    Ok(())
}
fn response(
    connection: &Connection,
    project: &str,
    partner: i64,
    form: usize,
) -> Result<Response, String> {
    let records = records(connection, project, partner, form)?;
    let active = records
        .iter()
        .filter(|record| !record.forgotten)
        .map(|record| domain::fingerprint(&record.proposal.steps))
        .collect::<Result<Vec<_>, _>>()?;
    let mut statement=connection.prepare("SELECT record,threshold,offered FROM move_candidates WHERE project=?1 AND partner=?2 AND form=?3 AND never=0 ORDER BY rowid").map_err(sql)?;
    let rows = statement
        .query_map(params![project, partner, form as i64], |row| {
            Ok((
                json_column::<Candidate>(row, 0, MAX_JSON)?,
                row.get::<_, u64>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    let mut candidates = Vec::new();
    for (mut candidate, threshold, offered) in rows {
        if active.contains(&domain::fingerprint(&candidate.proposal.steps)?) {
            continue;
        }
        candidate.postponed = offered || candidate.uses < threshold;
        candidates.push(candidate);
    }
    let mut statement = connection
        .prepare("SELECT record FROM move_usages WHERE partner=?1 ORDER BY rowid")
        .map_err(sql)?;
    let rows = statement
        .query_map([partner], |row| json_column::<Usage>(row, 0, MAX_JSON))
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    let mut usages = Vec::new();
    for usage in rows {
        if records.iter().any(|record| record.id == usage.move_id) {
            // Evidence can outlive release of the move or partner. Resolve only
            // retained linked facts; task purge removes its uses transactionally.
            let owner_project: String = connection
                .query_row(
                    "SELECT project FROM tasks WHERE id=?1",
                    [usage.task_id],
                    |row| row.get(0),
                )
                .map_err(sql)?;
            usages.push(authoritative_usage(
                connection,
                &owner_project,
                &usage,
                None,
            )?);
        }
    }
    Ok(Response {
        records,
        candidates,
        usages,
        portable: None,
    })
}

pub(super) fn purge_project(connection: &Connection, project: &str) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM moves WHERE project=?1 AND scope='project'",
            [project],
        )
        .map_err(sql)?;
    connection
        .execute("DELETE FROM move_candidates WHERE project=?1", [project])
        .map_err(sql)?;
    // Keep immutable dedupe watermarks while their actual journal still exists,
    // so Load cannot resurrect just-purged candidates from historical runs.
    connection
        .execute(
            "UPDATE move_sources SET observed=0 WHERE project=?1",
            [project],
        )
        .map_err(sql)?;
    // Historical use facts remain unless their task is actually purged.
    Ok(())
}
pub(super) fn purge_task(connection: &Connection, task_id: i64) -> Result<(), String> {
    connection
        .execute("DELETE FROM move_usages WHERE task_id=?1", [task_id])
        .map_err(sql)?;
    // Remove only task evidence; candidate counters must follow retained sources.
    connection
        .execute("DELETE FROM move_sources WHERE task_id=?1", [task_id])
        .map_err(sql)?;
    connection
        .execute("DELETE FROM move_dispatches WHERE task_id=?1", [task_id])
        .map_err(sql)?;
    reconcile_candidates(connection)
}
pub(super) fn release_partner(connection: &Connection, partner: i64) -> Result<(), String> {
    connection
        .execute("DELETE FROM moves WHERE partner=?1", [partner])
        .map_err(sql)?;
    connection
        .execute("DELETE FROM move_candidates WHERE partner=?1", [partner])
        .map_err(sql)?;
    // Run/provider journal and move-use receipts preserve historical attribution.
    Ok(())
}
fn reconcile_candidates(connection: &Connection) -> Result<(), String> {
    let mut statement=connection.prepare("SELECT key,project,partner,form,fingerprint,record,threshold,offered FROM move_candidates").map_err(sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, usize>(3)?,
                row.get::<_, String>(4)?,
                json_column::<Candidate>(row, 5, MAX_JSON)?,
                row.get::<_, u64>(6)?,
                row.get::<_, bool>(7)?,
            ))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    for (key, project, partner, form, fingerprint, mut candidate, threshold, offered) in rows {
        let mut statement=connection.prepare("SELECT id FROM move_sources WHERE observed=1 AND project=?1 AND partner=?2 AND form=?3 AND fingerprint=?4 ORDER BY rowid DESC").map_err(sql)?;
        let sources = statement
            .query_map(params![project, partner, form as i64, fingerprint], |row| {
                row.get::<_, String>(0)
            })
            .map_err(sql)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql)?;
        let removed = candidate.uses.saturating_sub(sources.len() as u64);
        let threshold = threshold.saturating_sub(removed).max(3);
        candidate.uses = sources.len() as u64;
        candidate.sources = sources.into_iter().take(3).collect();
        candidate.proposal.provenance.sources = candidate.sources.clone();
        candidate.postponed = offered || candidate.uses < threshold;
        // Reminders and decisions belong to the project, not the deleted task.
        // Retain their approved steps even when no supporting evidence survives.
        connection
            .execute(
                "UPDATE move_candidates SET record=?1,threshold=?2 WHERE key=?3",
                params![bounded_json(&candidate, MAX_JSON)?, threshold, key],
            )
            .map_err(sql)?;
    }
    Ok(())
}

pub(super) fn inspect(connection: &Connection) -> Result<(), String> {
    inspect_columns(
        connection,
        "moves",
        &[
            ("id", "INTEGER", false, 1),
            ("revision", "INTEGER", true, 0),
            ("project", "TEXT", true, 0),
            ("partner", "INTEGER", true, 0),
            ("form", "INTEGER", true, 0),
            ("scope", "TEXT", true, 0),
            ("proposal", "TEXT", true, 0),
            ("forgotten", "INTEGER", true, 0),
            ("created", "INTEGER", true, 0),
        ],
    )?;
    inspect_columns(
        connection,
        "move_candidates",
        &[
            ("key", "TEXT", false, 1),
            ("project", "TEXT", true, 0),
            ("partner", "INTEGER", true, 0),
            ("form", "INTEGER", true, 0),
            ("fingerprint", "TEXT", true, 0),
            ("record", "TEXT", true, 0),
            ("threshold", "INTEGER", true, 0),
            ("offered", "INTEGER", true, 0),
            ("never", "INTEGER", true, 0),
        ],
    )?;
    inspect_columns(
        connection,
        "move_dispatches",
        &[
            ("id", "TEXT", false, 1),
            ("task_id", "INTEGER", true, 0),
            ("project", "TEXT", true, 0),
            ("partner", "INTEGER", true, 0),
            ("pins", "TEXT", true, 0),
        ],
    )?;
    inspect_columns(
        connection,
        "move_sources",
        &[
            ("id", "TEXT", false, 1),
            ("task_id", "INTEGER", true, 0),
            ("project", "TEXT", true, 0),
            ("partner", "INTEGER", true, 0),
            ("form", "INTEGER", true, 0),
            ("fingerprint", "TEXT", true, 0),
            ("steps", "TEXT", true, 0),
            ("observed", "INTEGER", true, 0),
        ],
    )?;
    inspect_columns(
        connection,
        "move_usages",
        &[
            ("id", "TEXT", false, 1),
            ("task_id", "INTEGER", true, 0),
            ("project", "TEXT", true, 0),
            ("move_id", "INTEGER", true, 0),
            ("partner", "INTEGER", true, 0),
            ("record", "TEXT", true, 0),
        ],
    )?;
    let invalid:bool=connection.query_row("SELECT (SELECT count(*) FROM moves)>512 OR (SELECT count(*) FROM move_candidates)>512 OR (SELECT count(*) FROM move_sources)>16384 OR (SELECT count(*) FROM move_dispatches)>16384 OR (SELECT count(*) FROM move_usages)>8192 OR EXISTS(SELECT 1 FROM moves m LEFT JOIN partners p ON p.id=m.partner WHERE p.id IS NULL) OR EXISTS(SELECT 1 FROM move_candidates m LEFT JOIN partners p ON p.id=m.partner WHERE p.id IS NULL) OR EXISTS(SELECT 1 FROM move_sources s LEFT JOIN tasks t ON t.id=s.task_id WHERE t.id IS NULL OR t.project!=s.project) OR EXISTS(SELECT 1 FROM move_dispatches s LEFT JOIN tasks t ON t.id=s.task_id WHERE t.id IS NULL OR t.project!=s.project) OR EXISTS(SELECT 1 FROM move_usages u LEFT JOIN tasks t ON t.id=u.task_id WHERE t.id IS NULL OR t.project!=u.project) OR EXISTS(SELECT 1 FROM move_usages GROUP BY task_id HAVING count(*)>256)",[],|row|row.get(0)).map_err(sql)?;
    if invalid {
        return Err("Invalid move capacity or ownership".into());
    }
    let malformed:bool=connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM moves WHERE forgotten NOT IN (0,1))
        OR EXISTS(SELECT 1 FROM move_sources WHERE observed NOT IN (0,1))
        OR EXISTS(SELECT 1 FROM move_candidates WHERE offered NOT IN (0,1) OR never NOT IN (0,1)
        OR threshold<3 OR partner<=0 OR form NOT BETWEEN 0 AND 8 OR length(CAST(project AS BLOB)) NOT BETWEEN 1 AND 4096)",
        [],|row|row.get(0)).map_err(sql)?;
    if malformed {
        return Err("Invalid stored move flags or candidate ownership".into());
    }
    let mut statement = connection
        .prepare(&format!(
            "SELECT {RECORD_COLUMNS},partner,form,scope FROM moves"
        ))
        .map_err(sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                record_row(row)?,
                row.get::<_, i64>(6)?,
                row.get::<_, usize>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    for (record, partner, form, stored_scope) in &rows {
        validate_record(record)?;
        if record.proposal.partner != *partner
            || record.proposal.form != *form
            || scope(record.proposal.scope) != stored_scope
        {
            return Err("Stored move columns differ from proposal ownership".into());
        }
        if rows.iter().any(|(other, _, _, _)| {
            other.id != record.id
                && !record.forgotten
                && !other.forgotten
                && other.proposal.partner == *partner
                && other.proposal.form == *form
                && name_key(&other.proposal.name) == name_key(&record.proposal.name)
        }) {
            return Err("Duplicate active stored move name".into());
        }
    }
    enforce_slots(connection)?;
    inspect_observations(connection)?;
    let mut statement = connection
        .prepare("SELECT id,task_id,project,move_id,partner,record FROM move_usages")
        .map_err(sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                json_column::<Usage>(row, 5, MAX_JSON)?,
            ))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    for (id, task, project, move_id, partner, usage) in rows {
        validate_request_id(&id)?;
        if usage.id != id
            || usage.task_id != task
            || usage.move_id != move_id
            || usage.partner != partner
            || partner <= 0
            || move_id <= 0
        {
            return Err("Stored move usage identity differs from columns".into());
        }
        authoritative_usage(connection, &project, &usage, None)?;
    }
    Ok(())
}
fn inspect_dispatches(connection: &Connection) -> Result<(), String> {
    let mut statement = connection
        .prepare("SELECT id,task_id,project,partner,pins FROM move_dispatches")
        .map_err(sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                json_column::<Vec<Pin>>(row, 4, MAX_JSON)?,
            ))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    for (id, task, project, partner, pins) in rows {
        validate_request_id(&id)?;
        reject_secret_identity(&project)?;
        if task <= 0
            || partner <= 0
            || project.is_empty()
            || project.len() > 4096
            || canonical_pins(&pins)? != pins
        {
            return Err("Invalid immutable dispatch receipt".into());
        }
    }
    Ok(())
}

fn inspect_observations(connection: &Connection) -> Result<(), String> {
    inspect_dispatches(connection)?;
    let mut statement = connection
        .prepare(
            "SELECT id,task_id,project,partner,form,fingerprint,steps,observed FROM move_sources",
        )
        .map_err(sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, usize>(4)?,
                row.get::<_, String>(5)?,
                json_column::<Steps>(row, 6, MAX_JSON)?,
                row.get::<_, bool>(7)?,
            ))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    for (id, task, project, partner, form, fingerprint, steps, _) in &rows {
        reject_secret_identity(project)?;
        if *task <= 0
            || *partner <= 0
            || *form >= crate::domain::FORMS.len()
            || domain::fingerprint(steps)? != *fingerprint
        {
            return Err("Invalid stored move observation".into());
        }
        if let Some(run) = id.strip_prefix("run:") {
            let actual = journal_run(connection, *task, *partner, run)?;
            if domain::fingerprint(&domain::journal_steps(Path::new(project), &actual)?)?
                != *fingerprint
            {
                return Err("Stored move observation differs from actual journal".into());
            }
        } else if let Some(request) = id.strip_prefix("request:") {
            domain::validate_steps(*form, steps)?;
            validate_request_id(request)?;
            let receipt = dispatch_row(connection, request)?
                .ok_or("Stored pin observation has no dispatch receipt")?;
            let Steps::Pins { pins } = steps else {
                return Err("Invalid stored request pin receipt".into());
            };
            verify_dispatch(&receipt, project, *task, *partner, pins)?;
        } else {
            return Err("Unknown stored observation source kind".into());
        }
    }
    let mut groups = std::collections::BTreeMap::<(&str, i64, usize, &str), Vec<&str>>::new();
    for (id, _, project, partner, form, fingerprint, _, observed) in &rows {
        if *observed {
            groups
                .entry((project.as_str(), *partner, *form, fingerprint.as_str()))
                .or_default()
                .push(id.as_str());
        }
    }
    let mut statement=connection.prepare("SELECT key,project,partner,form,fingerprint,record,threshold,offered,never FROM move_candidates").map_err(sql)?;
    let candidates = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, usize>(3)?,
                row.get::<_, String>(4)?,
                json_column::<Candidate>(row, 5, MAX_JSON)?,
                row.get::<_, u64>(6)?,
                row.get::<_, bool>(7)?,
                row.get::<_, bool>(8)?,
            ))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    for (key, project, partner, form, fingerprint, candidate, threshold, offered, never) in
        candidates
    {
        domain::validate_proposal(&candidate.proposal)?;
        reject_secret_identity(&project)?;
        let sources = groups
            .get(&(project.as_str(), partner, form, fingerprint.as_str()))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if candidate.key != key
            || candidate_key(&project, partner, form, &fingerprint) != key
            || candidate.proposal.partner != partner
            || candidate.proposal.form != form
            || candidate.proposal.scope != Scope::Project
            || domain::fingerprint(&candidate.proposal.steps)? != fingerprint
            || candidate.uses != sources.len() as u64
            || candidate.sources.len() != sources.len().min(3)
            || candidate
                .sources
                .iter()
                .any(|source| !sources.contains(&source.as_str()))
            || candidate.proposal.provenance.sources != candidate.sources
            || threshold < 3
            || candidate.postponed != (offered || candidate.uses < threshold)
            || never && !offered
        {
            return Err("Invalid stored candidate identity, count, threshold or sources".into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "moves_tests.rs"]
mod tests;
