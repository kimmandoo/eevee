//! Game-loop skin over existing reviewed actions: the bag, Potions, Berries,
//! repetition-proposed moves and item notices. Nothing here sends, runs,
//! approves or grants; every row opens an existing command or review.
use super::{App, Overlay, StorageState};
use crate::{
    messages::Message,
    workspace::{
        game::{self, Berry},
        moves::{self, Proposal, Provenance, Scope, Steps},
        runner,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Sprites the item set provides; text icons stand in when none is cached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Potion,
    Berry,
    /// A technical machine of the given form's type.
    Tm(usize),
    Key,
}

impl Item {
    pub fn icon(self) -> &'static str {
        match self {
            Item::Potion => "[+]",
            Item::Berry => "(o)",
            Item::Tm(_) => "[TM]",
            Item::Key => "[K]",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pocket {
    Items,
    Tms,
    Key,
}

impl Pocket {
    pub fn label(self) -> &'static str {
        match self {
            Pocket::Items => "ITEMS",
            Pocket::Tms => "TMs",
            Pocket::Key => "KEY ITEMS",
        }
    }
}

/// What Enter on a bag row does. Each variant only opens an existing surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Command(&'static str),
    Rerun(String),
    ToggleBerry(Berry),
    MoveDetails(i64),
    None,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub pocket: Pocket,
    pub item: Item,
    pub name: String,
    /// What the row opens, shown with `d` and on the selected row.
    pub detail: String,
    pub action: Action,
}

#[derive(Default)]
pub struct Bag {
    pub selected: usize,
    pub details: bool,
}

/// One static notice for a real committed event. It never covers consent or
/// live code and the next real key dismisses it (that key still acts).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub item: Item,
    pub title: String,
    pub text: String,
}

/// A repeated instruction this partner/form could learn as a move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveProposal {
    pub key: String,
    pub instruction: String,
    pub name: String,
    pub sources: Vec<String>,
}

/// Distinct dispatched Sends needed before Eevee offers to learn a move.
pub const REPEATS: usize = 3;

impl App {
    fn game(&self) -> game::State {
        self.workflows.game.clone().unwrap_or_default()
    }

    fn set_game(&mut self, state: game::State) {
        self.workflows.game = (!state.is_empty()).then_some(state);
        self.changed_task();
    }

    pub fn berries(&self) -> &[Berry] {
        self.workflows
            .game
            .as_ref()
            .map_or(&[], |state| state.berries.as_slice())
    }

    pub fn is_berry(&self, berry: Berry) -> bool {
        self.berries().contains(&berry)
    }

    /// The latest task-owned run, when it failed: a Potion opens its rerun review.
    fn failed_run(&self) -> Option<&runner::RunRecord> {
        self.execution
            .history
            .iter()
            .find(|run| run.prepared.task_id == self.task_id)
            .filter(|run| run.state == runner::RunState::Failed)
    }

