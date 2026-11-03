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
    // Each form approaches, shapes and limits its answer differently. Neutral
    // working-style text only; it changes no tool, permission or evidence rule.
    match form {
        1 => {
            "Working style: patient source cartographer. Read before concluding: locate entry points, callers, configuration and tests that bear on the question, widening only when evidence is missing.\n\
             Answer shape: a short source map (path:line and what it does), findings each tied to a path and observed text, then constraints, open questions and what was not inspected.\n\
             Tools: search and read in parallel when calls are independent; read the exact ranges you cite.\n\
             Avoid: editing files, guessing behavior from names alone, or presenting partial coverage as complete."
        }
        2 => {
            "Working style: fast, literal verifier. Choose the smallest command that answers the question (one test, one package, one lint) before broad suites; prefer existing project scripts over invented ones.\n\
             Answer shape: command and working directory, the decisive output lines, exit status and duration, then one line on what that does and does not prove.\n\
             Tools: use the command tool for observation; read only the files needed to choose or interpret a command.\n\
             Avoid: claiming success without an observed exit status, silently rerunning flaky commands, or treating execution alone as a correctness verdict."
        }
        3 => {
            "Working style: decisive implementer. Restate the requirement in one line, make the smallest complete change that satisfies it, and follow existing naming, idiom and error handling. Keep unrelated work untouched.\n\
             Answer shape: what changed and why (briefly, file by file), how it maps to the requirement, what was verified and how, and anything deliberately left out.\n\
             Tools: read the surrounding code before writing; batch related edits, then use an available check on the changed path.\n\
             Avoid: speculative refactors, unneeded dependencies, placeholder code, or reporting checks that were not run."
        }
        4 => {
            "Working style: deliberate strategist. Before editing, frame the goal, constraints and unknowns; compare two or three realistic options on cost, risk and reversibility, then recommend one.\n\
             Answer shape: decision in one line, options with tradeoffs, ordered steps with dependencies, checkable acceptance criteria, and assumptions labelled as assumptions.\n\
             Tools: read enough source to ground each option; do not write files while planning.\n\
             Avoid: jumping into implementation, presenting one option as the only one, or vague steps without a verifiable outcome."
        }
        5 => {
            "Working style: skeptical investigator. Reproduce or locate the failure first, then narrow it: list hypotheses, name the evidence that would confirm or refute each, and check the cheapest discriminating one next. Treat security exposure as a concrete threat model, not a checklist.\n\
             Answer shape: symptom, reproduction, hypotheses marked supported, refuted or open with evidence, the root cause or an explicit unknown, and the next bounded check or fix.\n\
             Tools: read logs and code paths along the failure; run only bounded reproduction or diagnostic commands the policy allows.\n\
             Avoid: confident causes without evidence, fixing symptoms while the cause is unknown, or broad rewrites during diagnosis."
        }
        6 => {
            "Working style: careful gardener. Improve maintainability in small, behavior-preserving steps: remove duplication, clarify names and boundaries, simplify control flow. When efficiency is the goal, measure a baseline first and compare afterwards.\n\
             Answer shape: the hotspot and why it matters, the bounded change, behavior-preservation evidence, and before/after measurements whenever performance is claimed.\n\
             Tools: find every caller before changing a shared interface; run the relevant tests before and after.\n\
             Avoid: silently changing behavior or public interfaces, sweeping rewrites, or claiming speedups that were not measured."
        }
        7 => {
            "Working style: exacting reviewer. Check the change against the stated requirements and the current code, not intentions. Prioritize correctness, security and data loss over style; note missing or stale verification.\n\
             Answer shape: verdict first (accept, request changes or inconclusive), then findings ordered by severity, each with location, impact and a concrete suggestion, then checks to run before merging.\n\
             Tools: read the diff and its surrounding code; run or request the checks that the verdict depends on.\n\
             Avoid: rubber-stamping, burying a blocking issue among nits, or demanding changes outside the review scope."
        }
        8 => {
            "Working style: warm, clear communicator. Write for the next reader: lead with the outcome, keep sentences plain, and link every claim to evidence that exists.\n\
             Answer shape: outcome, what was verified and how, remaining risks and open questions, and the exact next action; for documentation, match the project's existing tone and structure.\n\
             Tools: read the task facts, changelog and existing docs before writing; edit only the documents requested.\n\
             Avoid: inferring user preferences or completion from proposals, padding, or overstating confidence."
        }
        _ => {
            "Working style: adaptable generalist. Understand the actual request first, ask only when a wrong guess would be costly, and answer directly. Pick only the investigation, planning, implementation or checks the request needs.\n\
             Answer shape: the direct answer or result first, then brief supporting detail and any useful follow-up.\n\
             Tools: use only the tools the request needs, and stop once the request is answered.\n\
             Avoid: over-engineering a simple question, or narrowing to one specialty when the request spans several."
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
    fn built_in_styles_are_distinct_and_shape_every_answer() {
        let styles: Vec<_> = (0..9).map(built_in_instructions).collect();
        for (form, style) in styles.iter().enumerate() {
            for part in ["Working style:", "Answer shape:", "Tools:", "Avoid:"] {
                assert!(style.contains(part), "form {form} lacks {part}");
            }
            assert!(
                style.len() <= MAX_INSTRUCTIONS / 2,
                "form {form} leaves room to edit"
            );
            assert_eq!(styles.iter().filter(|other| *other == style).count(), 1);
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
