//! Local-only cooperation navigation and explicit partner-release review.
//! Store acknowledgements, not UI selection, establish roster changes.
use super::*;

#[derive(Default)]
pub struct State {
    pub board_selected: usize,
    pub board_owner: Option<i64>,
    pub board_scroll: u16,
    pub board_max: u16,
    pub journal_discard_details: bool,
    pub journal_discard_scroll: u16,
    pub journal_discard_max: u16,
    pub release: Option<ReleaseReview>,
}

pub struct ReleaseReview {
    /// The identity reviewed by the user remains stable through a refusal.
    pub owner: store::Attribution,
    pub reviewed: bool,
    pub affirmative: bool,
    pub pending: bool,
    pub refusal: Option<String>,
}

impl App {
    /// Read the selected request's own parked/fronted state, never the lead's
    /// receipt or mutable next-request model selection.
    pub(crate) fn cooperation_work_details(&self, row: &cooperation::WorkRow) -> String {
        use std::fmt::Write;
        let mut detail = String::new();
        if row.local {
            let _ = writeln!(
                detail,
                "Local command; no provider request receipt or model billing applies."
            );
            if let Some(state) = self.execution_state(row.owner.partner) {
                detail.push_str(&state.details());
                detail.push('\n');
            } else {
                let _ = writeln!(detail, "This owner's command metadata is unavailable.");
            }
        } else if let Some(state) = self.planning_state(row.owner.partner) {
            let _ = writeln!(
                detail,
                "Provider: {} / requested {}",
                state.agent.label(),
                state.reviewed_selection().summary(),
            );
            let receipt_current = state
                .work
                .as_ref()
                .is_some_and(|claim| state.receipt_claim.as_ref() == Some(claim));
            let observed = if receipt_current {
                state.observed_model.as_deref()
            } else {
                None
            };
            let _ = writeln!(
                detail,
                "Observed model: {}",
                observed.unwrap_or("unreported")
            );
            let _ = writeln!(detail, "{}", self.owner_usage_line(row.owner.partner));
            let _ = writeln!(
                detail,
                "Receipt below is local serialization/estimate, not billing."
            );
            if !receipt_current || state.receipt.is_empty() {
                let _ = writeln!(detail, "No local payload receipt yet.");
            } else {
                for line in &state.receipt {
                    let _ = writeln!(detail, "{line}");
                }
            }
            if let Some(claim) = &state.work {
                let _ = writeln!(
                    detail,
                    "Request: {} / task {}",
                    claim.request_id, claim.task_id
                );
            }
        } else {
            let _ = writeln!(detail, "Request metadata unavailable; usage/cost unknown.");
        }
        detail.push_str(&row.detail);
        detail
    }

    pub(super) fn open_party_board(&mut self) {
        let rows = self.work_rows();
        self.cooperation_ui.board_selected = rows
            .iter()
            .position(|row| row.owner.partner == self.lead().id)
            .unwrap_or(0);
        self.cooperation_ui.board_owner = rows
            .get(self.cooperation_ui.board_selected)
            .map(|row| row.owner.partner);
        self.cooperation_ui.board_scroll = 0;
        self.open_overlay(Overlay::PartyBoard);
    }

    pub(super) fn request_partner_release(&mut self) {
        let partners = self.partners();
        let partner = if self.overlay == Some(Overlay::Party) {
            partners.get(self.selected).map(|partner| partner.id)
        } else {
            Some(self.lead().id)
        };
        if let Some(partner) = partner {
            self.open_partner_release(partner);
        }
    }

    pub(super) fn release_partner_command(&mut self, argument: &str) -> bool {
        let argument = argument.trim();
        if argument.is_empty() {
            self.request_partner_release();
            return true;
        }
        let partners = self.partners();
        let id = argument
            .strip_prefix('#')
            .unwrap_or(argument)
            .parse::<i64>()
            .ok();
        let partner = partners.iter().find(|partner| {
            Some(partner.id) == id || Self::partner_label(partner).eq_ignore_ascii_case(argument)
        });
        if let Some(partner) = partner {
            self.open_partner_release(partner.id);
        } else {
            self.notice = format!("No partner named \"{argument}\"; /party lists the roster.");
        }
        true
    }

    fn open_partner_release(&mut self, id: i64) {
        if self.party_inflight || self.party_request.is_some() {
            self.notice = "Wait for the current party change to finish.".into();
            return;
        }
        let Some(partner) = self.partners().into_iter().find(|partner| partner.id == id) else {
            self.notice = "That partner is no longer in the party.".into();
            return;
        };
        let refusal = self.partner_release_refusal(id);
        self.cooperation_ui.release = Some(ReleaseReview {
            owner: store::Attribution {
                partner: partner.id,
                name: Self::partner_label(&partner),
                form: partner.form,
            },
            reviewed: false,
            affirmative: false,
            pending: false,
            refusal,
        });
    }