    pub fn bag_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut potion = |name: &str, detail: &str, action: Action| {
            rows.push(Row {
                pocket: Pocket::Items,
                item: Item::Potion,
                name: format!("Potion · {name}"),
                detail: detail.into(),
                action,
            });
        };
        if let Some(run) = self.failed_run() {
            potion(
                "failed command",
                "Opens the rerun review for the failed command; it needs a new approval.",
                Action::Rerun(run.prepared.run_id.clone()),
            );
        }
        if matches!(self.storage, StorageState::Failed(_)) {
            potion(
                "storage",
                "Retries the failed local save (/storage-retry).",
                Action::Command("storage-retry"),
            );
        }
        if self.moves.failed.is_some() {
            potion(
                "move storage",
                "Opens the move retry (/move-retry).",
                Action::Command("move-retry"),
            );
        }
        if self.records.failed.is_some() {
            potion(
                "record storage",
                "Retries the failed record write (/records-retry).",
                Action::Command("records-retry"),
            );
        }
        if self.workflows.tool_plans.as_ref().is_some_and(|plan| {
            plan.nodes
                .iter()
                .any(|node| node.status == crate::workspace::tool_plans::Status::Failed)
        }) {
            potion(
                "tool plan",
                "Previews resuming failed or unstarted plan steps (/plan-resume).",
                Action::Command("plan-resume"),
            );
        }
        if self.agents.readiness(self.planning.agent.index()) == super::agents::Readiness::Blocked {
            potion(
                "connection",
                "Opens connection diagnostics (/provider); nothing is resent.",
                Action::Command("provider"),
            );
        }
        if !rows.iter().any(|row| row.item == Item::Potion) {
            rows.push(Row {
                pocket: Pocket::Items,
                item: Item::Potion,
                name: "Potion · nothing to recover".into(),
                detail: "A Potion appears when a command, save or connection fails.".into(),
                action: Action::None,
            });
        }
        for excerpt in &self.sources.basket {
            let berry = Berry::Source {
                excerpt_id: excerpt.id,
            };
            let on = self.is_berry(berry);
            rows.push(Row {
                pocket: Pocket::Items,
                item: Item::Berry,
                name: format!(
                    "Berry · {}:{}-{}{}",
                    excerpt.path,
                    excerpt.start_line,
                    excerpt.end_line,
                    if on { " · once" } else { "" }
                ),
                detail: if on {
                    "Sent once with the next Send, then removed. Enter keeps it attached."
                } else {
                    "Enter makes this attachment one-shot: the next Send includes it, then it is removed."
                }
                .into(),
                action: Action::ToggleBerry(berry),
            });
        }
        for id in &self.memory.selected_ids {
            let berry = Berry::Memory { id: *id };
            let on = self.is_berry(berry);
            rows.push(Row {
                pocket: Pocket::Items,
                item: Item::Berry,
                name: format!("Berry · memory #{id}{}", if on { " · once" } else { "" }),
                detail: if on {
                    "Sent once with the next Send, then deselected. Enter keeps it selected."
                } else {
                    "Enter makes this memory one-shot: the next Send includes it, then it is deselected."
                }
                .into(),
                action: Action::ToggleBerry(berry),
            });
        }
        if self.sources.basket.is_empty() && self.memory.selected_ids.is_empty() {
            rows.push(Row {
                pocket: Pocket::Items,
                item: Item::Berry,
                name: "Berry · attach something first".into(),
                detail: "Attach a file with @ or /files, or /select-memory, then make it a one-shot Berry here.".into(),
                action: Action::Command("files"),
            });
        }
        for record in self.moves.records.iter().filter(|record| {
            record.proposal.partner == self.lead.id && record.proposal.form == self.form()
        }) {
            rows.push(Row {
                pocket: Pocket::Tms,
                item: Item::Tm(record.proposal.form),
                name: format!("TM · {}", record.proposal.name),
                detail: "Shows this move's steps; export it as a portable TM from /move.".into(),
                action: Action::MoveDetails(record.id),
            });
        }
        rows.push(Row {
            pocket: Pocket::Tms,
            item: Item::Tm(self.form()),
            name: "Import a TM file".into(),
            detail: "Reads a portable TM and opens the learning review (/move-import).".into(),
            action: Action::Command("move-import"),
        });
        rows.push(Row {
            pocket: Pocket::Tms,
            item: Item::Tm(self.form()),
            name: "Teach the current work as a move".into(),
            detail: "Opens the teaching review (/teach-move); learning needs your review.".into(),
            action: Action::Command("teach-move"),
        });
        let held = self.held_target_label();
        rows.push(Row {
            pocket: Pocket::Key,
            item: Item::Key,
            name: if held.is_empty() {
                "Held item · none".into()
            } else {
                format!("Held item · {held}")
            },
            detail: "Inspects this form's held file or memory (/held).".into(),
            action: Action::Command("held"),
        });
        rows.push(Row {
            pocket: Pocket::Key,
            item: Item::Key,
            name: "Game terms".into(),
            detail: "Explains moves, TMs, held items, Berries and Potions (/help).".into(),
            action: Action::Command("help"),
        });
        rows
    }

    pub(super) fn open_bag(&mut self) {
        self.bag.selected = 0;
        self.bag.details = false;
        self.open_overlay(Overlay::Bag);
    }

    pub(super) fn bag_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.request_quit();
            return true;
        }
        let rows = self.bag_rows();
        let last = rows.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc => self.close_overlay(),
            KeyCode::Up => self.bag.selected = self.bag.selected.saturating_sub(1),
            KeyCode::Down => self.bag.selected = (self.bag.selected + 1).min(last),
            KeyCode::Home => self.bag.selected = 0,
            KeyCode::End => self.bag.selected = last,
            KeyCode::Char('d') => self.bag.details = !self.bag.details,
            KeyCode::Enter => {
                if let Some(row) = rows.get(self.bag.selected.min(last)) {
                    self.use_bag_row(row.action.clone());
                }
            }
            _ => return false,
        }
        true
    }

    fn use_bag_row(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::ToggleBerry(berry) => {
                if self.planning_any_busy() {
                    self.notice =
                        "Wait for the current request; Berries apply to the next Send.".into();
                    return;
                }
                let mut state = self.game();
                match state.toggle(berry) {
                    Ok(on) => {
                        self.set_game(state);
                        self.notice = if on {
                            "Berry ready: the next Send includes it once, then removes it. Nothing sent."
                        } else {
                            "Berry put back: it stays attached as ordinary context."
                        }
                        .into();
                    }
                    Err(error) => self.notice = error,
                }
            }
            Action::Rerun(run) => {
                self.close_overlay();
                if let Err(error) = self.stage_execution_rerun(run) {
                    self.notice = error;
                }
            }
            Action::MoveDetails(id) => {
                self.close_overlay();
                self.command("move");
                if let Some(index) = self.moves.records.iter().position(|record| record.id == id) {
                    self.moves.selected = index;
                    self.command("move-details");
                }
            }
            Action::Command(name) => {
                self.close_overlay();
                self.command(name);
            }
        }
    }

    /// After dispatch acknowledgment: remove one-shot context and announce it.
    pub(super) fn consume_berries(&mut self) {
        let mut state = self.game();
        if state.berries.is_empty() {
            return;
        }
        // Only Berries whose attachment or selection still exists went with
        // this Send; orphans (removed meanwhile) are dropped without a claim.
        let used: Vec<Berry> = std::mem::take(&mut state.berries)
            .into_iter()
            .filter(|berry| match *berry {
                Berry::Source { excerpt_id } => self
                    .sources
                    .basket
                    .iter()
                    .any(|excerpt| excerpt.id == excerpt_id),
                Berry::Memory { id } => self.memory.selected_ids.contains(&id),
            })
            .collect();
        if used.is_empty() {
            self.set_game(state);
            return;
        }
        for berry in &used {
            match *berry {
                Berry::Source { excerpt_id } => self.berry_removals.push(excerpt_id),
                Berry::Memory { id } => self.memory.selected_ids.retain(|selected| *selected != id),
            }
        }
        self.set_game(state);
        self.settle_berries();
        self.show_notice(Notice {
            item: Item::Berry,
            title: "Berry used!".into(),
            text: format!(
                "{} one-shot item{} went with this Send and {} removed.",
                used.len(),
                if used.len() == 1 { "" } else { "s" },
                if used.len() == 1 { "was" } else { "were" }
            ),
        });
    }

    /// Queue consumed basket removals once the source store is idle.
    pub(super) fn settle_berries(&mut self) {
        while let Some(&id) = self.berry_removals.first() {
            if !self.sources.remove_now(self.task_id, id) {
                return;
            }
            self.berry_removals.remove(0);
        }
    }

    pub fn show_notice(&mut self, notice: Notice) {
        self.item_notice = Some(notice);
    }

    /// Notices yield to consent, questions, live file code and active work.
    pub fn item_notice_visible(&self) -> Option<&Notice> {
        self.item_notice
            .as_ref()
            .filter(|_| self.evolution_popup_ready() || self.approval_count() == 0)
            .filter(|_| {
                self.planning.request.is_none()
                    && self.confirmation.is_none()
                    && self.command_popup.is_none()
                    && self.overlay.is_none()
                    && self.width >= 60
                    && self.height >= 18
            })
    }

    /// Repeated similar Sends of this partner/form that no move covers yet.
    pub fn move_proposal(&self) -> Option<MoveProposal> {
        let form = self.form();
        if !matches!(form, 3 | 4 | 7 | 8) || !self.records.loaded || self.task_id <= 0 {
            return None;
        }
        let dismissed = self.game().dismissed_moves;
        let covered = |key: &str| {
            self.moves.records.iter().any(|record| {
                record.proposal.partner == self.lead.id
                    && record.proposal.form == form
                    && matches!(&record.proposal.steps, Steps::Request { instruction, .. }
                        if game::loose_key(instruction) == key)
            })
        };
        let mut groups: Vec<(String, String, i64, Vec<String>)> = Vec::new();
        for habit in &self.records.snapshot.habits {
            if habit.instruction.is_empty() || habit.dismissed {
                continue;
            }
            // Read-only or negated instructions describe a constraint, not a move.
            if super::evolution::classify(&habit.instruction)
                .is_some_and(|selection| selection.reason == Message::CoreEvolutionReadonly)
            {
                continue;
            }
            let key = game::loose_key(&habit.instruction);
            for source in habit
                .sources
                .iter()
                .filter(|source| source.owner.partner == self.lead.id && source.owner.form == form)
            {
                let entry = match groups.iter_mut().position(|group| group.0 == key) {
                    Some(index) => &mut groups[index],
                    None => {
                        groups.push((key.clone(), habit.instruction.clone(), 0, Vec::new()));
                        groups.last_mut().unwrap()
                    }
                };
                if !entry.3.contains(&source.request_id) {
                    entry.3.push(source.request_id.clone());
                }
                if source.at >= entry.2 {
                    entry.2 = source.at;
                    entry.1.clone_from(&habit.instruction);
                }
            }
        }
        let (key, instruction, _, sources) = groups
            .into_iter()
            .filter(|group| {
                group.3.len() >= REPEATS && !dismissed.contains(&group.0) && !covered(&group.0)
            })
            .max_by_key(|group| group.2)?;
        let steps = Steps::Request {
            instruction: instruction.clone(),
            pins: Vec::new(),
        };
        let used: Vec<String> = self
            .moves
            .records
            .iter()
            .map(|record| record.proposal.name.clone())
            .collect();
        Some(MoveProposal {
            key,
            name: moves::suggested_name(form, &steps, &used),
            instruction,
            sources,
        })
    }

    /// Shown only when nothing else needs the user.
    pub fn move_proposal_visible(&self) -> Option<MoveProposal> {
        (self.planning.activity_label().is_none()
            && !self.planning_any_busy()
            && self.approval_count() == 0
            && self.planning.request.is_none())
        .then(|| self.move_proposal())
        .flatten()
    }

    pub(super) fn learn_proposed_move(&mut self) {
        let Some(proposal) = self.move_proposal() else {
            self.notice = "No repeated request to learn right now.".into();
            return;
        };
        if self.task_id <= 0 || self.saved_revision != self.revision {
            self.notice = "Save the task before teaching a move.".into();
            return;
        }
        let teach = Proposal {
            partner: self.lead.id,
            form: self.form(),
            name: proposal.name,
            scope: Scope::Project,
            steps: Steps::Request {
                instruction: proposal.instruction,
                pins: Vec::new(),
            },
            provenance: Provenance {
                task_id: self.task_id,
                sources: proposal.sources.into_iter().take(16).collect(),
                taught_by: None,
                teacher_name: None,
            },
        };
        if let Err(error) = moves::validate_proposal(&teach) {
            self.notice = error;
            return;
        }
        self.open_moves();
        if self.overlay == Some(Overlay::Moves) {
            self.moves
                .begin_learning(super::moves::Learning::Teach(teach));
        }
    }

    pub(super) fn dismiss_proposed_move(&mut self) {
        let Some(proposal) = self.move_proposal() else {
            self.notice = "No repeated request to dismiss.".into();
            return;
        };
        let mut state = self.game();
        state.dismiss(&proposal.key);
        self.set_game(state);
        self.notice = "Eevee won't offer that move again for this task.".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        store,
        workspace::{
            context,
            records::{Habit, InstructionSource},
        },
    };
    use crossterm::event::Event;

    fn app() -> App {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.task_id = 7;
        app.storage = StorageState::Saved;
        app.records.loaded = true;
        app.set_form(3);
        app
    }

    fn habit(instruction: &str, requests: &[&str], form: usize) -> Habit {
        Habit {
            key: crate::workspace::records::instruction_key(instruction)
                .unwrap()
                .0,
            instruction: instruction.into(),
            sources: requests
                .iter()
                .enumerate()
                .map(|(at, id)| InstructionSource {
                    request_id: (*id).into(),
                    task_id: 7,
                    owner: store::Attribution {
                        partner: 1,
                        name: "Eevee #1".into(),
                        form,
                    },
                    at: at as i64,
                })
                .collect(),
            dismissed: false,
            memory: None,
        }
    }

    fn excerpt(id: i64) -> context::Excerpt {
        context::Excerpt {
            origin_run: None,
            id,
            task_id: 7,
            root: "/workspace".into(),
            path: format!("src/{id}.rs"),
            start_line: 1,
            end_line: 2,
            source_sha256: [1; 32],
            text: "fn a() {}\n".into(),
            available: true,
        }
    }

    #[test]
    fn three_similar_sends_propose_a_move_and_other_cases_do_not() {
        let mut app = app();
        // Loosely equal variants of one instruction add up across habits.
        app.records.snapshot.habits = vec![
            habit("Run the tests and fix failures", &["r1", "r2"], 3),
            habit("run the tests and fix failures.", &["r3"], 3),
        ];
        let proposal = app.move_proposal().expect("three similar Sends propose");
        assert_eq!(proposal.sources.len(), 3);
        // The most recently sent wording is the one taught.
        assert_eq!(proposal.instruction, "Run the tests and fix failures");
        // Two Sends are not enough.
        app.records.snapshot.habits = vec![habit("Fix the bug", &["r1", "r2"], 3)];
        assert!(app.move_proposal().is_none());
        // Another form's Sends never count for this form.
        app.records.snapshot.habits = vec![habit("Fix the bug", &["r1", "r2", "r3"], 4)];
        assert!(app.move_proposal().is_none());
        // Read-only instructions describe a constraint, not a move.
        app.records.snapshot.habits = vec![habit(
            "Read src/main.rs; do not change files",
            &["r1", "r2", "r3"],
            3,
        )];
        assert!(app.move_proposal().is_none());
        // Forms whose moves are commands or pins cannot learn a request.
        app.records.snapshot.habits = vec![habit("Fix the bug", &["r1", "r2", "r3"], 2)];
        app.set_form(2);
        assert!(app.move_proposal().is_none());
        app.set_form(3);
        app.records.snapshot.habits = vec![habit("Fix the bug", &["r1", "r2", "r3"], 3)];
        assert!(app.move_proposal().is_some());
        // Dismissal is task-owned and persists with the task workflows.
        app.dismiss_proposed_move();
        assert!(app.move_proposal().is_none());
        let saved = serde_json::to_string(&app.workflows).unwrap();
        assert!(saved.contains("dismissed_moves"), "{saved}");
        // Nothing is shown while a request is busy or a question waits.
        app.workflows.game = None;
        assert!(app.move_proposal_visible().is_some());
    }

    #[test]
    fn learning_a_proposed_move_opens_the_ordinary_teach_review() {
        let mut app = app();
        app.saved_revision = app.revision;
        app.records.snapshot.habits = vec![habit("Fix the bug", &["r1", "r2", "r3"], 3)];
        app.learn_proposed_move();
        assert_eq!(app.overlay, Some(Overlay::Moves));
        let learning = app.moves.learning.as_ref().expect("teach review opened");
        let proposal = learning.proposal();
        assert!(
            matches!(&proposal.steps, Steps::Request { instruction, .. } if instruction == "Fix the bug")
        );
        assert_eq!(proposal.provenance.sources, vec!["r1", "r2", "r3"]);
        // Inventory only changes after the review commits through the store.
        assert!(app.moves.records.is_empty());
        assert!(app.pending_request.is_none() && app.planning.pending.is_none());
    }

    #[test]
    fn berries_are_one_shot_and_consumed_only_after_dispatch() {
        let mut app = app();
        app.sources.basket = vec![excerpt(4), excerpt(5)];
        app.memory.selected_ids = vec![9];
        app.open_bag();
        let rows = app.bag_rows();
        let berry = rows
            .iter()
            .position(|row| row.action == Action::ToggleBerry(Berry::Source { excerpt_id: 4 }))
            .unwrap();
        app.bag.selected = berry;
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let memory = app
            .bag_rows()
            .iter()
            .position(|row| row.action == Action::ToggleBerry(Berry::Memory { id: 9 }))
            .unwrap();
        app.bag.selected = memory;
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.berries().len(), 2);
        assert!(
            app.bag_rows()
                .iter()
                .any(|row| row.name.contains("src/4.rs") && row.name.contains("once"))
        );
        assert!(app.pending_request.is_none() && app.planning.pending.is_none());
        // Restart keeps waiting Berries with the task.
        let json = serde_json::to_string(&app.workflows).unwrap();
        let restored: crate::workspace::workflows::State = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.game.unwrap().berries.len(), 2);
        // Dispatch acknowledgment consumes them exactly once.
        app.close_overlay();
        app.consume_berries();
        assert!(app.berries().is_empty());
        assert!(app.memory.selected_ids.is_empty());
        assert!(matches!(
            app.sources
                .pending_store
                .as_ref()
                .map(|request| &request.operation),
            Some(store::ContextOperation::Remove(4))
        ));
        assert!(
            app.sources.basket.iter().any(|e| e.id == 5),
            "non-berry stays attached"
        );
        let notice = app.item_notice.as_ref().expect("berry notice");
        assert_eq!(notice.item, Item::Berry);
        // An orphaned Berry (its attachment was removed) is dropped silently.
        app.item_notice = None;
        app.workflows.game = Some(game::State {
            berries: vec![Berry::Source { excerpt_id: 99 }],
            ..Default::default()
        });
        app.consume_berries();
        assert!(app.berries().is_empty() && app.item_notice.is_none());
    }

    #[test]
    fn potions_open_existing_recovery_reviews_only() {
        let mut app = app();
        let none = app.bag_rows();
        assert!(
            none.iter()
                .any(|row| row.item == Item::Potion && row.action == Action::None)
        );
        app.storage = StorageState::Failed("disk full".into());
        let rows = app.bag_rows();
        let potion = rows
            .iter()
            .find(|row| row.item == Item::Potion)
            .expect("storage potion");
        assert_eq!(potion.action, Action::Command("storage-retry"));
        assert!(
            rows.iter()
                .any(|row| row.pocket == Pocket::Key && row.action == Action::Command("help"))
        );
    }

    #[test]
    fn bag_keeps_the_draft_and_notices_fade_on_the_next_key() {
        let mut app = app();
        app.editing = true;
        app.editor_field = store::EditorField::Request;
        app.input = "Keep this draft".into();
        app.cursor = 4;
        app.command("bag");
        assert_eq!(app.overlay, Some(Overlay::Bag));
        app.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert_eq!(app.overlay, None);
        assert_eq!((app.input.as_str(), app.cursor), ("Keep this draft", 4));
        app.show_notice(Notice {
            item: Item::Tm(3),
            title: "New move!".into(),
            text: "Eevee learned Ember!".into(),
        });
        assert!(app.item_notice_visible().is_some());
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
        )));
        assert!(app.item_notice.is_none());
        assert!(app.input.contains('x'), "the dismissing key still types");
    }
}
