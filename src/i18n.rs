//! Embedded Fluent catalogs. Locale selection happens at startup; redraws borrow cached text.
//!
//! Add a locale directory under `locales/` containing `.ftl` resources to extend the
//! registry without touching screens. English defines every public key; omitted
//! localized keys resolve in the English bundle, including English plural rules.
//! The build rejects malformed resources, duplicate keys, missing English values,
//! changed argument names, reference cycles and generated Rust identifier collisions.
//! Message/term references and selectors use Fluent semantics; unregistered Fluent
//! functions are rejected at build time rather than failing during display.
//! Explicit settings use BCP 47 lookup; POSIX encoding/modifier normalization is
//! reserved for automatic locale detection. No runtime filesystem or network is used.
use std::fmt;
use std::sync::LazyLock;

use fluent_bundle::{FluentArgs, FluentResource, FluentValue, concurrent::FluentBundle};
use language_tags::LanguageTag;
use unic_langid::LanguageIdentifier;

struct EmbeddedLocale {
    tag: &'static str,
    resources: &'static [&'static str],
}

include!(concat!(env!("OUT_DIR"), "/i18n_catalogs.rs"));

#[cfg(test)]
#[allow(dead_code)]
#[path = "../build.rs"]
mod catalog_build;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Language(usize);

/// Literal text remains text; numbers opt into Fluent's locale-aware plural rules.
#[derive(Clone, Copy, Debug)]
pub enum Argument<'a> {
    Text(&'a str),
    Number(f64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    MissingArgument { message: String, name: String },
    UnknownArgument { message: String, name: String },
    DuplicateArgument { message: String, name: String },
    InvalidNumber { message: String, name: String },
    Resource(String),
    Formatting { message: String, detail: String },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingArgument { message, name } => {
                write!(formatter, "{message}: missing argument {name:?}")
            }
            Self::UnknownArgument { message, name } => {
                write!(formatter, "{message}: unknown argument {name:?}")
            }
            Self::DuplicateArgument { message, name } => {
                write!(formatter, "{message}: duplicate argument {name:?}")
            }
            Self::InvalidNumber { message, name } => write!(
                formatter,
                "{message}: argument {name:?} must be a finite number"
            ),
            Self::Resource(detail) => write!(
                formatter,
                "invalid embedded localization resource: {detail}"
            ),
            Self::Formatting { message, detail } => write!(formatter, "{message}: {detail}"),
        }
    }
}

impl std::error::Error for Error {}

type Bundle = FluentBundle<FluentResource>;

struct Catalog {
    bundle: Bundle,
    text: Vec<Option<String>>,
}

static CATALOGS: LazyLock<Vec<Catalog>> = LazyLock::new(load_catalogs);

fn bundle(tag: &str, resources: &[&str]) -> Result<Bundle, Error> {
    let locale: LanguageIdentifier = tag
        .parse()
        .map_err(|error| Error::Resource(format!("{tag}: {error}")))?;
    let mut bundle = Bundle::new_concurrent(vec![locale]);
    // Terminal payloads, identifiers and existing English layout must stay byte-for-byte literal.
    bundle.set_use_isolating(false);
    for source in resources {
        let resource = FluentResource::try_new((*source).to_owned())
            .map_err(|(_, errors)| Error::Resource(format!("{tag}: {errors:?}")))?;
        bundle
            .add_resource(resource)
            .map_err(|errors| Error::Resource(format!("{tag}: {errors:?}")))?;
    }
    Ok(bundle)
}

fn resolved_bundle<'a>(
    catalogs: &'a [Catalog],
    language: usize,
    english: usize,
    id: &str,
) -> &'a Bundle {
    if catalogs[language]
        .bundle
        .get_message(id)
        .and_then(|message| message.value())
        .is_some()
    {
        &catalogs[language].bundle
    } else {
        // Use the English bundle itself, not English source under another locale's rules.
        &catalogs[english].bundle
    }
}

