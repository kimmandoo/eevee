use super::{Event, decoding::Decoder, io};
use crossterm::event::KeyModifiers;
use std::io::Read;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// A single raw-byte reader owns both user input and terminal replies. A signal
/// self-pipe wakes poll on resize without polling an idle terminal or racing a
/// SIGWINCH between checking a flag and entering a blocking read.
pub(super) struct TerminalInput {
    pub(super) decoder: Decoder,
    input: RawFd,
    resize: UnixStream,
    resize_signal: signal_hook::SigId,
    bytes: [u8; 4096],
    position: usize,
    length: usize,
    utf8: Utf8,
    escape_started: Option<Instant>,
}

impl TerminalInput {
    pub(super) fn new() -> io::Result<Self> {
        let (resize, writer) = UnixStream::pair()?;
        resize.set_nonblocking(true)?;
        let resize_signal = signal_hook::low_level::pipe::register(libc::SIGWINCH, writer)?;
        Ok(Self {
            decoder: Decoder::default(),
            input: io::stdin().as_raw_fd(),
            resize,
            resize_signal,
            bytes: [0; 4096],
            position: 0,
            length: 0,
            utf8: Utf8::default(),
            escape_started: None,
        })
    }

    pub(super) fn discard_pending(&mut self) -> io::Result<()> {
        // SAFETY: stdin is the terminal fd, and this explicit auth boundary is
        // the only place queued input is intentionally discarded.
        if unsafe { libc::tcflush(self.input, libc::TCIFLUSH) } != 0 {
            return Err(io::Error::last_os_error());
        }
        self.decoder = Decoder::default();
        self.position = 0;
        self.length = 0;
        self.utf8 = Utf8::default();
        self.escape_started = None;
        Ok(())
    }

    pub(super) fn read(&mut self) -> io::Result<Option<Event>> {
        self.read_inner(None)
    }

    pub(super) fn read_timeout(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        self.read_inner(Some(timeout))
    }

    fn read_inner(&mut self, timeout: Option<Duration>) -> io::Result<Option<Event>> {
        let started = Instant::now();
        let mut first_poll = true;
        loop {
            self.decoder.expire_graphics_query();
            if let Some(event) = self.decoder.next_event() {
                return Ok(Some(event));
            }
            if self.decoder.graphics_changed {
                return Ok(None);
            }
            // Finish already-owned bytes before expiring Escape. Returning an
            // event retains all following keys/replies for the next read.
            while self.position < self.length {
                let byte = self.bytes[self.position];
                self.position += 1;
                let Some(character) = self.utf8.push(byte)? else {
                    continue;
                };
                let modifiers = if character.is_uppercase() {
                    KeyModifiers::SHIFT
                } else {
                    KeyModifiers::NONE
                };
                let event = self.decoder.character(character, modifiers);
                self.escape_started = self.decoder.has_escape().then(Instant::now);
                if event.is_some() || self.decoder.graphics_changed {
                    return Ok(event);
                }
            }
            let remaining = timeout.map(|limit| limit.saturating_sub(started.elapsed()));
            if !first_poll && remaining == Some(Duration::ZERO) {
                return Ok(None);
            }
            let escape = self
                .escape_started
                .map(|start| Duration::from_millis(50).saturating_sub(start.elapsed()));
            let wait = match (remaining, escape) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (left, right) => left.or(right),
            };
            let wait = match (wait, self.decoder.graphics_wait()) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (left, right) => left.or(right),
            };
            let milliseconds = wait.map_or(-1, |wait| {
                wait.as_millis()
                    .saturating_add(u128::from(wait.subsec_nanos() % 1_000_000 != 0))
                    .min(i32::MAX as u128) as i32
            });
            let mut fds = [
                libc::pollfd {
                    fd: self.input,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.resize.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            first_poll = false;
            // SAFETY: fds is a live writable array for exactly two descriptors.
            let ready =
                unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, milliseconds) };
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if ready == 0 {
                if self
                    .escape_started
                    .is_some_and(|start| start.elapsed() >= Duration::from_millis(50))
                {
                    self.escape_started = None;
                    return Ok(Some(self.decoder.escape_timeout()));
                }
                continue;
            }
            if fds[1].revents & libc::POLLIN != 0 {
                let mut signals = [0; 128];
                match self.resize.read(&mut signals) {
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error),
                }
                let (width, height) = crossterm::terminal::size()?;
                return Ok(Some(Event::Resize(width, height)));
            }
            if fds[0].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
                // SAFETY: the buffer is writable for its full length; this is
                // the application's only read from the terminal input fd.
                let count = unsafe {
                    libc::read(self.input, self.bytes.as_mut_ptr().cast(), self.bytes.len())
                };
                if count < 0 {
                    let error = io::Error::last_os_error();
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) {
                        continue;
                    }
                    return Err(error);
                }
                if count == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "terminal input ended",
                    ));
                }
                self.position = 0;
                self.length = count as usize;
            } else if fds
                .iter()
                .any(|fd| fd.revents & (libc::POLLERR | libc::POLLNVAL) != 0)
            {
                return Err(io::Error::other("terminal input descriptor failed"));
            }
        }
    }
}

impl Drop for TerminalInput {
    fn drop(&mut self) {
        signal_hook::low_level::unregister(self.resize_signal);
    }
}

#[derive(Default)]
struct Utf8 {
    bytes: [u8; 4],
    length: usize,
}

impl Utf8 {
    fn push(&mut self, byte: u8) -> io::Result<Option<char>> {
        self.bytes[self.length] = byte;
        self.length += 1;
        match std::str::from_utf8(&self.bytes[..self.length]) {
            Ok(text) => {
                let character = text.chars().next();
                self.length = 0;
                Ok(character)
            }
            Err(error) if error.error_len().is_none() && self.length < self.bytes.len() => Ok(None),
            Err(_) => {
                self.length = 0;
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid terminal UTF-8",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent};

    #[test]
    fn utf8_and_combining_characters_survive_each_raw_read_boundary() {
        let text = "a한😀e\u{301}";
        for split in 0..=text.len() {
            let mut utf8 = Utf8::default();
            let mut decoder = Decoder::default();
            let mut events = Vec::new();
            for chunk in [&text.as_bytes()[..split], &text.as_bytes()[split..]] {
                for byte in chunk {
                    if let Some(character) = utf8.push(*byte).unwrap()
                        && let Some(event) = decoder.character(character, KeyModifiers::NONE)
                    {
                        events.push(event);
                    }
                }
            }
            assert_eq!(
                events,
                text.chars()
                    .map(|character| {
                        Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
                    })
                    .collect::<Vec<_>>()
            );
        }
    }
}
