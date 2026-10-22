use super::*;
use crate::{
    app::StorageState,
    providers, store,
    workspace::{
        context,
        held::{Item, Target},
        memory,
    },
};

const PROJECT: &str = if cfg!(windows) {
    r"C:\fixture"
} else {
    "/fixture"
};

fn app() -> App {
    let mut app = App::new(PROJECT.into(), 120, 36);
    app.task_id = 7;
    app.storage = StorageState::Saved;
    app.repo_loading = false;
    app.records.loaded = true;
    app.memory.reset(7, PROJECT);
    app.saved_revision = app.revision;
    app.overlay = Some(crate::app::Overlay::Memory);
    app
}

fn note(scope: memory::Scope) -> memory::Record {
    memory::Record {
        id: 31,
        revision: 4,
        project: PROJECT.into(),
        task_id: (scope == memory::Scope::Task).then_some(7),
        proposal: memory::Proposal {
            scope,
            content: "Use focused local verification before handoff.".into(),
            provenance: memory::Provenance {
                kind: memory::SourceKind::UserAuthored,
                reference: "task:7/next-action".into(),
                task_id: 7,
            },
        },
    }
}

fn saved(app: &mut App, target: Target) {
    app.records.snapshot.held_items = vec![Item {
        partner: app.attribution().partner,
        form: app.form(),
        target,
    }];
    app.reconcile_held();
}

fn memory_target() -> Target {
    Target::Memory {
        id: 31,
        revision: 4,
    }
}

fn capture(app: &mut App) -> context::Excerpt {
    let excerpt = context::Excerpt {
        origin_run: None,
        id: 0,
        task_id: 7,
        root: PROJECT.into(),
        path: "sample.rs".into(),
        start_line: 1,
        end_line: 1,
        source_sha256: [7; 32],
        text: "fn sample() {}\n".into(),
        available: true,
    };
    use crate::workspace::repo;
    use std::{collections::BTreeMap, sync::Arc};
    let path = std::path::PathBuf::from(&excerpt.path);
    let snapshot = Arc::new(repo::Snapshot {
        head: Some("fixture-head".into()),
        files: BTreeMap::from([(
            path.clone(),
            repo::FileIdentity {
                path,
                tracked: true,
                head: None,
                index: vec![],
                state: repo::FileState::Regular {
                    sha256: excerpt.source_sha256,
                    bytes: excerpt.text.len() as u64,
                    binary: false,
                    executable: false,
                },
                dirty: Some(false),
            },
        )]),
        working_diff: Default::default(),
        complete: true,
        stable: true,
        warnings: vec![],
    });
    app.repository = Some(Arc::new(repo::Report {
        repository: repo::RepositoryIdentity {
            root: PROJECT.into(),
            git_root: Some(PROJECT.into()),
            git_dir: Some(std::path::Path::new(PROJECT).join(".git")),
            storage_root: None,
        },
        baseline: snapshot.clone(),
        current: snapshot,
        changes: vec![],
        initial: true,
    }));
    app.overlay = Some(crate::app::Overlay::Sources);
    app.sources.files = vec![context::Source {
        path: "sample.rs".into(),
        bytes: excerpt.text.len() as u64,
        sha256: excerpt.source_sha256,
    }];
    app.sources.event(
        7,
        context::Event {
            task_id: 7,
            result: Ok(context::Response::Inspected(excerpt.clone())),
        },
    );
    app.sources.preview_reviewed = true;
    excerpt
}

#[test]
fn stored_source_boundary_disables_held_file_availability_without_rewriting_capture() {
    let mut app = app();
    let original = capture(&mut app);
    let target = Target::File {
        path: original.path.clone(),
    };
    assert!(app.held_available(&target));
    std::sync::Arc::make_mut(app.repository.as_mut().unwrap())
        .repository
        .storage_root = Some(std::path::PathBuf::from(PROJECT));
    assert!(!app.held_available(&target));
    assert_eq!(app.sources.preview.as_ref(), Some(&original));
}

