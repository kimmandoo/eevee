//! EV18: local intent at an explicit ordinary send, never a model classifier.
use super::App;
use crate::{messages::Message, providers::Operation};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub form: usize,
    pub operation: Operation,
    pub reason: Message,
}

#[derive(Clone, Copy)]
pub struct Transition {
    pub from: usize,
    pub to: usize,
    pub reason: Message,
    pub fresh: bool,
}

/// A local prediction, not a prepared request or an authority-bearing receipt.
pub struct Preview {
    pub selection: Selection,
    pub automatic: bool,
    pub restriction: Option<Operation>,
    pub effort: Option<crate::providers::effort::Resolution>,
    pub todo_step: Option<String>,
    input: String,
    task: i64,
    partner: i64,
    form: usize,
    operation: Operation,
    completed_plan: bool,
}

#[derive(Default)]
pub struct State {
    pub transition: Option<Transition>,
    pub popup_shown: bool,
    pub(super) motion: Option<super::evolution_motion::Motion>,
    preview: Option<Preview>,
}

struct Rule {
    form: usize,
    operation: Operation,
    reason: Message,
    actions: &'static [&'static str],
    objects: &'static [&'static str],
}

// Ordered task roles, not the old advisory EV01 table. A task noun is required:
// generic conversation ("write a greeting", "explain this fixture") stays put.
const RULES: &[Rule] = &[
    Rule {
        form: 7,
        operation: Operation::Review,
        reason: Message::CoreEvolutionReview,
        actions: &["review", "audit", "검토", "리뷰"],
        objects: &[
            "patch", "diff", "change", "code", "release", "변경", "패치", "코드", "출시",
        ],
    },
    Rule {
        form: 5,
        operation: Operation::Plan,
        reason: Message::CoreEvolutionDiagnosis,
        actions: &[
            "diagnose",
            "debug",
            "investigate",
            "analyze",
            "analyse",
            "진단",
            "분석",
            "조사",
        ],
        objects: &[
            "failure",
            "error",
            "crash",
            "bug",
            "security",
            "vulnerability",
            "root cause",
            "실패",
            "오류",
            "버그",
            "보안",
            "취약",
            "원인",
        ],
    },
    Rule {
        form: 4,
        operation: Operation::Plan,
        reason: Message::CoreEvolutionPlanning,
        actions: &["plan", "design", "compare", "설계", "계획", "비교"],
        objects: &[
            "project",
            "app",
            "application",
            "roadmap",
            "architecture",
            "implementation",
            "approach",
            "migration",
            "feature",
            "system",
            "api",
            "change",
            "프로젝트",
            "앱",
            "애플리케이션",
            "로드맵",
            "아키텍처",
            "구현",
            "접근",
            "마이그레이션",
            "기능",
            "시스템",
            "변경",
        ],
    },
    Rule {
        form: 8,
        operation: Operation::Plan,
        reason: Message::CoreEvolutionDocumentation,
        actions: &[
            "write",
            "create",
            "generate",
            "update",
            "save",
            "draft",
            "document",
            "prepare",
            "summarize",
            "summarise",
            "작성",
            "생성",
            "수정",
            "갱신",
            "정리",
            "저장",
            "요약",
            "준비",
        ],
        objects: &[
            "documentation",
            "docs",
            "readme",
            "changelog",
            "handoff",
            "hand-off",
            "문서",
            "인수인계",
            "작업 인계",
            "핸드오프",
            "변경 기록",
        ],
    },
    Rule {
        form: 6,
        operation: Operation::Implement,
        reason: Message::CoreEvolutionRefactor,
        actions: &[
            "refactor",
            "simplify",
            "optimize",
            "optimise",
            "리팩터",
            "리팩토",
            "단순화",
            "최적화",
        ],
        objects: &[
            "code",
            "module",
            "function",
            "implementation",
            "component",
            "코드",
            "모듈",
            "함수",
            "구현",
            "컴포넌트",
        ],
    },
    Rule {
        form: 3,
        operation: Operation::Implement,
        reason: Message::CoreEvolutionImplementation,
        actions: &[
            "implement",
            "create",
            "generate",
            "produce",
            "write",
            "save",
            "make",
            "edit",
            "modify",
            "replace",
            "overwrite",
            "repair",
            "fix",
            "add",
            "update",
            "구현",
            "생성",
            "만들",
            "뽑",
            "제작",
            "작성",
            "저장",
            "수정",
            "고쳐",
            "고치",
            "바꿔",
            "바꾸",
            "덮어",
            "추가",
        ],
        objects: &[
            "code",
            "file",
            "feature",
            "function",
            "module",
            "component",
            "bug",
            "error",
            "failure",
            "endpoint",
            "project",
            "app",
            "application",
            "코드",
            "파일",
            "기능",
            "함수",
            "모듈",
            "컴포넌트",
            "버그",
            "오류",
            "에러",
            "엔드포인트",
            "프로젝트",
            "앱",
            "애플리케이션",
        ],
    },
    Rule {
        form: 2,
        operation: Operation::Implement,
        reason: Message::CoreEvolutionVerification,
        actions: &[
            "build",
            "compile",
            "run",
            "execute",
            "verify",
            "test",
            "빌드",
            "컴파일",
            "실행",
            "검증",
            "테스트",
        ],
        objects: &[
            "command",
            "test",
            "tests",
            "suite",
            "build",
            "check",
            "compile",
            "project",
            "app",
            "application",
            "program",
            "binary",
            "code",
            "명령",
            "테스트",
            "빌드",
            "컴파일",
            "프로젝트",
            "앱",
            "프로그램",
            "바이너리",
            "코드",
            "검사",
        ],
    },
    Rule {
        form: 1,
        operation: Operation::Plan,
        reason: Message::CoreEvolutionDiscovery,
        actions: &[
            "read", "list", "find", "search", "inspect", "locate", "explore", "show", "읽", "목록",
            "찾", "검색", "살펴", "탐색", "보여", "봐", "열어",
        ],
        objects: &[
            "file",
            "files",
            "source",
            "code",
            "project",
            "repository",
            "directory",
            "directories",
            "module",
            "function",
            "파일",
            "소스",
            "코드",
            "프로젝트",
            "저장소",
            "디렉터리",
            "모듈",
            "함수",
        ],
    },
];

fn contains(text: &str, needle: &str) -> bool {
    if !needle.is_ascii() {
        return text.contains(needle);
    }
    text.match_indices(needle).any(|(at, _)| {
        let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
        (at == 0 || !word(text.as_bytes()[at - 1]))
            && (at + needle.len() == text.len() || !word(text.as_bytes()[at + needle.len()]))
    })
}

fn any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| contains(text, needle))
}

