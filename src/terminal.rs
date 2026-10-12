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

#[derive(Default)]
struct Modes {
    active: AtomicU8,
}

impl Modes {
    fn enter(&self) -> io::Result<()> {
        // Mark before each operation: a failed write may still have sent its escape.
        if !is_raw_mode_enabled()? {
            self.active.fetch_or(RAW, Ordering::SeqCst);
            enable_raw_mode()?;
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
        // The panic hook and normal return share one restoration claim, without a
        // mutex that could deadlock if a terminal operation itself panics.
        let active = self.active.swap(0, Ordering::SeqCst);
        let mut error = None;
        let mut stdout = io::stdout();
        if active & RAW != 0 {
            record(&mut error, "disable raw mode", disable_raw_mode());
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

pub fn run<F>(body: F) -> io::Result<()>
where
    F: FnOnce(&mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()>,
{
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Eevee requires interactive stdin and stdout terminals; use --help for usage",
        ));
    }

    let modes = Arc::new(Modes::default());
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
        // A new fullscreen buffer has no cursor to preserve. Terminal::clear
        // queries the terminal cursor and can time out on otherwise usable PTYs.
        terminal.backend_mut().clear()?;
        terminal.set_cursor_position((0, 0))?;
        body(&mut terminal)
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
