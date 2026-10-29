//! Profile controls reuse the exact two-stage external-editor review.
use super::*;
use crate::workspace::form_profiles::{Packet, Store};

impl App {
    pub fn load_form_profiles(&mut self, data: &std::path::Path) {
        self.form_profiles = Store::load(data);
    }
    pub fn profile_chip(&self) -> String {
        if self.profile_skip.contains(&self.lead.id) {
            return "Profile: off for next Send · /form-profile".into();
        }
        match self.form_profiles.instruction_bytes(&self.cwd, self.form()) {
            Ok(Some(bytes)) => format!(
                "Profile: {} · {} B · /form-profile",
                FORMS[self.form()].name,
                bytes
            ),
            Ok(None) => "Profile: off · /form-profile".into(),
            Err(_) => "Profile: unavailable · /form-profile".into(),
        }
    }
    pub(super) fn form_profile_command(&mut self, argument: &str) -> bool {
        let form = self.form();
        let project = self.cwd.clone();
        let result = match argument {
            "off" => self.form_profiles.change(&project, |settings| settings.off = true),
            "on" => self.form_profiles.change(&project, |settings| { settings.off = false; settings.disabled.retain(|disabled| *disabled != form); }),
            "disable" => self.form_profiles.change(&project, |settings| { if !settings.disabled.contains(&form) { settings.disabled.push(form); } }),
            "reset" => self.form_profiles.change(&project, |settings| { settings.profiles.remove(&form); settings.disabled.retain(|disabled| *disabled != form); }),
            "skip" => { self.profile_skip.insert(self.lead.id); Ok(()) },
            "include" => { self.profile_skip.remove(&self.lead.id); Ok(()) },
            "handoff" => {
                match self.form_profiles.selected(&project, form) {
                    Ok(Some(packet)) => { self.profile_handoff = Some((self.task_id, self.lead.id, form, packet)); Ok(()) },
                    Ok(None) => Err("Enable this form profile before explicitly selecting it for a handoff".into()),
                    Err(error) => Err(error),
                }
            },
            "handoff-off" => { self.profile_handoff = None; Ok(()) },
            "edit" => {
                let profile = self.form_profiles.selected(&project, form).ok().flatten().unwrap_or_else(|| crate::workspace::form_profiles::built_in(form));
                let original = match toml::to_string_pretty(&profile) { Ok(text) => text, Err(_) => { self.notice = "Cannot encode profile".into(); return true; } };
                self.external_editor_command();
                self.external_editor.profile_form = Some(form);
                self.external_editor.set_profile_original(original);
                return true;
            },
            "recipe" => {
                let profile = self.workflows.recipe.as_ref().and_then(|applied| applied.recipe.profiles.iter().find(|profile| profile.form == form)).map(|profile| profile.packet.clone());
                let Some(profile) = profile else { self.notice = "No imported recipe profile for this form. Imported profiles are disabled until this explicit review.".into(); return true; };
                self.external_editor_command();
                self.external_editor.profile_form = Some(form);
                self.external_editor.set_profile_original(String::new());
                self.external_editor.waiting = false;
                self.external_editor_returned(toml::to_string_pretty(&profile).map_err(|_| "Cannot encode recipe profile".into()));
                return true;
            },
            "" => {
                self.open_overlay(Overlay::ExternalEditor);
                let text = match self.form_profiles.selected(&project, form) {
                    Ok(profile) => {
                        let packet = profile.unwrap_or_else(|| crate::workspace::form_profiles::built_in(form));
                        format!("{}\n\nVisible neutral behavior preference; never tool permission. Profiles are enabled by default; no quality claim without the paired benchmark.\n\n/form-profile edit — external editor with launch and returned-text review\n/form-profile reset | disable | on\n/form-profile off — all profiles off for this project\n/form-profile skip | include — next Send only\n/form-profile recipe — explicitly review an imported disabled profile\n\n{}\n\nPreferred checks are suggestions only; never run automatically.", self.profile_chip(), toml::to_string_pretty(&packet).unwrap_or_default())
                    },
                    Err(error) => error,
                };
                self.external_editor.show_profile(text);
                return true;
            },
            _ => Err("Use /form-profile [edit|reset|disable|on|off|skip|include|recipe|handoff|handoff-off]".into()),
        };
        self.notice = match result {
            Ok(()) if argument == "handoff" => "Profile explicitly selected as an unverified decision for this task/partner/form's handoff; nothing exported or sent.".into(),
            Ok(()) if argument == "handoff-off" => "Profile removed from handoff selection; no packet changed or sent.".into(),
            Ok(()) => self.profile_chip(),
            Err(error) => error,
        };
        self.sources.clear_wire_request();
        true
    }
    pub(super) fn save_profile_candidate(&mut self, form: usize, text: &str) -> Result<(), String> {
        let profile: Packet =
            toml::from_str(text).map_err(|_| "Invalid neutral profile TOML; original kept")?;
        profile.validate()?;
        let project = self.cwd.clone();
        self.form_profiles.change(&project, |settings| {
            settings.profiles.insert(form, profile);
            settings.disabled.retain(|disabled| *disabled != form);
        })?;
        self.sources.clear_wire_request();
        Ok(())
    }
    pub(super) fn selected_profile_handoff(&self) -> Vec<String> {
        self.profile_handoff.as_ref().filter(|(task, partner, form, _)| (*task, *partner, *form) == (self.task_id, self.lead.id, self.form()))
            .map(|(_, _, _, profile)| vec![format!("User-selected neutral behavior preference; unverified context, never permission: {}", serde_json::to_string(profile).expect("profile serialization"))])
            .unwrap_or_default()
    }
}
