//! Readable projections of provider tool observations. The original item text
//! is unchanged and remains the `/verbose` view; this parses display fields
//! only and never grants, verifies or summarizes an effect as a success.
use serde_json::Value;

use super::{Item, ItemKind};

/// Detail rows kept under a tool title; the full text stays in `/verbose`.
const DETAIL_ROWS: usize = 2;
const DETAIL_COLUMNS: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    Done,
    Failed,
    Unknown,
    Stopped,
}

impl Status {
    /// ASCII markers keep alignment in every terminal and pair text with color.
    pub fn marker(self) -> &'static str {
        match self {
            Status::Running => "[..]",
            Status::Done => "[ok]",
            Status::Failed => "[!!]",
            Status::Unknown => "[?]",
            Status::Stopped => "[--]",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct View {
    pub status: Status,
    /// Tool or command name, e.g. `Read` or `$ cargo test`.
    pub name: String,
    /// Optional short target such as a file path.
    pub target: String,
    /// Extra status words such as `exit 1`.
    pub note: String,
    pub detail: Vec<String>,
    pub hidden: usize,
}

impl View {
    /// Qualified todo snapshots keep their own progress projection. Native
    /// todo results that were refused or malformed are ordinary tool rows here
    /// instead of raw argument JSON; SDK `TodoWrite` keeps its raw evidence.
    pub fn read(item: &Item) -> Option<Self> {
        let mut view = if item.kind == ItemKind::Tool {
            parse(&item.text, item.complete)
        } else if item.kind == ItemKind::Plan && item.item_id.starts_with("todo-") {
            let value = serde_json::from_str::<Value>(&item.text).ok()?;
            // Unknown or malformed "success" stays raw: its unadopted steps
            // are evidence, not a completed progress update.
            if value["tool"] != "todo"
                || !matches!(
                    value["result"]["status"].as_str(),
                    Some("refused" | "denied" | "failed" | "error")
                )
            {
                return None;
            }
            sdk(&value, item.complete)
        } else {
            return None;
        };
        if view.status == Status::Running
            && let Some(outcome) = item.status.as_ref().and_then(|status| status.outcome)
        {
            // Request settlement is not evidence of a tool result or remote
            // cancellation. Retire the live indicator without changing source.
            let stopped = outcome == crate::providers::Outcome::Cancelled;
            view.status = if stopped {
                Status::Stopped
            } else {
                Status::Unknown
            };
            view.note = if stopped {
                "request stopped · tool result unavailable"
            } else {
                "request ended · tool result unavailable"
            }
            .into();
        }
        (view.name != "TodoWrite").then_some(view)
    }

    /// One title row plus bounded indented detail rows for the transcript.
    pub fn display(&self) -> String {
        let mut text = format!("{} {}", self.status.marker(), self.title());
        for line in &self.detail {
            text.push_str("\n     ");
            text.push_str(line);
        }
        if self.hidden > 0 {
            text.push_str(&format!(
                "\n     … {} more line{}",
                self.hidden,
                if self.hidden == 1 { "" } else { "s" }
            ));
        }
        text
    }

    pub fn title(&self) -> String {
        let mut title = self.name.clone();
        for part in [&self.target, &self.note] {
            if !part.is_empty() {
                title.push_str(if title.is_empty() { "" } else { "  " });
                title.push_str(part);
            }
        }
        title
    }

    /// Short label for the side panel: name and the last path component.
    pub fn label(&self) -> String {
        let target = self
            .target
            .rsplit(['/', '\\'])
            .find(|part| !part.is_empty())
            .unwrap_or("");
        let mut label = self.name.clone();
        if !target.is_empty() {
            label.push(' ');
            label.push_str(target);
        }
        if !self.note.is_empty() {
            label.push_str(" · ");
            label.push_str(&self.note);
        }
        label
    }
}

fn single_line(value: &str) -> String {
    let value: String = value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    let value = value.trim();
    if value.chars().count() > DETAIL_COLUMNS {
        let mut clipped: String = value.chars().take(DETAIL_COLUMNS - 1).collect();
        clipped.push('…');
        clipped
    } else {
        value.to_owned()
    }
}

/// Keep the first (or last, for command output) non-empty rows.
fn detail(text: &str, tail: bool) -> (Vec<String>, usize) {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let shown = lines.len().min(DETAIL_ROWS);
    let picked = if tail {
        &lines[lines.len() - shown..]
    } else {
        &lines[..shown]
    };
    (
        picked.iter().map(|line| single_line(line)).collect(),
        lines.len() - shown,
    )
}

fn result_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.as_str().or_else(|| part.get("text")?.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        // Host tool results carry a readable message beside their status.
        Value::Object(_) => value["message"]
            .as_str()
            .or_else(|| value["error"].as_str())
            .unwrap_or("")
            .to_owned(),
        _ => String::new(),
    }
}

fn parse(text: &str, complete: bool) -> View {
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        return match serde_json::from_str::<Value>(trimmed) {
            Ok(value) => sdk(&value, complete),
            Err(_) => View {
                status: if complete {
                    Status::Unknown
                } else {
                    Status::Running
                },
                name: "Tool".into(),
                target: String::new(),
                note: "unreadable observation · /verbose".into(),
                detail: Vec::new(),
                hidden: 0,
            },
        };
    }
    if trimmed.starts_with("Provider-native ")
        && let Some(start) = trimmed.find(": {")
        && let Ok(value) = serde_json::from_str::<Value>(&trimmed[start + 2..])
    {
        return mcp(&value, complete);
    }
    if let Some(rest) = trimmed.strip_prefix("Command: ") {
        return command(rest, complete);
    }
    let (first, rest) = trimmed.split_once('\n').unwrap_or((trimmed, ""));
    let (detail, hidden) = detail(rest, false);
    View {
        status: if complete {
            Status::Unknown
        } else {
            Status::Running
        },
        name: if first.is_empty() {
            "Tool".into()
        } else {
            single_line(first)
        },
        target: String::new(),
        note: if complete {
            "outcome unknown".into()
        } else {
            "awaiting result".into()
        },
        detail,
        hidden,
    }
}

