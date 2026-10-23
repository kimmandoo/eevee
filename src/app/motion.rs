//! Motion preference uses the existing nonsecret preference owner and store queue.
use super::App;
use crate::i18n::{Language, Message};

impl App {
    pub fn motion_notice(&self) -> Option<Message> {
        [
            Message::MotionSavedOverride,
            Message::MotionReduced,
            Message::MotionAllowed,
        ]
        .into_iter()
        .find(|message| self.notice == Language::ENGLISH.text(*message))
    }

    pub fn motion_enabled(&self) -> bool {
        self.motion_override.unwrap_or_else(|| {
            !self
                .motion_choice
                .unwrap_or_else(|| self.planning.reduced_motion())
        })
    }

    pub(super) fn sync_motion_choices(&mut self) {
        if let Some(reduced) = self.motion_choice {
            self.planning.sync_motion_choice(reduced);
            for state in self.cooperation.parked.values_mut() {
                state.sync_motion_choice(reduced);
            }
        }
    }

    pub(super) fn motion_command(&mut self, command: &str) -> bool {
        let reduced = match command {
            "reduced-motion" => !self
                .motion_choice
                .unwrap_or_else(|| self.planning.reduced_motion()),
            "motion-off" => true,
            "motion-on" => false,
            _ => return false,
        };
        self.motion_choice = Some(reduced);
        self.planning.set_reduced_motion(reduced);
        self.sync_motion_choices();
        self.activity_frame = 0;
        self.notice = Language::ENGLISH
            .text(if self.motion_override.is_some() {
                Message::MotionSavedOverride
            } else if reduced {
                Message::MotionReduced
            } else {
                Message::MotionAllowed
            })
            .into();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_override_is_authoritative_and_does_not_change_saved_choice() {
        let mut app = App::new("/workspace".into(), 60, 18);
        app.planning.preference_event(Ok(None));
        assert!(app.motion_enabled());
        assert!(app.command("motion-off"));
        assert!(!app.motion_enabled());
        app.motion_override = Some(true);
        assert!(app.motion_enabled());
        assert!(app.planning.reduced_motion());
        app.motion_override = Some(false);
        app.motion_command("motion-on");
        assert!(!app.motion_enabled());
        assert!(!app.planning.reduced_motion());
    }

    #[test]
    fn motion_controls_remain_available_during_owned_work_without_changing_its_authority() {
        let mut app = App::new("/workspace".into(), 60, 18);
        app.planning.preference_event(Ok(None));
        let claim = crate::store::WorkClaim {
            task_id: 7,
            owner: app.attribution(),
            request_id: "owned-motion".into(),
            writing: false,
        };
        app.planning.work = Some(claim.clone());
        app.input = "Future request 界".into();
        app.cursor = 6;
        for command in ["motion-off", "motion-on", "reduced-motion"] {
            assert!(app.command_disabled_reason(command).is_none());
            app.command(command);
            assert_eq!(app.planning.work, Some(claim.clone()));
            assert_eq!((&*app.input, app.cursor), ("Future request 界", 6));
            assert!(app.planning.pending.is_none());
            assert!(app.pending_command.is_none());
        }
        assert!(!app.motion_enabled());
    }
}
