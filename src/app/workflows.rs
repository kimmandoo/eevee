//! Local specialist desk. Editing and navigation never dispatch work.
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::text::Span;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;

use super::{App, Overlay};
use crate::workspace::workflows::{
    self, EvidenceRef, EvidenceRelation, EvidenceStatus, EvidenceView, Verdict,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tab {
    #[default]
    Diagnosis,
    Refactor,
    Review,
    Recipe,
}
impl Tab {
    pub const ALL: [Self; 4] = [Self::Diagnosis, Self::Refactor, Self::Review, Self::Recipe];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Field {
    Hypothesis,
    Contract,
    Finding,
    Action,
    RecipePath,
    Criteria,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Chooser {
    Hypothesis,
    Finding,
    Hotspot,
    Coverage,
    Baseline,
    Post,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub tab: Tab,
    pub selected: usize,
    pub hypothesis: usize,
    pub finding: usize,
    pub criterion: usize,
    pub evidence: usize,
    pub link: usize,
    pub refuting: bool,
    pub editor: Option<Field>,
    pub cursor: usize,
    pub hypothesis_text: String,
    pub contract: String,
    pub hotspot: Vec<EvidenceRef>,
    pub finding_evidence: Vec<EvidenceRef>,
    pub baseline: String,
    pub finding_text: String,
    pub action: String,
    pub editor_done: bool,
    pub opened: bool,
    pub last_form: Option<usize>,
    pub recipe_path: String,
    pub chooser: Option<Chooser>,
    pub preview: bool,
    pub affirmative: bool,
    pub scroll: u16,
    pub notice: String,
    #[serde(skip)]
    pub relay: Option<super::relays::Pending>,
    pub pending_decision: Option<String>,
    pub criteria_text: String,
    pub criteria_decision: Option<workflows::ChosenDecision>,
    pub criteria_preview: bool,
    #[serde(skip)]
    pub scroll_page: u16,
    #[serde(skip)]
    pub scroll_max: u16,
    #[serde(skip)]
    pub reviewed: bool,
    #[serde(skip)]
    pub recipe: Option<crate::workspace::workflows::Recipe>,
    #[serde(skip)]
    pub statuses: Vec<(EvidenceRef, EvidenceStatus)>,
    #[serde(skip)]
    pub hypothesis_states: Vec<(u64, workflows::HypothesisStatus)>,
    #[serde(skip)]
    pub review_summary: String,
    #[serde(skip)]
    pub comparison_summary: String,
    #[serde(skip)]
    pub comparison_fingerprint: Option<String>,
    #[serde(skip)]
    pub recipe_preview_text: String,
    #[serde(skip)]
    pub baseline_expected: Option<crate::workspace::runner::Prepared>,
    #[serde(skip)]
    pub baseline_error: String,
    #[serde(skip)]
    pub addressed_fingerprint: Option<String>,
}
impl State {
    pub fn draft_snapshot(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
    pub fn restore_draft(&mut self, value: Value) {
        if let Ok(mut state) = serde_json::from_value::<Self>(value) {
            state.affirmative = false;
            state.preview = false; // A restored draft is never a reviewed recipe.
            state.pending_decision = None;
            if let Some(field) = state.editor {
                state.cursor = state
                    .value(field)
                    .grapheme_indices(true)
                    .map(|(i, _)| i)
                    .chain(std::iter::once(state.value(field).len()))
                    .take_while(|i| *i <= state.cursor)
                    .last()
                    .unwrap_or(0);
            }
            *self = state;
        }
    }
    pub fn value(&self, field: Field) -> &str {
        match field {
            Field::Hypothesis => &self.hypothesis_text,
            Field::Contract => &self.contract,
            Field::Finding => &self.finding_text,
            Field::Action => &self.action,
            Field::RecipePath => &self.recipe_path,
            Field::Criteria => &self.criteria_text,
        }
    }
    fn value_mut(&mut self, field: Field) -> &mut String {
        match field {
            Field::Hypothesis => &mut self.hypothesis_text,
            Field::Contract => &mut self.contract,
            Field::Finding => &mut self.finding_text,
            Field::Action => &mut self.action,
            Field::RecipePath => &mut self.recipe_path,
            Field::Criteria => &mut self.criteria_text,
        }
    }
    fn edit(&mut self, field: Field) {
        self.editor = Some(field);
        self.editor_done = false;
        self.cursor = self.value(field).len();
        self.notice =
            "Draft retained on Esc. Finish editing, then choose the explicit save action.".into();
    }
    pub fn paste(&mut self, input: &str) -> bool {
        let Some(field) = self.editor else {
            return false;
        };
        let text: String = input
            .chars()
            .filter(|ch| super::safe_character(*ch))
            .collect();
        if text.len() > workflows::MAX_TEXT_BYTES.saturating_sub(self.value(field).len()) {
            self.notice = "Field limit: 4096 UTF-8 bytes. Paste not inserted.".into();
            return true;
        }
        let cursor = self.cursor;
        self.value_mut(field).insert_str(cursor, &text);
        self.cursor = self
            .value(field)
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i >= cursor + text.len())
            .unwrap_or(self.value(field).len());
        true
    }
    fn editor_key(&mut self, key: KeyEvent) {
        let Some(field) = self.editor else {
            return;
        };
        if key.code == KeyCode::Tab || key.code == KeyCode::BackTab {
            self.editor_done = !self.editor_done;
            return;
        }
        if key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL))
            || (key.code == KeyCode::Enter
                && (self.editor_done || key.modifiers.contains(KeyModifiers::CONTROL)))
        {
            self.editor = None;
            self.notice = "Draft retained; choose the explicit save action to record it.".into();
            return;
        }
        if key.code == KeyCode::Enter {
            self.paste("\n");
            return;
        }
        if let KeyCode::Char(ch) = key.code
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.paste(ch.encode_utf8(&mut [0; 4]));
            return;
        }
        let value = self.value(field);
        let cursor = self.cursor;
        let before = value[..cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i);
        let after = value
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i > cursor)
            .unwrap_or(value.len());
        let start = value[..cursor].rfind('\n').map_or(0, |i| i + 1);
        let end = value[cursor..]
            .find('\n')
            .map_or(value.len(), |i| cursor + i);
        match key.code {
            KeyCode::Left => self.cursor = before,
            KeyCode::Right => self.cursor = after,
            KeyCode::Home => self.cursor = start,
            KeyCode::End => self.cursor = end,
            KeyCode::Backspace => {
                self.value_mut(field).replace_range(before..cursor, "");
                self.cursor = before;
            }
            KeyCode::Delete => {
                self.value_mut(field).replace_range(cursor..after, "");
            }
            KeyCode::Up | KeyCode::Down => {
                let column = Span::raw(&value[start..cursor]).width();
                let target = if key.code == KeyCode::Up && start > 0 {
                    let end = start - 1;
                    Some((value[..end].rfind('\n').map_or(0, |i| i + 1), end))
                } else if key.code == KeyCode::Down && end < value.len() {
                    Some((
                        end + 1,
                        value[end + 1..]
                            .find('\n')
                            .map_or(value.len(), |i| end + 1 + i),
                    ))
                } else {
                    None
                };
                if let Some((start, end)) = target {
                    let mut next = start;
                    let mut cells = 0;
                    for (i, grapheme) in value[start..end].grapheme_indices(true) {
                        cells += Span::raw(grapheme).width();
                        if cells > column {
                            break;
                        }
                        next = start + i + grapheme.len();
                    }
                    self.cursor = next;
                }
            }
            _ => {}
        }
        // Deletion can merge graphemes across the old boundary.
        self.cursor = self
            .value(field)
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(self.value(field).len()))
            .take_while(|i| *i <= self.cursor)
            .last()
            .unwrap_or(0);
    }
}

#[derive(Clone, Copy)]
enum Action {
    Edit(Field),
    AddHypothesis,
    NextHypothesis,
    Choose(Chooser),
    NextLink,
    OpenLink,
    Diagnostic,
    Fix,
    SaveRefactor,
    Compare,
    OpenBaseline,
    OpenPost,
    NextCriterion,
    AddFinding,
    NextFinding,
    ResolveFinding,
    Review,
    Verdict(Verdict),
    LoadRecipe,
    PreviewRecipe,
    Refresh,
    DecisionCriteria,
    EditCriteria,
    ConfirmCriteria,
    RelayDecision,
    FreshBaseline,
}