fn input(app: &App) -> providers::Input {
    let mut task = store::Snapshot::empty(PROJECT.into());
    task.id = app.task_id;
    task.lead_partner = app.attribution().partner;
    task.form = app.form();
    task.approved_memories = app.memory.selected_snapshot().unwrap();
    providers::Input {
        task,
        work: None,
        instruction_override: None,
        excerpts: app.sources.basket.clone(),
        report: None,
        resume: None,
        agent: providers::Agent::default(),
        operation: providers::Operation::Review,
        edit_paths: vec![],
        handoff: None,
        inspect_sessions: false,
        inspect_models: false,
        selection: Default::default(),
        conversation: None,
        credential: None,
    }
}

#[test]
fn saving_memory_preference_does_not_select_or_include_it() {
    let mut app = app();
    app.memory.records = vec![note(memory::Scope::Project)];
    assert!(app.held_command("held-set-memory"));
    assert!(app.memory.selected_ids.is_empty());
    assert!(input(&app).task.approved_memories.is_empty());
    assert!(app.records.queued.iter().chain(app.records.pending.iter()).any(|request|
        matches!(&request.operation, store::records::Operation::SetHeld {
            partner, form, target: Some(target), ..
        } if *partner == app.attribution().partner && *form == app.form() && target == &memory_target())));
}

#[test]
fn saving_reviewed_file_never_pins_capture() {
    let mut app = app();
    capture(&mut app);
    assert!(app.held_command("held-set-file"));
    assert!(app.sources.basket.is_empty());
    assert!(app.sources.pending_store.is_none());
    assert!(input(&app).excerpts.is_empty());
    assert!(
        app.records
            .queued
            .iter()
            .chain(app.records.pending.iter())
            .any(
                |request| matches!(&request.operation, store::records::Operation::SetHeld {
            target: Some(Target::File { path }), file_proof: Some(_), ..
        } if path == "sample.rs")
            )
    );
}

#[test]
fn unreviewed_file_cannot_become_preference() {
    let mut app = app();
    capture(&mut app);
    app.sources.preview_reviewed = false;
    app.held_command("held-set-file");
    assert!(app.records.queued.is_empty());
    assert!(app.records.pending.is_none());
    assert!(app.sources.basket.is_empty());
}

#[test]
fn memory_proposal_requires_explicit_accept_and_does_not_duplicate_selection() {
    let mut app = app();
    app.memory.records = vec![note(memory::Scope::Project)];
    saved(&mut app, memory_target());
    assert_eq!(app.held_status(), Status::Proposed);
    assert!(app.memory.selected_ids.is_empty());
    app.held_command("held-accept");
    assert_eq!(app.memory.selected_ids, vec![31]);
    assert_eq!(app.held_status(), Status::Included);
    app.held_command("held-accept");
    assert_eq!(app.memory.selected_ids, vec![31]);
    assert_eq!(
        input(&app).task.approved_memories,
        vec![note(memory::Scope::Project)]
    );
}

#[test]
fn dismissed_proposal_stays_dismissed_on_refresh_but_reappears_on_new_visit() {
    let mut app = app();
    app.memory.records = vec![note(memory::Scope::Project)];
    saved(&mut app, memory_target());
    app.held_command("held-dismiss");
    assert_eq!(app.held_status(), Status::Dismissed);
    saved(&mut app, memory_target());
    assert_eq!(app.held_status(), Status::Dismissed);
    assert!(app.memory.selected_ids.is_empty());
    app.task_id = 8;
    app.memory.reset(8, PROJECT);
    app.memory.records = vec![note(memory::Scope::Project)];
    app.reconcile_held();
    assert_eq!(app.held_status(), Status::Proposed);
}

#[test]
fn owner_and_form_slots_do_not_leak() {
    let mut app = app();
    app.memory.records = vec![note(memory::Scope::Project)];
    saved(&mut app, memory_target());
    let original_form = app.lead.form;
    app.lead.form = (original_form + 1) % 9;
    app.reconcile_held();
    assert!(app.held.target.is_none());
    app.lead.form = original_form;
    app.lead.id += 1;
    app.reconcile_held();
    assert!(app.held.target.is_none());
    assert!(app.memory.selected_ids.is_empty());
}