/// Claude Agent SDK and native host observations: argument metadata plus an
/// optional result, which may carry its own status.
fn sdk(value: &Value, complete: bool) -> View {
    let result = &value["result"];
    let outcome = result["status"].as_str().unwrap_or("");
    let state = value["status"].as_str().unwrap_or("");
    let run = &result["run"];
    let run_state = run["state"].as_str().unwrap_or("");
    let tool = value["tool"].as_str().unwrap_or("Tool");
    let is_command = tool == "eevee_run_command";
    let refused = matches!(outcome, "refused" | "denied") || matches!(state, "refused" | "denied");
    // Result/run evidence is authoritative. Transport item completion alone is
    // never proof of success, nor can an outer "completed" hide an unknown run.
    let status = if matches!(outcome, "unknown" | "outcome_unknown" | "incomplete")
        || matches!(state, "unknown" | "outcome_unknown" | "incomplete")
        || matches!(run_state, "OutcomeUnknown" | "Interrupted")
        || run["tree"] == "OwnershipUnknown"
        || complete && matches!(run_state, "AwaitingDispatch" | "Running")
    {
        Status::Unknown
    } else if matches!(outcome, "cancelled" | "canceled" | "stopped")
        || matches!(state, "cancelled" | "canceled" | "stopped")
        || matches!(run_state, "Cancelled" | "Cancelling")
        || run["cancelled"] == true
    {
        Status::Stopped
    } else if refused
        || value["is_error"] == true
        || matches!(state, "failed" | "error")
        || matches!(outcome, "failed" | "error")
        || run_state == "Failed"
        || run["exit_code"].as_i64().is_some_and(|code| code != 0)
    {
        Status::Failed
    } else if outcome == "success"
        || !tool.starts_with("eevee_")
            && state == "completed"
            && outcome.is_empty()
            && !result.is_null()
    {
        Status::Done
    } else if complete
        || matches!(state, "completed" | "incomplete")
        || !outcome.is_empty() && !matches!(outcome, "running" | "requested" | "in_progress")
    {
        Status::Unknown
    } else {
        Status::Running
    };
    let arguments = &value["arguments"];
    let name = match (tool, arguments["operation"].as_str()) {
        ("eevee_read_file", _) => "Read".into(),
        ("eevee_list_files", _) => "List files".into(),
        ("eevee_write_file", _) => match result["kind"].as_str() {
            Some("Create") => "Create".into(),
            Some("Replace") => "Write".into(),
            _ => "Write file".into(),
        },
        ("eevee_edit_file", _) => "Edit".into(),
        ("eevee_delete_file", _) => "Delete".into(),
        ("eevee_rename_file", _) => "Rename".into(),
        ("eevee_structural_search", _) => "Search source".into(),
        ("eevee_run_command", _) => "Run command".into(),
        ("eevee_artifact_list", _) => "List artifacts".into(),
        ("eevee_artifact_read", _) => "Read artifact".into(),
        ("eevee_artifact_search", _) => "Search artifacts".into(),
        ("eevee_artifact_locations", _) => "Artifact locations".into(),
        ("eevee_mcp", _) => "Connected tool".into(),
        ("eevee_delegate", _) => "Delegate".into(),
        ("eevee_web_search", _) => "Web search".into(),
        ("eevee_web_fetch", _) => "Read web page".into(),
        ("todo", Some(operation)) => format!("Todo {}", single_line(operation)),
        ("todo", None) => "Todo".into(),
        _ => single_line(tool),
    };
    let target = value["input"]["path"]
        .as_str()
        .or_else(|| value["input"]["file_path"].as_str())
        .or_else(|| arguments["label"].as_str())
        .or_else(|| arguments["path"].as_str())
        .or_else(|| arguments["directory"].as_str())
        .or_else(|| arguments["cwd"].as_str())
        .map(single_line)
        .filter(|target| !target.is_empty())
        .unwrap_or_else(|| {
            if matches!(
                tool,
                "eevee_list_files" | "eevee_structural_search" | "eevee_run_command"
            ) {
                ".".into()
            } else {
                String::new()
            }
        });
    let mut note = match status {
        Status::Unknown => "outcome unknown".into(),
        Status::Stopped => "stopped".into(),
        Status::Failed if refused => "refused".into(),
        Status::Failed => "failed".into(),
        Status::Running => "awaiting result".into(),
        _ => String::new(),
    };
    let body = if is_command {
        if let Some(code) = run["exit_code"].as_i64() {
            if !note.is_empty() {
                note.push_str(" · ");
            }
            note.push_str(&format!("exit {code}"));
        }
        if result["output_unavailable"] == true || run["output_unavailable"] == true {
            note.push_str(if note.is_empty() {
                "output unavailable"
            } else {
                " · output unavailable"
            });
        } else if result["preview_truncated"] == true
            || result["stdout_display_truncated"] == true
            || result["stderr_display_truncated"] == true
        {
            note.push_str(if note.is_empty() {
                "output preview"
            } else {
                " · output preview"
            });
        }
        let stdout = result["stdout"].as_str().unwrap_or("");
        let stderr = result["stderr"].as_str().unwrap_or("");
        if stdout.is_empty() && stderr.is_empty() {
            result_text(result)
        } else {
            format!("{stdout}\n{stderr}")
        }
    } else {
        result_text(result)
    };
    let (detail, hidden) = detail(&body, is_command);
    View {
        status,
        name,
        target,
        note,
        detail,
        hidden,
    }
}

