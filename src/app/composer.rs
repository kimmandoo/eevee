use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{App, Overlay};
use crate::store::EditorField;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Files,
    Commands,
}

#[derive(Clone, Copy)]
pub struct Menu {
    pub kind: Kind,
    pub start: usize,
    pub selected: usize,
}

pub struct Command {
    pub name: &'static str,
    pub description: &'static str,
    pub destination: Overlay,
}

pub const COMMANDS: &[Command] = &[
    Command {
        name: "help",
        description: "Keyboard help",
        destination: Overlay::Help,
    },
    Command {
        name: "connect",
        description: "Connect a provider; no task sent",
        destination: Overlay::Agents,
    },
    Command {
        name: "run",
        description: "Prepare a local command; approval required",
        destination: Overlay::Execution,
    },
    Command {
        name: "provider",
        description: "Review and start a provider request",
        destination: Overlay::Planning,
    },
    Command {
        name: "files",
        description: "Inspect and attach source context",
        destination: Overlay::Sources,
    },
    Command {
        name: "tasks",
        description: "Browse saved tasks",
        destination: Overlay::Tasks,
    },
    Command {
        name: "criteria",
        description: "Review acceptance criteria",
        destination: Overlay::Criteria,
    },
    Command {
        name: "forms",
        description: "Choose evolution",
        destination: Overlay::Forms,
    },
    Command {
        name: "inspect",
        description: "Task, storage and control details",
        destination: Overlay::Inspector,
    },
    Command {
        name: "diff",
        description: "Repository baseline and changes",
        destination: Overlay::Repository,
    },
    Command {
        name: "handoff",
        description: "Prepare a portable handoff",
        destination: Overlay::Handoff,
    },
    Command {
        name: "memory",
        description: "Review approved memories",
        destination: Overlay::Memory,
    },
    Command {
        name: "workflows",
        description: "Open the specialist desk",
        destination: Overlay::Workflows,
    },
];

impl App {
    pub fn composer_query(&self) -> &str {
        self.composer_menu
            .and_then(|menu| self.input.get(menu.start + 1..self.cursor))
            .unwrap_or("")
    }

    pub fn composer_match(&self, index: usize) -> bool {
        let Some(menu) = self.composer_menu else {
            return false;
        };
        let query = self.composer_query();
        match menu.kind {
            Kind::Commands => COMMANDS
                .get(index)
                .is_some_and(|command| command.name.starts_with(query)),
            Kind::Files => self.sources.files.get(index).is_some_and(|source| {
                source
                    .path
                    .to_str()
                    .is_some_and(|path| path.contains(query))
            }),
        }
    }