#[test]
fn changed_revision_or_task_scope_is_unavailable_not_auto_adopted() {
    let mut app = app();
    app.memory.records = vec![note(memory::Scope::Task)];
    saved(&mut app, memory_target());
    app.memory.records[0].revision += 1;
    app.reconcile_held();
    app.held_command("held-accept");
    assert_eq!(app.held_status(), Status::Unavailable);
    assert!(app.memory.selected_ids.is_empty());
    app.memory.records[0].revision = 4;
    app.task_id = 8;
    app.memory.task_id = 8;
    app.reconcile_held();
    app.held_command("held-accept");
    assert_eq!(app.held_status(), Status::Unavailable);
    assert!(app.memory.selected_ids.is_empty());
}

#[test]
fn clear_only_changes_preference_not_ordinary_accepted_memory() {
    let mut app = app();
    app.memory.records = vec![note(memory::Scope::Project)];
    saved(&mut app, memory_target());
    app.held_command("held-accept");
    // Ordinary selection dirties task state; clear is allowed only after its
    // independent save has settled.
    app.saved_revision = app.revision;
    app.storage = StorageState::Saved;
    app.save_requested = false;
    app.held_command("held-clear");
    assert_eq!(app.memory.selected_ids, vec![31]);
    assert!(
        app.records
            .queued
            .iter()
            .chain(app.records.pending.iter())
            .any(|request| matches!(
                &request.operation,
                store::records::Operation::SetHeld { target: None, .. }
            ))
    );
}

#[test]
fn frozen_receipt_uses_actual_packet_membership_and_retains_owner() {
    let mut app = app();
    app.memory.records = vec![note(memory::Scope::Project)];
    saved(&mut app, memory_target());
    app.begin_edit_field(store::EditorField::Request);
    app.input = "literal authored draft".into();
    app.cursor = 7;
    let before = input(&app);
    app.freeze_held_receipt(&before);
    assert!(!app.held.receipt.as_ref().unwrap().included);
    app.held_command("held-accept");
    let packet = input(&app);
    let owner = app.attribution();
    app.freeze_held_receipt(&packet);
    app.lead.form = (app.form() + 1) % 9;
    app.lead.id += 1;
    app.task_id = 8;
    app.reconcile_held();
    let receipt = app.held.receipt.as_ref().unwrap();
    assert_eq!(receipt.task, 7);
    assert_eq!(receipt.owner, owner);
    assert_eq!(receipt.target, Some(memory_target()));
    assert!(receipt.included);
    assert!(
        app.drafts
            .iter()
            .any(|draft| draft.field == store::EditorField::Request
                && draft.text == "literal authored draft"
                && draft.cursor == 7)
    );
    assert_eq!(
        packet.task.approved_memories,
        vec![note(memory::Scope::Project)]
    );
}

#[test]
fn failed_held_write_retains_exact_intent_and_ignores_unmatched_ack() {
    let mut app = app();
    app.memory.records = vec![note(memory::Scope::Project)];
    app.held_command("held-set-memory");
    let request = app
        .records
        .pending
        .take()
        .or_else(|| app.records.queued.pop_front())
        .unwrap();
    app.records.inflight = Some(request.clone());
    let mut unrelated = request.clone();
    unrelated.task_id += 1;
    app.records_event(unrelated, Err("late reply".into()));
    assert_eq!(app.records.inflight.as_ref(), Some(&request));
    app.records_event(request.clone(), Err("disk full".into()));
    assert_eq!(app.records.failed.as_ref(), Some(&request));
    assert!(app.records.mutation_pending());
    app.memory.records[0].revision += 1;
    app.held_command("held-set-memory");
    assert_eq!(app.records.failed.as_ref(), Some(&request));
    assert!(app.memory.selected_ids.is_empty());
    assert!(matches!(
        &request.operation,
        store::records::Operation::SetHeld {
            target: Some(Target::Memory {
                id: 31,
                revision: 4
            }),
            ..
        }
    ));
}