impl Action {
    fn command(self) -> &'static str {
        match self {
            Self::Edit(Field::Hypothesis) => "edit-hypothesis",
            Self::Edit(Field::Contract) => "edit-contract",
            Self::Edit(Field::Finding) => "edit-finding",
            Self::Edit(Field::Action) => "edit-recommendation",
            Self::Edit(Field::RecipePath) => "recipe-path",
            Self::AddHypothesis => "add-hypothesis",
            Self::NextHypothesis => "next-hypothesis",
            Self::Choose(Chooser::Hypothesis) => "hypothesis-evidence",
            Self::Choose(Chooser::Finding) => "finding-evidence",
            Self::Choose(Chooser::Hotspot) => "refactor-hotspot",
            Self::Choose(Chooser::Baseline) => "refactor-baseline",
            Self::Choose(Chooser::Post) => "refactor-post",
            Self::Choose(Chooser::Coverage) => "criterion-evidence",
            Self::NextLink => "next-link",
            Self::OpenLink => "open-link",
            Self::Diagnostic => "diagnostic",
            Self::Fix => "specialist-fix",
            Self::SaveRefactor => "record-refactor",
            Self::Compare => "compare-refactor",
            Self::OpenBaseline => "open-baseline",
            Self::OpenPost => "open-post",
            Self::NextCriterion => "next-criterion",
            Self::AddFinding => "record-finding",
            Self::NextFinding => "next-finding",
            Self::ResolveFinding => "resolve-finding",
            Self::Review => "managed-review",
            Self::Verdict(Verdict::Inconclusive) => "verdict-inconclusive",
            Self::Verdict(Verdict::RequestChanges) => "verdict-changes",
            Self::Verdict(Verdict::Accept) => "verdict-accept",
            Self::LoadRecipe => "load-recipe",
            Self::PreviewRecipe => "preview-recipe",
            Self::Refresh => "refresh-evidence",
            Self::Edit(Field::Criteria) => "edit-decision-criteria",
            Self::DecisionCriteria => "decision-criteria",
            Self::EditCriteria => "edit-criteria-proposal",
            Self::ConfirmCriteria => "confirm-decision-criteria",
            Self::RelayDecision => "relay-decision",
            Self::FreshBaseline => "fresh-baseline",
        }
    }
}

fn actions(tab: Tab) -> &'static [(Action, &'static str)] {
    use Action::*;
    match tab {
        Tab::Diagnosis => &[
            (Edit(Field::Hypothesis), "Edit hypothesis / explanation"),
            (AddHypothesis, "Add hypothesis"),
            (NextHypothesis, "Select next hypothesis"),
            (
                Choose(Chooser::Hypothesis),
                "Attach supporting / refuting evidence",
            ),
            (NextLink, "Select next attached link"),
            (OpenLink, "Open attached evidence"),
            (Diagnostic, "Request diagnostic run via Jolteon"),
            (Fix, "Handoff diagnosis to Flareon"),
            (Refresh, "Refresh evidence status"),
        ],
        Tab::Refactor => &[
            (Edit(Field::Contract), "Edit behavior contract"),
            (Choose(Chooser::Hotspot), "Select bounded source hotspot"),
            (Choose(Chooser::Baseline), "Select baseline run"),
            (FreshBaseline, "Choose none / request fresh baseline"),
            (SaveRefactor, "Record contract / hotspot / baseline"),
            (Fix, "Request bounded refactor via Flareon"),
            (Diagnostic, "Request measurement via Jolteon"),
            (Choose(Chooser::Post), "Select post-change run"),
            (Compare, "Compare baseline / post-change"),
            (OpenBaseline, "Open baseline output"),
            (OpenPost, "Open post-change output"),
            (Refresh, "Refresh evidence status"),
        ],
        Tab::Review => &[
            (NextCriterion, "Select acceptance criterion"),
            (Edit(Field::Finding), "Edit finding detail"),
            (
                Edit(Field::Action),
                "Edit actionable recommendation / resolution",
            ),
            (Choose(Chooser::Finding), "Attach finding evidence"),
            (AddFinding, "Record actionable finding"),
            (NextFinding, "Select next recorded finding"),
            (NextLink, "Select next attached link"),
            (OpenLink, "Open attached evidence"),
            (
                Choose(Chooser::Coverage),
                "Attach selected criterion evidence",
            ),
            (Review, "Open managed Review preview (no send)"),
            (
                Verdict(crate::workspace::workflows::Verdict::Inconclusive),
                "Verdict: Inconclusive",
            ),
            (
                Verdict(crate::workspace::workflows::Verdict::RequestChanges),
                "Verdict: Request changes",
            ),
            (
                Verdict(crate::workspace::workflows::Verdict::Accept),
                "Verdict: Accept (coverage guarded)",
            ),
            (Fix, "Handoff selected finding to Flareon"),
            (ResolveFinding, "Resolve finding using edited resolution"),
            (Refresh, "Refresh evidence status"),
        ],
        Tab::Recipe => &[
            (Edit(Field::RecipePath), "Edit literal recipe path"),
            (LoadRecipe, "Load recipe read-only"),
            (PreviewRecipe, "Preview complete recipe / apply plan only"),
            (DecisionCriteria, "Add chosen Espeon decision as criteria"),
            (EditCriteria, "Edit proposed criteria (one per line)"),
            (ConfirmCriteria, "Review / confirm decision criteria"),
            (RelayDecision, "Send chosen decision to idle Eevee"),
        ],
    }
}

impl App {
    pub fn workflow_open(&mut self) {
        if !self.workflow.opened
            || (self.workflow.last_form != Some(self.form()) && matches!(self.form(), 5..=7))
        {
            self.workflow.tab = match self.form() {
                6 => Tab::Refactor,
                7 => Tab::Review,
                8 => Tab::Recipe,
                _ => Tab::Diagnosis,
            };
            self.workflow.selected = 0;
            self.workflow.opened = true;
        }
        self.workflow.last_form = Some(self.form());
        self.execution.open(self.task_id);
        self.workflow_refresh();
    }

    /// Refresh only on deliberate interaction or a new repository observation,
    /// never from rendering: resolving sources performs bounded read-only I/O.
    pub fn workflow_refresh(&mut self) {
        self.suggestion_cache.invalidate();
        self.carry_over.invalidate();
        if self.workflow.tab == Tab::Refactor {
            self.workflow_refresh_baseline_identity();
        }
        self.workflow_refresh_addressed();
        let current = self.workflow_fingerprint();
        if current.fingerprint != self.workflow.comparison_fingerprint
            || !current.complete
            || !current.stable
        {
            self.workflow.comparison_summary.clear();
            self.workflow.comparison_fingerprint = None;
        }
        let view = EvidenceView {
            task_id: self.task_id,
            excerpts: &self.sources.basket,
            runs: &self.execution.history,
            current: &current,
        };
        let mut refs = Vec::new();
        for excerpt in &self.sources.basket {
            if let Ok(reference) = EvidenceRef::source(excerpt) {
                refs.push(reference);
            }
        }
        for run in &self.execution.history {
            if let Ok(reference) = EvidenceRef::run(run) {
                refs.push(reference);
            }
        }
        for hypothesis in &self.workflows.hypotheses {
            refs.extend(
                hypothesis
                    .evidence
                    .iter()
                    .map(|link| link.reference.clone()),
            );
        }
        for finding in &self.workflows.findings {
            refs.extend(finding.evidence.iter().cloned());
        }
        if let Some(review) = &self.workflows.review {
            for criterion in &review.criteria {
                refs.extend(criterion.evidence.iter().cloned());
            }
        }
        self.workflow.statuses.clear();
        for reference in refs {
            if !self
                .workflow
                .statuses
                .iter()
                .any(|(existing, _)| existing == &reference)
            {
                let status = view.status(&reference);
                self.workflow.statuses.push((reference, status));
            }
        }
        self.workflow.hypothesis_states = self
            .workflows
            .hypotheses
            .iter()
            .map(|hypothesis| {
                (
                    hypothesis.id,
                    self.workflows.hypothesis_status(hypothesis.id, &view),
                )
            })
            .collect();
        let criteria: Vec<_> = self
            .criteria
            .iter()
            .map(|criterion| criterion.text.clone())
            .collect();
        let guard = self.workflows.review_guard(&criteria, &view);
        self.workflow.review_summary = format!(
            "Effective verdict: {:?}\n{}",
            guard.verdict,
            guard.reasons.join("\n")
        );
    }

    pub(super) fn workflow_fingerprint(&self) -> crate::workspace::runner::InputSnapshot {
        if self.repo_loading || self.repo_refresh || !self.repo_error.is_empty() {
            crate::workspace::runner::InputSnapshot::default()
        } else {
            self.repository
                .as_deref()
                .map(super::execution::fingerprint)
                .unwrap_or_default()
        }
    }

    pub fn workflow_paste(&mut self, text: &str) -> bool {
        if self.width < 60 || self.height < 18 {
            return true;
        }
        self.workflow.paste(text)
    }

