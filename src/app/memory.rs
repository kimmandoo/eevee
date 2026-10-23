//! Local memory review; only the store worker may commit pending mutations.
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::workspace::memory::{self, Mutation, Proposal, Provenance, Record, Scope, SourceKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Browse,
    Edit,
    Preview,
    Delete,
    Discard,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    version: u8,
    task_id: i64,
    project: String,
    selected_ids: Vec<i64>,
    selected: usize,
    scroll: u16,
    mode: Mode,
    candidate: Option<Proposal>,
    target: Option<(i64, u64)>,
    text: String,
    original: String,
    cursor: usize,
    #[serde(default)]
    habit_sources: Vec<String>,
}

pub struct State {
    pub records: Vec<Record>,
    pub selected_ids: Vec<i64>,
    pub selected: usize,
    pub pending: Option<Mutation>,
    pub mode: Mode,
    pub candidate: Option<Proposal>,
    pub draft: String,
    pub cursor: usize,
    pub notice: String,
    pub saving: bool,
    pub reviewed: bool,
    pub affirmative: bool,
    pub show_help: bool,
    pub scroll: u16,
    pub scroll_max: u16,
    pub task_id: i64,
    pub project: String,
    target: Option<(i64, u64)>,
    original: String,
    habit_binding: Option<(String, i64, u64)>,
    pub habit_sources: Vec<String>,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            records: Vec::new(), selected_ids: Vec::new(), selected: 0,
            pending: None, mode: Mode::Browse, candidate: None, draft: String::new(),
            cursor: 0, notice: "Only explicitly approved notes are remembered. /select-memory includes a memory in future request previews.".into(),
            saving: false, reviewed: false, affirmative: false, scroll: 0,
            scroll_max: 0, task_id: 0, project: String::new(), target: None,
            original: String::new(), show_help: false,
            habit_binding: None,
            habit_sources: Vec::new(),
        }
    }

    pub fn reset(&mut self, task_id: i64, project: impl Into<String>) {
        *self = Self::new();
        self.task_id = task_id;
        self.project = project.into();
    }

    pub fn busy(&self) -> bool {
        self.saving || self.pending.is_some() || self.mode != Mode::Browse
    }

    pub fn editing(&self) -> bool {
        self.mode == Mode::Edit
    }

    pub fn invalidate_review(&mut self) {
        self.reviewed = false;
        self.affirmative = false;
    }

    pub fn selected_record(&self) -> Option<&Record> {
        self.records.get(self.selected)
    }

    pub fn selected_snapshot(&self) -> Result<Vec<Record>, String> {
        memory::snapshot(
            &self.records,
            &self.selected_ids,
            &self.project,
            self.task_id,
        )
    }

    /// Caller supplies only a deliberately selected user-authored/verified note,
    /// and must check source exclusions before exposing this Remember action.
    pub fn propose(
        &mut self,
        content: &str,
        provenance: Provenance,
        excluded: bool,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("Finish or reject the current memory draft first.".into());
        }
        if excluded {
            self.notice = "Excluded source content cannot be proposed as memory.".into();
            return Err(self.notice.clone());
        }
        if provenance.task_id != self.task_id {
            return Err("The selected note belongs to a different task.".into());
        }
        memory::validate_content(content)?;
        let proposal = Proposal {
            scope: Scope::Task,
            content: content.into(),
            provenance,
        };
        if let Err(error) = memory::validate_proposal(&proposal) {
            self.notice = error.clone();
            return Err(error);
        }
        self.draft = proposal.content.clone();
        self.original = self.draft.clone();
        self.cursor = self.draft.len();
        self.candidate = Some(proposal);
        self.target = None;
        self.mode = Mode::Preview;
        self.scroll = 0;
        self.invalidate_review();
        self.notice = "Review this local note and its scope. y Approve / n Back; /memory-scope changes scope; /verbose shows details.".into();
        Ok(())
    }

    /// Uses the existing S02 review/approval gate; no record or binding exists
    /// until the worker acknowledges the exact approved project-memory proposal.
    pub fn propose_habit(
        &mut self,
        habit: &crate::workspace::records::Habit,
    ) -> Result<(), String> {
        if !super::habits::eligible(habit) || habit.memory.is_some() {
            return Err(
                "This instruction is not an available repeated-instruction proposal.".into(),
            );
        }
        if self.selected_ids.len() >= memory::MAX_SELECTED {
            return Err(
                "Deselect one memory before approving and selecting this instruction.".into(),
            );
        }
        self.propose(
            &habit.instruction,
            Provenance {
                kind: SourceKind::UserAuthored,
                reference: format!("habit:{}", habit.key),
                task_id: self.task_id,
            },
            false,
        )?;
        self.candidate.as_mut().expect("proposed habit").scope = Scope::Project;
        self.habit_sources = habit
            .sources
            .iter()
            .take(3)
            .map(|source| format!("{} (task #{})", source.request_id, source.task_id))
            .collect();
        self.notice = self.habit_disclosure();
        Ok(())
    }

    /// The parent queues BindInstructionMemory through the sole records worker.
    pub fn take_habit_binding(&mut self) -> Option<(String, i64, u64)> {
        self.habit_binding.take()
    }

    pub fn habit_disclosure(&self) -> String {
        format!(
            "Review repeated authored instruction; submitted request sources: {}. Approval saves and selects a project memory, binds this exact instruction, and substitutes that memory reference for an exact repeat in a future current instruction. Drafts and past/provider history remain unchanged. Editing, deleting or deselecting the memory disables substitution. Explicit approval is required.",
            self.habit_sources.join(", "),
        )
    }

    pub fn receive_records(&mut self, records: Vec<Record>) {
        if records.len() > memory::MAX_RECORDS
            || records
                .iter()
                .any(|record| memory::validate_record(record).is_err())
        {
            self.notice = "Stored memories exceed their bounds or contain an invalid record; no context was replaced.".into();
            return;
        }
        self.invalidate_review();
        self.records = records
            .into_iter()
            .filter(|record| memory::visible(record, &self.project, self.task_id))
            .collect();
        self.selected = self.selected.min(self.records.len().saturating_sub(1));
    }

    /// Call only for the matching task's worker acknowledgement. A failed save
    /// keeps exact editable text and requires a fresh explicit review/approval.
    pub fn finish(&mut self, result: Result<Vec<Record>, String>) {
        if !self.saving {
            match result {
                Ok(records) => self.receive_records(records),
                Err(error) => {
                    self.notice = format!(
                        "Memory load failed: {}. Draft retained.",
                        crate::workspace::handoff::sanitize(&error)
                    )
                }
            }
            return;
        }
        let habit_proposal = (self.mode == Mode::Preview && self.target.is_none())
            .then(|| self.candidate.clone())
            .flatten()
            .filter(|proposal| {
                proposal.scope == Scope::Project
                    && proposal.provenance.reference.starts_with("habit:")
                    && proposal.content == self.original
                    && proposal.content == self.draft
            });
        self.saving = false;
        self.pending = None;
        self.invalidate_review();
        match result {
            Ok(records) => {
                self.receive_records(records);
                self.selected_ids
                    .retain(|id| self.records.iter().any(|record| record.id == *id));
                if let Some(proposal) = habit_proposal
                    && let Some(record) = self.records.iter().find(|record| {
                        record.project == self.project && record.proposal == proposal
                    })
                    && (self.selected_ids.contains(&record.id)
                        || self.selected_ids.len() < memory::MAX_SELECTED)
                {
                    if !self.selected_ids.contains(&record.id) {
                        self.selected_ids.push(record.id);
                    }
                    self.habit_binding = Some((
                        proposal.provenance.reference["habit:".len()..].into(),
                        record.id,
                        record.revision,
                    ));
                }
                self.clear_draft();
                self.notice = "Memory change committed. Context uses only /select-memory selections of approved memories; prior requests/exports are unchanged.".into();
            }
            Err(error) => {
                self.notice = format!(
                    "Save failed: {}. Draft retained; review then retry.",
                    crate::workspace::handoff::sanitize(&error)
                );
            }
        }
    }

    pub fn draft_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "version": 1, "task_id": self.task_id, "project": self.project,
            "selected_ids": self.selected_ids, "selected": self.selected,
            "scroll": self.scroll, "mode": self.mode, "candidate": self.candidate,
            "target": self.target, "text": self.draft, "original": self.original,
            "cursor": self.cursor,
            "habit_sources": self.habit_sources,
        })
    }

    pub fn restore_draft(&mut self, value: &serde_json::Value) -> Result<(), String> {
        if value.is_null() {
            return Ok(());
        }
        let draft: Draft = serde_json::from_value(value.clone())
            .map_err(|_| "Invalid memory draft; nothing was restored.")?;
        if draft.version != 1 || draft.task_id != self.task_id || draft.project != self.project {
            return Err("Memory draft belongs to a different task/project or schema.".into());
        }
        if draft.habit_sources.len() > 3 {
            return Err("Habit draft has too many source references.".into());
        }
        for source in &draft.habit_sources {
            memory::validate_text(source, memory::MAX_REFERENCE_BYTES)?;
        }
        if draft.selected_ids.len() > memory::MAX_SELECTED
            || draft
                .selected_ids
                .iter()
                .enumerate()
                .any(|(index, id)| *id <= 0 || draft.selected_ids[..index].contains(id))
        {
            return Err("Memory draft has an invalid context selection.".into());
        }
        memory::validate_content(&draft.text)?;
        memory::validate_content(&draft.original)?;
        if let Some(candidate) = &draft.candidate {
            memory::validate_content(&candidate.content)?;
            memory::validate_text(&candidate.provenance.reference, memory::MAX_REFERENCE_BYTES)?;
            if candidate.provenance.task_id <= 0 || candidate.provenance.reference.trim().is_empty()
            {
                return Err("Memory draft is missing source provenance.".into());
            }
        } else if !matches!(draft.mode, Mode::Browse | Mode::Delete) {
            return Err("Memory draft is missing its proposal.".into());
        }
        if draft
            .target
            .is_some_and(|(id, revision)| id <= 0 || revision == 0)
        {
            return Err("Memory draft has an invalid edit identity.".into());
        }
        self.selected_ids = draft.selected_ids;
        self.selected = draft.selected;
        self.scroll = draft.scroll;
        self.mode = match draft.mode {
            Mode::Delete => Mode::Browse,
            Mode::Discard => Mode::Edit,
            other => other,
        };
        self.candidate = draft.candidate;
        self.target = draft.target;
        self.draft = draft.text;
        self.original = draft.original;
        self.habit_sources = draft.habit_sources;
        self.cursor = self
            .draft
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(self.draft.len()))
            .take_while(|index| *index <= draft.cursor)
            .last()
            .unwrap_or(0);
        self.invalidate_review();
        self.notice = "Task-scoped memory draft and selection restored; no approval was restored or replayed.".into();
        if self
            .candidate
            .as_ref()
            .is_some_and(|proposal| proposal.provenance.reference.starts_with("habit:"))
        {
            self.notice = self.habit_disclosure();
        }
        Ok(())
    }

    pub(super) fn clear_draft(&mut self) {
        self.mode = Mode::Browse;
        self.candidate = None;
        self.target = None;
        self.habit_sources.clear();
        self.draft.clear();
        self.original.clear();
        self.cursor = 0;
        self.scroll = 0;
        self.invalidate_review();
    }

    fn new_note(&mut self) {
        if self.task_id <= 0 {
            self.notice = "Create and save a task before remembering a note.".into();
            return;
        }
        self.clear_draft();
        self.candidate = Some(Proposal {
            scope: Scope::Task,
            content: String::new(),
            provenance: Provenance {
                kind: SourceKind::UserAuthored,
                reference: format!("task:{}/user-authored-memory-note", self.task_id),
                task_id: self.task_id,
            },
        });
        self.mode = Mode::Edit;
        self.notice = "Write a user-authored note, not a captured secret. Enter previews; no memory is saved yet. Ctrl+N inserts a newline.".into();
    }

    fn edit_selected(&mut self) {
        let Some(record) = self.selected_record().cloned() else {
            return;
        };
        self.target = Some((record.id, record.revision));
        self.draft = record.proposal.content.clone();
        self.original = self.draft.clone();
        self.cursor = self.draft.len();
        self.candidate = Some(record.proposal);
        self.mode = Mode::Edit;
        self.scroll = 0;
        self.invalidate_review();
        self.notice = "Edit approved memory. Changed content is labelled user-authored; existing context snapshots become stale after commit.".into();
    }

    pub fn paste(&mut self, value: &str) -> bool {
        if !self.editing() || self.saving {
            return false;
        }
        if value.len() > memory::MAX_CONTENT_BYTES.saturating_sub(self.draft.len()) {
            self.notice = format!(
                "Memory limit is {} UTF-8 bytes; insertion refused.",
                memory::MAX_CONTENT_BYTES
            );
            return true;
        }
        let text = value.replace("\r\n", "\n").replace('\r', "\n");
        if let Err(error) = memory::validate_text(&text, memory::MAX_CONTENT_BYTES) {
            self.notice = error;
            return true;
        }
        self.draft.insert_str(self.cursor, &text);
        if let Err(error) = memory::validate_content(&self.draft) {
            self.draft
                .replace_range(self.cursor..self.cursor + text.len(), "");
            self.notice = error;
            return true;
        }
        self.cursor += text.len();
        self.cursor = self
            .draft
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|index| *index >= self.cursor)
            .unwrap_or(self.draft.len());
        self.invalidate_review();
        true
    }

    fn preview(&mut self) {
        let Some(candidate) = &mut self.candidate else {
            return;
        };
        candidate.content = self.draft.clone();
        if self.draft != self.original {
            candidate.provenance = Provenance {
                kind: SourceKind::UserAuthored,
                reference: self.target.map_or_else(
                    || format!("task:{}/user-authored-memory-note", self.task_id),
                    |(id, _)| format!("memory:{id}/user-authored-edit"),
                ),
                task_id: self.task_id,
            };
        }
        if let Err(error) = memory::validate_proposal(candidate) {
            self.notice = error;
            return;
        }
        self.mode = Mode::Preview;
        self.scroll = 0;
        self.invalidate_review();
        self.notice = "Not saved. Review the displayed scope; y Approve / n Edit. /verbose shows exact content and provenance.".into();
        if self
            .candidate
            .as_ref()
            .is_some_and(|proposal| proposal.provenance.reference.starts_with("habit:"))
        {
            self.notice = self.habit_disclosure();
        }
    }

    fn editor_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('n') {
            return self.paste("\n");
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return true;
        }
        let before = self.draft[..self.cursor]
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .next_back()
            .unwrap_or(0);
        let after = self
            .draft
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|index| *index > self.cursor)
            .unwrap_or(self.draft.len());
        match key.code {
            KeyCode::Char(ch) => return self.paste(ch.encode_utf8(&mut [0; 4])),
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                return self.paste("\n");
            }
            KeyCode::Enter => self.preview(),
            KeyCode::Esc => {
                if self.draft != self.original {
                    self.mode = Mode::Discard;
                    self.affirmative = false;
                } else {
                    self.clear_draft();
                }
            }
            KeyCode::Left => self.cursor = before,
            KeyCode::Right => self.cursor = after,
            KeyCode::Home => {
                self.cursor = self.draft[..self.cursor]
                    .rfind('\n')
                    .map_or(0, |index| index + 1)
            }
            KeyCode::End => self.cursor = self.line_content_end(self.cursor),
            KeyCode::Backspace => {
                self.draft.replace_range(before..self.cursor, "");
                self.cursor = before;
            }
            KeyCode::Delete => {
                self.draft.replace_range(self.cursor..after, "");
            }
            KeyCode::Up | KeyCode::Down => self.vertical_cursor(key.code == KeyCode::Down),
            _ => {}
        }
        true
    }
    fn line_content_end(&self, start: usize) -> usize {
        let end = self.draft[start..]
            .find('\n')
            .map_or(self.draft.len(), |index| start + index);
        if end < self.draft.len() && end > 0 && self.draft.as_bytes()[end - 1] == b'\r' {
            end - 1
        } else {
            end
        }
    }

    fn vertical_cursor(&mut self, down: bool) {
        let start = self.draft[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let column = self.draft[start..self.cursor].graphemes(true).count();
        let next_start = if down {
            let Some(end) = self.draft[self.cursor..].find('\n') else {
                return;
            };
            self.cursor + end + 1
        } else {
            if start == 0 {
                return;
            }
            self.draft[..start - 1]
                .rfind('\n')
                .map_or(0, |index| index + 1)
        };
        let end = self.line_content_end(next_start);
        self.cursor = self.draft[next_start..end]
            .grapheme_indices(true)
            .nth(column)
            .map_or(end, |(index, _)| next_start + index);
    }

    pub fn command_choices(&self) -> Vec<super::commands::Choice> {
        use super::commands::Choice;
        if self.saving || self.pending.is_some() {
            return Vec::new();
        }
        match self.mode {
            Mode::Browse => {
                let mut choices = vec![
                    Choice::new("create-memory", "Author a memory note"),
                    Choice::new("remember", "Remember the saved next-action note"),
                    Choice::new("memory-help", "Show memory actions and scope policy"),
                ];
                if self.selected_record().is_some() {
                    choices.extend([
                        Choice::new("edit-memory", "Edit selected memory"),
                        Choice::new(
                            "select-memory",
                            "Include/exclude selected memory in request context",
                        ),
                        Choice::new("delete-memory", "Review deletion of selected memory"),
                    ]);
                }
                choices
            }
            Mode::Edit => vec![Choice::new("preview-memory", "Review memory proposal")],
            Mode::Preview => vec![
                Choice::new("memory-scope", "Cycle Task / Project / Personal scope"),
                Choice::new("edit-memory", "Continue editing proposal"),
                Choice::new("reject-memory", "Reject unsaved proposal"),
            ],
            Mode::Delete | Mode::Discard => Vec::new(),
        }
    }

    pub fn command(&mut self, name: &str, large_enough: bool) -> bool {
        let available = match self.mode {
            Mode::Browse => {
                matches!(name, "create-memory" | "memory-help")
                    || (self.selected_record().is_some()
                        && matches!(name, "edit-memory" | "select-memory" | "delete-memory"))
            }
            Mode::Edit => name == "preview-memory",
            Mode::Preview => matches!(name, "memory-scope" | "edit-memory" | "reject-memory"),
            Mode::Delete | Mode::Discard => false,
        };
        if self.saving || self.pending.is_some() || !available {
            return false;
        }
        if !large_enough {
            self.invalidate_review();
            self.notice = "Resize to at least 60 columns x 18 rows before memory actions.".into();
            return true;
        }
        match name {
            "create-memory" => self.new_note(),
            "memory-help" => {
                self.show_help = !self.show_help;
                self.scroll = 0;
            }
            "edit-memory" if self.mode == Mode::Browse => self.edit_selected(),
            "edit-memory" => {
                self.mode = Mode::Edit;
                self.invalidate_review();
            }
            "preview-memory" => self.preview(),
            "select-memory" => {
                if let Some(id) = self.selected_record().map(|record| record.id) {
                    if self.selected_ids.contains(&id) {
                        self.selected_ids.retain(|selected| *selected != id);
                    } else if self.selected_ids.len() < memory::MAX_SELECTED {
                        self.selected_ids.push(id);
                    } else {
                        self.notice = format!(
                            "Select at most {} memories for a request.",
                            memory::MAX_SELECTED
                        );
                    }
                }
            }
            "delete-memory" => {
                self.mode = Mode::Delete;
                self.scroll = 0;
                self.invalidate_review();
                self.notice = "Delete only this approved memory? Review affected scope. Stay is selected; no deletion yet.".into();
            }
            "memory-scope" => {
                if let Some(candidate) = &mut self.candidate
                    && !candidate.provenance.reference.starts_with("habit:")
                {
                    candidate.scope = candidate.scope.next();
                }
                self.scroll = 0;
                self.invalidate_review();
            }
            "reject-memory" => {
                self.clear_draft();
                self.notice = "Proposal rejected. Nothing was saved or added to context.".into();
            }
            // Remember requires the App's committed next-action snapshot.
            "remember" => return false,
            _ => return false,
        }
        true
    }

    pub fn key(&mut self, mut key: KeyEvent, large_enough: bool) -> bool {
        if key.kind == KeyEventKind::Release {
            return true;
        }
        if key.kind == KeyEventKind::Repeat
            && !matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Backspace
                    | KeyCode::Delete
                    | KeyCode::Char(_)
            )
        {
            return true;
        }
        if key.kind == KeyEventKind::Repeat
            && !self.editing()
            && matches!(key.code, KeyCode::Char(_))
        {
            return true;
        }
        if self.saving {
            return true;
        }
        if !large_enough {
            self.invalidate_review();
            // Parent handles safe close/quit, never discard or approve here.
            return key.code != KeyCode::Esc;
        }
        if self.editing() {
            return self.editor_key(key);
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        if key.modifiers.is_empty() {
            match (self.mode, key.code) {
                (Mode::Preview | Mode::Delete, KeyCode::Char('y')) if self.reviewed => {
                    self.affirmative = true;
                    key.code = KeyCode::Enter;
                }
                (Mode::Discard, KeyCode::Char('y')) => {
                    self.affirmative = true;
                    key.code = KeyCode::Enter;
                }
                (Mode::Preview | Mode::Delete | Mode::Discard, KeyCode::Char('n')) => {
                    key.code = KeyCode::Esc;
                }
                _ => {}
            }
        }
        if key.code == KeyCode::PageUp {
            self.scroll = self.scroll.saturating_sub(6);
            return true;
        }
        if key.code == KeyCode::PageDown {
            self.scroll = self.scroll.saturating_add(6).min(self.scroll_max);
            return true;
        }
        match self.mode {
            Mode::Browse => match key.code {
                KeyCode::Esc => return false,
                KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    self.scroll = 0;
                }
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.records.len().saturating_sub(1));
                    self.scroll = 0;
                }
                _ => {}
            },
            Mode::Preview | Mode::Delete => match key.code {
                KeyCode::Esc => {
                    self.mode = if self.mode == Mode::Preview {
                        Mode::Edit
                    } else {
                        Mode::Browse
                    };
                    self.invalidate_review();
                }
                KeyCode::Left => self.affirmative = false,
                KeyCode::Right if self.reviewed => self.affirmative = true,
                KeyCode::Enter if self.affirmative && self.reviewed => {
                    let mutation = if self.mode == Mode::Delete {
                        self.selected_record().map(|record| Mutation::Delete {
                            id: record.id,
                            expected_revision: record.revision,
                        })
                    } else {
                        self.candidate.clone().map(|proposal| match self.target {
                            Some((id, expected_revision)) => Mutation::Edit {
                                id,
                                expected_revision,
                                proposal,
                            },
                            None => Mutation::Approve(proposal),
                        })
                    };
                    if let Some(mutation) = mutation {
                        self.pending = Some(mutation);
                        self.saving = true;
                        self.notice = "Saving approved memory change; waiting for committed store acknowledgement.".into();
                        self.invalidate_review();
                    }
                }
                _ => {}
            },
            Mode::Discard => match key.code {
                KeyCode::Left => self.affirmative = false,
                KeyCode::Right => self.affirmative = true,
                KeyCode::Esc => {
                    self.mode = Mode::Edit;
                    self.affirmative = false;
                }
                KeyCode::Enter if self.affirmative => self.clear_draft(),
                KeyCode::Enter => self.mode = Mode::Edit,
                _ => {}
            },
            Mode::Edit => unreachable!(),
        }
        true
    }
}

