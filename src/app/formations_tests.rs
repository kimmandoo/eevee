use super::*;
use crate::store::{Draft, EditorField, Partner, PartnerDrafts};

fn app() -> App {
    let mut app = App::new(
        std::env::temp_dir()
            .join("ev-formation-app")
            .to_string_lossy()
            .into_owned(),
        60,
        18,
    );
    app.task_id = 7;
    app.party = vec![
        Partner::default(),
        Partner {
            id: 2,
            ..Partner::default()
        },
    ];
    app.storage = StorageState::Saved;
    app.planning.preference_event(Ok(None));
    app.sources = super::super::sources::State::default();
    app.overlay = Some(Overlay::Formations);
    app.formations.mode = Mode::Mapping;
    app.formations.mapping = vec![Some(1), Some(2)];
    app.formations.recipe = Some(Recipe {
        schema_version: 2,
        name: "Two roles".into(),
        scope: vec![],
        context: vec![],
        steps: vec![RecipeStep {
            form: "Jolteon".into(),
            action: "Stage only".into(),
        }],
        validation_commands: vec![],
        completion_criteria: vec!["Review".into()],
        roles: vec![
            FormationSlot {
                role: "first".into(),
                form: "Jolteon".into(),
                request: "Inspect".into(),
            },
            FormationSlot {
                role: "second".into(),
                form: "Glaceon".into(),
                request: String::new(),
            },
        ],
    });
    app.formations.roster = app.partners();
    app.formations.owner = (app.task_id, app.lead.id);
    app
}
#[test]
fn formation_neutral_input_never_grants_and_mapping_changes_expire_the_rendered_review() {
    let mut app = app();
    let original = app.snapshot(&app.cwd);
    for key in [
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
    ] {
        app.formation_key(key);
        assert!(app.party_request.is_none());
    }
    app.formation_paste("y");
    let mut repeat = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
    repeat.kind = KeyEventKind::Repeat;
    app.formations.reviewed.set(true);
    app.formation_key(repeat);
    assert!(app.party_request.is_none());
    app.formation_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert!(!app.formations.reviewed.get());
    app.formation_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.party_request.is_none());
    assert_eq!(app.snapshot(&app.cwd), original);
}
#[test]
fn formation_acknowledgment_updates_only_owned_drafts_and_retains_other_task_facts() {
    let mut app = app();
    app.goal = "Original goal".into();
    let original_draft = PartnerDrafts {
        drafts: vec![Draft {
            field: EditorField::Request,
            text: "Draft 界 suffix".into(),
            cursor: 6,
        }],
        editing: true,
        editor_field: EditorField::Request,
        previous_form: None,
    };
    let mut task = app.snapshot(&app.cwd);
    task.partner_drafts.insert(1, original_draft.clone());
    task.form = 2;
    task.auto_form = false;
    task.revision = 1;
    task.partner_drafts.insert(
        2,
        PartnerDrafts {
            drafts: vec![Draft {
                field: EditorField::Request,
                text: "Independent review".into(),
                cursor: 18,
            }],
            ..PartnerDrafts::default()
        },
    );
    let roster = vec![
        Partner {
            form: 2,
            auto_form: false,
            ..Partner::default()
        },
        Partner {
            id: 2,
            form: 7,
            auto_form: false,
            ..Partner::default()
        },
    ];
    app.formation_event(Ok(store::formations::Response::Applied {
        task: Box::new(task),
        roster,
    }));
    assert_eq!(app.goal, "Original goal");
    assert_eq!(app.drafts, original_draft.drafts);
    assert_eq!(app.partner_drafts[&2].drafts[0].text, "Independent review");
    assert_eq!(app.form(), 2);
    assert!(!app.lead.auto_form);
    assert_eq!(app.form_selection, 2);
    assert!(app.planning.pending.is_none());
    assert!(app.evolution.transition.is_none());
    assert!(app.pending_command.is_none());
    assert!(app.pending_request.is_none());
}

