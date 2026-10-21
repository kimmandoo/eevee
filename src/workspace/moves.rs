//! Local reusable actions. Labels and provenance are never provider instructions or grants.
use std::{
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

use super::{
    context, privacy, runner,
    workflows::{self, Recipe, RecipeStep},
};

pub const MAX_MOVE_BYTES: usize = 64 * 1024;
const MAX_NAME_BYTES: usize = 256;
const MAX_NAME_GRAPHEMES: usize = 64;
const MAX_SOURCES: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Scope {
    #[default]
    Project,
    Personal,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "Project",
            Self::Personal => "Personal",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
}

impl Pin {
    pub fn from_excerpt(excerpt: &context::Excerpt) -> Result<Self, String> {
        excerpt.validate()?;
        let path = portable_relative(Path::new(&excerpt.path))?;
        literal_path(&path)?;
        Ok(Self {
            path,
            start_line: excerpt.start_line,
            end_line: excerpt.end_line,
        })
    }
    pub fn matches_excerpt(&self, excerpt: &context::Excerpt) -> bool {
        Path::new(&self.path) == Path::new(&excerpt.path)
            && self.start_line == excerpt.start_line
            && self.end_line == excerpt.end_line
    }
}

fn portable_relative(path: &Path) -> Result<String, String> {
    let mut value = String::new();
    for component in path.components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(name) => {
                if !value.is_empty() {
                    value.push('/');
                }
                value.push_str(name.to_str().ok_or("Move path is not Unicode")?);
            }
            _ => return Err("Move paths require literal relative components".into()),
        }
    }
    Ok(value)
}

