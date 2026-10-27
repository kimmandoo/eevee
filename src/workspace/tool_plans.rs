//! Bounded task-owned dependency plans. Records are evidence, never reusable grants.
use super::runner;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Component, Path},
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAX_NODES: usize = 32;
pub const MAX_RESULT_BYTES: usize = 2048;
pub(crate) mod worker;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Operation {
    Read {
        path: String,
        start_line: usize,
        end_line: usize,
    },
    List {
        directory: String,
        query: String,
        offset: usize,
    },
    Command {
        spec: runner::CommandSpec,
    },
}
impl Operation {
    pub fn readonly(&self) -> bool {
        !matches!(self, Self::Command { .. })
    }
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Read {
                path,
                start_line,
                end_line,
            } => {
                relative(path, false)?;
                if *start_line == 0 || (*end_line != 0 && end_line < start_line) {
                    return Err("Read requires a one-based inclusive range".into());
                }
            }
            Self::List {
                directory,
                query,
                offset,
            } => {
                relative(directory, true)?;
                safe(query, 1024)?;
                if *offset > 10_000 {
                    return Err("List offset exceeds 10000".into());
                }
            }
            Self::Command { spec } => {
                // Exactly the same accepted command dialect as the ordinary runner.
                let encoded = command_text(spec)?;
                if crate::app::execution::parse_command(&encoded)? != *spec {
                    return Err("Command arguments do not round-trip exactly".into());
                }
            }
        }
        Ok(())
    }
}
pub fn command_text(spec: &runner::CommandSpec) -> Result<String, String> {
    let value = match spec {
        runner::CommandSpec::Direct { program, args } => {
            serde_json::json!(std::iter::once(program).chain(args).collect::<Vec<_>>())
        }
        runner::CommandSpec::Shell {
            executable,
            dialect,
            text,
        } => {
            serde_json::json!({"shell":executable,"dialect":match dialect { runner::ShellDialect::Posix => "posix", runner::ShellDialect::Cmd => "cmd", runner::ShellDialect::PowerShell => "powershell" },"text":text})
        }
        runner::CommandSpec::SelectedCommit { .. } => return Err(
            "Selected Git commits require explicit local authoring and separate one-shot review; plans cannot authorize them".into()
        ),
    };
    let text = value.to_string();
    safe(&text, 16 * 1024)?;
    Ok(text)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSpec {
    pub id: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub operation: Operation,
    pub reason: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Pending,
    Running,
    Succeeded,
    Failed,
    Blocked,
    Cancelled,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Certainty {
    NotStarted,
    NoEffect,
    Confirmed,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub spec: NodeSpec,
    pub status: Status,
    pub certainty: Certainty,
    pub argument_sha256: String,
    pub input: runner::InputSnapshot,
    pub result: String,
    pub result_sha256: Option<String>,
    pub result_bytes: usize,
    pub observation_sha256: Option<String>,
    pub run_id: Option<String>,
    pub started_at_ms: Option<u64>,
    pub ended_at_ms: Option<u64>,
    pub elapsed_us: u64,
    pub query_workers: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Comparison {
    pub parallel_wall_us: u64,
    pub parallel_calls: u32,
    pub serial_wall_us: Option<u64>,
    pub serial_calls: Option<u32>,
    pub equal_results: Option<bool>,
    pub original_results: Vec<Option<String>>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub version: u8,
    pub id: String,
    pub title: String,
    pub task: i64,
    pub partner: i64,
    pub project: String,
    pub source: runner::InputSnapshot,
    pub nodes: Vec<Node>,
    pub cancelled: bool,
    pub started_at_ms: Option<u64>,
    pub ended_at_ms: Option<u64>,
    pub wall_us: u64,
    pub attempts: u32,
    pub execution_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison: Option<Comparison>,
}
impl Plan {
    pub fn new(
        title: String,
        specs: Vec<NodeSpec>,
        task: i64,
        partner: i64,
        project: String,
        source: runner::InputSnapshot,
    ) -> Result<Self, String> {
        let nodes = specs
            .into_iter()
            .map(|spec| {
                let argument_sha256 = digest(
                    serde_json::to_vec(&spec.operation)
                        .expect("typed arguments serialize")
                        .as_slice(),
                );
                Node {
                    spec,
                    status: Status::Pending,
                    certainty: Certainty::NotStarted,
                    argument_sha256,
                    input: source.clone(),
                    result: String::new(),
                    result_sha256: None,
                    observation_sha256: None,
                    result_bytes: 0,
                    run_id: None,
                    started_at_ms: None,
                    ended_at_ms: None,
                    elapsed_us: 0,
                    query_workers: 0,
                }
            })
            .collect();
        let plan = Self {
            version: 1,
            id: format!("plan-{}-{}", now_ms(), crate::providers::approval_arrival()),
            title,
            task,
            partner,
            project,
            source,
            nodes,
            cancelled: false,
            started_at_ms: None,
            ended_at_ms: None,
            wall_us: 0,
            attempts: 0,
            execution_count: 0,
            comparison: None,
        };
        plan.validate(task)?;
        Ok(plan)
    }
    pub fn validate(&self, task: i64) -> Result<(), String> {
        if self.version != 1
            || task <= 0
            || self.task != task
            || self.partner <= 0
            || self.nodes.is_empty()
            || self.nodes.len() > MAX_NODES
        {
            return Err("Invalid plan version, ownership or node count (1–32)".into());
        }
        safe(&self.title, 1024)?;
        safe(&self.id, 256)?;
        if self.project.is_empty() || self.project.len() > 4096 {
            return Err("Invalid plan project identity".into());
        }
        if self.comparison.as_ref().is_some_and(|comparison| {
            comparison.original_results.len() != self.nodes.len()
                || self
                    .nodes
                    .iter()
                    .any(|node| !node.spec.operation.readonly())
        }) {
            return Err("Serial comparison must retain the same bounded all-read graph".into());
        }
        let mut ids = BTreeSet::new();
        for node in &self.nodes {
            if node.spec.id.is_empty()
                || node.spec.id.len() > 64
                || !node
                    .spec
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                || !ids.insert(&node.spec.id)
            {
                return Err("Plan node IDs must be unique literal identifiers".into());
            }
            safe(&node.spec.reason, 1024)?;
            node.spec.operation.validate()?;
            if node.spec.dependencies.len() > MAX_NODES
                || node.result.len() > MAX_RESULT_BYTES
                || node.argument_sha256
                    != digest(&serde_json::to_vec(&node.spec.operation).map_err(|e| e.to_string())?)
            {
                return Err("Invalid plan dependency/result/argument binding".into());
            }
            if node
                .run_id
                .as_ref()
                .is_some_and(|id| id.len() > 256 || id.chars().any(char::is_control))
            {
                return Err("Invalid plan run reference".into());
            }
        }
        let mut reached = BTreeSet::new();
        loop {
            let before = reached.len();
            for node in &self.nodes {
                let deps: BTreeSet<_> = node.spec.dependencies.iter().collect();
                if deps.len() != node.spec.dependencies.len()
                    || deps.iter().any(|id| !ids.contains(id))
                {
                    return Err("Unknown or duplicate plan prerequisite".into());
                }
                if deps.iter().all(|id| reached.contains(*id)) {
                    reached.insert(&node.spec.id);
                }
            }
            if reached.len() == self.nodes.len() {
                break;
            }
            if reached.len() == before {
                return Err("Plan dependencies contain a cycle".into());
            }
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 240 * 1024 {
            return Err("Plan exceeds 240 KiB durable bound".into());
        }
        Ok(())
    }
    pub fn block_descendants(&mut self) {
        loop {
            let bad: BTreeSet<_> = self
                .nodes
                .iter()
                .filter(|node| {
                    matches!(
                        node.status,
                        Status::Failed | Status::Blocked | Status::Cancelled | Status::Unknown
                    )
                })
                .map(|node| node.spec.id.clone())
                .collect();
            let mut changed = false;
            for node in &mut self.nodes {
                if node.status == Status::Pending
                    && node.spec.dependencies.iter().any(|id| bad.contains(id))
                {
                    node.status = Status::Blocked;
                    node.retain_result("Prerequisite did not succeed; this node was not started");
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }
    pub fn ready(&self) -> Vec<usize> {
        if self.cancelled || self.nodes.iter().any(|node| node.status == Status::Unknown) {
            return Vec::new();
        }
        let running: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| node.status == Status::Running)
            .collect();
        if running.iter().any(|node| !node.spec.operation.readonly()) {
            return Vec::new();
        }
        let mut ready = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if node.status != Status::Pending
                || !node.spec.dependencies.iter().all(|id| {
                    self.nodes
                        .iter()
                        .any(|dep| dep.spec.id == *id && dep.status == Status::Succeeded)
                })
            {
                continue;
            }
            if !node.spec.operation.readonly() {
                if ready.is_empty() && running.is_empty() {
                    ready.push(index);
                }
                break;
            }
            if ready.len() + running.len() < 3 {
                ready.push(index);
            }
        }
        ready
    }
    /// Failed mutations with observed effects are never selected, even on exit 1.
    pub fn resume_candidates(&self) -> Result<Vec<usize>, String> {
        if self.nodes.iter().any(|node| {
            node.status == Status::Unknown
                || node.certainty == Certainty::Unknown
                || node.status == Status::Running
        }) {
            return Err("Unresolved/lost response: reconcile against the existing durable command journal before resuming; no exactly-once guarantee".into());
        }
        Ok(self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.status != Status::Succeeded
                    && (node.spec.operation.readonly()
                        || matches!(node.certainty, Certainty::NotStarted | Certainty::NoEffect))
            })
            .map(|(index, _)| index)
            .collect())
    }
    pub fn cancel(&mut self) {
        self.cancelled = true;
        for node in &mut self.nodes {
            if node.status == Status::Pending {
                node.status = Status::Cancelled;
                node.retain_result("Cancelled before start");
            }
        }
    }
    pub fn restore(&mut self) {
        // Startup only restores inspection. Running mutation windows are conservative.
        for node in &mut self.nodes {
            if node.status == Status::Running {
                node.status = if node.spec.operation.readonly() {
                    Status::Failed
                } else {
                    Status::Unknown
                };
                node.certainty = if node.spec.operation.readonly() {
                    Certainty::NoEffect
                } else {
                    Certainty::Unknown
                };
                node.retain_result("Interrupted record; restart did not replay this node");
            }
        }
    }
    pub fn serial_work_us(&self) -> u64 {
        self.nodes
            .iter()
            .fold(0u64, |sum, node| sum.saturating_add(node.elapsed_us))
    }
}
impl Node {
    pub fn retain_result(&mut self, text: &str) {
        self.result_bytes = text.len();
        self.result_sha256 = Some(digest(text.as_bytes()));
        let mut end = text.len().min(MAX_RESULT_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.result = text[..end].to_owned();
    }
}
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn safe(text: &str, max: usize) -> Result<(), String> {
    if text.len() > max
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        || super::privacy::redact(text) != text
    {
        return Err("Plan contains oversized, unsafe or credential-bearing text".into());
    }
    Ok(())
}
fn relative(value: &str, empty: bool) -> Result<(), String> {
    safe(value, 4096)?;
    if (!empty && value.is_empty())
        || !Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err("Plan paths must be literal project-relative paths".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read(id: &str, deps: &[&str]) -> NodeSpec {
        NodeSpec {
            id: id.into(),
            dependencies: deps.iter().map(|s| (*s).into()).collect(),
            operation: Operation::Read {
                path: "src/lib.rs".into(),
                start_line: 1,
                end_line: 1,
            },
            reason: "Inspect input".into(),
        }
    }
    fn plan(nodes: Vec<NodeSpec>) -> Plan {
        Plan::new(
            "Checks".into(),
            nodes,
            7,
            1,
            "/project".into(),
            Default::default(),
        )
        .unwrap()
    }
    #[test]
    fn rejects_cycles_and_argument_effect_lies() {
        assert!(
            Plan::new(
                "bad".into(),
                vec![read("a", &["b"]), read("b", &["a"])],
                7,
                1,
                "/project".into(),
                Default::default()
            )
            .is_err()
        );
        assert!(serde_json::from_str::<Operation>(r#"{"type":"Command","spec":{"Direct":{"program":"echo","args":[]}},"readonly":true}"#).is_err());
    }
    #[test]
    fn reads_overlap_mutations_serialize_failure_blocks_cancel_stops_queue() {
        let mut p = plan(vec![read("a", &[]), read("b", &[]), read("child", &["a"])]);
        assert_eq!(p.ready(), [0, 1]);
        p.nodes[0].status = Status::Failed;
        p.block_descendants();
        assert_eq!(p.nodes[2].status, Status::Blocked);
        p.cancel();
        assert!(p.ready().is_empty());
        p.nodes[0].spec.operation = Operation::Command {
            spec: runner::CommandSpec::Direct {
                program: "echo".into(),
                args: vec![],
            },
        };
        p.nodes[0].status = Status::Running;
        p.cancelled = false;
        assert!(p.ready().is_empty());
    }
    #[test]
    fn restart_and_resume_never_repeat_confirmed_mutations() {
        let mut p = plan(vec![read("a", &[]), read("b", &[])]);
        p.nodes[0].spec.operation = Operation::Command {
            spec: runner::CommandSpec::Direct {
                program: "echo".into(),
                args: vec![],
            },
        };
        p.nodes[0].status = Status::Failed;
        p.nodes[0].certainty = Certainty::Confirmed;
        assert_eq!(p.resume_candidates().unwrap(), [1]);
        p.nodes[0].status = Status::Running;
        p.restore();
        assert_eq!(p.nodes[0].status, Status::Unknown);
        assert!(p.resume_candidates().is_err());
        assert!(p.ready().is_empty());
    }
}