#[test]
fn reviewed_formation_waits_for_its_exact_save_and_cancellation_or_new_revision_revokes_it() {
    for outcome in 0..3 {
        let mut app = app();
        app.revision = 1;
        app.formations.revision = 1;
        app.saved_revision = 0;
        app.save_requested = true;
        app.storage = StorageState::Pending;
        app.formations.reviewed.set(true);
        app.formation_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.formations.apply_pending);
        assert!(app.party_request.is_none());
        let saved = app.snapshot(&app.cwd);
        if outcome == 1 {
            app.formation_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        } else if outcome == 2 {
            app.revision += 1;
        }
        app.store_event(store::Event::Saved(saved));
        app.settle_formations();
        assert_eq!(app.party_request.is_some(), outcome == 0);
        assert!(!app.formations.apply_pending);
        assert!(app.pending_request.is_none());
        assert!(app.pending_command.is_none());
    }
}

#[test]
fn unchanged_formation_role_keeps_an_explicit_read_only_operation() {
    let mut app = app();
    app.lead.form = 3;
    app.lead.auto_form = false;
    app.party[0] = app.lead.clone();
    app.formations.roster = app.partners();
    app.planning
        .set_operation(crate::providers::Operation::Plan);
    let mut task = app.snapshot(&app.cwd);
    task.revision += 1;
    let roster = app.partners();
    app.formation_event(Ok(store::formations::Response::Applied {
        task: Box::new(task),
        roster,
    }));
    assert_eq!(
        app.planning.explicit_operation(),
        Some(crate::providers::Operation::Plan)
    );
    assert!(app.planning.pending.is_none());
    assert_eq!(app.form(), 3);
}

#[test]
fn queued_formation_cannot_apply_to_a_new_task_or_after_search_or_quit() {
    for action in 0..3 {
        let mut app = app();
        app.revision = 1;
        app.formations.revision = 1;
        app.saved_revision = 0;
        app.save_requested = true;
        app.storage = StorageState::Pending;
        app.formations.reviewed.set(true);
        app.formation_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        let saved = app.snapshot(&app.cwd);
        match action {
            0 => app.task_id += 1,
            1 => {
                app.update(Event::Key(KeyEvent::new(
                    KeyCode::Char('k'),
                    KeyModifiers::CONTROL,
                )));
            }
            _ => app.quit = true,
        }
        app.store_event(store::Event::Saved(saved));
        app.settle_formations();
        assert!(!app.formations.apply_pending);
        assert!(app.party_request.is_none());
        assert!(app.planning.pending.is_none());
    }
}

fn three_party_readiness() -> App {
    use crate::workspace::records::PartnerSummary;
    let mut app = app();
    app.party.push(Partner {
        id: 3,
        ..Partner::default()
    });
    app.formations.mapping.push(Some(3));
    app.formations
        .recipe
        .as_mut()
        .unwrap()
        .roles
        .push(FormationSlot {
            role: "third".into(),
            form: "Vaporeon".into(),
            request: "Inspect independently".into(),
        });
    app.formations.roster = app.partners();
    app.drive_records();
    app.records.pending = None;
    app.records.queued.clear();
    app.records.loaded = true;
    app.records.snapshot.summaries = app
        .party
        .iter()
        .map(|partner| PartnerSummary {
            partner: partner.clone(),
            produced_tasks: [0; 9],
            moves: [0; 9],
            move_names: Default::default(),
            shared_bond_memories: 0,
            unknown_producer_artifacts: 0,
        })
        .collect();
    app.records.snapshot.summaries[1].moves[7] = 2;
    app.records.snapshot.summaries[1].move_names[7] = vec!["Inspect".into(), "Check".into()];
    app.memory.reset(app.task_id, &app.cwd);
    app.memory_loading = false;
    app.held_memory_result(&Ok(Vec::new()));
    app.records
        .snapshot
        .held_items
        .push(crate::workspace::held::Item {
            partner: 2,
            form: 7,
            target: crate::workspace::held::Target::Memory {
                id: 81,
                revision: 4,
            },
        });
    app.partner_drafts.insert(
        3,
        PartnerDrafts {
            drafts: vec![Draft {
                field: EditorField::Request,
                text: "Preserve 界e\u{301} bytes".into(),
                cursor: 9,
            }],
            editing: true,
            editor_field: EditorField::Request,
            previous_form: None,
        },
    );
    app.agents
        .event(crate::providers::agent::Event::Discovered(Vec::new()));
    app
}

