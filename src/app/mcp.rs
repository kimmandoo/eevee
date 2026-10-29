//! Explicit MCP registration, launch review and selection; browsing grants nothing.
use super::*;
use crate::providers::mcp::{Document, Effects, Registration, SelectedTool, Session};
use parking_lot::Mutex;
use std::{
    cell::Cell,
    collections::BTreeSet,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

pub struct Review {
    pub registration: Registration,
    pub prepared: crate::workspace::runner::Prepared,
    pub launch: bool,
    pub text: String,
    pub engaged: bool,
    pub exposed: Cell<usize>,
    pub width: Cell<u16>,
    pub complete: Cell<bool>,
}
#[derive(Default)]
pub struct State {
    pub review: Option<Review>,
    pub tools: Vec<SelectedTool>,
    pub selected: BTreeSet<String>,
    pub scroll: usize,
    pub status: String,
    pub project: String,
    pub document: Document,
    connecting: Option<mpsc::Receiver<Result<(String, Session), String>>>,
    cancelled: Option<Arc<AtomicBool>>,
    connection_thread: Option<std::thread::JoinHandle<()>>,
    effects: Option<Arc<Effects>>,
    uncertainty_error: bool,
}
impl State {
    pub fn restore(&mut self, data: &Path, project: &str) {
        self.project = project.to_owned();
        match Effects::load(data, project) {
            Ok(effects) => {
                if effects.unknown() {
                    self.status = "Unmanaged MCP effects UNKNOWN from an earlier failed/interrupted call. Automatic Command/Write/MCP grants are disabled; every effect needs fresh manual review. Reconnect or a later success does not clear missing evidence.".into();
                }
                self.effects = Some(effects);
                self.uncertainty_error = false;
            }
            Err(error) => {
                self.uncertainty_error = true;
                self.status = format!(
                    "MCP uncertainty state unavailable; automatic effect grants disabled: {error}"
                );
            }
        }
        if let Ok(document) = Document::load(data) {
            self.document = document;
        }
    }
    pub fn effects_unknown(&self) -> bool {
        self.uncertainty_error
            || self
                .effects
                .as_ref()
                .is_some_and(|effects| effects.unknown())
    }
    pub fn frozen(&self, project: &str, writing: bool) -> Vec<SelectedTool> {
        self.tools
            .iter()
            .filter(|tool| {
                tool.project == project
                    && self.selected.contains(&tool.wire_name())
                    && (writing || tool.user_read)
            })
            .cloned()
            .collect()
    }
    pub fn poll(&mut self) -> bool {
        let Some(receiver) = &self.connecting else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(_) => Err("MCP connection worker ended".into()),
        };
        self.connecting = None;
        if let Some(thread) = self.connection_thread.take() {
            let _ = thread.join();
        }
        self.cancelled = None;
        match result {
            Ok((server, session)) => {
                let handle = Arc::new(Mutex::new(session));
                let session = handle.lock();
                self.tools.retain(|tool| tool.server != server);
                self.selected
                    .retain(|name| !name.starts_with(&format!("mcp__{server}__")));
                for tool in &session.tools {
                    if let Some(effects) = &self.effects {
                        self.tools.push(SelectedTool {
                            server: server.clone(),
                            name: tool["name"].as_str().unwrap_or_default().into(),
                            description: crate::providers::mcp::clean(
                                tool["description"].as_str().unwrap_or_default(),
                            ),
                            schema: tool["inputSchema"].clone(),
                            annotations: tool.get("annotations").cloned().unwrap_or_default(),
                            user_read: false,
                            project: self.project.clone(),
                            session_id: session.id,
                            session: handle.clone(),
                            effects: effects.clone(),
                        });
                    }
                }
                self.status = format!(
                    "Connected {server}: initialize + tools/list once. No tools selected; schemas NOT SENT until /mcp select <server> <tool>. Annotations never grant permission."
                );
            }
            Err(error) => self.status = error,
        }
        true
    }
    pub fn observe(&self, width: u16, offset: usize, height: usize, total: usize) {
        if let Some(review) = &self.review {
            if review.width.get() != width {
                review.width.set(width);
                review.exposed.set(0);
                review.complete.set(false);
            }
            if offset <= review.exposed.get() {
                review.exposed.set(
                    review
                        .exposed
                        .get()
                        .max(offset.saturating_add(height).min(total)),
                );
            }
            review.complete.set(review.exposed.get() >= total);
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        if let Some(cancelled) = &self.cancelled {
            cancelled.store(true, Ordering::Release);
        }
        if let Some(thread) = self.connection_thread.take() {
            let _ = thread.join();
        }
    }
}
impl App {
    pub(super) fn mcp_command(&mut self, argument: &str) -> bool {
        self.command_popup = None;
        self.mcp.poll();
        let result = self.mcp_command_inner(argument);
        if let Err(error) = result {
            self.mcp.status = error;
        }
        self.open_overlay(Overlay::Mcp);
        true
    }
    fn mcp_command_inner(&mut self, argument: &str) -> Result<(), String> {
        let data = self
            .rules
            .path
            .parent()
            .ok_or("MCP requires the loaded application data directory")?
            .to_path_buf();
        if self.mcp.project != self.cwd {
            self.mcp = State::default();
            self.mcp.restore(&data, &self.cwd);
        }
        let (action, rest) = argument
            .split_once(char::is_whitespace)
            .unwrap_or((argument, ""));
        let rest = rest.trim();
        if action.is_empty() || action == "list" {
            self.mcp.document = Document::load(&data)?;
            return Ok(());
        }
        if self.planning_any_busy()
            || self.execution_any_pending()
            || self.planning_any_unknown_effects()
            || self.execution_any_unknown()
        {
            return Err("Finish/cancel active work and settle unknown local effects before changing MCP capabilities".into());
        }
        if self.mcp.connecting.is_some() && action != "disconnect" {
            return Err("MCP connecting; /mcp disconnect cancels its owned tree".into());
        }
        match action {
            "add" => {
                let registration: Registration = serde_json::from_str(rest).map_err(|_| "Use /mcp add {\"name\":\"fixture\",\"program\":\"python3\",\"args\":[\"/absolute/server.py\"],\"cwd\":\".\",\"env\":[]}")?;
                let prepared = registration.prepare(Path::new(&self.cwd))?;
                let text = registration.review(&prepared);
                self.mcp.review = Some(Review { registration, prepared, launch: false, text, engaged: false, exposed: Cell::new(0), width: Cell::new(0), complete: Cell::new(false) });
                self.mcp.scroll = 0;
            }
            "connect" => {
                self.mcp.document = Document::load(&data)?;
                let registration = self.mcp.document.projects.get(&self.cwd).and_then(|rows| rows.iter().find(|row| row.name == rest)).cloned().ok_or("No registered server; /mcp lists registrations")?;
                let prepared = registration.prepare(Path::new(&self.cwd))?;
                let text = format!("LAUNCH REVIEW\n{}\nAllow launches exactly this server now. No tool is selected or call granted.", registration.review(&prepared));
                self.mcp.review = Some(Review { registration, prepared, launch: true, text, engaged: false, exposed: Cell::new(0), width: Cell::new(0), complete: Cell::new(false) });
                self.mcp.scroll = 0;
            }
            "select" | "unselect" | "read" | "effect" => {
                let (server, name) = rest.split_once(char::is_whitespace).ok_or("Use /mcp select|unselect|read|effect <server> <tool>")?;
                let tool = self.mcp.tools.iter_mut().find(|tool| tool.server == server && tool.name == name.trim()).ok_or("Connect first; choose an exact listed server/tool")?;
                match action {
                    "select" => { if self.mcp.selected.len() >= 16 { return Err("Select at most 16 MCP tools".into()); } self.mcp.selected.insert(tool.wire_name()); },
                    "unselect" => { self.mcp.selected.remove(&tool.wire_name()); },
                    "read" => { tool.user_read = true; },
                    _ => { tool.user_read = false; },
                }
                self.mcp.status = "Selection changes only the next frozen native Send; no launch, call or permission grant. User-marked read tools are unmanaged assertions, not confinement.".into();
            }
            "disconnect" => {
                if let Some(cancelled) = &self.mcp.cancelled { cancelled.store(true, Ordering::Release); }
                for tool in &self.mcp.tools { if rest.is_empty() || tool.server == rest { tool.session.lock().stop(); self.mcp.selected.remove(&tool.wire_name()); } }
                self.mcp.tools.retain(|tool| !rest.is_empty() && tool.server != rest);
                self.mcp.status = "MCP owned process tree stopped; selection removed. Effects are not rolled back.".into();
            }
            _ => return Err("/mcp add <JSON> | connect <server> | select/unselect/read/effect <server> <tool> | disconnect [server]. Environment values come from the existing process environment, never literal config.".into()),
        }
        Ok(())
    }
    pub(super) fn mcp_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press || !key.modifiers.is_empty() {
            return false;
        }
        match key.code {
            KeyCode::Esc => {
                self.mcp.review = None;
                self.close_overlay();
            }
            KeyCode::Down | KeyCode::PageDown => {
                self.mcp.scroll = self
                    .mcp
                    .scroll
                    .saturating_add(if key.code == KeyCode::Down { 1 } else { 5 })
            }
            KeyCode::Up | KeyCode::PageUp => {
                self.mcp.scroll =
                    self.mcp
                        .scroll
                        .saturating_sub(if key.code == KeyCode::Up { 1 } else { 5 })
            }
            KeyCode::Enter => {
                if let Some(review) = &mut self.mcp.review {
                    review.engaged = true;
                }
            }
            KeyCode::Char('a') => {
                let Some(review) = self.mcp.review.take() else {
                    return true;
                };
                if !review.engaged || !review.complete.get() {
                    self.mcp.status =
                        "Enter to engage, then read every scope row before a confirms".into();
                    self.mcp.review = Some(review);
                    return true;
                }
                if self.planning_any_busy()
                    || self.execution_any_pending()
                    || self.planning_any_unknown_effects()
                    || self.execution_any_unknown()
                {
                    self.mcp.status = "MCP review invalidated by active work or unknown effects; nothing launched/saved".into();
                    return true;
                }
                let data = self
                    .rules
                    .path
                    .parent()
                    .unwrap_or(Path::new(""))
                    .to_path_buf();
                if review.launch {
                    let (sender, receiver) = mpsc::channel();
                    let cancelled = Arc::new(AtomicBool::new(false));
                    let stop = cancelled.clone();
                    let project = self.cwd.clone();
                    let result = std::thread::Builder::new()
                        .name("ev-mcp-connect".into())
                        .spawn(move || {
                            let result = Session::connect(
                                &review.registration,
                                &review.prepared,
                                &project,
                                &|| stop.load(Ordering::Acquire),
                            )
                            .map(|session| (review.registration.name, session));
                            let _ = sender.send(result);
                        });
                    match result {
                        Ok(thread) => {
                            self.mcp.connection_thread = Some(thread);
                            self.mcp.connecting = Some(receiver);
                            self.mcp.cancelled = Some(cancelled);
                            self.mcp.status =
                                "Connecting reviewed unmanaged MCP server… /mcp disconnect cancels"
                                    .into();
                        }
                        Err(_) => self.mcp.status = "Cannot start MCP connection worker".into(),
                    }
                } else {
                    let result = (|| {
                        let current = review.registration.prepare(Path::new(&self.cwd))?;
                        if current.executable != review.prepared.executable
                            || current.cwd_identity != review.prepared.cwd_identity
                        {
                            return Err("MCP identity changed; review again".into());
                        }
                        let mut document = Document::load(&data)?;
                        let rows = document.projects.entry(self.cwd.clone()).or_default();
                        if rows.iter().any(|row| row.name == review.registration.name) {
                            return Err("Server already registered; use a new name (no silent authority replacement)".into());
                        }
                        if rows.len() >= 16 {
                            return Err("At most 16 servers per project".into());
                        }
                        rows.push(review.registration);
                        document.save(&data)?;
                        self.mcp.document = document;
                        Ok(())
                    })();
                    self.mcp.status = result.map_or_else(|error| error, |()| "Registered only; nothing launched, selected or granted. /mcp connect <server> reviews launch.".into());
                }
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mcp_scope_requires_contiguous_exposure_and_resize_resets_it() {
        let root = std::env::temp_dir().join(format!("ev-mcp-review-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let program = if crate::workspace::runner::resolve_executable("python3", &root).is_ok() {
            "python3"
        } else {
            "python"
        };
        let registration: Registration = serde_json::from_value(
            serde_json::json!({"name":"fixture","program":program,"args":[],"env":[]}),
        )
        .unwrap();
        let prepared = registration.prepare(&root).unwrap();
        let review = Review {
            text: registration.review(&prepared),
            registration,
            prepared,
            launch: false,
            engaged: false,
            exposed: Cell::new(0),
            width: Cell::new(0),
            complete: Cell::new(false),
        };
        let state = State {
            review: Some(review),
            tools: Vec::new(),
            selected: BTreeSet::new(),
            scroll: 0,
            status: String::new(),
            project: String::new(),
            document: Document::default(),
            connecting: None,
            cancelled: None,
            connection_thread: None,
            effects: None,
            uncertainty_error: false,
        };
        state.observe(60, 0, 5, 20);
        assert!(!state.review.as_ref().unwrap().complete.get());
        state.observe(60, 15, 5, 20);
        assert!(
            !state.review.as_ref().unwrap().complete.get(),
            "skipped scope cannot authorize"
        );
        for offset in [5, 10, 15] {
            state.observe(60, offset, 5, 20);
        }
        assert!(state.review.as_ref().unwrap().complete.get());
        state.observe(120, 15, 5, 20);
        assert!(
            !state.review.as_ref().unwrap().complete.get(),
            "resize invalidates prior exposure"
        );
        assert!(
            state.frozen(root.to_str().unwrap(), true).is_empty(),
            "registration is not selection"
        );
        assert!(
            !root.join("mcp.pid").exists(),
            "preparing scope must never launch"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
