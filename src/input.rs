use crossterm::event::Event;
use std::io;

pub struct Input {
    #[cfg(windows)]
    windows: windows::ConsoleInput,
}

impl Input {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            #[cfg(windows)]
            windows: windows::ConsoleInput::new()?,
        })
    }

    /// Forget partial native decoding and queued application keys at an explicit
    /// terminal ownership boundary. No background reader survives this handoff.
    pub fn discard_pending(&mut self) -> io::Result<()> {
        #[cfg(windows)]
        {
            windows::flush()?;
            self.windows = windows::ConsoleInput::new()?;
        }
        #[cfg(not(windows))]
        {
            for _ in 0..256 {
                if !crossterm::event::poll(std::time::Duration::ZERO)? {
                    return Ok(());
                }
                let _ = crossterm::event::read()?;
            }
            Err(io::Error::other(
                "Input did not settle at authentication boundary",
            ))
        }
        #[cfg(windows)]
        Ok(())
    }

    pub fn read(&mut self) -> io::Result<Event> {
        #[cfg(windows)]
        {
            self.windows.read()
        }
        #[cfg(not(windows))]
        {
            crossterm::event::read()
        }
    }

    pub fn read_timeout(&mut self, timeout: std::time::Duration) -> io::Result<Option<Event>> {
        #[cfg(windows)]
        {
            self.windows.read_timeout(timeout)
        }
        #[cfg(not(windows))]
        {
            if crossterm::event::poll(timeout)? {
                crossterm::event::read().map(Some)
            } else {
                Ok(None)
            }
        }
    }
}

#[cfg(windows)]
pub use windows::{capture_mode, enable_vt_input, restore_mode};

// Windows' ConsoleInput record API loses paste boundaries unless VT input is
// enabled, and Crossterm drops Alt-code key releases carrying combining marks.
// Decode the bounded VT envelope before any key reaches application navigation.
#[cfg(any(windows, test))]
mod decoding {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    const END: &str = "\x1b[201~";
    const MAX_PASTE: usize = 64 * 1024;

    #[derive(Default)]
    pub struct Decoder {
        sequence: String,
        paste: Option<String>,
        tail: String,
        truncated: bool,
    }

    impl Decoder {
        pub fn has_escape(&self) -> bool {
            self.paste.is_none() && !self.sequence.is_empty()
        }

        pub fn escape_timeout(&mut self) -> Event {
            let sequence = std::mem::take(&mut self.sequence);
            if sequence == "\x1b" {
                Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
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
            if !self.sequence.is_empty() || character == '\x1b' {
                self.sequence.push(character);
                if self.sequence == "\x1b" || self.sequence == "\x1b[" {
                    return None;
                }
                if self.sequence == "\x1b[200~" {
                    self.sequence.clear();
                    self.paste = Some(String::new());
                    self.truncated = false;
                    return None;
                }
                if self.sequence.starts_with("\x1b[")
                    && (character.is_ascii_digit() || character == ';')
                    && self.sequence.len() < 32
                {
                    return None;
                }
                let sequence = std::mem::take(&mut self.sequence);
                if !sequence.starts_with("\x1b[") && sequence.chars().count() == 2 {
                    return Some(Event::Key(KeyEvent::new(
                        KeyCode::Char(character),
                        modifiers | KeyModifiers::ALT,
                    )));
                }
                return Some(csi(&sequence).unwrap_or(Event::Paste(sequence)));
            }
            let (code, modifiers) = match character {
                '\r' | '\n' => (KeyCode::Enter, modifiers),
                '\t' => (
                    if modifiers.contains(KeyModifiers::SHIFT) {
                        KeyCode::BackTab
                    } else {
                        KeyCode::Tab
                    },
                    modifiers,
                ),
                '\u{7f}' | '\u{8}' => (KeyCode::Backspace, modifiers),
                '\u{1}'..='\u{1a}' => (
                    KeyCode::Char(char::from(character as u8 + b'a' - 1)),
                    modifiers | KeyModifiers::CONTROL,
                ),
                _ => (KeyCode::Char(character), modifiers),
            };
            Some(Event::Key(KeyEvent::new(code, modifiers)))
        }

        pub fn is_pasting(&self) -> bool {
            self.paste.is_some()
        }
    }

    fn csi(sequence: &str) -> Option<Event> {
        let body = sequence.strip_prefix("\x1b[")?;
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
        let code = match final_char {
            'A' => KeyCode::Up,
            'B' => KeyCode::Down,
            'C' => KeyCode::Right,
            'D' => KeyCode::Left,
            'H' => KeyCode::Home,
            'F' => KeyCode::End,
            'Z' => KeyCode::BackTab,
            '~' => match first {
                "1" | "7" => KeyCode::Home,
                "4" | "8" => KeyCode::End,
                "3" => KeyCode::Delete,
                "5" => KeyCode::PageUp,
                "6" => KeyCode::PageDown,
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
            value
                .chars()
                .filter_map(|character| decoder.character(character, KeyModifiers::NONE))
                .collect()
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
        decoder: Decoder,
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
        pub fn read(&mut self) -> io::Result<Event> {
            self.read_inner(None)
                .map(|event| event.expect("blocking input has no deadline"))
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