#[test]
fn three_mapped_partners_separate_staging_from_missing_unknown_and_unavailable_submission_facts() {
    let mut app = three_party_readiness();
    let first = app.formation_readiness(0);
    assert_eq!(first.staging, Staging::Eligible);
    assert_eq!(first.route, Route::Missing);
    assert_eq!(first.moves, Some(0));
    assert_eq!(first.held, super::super::held::Readiness::NotConfigured);
    let second = app.formation_readiness(1);
    assert_eq!(second.moves, Some(2));
    assert_eq!(second.held, super::super::held::Readiness::Unavailable);
    assert_eq!(
        second.staging,
        Staging::Eligible,
        "an optional unavailable reference does not forbid staging"
    );
    assert!(app.formation_readiness(2).draft_nonempty);

    app.records
        .snapshot
        .pairings
        .push(crate::workspace::records::Pairing {
            partner: 2,
            form: 7,
            agent: crate::providers::Agent::Codex,
            selection: Default::default(),
        });
    assert_eq!(app.formation_readiness(1).route, Route::Unknown);
    app.records.loaded = false;
    let unknown = app.formation_readiness(1);
    assert_eq!(unknown.moves, None);
    assert_eq!(unknown.held, super::super::held::Readiness::Unknown);
    assert_eq!(unknown.route, Route::Unknown);
    assert_eq!(unknown.staging, Staging::Eligible);
}

