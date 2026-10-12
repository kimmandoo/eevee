use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

use crate::domain::FORMS;

const MAX_GOAL_BYTES: usize = 4096;

pub const PALETTE: [&str; 6] = [
    "Choose evolution",
    "Edit session goal",
    "Inspect session",
    "Show companion",
    "Keyboard help",
    "Quit Eevee",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Companion,
    Workspace,
    Inspector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    Forms,
    Palette,
    Help,
    Inspector,
    Companion,
}

pub struct App {
    pub form: usize,
    pub selected: usize,
    pub focus: Pane,
    pub overlay: Option<Overlay>,
    pub goal: String,
    pub input: String,
    pub editing: bool,
    pub cwd: String,
    pub width: u16,
    pub height: u16,
    pub quit: bool,
}

impl App {
    pub fn new(cwd: String, width: u16, height: u16) -> Self {
        Self {
            form: 0,
            selected: 0,
            focus: Pane::Workspace,
            overlay: None,
            goal: String::new(),
            input: String::new(),
            editing: false,
            cwd,
            width,
            height,
            quit: false,
        }
    }

    /// Returns whether visible state changed. The event loop otherwise stays asleep.
    pub fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Resize(width, height) => {
                if (width, height) == (self.width, self.height) {
                    return false;
                }
                self.width = width;
                self.height = height;
                if width < 90 || (width < 120 && self.focus == Pane::Inspector) {
                    self.focus = Pane::Workspace;
                }
                true
            }
            Event::Paste(text) if self.editing => self.append_input(&text),
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            _ => false,
        }
    }

    fn key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return true;
        }
        if self.editing {
            return self.edit_key(key);
        }
        if key.code == KeyCode::Char('q')
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.quit = true;
            return true;
        }
        if self.width < 60 || self.height < 18 {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('k') {
            self.overlay = Some(Overlay::Palette);
            self.selected = 0;
            return true;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        if let Some(overlay) = self.overlay {
            return self.overlay_key(overlay, key.code);
        }
        match key.code {
            KeyCode::Char('f') => self.open_forms(),
            KeyCode::Char('g') => self.begin_edit(),
            KeyCode::Char('i') => self.overlay = Some(Overlay::Inspector),
            KeyCode::Char('b') => self.overlay = Some(Overlay::Companion),
            KeyCode::Char('?') => self.overlay = Some(Overlay::Help),
            KeyCode::Tab | KeyCode::BackTab => {
                return self.cycle_focus(key.code == KeyCode::BackTab);
            }
            KeyCode::Enter => match self.focus {
                Pane::Companion => self.open_forms(),
                Pane::Workspace => self.begin_edit(),
                Pane::Inspector => self.overlay = Some(Overlay::Inspector),
            },
            _ => return false,
        }
        true
    }

    fn edit_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.editing = false;
                self.input.clear();
                true
            }
            KeyCode::Enter => {
                self.goal = std::mem::take(&mut self.input);
                self.editing = false;
                true
            }
            KeyCode::Backspace => {
                if let Some((index, _)) = self.input.grapheme_indices(true).next_back() {
                    self.input.truncate(index);
                    true
                } else {
                    false
                }
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && !character.is_control()
                    && self.input.len() + character.len_utf8() <= MAX_GOAL_BYTES =>
            {
                self.input.push(character);
                true
            }
            _ => false,
        }
    }

    fn append_input(&mut self, text: &str) -> bool {
        let before = self.input.len();
        for character in text.chars().filter(|character| !character.is_control()) {
            if self.input.len() + character.len_utf8() > MAX_GOAL_BYTES {
                break;
            }
            self.input.push(character);
        }
        self.input.len() != before
    }

    fn open_forms(&mut self) {
        self.overlay = Some(Overlay::Forms);
        self.selected = self.form;
    }

    fn begin_edit(&mut self) {
        self.overlay = None;
        self.input.clone_from(&self.goal);
        self.editing = true;
    }

    fn overlay_key(&mut self, overlay: Overlay, key: KeyCode) -> bool {
        if key == KeyCode::Esc {
            self.overlay = None;
            return true;
        }
        let count = match overlay {
            Overlay::Forms => FORMS.len(),
            Overlay::Palette => PALETTE.len(),
            Overlay::Help => {
                if key == KeyCode::Char('?') {
                    self.overlay = None;
                    return true;
                }
                return false;
            }
            Overlay::Inspector => {
                if key == KeyCode::Char('i') {
                    self.overlay = None;
                    return true;
                }
                return false;
            }
            Overlay::Companion => {
                if matches!(key, KeyCode::Enter | KeyCode::Char('f')) {
                    self.open_forms();
                    return true;
                }
                if key == KeyCode::Char('b') {
                    self.overlay = None;
                    return true;
                }
                return false;
            }
        };
        let previous = self.selected;
        match key {
            KeyCode::Up => self.selected = (self.selected + count - 1) % count,
            KeyCode::Down => self.selected = (self.selected + 1) % count,
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = count - 1,
            KeyCode::Char(character)
                if overlay == Overlay::Forms && ('1'..='9').contains(&character) =>
            {
                self.selected = (character as u8 - b'1') as usize;
            }
            KeyCode::Enter => {
                self.overlay = None;
                if overlay == Overlay::Forms {
                    self.form = self.selected;
                } else {
                    match self.selected {
                        0 => self.open_forms(),
                        1 => self.begin_edit(),
                        2 => self.overlay = Some(Overlay::Inspector),
                        3 => self.overlay = Some(Overlay::Companion),
                        4 => self.overlay = Some(Overlay::Help),
                        5 => self.quit = true,
                        _ => unreachable!("palette selection is bounded"),
                    }
                }
                return true;
            }
            _ => return false,
        }
        previous != self.selected
    }

    fn cycle_focus(&mut self, backwards: bool) -> bool {
        if self.width < 90 {
            return false;
        }
        self.focus = if self.width < 120 {
            match self.focus {
                Pane::Companion => Pane::Workspace,
                _ => Pane::Companion,
            }
        } else {
            match (self.focus, backwards) {
                (Pane::Companion, false) | (Pane::Inspector, true) => Pane::Workspace,
                (Pane::Workspace, false) | (Pane::Companion, true) => Pane::Inspector,
                (Pane::Inspector, false) | (Pane::Workspace, true) => Pane::Companion,
            }
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App::new("/workspace".into(), 120, 36)
    }

    fn key(app: &mut App, code: KeyCode) -> bool {
        app.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    #[test]
    fn selecting_a_form_requires_confirmation_and_preserves_goal() {
        let mut app = app();
        app.goal = "Keep existing work".into();
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('9'));
        assert_eq!(app.form, 0);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.form, 0);
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Up);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.form, 8);
        assert_eq!(app.goal, "Keep existing work");
        assert_eq!(app.overlay, None);
    }

    #[test]
    fn editing_is_modal_and_cancel_keeps_the_committed_goal() {
        let mut app = app();
        app.goal = "original".into();
        key(&mut app, KeyCode::Char('g'));
        key(&mut app, KeyCode::Char('q'));
        assert!(!app.quit);
        assert!(!key(&mut app, KeyCode::Tab));
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.goal, "original");
        key(&mut app, KeyCode::Char('g'));
        key(&mut app, KeyCode::Char('!'));
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.goal, "original!");
    }

    #[test]
    fn backspace_removes_whole_unicode_graphemes() {
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("한e\u{301}👩‍💻".into()));
        key(&mut app, KeyCode::Backspace);
        assert_eq!(app.input, "한e\u{301}");
        key(&mut app, KeyCode::Backspace);
        assert_eq!(app.input, "한");
    }

    #[test]
    fn pasted_input_is_bounded_and_cannot_inject_controls_or_commands() {
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Paste("q\r\n\x1b\x07한".into()));
        assert_eq!(app.input, "q한");
        assert!(!app.quit);
        app.update(Event::Paste("界".repeat(MAX_GOAL_BYTES)));
        assert!(app.input.len() <= MAX_GOAL_BYTES);
        assert!(app.input.len() > MAX_GOAL_BYTES - 3);
        assert!(!app.update(Event::Paste("界".into())));
    }

    #[test]
    fn resizing_never_leaves_focus_on_a_hidden_pane() {
        let mut app = app();
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Pane::Inspector);
        app.update(Event::Resize(100, 30));
        assert_eq!(app.focus, Pane::Workspace);
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Pane::Companion);
        app.update(Event::Resize(70, 24));
        assert_eq!(app.focus, Pane::Workspace);
        assert!(!key(&mut app, KeyCode::Tab));
    }

    #[test]
    fn tiny_terminal_still_allows_cancellation_while_editing() {
        let mut app = app();
        key(&mut app, KeyCode::Char('g'));
        app.update(Event::Resize(10, 3));
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.quit);
    }

    #[test]
    fn palette_can_open_editor_without_repeating_release_events() {
        let mut app = app();
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        )));
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        assert!(app.editing);
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        assert!(!app.update(Event::Key(release)));
        assert_eq!(app.input, "");
    }
}
