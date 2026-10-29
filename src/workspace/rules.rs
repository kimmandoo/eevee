//! Persistent, scoped approval rules (XC01). A rule exists only because the
//! user created it from a reviewed request; it matches a narrow, displayed
//! boundary and never covers deletes, renames, shells, selected commits or
//! anything outside the project. Model output can neither create nor alter one.
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

pub const FILE_NAME: &str = "rules.json";
const FILE_LIMIT: u64 = 256 * 1024;
pub const MAX_RULES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Allow,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expiry {
    Never,
    /// Not persisted: gone when this `ev` exits.
    ThisLaunch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteOp {
    Create,
    Edit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Kind {
    /// Direct argv only. The executable is pinned by resolved path and SHA-256.
    Command {
        program: PathBuf,
        program_sha256: String,
        argv_prefix: Vec<String>,
        /// Project-relative directory prefix; empty is the project root and below.
        cwd: String,
    },
    Write {
        ops: Vec<WriteOp>,
        /// Project-relative globs: `*` within one segment, `**` across segments.
        paths: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: u32,
    pub project: String,
    pub effect: Effect,
    pub kind: Kind,
    pub created_at: u64,
    pub expires: Expiry,
    #[serde(default = "enabled")]
    pub enabled: bool,
}

fn enabled() -> bool {
    true
}

impl Rule {
    /// The plain sentence shown before saving and in `/rules`.
    pub fn sentence(&self) -> String {
        let verb = match self.effect {
            Effect::Allow => "Allow",
            Effect::Deny => "Always deny",
        };
        let until = match self.expires {
            Expiry::Never => "until revoked",
            Expiry::ThisLaunch => "until ev exits",
        };
        match &self.kind {
            Kind::Command {
                program,
                argv_prefix,
                cwd,
                ..
            } => {
                let name = program.file_name().map_or_else(
                    || program.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                let prefix = std::iter::once(name.as_str())
                    .chain(argv_prefix.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join(" ");
                let code = if runs_project_code(&name, argv_prefix) {
                    " (runs project code; not a sandbox)"
                } else {
                    ""
                };
                format!(
                    "{verb} `{prefix} …` with any further arguments in `./{cwd}` for this project {until}{code}; executable {}",
                    program.display()
                )
            }
            Kind::Write { ops, paths } => {
                let ops = ops
                    .iter()
                    .map(|op| match op {
                        WriteOp::Create => "create",
                        WriteOp::Edit => "edit",
                    })
                    .collect::<Vec<_>>()
                    .join("/");
                format!(
                    "{verb} file {ops} of `{}` in this project {until}; never delete or rename",
                    paths.join("`, `")
                )
            }
        }
    }
}

/// Build and test programs execute repository code; the sentence must say so.
fn runs_project_code(program: &str, argv: &[String]) -> bool {
    let program = program.trim_end_matches(".exe");
    matches!(
        program,
        "cargo"
            | "npm"
            | "pnpm"
            | "yarn"
            | "make"
            | "python"
            | "python3"
            | "node"
            | "go"
            | "gradle"
            | "mvn"
            | "pytest"
            | "dotnet"
            | "bun"
            | "deno"
    ) || argv
        .first()
        .is_some_and(|first| matches!(first.as_str(), "test" | "run" | "build"))
}

/// A pending request, reduced to what rules may match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call<'a> {
    Command {
        program: &'a Path,
        program_sha256: &'a str,
        args: &'a [String],
        /// Project-relative cwd, already contained in the project.
        cwd: &'a str,
    },
    Write {
        op: WriteOp,
        path: &'a str,
    },
}

fn relative_ok(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn segment_match(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((head, tail)) => {
            text.starts_with(head)
                && (0..=text.len() - head.len()).any(|skip| {
                    text.is_char_boundary(head.len() + skip)
                        && segment_match(tail, &text[head.len() + skip..])
                })
        }
    }
}

pub fn glob(pattern: &str, path: &str) -> bool {
    fn parts(pattern: &[&str], path: &[&str]) -> bool {
        match pattern.split_first() {
            None => path.is_empty(),
            Some((&"**", rest)) => (0..=path.len()).any(|skip| parts(rest, &path[skip..])),
            Some((head, rest)) => path
                .split_first()
                .is_some_and(|(first, tail)| segment_match(head, first) && parts(rest, tail)),
        }
    }
    let pattern = pattern.split('/').collect::<Vec<_>>();
    let path = path.split('/').collect::<Vec<_>>();
    parts(&pattern, &path)
}

fn cwd_within(rule: &str, cwd: &str) -> bool {
    rule.is_empty()
        || cwd == rule
        || cwd
            .strip_prefix(rule)
            .is_some_and(|rest| rest.starts_with('/'))
}

impl Rule {
    pub fn matches(&self, project: &str, call: &Call<'_>) -> bool {
        if !self.enabled || self.project != project {
            return false;
        }
        match (&self.kind, call) {
            (
                Kind::Command {
                    program,
                    program_sha256,
                    argv_prefix,
                    cwd,
                },
                Call::Command {
                    program: called,
                    program_sha256: called_sha,
                    args,
                    cwd: called_cwd,
                },
            ) => {
                program == called
                    && program_sha256 == called_sha
                    && args.starts_with(argv_prefix)
                    && cwd_within(cwd, called_cwd)
            }
            (Kind::Write { ops, paths }, Call::Write { op, path }) => {
                relative_ok(path) && ops.contains(op) && paths.iter().any(|glob_| glob(glob_, path))
            }
            _ => false,
        }
    }
}

/// Deny wins; then the most specific Allow; otherwise ordinary review.
pub fn decide<'r>(rules: &'r [Rule], project: &str, call: &Call<'_>) -> Option<&'r Rule> {
    let matching = rules.iter().filter(|rule| rule.matches(project, call));
    if let Some(deny) = matching.clone().find(|rule| rule.effect == Effect::Deny) {
        return Some(deny);
    }
    matching.max_by_key(|rule| match &rule.kind {
        Kind::Command {
            argv_prefix, cwd, ..
        } => argv_prefix.len() * 4 + cwd.len(),
        Kind::Write { paths, .. } => paths.iter().map(String::len).max().unwrap_or(0),
    })
}

#[derive(Debug, Default)]
pub struct Store {
    pub path: PathBuf,
    pub rules: Vec<Rule>,
    pub error: Option<String>,
}

impl Store {
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(FILE_NAME);
        let mut store = Self {
            path: path.clone(),
            ..Self::default()
        };
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return store,
            Ok(meta) if meta.is_file() && meta.len() <= FILE_LIMIT => {}
            _ => {
                store.error = Some(format!(
                    "{} must be a regular file of at most 256 KiB; no rules loaded",
                    path.display()
                ));
                return store;
            }
        }
        match std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| {
                serde_json::from_str::<Vec<Rule>>(&text).map_err(|error| error.to_string())
            })
            .and_then(|rules| validate(&rules).map(|()| rules))
        {
            Ok(rules) => store.rules = rules,
            Err(error) => {
                store.error = Some(format!("{}: {error}; no rules loaded", path.display()))
            }
        }
        store
    }

    /// Persists every `Never` rule; `ThisLaunch` rules stay in memory only.
    pub fn save(&self) -> Result<(), String> {
        let lasting = self
            .rules
            .iter()
            .filter(|rule| rule.expires == Expiry::Never)
            .collect::<Vec<_>>();
        let text = serde_json::to_string_pretty(&lasting).map_err(|error| error.to_string())?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, text).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&temporary, &self.path).map_err(|error| error.to_string())
    }

    pub fn add(&mut self, mut rule: Rule) -> Result<u32, String> {
        if self.rules.len() >= MAX_RULES {
            return Err("At most 128 rules; revoke one in /rules first".into());
        }
        if let Some(existing) = self.rules.iter().find(|known| {
            known.project == rule.project && known.effect == rule.effect && known.kind == rule.kind
        }) {
            return Ok(existing.id);
        }
        rule.id = self.rules.iter().map(|known| known.id).max().unwrap_or(0) + 1;
        validate(std::slice::from_ref(&rule))?;
        let id = rule.id;
        self.rules.push(rule);
        Ok(id)
    }

    pub fn revoke(&mut self, id: u32) -> bool {
        let before = self.rules.len();
        self.rules.retain(|rule| rule.id != id);
        before != self.rules.len()
    }
}

