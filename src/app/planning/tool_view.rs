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
}

impl Status {
    /// ASCII markers keep alignment in every terminal and pair text with color.
    pub fn marker(self) -> &'static str {
        match self {
            Status::Running => "[..]",
            Status::Done => "[ok]",
            Status::Failed => "[!!]",
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
    /// Todo observations keep their own progress projection and raw evidence.
    pub fn read(item: &Item) -> Option<Self> {
        (item.kind == ItemKind::Tool)
            .then(|| parse(&item.text, item.complete))
            .filter(|view| view.name != "TodoWrite")
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
        _ => String::new(),
    }
}

fn parse(text: &str, complete: bool) -> View {
    let trimmed = text.trim();
    if trimmed.starts_with('{')
        && let Ok(value) = serde_json::from_str::<Value>(trimmed)
    {
        return sdk(&value, complete);
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
            Status::Done
        } else {
            Status::Running
        },
        name: if first.is_empty() {
            "Tool".into()
        } else {
            single_line(first)
        },
        target: String::new(),
        note: String::new(),
        detail,
        hidden,
    }
}

/// Claude Agent SDK observations: argument metadata plus an optional result.
fn sdk(value: &Value, complete: bool) -> View {
    let failed = value["is_error"] == true || value["status"] == "failed";
    let status = if failed {
        Status::Failed
    } else if complete || value["status"] == "completed" {
        Status::Done
    } else {
        Status::Running
    };
    let (detail, hidden) = detail(&result_text(&value["result"]), false);
    View {
        status,
        name: value["tool"].as_str().unwrap_or("Tool").to_owned(),
        target: value["input"]["path"]
            .as_str()
            .map(single_line)
            .unwrap_or_default(),
        note: if failed {
            "failed".into()
        } else {
            String::new()
        },
        detail,
        hidden,
    }
}

/// Codex provider-native MCP/dynamic tool calls.
fn mcp(value: &Value, complete: bool) -> View {
    let error = value["error"].as_str().unwrap_or("");
    let failed = value["success"] == false || !error.is_empty() || value["status"] == "failed";
    let status = if failed {
        Status::Failed
    } else if complete {
        Status::Done
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
        note: if failed {
            "failed".into()
        } else {
            String::new()
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
    let status = if failed {
        Status::Failed
    } else if complete && !matches!(state, "inProgress" | "in_progress") {
        Status::Done
    } else {
        Status::Running
    };
    let (detail, hidden) = detail(output, true);
    View {
        status,
        name: format!("$ {}", single_line(command)),
        target: String::new(),
        note: match code {
            "" | "null" if failed => state.to_owned(),
            "" | "null" | "0" => String::new(),
            code => format!("exit {code}"),
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
        assert_eq!(running.display(), "[..] Grep");
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
        assert_eq!(running.display(), "[..] $ sleep 1");
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
            "[ok] Native file-write activity: declined."
        );
    }
}