#[test]
fn accepted_file_waits_for_ordinary_pin_acknowledgment() {
    let mut app = app();
    let excerpt = capture(&mut app);
    saved(
        &mut app,
        Target::File {
            path: excerpt.path.clone(),
        },
    );
    app.held_command("held-accept");
    assert!(app.sources.basket.is_empty());
    // A held identity is not itself a capture. If acceptance schedules a fresh
    // inspection, deliver that worker result before reviewing the ordinary Pin.
    if app.sources.pending.is_some() {
        app.sources.pending = None;
        app.sources.event(
            7,
            context::Event {
                task_id: 7,
                result: Ok(context::Response::Inspected(excerpt.clone())),
            },
        );
    }
    app.sources.preview_reviewed = true;
    app.sources.command("attach", 7, None);
    assert!(matches!(&app.sources.pending_store,
        Some(store::ContextRequest { operation: store::ContextOperation::Pin(saved), .. })
            if saved == &excerpt));
    assert!(input(&app).excerpts.is_empty());
    let mut committed = excerpt;
    committed.id = 91;
    app.sources.stored(7, 7, Ok(vec![committed.clone()]));
    assert_eq!(input(&app).excerpts, vec![committed]);
    assert_eq!(app.held_status(), Status::Included);
}

#[test]
fn matching_commit_proposes_only_after_ack_and_never_selects_memory() {
    let mut app = app();
    app.memory.records = vec![note(memory::Scope::Project)];
    app.held_command("held-set-memory");
    let request = app
        .records
        .pending
        .take()
        .or_else(|| app.records.queued.pop_front())
        .unwrap();
    assert!(app.records.snapshot.held_items.is_empty());
    app.records.inflight = Some(request.clone());
    let mut snapshot = app.records.snapshot.clone();
    snapshot.held_items = vec![Item {
        partner: app.attribution().partner,
        form: app.form(),
        target: memory_target(),
    }];
    app.records_event(
        request,
        Ok(Box::new(store::records::Response::Snapshot(Box::new(
            snapshot,
        )))),
    );
    app.reconcile_held();
    assert_eq!(app.held.target, Some(memory_target()));
    assert!(app.memory.selected_ids.is_empty());
    assert!(input(&app).task.approved_memories.is_empty());
    assert!(!app.records.mutation_pending());
}

#[test]
fn deleted_or_foreign_project_memory_cannot_be_accepted() {
    let mut app = app();
    app.memory.records = vec![note(memory::Scope::Project)];
    saved(&mut app, memory_target());
    app.memory.records[0].project = "/other-project".into();
    app.reconcile_held();
    app.held_command("held-accept");
    assert_eq!(app.held_status(), Status::Unavailable);
    assert!(app.memory.selected_ids.is_empty());
    app.memory.records.clear();
    app.reconcile_held();
    app.held_command("held-accept");
    assert_eq!(app.held_status(), Status::Unavailable);
    assert!(app.memory.selected_ids.is_empty());
}

#[test]
fn receipt_cannot_claim_selected_memory_absent_from_frozen_packet() {
    let mut app = app();
    app.memory.records = vec![note(memory::Scope::Project)];
    saved(&mut app, memory_target());
    let packet_without_memory = input(&app);
    app.held_command("held-accept");
    assert_eq!(app.memory.selected_ids, vec![31]);
    app.freeze_held_receipt(&packet_without_memory);
    assert!(!app.held.receipt.as_ref().unwrap().included);
}

#[test]
fn custom_exclusion_blocks_reviewed_file_save_and_later_acceptance() {
    let mut app = app();
    let excerpt = capture(&mut app);
    app.held.exclusions = vec!["sample.rs".into()];
    app.held_command("held-set-file");
    assert!(app.records.pending.is_none());
    assert!(app.records.queued.is_empty());
    saved(&mut app, Target::File { path: excerpt.path });
    app.held_command("held-accept");
    assert_eq!(app.held_status(), Status::Unavailable);
    assert!(app.sources.pending.is_none());
    assert!(app.sources.pending_store.is_none());
    assert!(app.sources.basket.is_empty());
}

