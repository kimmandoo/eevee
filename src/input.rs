use crossterm::event::Event;
use std::io;
use std::time::{Duration, Instant};

/// Only positive terminal replies establish protocol support. Cell dimensions
/// are physical pixels, (width, height), never estimates from terminal columns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphicsCapabilities {
    pub kitty: bool,
    pub sixel: bool,
    pub cell_size: Option<(u16, u16)>,
}

const GRAPHICS_QUERY: &[u8] = b"\x1b[c\x1b_Gi=314159,a=q,f=24,s=1,v=1;AAAA\x1b\\\x1b[16t";
const QUERY_TIMEOUT: Duration = Duration::from_millis(500);

pub struct Input {
    #[cfg(windows)]
    windows: windows::ConsoleInput,
    #[cfg(unix)]
    unix: unix::TerminalInput,
}

impl Input {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            #[cfg(windows)]
            windows: windows::ConsoleInput::new()?,
            #[cfg(unix)]
            unix: unix::TerminalInput::new()?,
        })
    }

    fn decoder(&mut self) -> &mut decoding::Decoder {
        #[cfg(windows)]
        {
            &mut self.windows.decoder
        }
        #[cfg(unix)]
        {
            &mut self.unix.decoder
        }
    }

    /// The caller gates this on image/color policy. Queries are terminal control
    /// traffic only: the sole input owner demultiplexes replies from user input.
    /// No second reader, startup drain, or unbounded discovery wait is involved.
    pub fn query_graphics(&mut self, output: &mut impl io::Write) -> io::Result<()> {
        use io::IsTerminal;
        if !io::stdin().is_terminal()
            || !io::stdout().is_terminal()
            || std::env::var("TERM").as_deref() == Ok("dumb")
            || ["TMUX", "STY"]
                .iter()
                .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
        {
            return Ok(());
        }
        self.decoder().query_graphics();
        output.write_all(GRAPHICS_QUERY)?;
        output.flush()
    }

    pub fn graphics_pending(&mut self) -> bool {
        self.decoder().graphics_pending()
    }

    pub fn graphics_capabilities(&mut self) -> GraphicsCapabilities {
        self.decoder().graphics
    }

    pub fn take_graphics_update(&mut self) -> Option<GraphicsCapabilities> {
        self.decoder().expire_graphics_query();
        std::mem::take(&mut self.decoder().graphics_changed).then(|| self.graphics_capabilities())
    }

    /// Forget partial decoding and application keys only at an explicit auth
    /// ownership boundary. Late reply filtering remains active after the reset.
    pub fn discard_pending(&mut self) -> io::Result<()> {
        #[cfg(windows)]
        {
            windows::flush()?;
            self.windows = windows::ConsoleInput::new()?;
            Ok(())
        }
        #[cfg(unix)]
        {
            self.unix.discard_pending()
        }
    }

    /// None wakes the caller for a graphics update, without inventing a UI event.
    pub fn read(&mut self) -> io::Result<Option<Event>> {
        #[cfg(windows)]
        {
            self.windows.read()
        }
        #[cfg(unix)]
        {
            self.unix.read()
        }
    }

    pub fn read_timeout(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        #[cfg(windows)]
        {
            self.windows.read_timeout(timeout)
        }
        #[cfg(unix)]
        {
            self.unix.read_timeout(timeout)
        }
    }
}

#[cfg(unix)]
#[path = "input/unix.rs"]
mod unix;

#[cfg(windows)]
pub use windows::{capture_mode, enable_vt_input, restore_mode};

// Both platforms share framing and graphics reply ownership. Windows still uses
// native console records for Alt-code releases, repeats and supplementary input.
mod decoding {
    use super::{GraphicsCapabilities, Instant, QUERY_TIMEOUT};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use std::collections::VecDeque;
    const END: &str = "\x1b[201~";
    const MAX_PASTE: usize = 64 * 1024;

    #[derive(Default)]
    pub struct Decoder {
        sequence: String,
        paste: Option<String>,
        tail: String,
        truncated: bool,
        pub(super) graphics: GraphicsCapabilities,
        pub(super) graphics_changed: bool,
        query_deadline: Option<Instant>,
        query_finished: bool,
        reply_overflow: bool,
        expired_escape: bool,
        detached_escape: bool,
        queued: VecDeque<Event>,
    }

    impl Decoder {
        pub fn has_escape(&self) -> bool {
            self.paste.is_none() && matches!(self.sequence.as_str(), "\x1b" | "\x1bO")
        }

