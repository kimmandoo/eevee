//! Party partner rows: loading, roster mutations and the lead partner's form write.
use super::*;
use unicode_segmentation::UnicodeSegmentation;

const PARTNER_COLUMNS: &[ColumnSpec; 7] = &[
    ("id", "INTEGER", false, 1),
    ("nickname", "TEXT", false, 0),
    ("form", "INTEGER", true, 0),
    ("met_at", "INTEGER", true, 0),
    ("met_derived", "INTEGER", true, 0),
    ("created_project", "TEXT", true, 0),
    ("auto_form", "INTEGER", true, 0),
];

/// Validate a nonempty nickname before writing; legacy rows retain their
/// existing storage-only admission bounds.
pub fn validate_nickname(nickname: &str) -> Result<(), String> {
    if nickname.is_empty() {
        return Err("Use a nonempty nickname, or clear it to restore the default name".into());
    }
    if nickname.len() > MAX_NICKNAME_BYTES {
        return Err("A partner nickname must fit within 512 UTF-8 bytes".into());
    }
    if nickname.graphemes(true).count() > 12 {
        return Err("A partner nickname must contain at most 12 graphemes".into());
    }
    if nickname.chars().any(|character| {
        character.is_control()
            || matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    }) {
        return Err("A partner nickname cannot contain control or unsafe terminal characters".into());
    }
    Ok(())
}

pub(super) fn validate_partner(partner: &Partner) -> Result<(), String> {
    // Ids are AUTOINCREMENT and never reused, so they grow past the six-seat
    // capacity after releases; the roster cap applies to the row count.
    if partner.id < 1 {
        return Err("A partner id must be a positive AUTOINCREMENT value".into());
    }
    if partner.form >= 9 {
        return Err("A stored partner form is outside 0..=8".into());
    }
    if partner.met_at < 0 {
        return Err("A partner met timestamp must not be negative".into());
    }
    if partner
        .nickname
        .as_deref()
        .is_some_and(|n| n.len() > MAX_NICKNAME_BYTES)
    {
        return Err("A partner nickname exceeds the 512-byte storage limit".into());
    }
    if partner.created_project.len() > 4096 {
        return Err("A partner origin project exceeds the 4096-byte limit".into());
    }
    Ok(())
}

