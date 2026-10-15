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

    /// Roster-scoped command dispatch (`/add` inside the party overlay).
    pub(super) fn party_command(&mut self, name: &str) -> bool {
        match name {
            "add" => {
                self.request_add_partner();
                true
            }
            _ => false,
        }
    }

    /// Contextual `/` choices inside the roster: Add with its disabled reason.
    pub(super) fn party_command_choices(&self) -> Vec<commands::Choice> {
        if self.party.len() >= store::MAX_PARTNERS {
            vec![commands::Choice::disabled(
                "add",
                "Add partner",
                format!(
                    "The party is full: a partner can hold at most {} members; release one first.",
                    store::MAX_PARTNERS
                ),
            )]
        } else {
            vec![commands::Choice::new("add", "Add partner")]
        }
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
