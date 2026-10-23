//! Text density is a store-global reading preference, never request authority.
use super::App;
use crate::{
    i18n::{Language, Message},
    store::TextDensity,
};

impl App {
    pub fn text_density(&self) -> TextDensity {
        self.density_choice
            .unwrap_or_else(|| self.planning.text_density())
    }

    pub fn density_notice(&self) -> Option<Message> {
        [Message::DensityCompact, Message::DensityComfortable]
            .into_iter()
            .find(|message| self.notice == Language::ENGLISH.text(*message))
    }

    pub(super) fn sync_density_choices(&mut self) {
        if let Some(density) = self.density_choice {
            self.planning.sync_density_choice(density);
            for state in self.cooperation.parked.values_mut() {
                state.sync_density_choice(density);
            }
        }
    }

    pub(super) fn density_command(&mut self, command: &str) -> bool {
        let density = match command {
            "density" => match self.text_density() {
                TextDensity::Compact => TextDensity::Comfortable,
                TextDensity::Comfortable => TextDensity::Compact,
            },
            "density-compact" => TextDensity::Compact,
            "density-comfortable" => TextDensity::Comfortable,
            _ => return false,
        };
        self.density_choice = Some(density);
        self.planning.set_text_density(density);
        self.sync_density_choices();
        self.notice = Language::ENGLISH
            .text(match density {
                TextDensity::Compact => Message::DensityCompact,
                TextDensity::Comfortable => Message::DensityComfortable,
            })
            .into();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn density_changes_keep_owned_work_and_exact_cjk_draft_and_caret() {
        let mut app = App::new("/workspace".into(), 60, 18);
        app.planning.preference_event(Ok(None));
        let claim = crate::store::WorkClaim {
            task_id: 7,
            owner: app.attribution(),
            request_id: "owned-density".into(),
            writing: false,
        };
        app.planning.work = Some(claim.clone());
        app.input = "다음 界 e\u{301} request".into();
        app.cursor = "다음 界 ".len();
        let before = (app.input.clone(), app.cursor, app.navigation());
        let provider = app.planning.agent;
        let selection = app.planning.selection.clone();
        let motion = app.planning.reduced_motion();
        for command in ["density-comfortable", "density-compact", "density"] {
            assert!(app.density_command(command));
            assert_eq!(app.planning.work, Some(claim.clone()));
            assert_eq!((app.input.clone(), app.cursor, app.navigation()), before);
            assert_eq!(app.planning.agent, provider);
            assert_eq!(app.planning.selection, selection);
            assert_eq!(app.planning.reduced_motion(), motion);
            assert!(app.planning.pending.is_none());
            assert!(app.pending_command.is_none());
        }
        assert_eq!(app.text_density(), TextDensity::Comfortable);
    }
}