    pub fn composer_candidates(&self) -> impl Iterator<Item = usize> + '_ {
        let count = match self.composer_menu.map(|menu| menu.kind) {
            Some(Kind::Commands) => COMMANDS.len(),
            Some(Kind::Files) => self.sources.files.len(),
            None => 0,
        };
        (0..count).filter(|index| self.composer_match(*index))
    }

    /// Called after task-owned state is reset in the main loop. Initial '@'
    /// participates in ordinary draft autosave, then awaits its durable task ID.
    pub fn prepare_composer_sources(&mut self) {
        if !self.composer_catalogue || self.task_id <= 0 || self.sources.busy() {
            return;
        }
        self.composer_catalogue = false;
        if self.repository.is_none() && !self.repo_error.is_empty() {
            self.sources.error =
                "Repository unavailable. Esc, then /files and u to refresh; w shows details."
                    .into();
            return;
        }
        self.sources.query = self.composer_query().to_owned();
        self.sources
            .catalogue(self.task_id, self.repository.as_ref());
    }

    pub(super) fn composer_key(&mut self, key: KeyEvent) -> Option<bool> {
        if !matches!(
            self.editor_field,
            EditorField::Goal | EditorField::NextAction
        ) {
            return None;
        }
        if let Some(menu) = self.composer_menu {
            if self.cursor <= menu.start || self.input.get(menu.start + 1..self.cursor).is_none() {
                self.composer_menu = None;
                return None;
            }
            match key.code {
                KeyCode::Esc => {
                    self.composer_menu = None;
                    self.composer_catalogue = false;
                    return Some(true);
                }
                KeyCode::Up | KeyCode::BackTab | KeyCode::Down | KeyCode::Tab => {
                    let count = self.composer_candidates().count();
                    let selected = if matches!(key.code, KeyCode::Up | KeyCode::BackTab) {
                        menu.selected.saturating_sub(1)
                    } else {
                        menu.selected.saturating_add(1).min(count.saturating_sub(1))
                    };
                    self.composer_menu.as_mut().expect("open menu").selected = selected;
                    return Some(true);
                }
                KeyCode::Enter => {
                    if self.width < 60 || self.height < 18 {
                        return Some(false);
                    }
                    let selected = self.composer_candidates().nth(menu.selected);
                    if let Some(index) = selected {
                        if menu.kind == Kind::Files
                            && (self.sources.busy()
                                || self.composer_catalogue
                                || !self.sources.error.is_empty())
                        {
                            return Some(false);
                        }
                        self.choose_composer_item(index);
                    }
                    return Some(true);
                }
                KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => {
                    self.composer_menu = None;
                    self.composer_catalogue = false;
                }
                KeyCode::Backspace if self.cursor == menu.start + 1 => {
                    self.composer_menu = None;
                    self.composer_catalogue = false;
                }
                KeyCode::Char(character) if character.is_whitespace() => {
                    self.composer_menu = None;
                    self.composer_catalogue = false;
                }
                KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete => {
                    self.composer_menu.as_mut().expect("open menu").selected = 0;
                    self.composer_catalogue = menu.kind == Kind::Files;
                }
                _ => {}
            }
            return None;
        }
        let kind = match key.code {
            KeyCode::Char('@')
                if self.cursor == 0 || self.input[..self.cursor].ends_with(char::is_whitespace) =>
            {
                Kind::Files
            }
            KeyCode::Char('/')
                if self.input[..self.cursor]
                    .rsplit('\n')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .is_empty() =>
            {
                Kind::Commands
            }
            _ => return None,
        };
        Some(self.start_composer_menu(kind))
    }

    pub(super) fn start_composer_menu(&mut self, kind: Kind) -> bool {
        let start = self.cursor;
        if !self.insert_input(if kind == Kind::Files { "@" } else { "/" }) {
            return false;
        }
        self.composer_menu = Some(Menu {
            kind,
            start,
            selected: 0,
        });
        self.composer_catalogue = kind == Kind::Files;
        true
    }

    fn choose_composer_item(&mut self, index: usize) {
        let menu = self.composer_menu.take().expect("open menu");
        self.composer_catalogue = false;
        let field = self.editor_field;
        let replacement = if menu.kind == Kind::Files {
            format!("@{} ", self.sources.files[index].path.to_string_lossy())
        } else {
            String::new()
        };
        // Keep all text outside the trigger, including text after the caret.
        self.input.replace_range(menu.start..self.cursor, "");
        self.cursor = menu.start;
        self.insert_input(&replacement);
        self.edit_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        self.composer_return = Some(field);
        if menu.kind == Kind::Files {
            // Reuse the validated catalogue and the ordinary inspection/pin
            // workflow. Never bypass exclusions, capture review or persistence.
            self.save_selection();
            if let Some(previous) = self.overlay_frame() {
                self.overlay_history.push(previous);
            }
            self.overlay = Some(Overlay::Sources);
            self.sources.mode = super::sources::Mode::Files;
            self.sources.file_selection = index;
            self.sources
                .inspect_file(self.task_id, self.repository.as_ref());
            self.notice =
                "Inspect selected file; r chooses lines, p attaches. Esc returns to your draft."
                    .into();
        } else {
            let command = &COMMANDS[index];
            self.open_overlay(command.destination);
            if command.name == "run" && self.overlay == Some(Overlay::Execution) {
                self.execution_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
            }
            if self.overlay.is_none() && !self.editing {
                self.composer_return = None;
                self.begin_edit_field(field);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::Event;

    fn key(app: &mut App, code: KeyCode) {
        app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    #[test]
    fn slash_action_preserves_draft_and_escape_restores_editor() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.update(Event::Paste("draft\n".into()));
        key(&mut app, KeyCode::Char('/'));
        for ch in "help".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        assert_eq!(app.composer_candidates().count(), 1);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.overlay, Some(Overlay::Help));
        assert_eq!(app.drafts[0].text, "draft\n");
        key(&mut app, KeyCode::Esc);
        assert!(app.editing);
        assert_eq!(app.input, "draft\n");
        assert!(app.planning.pending.is_none());
    }

    #[test]
    fn paste_does_not_execute_commands_and_menu_escape_keeps_literal_text() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.update(Event::Paste("/run echo unsafe\n@private".into()));
        assert!(app.composer_menu.is_none());
        assert!(app.overlay.is_none());
        key(&mut app, KeyCode::Char(' '));
        key(&mut app, KeyCode::Char('@'));
        assert!(app.composer_menu.is_some());
        assert!(app.save_requested);
        key(&mut app, KeyCode::Esc);
        assert!(app.editing);
        assert!(app.input.ends_with(" @"));
        assert!(app.composer_menu.is_none());
    }

    #[test]
    fn run_command_opens_local_editor_without_execution() {
        let mut app = App::new("/workspace".into(), 120, 36);
        key(&mut app, KeyCode::Char('/'));
        for ch in "run".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        key(&mut app, KeyCode::Enter);
        assert!(app.editing);
        assert_eq!(app.editor_field, EditorField::Command);
        assert!(app.command.is_empty());
        assert!(app.planning.pending.is_none());
    }

    #[test]
    fn first_attachment_autosaves_draft_and_waits_for_its_task_owner() {
        let mut app = App::new("/workspace".into(), 120, 36);
        key(&mut app, KeyCode::Char('@'));
        assert_eq!(app.task_id, 0);
        assert!(app.save_requested);
        app.prepare_composer_sources();
        assert!(app.sources.pending.is_none());
        assert!(app.composer_catalogue);
        assert_eq!(app.input, "@");
        assert!(app.goal.is_empty());
        app.task_id = 7;
        app.repo_error = "not a repository".into();
        app.prepare_composer_sources();
        assert!(!app.composer_catalogue);
        assert!(!app.sources.error.is_empty());
        assert!(!app.sources.await_refresh);
        assert_eq!(app.input, "@");
    }

    #[test]
    fn attachment_filter_and_return_preserve_the_original_draft() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.task_id = 7;
        app.update(Event::Paste("Explain ".into()));
        key(&mut app, KeyCode::Char('@'));
        for ch in "README".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        app.sources.files = vec![
            crate::context::Source {
                path: "src/main.rs".into(),
                bytes: 12,
                sha256: [0; 32],
            },
            crate::context::Source {
                path: "README.md".into(),
                bytes: 20,
                sha256: [0; 32],
            },
        ];
        app.composer_catalogue = false;
        assert_eq!(app.composer_candidates().collect::<Vec<_>>(), vec![1]);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.overlay, Some(Overlay::Sources));
        assert_eq!(app.sources.file_selection, 1);
        assert_eq!(app.drafts[0].text, "Explain @README.md ");
        assert!(app.sources.basket.is_empty());
        key(&mut app, KeyCode::Esc);
        assert!(app.editing);
        assert_eq!(app.input, "Explain @README.md ");
        assert!(app.planning.pending.is_none());
    }

    #[test]
    fn saved_task_navigation_opens_commands_without_changing_the_goal() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.goal = "Existing task".into();
        app.editing = false;
        key(&mut app, KeyCode::Char('/'));
        for ch in "help".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.overlay, Some(Overlay::Help));
        assert_eq!(app.goal, "Existing task");
        key(&mut app, KeyCode::Esc);
        assert!(app.editing);
        assert_eq!(app.input, "Existing task");
    }
}
