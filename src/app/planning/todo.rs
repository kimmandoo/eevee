//! Display-only projections of qualified provider observations. Raw items stay untouched.
use std::{fmt::Write, sync::Arc};

use serde_json::Value;

use super::{Item, ItemKind};
use crate::{
    providers::Agent,
    workspace::progress::{Status, Step, normalize, validate_steps},
};

pub(super) struct Observation {
    pub source: &'static str,
    steps: Vec<Step>,
    explanation: String,
}

impl Observation {
    pub fn read(item: &Item) -> Option<Self> {
        let agent = item.status.as_ref()?.agent;
        if agent.is_direct() && item.kind == ItemKind::Plan && item.item_id.starts_with("todo-") {
            let value: Value = serde_json::from_str(&item.text).ok()?;
            if value["tool"] != "todo" || value["result"]["status"] != "success" || !item.complete {
                return None;
            }
            let steps = serde_json::from_value(value["result"]["steps"].clone()).ok()?;
            return Self::new("native todo", steps, String::new());
        }
        if matches!(agent, Agent::OpenCode | Agent::Gemini | Agent::Devin)
            && item.kind == ItemKind::Plan
            && item.item_id.starts_with("acp-plan-")
        {
            let value: Value = serde_json::from_str(&item.text).ok()?;
            return Self::new("ACP plan", normalize("acp", &value).ok()?, String::new());
        }
        if agent == Agent::Claude && item.kind == ItemKind::Tool && item.complete {
            let value: Value = serde_json::from_str(&item.text).ok()?;
            if value["provider"] != "Claude Agent SDK"
                || value["tool"] != "TodoWrite"
                || value["status"] != "completed"
                || value["is_error"] == true
            {
                return None;
            }
            return Self::new(
                "SDK todo",
                normalize("claude", value.get("progress")?).ok()?,
                String::new(),
            );
        }
        if agent == Agent::Codex && item.kind == ItemKind::Plan && item.item_id == "eevee-turn-plan"
        {
            // This adapter-owned item uses a readable explanation followed by
            // normalized status lines. Only consume that exact trailing suffix.
            let mut explanation = item.text.as_str();
            let mut steps = Vec::new();
            while let Some((prefix, line)) = explanation.rsplit_once('\n') {
                let Some((status, label)) = line.split_once(": ") else {
                    break;
                };
                let status = match status {
                    "pending" | "not_started" => Status::Pending,
                    "in_progress" | "inProgress" | "active" => Status::Active,
                    "completed" | "done" => Status::Done,
                    "blocked" => Status::Blocked,
                    "dropped" | "cancelled" => Status::Dropped,
                    _ => break,
                };
                steps.push(Step {
                    label: label.into(),
                    phase: None,
                    status,
                });
                explanation = prefix;
            }
            if steps.is_empty() {
                return None;
            }
            steps.reverse();
            return Self::new("Codex plan", steps, explanation.into());
        }
        None
    }

    fn new(source: &'static str, steps: Vec<Step>, explanation: String) -> Option<Self> {
        validate_steps(&steps).ok()?;
        Some(Self {
            source,
            steps,
            explanation,
        })
    }

    pub fn display(&self, updates: usize, item: &Item) -> String {
        let done = self
            .steps
            .iter()
            .filter(|step| step.status == Status::Done)
            .count();
        let mut text = format!("Todo progress · {done}/{} done", self.steps.len());
        let active = self
            .steps
            .iter()
            .filter(|step| step.status == Status::Active)
            .count();
        let blocked = self
            .steps
            .iter()
            .filter(|step| step.status == Status::Blocked)
            .count();
        if active > 0 {
            let _ = write!(text, " · {active} active");
        }
        if blocked > 0 {
            let _ = write!(text, " · {blocked} blocked");
        }
        let _ = write!(
            text,
            " · {}\n{updates} update{}",
            self.source,
            if updates == 1 { "" } else { "s" }
        );
        if let Some(owner) = item.owner.as_ref() {
            let label =
                crate::workspace::handoff::sanitize(&crate::store::Attribution::label(Some(owner)));
            let _ = write!(text, " · {label}");
        }
        for step in &self.steps {
            let marker = match step.status {
                Status::Pending => "[ ]",
                Status::Active => "[>]",
                Status::Done => "[x]",
                Status::Blocked => "[!]",
                Status::Dropped => "[-]",
            };
            let _ = write!(text, "\n{marker} {}", step.label);
            if let Some(phase) = &step.phase {
                let _ = write!(text, " · {phase}");
            }
        }
        if !self.explanation.is_empty() {
            let _ = write!(text, "\n{}", self.explanation);
        }
        if let Some(status) = &item.status
            && status
                .outcome
                .is_some_and(|outcome| outcome != crate::providers::Outcome::Completed)
        {
            let _ = write!(
                text,
                "\nRequest outcome: {}\nReason: {}",
                status.outcome_label().unwrap_or("Unknown"),
                status.reason
            );
        }
        text
    }
}

