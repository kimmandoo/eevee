mod acp;
mod agent;
mod app;
mod artwork;
mod auth;
mod claude;
mod codex;
mod config;
mod context;
mod distribution;
mod domain;
mod handoff;
mod input;
mod memory;
mod native;
mod privacy;
mod provider;
mod repo;
mod runner;
mod store;
mod terminal;
mod ui;
mod workflows;

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
        Ok(Command::Update { check_only }) => match distribution::update(check_only) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("ev: {error}");
                ExitCode::FAILURE
            }
        },
        Ok(Command::Uninstall) => match distribution::uninstall() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("ev: {error}");
                ExitCode::FAILURE
            }
        },
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
    terminal::run(native::cleanup, |terminal, modes| {
        let store =
            store::Worker::start(config.data_dir.clone(), cwd.clone(), config.storage_policy)?;
        let repository = repo::Worker::start(project.clone())?;
        let sources = context::Worker::start(config.context_exclusions.clone())?;
        let execution = runner::Worker::start()?;
        let agents = agent::Worker::start(project.clone())?;
        let artwork_worker = artwork::Worker::start(config.clone())?;
        let planning = provider::Worker::start(
            project.clone(),
            config.data_dir.clone(),
            config.context_exclusions.clone(),
        )?;
        let size = terminal.size()?;
        let mut app = App::new(
            cwd.chars().filter(|c| !c.is_control()).collect(),
            size.width,
            size.height,
        );
        app.linear = config.layout == config::LayoutMode::Linear;
        app.policy = config.storage_policy;
        app.images_enabled = config.images != config::ImageMode::Off;
        app.execution.set_capture_capacity(config.capture_capacity);
        let mut graphics = native::NativeGraphics::new(&config);
        let mut dirty = true;
        let mut targets = Vec::with_capacity(9);
        let mut input = input::Input::new()?;
        let graphics_queries = config.images == config::ImageMode::Auto
            && config.color != config::ColorMode::Mono
            && !config.ascii
            && std::env::var_os("TMUX").is_none()
            && std::env::var_os("STY").is_none();
        if graphics_queries {
            input.query_graphics(&mut io::stdout().lock())?;
        }
        let mut pending_save: Option<store::Snapshot> = None;
        let mut outstanding_store = 1_usize; // Initial load also yields one event.
        let mut outstanding_repo = 1_usize;
        let mut outstanding_sources = 0_usize;
        let mut context_task = None;
        let mut outstanding_agents = 0_usize;
        loop {
            if let Some(request) = app.images_pending
                && artwork_worker.try_request(request)
            {
                app.images_pending = None;
            }
            if app.images_cancel {
                artwork_worker.cancel();
                app.images_cancel = false;
            }
            if let Some(plan) = app.agents.pending_auth.take() {
                if app.overlay != Some(app::Overlay::Agents)
                    || !app.authentication_ready()
                    || app.width < 80
                    || app.height < 24
                {
                    app.agents
                        .auth_returned(plan.profile, auth::Outcome::Refused);
                    dirty = true;
                } else {
                    input.discard_pending()?;
                    let outcome = modes.suspend(|| auth::run(&plan, &project))?;
                    // Native auth can change terminal dimensions/modes and provider
                    // credentials even on cancellation. Never reuse prior Run consent.
                    if !plan.method.is_status() {
                        app.authentication_returned();
                    }
                    input = input::Input::new()?;
                    input.discard_pending()?;
                    graphics = native::NativeGraphics::new(&config);
                    terminal::clear(terminal)?;
                    if graphics_queries {
                        input.query_graphics(&mut io::stdout().lock())?;
                    }
                    let size = terminal.size()?;
                    app.update(event::Event::Resize(size.width, size.height));
                    app.agents.auth_returned(plan.profile, outcome);
                    dirty = true;
                }
            }
            let activity = app.execution.activity_label();
            if let Some(action) = app.execution.drive(&execution, app.task_id, &project) {
                app.apply_execution_action(action);
                dirty = true;
            }
            dirty |= activity != app.execution.activity_label();
            if context_task != Some(app.task_id) {
                context_task = Some(app.task_id);
                app.sources.reset(app.task_id);
                app.handoff.reset(app.task_id);
                if app.planning.task_id() != app.task_id {
                    app.planning.reset(app.task_id);
                }
            }
            app.prepare_composer_sources();
            if app.handoff.pending.is_some() {
                app.handoff.process_pending();
                dirty = true;
            }
            if dirty {
                // ConPTY's legacy cell projection can turn changed blanks into
                // cursor moves over old text, especially with indexed colors.
                // Repaint only on state changes; idle input stays blocking.
                #[cfg(windows)]
                terminal::clear(terminal)?;
                ui::portrait_targets(
                    &app,
                    ratatui::layout::Rect::new(0, 0, app.width, app.height),
                    &mut targets,
                );
                artwork.set_native_active(graphics.is_native());
                if !graphics.is_native() {
                    artwork.prepare(&targets, ui::image_background(&config));
                }
                if graphics.prepare(&artwork, &targets, ui::image_background(&config))? {
                    terminal::clear(terminal)?;
                }
                terminal.draw(|frame| {
                    ui::render(frame, &mut app, &config, &mut artwork, graphics.status());
                    graphics.render(frame.buffer_mut());
                    #[cfg(windows)]
                    ui::compose_console_graphemes(frame.buffer_mut());
                })?;
                graphics.paint()?;
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
            if app.memory_request.is_none()
                && let Some(mutation) = app.memory.pending.take()
            {
                app.memory_request = Some(store::MemoryRequest {
                    task_id: app.task_id,
                    mutation: Some(mutation),
                });
            }
            if store.try_memory(&mut app.memory_request) {
                outstanding_store += 1;
            }
            if let Some(request) = app.planning.pending_journal.front().cloned() {
                let mut pending = Some(request);
                if store.try_journal(&mut pending) {
                    app.planning.journal_started();
                    outstanding_store += 1;
                }
            }
            if agents.try_request(&mut app.agents.pending) {
                outstanding_agents += 1;
            }
            planning.try_request(&mut app.planning.pending);
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
                || outstanding_agents > 0
                || app.agents.pending.is_some()
                || artwork_worker.busy()
                || app.images_pending.is_some()
                || app.planning.busy()
                || app.planning.pending.is_some()
                || planning.busy()
                || app.sources.pending.is_some()
                || app.sources.pending_store.is_some()
                || app.memory_request.is_some()
                || app.memory.pending.is_some()
                || app.memory.saving
                || app.execution.busy()
                || execution.is_active()
                || app.repo_refresh
                || app.retry_storage
                || app.switch_task.is_some()
                || app.management.is_some()
                || input.graphics_pending()
                || (app.save_requested && matches!(app.storage, app::StorageState::Saving));
            let event = if pending {
                input.read_timeout(std::time::Duration::from_millis(30))?
            } else {
                input.read()?
            };
            dirty = false;
            let resized = matches!(event, Some(event::Event::Resize(_, _)));
            if resized && graphics_queries {
                input.query_graphics(&mut io::stdout().lock())?;
            }
            if let Some(capabilities) = input.take_graphics_update() {
                graphics.apply_capabilities(capabilities);
                dirty = true;
            }
            if let Some(event) = event {
                dirty |= app.update(event);
            }
            while let Some(event) = artwork_worker.try_event() {
                match event {
                    artwork::Event::Progress(message) => app.images_notice = message,
                    artwork::Event::Finished {
                        artwork: loaded,
                        result,
                    } => {
                        artwork = *loaded;
                        app.images_busy = false;
                        app.images_selected = 0;
                        app.images_notice = match result {
                            Ok(message) | Err(message) => message,
                        };
                    }
                }
                dirty = true;
            }
            while let Some(event) = agents.try_event() {
                outstanding_agents = outstanding_agents.saturating_sub(1);
                dirty |= app.agent_event(event);
            }
            while let Some(event) = planning.try_event() {
                if matches!(&event, provider::Event::Finished { .. })
                    && app.planning.operation == provider::Operation::Implement
                {
                    app.repo_refresh = true;
                }
                app.planning.event(event);
                dirty = true;
            }
            dirty |= app.settle_planning();
            while let Some(event) = store.try_event() {
                outstanding_store = outstanding_store.saturating_sub(1);
                if matches!(&event, store::Event::Managed { .. }) {
                    pending_save = None;
                }
                let restored =
                    matches!(&event, store::Event::Loaded(_) | store::Event::Selected(_));
                let hidden_memory_load = matches!(&event, store::Event::Memory { .. })
                    && app.overlay != Some(app::Overlay::Memory)
                    && !app.memory.saving;
                let revision = app.revision;
                app.store_event(event);
                if restored && config.layout_explicit {
                    app.linear = config.layout == config::LayoutMode::Linear;
                }
                dirty |= !hidden_memory_load || app.revision != revision;
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
                        app::Overlay::Repository
                            | app::Overlay::Sources
                            | app::Overlay::Execution
                            | app::Overlay::Workflows
                    )
                );
            }
            while let Some(event) = sources.try_event() {
                outstanding_sources = outstanding_sources.saturating_sub(1);
                app.sources.event(app.task_id, event);
                dirty |= app.overlay == Some(app::Overlay::Sources) || app.composer_menu.is_some();
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
