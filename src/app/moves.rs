//! Local move desk. Only store requests commit changes; using a move stages normal review.
use std::collections::{HashSet, VecDeque};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

use super::{App, Overlay, StorageState};
use crate::{
    store::moves::{Decision, Operation, Request, Response},
    workspace::moves::{self, Candidate, Outcome, Proposal, Record, Scope, Steps, Usage},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Menu,
    Detail,
    Name,
    Review,
    Forget,
    Replace,
    Reminder,
    Learn,
    TeachTarget,
    LoadingTarget,
    TaskReview,
}

#[derive(Clone, Debug)]
pub enum Learning {
    Teach(Proposal),
    Relearn(Record),
    Rename(Record),
}

impl Learning {
    pub fn proposal(&self) -> &Proposal {
        match self {
            Self::Teach(p) => p,
            Self::Relearn(r) | Self::Rename(r) => &r.proposal,
        }
    }
}

#[derive(Default)]
pub struct State {
    pub mode: Mode,
    pub records: Vec<Record>,
    pub candidates: Vec<Candidate>,
    pub usages: Vec<Usage>,
    pub selected: usize,
    pub detail: Option<i64>,
    pub task_id: i64,
    pub partner: i64,
    pub form: usize,
    pub loaded: bool,
    /// Retained on a full store queue. The runtime passes this to Worker::try_moves.
    pub pending: Option<Request>,
    pub inflight: Option<Request>,
    pub queued: VecDeque<Request>,
    /// Failed operations retain their exact owner and bytes until explicit retry or reviewed discard.
    pub failed: Option<Request>,
    pub failed_error: String,
    pub failure_discard: bool,
    pub learning: Option<Learning>,
    pub replacement: Option<(i64, u64)>,
    pub offered: Option<Candidate>,
    pub name: String,
    pub cursor: usize,
    pub scope: Scope,
    /// Tab moves between the independent name editor and scope control.
    pub scope_focus: bool,
    pub insertion_rejected: bool,
    pub reviewed: bool,
    pub affirmative: bool,
    pub scroll: u16,
    pub scroll_max: u16,
    pub scroll_page: u16,
    pub notice: String,
    /// Candidate ids already displayed in this process; decisions are persisted separately.
    pub displayed: HashSet<String>,
    deferred: Option<String>,
}

