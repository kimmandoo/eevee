mod app;
mod artwork;
mod config;
mod domain;
mod input;
mod native;
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
            println!("ev {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Command::Run(config)) => match run(config) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("ev: {error}");
                ExitCode::FAILURE
            }
        },
        Ok(Command::Download(config)) => match artwork::download(&config) {
            Ok(message) => {
                println!("{message}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("ev: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("ev: {error}\nTry 'ev --help' for usage.");
            ExitCode::from(2)
        }
    }
}

fn run(config: Config) -> io::Result<()> {
    let mut cwd = std::env::current_dir()?.to_string_lossy().into_owned();
    // Paths are untrusted terminal text too; never emit embedded control bytes.
    cwd.retain(|character| !character.is_control());
    let mut artwork = artwork::Artwork::load(&config);
    terminal::run(native::cleanup, |terminal| {
        let size = terminal.size()?;
        let mut app = App::new(cwd, size.width, size.height);
        app.linear = config.layout == config::LayoutMode::Linear;
        let mut graphics = native::NativeGraphics::new(&config);
        let mut dirty = true;
        let mut targets = Vec::with_capacity(9);
        let mut input = input::Input::new()?;
        loop {
            if dirty {
                // ConPTY's legacy cell projection can turn changed blanks into
                // cursor moves over old text, especially with indexed colors.
                // Repaint only on state changes; idle input stays blocking.
                #[cfg(windows)]
                terminal.clear()?;
                ui::portrait_targets(
                    &app,
                    ratatui::layout::Rect::new(0, 0, app.width, app.height),
                    &mut targets,
                );
                if !graphics.is_native() {
                    artwork.prepare(&targets, ui::image_background(&config));
                }
                graphics.prepare(&artwork, &targets)?;
                terminal.draw(|frame| {
                    ui::render(frame, &mut app, &config, &mut artwork, graphics.status());
                    graphics.render(frame.buffer_mut());
                    #[cfg(windows)]
                    ui::compose_console_graphemes(frame.buffer_mut());
                })?;
            }
            // One input owner; kernel cell-size discovery never reads stdin.
            let event = input.read()?;
            let resized = matches!(event, event::Event::Resize(_, _));
            if resized {
                graphics.discover_cell();
            }
            dirty = app.update(event);
            #[cfg(windows)]
            {
                dirty |= resized;
            }
            if app.quit {
                return Ok(());
            }
        }
    })
}
