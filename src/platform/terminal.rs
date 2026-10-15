use std::io::{self, IsTerminal, Stdout, Write};
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::style::{Attribute, ResetColor, SetAttribute};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    is_raw_mode_enabled,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};

const RAW: u8 = 1;
const ALTERNATE: u8 = 2;
const CURSOR: u8 = 4;
const PASTE: u8 = 8;
const SYNCHRONIZED: u8 = 32;
const BEGIN_FRAME: &[u8] = b"\x1b[?2026h";
const END_FRAME: &[u8] = b"\x1b[?2026l";
#[cfg(windows)]
const WINDOWS_INPUT: u8 = 16;

pub(crate) struct Modes {
    active: AtomicU8,
    cleanup: fn() -> io::Result<()>,
    #[cfg(windows)]
    original_input_mode: u32,
    #[cfg(unix)]
    original_termios: libc::termios,
}

/// DEC 2026 is ignored by terminals without synchronized-output support. Keep
/// ordinary drawing/clearing intact for those terminals, and never hold a frame
/// open while waiting for input or lending the terminal to another application.
pub(crate) struct Frame<'a, W: Write> {
    active: &'a AtomicU8,
    output: W,
}

impl<'a, W: Write> Frame<'a, W> {
    fn begin(active: &'a AtomicU8, output: W) -> io::Result<Self> {
        let mut frame = Self { active, output };
        // Claim before writing: even an error can have published the escape.
        active.fetch_or(SYNCHRONIZED, Ordering::SeqCst);
        frame.output.write_all(BEGIN_FRAME)?;
        frame.output.flush()?;
        Ok(frame)
    }

    pub fn finish(mut self) -> io::Result<()> {
        end_frame(self.active, &mut self.output)
    }
}

impl<W: Write> Drop for Frame<'_, W> {
    fn drop(&mut self) {
        // Also runs on draw/paint errors and unwinding. The panic hook restores
        // before diagnostics; its claim makes this a no-op after that cleanup.
        let _ = end_frame(self.active, &mut self.output);
    }
}

fn end_frame(active: &AtomicU8, output: &mut impl Write) -> io::Result<()> {
    if active.load(Ordering::SeqCst) & SYNCHRONIZED != 0 {
        output.write_all(END_FRAME)?;
        output.flush()?;
        // Retain the claim after an I/O error so terminal restoration retries.
        active.fetch_and(!SYNCHRONIZED, Ordering::SeqCst);
    }
    Ok(())
}

impl Modes {
    pub fn frame(&self) -> io::Result<Frame<'_, Stdout>> {
        Frame::begin(&self.active, io::stdout())
    }

    /// The caller pauses its sole input reader before lending the terminal.
    /// Restore every owned mode even if entering the native UI failed, then
    /// rebuild the fullscreen terminal state before allowing application input.
    pub fn suspend<T>(&self, body: impl FnOnce() -> T) -> io::Result<T> {
        self.restore()?;
        let result = body();
        // A failed/killed native UI may leave raw mode or its alternate screen
        // enabled. Normalize OS state, not only Crossterm's cached mode flag.
        disable_raw_mode()?;
        self.restore()?;
        execute!(
            io::stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen,
            ResetColor,
            SetAttribute(Attribute::Reset),
            Show
        )?;
        self.enter()?;
        Ok(result)
    }

    fn enter(&self) -> io::Result<()> {
        // Mark before each operation: a failed write may still have sent its escape.
        if !is_raw_mode_enabled()? {
            self.active.fetch_or(RAW, Ordering::SeqCst);
            enable_raw_mode()?;
        }
        #[cfg(windows)]
        {
            self.active.fetch_or(WINDOWS_INPUT, Ordering::SeqCst);
            super::input::enable_vt_input()?;
        }
        let mut stdout = io::stdout();
        self.active.fetch_or(ALTERNATE, Ordering::SeqCst);
        execute!(stdout, EnterAlternateScreen)?;
        self.active.fetch_or(CURSOR, Ordering::SeqCst);
        execute!(stdout, Hide)?;
        self.active.fetch_or(PASTE, Ordering::SeqCst);
        execute!(stdout, EnableBracketedPaste)?;
        Ok(())
    }

    fn restore(&self) -> io::Result<()> {
        let mut error = None;
        let mut stdout = io::stdout();
        // End before native cleanup, mode changes, suspension, or panic output.
        record(
            &mut error,
            "end synchronized frame",
            end_frame(&self.active, &mut stdout),
        );
        // Preserve a failed End claim for another restoration attempt.
        let active = self.active.fetch_and(SYNCHRONIZED, Ordering::SeqCst);
        record(&mut error, "clear owned graphics", (self.cleanup)());
        if active & RAW != 0 {
            record(&mut error, "disable raw mode", disable_raw_mode());
        }
        #[cfg(windows)]
        {
            record(
                &mut error,
                "restore original Windows input mode",
                super::input::restore_mode(self.original_input_mode),
            );
        }
        #[cfg(unix)]
        {
            // SAFETY: snapshot belongs to this terminal and remains valid.
            let restored = unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original_termios)
            };
            record(
                &mut error,
                "restore original terminal input",
                if restored == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                },
            );
        }
        if active & PASTE != 0 {
            record(
                &mut error,
                "disable bracketed paste",
                execute!(stdout, DisableBracketedPaste),
            );
        }
        if active & ALTERNATE != 0 {
            record(
                &mut error,
                "reset terminal colors",
                execute!(stdout, ResetColor),
            );
            record(
                &mut error,
                "reset terminal attributes",
                execute!(stdout, SetAttribute(Attribute::Reset)),
            );
            record(
                &mut error,
                "leave alternate screen",
                execute!(stdout, LeaveAlternateScreen),
            );
        }
        if active & CURSOR != 0 {
            record(&mut error, "show cursor", execute!(stdout, Show));
        }
        error.map_or(Ok(()), Err)
    }
}