/// Codex provider-native MCP/dynamic tool calls.
fn mcp(value: &Value, complete: bool) -> View {
    let error = value["error"].as_str().unwrap_or("");
    let state = value["status"].as_str().unwrap_or("");
    let failed = value["success"] == false
        || !value["error"].is_null()
        || matches!(state, "failed" | "error" | "declined" | "denied");
    let status = if matches!(state, "cancelled" | "canceled" | "stopped") {
        Status::Stopped
    } else if matches!(state, "unknown" | "incomplete") {
        Status::Unknown
    } else if failed {
        Status::Failed
    } else if value["success"] == true {
        Status::Done
    } else if complete || state == "completed" {
        Status::Unknown
    } else {
        Status::Running
    };
    let body = if error.is_empty() {
        result_text(&value["text"])
    } else {
        error.to_owned()
    };
    let (detail, hidden) = detail(&body, false);
    let tool = value["tool"].as_str().unwrap_or("tool");
    View {
        status,
        name: match value["server"].as_str() {
            Some(server) => format!("{server}/{tool}"),
            None => tool.to_owned(),
        },
        target: String::new(),
        note: match status {
            Status::Failed => "failed".into(),
            Status::Unknown => "outcome unknown".into(),
            Status::Stopped => "stopped".into(),
            Status::Running => "awaiting result".into(),
            _ => String::new(),
        },
        detail,
        hidden,
    }
}

