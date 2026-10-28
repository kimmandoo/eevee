//! English UI text compiled at build time. Static messages borrow literals directly;
//! templates interpolate named arguments as literal data, never as message syntax.
use std::fmt::{self, Write as _};

#[derive(Clone, Copy, Debug)]
pub enum Argument<'a> {
    Text(&'a str),
    Number(f64),
}

impl<'a> From<&'a str> for Argument<'a> {
    fn from(value: &'a str) -> Self {
        Self::Text(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    MissingArgument { message: String, name: String },
    UnknownArgument { message: String, name: String },
    DuplicateArgument { message: String, name: String },
    InvalidNumber { message: String, name: String },
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
        }
    }
}

impl std::error::Error for Error {}

enum Segment {
    Text(&'static str),
    Argument(usize),
}

struct CompiledTemplate {
    id: &'static str,
    variables: &'static [&'static str],
    segments: &'static [Segment],
}

include!(concat!(env!("OUT_DIR"), "/messages.rs"));

impl Message {
    pub fn text(self) -> &'static str {
        MESSAGE_TEXT[self as usize]
    }
}

impl Template {
    pub fn format(self, args: &[(&str, &str)]) -> Result<String, Error> {
        TEMPLATES[self as usize].format(args)
    }

    pub fn format_values(self, args: &[(&str, Argument<'_>)]) -> Result<String, Error> {
        TEMPLATES[self as usize].format(args)
    }
}

impl CompiledTemplate {
    fn format<'a, T: Copy + Into<Argument<'a>>>(
        &self,
        args: &[(&str, T)],
    ) -> Result<String, Error> {
        // Generated argument indexes and a stack array avoid per-format lookup tables.
        let mut values = [Argument::Text(""); MAX_ARGUMENTS];
        let mut provided = [false; MAX_ARGUMENTS];
        for &(name, value) in args {
            let Some(index) = self.variables.iter().position(|required| *required == name) else {
                return Err(Error::UnknownArgument {
                    message: self.id.into(),
                    name: name.into(),
                });
            };
            if provided[index] {
                return Err(Error::DuplicateArgument {
                    message: self.id.into(),
                    name: name.into(),
                });
            }
            let value = value.into();
            if matches!(value, Argument::Number(number) if !number.is_finite()) {
                return Err(Error::InvalidNumber {
                    message: self.id.into(),
                    name: name.into(),
                });
            }
            provided[index] = true;
            values[index] = value;
        }
        for (index, name) in self.variables.iter().enumerate() {
            if !provided[index] {
                return Err(Error::MissingArgument {
                    message: self.id.into(),
                    name: (*name).into(),
                });
            }
        }
        let capacity = self
            .segments
            .iter()
            .map(|segment| match segment {
                Segment::Text(text) => text.len(),
                Segment::Argument(index) => match values[*index] {
                    Argument::Text(text) => text.len(),
                    // Finite f64 Display needs at most 327 bytes, including sign and
                    // leading fractional zeros. Reserve once, without temporary Strings.
                    Argument::Number(_) => 327,
                },
            })
            .sum();
        let mut output = String::with_capacity(capacity);
        for segment in self.segments {
            match segment {
                Segment::Text(text) => output.push_str(text),
                Segment::Argument(index) => match values[*index] {
                    Argument::Text(text) => output.push_str(text),
                    Argument::Number(number) => {
                        write!(output, "{number}").expect("writing a number to String cannot fail")
                    }
                },
            }
        }
        Ok(output)
    }
}

#[cfg(test)]
#[allow(dead_code)]
#[path = "../build.rs"]
mod message_build;

#[cfg(test)]
mod tests {
    use super::*;

    /// `/` is the Commands key, so a separator must never sit beside it, and
    /// footer hint lines use ` | ` between keys (ASCII-safe).
    #[test]
    fn key_hints_never_put_a_separator_next_to_the_slash_key() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut checked = 0;
        for entry in std::fs::read_dir(root.join("messages")).unwrap() {
            let path = entry.unwrap().path();
            let mut id = String::new();
            for line in std::fs::read_to_string(&path).unwrap().lines() {
                if let Some((name, _)) = line.split_once(" =")
                    && !name.starts_with(' ')
                {
                    id = name.to_owned();
                }
                assert!(
                    !line.contains(" / /") && !line.contains("/ / "),
                    "{}: {line}",
                    path.display()
                );
                if id.ends_with("-control") || id.ends_with("-controls") {
                    // `/ Commands` names the key; any other spaced slash separates.
                    let separators = line.replace(" / Commands", "").replace("\"/ Commands", "");
                    assert!(!separators.contains(" / "), "{id}: {line}");
                    checked += 1;
                }
            }
        }
        assert!(checked > 20, "footer hint messages were found");
        for file in ["src/ui.rs", "src/app.rs"] {
            for line in std::fs::read_to_string(root.join(file)).unwrap().lines() {
                assert!(!line.contains(" / /"), "{file}: {line}");
            }
        }
    }

    #[test]
    fn named_untrusted_arguments_are_literal_and_order_independent() {
        let field = "{ $partner }\n\"quoted\" \\ /provider --unsafe <tag> 한글 e\u{301} 👩‍💻";
        let partner = "原本 /provider <identity>";
        let template = Template::CoreEditorTitle;
        let forward = template
            .format(&[("field", field), ("partner", partner)])
            .unwrap();
        let reverse = template
            .format(&[("partner", partner), ("field", field)])
            .unwrap();
        assert_eq!(forward, reverse);
        assert!(forward.contains(field));
        assert!(forward.contains(partner));
    }

    #[test]
    fn invalid_named_arguments_report_the_exact_key_and_failure() {
        let template = Template::CoreEditorTitle;
        assert!(matches!(
            template.format(&[("partner", "identity")]),
            Err(Error::MissingArgument { message, name })
                if message == "core-editor-title" && name == "field"
        ));
        assert!(matches!(
            template.format(&[("field", "text"), ("partner", "identity"), ("other", "x")]),
            Err(Error::UnknownArgument { name, .. }) if name == "other"
        ));
        assert!(matches!(
            template.format(&[("field", "first"), ("field", "second")]),
            Err(Error::DuplicateArgument { name, .. }) if name == "field"
        ));
        for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(matches!(
                Template::RecordsPosition.format_values(&[
                    ("index", Argument::Number(number)),
                    ("count", Argument::Number(1.0)),
                ]),
                Err(Error::InvalidNumber { message, name })
                    if message == "records-position" && name == "index"
            ));
        }
    }

    #[test]
    fn numeric_arguments_preserve_finite_values_without_touching_literal_text() {
        let literal = "{ $index } 한글";
        for number in [0.0, -1.5, f64::MAX, -f64::from_bits(1)] {
            let formatted = Template::RecordsPosition
                .format_values(&[
                    ("index", Argument::Number(number)),
                    ("count", Argument::Text(literal)),
                ])
                .unwrap();
            assert!(formatted.contains(&number.to_string()));
            assert!(formatted.contains(literal));
        }
    }
}
