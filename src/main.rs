mod app;
mod command_help;
mod config;
mod domain;
mod i18n;
mod platform;
mod providers;
mod store;
mod ui;
mod workspace;

use std::{io, process::ExitCode};

use platform::{distribution, input, native, terminal};
use providers::{agent, auth};
use ui::artwork;
use workspace::{context, repo, runner};

use app::App;
use config::{Command, Config};
use crossterm::event;

fn main() -> ExitCode {
    match config::parse() {
        Ok(Command::Help) => {
            println!("{}", config::HELP);
            print!("Embedded display languages:");
            for language in i18n::Language::registered() {
                print!(" {}", language.tag());
            }
            println!();
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

/// Canonical identity is never terminal-sanitized; renderers own display escaping.
fn project_identity(directory: &std::path::Path) -> io::Result<(std::path::PathBuf, String)> {
    let project = directory.canonicalize()?;
    let cwd = project
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "project path must be Unicode"))?
        .to_owned();
    Ok((project, cwd))
}

fn run(config: Config) -> io::Result<()> {
    let (project, cwd) = project_identity(&std::env::current_dir()?)?;
    let mut artwork = artwork::Artwork::load(&config);
    terminal::run(native::cleanup, |terminal, modes| {
        let store =
            store::Worker::start(config.data_dir.clone(), cwd.clone(), config.storage_policy)?;
        let repository = repo::Worker::start(project.clone(), config.data_dir.clone())?;
        let sources = context::Worker::start(config.context_exclusions.clone())?;
        let mut execution = std::collections::BTreeMap::new();
        let agents = agent::Worker::start(project.clone())?;
        let artwork_worker = artwork::Worker::start(config.clone())?;
        let mut planning = std::collections::BTreeMap::new();
        let size = terminal.size()?;
        let mut app = App::new(cwd.clone(), size.width, size.height);
        app.planning.set_startup_provider(
            providers::direct::config::preferred_agent(),
            std::env::var_os("EV_PROVIDER").is_some(),
        );
        app.agents.active = app.planning.agent.index();
        app.agents.selected = app.agents.active;
        app.linear = config.layout == config::LayoutMode::Linear;
        app.policy = config.storage_policy;
        app.held.exclusions.clone_from(&config.context_exclusions);
        app.images_enabled = config.images != config::ImageMode::Off;
        app.execution.set_capture_capacity(config.capture_capacity);
        let mut graphics = native::NativeGraphics::new(&config);
        // Auto mode falls back to cell art once detection settles without native support;
        // it stays latched across resize re-queries so the portrait never blinks.
        let mut graphics_settled = false;
        let mut dirty = true;
        let mut animation_dirty = false;
        let mut clear_needed = true;
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
        let mut activity_tick: Option<std::time::Instant> = None;
        let mut mailbox_pending = false;
        let mut local_store_first = false;
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
                    || app.width < 60
                    || app.height < 18
                {
                    app.agents
                        .auth_returned(plan.profile, auth::Outcome::Refused);
                    dirty = true;
                } else {
                    input.discard_pending()?;
                    let completion = modes.suspend(|| auth::run(&plan, &project))?;
                    // Native auth can change terminal dimensions/modes and provider
                    // credentials even on cancellation. Never reuse prior Run consent.
                    if !plan.method.is_status() {
                        app.authentication_returned();
                    }
                    // A website-OAuth result arrives with its own session
                    // credential; a native login clears it only when Devin
                    // itself reports success.
                    if let Some(credential) = completion.credential {
                        app.set_devin_credential(Some(credential));
                    } else if matches!(
                        plan.method,
                        auth::Method::DevinBrowser | auth::Method::DevinManualToken
                    ) && completion.outcome == auth::Outcome::Authenticated
                    {
                        app.set_devin_credential(None);
                    }
                    app.agents.session_credential_until = app
                        .planning
                        .devin_credential
                        .as_ref()
                        .map(|credential| credential.expires_at.unwrap_or(u64::MAX));
                    input = input::Input::new()?;
                    input.discard_pending()?;
                    graphics = native::NativeGraphics::new(&config);
                    clear_needed = true;
                    if graphics_queries {
                        input.query_graphics(&mut io::stdout().lock())?;
                    }
                    let size = terminal.size()?;
                    app.update(event::Event::Resize(size.width, size.height));
                    app.connection_returned(plan.profile, completion.outcome);
                    dirty = true;
                }
            }
            let activity = app.execution.activity_label();
            dirty |= app.drive_executions(&mut execution, &project);
            // A deferred cancel-and-exit can become ready here, before input
            // polling. Do not wait for another key after ownership is settled.
            if app.records_allow_quit(app.quit) && app.preferences_allow_quit(true) {
                return Ok(());
            }
            if app.quit {
                app.quit = false;
                dirty = true;
            }
            dirty |= activity != app.execution.activity_label();
            if context_task != Some(app.task_id) {
                context_task = Some(app.task_id);
                app.sources.reset(app.task_id);
                app.sources.set_producer(Some(app.attribution()));
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
            dirty |= app.refresh_suggestion();
            dirty |= app.refresh_carry_over();
            dirty |= app.drive_moves();
            app.records_pairing_selected();
            dirty |= app.drive_records();
            // Restoration reports readiness, not a changed frame.
            app.records_restore_pairing();
            if dirty || animation_dirty {
                ui::portrait_targets(
                    &app,
                    ratatui::layout::Rect::new(0, 0, app.width, app.height),
                    &mut targets,
                );
                graphics_settled |= !input.graphics_pending();
                artwork.set_native_active(graphics.is_native());
                artwork.set_cell_fallback(graphics_settled);
                if !graphics.is_native() {
                    artwork.prepare(&targets, ui::image_background(&config));
                }
                // Publish native deletions, cell changes and placements together.
                let frame = modes.frame()?;
                clear_needed |=
                    graphics.prepare(&artwork, &targets, ui::image_background(&config))?;
                // Ordinary redraws retain cell differences on every platform.
                // Clearing on each input can flash even inside DEC 2026 when
                // ConPTY forwards frame markers before publishing its cells.
                let repaint_native = dirty || clear_needed;
                if clear_needed {
                    terminal::clear(terminal)?;
                    clear_needed = false;
                }
                terminal.draw(|frame| {
                    ui::render(frame, &mut app, &config, &mut artwork, graphics.status());
                    graphics.render(frame.buffer_mut());
                    #[cfg(windows)]
                    ui::compose_console_graphemes(frame.buffer_mut());
                })?;
                if repaint_native {
                    graphics.paint()?;
                }
                frame.finish()?;
            }
            if app.execution_repo_requested() && outstanding_repo == 0 && repository.refresh() {
                app.execution_repo_started();
                outstanding_repo += 1;
            }
            if app.save_due(std::time::Instant::now())
                && !(app.task_id == 0 && outstanding_store > 0)
            {
                if pending_save.as_ref().is_none_or(|snapshot| {
                    snapshot.revision != app.revision || snapshot.id != app.task_id
                }) {
                    pending_save = Some(app.snapshot(&cwd));
                }
                if store.try_save(&mut pending_save) {
                    app.save_submitted();
                    outstanding_store += 1;
                }
            }
            // Persist a newly prepared package receipt before evidence can be removed.
            let handoff_receipt_pending = app.records.local.iter().any(|intent| {
                matches!(intent, workspace::records::LocalIntent::HandoffReady { .. })
            });
            if local_store_first && !handoff_receipt_pending {
                outstanding_store += app.submit_execution_stores(&store);
            }
            outstanding_store += app.drive_providers(
                &mut planning,
                &store,
                &project,
                &config.data_dir,
                &config.context_exclusions,
            )?;
            if !local_store_first && !handoff_receipt_pending {
                outstanding_store += app.submit_execution_stores(&store);
            }
            local_store_first = !local_store_first;
            if sources.try_request(&mut app.sources.pending) {
                outstanding_sources += 1;
            }
            let context_mutates = app.sources.pending_store.as_ref().is_some_and(|request| {
                matches!(
                    request.operation,
                    store::ContextOperation::Pin(_)
                        | store::ContextOperation::Remove(_)
                        | store::ContextOperation::Purge(_)
                )
            });
            if (!context_mutates || app.records.local.is_empty())
                && store.try_context(&mut app.sources.pending_store)
            {
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
            if store.try_moves(&mut app.moves.pending) {
                outstanding_store += 1;
            }
            outstanding_store += app.submit_record_store(&store);
            if agents.try_request(&mut app.agents.pending) {
                outstanding_agents += 1;
            }
            if app.repo_refresh && repository.refresh() {
                app.repo_refresh = false;
                outstanding_repo += 1;
            }
            if app.retry_storage && store.retry() {
                app.retry_storage = false;
                app.retry_cooperation_storage();
                outstanding_store += 1;
            }
            if !handoff_receipt_pending && store.try_manage(&mut app.management) {
                outstanding_store += 1;
            }
            if store.try_party(&mut app.party_request) {
                app.party_inflight = true;
                outstanding_store += 1;
            }
            if let Some(id) = app.switch_task
                && store.try_select(id)
            {
                app.switch_task = None;
                outstanding_store += 1;
            }
            // Deferred persistence sleeps until due; only outstanding I/O polls.
            // No idle frame timer or output is introduced by either wait.
            let save_wait = app.save_wait(std::time::Instant::now());
            let pending = outstanding_store > 0
                || outstanding_repo > 0
                || outstanding_sources > 0
                || outstanding_agents > 0
                || app.agents.pending.is_some()
                || artwork_worker.busy()
                || app.images_pending.is_some()
                || app.planning_any_busy()
                || planning.values().any(providers::Worker::busy)
                || app.sources.pending.is_some()
                || app.sources.pending_store.is_some()
                || app.memory_request.is_some()
                || app.memory.pending.is_some()
                || app.memory.saving
                || app.moves.pending.is_some()
                || app.moves.inflight.is_some()
                || app.execution_any_pending()
                || execution.values().any(runner::Worker::is_active)
                || app.repo_refresh
                || app.retry_storage
                || app.switch_task.is_some()
                || app.management.is_some()
                || app.party_request.is_some()
                || app.party_inflight
                || mailbox_pending
                || input.graphics_pending();
            let wait = if pending {
                Some(std::time::Duration::from_millis(30))
            } else {
                save_wait
            };
            let event = if let Some(wait) = wait {
                input.read_timeout(wait)?
            } else {
                input.read()?
            };
            dirty = false;
            animation_dirty = false;
            mailbox_pending = false;
            let resized = matches!(event, Some(event::Event::Resize(_, _)));
            clear_needed |= resized;
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
            for index in 0..64 {
                let Some(event) = artwork_worker.try_event() else {
                    break;
                };
                mailbox_pending |= index == 63;
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
            for index in 0..64 {
                let Some(event) = agents.try_event() else {
                    break;
                };
                mailbox_pending |= index == 63;
                outstanding_agents = outstanding_agents.saturating_sub(1);
                dirty |= app.agent_event(event);
                dirty |= app.connection_settings_changed();
            }
            // Bound each mailbox drain so a hot producer cannot monopolize the
            // UI thread. Input (including consent and cancellation) comes first;
            // available stream updates are coalesced into the next single draw.
            let (provider_changed, provider_more) = app.poll_providers(&mut planning);
            dirty |= provider_changed;
            mailbox_pending |= provider_more;
            dirty |= app.settle_planning();
            dirty |= app.drive_catalogue();
            for index in 0..64 {
                let Some(event) = store.try_event() else {
                    break;
                };
                mailbox_pending |= index == 63;
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
            for index in 0..64 {
                let Some(event) = repository.try_event() else {
                    break;
                };
                mailbox_pending |= index == 63;
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
                let observed = app.repository.clone();
                app.execution_repo_observed(if succeeded { observed.as_deref() } else { None });
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
            for index in 0..64 {
                let Some(event) = sources.try_event() else {
                    break;
                };
                mailbox_pending |= index == 63;
                outstanding_sources = outstanding_sources.saturating_sub(1);
                app.sources.event(app.task_id, event);
                dirty = true;
            }
            let (execution_changed, execution_more) = app.poll_executions(&execution);
            dirty |= execution_changed;
            mailbox_pending |= execution_more;
            dirty |= app.settle_planning();
            app.settle_exit();
            // Only genuine, visible provider work animates. Local reviews,
            // permission prompts and idle screens keep their blocking/no-redraw
            // behavior; frame changes never mutate persisted task state.
            let animate = !config.no_animation
                && app.width >= 60
                && app.height >= 18
                && matches!(app.overlay, None | Some(app::Overlay::Planning))
                && app.confirmation.is_none()
                && app.planning.activity_label().is_some();
            if animate {
                let now = std::time::Instant::now();
                let last = activity_tick.get_or_insert(now);
                if now.duration_since(*last) >= std::time::Duration::from_millis(120) {
                    app.activity_frame = (app.activity_frame + 1) % 20;
                    *last = now;
                    animation_dirty = true;
                }
            } else {
                activity_tick = None;
                app.activity_frame = 0;
            }
            #[cfg(windows)]
            {
                dirty |= resized;
            }
            if app.records_allow_quit(app.quit) && app.preferences_allow_quit(true) {
                return Ok(());
            }
            if app.quit {
                app.quit = false;
                dirty = true;
            }
        }
    })
}

#[cfg(all(test, unix))]
mod project_identity_tests {
    use super::*;

    #[test]
    fn canonical_launch_alias_preserves_control_characters_in_app_identity() {
        let fixture = std::env::temp_dir().join(format!(
            "ev-project-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir(&fixture).unwrap();
        let actual = fixture.join("project\twith\ncontrols");
        std::fs::create_dir(&actual).unwrap();
        let alias = fixture.join("alias");
        std::os::unix::fs::symlink(&actual, &alias).unwrap();
        let (root, cwd) = project_identity(&alias).unwrap();
        assert_eq!(root, actual.canonicalize().unwrap());
        assert_eq!(std::path::Path::new(&cwd), root);
        assert!(cwd.contains('\t') && cwd.contains('\n'));
        let app = App::new(cwd.clone(), 120, 36);
        assert_eq!(app.cwd, cwd);
        assert_eq!(project_identity(&root).unwrap(), (root, cwd));
        std::fs::remove_dir_all(fixture).unwrap();
    }
}
