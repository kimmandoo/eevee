//! GP08: bounded local request-openness rules. Never invokes a model or grants authority.
use super::ModelOption;
use serde::{Deserialize, Serialize};

const MAX_PROSE_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    Trivial,
    Moderate,
    Hard,
}

impl Bucket {
    pub fn label(self) -> &'static str {
        match self {
            Self::Trivial => "trivial",
            Self::Moderate => "moderate",
            Self::Hard => "hard",
        }
    }

    fn target(self) -> u8 {
        match self {
            Self::Trivial => 2,  // low
            Self::Moderate => 4, // high
            Self::Hard => 5,     // xhigh
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    GivenFix,
    BoundedAlternatives,
    OpenCause,
    SubtleInvariants,
    Ultrathink,
    KeepLast,
    ModelDefault,
}

impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Self::GivenFix => "fix or target given",
            Self::BoundedAlternatives => "bounded candidate causes or designs",
            Self::OpenCause => "open cause of flaky, concurrent or stale behavior",
            Self::SubtleInvariants => "subtle correctness invariants",
            Self::Ultrathink => "standalone lowercase prose ultrathink",
            Self::KeepLast => "ambiguous; keep last resolution",
            Self::ModelDefault => "ambiguous; initial model default",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    UserRequest,
    AdoptedActiveStep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Application {
    RequestedNotAcknowledged,
    FixedByProviderNotApplied,
}

impl Application {
    pub fn label(self) -> &'static str {
        match self {
            Self::RequestedNotAcknowledged => "requested; not acknowledged",
            Self::FixedByProviderNotApplied => "effort fixed by provider; not applied",
        }
    }
}

/// Frozen local resolution, not an observation of backend reasoning or usage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolution {
    pub bucket: Option<Bucket>,
    pub resolved: Option<String>,
    pub reason: Reason,
    pub source: Source,
    pub application: Application,
}