fn record(error: &mut Option<io::Error>, operation: &str, result: io::Result<()>) {
    if let Err(next) = result {
        *error = Some(match error.take() {
            Some(previous) => io::Error::other(format!("{previous}; {operation}: {next}")),
            None => io::Error::new(next.kind(), format!("{operation}: {next}")),
        });
    }
}

/// Invalidate the fullscreen buffers without requiring a cursor-report reply.
pub fn clear(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    terminal.backend_mut().clear()?;
    terminal.current_buffer_mut().reset();
    terminal.swap_buffers();
    terminal.set_cursor_position((0, 0))
}

pub fn run<F>(cleanup: fn() -> io::Result<()>, body: F) -> io::Result<()>
where
    F: FnOnce(&mut Terminal<CrosstermBackend<Stdout>>, &Modes) -> io::Result<()>,
{
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Eevee requires interactive stdin and stdout terminals; use --help for usage",
        ));
    }

    let modes = Arc::new(Modes {
        active: AtomicU8::new(0),
        cleanup,
        #[cfg(windows)]
        original_input_mode: super::input::capture_mode()?,
        #[cfg(unix)]
        original_termios: {
            let mut value = std::mem::MaybeUninit::<libc::termios>::uninit();
            // SAFETY: tcgetattr initializes value on success.
            if unsafe { libc::tcgetattr(libc::STDIN_FILENO, value.as_mut_ptr()) } != 0 {
                return Err(io::Error::last_os_error());
            }
            unsafe { value.assume_init() }
        },
    });
    let original_hook = Arc::new(panic::take_hook());
    let panic_modes = Arc::clone(&modes);
    let previous_hook = Arc::clone(&original_hook);
    panic::set_hook(Box::new(move |info| {
        // Restore before the original hook prints, so diagnostics survive leaving
        // the alternate screen. Report cleanup errors without panicking again.
        if let Err(error) = panic_modes.restore() {
            let _ = writeln!(io::stderr(), "Eevee terminal restoration failed: {error}");
        }
        previous_hook(info);
    }));

    // Catch only to restore the process-wide hook outside unwinding. The original
    // panic is resumed unchanged, not converted into a successful application exit.
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        modes.enter()?;
        let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        body(&mut terminal, &modes)
    }));
    let restored = modes.restore();
    drop(panic::take_hook());
    let original_hook = Arc::try_unwrap(original_hook)
        .unwrap_or_else(|_| unreachable!("temporary panic hook retained its predecessor"));
    panic::set_hook(original_hook);

    match result {
        Ok(Ok(())) => restored,
        Ok(Err(error)) => match restored {
            Ok(()) => Err(error),
            Err(restoration) => Err(io::Error::new(
                error.kind(),
                format!("{error}; terminal restoration failed: {restoration}"),
            )),
        },
        Err(payload) => panic::resume_unwind(payload),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Output {
        bytes: Vec<u8>,
        fail_next: bool,
        fail_flush: bool,
    }

    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if std::mem::take(&mut self.fail_next) {
                return Err(io::Error::other("write failed"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            if std::mem::take(&mut self.fail_flush) {
                return Err(io::Error::other("flush failed"));
            }
            Ok(())
        }
    }

    #[test]
    fn frame_publishes_clear_cells_and_graphics_together() {
        let active = AtomicU8::new(RAW | ALTERNATE);
        let mut output = Output::default();
        let frame = Frame::begin(&active, &mut output).unwrap();
        frame
            .output
            .write_all(b"\x1b[2Jcells-native-paint")
            .unwrap();
        frame.finish().unwrap();
        assert_eq!(
            output.bytes,
            b"\x1b[?2026h\x1b[2Jcells-native-paint\x1b[?2026l"
        );
        assert_eq!(active.load(Ordering::SeqCst), RAW | ALTERNATE);
    }

    #[test]
    fn frame_ends_on_body_error_and_unwind() {
        let active = AtomicU8::new(0);
        let mut output = Output::default();
        let failed = (|| -> io::Result<()> {
            let _frame = Frame::begin(&active, &mut output)?;
            Err(io::Error::other("draw failed"))
        })();
        assert!(failed.is_err());
        let panicked = panic::catch_unwind(AssertUnwindSafe(|| {
            let _frame = Frame::begin(&active, &mut output).unwrap();
            panic!("paint failed");
        }));
        assert!(panicked.is_err());
        assert_eq!(
            output.bytes,
            b"\x1b[?2026h\x1b[?2026l\x1b[?2026h\x1b[?2026l"
        );
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn failed_begin_and_end_still_restore() {
        let active = AtomicU8::new(0);
        let mut output = Output {
            fail_flush: true,
            ..Output::default()
        };
        assert!(Frame::begin(&active, &mut output).is_err());
        assert_eq!(output.bytes, b"\x1b[?2026h\x1b[?2026l");
        let frame = Frame::begin(&active, &mut output).unwrap();
        frame.output.fail_next = true;
        assert!(frame.finish().is_err());
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert!(output.bytes.ends_with(END_FRAME));
    }

    #[test]
    fn restoration_ends_before_guard_drop_without_duplicate_output() {
        let active = AtomicU8::new(RAW | SYNCHRONIZED);
        let mut output = Output::default();
        end_frame(&active, &mut output).unwrap();
        // Simulates the panic hook/suspension claiming End before unwinding.
        drop(Frame {
            active: &active,
            output: &mut output,
        });
        assert_eq!(output.bytes, END_FRAME);
        assert_eq!(active.load(Ordering::SeqCst), RAW);
    }
}