fn filename(text: &str) -> bool {
    let text = text.trim().trim_end_matches('.');
    let text = ["에서", "으로", "을", "를", "에", "로", "의"]
        .iter()
        .find_map(|suffix| text.strip_suffix(suffix))
        .unwrap_or(text);
    if text.is_empty()
        || text.contains("://")
        || text.contains([
            '\n', '\r', '\t', '(', ')', '{', '}', '[', ']', ';', '=', '<', '>', '|',
        ])
    {
        return false;
    }
    let name = text.rsplit(['/', '\\']).next().unwrap_or(text);
    name.rsplit_once('.').is_some_and(|(_, extension)| {
        !extension.is_empty()
            && extension.len() <= 16
            && extension
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            && extension.bytes().any(|byte| byte.is_ascii_alphabetic())
    }) || text.contains('/') && !text.chars().any(char::is_whitespace)
        || [
            "dockerfile",
            "makefile",
            "readme",
            "license",
            "notice",
            "justfile",
            "gemfile",
            "rakefile",
        ]
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

fn source_path(text: &str) -> bool {
    text.split_whitespace().any(|token| {
        let token = token.trim_matches([':', ',', ';', '(', ')']);
        token == "." || filename(token)
    })
}

fn task_object(text: &str, rule: &Rule) -> bool {
    any(text, rule.objects) || matches!(rule.form, 1 | 2 | 3 | 6 | 7) && source_path(text)
}

// Creating a plan is not creating its proposed files. Only suppress an
// artifact verb attached to the planning noun, never a separate edit clause.
fn planning_artifact(clause: &str, action: &str) -> bool {
    if action == "구현" {
        return any(clause, &["계획", "설계"]) && !any(clause, &["구현해", "구현하"]);
    }
    if source_path(clause) || any(clause, &["file", "files", "파일"]) {
        return false;
    }
    if !matches!(
        action,
        "create" | "generate" | "write" | "make" | "작성" | "만들" | "생성"
    ) {
        return false;
    }
    if !action.is_ascii() {
        return clause.find("계획").is_some_and(|plan| {
            clause.match_indices(action).all(|(verb, _)| {
                plan < verb
                    && !any(
                        &clause[plan..verb],
                        &["하고", "세우고", "만들고", "한 뒤", "다음"],
                    )
            })
        });
    }
    clause.match_indices(action).all(|(verb, _)| {
        let after = &clause[verb + action.len()..];
        after.find("plan").is_some_and(|plan| {
            contains(&after[plan..], "plan")
                && !any(&after[..plan], &["and", "then", "file", "files"])
        })
    })
}

// Describing a future command is not asking the host to execute it. A separate
// imperative clause ("plan ...; then build ...") still requests that capability.
fn execution_description(clause: &str, action: &str) -> bool {
    let Some(at) = clause.find(action) else {
        return false;
    };
    let prefix = clause[..at]
        .rsplit(" and ")
        .next()
        .unwrap_or("")
        .rsplit(" then ")
        .next()
        .unwrap_or("")
        .rsplit("하고")
        .next()
        .unwrap_or("");
    any(
        prefix,
        &[
            "plan",
            "design",
            "explain",
            "describe",
            "how to",
            "what",
            "why",
            "whether",
            "diagnose",
            "investigate",
            "analyze",
            "analyse",
            "inspect",
            "review",
            "계획",
            "설명",
            "방법",
            "분석",
            "진단",
            "왜",
        ],
    ) || action.is_ascii()
        && prefix
            .split_whitespace()
            .next_back()
            .is_some_and(|word| matches!(word, "the" | "this" | "a" | "project" | "current"))
        || !action.is_ascii()
            && [
                "가", "는", "된", "의", "오류", "결과", "환경", "시간", "실패",
            ]
            .iter()
            .any(|suffix| clause[at + action.len()..].trim_start().starts_with(suffix))
        || !action.is_ascii()
            && any(&clause[at + action.len()..], &["계획", "방법"])
            && !any(clause, &["실행해", "빌드해", "컴파일해"])
}

/// Strip literal/source instructions, retaining only object markers for quoted
/// filenames or opaque arguments. Only unquoted verbs can request a capability;
/// the original draft alone is submitted.
fn instruction(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut fence = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let marker = &trimmed[..3];
            if fence == Some(marker) {
                fence = None;
            } else if fence.is_none() {
                fence = Some(marker);
            }
            continue;
        }
        if fence.is_some()
            || trimmed.starts_with('>')
            || line.starts_with("    ")
            || [
                "provider:",
                "provider output:",
                "provider response:",
                "assistant:",
                "assistant output:",
                "system:",
                "tool:",
                "tool output:",
                "output:",
                "metadata:",
                "출력:",
                "응답:",
                "메타데이터:",
                "fn ",
                "let ",
                "const ",
                "import ",
                "use ",
                "{",
                "[",
                "<",
            ]
            .iter()
            .any(|prefix| {
                trimmed
                    .get(..prefix.len())
                    .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
            })
        {
            continue;
        }
        let mut quote = None;
        let mut previous = ' ';
        for (offset, ch) in line.char_indices() {
            if let Some((end, start)) = quote {
                if ch == end {
                    quote = None;
                    if filename(&line[start..offset]) {
                        result.push_str(" ./ ");
                    } else if !line[start..offset].trim().is_empty() {
                        // An opaque command such as `cargo build` is an object,
                        // never a source of verbs. Only an outer run/execute
                        // imperative can match the command rule with this noun.
                        result.push_str(" command ");
                    } else {
                        result.push(' ');
                    }
                }
            } else if matches!(ch, '`' | '"' | '“' | '‘')
                || ch == '\'' && !previous.is_alphanumeric()
            {
                quote = Some((
                    match ch {
                        '“' => '”',
                        '‘' => '’',
                        _ => ch,
                    },
                    offset + ch.len_utf8(),
                ));
                result.push(' ');
            } else {
                result.extend(ch.to_lowercase());
            }
            previous = ch;
        }
        result.push('\n');
    }
    result
}

const READONLY: &[&str] = &[
    "read-only",
    "read only",
    "readonly",
    "only read",
    "do not write",
    "don't write",
    "do not edit",
    "don't edit",
    "do not modify",
    "don't modify",
    "do not change files",
    "don't change files",
    "no changes",
    "without changes",
    "without changing anything",
    "without modifying files",
    "without writing",
    "without editing",
    "읽기 전용",
    "읽기만",
    "쓰기 금지",
    "수정하지",
    "변경하지",
    "작성하지",
];
const NEGATIVE: &[&str] = &[
    "do not",
    "don't",
    "never",
    "not",
    "avoid",
    "without",
    "하지 마",
    "하지마",
    "하지 않",
    "하지 말",
    "금지",
    "말고",
    "아니",
];

// Elliptical implementation requests still name the requested capability.
// Generic "continue" remains ambiguous; a plan-bound "proceed" is handled below.
fn implementation_followup(literal: &str) -> bool {
    let text = literal.trim().trim_end_matches(['.', '!', '?']).trim();
    matches!(
        text,
        "implement"
            | "implement it"
            | "implement this"
            | "implement the plan"
            | "please implement"
            | "please implement it"
            | "구현해줘"
            | "구현해 줘"
            | "구현해주세요"
            | "구현해 주세요"
            | "구현해"
            | "구현하자"
            | "이대로 구현해줘"
            | "계획대로 구현해줘"
            // Elliptical "act on what we discussed" requests.
            | "fix it"
            | "fix this"
            | "fix them"
            | "please fix it"
            | "do it"
            | "yes do it"
            | "yes, do it"
            | "make it"
            | "apply it"
            | "apply the fix"
            | "apply the changes"
            | "고쳐줘"
            | "고쳐 줘"
            | "고쳐주세요"
            | "수정해줘"
            | "수정해 줘"
            | "수정해주세요"
            | "적용해줘"
            | "적용해 줘"
            | "반영해줘"
            | "만들어줘"
            | "만들어 줘"
            | "그렇게 해줘"
            | "응 그렇게 해줘"
            | "그대로 해줘"
    )
}

fn proceed_with_plan(text: &str) -> bool {
    let literal = instruction(text);
    matches!(
        literal.trim().trim_end_matches(['.', '!', '?']).trim(),
        "proceed"
            | "please proceed"
            | "go ahead"
            | "proceed with the plan"
            | "진행해줘"
            | "진행해 줘"
            | "진행해주세요"
            | "진행해 주세요"
            | "진행하자"
            | "계획대로 진행해줘"
    )
}