/// Shared owner-idle gate for partner lifecycle and local scoped preferences.
pub(super) fn idle(connection: &Connection, partner: i64) -> Result<(), String> {
    let active: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM active_work WHERE partner=?1)",
            [partner],
            |row| row.get(0),
        )
        .map_err(|error| format!("Cannot inspect the partner's active work: {error}"))?;
    if active {
        return Err("This partner owns active work; cancel and wait before changing it".into());
    }
    let mut statement = connection.prepare(
        "SELECT e.record,t.lead_partner FROM executions e LEFT JOIN tasks t ON t.id=e.task_id WHERE e.reserved=1",
    ).map_err(|error| error.to_string())?;
    let reservations = statement
        .query_map([], |row| {
            Ok((
                json_column::<crate::workspace::runner::RunRecord>(row, 0, MAX_RUN_BYTES)?,
                row.get::<_, Option<i64>>(1)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    for reservation in reservations {
        let (record, lead) = reservation.map_err(|error| error.to_string())?;
        // Pre-attribution runs cannot prove a different owner.
        if record
            .prepared
            .partner
            .as_ref()
            .map_or(lead.is_none_or(|lead| lead == partner), |owner| {
                owner.partner == partner
            })
        {
            return Err(
                "This partner owns a reserved execution; cancel and wait before changing it".into(),
            );
        }
    }
    Ok(())
}

impl State {
    pub(super) fn nickname_partner(
        &mut self,
        partner: i64,
        nickname: Option<String>,
    ) -> Result<Vec<Partner>, String> {
        if let Some(nickname) = nickname.as_deref() {
            validate_nickname(nickname)?;
        }
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot begin partner rename: {error}"))?;
        let roster = partners(&transaction)?;
        if !roster.iter().any(|member| member.id == partner) {
            return Err(
                "The partner to rename does not exist; refresh the party and try again".into(),
            );
        }
        if let Some(candidate) = nickname.as_deref()
            && roster.iter().any(|member| {
                member.id != partner
                    && member.nickname.as_deref().is_some_and(|existing| {
                        existing
                            .chars()
                            .flat_map(char::to_lowercase)
                            .eq(candidate.chars().flat_map(char::to_lowercase))
                    })
            })
        {
            return Err("Another partner already uses that nickname; choose a unique name".into());
        }
        transaction
            .execute(
                "UPDATE partners SET nickname=?1 WHERE id=?2",
                params![nickname, partner],
            )
            .map_err(|error| format!("Cannot persist the partner nickname: {error}"))?;
        enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
        let roster = partners(&transaction)?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit the partner nickname: {error}"))?;
        Ok(roster)
    }

    /// Welcome a new partner: starts in Eevee form with today's met date on the
    /// current project. The six-seat party capacity is enforced inside the
    /// transaction, and AUTOINCREMENT never reuses a released partner's id.
    pub(super) fn add_partner(&mut self) -> Result<Vec<Partner>, String> {
        let met_at = timestamp()?;
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot begin partner addition: {error}"))?;
        let seated: i64 = transaction
            .query_row("SELECT count(*) FROM partners", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if seated >= MAX_PARTNERS as i64 {
            return Err(
                "The party is full: a partner can hold at most 6 members; release one first".into(),
            );
        }
        transaction
            .execute(
                "INSERT INTO partners(nickname,form,met_at,met_derived,created_project,auto_form)
                 VALUES (NULL,0,?1,0,?2,1)",
                params![met_at, self.project],
            )
            .map_err(|error| format!("Cannot add the partner: {error}"))?;
        enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
        let roster = partners(&transaction)?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit the new partner: {error}"))?;
        Ok(roster)
    }

    /// Release only an idle member. Task facts and historical attribution are
    /// shared evidence, while recipient drafts and pending relays are personal.
    pub(super) fn release_partner(&mut self, partner: i64) -> Result<Project, String> {
        let connection = self
            .connection
            .as_mut()
            .ok_or("Storage is unavailable; retry opening it")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot begin partner release: {error}"))?;
        let roster = partners(&transaction)?;
        if !roster.iter().any(|member| member.id == partner) {
            return Err(
                "The partner to release does not exist; refresh the party and try again".into(),
            );
        }
        let retained = roster
            .iter()
            .find(|member| member.id != partner)
            .ok_or("The last partner cannot be released")?;
        idle(&transaction, partner)?;
        let updated_at = timestamp()?;
        {
            let mut statement = transaction
                .prepare(&format!(
                    "SELECT {TASK_COLUMNS},{TASK_AUTO_FORM} FROM tasks ORDER BY id"
                ))
                .map_err(|error| error.to_string())?;
            let tasks = statement
                .query_map([], snapshot_row)
                .map_err(|error| error.to_string())?;
            for task in tasks {
                let mut task = task.map_err(|error| error.to_string())?;
                let lead_changed = task.lead_partner == partner;
                let removed_draft = task.partner_drafts.remove(&partner).is_some();
                let command_relays = task.workflows.command_relays.len();
                let draft_relays = task.workflows.draft_relays.len();
                let parked_drafts = task.workflows.parked_drafts.len();
                task.workflows
                    .parked_drafts
                    .retain(|entry| entry.partner != partner);
                task.workflows
                    .command_relays
                    .retain(|relay| relay.partner != partner);
                task.workflows
                    .draft_relays
                    .retain(|relay| relay.partner != partner);
                if !lead_changed
                    && !removed_draft
                    && command_relays == task.workflows.command_relays.len()
                    && draft_relays == task.workflows.draft_relays.len()
                    && parked_drafts == task.workflows.parked_drafts.len()
                {
                    continue;
                }
                if lead_changed {
                    task.lead_partner = retained.id;
                    task.form = retained.form;
                    task.auto_form = retained.auto_form;
                    task.memory_draft = serde_json::Value::Null;
                    task.workflow_draft = serde_json::Value::Null;
                    task.navigation.form_selection = retained.form;
                    task.navigation.previous_form = None;
                    task.navigation.overlay = None;
                    task.navigation.editing = false;
                    task.navigation.editor_field = EditorField::default();
                }
                task.revision = task
                    .revision
                    .checked_add(1)
                    .filter(|revision| *revision <= i64::MAX as u64)
                    .ok_or("Cannot release a partner: an affected task revision is exhausted")?;
                transaction
                    .execute(
                        "UPDATE tasks SET lead_partner=?1,form=?2,revision=?3,partner_drafts=?4,
                         drafts='[]',navigation=?5,memory_draft=?6,workflow_draft=?7,
                         workflows=?8,updated_at=?9 WHERE id=?10",
                        params![
                            task.lead_partner,
                            task.form as i64,
                            task.revision as i64,
                            bounded_json(&task.partner_drafts, MAX_PARTNER_DRAFTS_BYTES)?,
                            bounded_json(&task.navigation, 1024)?,
                            bounded_json(&task.memory_draft, MAX_MEMORY_BYTES)?,
                            bounded_json(&task.workflow_draft, MAX_WORKFLOW_BYTES)?,
                            bounded_json(&task.workflows, MAX_WORKFLOW_BYTES)?,
                            updated_at,
                            task.id,
                        ],
                    )
                    .map_err(|error| format!("Cannot repair a released partner's task: {error}"))?;
            }
        }
        super::moves::release_partner(&transaction, partner)?;
        transaction
            .execute("DELETE FROM partner_pairings WHERE partner=?1", [partner])
            .map_err(|error| error.to_string())?;
        transaction
            .execute("DELETE FROM held_items WHERE partner=?1", [partner])
            .map_err(|error| error.to_string())?;
        transaction
            .execute("DELETE FROM screenshots WHERE partner=?1", [partner])
            .map_err(|error| error.to_string())?;
        transaction
            .execute("DELETE FROM partners WHERE id=?1", [partner])
            .map_err(|error| format!("Cannot release the partner: {error}"))?;
        enforce_evidence_quota(&transaction, &self.data_dir, self.policy)?;
        let project = load(&transaction, &self.project)?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit the partner release: {error}"))?;
        Ok(project)
    }
}

fn partner_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Partner> {
    let auto_form: i64 = row.get(6)?;
    if !(0..=1).contains(&auto_form) {
        return Err(rusqlite::Error::IntegralValueOutOfRange(6, auto_form));
    }
    Ok(Partner {
        id: row.get(0)?,
        nickname: row.get(1)?,
        form: row.get(2)?,
        auto_form: auto_form != 0,
        met_at: row.get(3)?,
        met_derived: row.get(4)?,
        created_project: row.get(5)?,
    })
}

pub(super) fn partners(connection: &Connection) -> Result<Vec<Partner>, String> {
    let mut statement = connection
        .prepare("SELECT id,nickname,form,met_at,met_derived,created_project,auto_form FROM partners ORDER BY id LIMIT 7")
        .map_err(|error| error.to_string())?;
    let partners = statement
        .query_map([], partner_row)
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    if partners.len() > MAX_PARTNERS {
        return Err("Party exceeds its bounded capacity of six partners".into());
    }
    for partner in &partners {
        validate_partner(partner)?;
    }
    Ok(partners)
}

/// Persist the lead partner's form and mode together when its task is saved.
/// The partner row owns mode globally across all tasks and projects.
pub(super) fn set_partner_form(
    connection: &Connection,
    partner: i64,
    form: usize,
    auto_form: bool,
) -> Result<(), String> {
    let updated = connection
        .execute(
            "UPDATE partners SET form=?1,auto_form=?2 WHERE id=?3",
            params![form as i64, auto_form, partner],
        )
        .map_err(|error| format!("Cannot persist the partner form: {error}"))?;
    if updated != 1 {
        return Err("The task's lead partner does not exist".into());
    }
    Ok(())
}

/// Partner draft JSON uses the column reserved in Schema 7 and enlarged in
/// Schema 8. Typed draft bounds are validated with the task; inspection also
/// refuses a non-object container before any writable open.
fn valid_partner_drafts(text: &str) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| format!("Invalid partner drafts JSON: {error}"))?;
    if !value.is_object() {
        return Err("Partner drafts must be a JSON object keyed by partner id".into());
    }
    validate_draft_shape(&value, 0)
}
pub(super) fn inspect_partners(connection: &Connection, auto_form: bool) -> Result<(), String> {
    let columns = if auto_form {
        &PARTNER_COLUMNS[..]
    } else {
        &PARTNER_COLUMNS[..6]
    };
    inspect_columns(connection, "partners", columns)?;
    let mode = if auto_form { "auto_form" } else { "(form=0)" };
    let rows = {
        let mut statement = connection
            .prepare(&format!("SELECT id,nickname,form,met_at,met_derived,created_project,{mode} FROM partners ORDER BY id"))
            .map_err(|error| error.to_string())?;
        statement
            .query_map([], partner_row)
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?
    };
    if rows.is_empty() {
        return Err("A party store must hold at least one partner".into());
    }
    for (index, partner) in rows.iter().enumerate() {
        validate_partner(partner)?;
        if rows[..index].iter().any(|other| other.id == partner.id) {
            return Err("Duplicate partner identity".into());
        }
    }
    let invalid: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks t LEFT JOIN partners p ON p.id=t.lead_partner WHERE p.id IS NULL)",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if invalid {
        return Err("A task lead partner refers to a missing partner".into());
    }
    let mut statement = connection
        .prepare("SELECT partner_drafts FROM tasks")
        .map_err(|error| error.to_string())?;
    let drafts = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    for text in drafts {
        valid_partner_drafts(&text.map_err(|error| error.to_string())?)?;
    }
    let mut statement = connection
        .prepare("SELECT workflows FROM tasks")
        .map_err(|e| e.to_string())?;
    let workflows = statement
        .query_map([], |row| {
            json_column::<crate::workspace::workflows::State>(row, 0, MAX_WORKFLOW_BYTES)
        })
        .map_err(|e| e.to_string())?;
    for state in workflows {
        let state = state.map_err(|e| e.to_string())?;
        if state
            .parked_drafts
            .iter()
            .any(|entry| !rows.iter().any(|partner| partner.id == entry.partner))
        {
            return Err("A parked draft refers to a missing partner".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod release_tests {
    use super::*;
    use crate::workspace::{memory, runner, workflows};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "ev-partner-release-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn state(&self, project: &str) -> State {
            let mut state = State {
                data_dir: self.0.clone(),
                project: project.into(),
                policy: Policy::default(),
                connection: None,
                lock: None,
            };
            state.open_and_load().unwrap();
            state
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn task(store: &mut State, lead: i64, form: usize) -> Snapshot {
        let mut task = Snapshot::empty(store.project.clone());
        task.goal = "Retained task facts".into();
        task.revision = 1;
        task.lead_partner = lead;
        task.form = form;
        store.save(task).unwrap()
    }

    fn draft(text: &str) -> PartnerDrafts {
        PartnerDrafts {
            drafts: vec![Draft {
                field: EditorField::Request,
                text: text.into(),
                cursor: text.len(),
            }],
            editor_field: EditorField::Request,
            editing: true,
            previous_form: Some(3),
        }
    }

    fn prepared(store: &State, task: &Snapshot, owner: Option<i64>) -> runner::Prepared {
        runner::Prepared {
            request_id: "release-command-request".into(),
            run_id: "release-command-run".into(),
            task_id: task.id,
            spec: runner::CommandSpec::Direct {
                program: "echo".into(),
                args: vec!["retained output".into()],
            },
            cwd: store.data_dir.clone(),
            cwd_identity: "fixture-directory".into(),
            executable: runner::ExecutableIdentity {
                path: store.data_dir.join("fixture-executable"),
                sha256: "0".repeat(64),
                bytes: 1,
                modified_ns: 1,
                file_id: "fixture-file".into(),
            },
            prepared_at_ms: timestamp().unwrap() as u64,
            expires_at_ms: timestamp().unwrap() as u64 + 60_000,
            before: runner::InputSnapshot::default(),
            partner: owner.map(|id| {
                Attribution::of(
                    &partners(store.connection.as_ref().unwrap())
                        .unwrap()
                        .into_iter()
                        .find(|partner| partner.id == id)
                        .unwrap(),
                )
            }),
            rerun_of: None,
            approval_arrival: crate::providers::approval_arrival(),
        }
    }

    #[test]
    fn release_refuses_missing_and_last_partner_without_mutation() {
        let directory = Directory::new();
        let mut store = directory.state("project");
        let saved = task(&mut store, 1, 3);
        for partner in [0, -1, 999] {
            assert!(
                store
                    .release_partner(partner)
                    .unwrap_err()
                    .contains("does not exist")
            );
        }
        assert!(
            store
                .release_partner(1)
                .unwrap_err()
                .contains("last partner")
        );
        let project = load(store.connection.as_ref().unwrap(), "project").unwrap();
        assert_eq!(project.tasks, vec![saved]);
        assert_eq!(project.partners.len(), 1);
    }

    #[test]
    fn idle_release_repairs_every_project_and_retains_shared_evidence_after_restart() {
        let directory = Directory::new();
        let mut store = directory.state("project");
        store.add_partner().unwrap();
        store.add_partner().unwrap();
        let mut current = task(&mut store, 1, 3);
        let retained = draft("Retained partner request");
        current.partner_drafts.insert(1, draft("Released request"));
        current.partner_drafts.insert(2, retained.clone());
        current.memory_draft = serde_json::json!({"text": "Released memory proposal"});
        current.workflow_draft = serde_json::json!({"text": "Released workflow proposal"});
        current.navigation.editing = true;
        current.navigation.editor_field = EditorField::Request;
        current.navigation.previous_form = Some(2);
        current.navigation.overlay = Some(7);
        current.workflows.chosen_decision = Some(workflows::ChosenDecision {
            id: "retained-decision".into(),
            text: "Shared decision".into(),
        });
        current
            .workflows
            .patch_authors
            .push(workflows::PatchAuthor {
                partner: 1,
                path: "source.rs".into(),
                source_sha256: [0; 32],
                record_id: "retained-patch".into(),
            });
        for partner in [1, 2] {
            current
                .workflows
                .command_relays
                .push(workflows::CommandRelay {
                    partner,
                    run_id: "retained-run".into(),
                });
            current.workflows.draft_relays.push(workflows::DraftRelay {
                partner,
                form: 3,
                text: "Recipient intent".into(),
                paths: Vec::new(),
                decision: None,
                role_action: None,
            });
        }
        let memories = store
            .memory_request(MemoryRequest {
                task_id: current.id,
                mutation: Some(memory::Mutation::Approve(memory::Proposal {
                    scope: memory::Scope::Task,
                    content: "Retained approved memory".into(),
                    provenance: memory::Provenance {
                        kind: memory::SourceKind::UserAuthored,
                        reference: "Explicitly approved".into(),
                        task_id: current.id,
                    },
                })),
            })
            .unwrap();
        current.approved_memories = memories.clone();
        current.revision += 1;
        current = store.save(current).unwrap();
        let provider = ProviderRecord {
            id: "retained-provider-evidence".into(),
            agent: crate::providers::Agent::Codex,
            generation: 1,
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "item".into(),
            origin: "Codex provider-native".into(),
            coverage: "Public observed result only".into(),
            text: "Retained observation".into(),
            complete: true,
            partner: Some(Attribution {
                partner: 1,
                name: "Original name".into(),
                form: 3,
            }),
        };
        store
            .journal_request(JournalRequest {
                task_id: current.id,
                operation: JournalOperation::Put(Box::new(provider.clone())),
            })
            .unwrap();
        use sha2::Digest;
        let sources = Directory::new();
        fs::write(sources.0.join("source.rs"), b"Retained source\n").unwrap();
        let source = store
            .context(ContextRequest {
                producer: None,
                task_id: current.id,
                operation: ContextOperation::Pin(crate::workspace::context::Excerpt {
                    id: 0,
                    task_id: current.id,
                    root: sources.0.to_str().unwrap().into(),
                    path: "source.rs".into(),
                    start_line: 1,
                    end_line: 1,
                    source_sha256: sha2::Sha256::digest(b"Retained source\n").into(),
                    text: "Retained source\n".into(),
                    available: true,
                    origin_run: None,
                }),
            })
            .unwrap();
        let prepared = prepared(&store, &current, Some(1));
        store
            .reserve_run(current.id, prepared.clone(), 1024)
            .unwrap();
        let run = match store
            .run_request(RunRequest {
                task_id: current.id,
                operation: RunOperation::Abandon {
                    request_id: prepared.request_id,
                    run_id: prepared.run_id,
                },
            })
            .unwrap()
        {
            RunResponse::Updated(record) => record,
            other => panic!("expected retained run: {other:?}"),
        };
        store.project = "other".into();
        let other = task(&mut store, 1, 3);
        store
            .manage(Management::Archive {
                id: other.id,
                archived: true,
            })
            .unwrap();
        let other = load_task(store.connection.as_ref().unwrap(), "other", other.id)
            .unwrap()
            .unwrap();
        let unchanged = task(&mut store, 2, 5);
        let mut draft_only = task(&mut store, 3, 6);
        draft_only
            .partner_drafts
            .insert(1, draft("Discard only this draft"));
        draft_only.memory_draft = serde_json::json!({"text": "Keep other lead's proposal"});
        draft_only.workflow_draft = serde_json::json!({"text": "Keep other lead's workflow"});
        draft_only.revision += 1;
        draft_only = store.save(draft_only).unwrap();
        store.project = "project".into();
        let project = store.release_partner(1).unwrap();
        assert_eq!(project.active_id, Some(current.id));
        assert_eq!(
            project
                .partners
                .iter()
                .map(|partner| partner.id)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        let released = &project.tasks[0];
        assert_eq!(released.lead_partner, 2);
        assert_eq!(released.form, 5);
        assert_eq!(released.revision, current.revision + 1);
        assert_eq!(released.partner_drafts.get(&2), Some(&retained));
        assert!(!released.partner_drafts.contains_key(&1));
        assert!(released.memory_draft.is_null() && released.workflow_draft.is_null());
        assert_eq!(released.approved_memories, memories);
        assert!(!released.navigation.editing);
        assert_eq!(released.navigation.overlay, None);
        assert_eq!(released.navigation.previous_form, None);
        assert_eq!(released.navigation.form_selection, 5);
        let mut expected_workflows = current.workflows.clone();
        expected_workflows
            .command_relays
            .retain(|relay| relay.partner != 1);
        expected_workflows
            .draft_relays
            .retain(|relay| relay.partner != 1);
        assert_eq!(released.workflows, expected_workflows);
        let connection = store.connection.as_ref().unwrap();
        let repaired = load_task(connection, "other", other.id).unwrap().unwrap();
        assert_eq!((repaired.lead_partner, repaired.form), (2, 5));
        assert_eq!(repaired.revision, other.revision + 1);
        assert!(repaired.archived);
        assert_eq!(
            load_task(connection, "other", unchanged.id).unwrap(),
            Some(unchanged)
        );
        let cleaned = load_task(connection, "other", draft_only.id)
            .unwrap()
            .unwrap();
        assert_eq!(cleaned.revision, draft_only.revision + 1);
        assert_eq!(cleaned.lead_partner, 3);
        assert!(cleaned.partner_drafts.is_empty());
        assert_eq!(cleaned.memory_draft, draft_only.memory_draft);
        assert_eq!(cleaned.workflow_draft, draft_only.workflow_draft);
        drop(store);
        let mut reopened = directory.state("project");
        assert_eq!(
            load(reopened.connection.as_ref().unwrap(), "project")
                .unwrap()
                .tasks,
            project.tasks
        );
        assert_eq!(
            reopened
                .memory_request(MemoryRequest {
                    task_id: current.id,
                    mutation: None
                })
                .unwrap(),
            memories
        );
        assert!(matches!(
            reopened.journal_request(JournalRequest { task_id: current.id, operation: JournalOperation::List }).unwrap(),
            JournalResponse::Records(records) if records == vec![provider]
        ));
        assert_eq!(reopened.load_runs(current.id).unwrap(), vec![run]);
        assert_eq!(
            reopened
                .context(ContextRequest {
                    task_id: current.id,
                    producer: None,
                    operation: ContextOperation::Load
                })
                .unwrap(),
            source,
        );
    }

    #[test]
    fn release_refuses_work_claims_on_other_tasks_and_projects() {
        let directory = Directory::new();
        let mut store = directory.state("other");
        store.add_partner().unwrap();
        let saved = task(&mut store, 1, 3);
        store.project = "project".into();
        for writing in [false, true] {
            let connection = store.connection.as_ref().unwrap();
            connection
                .execute(
                    "INSERT INTO active_work(task_id,partner,request_id,owner,writing,kind)
                 VALUES (?1,2,'release-active',?2,?3,'provider')",
                    params![
                        saved.id,
                        serde_json::to_string(&Attribution {
                            partner: 2,
                            name: "Busy partner".into(),
                            form: 3,
                        })
                        .unwrap(),
                        writing
                    ],
                )
                .unwrap();
            assert!(
                store
                    .release_partner(2)
                    .unwrap_err()
                    .contains("active work")
            );
            let connection = store.connection.as_ref().unwrap();
            assert_eq!(partners(connection).unwrap().len(), 2);
            assert_eq!(
                load_task(connection, "other", saved.id).unwrap(),
                Some(saved.clone())
            );
            connection
                .execute(
                    "DELETE FROM active_work WHERE request_id='release-active'",
                    [],
                )
                .unwrap();
        }
        assert_eq!(store.release_partner(2).unwrap().partners.len(), 1);
    }

    #[test]
    fn idle_fourth_release_preserves_three_other_live_owners() {
        let directory = Directory::new();
        let mut store = directory.state("project");
        for _ in 0..3 {
            store.add_partner().unwrap();
        }
        let mut saved = task(&mut store, 4, 0);
        saved
            .partner_drafts
            .insert(4, draft("released future request"));
        saved.revision += 1;
        let saved = store.save(saved).unwrap();
        let roster = partners(store.connection.as_ref().unwrap()).unwrap();
        let mut expected = Vec::new();
        for partner in 1..=3 {
            let claim = WorkClaim {
                task_id: saved.id,
                owner: Attribution::of(roster.iter().find(|member| member.id == partner).unwrap()),
                request_id: format!("other-live-owner-{partner}"),
                writing: partner == 1,
            };
            store
                .work_request(&WorkRequest::Acquire {
                    claim: claim.clone(),
                    task: Box::new(saved.clone()),
                })
                .unwrap();
            expected.push(claim);
        }
        let project = store.release_partner(4).unwrap();
        assert_eq!(
            project
                .partners
                .iter()
                .map(|member| member.id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(project.active_id, Some(saved.id));
        assert_eq!(project.tasks[0].lead_partner, 1);
        assert!(!project.tasks[0].partner_drafts.contains_key(&4));
        assert_eq!(
            super::super::work::claims(store.connection.as_ref().unwrap()).unwrap(),
            expected
        );
        for claim in &expected {
            validate_work_input(&directory.0, &saved, &[], claim).unwrap();
            assert!(store.release_partner(claim.owner.partner).is_err());
        }
    }

    #[test]
    fn release_checks_reserved_execution_attribution_and_legacy_lead() {
        for owner in [Some(2), None, Some(1)] {
            let directory = Directory::new();
            let mut store = directory.state("other");
            store.add_partner().unwrap();
            let lead = if owner.is_none() { 2 } else { 1 };
            let saved = task(&mut store, lead, 2);
            let prepared = prepared(&store, &saved, owner);
            store.reserve_run(saved.id, prepared, 1024).unwrap();
            // Exercise the execution guard independently of the new claim
            // table, including reservations from pre-attribution stores.
            store
                .connection
                .as_ref()
                .unwrap()
                .execute("DELETE FROM active_work", [])
                .unwrap();
            store.project = "project".into();
            if owner == Some(1) {
                assert!(store.release_partner(2).is_ok());
            } else {
                assert!(
                    store
                        .release_partner(2)
                        .unwrap_err()
                        .contains("reserved execution")
                );
                assert_eq!(
                    partners(store.connection.as_ref().unwrap()).unwrap().len(),
                    2
                );
            }
        }
    }

    #[test]
    fn release_rolls_back_earlier_task_repairs_when_a_later_revision_is_exhausted() {
        let directory = Directory::new();
        let mut store = directory.state("project");
        store.add_partner().unwrap();
        let first = task(&mut store, 1, 3);
        let last = task(&mut store, 1, 3);
        store
            .connection
            .as_ref()
            .unwrap()
            .execute(
                "UPDATE tasks SET revision=?1 WHERE id=?2",
                params![i64::MAX, last.id],
            )
            .unwrap();
        assert!(
            store
                .release_partner(1)
                .unwrap_err()
                .contains("revision is exhausted")
        );
        let connection = store.connection.as_ref().unwrap();
        assert_eq!(partners(connection).unwrap().len(), 2);
        assert_eq!(
            load_task(connection, "project", first.id).unwrap(),
            Some(first)
        );
        assert_eq!(
            load_task(connection, "project", last.id)
                .unwrap()
                .unwrap()
                .revision,
            i64::MAX as u64
        );
    }
}