    pub fn workflow_command_choices(&self) -> Vec<super::commands::Choice> {
        use super::commands::Choice;
        if self.workflow.editor.is_some() || self.workflow.preview || self.workflow.relay.is_some()
        {
            return Vec::new();
        }
        if self.workflow.chooser.is_some() {
            return vec![
                Choice::new("open-evidence", "Open selected evidence"),
                Choice::new("attach-evidence", "Attach selected evidence"),
            ];
        }
        let mut choices = vec![
            Choice::new("workflow-diagnosis", "Diagnosis desk"),
            Choice::new("workflow-refactor", "Refactor desk"),
            Choice::new("workflow-review", "Review desk"),
            Choice::new("workflow-recipe", "Recipe desk"),
        ];
        choices.extend(
            actions(self.workflow.tab)
                .iter()
                .map(|(action, label)| Choice::new(action.command(), *label)),
        );
        choices
    }

    pub fn workflow_command(&mut self, name: &str) -> bool {
        let available = if self.workflow.chooser.is_some() {
            matches!(name, "open-evidence" | "attach-evidence")
        } else {
            matches!(
                name,
                "workflow-diagnosis" | "workflow-refactor" | "workflow-review" | "workflow-recipe"
            ) || actions(self.workflow.tab)
                .iter()
                .any(|(action, _)| action.command() == name)
        };
        if self.workflow.editor.is_some() || self.workflow.preview || !available {
            return false;
        }
        if self.width < 60 || self.height < 18 {
            self.workflow.affirmative = false;
            self.workflow.reviewed = false;
            self.workflow.notice =
                "Resize to at least 60 columns x 18 rows before specialist actions.".into();
            return true;
        }
        if let Some(chooser) = self.workflow.chooser {
            if name == "open-evidence" {
                if let Some((reference, _)) =
                    self.workflow_choices(chooser).get(self.workflow.evidence)
                {
                    self.workflow_open_reference(reference.clone());
                } else {
                    self.workflow.notice = "No evidence selected.".into();
                }
            } else {
                let result = self.workflow_attach(chooser);
                self.workflow_result(result);
                self.workflow_refresh();
            }
            return true;
        }
        let tab = match name {
            "workflow-diagnosis" => Some(Tab::Diagnosis),
            "workflow-refactor" => Some(Tab::Refactor),
            "workflow-review" => Some(Tab::Review),
            "workflow-recipe" => Some(Tab::Recipe),
            _ => None,
        };
        if let Some(tab) = tab {
            self.workflow.tab = tab;
            self.workflow.selected = 0;
            self.workflow.scroll = 0;
            self.workflow_refresh();
            return true;
        }
        if let Some((action, _)) = actions(self.workflow.tab)
            .iter()
            .find(|(action, _)| action.command() == name)
        {
            let result = self.workflow_action(*action);
            self.workflow_result(result);
            self.workflow_refresh();
            return true;
        }
        false
    }