impl State {
    /// Clear only the visible desk; accepted writes and observations retain ownership.
    pub fn reset(&mut self) {
        let pending = self.pending.take();
        let inflight = self.inflight.take();
        let queued = std::mem::take(&mut self.queued);
        let failed = self.failed.take();
        let failed_error = std::mem::take(&mut self.failed_error);
        *self = Self::default();
        self.pending = pending;
        self.inflight = inflight;
        self.queued = queued;
        self.failed = failed;
        self.failed_error = failed_error;
    }
    pub fn reset_view(&mut self, task: i64, partner: i64, form: usize) {
        self.reset();
        self.task_id = task;
        self.partner = partner;
        self.form = form;
    }
    pub fn editing(&self) -> bool {
        self.mode == Mode::Name
    }
    pub fn invalidate_review(&mut self) {
        self.reviewed = false;
        self.affirmative = false;
    }
    pub fn persistence_pending(&self) -> bool {
        self.pending.is_some()
            || self.inflight.is_some()
            || self.failed.is_some()
            || !self.queued.is_empty()
    }
    fn input_blocked(&self) -> bool {
        // A prompt-display receipt changes no slots or steps. Keep local
        // navigation/naming responsive; decisions queue behind that receipt.
        self.persistence_pending()
            && (self.failed.is_some()
                || !self.queued.is_empty()
                || self.inflight.as_ref().is_none_or(|request| {
                    !matches!(
                        request.operation,
                        Operation::CandidateDecision {
                            decision: Decision::Offered,
                            ..
                        }
                    )
                }))
    }
    pub fn busy(&self) -> bool {
        self.persistence_pending()
            || self.learning.is_some()
            || matches!(self.mode, Mode::Learn | Mode::TaskReview)
    }
    pub fn command_choices(&self) -> Vec<super::commands::Choice> {
        use super::commands::Choice;
        if self.failed.is_some() {
            return vec![
                Choice::new("move-retry", "Retry the exact failed local move operation"),
                Choice::new(
                    "move-discard",
                    "Review discarding the unsaved move operation",
                ),
            ];
        }
        if self.mode == Mode::Name || self.persistence_pending() {
            return Vec::new();
        }
        let mut choices = vec![
            Choice::new("move", "Move menu; no action executed"),
            Choice::new("teach-move", "Teach from actual approved work"),
            Choice::new("move-reminder", "Forgotten and postponed moves"),
        ];
        if self.mode == Mode::Menu && self.active().next().is_some() {
            choices.push(Choice::new("move-details", "Inspect the highlighted move"));
        } else if self.mode == Mode::Detail && self.selected_record().is_some() {
            choices.extend([
                Choice::new("move-use", "Stage this move's normal review"),
                Choice::new("move-rename", "Rename this local label"),
                Choice::new("move-forget", "Review forgetting this move"),
                Choice::new("move-teach", "Teach a copy to a party member"),
            ]);
        }
        choices
    }
    pub fn mutation_pending(&self) -> bool {
        self.pending
            .iter()
            .chain(self.inflight.iter())
            .chain(self.failed.iter())
            .chain(self.queued.iter())
            .any(|request| {
                !matches!(
                    request.operation,
                    Operation::Load | Operation::Validate { .. }
                )
            })
    }
    pub fn queue(&mut self, request: Request) {
        if matches!(&request.operation, Operation::Load)
            && self.inflight.iter().chain(self.queued.iter()).any(|r| {
                r.task_id == request.task_id
                    && r.partner == request.partner
                    && r.form == request.form
                    && matches!(&r.operation, Operation::Load)
            })
        {
            return;
        }
        self.queued.push_back(request);
        self.pump();
    }
    fn pump(&mut self) {
        if self.inflight.is_none() && self.failed.is_none() {
            self.inflight = self.queued.pop_front();
            self.pending = self.inflight.clone();
        }
    }
    pub fn retry_failed(&mut self) -> bool {
        let Some(request) = self.failed.take() else {
            return false;
        };
        self.pending = Some(request.clone());
        self.inflight = Some(request);
        self.failure_discard = false;
        self.invalidate_review();
        true
    }
    pub fn discard_failed(&mut self) {
        self.failed = None;
        self.failed_error.clear();
        self.failure_discard = false;
        self.learning = None;
        self.enter(Mode::Menu);
        self.pump();
    }
    fn request(&mut self, operation: Operation) {
        self.queue(Request {
            task_id: self.task_id,
            partner: self.partner,
            form: self.form,
            operation,
        });
    }
    pub fn active(&self) -> impl Iterator<Item = &Record> + Clone {
        self.records.iter().filter(|r| {
            !r.forgotten && r.proposal.partner == self.partner && r.proposal.form == self.form
        })
    }
    pub fn reminders(&self) -> impl Iterator<Item = &Record> + Clone {
        self.records.iter().filter(|r| {
            r.forgotten && r.proposal.partner == self.partner && r.proposal.form == self.form
        })
    }
    pub fn postponed(&self) -> impl Iterator<Item = &Candidate> + Clone {
        self.candidates.iter().filter(|c| {
            c.postponed && c.proposal.partner == self.partner && c.proposal.form == self.form
        })
    }
    pub fn selected_record(&self) -> Option<&Record> {
        self.detail
            .and_then(|id| self.records.iter().find(|r| r.id == id))
    }
    pub fn last_usage(&self, id: i64) -> Option<&Usage> {
        self.usages.iter().rev().find(|usage| usage.move_id == id)
    }
    fn enter(&mut self, mode: Mode) {
        self.mode = mode;
        self.scroll = 0;
        self.scroll_max = 0;
        self.selected = 0;
        self.invalidate_review();
    }
    fn begin_learning(&mut self, learning: Learning) {
        self.name = learning.proposal().name.clone();
        self.cursor = self.name.len();
        self.scope = learning.proposal().scope;
        self.learning = Some(learning);
        self.replacement = None;
        self.scope_focus = false;
        self.insertion_rejected = false;
        self.enter(Mode::Name);
    }
    fn validate_name(&self) -> Result<(), String> {
        if self.insertion_rejected {
            return Err("The last insertion was refused; edit the name before continuing.".into());
        }
        moves::validate_name(&self.name)?;
        let except = match &self.learning {
            Some(Learning::Rename(r) | Learning::Relearn(r)) => Some(r.id),
            _ => None,
        };
        let key = crate::store::moves::name_key(&self.name);
        if self.records.iter().any(|r| {
            Some(r.id) != except
                && r.proposal.partner == self.partner
                && r.proposal.form == self.form
                && crate::store::moves::name_key(&r.proposal.name) == key
        }) {
            return Err(
                "This partner already has that name in this form. Choose a distinct name.".into(),
            );
        }
        Ok(())
    }
    pub fn paste(&mut self, text: &str) -> bool {
        if self.mode != Mode::Name || self.scope_focus || self.input_blocked() {
            return false;
        }
        if text
            .chars()
            .any(|ch| ch.is_control() || !super::safe_character(ch))
            || self.name.len().saturating_add(text.len()) > 256
        {
            self.insertion_rejected = true;
            self.notice = "Name insertion refused: use a single line within 256 UTF-8 bytes, without controls.".into();
            return true;
        }
        self.name.insert_str(self.cursor, text);
        self.cursor = self
            .name
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i >= self.cursor + text.len())
            .unwrap_or(self.name.len());
        self.insertion_rejected = false;
        self.notice.clear();
        self.invalidate_review();
        true
    }
    fn name_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Tab || key.code == KeyCode::BackTab {
            self.scope_focus = !self.scope_focus;
            return;
        }
        if self.scope_focus {
            if matches!(
                key.code,
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
            ) && matches!(self.learning, Some(Learning::Teach(_)))
            {
                self.scope = match self.scope {
                    Scope::Project => Scope::Personal,
                    Scope::Personal => Scope::Project,
                };
            }
            return;
        }
        let before = self.name[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i);
        let after = self
            .name
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i > self.cursor)
            .unwrap_or(self.name.len());
        match key.code {
            KeyCode::Left => self.cursor = before,
            KeyCode::Right => self.cursor = after,
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.name.len(),
            KeyCode::Backspace | KeyCode::Delete => {
                let start = if key.code == KeyCode::Backspace {
                    before
                } else {
                    self.cursor
                };
                let end = if key.code == KeyCode::Backspace {
                    self.cursor
                } else {
                    after
                };
                self.name.replace_range(start..end, "");
                self.cursor = self
                    .name
                    .grapheme_indices(true)
                    .map(|(i, _)| i)
                    .chain(std::iter::once(self.name.len()))
                    .take_while(|i| *i <= start)
                    .last()
                    .unwrap_or(0);
                self.insertion_rejected = false;
                self.notice.clear();
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.paste(ch.encode_utf8(&mut [0; 4]));
            }
            _ => {}
        }
    }
    fn finish_name(&mut self) -> Result<(), String> {
        self.validate_name()?;
        let rename = matches!(self.learning, Some(Learning::Rename(_)));
        if !rename && self.active().count() >= 4 {
            self.enter(Mode::Replace);
        } else {
            self.enter(Mode::Review);
        }
        Ok(())
    }
    fn commit_learning(&mut self) -> Result<(), String> {
        self.validate_name()?;
        let learning = self
            .learning
            .as_ref()
            .ok_or("No move selected for learning.")?;
        let operation = match learning {
            Learning::Rename(record) => Operation::Rename {
                id: record.id,
                revision: record.revision,
                name: self.name.clone(),
            },
            Learning::Relearn(record) => {
                moves::validate_proposal(&record.proposal)?;
                Operation::Relearn {
                    id: record.id,
                    revision: record.revision,
                    replace: self.replacement,
                }
            }
            Learning::Teach(source) => {
                let mut proposal = source.clone();
                proposal.name.clone_from(&self.name);
                proposal.scope = self.scope;
                moves::validate_proposal(&proposal)?;
                Operation::Teach {
                    proposal,
                    replace: self.replacement,
                }
            }
        };
        self.request(operation);
        self.invalidate_review();
        self.notice =
            "Saving the reviewed move change. No execution or approval is included.".into();
        Ok(())
    }
    fn candidate_decision(&mut self, decision: Decision) {
        if let Some(candidate) = self.offered.take() {
            self.request(Operation::CandidateDecision {
                key: candidate.key,
                decision,
            });
        }
        self.enter(Mode::Menu);
    }
}