fn validate(rules: &[Rule]) -> Result<(), String> {
    if rules.len() > MAX_RULES {
        return Err("too many rules".into());
    }
    for rule in rules {
        match &rule.kind {
            Kind::Command {
                program,
                program_sha256,
                argv_prefix,
                cwd,
            } => {
                if !program.is_absolute()
                    || program_sha256.len() != 64
                    || !program_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                    || argv_prefix.len() > 64
                    || argv_prefix.iter().any(|arg| arg.len() > 4096)
                    || (!cwd.is_empty() && !relative_ok(cwd))
                {
                    return Err(format!("invalid command rule #{}", rule.id));
                }
            }
            Kind::Write { ops, paths } => {
                if ops.is_empty()
                    || paths.is_empty()
                    || paths.len() > 16
                    || paths.iter().any(|path| {
                        path.is_empty()
                            || path.len() > 512
                            || path.starts_with('/')
                            || path
                                .split('/')
                                .any(|part| part == ".." || part == "." || part.is_empty())
                    })
                {
                    return Err(format!("invalid write rule #{}", rule.id));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn command(prefix: &[&str], cwd: &str, effect: Effect) -> Rule {
        Rule {
            id: 1,
            project: "/p".into(),
            effect,
            kind: Kind::Command {
                program: "/usr/bin/cargo".into(),
                program_sha256: SHA.into(),
                argv_prefix: prefix.iter().map(|arg| arg.to_string()).collect(),
                cwd: cwd.into(),
            },
            created_at: 0,
            expires: Expiry::Never,
            enabled: true,
        }
    }

    fn call<'a>(args: &'a [String], cwd: &'a str, sha: &'a str) -> Call<'a> {
        Call::Command {
            program: Path::new("/usr/bin/cargo"),
            program_sha256: sha,
            args,
            cwd,
        }
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn command_prefixes_are_token_exact_and_pinned_to_the_executable() {
        let rule = command(&["test"], "", Effect::Allow);
        assert!(rule.matches("/p", &call(&args(&["test"]), "", SHA)));
        assert!(rule.matches("/p", &call(&args(&["test", "-p", "x"]), "crates/a", SHA)));
        assert!(!rule.matches("/p", &call(&args(&["testx"]), "", SHA)));
        assert!(!rule.matches("/p", &call(&args(&["build"]), "", SHA)));
        assert!(
            !rule.matches("/q", &call(&args(&["test"]), "", SHA)),
            "other project"
        );
        let other = "1".repeat(64);
        assert!(
            !rule.matches("/p", &call(&args(&["test"]), "", &other)),
            "rebuilt executable"
        );
        let scoped = command(&["test"], "crates", Effect::Allow);
        assert!(scoped.matches("/p", &call(&args(&["test"]), "crates/a", SHA)));
        assert!(!scoped.matches("/p", &call(&args(&["test"]), "cratesx", SHA)));
        assert!(!scoped.matches("/p", &call(&args(&["test"]), "", SHA)));
        assert!(rule.sentence().contains("runs project code"));
    }

    #[test]
    fn deny_wins_and_the_most_specific_allow_is_reported() {
        let broad = command(&[], "", Effect::Allow);
        let mut narrow = command(&["test"], "", Effect::Allow);
        narrow.id = 2;
        let mut deny = command(&["test", "--release"], "", Effect::Deny);
        deny.id = 3;
        let rules = [broad, narrow, deny];
        let test = args(&["test"]);
        assert_eq!(decide(&rules, "/p", &call(&test, "", SHA)).unwrap().id, 2);
        let release = args(&["test", "--release"]);
        assert_eq!(
            decide(&rules, "/p", &call(&release, "", SHA))
                .unwrap()
                .effect,
            Effect::Deny
        );
        let mut disabled = rules.clone();
        disabled.iter_mut().for_each(|rule| rule.enabled = false);
        assert!(decide(&disabled, "/p", &call(&test, "", SHA)).is_none());
    }

    #[test]
    fn write_rules_cover_only_create_and_edit_inside_matching_globs() {
        let rule = Rule {
            id: 1,
            project: "/p".into(),
            effect: Effect::Allow,
            kind: Kind::Write {
                ops: vec![WriteOp::Edit],
                paths: vec!["tests/**".into(), "src/*.rs".into()],
            },
            created_at: 0,
            expires: Expiry::Never,
            enabled: true,
        };
        let write = |op, path| rule.matches("/p", &Call::Write { op, path });
        assert!(write(WriteOp::Edit, "tests/a/b.rs"));
        assert!(write(WriteOp::Edit, "src/main.rs"));
        assert!(!write(WriteOp::Edit, "src/app/main.rs"));
        assert!(!write(WriteOp::Create, "tests/new.rs"));
        assert!(!write(WriteOp::Edit, "tests/../secret"));
        assert!(!write(WriteOp::Edit, "/etc/passwd"));
        assert!(glob("**", "a/b/c") && glob("a*b", "axxb") && !glob("a*b", "axxc"));
        assert!(rule.sentence().contains("never delete or rename"));
    }

    #[test]
    fn launch_rules_are_not_persisted_and_invalid_files_load_nothing() {
        let dir = std::env::temp_dir().join(format!(
            "ev-rules-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = Store::load(&dir);
        assert!(store.rules.is_empty() && store.error.is_none());
        let lasting = store.add(command(&["test"], "", Effect::Allow)).unwrap();
        let mut launch = command(&["build"], "", Effect::Allow);
        launch.expires = Expiry::ThisLaunch;
        store.add(launch).unwrap();
        assert_eq!(
            store.add(command(&["test"], "", Effect::Allow)).unwrap(),
            lasting,
            "deduplicated"
        );
        store.save().unwrap();
        let loaded = Store::load(&dir);
        assert_eq!(loaded.rules.len(), 1);
        assert_eq!(loaded.rules[0].id, lasting);
        std::fs::write(dir.join(FILE_NAME), "[{\"id\":1}]").unwrap();
        let broken = Store::load(&dir);
        assert!(broken.rules.is_empty() && broken.error.is_some());
        let mut bad = command(&["x"], "../up", Effect::Allow);
        bad.id = 9;
        assert!(validate(&[bad]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