pub fn classify(text: &str) -> Option<Selection> {
    if text.len() > super::MAX_GOAL_BYTES {
        return None;
    }
    let literal = instruction(text);
    let readonly = any(&literal, READONLY);
    if !readonly && implementation_followup(&literal) {
        return Some(Selection {
            form: 3,
            operation: Operation::Implement,
            reason: Message::CoreEvolutionImplementation,
        });
    }
    let clauses = || {
        literal
            .split(". ")
            .flat_map(|part| part.split(" but "))
            .flat_map(|part| part.split("말고"))
            .flat_map(|part| part.split(['\n', ';', ',', '!', '?']))
            .map(|clause| {
                clause
                    .split_once(" without ")
                    .map_or(clause, |(request, _)| request)
            })
            .filter(|clause| {
                !any(clause, NEGATIVE)
                    && !any(
                        clause,
                        &["수정하지", "변경하지", "작성하지", "실행하지", "검토하지"],
                    )
            })
    };
    let read = &RULES[RULES.len() - 1];
    // Source nouns can precede a later edit/refactor clause. Keep negated and
    // literal source out, but do not let a discovery prefix hide that action.
    let source_context = clauses().any(|clause| task_object(clause, read));
    if readonly
        && task_object(&literal, read)
        && clauses().any(|clause| any(clause, read.actions))
        && !clauses().any(|clause| any(clause, RULES[0].actions) && task_object(clause, &RULES[0]))
    {
        return Some(Selection {
            form: 1,
            operation: Operation::Plan,
            reason: Message::CoreEvolutionReadonly,
        });
    }
    // Request intent chooses capability independently of the displayed specialty.
    // A diagnosis/review prefix must not hide a later explicit change request.
    let mutation_requested = !readonly
        && clauses().any(|clause| {
            RULES.iter().any(|rule| {
                rule.operation == Operation::Implement
                    && rule.actions.iter().any(|action| {
                        contains(clause, action)
                            && !planning_artifact(clause, action)
                            && !(rule.form == 2 && execution_description(clause, action))
                    })
                    && (task_object(clause, rule) || source_context)
            }) || [
                "write", "create", "update", "generate", "save", "작성", "생성", "수정", "갱신",
                "저장",
            ]
            .iter()
            .any(|action| contains(clause, action) && !planning_artifact(clause, action))
                && (source_path(clause)
                    || any(
                        clause,
                        &[
                            "documentation",
                            "docs",
                            "readme",
                            "changelog",
                            "file",
                            "파일",
                            "문서",
                            "변경 기록",
                        ],
                    ))
        });
    let independent_review = any(&literal, &["independent", "independently", "독립"]);
    for rule in RULES {
        if clauses().any(|clause| {
            rule.actions.iter().any(|action| {
                contains(clause, action)
                    && !(rule.form == 2 && execution_description(clause, action))
            }) && (task_object(clause, rule) || matches!(rule.form, 3 | 6) && source_context)
        }) {
            // A read-only instruction cannot become a mutation-capable operation.
            // Prefer the actual read task over an incidental write-role noun.
            if readonly && matches!(rule.form, 3 | 6) {
                return Some(Selection {
                    form: 1,
                    operation: Operation::Plan,
                    reason: Message::CoreEvolutionReadonly,
                });
            }
            return Some(Selection {
                form: rule.form,
                operation: if rule.form == 7 && independent_review {
                    Operation::Review
                } else if readonly {
                    Operation::Plan
                } else if mutation_requested {
                    Operation::Implement
                } else if rule.operation == Operation::Review {
                    Operation::Plan
                } else {
                    rule.operation
                },
                reason: if readonly {
                    Message::CoreEvolutionReadonly
                } else {
                    rule.reason
                },
            });
        }
    }
    readonly.then_some(Selection {
        form: 1,
        operation: Operation::Plan,
        reason: Message::CoreEvolutionReadonly,
    })
}

impl App {
    /// The notification never covers consent or live generated-file code.
    pub fn evolution_popup_ready(&self) -> bool {
        self.evolution
            .transition
            .is_some_and(|transition| transition.from != transition.to)
            && self.approval_shortcut_available()
            && !self.carry_over_explicit()
            && self.approval_count() == 0
            && self.planning.request.is_none()
            && !self.planning.items.iter().any(|item| {
                std::sync::Arc::ptr_eq(&item.request, &self.planning.submitted_request)
                    && matches!(
                        item.kind,
                        crate::providers::ItemKind::File(
                            crate::providers::FilePhase::Generating
                                | crate::providers::FilePhase::Received
                                | crate::providers::FilePhase::AwaitingReview
                        )
                    )
            })
    }

    /// Shared by inspection and ordinary Submit; does not mutate or prepare work.
    fn request_evolution(&self) -> Option<Selection> {
        let mut choice = classify(&self.input)
            .or_else(|| {
                (self.lead.auto_form
                    && self.planning.explicit_operation().is_none()
                    && crate::providers::effort::ambiguous_followup(&self.input))
                .then(|| {
                    self.workflows
                        .progress
                        .as_ref()?
                        .active_label()
                        .and_then(classify)
                })
                .flatten()
            })
            .or_else(|| {
                (self.planning.operation == Operation::Plan
                    && proceed_with_plan(&self.input)
                    && classify(&self.planning.submitted_request)
                        .is_some_and(|prior| prior.reason == Message::CoreEvolutionPlanning)
                    && self.planning.has_completed_plan())
                .then_some(Selection {
                    form: 3,
                    operation: Operation::Implement,
                    reason: Message::CoreEvolutionImplementation,
                })
            })
            .or_else(|| {
                // Like other coding agents, an unclassified request gets the full
                // tool set: every file effect and command still needs exact
                // one-shot approval. An ongoing planning/diagnosis conversation
                // keeps its session for ambiguous follow-ups instead.
                (!self.planning.plan_conversation()).then_some(Selection {
                    form: self.form(),
                    operation: Operation::Implement,
                    reason: Message::CoreEvolutionImplementation,
                })
            })?;
        if !self.lead.auto_form {
            // Pin appearance, never the tools needed for the current request.
            choice.form = self.form();
        }
        if self.planning.explicit_operation() == Some(Operation::Review) {
            choice = Selection {
                form: if self.lead.auto_form { 7 } else { self.form() },
                operation: Operation::Review,
                reason: Message::CoreEvolutionReview,
            };
        } else if self.planning.explicit_operation() == Some(Operation::Plan) {
            choice.operation = Operation::Plan;
            choice.reason = Message::CoreEvolutionReadonly;
        }
        Some(choice)
    }

    pub(super) fn open_evolution_preview(&mut self) {
        if !self.composer_is_request() {
            self.notice =
                "Open the Request composer to inspect its next evolution; nothing sent.".into();
            return;
        }
        let selection = self.request_evolution().unwrap_or(Selection {
            form: self.form(),
            operation: self.planning.operation,
            reason: Message::CoreEvolutionUnchanged,
        });
        self.evolution.preview = Some(Preview {
            selection,
            automatic: self.lead.auto_form,
            restriction: self.planning.explicit_operation(),
            input: self.input.clone(),
            task: self.task_id,
            partner: self.lead.id,
            form: self.form(),
            operation: self.planning.operation,
            completed_plan: self.planning.has_completed_plan(),
            effort: self.planning.effort_preview(
                &self.input,
                self.workflows
                    .progress
                    .as_ref()
                    .and_then(|p| p.active_label()),
            ),
            todo_step: self.workflows.progress.as_ref().map(|p| p.summary()),
        });
        self.open_overlay(super::Overlay::EvolutionPreview);
    }

    pub fn evolution_preview(&self) -> Option<&Preview> {
        self.evolution.preview.as_ref().filter(|preview| {
            preview.input == self.input
                && preview.task == self.task_id
                && preview.partner == self.lead.id
                && preview.form == self.form()
                && preview.operation == self.planning.operation
                && preview.automatic == self.lead.auto_form
                && preview.restriction == self.planning.explicit_operation()
                && preview.completed_plan == self.planning.has_completed_plan()
                && preview.todo_step == self.workflows.progress.as_ref().map(|p| p.summary())
                && preview.effort
                    == self.planning.effort_preview(
                        &self.input,
                        self.workflows
                            .progress
                            .as_ref()
                            .and_then(|p| p.active_label()),
                    )
        })
    }