    fn partner_release_refusal(&self, partner: i64) -> Option<String> {
        let pending_records = self.records.local.iter().any(|intent| {
            use crate::workspace::records::LocalIntent;
            match intent {
                LocalIntent::FormRoute { owner, .. } | LocalIntent::Artifact { owner, .. } | LocalIntent::HandoffReady { owner, .. } => owner.partner == partner,
            }
        }) || self.records.pending.iter().chain(self.records.inflight.iter()).chain(self.records.failed.iter()).chain(self.records.queued.iter()).any(|request| {
            matches!(&request.operation, store::records::Operation::SetPairing { partner: owner, .. } if *owner == partner)
        });
        if self.partners().len() <= 1 {
            Some("Cannot release the last partner. Keep at least one partner in the party.".into())
        } else if self.partner_busy(partner) {
            Some("Cannot release the selected partner while it owns work or an execution, on any task. Finish or explicitly cancel its work first.".into())
        } else if self.party_inflight
            || self.party_request.is_some()
            || self.transitioning
            || pending_records
        {
            Some("Wait for the current local storage operation to finish.".into())
        } else if self.storage != StorageState::Saved
            || self.revision != self.saved_revision
            || self.save_requested
            || self.sources.persistence_pending()
        {
            Some("Save or retry task, partner drafts and sources before release. Nothing was removed.".into())
        } else {
            None
        }
    }

    fn confirm_partner_release(&mut self) {
        if self.width < 60 || self.height < 18 {
            return;
        }
        let Some(partner) = self
            .cooperation_ui
            .release
            .as_ref()
            .map(|review| review.owner.partner)
        else {
            return;
        };
        let refusal = self.partner_release_refusal(partner);
        let Some(review) = &mut self.cooperation_ui.release else {
            return;
        };
        if !review.reviewed || review.pending {
            return;
        }
        if let Some(refusal) = refusal {
            review.refusal = Some(refusal);
            review.affirmative = false;
            return;
        }
        self.party_request = Some(store::PartyRequest::Release {
            partner: review.owner.partner,
        });
        review.pending = true;
        review.affirmative = false;
        review.reviewed = false;
        review.refusal = None;
    }

    /// Runtime restores the refreshed project only after this acknowledgement.
    pub(super) fn release_partner_finished(&mut self, result: &Result<store::Project, String>) {
        match result {
            Ok(_) => self.cooperation_ui.release = None,
            Err(error) => {
                if let Some(review) = &mut self.cooperation_ui.release {
                    review.pending = false;
                    review.affirmative = false;
                    review.reviewed = false;
                    review.refusal = Some(error.clone());
                }
            }
        }
    }

