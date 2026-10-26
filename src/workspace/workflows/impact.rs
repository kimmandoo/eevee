//! Offline syntax-derived check recommendations, never coverage or permission.
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use syn::{parse::Parser, visit::Visit};

use crate::workspace::{context, privacy, records, repo, runner};

pub const MAX_FILES: usize = 256;
pub const MAX_FILE_BYTES: usize = 128 * 1024;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SYMBOLS: usize = 8192;
pub const MAX_CHECKS: usize = 16;

#[derive(Clone, Debug)]
pub struct Check {
    pub command: runner::CommandSpec,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub request: u64,
    pub task_id: i64,
    pub partner: i64,
    pub project: PathBuf,
    pub before: runner::InputSnapshot,
    pub checks: Vec<Check>,
    pub changed_paths: Vec<String>,
    pub affected_symbols: Vec<String>,
    pub omissions: Vec<String>,
    pub scanned_files: usize,
    pub scanned_bytes: usize,
    pub broader: bool,
}

impl Plan {
    pub fn review(&self) -> String {
        let mut text = "Impact-directed check proposal — no execution, verification or permission\nPassing these checks is TARGETED evidence, not complete verification.\n".to_owned();
        text.push_str(&format!(
            "{} / files={} bytes={}\n{}\n",
            if self.broader {
                "Broader recommendation"
            } else {
                "Targeted recommendation"
            },
            self.scanned_files,
            self.scanned_bytes,
            self.changed_paths.join(", ")
        ));
        for (index, check) in self.checks.iter().enumerate() {
            text.push_str(&format!(
                "\n{}. {}\n{}\n",
                index + 1,
                serde_json::to_string(&check.command).expect("command serialization"),
                check.reasons.join("\n")
            ));
        }
        if self.checks.is_empty() {
            text.push_str("\nNo trustworthy executable check mapping. Select an explicit command through Jolteon.\n");
        }
        text.push_str("\nSyntax-name candidates only: no semantic binding, dynamic dependency, macro expansion or completeness claim. Changed files conservatively affect every definition they contain.\n");
        for omission in &self.omissions {
            text.push_str(&format!("Omission: {omission}\n"));
        }
        text
    }
}

#[derive(Clone, Debug)]
struct Function {
    path: String,
    name: String,
    qualified: String,
    test: bool,
    ignored: bool,
    calls: BTreeSet<String>,
}

struct Functions<'a> {
    path: &'a str,
    modules: Vec<String>,
    values: Vec<Function>,
    overflow: bool,
    definitions: BTreeSet<String>,
}
impl Functions<'_> {
    fn add(&mut self, signature: &syn::Signature, attrs: &[syn::Attribute], body: &syn::Block) {
        if self.values.len() >= MAX_SYMBOLS {
            self.overflow = true;
            return;
        }
        let mut calls = Calls::default();
        calls.visit_signature(signature);
        calls.visit_block(body);
        let mut names = self.modules.clone();
        names.push(signature.ident.to_string());
        self.definitions.insert(signature.ident.to_string());
        self.values.push(Function {
            path: self.path.into(),
            name: signature.ident.to_string(),
            qualified: names.join("::"),
            test: attrs.iter().any(|attr| {
                attr.path()
                    .segments
                    .last()
                    .is_some_and(|part| part.ident == "test")
            }),
            ignored: attrs.iter().any(|attr| attr.path().is_ident("ignore")),
            calls: calls.0,
        });
    }
}
impl<'ast> Visit<'ast> for Functions<'_> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if self.definitions.len() >= MAX_SYMBOLS {
            self.overflow = true;
            return;
        }
        let name = match item {
            syn::Item::Const(value) => Some(&value.ident),
            syn::Item::Static(value) => Some(&value.ident),
            syn::Item::Struct(value) => Some(&value.ident),
            syn::Item::Enum(value) => Some(&value.ident),
            syn::Item::Trait(value) => Some(&value.ident),
            syn::Item::Type(value) => Some(&value.ident),
            _ => None,
        };
        if let Some(name) = name {
            self.definitions.insert(name.to_string());
        }
        syn::visit::visit_item(self, item);
    }
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        self.modules.push(module.ident.to_string());
        syn::visit::visit_item_mod(self, module);
        self.modules.pop();
    }
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        self.add(&function.sig, &function.attrs, &function.block);
    }
    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        self.add(&function.sig, &function.attrs, &function.block);
    }
    fn visit_trait_item_fn(&mut self, function: &'ast syn::TraitItemFn) {
        if let Some(body) = &function.default {
            self.add(&function.sig, &function.attrs, body);
        }
    }
}
#[derive(Default)]
struct Calls(BTreeSet<String>);
impl<'ast> Visit<'ast> for Calls {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if let Some(segment) = path.segments.last() {
            self.0.insert(segment.ident.to_string());
        }
        syn::visit::visit_path(self, path);
    }
    fn visit_macro(&mut self, expression: &'ast syn::Macro) {
        // Parse expression-list syntax, never execute or expand macros.
        if let Ok(expressions) =
            syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
                .parse2(expression.tokens.clone())
        {
            for expression in expressions {
                syn::visit::visit_expr(self, &expression);
            }
        }
    }
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = call.func.as_ref()
            && let Some(segment) = path.path.segments.last()
        {
            self.0.insert(segment.ident.to_string());
        }
        syn::visit::visit_expr_call(self, call);
    }
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.0.insert(call.method.to_string());
        syn::visit::visit_expr_method_call(self, call);
    }
    fn visit_item_fn(&mut self, _: &'ast syn::ItemFn) {}
}

