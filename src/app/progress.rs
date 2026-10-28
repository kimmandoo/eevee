//! Explicit local editing/adoption of task-owned progress; never dispatches work.
use super::*;
use crate::workspace::progress::{self, State, Step};
impl App {
    pub(super) fn progress_command(&mut self, name: &str, argument: &str) -> bool {
        if !matches!(name, "todo" | "todo-adopt" | "todo-edit" | "todo-eager") {
            return false;
        }
        self.command_popup = None;
        // This card has its own editor. Keep the underlying composer buffer,
        // caret and ownership live beneath the overlay instead of moving them
        // into a retained action draft and clearing the visible Request.
        self.workflow.todo_view = true;
        self.open_overlay(Overlay::Workflows);
        self.workflow.scroll = 0;
        if name == "todo" && argument.is_empty() {
            return true;
        }
        if self.task_id <= 0
            || self.complete
            || self.archived
            || !self.authentication_ready()
            || self.planning_any_busy()
            || self.execution_any_pending()
            || self.planning_any_unknown_effects()
            || self.execution_any_unknown()
        {
            self.workflow.notice =
                "Wait for an idle saved open task before editing/adopting progress; nothing sent."
                    .into();
            return true;
        }
        let mut state = self.workflows.progress.clone().unwrap_or_default();
        if name == "todo-edit" && argument.is_empty() {
            let json = serde_json::to_string_pretty(&state.steps).unwrap_or_else(|_| "[]".into());
            self.workflow.todo_cursor = json.len();
            self.workflow.todo_editor = Some(json);
            self.workflow.notice = "Edit proposed JSON; exact duplicate labels are refused. Enter saves proposal; Esc retains current list.".into();
            return true;
        }
        let result = match name {
            "todo-adopt" => {
                if self.workflow.todo_revision != Some(state.revision)
                    || !self.workflow.todo_reviewed
                {
                    Err("Inspect the whole proposed list and revision diff on /todo (PgDn/End), then /todo-adopt".into())
                } else {
                    state.adopt()
                }
            }
            "todo-edit" => {
                if argument.len() > 64 * 1024 {
                    Err("Todo edit exceeds 64 KiB".into())
                } else {
                    serde_json::from_str::<Vec<Step>>(argument)
                        .map_err(|e| e.to_string())
                        .and_then(|steps| state.propose("user edited", steps).map(|_| ()))
                }
            }
            "todo-eager" => match argument {
                "on" => {
                    state.eager = true;
                    state.eager_off = false;
                    Ok(())
                }
                "off" => {
                    state.eager = false;
                    state.eager_off = true;
                    Ok(())
                }
                _ => Err("Use /todo-eager on|off; the native todo prelude is on by default".into()),
            },
            _ => {
                let (operation, target) = argument
                    .split_once(char::is_whitespace)
                    .unwrap_or((argument, ""));
                if matches!(operation, "init" | "append") {
                    serde_json::from_str::<Vec<Step>>(target)
                        .map_err(|e| e.to_string())
                        .and_then(|steps| state.operate(operation, "", steps))
                } else {
                    state.operate(operation, target, Vec::new())
                }
            }
        };
        // A list the user authored or operated is already their intent; only
        // provider proposals in manual mode wait for /todo-adopt.
        let authored = matches!(name, "todo" | "todo-edit");
        let result = result.and_then(|()| {
            if authored && state.revision > 0 {
                state.adopt()
            } else {
                Ok(())
            }
        });
        match result.and_then(|_| state.validate()) {
            Ok(()) => {
                let mut workflows = self.workflows.clone();
                workflows.progress = Some(state);
                match workflows.validate(self.task_id) {
                    Ok(()) => {
                        self.workflows = workflows;
                        self.changed_task();
                        self.flush_save();
                        self.workflow.notice = match name {
                            "todo-adopt" => "Todo revision explicitly adopted. Provider status is not evidence; no criteria completed.",
                            "todo-eager" => "Native todo prelude setting saved for this task; nothing sent.",
                            _ => "Todo saved and adopted. Its current step guides automatic evolution at the next Send; nothing sent.",
                        }
                        .into();
                    }
                    Err(error) => self.workflow.notice = error,
                }
            }
            Err(error) => self.workflow.notice = format!("Todo refused: {error}"),
        }
        true
    }
    pub(super) fn provider_progress(
        &mut self,
        partner: i64,
        generation: u64,
        task_id: i64,
        source: String,
        steps: Result<Vec<Step>, String>,
    ) -> bool {
        let owned = self.planning_state(partner).is_some_and(|state| {
            state.matches(generation, task_id)
                && state.busy()
                && !state.cancelling
                && !state.inspect_models
                && !state.inspect_sessions
        });
        if !owned || task_id != self.task_id {
            return false;
        }
        match steps {
            Ok(steps) => {
                let mut state = self.workflows.progress.clone().unwrap_or_default();
                // Automatic mode follows the provider's list without a manual
                // gate; it only steers appearance at the next explicit Send.
                let auto = self.lead.auto_form;
                let proposed = state.propose(&source, steps);
                if auto && proposed == Ok(true) {
                    // A failed adoption leaves an ordinary inspectable proposal.
                    let _ = state.adopt();
                }
                match proposed {
                    Ok(true) => {
                        let focus = state.focus_position();
                        let mut workflows = self.workflows.clone();
                        workflows.progress = Some(state);
                        match workflows.validate(task_id) {
                            Ok(()) => {
                                self.workflows = workflows;
                                self.changed_task();
                                self.flush_save();
                                self.workflow.notice = if auto {
                                    format!(
                                        "Todo updated automatically: {}. It guides evolution at the next Send; no form changed now.",
                                        focus.unwrap_or_else(|| "all steps settled".into())
                                    )
                                } else {
                                    "Provider todo proposal saved; inspect diff and explicitly adopt. No form changed or request sent.".into()
                                };
                            }
                            Err(error) => {
                                self.workflow.notice =
                                    format!("Todo refused: {error}; transcript retained")
                            }
                        }
                    }
                    Ok(false) => {}
                    Err(error) => {
                        self.workflow.notice = format!("Todo refused: {error}; transcript retained")
                    }
                }
            }
            Err(error) => {
                self.workflow.notice = format!("Todo refused: {error}; transcript retained")
            }
        }
        true
    }
    pub(super) fn progress_paste(&mut self, text: &str) -> bool {
        if let Some(editor) = &mut self.workflow.todo_editor
            && editor.len() + text.len() <= 64 * 1024
            && !text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r'))
        {
            editor.insert_str(self.workflow.todo_cursor, text);
            self.workflow.todo_cursor += text.len();
        }
        true
    }
    pub(super) fn progress_key(&mut self, key: KeyEvent) -> bool {
        use unicode_segmentation::UnicodeSegmentation;
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.request_quit();
            return true;
        }
        if self.width < 60 || self.height < 18 {
            return true;
        }
        if let Some(editor) = self.workflow.todo_editor.as_ref() {
            let cursor = self.workflow.todo_cursor;
            let previous = editor[..cursor]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(i, _)| i);
            let next = editor[cursor..]
                .graphemes(true)
                .next()
                .map_or(cursor, |g| cursor + g.len());
            match key.code {
                KeyCode::Esc => {
                    self.workflow.todo_editor = None;
                }
                KeyCode::Enter => {
                    let json = editor.clone();
                    match serde_json::from_str::<Vec<Step>>(&json)
                        .map_err(|e| e.to_string())
                        .and_then(|steps| progress::validate_steps(&steps).map(|_| steps))
                    {
                        Ok(steps) => {
                            self.progress_command("todo-edit", &json);
                            if self
                                .workflows
                                .progress
                                .as_ref()
                                .is_some_and(|p| p.steps == steps)
                            {
                                self.workflow.todo_editor = None;
                            }
                        }
                        Err(error) => self.workflow.notice = format!("Todo refused: {error}"),
                    }
                }
                KeyCode::Left => self.workflow.todo_cursor = previous,
                KeyCode::Right => self.workflow.todo_cursor = next,
                KeyCode::Home => self.workflow.todo_cursor = 0,
                KeyCode::End => self.workflow.todo_cursor = editor.len(),
                KeyCode::Backspace => {
                    self.workflow
                        .todo_editor
                        .as_mut()
                        .unwrap()
                        .drain(previous..cursor);
                    self.workflow.todo_cursor = previous;
                }
                KeyCode::Delete => {
                    self.workflow
                        .todo_editor
                        .as_mut()
                        .unwrap()
                        .drain(cursor..next);
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    let mut bytes = [0; 4];
                    self.progress_paste(c.encode_utf8(&mut bytes));
                }
                _ => {}
            }
            return true;
        }
        match key.code {
            KeyCode::Char('e') => {
                self.progress_command("todo-edit", "");
            }
            KeyCode::Char('a') => {
                self.progress_command("todo-adopt", "");
            }
            KeyCode::Esc => {
                self.workflow.todo_view = false;
                self.close_overlay();
            }
            KeyCode::PageDown | KeyCode::Down => {
                self.workflow.scroll = self
                    .workflow
                    .scroll
                    .saturating_add(self.workflow.scroll_page.max(1))
                    .min(self.workflow.scroll_max)
            }
            KeyCode::PageUp | KeyCode::Up => {
                self.workflow.scroll = self
                    .workflow
                    .scroll
                    .saturating_sub(self.workflow.scroll_page.max(1))
            }
            KeyCode::Home => self.workflow.scroll = 0,
            KeyCode::End => self.workflow.scroll = self.workflow.scroll_max,
            _ => {}
        }
        true
    }
    pub fn progress_card(&self) -> String {
        let mut lines = vec![
            self.workflows
                .progress
                .as_ref()
                .filter(|progress| progress.revision > 0)
                .map_or_else(
                    || "No todo yet. The agent creates one when work starts; /todo init adds your own.".into(),
                    State::summary,
                ),
            "Provider status claims are not evidence, completion or permission.".into(),
            if self.lead.auto_form {
                "Auto: provider updates are adopted automatically; the current step guides evolution at the next Send."
            } else {
                "Manual form: inspect provider updates, then /todo-adopt. Your own edits apply immediately."
            }
            .into(),
            self.workflow.notice.clone(),
        ];
        if let Some(editor) = &self.workflow.todo_editor {
            lines.push(format!(
                "EDIT JSON / caret byte {} / Enter save proposal / Esc cancel",
                self.workflow.todo_cursor
            ));
            lines.push(editor.clone());
            return lines.join("\n");
        }
        if let Some(progress) = &self.workflows.progress {
            for step in &progress.steps {
                lines.push(format!(
                    "{:?} / {} / {}",
                    step.status,
                    step.phase.as_deref().unwrap_or("no phase"),
                    step.label
                ));
            }
            if progress.adopted_revision != Some(progress.revision) {
                lines.push("Revision diff (previous adopted -> proposed):".into());
                lines.extend(progress.diff());
            }
        }
        let eager = self.workflows.progress.as_ref().is_none_or(State::eager);
        lines.push(format!(
            "Native phased todo prelude: {} (/todo-eager on|off)",
            if eager { "on" } else { "off" }
        ));
        if eager {
            lines.push(format!("Exact added bytes: {:?}", progress::EAGER_PRELUDE));
        }
        lines.push("/todo-edit <JSON steps>  /todo-adopt  /todo-eager on|off".into());
        lines.push(
            "/todo init|append <JSON steps>  /todo start|done|drop|block|unblock <exact label>"
                .into(),
        );
        lines.join("\n")
    }
}
#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
