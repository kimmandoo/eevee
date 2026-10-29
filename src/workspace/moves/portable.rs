//! Versioned semantics only: local identities, evidence and permissions are absent.
use super::{MAX_MOVE_BYTES, Proposal, Provenance, Scope, Steps};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub schema_version: u32,
    pub name: String,
    pub form: String,
    pub steps: Steps,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<super::parameters::Template>,
}

impl Definition {
    pub fn from_proposal(proposal: &Proposal) -> Result<Self, String> {
        super::validate_proposal(proposal)?;
        let parameters = super::parameters::Template::of(proposal)?;
        let definition = Self {
            schema_version: if parameters.is_some() { 2 } else { 1 },
            name: proposal.name.clone(),
            form: crate::domain::FORMS[proposal.form].name.into(),
            steps: proposal.steps.clone(),
            parameters,
        };
        definition.validate()?;
        Ok(definition)
    }
    pub fn form_index(&self) -> Result<usize, String> {
        crate::domain::FORMS
            .iter()
            .position(|form| form.name == self.form)
            .ok_or_else(|| "TM form is unsupported".into())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != if self.parameters.is_some() { 2 } else { 1 } {
            return Err("TM1 is literal; TM2 requires a typed request parameter template".into());
        }
        if let Some(template) = &self.parameters {
            template.validate(&self.steps)?;
        }
        super::validate_name(&self.name)?;
        super::validate_steps(self.form_index()?, &self.steps)?;
        if let Steps::Run { argv, .. } = &self.steps {
            for argument in argv {
                let path = super::argument_path(argument);
                if Path::new(path).is_absolute()
                    || path.contains('\\')
                    || path.as_bytes().get(1) == Some(&b':')
                {
                    return Err("Portable run arguments must not contain absolute or platform-specific paths".into());
                }
            }
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<String, String> {
        self.validate()?;
        let text = toml::to_string_pretty(self).map_err(|_| "Cannot encode TM definition")?;
        if text.len() > MAX_MOVE_BYTES {
            return Err("TM exceeds the 64 KiB limit".into());
        }
        Ok(text)
    }
    pub fn decode(text: &str) -> Result<Self, String> {
        if text.len() > MAX_MOVE_BYTES {
            return Err("TM exceeds the 64 KiB limit".into());
        }
        super::recoverable_text(text)?;
        let definition: Self = toml::from_str(text)
            .map_err(|_| "Invalid TM TOML or unknown fields; nothing learned".to_string())?;
        definition.validate()?;
        Ok(definition)
    }
    /// This is a new local learning source, not imported teacher or execution evidence.
    pub fn proposal(&self, task_id: i64, partner: i64, scope: Scope) -> Result<Proposal, String> {
        let digest = format!("{:x}", Sha256::digest(self.encode()?.as_bytes()));
        let mut proposal = Proposal {
            extra: Default::default(),
            partner,
            form: self.form_index()?,
            name: self.name.clone(),
            scope,
            steps: self.steps.clone(),
            provenance: Provenance {
                task_id,
                sources: vec![format!("tm:{digest}")],
                taught_by: None,
                teacher_name: None,
            },
        };
        if let Some(template) = &self.parameters {
            template.store(&mut proposal)?;
        }
        super::validate_proposal(&proposal)?;
        Ok(proposal)
    }
}

pub fn read_file(path: &Path) -> Result<Definition, String> {
    validate_file_path(path)?;
    Definition::decode(&crate::workspace::handoff::read_bounded(
        path,
        MAX_MOVE_BYTES,
    )?)
}
pub fn write_new(path: &Path, definition: &Definition) -> Result<(), String> {
    validate_file_path(path)?;
    crate::workspace::handoff::write_new_bytes(path, definition.encode()?.as_bytes())
}
fn validate_file_path(path: &Path) -> Result<(), String> {
    let value = path.to_str().ok_or("TM filename must be Unicode")?;
    super::recoverable_text(value)?;
    if value.is_empty()
        || value.len() > 4096
        || value.chars().any(char::is_control)
        || path.components().any(|component| match component {
            std::path::Component::Normal(name) => {
                crate::workspace::context::excluded(Path::new(name), &[])
            }
            std::path::Component::ParentDir => true,
            _ => false,
        })
    {
        return Err("Choose a literal non-secret TM file path within 4096 bytes".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn definitions() -> Vec<Definition> {
        (0..9)
            .map(|form| Definition {
                parameters: None,
                schema_version: 1,
                name: format!("Portable {form}"),
                form: crate::domain::FORMS[form].name.into(),
                steps: match form {
                    0 => Steps::Task {
                        goal: "Inspect change".into(),
                        criteria: vec!["Result reviewed".into()],
                    },
                    1 => Steps::Pins {
                        pins: vec![super::super::Pin {
                            path: "missing.txt".into(),
                            start_line: 1,
                            end_line: 2,
                        }],
                    },
                    2 | 5 | 6 => Steps::Run {
                        argv: vec!["cargo".into(), "test".into(), "".into()],
                        cwd: ".".into(),
                    },
                    _ => Steps::Request {
                        instruction: "Review the change".into(),
                        pins: vec![],
                    },
                },
            })
            .collect()
    }
    #[test]
    fn every_form_roundtrips_without_local_authority() {
        for definition in definitions() {
            let text = definition.encode().unwrap();
            assert_eq!(Definition::decode(&text).unwrap(), definition);
            let proposal = definition.proposal(17, 23, Scope::Personal).unwrap();
            assert_eq!(proposal.provenance.task_id, 17);
            assert_eq!(proposal.partner, 23);
            assert_eq!(proposal.provenance.taught_by, None);
            assert_eq!(proposal.provenance.teacher_name, None);
            assert!(proposal.provenance.sources[0].starts_with("tm:"));
            assert_eq!(Definition::from_proposal(&proposal).unwrap(), definition);
            for key in [
                "task_id",
                "partner",
                "sources",
                "taught_by",
                "usage",
                "approval",
                "session",
            ] {
                assert!(!text.contains(key));
                assert!(Definition::decode(&format!("{key} = 7\n{text}")).is_err());
            }
        }
    }
    #[test]
    fn refuses_versions_secrets_paths_and_extra_capabilities() {
        let mut definition = definitions().remove(2);
        definition.schema_version = 2;
        assert!(definition.encode().is_err());
        definition.schema_version = 1;
        for argument in [
            "../outside",
            "/private/file",
            "C:\\file",
            "--config=.env",
            "--token=sk-1234567890123456789012345678901234567890",
            "[MASKED]",
        ] {
            definition.steps = Steps::Run {
                argv: vec!["cargo".into(), argument.into()],
                cwd: ".".into(),
            };
            assert!(definition.encode().is_err(), "{argument}");
        }
        assert!(Definition::decode(&"x".repeat(MAX_MOVE_BYTES + 1)).is_err());
        let text = definitions().remove(0).encode().unwrap();
        assert!(Definition::decode(&format!("{text}\ngrants = [\"write\"]\n")).is_err());
    }
    #[test]
    fn create_only_files_missing_pins_and_link_refusal() {
        let root = std::env::temp_dir().join(format!(
            "ev-tm-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let definition = definitions().remove(1);
        let path = root.join("move.toml");
        write_new(&path, &definition).unwrap();
        assert_eq!(read_file(&path).unwrap(), definition);
        let bytes = std::fs::read(&path).unwrap();
        assert!(write_new(&path, &definitions().remove(0)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        super::super::validate_steps_at(&root, 1, &definition.steps).unwrap();
        std::fs::write(root.join("large.toml"), vec![b'x'; MAX_MOVE_BYTES + 1]).unwrap();
        assert!(read_file(&root.join("large.toml")).is_err());
        assert!(read_file(&root.join("../outside")).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&path, root.join("link.toml")).unwrap();
            assert!(read_file(&root.join("link.toml")).is_err());
            assert!(write_new(&root.join("link.toml"), &definition).is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn xc07_tm2_preserves_parameters_and_tm1_stays_literal() {
        let steps = Steps::Request {
            instruction: "fix the bug in src/b.rs".into(),
            pins: vec![],
        };
        let mut proposal = definitions()
            .remove(3)
            .proposal(1, 1, Scope::Project)
            .unwrap();
        proposal.steps = steps;
        let template = super::super::parameters::Template::derive(
            &proposal.steps,
            vec![
                "fix the bug in src/a.rs".into(),
                "fix the bug in src/b.rs".into(),
            ],
        )
        .unwrap()
        .unwrap();
        template.store(&mut proposal).unwrap();
        let definition = Definition::from_proposal(&proposal).unwrap();
        assert_eq!(definition.schema_version, 2);
        let encoded = definition.encode().unwrap();
        let decoded = Definition::decode(&encoded).unwrap();
        assert_eq!(decoded, definition);
        let imported = decoded.proposal(2, 3, Scope::Personal).unwrap();
        assert_eq!(
            super::super::parameters::Template::of(&imported).unwrap(),
            Some(template.clone())
        );
        assert_eq!(
            template.fill(&imported.steps, "[\"src/c.rs\"]").unwrap(),
            "fix the bug in src/c.rs"
        );
        let literal = definitions().remove(3);
        assert_eq!(literal.schema_version, 1);
        assert!(!literal.encode().unwrap().contains("parameters"));
        assert!(
            Definition::decode(&encoded.replace("schema_version = 2", "schema_version = 1"))
                .is_err()
        );
    }
}
