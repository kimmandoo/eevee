use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use fluent_syntax::{ast, parser};
use sha2::{Digest, Sha256};
use unic_langid::LanguageIdentifier;

#[derive(Clone, Debug, Default)]
struct Node {
    variables: BTreeSet<String>,
    references: Vec<String>,
    public: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    pub messages: BTreeMap<String, BTreeSet<String>>,
    all: BTreeMap<String, BTreeSet<String>>,
}

fn inline(value: &ast::InlineExpression<&str>, node: &mut Node) -> Result<(), String> {
    use ast::InlineExpression::*;
    match value {
        VariableReference { id } => {
            node.variables.insert(id.name.to_owned());
        }
        Placeable { expression: value } => expression(value, node)?,
        MessageReference { id, attribute } => {
            node.references.push(reference(id.name, attribute.as_ref()));
        }
        TermReference {
            id,
            attribute,
            arguments,
        } => {
            if let Some(arguments) = arguments {
                for value in &arguments.positional {
                    inline(value, node)?;
                }
                for argument in &arguments.named {
                    inline(&argument.value, node)?;
                }
            }
            node.references
                .push(reference(&format!("-{}", id.name), attribute.as_ref()));
        }
        FunctionReference { id, .. } => {
            return Err(format!(
                "function {} is not registered by the localization engine",
                id.name
            ));
        }
        StringLiteral { .. } | NumberLiteral { .. } => {}
    }
    Ok(())
}

fn reference(id: &str, attribute: Option<&ast::Identifier<&str>>) -> String {
    attribute.map_or_else(
        || id.to_owned(),
        |attribute| format!("{id}.{}", attribute.name),
    )
}

fn expression(value: &ast::Expression<&str>, node: &mut Node) -> Result<(), String> {
    match value {
        ast::Expression::Inline(value) => inline(value, node),
        ast::Expression::Select { selector, variants } => {
            inline(selector, node)?;
            for variant in variants {
                pattern(&variant.value, node)?;
            }
            Ok(())
        }
    }
}

fn pattern(value: &ast::Pattern<&str>, node: &mut Node) -> Result<(), String> {
    for element in &value.elements {
        if let ast::PatternElement::Placeable { expression: value } = element {
            expression(value, node)?;
        }
    }
    Ok(())
}

fn insert(
    nodes: &mut BTreeMap<String, Node>,
    id: String,
    value: &ast::Pattern<&str>,
    public: bool,
) -> Result<(), String> {
    let mut node = Node {
        public,
        ..Node::default()
    };
    pattern(value, &mut node).map_err(|error| format!("{id}: {error}"))?;
    if nodes.insert(id.clone(), node).is_some() {
        return Err(format!("duplicate Fluent key {id}"));
    }
    Ok(())
}

