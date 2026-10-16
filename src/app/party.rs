//! Party roster: the derived view of every partner and what it is doing.
//! Status is computed from live request/approval state, never stored.
use super::*;

/// A roster row's activity state, derived per frame from live ownership.
/// Provider requests and local runs belong to the partner that started them
/// (PT06B), not to whoever is in front now; until PT07 only one partner per
/// task can be working.
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
    /// The party roster in id order. The lead partner row shows its live form:
    /// `set_form` mutates `self.lead` before a save lands, and identity fields
    /// (nickname, met date) are immutable store data so the row keeps them.
    pub fn partners(&self) -> Vec<store::Partner> {
        let mut partners = self.party.clone();
        match partners.iter_mut().find(|p| p.id == self.lead.id) {
            Some(partner) => partner.form = self.lead.form,
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
        self.partner_drafts
            .insert(self.lead.id, self.partner_draft());
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
    }

    /// Why a lead switch is refused right now: only an in-flight roster write.
    /// A working partner keeps its own request slot while another leads.
    fn lead_switch_blocked(&self) -> Option<String> {
        (self.party_inflight || self.party_request.is_some())
            .then(|| "Finish the in-flight party change first.".into())
    }

    /// The partner that owns this task's live provider request, if any.
    fn provider_owner(&self) -> Option<i64> {
        (self.planning.task_id() == self.task_id || self.task_id <= 0)
            .then(|| self.planning.working_partner())
            .flatten()
    }

    /// The partner that owns this task's command request or run, if any.
    fn command_owner(&self) -> Option<i64> {
        (self.execution.busy() && (self.execution.task_id == self.task_id || self.task_id <= 0))
            .then(|| {
                self.execution
                    .owner
                    .as_ref()
                    .map_or(self.lead.id, |owner| owner.partner)
            })
    }

    /// `Bolt (Jolteon) is working on this task…` when a partner other than the
    /// lead holds the task's single working slot (PT06B); `None` otherwise.
    pub(super) fn other_partner_working(&self) -> Option<String> {
        let owner = self
            .provider_owner()
            .or_else(|| self.command_owner())
            .filter(|owner| *owner != self.lead.id)?;
        let name = self
            .partners()
            .into_iter()
            .find(|partner| partner.id == owner)
            .map_or_else(
                || format!("Eevee #{owner}"),
                |partner| {
                    format!(
                        "{} ({})",
                        Self::partner_label(&partner),
                        FORMS[partner.form.min(FORMS.len() - 1)].name
                    )
                },
            );
        Some(format!(
            "{name} is working on this task; one partner works at a time, so lead it again or wait. Your draft is kept."
        ))
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
        let provider = self.provider_owner() == Some(partner.id);
        let command = self.command_owner() == Some(partner.id);
        if !provider && !command {
            return PartnerStatus::Idle;
        }
        if provider && (self.planning.preview.is_some() || self.planning.request.is_some())
            || command
                && self
                    .execution
                    .confirmation
                    .as_ref()
                    .is_some_and(|c| matches!(c, execution::Confirmation::Approval))
        {
            return PartnerStatus::Waiting;
        }
        PartnerStatus::Working
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