fn affected_names<'a>(functions: &'a [Function], seeds: &'a BTreeSet<String>) -> BTreeSet<&'a str> {
    let mut parents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for function in functions {
        for name in &function.calls {
            parents.entry(name).or_default().push(&function.name);
        }
    }
    let mut affected: BTreeSet<&str> = seeds.iter().map(String::as_str).collect();
    let mut pending: VecDeque<&str> = affected.iter().copied().collect();
    while let Some(name) = pending.pop_front() {
        for parent in parents.get(name).into_iter().flatten() {
            if affected.insert(parent) {
                pending.push_back(parent);
            }
        }
    }
    affected
}

fn shared_config(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    matches!(
        name,
        "Cargo.toml" | "Cargo.lock" | "build.rs" | "rust-toolchain" | "rust-toolchain.toml"
    ) || matches!(path, ".cargo/config" | ".cargo/config.toml")
        || path.starts_with(".github/workflows/")
}
fn command_key(command: &runner::CommandSpec) -> String {
    serde_json::to_string(command).expect("command serialization")
}
fn add_check(checks: &mut BTreeMap<String, Check>, command: runner::CommandSpec, reason: String) {
    let check = checks
        .entry(command_key(&command))
        .or_insert_with(|| Check {
            command,
            reasons: Vec::new(),
        });
    if !check.reasons.contains(&reason) && check.reasons.len() < 16 {
        check.reasons.push(reason);
    }
}
fn cargo_test(args: &[&str]) -> runner::CommandSpec {
    runner::CommandSpec::Direct {
        program: "cargo".into(),
        args: args.iter().map(|value| (*value).into()).collect(),
    }
}

