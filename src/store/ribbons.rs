//! First-recorded local milestones. Only authoritative commit paths call these writers.
use super::*;
use crate::workspace::records::{Artifact, ArtifactKind, Freshness};
use crate::workspace::ribbons::{Evidence, Kind, Ribbon, View};
use crate::workspace::{runner, workflows};

const MAX_RECORD: usize = 8192;
const MAX_SUPPORT: usize = 128 * 1024;
const TABLE: &str = "CREATE TABLE ribbons(
    project TEXT NOT NULL CHECK(length(CAST(project AS BLOB)) BETWEEN 1 AND 4096),
    kind TEXT NOT NULL CHECK(kind IN ('accepted_review','verified_handoff','learned_move')),
    record TEXT CHECK(record IS NULL OR length(CAST(record AS BLOB))<=8192),
    support TEXT CHECK(support IS NULL OR length(CAST(support AS BLOB))<=131072),
    PRIMARY KEY(project,kind), CHECK(record IS NOT NULL OR support IS NULL))";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Support {
    revision: u64,
    review_generation: u64,
    criteria: Vec<String>,
    evidence: Vec<workflows::EvidenceRef>,
}

pub(super) fn migrate(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(&format!("{TABLE}; PRAGMA user_version=18;"))
        .map_err(|error| format!("Cannot migrate local ribbon history: {error}"))
}