    pub fn workflow_key(&mut self, mut key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if self.relay_key(key) {
            return true;
        }
        if self.workflow.editor.is_some() && self.width >= 60 && self.height >= 18 {
            self.workflow.editor_key(key);
            return true;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.request_quit();
            return true;
        }
        if self.width < 60 || self.height < 18 {
            self.workflow.affirmative = false;
            self.workflow.reviewed = false;
            if key.code == KeyCode::Esc {
                self.close_overlay();
            }
            return true;
        }
        if self.workflow.preview && key.kind == KeyEventKind::Press && key.modifiers.is_empty() {
            match key.code {
                KeyCode::Char('y') if self.workflow.reviewed => {
                    self.workflow.affirmative = true;
                    key.code = KeyCode::Enter;
                }
                KeyCode::Char('n') => key.code = KeyCode::Esc,
                _ => {}
            }
        }
        if key.code == KeyCode::Esc {
            if self.workflow.chooser.take().is_some() || self.workflow.preview {
                self.workflow.preview = false;
                self.workflow.affirmative = false;
                self.workflow.scroll = 0;
            } else {
                self.close_overlay();
            }
            return true;
        }
        if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            let page = self.workflow.scroll_page.max(1);
            self.workflow.scroll = if key.code == KeyCode::PageDown {
                self.workflow
                    .scroll
                    .saturating_add(page)
                    .min(self.workflow.scroll_max)
            } else {
                self.workflow.scroll.saturating_sub(page)
            };
            return true;
        }
        if self.workflow.preview {
            match key.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                    self.workflow.affirmative = !self.workflow.affirmative
                }
                KeyCode::Enter if key.kind == KeyEventKind::Press => {
                    if !self.workflow.affirmative {
                        self.workflow.preview = false;
                    } else if !self.workflow.reviewed {
                        self.workflow.notice =
                            "Review the displayed scope before applying. Nothing changed.".into();
                    } else {
                        let result = if self.workflow.criteria_preview {
                            self.workflow_apply_criteria()
                        } else {
                            self.workflow_apply_recipe()
                        };
                        self.workflow_result(result);
                    }
                }
                _ => {}
            }
            return true;
        }
        if let Some(chooser) = self.workflow.chooser {
            let count = self.workflow_choices(chooser).len();
            match key.code {
                KeyCode::Up => self.workflow.evidence = self.workflow.evidence.saturating_sub(1),
                KeyCode::Down => {
                    self.workflow.evidence =
                        (self.workflow.evidence + 1).min(count.saturating_sub(1))
                }
                KeyCode::Left | KeyCode::Right => self.workflow.refuting = !self.workflow.refuting,
                KeyCode::Enter if key.kind == KeyEventKind::Press => {
                    let result = self.workflow_attach(chooser);
                    self.workflow_result(result);
                    self.workflow_refresh();
                }
                _ => {}
            }
            return true;
        }
        let count = actions(self.workflow.tab).len();
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                let index = Tab::ALL
                    .iter()
                    .position(|tab| *tab == self.workflow.tab)
                    .unwrap_or(0);
                let next = if matches!(key.code, KeyCode::Left | KeyCode::BackTab) {
                    (index + 3) % 4
                } else {
                    (index + 1) % 4
                };
                self.workflow.tab = Tab::ALL[next];
                self.workflow.selected = 0;
                self.workflow.scroll = 0;
                self.workflow_refresh();
            }
            KeyCode::Up => {
                self.workflow.selected = self.workflow.selected.saturating_sub(1);
                self.workflow.scroll = 0;
            }
            KeyCode::Down => {
                self.workflow.selected = (self.workflow.selected + 1).min(count.saturating_sub(1));
                self.workflow.scroll = 0;
            }
            KeyCode::Enter if key.kind == KeyEventKind::Press => {
                if let Some((action, _)) = actions(self.workflow.tab).get(self.workflow.selected) {
                    let result = self.workflow_action(*action);
                    self.workflow_result(result);
                    self.workflow_refresh();
                }
            }
            _ => {}
        }
        true
    }

    fn workflow_result(&mut self, result: Result<(), String>) {
        if let Err(error) = result {
            self.workflow.notice = error;
        }
    }

    fn workflow_choices(&self, chooser: Chooser) -> Vec<(EvidenceRef, String)> {
        let mut result = Vec::new();
        if !matches!(chooser, Chooser::Baseline | Chooser::Post) {
            for excerpt in &self.sources.basket {
                let reference = EvidenceRef::Source {
                    task_id: excerpt.task_id,
                    excerpt_id: excerpt.id,
                    path: excerpt.path.clone(),
                    start_line: excerpt.start_line,
                    end_line: excerpt.end_line,
                    source_sha256: excerpt.source_sha256,
                };
                let label = format!(
                    "Source {}:{}-{} / {:?}",
                    excerpt.path,
                    excerpt.start_line,
                    excerpt.end_line,
                    self.workflow_status(&reference)
                );
                result.push((reference, label));
            }
        }
        if chooser != Chooser::Hotspot {
            for run in &self.execution.history {
                if run.prepared.task_id != self.task_id {
                    continue;
                }
                if let Ok(reference) = EvidenceRef::run(run) {
                    let label = format!(
                        "Run {} / {:?} / {:?}",
                        run.prepared.run_id,
                        run.state,
                        self.workflow_status(&reference)
                    );
                    let label = if chooser == Chooser::Baseline {
                        let eligibility = self.workflow_baseline_eligible(run).map_or_else(
                            |reason| format!("excluded: {reason}"),
                            |_| "eligible passing baseline".into(),
                        );
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |v| v.as_millis() as u64);
                        format!(
                            "{} / {} / age {}s / rev {} / elapsed {:?}ms",
                            eligibility,
                            run.prepared.run_id,
                            now.saturating_sub(run.ended_at_ms.unwrap_or(0)) / 1000,
                            run.after.fingerprint.as_deref().unwrap_or("unknown"),
                            run.ended_at_ms
                                .zip(run.started_at_ms)
                                .map(|(e, s)| e.saturating_sub(s))
                        )
                    } else {
                        label
                    };
                    result.push((reference, label));
                }
            }
        }
        result
    }

    fn workflow_status(&self, reference: &EvidenceRef) -> EvidenceStatus {
        self.workflow
            .statuses
            .iter()
            .find(|(existing, _)| existing == reference)
            .map_or(EvidenceStatus::Unavailable, |(_, status)| *status)
    }

    fn workflow_attach(&mut self, chooser: Chooser) -> Result<(), String> {
        if !self.records_can_switch() {
            return Err(
                "Wait for the local record batch to commit before changing specialist facts."
                    .into(),
            );
        }
        if chooser == Chooser::Baseline {
            self.workflow_refresh_baseline_identity();
        }
        let reference = self
            .workflow_choices(chooser)
            .get(self.workflow.evidence)
            .map(|(reference, _)| reference.clone())
            .ok_or(
                "No evidence selected. Pin sources or run a separately approved Jolteon check.",
            )?;
        let current = self.workflow_fingerprint();
        let view = EvidenceView {
            task_id: self.task_id,
            excerpts: &self.sources.basket,
            runs: &self.execution.history,
            current: &current,
        };
        match chooser {
            Chooser::Hypothesis => {
                let id = self
                    .workflows
                    .hypotheses
                    .get(self.workflow.hypothesis)
                    .ok_or("Add a hypothesis first")?
                    .id;
                self.workflows.attach_hypothesis(
                    id,
                    if self.workflow.refuting {
                        EvidenceRelation::Refutes
                    } else {
                        EvidenceRelation::Supports
                    },
                    reference,
                    &view,
                )?;
                self.record_artifact(
                    crate::workspace::records::ArtifactKind::Hypothesis,
                    id.to_string(),
                );
                self.changed_task();
            }
            Chooser::Finding => {
                if let Some(index) = self
                    .workflow
                    .finding_evidence
                    .iter()
                    .position(|existing| existing == &reference)
                {
                    self.workflow.finding_evidence.remove(index);
                } else {
                    if view.status(&reference) == EvidenceStatus::Unavailable {
                        return Err("Selected evidence is unavailable.".into());
                    }
                    if self.workflow.finding_evidence.len() >= 16 {
                        return Err("Finding draft has 16 evidence links".into());
                    }
                    self.workflow.finding_evidence.push(reference);
                }
            }
            Chooser::Hotspot => {
                if let Some(index) = self
                    .workflow
                    .hotspot
                    .iter()
                    .position(|existing| existing == &reference)
                {
                    self.workflow.hotspot.remove(index);
                } else {
                    if view.status(&reference) != EvidenceStatus::Current {
                        return Err("Select current source evidence for the hotspot.".into());
                    }
                    if self.workflow.hotspot.len() >= 16 {
                        return Err("Hotspot has 16 source links".into());
                    }
                    self.workflow.hotspot.push(reference);
                }
            }
            Chooser::Baseline => {
                let EvidenceRef::Run { run_id, .. } = reference else {
                    return Err("Choose a local run".into());
                };
                let run = view.run(&run_id).ok_or("Baseline unavailable")?;
                self.workflow_baseline_eligible(run)?;
                self.workflow.baseline = run_id;
            }
            Chooser::Post => {
                let EvidenceRef::Run { run_id, .. } = reference else {
                    return Err("Choose a local run".into());
                };
                self.workflows.set_post_run(&run_id, &view)?;
                self.record_artifact(
                    crate::workspace::records::ArtifactKind::Refactor,
                    "refactor",
                );
                self.workflow.comparison_summary.clear();
                self.changed_task();
            }
            Chooser::Coverage => {
                let criterion = self
                    .criteria
                    .get(self.workflow.criterion)
                    .ok_or("Add task acceptance criteria first")?
                    .text
                    .clone();
                let mut criteria = self
                    .workflows
                    .review
                    .as_ref()
                    .map(|review| review.criteria.clone())
                    .unwrap_or_default();
                if let Some(entry) = criteria
                    .iter_mut()
                    .find(|entry| entry.criterion == criterion)
                {
                    if let Some(index) = entry
                        .evidence
                        .iter()
                        .position(|existing| existing == &reference)
                    {
                        entry.evidence.remove(index);
                    } else {
                        entry.evidence.push(reference);
                    }
                } else {
                    criteria.push(workflows::CriterionEvidence {
                        criterion,
                        evidence: vec![reference],
                    });
                }
                self.workflows.update_review_evidence(criteria, &view)?;
                self.record_artifact(
                    crate::workspace::records::ArtifactKind::Review,
                    self.workflows.review_generation.to_string(),
                );
                self.changed_task();
            }
        }
        self.workflow.chooser = None;
        self.workflow.notice = "Evidence selection updated explicitly. Re-select draft/criterion evidence to remove it; links are not proof of every requirement.".into();
        Ok(())
    }

    fn workflow_action(&mut self, action: Action) -> Result<(), String> {
        if !self.records_can_switch() && !matches!(action, Action::Edit(_)) {
            return Err(
                "Wait for the local record batch to commit before changing specialist facts."
                    .into(),
            );
        }
        if self.task_id <= 0 && !matches!(action, Action::Edit(_)) {
            return Err("Create and save a task before recording specialist work.".into());
        }
        if (self.planning_any_busy() || self.execution_any_busy())
            && matches!(
                action,
                Action::AddHypothesis
                    | Action::SaveRefactor
                    | Action::AddFinding
                    | Action::ResolveFinding
                    | Action::Review
                    | Action::Verdict(_)
            )
        {
            return Err("Finish owned work before changing committed specialist facts; the local draft is kept.".into());
        }
        if matches!(action, Action::SaveRefactor) {
            self.workflow_refresh_baseline_identity();
        }
        if matches!(
            action,
            Action::SaveRefactor | Action::Compare | Action::Verdict(Verdict::Accept)
        ) {
            let result = self
                .repository
                .as_deref()
                .ok_or_else(|| "Repository observation unavailable".to_string())
                .and_then(crate::workspace::repo::revalidate_review);
            if let Err(error) = result {
                self.repo_refresh = true;
                self.repo_loading = true;
                self.workflow.comparison_summary.clear();
                return Err(format!(
                    "Fresh evidence required: {error}. Refresh requested; no success recorded."
                ));
            }
        }
        let current = self.workflow_fingerprint();
        let view = EvidenceView {
            task_id: self.task_id,
            excerpts: &self.sources.basket,
            runs: &self.execution.history,
            current: &current,
        };
        match action {
            Action::Edit(field) => self.workflow.edit(field),
            Action::AddHypothesis => {
                self.workflows
                    .add_hypothesis(&self.workflow.hypothesis_text)?;
                self.workflow.hypothesis = self.workflows.hypotheses.len().saturating_sub(1);
                self.workflow.hypothesis_text.clear();
                let id = self.workflows.hypotheses[self.workflow.hypothesis]
                    .id
                    .to_string();
                self.record_artifact(crate::workspace::records::ArtifactKind::Hypothesis, id);
                self.workflow.notice =
                    "Hypothesis recorded as an unresolved inference. Attach real observations."
                        .into();
                self.changed_task();
            }
            Action::NextHypothesis => {
                self.workflow.hypothesis =
                    (self.workflow.hypothesis + 1) % self.workflows.hypotheses.len().max(1);
                self.workflow.link = 0;
            }
            Action::Choose(chooser) => {
                self.workflow.chooser = Some(chooser);
                self.workflow.evidence = 0;
                self.workflow.scroll = 0;
                self.workflow.notice = match chooser {
                    Chooser::Hypothesis => "Choose actual evidence. Left/Right sets Supporting or Refuting.",
                    Chooser::Hotspot => "Choose an existing pinned source range; each selection adds one bounded hotspot.",
                    Chooser::Coverage => "Choose evidence relevant to the selected criterion; a source and passing run are required.",
                    Chooser::Finding => "Attach evidence to the finding draft, then Record actionable finding.",
                    _ => "Choose a local execution record. Provider reports never masquerade as local runs.",
                }.into();
            }
            Action::NextLink => {
                self.workflow.link = (self.workflow.link + 1) % self.workflow_links().len().max(1)
            }
            Action::OpenLink => {
                let reference = self
                    .workflow_links()
                    .get(self.workflow.link)
                    .cloned()
                    .ok_or("No attached evidence link selected")?;
                self.workflow_open_reference(reference);
            }
            Action::Diagnostic => {
                if self.planning.busy() || self.execution.busy() {
                    return Err("Finish or cancel active work before a Jolteon handoff.".into());
                }
                self.set_form(2);
                self.changed_task();
                self.execution.open(self.task_id);
                self.open_overlay(Overlay::Execution);
                self.begin_edit_field(crate::store::EditorField::Command);
                self.notice = "Jolteon diagnostic handoff: edit an explicit command; normal preview and approval remain required.".into();
            }
            Action::Fix => self.workflow_fix()?,
            Action::SaveRefactor => {
                self.workflow_baseline_eligible(
                    view.run(&self.workflow.baseline).ok_or(
                        "Choose an eligible retained baseline or request a fresh baseline",
                    )?,
                )?;
                self.workflows.set_refactor(
                    &self.workflow.contract,
                    self.workflow.hotspot.clone(),
                    &self.workflow.baseline,
                    &view,
                )?;
                self.record_artifact(
                    crate::workspace::records::ArtifactKind::Refactor,
                    "refactor",
                );
                self.changed_task();
                self.workflow.notice =
                    "Bounded contract and baseline recorded. No files changed.".into();
            }
            Action::Compare => {
                let comparison = self.workflows.compare_refactor(&view);
                self.workflow.comparison_summary = format!(
                    "Observed comparison: {}\nBaseline {}: {:?} ms\nPost-change {}: {:?} ms\nSingle-run times cannot establish speedup.",
                    comparison.reason,
                    comparison.baseline_run.as_deref().unwrap_or("missing"),
                    comparison.baseline_ms,
                    comparison.post_run.as_deref().unwrap_or("missing"),
                    comparison.post_ms
                );
                self.workflow.comparison_fingerprint = current.fingerprint.clone();
                self.workflow.notice = comparison.reason;
                self.workflow.scroll = 0;
            }
            Action::OpenBaseline | Action::OpenPost => {
                let refactor = self
                    .workflows
                    .refactor
                    .as_ref()
                    .ok_or("Record a bounded refactor first")?;
                let run_id = if matches!(action, Action::OpenBaseline) {
                    &refactor.baseline_run
                } else {
                    refactor
                        .post_run
                        .as_ref()
                        .ok_or("Select a post-change run first")?
                };
                self.workflow_open_reference(EvidenceRef::Run {
                    task_id: self.task_id,
                    run_id: run_id.clone(),
                });
            }
            Action::NextCriterion => {
                self.workflow.criterion = (self.workflow.criterion + 1) % self.criteria.len().max(1)
            }
            Action::AddFinding => {
                let criterion = self
                    .criteria
                    .get(self.workflow.criterion)
                    .ok_or("Add current task acceptance criteria first")?;
                self.workflows.add_finding(
                    &criterion.text,
                    &self.workflow.finding_text,
                    &self.workflow.action,
                    self.workflow.finding_evidence.clone(),
                    &view,
                )?;
                self.workflow.finding = self.workflows.findings.len().saturating_sub(1);
                self.workflow.finding_text.clear();
                self.workflow.action.clear();
                self.workflow.finding_evidence.clear();
                self.changed_task();
                self.workflow.notice =
                    "Actionable finding recorded. It remains unresolved until explicitly resolved."
                        .into();
            }
            Action::NextFinding => {
                self.workflow.finding =
                    (self.workflow.finding + 1) % self.workflows.findings.len().max(1);
                self.workflow.link = 0;
            }
            Action::ResolveFinding => {
                let id = self
                    .workflows
                    .findings
                    .get(self.workflow.finding)
                    .ok_or("Select a finding first")?
                    .id;
                self.workflows.resolve_finding(id, &self.workflow.action)?;
                self.changed_task();
                self.workflow.notice = "User resolution recorded; acceptance still requires current criterion evidence.".into();
            }
            Action::Review => {
                self.stage_relay(super::relays::RelayRequest {
                    target_form: 7,
                    text: format!("Fresh independent re-review of task {}.\nRequirements:\n{}\nInspect the current patch and freshly selected evidence through managed Review. Implementation history, sessions and grants are not review context. Findings remain unresolved until a fresh verdict and explicit resolution.", self.task_id, self.criteria.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n")),
                    paths: Vec::new(),
                    source: super::relays::RelaySource::Review,
                    command: None,
                })?;
            }
            Action::Verdict(verdict) => {
                let criteria = self
                    .workflows
                    .review
                    .as_ref()
                    .map(|review| review.criteria.clone())
                    .unwrap_or_default();
                let previous = self.workflows.clone();
                self.workflows.record_review(verdict, criteria, &view)?;
                if verdict == Verdict::Accept {
                    let texts: Vec<_> = self
                        .criteria
                        .iter()
                        .map(|criterion| criterion.text.clone())
                        .collect();
                    let guard = self.workflows.review_guard(&texts, &view);
                    if guard.verdict != Verdict::Accept {
                        self.workflows = previous;
                        return Err(format!("Accept blocked: {}", guard.reasons.join("; ")));
                    }
                }
                self.record_artifact(
                    crate::workspace::records::ArtifactKind::Review,
                    self.workflows.review_generation.to_string(),
                );
                self.changed_task();
                self.workflow.notice = format!(
                    "Recorded {:?}; task completion is a separate explicit action.",
                    verdict
                );
            }
            Action::LoadRecipe => {
                self.workflow.recipe = None;
                self.workflow.reviewed = false;
                self.workflow.affirmative = false;
                let path = std::path::Path::new(&self.workflow.recipe_path);
                let path = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    std::path::Path::new(&self.cwd).join(path)
                };
                self.workflow.recipe = Some(workflows::Recipe::read_file(&path)?);
                self.workflow.notice =
                    "Recipe loaded read-only. Choose Preview; nothing applied or executed.".into();
            }
            Action::PreviewRecipe => {
                if self.workflow.recipe.is_none() {
                    self.workflow.recipe = self
                        .workflows
                        .recipe
                        .as_ref()
                        .map(|applied| applied.recipe.clone());
                }
                let preview = self
                    .workflow
                    .recipe
                    .as_ref()
                    .ok_or("Load a valid local recipe first")?
                    .preview(self.task_id)?;
                self.workflow.recipe_preview_text = format!(
                    "{}\nApplication: add missing completion criteria as unverified; retain the full plan and context selection proposals in this desk; set a bounded Next action referring to the staged plan. Existing evidence, completed criteria, commands, model selection and grants are unchanged. No sources are read or pinned automatically.",
                    preview.text()
                );
                self.workflow.criteria_preview = false;
                self.workflow.preview = true;
                self.workflow.affirmative = false;
                self.workflow.reviewed = false;
                self.workflow.scroll = 0;
                self.workflow.notice =
                    "Untrusted recipe preview. Stay is selected. Applying stages task plan only."
                        .into();
            }
            Action::DecisionCriteria => {
                let decision = self.workflows.chosen_decision.clone().ok_or("No canonical chosen decision. In Espeon, select a complete proposal, /decision, then explicitly save Next action first.")?;
                self.workflow.criteria_text = decision
                    .text
                    .lines()
                    .filter(|line| {
                        !line.trim().is_empty() && !line.starts_with("[User-selected, unverified ")
                    })
                    .map(|line| line.trim().trim_start_matches(['-', '*', ' ']))
                    .collect::<Vec<_>>()
                    .join("\n");
                self.workflow.criteria_decision = Some(decision);
                self.workflow.edit(Field::Criteria);
            }
            Action::EditCriteria => {
                if self.workflow.criteria_decision.is_none() {
                    return Err("Add a chosen decision as criteria first.".into());
                }
                self.workflow.edit(Field::Criteria);
            }
            Action::ConfirmCriteria => {
                self.workflow_criteria_items()?;
                self.workflow.criteria_preview = true;
                self.workflow.preview = true;
                self.workflow.affirmative = false;
                self.workflow.reviewed = false;
                self.workflow.scroll = 0;
            }
            Action::RelayDecision => {
                let decision = self
                    .workflows
                    .chosen_decision
                    .as_ref()
                    .ok_or("Choose and save an Espeon decision first")?;
                self.stage_relay(super::relays::RelayRequest { target_form: 0, text: format!("Chosen decision {}:\n{}\nReview and edit acceptance criteria locally; this is not verified evidence.", decision.id, decision.text), paths: Vec::new(), source: super::relays::RelaySource::Decision(decision.id.clone()), command: None })?;
            }
            Action::FreshBaseline => {
                if self.planning.busy()
                    || self.execution.active.as_ref().is_some_and(|r| {
                        matches!(
                            r.state,
                            crate::workspace::runner::RunState::Running
                                | crate::workspace::runner::RunState::Cancelling
                        )
                    })
                {
                    return Err("Finish or explicitly cancel active work before requesting a fresh baseline.".into());
                }
                self.workflow.baseline.clear();
                self.workflow.notice = "No retained baseline selected. Fresh baseline: edit the same check, review and approve in Jolteon, then return to select its passing run.".into();
                self.set_form(2);
                self.changed_task();
                self.begin_edit_field(crate::store::EditorField::Command);
            }
            Action::Refresh => {
                self.repo_refresh = true;
                self.repo_loading = true;
                self.workflow.notice =
                    "Fresh repository observation requested; no command or provider starts.".into();
            }
        }
        Ok(())
    }

    fn workflow_links(&self) -> Vec<EvidenceRef> {
        match self.workflow.tab {
            Tab::Diagnosis => self
                .workflows
                .hypotheses
                .get(self.workflow.hypothesis)
                .map(|hypothesis| {
                    hypothesis
                        .evidence
                        .iter()
                        .map(|link| link.reference.clone())
                        .collect()
                })
                .unwrap_or_default(),
            Tab::Review => self
                .workflows
                .findings
                .get(self.workflow.finding)
                .map(|finding| finding.evidence.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn workflow_open_reference(&mut self, reference: EvidenceRef) {
        match reference {
            EvidenceRef::Source {
                task_id,
                excerpt_id,
                ..
            } => {
                if task_id != self.task_id {
                    return;
                }
                if self.sources.busy() || self.planning.busy() {
                    self.workflow.notice = "Wait for the active source/provider operation before opening this exact link.".into();
                    return;
                }
                if let Some(index) = self
                    .sources
                    .basket
                    .iter()
                    .position(|excerpt| excerpt.id == excerpt_id)
                {
                    self.open_overlay(Overlay::Sources);
                    // Opening the ordinary drawer queues a catalogue. Replace that
                    // not-yet-dispatched read with this deliberate stored reference.
                    self.sources.pending = None;
                    self.sources.source_busy = false;
                    self.sources.await_refresh = false;
                    self.sources.basket_selection = index;
                    let excerpt = &self.sources.basket[index];
                    self.sources.detail = format!(
                        "Stored source reference {}:{}-{}\nCaptured evidence / {}. Refresh the source separately to observe later changes.\n\n{}",
                        excerpt.path,
                        excerpt.start_line,
                        excerpt.end_line,
                        if excerpt.available {
                            "available"
                        } else {
                            "UNAVAILABLE"
                        },
                        excerpt.text
                    );
                    self.sources.preview = Some(excerpt.clone());
                    self.sources.mode = super::sources::Mode::Preview;
                    self.sources.status =
                        "Linked source evidence / read-only inspection / Esc Back".into();
                    self.sources.error.clear();
                } else {
                    self.workflow.notice = "Source reference unavailable; it was not replaced with unrelated evidence.".into();
                }
            }
            EvidenceRef::Run { task_id, run_id } => {
                if task_id != self.task_id {
                    return;
                }
                if let Some(index) = self
                    .execution
                    .history
                    .iter()
                    .position(|run| run.prepared.run_id == run_id)
                {
                    self.execution.open(self.task_id);
                    self.execution.history_index = index;
                    self.execution.selected_run = Some(run_id);
                    self.execution.view = super::execution::View::History;
                    self.execution.command("open");
                    self.open_overlay(Overlay::Execution);
                } else {
                    self.workflow.notice =
                        "Run reference unavailable; no execution was started.".into();
                }
            }
        }
    }

    fn workflow_fix(&mut self) -> Result<(), String> {
        use super::relays::{RelayRequest, RelaySource};
        let (source, text, references) = match self.workflow.tab {
            Tab::Diagnosis => {
                let hypothesis = self
                    .workflows
                    .hypotheses
                    .get(self.workflow.hypothesis)
                    .ok_or("Select a supported hypothesis first")?;
                (
                    RelaySource::Hypothesis(hypothesis.id),
                    format!(
                        "Fix hypothesis {}: {}\nEvidence:\n{}",
                        hypothesis.id,
                        hypothesis.explanation,
                        hypothesis
                            .evidence
                            .iter()
                            .map(|e| e.reference.label())
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                    hypothesis
                        .evidence
                        .iter()
                        .map(|e| e.reference.clone())
                        .collect::<Vec<_>>(),
                )
            }
            Tab::Review => {
                let finding = self
                    .workflows
                    .findings
                    .get(self.workflow.finding)
                    .ok_or("Select a request-changes finding first")?;
                (
                    RelaySource::Finding(finding.id),
                    format!(
                        "Fix review finding {}\nCriterion: {}\nFinding: {}\nAction: {}\nEvidence:\n{}",
                        finding.id,
                        finding.criterion,
                        finding.detail,
                        finding.next_action,
                        finding
                            .evidence
                            .iter()
                            .map(EvidenceRef::label)
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                    finding.evidence.clone(),
                )
            }
            Tab::Refactor => {
                let refactor = self
                    .workflows
                    .refactor
                    .as_ref()
                    .ok_or("Record contract, hotspot and baseline first")?;
                (
                    RelaySource::Review,
                    format!(
                        "Behavior-preserving refactor. Contract: {}\nBaseline: {}\nHotspot:\n{}\nPreserve criteria; use the same check after the patch. Single-run timings are not speedup evidence.",
                        refactor.contract,
                        refactor.baseline_run,
                        refactor
                            .hotspot
                            .iter()
                            .map(EvidenceRef::label)
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                    refactor.hotspot.clone(),
                )
            }
            Tab::Recipe => {
                return Err("Choose a supported diagnosis or request-changes finding.".into());
            }
        };
        let mut paths = Vec::new();
        for reference in references {
            if let EvidenceRef::Source { path, .. } = reference {
                let path = std::path::PathBuf::from(path);
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }
        self.stage_relay(RelayRequest {
            target_form: if self.workflow.tab == Tab::Refactor {
                6
            } else {
                3
            },
            text,
            paths,
            source,
            command: None,
        })
    }

    pub fn workflow_commit_decision(&mut self) {
        if !self.records_can_switch() {
            self.workflow.notice =
                "Wait for the local record batch to commit before saving the decision.".into();
            return;
        }
        if self.workflow.pending_decision.take().is_none() || self.form() != 4 {
            return;
        }
        if self.next_action.trim().is_empty() {
            self.workflow.notice =
                "Empty Next action was not accepted as a chosen decision.".into();
            return;
        }
        use sha2::{Digest, Sha256};
        let id = format!("decision:{:x}", Sha256::digest(self.next_action.as_bytes()));
        self.workflows.chosen_decision = Some(workflows::ChosenDecision {
            id: id.clone(),
            text: self.next_action.clone(),
        });
        self.record_artifact(crate::workspace::records::ArtifactKind::Decision, id);
    }

    fn workflow_criteria_items(&self) -> Result<Vec<String>, String> {
        let decision = self
            .workflow
            .criteria_decision
            .as_ref()
            .ok_or("Select a chosen decision first")?;
        if self.workflows.chosen_decision.as_ref() != Some(decision) {
            return Err("Chosen decision changed; stage its criteria again.".into());
        }
        let items: Vec<_> = self
            .workflow
            .criteria_text
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        if items.is_empty() || items.len() > 64 || items.iter().any(|s| s.len() > 256) {
            return Err("Criteria require 1–64 editable lines, each at most 256 UTF-8 bytes; nothing trimmed.".into());
        }
        Ok(items)
    }

    pub fn workflow_criteria_preview(&self) -> String {
        let Some(decision) = &self.workflow.criteria_decision else {
            return "No decision selected.".into();
        };
        let mut seen = self
            .criteria
            .iter()
            .map(|c| c.text.trim().to_owned())
            .collect::<Vec<_>>();
        let items = self
            .workflow
            .criteria_text
            .lines()
            .filter(|s| !s.trim().is_empty())
            .map(|s| {
                let key = s.trim().to_owned();
                let duplicate = seen.contains(&key);
                seen.push(key);
                format!(
                    "{} {}",
                    if duplicate {
                        "[duplicate: skipped]"
                    } else {
                        "[new / unverified]"
                    },
                    s.trim()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "Add editable criteria linked to {}.\nNo execution, permissions or verification.\n{items}\nMet/total now: {}/{}.\nEsc cancels all additions; edit proposal before confirming.",
            decision.id,
            self.criteria.iter().filter(|c| c.met).count(),
            self.criteria.len()
        )
    }

    fn workflow_apply_criteria(&mut self) -> Result<(), String> {
        if self.planning_any_busy() || self.execution_any_busy() {
            return Err(
                "Finish owned work before changing committed criteria; the criteria draft is kept."
                    .into(),
            );
        }
        if self.complete {
            return Err("Reopen the task before adding criteria.".into());
        }
        let items = self.workflow_criteria_items()?;
        let decision = self
            .workflow
            .criteria_decision
            .clone()
            .ok_or("Decision unavailable")?;
        let mut criteria = self.criteria.clone();
        let mut state = self.workflows.clone();
        for item in items {
            if criteria.iter().any(|c| c.text.trim() == item) {
                continue;
            }
            criteria.push(crate::store::Criterion {
                text: item.clone(),
                met: false,
            });
            state.criterion_origins.push(workflows::CriterionOrigin {
                criterion: item,
                decision_id: decision.id.clone(),
                decision_text: decision.text.clone(),
            });
        }
        if criteria.len() > 64 {
            return Err("Merged task exceeds 64 criteria; nothing changed.".into());
        }
        state.validate(self.task_id)?;
        self.criteria = criteria;
        self.workflows = state;
        self.workflow.preview = false;
        self.workflow.criteria_preview = false;
        self.workflow.affirmative = false;
        self.workflow.notice = format!(
            "Criteria confirmed with decision provenance; {}/{} met. New criteria remain unverified.",
            self.criteria.iter().filter(|c| c.met).count(),
            self.criteria.len()
        );
        self.changed_task();
        Ok(())
    }

    /// Text is the existing criterion identity. Preserve exact identities across
    /// reorder; infer a rename only for one changed row with all retained rows
    /// unmoved. Never position-bind a removed criterion to unrelated reordered text.
    pub(super) fn workflow_reconcile_criteria_origins(
        &mut self,
        next: &[crate::store::Criterion],
    ) -> Result<(), String> {
        let removed: Vec<_> = self
            .criteria
            .iter()
            .enumerate()
            .filter(|(_, old)| !next.iter().any(|new| new.text == old.text))
            .collect();
        let added: Vec<_> = next
            .iter()
            .enumerate()
            .filter(|(_, new)| !self.criteria.iter().any(|old| old.text == new.text))
            .collect();
        let rename = match (removed.as_slice(), added.as_slice()) {
            ([(old_index, old)], [(new_index, new)])
                if old_index == new_index
                    && self.criteria.iter().enumerate().all(|(index, old)| {
                        !next.iter().any(|new| new.text == old.text)
                            || next.get(index).is_some_and(|new| new.text == old.text)
                    }) =>
            {
                Some((old.text.as_str(), new.text.as_str()))
            }
            _ => None,
        };
        let mut state = self.workflows.clone();
        state.criterion_origins.clear();
        for origin in &self.workflows.criterion_origins {
            if !self.criteria.iter().any(|old| old.text == origin.criterion) {
                continue;
            }
            let exact = next
                .iter()
                .filter(|new| new.text == origin.criterion)
                .count();
            if exact > 1 {
                return Err("Duplicate decision-linked criteria need distinct text; criteria and provenance are unchanged.".into());
            }
            let text = if exact == 1 {
                Some(origin.criterion.as_str())
            } else {
                rename
                    .filter(|(old, _)| *old == origin.criterion)
                    .map(|(_, new)| new)
            };
            let Some(text) = text else {
                continue;
            };
            if state
                .criterion_origins
                .iter()
                .any(|existing| existing.criterion == text)
            {
                return Err("Ambiguous duplicate decision provenance; criteria and provenance are unchanged.".into());
            }
            state.criterion_origins.push(workflows::CriterionOrigin {
                criterion: text.to_owned(),
                decision_id: origin.decision_id.clone(),
                decision_text: origin.decision_text.clone(),
            });
        }
        state.validate(self.task_id)?;
        self.workflows = state;
        Ok(())
    }

    fn workflow_baseline_eligible(
        &self,
        run: &crate::workspace::runner::RunRecord,
    ) -> Result<(), String> {
        if run.revision_moving {
            return Err("Revision moving: this run overlapped editing and is ineligible as a stable baseline.".into());
        }
        let expected = self
            .workflow
            .baseline_expected
            .as_ref()
            .ok_or_else(|| self.workflow.baseline_error.clone())?;
        workflows::baseline_eligibility(run, expected, &self.workflow_fingerprint())
    }

    fn workflow_refresh_baseline_identity(&mut self) {
        self.workflow.baseline_expected = None;
        let result = (|| {
            let spec = super::execution::parse_command(&self.command).map_err(|_| "Choose the intended check with /command-edit before selecting a retained baseline".to_string())?;
            let mut expected = self.execution.prepared.as_ref().filter(|p| p.spec == spec).or_else(|| self.execution.history.iter().filter(|r| r.prepared.task_id == self.task_id && r.prepared.spec == spec && r.prepared.cwd == std::path::Path::new(&self.cwd)).max_by_key(|r| r.prepared.prepared_at_ms).map(|r| &r.prepared)).cloned().ok_or("Prepare the intended check in Jolteon first, or run it once to establish command identity".to_string())?;
            let program = match &spec {
                crate::workspace::runner::CommandSpec::Direct { program, .. } => program,
                crate::workspace::runner::CommandSpec::Shell { executable, .. } => executable,
            };
            let path = crate::workspace::runner::resolve_executable(
                program,
                std::path::Path::new(&self.cwd),
            )?;
            expected.executable = crate::workspace::runner::executable_identity(&path)
                .map_err(|e| format!("Baseline executable identity unavailable: {e}"))?;
            expected.cwd_identity = crate::workspace::runner::native::directory_identity(
                std::path::Path::new(&self.cwd),
            )
            .map_err(|e| e.to_string())?;
            Ok::<_, String>(expected)
        })();
        match result {
            Ok(expected) => {
                self.workflow.baseline_expected = Some(expected);
                self.workflow.baseline_error.clear();
            }
            Err(error) => self.workflow.baseline_error = error,
        }
    }

    fn workflow_refresh_addressed(&mut self) {
        if self.repo_loading || self.repo_refresh {
            return;
        }
        let Some(report) = &self.repository else {
            return;
        };
        if !report.current.complete || !report.current.stable {
            return;
        }
        let fingerprint = self.workflow_fingerprint().fingerprint;
        if fingerprint.is_none() || self.workflow.addressed_fingerprint == fingerprint {
            return;
        }
        self.workflow.addressed_fingerprint = fingerprint;
        let mut changed = false;
        for relay in &mut self.workflows.relayed_findings {
            if relay.addressed {
                continue;
            }
            for range in &relay.ranges {
                if report
                    .current
                    .files
                    .get(std::path::Path::new(&range.path))
                    .is_some_and(|file| {
                        matches!(file.state, crate::workspace::repo::FileState::Missing)
                    })
                {
                    relay.addressed = true;
                    changed = true;
                    break;
                }
                let Ok((bytes, _)) = crate::workspace::repo::read_source(
                    &report.repository.root,
                    std::path::Path::new(&range.path),
                    12 * 1024,
                    &std::sync::atomic::AtomicBool::new(false),
                ) else {
                    if workflows::confirmed_source_absence(
                        &report.repository.root,
                        std::path::Path::new(&range.path),
                    ) {
                        relay.addressed = true;
                        changed = true;
                        break;
                    }
                    continue;
                };
                let Ok(text) = std::str::from_utf8(&bytes) else {
                    continue;
                };
                if workflows::range_touched(&range.text, text, range.start, range.end) {
                    relay.addressed = true;
                    changed = true;
                    break;
                }
            }
        }
        if changed {
            self.workflows.review_stale = true;
            self.changed_task();
        }
    }

    pub fn workflow_recipe_preview(&self) -> &str {
        &self.workflow.recipe_preview_text
    }

    fn workflow_apply_recipe(&mut self) -> Result<(), String> {
        if self.complete {
            return Err("Reopen the completed task explicitly before applying a new plan.".into());
        }
        if self.planning_any_busy() || self.execution_any_pending() {
            return Err("Finish or cancel active work before applying a recipe plan.".into());
        }
        let recipe = self
            .workflow
            .recipe
            .as_ref()
            .ok_or("Load and preview a recipe first")?;
        let preview = recipe.preview(self.task_id)?;
        let additions = preview
            .recipe()
            .completion_criteria
            .iter()
            .filter(|criterion| {
                !self
                    .criteria
                    .iter()
                    .any(|existing| existing.text == **criterion)
            })
            .count();
        if preview
            .recipe()
            .completion_criteria
            .iter()
            .any(|criterion| criterion.len() > 256)
            || self.criteria.len().saturating_add(additions) > 64
        {
            return Err("Recipe cannot be staged: criteria must fit 256 UTF-8 bytes each and the merged task must fit 64 criteria. Nothing was applied.".into());
        }
        self.workflows.apply_recipe(&preview, self.task_id)?;
        for criterion in &preview.recipe().completion_criteria {
            if !self
                .criteria
                .iter()
                .any(|existing| existing.text == *criterion)
            {
                self.criteria.push(crate::store::Criterion {
                    text: criterion.clone(),
                    met: false,
                });
            }
        }
        self.next_action = format!(
            "Recipe '{}' staged in Specialist desk > Recipe. Inspect the full stored plan, scope and context selection proposals; follow its {} form steps. Validation commands require normal Jolteon approval; source writes require ordinary reviewed scope. No permissions were granted.",
            preview.recipe().name,
            preview.recipe().steps.len()
        );
        self.changed_task();
        self.workflow.preview = false;
        self.workflow.affirmative = false;
        self.workflow.notice = "Recipe plan staged: criteria and Next action/context proposals only. No command, model, write or grant.".into();
        Ok(())
    }

    pub fn workflow_rows(&self, language: crate::i18n::Language) -> Vec<String> {
        if let Some(chooser) = self.workflow.chooser {
            let choices = self.workflow_choices(chooser);
            return if choices.is_empty() {
                vec!["No evidence: pin a source / approve a Jolteon run".into()]
            } else {
                choices.into_iter().map(|(_, label)| label).collect()
            };
        }
        actions(self.workflow.tab)
            .iter()
            .copied()
            .map(|(action, original_label)| {
                let command_key = crate::command_help::key(action.command());
                // Shared commands have distinct effects in each specialist tab.
                let label_key = match (self.workflow.tab, action) {
                    (Tab::Diagnosis, Action::Diagnostic) => {
                        Some(crate::i18n::Message::DesksWorkflowsDiagnosisDiagnostic)
                    }
                    (Tab::Refactor, Action::Diagnostic) => {
                        Some(crate::i18n::Message::DesksWorkflowsRefactorDiagnostic)
                    }
                    (Tab::Diagnosis, Action::Fix) => {
                        Some(crate::i18n::Message::DesksWorkflowsDiagnosisFix)
                    }
                    (Tab::Refactor, Action::Fix) => {
                        Some(crate::i18n::Message::DesksWorkflowsRefactorFix)
                    }
                    (Tab::Review, Action::Fix) => {
                        Some(crate::i18n::Message::DesksWorkflowsReviewFix)
                    }
                    _ => command_key,
                };
                let label = label_key
                    .map(|key| language.text(key))
                    .unwrap_or(original_label);
                match action {
                    Action::Edit(field) => {
                        format!("{label}: {}", self.workflow.value(field).replace('\n', " "))
                    }
                    Action::NextHypothesis => format!(
                        "{label} ({}/{})",
                        if self.workflows.hypotheses.is_empty() {
                            0
                        } else {
                            self.workflow.hypothesis + 1
                        },
                        self.workflows.hypotheses.len()
                    ),
                    Action::NextFinding => format!(
                        "{label} ({}/{})",
                        if self.workflows.findings.is_empty() {
                            0
                        } else {
                            self.workflow.finding + 1
                        },
                        self.workflows.findings.len()
                    ),
                    Action::NextCriterion => format!(
                        "{label}: {}",
                        self.criteria
                            .get(self.workflow.criterion)
                            .map_or("No criteria", |criterion| &criterion.text)
                    ),
                    Action::Choose(Chooser::Hotspot) => {
                        format!("{label} ({} selected)", self.workflow.hotspot.len())
                    }
                    Action::Choose(Chooser::Baseline) => format!(
                        "{label}: {}",
                        if self.workflow.baseline.is_empty() {
                            "not selected"
                        } else {
                            &self.workflow.baseline
                        }
                    ),
                    Action::Fix if self.workflow.tab == Tab::Diagnosis => {
                        let supported = self
                            .workflows
                            .hypotheses
                            .get(self.workflow.hypothesis)
                            .is_some_and(|h| {
                                self.workflow.hypothesis_states.iter().any(|(id, status)| {
                                    *id == h.id && *status == workflows::HypothesisStatus::Supported
                                })
                            });
                        if supported {
                            label.into()
                        } else {
                            format!("{label} [disabled: supported hypothesis required]")
                        }
                    }
                    Action::Fix
                        if self.workflow.tab == Tab::Review
                            && !self
                                .workflows
                                .review
                                .as_ref()
                                .is_some_and(|r| r.requested == Verdict::RequestChanges) =>
                    {
                        format!("{label} [disabled: Request changes verdict required]")
                    }
                    _ => label.into(),
                }
            })
            .collect()
    }

    pub fn workflow_details(&self) -> String {
        if let Some(chooser) = self.workflow.chooser {
            let relation = if chooser == Chooser::Hypothesis {
                if self.workflow.refuting {
                    "Refuting / user interpretation\n"
                } else {
                    "Supporting / user interpretation\n"
                }
            } else {
                ""
            };
            return if let Some((reference, label)) =
                self.workflow_choices(chooser).get(self.workflow.evidence)
            {
                format!(
                    "{relation}{label}\n{}\n{:?}\nOpen deliberately with o to inspect exact output/source. Failed is an observation, not a passing check.",
                    reference.label(),
                    self.workflow_status(reference)
                )
            } else {
                format!(
                    "{relation}No available committed evidence. Open Sources to pin a range or Jolteon to approve a diagnostic run."
                )
            };
        }
        match self.workflow.tab {
            Tab::Diagnosis => {
                if let Some(hypothesis) = self.workflows.hypotheses.get(self.workflow.hypothesis) {
                    let status = self.workflow.hypothesis_states.iter().find(|(id, _)| *id == hypothesis.id)
                        .map_or(workflows::HypothesisStatus::Unresolved, |(_, status)| *status);
                    let links = hypothesis.evidence.iter().enumerate().map(|(index, link)| format!("{} {:?} / {:?} / {}", if index == self.workflow.link { ">" } else { " " }, link.relation, self.workflow_status(&link.reference), link.reference.label())).collect::<Vec<_>>().join("\n");
                    format!("Hypothesis {} / {:?}\nInference: {}\n{}\nSupported requires current source + run; missing proof remains unresolved. Refuted requires a current refuting run.", hypothesis.id, status, hypothesis.explanation, links)
                } else { "No hypotheses yet. Edit an explanation, then Add hypothesis. Evidence links distinguish support, refutation and unresolved inference.".into() }
            }
            Tab::Refactor => {
                let refactor = self.workflows.refactor.as_ref();
                let contract = refactor.map_or("Not recorded", |refactor| refactor.contract.as_str());
                let baseline = refactor.map_or("missing", |refactor| refactor.baseline_run.as_str());
                let post = refactor.and_then(|refactor| refactor.post_run.as_deref()).unwrap_or("missing");
                let comparison = if self.workflow.comparison_summary.is_empty() {
                    "Comparison not current. Choose Compare for fresh evidence validation."
                } else { &self.workflow.comparison_summary };
                format!("Recorded contract: {contract}\nSelected hotspot draft:\n{}\nBaseline: {baseline}\nPost-change: {post}\n{comparison}\nOpen both outputs deliberately. Claims cover only the selected check.", self.workflow.hotspot.iter().map(EvidenceRef::label).collect::<Vec<_>>().join("\n"))
            }
            Tab::Review => {
                let criterion = self.criteria.get(self.workflow.criterion).map_or("No acceptance criteria", |criterion| criterion.text.as_str());
                let coverage = self.workflows.review.as_ref().and_then(|review| review.criteria.iter().find(|entry| entry.criterion == criterion))
                    .map(|entry| entry.evidence.iter().map(|reference| format!("{:?}: {}", self.workflow_status(reference), reference.label())).collect::<Vec<_>>().join("\n")).unwrap_or_else(|| "No linked coverage / inconclusive".into());
                let finding = self.workflows.findings.get(self.workflow.finding).map(|finding| {
                    let addressed = self.workflows.relayed_findings.iter().find(|r| r.finding == finding.id).filter(|r| r.addressed).map_or("", |r| if self.workflows.review_generation <= r.review_generation { "addressed, needs re-review / verdict stale\n" } else { "addressed / new verdict recorded; explicit resolution still required\n" });
                    format!("{addressed}Finding {} / {}: {}\nAction: {}\nResolution: {}\n{}", finding.id, finding.criterion, finding.detail, finding.next_action, finding.resolution.as_deref().unwrap_or("unresolved"),
                        finding.evidence.iter().enumerate().map(|(index, reference)| format!("{} {:?}: {}", if index == self.workflow.link { ">" } else { " " }, self.workflow_status(reference), reference.label())).collect::<Vec<_>>().join("\n"))
                }).unwrap_or_else(|| "No recorded findings. Missing verification is not acceptance.".into());
                format!("Selected criterion: {criterion}\n{coverage}\n{}\n{finding}\nFinding draft evidence: {} links. Managed review has no source-writing tools.", self.workflow.review_summary, self.workflow.finding_evidence.len())
            }
            Tab::Recipe => self.workflow.recipe.as_ref().or_else(|| self.workflows.recipe.as_ref().map(|applied| &applied.recipe)).map(|recipe| format!("Loaded or staged recipe: {}\nSchema {} / {} steps / {} criteria\nChoose full preview before Apply. Commands and scope are proposals, never permissions.", recipe.name, recipe.schema_version, recipe.steps.len(), recipe.completion_criteria.len()))
                .unwrap_or_else(|| "Enter a literal local TOML path. Load validates strict schema 1 without running anything. Preview exposes every step, scope, context proposal, validation command and criterion; initial choice is Stay.".into()),
        }
    }
}

#[cfg(test)]
mod tests;
