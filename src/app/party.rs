//! Party roster: the derived view of every partner and what it is doing.
//! Status is computed from live request/approval state, never stored.
use super::*;

/// Local-only text and caret, independent of every partner's task draft.
pub struct NicknameEditor {
    pub partner: i64,
    pub text: String,
    pub cursor: usize,
    pub error: String,
    /// A refused insertion must not turn Enter into a save of the old/empty text.
    insertion_rejected: bool,
}

/// A roster row's activity state, derived per frame from live ownership.
/// Provider requests and local commands stay with their starting partner,
/// independently of whoever is in front now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartnerStatus {
    Idle,
    Working,
    Waiting,
}

impl PartnerStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Waiting => "waiting for approval",
        }
    }
}

impl App {
    pub(super) fn open_nickname(&mut self, partner: i64) {
        if self.party_request.is_some() || self.party_inflight {
            self.notice = "Wait for the current party change to finish.".into();
            return;
        }
        if self.planning.busy() || self.confirmation.is_some() || self.transitioning {
            self.notice = "Finish the current request or review before editing a nickname.".into();
            return;
        }
        let Some(partner) = self.partners().into_iter().find(|p| p.id == partner) else {
            return;
        };
        let text = partner.nickname.unwrap_or_default();
        self.nickname_editor = Some(NicknameEditor {
            partner: partner.id,
            cursor: text.len(),
            text,
            error: String::new(),
            insertion_rejected: false,
        });
    }