impl super::App {
    pub(super) fn memory_key(&mut self, key: KeyEvent, now: std::time::Instant) -> bool {
        if key.kind == KeyEventKind::Press
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('c')
        {
            self.request_quit();
            return true;
        }
        if key.code == KeyCode::Char('s') && key.modifiers == KeyModifiers::CONTROL {
            self.flush_save();
            return true;
        }
        if self.memory_loading {
            if key.code == KeyCode::Esc {
                self.close_overlay();
            } else {
                self.memory.notice =
                    "Loading approved memories; wait before editing. Esc Back / Ctrl+C safe quit."
                        .into();
            }
            return true;
        }
        let before = self.memory.draft_snapshot();
        let handled = self.memory.key(key, self.width >= 60 && self.height >= 18);
        if !handled && key.code == KeyCode::Esc {
            self.close_overlay();
        }
        if before != self.memory.draft_snapshot() {
            if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
                self.changed_task();
            } else {
                self.changed_implicitly(now);
            }
        }
        true
    }

    pub fn memory_command_choices(&self) -> Vec<super::commands::Choice> {
        if self.memory_loading {
            Vec::new()
        } else {
            self.memory.command_choices()
        }
    }

    pub fn memory_command(&mut self, name: &str) -> bool {
        if self.memory_loading {
            self.memory.notice = "Loading approved memories; wait before editing.".into();
            return true;
        }
        let before = self.memory.draft_snapshot();
        let remember = name == "remember"
            && self.memory.mode == Mode::Browse
            && !self.memory.saving
            && self.memory.pending.is_none()
            && self.width >= 60
            && self.height >= 18;
        let handled = if remember {
            if !matches!(self.storage, super::StorageState::Saved)
                || self.saved_revision != self.revision
                || self.save_requested
            {
                self.memory.notice =
                    "Save the task first, then Remember its committed next-action note.".into();
            } else {
                let provenance = Provenance {
                    kind: SourceKind::UserAuthored,
                    reference: format!(
                        "task:{}/next-action/revision:{}",
                        self.task_id, self.saved_revision
                    ),
                    task_id: self.task_id,
                };
                if let Err(error) = self.memory.propose(&self.next_action, provenance, false) {
                    self.memory.notice = if self.next_action.trim().is_empty() {
                        "No saved next-action note. Set Next action first, or /create-memory to author a memory note.".into()
                    } else {
                        error
                    };
                }
            }
            true
        } else {
            self.memory
                .command(name, self.width >= 60 && self.height >= 18)
        };
        if before != self.memory.draft_snapshot() {
            self.changed_task();
        }
        handled
    }

    pub(super) fn memory_paste(&mut self, value: &str, now: std::time::Instant) -> bool {
        if self.memory_loading {
            self.memory.notice = "Loading approved memories; paste was not inserted.".into();
            return true;
        }
        let before = self.memory.draft_snapshot();
        let handled = self.memory.paste(value);
        if before != self.memory.draft_snapshot() {
            self.changed_implicitly(now);
        }
        handled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        let mut state = State::new();
        state.reset(7, "/project/a");
        state
    }

    fn key(state: &mut State, code: KeyCode) {
        state.key(KeyEvent::new(code, KeyModifiers::NONE), true);
    }

    #[test]
    fn neutral_preview_reject_and_failed_save_preserve_draft() {
        let mut state = state();
        state.command("create-memory", true);
        state.paste("Keep focused tests.");
        key(&mut state, KeyCode::Enter);
        key(&mut state, KeyCode::Enter);
        assert!(state.pending.is_none());
        state.reviewed = true;
        key(&mut state, KeyCode::Right);
        key(&mut state, KeyCode::Enter);
        assert!(matches!(state.pending, Some(Mutation::Approve(_))));
        state.finish(Err("database full".into()));
        assert_eq!(state.draft, "Keep focused tests.");
        assert!(!state.affirmative);
        assert!(!state.reviewed);
        state.command("reject-memory", true);
        assert!(state.candidate.is_none());
        assert!(state.records.is_empty());
    }

    #[test]
    fn literal_paste_graphemes_and_durable_task_scope() {
        let mut state = state();
        state.command("create-memory", true);
        state.paste("中e\u{301}\nq approve");
        assert_eq!(state.mode, Mode::Edit);
        key(&mut state, KeyCode::Home);
        key(&mut state, KeyCode::Backspace);
        key(&mut state, KeyCode::Backspace);
        assert_eq!(state.draft, "中q approve");
        let saved = state.draft_snapshot();
        let mut restored = super::State::new();
        restored.reset(7, "/project/a");
        restored.restore_draft(&saved).unwrap();
        assert_eq!(restored.draft, state.draft);
        assert!(!restored.reviewed);
        restored.reset(8, "/project/a");
        assert!(restored.restore_draft(&saved).is_err());
    }

    #[test]
    fn source_note_crlf_end_vertical_edit_and_saved_draft_keep_valid_byte_boundaries() {
        let exact = "中e\u{301}tail\r\n短\r\nlast";
        let mut original = state();
        original
            .propose(
                exact,
                Provenance {
                    kind: SourceKind::UserAuthored,
                    reference: "task:7/source:41".into(),
                    task_id: 7,
                },
                false,
            )
            .unwrap();
        key(&mut original, KeyCode::Esc);
        assert_eq!(original.mode, Mode::Edit);
        original.cursor = 0;
        key(&mut original, KeyCode::End);
        assert_eq!(original.cursor, "中e\u{301}tail".len());
        key(&mut original, KeyCode::Down);
        assert_eq!(original.cursor, "中e\u{301}tail\r\n短".len());
        assert_eq!(
            &original.draft[original.cursor..original.cursor + 2],
            "\r\n"
        );
        key(&mut original, KeyCode::Up);
        assert_eq!(original.cursor, "中".len());
        key(&mut original, KeyCode::End);
        let first_end = original.cursor;
        let saved = original.draft_snapshot();
        let mut restored = state();
        restored.restore_draft(&saved).unwrap();
        assert_eq!(restored.cursor, first_end);
        assert_eq!(restored.draft.as_bytes(), exact.as_bytes());
        assert!(restored.paste("!"));
        assert_eq!(restored.draft, "中e\u{301}tail!\r\n短\r\nlast");
        let edited = restored.draft_snapshot();
        let mut resumed = state();
        resumed.restore_draft(&edited).unwrap();
        assert_eq!(resumed.cursor, restored.cursor);
        assert_eq!(resumed.draft.as_bytes(), restored.draft.as_bytes());
        key(&mut resumed, KeyCode::Delete);
        assert_eq!(resumed.draft, "中e\u{301}tail!短\r\nlast");
        assert!(memory::validate_content(&resumed.draft).is_ok());
        let merged = resumed.draft_snapshot();
        restored.restore_draft(&merged).unwrap();
        assert_eq!(restored.cursor, resumed.cursor);
        assert_eq!(restored.draft.as_bytes(), resumed.draft.as_bytes());
        key(&mut restored, KeyCode::Enter);
        assert_eq!(restored.mode, Mode::Preview);
        assert_eq!(
            restored.candidate.as_ref().unwrap().provenance.reference,
            "task:7/user-authored-memory-note",
        );
        assert!(restored.pending.is_none());
        assert!(!restored.reviewed);
    }

    #[test]
    fn excluded_and_secret_source_notes_are_never_candidates() {
        let mut state = state();
        let source = Provenance {
            kind: SourceKind::VerifiedNote,
            reference: "run:3/verified-note".into(),
            task_id: 7,
        };
        assert!(
            state
                .propose("private content", source.clone(), true)
                .is_err()
        );
        assert!(
            state
                .propose("token=private-content", source, false)
                .is_err()
        );
        assert!(state.candidate.is_none());
    }

    #[test]
    fn dirty_discard_and_small_terminal_cannot_approve() {
        let mut state = state();
        state.command("create-memory", true);
        state.paste("Draft");
        key(&mut state, KeyCode::Esc);
        assert_eq!(state.mode, Mode::Discard);
        key(&mut state, KeyCode::Enter);
        assert_eq!(state.mode, Mode::Edit);
        assert_eq!(state.draft, "Draft");
        key(&mut state, KeyCode::Enter);
        state.reviewed = true;
        state.affirmative = true;
        state.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), false);
        assert!(state.pending.is_none());
        assert!(!state.affirmative);
    }

    #[test]
    fn restoring_or_loading_never_discards_a_pending_draft() {
        let mut original = state();
        original.command("create-memory", true);
        original.paste("Uncommitted note");
        key(&mut original, KeyCode::Esc);
        let saved = original.draft_snapshot();
        let mut restored = state();
        restored.restore_draft(&saved).unwrap();
        assert_eq!(restored.mode, Mode::Edit);
        restored.finish(Ok(Vec::new()));
        assert_eq!(restored.draft, "Uncommitted note");
        assert_eq!(restored.mode, Mode::Edit);
        assert!(restored.pending.is_none());
    }

    #[test]
    fn scope_changes_and_repeated_enter_never_reuse_approval() {
        let mut state = state();
        let source = Provenance {
            kind: SourceKind::VerifiedNote,
            reference: "run:3/verified-note".into(),
            task_id: 7,
        };
        state
            .propose("Use the observed command.", source, false)
            .unwrap();
        state.reviewed = true;
        state.affirmative = true;
        state.command("memory-scope", true);
        assert_eq!(state.candidate.as_ref().unwrap().scope, Scope::Project);
        assert!(!state.reviewed);
        assert!(!state.affirmative);
        state.reviewed = true;
        state.affirmative = true;
        state.key(
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat),
            true,
        );
        assert!(state.pending.is_none());
        state.command("edit-memory", true);
        state.paste(" Updated by the user.");
        key(&mut state, KeyCode::Enter);
        assert_eq!(
            state.candidate.as_ref().unwrap().provenance.kind,
            SourceKind::UserAuthored
        );
    }

    #[test]
    fn semantic_actions_select_edit_and_review_deletion_without_implicit_consent() {
        let mut state = state();
        assert!(state.command("create-memory", true));
        state.paste("Keep checks scoped.");
        key(&mut state, KeyCode::Enter);
        let proposal = state.candidate.clone().unwrap();
        state.clear_draft();
        state.receive_records(vec![Record {
            id: 11,
            revision: 3,
            project: "/project/a".into(),
            task_id: Some(7),
            proposal,
        }]);
        assert!(state.command("select-memory", true));
        assert_eq!(state.selected_ids, vec![11]);
        assert!(state.command("select-memory", true));
        assert!(state.selected_ids.is_empty());
        assert!(state.command("edit-memory", true));
        assert_eq!(state.draft, "Keep checks scoped.");
        assert!(!state.command("delete-memory", true));
        assert_eq!(state.mode, Mode::Edit);
        key(&mut state, KeyCode::Esc);
        assert!(state.command("delete-memory", true));
        assert_eq!(state.mode, Mode::Delete);
        assert!(!state.affirmative);
        key(&mut state, KeyCode::Enter);
        assert!(state.pending.is_none());
        state.reviewed = true;
        key(&mut state, KeyCode::Right);
        key(&mut state, KeyCode::Enter);
        assert!(matches!(
            state.pending,
            Some(Mutation::Delete {
                id: 11,
                expected_revision: 3
            })
        ));
    }

    fn habit() -> crate::workspace::records::Habit {
        use crate::workspace::records::{self, Habit, InstructionSource};
        let (key, instruction) = records::instruction_key("Keep focused tests.").unwrap();
        Habit {
            key,
            instruction,
            sources: (1..=3)
                .map(|id| InstructionSource {
                    request_id: format!("submitted-{id}"),
                    task_id: 7,
                    owner: crate::store::Attribution {
                        partner: 1,
                        name: "Bolt".into(),
                        form: 8,
                    },
                    at: id,
                })
                .collect(),
            dismissed: false,
            memory: None,
        }
    }

    #[test]
    fn habit_requires_explicit_approval_then_selects_only_committed_exact_memory() {
        let mut state = state();
        let habit = habit();
        state.propose_habit(&habit).unwrap();
        assert_eq!(state.candidate.as_ref().unwrap().scope, Scope::Project);
        assert_eq!(state.habit_sources.len(), 3);
        assert!(state.notice.contains("substitutes"));
        assert!(state.selected_ids.is_empty());
        assert!(state.take_habit_binding().is_none());
        key(&mut state, KeyCode::Enter);
        assert!(state.pending.is_none());
        state.reviewed = true;
        key(&mut state, KeyCode::Right);
        key(&mut state, KeyCode::Enter);
        let proposal = state.candidate.clone().unwrap();
        state.finish(Ok(vec![Record {
            id: 19,
            revision: 1,
            project: "/project/a".into(),
            task_id: None,
            proposal,
        }]));
        assert_eq!(state.selected_ids, vec![19]);
        assert_eq!(state.take_habit_binding(), Some((habit.key, 19, 1)));
        assert!(state.take_habit_binding().is_none());
    }

    #[test]
    fn habit_failure_and_restart_keep_sources_but_never_replay_approval() {
        let mut original = state();
        original.propose_habit(&habit()).unwrap();
        original.reviewed = true;
        key(&mut original, KeyCode::Right);
        key(&mut original, KeyCode::Enter);
        original.finish(Err("storage full".into()));
        assert!(original.take_habit_binding().is_none());
        assert!(original.selected_ids.is_empty());
        let saved = original.draft_snapshot();
        let mut restored = state();
        restored.restore_draft(&saved).unwrap();
        assert_eq!(restored.habit_sources, original.habit_sources);
        assert!(restored.notice.contains("submitted-1"));
        assert!(restored.notice.contains("Explicit approval"));
        assert!(restored.pending.is_none());
        assert!(!restored.reviewed);
        assert!(!restored.affirmative);
        assert!(restored.take_habit_binding().is_none());
    }
}
