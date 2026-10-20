//! Locations are untrusted references parsed from already-retained output, never filesystem reads.
use std::fmt;

pub const MAX_LOCATIONS: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub path: String,
    pub line: usize,
    pub column: Option<usize>,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.path, self.line)?;
        if let Some(column) = self.column {
            write!(f, ":{column}")?;
        }
        Ok(())
    }
}

/// Stable first-occurrence order; bounds apply across the complete supplied window.
pub fn parse<'a>(lines: impl IntoIterator<Item = &'a str>) -> Vec<Location> {
    let mut found = Vec::new();
    for line in lines {
        if let Some(location) = python(line) {
            push(&mut found, location);
        }
        for (colon, _) in line.match_indices(':') {
            let tail = &line[colon + 1..];
            let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 {
                continue;
            }
            let Some(number) = positive(&tail[..digits]) else {
                continue;
            };
            // A second numeric component belongs to the previous path:line pair.
            let prefix = &line[..colon];
            if prefix.rsplit_once(':').is_some_and(|(_, suffix)| {
                !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit())
            }) {
                continue;
            }
            let rest = &tail[digits..];
            let column = rest.strip_prefix(':').and_then(|text| {
                let n = text.bytes().take_while(u8::is_ascii_digit).count();
                positive(&text[..n])
            });
            if let Some(path) = path_before(prefix) {
                push(
                    &mut found,
                    Location {
                        path: path.into(),
                        line: number,
                        column,
                    },
                );
            }
        }
        // TypeScript's diagnostic form: src/file.ts(12,3): error TS…
        for (open, _) in line.match_indices('(') {
            let Some((coords, _)) = line[open + 1..].split_once(')') else {
                continue;
            };
            let Some((row, col)) = coords.split_once(',') else {
                continue;
            };
            if let (Some(row), Some(col), Some(path)) = (
                positive(row.trim()),
                positive(col.trim()),
                path_before(&line[..open]),
            ) {
                push(
                    &mut found,
                    Location {
                        path: path.into(),
                        line: row,
                        column: Some(col),
                    },
                );
            }
        }
        if found.len() == MAX_LOCATIONS {
            break;
        }
    }
    found
}

fn positive(value: &str) -> Option<usize> {
    value.parse::<usize>().ok().filter(|n| *n > 0)
}

fn push(found: &mut Vec<Location>, location: Location) {
    if found.len() < MAX_LOCATIONS && !found.contains(&location) {
        found.push(location);
    }
}

fn python(line: &str) -> Option<Location> {
    let (_, rest) = line.split_once("File \"")?;
    let (path, rest) = rest.split_once("\", line ")?;
    let count = rest.bytes().take_while(u8::is_ascii_digit).count();
    valid_path(path).then_some(())?;
    Some(Location {
        path: path.into(),
        line: positive(&rest[..count])?,
        column: None,
    })
}

fn path_before(prefix: &str) -> Option<&str> {
    let quoted = prefix.ends_with(['\'', '"']);
    let prefix = prefix.trim_end_matches(['\'', '"']);
    let path = if quoted {
        prefix.rsplit_once(['\'', '"'])?.1
    } else if let Some((_, parenthesized)) = prefix
        .rsplit_once('(')
        .filter(|(_, suffix)| !suffix.contains(')'))
    {
        parenthesized
    } else {
        prefix.rsplit([' ', '\t', '(', '[', '<', '>']).next()?
    };
    valid_path(path).then_some(path)
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 32 * 1024
        && !path.chars().any(char::is_control)
        && !path.contains("://")
        && path != "<anonymous>"
        && (path.contains(['/', '\\'])
            || (path.starts_with('.') && path.len() > 1)
            || path
                .rsplit_once('.')
                .is_some_and(|(base, ext)| !base.is_empty() && !ext.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_traceback_and_runtime_locations_keep_exact_coordinates() {
        let cases = [
            ("  --> src/main.rs:17:9", "src/main.rs", 17, Some(9)),
            (
                "thread 'actual_failure' (5894500) panicked at src/lib.rs:3:5",
                "src/lib.rs",
                3,
                Some(5),
            ),
            (
                "    at async run (/tmp/project space/file.js:42:6)",
                "/tmp/project space/file.js",
                42,
                Some(6),
            ),
            (
                "thread 'tests::bad' panicked at src/lib.rs:25:5:",
                "src/lib.rs",
                25,
                Some(5),
            ),
            (
                "  File \"/tmp/project space/test_bad.py\", line 8, in test_bad",
                "/tmp/project space/test_bad.py",
                8,
                None,
            ),
            ("test_bad.py:8: AssertionError", "test_bad.py", 8, None),
            (
                "    at async run (/tmp/project/file.js:42:6)",
                "/tmp/project/file.js",
                42,
                Some(6),
            ),
            (
                "src/widget.ts(11,4): error TS2322",
                "src/widget.ts",
                11,
                Some(4),
            ),
            (
                "C:\\project\\main.ts:9:2",
                "C:\\project\\main.ts",
                9,
                Some(2),
            ),
            (
                "\"source space/東京.rs\":3:7",
                "source space/東京.rs",
                3,
                Some(7),
            ),
        ];
        for (text, path, line, column) in cases {
            assert_eq!(
                parse([text]),
                vec![Location {
                    path: path.into(),
                    line,
                    column
                }],
                "{text}"
            );
        }
    }

    #[test]
    fn rejects_nonlocations_and_zero_or_overflow_coordinates() {
        for text in [
            "12:30",
            "error:101",
            "https://localhost:8080",
            "file.rs:0",
            "file.rs:9999999999999999999999999999999",
            "nothing here",
        ] {
            assert!(parse([text]).is_empty(), "{text}");
        }
    }

    #[test]
    fn deduplicates_without_reordering_and_bounds_untrusted_output() {
        let lines: Vec<_> = (1..200).map(|n| format!("src/lib.rs:{n}:2")).collect();
        let found = parse(lines.iter().map(String::as_str).chain(["src/lib.rs:1:2"]));
        assert_eq!(found.len(), MAX_LOCATIONS);
        assert_eq!(found.first().unwrap().line, 1);
        assert_eq!(found.last().unwrap().line, MAX_LOCATIONS);
        assert_eq!(parse(["x.py:4", "x.py:4"]).len(), 1);
    }
}