    /// Called only after ordinary Submit's owner/safety guards, before save.
    pub(super) fn evolve_request(&mut self) -> bool {
        let Some(choice) = self.request_evolution() else {
            return false;
        };
        let from = self.form();
        let fresh = self.planning.prepare_automatic_operation(choice.operation);
        if from != choice.form || fresh {
            self.set_form(choice.form);
            self.evolution.transition = Some(Transition {
                from,
                to: choice.form,
                reason: choice.reason,
                fresh,
            });
            self.evolution.popup_shown = false;
            if self.lead.auto_form
                && self.planning.explicit_operation().is_none()
                && crate::providers::effort::ambiguous_followup(&self.input)
                && let Some(progress) = &self.workflows.progress
                && let Some(step) = progress.active_label()
            {
                self.notice = format!(
                    "Automatic form follows adopted todo revision {} / {step}",
                    progress.revision
                );
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{Overlay, StorageState, execution},
        providers::{self, Agent, ModelSelection, Request},
        store,
    };
    use crossterm::event::Event;

    fn app() -> App {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.task_id = 7;
        app.storage = StorageState::Saved;
        app.planning.reset(7);
        app.planning.preference_event(Ok(None));
        app
    }
    #[test]
    fn adopted_progress_predicts_explicit_continuations_without_midrun_evolution() {
        use crate::workspace::progress::{State, Status, Step};
        for (width, height) in [(120, 36), (60, 18)] {
            for (followup, labels) in [
                (
                    "continue",
                    ["Investigate error", "Fix code", "Verify tests"],
                ),
                ("계속", ["오류 조사", "코드 수정", "테스트 실행"]),
            ] {
                let mut app = app();
                app.width = width;
                app.height = height;
                app.open_editor(store::EditorField::Request);
                app.input = followup.into();
                let mut progress = State::default();
                for active in 0..3 {
                    let steps = labels
                        .iter()
                        .enumerate()
                        .map(|(index, label)| Step {
                            label: (*label).into(),
                            phase: Some(["investigate", "fix", "verify"][index].into()),
                            status: if index < active {
                                Status::Done
                            } else if index == active {
                                Status::Active
                            } else {
                                Status::Pending
                            },
                        })
                        .collect();
                    progress.propose("fixture", steps).unwrap();
                    app.workflows.progress = Some(progress.clone());
                    let before = (
                        app.form(),
                        app.planning.operation,
                        app.input.clone(),
                        app.cursor,
                    );
                    assert!(
                        app.workflows
                            .progress
                            .as_ref()
                            .unwrap()
                            .active_label()
                            .is_none()
                    );
                    progress.adopt().unwrap();
                    app.workflows.progress = Some(progress.clone());
                    assert_eq!(
                        (
                            app.form(),
                            app.planning.operation,
                            app.input.clone(),
                            app.cursor
                        ),
                        before
                    );
                    app.open_evolution_preview();
                    let predicted = app.evolution_preview().unwrap();
                    assert_eq!(predicted.selection.form, [5, 3, 2][active]);
                    assert!(
                        predicted
                            .todo_step
                            .as_deref()
                            .unwrap()
                            .contains(labels[active])
                    );
                    let selection = predicted.selection;
                    app.close_overlay();
                    assert!(app.evolve_request());
                    assert_eq!(
                        (app.form(), app.planning.operation),
                        (selection.form, selection.operation)
                    );
                    assert!(app.pending_request.is_none() && app.planning.pending.is_none());
                }
            }
        }
    }

    #[test]
    fn adopted_progress_never_overrides_manual_or_explicit_user_policy() {
        use crate::workspace::progress::{State, Status, Step};
        let mut app = app();
        let mut progress = State::default();
        progress
            .propose(
                "fixture",
                vec![Step {
                    label: "Fix code".into(),
                    phase: None,
                    status: Status::Active,
                }],
            )
            .unwrap();
        progress.adopt().unwrap();
        app.workflows.progress = Some(progress);
        app.input = "Review the code".into();
        assert_eq!(app.request_evolution().unwrap().form, 7);
        app.input = "continue".into();
        app.planning.set_operation(Operation::Plan);
        assert_eq!(app.request_evolution().unwrap().operation, Operation::Plan);
        app.lead.auto_form = false;
        assert_eq!(app.request_evolution().unwrap().form, app.form());
        app.input = "Read src/main.rs; do not change files".into();
        assert_eq!(app.request_evolution().unwrap().operation, Operation::Plan);
    }

    #[test]
    fn prediction_preserves_draft_and_matches_submitted_appearance_and_policy() {
        for draft in [
            "Review the code and fix src/main.rs.",
            "코드를 검토하고 src/main.rs를 수정해줘.",
            "Read src/main.rs; do not change files.",
            "\"Create src/main.rs\" is quoted text.",
            "계속 설명해줘.",
        ] {
            for manual in [false, true] {
                for restriction in [None, Some(Operation::Plan), Some(Operation::Review)] {
                    let mut app = app();
                    app.open_editor(store::EditorField::Request);
                    app.lead.auto_form = !manual;
                    app.input = draft.into();
                    app.cursor = draft.len();
                    if let Some(restriction) = restriction {
                        app.planning.set_operation(restriction);
                    }
                    let before = (app.form(), app.planning.operation, app.cursor);
                    app.open_evolution_preview();
                    let predicted = app.evolution_preview().unwrap().selection;
                    assert_eq!((app.form(), app.planning.operation, app.cursor), before);
                    assert_eq!(app.input, draft);
                    assert!(app.pending_request.is_none() && app.planning.pending.is_none());
                    app.close_overlay();
                    app.submit_request();
                    assert_eq!(
                        (app.form(), app.planning.operation),
                        (predicted.form, predicted.operation),
                        "{draft}"
                    );
                }
            }
        }
    }

    #[test]
    fn prediction_invalidates_changed_draft_owner_and_policy() {
        for change in 0..5 {
            let mut app = app();
            app.open_editor(store::EditorField::Request);
            app.input = "Create src/new.rs".into();
            app.open_evolution_preview();
            match change {
                0 => app.input.push('!'),
                1 => app.task_id += 1,
                2 => app.lead.id += 1,
                3 => app.lead.auto_form = false,
                _ => app.planning.set_operation(Operation::Review),
            }
            assert!(app.evolution_preview().is_none());
            assert!(app.pending_request.is_none() && app.planning.pending.is_none());
        }
    }

    #[test]
    fn prediction_inspector_cannot_send_or_edit_the_hidden_request() {
        let mut app = app();
        app.open_editor(store::EditorField::Request);
        app.input = "Create src/한글.rs".into();
        app.cursor = app.input.len() - 3;
        let original = (app.input.clone(), app.cursor);
        app.open_evolution_preview();
        let now = std::time::Instant::now();
        app.update_at(
            Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            )),
            now,
        );
        app.update_at(Event::Paste("unrequested replacement".into()), now);
        assert!(app.pending_request.is_none() && app.planning.pending.is_none());
        assert_eq!((&app.input, app.cursor), (&original.0, original.1));
        app.update_at(
            Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            )),
            now,
        );
        assert_eq!(app.overlay, None);
        assert!(app.composer_is_request());
        assert_eq!((&app.input, app.cursor), (&original.0, original.1));
    }
    fn planned_app() -> App {
        let mut app = app();
        app.planning.agent = Agent::Compatible;
        app.input = "새 프로젝트 구현 계획 세워줘".into();
        app.submit_request();
        app.pending_request = None;
        app.planning.submitted_request = app.input.as_str().into();
        app.planning.front = Some(app.attribution());
        app.planning.items.push(super::super::planning::Item {
            thread_id: "planning-thread".into(),
            turn_id: "planning-turn".into(),
            item_id: "public-plan".into(),
            kind: providers::ItemKind::Message,
            text: "Create planned/한글/saved.txt containing the chosen result.".into(),
            complete: true,
            request: app.planning.submitted_request.clone(),
            owner: Some(app.attribution()),
            status: None,
        });
        app.planning.thread_id = Some("planning-thread".into());
        app.planning.plan_completed = true;
        app
    }

    #[test]
    fn ordinary_implementation_followups_carry_public_plan_without_old_authority() {
        for followup in [
            "구현해줘",
            "구현해 주세요.",
            "Implement it.",
            "진행해줘",
            "Go ahead.",
        ] {
            let mut app = planned_app();
            app.input = followup.into();
            app.submit_request();
            assert_eq!(app.planning.operation, Operation::Implement, "{followup}");
            assert!(app.planning.thread_id.is_none());
            assert_eq!(app.planning.items.len(), 1);
            let snapshot = app.snapshot("/workspace");
            app.planning.command("new", true, &snapshot, &[], None);
            let Some(Request::Preview(input)) = app.planning.pending.take() else {
                panic!("missing implementation preview for {followup}");
            };
            assert_eq!(input.instruction(), followup);
            assert!(input.resume.is_none() && input.conversation.is_none());
            assert!(input.work.is_none());
            let context = input.prior_plan_payload().unwrap();
            assert!(context.to_string().contains("planned/한글/saved.txt"));
            assert!(!context.to_string().contains("planning-thread"));
            assert_eq!(context["messages"][0]["role"], "user");
            assert_eq!(context["messages"][1]["role"], "assistant");
            assert!(input.validate_selection().is_ok());
        }
    }

    #[test]
    fn proceeding_requires_a_complete_current_plan_not_analysis_or_source_instructions() {
        for prior in [
            "Analyze the error.",
            "Read src/main.rs.",
            "Plan the project, read only.",
        ] {
            let mut app = planned_app();
            app.planning.submitted_request = prior.into();
            app.planning.items[0].request = app.planning.submitted_request.clone();
            app.input = "진행해줘".into();
            app.submit_request();
            assert_eq!(app.planning.operation, Operation::Plan, "{prior}");
        }
        for invalid in 0..5 {
            let mut app = planned_app();
            match invalid {
                0 => app.planning.items[0].complete = false,
                1 => app.planning.plan_completed = false,
                2 => app.planning.items[0].owner = None,
                3 => app.planning.items[0].text = "x".repeat(64 * 1024),
                _ => app.planning.items[0].request = "unrelated request".into(),
            }
            app.input = "진행해줘".into();
            app.submit_request();
            assert_eq!(app.planning.operation, Operation::Plan);
        }
        for operation in [Operation::Plan, Operation::Review] {
            let mut app = planned_app();
            app.planning.set_operation(operation);
            app.input = "구현해줘".into();
            app.submit_request();
            assert_eq!(app.planning.operation, operation);
        }
        for text in [
            "\"구현해줘\"",
            "> 구현해줘",
            "구현하지 마",
            "Explain how to implement it.",
        ] {
            assert!(classify(text).is_none(), "{text}");
        }
        for text in [
            "프로젝트 만들어줘",
            "앱을 만들어줘",
            "Overwrite saved.txt",
            "saved.txt를 덮어써줘",
        ] {
            assert_eq!(
                classify(text).unwrap().operation,
                Operation::Implement,
                "{text}"
            );
        }
    }

    #[test]
    fn ordinary_project_planning_is_readonly_without_setup_or_selected_paths() {
        for text in [
            "프로젝트 계획 세워줘",
            "프로젝트 계획을 작성해줘.",
            "앱 구현 계획을 만들어줘.",
            "Help me plan this project.",
            "Make a project plan.",
            "Create an implementation plan for this app.",
            "Write a project plan.",
            "Design the project architecture.",
            "Plan the project; read .env and .git/config.",
        ] {
            let selected = classify(text).unwrap();
            assert_eq!(
                (selected.form, selected.operation),
                (4, Operation::Plan),
                "{text}"
            );
            for agent in [Agent::Compatible, Agent::Codex, Agent::Claude] {
                let mut app = app();
                app.planning.agent = agent;
                app.planning.edit_paths = vec![".env".into(), ".git/config".into()];
                assert!(app.goal.is_empty());
                app.input = text.into();
                app.submit_request();
                assert_eq!(app.planning.operation, Operation::Plan, "{text}");
                assert_eq!(app.goal, text);
                let snapshot = app.snapshot("/workspace");
                app.planning.command("new", true, &snapshot, &[], None);
                let Some(Request::Preview(input)) = app.planning.pending.take() else {
                    panic!("ordinary planning did not prepare: {text}");
                };
                assert_eq!(input.instruction(), text);
                assert!(input.edit_paths.is_empty());
                assert!(!providers::writing_required(input.operation));
                assert!(input.report.is_none());
                assert!(input.validate_review().is_ok());
                assert!(!app.execution.busy());
            }
        }
    }

    #[test]
    fn ordinary_coding_requests_get_write_tools_like_other_agents() {
        for text in [
            "make a snake game in python",
            "add a login page",
            "implement dark mode",
            "다크모드 구현해줘",
            "README에 설치 방법 써줘",
            "의존성 업데이트해줘",
            "hello world 출력하는 파이썬 파일",
            "뭐 더하면 좋을까?",
        ] {
            let mut app = app();
            app.input = text.into();
            app.submit_request();
            assert_eq!(app.planning.operation, Operation::Implement, "{text}");
            let snapshot = app.snapshot("/workspace");
            app.planning.command("new", true, &snapshot, &[], None);
            let Some(Request::Preview(input)) = app.planning.pending.take() else {
                panic!("missing preparation: {text}");
            };
            assert!(providers::writing_required(input.operation), "{text}");
        }
        // Explicit read-only intent and a Plan restriction still win.
        for (text, restrict) in [("이 코드 설명해줘, 읽기만", false), ("make a game", true)]
        {
            let mut app = app();
            if restrict {
                app.planning.set_operation(Operation::Plan);
            }
            app.input = text.into();
            app.submit_request();
            assert_eq!(app.planning.operation, Operation::Plan, "{text}");
        }
        // After a diagnosis, an elliptical fix request gains write tools.
        for followup in ["고쳐줘", "fix it", "응 그렇게 해줘"] {
            let mut app = app();
            app.input = "Analyze the error.".into();
            app.submit_request();
            assert_eq!(app.planning.operation, Operation::Plan);
            app.pending_request = None;
            app.planning.thread_id = Some("diagnosis-thread".into());
            app.input = followup.into();
            app.submit_request();
            assert_eq!(app.planning.operation, Operation::Implement, "{followup}");
        }
    }

    #[test]
    fn planning_followups_preserve_session_until_an_actual_mutation_request() {
        for followup in [
            "Explain the first step.",
            "첫 단계를 자세히 설명해줘.",
            "Continue.",
        ] {
            let mut app = app();
            app.input = "프로젝트 계획 세워줘".into();
            app.submit_request();
            app.pending_request = None;
            app.planning.thread_id = Some("owned-planning-thread".into());
            app.input = followup.into();
            app.submit_request();
            assert_eq!(app.planning.operation, Operation::Plan);
            assert_eq!(
                app.planning.thread_id.as_deref(),
                Some("owned-planning-thread")
            );
            assert!(app.pending_request.unwrap().1);
            app.pending_request = None;
            app.input = "Create `src/main.rs`.".into();
            app.submit_request();
            assert_eq!(app.planning.operation, Operation::Implement);
            assert!(app.planning.thread_id.is_none());
            assert!(!app.pending_request.unwrap().1);
            app.pending_request = None;
            app.input = "Continue.".into();
            app.submit_request();
            assert_eq!(app.planning.operation, Operation::Implement);
        }
    }

    #[test]
    fn planning_artifacts_do_not_hide_explicit_file_effects() {
        for text in [
            "Plan the project and implement its feature.",
            "Make a project plan and create the code.",
            "Plan the project; create src/main.rs.",
            "Write the project plan to plan.md.",
            "프로젝트 계획을 세우고 기능을 구현해줘.",
            "프로젝트 계획을 작성하고 파일을 생성해줘.",
            "프로젝트 구현 계획을 세우고 코드를 구현해줘.",
            "프로젝트 계획을 만들고 기능을 만들어줘.",
        ] {
            assert_eq!(
                classify(text).unwrap().operation,
                Operation::Implement,
                "{text}"
            );
        }
        for text in [
            "Plan the project and create code; do not modify files.",
            "프로젝트 계획을 세우고 코드를 구현해줘. 파일은 수정하지 마.",
        ] {
            assert_eq!(classify(text).unwrap().operation, Operation::Plan, "{text}");
        }
    }

    #[test]
    fn deterministic_task_roles_in_both_languages() {
        for (form, operation, en, ko) in [
            (
                1,
                Operation::Plan,
                "Read the project source files.",
                "프로젝트 소스 파일을 읽어줘.",
            ),
            (
                2,
                Operation::Implement,
                "Run the project tests.",
                "프로젝트 테스트를 실행해줘.",
            ),
            (
                3,
                Operation::Implement,
                "Create a Python code file.",
                "파이썬 코드 파일을 생성해줘.",
            ),
            (
                4,
                Operation::Plan,
                "Design the module architecture and implementation plan.",
                "모듈 아키텍처와 구현 계획을 설계해줘.",
            ),
            (
                5,
                Operation::Plan,
                "Diagnose the root cause of this runtime failure.",
                "실행 오류의 근본 원인을 진단해줘.",
            ),
            (
                6,
                Operation::Implement,
                "Refactor this code without changing behavior.",
                "동작을 유지하면서 코드를 리팩터링해줘.",
            ),
            (
                7,
                Operation::Review,
                "Perform an independent code review of the changes.",
                "변경한 코드를 독립적으로 리뷰해줘.",
            ),
            (
                8,
                Operation::Plan,
                "Prepare a handoff summary.",
                "작업 인계 요약을 작성해줘.",
            ),
        ] {
            for text in [en, ko] {
                let selected = classify(text).unwrap_or_else(|| panic!("missing role for {text}"));
                assert_eq!(
                    (selected.form, selected.operation),
                    (form, operation),
                    "{text}"
                );
            }
        }
    }

    #[test]
    fn negated_without_tail_cannot_authorize_changes() {
        for text in [
            "Review the code changes without implementing a fix.",
            "Without implementing a fix, review the code changes.",
            "코드를 수정하지 말고 검토해줘.",
        ] {
            let selected = classify(text).unwrap();
            assert_eq!(selected.operation, Operation::Plan, "{text}");
        }
        for text in [
            "Review the code changes without implementing a fix; implement the code fix afterwards.",
            "Refactor this code without changing behavior.",
        ] {
            assert_eq!(
                classify(text).unwrap().operation,
                Operation::Implement,
                "{text}"
            );
        }
    }

    #[test]
    fn precedence_readonly_negation_and_documentation_capability() {
        for text in [
            "Do not write code; read the project files.",
            "Read project source without changing anything.",
            "파일을 수정하지 말고 읽어줘.",
            "Do not create a file, but read the source files.",
        ] {
            let selected = classify(text).unwrap();
            assert_eq!(
                (selected.form, selected.operation),
                (1, Operation::Plan),
                "{text}"
            );
        }
        for text in [
            "Do not implement code.",
            "Never run tests.",
            "파일을 생성하지 마.",
            "코드를 검토하지 마.",
        ] {
            assert!(classify(text).is_none(), "{text}");
        }
        for text in [
            "Write project documentation to README.md.",
            "README.md에 프로젝트 문서를 작성해줘.",
        ] {
            let selected = classify(text).unwrap();
            assert_eq!(
                (selected.form, selected.operation),
                (8, Operation::Implement)
            );
        }
        let selected = classify("Draft project documentation; do not modify files.").unwrap();
        assert_eq!((selected.form, selected.operation), (8, Operation::Plan));
        assert_eq!(
            classify("Review the code changes; implement a code fix afterwards.")
                .unwrap()
                .form,
            7
        );
        assert_eq!(
            classify("Plan the architecture; implement the feature afterwards.")
                .unwrap()
                .form,
            4
        );
        assert_eq!(classify("Read src/main.rs").unwrap().form, 1);
        assert_eq!(classify("Create hello.py").unwrap().form, 3);
        assert_eq!(classify("Review src/main.rs").unwrap().form, 7);
        // Discovery must not hide an explicit edit in the same instruction.
        assert_eq!(
            classify("Read the source file and edit its value.")
                .unwrap()
                .operation,
            Operation::Implement
        );
        assert_eq!(classify("Make a Python file.").unwrap().form, 3);
        assert_eq!(classify("파이썬 파일을 만들어줘.").unwrap().form, 3);
        assert_eq!(classify("소스 파일의 값을 바꿔줘.").unwrap().form, 3);
    }

    #[test]
    fn discovery_and_mutation_share_source_context_across_clauses() {
        for (text, form) in [
            (
                "List the project files, read the generated Python source, and edit only its value using the observed digest.",
                3,
            ),
            ("Read src/main.rs; edit its value.", 3),
            ("프로젝트 소스 파일을 읽고, 그 값을 수정해줘.", 3),
            ("List the source files, then refactor their structure.", 6),
            ("코드 파일을 찾아줘. 그 다음 리팩터링해줘.", 6),
        ] {
            let selected = classify(text).unwrap();
            assert_eq!(
                (selected.form, selected.operation),
                (form, Operation::Implement),
                "{text}"
            );
        }
        for text in [
            "List project source files, but do not edit their values.",
            "프로젝트 파일 목록을 찾아줘. 파일을 수정하지 마.",
        ] {
            let selected = classify(text).unwrap();
            assert_eq!(
                (selected.form, selected.operation),
                (1, Operation::Plan),
                "{text}"
            );
        }
        assert!(classify("Do not read source files; write a greeting.").is_none());
        assert_eq!(
            classify("Read the source files:\n```\nedit their values\n```")
                .unwrap()
                .form,
            1
        );
    }

    #[test]
    fn literals_metadata_generic_chat_and_ambiguous_followups_never_classify() {
        for text in [
            "Continue.",
            "Yes, that one.",
            "계속해줘.",
            "Write a greeting.",
            "Explain this fixture.",
            "\"implement code\"",
            "'run tests'",
            "`create code file`",
            "> Review the code changes.\nThanks.",
            "```rust\nimplement code\n```\nContinue.",
            "~~~text\nRun the tests\n~~~",
            "    create code file",
            "Provider: implement code\nMetadata: run tests\nOutput: review code",
            "fn implement_code() { create_file(); }",
            "{\"instruction\": \"implement code\"}",
        ] {
            assert!(classify(text).is_none(), "{text}");
        }
        assert_eq!(
            classify("Read the source files:\n```\nimplement code\n```")
                .unwrap()
                .form,
            1
        );
    }

    #[test]
    fn ordinary_build_and_run_need_writer_capability_not_a_form_menu() {
        for draft in [
            "Build the project.",
            "Compile src/main.rs and run the program.",
            "빌드하고 실행해줘",
            "프로젝트를 빌드하고 프로그램을 실행해줘.",
            "Read src/main.rs; build the project.",
            "Create main.c, edit its value, then compile and run the program.",
            "코드 파일을 만들고 수정한 뒤 빌드하고 실행해줘.",
            "Run `cargo build`.",
            "Execute `./app`.",
            "`cargo test` 실행해줘.",
            "Run \"cargo test --offline\".",
        ] {
            assert_eq!(
                classify(draft).unwrap().operation,
                Operation::Implement,
                "{draft}"
            );
            for manual in [false, true] {
                for policy in [None, Some(Operation::Plan), Some(Operation::Review)] {
                    let mut app = app();
                    app.planning.agent = Agent::Compatible;
                    app.lead.auto_form = !manual;
                    if let Some(policy) = policy {
                        app.planning.set_operation(policy);
                    }
                    let form = app.form();
                    app.open_editor(store::EditorField::Request);
                    app.input = draft.into();
                    app.cursor = draft.len();
                    app.submit_request();
                    assert_eq!((&*app.input, app.cursor), (draft, draft.len()));
                    if manual {
                        assert_eq!(app.form(), form, "manual appearance: {draft}");
                    }
                    assert_eq!(
                        app.planning.operation,
                        policy.unwrap_or(Operation::Implement)
                    );
                    assert!(!app.execution.busy(), "classification executed a command");
                    let snapshot = app.snapshot("/workspace");
                    assert!(app.planning.command("new", true, &snapshot, &[], None));
                    let Some(Request::Preview(input)) = app.planning.pending.take() else {
                        panic!("ordinary request did not freeze: {draft}");
                    };
                    assert_eq!(input.instruction(), draft);
                    assert_eq!(input.operation, policy.unwrap_or(Operation::Implement));
                    assert!(
                        input.work.is_none(),
                        "intent manufactured command authority"
                    );
                }
            }
        }
    }

    #[test]
    fn future_commands_literals_and_readonly_requests_do_not_grant_execution() {
        for draft in [
            "Plan how to build the project.",
            "Design the project build plan.",
            "프로젝트 빌드 계획을 세워줘.",
            "Read the source and explain how to compile the program.",
            "Read the source and explain what this test does.",
            "프로젝트 실행 오류를 분석해줘.",
            "Build the project; do not change files.",
            "읽기 전용으로 프로젝트 빌드 방법을 설명해줘.",
            "Run `cargo build`; read only.",
            "`cargo test` 실행해줘. 읽기 전용.",
        ] {
            assert_eq!(
                classify(draft).unwrap().operation,
                Operation::Plan,
                "{draft}"
            );
        }
        for draft in [
            "Explain how to run the program.",
            "What does this test do?",
            "The project build is failing.",
            "프로젝트 빌드가 실패했어.",
            "\"Build the project\"",
            "> 빌드하고 실행해줘",
            "```\ncompile src/main.rs\n```",
            "Tool output: run the program",
            "Do not build the project.",
            "프로젝트를 빌드하지 마.",
            "Continue.",
            "Explain `cargo build`.",
            "Explain how to run `cargo build`.",
            "Describe the command `cargo test`.",
            "Do not run `cargo build`.",
            "`cargo test` 실행하지 마.",
            "`Run cargo build`",
            "Output: Run `cargo build`",
            "> Run `cargo build`",
            "```\nRun `cargo build`\n```",
        ] {
            assert!(classify(draft).is_none(), "{draft}");
        }
        assert_eq!(
            classify("Plan the project; then build the program.")
                .unwrap()
                .operation,
            Operation::Implement
        );
    }

    #[test]
    fn explicit_file_changes_leave_automatic_readonly_roles() {
        for text in [
            "Create `hello.py`",
            "Edit \"src/main.rs\"",
            "`새 파일.py`를 생성해줘.",
            "Save this exact text as notes.custom.",
            "Create handoff.md",
            "handoff.md를 생성해줘.",
            "Create `Dockerfile`",
            "Save the handoff file.",
        ] {
            let mut app = app();
            app.planning.agent = Agent::Compatible;
            app.input = "Read src/main.rs".into();
            assert!(app.evolve_request());
            assert_eq!(app.planning.operation, Operation::Plan);
            app.input = text.into();
            app.cursor = app.input.len();
            app.submit_request();
            assert_eq!(app.planning.operation, Operation::Implement, "{text}");
            assert!(app.pending_request.is_some(), "{text}");
        }
        for text in [
            "\"Create hello.py\"",
            "`Edit src/main.rs`",
            "Provider: create hello.py",
            "Do not create `hello.py`.",
        ] {
            assert!(classify(text).is_none(), "{text}");
        }
        assert_eq!(
            classify("Prepare a handoff summary.").unwrap().operation,
            Operation::Plan
        );
        assert_eq!(
            classify("Read `src/main.rs` without writing.")
                .unwrap()
                .operation,
            Operation::Plan
        );
    }

    #[test]
    fn only_submit_routes_before_save_and_frozen_input_without_an_extra_dispatch() {
        let mut app = app();
        let text = "Create a new Python code file.";
        app.update(Event::Paste(text.into()));
        let caret = app.cursor;
        assert_eq!(app.form(), 0);
        assert!(app.planning.pending.is_none() && app.pending_request.is_none());
        app.command("help");
        assert_eq!(app.overlay, Some(Overlay::Help));
        assert_eq!(app.form(), 0);
        app.close_overlay();
        app.submit_request();
        assert_eq!(app.form(), 3);
        assert!(app.lead.auto_form);
        assert_eq!((app.input.as_str(), app.cursor), (text, caret));
        assert!(app.pending_request.is_some() && app.save_requested);
        assert!(app.planning.pending.is_none() && !app.execution.busy());
        let snapshot = app.snapshot("/workspace");
        assert_eq!((snapshot.form, snapshot.auto_form), (3, true));
        assert_eq!(snapshot.request_text(), text);
        assert!(app.planning.command("new", true, &snapshot, &[], None));
        let Some(Request::Preview(input)) = app.planning.pending.take() else {
            panic!("missing frozen input");
        };
        assert_eq!(input.task.form, 3);
        assert!(input.task.auto_form);
        assert_eq!(input.task.request_text(), text);
        assert_eq!(input.operation, Operation::Implement);
        assert_eq!(app.planning.owner().unwrap().form, 3);
        assert!(!app.execution.busy());
    }

    #[test]
    fn first_saved_identity_preserves_requested_capability_and_explicit_readonly_policy() {
        for policy in [None, Some(Operation::Plan), Some(Operation::Review)] {
            let mut app = App::new("/workspace".into(), 120, 36);
            app.storage = StorageState::Saved;
            app.planning.preference_event(Ok(None));
            app.planning.agent = Agent::Compatible;
            if let Some(policy) = policy {
                app.planning.set_operation(policy);
            }
            let text = "Generate three project files; propose their paths and preserve literal CRLF/Unicode and an empty file.";
            app.input = text.into();
            app.submit_request();
            let mut snapshot = app.snapshot("/workspace");
            snapshot.id = 7;
            app.store_event(store::Event::Saved(snapshot.clone()));
            assert!(app.planning.command("new", true, &snapshot, &[], None));
            let Some(Request::Preview(input)) = app.planning.pending.take() else {
                panic!("missing first saved request preparation");
            };
            assert_eq!(input.task.id, 7);
            assert_eq!(input.task.request_text(), text);
            assert_eq!(input.operation, policy.unwrap_or(Operation::Implement));
        }
    }

    #[test]
    fn mixed_discovery_edit_freezes_writing_after_automatic_readonly_request() {
        let mut app = app();
        app.input = "Read the project source files.".into();
        app.submit_request();
        assert_eq!((app.form(), app.planning.operation), (1, Operation::Plan));
        app.pending_request = None;
        let text = "List the project files, read the generated Python source, and edit only its value using the observed digest.";
        app.input = text.into();
        app.submit_request();
        assert_eq!((app.form(), app.lead.auto_form), (3, true));
        assert_eq!(app.planning.operation, Operation::Implement);
        let snapshot = app.snapshot("/workspace");
        assert!(app.planning.command("new", true, &snapshot, &[], None));
        let Some(Request::Preview(input)) = app.planning.pending.take() else {
            panic!("missing frozen mixed discovery/edit input");
        };
        assert_eq!(input.task.request_text(), text);
        assert_eq!(input.task.form, 3);
        assert_eq!(input.operation, Operation::Implement);
        assert_eq!(app.planning.owner().unwrap().form, 3);
        assert!(!app.execution.busy());
    }

    #[test]
    fn manual_pin_and_eevee_mode_respect_explicit_readonly_during_automatic_selection() {
        let mut app = app();
        app.apply_execution_action(execution::Action::SelectForm(3));
        app.input = "Read the project source files.".into();
        app.submit_request();
        assert_eq!((app.form(), app.lead.auto_form), (3, false));
        app.pending_request = None;
        app.apply_execution_action(execution::Action::SelectForm(0));
        app.planning.set_operation(Operation::Plan);
        assert_eq!((app.form(), app.lead.auto_form), (0, true));
        app.input = "Create a code file.".into();
        app.submit_request();
        assert_eq!((app.form(), app.lead.auto_form), (3, true));
        assert_eq!(app.planning.operation, Operation::Plan);
        app.pending_request = None;
        app.apply_execution_action(execution::Action::SelectForm(0));
        app.planning.set_operation(Operation::Review);
        app.input = "Implement the code feature.".into();
        app.submit_request();
        assert_eq!((app.form(), app.planning.operation), (7, Operation::Review));
    }

    #[test]
    fn form_choice_and_restore_preserve_explicit_readonly_policy() {
        let mut app = app();
        app.planning.set_operation(Operation::Plan);
        app.apply_execution_action(execution::Action::SelectForm(3));
        assert_eq!(app.planning.operation, Operation::Plan);
        app.planning.set_operation(Operation::Review);
        let snapshot = app.snapshot("/workspace");
        app.load_task(snapshot);
        assert_eq!(app.planning.operation, Operation::Review);
        assert!(!app.lead.auto_form);
    }

    #[test]
    fn every_fixed_form_can_discover_create_and_edit_without_changing_appearance() {
        for form in 1..9 {
            for (text, operation) in [
                ("Read the current project source files.", Operation::Plan),
                ("Create `hello.py`.", Operation::Implement),
                ("구현해줘", Operation::Implement),
                ("Implement it.", Operation::Implement),
                ("Edit `src/main.rs`.", Operation::Implement),
                (
                    "Write project documentation to README.md.",
                    Operation::Implement,
                ),
                ("Review the code and fix its error.", Operation::Implement),
                (
                    "Diagnose the failure and repair the code.",
                    Operation::Implement,
                ),
                // A fresh unclassified request has tools, like other agents.
                ("Continue.", Operation::Implement),
                ("Read only.", Operation::Plan),
            ] {
                let mut app = app();
                app.apply_execution_action(execution::Action::SelectForm(form));
                app.input = text.into();
                app.submit_request();
                assert_eq!((app.form(), app.lead.auto_form), (form, false), "{text}");
                let snapshot = app.snapshot("/workspace");
                app.planning.command("new", true, &snapshot, &[], None);
                let Some(Request::Preview(input)) = app.planning.pending.take() else {
                    panic!("missing preparation for form {form}: {text}");
                };
                assert_eq!(input.operation, operation, "form {form}: {text}");
            }
        }
        assert_eq!(
            classify("Review src/main.rs").unwrap().operation,
            Operation::Plan
        );
        assert_eq!(
            classify("Perform an independent code review.")
                .unwrap()
                .operation,
            Operation::Review
        );
    }

    #[test]
    fn manual_form_stays_fixed_while_literal_readonly_and_documentation_choose_capability() {
        let mut app = app();
        app.apply_execution_action(execution::Action::SelectForm(8));
        app.input = "Write project documentation to README.md.".into();
        app.submit_request();
        assert_eq!(
            (app.form(), app.lead.auto_form, app.planning.operation),
            (8, false, Operation::Implement)
        );
        app.pending_request = None;
        app.apply_execution_action(execution::Action::SelectForm(3));
        app.planning.set_operation(Operation::Implement);
        app.input = "Read the source files without changing anything.".into();
        app.submit_request();
        assert_eq!(
            (app.form(), app.lead.auto_form, app.planning.operation),
            (3, false, Operation::Plan)
        );
    }

    #[test]
    fn pending_busy_cancel_and_approval_keep_frozen_owner_and_future_draft() {
        for state in 0..4 {
            let mut app = app();
            app.input = "Create a code file.".into();
            app.submit_request();
            let owner = app.attribution();
            let snapshot = app.snapshot("/workspace");
            if state != 0 {
                app.pending_request = None;
                app.planning.command("new", true, &snapshot, &[], None);
            }
            if state == 2 {
                app.planning.cancelling = true;
            }
            if state == 3 {
                app.planning.request = Some(providers::Prompt {
                    arrival: providers::approval_arrival(),
                    request_id: serde_json::json!("exact-owner"),
                    thread_id: "owned".into(),
                    turn_id: "turn".into(),
                    item_id: "item".into(),
                    kind: providers::PromptKind::Questions(Vec::new()),
                });
            }
            app.input = "Review the code changes independently.".into();
            app.cursor = 8;
            app.submit_request();
            assert_eq!(app.attribution(), owner);
            assert_eq!(
                (app.input.as_str(), app.cursor),
                ("Review the code changes independently.", 8)
            );
            if state != 0 {
                assert!(app.pending_request.is_none());
                assert_eq!(app.planning.owner(), Some(&owner));
            }
        }
    }

    #[test]
    fn canonical_mode_hydrates_without_pinning_and_survives_roster_merging() {
        for auto_form in [false, true] {
            let mut app = app();
            let mut snapshot = store::Snapshot::empty("/workspace".into());
            snapshot.id = 7;
            snapshot.form = 1;
            snapshot.auto_form = auto_form;
            app.load_task(snapshot);
            assert_eq!((app.form(), app.lead.auto_form), (1, auto_form));
            assert_eq!(app.snapshot("/workspace").auto_form, auto_form);
            app.party = vec![
                app.lead.clone(),
                store::Partner {
                    id: 2,
                    auto_form: !auto_form,
                    ..Default::default()
                },
            ];
            app.request_lead(2);
            assert_eq!(app.lead.auto_form, !auto_form);
            app.request_lead(1);
            assert_eq!((app.form(), app.lead.auto_form), (1, auto_form));
            app.lead.auto_form = !auto_form;
            assert_eq!(app.partners()[0].auto_form, !auto_form);
        }
    }

    #[test]
    fn evolution_preserves_selected_route_and_ignores_saved_specialist_pairings() {
        let mut app = app();
        app.planning.agent = Agent::Compatible;
        app.planning.selection = ModelSelection {
            model: Some("chosen/model".into()),
            effort: Some("high".into()),
        };
        let selection = app.planning.selection.clone();
        app.records.loaded = true;
        app.records
            .snapshot
            .pairings
            .push(crate::workspace::records::Pairing {
                partner: app.lead.id,
                form: 3,
                agent: Agent::OpenAI,
                selection: ModelSelection {
                    model: Some("must-not-activate".into()),
                    effort: None,
                },
            });
        app.planning.thread_id = Some("opaque-old-role".into());
        app.input = "Create a code file.".into();
        app.submit_request();
        assert_eq!(app.planning.agent, Agent::Compatible);
        assert_eq!(app.planning.selection, selection);
        app.records_restore_pairing();
        assert_eq!(app.planning.agent, Agent::Compatible);
        assert_eq!(app.planning.selection, selection);
        assert!(app.planning.thread_id.is_none());
        assert!(app.evolution.transition.unwrap().fresh);
        assert!(!app.pending_request.unwrap().1);
    }
}
