//! Ephemeral reviewed authoring. Portable bytes contain semantics, never local evidence or authority.
use super::{MAX_ENTRIES, MAX_RECIPE_BYTES, MAX_TEXT_BYTES, Recipe, RecipeStep};
use crate::workspace::{context, handoff, moves, runner};
use std::path::{Component, Path, PathBuf};

const PROVENANCE: &str = "# Eevee recipe: user-authored steps, with validation commands selected from prior reviewed passing local runs.\n# Historical provenance only: this file is not execution evidence or a claim about another or future run.\n# Loading and applying stage proposals only; every command, request, source selection and write needs fresh review.\n# No local evidence identities, permissions, sessions or credentials are transferred.\n";

/// Only the authoring editor supplies steps. Provider responses and retained plans are not inputs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub name: String,
    pub scope: String,
    pub criteria: String,
    /// One literal `Form: user-authored action` per line.
    pub steps: String,
    pub path: String,
    pub runs: Vec<String>,
}

/// The exact review is immutable. Local references are private and are never encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    generation: u64,
    task: i64,
    partner: i64,
    revision: u64,
    root: PathBuf,
    path: PathBuf,
    recipe: Recipe,
    runs: Vec<runner::RunRecord>,
    bytes: String,
}

pub fn command(root: &Path, task: i64, run: &runner::RunRecord) -> Result<Vec<String>, String> {
    if task <= 0 || run.prepared.task_id != task || !super::passed(run) {
        return Err(
            "Choose complete, stable, task-owned passing run evidence with available output".into(),
        );
    }
    let steps = moves::journal_steps(root, run)?;
    let moves::Steps::Run { cwd, .. } = &steps else {
        return Err("Only actual literal validation argv can be exported".into());
    };
    // Schema 1 has no per-command cwd: never silently move a subdirectory run to the root.
    if cwd != "." {
        return Err("Schema-1 validation commands require a run from the project root; subdirectory cwd cannot be represented".into());
    }
    let definition = moves::portable::Definition {
        schema_version: 1,
        parameters: None,
        name: "Validation command".into(),
        form: "Jolteon".into(),
        steps,
    };
    definition.validate()?;
    let moves::Steps::Run { argv, .. } = definition.steps else {
        return Err("Only literal validation argv can be exported".into());
    };
    Ok(argv)
}

impl Draft {
    pub fn preview(
        &self,
        generation: u64,
        task: i64,
        partner: i64,
        revision: u64,
        root: &Path,
        history: &[runner::RunRecord],
    ) -> Result<Export, String> {
        if task <= 0 || partner <= 0 || !root.is_absolute() || generation == 0 {
            return Err("Save an owned task before reviewing a workflow export".into());
        }
        if self.runs.is_empty() || self.runs.len() > MAX_ENTRIES {
            return Err("Select 1–32 actual passing validation commands".into());
        }
        let mut runs = Vec::with_capacity(self.runs.len());
        let mut commands = Vec::with_capacity(self.runs.len());
        for id in &self.runs {
            if runs
                .iter()
                .any(|run: &runner::RunRecord| run.prepared.run_id == *id)
            {
                return Err("A validation run is selected more than once".into());
            }
            let run = history
                .iter()
                .find(|run| run.prepared.run_id == *id)
                .ok_or("Selected validation evidence is no longer available")?;
            commands.push(command(root, task, run)?);
            runs.push(run.clone());
        }
        let mut steps = Vec::new();
        for line in self.steps.lines().filter(|line| !line.trim().is_empty()) {
            let (form, action) = line.split_once(':')
                .ok_or("Each user step must be Form: action (for example, Flareon: Make the reviewed change)")?;
            steps.push(RecipeStep {
                form: form.trim().into(),
                action: action.trim().into(),
            });
        }
        let recipe = Recipe {
            schema_version: 1,
            profiles: Vec::new(),
            name: self.name.trim().into(),
            scope: lines(&self.scope),
            context: Vec::new(),
            steps,
            validation_commands: commands,
            completion_criteria: lines(&self.criteria),
            roles: Vec::new(),
        };
        validate_portable(&recipe)?;
        for value in std::iter::once(recipe.name.as_str())
            .chain(recipe.steps.iter().map(|step| step.action.as_str()))
            .chain(recipe.completion_criteria.iter().map(String::as_str))
        {
            if history.iter().any(|run| {
                [&run.prepared.run_id, &run.prepared.request_id]
                    .iter()
                    .any(|id| !id.is_empty() && value.contains(id.as_str()))
            }) {
                return Err(
                    "Portable user steps and criteria cannot include local run/request identities"
                        .into(),
                );
            }
        }
        let path = output_path(root, &self.path)?;
        let bytes = encode(&recipe)?;
        Ok(Export {
            generation,
            task,
            partner,
            revision,
            root: root.into(),
            path,
            recipe,
            runs,
            bytes,
        })
    }
}