        pub(super) fn query_graphics(&mut self) {
            self.query_finished = false;
            self.query_deadline = Some(Instant::now() + QUERY_TIMEOUT);
            if self.graphics.cell_size.take().is_some() {
                self.graphics_changed = true;
            }
        }

        pub(super) fn graphics_pending(&mut self) -> bool {
            self.expire_graphics_query();
            self.query_deadline.is_some() && !self.query_finished
        }

        pub(super) fn graphics_wait(&self) -> Option<std::time::Duration> {
            self.query_deadline
                .filter(|_| !self.query_finished)
                .map(|deadline| deadline.saturating_duration_since(Instant::now()))
        }

        pub(super) fn expire_graphics_query(&mut self) {
            if !self.query_finished
                && self
                    .query_deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
            {
                self.query_finished = true;
                self.graphics_changed = true;
            }
        }

        pub(super) fn next_event(&mut self) -> Option<Event> {
            self.queued.pop_front()
        }

        fn graphics_reply(&mut self, sequence: &str) -> bool {
            let before = self.graphics;
            if let Some(body) = sequence
                .strip_prefix("\x1b[?")
                .and_then(|s| s.strip_suffix('c'))
            {
                self.graphics.sixel |= body.split(';').any(|part| part == "4");
            } else if let Some(body) = sequence
                .strip_prefix("\x1b[6;")
                .and_then(|s| s.strip_suffix('t'))
            {
                let mut parts = body.split(';');
                if let (Some(height), Some(width), None) =
                    (parts.next(), parts.next(), parts.next())
                    && let (Ok(height), Ok(width)) = (height.parse::<u16>(), width.parse::<u16>())
                    && height > 0
                    && width > 0
                {
                    self.graphics.cell_size = Some((width, height));
                }
            } else if let Some(body) = sequence
                .strip_prefix("\x1b_G")
                .and_then(|s| s.strip_suffix("\x1b\\"))
            {
                if let Some((parameters, "OK")) = body.split_once(';')
                    && parameters.split(',').any(|part| part == "i=314159")
                {
                    self.graphics.kitty = true;
                }
            } else {
                return false;
            }
            if self.query_deadline.is_none() {
                self.graphics = before;
            }
            self.graphics_changed |= before != self.graphics;
            true
        }

        pub fn escape_timeout(&mut self) -> Event {
            let sequence = std::mem::take(&mut self.sequence);
            if sequence == "\x1b" {
                // Escape has a bounded ambiguity wait, but a very late reply
                // can still continue with '[' or '_'. Retain that possibility
                // so its printable suffix never becomes composer input.
                self.expired_escape = self.query_deadline.is_some();
                Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            } else if sequence == "\x1bO" {
                Event::Key(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::ALT))
            } else {
                Event::Paste(sequence)
            }
        }

