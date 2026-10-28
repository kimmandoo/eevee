//! Bounded task-owned provider claims. Adoption is user intent, never execution evidence.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_STEPS: usize = 64;
pub const MAX_LABEL_BYTES: usize = 240;
pub const EAGER_PRELUDE: &str = "Before working, initialize a phased task list with the todo tool. Keep one active step; report status as claims, not verification evidence.\n";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Active,
    Done,
    Blocked,
    Dropped,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub label: String,
    #[serde(default)]
    pub phase: Option<String>,
    pub status: Status,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub revision: u64,
    pub adopted_revision: Option<u64>,
    pub source: String,
    pub steps: Vec<Step>,
    pub adopted_steps: Vec<Step>,
    /// Legacy opt-in flag; the prelude is now on unless `eager_off` is set.
    pub eager: bool,
    /// Explicit `/todo-eager off`. Omitted while the default (on) applies.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub eager_off: bool,
}
fn label(text: &str) -> Result<(), String> {
    if text.trim() != text || text.is_empty() || text.len() > MAX_LABEL_BYTES || text.chars().any(|c| c.is_control() || matches!(c, '\u{061c}'|'\u{200e}'|'\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')) {
        return Err("Todo labels/phases must be nonempty single lines of at most 240 bytes".into());
    }
    if crate::workspace::privacy::redact(text) != text {
        return Err("Todo text contains recognizable credentials; edit the proposal".into());
    }
    Ok(())
}
pub fn validate_steps(steps: &[Step]) -> Result<(), String> {
    if steps.len() > MAX_STEPS {
        return Err("Todo list exceeds 64 steps; original transcript retained".into());
    }
    let mut labels = std::collections::BTreeSet::new();
    let mut active = false;
    for step in steps {
        label(&step.label)?;
        if let Some(phase) = &step.phase {
            label(phase)?;
        }
        if !labels.insert(&step.label) {
            return Err(format!("Duplicate exact todo label: {}", step.label));
        }
        if step.status == Status::Active {
            if active {
                return Err("Todo list has more than one active step".into());
            }
            active = true;
        }
    }
    Ok(())
}
impl State {
    pub fn validate(&self) -> Result<(), String> {
        validate_steps(&self.steps)?;
        validate_steps(&self.adopted_steps)?;
        if self.source.len() > 80
            || self.source.chars().any(char::is_control)
            || self
                .adopted_revision
                .is_some_and(|r| r == 0 || r > self.revision)
            || (self.revision == 0 && !self.steps.is_empty())
        {
            return Err("Invalid todo revision/source".into());
        }
        if self.adopted_revision == Some(self.revision) && self.steps != self.adopted_steps {
            return Err("Adopted todo bytes differ from revision".into());
        }
        Ok(())
    }
    /// Whether native requests carry the phased-todo prelude (default on).
    pub fn eager(&self) -> bool {
        !self.eager_off
    }
    pub fn active_step(&self) -> Option<&Step> {
        if self.adopted_revision != Some(self.revision) {
            return None;
        }
        self.steps.iter().find(|s| s.status == Status::Active)
    }
    /// The adopted step the next request continues: the active step, otherwise
    /// the first pending one. Done/blocked/dropped steps never steer work.
    pub fn focus_step(&self) -> Option<(usize, &Step)> {
        if self.adopted_revision != Some(self.revision) {
            return None;
        }
        let find = |status| self.steps.iter().position(|s| s.status == status);
        let index = find(Status::Active).or_else(|| find(Status::Pending))?;
        Some((index, &self.steps[index]))
    }
    pub fn focus_label(&self) -> Option<&str> {
        self.focus_step().map(|(_, s)| s.label.as_str())
    }
    /// Compact "2/5 label" position of the focus step.
    pub fn focus_position(&self) -> Option<String> {
        self.focus_step()
            .map(|(index, step)| format!("{}/{} {}", index + 1, self.steps.len(), step.label))
    }
    pub fn propose(&mut self, source: &str, steps: Vec<Step>) -> Result<bool, String> {
        validate_steps(&steps)?;
        if source.is_empty() || source.len() > 80 || source.chars().any(char::is_control) {
            return Err("Invalid todo source".into());
        }
        if self.steps == steps && self.revision != 0 {
            return Ok(false);
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or("Todo revision exhausted")?;
        self.revision = revision;
        self.source = source.into();
        self.steps = steps;
        Ok(true)
    }
    pub fn adopt(&mut self) -> Result<(), String> {
        self.validate()?;
        if self.revision == 0 {
            return Err("No provider proposal to adopt".into());
        }
        self.adopted_revision = Some(self.revision);
        self.adopted_steps = self.steps.clone();
        Ok(())
    }
    pub fn apply_tool(&mut self, value: &Value) -> Result<(), String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            operation: String,
            label: String,
            steps: Vec<Step>,
        }
        let input = Input::deserialize(value)
            .map_err(|error| format!("Malformed todo arguments: {error}"))?;
        if matches!(input.operation.as_str(), "init" | "append" | "view") {
            if !input.label.is_empty() {
                return Err("Todo init/append/view require an empty target label".into());
            }
        } else if !input.steps.is_empty() {
            return Err("Exact-label todo status operations require empty steps".into());
        }
        if input.operation == "view" && !input.steps.is_empty() {
            return Err("Todo view takes empty steps".into());
        }
        self.operate(&input.operation, &input.label, input.steps)
    }
    /// Exact-label operations are transactional, including the single-active invariant.
    pub fn operate(
        &mut self,
        operation: &str,
        target: &str,
        steps: Vec<Step>,
    ) -> Result<(), String> {
        if operation == "view" {
            return Ok(());
        }
        let mut next = self.steps.clone();
        match operation {
            "init" => next = steps,
            "append" => next.extend(steps),
            "start" | "done" | "drop" | "block" | "unblock" => {
                let step = next
                    .iter_mut()
                    .find(|s| s.label == target)
                    .ok_or("Todo operation requires an existing exact label")?;
                step.status = match operation {
                    "start" => Status::Active,
                    "done" => Status::Done,
                    "drop" => Status::Dropped,
                    "block" => Status::Blocked,
                    _ => Status::Pending,
                };
            }
            _ => return Err("Unknown todo operation".into()),
        }
        self.propose("native todo", next)?;
        Ok(())
    }
    pub fn summary(&self) -> String {
        match self.active_step() {
            Some(step) => format!("adopted revision {} / {}", self.revision, step.label),
            None if self.revision == 0 => "no step list from this route".into(),
            None if self.focus_step().is_some() => format!(
                "adopted revision {} / next: {}",
                self.revision,
                self.focus_label().unwrap_or_default()
            ),
            None => format!(
                "revision {} / {} / no adopted active step",
                self.revision,
                if self.adopted_revision == Some(self.revision) {
                    "adopted"
                } else {
                    "proposed; adoption required"
                }
            ),
        }
    }
    pub fn diff(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for old in &self.adopted_steps {
            if !self.steps.contains(old) {
                lines.push(format!(
                    "- {:?} / {} / {}",
                    old.status,
                    old.phase.as_deref().unwrap_or("no phase"),
                    old.label
                ));
            }
        }
        for new in &self.steps {
            if !self.adopted_steps.contains(new) {
                lines.push(format!(
                    "+ {:?} / {} / {}",
                    new.status,
                    new.phase.as_deref().unwrap_or("no phase"),
                    new.label
                ));
            }
        }
        lines
    }
}
/// Preserve provider vocabulary at adapter edges, reject unsupported shapes visibly.
pub fn normalize(source: &str, value: &Value) -> Result<Vec<Step>, String> {
    let array = value
        .as_array()
        .or_else(|| value.get("plan").and_then(Value::as_array))
        .or_else(|| value.get("entries").and_then(Value::as_array))
        .or_else(|| value.get("todos").and_then(Value::as_array))
        .ok_or("Malformed todo observation: expected step array")?;
    if array.len() > MAX_STEPS {
        return Err("Todo list exceeds 64 steps; transcript retained".into());
    }
    let mut steps = Vec::with_capacity(array.len());
    for entry in array {
        let text = match source {
            "codex" => entry.get("step"),
            "acp" => entry.get("content"),
            "claude" => entry.get("content").or_else(|| entry.get("label")),
            _ => entry.get("label"),
        }
        .and_then(Value::as_str)
        .ok_or("Malformed todo label")?;
        let status = match entry
            .get("status")
            .and_then(Value::as_str)
            .ok_or("Missing todo status")?
        {
            "pending" | "not_started" => Status::Pending,
            "in_progress" | "inProgress" | "active" => Status::Active,
            "completed" | "done" => Status::Done,
            "blocked" => Status::Blocked,
            "dropped" | "cancelled" => Status::Dropped,
            _ => return Err("Unknown todo status; transcript retained".into()),
        };
        let phase = match entry.get("phase") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            _ => return Err("Malformed todo phase".into()),
        };
        steps.push(Step {
            label: text.into(),
            phase,
            status,
        });
    }
    validate_steps(&steps)?;
    Ok(steps)
}
pub fn definition() -> Value {
    serde_json::json!({"name":"todo","description":"Task progress claims only; no file, command or network effect. Exact labels; at most 64 steps, one active. Changed revisions require explicit user adoption.","parameters":{"type":"object","properties":{"operation":{"type":"string","enum":["init","start","done","drop","block","unblock","append","view"]},"label":{"type":"string"},"steps":{"type":"array","maxItems":64,"items":{"type":"object","properties":{"label":{"type":"string","maxLength":240},"phase":{"type":["string","null"]},"status":{"type":"string","enum":["pending","active","done","blocked","dropped"]}},"required":["label","phase","status"],"additionalProperties":false}}},"required":["operation","label","steps"],"additionalProperties":false}})
}
#[cfg(test)]
mod tests {
    use super::*;
    fn step(label: &str, status: Status) -> Step {
        Step {
            label: label.into(),
            phase: Some("investigate".into()),
            status,
        }
    }
    #[test]
    fn adoption_revision_and_exact_operations() {
        let mut state = State::default();
        state
            .propose(
                "fixture",
                vec![
                    step("조사 오류", Status::Active),
                    step("fix code", Status::Pending),
                ],
            )
            .unwrap();
        assert!(state.focus_label().is_none());
        state.adopt().unwrap();
        assert_eq!(state.focus_label(), Some("조사 오류"));
        let old = state.clone();
        assert!(state.operate("start", "fix code", vec![]).is_err());
        assert_eq!(state, old);
        state.operate("done", "조사 오류", vec![]).unwrap();
        state.operate("start", "fix code", vec![]).unwrap();
        assert!(state.focus_label().is_none());
        assert!(!state.diff().is_empty());
        state.adopt().unwrap();
        assert_eq!(state.focus_label(), Some("fix code"));
        assert_eq!(
            serde_json::from_str::<State>(&serde_json::to_string(&state).unwrap()).unwrap(),
            state
        );
    }
    #[test]
    fn focus_follows_active_then_next_pending_adopted_step() {
        let mut state = State::default();
        state
            .propose(
                "fixture",
                vec![
                    step("오류 조사", Status::Done),
                    step("코드 수정", Status::Pending),
                    step("테스트 실행", Status::Pending),
                ],
            )
            .unwrap();
        assert!(
            state.focus_step().is_none(),
            "unadopted proposals never steer"
        );
        state.adopt().unwrap();
        assert_eq!(state.focus_label(), Some("코드 수정"));
        assert_eq!(state.focus_position().as_deref(), Some("2/3 코드 수정"));
        assert_eq!(state.summary(), "adopted revision 1 / next: 코드 수정");
        state.operate("start", "테스트 실행", vec![]).unwrap();
        state.adopt().unwrap();
        assert_eq!(state.focus_label(), Some("테스트 실행"));
        for label in ["코드 수정", "테스트 실행"] {
            state.operate("done", label, vec![]).unwrap();
        }
        state.adopt().unwrap();
        assert!(
            state.focus_step().is_none(),
            "completed lists steer nothing"
        );
    }
    #[test]
    fn todo_prelude_defaults_on_and_only_an_explicit_off_is_persisted() {
        let state = State::default();
        assert!(state.eager());
        let json = serde_json::to_string(&state).unwrap();
        assert!(
            !json.contains("eager_off"),
            "default bytes stay legacy-readable"
        );
        // Legacy stored lists without the field (eager false) now use the default.
        let legacy: State = serde_json::from_str(&json).unwrap();
        assert!(legacy.eager());
        let off = State {
            eager_off: true,
            ..State::default()
        };
        assert!(!off.eager());
        let off: State = serde_json::from_str(&serde_json::to_string(&off).unwrap()).unwrap();
        assert!(!off.eager());
    }
    #[test]
    fn malformed_native_tool_is_transactional_and_view_grants_nothing() {
        let mut state = State::default();
        state.apply_tool(&serde_json::json!({"operation":"init","label":"","steps":[{"label":"오류 조사","phase":"investigate","status":"active"}]})).unwrap();
        let before = state.clone();
        for arguments in [
            serde_json::json!({"operation":"start","label":"오류","steps":[]}),
            serde_json::json!({"operation":"init","label":"","steps":[{"label":"bad","phase":null,"status":"invented"}]}),
            serde_json::json!({"operation":"view","label":"","steps":[],"command":"execute"}),
            serde_json::json!({"operation":"init","label":"","steps":[{"label":"duplicate","phase":null,"status":"pending"},{"label":"duplicate","phase":null,"status":"pending"}]}),
        ] {
            assert!(state.apply_tool(&arguments).is_err());
            assert_eq!(state, before);
        }
        state
            .apply_tool(&serde_json::json!({"operation":"view","label":"","steps":[]}))
            .unwrap();
        assert_eq!(state, before);
        assert!(state.focus_label().is_none());
    }
    #[test]
    fn adapters_agree_and_bounds_refuse() {
        let codex = serde_json::json!([{"step":"Investigate error","status":"in_progress"}]);
        let acp = serde_json::json!([{"content":"Investigate error","status":"in_progress"}]);
        assert_eq!(
            normalize("codex", &codex).unwrap(),
            normalize("acp", &acp).unwrap()
        );
        assert_eq!(
            normalize("acp", &acp).unwrap(),
            normalize("claude", &acp).unwrap()
        );
        assert!(validate_steps(&[step("x", Status::Pending), step("x", Status::Done)]).is_err());
        assert!(validate_steps(&vec![step("x", Status::Pending); 65]).is_err());
        assert!(validate_steps(&[step("x\ny", Status::Pending)]).is_err());
    }
}