    pub(super) fn nickname_paste(&mut self, text: &str) -> bool {
        if text.is_empty() || self.nickname_pending.is_some() {
            return false;
        }
        let Some(editor) = &mut self.nickname_editor else {
            return false;
        };
        // Refuse the complete insertion rather than silently saving a clipped name.
        if text.len() > 512 {
            editor.error = "Nickname exceeds the 512-byte storage limit.".into();
            editor.insertion_rejected = true;
            return true;
        }
        let mut candidate = editor.text.clone();
        candidate.insert_str(editor.cursor, text);
        if let Err(error) = store::validate_nickname(&candidate) {
            editor.error = error;
            editor.insertion_rejected = true;
            return true;
        }
        editor.text = candidate;
        editor.cursor = editor
            .text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|index| *index >= editor.cursor + text.len())
            .unwrap_or(editor.text.len());
        editor.error.clear();
        editor.insertion_rejected = false;
        true
    }

    pub(super) fn nickname_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Esc {
            self.nickname_editor = None;
            return true;
        }
        if self.nickname_pending.is_some()
            || key.kind == KeyEventKind::Repeat && key.code == KeyCode::Enter
        {
            return true;
        }
        let Some(editor) = &mut self.nickname_editor else {
            return false;
        };
        let before = editor.text[..editor.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(index, _)| index);
        let after = editor
            .text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|index| *index > editor.cursor)
            .unwrap_or(editor.text.len());
        match key.code {
            KeyCode::Enter => {
                if editor.insertion_rejected {
                    return true;
                }
                let candidate = editor.text.trim();
                if !candidate.is_empty()
                    && let Err(error) = store::validate_nickname(candidate)
                {
                    editor.error = error;
                    return true;
                }
                self.party_request = Some(store::PartyRequest::Nickname {
                    partner: editor.partner,
                    nickname: (!candidate.is_empty()).then(|| candidate.to_owned()),
                });
                self.nickname_pending = Some(editor.partner);
                editor.error.clear();
            }
            KeyCode::Left => editor.cursor = before,
            KeyCode::Right => editor.cursor = after,
            KeyCode::Home => editor.cursor = 0,
            KeyCode::End => editor.cursor = editor.text.len(),
            KeyCode::Backspace | KeyCode::Delete => {
                let range = if key.code == KeyCode::Backspace {
                    before..editor.cursor
                } else {
                    editor.cursor..after
                };
                editor.text.replace_range(range, "");
                editor.cursor = editor
                    .text
                    .grapheme_indices(true)
                    .map(|(index, _)| index)
                    .chain(std::iter::once(editor.text.len()))
                    .take_while(|index| {
                        *index
                            <= if key.code == KeyCode::Backspace {
                                before
                            } else {
                                editor.cursor
                            }
                    })
                    .last()
                    .unwrap_or(0);
                editor.error.clear();
                editor.insertion_rejected = false;
            }
            KeyCode::Char(value)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                let mut bytes = [0; 4];
                return self.nickname_paste(value.encode_utf8(&mut bytes));
            }
            _ => {}
        }
        true
    }

    /// The party roster in id order. The lead partner row shows its live form:
    /// `set_form` mutates `self.lead` before a save lands; committed identity
    /// fields are refreshed separately by party acknowledgements.
    pub fn partners(&self) -> Vec<store::Partner> {
        let mut partners = self.party.clone();
        match partners.iter_mut().find(|p| p.id == self.lead.id) {
            Some(partner) => {
                partner.form = self.lead.form;
                partner.auto_form = self.lead.auto_form;
            }
            None => partners.insert(0, self.lead.clone()),
        }
        partners
    }

    /// Roster `Add` action. The store layer re-checks the six-seat cap inside
    /// its transaction; here a full or already-queued request only explains why.
    pub(super) fn request_add_partner(&mut self) {
        if self.party_request.is_some() || self.party_inflight {
            self.notice = "A partner is already joining; wait for storage.".into();
        } else if self.party.len() >= store::MAX_PARTNERS {
            self.notice = format!(
                "The party is full: a partner can hold at most {} members; release one first.",
                store::MAX_PARTNERS
            );
        } else {
            self.party_request = Some(store::PartyRequest::Add);
            self.notice = "Adding a new partner in Eevee form.".into();
        }
    }

    /// Roster navigation and actions: Enter/`l` (or `/lead`) fronts the selected
    /// partner, `a` seats a new one. Other keys still scroll the preview surface.
    pub(super) fn party_overlay_key(&mut self, key: KeyCode) -> bool {
        let partners = self.partners();
        let count = partners.len();
        if count == 0 {
            return false;
        }
        match key {
            KeyCode::Enter | KeyCode::Char('l') => {
                self.request_lead(partners[self.selected.min(count - 1)].id);
                true
            }
            KeyCode::Char('a') => {
                self.request_add_partner();
                true
            }
            KeyCode::Char('n') => {
                self.open_nickname(partners[self.selected.min(count - 1)].id);
                true
            }
            KeyCode::Up => {
                self.selected = self.selected.min(count - 1).saturating_sub(1);
                true
            }
            KeyCode::Down => {
                self.selected = (self.selected.min(count - 1) + 1).min(count - 1);
                true
            }
            KeyCode::Home => {
                self.selected = 0;
                true
            }
            KeyCode::End => {
                self.selected = count - 1;
                true
            }
            _ => self.scroll_key(key),
        }
    }

    /// `/lead` and `/lead <name>`: no argument opens the roster for a chosen
    /// front; an argument resolves a nickname or `Eevee #n` (also `#n`/bare id).
    pub(super) fn lead_command(&mut self, argument: &str) -> bool {
        if self.editing {
            // Same retention as command dispatch: the live draft stays local.
            let field = self.editor_field;
            self.retain_editor();
            self.composer_return = Some(field);
        }
        let argument = argument.trim();
        if argument.is_empty() {
            self.open_overlay(Overlay::Party);
            return true;
        }
        let partners = self.partners();
        let by_id = argument
            .strip_prefix('#')
            .unwrap_or(argument)
            .parse::<i64>()
            .ok()
            .and_then(|id| partners.iter().find(|partner| partner.id == id));
        let partner = by_id
            .or_else(|| {
                partners
                    .iter()
                    .find(|partner| Self::partner_label(partner).eq_ignore_ascii_case(argument))
            })
            .cloned();
        match partner {
            Some(partner) => {
                self.request_lead(partner.id);
                true
            }
            None => {
                self.notice = format!("No partner named \"{argument}\"; /party lists the roster.");
                true
            }
        }
    }

    /// Front `partner` as this task's lead (PT05A). The lead's own stored form
    /// becomes the active form, the picker follows it, and the choice is
    /// persisted through the normal task save (`Snapshot.lead_partner` plus the
    /// partner row's `form` written by `set_partner_form`).
    pub(super) fn request_lead(&mut self, partner: i64) {
        if partner == self.lead.id {
            self.notice = format!(
                "{} already leads this task.",
                Self::partner_label(&self.lead)
            );
            return;
        }
        if let Some(reason) = self.lead_switch_blocked() {
            self.notice = reason;
            return;
        }
        let Some(partner) = self
            .partners()
            .into_iter()
            .find(|candidate| candidate.id == partner)
        else {
            self.notice = "That partner is not in the party.".into();
            return;
        };
        // Fronting another partner never transfers live work: the request
        // slot, its events and any pending approval stay with their owner.
        let label = Self::partner_label(&partner);
        self.switch_lead(partner);
        self.notice = format!(
            "{label} leads this task; its {} form is active.",
            FORMS[self.form().min(FORMS.len() - 1)].name
        );
        self.changed_task();
    }

    /// Retain the outgoing partner's composer and restore only the incoming
    /// partner's draft, caret and previous-form memory.
    fn switch_lead(&mut self, partner: store::Partner) {
        self.clear_carry_over();
        self.evolution.transition = None;
        self.partner_drafts
            .insert(self.lead.id, self.partner_draft());
        self.front_planning(&partner);
        self.lead = partner;
        self.sync_front();
        self.form_selection = self.lead.form;
        self.preview_form = self.lead.form;
        let draft = self
            .partner_drafts
            .get(&self.lead.id)
            .cloned()
            .unwrap_or_default();
        self.previous_form = draft.previous_form;
        self.drafts = draft.drafts;
        self.input.clear();
        self.cursor = 0;
        self.editing = false;
        self.editor_field = draft.editor_field;
        self.edit_return = None;
        self.composer_return = None;
        if self.overlay == Some(Overlay::Party) {
            self.close_overlay();
        }
        if draft.editing {
            self.open_editor(draft.editor_field);
        }
        self.apply_relay_draft_role();
    }

    /// Why a lead switch is refused right now: only an in-flight roster write.
    /// A working partner keeps its own request slot while another leads.
    fn lead_switch_blocked(&self) -> Option<String> {
        (self.party_inflight || self.party_request.is_some())
            .then(|| "Finish the in-flight party change first.".into())
    }

    pub(super) fn command_owner(&self) -> Option<i64> {
        (self.execution.busy() && (self.execution.task_id == self.task_id || self.task_id <= 0))
            .then(|| {
                self.execution
                    .owner
                    .as_ref()
                    .map_or(self.lead.id, |owner| owner.partner)
            })
    }

    pub fn partner_busy(&self, partner: i64) -> bool {
        std::iter::once(&self.planning)
            .chain(self.cooperation.parked.values())
            .any(|state| {
                state.owns_work()
                    && state
                        .owner()
                        .or(state.front.as_ref())
                        .is_some_and(|owner| owner.partner == partner)
            })
            || std::iter::once(&self.execution)
                .chain(self.cooperation.executions.values())
                .any(|state| {
                    (state.busy() || state.outcome_unknown())
                        && state
                            .owner
                            .as_ref()
                            .is_some_and(|owner| owner.partner == partner)
                })
            || self.cooperation.command_saves.contains_key(&partner)
    }

    /// Roster-scoped command dispatch (`/add` and `/lead` inside the party overlay).
    pub(super) fn party_command(&mut self, name: &str) -> bool {
        match name {
            "add" => {
                self.request_add_partner();
                true
            }
            "lead" => {
                let partners = self.partners();
                match partners.get(self.selected) {
                    Some(partner) => self.request_lead(partner.id),
                    None => self.notice = "No partner is selected in the roster.".into(),
                }
                true
            }
            _ => false,
        }
    }

    /// Contextual `/` choices inside the roster: Lead for the selected row and
    /// Add with its disabled reason.
    pub(super) fn party_command_choices(&self) -> Vec<commands::Choice> {
        let mut choices = Vec::new();
        let partners = self.partners();
        if let Some(partner) = partners.get(self.selected.min(partners.len().saturating_sub(1))) {
            let label = format!("Lead with {}", Self::partner_label(partner));
            let reason = if partner.id == self.lead.id {
                Some(format!(
                    "{} already leads this task.",
                    Self::partner_label(partner)
                ))
            } else {
                self.lead_switch_blocked()
            };
            choices.push(match reason {
                Some(reason) => commands::Choice::disabled("lead", label, reason),
                None => commands::Choice::new("lead", label),
            });
        }
        if self.party.len() >= store::MAX_PARTNERS {
            choices.push(commands::Choice::disabled(
                "add",
                "Add partner",
                format!(
                    "The party is full: a partner can hold at most {} members; release one first.",
                    store::MAX_PARTNERS
                ),
            ));
        } else {
            choices.push(commands::Choice::new("add", "Add partner"));
        }
        choices
    }

    /// Derived, never stored: a partner is waiting while an owned approval or
    /// provider prompt is open, working while it owns a live provider request
    /// or local run, and idle otherwise.
    pub fn partner_status(&self, partner: &store::Partner) -> PartnerStatus {
        if !self.partner_busy(partner.id) {
            return PartnerStatus::Idle;
        }
        let waiting = self
            .planning_state(partner.id)
            .is_some_and(|state| state.preview.is_some() || state.request.is_some())
            || self
                .execution_state(partner.id)
                .is_some_and(|state| state.approval_identity().is_some());
        if waiting {
            PartnerStatus::Waiting
        } else {
            PartnerStatus::Working
        }
    }

    /// Display identity shared by the header and the roster: nickname when set
    /// (PK01), else `Eevee #<id>`.
    pub fn partner_label(partner: &store::Partner) -> String {
        partner
            .nickname
            .clone()
            .unwrap_or_else(|| format!("Eevee #{}", partner.id))
    }

    /// A failed request returns to its sender's own Request draft when that
    /// draft is empty; another partner's composer is never touched.
    pub(super) fn restore_partner_request(&mut self, owner: &store::Attribution) {
        let drafts = self.partner_drafts.entry(owner.partner).or_default();
        let empty = !drafts
            .drafts
            .iter()
            .any(|draft| draft.field == store::EditorField::Request && !draft.text.is_empty());
        if empty {
            let text = self.planning.submitted_request.to_string();
            drafts
                .drafts
                .retain(|draft| draft.field != store::EditorField::Request);
            drafts.drafts.push(store::Draft {
                field: store::EditorField::Request,
                cursor: text.len(),
                text,
            });
            self.changed_task();
        }
        self.notice = format!(
            "{} {}'s request {}.",
            self.planning.notice,
            owner.name,
            if empty {
                "was restored to its draft"
            } else {
                "failed; its newer draft is kept"
            }
        );
    }

    /// Refuse late events for partners no longer in the party (PT06B).
    pub(super) fn retain_slots(&mut self) {
        let ids: Vec<i64> = self.partners().iter().map(|partner| partner.id).collect();
        self.planning.retain_partners(&ids);
        for state in self.cooperation.parked.values_mut() {
            state.retain_partners(&ids);
        }
        self.cooperation.parked.retain(|id, state| {
            ids.contains(id) || state.owns_work() || state.preferences_pending()
        });
        self.cooperation.executions.retain(|id, state| {
            ids.contains(id) || state.busy() || state.storage_pending() || state.outcome_unknown()
        });
    }

    /// The task a partner is engaged with: the lead fronts it and a working
    /// partner keeps it; an idle non-lead shows `-`.
    pub fn partner_task(&self, partner: &store::Partner) -> &str {
        let engaged =
            partner.id == self.lead.id || self.partner_status(partner) != PartnerStatus::Idle;
        if engaged && self.task_id > 0 && !self.goal.is_empty() {
            self.goal.lines().next().unwrap_or("-")
        } else {
            "-"
        }
    }
}
