//! EV18: local intent at an explicit ordinary send, never a model classifier.
use super::App;
use crate::{i18n::Message, providers::Operation};

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

#[derive(Default)]
pub struct State {
    pub transition: Option<Transition>,
    pub popup_shown: bool,
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
            "architecture",
            "implementation",
            "approach",
            "migration",
            "feature",
            "system",
            "api",
            "change",
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
            "make",
            "edit",
            "modify",
            "replace",
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
            "수정",
            "고쳐",
            "고치",
            "바꿔",
            "바꾸",
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
        ],
    },
    Rule {
        form: 2,
        operation: Operation::Plan,
        reason: Message::CoreEvolutionVerification,
        actions: &["run", "execute", "verify", "test", "실행", "검증", "테스트"],
        objects: &[
            "command",
            "test",
            "tests",
            "suite",
            "build",
            "check",
            "명령",
            "테스트",
            "빌드",
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

fn source_path(text: &str) -> bool {
    text.split_whitespace().any(|token| {
        let token = token.trim_matches([':', ',', ';', '(', ')']);
        !token.contains("://")
            && (token == "."
                || token.contains('/')
                || [
                    ".rs", ".py", ".ts", ".tsx", ".js", ".jsx", ".toml", ".json", ".yaml", ".yml",
                    ".md", ".txt", ".go", ".c", ".cpp", ".h", ".java", ".sh",
                ]
                .iter()
                .any(|extension| token.trim_end_matches('.').ends_with(extension)))
    })
}

fn task_object(text: &str, rule: &Rule) -> bool {
    any(text, rule.objects) || matches!(rule.form, 1 | 3 | 6 | 7) && source_path(text)
}

/// Strip literal/source regions before matching. This bounded copy never enters
/// a provider packet; the original draft remains the sole submitted instruction.
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
        for ch in line.chars() {
            if let Some(end) = quote {
                if ch == end {
                    quote = None;
                    result.push(' ');
                }
            } else if matches!(ch, '`' | '"' | '“' | '‘')
                || ch == '\'' && !previous.is_alphanumeric()
            {
                quote = Some(match ch {
                    '“' => '”',
                    '‘' => '’',
                    _ => ch,
                });
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
    "without writing",
    "without editing",
    "without running",
    "하지 마",
    "하지마",
    "하지 않",
    "하지 말",
    "금지",
    "말고",
    "아니",
];

pub fn classify(text: &str) -> Option<Selection> {
    if text.len() > super::MAX_GOAL_BYTES {
        return None;
    }
    let literal = instruction(text);
    let readonly = any(&literal, READONLY);
    let clauses = || {
        literal
            .split(". ")
            .flat_map(|part| part.split(" but "))
            .flat_map(|part| part.split("말고"))
            .flat_map(|part| part.split(['\n', ';', ',', '!', '?']))
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
    for rule in RULES {
        if clauses().any(|clause| {
            any(clause, rule.actions)
                && (task_object(clause, rule) || matches!(rule.form, 3 | 6) && source_context)
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
            let docs_write = rule.form == 8
                && !readonly
                && clauses().any(|clause| {
                    any(
                        clause,
                        &[
                            "write", "create", "update", "generate", "작성", "생성", "수정", "갱신",
                        ],
                    ) && any(
                        clause,
                        &[
                            "documentation",
                            "docs",
                            "readme",
                            "changelog",
                            "문서",
                            "변경 기록",
                        ],
                    )
                });
            return Some(Selection {
                form: rule.form,
                operation: if docs_write {
                    Operation::Implement
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
    None
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

    /// Called only after ordinary Submit's owner/safety guards, before save.
    pub(super) fn evolve_request(&mut self) -> bool {
        let Some(mut choice) = classify(&self.input) else {
            return false;
        };
        if !self.lead.auto_form {
            // A fixed form stays fixed. Only a literal read-only constraint or
            // that form's documentation-writing task changes ordinary capability.
            if choice.reason != Message::CoreEvolutionReadonly
                && !(self.form() == 8 && choice.form == 8)
            {
                return false;
            }
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
        let from = self.form();
        let fresh = self
            .planning
            .prepare_automatic_operation(choice.operation, from != choice.form);
        if from != choice.form || fresh {
            self.set_form(choice.form);
            self.evolution.transition = Some(Transition {
                from,
                to: choice.form,
                reason: choice.reason,
                fresh,
            });
            self.evolution.popup_shown = false;
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
                Operation::Plan,
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
    fn unclassified_file_request_keeps_ordinary_eevee_coding_available() {
        let mut app = app();
        app.planning.agent = Agent::Compatible;
        app.update(Event::Paste("Save this exact text as notes.custom.".into()));
        assert!(classify(&app.input).is_none());
        app.submit_request();
        assert_eq!(app.form(), 0);
        assert!(app.lead().auto_form);
        assert_eq!(app.planning.operation, Operation::Implement);
        assert!(app.pending_request.is_some());
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
    fn deliberate_form_choice_resets_policy_but_restore_does_not() {
        let mut app = app();
        app.planning.set_operation(Operation::Plan);
        app.apply_execution_action(execution::Action::SelectForm(3));
        assert_eq!(app.planning.operation, Operation::Implement);
        app.planning.set_operation(Operation::Review);
        let snapshot = app.snapshot("/workspace");
        app.load_task(snapshot);
        assert_eq!(app.planning.operation, Operation::Review);
        assert!(!app.lead.auto_form);
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
        assert!(app.records_restore_pairing());
        assert_eq!(app.planning.agent, Agent::Compatible);
        assert_eq!(app.planning.selection, selection);
        assert!(app.planning.thread_id.is_none());
        assert!(app.evolution.transition.unwrap().fresh);
        assert!(!app.pending_request.unwrap().1);
    }
}