        pub fn character(&mut self, character: char, modifiers: KeyModifiers) -> Option<Event> {
            if let Some(paste) = self.paste.as_mut() {
                self.tail.push(character);
                while !END.starts_with(&self.tail) {
                    let first = self.tail.chars().next().expect("pending paste character");
                    if !self.truncated && paste.len() + first.len_utf8() <= MAX_PASTE {
                        paste.push(first);
                    } else {
                        self.truncated = true;
                    }
                    self.tail.drain(..first.len_utf8());
                }
                if self.tail == END {
                    self.tail.clear();
                    return Some(Event::Paste(self.paste.take().expect("active paste")));
                }
                return None;
            }
            if std::mem::take(&mut self.expired_escape) && matches!(character, '[' | '_') {
                self.sequence.push('\x1b');
                self.detached_escape = true;
            }
            if !self.sequence.is_empty() || character == '\x1b' {
                // APC graphics replies are bounded in memory but drained through
                // ST even after the query deadline. Their bytes are never text.
                if self.sequence.starts_with("\x1b_") {
                    if !self.reply_overflow && self.sequence == "\x1b_" && character != 'G' {
                        self.sequence.clear();
                        let first = key(
                            '_',
                            if std::mem::take(&mut self.detached_escape) {
                                KeyModifiers::NONE
                            } else {
                                KeyModifiers::ALT
                            },
                        );
                        self.queued.push_back(key(character, modifiers));
                        return Some(first);
                    }
                    let ended = self.sequence.ends_with('\x1b') && character == '\\';
                    if !self.reply_overflow {
                        self.sequence.push(character);
                        self.reply_overflow = self.sequence.len() > 512;
                    } else {
                        self.sequence.truncate(2);
                        if character == '\x1b' {
                            self.sequence.push(character);
                        }
                    }
                    if ended {
                        let sequence = std::mem::take(&mut self.sequence);
                        if !self.reply_overflow {
                            self.graphics_reply(&sequence);
                        }
                        self.reply_overflow = false;
                        self.detached_escape = false;
                    }
                    return None;
                }
                self.sequence.push(character);
                if matches!(
                    self.sequence.as_str(),
                    "\x1b" | "\x1b[" | "\x1b[[" | "\x1bO" | "\x1b_"
                ) {
                    return None;
                }
                if self.sequence == "\x1b[200~" {
                    self.sequence.clear();
                    self.paste = Some(String::new());
                    self.truncated = false;
                    self.detached_escape = false;
                    return None;
                }
                if self.sequence.starts_with("\x1b[") && (' '..='?').contains(&character) {
                    if self.sequence.len() > 512 {
                        self.sequence.truncate(2);
                        self.reply_overflow = true;
                    }
                    return None;
                }
                let sequence = std::mem::take(&mut self.sequence);
                if std::mem::take(&mut self.reply_overflow) || self.graphics_reply(&sequence) {
                    self.detached_escape = false;
                    return None;
                }
                if std::mem::take(&mut self.detached_escape) {
                    self.queued.extend(
                        sequence
                            .chars()
                            .skip(1)
                            .map(|character| key(character, KeyModifiers::NONE)),
                    );
                    return self.next_event();
                }
                if !sequence.starts_with("\x1b[") && sequence.chars().count() == 2 {
                    return Some(key(character, modifiers | KeyModifiers::ALT));
                }
                return Some(csi(&sequence).unwrap_or(Event::Paste(sequence)));
            }
            Some(key(character, modifiers))
        }