fn lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn portable_text(value: &str) -> Result<(), String> {
    super::safe_text(value, MAX_TEXT_BYTES)?;
    moves::validate_steps(
        3,
        &moves::Steps::Request {
            instruction: value.into(),
            pins: Vec::new(),
        },
    )?;
    if value.contains(['\\', '\r'])
        || handoff::without_local_provenance(value) != value
        || value.split_whitespace().any(|word| {
            let word = word.trim_matches(['\'', '"', '`', '(', ')', '[', ']', ',', ';']);
            word.split('=').any(|part| {
                let part = part.trim_matches(['\'', '"', '`']);
                part.starts_with('/')
                    || part.starts_with('~')
                    || part.len() > 2 && part.as_bytes().get(1) == Some(&b':')
            }) || ["task:", "run:", "source:", "thread:", "turn:", "session:"]
                .iter()
                .any(|prefix| word.starts_with(prefix))
        })
    {
        return Err("Portable workflow text cannot contain machine-specific paths or local evidence/session identities".into());
    }
    Ok(())
}

fn validate_portable(recipe: &Recipe) -> Result<(), String> {
    recipe.validate()?;
    if recipe
        .completion_criteria
        .iter()
        .any(|criterion| criterion.len() > 256)
    {
        return Err(
            "Authored recipe criteria must fit the task's 256 UTF-8 byte criterion bound".into(),
        );
    }
    if recipe.schema_version != 1
        || !recipe.context.is_empty()
        || !recipe.roles.is_empty()
        || recipe.validation_commands.is_empty()
    {
        return Err(
            "Authored workflows contain schema-1 user steps and actual validation commands only"
                .into(),
        );
    }
    portable_text(&recipe.name)?;
    for text in recipe
        .steps
        .iter()
        .map(|step| step.action.as_str())
        .chain(recipe.completion_criteria.iter().map(String::as_str))
    {
        portable_text(text)?;
    }
    for argv in &recipe.validation_commands {
        moves::portable::Definition {
            schema_version: 1,
            parameters: None,
            name: "Validation command".into(),
            form: "Jolteon".into(),
            steps: moves::Steps::Run {
                argv: argv.clone(),
                cwd: ".".into(),
            },
        }
        .validate()?;
    }
    Ok(())
}

fn encode(recipe: &Recipe) -> Result<String, String> {
    validate_portable(recipe)?;
    let mut bytes = String::from(PROVENANCE);
    bytes.push_str(&toml::to_string_pretty(recipe).map_err(|_| "Cannot encode workflow recipe")?);
    if bytes.len() > MAX_RECIPE_BYTES {
        return Err("Workflow recipe exceeds 64 KiB".into());
    }
    Ok(bytes)
}

fn output_path(root: &Path, value: &str) -> Result<PathBuf, String> {
    super::text(value, MAX_TEXT_BYTES)?;
    let path = Path::new(value);
    if value.chars().any(char::is_control)
        || path.extension().and_then(|value| value.to_str()) != Some("toml")
        || path.components().any(|part| match part {
            Component::ParentDir => true,
            Component::Normal(name) => context::excluded(Path::new(name), &[]),
            _ => false,
        })
    {
        return Err("Choose a literal non-secret new .toml file path without traversal".into());
    }
    Ok(if path.is_absolute() {
        path.into()
    } else {
        root.join(path)
    })
}

impl Export {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn owner(&self) -> (i64, i64) {
        (self.task, self.partner)
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn bytes(&self) -> &str {
        &self.bytes
    }
    pub fn runs(&self) -> &[runner::RunRecord] {
        &self.runs
    }
    pub fn matches(
        &self,
        generation: u64,
        task: i64,
        partner: i64,
        revision: u64,
        root: &Path,
    ) -> bool {
        self.generation == generation
            && self.owner() == (task, partner)
            && self.revision == revision
            && self.root == root
    }
    /// Worker-only after persisted owner/revision and exact journal admission.
    pub(crate) fn publish(&self) -> Result<(), String> {
        if encode(&self.recipe)? != self.bytes
            || self.recipe.validation_commands.len() != self.runs.len()
        {
            return Err("Workflow export changed after review".into());
        }
        for (run, expected) in self.runs.iter().zip(&self.recipe.validation_commands) {
            if command(&self.root, self.task, run)? != *expected {
                return Err("Validation command changed after review".into());
            }
            // Keep the existing no-link/protected-path runtime policy, not a second path validator.
            moves::run_steps(&self.root, run)?;
        }
        let mut proposal = moves::Proposal {
            extra: Default::default(),
            partner: self.partner,
            form: 3,
            name: "Workflow scope".into(),
            scope: moves::Scope::Project,
            steps: moves::Steps::Request {
                instruction: "Review the workflow scope".into(),
                pins: Vec::with_capacity(1),
            },
            provenance: moves::Provenance {
                task_id: self.task,
                sources: Vec::new(),
                taught_by: None,
                teacher_name: None,
            },
        };
        // The move validator permits missing selectors but refuses links and protected paths.
        // Its pin limit is 16; validate each selector independently without changing recipe's 32-path bound.
        for path in &self.recipe.scope {
            if let moves::Steps::Request { pins, .. } = &mut proposal.steps {
                pins.clear();
                pins.push(moves::Pin {
                    path: path.clone(),
                    start_line: 1,
                    end_line: 1,
                });
            }
            moves::validate_for_root(&self.root, &proposal)?;
        }
        handoff::write_new_bytes(&self.path, self.bytes.as_bytes())
    }
}

#[cfg(test)]
mod tests;