fn variables(
    id: &str,
    nodes: &BTreeMap<String, Node>,
    visiting: &mut BTreeSet<String>,
    resolved: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<BTreeSet<String>, String> {
    if let Some(value) = resolved.get(id) {
        return Ok(value.clone());
    }
    if !visiting.insert(id.to_owned()) {
        return Err(format!("cyclic Fluent reference at {id}"));
    }
    let node = nodes
        .get(id)
        .ok_or_else(|| format!("missing Fluent reference {id}"))?;
    let mut result = node.variables.clone();
    for target in &node.references {
        let required = variables(target, nodes, visiting, resolved)?;
        // Fluent terms have their own named-argument scope, never the caller's.
        if !target.starts_with('-') {
            result.extend(required);
        }
    }
    visiting.remove(id);
    resolved.insert(id.to_owned(), result.clone());
    Ok(result)
}

/// The build and consumer regressions use the same parser and semantic validation.
pub fn validate_resources(resources: &[(&str, &str)]) -> Result<Catalog, String> {
    let mut nodes = BTreeMap::new();
    let mut entry_ids = BTreeSet::new();
    for (name, source) in resources {
        let resource = parser::parse(*source)
            .map_err(|(_, errors)| format!("{name}: malformed Fluent resource: {errors:?}"))?;
        for entry in resource.body {
            let (id, value, attributes, public) = match entry {
                ast::Entry::Message(message) => (
                    message.id.name.to_owned(),
                    message.value,
                    message.attributes,
                    true,
                ),
                ast::Entry::Term(term) => (
                    format!("-{}", term.id.name),
                    Some(term.value),
                    term.attributes,
                    false,
                ),
                ast::Entry::Comment(_)
                | ast::Entry::GroupComment(_)
                | ast::Entry::ResourceComment(_) => continue,
                ast::Entry::Junk { .. } => {
                    return Err(format!("{name}: malformed Fluent resource"));
                }
            };
            if !entry_ids.insert(id.clone()) {
                return Err(format!("{name}: duplicate Fluent key {id}"));
            }
            if let Some(value) = value {
                insert(&mut nodes, id.clone(), &value, public)?;
            }
            for attribute in attributes {
                insert(
                    &mut nodes,
                    format!("{id}.{}", attribute.id.name),
                    &attribute.value,
                    false,
                )?;
            }
        }
    }
    let mut all = BTreeMap::new();
    for id in nodes.keys() {
        variables(id, &nodes, &mut BTreeSet::new(), &mut all)?;
    }
    let messages = all
        .iter()
        .filter(|(id, _)| nodes[*id].public)
        .map(|(id, vars)| (id.clone(), vars.clone()))
        .collect();
    let catalog = Catalog { messages, all };
    identifiers(&catalog)?;
    Ok(catalog)
}

/// English must define every key; translations may omit keys but not change arguments.
pub fn validate_translation(default: &Catalog, translation: &Catalog) -> Result<(), String> {
    for (id, variables) in &translation.all {
        let expected = default
            .all
            .get(id)
            .ok_or_else(|| format!("English catalog is incomplete: missing {id}"))?;
        if expected != variables {
            return Err(format!(
                "{id}: placeholder mismatch: expected {expected:?}, found {variables:?}"
            ));
        }
    }
    for id in translation.messages.keys() {
        if !default.messages.contains_key(id) {
            return Err(format!("{id}: translated message has no English value"));
        }
    }
    Ok(())
}

fn identifier(id: &str) -> Result<String, String> {
    let mut variant = String::new();
    for part in id.split('-') {
        let mut characters = part.chars();
        let first = characters
            .next()
            .ok_or_else(|| format!("{id}: empty Rust identifier segment"))?;
        variant.extend(first.to_uppercase());
        variant.extend(characters);
    }
    if !variant.starts_with(|value: char| value.is_ascii_alphabetic())
        || !variant
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || value == '_')
        || variant == "Self"
    {
        return Err(format!("{id}: cannot generate a Rust identifier"));
    }
    Ok(variant)
}

fn identifiers(catalog: &Catalog) -> Result<(), String> {
    let mut variants = BTreeMap::new();
    for id in catalog.messages.keys() {
        let variant = identifier(id)?;
        if let Some(previous) = variants.insert(variant.clone(), id) {
            return Err(format!(
                "Rust identifier collision: {previous} and {id} both generate {variant}"
            ));
        }
    }
    Ok(())
}

fn entries(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    Ok(entries)
}

