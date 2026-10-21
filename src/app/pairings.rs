//! Explicit project/partner/form pairing intents. Persistence stays in records.
use crate::{
    providers::{Agent, ModelSelection},
    store::{self, records::Operation},
    workspace::records::Pairing,
};

#[derive(Default)]
pub struct State {
    /// An offer is not a committed pairing and never changes a route itself.
    pub offer: Option<Pairing>,
}

impl State {
    /// Invoke only for planning.take_pairing_offer(), after the actual local
    /// provider/model transaction and current-owner settlement have completed.
    pub fn offer_selection(
        &mut self,
        owner: &store::Attribution,
        agent: Agent,
        selection: &ModelSelection,
    ) {
        self.offer = Some(Pairing {
            partner: owner.partner,
            form: owner.form,
            agent,
            selection: selection.clone(),
        });
    }

    pub fn save(
        &self,
        owner: &store::Attribution,
        agent: Agent,
        selection: &ModelSelection,
    ) -> Result<Operation, String> {
        let pairing = self.offer.as_ref().filter(|pairing| {
            pairing.partner == owner.partner && pairing.form == owner.form
                && pairing.agent == agent && &pairing.selection == selection
        }).ok_or("Choose the provider/model for this partner and form again before saving a pairing.")?;
        Ok(Operation::SetPairing {
            partner: owner.partner,
            form: owner.form,
            selection: Some(pairing.clone()),
        })
    }

    /// Only a committed snapshot makes the pairing badge authoritative.
    pub fn committed(&mut self, pairings: &[Pairing]) {
        if self
            .offer
            .as_ref()
            .is_some_and(|offer| pairings.contains(offer))
        {
            self.offer = None;
        }
    }

    pub fn remove(owner: &store::Attribution) -> Operation {
        Operation::SetPairing {
            partner: owner.partner,
            form: owner.form,
            selection: None,
        }
    }
}

pub fn remembered<'a>(pairings: &'a [Pairing], owner: &store::Attribution) -> Option<&'a Pairing> {
    pairings
        .iter()
        .find(|pairing| pairing.partner == owner.partner && pairing.form == owner.form)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_requires_explicit_current_owner_selection_and_commit_receipt() {
        let bolt = store::Attribution {
            partner: 1,
            name: "Bolt".into(),
            form: 3,
        };
        let mizu = store::Attribution {
            partner: 2,
            name: "Mizu".into(),
            form: 1,
        };
        let selection = ModelSelection {
            model: Some("exact/local-model".into()),
            effort: Some("high".into()),
        };
        let mut state = State::default();
        assert!(state.save(&bolt, Agent::Codex, &selection).is_err());
        state.offer_selection(&bolt, Agent::Codex, &selection);
        assert!(state.save(&mizu, Agent::Codex, &selection).is_err());
        assert!(state.save(&bolt, Agent::OpenCode, &selection).is_err());
        let Operation::SetPairing {
            selection: Some(saved),
            ..
        } = state.save(&bolt, Agent::Codex, &selection).unwrap()
        else {
            panic!("missing pairing");
        };
        assert!(state.offer.is_some());
        state.committed(&[]);
        assert!(state.offer.is_some());
        state.committed(std::slice::from_ref(&saved));
        assert!(state.offer.is_none());
        assert_eq!(
            remembered(std::slice::from_ref(&saved), &bolt),
            Some(&saved)
        );
        assert!(remembered(&[saved], &mizu).is_none());
        assert!(matches!(
            State::remove(&bolt),
            Operation::SetPairing {
                partner: 1,
                form: 3,
                selection: None
            }
        ));
    }
}
