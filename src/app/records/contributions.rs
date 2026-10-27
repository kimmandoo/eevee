//! Reference-only contribution navigation: route visits never assign outputs.
use super::{App, ArtifactKind, Mode};
use crate::{
    app::Overlay,
    workspace::records::{ArtifactView, Snapshot},
};
use crossterm::event::{KeyCode, KeyEvent};

#[derive(Default)]
pub struct State {
    pub partner: Option<i64>,
    pub task: Option<i64>,
    pub form: Option<usize>,
    returning: Option<(Mode, usize, u16)>,
}
impl State {
    pub fn matches(&self, view: &ArtifactView) -> bool {
        let artifact = &view.artifact;
        artifact.producer.as_ref().map(|owner| owner.partner) == self.partner
            && self.task.is_none_or(|task| task == artifact.task_id)
            && self.form.is_none_or(|form| {
                artifact
                    .producer
                    .as_ref()
                    .is_some_and(|owner| owner.form == form)
            })
    }
    pub fn entries<'a>(&'a self, snapshot: &'a Snapshot) -> impl Iterator<Item = &'a ArtifactView> {
        snapshot.artifacts.iter().filter(|view| self.matches(view))
    }
}
impl App {
    pub(super) fn contribution_key(&mut self, key: KeyEvent) -> bool {
        if self.formation_inspector_active() {
            return false;
        }
        if matches!(self.records.mode, Mode::Summary | Mode::Routes)
            && key.code == KeyCode::Char('c')
        {
            let Some(summary) = self.records.snapshot.summaries.get(self.records.selected) else {
                return true;
            };
            self.records.contributions = State {
                partner: Some(summary.partner.id),
                task: (self.records.mode == Mode::Routes).then_some(self.task_id),
                form: None,
                returning: Some((
                    self.records.mode,
                    self.records.selected,
                    self.records.scroll,
                )),
            };
            self.records.enter(Mode::Contributions);
            return true;
        }
        if self.records.mode != Mode::Contributions {
            return false;
        }
        match key.code {
            KeyCode::Esc => {
                if let Some((mode, selected, scroll)) = self.records.contributions.returning.take()
                {
                    self.records.enter(mode);
                    self.records.selected = selected;
                    self.records.scroll = scroll;
                } else {
                    self.close_overlay();
                }
            }
            KeyCode::Char('f') => {
                self.records.contributions.form = match self.records.contributions.form {
                    None => Some(0),
                    Some(8) => None,
                    Some(form) => Some(form + 1),
                };
                self.records.selected = 0;
                self.records.scroll = 0;
            }
            KeyCode::Char('t') => {
                let mut tasks: Vec<_> = self.tasks.iter().map(|task| Some(task.id)).collect();
                tasks.extend(
                    self.records
                        .snapshot
                        .artifacts
                        .iter()
                        .map(|view| Some(view.artifact.task_id)),
                );
                tasks.extend([None, Some(self.task_id)]);
                tasks.sort_unstable();
                tasks.dedup();
                let next = tasks
                    .iter()
                    .position(|task| *task == self.records.contributions.task)
                    .map_or(0, |index| (index + 1) % tasks.len());
                self.records.contributions.task = tasks[next];
                self.records.selected = 0;
                self.records.scroll = 0;
            }
            KeyCode::Char('p') => {
                // Include released producers and legacy unknown without giving
                // either the current roster's name or form.
                let mut producers: Vec<_> = self
                    .records
                    .snapshot
                    .artifacts
                    .iter()
                    .map(|view| view.artifact.producer.as_ref().map(|owner| owner.partner))
                    .collect();
                producers.extend(
                    self.records
                        .snapshot
                        .summaries
                        .iter()
                        .map(|summary| Some(summary.partner.id)),
                );
                producers.push(None);
                producers.sort_unstable();
                producers.dedup();
                let next = producers
                    .iter()
                    .position(|partner| *partner == self.records.contributions.partner)
                    .map_or(0, |index| (index + 1) % producers.len());
                self.records.contributions.partner = producers[next];
                self.records.selected = 0;
                self.records.scroll = 0;
            }
            KeyCode::Char('g') | KeyCode::Enter => {
                let Some(view) = self
                    .records
                    .contributions
                    .entries(&self.records.snapshot)
                    .nth(self.records.selected)
                else {
                    return true;
                };
                let artifact = &view.artifact;
                if artifact.task_id != self.task_id || key.code == KeyCode::Char('g') {
                    // Highlight only. The ordinary task picker owns explicit
                    // Enter, autosave, dirty-draft and outstanding-work guards.
                    if let Some(task) = self.tasks.iter().find(|task| task.id == artifact.task_id) {
                        self.show_archived = task.archived;
                        let selected = self
                            .visible_tasks()
                            .position(|task| task.id == artifact.task_id)
                            .unwrap_or(0);
                        self.task_selection = selected;
                        self.command("tasks");
                    } else {
                        self.records.notice =
                            "Retained task unavailable; no owner or draft changed.".into();
                    }
                    return true;
                }
                match artifact.kind {
                    ArtifactKind::Capture | ArtifactKind::Run => {
                        if let Some(reference) = artifact.evidence.first().cloned() {
                            self.workflow_open_reference(reference);
                            if self.overlay == Some(Overlay::Records) {
                                self.records.notice = self.workflow.notice.clone();
                            }
                        }
                    }
                    ArtifactKind::Review
                        if artifact.reference.parse::<u64>().ok()
                            == Some(self.workflows.review_generation)
                            && self.workflows.review.is_some() =>
                    {
                        self.command("workflow-review");
                    }
                    _ => {
                        // Historical metadata stays here; exact retained support
                        // is navigable without substituting a newer workflow.
                        if let Some(reference) = artifact.evidence.first().cloned() {
                            self.workflow_open_reference(reference);
                            if self.overlay == Some(Overlay::Records) {
                                self.records.notice = self.workflow.notice.clone();
                            }
                        } else {
                            self.records.notice = "Exact historical payload unavailable; retained attribution is shown, not a replacement.".into();
                        }
                    }
                }
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        store,
        workspace::records::{Artifact, Freshness},
    };
    use crossterm::event::{Event, KeyEventKind, KeyModifiers};

    fn artifact(task: i64, producer: Option<i64>, form: usize, kind: ArtifactKind) -> ArtifactView {
        ArtifactView {
            artifact: Artifact {
                id: format!("{task}-{producer:?}-{form}-{kind:?}"),
                task_id: task,
                kind,
                role_form: 7,
                ordinal: 10,
                supported_diagnosis: false,
                handoff_revision: None,
                observation: None,
                producer: producer.map(|partner| store::Attribution {
                    partner,
                    name: "Original 한글".into(),
                    form,
                }),
                reference: "retained-exact".into(),
                created_at: 1,
                evidence: vec![],
                detail: "Immutable fact".into(),
            },
            freshness: Freshness::Unavailable,
            explanation: "Purged support".into(),
        }
    }
    fn key(app: &mut App, code: KeyCode) {
        app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }
    #[test]
    fn contribution_filters_use_exact_producer_form_not_role_route_or_front_owner() {
        let mut app = App::new("/project".into(), 60, 18);
        app.records.snapshot.artifacts = vec![
            artifact(7, Some(1), 1, ArtifactKind::Capture),
            artifact(7, Some(2), 2, ArtifactKind::Run),
            artifact(8, Some(1), 7, ArtifactKind::Review),
            artifact(7, None, 7, ArtifactKind::Review),
        ];
        let mut state = State {
            partner: Some(1),
            ..State::default()
        };
        assert_eq!(state.entries(&app.records.snapshot).count(), 2);
        state.form = Some(7);
        assert_eq!(
            state
                .entries(&app.records.snapshot)
                .next()
                .unwrap()
                .artifact
                .task_id,
            8
        );
        state.task = Some(7);
        assert_eq!(state.entries(&app.records.snapshot).count(), 0);
        state.partner = None;
        state.form = None;
        assert!(
            state
                .entries(&app.records.snapshot)
                .next()
                .unwrap()
                .artifact
                .producer
                .is_none()
        );
        state.form = Some(7);
        assert_eq!(state.entries(&app.records.snapshot).count(), 0);
    }
    #[test]
    fn contribution_paging_back_and_foreign_task_picker_preserve_exact_draft_owner() {
        let mut app = App::new("/project".into(), 60, 18);
        app.task_id = 7;
        app.begin_edit_field(store::EditorField::Request);
        app.input = "keep 한글 e\u{301}\r\nbytes".into();
        app.cursor = 5;
        let original = (app.input.clone(), app.cursor, app.lead.id, app.form());
        let assert_retained = |app: &App| {
            assert!(!app.editing);
            assert_eq!(app.task_id, 7);
            assert_eq!(app.composer_return, Some(store::EditorField::Request));
            assert_eq!(app.composer_return_owner, Some((7, original.2)));
            let retained = app
                .drafts
                .iter()
                .find(|draft| draft.field == store::EditorField::Request)
                .unwrap();
            assert_eq!(
                (
                    retained.text.as_str(),
                    retained.cursor,
                    app.lead.id,
                    app.form()
                ),
                (original.0.as_str(), original.1, original.2, original.3)
            );
            assert!(app.switch_task.is_none());
            assert!(app.confirmation.is_none());
            assert!(app.planning.pending.is_none());
            assert!(!app.planning.auto_approve);
            assert!(app.execution.active.is_none());
            assert!(!app.records.mutation_pending());
        };
        app.records
            .snapshot
            .summaries
            .push(crate::workspace::records::PartnerSummary {
                partner: app.lead.clone(),
                produced_tasks: [0; 9],
                moves: [0; 9],
                move_names: Default::default(),
                shared_bond_memories: 0,
                unknown_producer_artifacts: 0,
            });
        assert!(app.command("routes"));
        assert_eq!(app.overlay, Some(Overlay::Records));
        assert_eq!(app.records.mode, Mode::Routes);
        assert_retained(&app);
        app.records.scroll = 3;
        key(&mut app, KeyCode::Char('c'));
        assert_eq!(app.records.mode, Mode::Contributions);
        assert_eq!(app.records.contributions.task, Some(7));
        app.records.scroll_max = 100;
        app.records.page = 4;
        key(&mut app, KeyCode::PageDown);
        assert_eq!(app.records.scroll, 4);
        key(&mut app, KeyCode::Esc);
        assert_eq!((app.records.mode, app.records.scroll), (Mode::Routes, 3));
        assert_retained(&app);
        key(&mut app, KeyCode::Char('c'));
        key(&mut app, KeyCode::Char('t'));
        app.records.snapshot.artifacts.push(artifact(
            8,
            Some(app.lead.id),
            1,
            ArtifactKind::Capture,
        ));
        let mut foreign = store::Snapshot::empty("/project".into());
        foreign.id = 8;
        app.tasks.push(foreign);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.overlay, Some(Overlay::Tasks));
        assert_eq!(app.visible_tasks().nth(app.task_selection).unwrap().id, 8);
        assert_retained(&app);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.overlay, Some(Overlay::Records));
        assert_eq!(app.records.mode, Mode::Contributions);
        assert_retained(&app);
        let mut repeated = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE);
        repeated.kind = KeyEventKind::Repeat;
        app.update(Event::Key(repeated));
        assert_eq!(app.records.contributions.partner, Some(app.lead.id));
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.records.mode, Mode::Routes);
        assert_retained(&app);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.overlay, None);
        assert!(app.composer_is_request());
        assert_eq!(app.task_id, 7);
        assert_eq!(
            (app.input.clone(), app.cursor, app.lead.id, app.form()),
            original
        );
        assert!(app.switch_task.is_none());
        assert!(app.confirmation.is_none());
        assert!(app.planning.pending.is_none());
        assert!(!app.planning.auto_approve);
        assert!(app.execution.active.is_none());
        assert!(!app.records.mutation_pending());
    }
}
