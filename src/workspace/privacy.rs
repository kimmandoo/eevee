//! Best-effort credential redaction, not a guarantee of detection or secure erasure.
use std::borrow::Cow;

const REPLACEMENT: &str = "[REDACTED]";

/// Redacts a cumulative response snapshot, withholding only its unfinished token.
///
/// Never call this with isolated deltas: quoted values, bearer credentials and PEM
/// bodies need their preceding context. Completion releases the final token through
/// the same redactor. An unfinished PEM header is held even across whitespace,
/// because a later `PRIVATE KEY-----` can classify the entire header as secret.
pub fn redact_stream(text: &str, complete: bool) -> Cow<'_, str> {
    redact_stream_from(text, complete, 0)
}

/// Projects a chronological segment while retaining all earlier credential
/// context. `from` is a trusted UTF-8 boundary in the cumulative response.
pub(crate) fn redact_stream_from(text: &str, complete: bool, from: usize) -> Cow<'_, str> {
    let mut end = text.len();
    if !complete {
        // Unicode whitespace can still be URL userinfo before @.
        end = text.rfind([' ', '\t', '\n']).map_or(0, |index| index + 1);
        let prefix = &text[..end];
        for (start, rest) in prefix.match_indices("-----BEGIN ") {
            if !prefix[start + rest.len()..].contains("-----") {
                end = start;
                break;
            }
        }
    }
    redact_from(&text[..end], from.min(end))
}

/// Redacts recognizable credentials without allocating for unchanged text.
pub fn redact(text: &str) -> Cow<'_, str> {
    redact_from(text, 0)
}

fn redact_from(text: &str, from: usize) -> Cow<'_, str> {
    let mut ranges = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &text[i..];
        if let Some(end) = private_key_end(rest) {
            ranges.push((i, i + end));
            i += end;
            continue;
        }
        let prefix = [
            "sk-",
            "ghp_",
            "gho_",
            "ghu_",
            "ghs_",
            "ghr_",
            "github_pat_",
            "xoxb-",
            "xoxp-",
            "xoxa-",
            "xoxr-",
            "AKIA",
            "ASIA",
            "AIza",
        ]
        .iter()
        .find(|prefix| rest.starts_with(**prefix));
        if let Some(prefix) = prefix
            && (i == 0 || !is_name(bytes[i - 1]))
        {
            let end = token_end(text, i);
            if end >= i + prefix.len() + 4 {
                ranges.push((i, end));
                i = end;
                continue;
            }
        }
        if rest.len() >= 7
            && rest.as_bytes()[..6].eq_ignore_ascii_case(b"bearer")
            && matches!(bytes[i + 6], b' ' | b'\t')
        {
            let start = skip_space(bytes, i + 6);
            let end = token_end(text, start);
            if end > start && &text[start..end] != REPLACEMENT {
                ranges.push((start, end));
                i = end;
                continue;
            }
        }
        if bytes[i].is_ascii_alphabetic() && (i == 0 || !is_name(bytes[i - 1])) {
            let end = (i..bytes.len())
                .find(|&j| !is_name(bytes[j]))
                .unwrap_or(bytes.len());
            let name = &text[i..end];
            let secret = [
                "password",
                "passwd",
                "pwd",
                "secret",
                "token",
                "api_key",
                "apikey",
                "api-key",
                "access_key",
                "private_key",
                "client_secret",
                "authorization",
            ]
            .iter()
            .any(|key| {
                name.eq_ignore_ascii_case(key)
                    || name.len() > key.len()
                        && name.as_bytes()[name.len() - key.len() - 1] == b'_'
                        && name[name.len() - key.len()..].eq_ignore_ascii_case(key)
            });
            let mut value = skip_space(bytes, end);
            if value < bytes.len() && matches!(bytes[value], b'\'' | b'"') {
                value = skip_space(bytes, value + 1);
            }
            if secret && value < bytes.len() && matches!(bytes[value], b'=' | b':') {
                value = skip_space(bytes, value + 1);
                let end = if value < bytes.len() && matches!(bytes[value], b'\'' | b'"') {
                    let quote = bytes[value];
                    value += 1;
                    quoted_end(bytes, value, quote)
                } else if bytes[value..].len() >= 7
                    && bytes[value..value + 6].eq_ignore_ascii_case(b"bearer")
                    && matches!(bytes[value + 6], b' ' | b'\t')
                {
                    token_end(text, skip_space(bytes, value + 6))
                } else if let Some(end) = private_key_end(&text[value..]) {
                    value + end
                } else {
                    token_end(text, value)
                };
                if end > value && &text[value..end] != REPLACEMENT {
                    ranges.push((value, end));
                    i = end;
                    continue;
                }
            }
        }
        if rest.starts_with("://") {
            let start = i + 3;
            let end = (start..bytes.len())
                .find(|&j| matches!(bytes[j], b'/' | b'?' | b'#') || bytes[j].is_ascii_whitespace())
                .unwrap_or(bytes.len());
            if let Some(at) = text[start..end].rfind('@') {
                if at > 0 && &text[start..start + at] != REPLACEMENT {
                    ranges.push((start, start + at));
                }
                i = end;
                continue;
            }
        }
        i += text[i..].chars().next().unwrap().len_utf8();
    }
    if ranges.iter().all(|&(_, end)| end <= from) {
        return Cow::Borrowed(&text[from..]);
    }
    let mut output = String::with_capacity(text.len() - from);
    let mut previous = from;
    for (start, end) in ranges {
        if end <= previous {
            continue;
        }
        output.push_str(&text[previous..start.max(previous)]);
        output.push_str(REPLACEMENT);
        previous = end;
    }
    output.push_str(&text[previous..]);
    Cow::Owned(output)
}