fn recorded(connection: &Connection, project: &str, kind: Kind) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM ribbons WHERE project=?1 AND kind=?2)",
            params![project, kind.as_str()],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn insert(
    connection: &Connection,
    project: &str,
    ribbon: &Ribbon,
    support: Option<&Support>,
) -> Result<(), String> {
    validate(ribbon, support)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO ribbons(project,kind,record,support) VALUES(?1,?2,?3,?4)",
            params![
                project,
                ribbon.kind.as_str(),
                bounded_json(ribbon, MAX_RECORD)?,
                support
                    .map(|value| bounded_json(value, MAX_SUPPORT))
                    .transpose()?
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Resolve real retained local journals and their anchored output identities. A
/// missing/replaced spool is not verification even if its journal looks successful.
fn observations(
    connection: &Connection,
    data_dir: &Path,
    task: i64,
) -> Result<Vec<runner::RunRecord>, String> {
    let mut runs = super::records::runs_for(connection, task)?;
    for run in &mut runs {
        let stored = stored_run(connection, task, &run.prepared.run_id)?;
        if stored.reserved
            || !SpoolDirectory::open(data_dir)
                .and_then(|directory| directory.read(&stored.output_name, &stored.output_identity))
                .and_then(|file| file.metadata().map_err(|error| error.to_string()))
                .is_ok_and(|metadata| metadata.len() == run.captured_bytes)
        {
            run.output_unavailable = true;
        }
    }
    Ok(runs)
}

fn sources_owned(
    project: &str,
    captures: &[crate::workspace::context::Excerpt],
    evidence: &[workflows::EvidenceRef],
) -> bool {
    evidence.iter().all(|reference| match reference {
        workflows::EvidenceRef::Source { excerpt_id, .. } => captures
            .iter()
            .find(|capture| capture.id == *excerpt_id)
            .is_some_and(|capture| {
                Path::new(project).starts_with(&capture.root)
                    && Path::new(&capture.root)
                        .join(&capture.path)
                        .starts_with(project)
            }),
        workflows::EvidenceRef::Run { .. } => true,
    })
}

/// Evaluate the existing effective Accept authority using a fresh bounded source
/// observation, never the display string or caller's supplied fingerprint.
fn qualified_support(
    connection: &Connection,
    data_dir: &Path,
    task: &Snapshot,
    extra: &[workflows::EvidenceRef],
) -> Result<Option<Support>, String> {
    if task.workflows.review_generation == 0 || task.workflows.review.is_none() {
        return Ok(None);
    }
    let writing: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM active_work WHERE task_id=?1 AND writing=1)",
            [task.id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if writing {
        return Ok(None);
    }
    let Ok(current) = crate::workspace::repo::observe_inputs(Path::new(&task.project), data_dir)
    else {
        return Ok(None);
    };
    let captures = excerpts(connection, task.id)?;
    let runs = observations(connection, data_dir, task.id)?;
    let view = workflows::EvidenceView {
        task_id: task.id,
        excerpts: &captures,
        runs: &runs,
        current: &current,
    };
    let criteria: Vec<String> = task
        .criteria
        .iter()
        .map(|criterion| criterion.text.clone())
        .collect();
    if task.workflows.review_guard(&criteria, &view).verdict != workflows::Verdict::Accept {
        return Ok(None);
    }
    let review = task.workflows.review.as_ref().expect("checked review");
    let mut evidence = Vec::new();
    for reference in review
        .criteria
        .iter()
        .flat_map(|criterion| &criterion.evidence)
        .chain(extra)
    {
        if view.status(reference) != workflows::EvidenceStatus::Current {
            return Ok(None);
        }
        if !evidence.contains(reference) {
            evidence.push(reference.clone());
        }
    }
    if evidence.is_empty() || !sources_owned(&task.project, &captures, &evidence) {
        return Ok(None);
    }
    Ok(Some(Support {
        revision: task.revision,
        review_generation: task.workflows.review_generation,
        criteria,
        evidence,
    }))
}

pub(super) fn artifact_committed(
    connection: &Connection,
    data_dir: &Path,
    task: &Snapshot,
    artifact: &Artifact,
) -> Result<(), String> {
    let kind = match artifact.kind {
        ArtifactKind::Review
            if artifact.reference == task.workflows.review_generation.to_string() =>
        {
            Kind::AcceptedReview
        }
        ArtifactKind::Handoff
            if artifact.handoff_revision == Some(task.revision)
                && !artifact.evidence.is_empty() =>
        {
            Kind::VerifiedHandoff
        }
        _ => return Ok(()),
    };
    let Some(owner) = artifact.producer.as_ref() else {
        return Ok(());
    };
    if recorded(connection, &task.project, kind)? {
        return Ok(());
    }
    let Some(support) = qualified_support(connection, data_dir, task, &artifact.evidence)? else {
        return Ok(());
    };
    insert(
        connection,
        &task.project,
        &Ribbon {
            kind,
            task_id: task.id,
            owner: owner.clone(),
            at: artifact.created_at,
            evidence: Evidence::Artifact {
                id: artifact.id.clone(),
            },
            hidden: false,
        },
        Some(&support),
    )
}

pub(super) fn move_taught(
    connection: &Connection,
    task: &Snapshot,
    owner: Attribution,
    id: i64,
) -> Result<(), String> {
    if recorded(connection, &task.project, Kind::LearnedMove)? {
        return Ok(());
    }
    let at = connection.query_row("SELECT created FROM moves WHERE id=?1 AND project=?2 AND partner=?3 AND form=?4 AND revision=1 AND forgotten=0",
        params![id, task.project, owner.partner, owner.form as i64], |row| row.get(0))
        .map_err(|error| format!("Committed learning source is unavailable: {error}"))?;
    insert(
        connection,
        &task.project,
        &Ribbon {
            kind: Kind::LearnedMove,
            task_id: task.id,
            owner,
            at,
            evidence: Evidence::Move { id, revision: 1 },
            hidden: false,
        },
        None,
    )
}

pub(super) fn set_hidden(
    connection: &Connection,
    project: &str,
    kind: Kind,
    hidden: bool,
) -> Result<(), String> {
    let mut ribbon: Ribbon = connection
        .query_row(
            "SELECT record FROM ribbons WHERE project=?1 AND kind=?2 AND record IS NOT NULL",
            params![project, kind.as_str()],
            |row| json_column(row, 0, MAX_RECORD),
        )
        .map_err(|_| "Ribbon is missing or its evidence was purged")?;
    ribbon.hidden = hidden;
    connection
        .execute(
            "UPDATE ribbons SET record=?3 WHERE project=?1 AND kind=?2 AND record IS NOT NULL",
            params![project, kind.as_str(), bounded_json(&ribbon, MAX_RECORD)?],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn records(
    connection: &Connection,
    project: &str,
) -> Result<Vec<(Ribbon, Option<Support>)>, String> {
    let mut statement = connection.prepare("SELECT kind,record,support FROM ribbons WHERE project=?1 AND record IS NOT NULL ORDER BY kind LIMIT 4")
        .map_err(|error| error.to_string())?;
    let records = statement
        .query_map([project], |row| {
            let kind: String = row.get(0)?;
            let ribbon: Ribbon = json_column(row, 1, MAX_RECORD)?;
            let support = if matches!(row.get_ref(2)?, rusqlite::types::ValueRef::Null) {
                None
            } else {
                Some(json_column(row, 2, MAX_SUPPORT)?)
            };
            if Kind::parse(&kind) != Some(ribbon.kind) {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok((ribbon, support))
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    if records.len() > 3 {
        return Err("Ribbon project capacity exceeded".into());
    }
    for (ribbon, support) in &records {
        validate(ribbon, support.as_ref())?;
    }
    Ok(records)
}

pub(super) fn views(
    connection: &Connection,
    data_dir: &Path,
    project: &str,
) -> Result<Vec<View>, String> {
    let records = records(connection, project)?;
    let current = if records.iter().any(|(_, support)| support.is_some()) {
        crate::workspace::repo::observe_inputs(Path::new(project), data_dir).unwrap_or_default()
    } else {
        runner::InputSnapshot::default()
    };
    let mut views = Vec::with_capacity(records.len());
    for (ribbon, support) in records {
        let (freshness, explanation) = inspect_source(
            connection,
            data_dir,
            project,
            &ribbon,
            support.as_ref(),
            &current,
        )?;
        views.push(View {
            ribbon,
            freshness,
            explanation,
        });
    }
    Ok(views)
}

fn inspect_source(
    connection: &Connection,
    data_dir: &Path,
    project: &str,
    ribbon: &Ribbon,
    support: Option<&Support>,
    current: &runner::InputSnapshot,
) -> Result<(Freshness, String), String> {
    let Some(task) = load_task(connection, project, ribbon.task_id)? else {
        return Ok((
            Freshness::Unavailable,
            "The original task is unavailable; historical attribution is unchanged.".into(),
        ));
    };
    match &ribbon.evidence {
        Evidence::Move { id, revision } => {
            let source: Option<(u64, i64, usize, bool, String)> = connection
                .query_row(
                    "SELECT revision,partner,form,forgotten,project FROM moves WHERE id=?1",
                    [id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| error.to_string())?;
            Ok(match source {
                None => (Freshness::Unavailable, "The original learned move is unavailable; it is never assigned to another partner.".into()),
                Some((_, partner, form, _, source_project)) if partner != ribbon.owner.partner || form != ribbon.owner.form || source_project != project =>
                    (Freshness::Ambiguous, "The retained move has conflicting original ownership.".into()),
                Some((actual, _, _, forgotten, _)) if actual != *revision || forgotten =>
                    (Freshness::Stale, "The original move revision changed or was forgotten; this is historical learning, not current move validation.".into()),
                Some(_) => (Freshness::Current, "The original committed move revision is retained; learning grants no permission.".into()),
            })
        }
        Evidence::Artifact { id } => {
            let artifact: Option<Artifact> = connection
                .query_row(
                    "SELECT record FROM local_artifacts WHERE id=?1 AND project=?2 AND task_id=?3",
                    params![id, project, task.id],
                    |row| json_column(row, 0, MAX_SUPPORT),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            let Some(artifact) = artifact else {
                return Ok((
                    Freshness::Unavailable,
                    "The original milestone artifact is unavailable.".into(),
                ));
            };
            if artifact.producer.as_ref() != Some(&ribbon.owner) || artifact.created_at != ribbon.at
            {
                return Ok((
                    Freshness::Ambiguous,
                    "The retained artifact has conflicting original attribution.".into(),
                ));
            }
            let support = support.ok_or("Artifact ribbon is missing immutable support")?;
            let captures = excerpts(connection, task.id)?;
            let runs = observations(connection, data_dir, task.id)?;
            let view = workflows::EvidenceView {
                task_id: task.id,
                excerpts: &captures,
                runs: &runs,
                current,
            };
            let mut status = Freshness::Current;
            for reference in &support.evidence {
                match view.status(reference) {
                    workflows::EvidenceStatus::Unavailable => {
                        status = Freshness::Unavailable;
                        break;
                    }
                    workflows::EvidenceStatus::Stale if status != Freshness::Unavailable => {
                        status = Freshness::Stale
                    }
                    workflows::EvidenceStatus::Ambiguous if status == Freshness::Current => {
                        status = Freshness::Ambiguous
                    }
                    _ => {}
                }
            }
            let criteria: Vec<String> = task
                .criteria
                .iter()
                .map(|criterion| criterion.text.clone())
                .collect();
            if status == Freshness::Current
                && (task.workflows.review_generation != support.review_generation
                    || criteria != support.criteria
                    || task.workflows.review_guard(&criteria, &view).verdict
                        != workflows::Verdict::Accept
                    || ribbon.kind == Kind::VerifiedHandoff && task.revision != support.revision)
            {
                status = Freshness::Stale;
            }
            let explanation = match status {
                Freshness::Current => {
                    "The original support is current at this local observation; the milestone remains historical, not a present correctness guarantee."
                }
                Freshness::Stale => {
                    "The source, review generation or handoff revision changed; the original milestone remains historical."
                }
                Freshness::Unavailable => {
                    "Original supporting evidence is missing, purged or unavailable."
                }
                Freshness::Ambiguous => {
                    "Current support cannot be established; no present verification is claimed."
                }
            };
            Ok((status, explanation.into()))
        }
    }
}

pub(super) fn purge_task(connection: &Connection, project: &str, task: i64) -> Result<(), String> {
    connection.execute("UPDATE ribbons SET record=NULL,support=NULL WHERE project=?1 AND json_extract(record,'$.task_id')=?2",
        params![project, task]).map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn purge_artifact(
    connection: &Connection,
    project: &str,
    id: &str,
) -> Result<(), String> {
    connection.execute("UPDATE ribbons SET record=NULL,support=NULL WHERE project=?1 AND json_extract(record,'$.evidence.artifact.id')=?2",
        params![project, id]).map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn purge_evidence(
    connection: &Connection,
    project: &str,
    task: i64,
    source: Option<i64>,
    run: Option<&str>,
) -> Result<(), String> {
    for (ribbon, support) in records(connection, project)? {
        if ribbon.task_id == task
            && support.is_some_and(|support| {
                support.evidence.iter().any(|reference| match reference {
                    workflows::EvidenceRef::Source { excerpt_id, .. } => {
                        source == Some(*excerpt_id)
                    }
                    workflows::EvidenceRef::Run { run_id, .. } => run == Some(run_id.as_str()),
                })
            })
        {
            connection
                .execute(
                    "UPDATE ribbons SET record=NULL,support=NULL WHERE project=?1 AND kind=?2",
                    params![project, ribbon.kind.as_str()],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn validate(ribbon: &Ribbon, support: Option<&Support>) -> Result<(), String> {
    ribbon.owner.validate()?;
    if ribbon.task_id <= 0 || ribbon.at < 0 {
        return Err("Invalid ribbon identity or timestamp".into());
    }
    match (&ribbon.evidence, ribbon.kind, support) {
        (Evidence::Move { id, revision }, Kind::LearnedMove, None)
            if *id > 0 && *revision > 0 && *revision <= i64::MAX as u64 =>
        {
            Ok(())
        }
        (
            Evidence::Artifact { id },
            Kind::AcceptedReview | Kind::VerifiedHandoff,
            Some(support),
        ) => {
            reject_secret_identity(id)?;
            if id.is_empty()
                || id.len() > 1024
                || id.chars().any(char::is_control)
                || support.revision > i64::MAX as u64
                || support.review_generation == 0
                || support.evidence.is_empty()
                || support.evidence.len() > 1024
                || !support
                    .evidence
                    .iter()
                    .any(|reference| matches!(reference, workflows::EvidenceRef::Source { .. }))
                || !support
                    .evidence
                    .iter()
                    .any(|reference| matches!(reference, workflows::EvidenceRef::Run { .. }))
            {
                return Err("Invalid ribbon artifact support".into());
            }
            if support.criteria.is_empty()
                || support.criteria.len() > workflows::MAX_ENTRIES
                || support.criteria.iter().any(|criterion| {
                    criterion.trim().is_empty()
                        || criterion.len() > 256
                        || crate::workspace::privacy::redact(criterion).as_ref() != criterion
                })
            {
                return Err("Invalid ribbon acceptance criteria".into());
            }
            for (index, reference) in support.evidence.iter().enumerate() {
                reference.validate(ribbon.task_id)?;
                if support.evidence[..index].contains(reference) {
                    return Err("Duplicate ribbon support reference".into());
                }
            }
            Ok(())
        }
        _ => Err("Ribbon kind, evidence and support disagree".into()),
    }
}

pub(super) fn inspect(connection: &Connection) -> Result<(), String> {
    inspect_columns(
        connection,
        "ribbons",
        &[
            ("project", "TEXT", true, 1),
            ("kind", "TEXT", true, 2),
            ("record", "TEXT", false, 0),
            ("support", "TEXT", false, 0),
        ],
    )?;
    let sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='ribbons'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let normalized = |value: &str| {
        value
            .chars()
            .filter(|ch| !ch.is_ascii_whitespace())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    if normalized(&sql) != normalized(TABLE) {
        return Err("Unrecognized ribbon table constraints".into());
    }
    let invalid: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM ribbons r WHERE kind NOT IN ('accepted_review','verified_handoff','learned_move')
        OR typeof(project)!='text' OR length(CAST(project AS BLOB)) NOT BETWEEN 1 AND 4096
        OR (record IS NULL AND support IS NOT NULL) OR (record IS NOT NULL AND typeof(record)!='text')
        OR (support IS NOT NULL AND typeof(support)!='text')
        OR EXISTS(SELECT 1 FROM json_each(r.record,'$.owner') WHERE key NOT IN ('partner','name','form'))
        OR NOT EXISTS(SELECT 1 FROM tasks t WHERE t.project=r.project))
        OR EXISTS(SELECT 1 FROM ribbons GROUP BY project,kind HAVING count(*)!=1)
        OR EXISTS(SELECT 1 FROM ribbons GROUP BY project HAVING count(*)>3)", [], |row| row.get(0)).map_err(|error| error.to_string())?;
    if invalid {
        return Err("Invalid ribbon project, tombstone or bounded identity".into());
    }
    let mut statement = connection
        .prepare("SELECT DISTINCT project FROM ribbons")
        .map_err(|error| error.to_string())?;
    for project in statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
    {
        let project = project.map_err(|error| error.to_string())?;
        reject_secret_identity(&project)?;
        if project.chars().any(char::is_control) {
            return Err("Invalid ribbon project identity".into());
        }
        for (ribbon, support) in records(connection, &project)? {
            if load_task(connection, &project, ribbon.task_id)?.is_none() {
                return Err("Ribbon task ownership is unavailable".into());
            }
            match &ribbon.evidence {
                Evidence::Artifact { id } => {
                    let artifact: Artifact = connection.query_row("SELECT record FROM local_artifacts WHERE id=?1 AND project=?2 AND task_id=?3",
                        params![id, project, ribbon.task_id], |row| json_column(row, 0, MAX_SUPPORT))
                        .map_err(|_| "Ribbon artifact source is missing or foreign")?;
                    let support = support.as_ref().ok_or("Missing ribbon support")?;
                    let captures = excerpts(connection, ribbon.task_id)?;
                    let runs = super::records::runs_for(connection, ribbon.task_id)?;
                    super::records::validate_references(
                        ribbon.task_id,
                        &support.evidence,
                        &captures,
                        &runs,
                    )?;
                    if !sources_owned(&project, &captures, &support.evidence) {
                        return Err("Foreign ribbon source scope".into());
                    }
                    if artifact.producer.as_ref() != Some(&ribbon.owner)
                        || artifact.created_at != ribbon.at
                        || !artifact
                            .evidence
                            .iter()
                            .all(|reference| support.evidence.contains(reference))
                        || match ribbon.kind {
                            Kind::AcceptedReview => {
                                artifact.kind != ArtifactKind::Review
                                    || artifact.reference != support.review_generation.to_string()
                            }
                            Kind::VerifiedHandoff => {
                                artifact.kind != ArtifactKind::Handoff
                                    || artifact.handoff_revision != Some(support.revision)
                            }
                            Kind::LearnedMove => true,
                        }
                    {
                        return Err(
                            "Ribbon source differs from its immutable recorded evidence".into()
                        );
                    }
                }
                Evidence::Move { id, revision } => {
                    let original: Option<(u64, i64, usize, i64, String)> = connection
                        .query_row(
                            "SELECT revision,partner,form,created,project FROM moves WHERE id=?1",
                            [id],
                            |row| {
                                Ok((
                                    row.get(0)?,
                                    row.get(1)?,
                                    row.get(2)?,
                                    row.get(3)?,
                                    row.get(4)?,
                                ))
                            },
                        )
                        .optional()
                        .map_err(|error| error.to_string())?;
                    if original.is_some_and(|(actual, partner, form, at, source_project)| {
                        actual < *revision
                            || partner != ribbon.owner.partner
                            || form != ribbon.owner.form
                            || at != ribbon.at
                            || source_project != project
                    }) {
                        return Err("Ribbon move source has conflicting immutable ownership".into());
                    }
                }
            }
        }
    }
    Ok(())
}