pub(super) fn same_request(left: &Item, right: &Item) -> bool {
    Arc::ptr_eq(&left.request, &right.request)
        && left.owner == right.owner
        && left
            .status
            .as_ref()
            .zip(right.status.as_ref())
            .is_some_and(|(a, b)| {
                a.generation == b.generation && a.task_id == b.task_id && a.agent == b.agent
            })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::planning::{RequestStatus, State};
    use crate::providers::Operation;
    use serde_json::json;

    fn item(agent: Agent, request: &Arc<str>, id: &str, kind: ItemKind, text: String) -> Item {
        Item {
            thread_id: "thread".into(),
            turn_id: id.into(),
            item_id: id.into(),
            kind,
            text,
            complete: true,
            request: Arc::clone(request),
            owner: None,
            status: Some(Arc::new(RequestStatus {
                generation: 7,
                task_id: 1,
                agent,
                operation: Operation::Plan,
                model: None,
                transport: None,
                outcome: None,
                reason: String::new(),
                public_context: None,
            })),
        }
    }

    fn native(request: &Arc<str>, id: &str, done: bool) -> Item {
        item(Agent::Compatible, request, id, ItemKind::Plan, json!({
            "tool":"todo", "arguments":{"operation":"view","label":"","steps":[]},
            "result":{"status":"success","steps":[
                {"label":"오류 조사 e\u{301}","status":if done {"done"} else {"active"},"phase":"조사"},
                {"label":"Verify tests","status":"pending"}
            ]}
        }).to_string())
    }

    fn transcript(state: &mut State, width: u16, verbose: bool) -> String {
        state
            .response_rows(width, 4096, verbose)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn todo_updates_coalesce_only_display_and_keep_final_answer_and_raw_inspection() {
        for width in [60, 120] {
            let request: Arc<str> = "Inspect error".into();
            let mut state = State::default();
            state.items.push(native(&request, "todo-0-0", false));
            state.items.push(native(&request, "todo-1-0", true));
            state.items.push(native(&request, "todo-2-0", true));
            state.items.push(item(
                Agent::Compatible,
                &request,
                "answer",
                ItemKind::Message,
                "Final answer stays visible".into(),
            ));
            let before = state
                .items
                .iter()
                .map(|item| item.text.clone())
                .collect::<Vec<_>>();
            let text = transcript(&mut state, width, false);
            assert_eq!(text.matches("Todo progress").count(), 1, "{text}");
            assert!(
                text.contains("1/2 done") && text.contains("3 updates"),
                "{text}"
            );
            assert!(
                text.contains("[x] 오류 조사 e\u{301}") && text.contains("[ ] Verify tests"),
                "{text}"
            );
            assert!(!text.contains("provider claims"), "{text}");
            assert!(
                text.contains("Final answer stays visible") && !text.contains("\"arguments\""),
                "{text}"
            );
            let raw = transcript(&mut state, width, true);
            assert_eq!(raw.matches("\"arguments\"").count(), 3, "{raw}");
            assert!(
                raw.contains("todo-0-0") && raw.contains("todo-2-0"),
                "{raw}"
            );
            assert_eq!(
                state
                    .items
                    .iter()
                    .map(|item| item.text.clone())
                    .collect::<Vec<_>>(),
                before
            );
            assert_eq!(transcript(&mut state, width, false), text);
        }
    }

    #[test]
    fn todo_projection_preserves_refusal_malformed_ordinary_tool_and_identity_boundaries() {
        let request: Arc<str> = "same request text".into();
        let mut state = State::default();
        state.items.push(native(&request, "todo-0-0", false));
        state.items.push(item(
            Agent::Compatible,
            &request,
            "todo-1-0",
            ItemKind::Plan,
            json!({"tool":"todo","arguments":{"operation":"start","label":"missing","steps":[]},
                "result":{"status":"refused","message":"Existing exact label required"}})
            .to_string(),
        ));
        state.items.push(item(Agent::Compatible, &request, "todo-2-0", ItemKind::Plan,
            json!({"tool":"todo","result":{"status":"success","steps":[{"label":"Malformed marker","status":"unknown"}]}}).to_string()));
        state.items.push(item(
            Agent::Compatible,
            &request,
            "ordinary",
            ItemKind::Tool,
            "ordinary duplicate".into(),
        ));
        state.items.push(item(
            Agent::Compatible,
            &request,
            "ordinary-2",
            ItemKind::Tool,
            "ordinary duplicate".into(),
        ));
        state.items.push(native(&request, "todo-3-0", true));
        // Equal bytes are not the immutable request identity.
        state
            .items
            .push(native(&Arc::from(request.as_ref()), "todo-4-0", false));
        let mut other_owner = native(&request, "todo-5-0", false);
        other_owner.owner = Some(crate::store::Attribution {
            partner: 2,
            name: "Other".into(),
            form: 0,
        });
        state.items.push(other_owner);
        let mut other_generation = native(&request, "todo-6-0", false);
        Arc::make_mut(other_generation.status.as_mut().unwrap()).generation = 8;
        state.items.push(other_generation);
        let mut other_task = native(&request, "todo-7-0", false);
        Arc::make_mut(other_task.status.as_mut().unwrap()).task_id = 2;
        state.items.push(other_task);
        let text = transcript(&mut state, 120, false);
        assert_eq!(text.matches("Todo progress").count(), 5, "{text}");
        let unwrapped = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            text.contains("refused")
                && unwrapped.contains("Existing exact label required")
                && text.contains("Malformed marker"),
            "{text}"
        );
        assert_eq!(text.matches("ordinary duplicate").count(), 2, "{text}");
    }

    #[test]
    fn qualified_acp_codex_and_sdk_snapshots_share_todo_display_not_generic_json() {
        let request: Arc<str> = "Plan".into();
        for (agent, id, kind, text) in [
            (Agent::OpenCode, "acp-plan-7", ItemKind::Plan, json!([
                {"content":"Inspect source","status":"completed"},{"content":"Verify tests","status":"in_progress"}
            ]).to_string()),
            (Agent::Codex, "eevee-turn-plan", ItemKind::Plan, "Keep the explanation\ncompleted: Inspect source\nin_progress: Verify tests".into()),
            (Agent::Claude, "sdk-todo", ItemKind::Tool, json!({
                "provider":"Claude Agent SDK","tool":"TodoWrite","status":"completed","is_error":false,
                "result":"Observed provider result","progress":{"todos":[
                    {"content":"Inspect source","status":"completed"},{"content":"Verify tests","status":"in_progress"}
                ]}
            }).to_string()),
        ] {
            let mut state = State::default();
            state.items.push(item(agent, &request, id, kind, text.clone()));
            let display = transcript(&mut state, 60, false);
            assert!(
                display.contains("Todo progress · 1/2 done")
                    && display.contains("[>] Verify tests"),
                "{display}"
            );
            if agent == Agent::Codex {
                assert!(display.contains("Keep the explanation"));
            }
            let raw = transcript(&mut state, 60, true);
            assert!(!raw.contains("Todo progress"), "{raw}");
            assert_eq!(state.items[0].text, text);
            assert_eq!(transcript(&mut state, 60, false), display);
            state.items[0].item_id = "unqualified".into();
            state.items[0].kind = ItemKind::Message;
            state.response_cache.invalidate();
            state.response_cache.item_changed(0);
            assert!(Observation::read(&state.items[0]).is_none());
            assert!(!transcript(&mut state, 60, false).contains("[>] Verify tests"));
        }
    }

    #[test]
    fn sdk_incomplete_failed_and_malformed_results_are_never_collapsed() {
        let request: Arc<str> = "Plan".into();
        let mut state = State::default();
        for (index, status, complete, steps) in [
            (
                0,
                "requested",
                false,
                json!([{"content":"Inspect","status":"pending"}]),
            ),
            (
                1,
                "failed",
                true,
                json!([{"content":"Inspect","status":"pending"}]),
            ),
            (
                2,
                "completed",
                true,
                json!([{"content":"Malformed marker","status":"unknown"}]),
            ),
        ] {
            let mut observation = item(Agent::Claude, &request, &format!("sdk-{index}"), ItemKind::Tool,
                json!({"provider":"Claude Agent SDK","tool":"TodoWrite","status":status,"is_error":status=="failed",
                    "result":"Refusal/incomplete evidence","progress":{"todos":steps}}).to_string());
            observation.complete = complete;
            state.items.push(observation);
        }
        let text = transcript(&mut state, 120, false);
        assert!(!text.contains("Todo progress"), "{text}");
        assert_eq!(
            text.matches("Refusal/incomplete evidence").count(),
            3,
            "{text}"
        );
        assert!(text.contains("Malformed marker"), "{text}");
    }

    #[test]
    fn todo_cached_projection_changes_after_same_item_update_and_resize() {
        let request: Arc<str> = "Plan".into();
        let mut state = State::default();
        state.items.push(native(&request, "todo-0-0", false));
        assert!(transcript(&mut state, 120, false).contains("[>] 오류 조사"));
        state.items[0].text = native(&request, "todo-0-0", true).text;
        state.response_cache.item_changed(0);
        let updated = transcript(&mut state, 60, false);
        assert!(
            updated.contains("[x] 오류 조사") && updated.contains("1/2 done"),
            "{updated}"
        );
        assert!(!updated.contains("[>] 오류 조사"), "{updated}");
        state.items[0].text = json!({"tool":"todo","result":{"status":"refused","message":"Refusal must replace stale projection"}}).to_string();
        state.response_cache.item_changed(0);
        let refused = transcript(&mut state, 120, false);
        assert!(
            refused.contains("Refusal must replace stale projection")
                && !refused.contains("Todo progress"),
            "{refused}"
        );
    }

    #[test]
    fn todo_settlement_keeps_terminal_failure_reason_visible_after_cached_success() {
        let request: Arc<str> = "Plan".into();
        let mut state = State::default();
        state.items.push(native(&request, "todo-0-0", false));
        assert!(transcript(&mut state, 120, false).contains("Todo progress"));
        let status = Arc::make_mut(state.items[0].status.as_mut().unwrap());
        status.outcome = Some(crate::providers::Outcome::Failed);
        status.reason = "Transport stopped after the observed todo; no replay".into();
        state.response_cache.settlement_changed(0);
        let text = transcript(&mut state, 120, false);
        assert!(
            text.contains("Request outcome: Failed")
                && text.contains("Transport stopped after the observed todo; no replay"),
            "{text}"
        );
    }

    #[test]
    fn compact_todo_metadata_keeps_the_active_step_near_the_top() {
        let request: Arc<str> = "Plan".into();
        let mut state = State::default();
        let mut observation = native(&request, "todo-0-0", false);
        observation.owner = Some(crate::store::Attribution {
            partner: 1,
            name: "Lead".into(),
            form: 0,
        });
        state.items.push(observation);
        let rows = state
            .response_display_rows(60, 18, false)
            .map(|row| row.text.to_owned())
            .collect::<Vec<_>>();
        assert!(rows[0].contains("native todo"), "{rows:?}");
        assert!(
            rows[1].contains("1 update") && rows[1].contains("Lead"),
            "{rows:?}"
        );
        assert!(
            rows.iter().take(4).any(|row| row.starts_with("[>]")),
            "{rows:?}"
        );
        assert!(!rows.join("\n").contains("provider claims"), "{rows:?}");
    }

    #[test]
    fn todo_projection_and_raw_inspection_keep_source_driven_file_effects() {
        let request: Arc<str> = "Implement".into();
        let mut state = State::default();
        state.items.push(native(&request, "todo-0-0", false));
        state.items.push(native(&request, "todo-1-0", true));
        let source = "src/main.rs\nold line\nnew line\n";
        state.items.push(item(
            Agent::Compatible,
            &request,
            "file-preview",
            ItemKind::File(crate::providers::FilePhase::Generating),
            source.into(),
        ));
        state.file_growth.insert(
            "file-preview".into(),
            (source.find("new line").unwrap(), std::time::Instant::now()),
        );
        for verbose in [false, true, false] {
            let rows = state
                .response_display_rows(60, 18, verbose)
                .collect::<Vec<_>>();
            assert_eq!(
                rows.iter()
                    .filter(|row| row.todo && row.text.starts_with("Todo progress"))
                    .count(),
                usize::from(!verbose)
            );
            assert!(rows.iter().any(|row| {
                row.kind == ItemKind::File(crate::providers::FilePhase::Generating)
                    && row.text == "new line"
                    && row.fresh
                    && row.writing_edge
                    && !row.todo
            }));
            assert!(!rows.iter().any(|row| row.text == "old line" && row.fresh));
        }
        assert_eq!(state.items[2].text, source);
    }
}