#[test]
fn repeated_file_acceptance_reinspects_but_does_not_duplicate_pin() {
    let mut app = app();
    let inspected = capture(&mut app);
    let mut committed = inspected.clone();
    committed.id = 91;
    app.sources.basket = vec![committed.clone()];
    saved(
        &mut app,
        Target::File {
            path: inspected.path.clone(),
        },
    );
    assert_eq!(app.held_status(), Status::Included);
    app.held_command("held-accept");
    assert!(matches!(&app.sources.pending,
        Some(context::Request { operation: context::Operation::InspectFile { path, .. }, .. })
            if path == std::path::Path::new("sample.rs")));
    assert!(app.sources.preview.is_none());
    app.sources.pending = None;
    app.sources.event(
        7,
        context::Event {
            task_id: 7,
            result: Ok(context::Response::Inspected(inspected)),
        },
    );
    app.sources.preview_reviewed = true;
    app.sources.command("attach", 7, None);
    assert!(app.sources.pending_store.is_none());
    assert_eq!(app.sources.basket, vec![committed.clone()]);
    assert_eq!(input(&app).excerpts, vec![committed]);
    assert_eq!(app.held_status(), Status::Included);
}

#[test]
fn automatic_submit_proposes_new_form_slot_without_retroactive_inclusion() {
    let mut app = app();
    app.overlay = None;
    app.planning.reset(7);
    app.planning.preference_event(Ok(None));
    app.lead.form = 0;
    app.lead.auto_form = true;
    app.memory.records = vec![note(memory::Scope::Project)];
    app.records.snapshot.held_items = vec![Item {
        partner: app.attribution().partner,
        form: 3,
        target: memory_target(),
    }];
    app.reconcile_held();
    assert!(app.held.target.is_none());
    let text = "Create a new Python code file.";
    app.input = text.into();
    app.cursor = 8;
    app.submit_request();
    assert_eq!(app.form(), 3);
    assert_eq!(app.held.target, Some(memory_target()));
    assert_eq!(app.held_status(), Status::Proposed);
    assert!(app.memory.selected_ids.is_empty());
    assert!(app.sources.basket.is_empty());
    assert_eq!((app.input.as_str(), app.cursor), (text, 8));
    assert!(app.pending_request.is_some());
    assert!(app.save_requested);
    assert!(app.planning.pending.is_none());
    assert!(!app.execution.busy());
    let snapshot = app.snapshot(PROJECT);
    assert!(snapshot.approved_memories.is_empty());
    assert!(app.planning.command("new", true, &snapshot, &[], None));
    let Some(providers::Request::Preview(packet)) = app.planning.pending.take() else {
        panic!("missing frozen request");
    };
    assert_eq!(packet.task.form, 3);
    assert_eq!(packet.task.request_text(), text);
    assert!(packet.task.approved_memories.is_empty());
    assert!(packet.excerpts.is_empty());
    assert!(app.memory.selected_ids.is_empty());
}

#[test]
fn dismissing_while_saving_only_hides_visit_and_preserves_literal_draft() {
    let mut app = app();
    app.memory.records = vec![note(memory::Scope::Project)];
    saved(&mut app, memory_target());
    app.input = "Unsent 한글 draft".into();
    app.cursor = 7;
    app.storage = StorageState::Saving;
    assert!(app.held_command("held-dismiss"));
    assert!(app.held.dismissed);
    assert_eq!(app.input, "Unsent 한글 draft");
    assert_eq!(app.cursor, 7);
    assert!(app.memory.selected_ids.is_empty());
    assert!(!app.records.mutation_pending());
    app.reconcile_held();
    assert!(app.held.dismissed);
}

#[test]
fn dismissing_during_held_save_survives_its_commit_acknowledgment() {
    let mut app = app();
    app.memory.records = vec![note(memory::Scope::Project)];
    saved(&mut app, memory_target());
    app.held_command("held-set-memory");
    let request = app
        .records
        .pending
        .take()
        .or_else(|| app.records.queued.pop_front())
        .unwrap();
    app.records.inflight = Some(request.clone());
    app.held_command("held-dismiss");
    assert!(app.held.dismissed);
    let snapshot = app.records.snapshot.clone();
    app.records_event(
        request,
        Ok(Box::new(store::records::Response::Snapshot(Box::new(
            snapshot,
        )))),
    );
    assert!(app.held.dismissed);
    assert_eq!(app.held.target, Some(memory_target()));
    assert!(app.memory.selected_ids.is_empty());
}
