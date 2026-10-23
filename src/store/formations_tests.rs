use super::{Request, Response};
use crate::store::*;
use crate::workspace::workflows::{FormationSlot, Recipe, RecipeStep};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ev-formation-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
    fn open(&self) -> State {
        let mut state = State {
            data_dir: self.0.join("data"),
            project: self.0.to_str().unwrap().into(),
            policy: Policy::default(),
            connection: None,
            lock: None,
        };
        state.open_and_load().unwrap();
        state
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn recipe() -> Recipe {
    Recipe {
        schema_version: 2,
        name: "Fixture arrangement".into(),
        scope: vec![],
        context: vec![],
        validation_commands: vec![],
        completion_criteria: vec!["Review independently".into()],
        steps: vec![RecipeStep {
            form: "Vaporeon".into(),
            action: "Stage only".into(),
        }],
        roles: [
            ("scout", "Vaporeon", "Inspect source"),
            ("runner", "Jolteon", "Review a check"),
            ("reviewer", "Glaceon", "Assess evidence"),
        ]
        .into_iter()
        .map(|(role, form, request)| FormationSlot {
            role: role.into(),
            form: form.into(),
            request: request.into(),
        })
        .collect(),
    }
}
fn setup(state: &mut State) -> Snapshot {
    state.add_partner().unwrap();
    state.add_partner().unwrap();
    let mut task = Snapshot::empty(state.project.clone());
    task.goal = "Formation retains owned state".into();
    task.partner_drafts.insert(
        1,
        PartnerDrafts {
            drafts: vec![Draft {
                field: EditorField::Request,
                text: "Keep 界e\u{301} suffix".into(),
                cursor: 5,
            }],
            editing: true,
            editor_field: EditorField::Request,
            previous_form: None,
        },
    );
    state.save(task).unwrap()
}
fn request(state: &State, task: &Snapshot, mapping: Vec<Option<i64>>) -> Request {
    Request::Apply {
        expected: Box::new(task.clone()),
        roster: partners(state.connection.as_ref().unwrap()).unwrap(),
        recipe: recipe(),
        mapping,
    }
}
#[test]
fn formation_stages_three_owned_partners_atomically_and_survives_restart() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = setup(&mut state);
    let original = task.partner_drafts[&1].clone();
    let response = state
        .formation(request(&state, &task, vec![Some(1), Some(2), Some(3)]))
        .unwrap();
    let Response::Applied {
        task: applied,
        roster,
    } = response
    else {
        panic!("expected committed formation")
    };
    assert_eq!(
        roster
            .iter()
            .map(|p| (p.form, p.auto_form))
            .collect::<Vec<_>>(),
        vec![(1, false), (2, false), (7, false)]
    );
    assert_eq!(applied.partner_drafts[&1].drafts, original.drafts);
    assert_eq!(applied.partner_drafts[&2].drafts[0].text, "Review a check");
    assert_eq!(applied.partner_drafts[&3].drafts[0].text, "Assess evidence");
    let db = state.connection.as_ref().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM active_work", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM executions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM form_routes", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        3
    );
    drop(state);
    let mut state = fixture.open();
    let restored = state.open_and_load().unwrap();
    assert_eq!(restored.tasks[0], *applied);
    assert_eq!(restored.partners, roster);
}
#[test]
fn late_duplicate_mapping_rolls_back_earlier_partner_and_draft_changes() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = setup(&mut state);
    let before = load(state.connection.as_ref().unwrap(), &state.project).unwrap();
    assert!(
        state
            .formation(request(&state, &task, vec![Some(1), Some(1), Some(3)]))
            .is_err()
    );
    assert_eq!(
        load(state.connection.as_ref().unwrap(), &state.project).unwrap(),
        before
    );
}
#[test]
fn stale_task_roster_busy_owner_and_invalid_definition_refuse_without_partial_application() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = setup(&mut state);
    let stale = request(&state, &task, vec![Some(1), Some(2), None]);
    let mut updated = task.clone();
    updated.revision += 1;
    updated.next_action = "Newer exact fact".into();
    let updated = state.save(updated).unwrap();
    assert!(state.formation(stale).is_err());
    let stale = request(&state, &updated, vec![Some(1), Some(2), None]);
    state.nickname_partner(2, Some("Mizu".into())).unwrap();
    assert!(state.formation(stale).is_err());
    let claim = WorkClaim {
        task_id: updated.id,
        request_id: "formation-busy".into(),
        owner: Attribution::of(&partners(state.connection.as_ref().unwrap()).unwrap()[1]),
        writing: false,
    };
    state
        .work_request(&WorkRequest::Acquire {
            claim,
            task: Box::new(updated.clone()),
        })
        .unwrap();
    let before = load(state.connection.as_ref().unwrap(), &state.project).unwrap();
    assert!(
        state
            .formation(request(&state, &updated, vec![Some(1), Some(2), None]))
            .is_err()
    );
    assert_eq!(
        load(state.connection.as_ref().unwrap(), &state.project).unwrap(),
        before
    );
    let mut invalid = recipe();
    invalid.roles[0].request = "password=not-a-real-credential".into();
    assert!(invalid.validate().is_err());
    invalid = recipe();
    invalid.schema_version = 1;
    assert!(invalid.validate().is_err());
    invalid = recipe();
    invalid.roles[1].role = invalid.roles[0].role.clone();
    assert!(invalid.validate().is_err());
}
#[test]
fn unmapped_roles_remain_empty_and_portable_export_never_overwrites() {
    let fixture = Fixture::new();
    let mut state = fixture.open();
    let task = setup(&mut state);
    let path = fixture.0.join("formation.toml");
    state
        .formation(Request::Export {
            path: path.clone(),
            recipe: recipe(),
        })
        .unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(
        state
            .formation(Request::Export {
                path: path.clone(),
                recipe: recipe()
            })
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let Response::Read(definition) = state.formation(Request::Read { path }).unwrap() else {
        panic!("expected definition")
    };
    assert_eq!(definition, recipe());
    let Response::Applied { task, roster } = state
        .formation(request(&state, &task, vec![None, Some(2), None]))
        .unwrap()
    else {
        panic!("expected staged partial mapping")
    };
    assert_eq!(
        roster.iter().map(|p| p.form).collect::<Vec<_>>(),
        vec![0, 2, 0]
    );
    assert!(!task.partner_drafts.contains_key(&3));
}