impl Resolution {
    pub fn line(&self) -> String {
        format!(
            "Effort: auto → {} / {}{}{} / {}{}",
            self.resolved.as_deref().unwrap_or("provider default"),
            self.bucket.map_or("", Bucket::label),
            if self.bucket.is_some() { ": " } else { "" },
            self.reason.label(),
            self.application.label(),
            if self.source == Source::AdoptedActiveStep {
                " / adopted active step"
            } else {
                ""
            },
        )
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.resolved.as_ref().is_some_and(|value| {
            value.is_empty() || value.len() > 256 || value == "auto"
                || value.chars().any(|ch| ch.is_whitespace() || ch.is_control()
                    || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
                || crate::workspace::privacy::redact(value).as_ref() != value.as_str()
        }) || self.application == Application::FixedByProviderNotApplied && self.resolved.is_some()
            || matches!(self.reason, Reason::GivenFix) && self.bucket != Some(Bucket::Trivial)
            || matches!(self.reason, Reason::BoundedAlternatives) && self.bucket != Some(Bucket::Moderate)
            || matches!(self.reason, Reason::OpenCause | Reason::SubtleInvariants) && self.bucket != Some(Bucket::Hard)
            || matches!(self.reason, Reason::Ultrathink | Reason::KeepLast | Reason::ModelDefault) && self.bucket.is_some()
        {
            return Err("Invalid local automatic-effort receipt".into());
        }
        Ok(())
    }
}

struct Rule {
    bucket: Bucket,
    reason: Reason,
    /// Every group must match; alternatives inside a group are equivalent.
    groups: &'static [&'static [&'static str]],
}

// Fixed order is the tie-lower rule. Work count, politeness and request length
// are deliberately absent. A concrete supplied fix wins over incidental risk nouns.
const RULES: &[Rule] = &[
    Rule {
        bucket: Bucket::Trivial,
        reason: Reason::GivenFix,
        groups: &[&[
            "one fix",
            "fix is given",
            "given fix",
            "fix is:",
            "fix is to",
            "target is",
            "target given",
            "mechanical change",
            "exact replacement",
            "수정안은",
            "해결책은",
            "정해진 수정",
            "주어진 수정",
            "목표는",
            "단순 치환",
        ]],
    },
    Rule {
        bucket: Bucket::Trivial,
        reason: Reason::GivenFix,
        groups: &[
            &["rename", "replace", "이름을", "교체", "바꿔", "치환"],
            &[
                " to ",
                " with ",
                " from ",
                "으로",
                "로 변경",
                "로 바꿔",
                "로 교체",
            ],
        ],
    },
    Rule {
        bucket: Bucket::Trivial,
        reason: Reason::GivenFix,
        groups: &[
            &["implement", "apply", "구현", "적용"],
            &[
                "specified",
                "provided patch",
                "given patch",
                "given design",
                "exact",
                "지정한",
                "주어진 패치",
                "주어진 설계",
                "명시된",
            ],
        ],
    },
    Rule {
        bucket: Bucket::Moderate,
        reason: Reason::BoundedAlternatives,
        groups: &[&[
            "few candidate",
            "two candidate",
            "three candidate",
            "several candidate",
            "two possible",
            "three possible",
            "a few causes",
            "두 후보",
            "세 후보",
            "몇 가지 후보",
            "두 가지 원인",
            "세 가지 원인",
            "몇 가지 원인",
            "후보 원인",
            "후보 설계",
        ]],
    },
    Rule {
        bucket: Bucket::Moderate,
        reason: Reason::BoundedAlternatives,
        groups: &[
            &[
                "choose", "compare", "select", "weigh", "비교", "선택", "검토",
            ],
            &[
                "between",
                "two designs",
                "three designs",
                "options",
                "alternatives",
                "api shapes",
                "error classes",
                "candidate causes",
                "candidate designs",
                "몇 가지",
                "두 가지",
                "세 가지",
                "설계안",
                "대안",
                "중에서",
            ],
        ],
    },
    Rule {
        bucket: Bucket::Hard,
        reason: Reason::OpenCause,
        groups: &[
            &[
                "flaky",
                "intermittent",
                "concurrent",
                "concurrency",
                "race",
                "deadlock",
                "stale",
                "간헐",
                "동시성",
                "경쟁 상태",
                "교착",
                "오래된 상태",
                "가끔",
            ],
            &[
                "cause",
                "why",
                "diagnose",
                "investigate",
                "debug",
                "unknown",
                "find",
                "returns",
                "fails",
                "hangs",
                "원인",
                "왜",
                "진단",
                "조사",
                "추적",
                "찾아",
                "반환",
                "멈춰",
                "실패",
                "발생",
            ],
        ],
    },
    Rule {
        bucket: Bucket::Hard,
        reason: Reason::SubtleInvariants,
        groups: &[
            &[
                "invariant",
                "linearizability",
                "exactly-once",
                "memory ordering",
                "single-flight",
                "불변식",
                "선형화",
                "정확히 한 번",
                "메모리 순서",
                "단일 실행",
            ],
            &[
                "subtle", "easy to", "preserve", "prove", "design", "race", "correct", "미묘",
                "지켜", "보존", "증명", "설계", "경쟁", "정확",
            ],
        ],
    },
];

fn word_char(ch: char) -> bool {
    ch.is_alphanumeric()
        || matches!(
            ch,
            '_' | '-' | '/' | '\\' | '.' | ':' | '@' | '#' | '\'' | '$' | '%' | '='
        )
}

fn right_boundary(tail: &str) -> bool {
    let mut chars = tail.chars();
    let Some(ch) = chars.next() else {
        return true;
    };
    if ch == '.' {
        return chars.find(|ch| *ch != '.').is_none_or(|ch| {
            ch.is_whitespace() || matches!(ch, ')' | ']' | '}' | ',' | ';' | '!' | '?')
        });
    }
    if ch == ':' {
        return chars.next().is_none_or(char::is_whitespace);
    }
    if matches!(ch, '(' | '[' | '{') || ch == '!' && chars.next() == Some('(') {
        return false;
    }
    !word_char(ch)
}

fn contains(prose: &str, term: &str) -> bool {
    prose.match_indices(term).any(|(start, _)| {
        let end = start + term.len();
        // Korean inflections are matched as bounded stems; English words are
        // never substrings of identifiers or other English words.
        (!term.is_ascii()
            || term.starts_with(' ')
            || !prose[..start].ends_with("${")
                && prose[..start]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !word_char(c)))
            && (!term.is_ascii() || term.ends_with(' ') || right_boundary(&prose[end..]))
    })
}

/// Mask code, quotes, blockquoted source and indented code before any rule.
/// Scan at most 16 KiB of current user-authored text, never provider output.
fn prose(text: &str) -> String {
    let mut end = text.len().min(MAX_PROSE_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut result = String::with_capacity(end);
    let mut fence = None;
    let mut quote = None;
    let mut ticks = 0usize;
    for line in text[..end].split_inclusive('\n') {
        let trimmed = line.trim_start();
        let marker = trimmed
            .chars()
            .next()
            .filter(|ch| matches!(*ch, '`' | '~'))
            .map(|ch| (ch, trimmed.chars().take_while(|c| *c == ch).count()))
            .filter(|(_, count)| *count >= 3);
        if let Some((open, length)) = fence {
            if marker.is_some_and(|(ch, count)| ch == open && count >= length) {
                fence = None;
            }
            result.push('\n');
            continue;
        }
        if let Some(marker) = marker {
            fence = Some(marker);
            result.push('\n');
            continue;
        }
        if trimmed.starts_with('>')
            || line.starts_with("    ")
            || line.starts_with('\t')
            || [
                "let ",
                "const ",
                "var ",
                "fn ",
                "pub fn ",
                "def ",
                "class ",
                "function ",
                "import ",
                "use ",
                "//",
                "/*",
                "*/",
            ]
            .iter()
            .any(|prefix| trimmed.starts_with(prefix))
        {
            result.push('\n');
            continue;
        }
        let mut chars = line.char_indices().peekable();
        while let Some((index, ch)) = chars.next() {
            if ch == '`' {
                let mut count = 1;
                while chars.peek().is_some_and(|(_, c)| *c == '`') {
                    chars.next();
                    count += 1;
                }
                if ticks == 0 {
                    ticks = count;
                } else if ticks == count {
                    ticks = 0;
                }
                result.push(' ');
                continue;
            }
            if ticks != 0 {
                continue;
            }
            if let Some(close) = quote {
                if ch == '\\' {
                    chars.next();
                    continue;
                }
                if ch == close {
                    quote = None;
                }
                if ch == '\n' {
                    result.push('\n');
                }
                continue;
            }
            let close = match ch {
                '"' => Some('"'),
                '“' => Some('”'),
                '‘' => Some('’'),
                '«' => Some('»'),
                '「' => Some('」'),
                '『' => Some('』'),
                '＂' => Some('＂'),
                '\'' if !line[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric)
                    || !chars.peek().is_some_and(|(_, c)| c.is_alphanumeric()) =>
                {
                    Some('\'')
                }
                _ => None,
            };
            if let Some(close) = close {
                quote = Some(close);
                result.push(' ');
            } else {
                result.push(ch);
            }
        }
    }
    result
}

/// Exact short follow-ups only; descriptive or explicitly constrained requests
/// never borrow a provider's adopted label as if it were current user intent.
pub fn ambiguous_followup(text: &str) -> bool {
    if text.len() > 128 {
        return false;
    }
    let text = text.trim().trim_end_matches(['.', '!', '?', '。']).trim();
    [
        "continue",
        "next",
        "go on",
        "proceed",
        "keep going",
        "continue please",
        "next step",
        "계속",
        "계속해",
        "계속해줘",
        "계속해 주세요",
        "다음",
        "다음 단계",
        "진행",
        "진행해줘",
    ]
    .iter()
    .any(|candidate| text.eq_ignore_ascii_case(candidate))
}

fn rank(value: &str, index: usize) -> u8 {
    match value {
        "none" => 0,
        "minimal" => 1,
        "low" => 2,
        "medium" => 3,
        "high" => 4,
        "xhigh" => 5,
        "max" => 6,
        _ => index.min(31) as u8,
    }
}

fn mapped(model: &ModelOption, target: u8, top: bool) -> Option<String> {
    // The catalogue is bounded to 32. Stack indices avoid copying/sorting IDs.
    let mut order = [0usize; 32];
    let count = model.efforts.len().min(order.len());
    for (index, slot) in order[..count].iter_mut().enumerate() {
        *slot = index;
    }
    order[..count].sort_unstable_by_key(|index| (rank(&model.efforts[*index], *index), *index));
    if count == 0 {
        return None;
    }
    let ceiling = if top {
        count - 1
    } else {
        count.saturating_sub(2)
    };
    let chosen = if top {
        order[ceiling]
    } else {
        order[..=ceiling]
            .iter()
            .rev()
            .copied()
            .find(|index| rank(&model.efforts[*index], *index) <= target)
            .unwrap_or(order[0])
    };
    Some(model.efforts[chosen].clone())
}

pub fn resolve(
    text: &str,
    active_label: Option<&str>,
    model: Option<&ModelOption>,
    last: Option<&str>,
    controllable: bool,
) -> Resolution {
    let last = last.filter(|value| {
        model.is_some_and(|model| model.efforts.iter().any(|effort| effort.as_str() == *value))
    });
    let source = if ambiguous_followup(text) && active_label.is_some() {
        Source::AdoptedActiveStep
    } else {
        Source::UserRequest
    };
    let mut literal = prose(if source == Source::AdoptedActiveStep {
        active_label.unwrap_or(text)
    } else {
        text
    });
    // Override is tested before ASCII case folding, so ULTRATHINK never matches.
    let override_top = source == Source::UserRequest && contains(&literal, "ultrathink");
    literal.make_ascii_lowercase();
    let matched = RULES.iter().find(|rule| {
        rule.groups
            .iter()
            .all(|group| group.iter().any(|term| contains(&literal, term)))
    });
    let bucket = if override_top {
        None
    } else {
        matched.map(|rule| rule.bucket)
    };
    let reason = if override_top {
        Reason::Ultrathink
    } else if let Some(rule) = matched {
        rule.reason
    } else if last.is_some() {
        Reason::KeepLast
    } else {
        Reason::ModelDefault
    };
    let can_apply = controllable && model.is_some_and(|model| !model.efforts.is_empty());
    let resolved = if !can_apply {
        None
    } else if override_top {
        model.and_then(|model| mapped(model, 0, true))
    } else if let Some(bucket) = bucket {
        model.and_then(|model| mapped(model, bucket.target(), false))
    } else {
        last.map(str::to_owned)
            .or_else(|| model.and_then(|model| model.default_effort.clone()))
    };
    Resolution {
        bucket,
        resolved,
        reason,
        source,
        application: if can_apply {
            Application::RequestedNotAcknowledged
        } else {
            Application::FixedByProviderNotApplied
        },
    }
}

/// Persist `auto` as a typed record. Previous string-only readers refuse rather
/// than silently treating a new mode as a fixed wire value; fixed JSON is exact.
pub mod selection_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Automatic {
        mode: Mode,
    }
    #[derive(Serialize, Deserialize)]
    enum Mode {
        #[serde(rename = "auto")]
        Auto,
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Fixed(String),
        Automatic(Automatic),
    }

    pub fn serialize<S: Serializer>(
        value: &Option<String>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value.as_deref() {
            Some("auto") => Automatic { mode: Mode::Auto }.serialize(serializer),
            _ => value.serialize(serializer),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<String>, D::Error> {
        Ok(
            Option::<Stored>::deserialize(deserializer)?.map(|stored| match stored {
                Stored::Fixed(value) => value,
                Stored::Automatic(Automatic { mode: Mode::Auto }) => "auto".into(),
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(values: &[&str], default: Option<&str>) -> ModelOption {
        ModelOption {
            id: "fixture".into(),
            name: "fixture".into(),
            description: String::new(),
            efforts: values.iter().map(|value| (*value).into()).collect(),
            default_effort: default.map(str::to_owned),
        }
    }

    #[test]
    fn bilingual_rule_table_observations_and_lower_ties() {
        let model = model(&["low", "medium", "high", "xhigh", "max"], Some("medium"));
        for (text, bucket, effort) in [
            (
                "Rename count to total in every supplied file, please.",
                Bucket::Trivial,
                "low",
            ),
            (
                "변수 이름을 count에서 total로 변경해줘.",
                Bucket::Trivial,
                "low",
            ),
            (
                "Compare two designs for the retry API.",
                Bucket::Moderate,
                "high",
            ),
            ("두 가지 설계안을 비교해줘.", Bucket::Moderate, "high"),
            (
                "Investigate why the concurrent cache returns stale results.",
                Bucket::Hard,
                "xhigh",
            ),
            (
                "동시성 캐시가 간헐적으로 잘못되는 원인을 조사해줘.",
                Bucket::Hard,
                "xhigh",
            ),
            (
                "Design and prove subtle exactly-once invariants.",
                Bucket::Hard,
                "xhigh",
            ),
            (
                "정확히 한 번 불변식을 보존하는 설계를 증명해줘.",
                Bucket::Hard,
                "xhigh",
            ),
            (
                "The fix is given: rename old to new; investigate why the race is flaky.",
                Bucket::Trivial,
                "low",
            ),
            (
                "Compare two designs and investigate why the concurrent cache is stale.",
                Bucket::Moderate,
                "high",
            ),
        ] {
            let result = resolve(text, None, Some(&model), None, true);
            assert_eq!(result.bucket, Some(bucket), "{text}");
            assert_eq!(result.resolved.as_deref(), Some(effort), "{text}");
            assert!(result.validate().is_ok());
        }
    }

    #[test]
    fn lowercase_standalone_prose_only_can_escape_ceiling() {
        let model = model(&["low", "high", "xhigh"], Some("high"));
        for text in [
            "ultrathink",
            "ultrathink.",
            "ultrathink...",
            "ultrathink: analyze this.",
            "Please ultrathink, then answer.",
            "한글 요청: ultrathink!",
        ] {
            let result = resolve(text, None, Some(&model), None, true);
            assert_eq!(result.reason, Reason::Ultrathink, "{text}");
            assert_eq!(result.resolved.as_deref(), Some("xhigh"));
        }
        for text in [
            "ultrathinking",
            "ULTRATHINK",
            "Ultrathink",
            "`ultrathink`",
            "``ultrathink``",
            "ultrathink.rs",
            "src/ultrathink",
            "C:\\ultrathink",
            "my_ultrathink",
            "ultrathink-value",
            "ultrathink::item",
            "ultrathink's",
            "'ultrathink'",
            "\"ultrathink\"",
            "“ultrathink”",
            "> ultrathink",
            "    ultrathink",
            "```rs\nultrathink\n```",
            "~~~\nultrathink\n~~~",
            "````rs\n```\nultrathink\n````",
            "\"quoted \\\" ultrathink source\"",
        ] {
            assert_eq!(
                resolve(text, None, Some(&model), None, true).reason,
                Reason::ModelDefault,
                "{text}"
            );
        }
    }

    #[test]
    fn sparse_single_no_ladder_unknown_and_order_are_honest() {
        let sparse = model(&["xhigh", "low", "high"], Some("high"));
        assert_eq!(
            resolve(
                "Investigate why the race is flaky",
                None,
                Some(&sparse),
                None,
                true
            )
            .resolved
            .as_deref(),
            Some("high")
        );
        let two = model(&["low", "xhigh"], Some("xhigh"));
        assert_eq!(
            resolve("Compare two designs", None, Some(&two), None, true)
                .resolved
                .as_deref(),
            Some("low")
        );
        let single = model(&["high"], Some("high"));
        assert_eq!(
            resolve("Rename old to new", None, Some(&single), None, true)
                .resolved
                .as_deref(),
            Some("high")
        );
        for model in [None, Some(model(&[], None))] {
            let result = resolve(
                "Investigate why the race is flaky",
                None,
                model.as_ref(),
                None,
                true,
            );
            assert_eq!(result.bucket, Some(Bucket::Hard));
            assert!(result.resolved.is_none());
            assert_eq!(result.application, Application::FixedByProviderNotApplied);
        }
        let fixed = resolve("ultrathink", None, Some(&sparse), None, false);
        assert!(fixed.resolved.is_none());
        assert!(
            fixed
                .line()
                .contains("effort fixed by provider; not applied")
        );
    }

    #[test]
    fn ambiguity_retains_last_or_default_and_only_exact_followup_uses_adopted_label() {
        let model = model(&["low", "high", "xhigh"], Some("high"));
        assert_eq!(
            resolve("Thank you!", None, Some(&model), Some("low"), true)
                .resolved
                .as_deref(),
            Some("low")
        );
        assert_eq!(
            resolve("Thank you!", None, Some(&model), None, true)
                .resolved
                .as_deref(),
            Some("high")
        );
        for text in ["continue", "next", "계속", "다음 단계"] {
            let result = resolve(
                text,
                Some("Investigate why the cache is stale"),
                Some(&model),
                Some("low"),
                true,
            );
            assert_eq!(result.source, Source::AdoptedActiveStep);
            assert_eq!(result.bucket, Some(Bucket::Hard));
        }
        let result = resolve(
            "Compare two designs; read only",
            Some("Rename old to new"),
            Some(&model),
            None,
            true,
        );
        assert_eq!(result.source, Source::UserRequest);
        assert_eq!(result.bucket, Some(Bucket::Moderate));
        let adopted_override = resolve(
            "continue",
            Some("ultrathink"),
            Some(&model),
            Some("low"),
            true,
        );
        assert_eq!(adopted_override.reason, Reason::KeepLast);
        assert_eq!(adopted_override.resolved.as_deref(), Some("low"));
    }

    #[test]
    fn quoted_provider_source_volume_politeness_and_bound_do_not_raise_effort() {
        let model = model(&["low", "high", "xhigh"], Some("low"));
        for text in [
            "Please handle 1000 files.",
            "\"Investigate why the concurrent cache is stale\"",
            "```\nDesign subtle invariants\n```",
            "'Compare two designs'",
            "A very long and important request with many steps",
        ] {
            assert_eq!(
                resolve(text, None, Some(&model), Some("low"), true).reason,
                Reason::KeepLast
            );
        }
        let text = format!("{} ultrathink", " ".repeat(MAX_PROSE_BYTES));
        assert_eq!(
            resolve(&text, None, Some(&model), None, true).reason,
            Reason::ModelDefault
        );
    }

    #[test]
    fn typed_auto_persistence_refuses_actual_previous_string_reader_and_preserves_fixed_bytes() {
        #[derive(Deserialize)]
        struct PreviousSelection {
            #[serde(rename = "model")]
            _model: Option<String>,
            #[serde(rename = "effort")]
            _effort: Option<String>,
        }
        let fixed = super::super::ModelSelection {
            model: Some("exact/model".into()),
            effort: Some("high".into()),
        };
        let bytes = serde_json::to_string(&fixed).unwrap();
        assert_eq!(bytes, r#"{"model":"exact/model","effort":"high"}"#);
        assert!(serde_json::from_str::<PreviousSelection>(&bytes).is_ok());
        let automatic = super::super::ModelSelection {
            effort: Some("auto".into()),
            ..fixed
        };
        let bytes = serde_json::to_string(&automatic).unwrap();
        assert!(serde_json::from_str::<PreviousSelection>(&bytes).is_err());
        assert_eq!(
            serde_json::from_str::<super::super::ModelSelection>(&bytes).unwrap(),
            automatic
        );
        assert!(
            serde_json::from_str::<super::super::ModelSelection>(
                r#"{"model":null,"effort":{"mode":"auto","unknown":true}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn preference_and_pairing_persistence_reuse_the_typed_selection_and_refuse_string_only_readers()
    {
        #[derive(Deserialize)]
        struct PreviousSelection {
            #[serde(rename = "model")]
            _model: Option<String>,
            #[serde(rename = "effort")]
            _effort: Option<String>,
        }
        #[derive(Deserialize)]
        struct PreviousPreferences {
            #[serde(rename = "provider")]
            _provider: super::super::Agent,
            #[serde(rename = "selections")]
            _selections: std::collections::HashMap<super::super::Agent, PreviousSelection>,
        }
        #[derive(Deserialize)]
        struct PreviousPairing {
            #[serde(rename = "partner")]
            _partner: i64,
            #[serde(rename = "form")]
            _form: usize,
            #[serde(rename = "agent")]
            _agent: super::super::Agent,
            #[serde(rename = "selection")]
            _selection: PreviousSelection,
        }
        let automatic = super::super::ModelSelection {
            model: Some("literal-한글/model".into()),
            effort: Some("auto".into()),
        };
        let preferences = crate::store::ProviderPreferences {
            provider: super::super::Agent::Compatible,
            selections: [(super::super::Agent::Compatible, automatic.clone())]
                .into_iter()
                .collect(),
            reduced_motion: false,
            text_density: crate::store::TextDensity::Compact,
        };
        let saved = serde_json::to_vec(&preferences).unwrap();
        assert!(serde_json::from_slice::<PreviousPreferences>(&saved).is_err());
        assert_eq!(
            serde_json::from_slice::<crate::store::ProviderPreferences>(&saved).unwrap(),
            preferences
        );
        let pairing = crate::workspace::records::Pairing {
            partner: 1,
            form: 3,
            agent: super::super::Agent::Compatible,
            selection: automatic,
        };
        let saved = serde_json::to_vec(&pairing).unwrap();
        assert!(serde_json::from_slice::<PreviousPairing>(&saved).is_err());
        assert_eq!(
            serde_json::from_slice::<crate::workspace::records::Pairing>(&saved).unwrap(),
            pairing
        );
    }
}

#[cfg(test)]
#[test]
fn recognizable_identifier_syntax_and_quoted_source_are_not_prose_overrides() {
    let model = ModelOption {
        id: "fixture".into(),
        name: "fixture".into(),
        description: String::new(),
        efforts: vec!["low".into(), "high".into(), "xhigh".into()],
        default_effort: Some("low".into()),
    };
    for text in [
        "$ultrathink",
        "${ultrathink}",
        "%ultrathink%",
        "ultrathink()",
        "ultrathink(foo)",
        "ultrathink!()",
        "let ultrathink = true;",
        "fn ultrathink() {}",
        "def ultrathink(): pass",
        "「ultrathink」",
        "『ultrathink』",
    ] {
        assert_eq!(
            resolve(text, None, Some(&model), None, true).reason,
            Reason::ModelDefault,
            "{text}"
        );
    }
    let unresolved_fix = resolve(
        "Change the cache to fix why stale data returns.",
        None,
        Some(&model),
        None,
        true,
    );
    assert_eq!(unresolved_fix.bucket, Some(Bucket::Hard));
    let volume = resolve(
        "Please process 1000 files and explain each one.",
        None,
        Some(&model),
        None,
        true,
    );
    assert_eq!(volume.reason, Reason::ModelDefault);
}
