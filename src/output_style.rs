//! Display-only byte ranges. Text, evidence, permissions and provider input are unchanged.
use std::{ops::Range, sync::LazyLock};

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Function,
    Fence,
    Error,
    Warning,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: Kind,
}

// Presentation work is bounded independently of retained output. Long/unsupported
// code remains literal, not truncated or interpreted as terminal escape sequences.
const MAX_BYTES: usize = 128 * 1024;
const MAX_LINE: usize = 4096;
const MAX_TOKENS: usize = 8192;

fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
    &SYNTAXES
}

fn scope_kind(stack: &ScopeStack) -> Option<Kind> {
    static SCOPES: LazyLock<[(Scope, Kind); 9]> = LazyLock::new(|| {
        [
            ("comment", Kind::Comment),
            ("string", Kind::String),
            ("constant.numeric", Kind::Number),
            ("constant.language", Kind::Keyword),
            ("keyword", Kind::Keyword),
            ("storage", Kind::Keyword),
            ("entity.name.type", Kind::Type),
            ("support.type", Kind::Type),
            ("entity.name.function", Kind::Function),
        ]
        .map(|(scope, kind)| (Scope::new(scope).expect("static syntax scope"), kind))
    });
    // Enclosing strings/comments win over embedded words such as `error` or `fn`.
    for &(prefix, kind) in SCOPES.iter() {
        if stack
            .as_slice()
            .iter()
            .any(|scope| prefix.is_prefix_of(*scope))
        {
            return Some(kind);
        }
    }
    None
}

fn push(tokens: &mut Vec<Token>, range: Range<usize>, kind: Option<Kind>) {
    if range.is_empty() || tokens.len() >= MAX_TOKENS {
        return;
    }
    if let Some(kind) = kind {
        if let Some(last) = tokens.last_mut()
            && last.kind == kind
            && last.range.end == range.start
        {
            last.range.end = range.end;
        } else {
            tokens.push(Token { range, kind });
        }
    }
}

fn starts(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|part| part.eq_ignore_ascii_case(prefix))
}

/// Conservative log labels, not a verdict about a command or a piece of code.
pub fn diagnostic(line: &str) -> Option<Kind> {
    let line = line.trim_start();
    let line = line.strip_prefix("[stderr] ").unwrap_or(line);
    if [
        "error:",
        "error[",
        "fatal:",
        "fatal error:",
        "panic:",
        "traceback (most recent call last):",
        "오류:",
    ]
    .iter()
    .any(|prefix| starts(line, prefix))
        || line.starts_with("thread '") && line.contains(" panicked at ")
        || line.split_once(':').is_some_and(|(name, _)| {
            !name.is_empty() && !name.contains(char::is_whitespace) && name.ends_with("Error")
        })
    {
        return Some(Kind::Error);
    }
    if ["warning:", "warning[", "warn:", "경고:"]
        .iter()
        .any(|prefix| starts(line, prefix))
    {
        return Some(Kind::Warning);
    }
    // Compiler diagnostics with an actual numeric source location before severity.
    for (label, kind) in [(": error", Kind::Error), (": warning", Kind::Warning)] {
        if let Some((location, after)) = line.split_once(label)
            && location.rsplit(':').next().is_some_and(|column| {
                !column.is_empty() && column.bytes().all(|byte| byte.is_ascii_digit())
            })
            && after.starts_with([':', '[', ' '])
        {
            return Some(kind);
        }
    }
    None
}

struct Code {
    parser: ParseState,
    scopes: ScopeStack,
}

impl Code {
    fn new(language: &str) -> Option<Self> {
        let language = match language {
            "sh" | "shell" => "bash",
            "py" => "python",
            "js" => "javascript",
            "rs" => "rust",
            other => other,
        };
        if language.is_empty() {
            return None;
        }
        let set = syntaxes();
        let syntax = set.find_syntax_by_token(language)?;
        Some(Self {
            parser: ParseState::new(syntax),
            scopes: ScopeStack::new(),
        })
    }

