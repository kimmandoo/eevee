//! Reviewed local formation files. Navigation never starts a partner's work.
use super::*;
use crate::messages::Message;
use crate::workspace::workflows::{FormationSlot, Recipe, RecipeStep};
use std::{cell::Cell, path::PathBuf};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    LoadPath,
    SavePath,
    Export,
    Mapping,
}

/// Cached, local observations only. None of these values is dispatch authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Staging {
    Eligible,
    Unmapped,
    Duplicate,
    Changed,
    Busy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Missing,
    Unknown,
    Configured,
    Observed,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Readiness {
    pub staging: Staging,
    pub moves: Option<usize>,
    pub held: super::held::Readiness,
    pub route: Route,
    pub draft_nonempty: bool,
    pub owner_busy: bool,
    pub writer: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inspector {
    Moves { partner: i64, form: usize },
    Route { partner: i64, form: usize },
}
#[derive(Default)]
pub struct State {
    pub mode: Mode,
    pub path: String,
    pub cursor: usize,
    pub recipe: Option<Recipe>,
    pub mapping: Vec<Option<i64>>,
    pub selected: usize,
    pub details: String,
    pub scroll: u16,
    pub inspect: bool,
    pub reviewed: Cell<bool>,
    pub notice: Option<Message>,
    pub error: String,
    pub loading: bool,
    pub apply_pending: bool,
    revision: u64,
    owner: (i64, i64),
    pub roster: Vec<store::Partner>,
    pub inspector: Option<Inspector>,
    pub inspector_scroll: u16,
}
impl State {
    pub fn editing(&self) -> bool {
        matches!(self.mode, Mode::LoadPath | Mode::SavePath)
    }
    pub fn invalidate(&self) {
        self.reviewed.set(false);
    }
}
impl App {
    fn formation_target(&self, index: usize) -> Option<(i64, usize)> {
        let partner = self.formations.mapping.get(index).copied().flatten()?;
        let slot = self.formations.recipe.as_ref()?.roles.get(index)?;
        let form = FORMS.iter().position(|form| form.name == slot.form)?;
        Some((partner, form))
    }

    pub fn formation_pairing(
        &self,
        partner: i64,
        form: usize,
    ) -> Option<&crate::workspace::records::Pairing> {
        if !self.records.current_for(self.task_id, &self.cwd) {
            return None;
        }
        self.records
            .snapshot
            .pairings
            .iter()
            .find(|pairing| pairing.partner == partner && pairing.form == form)
    }

    fn formation_owned_busy(&self, partner: i64) -> bool {
        self.planning_state(partner)
            .is_some_and(|state| state.busy() || state.owns_work())
            || if partner == self.lead.id {
                self.execution.busy() || self.execution.outcome_unknown()
            } else {
                self.cooperation
                    .executions
                    .get(&partner)
                    .is_some_and(|state| state.busy() || state.outcome_unknown())
            }
    }

    fn formation_busy(&self) -> bool {
        // A terminal native outcome can remain unknown without owning work.
        // Frozen ownership still blocks CLI uncertainty and unsettled effects.
        std::iter::once(&self.planning)
            .chain(self.cooperation.parked.values())
            .any(|state| state.busy() || state.owns_work() || state.preferences_pending())
            || self.execution_any_pending()
            || self.pending_request.is_some()
            || self.pending_command.is_some()
    }

    fn formation_roster_current(&self) -> bool {
        self.party.len() == self.formations.roster.len()
            && self
                .party
                .iter()
                .zip(&self.formations.roster)
                .all(|(actual, expected)| {
                    if actual.id == self.lead.id {
                        expected == &self.lead
                    } else {
                        actual == expected
                    }
                })
    }

    /// Bounded borrowed metadata from the committed record snapshot and runtime
    /// owner caches. Never read configuration, files, credentials or the network.
    pub fn formation_readiness(&self, index: usize) -> Readiness {
        let target = self.formation_target(index);
        let partner = target.map(|(partner, _)| partner);
        let staging = if self.formations.owner != (self.task_id, self.lead.id)
            || self.revision != self.formations.revision
            || self.complete
            || self.archived
            || !self.formation_roster_current()
            || partner.is_some_and(|id| !self.formations.roster.iter().any(|p| p.id == id))
        {
            Staging::Changed
        } else if partner.is_none() {
            Staging::Unmapped
        } else if self
            .formations
            .mapping
            .iter()
            .filter(|id| **id == partner)
            .count()
            > 1
        {
            Staging::Duplicate
        } else if self.formation_busy() {
            Staging::Busy
        } else {
            Staging::Eligible
        };
        let current_records = self.records.current_for(self.task_id, &self.cwd);
        let moves = target.and_then(|(partner, form)| {
            current_records
                .then(|| {
                    self.records
                        .snapshot
                        .summaries
                        .iter()
                        .find(|summary| summary.partner.id == partner)
                        .map(|summary| summary.moves[form])
                })
                .flatten()
        });
        let held = target.map_or(super::held::Readiness::NotConfigured, |(partner, form)| {
            self.held_reference_readiness(partner, form)
        });
        let route = if !current_records {
            Route::Unknown
        } else if let Some(pairing) =
            target.and_then(|(partner, form)| self.formation_pairing(partner, form))
        {
            match self.agents.cached_readiness(pairing.agent.index()) {
                super::agents::Readiness::Configured => Route::Configured,
                super::agents::Readiness::SignedIn | super::agents::Readiness::Session => {
                    Route::Observed
                }
                super::agents::Readiness::Setup
                | super::agents::Readiness::MissingCli
                | super::agents::Readiness::Blocked
                | super::agents::Readiness::SignedOut => Route::Unavailable,
                super::agents::Readiness::Checking | super::agents::Readiness::Unknown => {
                    Route::Unknown
                }
            }
        } else {
            Route::Missing
        };
        let draft_nonempty = partner.is_some_and(|partner| {
            let drafts = if partner == self.lead.id {
                Some(&self.drafts)
            } else {
                self.partner_drafts.get(&partner).map(|draft| &draft.drafts)
            };
            drafts.is_some_and(|drafts| {
                drafts.iter().any(|draft| {
                    draft.field == store::EditorField::Request && !draft.text.is_empty()
                })
            }) || (partner == self.lead.id
                && self.editing
                && self.editor_field == store::EditorField::Request
                && !self.input.is_empty())
        });
        let writer = std::iter::once(&self.planning)
            .chain(self.cooperation.parked.values())
            .filter(|state| state.work_acquired)
            .filter_map(|state| state.work.as_ref())
            .find(|claim| claim.task_id == self.task_id && claim.writing)
            .map(|claim| claim.owner.partner);
        Readiness {
            staging,
            moves,
            held,
            route,
            draft_nonempty,
            owner_busy: partner.is_some_and(|partner| self.formation_owned_busy(partner)),
            writer,
        }
    }

    fn formation_inspect(&mut self, kind: char) {
        let Some((partner, form)) = self.formation_target(self.formations.selected) else {
            return;
        };
        self.formations.apply_pending = false;
        self.formations.invalidate();
        self.formations.inspector_scroll = 0;
        match kind {
            'h' => {
                self.inspect_held_reference(partner, form);
            }
            'm' => {
                let Some(index) = self
                    .records
                    .snapshot
                    .summaries
                    .iter()
                    .position(|summary| summary.partner.id == partner)
                else {
                    return;
                };
                self.formations.inspector = Some(Inspector::Moves { partner, form });
                // Reuse the existing factual move/name summary, not the mutable
                // move desk (whose Use/Teach actions belong to another scope).
                self.open_records(super::records::Mode::Summary);
                self.records.selected = index;
                self.records.scroll = 0;
            }
            'r' => {
                self.formations.inspector = Some(Inspector::Route { partner, form });
                self.open_overlay(Overlay::Agents);
            }
            _ => {}
        }
    }

    pub fn formation_inspector_active(&self) -> bool {
        matches!(
            (self.formations.inspector, self.overlay),
            (Some(Inspector::Moves { .. }), Some(Overlay::Records))
                | (Some(Inspector::Route { .. }), Some(Overlay::Agents))
        )
    }

    /// Called before command/global key routing so an inspector cannot select a
    /// route, teach/use a move, dispatch or change the owner behind the review.
    pub fn formation_inspector_key(&mut self, key: KeyEvent) -> bool {
        if !self.formation_inspector_active() {
            return false;
        }
        if key.kind != KeyEventKind::Press {
            return true;
        }
        if key.code == KeyCode::Esc && key.modifiers.is_empty() {
            self.formations.inspector = None;
            // Opening an inspector already in history can unwind and remove
            // the mapping frame. Return to its explicit owner, not an assumed
            // immediately preceding overlay.
            self.open_overlay(Overlay::Formations);
        } else if matches!(self.formations.inspector, Some(Inspector::Moves { .. }))
            && key.modifiers.is_empty()
        {
            match key.code {
                KeyCode::Up | KeyCode::Down => {
                    // Ordinary record arrows retarget the owner; this visit
                    // instead scrolls within its frozen mapped owner.
                    self.records.scroll = if key.code == KeyCode::Down {
                        self.records
                            .scroll
                            .saturating_add(1)
                            .min(self.records.scroll_max)
                    } else {
                        self.records.scroll.saturating_sub(1)
                    };
                    self.records.affirmative = false;
                }
                KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End => {
                    self.record_key(key);
                }
                _ => {}
            }
        } else if matches!(self.formations.inspector, Some(Inspector::Route { .. }))
            && key.modifiers.is_empty()
        {
            match key.code {
                KeyCode::PageDown => {
                    self.formations.inspector_scroll =
                        self.formations.inspector_scroll.saturating_add(4)
                }
                KeyCode::PageUp => {
                    self.formations.inspector_scroll =
                        self.formations.inspector_scroll.saturating_sub(4)
                }
                KeyCode::Home => self.formations.inspector_scroll = 0,
                KeyCode::End => self.formations.inspector_scroll = u16::MAX,
                _ => {}
            }
        }
        true
    }
    pub fn formation_notice(&self) -> Option<Message> {
        [Message::FormationSaved, Message::FormationApplied]
            .into_iter()
            .find(|message| self.notice == (*message).text())
    }
    pub(super) fn formation_command(&mut self, name: &str, argument: &str) -> bool {
        if !matches!(name, "formation" | "formation-save") {
            return false;
        }
        if let Some(reason) = self.command_disabled_reason(name) {
            self.notice = reason;
            return true;
        }
        self.command_popup = None;
        if self.party_inflight
            || self.party_request.is_some()
            || self.confirmation.is_some()
            || self.transitioning
            || self.quit_after_save
            || self.reset_after_save
        {
            self.notice = "Finish the current party change or review first.".into();
            return true;
        }
        if self.editing {
            self.retain_action_editor();
        }
        self.open_overlay(Overlay::Formations);
        self.formations = State {
            mode: if name == "formation-save" {
                Mode::SavePath
            } else {
                Mode::LoadPath
            },
            path: argument.into(),
            cursor: argument.len(),
            owner: (self.task_id, self.lead.id),
            ..State::default()
        };
        if !argument.is_empty() {
            self.formation_path();
        }
        true
    }
    fn formation_path(&mut self) {
        let path = PathBuf::from(&self.formations.path);
        if self.formations.path.is_empty()
            || self.formations.path.len() > 4096
            || self.formations.path.chars().any(char::is_control)
        {
            self.formations.notice = Some(Message::FormationChoosePath);
            return;
        }
        let path = if path.is_absolute() {
            path
        } else {
            PathBuf::from(&self.cwd).join(path)
        };
        self.formations.error.clear();
        if self.formations.mode == Mode::LoadPath {
            self.party_request = Some(store::PartyRequest::Formation(Box::new(
                store::formations::Request::Read { path },
            )));
            self.formations.loading = true;
            return;
        }
        let snapshot = self.snapshot(&self.cwd);
        let roster = self.partners();
        let roles = roster
            .iter()
            .enumerate()
            .map(|(index, partner)| FormationSlot {
                role: format!("slot-{}", index + 1),
                form: FORMS[partner.form].name.into(),
                request: snapshot
                    .partner_drafts
                    .get(&partner.id)
                    .and_then(|draft| {
                        draft
                            .drafts
                            .iter()
                            .find(|draft| draft.field == store::EditorField::Request)
                    })
                    .map_or_else(String::new, |draft| draft.text.clone()),
            })
            .collect::<Vec<_>>();
        let recipe = Recipe {
            schema_version: 2,
            profiles: Vec::new(),
            name: "Saved party arrangement".into(),
            steps: roles
                .iter()
                .map(|slot| RecipeStep {
                    form: slot.form.clone(),
                    action: "Stage the mapped partner only; review each action independently"
                        .into(),
                })
                .collect(),
            roles,
            scope: Vec::new(),
            context: Vec::new(),
            validation_commands: Vec::new(),
            completion_criteria: vec!["Review each staged action independently".into()],
        };
        match recipe.validate().and_then(|()| {
            toml::to_string_pretty(&recipe).map_err(|_| "Cannot encode formation".into())
        }) {
            Ok(details) => {
                self.formations.recipe = Some(recipe);
                self.formations.details = details;
                self.formations.mode = Mode::Export;
                self.formations.reviewed.set(false);
            }
            Err(error) => self.formations.error = error,
        }
    }
    pub(super) fn formation_paste(&mut self, value: &str) -> bool {
        if !self.formations.editing() || self.formations.loading || self.party_inflight {
            return true;
        }
        if self.formations.path.len().saturating_add(value.len()) > 4096
            || value.chars().any(char::is_control)
        {
            self.formations.notice = Some(Message::FormationChoosePath);
            return true;
        }
        let position = self.formations.cursor;
        self.formations.path.insert_str(position, value);
        self.formations.cursor = self
            .formations
            .path
            .grapheme_indices(true)
            .map(|(p, _)| p)
            .find(|p| *p >= position + value.len())
            .unwrap_or(self.formations.path.len());
        self.formations.notice = None;
        true
    }
    pub(super) fn formation_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return false;
        }
        if self.party_inflight || self.formations.loading {
            return true;
        }
        if key.code == KeyCode::Esc {
            self.formations.apply_pending = false;
            self.close_overlay();
            return true;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        if self.formations.editing() {
            let before = self.formations.path[..self.formations.cursor]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(p, _)| p);
            let after = self
                .formations
                .path
                .grapheme_indices(true)
                .map(|(p, _)| p)
                .find(|p| *p > self.formations.cursor)
                .unwrap_or(self.formations.path.len());
            match key.code {
                KeyCode::Enter => self.formation_path(),
                KeyCode::Left => self.formations.cursor = before,
                KeyCode::Right => self.formations.cursor = after,
                KeyCode::Home => self.formations.cursor = 0,
                KeyCode::End => self.formations.cursor = self.formations.path.len(),
                KeyCode::Backspace => {
                    self.formations
                        .path
                        .replace_range(before..self.formations.cursor, "");
                    self.formations.cursor = before;
                }
                KeyCode::Delete => {
                    self.formations
                        .path
                        .replace_range(self.formations.cursor..after, "");
                }
                KeyCode::Char(c) => {
                    let mut bytes = [0; 4];
                    self.formation_paste(c.encode_utf8(&mut bytes));
                }
                _ => {}
            }
            return true;
        }
        if key.code != KeyCode::Char('y') {
            self.formations.apply_pending = false;
        }
        match key.code {
            KeyCode::Char('n') => self.close_overlay(),
            KeyCode::Home => self.formations.scroll = 0,
            KeyCode::End => self.formations.scroll = u16::MAX,
            KeyCode::Char('d') => {
                self.formations.inspect = !self.formations.inspect;
                self.formations.scroll = 0;
                self.formations.invalidate();
            }
            KeyCode::PageDown => self.formations.scroll = self.formations.scroll.saturating_add(4),
            KeyCode::PageUp => self.formations.scroll = self.formations.scroll.saturating_sub(4),
            KeyCode::Char(kind @ ('m' | 'h' | 'r'))
                if self.formations.mode == Mode::Mapping && !self.formations.inspect =>
            {
                self.formation_inspect(kind);
            }
            KeyCode::Up => self.formations.selected = self.formations.selected.saturating_sub(1),
            KeyCode::Down => {
                self.formations.selected = (self.formations.selected + 1)
                    .min(self.formations.mapping.len().saturating_sub(1))
            }
            KeyCode::Left | KeyCode::Right
                if self.formations.mode == Mode::Mapping && !self.formations.inspect =>
            {
                let roster = self.partners();
                let selected = self.formations.selected;
                let current = self.formations.mapping[selected]
                    .and_then(|id| roster.iter().position(|p| p.id == id))
                    .map_or(0, |index| index + 1);
                let next = if key.code == KeyCode::Right {
                    (current + 1) % (roster.len() + 1)
                } else {
                    (current + roster.len()) % (roster.len() + 1)
                };
                self.formations.mapping[selected] =
                    next.checked_sub(1).map(|index| roster[index].id);
                self.formations.invalidate();
                self.formations.error.clear();
                self.formations.notice = None;
            }
            KeyCode::Char('y')
                if !self.formations.inspect
                    && self.formations.reviewed.get()
                    && self.width >= 60
                    && self.height >= 18 =>
            {
                self.formation_confirm()
            }
            _ => {}
        }
        true
    }
    fn formation_confirm(&mut self) {
        let Some(recipe) = self.formations.recipe.clone() else {
            return;
        };
        let request = if self.formations.mode == Mode::Export {
            let path = PathBuf::from(&self.formations.path);
            let path = if path.is_absolute() {
                path
            } else {
                PathBuf::from(&self.cwd).join(path)
            };
            store::formations::Request::Export { path, recipe }
        } else {
            if self.task_id <= 0
                || self.formations.owner != (self.task_id, self.lead.id)
                || self.complete
                || self.archived
                || self.revision != self.formations.revision
                || self.partners() != self.formations.roster
            {
                self.formations.notice = Some(Message::FormationChanged);
                self.formations.invalidate();
                return;
            }
            if self.formation_busy() {
                self.formations.notice = Some(Message::FormationBusy);
                self.formations.apply_pending = false;
                return;
            }
            if !self.committed_input_ready()
                || self.moves.mutation_pending()
                || self.records.persistence_pending()
                || !self.records.local.is_empty()
            {
                self.formations.apply_pending = true;
                self.formations.notice = Some(Message::FormationWaiting);
                self.flush_save();
                return;
            }
            self.formations.apply_pending = false;
            self.transitioning = true;
            store::formations::Request::Apply {
                expected: Box::new(self.snapshot(&self.cwd)),
                roster: self.formations.roster.clone(),
                recipe,
                mapping: self.formations.mapping.clone(),
            }
        };
        self.formations.invalidate();
        self.formations.loading = true;
        self.party_request = Some(store::PartyRequest::Formation(Box::new(request)));
    }
    pub fn settle_formations(&mut self) -> bool {
        if !self.formations.apply_pending {
            return false;
        }
        if self.overlay != Some(Overlay::Formations)
            || self.formations.inspect
            || self.command_popup.is_some()
            || self.quit
            || self.transitioning
            || self.confirmation.is_some()
            || self.quit_after_save
            || self.reset_after_save
        {
            self.formations.apply_pending = false;
            return false;
        }
        if self.revision != self.formations.revision
            || self.formations.owner != (self.task_id, self.lead.id)
            || self.partners() != self.formations.roster
            || self.planning_any_busy()
            || self.execution_any_pending()
            || matches!(self.storage, StorageState::Failed(_))
        {
            self.formations.apply_pending = false;
            self.formations.notice = Some(Message::FormationChanged);
            return true;
        }
        if !self.committed_input_ready()
            || self.moves.mutation_pending()
            || self.records.persistence_pending()
            || !self.records.local.is_empty()
        {
            return false;
        }
        self.formations.apply_pending = false;
        self.formation_confirm();
        true
    }
    pub(super) fn formation_event(&mut self, result: Result<store::formations::Response, String>) {
        self.party_inflight = false;
        if self.recipe_export_busy() {
            self.recipe_export_event(result.and_then(|response| match response {
                store::formations::Response::WorkflowExported { generation } => Ok(generation),
                _ => Err("Unexpected workflow export response; no authority transferred".into()),
            }));
            return;
        }
        self.formations.loading = false;
        self.transitioning = false;
        match result {
            Ok(store::formations::Response::WorkflowExported { .. }) => {}
            Err(error) => {
                self.formations.error = error;
                self.formations.invalidate();
            }
            Ok(store::formations::Response::Read(recipe)) => {
                if self.formations.owner != (self.task_id, self.lead.id) {
                    self.formations.notice = Some(Message::FormationChanged);
                    self.formations.invalidate();
                    return;
                }
                self.formations.details = toml::to_string_pretty(&recipe).unwrap_or_default();
                let roster = self.partners();
                self.formations.mapping = (0..recipe.roles.len())
                    .map(|index| roster.get(index).map(|p| p.id))
                    .collect();
                self.formations.recipe = Some(recipe);
                self.formations.mode = Mode::Mapping;
                self.formations.roster = roster;
                self.formations.revision = self.revision;
                self.formations.invalidate();
            }
            Ok(store::formations::Response::Exported) => {
                self.close_overlay();
                self.notice = (Message::FormationSaved).text().into();
            }
            Ok(store::formations::Response::Applied { task, roster }) => {
                for partner in &roster {
                    if self.formations.mapping.contains(&Some(partner.id))
                        && let Some(state) = self.planning_state_mut(partner.id)
                    {
                        state.front = Some(store::Attribution::of(partner));
                    }
                }
                self.party = roster;
                if let Some(partner) = self.party.iter().find(|p| p.id == self.lead.id) {
                    self.lead = partner.clone();
                }
                self.partner_drafts = task.partner_drafts.clone();
                self.revision = task.revision;
                self.saved_revision = task.revision;
                self.updated_at = task.updated_at;
                self.save_requested = false;
                self.autosave = None;
                self.previous_form = task.navigation.previous_form;
                self.form_selection = task.form;
                if let Some(draft) = self.partner_drafts.get(&self.lead.id) {
                    self.drafts = draft.drafts.clone();
                }
                if let Some(saved) = self.tasks.iter_mut().find(|t| t.id == task.id) {
                    *saved = *task;
                }
                self.clear_carry_over();
                self.evolution.transition = None;
                self.sync_front();
                self.records_invalidate();
                self.close_overlay();
                self.refresh_save_state();
                self.notice = (Message::FormationApplied).text().into();
            }
        }
    }
}

#[cfg(test)]
#[path = "formations_tests.rs"]
mod tests;