    /// Run before editor/overlay keys. Browsing never sends, denies or cancels.
    pub(super) fn cooperation_key(&mut self, key: KeyEvent) -> bool {
        if self.cooperation_ui.release.is_some() {
            if key.kind == KeyEventKind::Release {
                return true;
            }
            if self
                .cooperation_ui
                .release
                .as_ref()
                .is_some_and(|review| review.pending)
            {
                // Once explicitly submitted, dismissal cannot revoke the write
                // or discard the identity needed to attribute a store refusal.
                return true;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                self.cooperation_ui.release = None;
                self.request_quit();
                return true;
            }
            if key.kind != KeyEventKind::Press || !key.modifiers.is_empty() {
                return true;
            }
            let review = self
                .cooperation_ui
                .release
                .as_mut()
                .expect("release review");
            match key.code {
                KeyCode::Esc | KeyCode::Char('n') => self.cooperation_ui.release = None,
                KeyCode::Left => review.affirmative = false,
                KeyCode::Right if review.reviewed => review.affirmative = true,
                KeyCode::Tab if review.reviewed => review.affirmative = !review.affirmative,
                KeyCode::Char('y') => self.confirm_partner_release(),
                KeyCode::Enter => {
                    if review.affirmative {
                        self.confirm_partner_release();
                    } else {
                        self.cooperation_ui.release = None;
                    }
                }
                _ => {}
            }
            return true;
        }
        if self.overlay != Some(Overlay::PartyBoard) {
            return false;
        }
        if key.kind == KeyEventKind::Release {
            return true;
        }
        if self.command_popup.is_some()
            || key.code == KeyCode::Char('/') && key.modifiers.is_empty()
            || key.code == KeyCode::Char('k') && key.modifiers == KeyModifiers::CONTROL
        {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.request_quit();
            return true;
        }
        let rows = self.work_rows();
        let selected = self
            .cooperation_ui
            .board_owner
            .and_then(|owner| rows.iter().position(|row| row.owner.partner == owner))
            .unwrap_or_else(|| {
                self.cooperation_ui
                    .board_selected
                    .min(rows.len().saturating_sub(1))
            });
        let selected = match key.code {
            KeyCode::Esc => {
                self.close_overlay();
                return true;
            }
            KeyCode::Up => selected.saturating_sub(1),
            KeyCode::Down => (selected + 1).min(rows.len().saturating_sub(1)),
            KeyCode::Home => 0,
            KeyCode::End => rows.len().saturating_sub(1),
            KeyCode::PageUp => {
                self.cooperation_ui.board_scroll =
                    self.cooperation_ui.board_scroll.saturating_sub(3);
                return true;
            }
            KeyCode::PageDown => {
                self.cooperation_ui.board_scroll = self
                    .cooperation_ui
                    .board_scroll
                    .saturating_add(3)
                    .min(self.cooperation_ui.board_max);
                return true;
            }
            KeyCode::Enter if key.kind != KeyEventKind::Repeat => {
                if let Some(row) = rows.get(selected) {
                    let owner = row.owner.partner;
                    self.close_overlay();
                    self.request_lead(owner);
                }
                return true;
            }
            _ => return true,
        };
        self.cooperation_ui.board_selected = selected;
        self.cooperation_ui.board_owner = rows.get(selected).map(|row| row.owner.partner);
        self.cooperation_ui.board_scroll = 0;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{ColorMode, Config, ImageMode, LayoutMode, Theme},
        providers,
    };
    use ratatui::{Terminal, backend::TestBackend};

    fn config() -> Config {
        Config {
            native_transport: crate::providers::NativeTransport::Auto,
            retry: Default::default(),
            ascii: true,
            no_animation: true,
            animation_override: Some(false),
            theme: Theme::Dark,
            color: ColorMode::Mono,
            images: ImageMode::Off,
            assets_dir: "unused".into(),
            layout: LayoutMode::Sanctuary,
            layout_explicit: false,
            data_dir: "unused-data".into(),
            storage_policy: store::Policy::default(),
            context_exclusions: Vec::new(),
            capture_capacity: 64 * 1024,
        }
    }

