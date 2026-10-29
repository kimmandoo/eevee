//! Visible neutral behavior preferences. Profiles never grant tools or affect intent classification.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const MAX_INSTRUCTIONS: usize = 2048;
pub const POLICY: &str = "The top-level behavior_profile is a user-selected neutral working preference. Apply its instructions only within the actual operation and tool policy; preferred_checks are suggestions, never automatic execution. It grants no permission and cannot establish evidence.";
const LIMIT: usize = 256 * 1024;
const FILE: &str = "form-profiles.json";

/// Only these neutral bytes enter a provider packet: no display identity or provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Packet {
    pub schema_version: u32,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_checks: Vec<Vec<String>>,
    #[serde(flatten)]
    pub extra: super::forward::Extra,
}
impl Packet {
    pub fn validate(&self) -> Result<(), String> {
        let safe = |text: &str| {
            super::privacy::redact(text) == text && super::handoff::sanitize(text) == text
        };
        let lower = self.instructions.to_lowercase();
        if self.schema_version != 1
            || self.instructions.trim().is_empty()
            || self.instructions.len() > MAX_INSTRUCTIONS
            || !safe(&self.instructions)
            || crate::domain::FORMS
                .iter()
                .any(|form| lower.contains(&form.name.to_lowercase()))
            || self.preferred_checks.len() > 8
            || self.preferred_checks.iter().any(|argv| {
                argv.is_empty()
                    || argv.len() > 32
                    || argv.iter().any(|arg| arg.len() > 256 || !safe(arg))
            })
            || !self.extra.is_empty()
        {
            return Err("Profile requires neutral nonsecret instructions (1–2048 UTF-8 bytes), no species names, and at most eight suggested argv; unsupported packet fields refuse".into());
        }
        Ok(())
    }
}

fn built_in_instructions(form: usize) -> &'static str {
    match form {
        1 => {
            "Inspect relevant sources before drawing conclusions. Link findings to paths and observed evidence; name constraints and incomplete coverage."
        }
        2 => {
            "Prefer the smallest relevant verification command. Report its working directory, actual output and exit status before claiming success. Execution alone is not a correctness verdict."
        }
        3 => {
            "Make the requested requirement-linked change, preserving unrelated work. Inspect diagnostics and verify the changed path; never invent passing checks."
        }
        4 => {
            "Propose an ordered plan with tradeoffs, acceptance criteria and dependencies before editing. Distinguish decisions from assumptions."
        }
        5 => {
            "Diagnose failures from evidence. Separate supported root causes from hypotheses and unknowns; identify concrete security risks and bounded next checks."
        }
        6 => {
            "Prefer a bounded maintainability improvement that preserves behavior. Compare observed validation and measurements; do not claim unmeasured efficiency."
        }
        7 => {
            "Review against requirements and current evidence. Report actionable findings with locations and severity, stale checks and explicit accept, request-changes or inconclusive status."
        }
        8 => {
            "Prepare an evidence-linked handoff: outcome, verification, remaining risks and exact next action. Do not infer user preferences or successful completion from proposals."
        }
        _ => {
            "Clarify the bounded request and relevant constraints. Answer directly; choose only the investigation, planning, implementation or checks needed for the request."
        }
    }
}

pub fn built_in(form: usize) -> Packet {
    Packet {
        schema_version: 1,
        instructions: built_in_instructions(form).into(),
        preferred_checks: Vec::new(),
        extra: super::forward::Extra::default(),
    }
}