impl App {
    pub fn open_moves(&mut self) {
        let owner = (self.task_id, self.lead.id, self.form());
        if owner.0 <= 0 {
            self.notice = "Save a task before teaching or using moves.".into();
            return;
        }
        if (self.moves.task_id, self.moves.partner, self.moves.form) != owner {
            self.moves.reset_view(owner.0, owner.1, owner.2);
        }
        if self.editing {
            self.retain_action_editor();
        }
        self.open_overlay(Overlay::Moves);
        if !self.moves.loaded {
            self.moves.request(Operation::Load);
        }
    }
    pub fn teach_move(&mut self) {
        if self.moves.persistence_pending() {
            self.notice = "Wait for move storage before teaching.".into();
            return;
        }
        match self.move_teaching_proposal() {
            Ok(mut proposal) => {
                self.open_moves();
                if self.overlay != Some(Overlay::Moves) {
                    return;
                }
                let used: Vec<String> = self
                    .moves
                    .records
                    .iter()
                    .map(|r| r.proposal.name.clone())
                    .collect();
                proposal.name = moves::suggested_name(proposal.form, &proposal.steps, &used);
                self.moves.begin_learning(Learning::Teach(proposal));
            }
            Err(error) => self.notice = error,
        }
    }
    pub fn move_command(&mut self, command: &str) -> bool {
        let command = command.trim_start_matches('/');
        if !matches!(
            command,
            "move"
                | "teach-move"
                | "move-reminder"
                | "move-use"
                | "move-details"
                | "move-rename"
                | "move-forget"
                | "move-teach"
                | "move-retry"
                | "move-discard"
        ) {
            return false;
        }
        if command == "teach-move" {
            self.teach_move();
            return true;
        }
        let was_menu = self.overlay == Some(Overlay::Moves) && self.moves.mode == Mode::Menu;
        self.open_moves();
        if self.overlay != Some(Overlay::Moves) {
            return true;
        }
        if command == "move-retry" {
            self.moves.retry_failed();
            return true;
        }
        if command == "move-discard" {
            if self.moves.failed.is_some() {
                self.moves.failure_discard = true;
                self.moves.invalidate_review();
            }
            return true;
        }
        if self.moves.failed.is_some() {
            return true;
        }
        if self.moves.persistence_pending() {
            self.moves.deferred = Some(command.to_owned());
            return true;
        }
        match command {
            "move" => self.moves.enter(Mode::Menu),
            "move-reminder" => self.moves.enter(Mode::Reminder),
            "move-use" | "move-details" | "move-rename" | "move-forget" | "move-teach" => {
                if command == "move-details" && was_menu {
                    self.move_menu_details();
                    return true;
                }
                if self.moves.selected_record().is_none() {
                    self.moves.enter(Mode::Menu);
                    self.moves.notice = "Choose a move: Enter opens its normal review; d opens Details for Rename, Forget, or Teach. No move was guessed.".into();
                    return true;
                }
                self.moves.enter(Mode::Detail);
                match command {
                    "move-use" => self.move_use_selected(),
                    "move-rename" => self.move_rename_selected(),
                    "move-forget" => self.moves.enter(Mode::Forget),
                    "move-teach" => self.moves.enter(Mode::TeachTarget),
                    _ => {}
                }
            }
            _ => {
                self.moves.notice =
                    "Choose a move, then Use, Rename, Forget, or Teach to a party member.".into()
            }
        }
        true
    }
    fn move_use_selected(&mut self) {
        let Some(record) = self.moves.selected_record().cloned() else {
            return;
        };
        if record.forgotten {
            self.moves.notice = "Relearn this move before using it.".into();
            return;
        }
        if matches!(&record.proposal.steps, Steps::Task { .. }) {
            self.moves.enter(Mode::TaskReview);
            return;
        }
        match moves::validate_proposal(&record.proposal).and_then(|_| self.stage_move_use(&record))
        {
            Ok(()) => {
                self.moves.notice = "Move staged in the normal review. It is not approved.".into()
            }
            Err(error) => self.moves.notice = error,
        }
    }
    fn move_rename_selected(&mut self) {
        if let Some(record) = self.moves.selected_record().cloned() {
            self.moves.begin_learning(Learning::Rename(record));
        }
    }
    pub fn move_paste(&mut self, value: &str) -> bool {
        self.moves.paste(value)
    }
    pub fn move_key(&mut self, key: KeyEvent) -> bool {
        if self.overlay != Some(Overlay::Moves) {
            return false;
        }
        if key.kind == KeyEventKind::Release {
            return true;
        }
        if key.code == KeyCode::Esc {
            if self.moves.failed.is_some() {
                self.close_overlay();
                return true;
            }
            if self.moves.mode == Mode::Learn {
                self.moves.candidate_decision(Decision::NotNow);
                self.close_overlay();
                return true;
            }
            if self.moves.persistence_pending() {
                if matches!(self.moves.mode, Mode::Name | Mode::LoadingTarget) {
                    self.moves.learning = None;
                    self.moves.enter(Mode::Menu);
                }
                self.close_overlay();
                return true;
            }
            match self.moves.mode {
                Mode::Menu => self.close_overlay(),
                _ => self.move_return_menu(),
            }
            return true;
        }
        if self.width < 60 || self.height < 18 {
            self.moves.invalidate_review();
            return true;
        }
        if key.kind == KeyEventKind::Repeat
            && ((self.moves.mode != Mode::Name
                && matches!(key.code, KeyCode::Enter | KeyCode::Char('y')))
                || (self.moves.failed.is_some()
                    && matches!(key.code, KeyCode::Char('r') | KeyCode::Char('d'))))
        {
            return true;
        }
        if self.moves.failed.is_some() {
            match key.code {
                KeyCode::Char('r') if !self.moves.failure_discard => {
                    self.moves.retry_failed();
                }
                KeyCode::Char('d') => {
                    self.moves.failure_discard = true;
                    self.moves.invalidate_review();
                }
                KeyCode::Tab | KeyCode::Left | KeyCode::Right
                    if self.moves.failure_discard && self.moves.reviewed =>
                {
                    self.moves.affirmative = !self.moves.affirmative
                }
                KeyCode::Char('y') if self.moves.failure_discard && self.moves.reviewed => {
                    self.moves.discard_failed();
                    self.move_return_menu();
                }
                KeyCode::Enter
                    if self.moves.failure_discard
                        && self.moves.reviewed
                        && self.moves.affirmative =>
                {
                    self.moves.discard_failed();
                    self.move_return_menu();
                }
                KeyCode::Enter => {
                    self.moves.failure_discard = false;
                    self.moves.invalidate_review();
                }
                KeyCode::PageDown => {
                    self.moves.scroll = self
                        .moves
                        .scroll
                        .saturating_add(self.moves.scroll_page.max(1))
                        .min(self.moves.scroll_max)
                }
                KeyCode::PageUp => {
                    self.moves.scroll = self
                        .moves
                        .scroll
                        .saturating_sub(self.moves.scroll_page.max(1))
                }
                KeyCode::Home => self.moves.scroll = 0,
                KeyCode::End => self.moves.scroll = self.moves.scroll_max,
                _ => {}
            }
            return true;
        }
        if self.moves.input_blocked() {
            return true;
        }
        if self.moves.mode == Mode::Name {
            if key.code == KeyCode::Enter {
                if let Err(error) = self.moves.finish_name() {
                    self.moves.notice = error;
                }
            } else {
                self.moves.name_key(key);
            }
            return true;
        }
        let page = self.moves.scroll_page.max(1);
        match key.code {
            KeyCode::PageDown => {
                self.moves.scroll = self
                    .moves
                    .scroll
                    .saturating_add(page)
                    .min(self.moves.scroll_max);
                return true;
            }
            KeyCode::PageUp => {
                self.moves.scroll = self.moves.scroll.saturating_sub(page);
                return true;
            }
            KeyCode::Home => {
                self.moves.scroll = 0;
                return true;
            }
            KeyCode::End => {
                self.moves.scroll = self.moves.scroll_max;
                return true;
            }
            _ => {}
        }
        let mode = self.moves.mode;
        if matches!(mode, Mode::Review | Mode::Forget | Mode::TaskReview) {
            match key.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab
                    if self.moves.reviewed =>
                {
                    self.moves.affirmative = !self.moves.affirmative
                }
                KeyCode::Char('y') if self.moves.reviewed => {
                    self.moves.affirmative = true;
                    self.move_confirm();
                }
                KeyCode::Enter if self.moves.reviewed && self.moves.affirmative => {
                    self.move_confirm()
                }
                KeyCode::Enter => self.move_return_menu(),
                _ => {}
            }
            return true;
        }
        let count = match mode {
            Mode::Menu => self.moves.active().count(),
            Mode::Detail => 4,
            Mode::Replace => self.moves.active().count() + 1,
            Mode::Reminder => self.moves.reminders().count() + self.moves.postponed().count(),
            Mode::Learn => 3,
            Mode::TeachTarget => self
                .partners()
                .iter()
                .filter(|p| p.id != self.lead.id)
                .count(),
            _ => 0,
        };
        match key.code {
            KeyCode::Up => {
                self.moves.selected =
                    self.moves
                        .selected
                        .saturating_sub(if mode == Mode::Menu && self.width >= 90 {
                            2
                        } else {
                            1
                        })
            }
            KeyCode::Down => {
                self.moves.selected = self
                    .moves
                    .selected
                    .saturating_add(if mode == Mode::Menu && self.width >= 90 {
                        2
                    } else {
                        1
                    })
                    .min(count.saturating_sub(1))
            }
            KeyCode::BackTab => self.moves.selected = self.moves.selected.saturating_sub(1),
            KeyCode::Tab => {
                self.moves.selected = (self.moves.selected + 1).min(count.saturating_sub(1))
            }
            KeyCode::Left if mode == Mode::Menu => {
                self.moves.selected = self.moves.selected.saturating_sub(1)
            }
            KeyCode::Right if mode == Mode::Menu => {
                self.moves.selected = (self.moves.selected + 1).min(count.saturating_sub(1))
            }
            KeyCode::Char('t') if mode == Mode::Menu => self.teach_move(),
            KeyCode::Char('r') if mode == Mode::Menu => self.moves.enter(Mode::Reminder),
            KeyCode::Char('d') if mode == Mode::Menu => self.move_menu_details(),
            KeyCode::Char('u') if mode == Mode::Detail => self.move_use_selected(),
            KeyCode::Char('r') if mode == Mode::Detail => self.move_rename_selected(),
            KeyCode::Char('f') if mode == Mode::Detail => self.moves.enter(Mode::Forget),
            KeyCode::Char('t') if mode == Mode::Detail => self.moves.enter(Mode::TeachTarget),
            KeyCode::Enter => self.move_activate(),
            _ => {}
        }
        true
    }
    fn move_return_menu(&mut self) {
        if (self.moves.task_id, self.moves.partner, self.moves.form)
            != (self.task_id, self.lead.id, self.form())
        {
            self.moves
                .reset_view(self.task_id, self.lead.id, self.form());
            self.moves.request(Operation::Load);
        } else {
            self.moves.learning = None;
            self.moves.replacement = None;
            self.moves.enter(Mode::Menu);
        }
    }
    fn move_confirm(&mut self) {
        if self.moves.mode == Mode::TaskReview {
            if let Some(record) = self.moves.selected_record().cloned() {
                match moves::validate_proposal(&record.proposal)
                    .and_then(|_| self.stage_move_use(&record))
                {
                    Ok(()) => self.moves.invalidate_review(),
                    Err(error) => self.moves.notice = error,
                }
            }
            return;
        }
        if self.moves.mode == Mode::Forget {
            if let Some(record) = self.moves.selected_record().cloned() {
                self.moves.request(Operation::Forget {
                    id: record.id,
                    revision: record.revision,
                });
                self.moves.invalidate_review();
            }
        } else if let Err(error) = self.moves.commit_learning() {
            self.moves.notice = error;
        }
    }
    fn move_menu_details(&mut self) {
        let detail = self.moves.active().nth(self.moves.selected).map(|r| r.id);
        self.moves.detail = detail;
        if detail.is_some() {
            self.moves.enter(Mode::Detail);
        }
    }
    fn move_activate(&mut self) {
        let selected = self.moves.selected;
        match self.moves.mode {
            Mode::Menu => {
                self.move_menu_details();
                if self.moves.detail.is_some() {
                    self.move_use_selected();
                }
            }
            Mode::Detail => match selected {
                0 => self.move_use_selected(),
                1 => self.move_rename_selected(),
                2 => self.moves.enter(Mode::Forget),
                3 => self.moves.enter(Mode::TeachTarget),
                _ => {}
            },
            Mode::Replace => {
                let replacement = selected.checked_sub(1).and_then(|index| {
                    self.moves
                        .active()
                        .nth(index)
                        .map(|record| (record.id, record.revision))
                });
                if selected == 0 {
                    self.move_return_menu();
                } else if let Some(replacement) = replacement {
                    self.moves.replacement = Some(replacement);
                    self.moves.enter(Mode::Review);
                }
            }
            Mode::Reminder => {
                let forgotten_count = self.moves.reminders().count();
                let selected_record = self.moves.reminders().nth(selected).cloned();
                if let Some(record) = selected_record {
                    if let Err(error) = moves::validate_proposal(&record.proposal) {
                        self.moves.notice = error;
                        return;
                    }
                    // Relearn preserves the stored steps, name and scope exactly.
                    self.moves.name = record.proposal.name.clone();
                    self.moves.scope = record.proposal.scope;
                    self.moves.learning = Some(Learning::Relearn(record));
                    self.moves.replacement = None;
                    self.moves.enter(if self.moves.active().count() >= 4 {
                        Mode::Replace
                    } else {
                        Mode::Review
                    });
                } else {
                    let offset = selected.saturating_sub(forgotten_count);
                    let candidate = self.moves.postponed().nth(offset).cloned();
                    if let Some(candidate) = candidate {
                        self.moves.offered = Some(candidate);
                        self.moves.enter(Mode::Learn);
                    }
                }
            }
            Mode::Learn => match selected {
                0 => self.moves.candidate_decision(Decision::NotNow),
                1 => {
                    if let Some(candidate) = self.moves.offered.clone() {
                        let mut proposal = candidate.proposal;
                        let used = self
                            .moves
                            .records
                            .iter()
                            .map(|r| r.proposal.name.clone())
                            .collect::<Vec<_>>();
                        proposal.name =
                            moves::suggested_name(proposal.form, &proposal.steps, &used);
                        self.moves.begin_learning(Learning::Teach(proposal));
                    }
                }
                2 => self.moves.candidate_decision(Decision::Never),
                _ => {}
            },
            Mode::TeachTarget => {
                let targets: Vec<_> = self
                    .partners()
                    .into_iter()
                    .filter(|p| p.id != self.lead.id)
                    .collect();
                let Some(target) = targets.get(selected) else {
                    return;
                };
                let Some(record) = self.moves.selected_record().cloned() else {
                    return;
                };
                let mut proposal = record.proposal;
                proposal.partner = target.id;
                proposal.provenance.taught_by = Some(self.lead.id);
                proposal.provenance.teacher_name =
                    Some(crate::store::Attribution::of(&self.lead).name);
                if let Err(error) = moves::validate_proposal(&proposal) {
                    self.moves.notice = error;
                    return;
                }
                self.moves.learning = Some(Learning::Teach(proposal));
                self.moves.partner = target.id;
                self.moves.loaded = false;
                self.moves.enter(Mode::LoadingTarget);
                self.moves.request(Operation::Load);
            }
            _ => {}
        }
    }
    pub fn move_store_event(
        &mut self,
        task: i64,
        partner: i64,
        form: usize,
        result: Result<Response, String>,
    ) {
        let matched = self
            .moves
            .inflight
            .as_ref()
            .is_some_and(|r| (r.task_id, r.partner, r.form) == (task, partner, form));
        if !matched {
            return;
        }
        let request = self.moves.inflight.take().unwrap();
        self.moves.pending = None;
        let load = matches!(request.operation, Operation::Load);
        let mutation = matches!(
            request.operation,
            Operation::Teach { .. }
                | Operation::Rename { .. }
                | Operation::Forget { .. }
                | Operation::Relearn { .. }
        );
        if let Err(error) = &result {
            self.moves.failed = Some(request);
            self.moves.failed_error.clone_from(error);
            self.moves.notice = format!(
                "Move storage failed: {error}. Retry retains the exact request; discard requires review."
            );
            self.notice = format!(
                "Move storage for task #{task}, partner #{partner}: {error}. Open /move to retry or review discard."
            );
            self.moves.invalidate_review();
            return;
        }
        self.moves.failed_error.clear();
        let visible = (self.moves.task_id, self.moves.partner, self.moves.form)
            == (task, partner, form)
            && task == self.task_id;
        if visible {
            match result {
                Ok(response) => {
                    self.moves.records = response.records;
                    self.moves.candidates = response.candidates;
                    self.moves.usages = response.usages;
                    self.moves.loaded = true;
                    if mutation {
                        self.moves.offered = None;
                        self.move_return_menu();
                        self.moves.notice =
                            "Saved. Move use still requires its normal review and approval.".into();
                    } else if load && self.moves.mode == Mode::LoadingTarget {
                        if let Some(mut learning) = self.moves.learning.take() {
                            let used = self
                                .moves
                                .records
                                .iter()
                                .map(|r| r.proposal.name.clone())
                                .collect::<Vec<_>>();
                            if let Learning::Teach(proposal) = &mut learning
                                && self.moves.records.iter().any(|r| {
                                    crate::store::moves::name_key(&r.proposal.name)
                                        == crate::store::moves::name_key(&proposal.name)
                                })
                            {
                                proposal.name =
                                    moves::suggested_name(proposal.form, &proposal.steps, &used);
                            }
                            self.moves.begin_learning(learning);
                        }
                    } else if load && self.moves.mode == Mode::Name {
                        // Teaching may begin while the initial owner snapshot is loading.
                        let used = self
                            .moves
                            .records
                            .iter()
                            .map(|r| r.proposal.name.clone())
                            .collect::<Vec<_>>();
                        if self.moves.records.iter().any(|r| {
                            crate::store::moves::name_key(&r.proposal.name)
                                == crate::store::moves::name_key(&self.moves.name)
                        }) && let Some(learning) = &self.moves.learning
                        {
                            self.moves.name =
                                moves::suggested_name(form, &learning.proposal().steps, &used);
                            self.moves.cursor = self.moves.name.len();
                        }
                    }
                }
                Err(_) => {
                    unreachable!("store failures are retained before applying the visible response")
                }
            }
        }
        self.moves.pump();
        if !self.moves.persistence_pending()
            && let Some(command) = self.moves.deferred.take()
        {
            self.move_command(&command);
        }
    }
    /// Parent also checks all request/run/save/preferences ownership before calling.
    pub fn move_idle_offer(&mut self) -> bool {
        if self.overlay.is_some()
            || self.confirmation.is_some()
            || self.transitioning
            || (self.editing && !(self.composer_is_request() && self.input.is_empty()))
            || self.draft_dirty()
            || self.planning_any_busy()
            || self.pending_request.is_some()
            || self.pending_command.is_some()
            || self.storage != StorageState::Saved
            || self.execution_any_busy()
            || self.execution_any_pending()
            || self.sources.source_busy
            || self.sources.store_busy
            || self.sources.pending.is_some()
            || self.sources.pending_store.is_some()
            || self.memory.busy()
            || self.memory_loading
            || self.memory_request.is_some()
            || self.command_popup.is_some()
            || self.composer_menu.is_some()
            || self.nickname_editor.is_some()
            || self.save_requested
            || self.save_inflight.is_some()
            || self.images_busy
            || self.moves.busy()
            || !self.moves.loaded
            || (self.moves.task_id, self.moves.partner, self.moves.form)
                != (self.task_id, self.lead.id, self.form())
        {
            return false;
        }
        let candidate = self
            .moves
            .candidates
            .iter()
            .find(|c| {
                !c.postponed
                    && !self
                        .moves
                        .displayed
                        .contains(&format!("{}:{}", c.key, c.uses))
            })
            .cloned();
        if let Some(candidate) = candidate {
            self.moves
                .displayed
                .insert(format!("{}:{}", candidate.key, candidate.uses));
            let key = candidate.key.clone();
            self.moves.offered = Some(candidate);
            if self.editing {
                self.retain_action_editor();
            }
            self.open_overlay(Overlay::Moves);
            self.moves.enter(Mode::Learn);
            self.moves.request(Operation::CandidateDecision {
                key,
                decision: Decision::Offered,
            });
            return true;
        }
        false
    }
}