fn render(bundle: &Bundle, id: &str, args: Option<&FluentArgs<'_>>) -> Result<String, Error> {
    let message = bundle.get_message(id).ok_or_else(|| Error::Formatting {
        message: id.into(),
        detail: "message is absent from the complete English catalog".into(),
    })?;
    let pattern = message.value().ok_or_else(|| Error::Formatting {
        message: id.into(),
        detail: "message has no value".into(),
    })?;
    let mut errors = Vec::new();
    let value = bundle.format_pattern(pattern, args, &mut errors);
    if errors.is_empty() {
        Ok(value.into_owned())
    } else {
        Err(Error::Formatting {
            message: id.into(),
            detail: format!("{errors:?}"),
        })
    }
}

fn load_catalogs() -> Vec<Catalog> {
    LOCALES
        .iter()
        .map(|locale| {
            let bundle = bundle(locale.tag, locale.resources)
                .expect("build-validated embedded Fluent catalog");
            let text = MESSAGE_IDS
                .iter()
                .map(|id| {
                    bundle
                        .get_message(id)
                        .and_then(|message| message.value())
                        .map(|_| {
                            render(&bundle, id, None)
                                .expect("build-validated static Fluent message")
                        })
                })
                .collect();
            Catalog { bundle, text }
        })
        .collect()
}
fn catalogs() -> &'static [Catalog] {
    &CATALOGS
}

/// RFC 4647-style lookup: exact tag, progressively shorter base tags, then a
/// registered specialization of the base. No UI language cases are hardcoded.
fn negotiate(tag: &LanguageTag, available: &[&str]) -> Option<usize> {
    let mut candidate = tag.as_str();
    loop {
        if let Some(index) = available
            .iter()
            .position(|tag| tag.eq_ignore_ascii_case(candidate))
        {
            return Some(index);
        }
        if let Some(index) = available.iter().position(|tag| {
            tag.get(..candidate.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(candidate))
                && tag.as_bytes().get(candidate.len()) == Some(&b'-')
        }) {
            return Some(index);
        }
        match candidate.rsplit_once('-') {
            Some((base, _)) => candidate = base,
            None => return None,
        }
    }
}

fn select(tag: &str) -> Result<Language, String> {
    let parsed: LanguageTag = tag
        .parse()
        .map_err(|error| format!("invalid language tag {tag:?}: {error}"))?;
    negotiate(&parsed, LOCALE_TAGS)
        .map(Language)
        .ok_or_else(|| {
            format!(
                "unsupported language tag {tag:?}; supported embedded languages: {}",
                LOCALE_TAGS.join(", ")
            )
        })
}

impl Language {
    pub const ENGLISH: Self = Self(ENGLISH_INDEX);

    pub fn registered() -> impl Iterator<Item = Self> {
        (0..LOCALES.len()).map(Self)
    }

    pub fn parse(tag: &str) -> Result<Self, String> {
        let language = select(tag)?;
        catalogs();
        Ok(language)
    }

    /// Accept POSIX locale encodings/modifiers and underscores only for automatic
    /// environment detection. Explicit settings require a valid BCP 47 tag.
    pub fn from_locale(value: &str) -> Self {
        let tag = value
            .split(['.', '@'])
            .next()
            .unwrap_or(value)
            .replace('_', "-");
        let language = select(&tag).unwrap_or(Self::ENGLISH);
        catalogs();
        language
    }

    pub fn tag(self) -> &'static str {
        LOCALES[self.0].tag
    }

    pub fn text(self, message: Message) -> &'static str {
        cached_text(catalogs(), self.0, message as usize)
    }

    pub fn format(self, template: Template, args: &[(&str, &str)]) -> Result<String, Error> {
        let index = template as usize;
        let id = TEMPLATE_IDS[index];
        let args = arguments(
            id,
            TEMPLATE_VARIABLES[index],
            args.iter()
                .map(|&(name, value)| (name, Argument::Text(value))),
        )?;
        render(
            resolved_bundle(catalogs(), self.0, ENGLISH_INDEX, id),
            id,
            Some(&args),
        )
    }

    /// Use explicit numeric arguments when a catalog employs plural selectors.
    pub fn format_values(
        self,
        template: Template,
        args: &[(&str, Argument<'_>)],
    ) -> Result<String, Error> {
        let index = template as usize;
        let id = TEMPLATE_IDS[index];
        let args = arguments(id, TEMPLATE_VARIABLES[index], args.iter().copied())?;
        render(
            resolved_bundle(catalogs(), self.0, ENGLISH_INDEX, id),
            id,
            Some(&args),
        )
    }
}

/// Search every embedded language without allocating a per-command dictionary.
pub fn translations(message: Message) -> impl Iterator<Item = &'static str> {
    let catalogs = catalogs();
    (0..catalogs.len()).map(move |language| cached_text(catalogs, language, message as usize))
}

