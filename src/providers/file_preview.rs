//! Display-only file argument decoding. This never validates or authorizes a tool call.
use std::collections::HashSet;

const PATH_LIMIT: usize = 4096;
const CONTENT_LIMIT: usize = crate::providers::writes::MAX_BYTES;
// Edits may also contain a fully escaped old value, which is never retained.
const WIRE_LIMIT: usize = 6 * (2 * CONTENT_LIMIT + PATH_LIMIT) + 4096;
const DISPLAY_LIMIT: usize = 16 * 1024;
const TAIL_LINES: usize = 12;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Ignore,
    Key,
    Path,
    Content,
}
#[derive(Clone, Copy)]
enum Container {
    Object(ObjectState),
    Array(ArrayState),
}
#[derive(Clone, Copy)]
enum ObjectState {
    FirstKey,
    Key,
    Colon,
    Value,
    Comma,
}
#[derive(Clone, Copy)]
enum ArrayState {
    FirstValue,
    Value,
    Comma,
}
#[derive(Clone, Copy)]
enum Escape {
    None,
    Slash,
    Unicode(u16, u8),
    LowSlash(u16),
    LowU(u16),
    LowUnicode(u16, u16, u8),
}
struct Quoted {
    field: Field,
    escape: Escape,
}

/// Incrementally decodes just the public path and proposed replacement string.
/// Unknown/nested fields are consumed, not retained. All retained state is bounded.
pub(crate) struct Stream {
    edit: bool,
    stack: Vec<Container>,
    started: bool,
    done: bool,
    invalid: bool,
    bytes: usize,
    quoted: Option<Quoted>,
    primitive: String,
    key: String,
    keys: Vec<HashSet<String>>,
    field: Field,
    path: String,
    path_done: bool,
    content: String,
    content_seen: bool,
    line_revision: usize,
}
impl Stream {
    pub(crate) fn new(edit: bool) -> Self {
        Self {
            edit,
            stack: Vec::new(),
            started: false,
            done: false,
            invalid: false,
            bytes: 0,
            quoted: None,
            primitive: String::new(),
            key: String::new(),
            keys: Vec::new(),
            field: Field::Ignore,
            path: String::new(),
            path_done: false,
            content: String::new(),
            content_seen: false,
            line_revision: 0,
        }
    }
    pub(crate) fn push(&mut self, fragment: &str) {
        if self.invalid {
            return;
        }
        self.bytes = self.bytes.saturating_add(fragment.len());
        if self.bytes > WIRE_LIMIT {
            self.invalid = true;
            return;
        }
        for ch in fragment.chars() {
            if self.invalid {
                break;
            }
            self.character(ch);
        }
    }
    pub(crate) fn text(&self, complete: bool) -> Option<String> {
        if self.invalid
            || !self.path_done
            || self.path.trim().is_empty()
            || (complete && (!self.done || !self.content_seen))
        {
            return None;
        }
        Some(display(&self.path, &self.content, complete && self.done))
    }
    /// A decoded line boundary can be published immediately without reparsing
    /// or repeatedly rendering every byte of an unfinished code token.
    pub(super) fn line_revision(&self) -> usize {
        self.line_revision
    }
    fn character(&mut self, ch: char) {
        if let Some(mut quoted) = self.quoted.take() {
            let mut decoded = None;
            let mut closed = false;
            quoted.escape = match quoted.escape {
                Escape::None => match ch {
                    '"' => {
                        closed = true;
                        Escape::None
                    }
                    '\\' => Escape::Slash,
                    c if c <= '\u{001f}' => {
                        self.invalid = true;
                        Escape::None
                    }
                    c => {
                        decoded = Some(c);
                        Escape::None
                    }
                },
                Escape::Slash => match ch {
                    '"' | '\\' | '/' => {
                        decoded = Some(ch);
                        Escape::None
                    }
                    'b' => {
                        decoded = Some('\u{0008}');
                        Escape::None
                    }
                    'f' => {
                        decoded = Some('\u{000c}');
                        Escape::None
                    }
                    'n' => {
                        decoded = Some('\n');
                        Escape::None
                    }
                    'r' => {
                        decoded = Some('\r');
                        Escape::None
                    }
                    't' => {
                        decoded = Some('\t');
                        Escape::None
                    }
                    'u' => Escape::Unicode(0, 0),
                    _ => {
                        self.invalid = true;
                        Escape::None
                    }
                },
                Escape::Unicode(value, digits) => {
                    let Some(digit) = ch.to_digit(16) else {
                        self.invalid = true;
                        return;
                    };
                    let value = (value << 4) | digit as u16;
                    if digits < 3 {
                        Escape::Unicode(value, digits + 1)
                    } else if (0xd800..=0xdbff).contains(&value) {
                        Escape::LowSlash(value)
                    } else {
                        decoded = char::from_u32(value as u32);
                        if decoded.is_none() {
                            self.invalid = true;
                        }
                        Escape::None
                    }
                }
                Escape::LowSlash(high) => {
                    if ch != '\\' {
                        self.invalid = true;
                    }
                    Escape::LowU(high)
                }
                Escape::LowU(high) => {
                    if ch != 'u' {
                        self.invalid = true;
                    }
                    Escape::LowUnicode(high, 0, 0)
                }
                Escape::LowUnicode(high, value, digits) => {
                    let Some(digit) = ch.to_digit(16) else {
                        self.invalid = true;
                        return;
                    };
                    let value = (value << 4) | digit as u16;
                    if digits < 3 {
                        Escape::LowUnicode(high, value, digits + 1)
                    } else {
                        if !(0xdc00..=0xdfff).contains(&value) {
                            self.invalid = true;
                        } else {
                            decoded = char::from_u32(
                                0x10000
                                    + (((high as u32 - 0xd800) << 10) | (value as u32 - 0xdc00)),
                            );
                        }
                        Escape::None
                    }
                }
            };
            if let Some(ch) = decoded {
                self.decoded(quoted.field, ch);
            }
            if !closed {
                self.quoted = Some(quoted);
            } else if quoted.field == Field::Key {
                let Some(keys) = self.keys.last_mut() else {
                    self.invalid = true;
                    return;
                };
                if keys.len() >= 128 || !keys.insert(self.key.clone()) {
                    self.invalid = true;
                    return;
                }
                if self.stack.len() == 1 {
                    self.field = match self.key.as_str() {
                        "path" | "file_path" => {
                            if self.path_done {
                                self.invalid = true;
                            }
                            Field::Path
                        }
                        "new_string" if self.edit => {
                            if self.content_seen {
                                self.invalid = true;
                            }
                            Field::Content
                        }
                        "content" if !self.edit => {
                            if self.content_seen {
                                self.invalid = true;
                            }
                            Field::Content
                        }
                        _ => Field::Ignore,
                    };
                }
                self.set_object(ObjectState::Colon);
            } else {
                if quoted.field == Field::Path {
                    self.path_done = true;
                }
                if quoted.field == Field::Content {
                    self.content_seen = true;
                }
                self.value_done();
            }
            return;
        }
        if !self.primitive.is_empty() {
            if ch.is_ascii_whitespace() || matches!(ch, ',' | '}' | ']') {
                if serde_json::from_str::<serde_json::Value>(&self.primitive).is_err() {
                    self.invalid = true;
                    return;
                }
                self.primitive.clear();
                self.value_done();
            } else {
                if self.primitive.len() >= 128 {
                    self.invalid = true;
                } else {
                    self.primitive.push(ch);
                }
                return;
            }
        }
        if ch.is_ascii_whitespace() {
            return;
        }
        if !self.started {
            if ch != '{' {
                self.invalid = true;
                return;
            }
            self.started = true;
            self.stack.push(Container::Object(ObjectState::FirstKey));
            self.keys.push(HashSet::new());
            return;
        }
        if self.done {
            self.invalid = true;
            return;
        }
        match self.stack.last().copied() {
            Some(Container::Object(ObjectState::FirstKey | ObjectState::Key)) => {
                if ch == '}'
                    && matches!(
                        self.stack.last(),
                        Some(Container::Object(ObjectState::FirstKey))
                    )
                {
                    self.close();
                } else if ch == '"' {
                    self.key.clear();
                    self.quoted = Some(Quoted {
                        field: Field::Key,
                        escape: Escape::None,
                    });
                } else {
                    self.invalid = true;
                }
            }
            Some(Container::Object(ObjectState::Colon)) => {
                if ch == ':' {
                    self.set_object(ObjectState::Value);
                } else {
                    self.invalid = true;
                }
            }
            Some(Container::Object(ObjectState::Comma)) => match ch {
                ',' => self.set_object(ObjectState::Key),
                '}' => self.close(),
                _ => self.invalid = true,
            },
            Some(Container::Array(ArrayState::Comma)) => match ch {
                ',' => *self.stack.last_mut().unwrap() = Container::Array(ArrayState::Value),
                ']' => self.close(),
                _ => self.invalid = true,
            },
            Some(Container::Array(ArrayState::FirstValue)) if ch == ']' => self.close(),
            Some(Container::Object(ObjectState::Value))
            | Some(Container::Array(ArrayState::FirstValue | ArrayState::Value)) => self.value(ch),
            None => self.invalid = true,
        }
    }
    fn decoded(&mut self, field: Field, ch: char) {
        let (text, limit) = match field {
            Field::Key => (&mut self.key, 256),
            Field::Path => (&mut self.path, PATH_LIMIT),
            Field::Content => (&mut self.content, CONTENT_LIMIT),
            _ => return,
        };
        if text.len() + ch.len_utf8() > limit {
            self.invalid = true;
        } else {
            text.push(ch);
        }
        if field == Field::Content && ch == '\n' {
            self.line_revision += 1;
        }
    }
    fn value(&mut self, ch: char) {
        let field = if self.stack.len() == 1 {
            self.field
        } else {
            Field::Ignore
        };
        if field != Field::Ignore && ch != '"' {
            self.invalid = true;
            return;
        }
        match ch {
            '"' => {
                self.quoted = Some(Quoted {
                    field,
                    escape: Escape::None,
                })
            }
            '{' | '[' => {
                if self.stack.len() >= 64 {
                    self.invalid = true;
                    return;
                }
                self.stack.push(if ch == '{' {
                    Container::Object(ObjectState::FirstKey)
                } else {
                    Container::Array(ArrayState::FirstValue)
                });
                self.keys.push(HashSet::new());
            }
            '-' | '0'..='9' | 't' | 'f' | 'n' => self.primitive.push(ch),
            _ => self.invalid = true,
        }
    }
    fn value_done(&mut self) {
        match self.stack.last_mut() {
            Some(Container::Object(state)) => *state = ObjectState::Comma,
            Some(Container::Array(state)) => *state = ArrayState::Comma,
            None => self.invalid = true,
        }
    }
    fn set_object(&mut self, state: ObjectState) {
        if let Some(container) = self.stack.last_mut() {
            *container = Container::Object(state);
        } else {
            self.invalid = true;
        }
    }
    fn close(&mut self) {
        self.stack.pop();
        self.keys.pop();
        if self.stack.is_empty() {
            self.done = true;
        } else {
            self.value_done();
        }
    }
}

