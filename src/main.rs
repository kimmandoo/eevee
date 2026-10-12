mod app;
mod artwork;
mod config;
mod context;
mod domain;
mod input;
mod native;
mod privacy;
mod repo;
mod runner;
mod store;
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
        Ok(Command::Export { data_dir, output }) => {
            match store::export_readable(&data_dir, &output) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("ev: {error}");
                    ExitCode::FAILURE
                }
            }
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
    let project = std::env::current_dir()?.canonicalize()?;
    let cwd = project
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "project path must be Unicode"))?
        .to_owned();
    let mut artwork = artwork::Artwork::load(&config);
    terminal::run(native::cleanup, |terminal| {
        let store =
            store::Worker::start(config.data_dir.clone(), cwd.clone(), config.storage_policy)?;
        let repository = repo::Worker::start(project.clone())?;
        let sources = context::Worker::start(config.context_exclusions.clone())?;
        let execution = runner::Worker::start()?;
        let size = terminal.size()?;
        let mut app = App::new(
            cwd.chars().filter(|c| !c.is_control()).collect(),
            size.width,
            size.height,
        );
        app.linear = config.layout == config::LayoutMode::Linear;
        app.policy = config.storage_policy;
        app.execution.set_capture_capacity(config.capture_capacity);
        let mut graphics = native::NativeGraphics::new(&config);
        let mut dirty = true;
        let mut targets = Vec::with_capacity(9);
        let mut input = input::Input::new()?;
        let mut pending_save: Option<store::Snapshot> = None;
        let mut outstanding_store = 1_usize; // Initial load also yields one event.
        let mut outstanding_repo = 1_usize;
        let mut outstanding_sources = 0_usize;
        let mut context_task = None;
        loop {
            let activity = app.execution.activity_label();
            if let Some(action) = app.execution.drive(&execution, app.task_id, &project) {
                app.apply_execution_action(action);
                dirty = true;
            }
            dirty |= activity != app.execution.activity_label();
            if context_task != Some(app.task_id) {
                context_task = Some(app.task_id);
                app.sources.reset(app.task_id);
            }
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
            if app.execution.repo_requested() && outstanding_repo == 0 && repository.refresh() {
                app.execution.repo_started();
                outstanding_repo += 1;
            }
            if sources.try_request(&mut app.sources.pending) {
                outstanding_sources += 1;
            }
            if store.try_context(&mut app.sources.pending_store) {
                outstanding_store += 1;
            }
            if let Some(request) = app.execution.take_store_request() {
                let mut pending = Some(request);
                if store.try_run(&mut pending) {
                    outstanding_store += 1;
                } else if let Some(request) = pending {
                    app.execution.requeue_store_request(request);
                }
            }
            if app.repo_refresh && repository.refresh() {
                app.repo_refresh = false;
                outstanding_repo += 1;
            }
            if app.retry_storage && store.retry() {
                app.retry_storage = false;
                outstanding_store += 1;
            }
            if store.try_manage(&mut app.management) {
                outstanding_store += 1;
            }
            if let Some(id) = app.switch_task
                && store.try_select(id)
            {
                app.switch_task = None;
                outstanding_store += 1;
            }
            if app.save_requested
                && !app.transitioning
                && !(app.task_id == 0 && outstanding_store > 0)
                && matches!(
                    app.storage,
                    app::StorageState::Saved | app::StorageState::Saving
                )
            {
                if pending_save.as_ref().is_none_or(|snapshot| {
                    snapshot.revision != app.revision || snapshot.id != app.task_id
                }) {
                    pending_save = Some(app.snapshot(&cwd));
                }
                if store.try_save(&mut pending_save) {
                    app.save_requested = false;
                    outstanding_store += 1;
                }
            }
            // Only outstanding I/O uses a bounded wait; an idle saved UI blocks.
            let pending = outstanding_store > 0
                || outstanding_repo > 0
                || outstanding_sources > 0
                || app.sources.pending.is_some()
                || app.sources.pending_store.is_some()
                || app.execution.busy()
                || execution.is_active()
                || app.repo_refresh
                || app.retry_storage
                || app.switch_task.is_some()
                || app.management.is_some()
                || (app.save_requested && matches!(app.storage, app::StorageState::Saving));
            let event = if pending {
                input.read_timeout(std::time::Duration::from_millis(30))?
            } else {
                Some(input.read()?)
            };
            dirty = false;
            let resized = matches!(event, Some(event::Event::Resize(_, _)));
            if resized {
                graphics.discover_cell();
            }
            if let Some(event) = event {
                dirty |= app.update(event);
            }
            while let Some(event) = store.try_event() {
                outstanding_store = outstanding_store.saturating_sub(1);
                if matches!(&event, store::Event::Managed { .. }) {
                    pending_save = None;
                }
                let restored =
                    matches!(&event, store::Event::Loaded(_) | store::Event::Selected(_));
                app.store_event(event);
                if restored && config.layout_explicit {
                    app.linear = config.layout == config::LayoutMode::Linear;
                }
                dirty = true;
            }
            while let Some(event) = repository.try_event() {
                outstanding_repo = outstanding_repo.saturating_sub(1);
                let succeeded = matches!(&event, repo::Event::Updated(_));
                app.repo_event(event);
                app.sources.observed_repository(
                    app.task_id,
                    if succeeded {
                        app.repository.as_ref()
                    } else {
                        None
                    },
                );
                app.execution.repo_observed(if succeeded {
                    app.repository.as_deref()
                } else {
                    None
                });
                dirty |= matches!(
                    app.overlay,
                    Some(
                        app::Overlay::Repository | app::Overlay::Sources | app::Overlay::Execution
                    )
                );
            }
            while let Some(event) = sources.try_event() {
                outstanding_sources = outstanding_sources.saturating_sub(1);
                app.sources.event(app.task_id, event);
                dirty |= app.overlay == Some(app::Overlay::Sources);
            }
            while let Some(event) = execution.try_event() {
                app.execution.runner_event(event);
                dirty = true;
            }
            app.settle_exit();
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
