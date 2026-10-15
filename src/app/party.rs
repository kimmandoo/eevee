//! Party roster: the derived view of every partner and what it is doing.
//! Status is computed from live request/approval state, never stored.
use super::*;

/// A roster row's activity state, derived per frame from live ownership.
/// Until PT06/PT07 land per-partner request slots, only the lead partner can
/// own the single planning request or the execution flow, so other partners
/// always derive to `Idle` here.
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
        if self.partner_status(&partner) != PartnerStatus::Idle {
            // Forward hook for PT06+ request slots: a partner that already owns
            // live work cannot front this task at the same time.
            self.notice = format!(
                "{} is {}; it cannot lead this task yet.",
                Self::partner_label(&partner),
                self.partner_status(&partner).label()
            );
            return;
        }
        let label = Self::partner_label(&partner);
        self.switch_lead(partner);
        self.notice = format!(
            "{label} leads this task; its {} form is active.",
            FORMS[self.form().min(FORMS.len() - 1)].name
        );
        self.changed_task();
    }

    /// The actual lead swap: the new partner's own form becomes the active form
    /// and the picker's selection follows. The previous-form memory is the
    /// former lead's, so it resets rather than applying to the new lead.
    fn switch_lead(&mut self, partner: store::Partner) {
        self.lead = partner;
        self.form_selection = self.lead.form;
        self.preview_form = self.lead.form;
        self.previous_form = None;
    }

    /// Why a lead switch is refused right now: a party roster write is in
    /// flight, or the current lead still owns the task's single request slot
    /// (working / waiting). PT06 keeps this hook when requests gain per-partner
    /// routing.
    fn lead_switch_blocked(&self) -> Option<String> {
        if self.party_inflight || self.party_request.is_some() {
            return Some("Finish the in-flight party change first.".into());
        }
        match self.partner_status(&self.lead) {
            PartnerStatus::Idle => None,
            status => Some(format!(
                "{} is {}; finish or cancel its work before changing the lead.",
                Self::partner_label(&self.lead),
                status.label()
            )),
        }
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
        let owns_work = partner.id == self.lead.id
            && (self.planning.task_id() == self.task_id || self.task_id <= 0)
            && (self.planning.busy() || self.execution.busy());
        if !owns_work {
            return PartnerStatus::Idle;
        }
        if self.planning.preview.is_some()
            || self.planning.request.is_some()
            || self
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

    /// The task a partner is engaged with. Only a live status binds a partner
    /// to a task; an idle non-lead shows `-`.
    pub fn partner_task(&self, partner: &store::Partner) -> &str {
        if partner.id == self.lead.id && self.task_id > 0 && !self.goal.is_empty() {
            self.goal.lines().next().unwrap_or("-")
        } else {
            "-"
        }
    }
}