    fn line(&mut self, tokens: &mut Vec<Token>, line: &str, offset: usize) -> bool {
        if line.len() > MAX_LINE {
            return false;
        }
        let Ok(operations) = self.parser.parse_line(line, syntaxes()) else {
            return false;
        };
        let mut start = 0;
        for (end, op) in operations {
            push(
                tokens,
                offset + start..offset + end,
                scope_kind(&self.scopes),
            );
            if self.scopes.apply(&op).is_err() {
                return false;
            }
            start = end;
        }
        push(
            tokens,
            offset + start..offset + line.len(),
            scope_kind(&self.scopes),
        );
        true
    }
}

/// Classify a changed observation once, before wrapping. `file` means the first
/// line is a typed file path and numeric gutters are presentation, not source.
pub fn classify(tokens: &mut Vec<Token>, text: &str, file: bool) {
    tokens.clear();
    let mut lines = text.split_inclusive('\n');
    let mut offset = 0;
    let mut code = None;
    if file && let Some(path) = lines.next() {
        offset = path.len();
        let extension = std::path::Path::new(path.trim())
            .extension()
            .and_then(|v| v.to_str());
        code = extension.and_then(Code::new);
    }
    let mut fence: Option<(u8, usize)> = None;
    let mut log = !file;
    for line in lines {
        if offset >= MAX_BYTES || tokens.len() >= MAX_TOKENS {
            break;
        }
        let body = line.trim_end_matches(['\r', '\n']);
        if !file {
            let trimmed = body.trim_start_matches(' ');
            let indent = body.len() - trimmed.len();
            let marker = trimmed.as_bytes().first().copied();
            if indent <= 3 && matches!(marker, Some(b'`' | b'~')) {
                let marker = marker.unwrap();
                let count = trimmed.bytes().take_while(|byte| *byte == marker).count();
                let rest = &trimmed[count..];
                let closing = fence.is_some_and(|(open, length)| {
                    marker == open && count >= length && rest.trim().is_empty()
                });
                if closing || fence.is_none() && count >= 3 {
                    push(tokens, offset..offset + body.len(), Some(Kind::Fence));
                    if closing {
                        fence = None;
                        code = None;
                        log = true;
                    } else {
                        fence = Some((marker, count));
                        let language = rest.split_whitespace().next().unwrap_or("");
                        code = Code::new(language);
                        log = matches!(
                            language,
                            "" | "text" | "console" | "log" | "output" | "stderr" | "stdout"
                        );
                    }
                    offset += line.len();
                    continue;
                }
            }
        }
        let source = if file {
            line.split_once(" │ ")
                .or_else(|| line.split_once(" | "))
                .filter(|(number, _)| {
                    !number.trim().is_empty()
                        && number.trim().bytes().all(|byte| byte.is_ascii_digit())
                })
                .map_or(line, |(_, code)| code)
        } else {
            line
        };
        if let Some(parser) = &mut code {
            if !parser.line(tokens, source, offset + line.len() - source.len()) {
                // A long/unparseable line invalidates multiline state; remain
                // plain until the next independently declared code block.
                code = None;
            }
        } else if log {
            push(tokens, offset..offset + body.len(), diagnostic(body));
        }
        offset += line.len();
    }
    // A syntax scope can end between a base glyph and its combining mark.
    // Extend to whole graphemes, choosing the scope at each cluster's start.
    // The monotonic iterator reuses the range buffer without another allocation.
    let mut ends = text
        .grapheme_indices(true)
        .map(|(at, glyph)| at + glyph.len());
    let mut boundary = 0;
    tokens.retain_mut(|token| {
        while boundary < token.range.start {
            boundary = ends.next().unwrap_or(text.len());
        }
        token.range.start = boundary;
        while boundary < token.range.end {
            boundary = ends.next().unwrap_or(text.len());
        }
        token.range.end = boundary;
        token.range.start < token.range.end
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind_at(tokens: &[Token], text: &str, needle: &str) -> Option<Kind> {
        let offset = text.find(needle).unwrap();
        tokens
            .iter()
            .find(|token| token.range.contains(&offset))
            .map(|token| token.kind)
    }

    #[test]
    fn fenced_code_preserves_unicode_and_distinguishes_strings_comments_and_keywords() {
        let text = "설명\n```rust\nfn 인사() {\n    let 값 = \"error: 한글\"; // warning: not a diagnostic\n}\n```\nerror[E0308]: mismatched types\nwarning: unused value\n";
        let mut tokens = Vec::new();
        classify(&mut tokens, text, false);
        assert_eq!(kind_at(&tokens, text, "fn"), Some(Kind::Keyword));
        assert_eq!(kind_at(&tokens, text, "error: 한글"), Some(Kind::String));
        assert_eq!(kind_at(&tokens, text, "warning: not"), Some(Kind::Comment));
        assert_eq!(kind_at(&tokens, text, "error[E"), Some(Kind::Error));
        assert_eq!(
            kind_at(&tokens, text, "warning: unused"),
            Some(Kind::Warning)
        );
        assert!(
            tokens
                .iter()
                .all(|token| text.get(token.range.clone()).is_some())
        );
    }

    #[test]
    fn multiline_state_survives_wrapping_and_shorter_or_other_fences_do_not_close_it() {
        let text = "````python\nvalue = \"\"\"first\n```\nerror: still a string\n\"\"\"\n~~~\n````\nwarning: outside\n";
        let mut tokens = Vec::new();
        classify(&mut tokens, text, false);
        assert_eq!(kind_at(&tokens, text, "error:"), Some(Kind::String));
        assert_eq!(kind_at(&tokens, text, "warning:"), Some(Kind::Warning));
    }

    #[test]
    fn numbered_file_gutters_are_not_parsed_as_source() {
        let text = "src/한글.rs\r\n   1 │ fn main() {\r\n   2 │     let x = 42; // literal\r\n   3 │ }\r\n";
        let mut tokens = Vec::new();
        classify(&mut tokens, text, true);
        assert_eq!(kind_at(&tokens, text, "fn"), Some(Kind::Keyword));
        assert_eq!(kind_at(&tokens, text, "42"), Some(Kind::Number));
        assert_eq!(kind_at(&tokens, text, "   2"), None);
    }

    #[test]
    fn diagnostic_labels_do_not_turn_prose_or_success_counts_into_errors() {
        for text in [
            "src/한글.rs:12:4: error[E0308]: type mismatch",
            "ValueError: bad input",
            "ERROR: failed",
        ] {
            assert_eq!(diagnostic(text), Some(Kind::Error), "{text}");
        }
        assert_eq!(
            diagnostic("src/lib.rs:3: warning: unused"),
            Some(Kind::Warning)
        );
        for text in [
            "There are 0 errors",
            "error handling is useful",
            "let error = 3;",
            "https://host: error docs",
        ] {
            assert_eq!(diagnostic(text), None, "{text}");
        }
    }

    #[test]
    fn thread_lifecycle_is_not_a_diagnostic() {
        assert_eq!(diagnostic("thread 'worker' started successfully"), None);
        assert_eq!(
            diagnostic("thread 'worker' panicked at src/main.rs:1:1:"),
            Some(Kind::Error)
        );
    }

    #[test]
    fn changed_stream_and_unknown_language_do_not_keep_old_styles() {
        let mut tokens = Vec::new();
        classify(&mut tokens, "```rust\nlet x = \"partial", false);
        classify(
            &mut tokens,
            "```rust\nlet x = \"partial end\";\n```\nplain",
            false,
        );
        assert_eq!(
            kind_at(
                &tokens,
                "```rust\nlet x = \"partial end\";\n```\nplain",
                "plain"
            ),
            None
        );
        let unknown = "```not-a-language\nerror: literal unknown source\n```";
        classify(&mut tokens, unknown, false);
        assert_eq!(kind_at(&tokens, unknown, "error:"), None);
    }
}