    fn screen(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn key(app: &mut App, code: KeyCode) {
        app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    fn three_owners(width: u16, height: u16) -> App {
        let mut task = store::Snapshot::empty("/workspace".into());
        task.id = 7;
        task.goal = "Inspect independent cooperating owners".into();
        let partners = vec![
            store::Partner {
                nickname: Some("Bolt".into()),
                form: 3,
                ..Default::default()
            },
            store::Partner {
                id: 2,
                nickname: Some("Mizu".into()),
                form: 1,
                ..Default::default()
            },
            store::Partner {
                id: 3,
                nickname: Some("Shade".into()),
                form: 7,
                ..Default::default()
            },
        ];
        let mut app = App::new("/workspace".into(), width, height);
        app.store_event(store::Event::Loaded(store::Project {
            tasks: vec![task.clone()],
            partners: partners.clone(),
            active_id: Some(7),
            usage_bytes: 0,
        }));
        for partner in &partners {
            let owner = store::Attribution::of(partner);
            let mut state = app.planning.new_partner(owner.clone(), 7);
            state.selection.model = Some(format!("request-model-{}", partner.id));
            state.command("new", true, &task, &[], None);
            state.pending.take();
            if partner.id != 1 {
                state.event(providers::Event::Preview(providers::Preview {
                    transport: None,
                    comparison_scope: None,
                    arrival: providers::approval_arrival(),
                    destination: None,
                    requires_connection_review: true,
                    generation: partner.id as u64,
                    task_id: 7,
                    thread_id: None,
                    trust: Vec::new(),
                    payload: "{}".into(),
                    blockers: Vec::new(),
                }));
            }
            state.selection.model = Some("future-draft-model".into());
            state.receipt = vec![format!(
                "Owner {} local payload: {} B; token estimate only.",
                partner.id,
                partner.id * 111
            )];
            state.work = Some(store::WorkClaim {
                task_id: 7,
                owner,
                request_id: format!("ui-owner-{}", partner.id),
                writing: partner.id == 1,
            });
            state.work_acquired = true;
            state.receipt_claim = state.work.clone();
            state.notice = format!(
                "Snapshot-owned request {}; no invented progress.",
                partner.id
            );
            if partner.id == 1 {
                app.planning = state;
            } else {
                app.cooperation.parked.insert(partner.id, state);
            }
        }
        app.refresh_approvals();
        app
    }

    #[test]
    fn board_commands_keep_browsing_owner_and_pending_decisions_intact() {
        for (width, height) in [(120, 36), (60, 18)] {
            let mut app = three_owners(width, height);
            app.open_party_board();
            app.update(Event::Key(KeyEvent::new(
                KeyCode::Char('k'),
                KeyModifiers::CONTROL,
            )));
            assert!(app.command_popup.is_some());
            key(&mut app, KeyCode::Char('g'));
            assert_eq!(app.command_popup.as_ref().unwrap().query, "g");
            key(&mut app, KeyCode::Esc);
            assert!(app.command_popup.is_none());
            assert_eq!(app.overlay, Some(Overlay::PartyBoard));
            assert_eq!(app.lead().id, 1);
            assert_eq!(app.work_rows().len(), 3);
            assert_eq!(app.approval_count(), 2);
            assert!(!app.planning.cancelling);
            assert!(
                app.cooperation
                    .parked
                    .values()
                    .all(|state| !state.cancelling)
            );
        }
    }

    #[test]
    fn board_and_fifo_review_front_exact_owner_without_answering_or_cancelling() {
        for (width, height) in [(120, 36), (60, 18)] {
            let config = config();
            let mut artwork = crate::ui::artwork::Artwork::load(&config);
            let mut app = three_owners(width, height);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| crate::ui::render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let home = screen(&terminal);
            assert!(home.contains("Bolt (Flareon)"), "{home}");
            assert!(!home.contains("is editing"), "{home}");
            if width == 60 {
                assert!(home.contains("3 partners working / waiting"), "{home}");
                assert!(home.contains("/party-board"), "{home}");
            }
            app.command("party-board");
            terminal
                .draw(|frame| crate::ui::render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let board = screen(&terminal);
            for identity in ["Bolt (Flareon)", "Mizu (Vaporeon)", "Shade (Glaceon)"] {
                assert!(board.contains(identity), "{identity}: {board}");
            }
            assert!(
                board.contains("working") && board.contains("waiting"),
                "{board}"
            );
            key(&mut app, KeyCode::Down);
            key(&mut app, KeyCode::Enter);
            assert_eq!(app.lead().id, 2, "{}", app.notice);
            assert_eq!(app.work_rows().len(), 3);
            assert_eq!(app.approval_count(), 2);
            assert!(app.planning.preview.is_some());
            assert!(app.planning.pending.is_none() && app.planning.decision.is_none());
            assert!(app.cooperation.parked.get(&1).unwrap().work.is_some());
            app.command("approvals");
            terminal
                .draw(|frame| crate::ui::render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let review = screen(&terminal);
            assert!(
                review.contains("1 of 2") && review.contains("Mizu (Vaporeon)"),
                "{review}"
            );
            assert!(review.contains("Storage:"), "{review}");
            assert_eq!(app.approval_position(), Some((1, 2)));
            key(&mut app, KeyCode::Esc);
            assert_eq!(app.approval_count(), 2);
            assert!(app.planning.preview.is_some());
        }
    }

    #[test]
    fn all_owner_reviews_are_neutral_and_busy_release_keeps_snapshot() {
        for (width, height) in [(120, 36), (60, 18), (59, 17)] {
            let config = config();
            let mut artwork = crate::ui::artwork::Artwork::load(&config);
            let mut app = three_owners(width, height);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            app.request_cancel_all();
            terminal
                .draw(|frame| crate::ui::render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let shown = screen(&terminal);
            for name in [
                "Bolt (Flareon)",
                "Mizu (Vaporeon)",
                "Shade (Glaceon)",
                "No file rollback",
                "Storage:",
            ] {
                assert!(shown.contains(name), "{name}: {shown}");
            }
            assert!(!app.cooperation.cancel_review.as_ref().unwrap().affirmative);
            key(&mut app, KeyCode::Enter);
            assert!(app.cooperation.cancel_review.is_none());
            assert_eq!(app.work_rows().len(), 3);
            app.open_cancel_review(true);
            terminal
                .draw(|frame| crate::ui::render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            assert!(screen(&terminal).contains("ALL owned work"));
            key(&mut app, KeyCode::Esc);
            assert!(!app.quit);
            app.release_partner_command("#2");
            terminal
                .draw(|frame| crate::ui::render(frame, &mut app, &config, &mut artwork, "off"))
                .unwrap();
            let refusal = screen(&terminal);
            assert!(refusal.contains("Mizu (Vaporeon)"), "{refusal}");
            assert!(refusal.contains("Cannot release"), "{refusal}");
            key(&mut app, KeyCode::Char('y'));
            assert!(app.party_request.is_none());
            assert_eq!(
                app.cooperation_ui.release.as_ref().unwrap().owner.partner,
                2
            );
            assert_eq!(app.work_rows().len(), 3);
        }
    }

    #[test]
    fn board_details_keep_each_request_model_and_local_receipt_separate() {
        for (width, height) in [(120, 36), (60, 18)] {
            let config = config();
            let mut artwork = crate::ui::artwork::Artwork::load(&config);
            let mut app = three_owners(width, height);
            app.open_party_board();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for owner in 1..=3 {
                let mut shown = String::new();
                loop {
                    terminal
                        .draw(|frame| {
                            crate::ui::render(frame, &mut app, &config, &mut artwork, "off")
                        })
                        .unwrap();
                    shown.push_str(&screen(&terminal));
                    if app.cooperation_ui.board_scroll == app.cooperation_ui.board_max {
                        break;
                    }
                    key(&mut app, KeyCode::PageDown);
                }
                assert!(shown.contains(&format!("request-model-{owner}")), "{shown}");
                assert!(
                    shown.contains("Provider usage/cost: no request dispatched yet"),
                    "{shown}"
                );
                assert!(
                    shown.contains(&format!("Owner {owner} local payload: {} B", owner * 111)),
                    "{shown}"
                );
                let selected = app
                    .work_rows()
                    .into_iter()
                    .find(|row| row.owner.partner == owner)
                    .unwrap();
                let selected_details = app.cooperation_work_details(&selected);
                assert!(
                    !selected_details.contains("future-draft-model"),
                    "{selected_details}"
                );
                assert!(
                    !shown.contains(&format!("Owner {} local payload", owner % 3 + 1)),
                    "{shown}"
                );
                key(&mut app, KeyCode::Down);
            }
            assert_eq!(app.lead().id, 1);
            assert_eq!(app.work_rows().len(), 3);
            assert_eq!(app.approval_count(), 2);
        }
    }

    #[test]
    fn new_claim_does_not_display_an_earlier_requests_receipt_or_observed_model() {
        let mut app = three_owners(60, 18);
        app.planning.observed_model = Some("prior-observed-model".into());
        app.planning.work.as_mut().unwrap().request_id = "new-reservation-awaiting-preview".into();
        let row = app
            .work_rows()
            .into_iter()
            .find(|row| row.owner.partner == 1)
            .unwrap();
        let detail = app.cooperation_work_details(&row);
        assert!(detail.contains("new-reservation-awaiting-preview"));
        assert!(detail.contains("Observed model: unreported"));
        assert!(detail.contains("No local payload receipt yet."));
        assert!(!detail.contains("prior-observed-model"));
        assert!(!detail.contains("Owner 1 local payload"));
    }

    #[test]
    fn pending_pairing_write_blocks_only_its_own_partner_release() {
        let mut app = three_owners(60, 18);
        for id in [4, 5] {
            app.party.push(store::Partner {
                id,
                ..Default::default()
            });
        }
        assert!(app.queue_record_operation(
            app.task_id,
            store::records::Operation::SetPairing {
                partner: 4,
                form: 0,
                selection: None,
            }
        ));
        assert!(app.partner_release_refusal(4).is_some());
        assert!(app.partner_release_refusal(5).is_none());
        assert_eq!(app.work_rows().len(), 3);
        assert!(app.cooperation.cancel_review.is_none());
    }

    #[test]
    fn idle_partner_release_does_not_cancel_or_block_other_working_owners() {
        let mut app = three_owners(60, 18);
        app.party.push(store::Partner {
            id: 4,
            nickname: Some("Idle".into()),
            ..Default::default()
        });
        app.release_partner_command("#4");
        assert!(
            app.cooperation_ui
                .release
                .as_ref()
                .unwrap()
                .refusal
                .is_none()
        );
        let config = config();
        let mut artwork = crate::ui::artwork::Artwork::load(&config);
        let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app, &config, &mut artwork, "off"))
            .unwrap();
        key(&mut app, KeyCode::Char('y'));
        assert_eq!(
            app.party_request,
            Some(store::PartyRequest::Release { partner: 4 })
        );
        assert_eq!(app.work_rows().len(), 3);
        assert_eq!(app.approval_count(), 2);
        assert!(!app.planning.cancelling);
        assert!(
            app.cooperation
                .parked
                .values()
                .all(|state| !state.cancelling)
        );
    }
}