fn cached_text(catalogs: &[Catalog], language: usize, message: usize) -> &str {
    catalogs[language].text[message]
        .as_deref()
        .unwrap_or_else(|| {
            catalogs[ENGLISH_INDEX].text[message]
                .as_deref()
                .expect("complete English static cache")
        })
}

fn arguments<'a>(
    id: &str,
    required: &[&str],
    values: impl IntoIterator<Item = (&'a str, Argument<'a>)>,
) -> Result<FluentArgs<'a>, Error> {
    let mut args = FluentArgs::with_capacity(required.len());
    for (name, value) in values {
        if !required.contains(&name) {
            return Err(Error::UnknownArgument {
                message: id.into(),
                name: name.into(),
            });
        }
        if args.get(name).is_some() {
            return Err(Error::DuplicateArgument {
                message: id.into(),
                name: name.into(),
            });
        }
        let value = match value {
            Argument::Text(text) => FluentValue::from(text),
            Argument::Number(number) if number.is_finite() => FluentValue::from(number),
            Argument::Number(_) => {
                return Err(Error::InvalidNumber {
                    message: id.into(),
                    name: name.into(),
                });
            }
        };
        args.set(name, value);
    }
    for &name in required {
        if !args.iter().any(|(key, _)| key == name) {
            return Err(Error::MissingArgument {
                message: id.into(),
                name: name.into(),
            });
        }
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_catalogs() -> Vec<Catalog> {
        vec![
            Catalog { bundle: bundle("en", &["fallback = { $count ->\n    [one] singular\n   *[other] plural\n}\nliteral = { $owner }::{ $payload }\nstatic = retained"]).unwrap(), text: Vec::new() },
            Catalog { bundle: bundle("fr", &["localized = traduit"]).unwrap(), text: Vec::new() },
            Catalog { bundle: bundle("ja", &["localized = 翻訳"]).unwrap(), text: Vec::new() },
        ]
    }

    #[test]
    fn missing_translation_uses_english_rules_not_selected_locale_rules() {
        let catalogs = fixture_catalogs();
        let args = arguments("fallback", &["count"], [("count", Argument::Number(0.0))]).unwrap();
        // French treats 0 as `one`; the missing message must instead use English `other`.
        assert_eq!(
            render(
                resolved_bundle(&catalogs, 1, 0, "fallback"),
                "fallback",
                Some(&args)
            )
            .unwrap(),
            "plural"
        );
        assert_eq!(
            render(resolved_bundle(&catalogs, 2, 0, "static"), "static", None).unwrap(),
            "retained"
        );
    }

    #[test]
    fn third_locale_negotiates_without_language_specific_source_cases() {
        let available = ["en", "ko", "fr", "zh-Hant"];
        assert_eq!(negotiate(&"fr-CA".parse().unwrap(), &available), Some(2));
        assert_eq!(
            negotiate(&"zh-Hant-TW".parse().unwrap(), &available),
            Some(3)
        );
        assert_eq!(
            negotiate(&"fr-CA-u-nu-latn".parse().unwrap(), &available),
            Some(2)
        );
        assert_eq!(negotiate(&"de-DE".parse().unwrap(), &available), None);
        assert_eq!(Language::from_locale("KO_kr.UTF-8@modifier").tag(), "ko");
        assert_eq!(Language::from_locale("C.UTF-8"), Language::ENGLISH);
        assert!(Language::parse("ko--KR").unwrap_err().contains("invalid"));
        assert!(Language::parse("ko_KR").unwrap_err().contains("invalid"));
        assert!(
            Language::parse("zz-ZZ")
                .unwrap_err()
                .contains("unsupported")
        );
    }

    #[test]
    fn named_untrusted_arguments_are_literal_and_order_independent() {
        let catalogs = fixture_catalogs();
        let payload = "{ $owner }\n\"quoted\" \\ /provider --unsafe <tag>";
        let args = arguments(
            "literal",
            &["owner", "payload"],
            [
                ("payload", Argument::Text(payload)),
                ("owner", Argument::Text("original identity")),
            ],
        )
        .unwrap();
        assert_eq!(
            render(&catalogs[0].bundle, "literal", Some(&args)).unwrap(),
            format!("original identity::{payload}")
        );
    }

    #[test]
    fn public_formatter_preserves_identity_and_rejects_invalid_named_arguments() {
        let field = "{ $partner }\n\"quoted\"";
        let partner = "original /provider <identity>";
        let template = Template::CoreEditorTitle;
        for language in [Language::ENGLISH, Language::parse("ko").unwrap()] {
            let forward = language
                .format(template, &[("field", field), ("partner", partner)])
                .unwrap();
            let reverse = language
                .format(template, &[("partner", partner), ("field", field)])
                .unwrap();
            assert_eq!(forward, reverse);
            assert!(forward.contains(field));
            assert!(forward.contains(partner));
            assert!(
                matches!(language.format(template, &[("partner", partner)]), Err(Error::MissingArgument { name, .. }) if name == "field")
            );
            assert!(
                matches!(language.format(template, &[("field", field), ("partner", partner), ("other", "x")]), Err(Error::UnknownArgument { name, .. }) if name == "other")
            );
        }
    }

    #[test]
    fn required_unknown_and_duplicate_arguments_are_errors() {
        assert!(matches!(
            arguments("literal", &["owner"], []),
            Err(Error::MissingArgument { .. })
        ));
        assert!(matches!(
            arguments("literal", &["owner"], [("other", Argument::Text("x"))]),
            Err(Error::UnknownArgument { .. })
        ));
        assert!(matches!(
            arguments(
                "literal",
                &["owner"],
                [
                    ("owner", Argument::Text("x")),
                    ("owner", Argument::Text("y"))
                ]
            ),
            Err(Error::DuplicateArgument { .. })
        ));
        assert!(matches!(
            arguments("count", &["n"], [("n", Argument::Number(f64::NAN))]),
            Err(Error::InvalidNumber { .. })
        ));
    }

    #[test]
    fn malformed_or_inconsistent_resources_are_rejected_before_consumption() {
        use catalog_build::{validate_resources, validate_translation};
        assert!(validate_resources(&[("broken.ftl", "broken = { $value")]).is_err());
        assert!(
            validate_resources(&[("a.ftl", "same = first"), ("b.ftl", "same = second")]).is_err()
        );
        assert!(validate_resources(&[("collision.ftl", "case-id = a\ncase-Id = b")]).is_err());
        let english =
            validate_resources(&[("en.ftl", "message = { $owner }\nstatic = value")]).unwrap();
        let missing = validate_resources(&[("fr.ftl", "static = valeur")]).unwrap();
        assert!(validate_translation(&english, &missing).is_ok());
        let changed = validate_resources(&[("fr.ftl", "message = { $someone }")]).unwrap();
        assert!(validate_translation(&english, &changed).is_err());
        let extra = validate_resources(&[("fr.ftl", "unregistered = value")]).unwrap();
        assert!(validate_translation(&english, &extra).is_err());
    }

    #[test]
    fn redraw_and_search_borrow_the_initialized_static_cache() {
        let first = Language::ENGLISH.text(Message::CommonStay);
        let second = Language::ENGLISH.text(Message::CommonStay);
        assert!(std::ptr::eq(first, second));
        assert!(translations(Message::CommonStay).any(|text| std::ptr::eq(text, first)));
    }
}