/// Codex command executions: `Command/Cwd/Status/Exit code` then output.
fn command(rest: &str, complete: bool) -> View {
    let mut parts = rest.splitn(5, '\n');
    let command = parts.next().unwrap_or("");
    let _cwd = parts.next();
    let mut field = |prefix: &str| {
        parts
            .next()
            .and_then(|line| line.strip_prefix(prefix))
            .unwrap_or("")
    };
    let state = field("Status: ");
    let code = field("Exit code: ").trim();
    let output = parts.next().unwrap_or("");
    let failed = matches!(state, "failed" | "declined") || !matches!(code, "" | "0" | "null");
    let status = if matches!(state, "cancelled" | "canceled" | "stopped") {
        Status::Stopped
    } else if matches!(state, "unknown" | "incomplete") {
        Status::Unknown
    } else if failed {
        Status::Failed
    } else if state == "completed" && code == "0" {
        Status::Done
    } else if complete {
        Status::Unknown
    } else {
        Status::Running
    };
    let (detail, hidden) = detail(output, true);
    View {
        status,
        name: format!("$ {}", single_line(command)),
        target: String::new(),
        note: match (status, code) {
            (Status::Stopped, _) => "stopped".into(),
            (Status::Unknown, _) => "outcome unknown".into(),
            (Status::Running, _) => "awaiting result".into(),
            (Status::Failed, "" | "null") => single_line(state),
            (_, "" | "null" | "0") => String::new(),
            (_, code) => format!("exit {}", single_line(code)),
        },
        detail,
        hidden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_result_shows_name_path_and_bounded_preview() {
        let view = parse(
            r#"{"provider":"Claude Agent SDK","tool":"Read","input":{"path":"/w/src/parser.rs","bytes":4},"result":"a\nb\n\nc","is_error":false,"status":"completed"}"#,
            true,
        );
        assert_eq!(view.status, Status::Done);
        assert_eq!(
            view.display(),
            "[ok] Read  /w/src/parser.rs\n     a\n     b\n     … 1 more line"
        );
        assert_eq!(view.label(), "Read parser.rs");
    }

    #[test]
    fn sdk_request_and_error_are_running_and_failed() {
        let running = parse(
            r#"{"tool":"Grep","input":{"bytes":3},"status":"requested"}"#,
            false,
        );
        assert_eq!(running.display(), "[..] Grep  awaiting result");
        let failed = parse(
            r#"{"tool":"Bash","is_error":true,"result":[{"type":"text","text":"denied"}],"status":"failed"}"#,
            true,
        );
        assert_eq!(failed.display(), "[!!] Bash  failed\n     denied");
    }

    #[test]
    fn command_keeps_exit_and_output_tail() {
        let ok = parse(
            "Command: cargo test\nCwd: /w\nStatus: completed\nExit code: 0\nrunning\n...\nok. 3 passed",
            true,
        );
        assert_eq!(
            ok.display(),
            "[ok] $ cargo test\n     ...\n     ok. 3 passed\n     … 1 more line"
        );
        let failed = parse(
            "Command: make\nCwd: /w\nStatus: failed\nExit code: 2\nerror",
            true,
        );
        assert_eq!(failed.status, Status::Failed);
        assert_eq!(failed.label(), "$ make · exit 2");
        let running = parse(
            "Command: sleep 1\nCwd: /w\nStatus: inProgress\nExit code: null\n",
            false,
        );
        assert_eq!(running.display(), "[..] $ sleep 1  awaiting result");
    }

    #[test]
    fn refused_native_todo_is_a_readable_row_not_argument_json() {
        let mut item = Item {
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            item_id: "todo-init".into(),
            kind: ItemKind::Plan,
            text: r#"{"arguments":{"label":"Windows 11 파일 탐색기","operation":"init","steps":[{"label":"a","status":"pending"}]},"result":{"message":"Todo init/append/view require an empty target label","path":"","status":"refused"},"tool":"todo"}"#.into(),
            complete: true,
            request: "fixture".into(),
            owner: None,
            status: None,
        };
        let view = View::read(&item).unwrap();
        assert_eq!(
            view.display(),
            "[!!] Todo init  Windows 11 파일 탐색기  refused\n     Todo init/append/view require an empty target label"
        );
        assert_eq!(view.status, Status::Failed);
        for status in ["success", "unknown", ""] {
            item.text = serde_json::json!({
                "tool": "todo",
                "arguments": {"operation": "init"},
                "result": {"status": status}
            })
            .to_string();
            assert!(View::read(&item).is_none(), "{status}");
        }
    }

    #[test]
    fn mcp_and_plain_text_stay_readable() {
        let mcp = parse(
            r#"Provider-native "mcpToolCall" (public text result only): {"tool":"lookup","server":"docs","status":"completed","success":true,"text":["found"],"error":null}"#,
            true,
        );
        assert_eq!(mcp.display(), "[ok] docs/lookup\n     found");
        let plain = parse("Native file-write activity: declined.", true);
        assert_eq!(
            plain.display(),
            "[?] Native file-write activity: declined.  outcome unknown"
        );
    }

    #[test]
    fn native_operations_are_readable_without_argument_bodies() {
        for (tool, arguments, expected) in [
            (
                "eevee_read_file",
                serde_json::json!({"path":"src/lib.rs"}),
                "[..] Read  src/lib.rs",
            ),
            (
                "eevee_list_files",
                serde_json::json!({"directory":"src"}),
                "[..] List files  src",
            ),
            (
                "eevee_edit_file",
                serde_json::json!({"path":"src/lib.rs","replacements":[{"old_text":"PRIVATE","new_text":"SECRET"}]}),
                "[..] Edit  src/lib.rs",
            ),
            (
                "eevee_write_file",
                serde_json::json!({"path":"new.rs","content":"PRIVATE"}),
                "[..] Write file  new.rs",
            ),
            (
                "eevee_run_command",
                serde_json::json!({"cwd":"","command":{"Direct":{"program":"PRIVATE","args":["SECRET"]}}}),
                "[..] Run command  .",
            ),
        ] {
            let value = serde_json::json!({"tool":tool,"arguments":arguments,"status":"running"});
            assert_eq!(
                parse(&value.to_string(), false).display(),
                format!("{expected}  awaiting result")
            );
        }
        let created = parse(
            r#"{"tool":"eevee_write_file","arguments":{"path":"new.rs"},"status":"completed","result":{"status":"success","kind":"Create","message":"Applied"}}"#,
            true,
        );
        assert_eq!(created.display(), "[ok] Create  new.rs\n     Applied");
    }

    #[test]
    fn item_completion_is_not_success_evidence() {
        for text in [
            r#"{"tool":"Read","status":"completed"}"#,
            r#"{"tool":"Read","result":{"status":"unexpected"},"status":"completed"}"#,
            r#"{"tool":"eevee_read_file","result":{},"status":"completed"}"#,
            r#"{"tool":"eevee_read_file","result":{"status":"unknown"},"status":"completed"}"#,
            r#"{"tool":"eevee_read_file","result":{"status":"success"},"status":"incomplete"}"#,
            r#"{"tool":"Read","arguments":{"content":"PRIVATE"}"#,
            "Command: build\nCwd: /w\nStatus: completed\nExit code: null\n",
            "Command: build\nCwd: /w\nStatus: unexpected\nExit code: 0\n",
            r#"Provider-native "mcpToolCall": {"tool":"lookup","status":"completed","text":["partial"]}"#,
        ] {
            let view = parse(text, true);
            assert_eq!(view.status, Status::Unknown, "{text}");
            assert!(!view.display().contains("[ok]"), "{text}");
            assert!(!view.display().contains("PRIVATE"), "{text}");
        }
    }

    #[test]
    fn command_result_keeps_exit_output_tail_and_non_success_states() {
        let value = serde_json::json!({
            "tool":"eevee_run_command","arguments":{"cwd":"src"},"status":"failed",
            "result":{"status":"error","run":{"state":"Failed","exit_code":2},
                "stdout":"compiling\nrunning","stderr":"error: invalid type\nbuild failed","preview_truncated":true}
        });
        let view = parse(&value.to_string(), true);
        assert_eq!(view.status, Status::Failed);
        assert_eq!(view.detail, ["error: invalid type", "build failed"]);
        assert_eq!(view.hidden, 2);
        assert!(view.title().contains("exit 2 · output preview"));
        for (state, expected) in [
            ("Cancelled", Status::Stopped),
            ("OutcomeUnknown", Status::Unknown),
            ("Interrupted", Status::Unknown),
        ] {
            let value = serde_json::json!({"tool":"eevee_run_command","status":"completed",
                "result":{"status":"success","run":{"state":state,"exit_code":0}}});
            assert_eq!(parse(&value.to_string(), true).status, expected);
        }
        assert_eq!(
            parse(
                "Command: build\nCwd: /w\nStatus: cancelled\nExit code: 0\n",
                true
            )
            .status,
            Status::Stopped
        );
    }
}