#[test]
fn warning_inspection_is_scoped_readonly_and_returns_to_exact_mapping_without_applying() {
    for kind in ['m', 'r', 'h'] {
        let mut app = three_party_readiness();
        app.formations.selected = 1;
        let before = app.snapshot(&app.cwd).partner_drafts;
        let mapping = app.formations.mapping.clone();
        let selected = app.formations.selected;
        let lead = app.lead.clone();
        let source_selection = app.sources.basket.clone();
        let memories = app.memory.selected_ids.clone();
        app.formation_key(KeyEvent::new(KeyCode::Char(kind), KeyModifiers::NONE));
        match kind {
            'm' => {
                assert_eq!(
                    app.formations.inspector,
                    Some(Inspector::Moves {
                        partner: 2,
                        form: 7
                    })
                );
                app.records.scroll_max = 3;
                for _ in 0..4 {
                    app.update(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
                }
                assert_eq!(app.records.scroll, 3);
                app.update(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
                assert_eq!(app.records.scroll, 2);
                assert_eq!(
                    app.records.snapshot.summaries[app.records.selected]
                        .partner
                        .id,
                    2
                );
            }
            'r' => assert_eq!(
                app.formations.inspector,
                Some(Inspector::Route {
                    partner: 2,
                    form: 7
                })
            ),
            'h' => assert!(app.held_inspection_active()),
            _ => unreachable!(),
        }
        // These are actions on the ordinary desks, but not inside a mapped
        // observational visit. Include global navigation and direct approval.
        for key in [
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
        ] {
            app.update(Event::Key(key));
        }
        app.update(Event::Paste("never select or send this".into()));
        assert_eq!(app.snapshot(&app.cwd).partner_drafts, before);
        assert_eq!(app.lead, lead);
        assert_eq!(app.sources.basket, source_selection);
        assert_eq!(app.memory.selected_ids, memories);
        assert!(app.party_request.is_none());
        assert!(app.pending_request.is_none());
        assert!(app.pending_command.is_none());
        app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert_eq!(app.overlay, Some(Overlay::Formations));
        assert_eq!(app.formations.mapping, mapping);
        assert_eq!(app.formations.selected, selected);
        assert_eq!(app.snapshot(&app.cwd).partner_drafts, before);
        assert!(!app.formations.apply_pending);
        assert!(
            !app.formations.reviewed.get(),
            "return requires a newly visible review"
        );
    }
}

#[test]
fn warning_escape_restores_mapping_even_when_the_inspector_was_already_in_overlay_history() {
    for (origin, kind) in [(Overlay::Agents, 'r'), (Overlay::Records, 'm')] {
        let mut app = three_party_readiness();
        app.overlay = Some(origin);
        app.open_overlay(Overlay::Formations);
        app.formations.mapping[0] = None;
        app.formations.selected = 1;
        let mapping = app.formations.mapping.clone();
        let before = app.snapshot(&app.cwd).partner_drafts;
        app.formation_key(KeyEvent::new(KeyCode::Char(kind), KeyModifiers::NONE));
        assert_eq!(app.overlay, Some(origin));
        app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert_eq!(app.overlay, Some(Overlay::Formations));
        assert_eq!(app.formations.mapping, mapping);
        assert_eq!(app.formations.selected, 1);
        assert_eq!(app.snapshot(&app.cwd).partner_drafts, before);
        assert!(!app.formations.apply_pending);
        assert!(
            app.party_request.is_none()
                && app.pending_request.is_none()
                && app.pending_command.is_none()
        );
    }
}

#[test]
fn failed_version_qualification_is_unknown_for_the_mapped_route_not_unavailable() {
    use crate::providers::{Agent, diagnostics::Failure};
    let mut app = three_party_readiness();
    app.records
        .snapshot
        .pairings
        .push(crate::workspace::records::Pairing {
            partner: 2,
            form: 7,
            agent: Agent::Codex,
            selection: Default::default(),
        });
    app.agents
        .connection_failed(Agent::Codex, Failure::VersionCheck);
    assert_eq!(app.formation_readiness(1).route, Route::Unknown);
    assert_eq!(app.formation_readiness(1).staging, Staging::Eligible);
    assert_eq!(
        app.agents.failure(Agent::Codex.index()),
        Some(Failure::VersionCheck)
    );
    app.agents
        .connection_failed(Agent::Codex, Failure::Incompatible);
    assert_eq!(app.formation_readiness(1).route, Route::Unavailable);
}

#[test]
fn late_writer_ownership_blocks_staging_without_confusing_cached_route_state() {
    let mut app = three_party_readiness();
    let before = app.snapshot(&app.cwd);
    app.formations.reviewed.set(true);
    let owner = store::Attribution::of(&app.party[2]);
    let mut writer = app.planning.new_partner(owner.clone(), app.task_id);
    writer.work = Some(store::WorkClaim {
        task_id: app.task_id,
        owner,
        request_id: "late-writer".into(),
        writing: true,
    });
    writer.work_acquired = true;
    app.cooperation.parked.insert(3, writer);
    let readiness = app.formation_readiness(2);
    assert_eq!(readiness.staging, Staging::Busy);
    assert_eq!(readiness.writer, Some(3));
    assert!(readiness.owner_busy);
    assert_eq!(readiness.route, Route::Missing);
    app.formation_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.party_request.is_none());
    assert!(!app.formations.apply_pending);
    assert_eq!(app.snapshot(&app.cwd), before);
}

#[test]
fn readiness_tracks_mapping_not_the_lead_and_duplicate_or_missing_targets_do_not_gain_authority() {
    let mut app = three_party_readiness();
    let before = app.snapshot(&app.cwd);
    app.formations.mapping.swap(0, 1);
    assert_eq!(
        app.formation_readiness(0).moves,
        Some(0),
        "mapped form, not the partner's other form"
    );
    app.formations.mapping[2] = Some(2);
    assert_eq!(app.formation_readiness(0).staging, Staging::Duplicate);
    app.formations.mapping[2] = Some(99);
    assert_eq!(app.formation_readiness(2).staging, Staging::Changed);
    app.formations.mapping[2] = None;
    assert_eq!(app.formation_readiness(2).staging, Staging::Unmapped);
    assert_eq!(app.snapshot(&app.cwd), before);
}