pub fn kind(steps: &Steps) -> &'static str {
    match steps {
        Steps::Run { .. } => "run",
        Steps::Pins { .. } => "pins",
        Steps::Request { .. } => "request",
        Steps::Task { .. } => "task template",
    }
}
pub fn outcome(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Pending => "pending",
        Outcome::Completed => "completed (not a correctness verdict)",
        Outcome::Succeeded => "completed (not a correctness verdict)",
        Outcome::Failed => "failed",
        Outcome::Denied => "denied",
        Outcome::Cancelled => "cancelled",
        Outcome::Unknown => "unknown",
    }
}
pub fn usage_outcome(usage: &Usage) -> &'static str {
    if usage.run_id.is_some() && usage.outcome == Outcome::Succeeded {
        "exited 0"
    } else {
        outcome(&usage.outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::moves::{Pin, Provenance};
    fn proposal(name: &str) -> Proposal {
        Proposal {
            partner: 1,
            form: 1,
            name: name.into(),
            scope: Scope::Project,
            steps: Steps::Pins {
                pins: vec![Pin {
                    path: "src/lib.rs".into(),
                    start_line: 1,
                    end_line: 3,
                }],
            },
            provenance: Provenance {
                task_id: 1,
                sources: vec!["request:1".into()],
                taught_by: None,
                teacher_name: None,
            },
        }
    }
    fn record(id: i64, name: &str) -> Record {
        Record {
            id,
            revision: 1,
            project: "project".into(),
            proposal: proposal(name),
            forgotten: false,
            created: 1,
        }
    }
    fn state() -> State {
        State {
            task_id: 1,
            partner: 1,
            form: 1,
            loaded: true,
            ..State::default()
        }
    }
    #[test]
    fn opening_and_naming_have_no_store_mutation() {
        let mut state = state();
        state.begin_learning(Learning::Teach(proposal("Surf")));
        state.finish_name().unwrap();
        assert_eq!(state.mode, Mode::Review);
        assert!(!state.affirmative);
        assert!(!state.reviewed);
        assert!(state.pending.is_none());
    }
    #[test]
    fn fifth_move_defaults_to_give_up_and_replacement_is_exact() {
        let mut state = state();
        state.records = (1..=4)
            .map(|id| record(id, &format!("Surf {id}")))
            .collect();
        state.begin_learning(Learning::Teach(proposal("Water Pulse")));
        state.finish_name().unwrap();
        assert_eq!(state.mode, Mode::Replace);
        assert_eq!(state.selected, 0);
        assert!(state.pending.is_none());
        state.replacement = Some((3, 1));
        state.commit_learning().unwrap();
        assert!(matches!(
            state.pending.unwrap().operation,
            Operation::Teach {
                replace: Some((3, 1)),
                ..
            }
        ));
    }
    #[test]
    fn cjk_and_combining_graphemes_delete_as_units() {
        let mut state = state();
        state.begin_learning(Learning::Teach(proposal("水e\u{301}")));
        state.name_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(state.name, "水");
        assert_eq!(state.cursor, "水".len());
        assert!(state.paste("の波"));
        assert_eq!(state.name, "水の波");
        assert!(!state.paste("\n") || state.insertion_rejected);
    }
    #[test]
    fn duplicate_unicode_name_refused_before_any_request() {
        let mut state = state();
        state.records.push(record(1, "Straße"));
        state.begin_learning(Learning::Teach(proposal("STRASSE")));
        assert!(state.finish_name().is_err());
        assert!(state.pending.is_none());
    }
    #[test]
    fn queue_retains_each_request_and_deduplicates_loads() {
        let mut state = state();
        state.request(Operation::Load);
        state.request(Operation::Load);
        assert!(state.queued.is_empty());
        state.request(Operation::CandidateDecision {
            key: "candidate".into(),
            decision: Decision::NotNow,
        });
        assert_eq!(state.queued.len(), 1);
        assert!(matches!(
            state.pending.as_ref().unwrap().operation,
            Operation::Load
        ));
    }
    #[test]
    fn postpone_uses_durable_store_decision_not_learning() {
        let mut state = state();
        state.offered = Some(Candidate {
            key: "candidate".into(),
            proposal: proposal("Surf"),
            uses: 3,
            sources: vec!["a".into(), "b".into(), "c".into()],
            postponed: false,
        });
        state.candidate_decision(Decision::NotNow);
        assert!(matches!(
            state.pending.unwrap().operation,
            Operation::CandidateDecision {
                decision: Decision::NotNow,
                ..
            }
        ));
        assert!(state.learning.is_none());
    }
    #[test]
    fn relearn_preserves_steps_and_revalidates() {
        let mut state = state();
        let mut original = record(1, "Surf");
        original.forgotten = true;
        state.name = original.proposal.name.clone();
        state.learning = Some(Learning::Relearn(original));
        state.commit_learning().unwrap();
        assert!(matches!(
            state.pending.unwrap().operation,
            Operation::Relearn {
                id: 1,
                revision: 1,
                replace: None
            }
        ));
    }
    fn app() -> App {
        let mut app = App::new("project".into(), 120, 36);
        app.task_id = 1;
        app.lead.id = 1;
        app.set_form(1);
        app.moves = state();
        app
    }
    #[test]
    fn reset_view_keeps_transport_for_original_owner() {
        let mut state = state();
        state.request(Operation::Forget { id: 7, revision: 2 });
        state.request(Operation::CandidateDecision {
            key: "candidate".into(),
            decision: Decision::NotNow,
        });
        state.reset_view(8, 4, 2);
        assert_eq!((state.task_id, state.partner, state.form), (8, 4, 2));
        assert_eq!(state.inflight.as_ref().unwrap().task_id, 1);
        assert_eq!(state.queued.len(), 1);
        assert!(state.pending.is_some());
    }
    #[test]
    fn stale_owner_event_finishes_transport_without_replacing_visible_records() {
        let mut app = app();
        app.moves.request(Operation::Load);
        app.moves.reset_view(1, 2, 1);
        app.moves.records.push(record(9, "Current target"));
        app.move_store_event(
            1,
            1,
            1,
            Ok(Response {
                records: vec![record(1, "Old owner")],
                candidates: vec![],
                usages: vec![],
            }),
        );
        assert_eq!(app.moves.records[0].id, 9);
        assert!(!app.moves.persistence_pending());
    }
    #[test]
    fn sharing_loads_actual_target_and_uses_full_slot_review() {
        let mut app = app();
        app.open_moves();
        app.party = vec![
            app.lead.clone(),
            crate::store::Partner {
                id: 2,
                nickname: Some("Mizu".into()),
                ..crate::store::Partner::default()
            },
        ];
        let original = record(1, "Surf");
        app.moves.records.push(original.clone());
        app.moves.detail = Some(1);
        app.moves.mode = Mode::TeachTarget;
        app.move_activate();
        assert_eq!(app.moves.partner, 2);
        assert_eq!(app.moves.mode, Mode::LoadingTarget);
        assert!(matches!(
            app.moves.pending.as_ref().unwrap().operation,
            Operation::Load
        ));
        let records = (2..=5)
            .map(|id| {
                let mut record = record(id, &format!("Wave {id}"));
                record.proposal.partner = 2;
                record
            })
            .collect();
        app.move_store_event(
            1,
            2,
            1,
            Ok(Response {
                records,
                candidates: vec![],
                usages: vec![],
            }),
        );
        assert_eq!(app.moves.mode, Mode::Name);
        app.moves.finish_name().unwrap();
        assert_eq!(app.moves.mode, Mode::Replace);
        assert_eq!(app.moves.selected, 0);
        assert!(app.moves.pending.is_none());
        app.moves.selected = 2;
        app.move_activate();
        assert_eq!(app.moves.mode, Mode::Review);
        assert!(!app.moves.affirmative);
        app.moves.reviewed = true;
        app.move_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        let Operation::Teach { proposal, replace } = &app.moves.pending.as_ref().unwrap().operation
        else {
            panic!("expected reviewed copy");
        };
        assert_eq!(proposal.partner, 2);
        assert_eq!(proposal.steps, original.proposal.steps);
        assert_eq!(proposal.provenance.taught_by, Some(1));
        assert_eq!(*replace, Some((3, 1)));
        assert_eq!(original.proposal.partner, 1);
    }
    #[test]
    fn give_up_target_learning_keeps_all_slots_and_returns_lead() {
        let mut app = app();
        app.moves.partner = 2;
        app.moves.learning = Some(Learning::Teach(proposal("Surf")));
        app.moves.mode = Mode::Replace;
        app.move_activate();
        assert!(app.moves.learning.is_none());
        assert_eq!(app.moves.partner, 1);
        assert!(matches!(
            app.moves.pending.as_ref().unwrap().operation,
            Operation::Load
        ));
    }
    #[test]
    fn neutral_task_template_enter_does_not_change_task() {
        let mut app = app();
        app.goal = "Keep this task".into();
        let mut template = record(1, "Helping Hand");
        template.proposal.form = 0;
        template.proposal.steps = Steps::Task {
            goal: "Replacement".into(),
            criteria: vec!["Unverified criterion".into()],
        };
        app.moves.records.push(template);
        app.moves.detail = Some(1);
        app.overlay = Some(Overlay::Moves);
        app.move_use_selected();
        assert_eq!(app.moves.mode, Mode::TaskReview);
        app.move_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.goal, "Keep this task");
        assert!(app.moves.pending.is_none());
    }
    #[test]
    fn below_minimum_never_confirms_a_previously_reviewed_change() {
        let mut app = app();
        app.overlay = Some(Overlay::Moves);
        app.moves.begin_learning(Learning::Teach(proposal("Surf")));
        app.moves.finish_name().unwrap();
        app.moves.reviewed = true;
        app.moves.affirmative = true;
        app.width = 40;
        app.height = 10;
        app.move_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.moves.pending.is_none());
        assert!(!app.moves.reviewed);
        assert!(!app.moves.affirmative);
    }
    #[test]
    fn escape_prompt_postpones_even_while_offer_acknowledgment_is_pending() {
        let mut app = app();
        app.overlay = Some(Overlay::Moves);
        app.moves.mode = Mode::Learn;
        app.moves.offered = Some(Candidate {
            key: "candidate".into(),
            proposal: proposal("Surf"),
            uses: 3,
            sources: vec![],
            postponed: false,
        });
        app.moves.request(Operation::CandidateDecision {
            key: "candidate".into(),
            decision: Decision::Offered,
        });
        app.move_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(
            app.moves.queued.front().unwrap().operation,
            Operation::CandidateDecision {
                decision: Decision::NotNow,
                ..
            }
        ));
        assert!(app.moves.offered.is_none());
    }
    #[test]
    fn learning_choices_remain_responsive_while_offer_is_persisting() {
        for choice in 0..3 {
            let mut app = app();
            app.overlay = Some(Overlay::Moves);
            app.moves.mode = Mode::Learn;
            app.moves.offered = Some(Candidate {
                key: "candidate".into(),
                proposal: proposal("Surf"),
                uses: 3,
                sources: vec!["a".into(), "b".into(), "c".into()],
                postponed: false,
            });
            app.moves.request(Operation::CandidateDecision {
                key: "candidate".into(),
                decision: Decision::Offered,
            });
            for _ in 0..choice {
                app.move_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
            }
            app.move_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            if choice == 1 {
                assert_eq!(app.moves.mode, Mode::Name);
                assert_eq!(
                    app.moves.learning.as_ref().unwrap().proposal().steps,
                    proposal("Surf").steps
                );
                assert!(app.moves.queued.is_empty());
                app.move_key(KeyEvent::new(KeyCode::Char('水'), KeyModifiers::NONE));
                assert!(app.move_paste("の手順"));
                assert_eq!(app.moves.name, "Surf水の手順");
            } else {
                assert_eq!(app.moves.mode, Mode::Menu);
                assert!(app.moves.learning.is_none());
                assert!(matches!(&app.moves.queued.front().unwrap().operation,
                    Operation::CandidateDecision {key,decision}
                    if key == "candidate" && *decision == if choice == 0 {Decision::NotNow} else {Decision::Never}));
            }
            assert!(app.moves.records.is_empty());
        }
    }
    #[test]
    fn opening_and_closing_restores_exact_composer_at_both_sizes() {
        for (width, height) in [(120, 36), (60, 18)] {
            let mut app = app();
            app.width = width;
            app.height = height;
            app.editor_field = crate::store::EditorField::Request;
            app.editing = true;
            app.input = "Keep 水の draft".into();
            app.cursor = "Keep 水".len();
            app.focus = super::super::Pane::Workspace;
            app.open_moves();
            assert_eq!(app.overlay, Some(Overlay::Moves));
            assert!(app.moves.pending.is_none());
            app.move_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            assert!(app.editing);
            assert_eq!(app.input, "Keep 水の draft");
            assert_eq!(app.cursor, "Keep 水".len());
            assert_eq!(app.editor_field, crate::store::EditorField::Request);
            assert_eq!(app.focus, super::super::Pane::Workspace);
        }
    }
    #[test]
    fn failed_use_receipt_retains_exact_bytes_and_owner_until_explicit_retry() {
        let mut app = app();
        let request = Request {
            task_id: 1,
            partner: 1,
            form: 1,
            operation: Operation::Use {
                usage: Usage {
                    id: "use:owned".into(),
                    move_id: 1,
                    task_id: 1,
                    partner: 1,
                    run_id: None,
                    request_id: Some("request:owned".into()),
                    outcome: Outcome::Completed,
                    revision: None,
                },
            },
        };
        app.moves.queue(request.clone());
        app.moves.queue(Request {
            task_id: 1,
            partner: 1,
            form: 1,
            operation: Operation::Load,
        });
        app.move_store_event(1, 1, 1, Err("Storage quota reached".into()));
        assert_eq!(app.moves.failed.as_ref(), Some(&request));
        assert!(app.moves.pending.is_none());
        assert_eq!(app.moves.queued.len(), 1);
        app.moves.reset_view(9, 3, 2);
        assert_eq!(app.moves.failed.as_ref(), Some(&request));
        assert!(app.moves.retry_failed());
        assert_eq!(app.moves.pending.as_ref(), Some(&request));
        assert_eq!(app.moves.inflight.as_ref(), Some(&request));
    }
    #[test]
    fn direct_use_without_deliberate_selection_opens_menu_without_staging() {
        let mut app = app();
        app.moves.records.push(record(1, "Surf"));
        assert!(app.move_command("move-use"));
        assert_eq!(app.moves.mode, Mode::Menu);
        assert!(app.moves.detail.is_none());
        assert!(app.moves.pending.is_none());
        assert!(app.moves.notice.contains("No move was guessed"));
    }
    #[test]
    fn menu_enter_opens_task_review_directly_without_applying_or_executing() {
        let mut app = app();
        app.overlay = Some(Overlay::Moves);
        app.goal = "Original task".into();
        let mut template = record(1, "Helping Hand");
        template.proposal.form = 0;
        template.proposal.steps = Steps::Task {
            goal: "Proposed task".into(),
            criteria: vec!["Proposed criterion".into()],
        };
        app.moves.form = 0;
        app.moves.records.push(template);
        app.move_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.moves.mode, Mode::TaskReview);
        assert_eq!(app.moves.detail, Some(1));
        assert_eq!(app.goal, "Original task");
        assert!(!app.moves.affirmative);
        assert!(app.moves.pending.is_none());
        assert!(app.execution.active.is_none());
        assert!(app.execution.prepared.is_none());
        assert!(app.planning.pending.is_none());
    }
    #[test]
    fn explicit_details_uses_chosen_row_without_staging_use() {
        for direct_command in [false, true] {
            let mut app = app();
            app.overlay = Some(Overlay::Moves);
            app.moves.records = vec![record(1, "Surf"), record(2, "Water Pulse")];
            app.moves.selected = 1;
            if direct_command {
                assert!(app.move_command("move-details"));
            } else {
                app.move_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
            }
            assert_eq!(app.moves.mode, Mode::Detail);
            assert_eq!(app.moves.detail, Some(2));
            assert!(app.moves.pending.is_none());
            assert!(app.execution.active.is_none());
            assert!(app.execution.prepared.is_none());
            assert!(app.planning.pending.is_none());
        }
    }
}
