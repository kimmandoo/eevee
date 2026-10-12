mod app;
mod config;
mod domain;
mod terminal;
mod ui;

use std::{io, process::ExitCode};

use app::App;
use config::{Command, Config};
use crossterm::event;

fn main() -> ExitCode {
    match config::parse() {
        Ok(Command::Help) => {
            println!("{}", config::HELP);
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("eevee {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Command::Run(config)) => match run(config) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("eevee: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("eevee: {error}\nTry 'eevee --help' for usage.");
            ExitCode::from(2)
        }
    }
}

fn run(config: Config) -> io::Result<()> {
    let mut cwd = std::env::current_dir()?.to_string_lossy().into_owned();
    // Paths are untrusted terminal text too; never emit embedded control bytes.
    cwd.retain(|character| !character.is_control());
    terminal::run(|terminal| {
        let size = terminal.size()?;
        let mut app = App::new(cwd, size.width, size.height);
        let mut dirty = true;
        loop {
            if dirty {
                terminal.draw(|frame| ui::render(frame, &app, &config))?;
            }
            // Blocking input means zero idle redraws and no timer wakeups.
            dirty = app.update(event::read()?);
            if app.quit {
                return Ok(());
            }
        }
    })
}