/// Uses the current admitted catalogue and the same no-follow bounded source reader.
/// All source I/O and parsing belong to the existing context worker.
#[allow(clippy::too_many_arguments)]
pub fn recommend(
    request: u64,
    task_id: i64,
    partner: i64,
    report: &repo::Report,
    runs: &[runner::RunRecord],
    failures: &[records::FailureSignature],
    validations: &[Vec<String>],
    exclusions: &[PathBuf],
    stopped: &AtomicBool,
) -> Result<Plan, String> {
    context::validate_project_root(&report.repository.root)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut changed = BTreeSet::new();
    for (path, file) in &report.current.files {
        if file.dirty == Some(true)
            && !context::excluded(path, exclusions)
            && !report.repository.excludes_source(path)
            && let Some(path) = path.to_str()
            && privacy::redact(path) == path
        {
            changed.insert(path.to_owned());
        }
    }
    for change in &report.changes {
        if !context::excluded(&change.path, exclusions)
            && !report.repository.excludes_source(&change.path)
            && let Some(path) = change.path.to_str()
            && privacy::redact(path) == path
        {
            changed.insert(path.to_owned());
        }
    }
    if changed.len() > MAX_FILES {
        return Err("Impact selection exceeds 256 changed paths; narrow the change first".into());
    }
    let mut plan = Plan {
        request,
        task_id,
        partner,
        project: report.repository.root.clone(),
        before: repo::fingerprint(report),
        checks: Vec::new(),
        changed_paths: changed.iter().cloned().collect(),
        affected_symbols: Vec::new(),
        omissions: Vec::new(),
        scanned_files: 0,
        scanned_bytes: 0,
        broader: false,
    };
    if !report.current.complete || !report.current.stable {
        plan.omissions
            .push("Repository observation is incomplete or moving".into());
        plan.broader = true;
    }
    let mut functions = Vec::new();
    let mut seeds = BTreeSet::new();
    let mut has_cargo = false;
    let mut scanned = BTreeSet::new();
    for (path, file) in &report.current.files {
        if stopped.load(Ordering::Relaxed) {
            return Err("Impact recommendation cancelled".into());
        }
        if Instant::now() >= deadline {
            plan.omissions
                .push("Ten-second analysis deadline reached".into());
            plan.broader = true;
            break;
        }
        if context::excluded(path, exclusions) || report.repository.excludes_source(path) {
            continue;
        }
        let Some(relative) = path.to_str().filter(|path| privacy::redact(path) == *path) else {
            continue;
        };
        let rust = path.extension().is_some_and(|extension| extension == "rs");
        if !rust && relative != "Cargo.toml" {
            continue;
        }
        if plan.scanned_files == MAX_FILES || functions.len() >= MAX_SYMBOLS {
            plan.omissions
                .push("256-file or 8192-symbol analysis bound reached".into());
            plan.broader = true;
            break;
        }
        let repo::FileState::Regular {
            sha256,
            bytes,
            binary: false,
            ..
        } = &file.state
        else {
            plan.omissions.push(format!(
                "{relative}: source is not an admitted regular text file"
            ));
            plan.broader = true;
            continue;
        };
        if *bytes > MAX_FILE_BYTES as u64
            || plan.scanned_bytes.saturating_add(*bytes as usize) > MAX_BYTES
        {
            plan.omissions
                .push(format!("{relative}: 128-KiB source / 8-MiB total bound"));
            plan.broader = true;
            continue;
        }
        if !repo::source_visible(&report.repository.root, path, stopped)? {
            return Err("Source policy changed; refresh impact review".into());
        }
        let (bytes, digest) =
            repo::read_source(&report.repository.root, path, MAX_FILE_BYTES, stopped)?;
        if &digest != sha256 {
            return Err("Source changed during impact analysis; refresh first".into());
        }
        plan.scanned_files += 1;
        plan.scanned_bytes += bytes.len();
        let Ok(text) = std::str::from_utf8(&bytes) else {
            plan.omissions
                .push(format!("{relative}: unsupported text encoding"));
            plan.broader = true;
            continue;
        };
        if privacy::redact(text) != text {
            plan.omissions
                .push(format!("{relative}: credential-bearing source omitted"));
            plan.broader = true;
            continue;
        }
        scanned.insert(relative.to_owned());
        if !rust {
            has_cargo = toml::from_str::<toml::Value>(text).is_ok_and(|value| {
                value.get("package").is_some() || value.get("workspace").is_some()
            });
            continue;
        }
        if context::symbols::check_parse_bound(text).is_err() {
            plan.omissions.push(format!(
                "{relative}: conservative Rust parse/recursion bound or invalid tokens"
            ));
            plan.broader = true;
            continue;
        }
        let syntax = match syn::parse_file(text) {
            Ok(syntax) => syntax,
            Err(_) => {
                plan.omissions
                    .push(format!("{relative}: Rust parse failure"));
                plan.broader = true;
                continue;
            }
        };
        let mut collector = Functions {
            path: relative,
            modules: Vec::new(),
            values: Vec::new(),
            overflow: false,
            definitions: BTreeSet::new(),
        };
        collector.visit_file(&syntax);
        if collector.overflow || functions.len() + collector.values.len() > MAX_SYMBOLS {
            plan.omissions.push("8192-symbol bound reached".into());
            plan.broader = true;
            break;
        }
        if changed.contains(relative) {
            seeds.extend(collector.definitions);
        }
        functions.extend(collector.values);
    }
    let affected = affected_names(&functions, &seeds);
    plan.affected_symbols = affected
        .iter()
        .take(256)
        .map(|name| (*name).to_owned())
        .collect();
    if affected.len() > 256 {
        plan.omissions.push(
            "Affected-symbol display limited to256; selection uses all bounded symbols".into(),
        );
    }
    let mut checks = BTreeMap::new();
    for signature in failures.iter().take(256) {
        let relevant =
            signature.paths.is_empty() || signature.paths.iter().any(|path| changed.contains(path));
        if !relevant {
            continue;
        }
        for run_id in &signature.runs {
            if let Some(run) = runs.iter().find(|run| {
                run.prepared.task_id == task_id
                    && &run.prepared.run_id == run_id
                    && run.state == runner::RunState::Failed
                    && run.prepared.cwd == report.repository.root
                    && !run.output_unavailable
            }) && privacy::redact(&command_key(&run.prepared.spec))
                == command_key(&run.prepared.spec)
            {
                add_check(
                    &mut checks,
                    run.prepared.spec.clone(),
                    format!(
                        "Dex retained failure {} / run {}: {}",
                        signature.key,
                        run_id,
                        if signature.paths.is_empty() {
                            "no reliable path mapping; conservatively retain this task's check"
                        } else {
                            "failure source overlaps this change"
                        }
                    ),
                );
            }
        }
    }
    let tests: Vec<_> = functions
        .iter()
        .filter(|function| function.test && affected.contains(function.name.as_str()))
        .collect();
    let unsupported_change = changed
        .iter()
        .any(|path| !path.ends_with(".rs") && !shared_config(path));
    let unmatched_rust = changed.iter().any(|path| {
        path.ends_with(".rs")
            && (!scanned.contains(path) || !functions.iter().any(|function| &function.path == path))
    });
    plan.broader |=
        changed.iter().any(|path| shared_config(path)) || unsupported_change || unmatched_rust;
    if has_cargo && !changed.is_empty() {
        if plan.broader || tests.is_empty() || tests.len() > MAX_CHECKS {
            plan.broader = true;
            add_check(&mut checks, cargo_test(&["test", "--locked", "--workspace", "--all-targets"]),
                "Shared configuration, missing/unsupported mapping, analysis omission or no bounded caller-test mapping requires broader Cargo checks; still not complete verification".into());
        } else {
            for test in tests {
                let mut args = vec!["test".to_owned(), "--locked".into(), "--workspace".into()];
                if let Some(target) = test
                    .path
                    .strip_prefix("tests/")
                    .and_then(|path| path.strip_suffix(".rs"))
                    .filter(|path| !path.contains('/'))
                {
                    args.extend(["--test".into(), target.into()]);
                }
                args.push(test.qualified.clone());
                if test.ignored {
                    args.extend(["--".into(), "--ignored".into()]);
                }
                add_check(
                    &mut checks,
                    runner::CommandSpec::Direct {
                        program: "cargo".into(),
                        args,
                    },
                    format!(
                        "{}: test {} reaches changed definitions through syntax-name dependencies (conservative, no semantic binding){}",
                        test.path,
                        test.qualified,
                        if test.ignored {
                            "; explicitly includes the ignored test via -- --ignored"
                        } else {
                            ""
                        }
                    ),
                );
            }
        }
    }
    if plan.broader || (!changed.is_empty() && checks.is_empty()) {
        for args in validations.iter().take(MAX_CHECKS) {
            if let Some((program, args)) = args.split_first()
                && !program.is_empty()
                && args.len() < 128
                && privacy::redact(&serde_json::to_string(&(program, args)).unwrap_or_default())
                    == serde_json::to_string(&(program, args)).unwrap_or_default()
            {
                add_check(&mut checks, runner::CommandSpec::Direct { program: program.clone(), args: args.to_vec() },
                    "Explicit existing recipe validation mapping retained as broader coverage proposal".into());
            }
        }
    }
    if has_cargo && plan.broader {
        let broad = command_key(&cargo_test(&[
            "test",
            "--locked",
            "--workspace",
            "--all-targets",
        ]));
        if checks.contains_key(&broad) {
            let mut reasons = Vec::new();
            checks.retain(|key, check| {
                let covered = key != &broad && matches!(&check.command,
                    runner::CommandSpec::Direct { program, args }
                    if program == "cargo" && (matches!(args.as_slice(), [test, locked, filter]
                        if test == "test" && locked == "--locked" && !filter.starts_with('-'))
                        || matches!(args.as_slice(), [test, locked, workspace, filter]
                            if test == "test" && locked == "--locked" && workspace == "--workspace" && !filter.starts_with('-'))));
                if covered { reasons.append(&mut check.reasons); }
                !covered
            });
            let check = checks.get_mut(&broad).expect("retained broader check");
            for reason in reasons {
                if check.reasons.len() < 16 && !check.reasons.contains(&reason) {
                    check.reasons.push(reason);
                }
            }
        }
    }
    if checks.len() > MAX_CHECKS {
        return Err("More than16 distinct relevant checks; narrow the change or explicitly review commands separately (none silently omitted)".into());
    }
    plan.checks = checks.into_values().collect();
    plan.omissions.truncate(MAX_FILES + 8);
    if stopped.load(Ordering::Relaxed) {
        return Err("Impact recommendation cancelled".into());
    }
    if report.repository.git_dir.is_some() {
        repo::revalidate_review(report)?;
    } else {
        plan.omissions.push("Filesystem-only project: source observations are not a stable Git verification fingerprint; command-plan admission may refuse".into());
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_edges_link_caller_test_without_comment_or_string_dependencies() {
        let syntax = syn::parse_file(
            r#"
            fn callee() -> usize { 2 }
            fn caller() -> usize { callee() }
            #[cfg(test)] mod tests {
                #[test] fn consumer_boundary() { assert_eq!(super::caller(), 2); }
                #[test] fn lookalike() { let _ = "callee()"; /* caller() */ }
            }
        "#,
        )
        .unwrap();
        let mut collector = Functions {
            path: "src/lib.rs",
            modules: Vec::new(),
            values: Vec::new(),
            overflow: false,
            definitions: BTreeSet::new(),
        };
        collector.visit_file(&syntax);
        let caller = collector
            .values
            .iter()
            .find(|function| function.name == "caller")
            .unwrap();
        assert!(caller.calls.contains("callee"));
        let test = collector
            .values
            .iter()
            .find(|function| function.name == "consumer_boundary")
            .unwrap();
        assert!(test.test);
        assert_eq!(test.qualified, "tests::consumer_boundary");
        assert!(test.calls.contains("caller"));
        let seeds = BTreeSet::from(["callee".to_owned()]);
        let affected = affected_names(&collector.values, &seeds);
        assert!(affected.contains("consumer_boundary"));
        assert!(!affected.contains("lookalike"));
        let lookalike = collector
            .values
            .iter()
            .find(|function| function.name == "lookalike")
            .unwrap();
        assert!(!lookalike.calls.contains("callee"));
        assert!(!lookalike.calls.contains("caller"));
    }

    #[test]
    fn shared_cargo_configuration_and_build_inputs_require_broader_checks() {
        for path in [
            "Cargo.toml",
            "nested/Cargo.toml",
            "Cargo.lock",
            "build.rs",
            ".cargo/config.toml",
            ".github/workflows/native.yml",
        ] {
            assert!(shared_config(path), "{path}");
        }
        assert!(!shared_config("src/config.rs"));
    }
}