fn private_key_end(text: &str) -> Option<usize> {
    let header = text.strip_prefix("-----BEGIN ")?;
    let header_end = header.find("-----")?;
    if !header[..header_end].contains("PRIVATE KEY") {
        return None;
    }
    Some(
        text.find("-----END ")
            .and_then(|end| text[end + 9..].find("-----").map(|tail| end + 9 + tail + 5))
            .unwrap_or(text.len()),
    )
}

fn quoted_end(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == quote {
            return i;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
        }
        i += 1;
    }
    bytes.len()
}

fn is_name(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}
fn skip_space(bytes: &[u8], start: usize) -> usize {
    (start..bytes.len())
        .find(|&i| !matches!(bytes[i], b' ' | b'\t'))
        .unwrap_or(bytes.len())
}
fn token_end(text: &str, start: usize) -> usize {
    text[start..]
        .char_indices()
        .find(|(_, c)| {
            c.is_whitespace() || matches!(c, '\'' | '"' | ',' | ';' | '<' | '>' | ')' | '}')
        })
        .map_or(text.len(), |(offset, _)| start + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stream_split_keeps_recognizable_credentials_private() {
        for secret in [
            "sk-secret123456",
            "ghp_secret123456",
            "gho_secret123456",
            "ghu_secret123456",
            "ghs_secret123456",
            "ghr_secret123456",
            "github_pat_secret123456",
            "xoxb-secret123456",
            "xoxp-secret123456",
            "xoxa-secret123456",
            "xoxr-secret123456",
            "AKIA1234567890123456",
            "ASIA1234567890123456",
            "AIza1234567890123456",
            "Bearer opaque-value",
            "Bearer\topaque-value",
            "Authorization: Bearer opaque-value",
            "password=opaque-value",
            "\"api_key\" : \"opaque value with spaces\"",
            "CLIENT_SECRET='opaque value'",
            "password=\"opaque\\\" value\"",
            "password=\"秘密 値 with spaces\"",
            "password=\"opaque\nmultiline value\"",
            "https://user:opaque-value@example.test/path",
            "https://user:opaque\u{3000}value@example.test/path",
            "-----BEGIN RSA PRIVATE KEY-----\nopaque value\n-----END RSA PRIVATE KEY-----",
            "private_key=-----BEGIN PRIVATE KEY-----\nopaque-value\n-----END PRIVATE KEY-----",
            "-----BEGIN EC PRIVATE KEY-----\nopaque-value",
        ] {
            let text = format!("前 ordinary prose {secret} 後 more words");
            let final_text = redact(&text);
            for split in text
                .char_indices()
                .map(|(index, _)| index)
                .chain([text.len()])
            {
                let mut cumulative = text[..split].to_owned();
                let visible = redact_stream(&cumulative, false);
                assert!(
                    final_text.starts_with(visible.as_ref()),
                    "split {split} of {secret:?}: {visible:?} is not a safe final prefix {final_text:?}",
                );
                cumulative.push_str(&text[split..]);
                assert_eq!(redact_stream(&cumulative, true), final_text);
            }
        }
    }

    #[test]
    fn no_newline_prose_releases_multiple_borrowed_prefixes_before_completion() {
        let text = "前 ordinary words 日本語 more prose without newline";
        let mut previous = "";
        let mut updates = 0;
        for end in text
            .char_indices()
            .map(|(index, _)| index)
            .chain([text.len()])
        {
            let visible = redact_stream(&text[..end], false);
            assert!(matches!(visible, Cow::Borrowed(_)));
            assert!(text.starts_with(visible.as_ref()));
            if visible != previous {
                updates += 1;
                previous = &text[..visible.len()];
            }
        }
        assert!(updates >= 7);
        assert_eq!(previous, "前 ordinary words 日本語 more prose without ");
        assert_eq!(redact_stream(text, true), text);
    }

    #[test]
    fn unfinished_pem_headers_wait_but_nonprivate_headers_resume() {
        assert_eq!(redact_stream("前 -----BEGIN RSA ", false), "前 ");
        assert_eq!(
            redact_stream("前 -----BEGIN RSA PRIVATE KEY-----", false),
            "前 "
        );
        assert_eq!(
            redact_stream("前 -----BEGIN RSA PRIVATE KEY-----\nopaque ", false),
            "前 [REDACTED]",
        );
        assert_eq!(
            redact_stream("前 -----BEGIN CERTIFICATE-----\npublic ", false),
            "前 -----BEGIN CERTIFICATE-----\npublic ",
        );
    }

    #[test]
    fn recognizable_credentials_are_removed_without_losing_unicode() {
        for value in [
            "sk-secret123456",
            "ghp_secret123456",
            "AKIA1234567890123456",
            "Bearer opaque-value",
            "Bearer\topaque-value",
            "Authorization: Bearer opaque-value",
            "password=opaque-value",
            "\"api_key\": \"opaque-value\"",
            "CLIENT_SECRET='opaque value'",
            "https://user:opaque-value@example.test/path",
            "-----BEGIN RSA PRIVATE KEY-----\nopaque-value\n-----END RSA PRIVATE KEY-----",
            "private_key=-----BEGIN PRIVATE KEY-----\nopaque-value\n-----END PRIVATE KEY-----",
            "password=\"opaque\\\"value\"",
        ] {
            let input = format!("前 {value} 後");
            let output = redact(&input);
            assert!(output.contains(REPLACEMENT), "{value}");
            assert!(!output.contains("opaque"));
            assert!(!output.contains("secret123456"));
            assert!(output.starts_with("前 ") && output.ends_with(" 後"));
            assert_eq!(redact(&output), output);
        }
    }

    #[test]
    fn unchanged_text_is_borrowed_and_incomplete_private_key_is_hidden() {
        assert!(matches!(
            redact("普通 goal; tokenizer=visible"),
            Cow::Borrowed(_)
        ));
        assert_eq!(redact("-----BEGIN PRIVATE KEY-----\nsecret"), REPLACEMENT);
    }

    #[test]
    fn token_prefixes_inside_ordinary_words_do_not_destroy_task_text() {
        let text = "task-card task-navigation mask-value /projects/task-workbench";
        assert!(matches!(redact(text), Cow::Borrowed(value) if value == text));
        assert_eq!(redact("key: sk-secret123456"), "key: [REDACTED]");
    }
}