fn generate(root: &Path) -> Result<String, String> {
    let mut locales = Vec::new();
    let mut tags = BTreeSet::new();
    for directory in entries(root)? {
        if !directory.is_dir() {
            continue;
        }
        let name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("non-Unicode locale directory")?;
        let tag: LanguageIdentifier = name
            .parse()
            .map_err(|error| format!("invalid locale directory {name}: {error}"))?;
        let tag = tag.to_string();
        if !tags.insert(tag.clone()) {
            return Err(format!("duplicate normalized locale {tag}"));
        }
        println!("cargo:rerun-if-changed={}", directory.display());
        let resources = entries(&directory)?
            .into_iter()
            .filter(|path| path.extension().is_some_and(|extension| extension == "ftl"))
            .map(|path| {
                fs::read_to_string(&path)
                    .map(|source| (path, source))
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if resources.is_empty() {
            return Err(format!("{name}: locale contains no Fluent resources"));
        }
        let named = resources
            .iter()
            .map(|(path, source)| (path.to_str().unwrap(), source.as_str()))
            .collect::<Vec<_>>();
        let catalog = validate_resources(&named).map_err(|error| format!("{tag}: {error}"))?;
        locales.push((tag, resources, catalog));
    }
    let english = locales
        .iter()
        .position(|(tag, _, _)| tag == "en")
        .ok_or("complete default locales/en catalog is required")?;
    let default = &locales[english].2;
    if default.messages.is_empty() {
        return Err("English catalog contains no messages".into());
    }
    for (tag, _, catalog) in &locales {
        validate_translation(default, catalog).map_err(|error| format!("{tag}: {error}"))?;
    }
    let mut output = String::from("// Generated from embedded Fluent catalogs by build.rs.\n");
    writeln!(output, "const ENGLISH_INDEX: usize = {english};").unwrap();
    writeln!(
        output,
        "const LOCALE_TAGS: &[&str] = &{:?};",
        locales.iter().map(|(tag, _, _)| tag).collect::<Vec<_>>()
    )
    .unwrap();
    output.push_str("const LOCALES: &[EmbeddedLocale] = &[\n");
    for (tag, resources, _) in &locales {
        writeln!(output, "EmbeddedLocale {{ tag: {tag:?}, resources: &[").unwrap();
        for (path, _) in resources {
            let absolute = path.canonicalize().map_err(|error| error.to_string())?;
            writeln!(
                output,
                "include_str!({:?}),",
                absolute.to_str().ok_or("non-Unicode catalog path")?
            )
            .unwrap();
        }
        output.push_str("] },\n");
    }
    output.push_str("];\n");
    for (name, dynamic) in [("Message", false), ("Template", true)] {
        writeln!(
            output,
            "#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n#[repr(usize)]\npub enum {name} {{"
        )
        .unwrap();
        for (id, _) in default
            .messages
            .iter()
            .filter(|(_, variables)| variables.is_empty() != dynamic)
        {
            writeln!(output, "{},", identifier(id)?).unwrap();
        }
        output.push_str("}\n");
        let prefix = if dynamic { "TEMPLATE" } else { "MESSAGE" };
        writeln!(output, "const {prefix}_IDS: &[&str] = &[").unwrap();
        for (id, _) in default
            .messages
            .iter()
            .filter(|(_, variables)| variables.is_empty() != dynamic)
        {
            writeln!(output, "{id:?},").unwrap();
        }
        output.push_str("];\n");
    }
    output.push_str("const TEMPLATE_VARIABLES: &[&[&str]] = &[\n");
    for (_, variables) in default
        .messages
        .iter()
        .filter(|(_, variables)| !variables.is_empty())
    {
        writeln!(output, "&{:?},", variables.iter().collect::<Vec<_>>()).unwrap();
    }
    output.push_str("];\n");
    Ok(output)
}

fn rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) -> Result<(), String> {
    for path in entries(directory)? {
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "lifecycle identity refuses linked source: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            rust_sources(&path, sources)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
    Ok(())
}

fn identity_field(digest: &mut Sha256, name: &[u8], value: &[u8]) {
    digest.update((name.len() as u64).to_le_bytes());
    digest.update(name);
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value);
}

/// Conservative compiled-reader identity: recursively includes child modules,
/// not a hand-maintained list that can omit a newly added serialized type.
pub fn lifecycle_identity(
    root: &Path,
    package_name: &str,
    configuration: &str,
) -> Result<String, String> {
    let mut digest = Sha256::new();
    identity_field(&mut digest, b"configuration", configuration.as_bytes());
    let mut sources = Vec::new();
    rust_sources(&root.join("src"), &mut sources)?;
    sources.push(root.join("build.rs"));
    sources.sort();
    for source in sources {
        let name = source
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_str()
            .ok_or("Non-Unicode lifecycle source path")?
            .replace('\\', "/");
        let bytes =
            fs::read(&source).map_err(|error| format!("Cannot fingerprint {name}: {error}"))?;
        identity_field(&mut digest, name.as_bytes(), &bytes);
    }
    let manifest =
        fs::read_to_string(root.join("Cargo.toml")).map_err(|error| error.to_string())?;
    let mut package_section = false;
    let mut normalized = String::new();
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            package_section = trimmed == "[package]";
        }
        if package_section
            && trimmed
                .split_once('=')
                .is_some_and(|(key, _)| key.trim() == "version")
        {
            continue;
        }
        normalized.push_str(line);
        normalized.push('\n');
    }
    identity_field(&mut digest, b"Cargo.toml", normalized.as_bytes());
    let lock = fs::read_to_string(root.join("Cargo.lock")).map_err(|error| error.to_string())?;
    let own_name = format!("name = {package_name:?}");
    for package in lock.split("[[package]]") {
        if !package.lines().any(|line| line.trim() == own_name) {
            identity_field(&mut digest, b"locked-dependency", package.as_bytes());
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn main() {
    for path in ["src", "build.rs", "Cargo.toml", "Cargo.lock"] {
        println!("cargo:rerun-if-changed={path}");
    }
    let mut configuration = std::env::vars()
        .filter(|(name, _)| {
            name == "TARGET" || name.starts_with("CARGO_CFG_") || name.starts_with("CARGO_FEATURE_")
        })
        .collect::<Vec<_>>();
    configuration.sort();
    let identity = lifecycle_identity(
        Path::new("."),
        &std::env::var("CARGO_PKG_NAME").expect("Cargo provides package name"),
        &format!("{configuration:?}"),
    )
    .unwrap_or_else(|error| panic!("lifecycle format identity failed: {error}"));
    println!("cargo:rustc-env=EV_LIFECYCLE_FORMAT_SHA256={identity}");
    println!("cargo:rerun-if-changed=locales");
    let generated = generate(Path::new("locales"))
        .unwrap_or_else(|error| panic!("localization catalog validation failed: {error}"));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo provides OUT_DIR"));
    fs::write(out.join("i18n_catalogs.rs"), generated)
        .expect("write generated localization registry");
}

#[cfg(test)]
mod lifecycle_identity_tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "ev-format-identity-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ));
            fs::create_dir_all(directory.join("src/providers/nested")).unwrap();
            fs::write(directory.join("src/main.rs"), "mod providers;\n").unwrap();
            fs::write(
                directory.join("src/providers/nested/reader.rs"),
                "struct Stored { value: String }\n",
            )
            .unwrap();
            fs::write(directory.join("build.rs"), "fn main() {}\n").unwrap();
            let fixture = Self(directory);
            fixture.version("0.1.27");
            fixture
        }
        fn version(&self, version: &str) {
            fs::write(self.0.join("Cargo.toml"), format!(
                "[package]\nname = \"eevee\"\nversion = \"{version}\"\n[dependencies]\nserde = \"1.0.228\"\n"
            )).unwrap();
            fs::write(self.0.join("Cargo.lock"), format!(
                "version = 4\n\n[[package]]\nname = \"eevee\"\nversion = \"{version}\"\ndependencies = [\"serde\"]\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.228\"\n"
            )).unwrap();
        }
        fn identity(&self) -> String {
            lifecycle_identity(&self.0, "eevee", "same-target-and-features").unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn distinct_release_versions_with_identical_recursive_readers_keep_identity() {
        let fixture = Fixture::new();
        let previous = fixture.identity();
        fixture.version("0.1.28");
        assert_eq!(fixture.identity(), previous);
    }

    #[test]
    fn nested_reader_changes_and_new_child_modules_invalidate_compatibility() {
        let fixture = Fixture::new();
        let previous = fixture.identity();
        fs::write(
            fixture.0.join("src/providers/nested/reader.rs"),
            "struct Stored { value: String, strict_new_field: bool }\n",
        )
        .unwrap();
        let changed = fixture.identity();
        assert_ne!(changed, previous);
        fs::write(
            fixture.0.join("src/providers/nested/new_reader.rs"),
            "enum NewStoredVariant { Unknown }\n",
        )
        .unwrap();
        assert_ne!(fixture.identity(), changed);
    }

    #[test]
    fn dependencies_and_compiled_configuration_are_part_of_reader_identity() {
        let fixture = Fixture::new();
        let previous = fixture.identity();
        assert_ne!(
            lifecycle_identity(&fixture.0, "eevee", "different-target-or-features").unwrap(),
            previous
        );
        let lock = fixture.0.join("Cargo.lock");
        fs::write(
            &lock,
            fs::read_to_string(&lock)
                .unwrap()
                .replace("1.0.228", "1.0.229"),
        )
        .unwrap();
        assert_ne!(fixture.identity(), previous);
    }
}