pub fn relative_cwd(root: &Path, cwd: &Path) -> Result<String, String> {
    let relative = if cwd.is_absolute() {
        cwd.strip_prefix(root)
            .map_err(|_| "Run working directory is outside this project")?
    } else {
        cwd
    };
    let value = portable_relative(relative)?;
    if value.is_empty() {
        Ok(".".into())
    } else {
        validate_cwd(&value)?;
        Ok(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Steps {
    Run { argv: Vec<String>, cwd: String },
    Pins { pins: Vec<Pin> },
    Request { instruction: String, pins: Vec<Pin> },
    Task { goal: String, criteria: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub task_id: i64,
    pub sources: Vec<String>,
    pub taught_by: Option<i64>,
    pub teacher_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub partner: i64,
    pub form: usize,
    pub name: String,
    pub scope: Scope,
    pub steps: Steps,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: i64,
    pub revision: u64,
    pub project: String,
    pub proposal: Proposal,
    pub forgotten: bool,
    pub created: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub key: String,
    pub proposal: Proposal,
    pub uses: u64,
    pub sources: Vec<String>,
    pub postponed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Outcome {
    Pending,
    Completed,
    Succeeded,
    Failed,
    Denied,
    Cancelled,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub id: String,
    pub move_id: i64,
    pub task_id: i64,
    pub partner: i64,
    pub run_id: Option<String>,
    pub request_id: Option<String>,
    pub outcome: Outcome,
    pub revision: Option<String>,
}

pub fn validate_name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_NAME_BYTES
        || value.graphemes(true).count() > MAX_NAME_GRAPHEMES
        || value
            .chars()
            .any(|ch| ch.is_control() || unsafe_direction(ch))
    {
        return Err(
            "Choose a trimmed name of 1–64 characters (at most 256 bytes), without controls".into(),
        );
    }
    recoverable_text(value)?;
    Ok(())
}

pub fn validate_proposal(proposal: &Proposal) -> Result<(), String> {
    if proposal.partner <= 0 || proposal.provenance.task_id <= 0 {
        return Err("Save a task and choose an existing partner before teaching a move".into());
    }
    validate_name(&proposal.name)?;
    let provenance = &proposal.provenance;
    if provenance.sources.len() > MAX_SOURCES {
        return Err("Move provenance exceeds 16 source identifiers".into());
    }
    for (index, source) in provenance.sources.iter().enumerate() {
        if source.is_empty() || source.len() > 256 || source.chars().any(char::is_control) {
            return Err("Move source identifiers must be nonempty bounded local references".into());
        }
        recoverable_text(source)?;
        if provenance.sources[..index].contains(source) {
            return Err("Move source identifiers must be distinct".into());
        }
    }
    match (provenance.taught_by, &provenance.teacher_name) {
        (None, None) => {}
        (Some(partner), Some(name)) if partner > 0 && partner != proposal.partner => {
            // Preserve the full stored attribution, including legacy nicknames;
            // teaching labels are not constrained by the editable move-name cap.
            if name.is_empty() || name.len() > 512 || name.chars().any(char::is_control) {
                return Err("Teaching partner labels must fit the 512-byte attribution bound without controls".into());
            }
            recoverable_text(name)?;
        }
        _ => {
            return Err("A copied move requires a distinct teaching partner and their name".into());
        }
    }
    bounded(proposal)?;
    validate_steps(proposal.form, &proposal.steps)
}

/// Syntax-only admission. Project paths must additionally pass `validate_steps_at`
/// before storage or use; a personal move may contain an old project's absolute argv.
pub fn validate_steps(form: usize, steps: &Steps) -> Result<(), String> {
    let compatible = matches!(
        (form, steps),
        (0, Steps::Task { .. })
            | (1, Steps::Pins { .. })
            | (2 | 5 | 6, Steps::Run { .. })
            | (3 | 4 | 7 | 8, Steps::Request { .. })
    );
    if !compatible {
        return Err("Move kind does not belong to this form".into());
    }
    bounded(steps)?;
    // Use the established recipe validator, with fact-only synthetic labels and
    // criteria. A move's display name never becomes an action or a provider packet.
    let mut recipe = Recipe {
        schema_version: 1,
        name: "Reusable local action".into(),
        scope: Vec::new(),
        context: Vec::new(),
        steps: vec![RecipeStep {
            form: crate::domain::FORMS[form].name.into(),
            action: "Stage the selected action for normal independent review".into(),
        }],
        validation_commands: Vec::new(),
        completion_criteria: vec![
            "Review the actual result independently; no success is inferred".into(),
        ],
    };
    match steps {
        Steps::Run { argv, cwd } => {
            validate_argv(argv)?;
            validate_cwd(cwd)?;
            if cwd != "." {
                recipe.scope.push(cwd.clone());
            }
            recipe.validation_commands.push(argv.clone());
        }
        Steps::Pins { pins } => {
            validate_pins(pins, false)?;
            recipe
                .context
                .extend(pins.iter().map(|pin| pin.path.clone()));
        }
        Steps::Request { instruction, pins } => {
            validate_pins(pins, true)?;
            user_text(instruction)?;
            recipe.steps[0].action.clone_from(instruction);
            recipe
                .context
                .extend(pins.iter().map(|pin| pin.path.clone()));
        }
        Steps::Task { goal, criteria } => {
            user_text(goal)?;
            if criteria.is_empty() || criteria.len() > workflows::MAX_ENTRIES {
                return Err("Task moves require 1–32 user-authored completion criteria".into());
            }
            for criterion in criteria {
                user_text(criterion)?;
            }
            recipe.steps[0].action.clone_from(goal);
            recipe.completion_criteria.clone_from(criteria);
        }
    }
    recipe.validate()
}

pub fn validate_for_root(root: &Path, proposal: &Proposal) -> Result<(), String> {
    validate_proposal(proposal)?;
    validate_paths_at(root, &proposal.steps)
}

/// Resolve no captured bytes and grant nothing. Missing pins stay selectors;
/// source resolution at use time reports availability and applies custom exclusions.
#[cfg(test)]
pub fn validate_steps_at(root: &Path, form: usize, steps: &Steps) -> Result<(), String> {
    validate_steps(form, steps)?;
    validate_paths_at(root, steps)
}

fn validate_paths_at(root: &Path, steps: &Steps) -> Result<(), String> {
    let canonical = canonical_root(root)?;
    match steps {
        Steps::Run { argv, cwd } => validate_run_paths(root, &canonical, argv, cwd),
        Steps::Pins { pins } | Steps::Request { pins, .. } => {
            for pin in pins {
                no_links(&canonical, Path::new(&pin.path), false)?;
            }
            Ok(())
        }
        Steps::Task { .. } => Ok(()),
    }
}

/// Validate a teachable run against the current project's filesystem.
pub fn run_steps(root: &Path, record: &runner::RunRecord) -> Result<Steps, String> {
    let canonical = canonical_root(root)?;
    let steps = journal_steps(&canonical, record)?;
    if let Steps::Run { argv, cwd } = &steps {
        validate_run_paths(root, &canonical, argv, cwd)?;
    }
    Ok(steps)
}

/// Extract historical journal facts without requiring the old project or cwd to
/// still exist. `root` is the stored absolute project identity. Commitment and
/// correspondence to the requesting task/partner are additional store checks.
pub fn journal_steps(root: &Path, record: &runner::RunRecord) -> Result<Steps, String> {
    if !root.is_absolute() {
        return Err("Historical run extraction requires an absolute project identity".into());
    }
    let prepared = &record.prepared;
    if prepared.task_id <= 0
        || !crate::store::valid_run_id(&prepared.run_id)
        || !crate::store::valid_run_id(&prepared.request_id)
    {
        return Err("Choose a committed run with valid task and run identifiers".into());
    }
    let attribution = prepared
        .partner
        .as_ref()
        .ok_or("Runs without partner attribution cannot teach a move")?;
    attribution.validate()?;
    if record.started_at_ms.is_none()
        || record.tree == runner::TreeOutcome::NotStarted
        || record.state == runner::RunState::AwaitingDispatch
    {
        return Err("Only an approved run that actually started can teach a move".into());
    }
    let runner::CommandSpec::Direct { program, args } = &prepared.spec else {
        return Err("Shell runs cannot teach a literal argv move".into());
    };
    if args.len() >= 64
        || std::iter::once(program)
            .chain(args)
            .any(|arg| arg.len() > workflows::MAX_TEXT_BYTES)
    {
        return Err("Run moves require at most 64 bounded literal argv entries".into());
    }
    let cwd = relative_cwd(root, &prepared.cwd)?;
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(program.clone());
    argv.extend(args.iter().cloned());
    let steps = Steps::Run { argv, cwd };
    // A diagnostic/measurement run can be taught to its own form; attribution
    // from other forms still stages only a Jolteon-compatible literal run.
    let form = if matches!(attribution.form, 2 | 5 | 6) {
        attribution.form
    } else {
        2
    };
    validate_steps(form, &steps)?;
    if let Steps::Run { argv, .. } = &steps {
        for argument in argv {
            let path = Path::new(argument_path(argument));
            if path.is_absolute() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| "An absolute run argument is outside this project")?;
                if !relative.as_os_str().is_empty() && context::excluded(relative, &[]) {
                    return Err("Run paths cannot select credential or excluded files".into());
                }
            }
        }
    }
    Ok(steps)
}

pub fn pin_steps(excerpts: &[context::Excerpt]) -> Result<Steps, String> {
    if excerpts.is_empty() || excerpts.len() > context::MAX_EXCERPTS {
        return Err("Choose 1–16 available source pins".into());
    }
    let first = &excerpts[0];
    let mut pins = Vec::with_capacity(excerpts.len());
    for excerpt in excerpts {
        excerpt.validate()?;
        if !excerpt.available {
            return Err("Unavailable source excerpts cannot teach a pin set".into());
        }
        if excerpt.task_id != first.task_id || excerpt.root != first.root {
            return Err("A pin set must belong to one task in one project".into());
        }
        pins.push(Pin::from_excerpt(excerpt)?);
    }
    pins.sort_unstable_by(pin_order);
    pins.dedup();
    let steps = Steps::Pins { pins };
    validate_steps(1, &steps)?;
    Ok(steps)
}

/// Pins are sets; argv and cwd are exact ordered literals, including empty args.
/// Hash a canonical serialization, never captured text, result output or a label.
pub fn fingerprint(steps: &Steps) -> Result<String, String> {
    validate_steps(kind_form(steps), steps)?;
    let mut hash = Sha256::new();
    match steps {
        Steps::Pins { pins } => {
            let pins = canonical_pins(pins);
            serde_json::to_writer(HashWriter(&mut hash), &("Pins", pins))
        }
        Steps::Request { instruction, pins } => {
            let pins = canonical_pins(pins);
            serde_json::to_writer(HashWriter(&mut hash), &("Request", instruction, pins))
        }
        _ => serde_json::to_writer(HashWriter(&mut hash), steps),
    }
    .map_err(|_| "Cannot fingerprint move steps".to_string())?;
    Ok(format!("{:x}", hash.finalize()))
}

pub fn suggested_name(form: usize, steps: &Steps, used: &[String]) -> String {
    let names = match form {
        0 => &NORMAL_NAMES,
        1 => &WATER_NAMES,
        2 => &ELECTRIC_NAMES,
        3 => &FIRE_NAMES,
        4 => &PSYCHIC_NAMES,
        5 => &DARK_NAMES,
        6 => &GRASS_NAMES,
        7 => &ICE_NAMES,
        8 => &FAIRY_NAMES,
        _ => &NORMAL_NAMES,
    };
    let semantic = match (form, steps) {
        (2, Steps::Run { argv, .. })
            if argv
                .iter()
                .any(|arg| arg == "test" || arg == "check" || arg == "pytest") =>
        {
            Some("Thunderbolt")
        }
        (2, Steps::Run { argv, .. })
            if argv.iter().any(|arg| arg == "build" || arg == "compile") =>
        {
            Some("Charge Beam")
        }
        (4, Steps::Request { instruction, .. })
            if contains_word(instruction, &["plan", "design", "architecture"]) =>
        {
            Some("Future Sight")
        }
        (7, Steps::Request { instruction, .. })
            if contains_word(instruction, &["review", "audit", "readiness"]) =>
        {
            Some("Ice Beam")
        }
        (8, Steps::Request { instruction, .. })
            if contains_word(instruction, &["handoff", "document", "docs"]) =>
        {
            Some("Moonblast")
        }
        _ => None,
    };
    for name in semantic.into_iter().chain(names.iter().copied()) {
        if !used.iter().any(|used| caseless_eq(used, name)) {
            return name.into();
        }
    }
    // Every canonical label is already present in the bounded reminder history.
    // A local numeric disambiguator is a label only, never executable semantics.
    let mut suffix = 2u64;
    loop {
        let name = format!("{} {suffix}", names[0]);
        if !used.iter().any(|used| caseless_eq(used, &name)) {
            return name;
        }
        suffix = suffix.saturating_add(1);
    }
}

fn kind_form(steps: &Steps) -> usize {
    match steps {
        Steps::Task { .. } => 0,
        Steps::Pins { .. } => 1,
        Steps::Run { .. } => 2,
        Steps::Request { .. } => 4,
    }
}

fn unsafe_direction(ch: char) -> bool {
    matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn recoverable_text(value: &str) -> Result<(), String> {
    if privacy::redact(value) != value
        || value
            .as_bytes()
            .windows(10)
            .any(|part| part.eq_ignore_ascii_case(b"[REDACTED]"))
        || value
            .as_bytes()
            .windows(8)
            .any(|part| part.eq_ignore_ascii_case(b"[MASKED]"))
        || value
            .as_bytes()
            .windows(10)
            .any(|part| part.eq_ignore_ascii_case(b"<redacted>"))
        || value
            .as_bytes()
            .windows(8)
            .any(|part| part.eq_ignore_ascii_case(b"<masked>"))
        || (value.len() >= 3 && value.chars().all(|ch| matches!(ch, '*' | '•')))
    {
        return Err(
            "Moves cannot store credentials or irrecoverably redacted/masked values".into(),
        );
    }
    if value.chars().any(unsafe_direction) {
        return Err("Moves cannot contain hidden direction controls".into());
    }
    Ok(())
}

fn user_text(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > workflows::MAX_TEXT_BYTES {
        return Err("Move user-authored text must contain 1–4096 bytes".into());
    }
    recoverable_text(value)
}

fn literal_path(value: &str) -> Result<(), String> {
    recoverable_text(value)?;
    if value.is_empty()
        || value.len() > workflows::MAX_TEXT_BYTES
        || value.chars().any(char::is_control)
        || value.contains([':', '\\', '*', '?', '[', ']'])
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || context::excluded(Path::new(value), &[])
    {
        return Err(
            "Choose a literal project-relative non-secret path without traversal or globs".into(),
        );
    }
    Ok(())
}

fn validate_cwd(cwd: &str) -> Result<(), String> {
    if cwd == "." {
        Ok(())
    } else {
        literal_path(cwd)
    }
}

fn validate_pins(pins: &[Pin], empty_allowed: bool) -> Result<(), String> {
    if (!empty_allowed && pins.is_empty()) || pins.len() > context::MAX_EXCERPTS {
        return Err("Pin-set moves require 1–16 selectors; request moves allow 0–16".into());
    }
    for pin in pins {
        literal_path(&pin.path)?;
        if pin.start_line == 0 || pin.end_line < pin.start_line {
            return Err("Pin selectors require a valid inclusive line range starting at 1".into());
        }
    }
    Ok(())
}

fn validate_argv(argv: &[String]) -> Result<(), String> {
    if argv.is_empty() || argv.len() > 64 || argv[0].trim().is_empty() {
        return Err("Run moves require a program and at most 64 literal argv entries".into());
    }
    let program = Path::new(&argv[0])
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if shell_program(program)
        || matches!(program, "env" | "env.exe")
        || (matches!(program, "busybox" | "toybox")
            && argv.get(1).is_some_and(|arg| shell_program(arg)))
    {
        return Err("Shell and environment launchers cannot teach literal run moves".into());
    }
    for argument in argv {
        if argument.len() > workflows::MAX_TEXT_BYTES || argument.chars().any(char::is_control) {
            return Err("Run argv entries must be bounded literals without controls".into());
        }
        recoverable_text(argument.trim_start_matches('-'))?;
        if argument.split_once('=').is_some_and(|(name, _)| {
            name.as_bytes()
                .first()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        }) {
            return Err("Environment assignments cannot be stored in run moves".into());
        }
        let path = argument_path(argument);
        let native = Path::new(path);
        if path.starts_with('~')
            || !cfg!(windows) && path.contains('\\')
            || native
                .components()
                .any(|component| matches!(component, Component::ParentDir))
            || native.has_root() && !native.is_absolute()
            || path.len() >= 2
                && path.as_bytes()[0].is_ascii_alphabetic()
                && path.as_bytes()[1] == b':'
                && !native.is_absolute()
        {
            return Err(
                "Run paths cannot use traversal, home expansion or platform-ambiguous roots".into(),
            );
        }
        if !Path::new(path).is_absolute() {
            let normalized = path.strip_prefix("./").unwrap_or(path);
            if normalized != "."
                && !normalized.is_empty()
                && context::excluded(Path::new(normalized), &[])
            {
                return Err("Run paths cannot select credential or excluded files".into());
            }
        }
    }
    Ok(())
}

fn shell_program(program: &str) -> bool {
    [
        "sh",
        "bash",
        "zsh",
        "dash",
        "fish",
        "ksh",
        "csh",
        "tcsh",
        "cmd",
        "cmd.exe",
        "powershell",
        "powershell.exe",
        "pwsh",
        "pwsh.exe",
    ]
    .iter()
    .any(|shell| program.eq_ignore_ascii_case(shell))
}

fn argument_path(argument: &str) -> &str {
    if argument.starts_with('-')
        && let Some((_, value)) = argument.split_once('=')
    {
        return value;
    }
    for prefix in ["-isystem", "-iquote", "-include", "-I", "-L", "-F", "@"] {
        if let Some(path) = argument.strip_prefix(prefix)
            && !path.is_empty()
        {
            return path;
        }
    }
    // Common joined short options (-C/path, -o/path) still carry a path.
    if argument
        .as_bytes()
        .get(1)
        .is_some_and(u8::is_ascii_alphabetic)
        && argument.starts_with('-')
        && argument.get(2..).is_some_and(|value| {
            Path::new(value).is_absolute() || value.starts_with("./") || value.starts_with("../")
        })
    {
        return &argument[2..];
    }
    argument
}

fn canonical_root(root: &Path) -> Result<PathBuf, String> {
    let canonical = root
        .canonicalize()
        .map_err(|_| "Project root is missing or unavailable")?;
    if !canonical.is_dir() {
        return Err("Project root must be a directory".into());
    }
    Ok(canonical)
}

fn validate_run_paths(
    root: &Path,
    canonical: &Path,
    argv: &[String],
    cwd: &str,
) -> Result<(), String> {
    // A bare program is resolved through PATH, but every argument value can name
    // a file relative to cwd even when it has no slash (for example `cat input`).
    no_links(canonical, Path::new(cwd), true)?;
    for (index, argument) in argv.iter().enumerate() {
        let value = argument_path(argument);
        let path = Path::new(value);
        if path.is_absolute() {
            let relative = path
                .strip_prefix(canonical)
                .or_else(|_| path.strip_prefix(root))
                .map_err(|_| "An absolute run argument is outside this project")?;
            if !relative.as_os_str().is_empty() {
                if context::excluded(relative, &[]) {
                    return Err("Run paths cannot select credential or excluded files".into());
                }
                no_links(canonical, relative, false)?;
            }
        } else if !(argument.starts_with('-') && value == argument.as_str()
            || index == 0
                && !value.contains(['/', '\\'])
                && matches!(path.components().next(), Some(Component::Normal(_))))
        {
            let path = Path::new(cwd).join(path);
            no_links(canonical, &path, false)?;
        }
    }
    Ok(())
}

fn no_links(root: &Path, relative: &Path, require_directory: bool) -> Result<(), String> {
    let mut path = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(name) => path.push(name),
            _ => return Err("Move paths must remain inside the project without traversal".into()),
        }
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("Move paths cannot traverse symbolic links".into());
            }
            Ok(metadata) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err("Move paths cannot traverse reparse points".into());
                    }
                }
                #[cfg(not(windows))]
                let _ = metadata;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && !require_directory => {
                return Ok(());
            }
            Err(_) => return Err("Move working directory or path is missing or unavailable".into()),
        }
    }
    if require_directory && !path.is_dir() {
        return Err("Move working directory is missing or is not a directory".into());
    }
    Ok(())
}