        #[cfg(any(windows, test))]
        pub fn is_pasting(&self) -> bool {
            self.paste.is_some()
        }
    }

    fn key(character: char, modifiers: KeyModifiers) -> Event {
        let (code, modifiers) = match character {
            '\x1b' => (KeyCode::Esc, modifiers),
            '\r' => (KeyCode::Enter, modifiers),
            '\n' if cfg!(windows) => (KeyCode::Enter, modifiers),
            '\t' => (
                if modifiers.contains(KeyModifiers::SHIFT) {
                    KeyCode::BackTab
                } else {
                    KeyCode::Tab
                },
                modifiers,
            ),
            '\u{7f}' => (KeyCode::Backspace, modifiers),
            '\u{8}' if cfg!(windows) => (KeyCode::Backspace, modifiers),
            '\u{1}'..='\u{1a}' => (
                KeyCode::Char(char::from(character as u8 + b'a' - 1)),
                modifiers | KeyModifiers::CONTROL,
            ),
            '\u{1c}'..='\u{1f}' => (
                KeyCode::Char(char::from(character as u8 - 0x1c + b'4')),
                modifiers | KeyModifiers::CONTROL,
            ),
            '\0' => (KeyCode::Char(' '), modifiers | KeyModifiers::CONTROL),
            _ => (KeyCode::Char(character), modifiers),
        };
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn csi(sequence: &str) -> Option<Event> {
        if let Some(body) = sequence.strip_prefix("\x1b[[") {
            let bytes = body.as_bytes();
            return (bytes.len() == 1 && (b'A'..=b'E').contains(&bytes[0])).then(|| {
                Event::Key(KeyEvent::new(
                    KeyCode::F(bytes[0] - b'A' + 1),
                    KeyModifiers::NONE,
                ))
            });
        }
        let body = sequence
            .strip_prefix("\x1b[")
            .or_else(|| sequence.strip_prefix("\x1bO"))?;
        let final_byte = *body.as_bytes().last()?;
        if !final_byte.is_ascii() {
            return None;
        }
        let final_char = char::from(final_byte);
        let parameters = &body[..body.len() - 1];
        let mut values = parameters.split(';');
        let first = values.next().unwrap_or("");
        let flags = values
            .next()
            .unwrap_or("1")
            .parse::<u8>()
            .ok()?
            .checked_sub(1)?;
        if flags > 7 || values.next().is_some() {
            return None;
        }
        let mut modifiers = KeyModifiers::NONE;
        if flags & 1 != 0 {
            modifiers |= KeyModifiers::SHIFT;
        }
        if flags & 2 != 0 {
            modifiers |= KeyModifiers::ALT;
        }
        if flags & 4 != 0 {
            modifiers |= KeyModifiers::CONTROL;
        }
        if final_char == 'Z' {
            modifiers |= KeyModifiers::SHIFT;
        }
        let code = match final_char {
            'A' => KeyCode::Up,
            'B' => KeyCode::Down,
            'C' => KeyCode::Right,
            'D' => KeyCode::Left,
            'H' => KeyCode::Home,
            'F' => KeyCode::End,
            'I' if parameters.is_empty() => return Some(Event::FocusGained),
            'O' if parameters.is_empty() => return Some(Event::FocusLost),
            'Z' => KeyCode::BackTab,
            'P'..='S' => KeyCode::F(final_byte - b'P' + 1),
            '~' => match first.parse::<u8>().ok()? {
                1 | 7 => KeyCode::Home,
                2 => KeyCode::Insert,
                4 | 8 => KeyCode::End,
                3 => KeyCode::Delete,
                5 => KeyCode::PageUp,
                6 => KeyCode::PageDown,
                value @ 11..=15 => KeyCode::F(value - 10),
                value @ 17..=21 => KeyCode::F(value - 11),
                value @ 23..=26 => KeyCode::F(value - 12),
                value @ 28..=29 => KeyCode::F(value - 15),
                value @ 31..=34 => KeyCode::F(value - 17),
                _ => return None,
            },
            'u' => match first.parse::<u32>().ok()? {
                13 => KeyCode::Enter,
                27 => KeyCode::Esc,
                code => KeyCode::Char(char::from_u32(code)?),
            },
            _ => return None,
        };
        Some(Event::Key(KeyEvent::new(code, modifiers)))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn feed(decoder: &mut Decoder, value: &str) -> Vec<Event> {
            let mut events = Vec::new();
            for character in value.chars() {
                events.extend(decoder.character(character, KeyModifiers::NONE));
                while let Some(event) = decoder.next_event() {
                    events.push(event);
                }
            }
            events
        }
        #[test]
        fn function_navigation_decodes_ss3_csi_and_modified_keys_without_typing_letters() {
            let mut decoder = Decoder::default();
            for (sequence, function, modifiers) in [
                ("\x1bOP", 1, KeyModifiers::NONE),
                ("\x1bOQ", 2, KeyModifiers::NONE),
                ("\x1bOR", 3, KeyModifiers::NONE),
                ("\x1bOS", 4, KeyModifiers::NONE),
                ("\x1b[12~", 2, KeyModifiers::NONE),
                ("\x1b[24~", 12, KeyModifiers::NONE),
                ("\x1b[34;5~", 17, KeyModifiers::CONTROL),
                ("\x1b[[E", 5, KeyModifiers::NONE),
                ("\x1b[1;2S", 4, KeyModifiers::SHIFT),
            ] {
                assert_eq!(
                    feed(&mut decoder, sequence),
                    vec![Event::Key(KeyEvent::new(KeyCode::F(function), modifiers))]
                );
            }
            assert_eq!(
                feed(&mut decoder, "\x1b[200~\x1bOQ\x1b[201~"),
                vec![Event::Paste("\x1bOQ".into())]
            );
            assert!(feed(&mut decoder, "\x1bO").is_empty());
            assert_eq!(
                decoder.escape_timeout(),
                Event::Key(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::ALT))
            );
        }
        #[test]
        fn paste_is_one_literal_event_not_navigation_or_local_apply() {
            let mut decoder = Decoder::default();
            let payload = "qf?\n漢字 cafe\u{301}\x1b]0;title\x07";
            assert_eq!(
                feed(&mut decoder, &format!("\x1b[200~{payload}\x1b[201~")),
                vec![Event::Paste(payload.into())]
            );
            assert_eq!(
                feed(&mut decoder, "g"),
                vec![Event::Key(KeyEvent::new(
                    KeyCode::Char('g'),
                    KeyModifiers::NONE
                ))]
            );
        }
        #[test]
        fn truncated_paste_still_drains_until_its_exact_end_marker() {
            let mut decoder = Decoder::default();
            assert!(feed(&mut decoder, "\x1b[200~").is_empty());
            let prefix = "x".repeat(MAX_PASTE - 1);
            assert!(feed(&mut decoder, &format!("{prefix}漢z{}", "x".repeat(100))).is_empty());
            assert!(feed(&mut decoder, "\x1b[20").is_empty());
            assert_eq!(feed(&mut decoder, "1~"), vec![Event::Paste(prefix)]);
            assert!(!decoder.is_pasting());
        }
        #[test]
        fn escape_and_modified_cursor_keys_are_distinct_from_paste() {
            let mut decoder = Decoder::default();
            assert!(feed(&mut decoder, "\x1b").is_empty());
            assert!(decoder.has_escape());
            assert_eq!(
                decoder.escape_timeout(),
                Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            );
            assert_eq!(
                feed(&mut decoder, "\x1b[1;2C"),
                vec![Event::Key(KeyEvent::new(
                    KeyCode::Right,
                    KeyModifiers::SHIFT
                ))]
            );
        }

        #[test]
        fn partial_vt_and_paste_resume_at_every_character_boundary() {
            for (input, expected) in [
                (
                    "\x1b[1;2C",
                    Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT)),
                ),
                (
                    "\x1b[200~漢😀e\u{301}\x1b[20x\x1b[201~",
                    Event::Paste("漢😀e\u{301}\x1b[20x".into()),
                ),
            ] {
                for (split, _) in input.char_indices().skip(1) {
                    let mut decoder = Decoder::default();
                    assert!(feed(&mut decoder, &input[..split]).is_empty());
                    // A caller deadline is not an Escape timeout: preserve the
                    // prefix/paste until the next read supplies more characters.
                    assert_eq!(feed(&mut decoder, &input[split..]), vec![expected.clone()]);
                    assert!(!decoder.has_escape());
                    assert!(!decoder.is_pasting());
                }
            }
        }

        #[test]
        fn non_ascii_after_incomplete_csi_remains_literal_without_panicking() {
            let mut decoder = Decoder::default();
            assert_eq!(
                feed(&mut decoder, "\x1b[한"),
                vec![Event::Paste("\x1b[한".into())]
            );
        }

        const REPLIES: &str = "\x1b[?62;4c\x1b_Gi=314159;OK\x1b\\\x1b[6;20;10t";

        #[test]
        fn capability_replies_and_typing_have_one_owner_at_every_split() {
            for split in 0..=REPLIES.len() {
                let mut decoder = Decoder::default();
                decoder.query_graphics();
                assert_eq!(
                    feed(&mut decoder, "a@"),
                    feed(&mut Decoder::default(), "a@")
                );
                assert!(feed(&mut decoder, &REPLIES[..split]).is_empty());
                // A deadline stops waiting, not ownership of a partially read
                // control envelope. Even wholly late replies remain filtered.
                decoder.query_deadline = Some(Instant::now() - QUERY_TIMEOUT);
                assert!(!decoder.graphics_pending());
                assert!(feed(&mut decoder, &REPLIES[split..]).is_empty());
                assert_eq!(
                    feed(&mut decoder, "/漢"),
                    feed(&mut Decoder::default(), "/漢")
                );
                assert_eq!(
                    decoder.graphics,
                    GraphicsCapabilities {
                        kitty: true,
                        sixel: true,
                        cell_size: Some((10, 20)),
                    }
                );
                assert!(decoder.graphics_changed);
            }
        }

        #[test]
        fn late_reply_after_escape_ambiguity_timeout_never_types_its_suffix() {
            for reply in ["\x1b[?62;4c", "\x1b_Gi=314159;OK\x1b\\", "\x1b[6;20;10t"] {
                let mut decoder = Decoder::default();
                decoder.query_graphics();
                assert!(feed(&mut decoder, "\x1b").is_empty());
                assert_eq!(decoder.escape_timeout(), key('\x1b', KeyModifiers::NONE));
                decoder.query_deadline = Some(Instant::now() - QUERY_TIMEOUT);
                assert!(feed(&mut decoder, &reply[1..]).is_empty());
                assert_eq!(feed(&mut decoder, "z"), vec![key('z', KeyModifiers::NONE)]);
            }
            let mut decoder = Decoder::default();
            decoder.query_graphics();
            assert!(feed(&mut decoder, "\x1b").is_empty());
            let _ = decoder.escape_timeout();
            // If it was a real Escape followed by literal typing, replay each
            // non-response character once, rather than inventing a key chord.
            assert_eq!(
                feed(&mut decoder, "[word"),
                feed(&mut Decoder::default(), "[word")
            );
        }

        #[test]
        fn paste_keeps_reply_looking_bytes_literal_across_query_expiry() {
            let mut decoder = Decoder::default();
            decoder.query_graphics();
            let payload = format!("q\n漢e\u{301}{REPLIES}@/");
            assert!(feed(&mut decoder, "\x1b[200~").is_empty());
            assert!(feed(&mut decoder, &payload).is_empty());
            decoder.query_deadline = Some(Instant::now() - QUERY_TIMEOUT);
            assert!(feed(&mut decoder, "\x1b[20").is_empty());
            assert_eq!(feed(&mut decoder, "1~"), vec![Event::Paste(payload)]);
            assert_eq!(decoder.graphics, GraphicsCapabilities::default());
            assert!(feed(&mut decoder, REPLIES).is_empty());
            assert!(decoder.graphics.kitty && decoder.graphics.sixel);
        }

        #[test]
        fn unsupported_unsolicited_and_malformed_replies_do_not_claim_support() {
            let mut decoder = Decoder::default();
            assert!(feed(&mut decoder, REPLIES).is_empty());
            assert_eq!(decoder.graphics, GraphicsCapabilities::default());
            decoder.query_graphics();
            assert!(feed(&mut decoder,
                "\x1b[?62;1;2c\x1b_Gi=99;OK\x1b\\\x1b_Gi=314159;EINVAL\x1b\\\x1b[6;0;10t\x1b[6;20;999999t"
            ).is_empty());
            assert_eq!(decoder.graphics, GraphicsCapabilities::default());
            assert!(!decoder.graphics_changed);
            assert_eq!(
                feed(&mut decoder, "ok"),
                feed(&mut Decoder::default(), "ok")
            );
        }

        #[test]
        fn oversized_reply_is_bounded_and_drains_before_the_next_key() {
            let mut decoder = Decoder::default();
            decoder.query_graphics();
            assert!(feed(&mut decoder, &format!("\x1b_G{}", "x".repeat(2048))).is_empty());
            assert!(decoder.sequence.len() <= 513);
            assert!(feed(&mut decoder, "\x1b\\").is_empty());
            assert!(feed(&mut decoder, &format!("\x1b[?{}", "1;".repeat(2048))).is_empty());
            assert!(decoder.sequence.len() <= 513);
            assert!(feed(&mut decoder, "c").is_empty());
            assert_eq!(feed(&mut decoder, "q"), vec![key('q', KeyModifiers::NONE)]);
            assert_eq!(decoder.graphics, GraphicsCapabilities::default());
        }

        #[test]
        fn resizing_invalidates_measured_cells_until_the_terminal_replies() {
            let mut decoder = Decoder::default();
            decoder.query_graphics();
            assert!(feed(&mut decoder, REPLIES).is_empty());
            decoder.graphics_changed = false;
            decoder.query_graphics();
            assert_eq!(decoder.graphics.cell_size, None);
            assert!(decoder.graphics.kitty && decoder.graphics.sixel);
            assert!(decoder.graphics_changed);
            assert!(feed(&mut decoder, "\x1b[6;24;12t").is_empty());
            assert_eq!(decoder.graphics.cell_size, Some((12, 24)));
        }

        #[test]
        fn unsupported_probe_completion_notifies_once_without_releasing_reply_ownership() {
            let mut decoder = Decoder::default();
            decoder.query_graphics();
            decoder.query_deadline = Some(Instant::now() - QUERY_TIMEOUT);
            assert!(!decoder.graphics_pending());
            assert!(decoder.graphics_changed);
            assert_eq!(decoder.graphics, GraphicsCapabilities::default());
            assert_eq!(decoder.graphics_wait(), None);
            decoder.graphics_changed = false;
            decoder.expire_graphics_query();
            assert!(!decoder.graphics_changed);
            assert!(feed(&mut decoder, REPLIES).is_empty());
            assert!(decoder.graphics_changed);
            assert!(decoder.graphics.kitty && decoder.graphics.sixel);
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::{Event, decoding::Decoder, io};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::{
        Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_TIMEOUT},
        System::{Console::*, Threading::WaitForSingleObject},
    };

    fn handle() -> io::Result<HANDLE> {
        // SAFETY: fetching the process's existing input handle allocates nothing.
        let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(handle)
        }
    }
    pub fn capture_mode() -> io::Result<u32> {
        let mut mode = 0;
        // SAFETY: mode is a writable DWORD and handle belongs to this console.
        if unsafe { GetConsoleMode(handle()?, &mut mode) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(mode)
        }
    }
    pub fn enable_vt_input() -> io::Result<()> {
        restore_mode(capture_mode()? | ENABLE_WINDOW_INPUT | ENABLE_VIRTUAL_TERMINAL_INPUT)
    }
    pub fn restore_mode(mode: u32) -> io::Result<()> {
        // SAFETY: mode is a console input mode, not a pointer or external command.
        if unsafe { SetConsoleMode(handle()?, mode) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn flush() -> io::Result<()> {
        // SAFETY: discard queued input only at the explicit ownership boundary.
        if unsafe { FlushConsoleInputBuffer(handle()?) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub struct ConsoleInput {
        handle: HANDLE,
        pub(super) decoder: Decoder,
        surrogate: Option<u16>,
        pending: Option<(KEY_EVENT_RECORD, u16)>,
        queued: Option<Event>,
        escape_started: Option<Instant>,
    }
    impl ConsoleInput {
        pub fn new() -> io::Result<Self> {
            Ok(Self {
                handle: handle()?,
                decoder: Decoder::default(),
                surrogate: None,
                pending: None,
                queued: None,
                escape_started: None,
            })
        }
        pub fn read(&mut self) -> io::Result<Option<Event>> {
            self.read_inner(None)
        }

        pub fn read_timeout(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
            self.read_inner(Some(timeout))
        }

        fn read_inner(&mut self, timeout: Option<Duration>) -> io::Result<Option<Event>> {
            let started = Instant::now();
            let mut first_poll = true;
            let mut expired_records = None;
            if let Some(event) = self.queued.take() {
                return Ok(Some(event));
            }
            loop {
                self.decoder.expire_graphics_query();
                if let Some(event) = self.decoder.next_event() {
                    return Ok(Some(event));
                }
                if self.decoder.graphics_changed {
                    return Ok(None);
                }
                let (key, repeated) = if let Some((key, remaining)) = self.pending.take() {
                    first_poll = false;
                    if remaining > 1 {
                        self.pending = Some((key, remaining - 1));
                    }
                    (key, true)
                } else {
                    // Caller timeouts leave all decoding state intact, including
                    // the Escape ambiguity timer across successive reads.
                    let escape_remaining = self
                        .escape_started
                        .map(|started| Duration::from_millis(50).saturating_sub(started.elapsed()));
                    if escape_remaining != Some(Duration::ZERO) {
                        expired_records = None;
                    }
                    let remaining =
                        timeout.map(|timeout| timeout.saturating_sub(started.elapsed()));
                    if !first_poll && remaining == Some(Duration::ZERO) {
                        return Ok(None);
                    }
                    // After Escape expires, drain only the already-ready batch.
                    // This preserves queued CSI bytes after a slow UI turn,
                    // without allowing a stream of irrelevant records to keep
                    // an expired Escape alive forever.
                    if expired_records == Some(0) {
                        self.escape_started = None;
                        return Ok(Some(self.decoder.escape_timeout()));
                    }
                    first_poll = false;
                    let wait = match (remaining, escape_remaining) {
                        (Some(left), Some(right)) => Some(left.min(right)),
                        (left, right) => left.or(right),
                    };
                    let wait = match (wait, self.decoder.graphics_wait()) {
                        (Some(left), Some(right)) => Some(left.min(right)),
                        (left, right) => left.or(right),
                    };
                    // Round up sub-millisecond waits; never use INFINITE for a
                    // finite deadline. Recheck long waits and non-key records.
                    let milliseconds = wait.map_or(u32::MAX, |wait| {
                        wait.as_millis()
                            .saturating_add(u128::from(wait.subsec_nanos() % 1_000_000 != 0))
                            .min(u128::from(u32::MAX - 1)) as u32
                    });
                    // SAFETY: this is the existing console input wait handle;
                    // this object remains the application's sole input reader.
                    match unsafe { WaitForSingleObject(self.handle, milliseconds) } {
                        WAIT_FAILED => return Err(io::Error::last_os_error()),
                        WAIT_TIMEOUT => {
                            if escape_remaining == Some(Duration::ZERO) {
                                self.escape_started = None;
                                return Ok(Some(self.decoder.escape_timeout()));
                            }
                            continue;
                        }
                        _ => {}
                    }
                    if escape_remaining == Some(Duration::ZERO) {
                        let remaining = match expired_records {
                            Some(remaining) => remaining,
                            None => {
                                let mut count = 0;
                                // SAFETY: count is writable and this object is
                                // the console's sole reader.
                                if unsafe { GetNumberOfConsoleInputEvents(self.handle, &mut count) }
                                    == 0
                                {
                                    return Err(io::Error::last_os_error());
                                }
                                count
                            }
                        };
                        expired_records = Some(remaining.saturating_sub(1));
                    }
                    // SAFETY: these writable values are properly sized, and one
                    // record is requested from the application's sole reader.
                    let mut record: INPUT_RECORD = unsafe { std::mem::zeroed() };
                    let mut count = 0;
                    if unsafe { ReadConsoleInputW(self.handle, &mut record, 1, &mut count) } == 0 {
                        return Err(io::Error::last_os_error());
                    }
                    if count == 0 {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "console input ended",
                        ));
                    }
                    if u32::from(record.EventType) == WINDOW_BUFFER_SIZE_EVENT {
                        let (width, height) = crossterm::terminal::size()?;
                        return Ok(Some(Event::Resize(width, height)));
                    }
                    if u32::from(record.EventType) != KEY_EVENT {
                        continue;
                    }
                    // SAFETY: the record's union tag was checked above.
                    let key = unsafe { record.Event.KeyEvent };
                    if key.wRepeatCount > 1 {
                        self.pending = Some((key, key.wRepeatCount - 1));
                    }
                    (key, false)
                };
                // SAFETY: this is the Unicode variant of ReadConsoleInputW.
                let code = unsafe { key.uChar.UnicodeChar };
                let alt_code = key.wVirtualKeyCode == 0x12 && key.bKeyDown == 0 && code != 0;
                if key.bKeyDown == 0 && !alt_code {
                    continue;
                }
                let state = key.dwControlKeyState;
                let mut modifiers = KeyModifiers::NONE;
                if state & SHIFT_PRESSED != 0 {
                    modifiers |= KeyModifiers::SHIFT;
                }
                if state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0 {
                    modifiers |= KeyModifiers::CONTROL;
                }
                if state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0 {
                    modifiers |= KeyModifiers::ALT;
                }
                if alt_code
                    || (code >= 32
                        && state & RIGHT_ALT_PRESSED != 0
                        && state & LEFT_CTRL_PRESSED != 0)
                {
                    modifiers.remove(KeyModifiers::CONTROL | KeyModifiers::ALT);
                }
                if code != 0 {
                    let character = match code {
                        0xD800..=0xDBFF => {
                            self.surrogate = Some(code);
                            continue;
                        }
                        0xDC00..=0xDFFF => {
                            let Some(high) = self.surrogate.take() else {
                                continue;
                            };
                            let Some(Ok(character)) = char::decode_utf16([high, code]).next()
                            else {
                                continue;
                            };
                            character
                        }
                        code => {
                            self.surrogate = None;
                            char::from_u32(u32::from(code)).expect("non-surrogate u16")
                        }
                    };
                    let decoded = self.decoder.character(character, modifiers);
                    self.escape_started = self.decoder.has_escape().then(Instant::now);
                    if let Some(mut event) = decoded {
                        if repeated && let Event::Key(key) = &mut event {
                            key.kind = crossterm::event::KeyEventKind::Repeat;
                        }
                        return Ok(Some(event));
                    }
                    continue;
                }
                if self.decoder.is_pasting() {
                    continue;
                }
                let code = match key.wVirtualKeyCode {
                    0x08 => KeyCode::Backspace,
                    0x09 => {
                        if modifiers.contains(KeyModifiers::SHIFT) {
                            KeyCode::BackTab
                        } else {
                            KeyCode::Tab
                        }
                    }
                    0x0D => KeyCode::Enter,
                    0x1B => KeyCode::Esc,
                    0x21 => KeyCode::PageUp,
                    0x22 => KeyCode::PageDown,
                    0x23 => KeyCode::End,
                    0x24 => KeyCode::Home,
                    0x25 => KeyCode::Left,
                    0x26 => KeyCode::Up,
                    0x27 => KeyCode::Right,
                    0x28 => KeyCode::Down,
                    0x2E => KeyCode::Delete,
                    0x70..=0x87 => KeyCode::F((key.wVirtualKeyCode - 0x6F) as u8),
                    _ => continue,
                };
                let event = KeyEvent::new(code, modifiers);
                if self.decoder.has_escape() {
                    self.queued = Some(Event::Key(event));
                    self.escape_started = None;
                    return Ok(Some(self.decoder.escape_timeout()));
                }
                let mut event = event;
                if repeated {
                    event.kind = crossterm::event::KeyEventKind::Repeat;
                }
                return Ok(Some(Event::Key(event)));
            }
        }
    }
}