fn normalized(text: &str, complete: bool, path: bool) -> String {
    let redacted = crate::workspace::privacy::redact_stream(text, complete);
    let mut chars = redacted.chars().peekable();
    let mut out = String::with_capacity(redacted.len());
    while let Some(ch) = chars.next() {
        if !path && ch == '\r' && chars.peek() == Some(&'\n') {
            continue;
        }
        out.push(if (ch.is_control() && (path || !matches!(ch, '\n' | '\t')))
            || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
            '\u{fffd}'
        } else { ch });
    }
    out
}
fn prefix(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
fn suffix(text: &str, limit: usize) -> &str {
    let mut start = text.len().saturating_sub(limit);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// Redact the entire cumulative value before choosing a bounded, numbered code tail.
pub(crate) fn display(path: &str, content: &str, complete: bool) -> String {
    let path = normalized(path, true, true);
    let content = normalized(content, complete, false);
    let mut out = prefix(&path, PATH_LIMIT).to_owned();
    let count = content.lines().count();
    let first = count.saturating_sub(TAIL_LINES);
    let lines: Vec<_> = content.lines().skip(first).collect();
    if first > 0 {
        out.push_str(&format!("\n… {first} earlier lines"));
    }
    let overhead = lines.len() * 24;
    let mut remaining = DISPLAY_LIMIT.saturating_sub(out.len() + overhead);
    // Long individual lines share the available budget; never clip before redaction.
    for (offset, line) in lines.iter().enumerate() {
        let budget = remaining / (lines.len() - offset);
        let tail = suffix(line, budget.saturating_sub(4));
        out.push_str(&format!("\n{:>4} │ ", first + offset + 1));
        if tail.len() < line.len() {
            out.push_str("… ");
        }
        out.push_str(tail);
        remaining = remaining.saturating_sub(tail.len() + 4);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_preview_matches_lf_without_hiding_standalone_controls() {
        let source = "value = 1\r\nprint(value)\r\n";
        assert_eq!(
            display("src/main.py", source, true),
            display("src/main.py", "value = 1\nprint(value)\n", true)
        );
        assert!(display("bad\rpath.py", "value\rcontrol", true).contains('\u{fffd}'));
    }

    #[test]
    fn every_fragment_boundary_decodes_escapes_and_surrogate_pairs() {
        let json = r#"{"ignored":{"path":"wrong","content":"wrong"},"path":"src/日本語.rs","content":"one\nquote: \" slash: \\\n\uD840\uDC00\tend\n"}"#;
        for split in json.char_indices().map(|(i, _)| i).chain([json.len()]) {
            let mut stream = Stream::new(false);
            stream.push(&json[..split]);
            stream.push(&json[split..]);
            let text = stream.text(true).unwrap();
            assert!(text.starts_with("src/日本語.rs\n"));
            assert!(text.contains("quote: \" slash: \\"));
            assert!(text.contains("\u{20000}\tend"));
            assert!(!text.contains("wrong"));
        }
        let mut stream = Stream::new(false);
        for ch in json.chars() {
            stream.push(&ch.to_string());
        }
        assert!(stream.text(true).unwrap().contains('\u{20000}'));
    }

    #[test]
    fn decoded_line_boundaries_are_ready_even_when_the_wire_goes_quiet() {
        for escape in [r"\n", r"\u000a", r"\u000A"] {
            let mut stream = Stream::new(false);
            stream.push(r#"{"path":"short.rs","content":"fn short() {}"#);
            assert_eq!(stream.line_revision(), 0);
            stream.push(&escape[..escape.len() - 1]);
            assert_eq!(stream.line_revision(), 0);
            stream.push(&escape[escape.len() - 1..]);
            assert_eq!(stream.line_revision(), 1);
            assert!(stream.text(false).unwrap().contains("fn short() {}"));
        }
    }

    #[test]
    fn sdk_sized_edits_retain_latest_tail_without_retaining_the_old_value() {
        let mut stream = Stream::new(true);
        stream.push(r#"{"file_path":"large.rs","old_string":""#);
        for _ in 0..CONTENT_LIMIT {
            stream.push(r"\u0078");
        }
        stream.push(r#"","new_string":""#);
        stream.push(&"x".repeat(CONTENT_LIMIT - 16));
        stream.push(r#"\nlatest code\n"}"#);
        assert!(stream.text(true).unwrap().contains("latest code"));
    }

    #[test]
    fn invalid_duplicate_alias_rewind_and_nested_values_never_complete() {
        for json in [
            r#"{"path":"a","path":"b","content":"x"}"#,
            r#"{"path":"a","file_path":"b","content":"x"}"#,
            r#"{"path":"a","content":"x",}"#,
            r#"{"path":"a","content":"\uD800x"}"#,
            r#"{"path":"a","content":"\uDC00"}"#,
            r#"{"path":"a","content":{}}"#,
            r#"{"nested":{"path":"a","content":"x"}}"#,
            r#"{"path":"a","content":"x"}{"path":"b"}"#,
            r#"{"path":"a","content":"x","ignored":truth}"#,
            r#"{"path":"a","content":"x","ignored":{"duplicate":1,"duplicate":2}}"#,
        ] {
            let mut stream = Stream::new(false);
            stream.push(json);
            assert!(stream.text(true).is_none(), "{json}");
        }
        let mut edit = Stream::new(true);
        edit.push(r#"{"file_path":"a","old_string":"private old","new_string":"new\n"}"#);
        assert!(edit.text(true).unwrap().contains("new"));
        assert!(!edit.text(true).unwrap().contains("private old"));
    }

    #[test]
    fn privacy_is_cumulative_before_tail_or_byte_clipping() {
        for content in [
            format!("password=\"{} secret tail\"\npublic\n", "line\n".repeat(30)),
            format!(
                "-----BEGIN PRIVATE KEY-----\n{}secret tail",
                "line\n".repeat(30)
            ),
            format!("token=\"{} secret tail\"\npublic\n", "界".repeat(20_000)),
        ] {
            for complete in [false, true] {
                let text = display("safe.rs\n\u{001b}bad", &content, complete);
                assert!(!text.contains("secret tail"));
                assert!(text.len() <= DISPLAY_LIMIT);
                assert!(!text.contains('\u{001b}'));
                assert!(text.lines().next().unwrap().contains('�'));
            }
        }
        let text = display(
            "safe.rs",
            &(0..40).map(|i| format!("line {i}\n")).collect::<String>(),
            true,
        );
        assert!(text.contains("28 earlier lines"));
        assert!(text.contains("  29 │ line 28"));
        assert_eq!(text.lines().filter(|line| line.contains('│')).count(), 12);
    }

    #[test]
    fn interrupted_snapshots_hold_unfinished_secret_tokens() {
        let json = r#"{"path":"safe.rs","content":"visible\npassword=private-tail"#;
        for split in json.char_indices().map(|(i, _)| i).chain([json.len()]) {
            let mut stream = Stream::new(false);
            stream.push(&json[..split]);
            if let Some(text) = stream.text(false) {
                assert!(!text.contains("private"));
            }
            assert!(stream.text(true).is_none());
        }
        let mut stream = Stream::new(false);
        stream.push(&format!(
            r#"{{"path":"a","content":"{}"}}"#,
            "x".repeat(CONTENT_LIMIT + 1)
        ));
        assert!(stream.text(false).is_none());
    }
}
