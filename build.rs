use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use fluent_syntax::unicode::unescape_unicode_to_string;
use fluent_syntax::{ast, parser};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Segment {
    Text(String),
    Argument(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CompiledMessage {
    variables: BTreeSet<String>,
    segments: Vec<Segment>,
}

impl CompiledMessage {
    fn text(&mut self, value: &str) {
        if let Some(Segment::Text(previous)) = self.segments.last_mut() {
            previous.push_str(value);
        } else if !value.is_empty() {
            self.segments.push(Segment::Text(value.to_owned()));
        }
    }
}

/// Compile only the literal and named-interpolation syntax used by the UI.
/// Unsupported Fluent features fail the build rather than acquiring runtime semantics.
fn compile_resources(
    resources: &[(&str, &str)],
) -> Result<BTreeMap<String, CompiledMessage>, String> {
    let mut messages = BTreeMap::new();
    let mut identifiers = BTreeMap::new();
    for (name, source) in resources {
        let resource = parser::parse(*source)
            .map_err(|(_, errors)| format!("{name}: malformed message resource: {errors:?}"))?;
        for entry in resource.body {
            let message = match entry {
                ast::Entry::Message(message) => message,
                ast::Entry::Comment(_)
                | ast::Entry::GroupComment(_)
                | ast::Entry::ResourceComment(_) => continue,
                _ => return Err(format!("{name}: only messages and comments are supported")),
            };
            let id = message.id.name;
            if !message.attributes.is_empty() {
                return Err(format!(
                    "{name}: {id}: message attributes are not supported"
                ));
            }
            let pattern = message
                .value
                .ok_or_else(|| format!("{name}: {id}: missing value"))?;
            let mut compiled = CompiledMessage::default();
            for element in pattern.elements {
                match element {
                    ast::PatternElement::TextElement { value } => compiled.text(value),
                    ast::PatternElement::Placeable {
                        expression:
                            ast::Expression::Inline(ast::InlineExpression::StringLiteral { value }),
                    } => compiled.text(&unescape_unicode_to_string(value)),
                    ast::PatternElement::Placeable {
                        expression:
                            ast::Expression::Inline(ast::InlineExpression::VariableReference { id }),
                    } => {
                        compiled.variables.insert(id.name.to_owned());
                        compiled
                            .segments
                            .push(Segment::Argument(id.name.to_owned()));
                    }
                    _ => {
                        return Err(format!(
                            "{name}: {id}: only literal text and named arguments are supported"
                        ));
                    }
                }
            }
            if messages.insert(id.to_owned(), compiled).is_some() {
                return Err(format!("{name}: duplicate message key {id}"));
            }
            let variant = identifier(id)?;
            if let Some(previous) = identifiers.insert(variant.clone(), id.to_owned()) {
                return Err(format!(
                    "Rust identifier collision: {previous} and {id} both generate {variant}"
                ));
            }
        }
    }
    Ok(messages)
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

fn generate_messages(messages: &BTreeMap<String, CompiledMessage>) -> Result<String, String> {
    if messages.is_empty() {
        return Err("message resources contain no messages".into());
    }
    let mut output = String::from("// Generated static English UI messages by build.rs.\n");
    for (name, dynamic) in [("Message", false), ("Template", true)] {
        writeln!(
            output,
            "#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n#[repr(usize)]\npub enum {name} {{"
        )
        .unwrap();
        for (id, _) in messages
            .iter()
            .filter(|(_, message)| message.variables.is_empty() != dynamic)
        {
            writeln!(output, "{},", identifier(id)?).unwrap();
        }
        output.push_str("}\n");
    }
    output.push_str("const MESSAGE_TEXT: &[&str] = &[\n");
    for message in messages
        .values()
        .filter(|message| message.variables.is_empty())
    {
        let text = message
            .segments
            .iter()
            .map(|segment| match segment {
                Segment::Text(text) => text.as_str(),
                Segment::Argument(_) => unreachable!("static message has no arguments"),
            })
            .collect::<String>();
        writeln!(output, "{text:?},").unwrap();
    }
    output.push_str("];\nconst TEMPLATES: &[CompiledTemplate] = &[\n");
    for (id, message) in messages
        .iter()
        .filter(|(_, message)| !message.variables.is_empty())
    {
        writeln!(
            output,
            "CompiledTemplate {{ id: {id:?}, variables: &{:?}, segments: &[",
            message.variables.iter().collect::<Vec<_>>(),
        )
        .unwrap();
        for segment in &message.segments {
            match segment {
                Segment::Text(text) => writeln!(output, "Segment::Text({text:?}),").unwrap(),
                Segment::Argument(name) => {
                    let index = message
                        .variables
                        .iter()
                        .position(|variable| variable == name)
                        .expect("compiled argument is registered");
                    writeln!(output, "Segment::Argument({index}),").unwrap();
                }
            }
        }
        output.push_str("] },\n");
    }
    output.push_str("];\n");
    writeln!(
        output,
        "const MAX_ARGUMENTS: usize = {};",
        messages
            .values()
            .map(|message| message.variables.len())
            .max()
            .unwrap_or(0),
    )
    .unwrap();
    Ok(output)
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
    let resources = entries(root)?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "ftl"))
        .map(|path| {
            println!("cargo:rerun-if-changed={}", path.display());
            fs::read_to_string(&path)
                .map(|source| (path, source))
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let named = resources
        .iter()
        .map(|(path, source)| {
            (
                path.to_str().expect("Unicode message path"),
                source.as_str(),
            )
        })
        .collect::<Vec<_>>();
    generate_messages(&compile_resources(&named)?)
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
    println!("cargo:rerun-if-changed=messages");
    let generated = generate(Path::new("messages"))
        .unwrap_or_else(|error| panic!("message compilation failed: {error}"));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo provides OUT_DIR"));
    fs::write(out.join("messages.rs"), generated).expect("write generated static messages");
}

#[cfg(test)]
mod message_compilation_tests {
    use super::*;

    #[test]
    fn malformed_duplicate_and_colliding_messages_fail_before_consumption() {
        assert!(compile_resources(&[("broken.ftl", "broken = { $value")]).is_err());
        assert!(
            compile_resources(&[("a.ftl", "same = first"), ("b.ftl", "same = second")]).is_err()
        );
        assert!(compile_resources(&[("collision.ftl", "case-id = a\ncase-Id = b")]).is_err());
        for source in [
            "message = { other }",
            "message = { -term }\n-term = value",
            "message = { NUMBER($count) }",
            "message = { $count ->\n    [one] one\n   *[other] many\n}",
            "message = value\n    .attribute = hidden",
        ] {
            assert!(compile_resources(&[("unsupported.ftl", source)]).is_err());
        }
    }

    #[test]
    fn compiler_preserves_literal_escapes_whitespace_and_named_interpolation() {
        let compiled = compile_resources(&[(
            "literal.ftl",
            "literal = { \" \\\"\\\\\\u5bd2\\U01F68A \" }\n    { \"\" }\n    { $payload }::{ $owner }",
        )]).unwrap();
        let message = &compiled["literal"];
        assert_eq!(
            message.variables,
            BTreeSet::from(["owner".to_owned(), "payload".to_owned()])
        );
        assert_eq!(
            message.segments,
            vec![
                Segment::Text(" \"\\寒🚊 \n\n".to_owned()),
                Segment::Argument("payload".to_owned()),
                Segment::Text("::".to_owned()),
                Segment::Argument("owner".to_owned()),
            ]
        );
    }
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