fn pin_order(left: &Pin, right: &Pin) -> std::cmp::Ordering {
    (&left.path, left.start_line, left.end_line).cmp(&(
        &right.path,
        right.start_line,
        right.end_line,
    ))
}

fn canonical_pins(pins: &[Pin]) -> Vec<&Pin> {
    let mut pins: Vec<_> = pins.iter().collect();
    pins.sort_unstable_by(|left, right| pin_order(left, right));
    pins.dedup();
    pins
}

fn caseless_eq(left: &str, right: &str) -> bool {
    left.nfkc()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .eq(right
            .nfkc()
            .flat_map(char::to_uppercase)
            .flat_map(char::to_lowercase))
}

fn contains_word(text: &str, words: &[&str]) -> bool {
    text.split(|ch: char| !ch.is_alphanumeric()).any(|word| {
        words
            .iter()
            .any(|candidate| word.eq_ignore_ascii_case(candidate))
    })
}

fn bounded<T: Serialize>(value: &T) -> Result<(), String> {
    struct Bound(usize);
    impl Write for Bound {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self
                .0
                .checked_sub(bytes.len())
                .ok_or_else(|| io::Error::other("Move exceeds 64 KiB"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Bound(MAX_MOVE_BYTES), value)
        .map_err(|_| "Move exceeds its 64 KiB serialization bound".into())
}

struct HashWriter<'a>(&'a mut Sha256);
impl Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// Fixed local naming vocabulary, selected by form and facts only. Labels never
// enter a request packet and never imply power, accuracy, approval or correctness.
const NORMAL_NAMES: [&str; 8] = [
    "Helping Hand",
    "Quick Attack",
    "Swift",
    "Tackle",
    "Baton Pass",
    "Work Up",
    "Protect",
    "Last Resort",
];
const WATER_NAMES: [&str; 8] = [
    "Surf",
    "Water Pulse",
    "Aqua Ring",
    "Muddy Water",
    "Hydro Pump",
    "Aqua Tail",
    "Whirlpool",
    "Water Gun",
];
const ELECTRIC_NAMES: [&str; 8] = [
    "Thunderbolt",
    "Charge Beam",
    "Thunder Wave",
    "Discharge",
    "Volt Switch",
    "Spark",
    "Thunder",
    "Electro Ball",
];
const FIRE_NAMES: [&str; 8] = [
    "Flamethrower",
    "Flame Charge",
    "Fire Fang",
    "Flare Blitz",
    "Fire Spin",
    "Ember",
    "Heat Wave",
    "Will-O-Wisp",
];
const PSYCHIC_NAMES: [&str; 8] = [
    "Future Sight",
    "Psychic",
    "Calm Mind",
    "Psybeam",
    "Stored Power",
    "Reflect",
    "Light Screen",
    "Psyshock",
];
const DARK_NAMES: [&str; 8] = [
    "Foul Play",
    "Snarl",
    "Dark Pulse",
    "Payback",
    "Bite",
    "Assurance",
    "Taunt",
    "Pursuit",
];
const GRASS_NAMES: [&str; 8] = [
    "Leaf Blade",
    "Giga Drain",
    "Synthesis",
    "Razor Leaf",
    "Magical Leaf",
    "Energy Ball",
    "Seed Bomb",
    "Leaf Storm",
];
const ICE_NAMES: [&str; 8] = [
    "Ice Beam",
    "Icy Wind",
    "Blizzard",
    "Ice Shard",
    "Frost Breath",
    "Haze",
    "Aurora Veil",
    "Freeze-Dry",
];
const FAIRY_NAMES: [&str; 8] = [
    "Moonblast",
    "Dazzling Gleam",
    "Draining Kiss",
    "Misty Terrain",
    "Fairy Wind",
    "Charm",
    "Disarming Voice",
    "Baby-Doll Eyes",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT: AtomicU64 = AtomicU64::new(1);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "eevee-moves-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&root).unwrap();
            Self(root.canonicalize().unwrap())
        }

        fn run(&self) -> runner::RunRecord {
            let prepared = runner::Prepared {
                request_id: "request-1".into(),
                run_id: "run-1".into(),
                task_id: 7,
                spec: runner::CommandSpec::Direct {
                    program: "cargo".into(),
                    args: vec!["test".into(), "--workspace".into()],
                },
                cwd: self.0.clone(),
                cwd_identity: "fixture-directory".into(),
                executable: runner::ExecutableIdentity {
                    path: self.0.join("cargo"),
                    sha256: "0".repeat(64),
                    bytes: 1,
                    modified_ns: 1,
                    file_id: "fixture-executable".into(),
                },
                prepared_at_ms: 1,
                expires_at_ms: 1000,
                approval_arrival: 1,
                before: runner::InputSnapshot::default(),
                partner: Some(crate::store::Attribution {
                    partner: 1,
                    name: "Bolt".into(),
                    form: 2,
                }),
                rerun_of: None,
            };
            let mut record = runner::RunRecord::awaiting(&prepared);
            record.started_at_ms = Some(2);
            record.state = runner::RunState::Succeeded;
            record.tree = runner::TreeOutcome::OwnedTreeExited;
            record.ended_at_ms = Some(3);
            record.exit_code = Some(0);
            record
        }

        fn excerpt(&self, path: &str) -> context::Excerpt {
            context::Excerpt {
                id: 1,
                task_id: 7,
                root: self.0.to_str().unwrap().into(),
                path: path.into(),
                start_line: 2,
                end_line: 5,
                source_sha256: [3; 32],
                text: "captured bytes are not a reusable move".into(),
                available: true,
                origin_run: None,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn run(argv: &[&str], cwd: &str) -> Steps {
        Steps::Run {
            argv: argv.iter().map(|arg| (*arg).into()).collect(),
            cwd: cwd.into(),
        }
    }

    fn pin(path: &str, start_line: usize, end_line: usize) -> Pin {
        Pin {
            path: path.into(),
            start_line,
            end_line,
        }
    }

    fn proposal(form: usize, steps: Steps) -> Proposal {
        Proposal {
            partner: 1,
            form,
            name: suggested_name(form, &steps, &[]),
            scope: Scope::Project,
            steps,
            provenance: Provenance {
                task_id: 7,
                sources: vec!["run-1".into()],
                taught_by: None,
                teacher_name: None,
            },
        }
    }

    #[test]
    fn kinds_reuse_recipe_validation_and_match_only_their_forms() {
        let kinds = [
            Steps::Task {
                goal: "Bound the change".into(),
                criteria: vec!["User-visible behavior is reviewed".into()],
            },
            Steps::Pins {
                pins: vec![pin("src/main.rs", 1, 4)],
            },
            run(&["cargo", "test", "--workspace"], "."),
            Steps::Request {
                instruction: "Compare the design alternatives".into(),
                pins: vec![],
            },
        ];
        for form in 0..crate::domain::FORMS.len() {
            for (kind, steps) in kinds.iter().enumerate() {
                let expected = matches!(
                    (form, kind),
                    (0, 0) | (1, 1) | (2 | 5 | 6, 2) | (3 | 4 | 7 | 8, 3)
                );
                assert_eq!(
                    validate_steps(form, steps).is_ok(),
                    expected,
                    "form {form}, kind {kind}"
                );
            }
        }
        assert!(validate_steps(usize::MAX, &kinds[0]).is_err());
        let invalid = Steps::Task {
            goal: "Task".into(),
            criteria: vec![],
        };
        assert!(validate_steps(0, &invalid).is_err());
        let invalid = Steps::Request {
            instruction: "Unsafe\u{1b}text".into(),
            pins: vec![],
        };
        assert!(validate_steps(4, &invalid).is_err());
    }

    #[test]
    fn storage_shapes_refuse_capabilities_and_oversized_steps() {
        let proposal = proposal(2, run(&["cargo", "test", ""], "."));
        let mut value = serde_json::to_value(&proposal).unwrap();
        value["grant"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Proposal>(value).is_err());
        let mut value = serde_json::to_value(&proposal).unwrap();
        value["steps"]["Run"]["output"] = serde_json::json!("provider output");
        assert!(serde_json::from_value::<Proposal>(value).is_err());
        let mut value = serde_json::to_value(&proposal).unwrap();
        value["provenance"]["memories"] = serde_json::json!([]);
        assert!(serde_json::from_value::<Proposal>(value).is_err());
        assert!(serde_json::from_str::<Steps>(r#"{"Pins":{"pins":[{"path":"src/main.rs","start_line":1,"end_line":2,"text":"captured"}]}}"#).is_err());
        let oversized = Steps::Task {
            goal: "User goal".into(),
            criteria: vec!["a".repeat(workflows::MAX_TEXT_BYTES); workflows::MAX_ENTRIES],
        };
        assert!(validate_steps(0, &oversized).is_err());
        let record = Record {
            id: 1,
            revision: 1,
            project: "project".into(),
            proposal,
            forgotten: false,
            created: 1,
        };
        let mut value = serde_json::to_value(record).unwrap();
        value["approval"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Record>(value).is_err());
    }

    #[test]
    fn approved_run_requires_actual_start_and_valid_task_partner_correlation() {
        let fixture = Fixture::new();
        let mut record = fixture.run();
        assert_eq!(
            run_steps(&fixture.0, &record).unwrap(),
            run(&["cargo", "test", "--workspace"], ".")
        );
        record.started_at_ms = None;
        assert!(
            run_steps(&fixture.0, &record)
                .unwrap_err()
                .contains("actually started")
        );
        record = fixture.run();
        record.tree = runner::TreeOutcome::NotStarted;
        assert!(run_steps(&fixture.0, &record).is_err());
        record = fixture.run();
        record.state = runner::RunState::AwaitingDispatch;
        assert!(run_steps(&fixture.0, &record).is_err());
        record = fixture.run();
        record.prepared.partner = None;
        assert!(
            run_steps(&fixture.0, &record)
                .unwrap_err()
                .contains("attribution")
        );
        record = fixture.run();
        record.prepared.partner.as_mut().unwrap().partner = 0;
        assert!(run_steps(&fixture.0, &record).is_err());
        record = fixture.run();
        record.prepared.task_id = 0;
        assert!(run_steps(&fixture.0, &record).is_err());
        record = fixture.run();
        record.prepared.run_id = "../run".into();
        assert!(run_steps(&fixture.0, &record).is_err());
        // Approval is not a correctness verdict; started failed/unknown runs
        // remain teachable and their usage outcome is a separate journal fact.
        record = fixture.run();
        record.state = runner::RunState::OutcomeUnknown;
        record.tree = runner::TreeOutcome::OwnershipUnknown;
        record.exit_code = None;
        assert!(run_steps(&fixture.0, &record).is_ok());
    }

    #[test]
    fn run_paths_preserve_exact_in_project_argv_and_relative_cwd() {
        let fixture = Fixture::new();
        let relative_cwd = Path::new("tools").join("nested");
        fs::create_dir_all(fixture.0.join(&relative_cwd)).unwrap();
        let executable = fixture
            .0
            .join("tools")
            .join("check")
            .to_str()
            .unwrap()
            .to_string();
        let source = fixture
            .0
            .join("source file.rs")
            .to_str()
            .unwrap()
            .to_string();
        let argv = vec![
            executable.clone(),
            "--input".into(),
            source.clone(),
            Path::new("data")
                .join("source file.rs")
                .to_str()
                .unwrap()
                .into(),
            String::new(),
        ];
        let mut record = fixture.run();
        record.prepared.spec = runner::CommandSpec::Direct {
            program: executable,
            args: argv[1..].to_vec(),
        };
        record.prepared.cwd = fixture.0.join(&relative_cwd);
        let steps = run_steps(&fixture.0, &record).unwrap();
        assert_eq!(
            steps,
            Steps::Run {
                argv,
                cwd: "tools/nested".into()
            }
        );
        assert!(validate_for_root(&fixture.0, &proposal(2, steps)).is_ok());
        record.prepared.cwd = relative_cwd;
        assert!(run_steps(&fixture.0, &record).is_ok());
        record.prepared.cwd = "../elsewhere".into();
        assert!(run_steps(&fixture.0, &record).is_err());
        record.prepared.cwd = fixture.0.parent().unwrap().to_path_buf();
        assert!(run_steps(&fixture.0, &record).is_err());
        let outside = Fixture::new();
        let outside_input = outside.0.join("input").to_str().unwrap().to_string();
        let outside_equal = outside.0.join("name=value").to_str().unwrap().to_string();
        for argument in [
            outside_input.clone(),
            format!("--input={outside_input}"),
            format!("-I{outside_input}"),
            format!("-C{outside_input}"),
            format!("@{outside_input}"),
            outside_equal,
        ] {
            let steps = run(&["cargo", &argument], ".");
            // Syntax validation cannot authorize a project root.
            assert!(validate_steps(2, &steps).is_ok(), "{argument}");
            assert!(
                validate_steps_at(&fixture.0, 2, &steps).is_err(),
                "{argument}"
            );
        }
        assert!(validate_steps_at(&fixture.0, 2, &run(&["cargo", "test"], "missing")).is_err());
    }

    #[test]
    fn cwd_metadata_uses_portable_components_and_respects_project_roots() {
        let fixture = Fixture::new();
        let relative = Path::new("tools").join("missing-directory");
        assert_eq!(relative_cwd(&fixture.0, &fixture.0).unwrap(), ".");
        assert_eq!(relative_cwd(&fixture.0, Path::new(".")).unwrap(), ".");
        assert_eq!(
            relative_cwd(&fixture.0, &relative).unwrap(),
            "tools/missing-directory"
        );
        assert_eq!(
            relative_cwd(&fixture.0, &fixture.0.join(&relative)).unwrap(),
            "tools/missing-directory"
        );
        let sibling = fixture.0.parent().unwrap().join(format!(
            "{}-other",
            fixture.0.file_name().unwrap().to_str().unwrap()
        ));
        assert!(relative_cwd(&fixture.0, &sibling).is_err());
        assert!(relative_cwd(&fixture.0, &relative.join("..")).is_err());
    }

    #[test]
    fn unsafe_run_consumers_receive_reasons_without_secret_echoes() {
        let forbidden: &[&[&str]] = &[
            &["sh", "-c", "cargo test"],
            &["bash", "script"],
            &["powershell.exe", "-Command", "Get-Item ."],
            &["busybox", "sh", "-c", "true"],
            &["env", "cargo", "test"],
            &["MODE=ci", "cargo"],
            &["cargo", "MODE=ci"],
            &["cargo", "_MODE=ci"],
            &["cargo", "--token", "private-value"],
            &["cargo", "--password=private-value"],
            &["cargo", "ghp_privatevalue"],
            &["cargo", "[ReDaCtEd]"],
            &["cargo", "--input=[MaSkEd]"],
            &["cargo", "../outside"],
            &["cargo", "--input=src/../../outside"],
            &["cargo", "~/outside"],
            &["cargo", "src/private.key"],
            &["cargo", ".env"],
            &["cargo", "credentials.json"],
        ];
        for argv in forbidden {
            let reason = validate_steps(2, &run(argv, ".")).unwrap_err();
            assert!(!reason.is_empty());
            assert!(!reason.contains("private-value"));
            assert!(!reason.contains("ghp_privatevalue"));
        }
        assert!(
            validate_steps(
                2,
                &run(
                    &["cargo", "test", "--features=one,two", "", "literal && text"],
                    "."
                )
            )
            .is_ok()
        );
        let fixture = Fixture::new();
        // A drive-qualified path is native on Windows and ambiguous elsewhere;
        // either way only root-aware validation decides whether it is reusable.
        assert!(validate_steps_at(&fixture.0, 2, &run(&["cargo", "C:\\outside"], ".")).is_err());
        let mut record = fixture.run();
        record.prepared.spec = runner::CommandSpec::Shell {
            executable: "sh".into(),
            dialect: runner::ShellDialect::Posix,
            text: "cargo test".into(),
        };
        assert!(
            run_steps(&fixture.0, &record)
                .unwrap_err()
                .contains("Shell")
        );
        record = fixture.run();
        record.prepared.spec = runner::CommandSpec::Direct {
            program: "cargo".into(),
            args: vec!["--token".into(), "private-value".into()],
        };
        record.redact_for_storage();
        assert!(run_steps(&fixture.0, &record).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn cwd_and_pin_links_are_not_reusable_path_capabilities() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let outside = Fixture::new();
        symlink(&outside.0, fixture.0.join("linked")).unwrap();
        let mut record = fixture.run();
        record.prepared.cwd = fixture.0.join("linked");
        assert!(
            run_steps(&fixture.0, &record)
                .unwrap_err()
                .contains("links")
        );
        let steps = Steps::Pins {
            pins: vec![pin("linked/source.rs", 1, 2)],
        };
        assert!(
            validate_steps_at(&fixture.0, 1, &steps)
                .unwrap_err()
                .contains("links")
        );
        let steps = run(&["cargo", "--input=linked/source.rs"], ".");
        assert!(validate_steps_at(&fixture.0, 2, &steps).is_err());
        let missing = Steps::Pins {
            pins: vec![pin("missing/source.rs", 1, 2)],
        };
        assert!(validate_steps_at(&fixture.0, 1, &missing).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn bare_run_arguments_check_links_at_selected_cwd_without_changing_path_lookup() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let outside = Fixture::new();
        fs::write(outside.0.join("input"), "outside bytes must not be read").unwrap();
        let relative_cwd = Path::new("tools").join("nested");
        let cwd = fixture.0.join(&relative_cwd);
        fs::create_dir_all(&cwd).unwrap();
        for name in ["input", "cat", "-x"] {
            symlink(outside.0.join("input"), cwd.join(name)).unwrap();
        }
        // A root-local file must not hide the symlink in the selected cwd.
        fs::write(fixture.0.join("input"), "root-local bytes").unwrap();
        for argument in ["input", "--input=input", "-Iinput", "@input"] {
            let steps = run(&["cat", argument], "tools/nested");
            assert!(validate_steps(2, &steps).is_ok());
            assert!(
                validate_steps_at(&fixture.0, 2, &steps)
                    .unwrap_err()
                    .contains("links"),
                "{argument}"
            );
            let mut record = fixture.run();
            record.prepared.cwd = cwd.clone();
            record.prepared.spec = runner::CommandSpec::Direct {
                program: "cat".into(),
                args: vec![argument.into()],
            };
            assert!(
                run_steps(&fixture.0, &record)
                    .unwrap_err()
                    .contains("links")
            );
        }
        // Bare argv[0] still uses PATH; flags and missing values remain literals.
        let steps = run(
            &["cat", "-x", "missing", "--input=missing", ""],
            "tools/nested",
        );
        assert!(validate_steps_at(&fixture.0, 2, &steps).is_ok());
        assert!(validate_steps_at(&fixture.0, 2, &run(&["./cat"], "tools/nested")).is_err());
    }

    #[test]
    fn fingerprints_use_pin_sets_but_exact_ordered_argv() {
        let a = pin("src/a.rs", 1, 3);
        let b = pin("src/b.rs", 4, 8);
        let left = Steps::Pins {
            pins: vec![a.clone(), b.clone()],
        };
        let right = Steps::Pins {
            pins: vec![b.clone(), a.clone(), a.clone()],
        };
        assert_eq!(fingerprint(&left).unwrap(), fingerprint(&right).unwrap());
        let different = Steps::Pins {
            pins: vec![pin("src/a.rs", 1, 4), b.clone()],
        };
        assert_ne!(
            fingerprint(&left).unwrap(),
            fingerprint(&different).unwrap()
        );
        let request = Steps::Request {
            instruction: "Review the change".into(),
            pins: vec![a.clone(), b.clone()],
        };
        let reordered = Steps::Request {
            instruction: "Review the change".into(),
            pins: vec![b, a],
        };
        assert_eq!(
            fingerprint(&request).unwrap(),
            fingerprint(&reordered).unwrap()
        );
        assert_ne!(fingerprint(&left).unwrap(), fingerprint(&request).unwrap());
        let exact = run(&["cargo", "test", "--workspace"], ".");
        assert_eq!(
            fingerprint(&exact).unwrap(),
            fingerprint(&exact.clone()).unwrap()
        );
        assert_ne!(
            fingerprint(&exact).unwrap(),
            fingerprint(&run(&["cargo", "test", "--all-targets"], ".")).unwrap()
        );
        assert_ne!(
            fingerprint(&exact).unwrap(),
            fingerprint(&run(&["cargo", "--workspace", "test"], ".")).unwrap()
        );
        assert_ne!(
            fingerprint(&exact).unwrap(),
            fingerprint(&run(&["cargo", "test", "--workspace", ""], ".")).unwrap()
        );
        assert_ne!(
            fingerprint(&exact).unwrap(),
            fingerprint(&run(&["cargo", "test", "--workspace"], "subdir")).unwrap()
        );
        assert_eq!(fingerprint(&exact).unwrap().len(), 64);
    }

    #[test]
    fn historical_journal_validation_does_not_require_existing_project_paths() {
        let fixture = Fixture::new();
        let mut record = fixture.run();
        record.prepared.cwd = fixture.0.join("removed-directory");
        let expected = run(&["cargo", "test", "--workspace"], "removed-directory");
        assert_eq!(journal_steps(&fixture.0, &record).unwrap(), expected);
        assert!(run_steps(&fixture.0, &record).is_err());
        let missing_project = fixture.0.join("removed-project");
        record.prepared.cwd = missing_project.clone();
        assert!(journal_steps(&missing_project, &record).is_ok());
        assert!(run_steps(&missing_project, &record).is_err());
        record.prepared.spec = runner::CommandSpec::Direct {
            program: "cargo".into(),
            args: vec![format!(
                "--input={}",
                fixture.0.join("outside-project").display()
            )],
        };
        assert!(journal_steps(&missing_project, &record).is_err());
    }

    #[test]
    fn pin_extraction_stores_only_normalized_selectors_and_checks_owners() {
        let fixture = Fixture::new();
        let a_path = Path::new("src").join("a.rs");
        let b_path = Path::new("src").join("b.rs");
        let a = fixture.excerpt(a_path.to_str().unwrap());
        let b = fixture.excerpt(b_path.to_str().unwrap());
        let selector = Pin::from_excerpt(&a).unwrap();
        assert_eq!(selector, pin("src/a.rs", 2, 5));
        assert!(selector.matches_excerpt(&a));
        let mut portable = a.clone();
        portable.path.clone_from(&selector.path);
        assert!(selector.matches_excerpt(&portable));
        let mut changed = a.clone();
        changed.end_line += 1;
        assert!(!selector.matches_excerpt(&changed));
        assert!(!selector.matches_excerpt(&b));
        let steps = pin_steps(&[b.clone(), a.clone(), a.clone()]).unwrap();
        assert_eq!(
            steps,
            Steps::Pins {
                pins: vec![pin("src/a.rs", 2, 5), pin("src/b.rs", 2, 5)]
            }
        );
        let encoded = serde_json::to_string(&steps).unwrap();
        assert!(!encoded.contains("captured bytes"));
        assert!(!encoded.contains("source_sha256"));
        assert!(!encoded.contains(fixture.0.to_str().unwrap()));
        let mut bad = a.clone();
        bad.task_id += 1;
        assert!(pin_steps(&[a.clone(), bad]).is_err());
        let mut bad = a.clone();
        bad.available = false;
        bad.text.clear();
        assert!(pin_steps(&[bad]).is_err());
        let mut bad = a.clone();
        bad.path = ".env".into();
        assert!(pin_steps(&[bad]).is_err());
        for path in [
            "../outside",
            "/outside",
            "src/*.rs",
            "src//a.rs",
            "src/./a.rs",
            "src/key.pem",
        ] {
            assert!(
                validate_steps(
                    1,
                    &Steps::Pins {
                        pins: vec![pin(path, 1, 2)]
                    }
                )
                .is_err(),
                "{path}"
            );
        }
        assert!(
            validate_steps(
                1,
                &Steps::Pins {
                    pins: vec![pin("src/a.rs", 0, 2)]
                }
            )
            .is_err()
        );
        assert!(pin_steps(&[]).is_err());
    }

    #[test]
    fn unicode_names_and_fixed_local_suggestions_are_distinct_without_changing_steps() {
        for name in ["雷光一閃", "검토", "Éclair", "波乗り 🫧", &"雷".repeat(64)] {
            assert!(validate_name(name).is_ok(), "{name}");
        }
        for name in [
            "",
            " ",
            " name",
            "name ",
            "bad\nname",
            "hidden\u{202e}name",
            "[REDACTED]",
            &"雷".repeat(65),
        ] {
            assert!(validate_name(name).is_err(), "{name}");
        }
        let steps = run(&["cargo", "test"], ".");
        let fingerprint_before = fingerprint(&steps).unwrap();
        let mut used = Vec::new();
        for _ in 0..3 {
            let name = suggested_name(2, &steps, &used);
            assert!(!used.contains(&name));
            used.push(name);
        }
        assert_eq!(used, ["Thunderbolt", "Charge Beam", "Thunder Wave"]);
        assert_eq!(
            suggested_name(2, &steps, &["thunderbolt".into(), "Thunderbolt".into()]),
            "Charge Beam"
        );
        assert_eq!(
            suggested_name(2, &run(&["cargo", "build"], "."), &[]),
            "Charge Beam"
        );
        let mut exhausted: Vec<String> = ELECTRIC_NAMES.iter().map(|name| (*name).into()).collect();
        exhausted.push("Thunderbolt 2".into());
        assert_eq!(suggested_name(2, &steps, &exhausted), "Thunderbolt 3");
        let expected = [
            "Helping Hand",
            "Surf",
            "Thunderbolt",
            "Flamethrower",
            "Future Sight",
            "Foul Play",
            "Leaf Blade",
            "Ice Beam",
            "Moonblast",
        ];
        for (form, name) in expected.iter().enumerate() {
            assert_eq!(suggested_name(form, &steps, &[]), *name);
        }
        let mut proposal = proposal(2, steps);
        proposal.name = "雷光一閃".into();
        assert!(validate_proposal(&proposal).is_ok());
        assert_eq!(fingerprint(&proposal.steps).unwrap(), fingerprint_before);
    }

    #[test]
    fn provenance_is_bounded_attributed_and_not_a_capability() {
        let mut proposal = proposal(2, run(&["cargo", "test"], "."));
        proposal.provenance.taught_by = Some(2);
        assert!(validate_proposal(&proposal).is_err());
        proposal.provenance.teacher_name = Some("Mizu".into());
        assert!(validate_proposal(&proposal).is_ok());
        let long_label = format!("e{}", "\u{301}".repeat(140));
        assert!(long_label.len() > MAX_NAME_BYTES);
        crate::store::validate_nickname(&long_label).unwrap();
        proposal.provenance.teacher_name = Some(long_label);
        assert!(validate_proposal(&proposal).is_ok());
        proposal.provenance.taught_by = Some(proposal.partner);
        assert!(validate_proposal(&proposal).is_err());
        proposal.provenance.taught_by = Some(2);
        proposal.provenance.sources.push("run-1".into());
        assert!(validate_proposal(&proposal).is_err());
        proposal.provenance.sources = vec!["run-1".into(); MAX_SOURCES + 1];
        assert!(validate_proposal(&proposal).is_err());
    }
}