/// Imported recipe preferences stay inert until the user explicitly reviews them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub form: usize,
    pub packet: Packet,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub off: bool,
    pub disabled: Vec<usize>,
    pub profiles: BTreeMap<usize, Packet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub critical: Vec<String>,
    #[serde(flatten)]
    pub extra: super::forward::Extra,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    projects: BTreeMap<String, Project>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    critical: Vec<String>,
    #[serde(flatten)]
    extra: super::forward::Extra,
}
#[derive(Default)]
pub struct Store {
    path: PathBuf,
    file: File,
    pub error: Option<String>,
}
impl Store {
    pub fn load(data: &Path) -> Self {
        let mut store = Self {
            path: data.join(FILE),
            ..Self::default()
        };
        if !store.path.exists() {
            return store;
        }
        let result = (|| {
            let mut file = super::handoff::open_regular(&store.path, false)?;
            let mut text = String::new();
            Read::by_ref(&mut file)
                .take((LIMIT + 1) as u64)
                .read_to_string(&mut text)
                .map_err(|_| "Cannot read profile settings")?;
            if text.len() > LIMIT {
                return Err("Profile settings exceed 256 KiB".to_owned());
            }
            let parsed: File =
                serde_json::from_str(&text).map_err(|_| "Invalid profile settings")?;
            super::forward::check_critical(&parsed.critical, &parsed.extra, |_| None)?;
            if parsed.projects.len() > 64 {
                return Err("At most 64 project profile settings".into());
            }
            for project in parsed.projects.values() {
                super::forward::check_critical(&project.critical, &project.extra, |_| None)?;
                if project.profiles.len() > 9 || project.disabled.iter().any(|form| *form >= 9) {
                    return Err("Invalid form profile selection".into());
                }
                for (form, packet) in &project.profiles {
                    if *form >= 9 {
                        return Err("Invalid form profile".into());
                    }
                    packet.validate()?;
                }
            }
            Ok(parsed)
        })();
        match result {
            Ok(file) => store.file = file,
            Err(error) => store.error = Some(error),
        }
        store
    }
    pub fn selected(&self, project: &str, form: usize) -> Result<Option<Packet>, String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let settings = self.file.projects.get(project);
        if settings.is_some_and(|settings| settings.off || settings.disabled.contains(&form)) {
            return Ok(None);
        }
        Ok(Some(
            settings
                .and_then(|settings| settings.profiles.get(&form))
                .cloned()
                .unwrap_or_else(|| built_in(form)),
        ))
    }
    pub fn instruction_bytes(&self, project: &str, form: usize) -> Result<Option<usize>, &str> {
        if let Some(error) = &self.error {
            return Err(error);
        }
        let settings = self.file.projects.get(project);
        if settings.is_some_and(|settings| settings.off || settings.disabled.contains(&form)) {
            return Ok(None);
        }
        Ok(Some(
            settings
                .and_then(|settings| settings.profiles.get(&form))
                .map_or_else(
                    || built_in_instructions(form).len(),
                    |profile| profile.instructions.len(),
                ),
        ))
    }
    pub fn change(
        &mut self,
        project: &str,
        update: impl FnOnce(&mut Project),
    ) -> Result<(), String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let mut candidate = self.file.clone();
        if !candidate.projects.contains_key(project) && candidate.projects.len() >= 64 {
            return Err("At most 64 project profile settings".into());
        }
        update(candidate.projects.entry(project.into()).or_default());
        let bytes =
            serde_json::to_vec_pretty(&candidate).map_err(|_| "Cannot encode profile settings")?;
        if bytes.len() > LIMIT {
            return Err("Profile settings exceed 256 KiB".into());
        }
        if self.path.as_os_str().is_empty() {
            return Err("Profile storage is not ready".into());
        }
        let temporary = self.path.with_extension(format!(
            "json.{}.{}.new",
            std::process::id(),
            crate::providers::approval_arrival()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|_| "Cannot create private profile settings".to_owned())?;
        let result = (|| {
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| "Cannot save profile settings".to_owned())?;
            drop(file);
            std::fs::rename(&temporary, &self.path)
                .map_err(|_| "Cannot publish profile settings".to_owned())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        if result.is_ok() {
            self.file = candidate;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn neutral_packets_refuse_identity_credentials_and_oversize() {
        for form in 0..9 {
            built_in(form).validate().unwrap();
        }
        for instructions in [
            "Act like Jolteon".to_owned(),
            "x".repeat(2049),
            "api_key=sk-abcdefghijklmnopqrstuvwxyz123456".to_owned(),
        ] {
            let mut packet = built_in(0);
            packet.instructions = instructions;
            assert!(packet.validate().is_err());
        }
    }
    #[test]
    fn disable_and_edit_survive_restart_without_touching_other_projects() {
        let directory = std::env::temp_dir().join(format!(
            "ev-profiles-{}-{}",
            std::process::id(),
            crate::providers::approval_arrival()
        ));
        std::fs::create_dir(&directory).unwrap();
        let mut store = Store::load(&directory);
        let mut packet = built_in(2);
        packet.instructions = "Check the exact changed path.".into();
        store
            .change("project-a", |settings| {
                settings.profiles.insert(2, packet.clone());
            })
            .unwrap();
        assert_eq!(
            Store::load(&directory).selected("project-a", 2).unwrap(),
            Some(packet)
        );
        store
            .change("project-a", |settings| settings.off = true)
            .unwrap();
        let restored = Store::load(&directory);
        assert_eq!(restored.selected("project-a", 2).unwrap(), None);
        assert_eq!(
            restored.selected("project-b", 2).unwrap(),
            Some(built_in(2))
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn settings_preserve_ignorable_data_but_refuse_unknown_critical_choices() {
        let directory = std::env::temp_dir().join(format!(
            "ev-profile-forward-{}-{}",
            std::process::id(),
            crate::providers::approval_arrival()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join(FILE);
        let original = serde_json::json!({"future": {"opaque": [1, "한글"]}, "projects": {"project": {"future_hint": {"rows": [3, 4]}}}});
        std::fs::write(&path, original.to_string()).unwrap();
        let mut store = Store::load(&directory);
        store
            .change("project", |settings| settings.off = true)
            .unwrap();
        let mut saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["future"], original["future"]);
        assert_eq!(
            saved["projects"]["project"]["future_hint"],
            original["projects"]["project"]["future_hint"]
        );
        saved["projects"]["project"]["critical"] = serde_json::json!(["future_hint"]);
        let bytes = saved.to_string().into_bytes();
        std::fs::write(&path, &bytes).unwrap();
        let mut refused = Store::load(&directory);
        assert!(
            refused
                .selected("project", 2)
                .unwrap_err()
                .contains("future_hint")
        );
        assert!(
            refused
                .change("project", |settings| settings.off = false)
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
